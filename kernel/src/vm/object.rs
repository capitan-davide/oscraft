//! Memory objects: memory that is not one address space's, shared by
//! whoever maps it (`/dev/shmem`'s objects, see `proc::shmem`).
//!
//! An object has a size and page frames, allocated zeroed when it is
//! sized (`grow`, as procnto backs `ftruncate()` and `shm_ctl()` at once),
//! or when first written or mapped for the paths that only set the size. Its frames are freed when the
//! object is (the last `Arc` goes: it is unlinked, closed and unmapped).
//! When it shrinks, the bytes past its end are zeroed but the frames kept,
//! as processes may have them mapped. Frames can also be given to it
//! (`add_frames`: contiguous RAM it owns, or physical memory such as a
//! device's, which it neither zeroes nor frees).

use alloc::vec::Vec;

use crate::ker::errno::{EINVAL, ENOMEM};
use crate::sync::IrqSpinLock;
use crate::vm::kmem;
use crate::vm::pmem::{self, PAGE_SIZE};

/// The largest object (bounds the frame table): more is ENOMEM, as
/// procnto answers a size it can't back (measured with a length near the
/// top of the range, review PATHMEM-6).
const MAX_SIZE: u64 = 1 << 32;

pub struct MemObject {
    inner: IrqSpinLock<Inner>,
}

struct Inner {
    size: u64,
    /// Indexed by page; 0: not allocated yet (reads as zeros). `FOREIGN`
    /// marks a frame the object doesn't own.
    frames: Vec<u64>,
}

/// Tag of a frame that isn't the object's (physical memory given to it).
const FOREIGN: u64 = 1;

impl MemObject {
    pub const fn new() -> Self {
        Self {
            inner: IrqSpinLock::new(Inner {
                size: 0,
                frames: Vec::new(),
            }),
        }
    }

    pub fn size(&self) -> u64 {
        self.inner.lock().size
    }

    /// Change the size; what lies past a new, smaller end is zeroed (it
    /// reads as zeros if the object grows again).
    pub fn set_size(&self, size: u64) -> Result<(), u32> {
        if size > MAX_SIZE {
            return Err(ENOMEM);
        }
        let mut inner = self.inner.lock();
        if size < inner.size {
            // Zeroed, not freed: a process may have them mapped.
            let first = (size / PAGE_SIZE) as usize;
            let last = inner.size.div_ceil(PAGE_SIZE) as usize;
            for (i, &pa) in inner.frames.iter().enumerate().take(last).skip(first) {
                if pa != 0 && pa & FOREIGN == 0 {
                    let from = if i == first {
                        (size % PAGE_SIZE) as usize
                    } else {
                        0
                    };
                    kmem::with_frame(pa, |f| f[from..].fill(0));
                }
            }
        }
        inner.size = size;
        Ok(())
    }

    /// The frame of page `index` if it has one already (nothing is
    /// allocated): for a copy of the object page by page, where a page
    /// without a frame reads as zeros and needs no copying.
    pub fn present_frame(&self, index: u64) -> Option<u64> {
        let inner = self.inner.lock();
        let pa = *inner.frames.get(usize::try_from(index).ok()?)?;
        (pa != 0).then_some(pa & !FOREIGN)
    }

    /// Bytes of the object that have a frame (a sparse object, grown
    /// with `ftruncate()` and not written, has few).
    pub fn backed_bytes(&self) -> u64 {
        self.inner.lock().frames.iter().filter(|&&pa| pa != 0).count() as u64 * PAGE_SIZE
    }

    /// The frame of page `index`, allocated (zeroed) if need be.
    pub fn frame(&self, index: u64) -> Result<u64, u32> {
        if index >= MAX_SIZE / PAGE_SIZE {
            return Err(ENOMEM);
        }
        let mut inner = self.inner.lock();
        frame_of(&mut inner, index as usize)
    }

    /// Grow (or shrink) the object to `size`, with every page up to the new
    /// end backed now: memory is taken when an object is sized, not when
    /// it is touched (procnto, measured: `ftruncate()` of 64 MiB takes
    /// 64 MiB of free memory at once, and a size beyond what is free is
    /// ENOMEM there, the object left as it was). What is missing is
    /// counted first, and refused if it would eat the kernel's reserve
    /// (`pmem::can_spare`); then frames are allocated and zeroed a chunk at
    /// a time without the object's lock held, and put in place. Should
    /// memory still run out (another allocation in between), the frames
    /// put in place stay the object's, and the size is unchanged.
    pub fn grow(&self, size: u64) -> Result<(), u32> {
        const CHUNK: usize = 512;
        if size > MAX_SIZE {
            return Err(ENOMEM);
        }
        let pages = size.div_ceil(PAGE_SIZE) as usize;
        {
            let inner = self.inner.lock();
            if size <= inner.size {
                drop(inner);
                return self.set_size(size);
            }
            let missing = (0..pages).filter(|&i| inner.frames.get(i).is_none_or(|&f| f == 0)).count();
            if !pmem::can_spare(missing as u64) {
                return Err(ENOMEM);
            }
        }
        // The frame table at its full length, allocated with no lock held:
        // growing it under the lock (which masks interrupts) copies it
        // over and over (review PATHMEM-4).
        let mut table: Vec<u64> = Vec::with_capacity(pages);
        {
            let mut inner = self.inner.lock();
            if inner.frames.len() < pages {
                table.extend_from_slice(&inner.frames);
                table.resize(pages, 0);
                core::mem::swap(&mut inner.frames, &mut table);
            }
        }
        drop(table);
        let mut new: Vec<u64> = Vec::with_capacity(CHUNK);
        // The frames this call put in, given back if it fails part-way:
        // the object stays as it was (review PATHMEM-15).
        let mut added: Vec<usize> = Vec::new();
        for start in (0..pages).step_by(CHUNK) {
            let end = (start + CHUNK).min(pages);
            let missing: Vec<usize> = {
                let inner = self.inner.lock();
                (start..end).filter(|&i| inner.frames.get(i).is_none_or(|&f| f == 0)).collect()
            };
            for _ in &missing {
                let Some(pa) = pmem::alloc_user_page() else {
                    new.drain(..).for_each(pmem::free_page);
                    let back: Vec<u64> = {
                        let mut inner = self.inner.lock();
                        added.iter().map(|&i| core::mem::take(&mut inner.frames[i])).collect()
                    };
                    back.into_iter().filter(|&pa| pa != 0).for_each(pmem::free_page);
                    return Err(ENOMEM);
                };
                kmem::with_frame(pa, |f| f.fill(0));
                new.push(pa);
            }
            let mut inner = self.inner.lock();
            if inner.frames.len() < end {
                inner.frames.resize(end, 0);
            }
            for (&i, pa) in missing.iter().zip(new.drain(..)) {
                // Another call may have backed it meanwhile.
                if inner.frames[i] == 0 {
                    inner.frames[i] = pa;
                    added.push(i);
                } else {
                    pmem::free_page(pa);
                }
            }
        }
        let mut inner = self.inner.lock();
        inner.size = inner.size.max(size);
        Ok(())
    }

    /// Give the object `len` bytes of physical memory from `pa` on, at
    /// offset `at` (page-aligned), which it grows to cover: RAM it owns
    /// (freed with it) if `owned`, else memory it neither zeroes nor
    /// frees. The pages there must not be allocated yet.
    pub fn add_frames(&self, at: u64, pa: u64, len: u64, owned: bool) -> Result<(), u32> {
        let end = at.checked_add(len).filter(|&e| e <= MAX_SIZE).ok_or(ENOMEM)?;
        // Physical memory that wraps around the top of the address range
        // is no memory.
        pa.checked_add(len).ok_or(EINVAL)?;
        let mut inner = self.inner.lock();
        let (first, pages) = ((at / PAGE_SIZE) as usize, (len / PAGE_SIZE) as usize);
        if inner.frames.len() < first + pages {
            inner.frames.resize(first + pages, 0);
        }
        if inner.frames[first..first + pages].iter().any(|&f| f != 0) {
            return Err(crate::ker::errno::EBUSY);
        }
        let tag = if owned { 0 } else { FOREIGN };
        for i in 0..pages {
            inner.frames[first + i] = (pa + i as u64 * PAGE_SIZE) | tag;
        }
        inner.size = inner.size.max(end);
        Ok(())
    }

    /// Undo `add_frames()` from offset `at` for `len` bytes, and set the
    /// size back to `size`: a grow that failed part-way leaves the object
    /// as it was. The frames stay the caller's, to free.
    pub fn take_frames(&self, at: u64, len: u64, size: u64) {
        let mut inner = self.inner.lock();
        let first = (at / PAGE_SIZE) as usize;
        let end = (first + (len / PAGE_SIZE) as usize).min(inner.frames.len());
        inner.frames[first.min(end)..end].fill(0);
        inner.size = size;
    }

    /// The physical memory at offset `at`: its address and how far it runs
    /// on contiguously from there (to the end of the object), if `at` has
    /// memory.
    pub fn phys_run(&self, at: u64) -> Option<(u64, u64)> {
        let inner = self.inner.lock();
        if at >= inner.size {
            return None;
        }
        let first = (at / PAGE_SIZE) as usize;
        let pa = *inner.frames.get(first).filter(|&&f| f != 0)? & !FOREIGN;
        let last = inner.size.div_ceil(PAGE_SIZE) as usize;
        let pages = (first..last)
            .take_while(|&i| inner.frames.get(i).is_some_and(|&f| f & !FOREIGN == pa + (i - first) as u64 * PAGE_SIZE))
            .count() as u64;
        Some((pa + at % PAGE_SIZE, pages * PAGE_SIZE - at % PAGE_SIZE))
    }

    /// Put physical memory the object doesn't own (`len` bytes from `pa`)
    /// at offset `at` (page-aligned) where it has none yet (the pages it
    /// has stay), and make `at + len` the object's size.
    pub fn fill_frames(&self, at: u64, pa: u64, len: u64) -> Result<(), u32> {
        let end = at.checked_add(len).filter(|&e| e <= MAX_SIZE).ok_or(ENOMEM)?;
        let mut inner = self.inner.lock();
        let (first, pages) = ((at / PAGE_SIZE) as usize, (len / PAGE_SIZE) as usize);
        if inner.frames.len() < first + pages {
            inner.frames.resize(first + pages, 0);
        }
        for i in 0..pages {
            if inner.frames[first + i] == 0 {
                inner.frames[first + i] = (pa + i as u64 * PAGE_SIZE) | FOREIGN;
            }
        }
        inner.size = end;
        Ok(())
    }

    /// Copy bytes from offset `at` into `buf`, up to the end of the
    /// object; returns how many, or ESRVRFAULT if a device behind the
    /// object refused a read (`device_copy`).
    pub fn read(&self, at: u64, buf: &mut [u8]) -> Result<usize, u32> {
        let inner = self.inner.lock();
        let n = (inner.size.saturating_sub(at) as usize).min(buf.len());
        let mut done = 0;
        while done < n {
            let off = at + done as u64;
            let (page, in_page) = ((off / PAGE_SIZE) as usize, (off % PAGE_SIZE) as usize);
            let len = (PAGE_SIZE as usize - in_page).min(n - done);
            let out = &mut buf[done..done + len];
            match inner.frames.get(page) {
                Some(&pa) if is_device(pa) => {
                    device_copy(pa, in_page, out.as_mut_ptr(), len, false)?;
                }
                Some(&pa) if pa != 0 => {
                    kmem::with_frame(pa & !FOREIGN, |f| out.copy_from_slice(&f[in_page..in_page + len]))
                }
                _ => out.fill(0),
            }
            done += len;
        }
        Ok(n)
    }

    /// Copy `data` in at offset `at`, growing the object if need be. The
    /// lock (which masks interrupts) is taken for 64 KiB at a time, so a
    /// long write leaves its CPU open between pieces (review PATHMEM-4).
    pub fn write(&self, at: u64, data: &[u8]) -> Result<(), u32> {
        const PIECE: usize = 64 * 1024;
        let end = at.checked_add(data.len() as u64).filter(|&e| e <= MAX_SIZE);
        let end = end.ok_or(ENOMEM)?;
        let mut done = 0;
        while done < data.len() {
            let mut inner = self.inner.lock();
            let stop = (done + PIECE).min(data.len());
            while done < stop {
                let off = at + done as u64;
                let (page, in_page) = ((off / PAGE_SIZE) as usize, (off % PAGE_SIZE) as usize);
                let len = (PAGE_SIZE as usize - in_page).min(stop - done);
                let pa = frame_of(&mut inner, page)?;
                // `frame_of` gives the frame untagged; the table keeps it.
                if is_device(inner.frames[page]) {
                    device_copy(pa, in_page, data[done..].as_ptr().cast_mut(), len, true)?;
                } else {
                    kmem::with_frame(pa & !FOREIGN, |f| {
                        f[in_page..in_page + len].copy_from_slice(&data[done..done + len])
                    });
                }
                done += len;
            }
        }
        let mut inner = self.inner.lock();
        inner.size = inner.size.max(end);
        Ok(())
    }
}

/// Is frame `pa` (as `frames` holds it) a device's: foreign, and not in
/// RAM (`kmem::linear` doesn't reach it)?
fn is_device(pa: u64) -> bool {
    pa & FOREIGN != 0 && kmem::linear(pa & !FOREIGN).is_none()
}

/// Copy `len` bytes between `buf` and offset `in_page` of a device's
/// frame `pa` (into the frame if `to_frame`), through a device mapping:
/// never a cacheable alias of a device's registers (review CORE-23). An
/// access the device refuses ends the copy with ESRVRFAULT, as procnto
/// answers it (measured: a write through `SHMCTL_PHYS` where no device
/// answers, `tests/qnx/devshmtest.c`), instead of a kernel fault.
fn device_copy(pa: u64, in_page: usize, buf: *mut u8, len: usize, to_frame: bool) -> Result<(), u32> {
    let ok = kmem::with_device_frame(pa & !FOREIGN, |dev| {
        // SAFETY: the window maps the whole frame, and `in_page + len` stays
        // in it; `buf` has `len` bytes (the caller's slice), not in the
        // window.
        let at = unsafe { dev.add(in_page) };
        let (dst, src) = if to_frame { (at, buf.cast_const()) } else { (buf, at.cast_const()) };
        // SAFETY: as above.
        unsafe { crate::arch::exception::fault_copy(dst, src, len) }
    });
    if ok { Ok(()) } else { Err(crate::ker::errno::ESRVRFAULT) }
}

fn frame_of(inner: &mut Inner, index: usize) -> Result<u64, u32> {
    if inner.frames.len() <= index {
        inner.frames.resize(index + 1, 0);
    }
    if inner.frames[index] == 0 {
        let pa = pmem::alloc_user_page().ok_or(ENOMEM)?;
        kmem::with_frame(pa, |f| f.fill(0));
        inner.frames[index] = pa;
    }
    Ok(inner.frames[index] & !FOREIGN)
}

impl Drop for MemObject {
    fn drop(&mut self) {
        // Freed after the lock, which masks interrupts (review PATHMEM-4).
        let frames = core::mem::take(&mut self.inner.lock().frames);
        for pa in frames {
            if pa != 0 && pa & FOREIGN == 0 {
                pmem::free_page(pa);
            }
        }
    }
}
