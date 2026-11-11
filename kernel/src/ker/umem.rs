//! Access to memory described by kernel calls: kernel buffers (procnto's
//! own threads) or user buffers of a process.
//!
//! User memory is never touched through TTBR0 here. Each user page is
//! translated through the owning process's tables and its frame mapped into
//! one of this CPU's two copy windows, so copies work whichever address
//! space is active (a receiving server's, a sending client's) and an
//! unmapped or read-only page gives EFAULT instead of a kernel fault.
//! A copy runs with IRQs masked (kernel calls already do; procnto's own
//! threads don't), so a CPU's windows are never used by two copies at once
//! and the copying thread can't move to another CPU meanwhile.

use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use super::errno::{EFAULT, EINVAL};
use super::process::{KERNEL, Process};
use crate::arch::mmu::{self, Prot};
use crate::arch::uspace::USER_END;
use crate::percpu;
use crate::vm::kmem;
use crate::vm::pmem::PAGE_SIZE;

/// Where a buffer lives.
#[derive(Clone, Copy)]
pub enum Space {
    Kernel,
    User(&'static Process),
}

impl Space {
    /// Process `p`'s space.
    ///
    /// # Safety
    ///
    /// `p` must outlive every use of the space: hold a `ProcessRef` to
    /// it meanwhile (the calling thread's own process lives as long as
    /// the thread).
    pub unsafe fn user(p: &Process) -> Self {
        // SAFETY: extended as the caller promises.
        Space::User(unsafe { &*(p as *const Process) })
    }

    /// The calling thread's space.
    pub fn current() -> Self {
        let p = super::process::current();
        if core::ptr::eq(p, &KERNEL) {
            Space::Kernel
        } else {
            Space::User(p)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Seg {
    pub base: u64,
    pub len: u64,
}

const NO_SEG: Seg = Seg { base: 0, len: 0 };
/// Segments a buffer keeps without a heap allocation.
const INLINE_SEGS: usize = 4;

/// A buffer's segments: up to `INLINE_SEGS` in place (kernel calls take
/// these on every message), more on the heap.
#[derive(Clone)]
pub enum Segs {
    Inline([Seg; INLINE_SEGS], usize),
    Heap(Vec<Seg>),
}

impl Segs {
    pub const fn new() -> Self {
        Segs::Inline([NO_SEG; INLINE_SEGS], 0)
    }

    pub fn one(s: Seg) -> Self {
        let mut a = [NO_SEG; INLINE_SEGS];
        a[0] = s;
        Segs::Inline(a, 1)
    }

    pub fn push(&mut self, s: Seg) {
        match self {
            Segs::Inline(a, n) if *n < INLINE_SEGS => {
                a[*n] = s;
                *n += 1;
            }
            Segs::Inline(a, n) => {
                let mut v = a[..*n].to_vec();
                v.push(s);
                *self = Segs::Heap(v);
            }
            Segs::Heap(v) => v.push(s),
        }
    }

    /// `push`, ENOMEM if the heap can't take one more.
    pub fn try_push(&mut self, s: Seg) -> Result<(), u32> {
        if let Segs::Heap(v) = self {
            v.try_reserve(1).map_err(|_| crate::ker::errno::ENOMEM)?;
        }
        self.push(s);
        Ok(())
    }
}

impl core::ops::Deref for Segs {
    type Target = [Seg];

    fn deref(&self) -> &[Seg] {
        match self {
            Segs::Inline(a, n) => &a[..*n],
            Segs::Heap(v) => v,
        }
    }
}

/// A gather/scatter buffer in one space.
#[derive(Clone)]
pub struct Buf {
    pub space: Space,
    pub segs: Segs,
}

impl Buf {
    pub const fn empty() -> Self {
        Buf {
            space: Space::Kernel,
            segs: Segs::new(),
        }
    }

    pub fn kernel(bytes: &[u8]) -> Self {
        Buf {
            space: Space::Kernel,
            segs: Segs::one(Seg {
                base: bytes.as_ptr() as u64,
                len: bytes.len() as u64,
            }),
        }
    }

    /// A kernel buffer to copy into (`copy`'s destination): mutably
    /// borrowed, so that the compiler doesn't take its contents as fixed.
    pub fn kernel_mut(bytes: &mut [u8]) -> Self {
        Buf {
            space: Space::Kernel,
            segs: Segs::one(Seg {
                base: bytes.as_mut_ptr() as u64,
                len: bytes.len() as u64,
            }),
        }
    }

    /// The buffer without its first `offset` bytes. A segment that wraps
    /// around the top of the range (a client's reply buffers are not
    /// checked for that, see `bounded`) moves to the top, where `access`
    /// refuses it.
    pub fn skip(&self, mut offset: u64) -> Buf {
        let mut segs = Segs::new();
        for s in self.segs.iter() {
            if offset >= s.len {
                offset -= s.len;
                continue;
            }
            segs.push(Seg {
                base: s.base.saturating_add(offset),
                len: s.len - offset,
            });
            offset = 0;
        }
        Buf {
            space: self.space,
            segs,
        }
    }

    /// The buffer's first `len` bytes (all of it, if it is shorter).
    pub fn take(&self, mut len: u64) -> Buf {
        let mut segs = Segs::new();
        for s in self.segs.iter() {
            if len == 0 {
                break;
            }
            let n = s.len.min(len);
            segs.push(Seg { base: s.base, len: n });
            len -= n;
        }
        Buf {
            space: self.space,
            segs,
        }
    }

    /// The bytes of the buffer, at most `u64::MAX`.
    pub fn total(&self) -> u64 {
        self.segs.iter().fold(0u64, |n, s| n.saturating_add(s.len))
    }

    /// The bytes of the buffer as `struct _msg_info` reports them: the
    /// sum wrapped around, as procnto's (measured, review IPC-4: a reply
    /// buffer of `UINT64_MAX` bytes is `dstmsglen` -1, one of 64 and
    /// `UINT64_MAX` is 63).
    pub fn wrapping_total(&self) -> u64 {
        self.segs.iter().fold(0u64, |n, s| n.wrapping_add(s.len))
    }

    /// Whether every segment lies within the user address space, its end
    /// computed without wrapping: what procnto checks on a client's send
    /// buffers at `MsgSendv()` and on a server's receive buffers as a
    /// message or pulse arrives, whether or not the bytes are used
    /// (EFAULT; measured, review IPC-4). A kernel buffer is the kernel's
    /// own.
    pub fn bounded(&self) -> bool {
        match self.space {
            Space::Kernel => true,
            Space::User(_) => self.segs.iter().all(Seg::bounded),
        }
    }

    /// `bounded` for the first segment alone: a client's reply buffers
    /// are checked so when a reply with any bytes comes, the others only
    /// where the reply reaches them (procnto, measured).
    pub fn first_bounded(&self) -> bool {
        match self.space {
            Space::Kernel => true,
            Space::User(_) => self.segs.iter().next().is_none_or(Seg::bounded),
        }
    }

    /// Whether the first byte of every segment with any bytes can be read:
    /// procnto refuses a client's send buffers with a segment starting in
    /// unmapped memory even when the server takes none of it, while a
    /// segment that runs on into unmapped memory passes (measured).
    pub fn heads_readable(&self) -> bool {
        match self.space {
            Space::Kernel => true,
            Space::User(p) => self
                .segs
                .iter()
                .filter(|s| s.len > 0)
                .all(|s| translate_cached(p, s.base, false).is_ok()),
        }
    }
}

impl Seg {
    fn bounded(&self) -> bool {
        self.base.checked_add(self.len).is_some_and(|end| end <= USER_END)
    }
}

/// `struct iovec` entries read from the caller at a time.
const PARTS_CHUNK: usize = 64;

/// Reserve this CPU's two copy windows (one page each, unmapped).
pub fn init_cpu() {
    let va = kmem::reserve(2).expect("copy windows");
    percpu::this().copy_window.store(va, Ordering::Relaxed);
}

/// Buffer description from a kernel call's (pointer, parts) pair in `space`:
/// negative parts = one buffer of -parts bytes (libc's non-vector calls),
/// otherwise an array of `parts` `struct iovec` { base, len }.
pub fn parse(space: Space, ptr: u64, parts: i64) -> Result<Buf, u32> {
    if parts < 0 {
        let len = parts.unsigned_abs();
        return Ok(Buf {
            space,
            segs: Segs::one(Seg { base: ptr, len }),
        });
    }
    // As many parts as the caller gives, as procnto (measured: 1025 and
    // 4096 work; review IPC-23, where 1024 was the most). They are read a
    // chunk at a time, so an array that isn't there fails with EFAULT
    // before the kernel allocates for it, and the segments grow only as
    // they are read (ENOMEM if the heap can't take them).
    let mut segs = Segs::new();
    let mut chunk = [[0u64; 2]; PARTS_CHUNK];
    let mut done = 0u64;
    while done < parts as u64 {
        let n = (parts as u64 - done).min(PARTS_CHUNK as u64) as usize;
        let base = ptr.checked_add(done * 16).ok_or(EFAULT)?;
        let dst = Buf::kernel(bytemuck_slice(&mut chunk[..n]));
        let src = Buf {
            space,
            segs: Segs::one(Seg { base, len: n as u64 * 16 }),
        };
        if copy(&dst, &src)? != n as u64 * 16 {
            return Err(EFAULT);
        }
        for &[base, len] in &chunk[..n] {
            segs.try_push(Seg { base, len })?;
        }
        done += n as u64;
    }
    Ok(Buf { space, segs })
}

fn bytemuck_slice(v: &mut [[u64; 2]]) -> &[u8] {
    // SAFETY: plain integers, viewed as bytes for the copy.
    unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 16) }
}

// ANCHOR: access
/// A kernel pointer to `len` bytes at `addr` in `space`, at most up to the
/// end of the page; `slot` selects the copy window for user memory.
fn access(
    space: Space,
    addr: u64,
    len: u64,
    write: bool,
    slot: u64,
) -> Result<(*mut u8, u64), u32> {
    match space {
        Space::Kernel => {
            if addr == 0 {
                return Err(EFAULT);
            }
            Ok((addr as *mut u8, len))
        }
        Space::User(p) => {
            if addr >= USER_END || len > USER_END - addr {
                return Err(EFAULT);
            }
            let (pa, cached) = translate_cached(p, addr, write)?;
            // RAM the process maps cacheable is reached through the linear
            // map; other memory (uncached buffers, device registers) through
            // a window, as no cacheable alias of it is used.
            let off = addr & (PAGE_SIZE - 1);
            if let Some(va) = kmem::linear(pa - off).filter(|_| cached) {
                return Ok(((va + off) as *mut u8, len.min(PAGE_SIZE - off)));
            }
            let cpu = percpu::this();
            let window = cpu.copy_window.load(Ordering::Relaxed) + slot * PAGE_SIZE;
            let frame = pa & !(PAGE_SIZE - 1);
            // Remapped only when it maps another frame.
            let mapped = &cpu.copy_window_pa[slot as usize];
            if mapped.load(Ordering::Relaxed) != frame {
                mmu::set_window(window, frame, Prot::Data).map_err(|_| EFAULT)?;
                mapped.store(frame, Ordering::Relaxed);
            }
            let off = addr & (PAGE_SIZE - 1);
            Ok(((window + off) as *mut u8, len.min(PAGE_SIZE - off)))
        }
    }
}
// ANCHOR_END: access

/// The buffer a copy could not access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Src,
    Dst,
}

impl From<Side> for u32 {
    fn from(_: Side) -> u32 {
        EFAULT
    }
}

/// A copy in flight into or out of a process's memory, counted in
/// `Process::copies` from before its first translation to its end, so that
/// an `munmap()` meanwhile waits before it frees the frames (`vm::map`;
/// review IPC-2). Made and dropped with IRQs masked only, so that a
/// counted copy always runs to its end without yielding the CPU. The `dsb` orders the count before the table walks that
/// follow: a translation of a page still mapped implies the count is seen.
struct InFlight(Option<&'static Process>);

impl InFlight {
    fn new(space: Space) -> Self {
        match space {
            Space::Kernel => Self(None),
            Space::User(p) => {
                p.copies.fetch_add(1, Ordering::SeqCst);
                // SAFETY: a barrier only.
                unsafe { core::arch::asm!("dsb ish", options(nostack, preserves_flags)) };
                Self(Some(p))
            }
        }
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        if let Some(p) = self.0 {
            p.copies.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Gather from `src` and scatter into `dst`; returns the bytes copied
/// (the smaller of the two sizes), or which side faulted. Each user space
/// is counted as in use (`InFlight`) throughout, with IRQs masked, so the
/// frames it reads or writes can't be freed under it.
pub fn copy(dst: &Buf, src: &Buf) -> Result<u64, Side> {
    let daif = crate::arch::irq_save();
    // Counted only while IRQs are masked: a thread counted and preempted
    // (procnto's own call with IRQs enabled) would keep an munmap() on its
    // CPU spinning for a copy that can't run.
    let r = {
        let _pins = (InFlight::new(src.space), InFlight::new(dst.space));
        copy_masked(dst, src)
    };
    crate::arch::irq_restore(daif);
    r
}

/// The bytes `copy_preemptible` moves between two chances to switch.
pub const CHUNK: u64 = 64 * 1024;

/// `copy`, for a long copy made with no lock held (a message, a reply, a
/// `MsgRead()`): in pieces of `CHUNK` bytes, each with IRQs masked and
/// counted (`InFlight`), and between pieces a moment in which interrupts
/// are taken and a higher-priority thread may have this CPU
/// (`sched::preemption_point`). As one piece, a 64 MiB message kept a
/// thread at a higher priority off its CPU for up to 176 ms, where procnto
/// takes 0.8 ms (measured, review IPC-10). The calling thread may be
/// switched out, and go on on another CPU, between pieces.
pub fn copy_preemptible(dst: &Buf, src: &Buf) -> Result<u64, Side> {
    if dst.total().min(src.total()) <= CHUNK {
        return copy(dst, src);
    }
    let mut done = 0;
    loop {
        let n = copy(&dst.skip(done).take(CHUNK), &src.skip(done))?;
        done += n;
        if n < CHUNK {
            return Ok(done);
        }
        crate::sched::preemption_point();
    }
}

fn copy_masked(dst: &Buf, src: &Buf) -> Result<u64, Side> {
    let (mut di, mut doff, mut si, mut soff, mut n) = (0, 0, 0, 0, 0);
    while di < dst.segs.len() && si < src.segs.len() {
        let (d, s) = (dst.segs[di], src.segs[si]);
        let len = (d.len - doff).min(s.len - soff);
        if len > 0 {
            // A segment wrapping around the top of the range moves to the
            // top, which `access` refuses.
            let (sp, slen) =
                access(src.space, s.base.saturating_add(soff), len, false, 0).map_err(|_| Side::Src)?;
            let (dp, dlen) =
                access(dst.space, d.base.saturating_add(doff), len, true, 1).map_err(|_| Side::Dst)?;
            let chunk = len.min(slen).min(dlen);
            // SAFETY: both pointers are valid for `chunk` bytes (kernel
            // buffers of kernel threads, or frames mapped in our windows).
            unsafe { core::ptr::copy(sp, dp, chunk as usize) };
            (doff, soff, n) = (doff + chunk, soff + chunk, n + chunk);
        }
        if doff == d.len {
            (di, doff) = (di + 1, 0);
        }
        if soff == s.len {
            (si, soff) = (si + 1, 0);
        }
    }
    Ok(n)
}

/// Copy `bytes` to `addr` in `space`.
pub fn write(space: Space, addr: u64, bytes: &[u8]) -> Result<(), u32> {
    let dst = Buf {
        space,
        segs: Segs::one(Seg {
            base: addr,
            len: bytes.len() as u64,
        }),
    };
    (copy(&dst, &Buf::kernel(bytes))? == bytes.len() as u64)
        .then_some(())
        .ok_or(EFAULT)
}

/// Copy `buf.len()` bytes from `addr` in `space` into `buf`.
pub fn read(space: Space, addr: u64, buf: &mut [u8]) -> Result<(), u32> {
    let src = Buf {
        space,
        segs: Segs::one(Seg {
            base: addr,
            len: buf.len() as u64,
        }),
    };
    (copy(&Buf::kernel(buf), &src)? == buf.len() as u64)
        .then_some(())
        .ok_or(EFAULT)
}

/// Run `f` on the 32-bit word at `addr` in `space` as an atomic (for the
/// `sync_t` objects user code also updates with atomic instructions). The
/// word must be aligned and writable.
pub fn with_atomic_u32<R>(
    space: Space,
    addr: u64,
    f: impl FnOnce(&core::sync::atomic::AtomicU32) -> R,
) -> Result<R, u32> {
    if !addr.is_multiple_of(4) {
        return Err(EINVAL);
    }
    let daif = crate::arch::irq_save();
    let r = {
        // Counted with IRQs masked, as in `copy`.
        let _pin = InFlight::new(space);
        access(space, addr, 4, true, 0).map(|(ptr, _)| {
            // SAFETY: `ptr` maps the (aligned, writable) word; with IRQs
            // masked the window stays ours.
            f(unsafe { &*(ptr as *const core::sync::atomic::AtomicU32) })
        })
    };
    crate::arch::irq_restore(daif);
    r
}

/// The physical address of user address `addr` (or `addr` itself for
/// kernel memory): identifies a synchronisation object across processes.
pub fn object_key(space: Space, addr: u64) -> Result<u64, u32> {
    match space {
        Space::Kernel => Ok(addr),
        Space::User(p) => translate(p, addr, false),
    }
}

/// The physical address of user address `addr` of `p`, faulting a lazy
/// page in (`vm::map::fault_in`) if need be.
fn translate(p: &Process, addr: u64, write: bool) -> Result<u64, u32> {
    translate_cached(p, addr, write).map(|(pa, _)| pa)
}

/// `translate`, and whether the page is mapped as normal cacheable memory.
fn translate_cached(p: &Process, addr: u64, write: bool) -> Result<(u64, bool), u32> {
    // If its tables are the ones loaded, the MMU translates.
    if let Some(r) = crate::arch::uspace::el0_translate(p.aspace().ttbr0(), addr, write) {
        return Ok(r);
    }
    p.aspace()
        .translate_cached(addr, write)
        .or_else(|| {
            crate::vm::map::fault_in(p, addr, write)
                .then(|| p.aspace().translate_cached(addr, write))
                .flatten()
        })
        .ok_or(EFAULT)
}
