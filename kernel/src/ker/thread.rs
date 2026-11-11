//! Thread creation (QNX `ker/ker_thread.c`): `ThreadCreate(pid, func, arg,
//! attr)`.
//!
//! As in procnto's `ker_thread_create` and `thread_bootstrap`, libc
//! supplies the stack (`struct _thread_attr`: flags at 0, stacksize at 8,
//! stackaddr at 0x10, sched param at 0x28, guardsize at 0x58; 0x68
//! bytes). The kernel puts the ELF TLS control block at `(top - 16) & ~15`
//! (dtv 0, then the TLS address; it is the thread pointer) and `struct
//! _thread_local_storage` 0x80 bytes below it, fills in the TLS (stack,
//! arg, errno pointer, flags, pid, tid, owner) and starts the thread at
//! libc's thread entry (`__entryfunc` in the process-local storage) with
//! sp at the TLS, x0 = arg and x1 = func. The new thread inherits the
//! creator's signal mask; it gets the next per-process thread ID, which
//! is the call's result.
//!
//! Also `ThreadDetach(tid)` (0: the caller; EINVAL if already detached,
//! ESRCH if none) and `ThreadCtl(cmd, data)`/`ThreadCtlExt(pid, tid, cmd,
//! data)` (pid and tid 0: the caller; a pid without a tid is EINVAL; in
//! another process only a thread's name and runmask can be read, the
//! runmask by the same user or with `PROCMGR_AID_XPROCESS_QUERY`, else
//! EPERM):
//! - `_NTO_TCTL_NAME`, `struct _thread_name` { new_name_len,
//!   name_buf_len, name_buf }: a negative `new_name_len` reads the name
//!   into `name_buf`; 0 deletes it, more sets it to that many bytes of
//!   `name_buf` (at most `name_buf_len`, else EINVAL; at most
//!   `_NTO_THREAD_NAME_MAX`, else E2BIG), and the old name is copied
//!   back. Copies are NUL-terminated, cut to `name_buf_len`.
//! - `_NTO_TCTL_IO`, `_NTO_TCTL_IO_PRIV`, `_NTO_TCTL_IO_LEVEL`: granted
//!   with `PROCMGR_AID_IO` (nothing to grant: device access is by mapping
//!   its registers), EPERM without.
//! - `_NTO_TCTL_TLS_SET_TP`: the calling thread's thread pointer becomes
//!   `data`.
//! - `_NTO_TCTL_RUNMASK` (`data` the mask), `_NTO_TCTL_RUNMASK_GET_AND_SET`
//!   and `_64` (`data` points to the new mask, 0 to leave it, and gets the
//!   old one) and `_NTO_TCTL_RUNMASK_GET_AND_SET_INHERIT` (`struct
//!   _thread_runmask` { size, runmask[size], inherit_mask[size] }; the
//!   inherit mask is accepted and not applied): the CPUs the thread may
//!   run on. A mask without an online CPU is EINVAL, and so is
//!   `_NTO_TCTL_RUNMASK` with 0. Another thread's mask may be read (a new
//!   mask of 0), never set: ENOTSUP, before the thread is looked up (as
//!   procnto). A thread that leaves its CPU's mask moves at once.
//!
//! `ThreadJoin(tid, status)` waits for thread `tid` of the caller's
//! process to end (STATE_JOIN) and stores its exit status (if `status` is
//! set). EDEADLK for the caller itself, EINVAL for a detached thread,
//! ESRCH for none or one joined already, EINTR or ETIMEDOUT if a signal or
//! a `TimerTimeout()` ends the wait.
//!
//! - `_NTO_TCTL_THREADS_HOLD`/`_CONT` (every other thread of the
//!   process) and `_NTO_TCTL_ONE_THREAD_HOLD`/`_CONT` (`data`: a thread
//!   ID, ESRCH if none; the caller or 0: nothing): a held thread stops
//!   (STATE_STOPPED) before it next returns to EL0. Holds don't count, and
//!   either continue command undoes either hold; SIGCONT doesn't.
//! - `_NTO_TCTL_ALIGN_FAULT` (`data` points to an int: > 0 misaligned
//!   accesses fault with SIGBUS, < 0 they are done, 0 leaves it; the old
//!   setting comes back as 1 or -1, initially -1). New threads inherit it.
//! - `_NTO_TCTL_ADD_EXIT_EVENT` (`data` a sigevent; one per thread, a new
//!   one replaces it) is delivered when the thread ends;
//!   `_NTO_TCTL_DEL_EXIT_EVENT` removes it.
//! - `_NTO_TCTL_HYP_VCPU`: nothing to do, 0 (as procnto).

use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU32, Ordering};

use super::KerResult;
use super::errno::{E2BIG, EBUSY, EDEADLK, EFAULT, EINVAL, ENOMEM, ENOSYS, ENOTSUP, EPERM, ESRCH};
use super::process::{self, Process, ProcessRef};
use super::umem::{self, Space};
use crate::arch::context::FpState;
use crate::arch::exception::TrapFrame;
use crate::arch::uspace::USER_END;
use crate::sched;
use crate::sync::IrqSpinLock;

const ATTR_SIZE: usize = 0x68;
/// Most words a `_NTO_TCTL_RUNMASK_GET_AND_SET_INHERIT` size may give.
const RUNMASK_WORDS_MAX: i32 = 2;
// _thread_attr flags (<sys/neutrino.h>)
/// `PTHREAD_CREATE_DETACHED`.
const DETACHED: u32 = 0x1;
const EXPLICIT_SCHED: u32 = 0x2;
const GUARD: u32 = 0x800;
/// Attribute flags copied into the TLS (`thread_bootstrap`).
const TLS_FLAGS: u32 = 0x830;
const TLS_SIZE: usize = 0x78;
/// Offset of `__entryfunc` in `struct _process_local_storage`.
const PLS_ENTRYFUNC: u64 = 0x18;

/// `ThreadCreate(pid, func, arg, attr)`: a new thread in the caller's
/// process; returns its thread ID.
pub fn thread_create(pid: i32, func: u64, arg: u64, attr: u64) -> KerResult {
    let me = sched::current();
    let p = &*me.process;
    if pid != 0 && pid != p.pid {
        return Err(if process::lookup(pid).is_some() {
            ENOSYS
        } else {
            ESRCH
        });
    }
    let space = Space::current();
    let mut a = [0u8; ATTR_SIZE];
    umem::read(space, attr, &mut a)?;
    let u32_at = |o: usize| u32::from_le_bytes(a[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(a[o..o + 8].try_into().unwrap());
    let flags = u32_at(0);
    let (stacksize, stackaddr) = (u64_at(8), u64_at(0x10));
    let guard = if flags & GUARD != 0 {
        u32_at(0x58) as u64
    } else {
        0
    };
    if stacksize <= guard {
        return Err(EINVAL);
    }
    // The creator's policy and parameters (a sporadic one's normal
    // priority), which a new thread takes unless the attribute has its own
    // (`PTHREAD_EXPLICIT_SCHED`: the policy at 0x20, `sched_param` at 0x28).
    let ts = |ns: u64| ((ns / 1_000_000_000) as i64, (ns % 1_000_000_000) as i64);
    let (parent_prio, parent_ss) = match crate::sporadic::get(me) {
        Some((ss, normal)) => (
            normal.unwrap_or(me.base_priority()),
            Some(crate::sporadic::Params {
                low_priority: ss.low_priority as i32,
                max_repl: ss.max_repl as i32,
                repl_period: ts(ss.repl_period),
                init_budget: ts(ss.init_budget),
            }),
        ),
        None => (me.base_priority(), None),
    };
    let (prio, policy, sporadic) = if flags & EXPLICIT_SCHED != 0 {
        // As procnto: policy 0 is the creator's; priority -1 the creator's
        // parameters, -2 its current priority (neither for a new sporadic
        // policy).
        let i64_at = |o: usize| i64::from_le_bytes(a[o..o + 8].try_into().unwrap());
        let policy = match u32_at(0x20) {
            0 => me.policy(),
            1 => sched::Policy::Fifo,
            2 => sched::Policy::RoundRobin,
            3 => sched::Policy::Other,
            4 => sched::Policy::Sporadic,
            _ => return Err(EINVAL),
        };
        let sporadic = policy == sched::Policy::Sporadic;
        match u32_at(0x28) as i32 {
            -1 if sporadic && parent_ss.is_none() => return Err(EINVAL),
            // The creator's sporadic parameters only for a sporadic
            // thread (review SCHED-14: a FIFO thread got them too).
            -1 => (parent_prio, policy, parent_ss.filter(|_| sporadic)),
            -2 if sporadic => return Err(EINVAL),
            -2 => (me.priority(), policy, None),
            p => {
                let prio = super::ability::user_priority(p as i64)?;
                let params = sporadic.then(|| crate::sporadic::Params {
                    low_priority: u32_at(0x30) as i32,
                    max_repl: u32_at(0x34) as i32,
                    repl_period: (i64_at(0x38), i64_at(0x40)),
                    init_budget: (i64_at(0x48), i64_at(0x50)),
                });
                if let Some(params) = &params {
                    crate::sporadic::validate(prio, params)?;
                }
                (prio, policy, params)
            }
        }
    } else {
        (parent_prio, me.policy(), parent_ss)
    };
    let detached = flags & DETACHED != 0;
    let ptid = start_qnx(
        p,
        space,
        Start {
            stackaddr,
            stacksize,
            guard,
            flags,
            prio,
            sigmask: me.sigmask.load(Ordering::Relaxed),
            func,
            arg,
        },
        // Before it can run (and end): a detached thread is freed as it
        // ends; it inherits the creator's alignment fault handling.
        |t| {
            t.set_policy(policy);
            if let Some(params) = &sporadic {
                crate::sporadic::set(t, prio, params);
            }
            t.detached.store(detached, Ordering::Relaxed);
            t.align_fault.store(me.align_fault.load(Ordering::Relaxed), Ordering::Relaxed);
        },
    )?;
    Ok(ptid as u64)
}

/// A new user thread, as `ThreadCreate()` lays it out.
struct Start {
    stackaddr: u64,
    stacksize: u64,
    guard: u64,
    flags: u32,
    prio: u8,
    sigmask: crate::ker::signal::SigSet,
    func: u64,
    arg: u64,
}

/// Start a thread of process `p` (whose memory is `space`) on the stack
/// `s` gives: its TLS and ELF TLS control block at the top, libc's thread
/// entry with x0 = arg and x1 = func. Returns its thread ID.
fn start_qnx(p: &Process, space: Space, s: Start, init: impl FnOnce(&sched::Thread)) -> Result<u32, u32> {
    let top = s
        .stackaddr
        .checked_add(s.stacksize)
        .filter(|&t| t <= USER_END)
        .ok_or(EFAULT)?;
    let elf_tls = (top - 16) & !15;
    let tls = elf_tls - 0x80;
    if tls < s.stackaddr + s.guard {
        return Err(EFAULT);
    }
    let mut pls_entry = [0u8; 8];
    umem::read(
        space,
        p.pls.load(Ordering::Relaxed) + PLS_ENTRYFUNC,
        &mut pls_entry,
    )?;
    let entry = u64::from_le_bytes(pls_entry);
    let ptid = p.new_ptid();
    // From here a failure gives the thread ID back (review SCHED-12: it
    // leaked, and the next thread got a higher one than procnto's).
    start_as(p, space, s, entry, tls, elf_tls, ptid, init).inspect_err(|_| p.release_ptid(ptid))
}

/// `start`, its thread ID taken: the TLS, the TCB and the thread.
#[allow(clippy::too_many_arguments)]
fn start_as(
    p: &Process,
    space: Space,
    s: Start,
    entry: u64,
    tls: u64,
    elf_tls: u64,
    ptid: u32,
    init: impl FnOnce(&sched::Thread),
) -> Result<u32, u32> {
    let mut t = [0u8; TLS_SIZE];
    let mut put = |o: usize, b: &[u8]| t[o..o + b.len()].copy_from_slice(b);
    put(0x00, &(s.stacksize - s.guard).to_le_bytes()); // __stacksize
    put(0x08, &s.arg.to_le_bytes()); // __arg
    put(0x10, &(tls + 0x18).to_le_bytes()); // __errptr = &__errval
    put(0x1c, &(s.flags & TLS_FLAGS).to_le_bytes()); // __flags
    put(0x20, &p.pid.to_le_bytes()); // __pid
    put(0x24, &ptid.to_le_bytes()); // __tid
    put(
        0x28,
        &(((p.pid as u32 & 0x7fff) << 16) | ptid).to_le_bytes(),
    ); // __owner
    put(0x30, &(s.stackaddr + s.guard).to_le_bytes()); // __stackaddr
    put(0x38, &(s.guard as u32).to_le_bytes()); // __guardsize
    let mut tcb = [0u8; 16];
    tcb[8..].copy_from_slice(&tls.to_le_bytes());
    // A stack the caller can't write (read-only, or not mapped) doesn't
    // fail the call: procnto creates the thread, which faults as it
    // first uses its stack, and the process takes the SIGSEGV (measured,
    // review SCHED-12). The thread then starts with no TLS written.
    let _ = umem::write(space, tls, &t).and_then(|()| umem::write(space, elf_tls, &tcb));
    let mut frame = TrapFrame::new_user(entry, tls, s.arg);
    frame.x[1] = s.func;
    sched::spawn_user_frame_init(
        p,
        "thread",
        s.prio,
        frame,
        elf_tls,
        FpState::new(),
        s.sigmask,
        ptid,
        init,
    )
    .ok_or(ENOMEM)?;
    Ok(ptid)
}

/// A thread of a FreeBSD process (`personality::native::thr_new`), with the
/// ID `ptid` its creator took: on the stack given, with no thread-local
/// block of the kernel's (its libc built the thread pointer's), started
/// at `entry(arg)`. The ID is given back on failure.
#[allow(clippy::too_many_arguments)]
pub fn start(
    p: &Process,
    stackaddr: u64,
    stacksize: u64,
    entry: u64,
    arg: u64,
    tp: u64,
    prio: u8,
    sigmask: crate::ker::signal::SigSet,
    ptid: u32,
    init: impl FnOnce(&sched::Thread),
) -> Result<(), u32> {
    let top = stackaddr
        .checked_add(stacksize)
        .filter(|&t| t <= USER_END && t >= stackaddr + 16)
        .ok_or(EFAULT)
        .inspect_err(|_| p.release_ptid(ptid))?;
    let frame = TrapFrame::new_user(entry, top & !15, arg);
    match sched::spawn_user_frame_init(p, "thread", prio, frame, tp, FpState::new(), sigmask, ptid, init) {
        Some(_) => Ok(()),
        None => {
            p.release_ptid(ptid);
            Err(ENOMEM)
        }
    }
}

/// An event delivery left to the `sigev_worker` kernel thread.
enum SigevRequest {
    /// `SIGEV_THREAD`: (process, function, value, attributes, priority).
    Thread(ProcessRef, u64, u64, u64, u8),
    /// `SIGEV_MEMORY` from an interrupt: (process, address, operation,
    /// value), see `event::memory`.
    Memory(ProcessRef, u64, u32, u32),
}

static SIGEV_REQUESTS: IrqSpinLock<VecDeque<SigevRequest>> = IrqSpinLock::new(VecDeque::new());
static SIGEV_WORKER: AtomicU32 = AtomicU32::new(0);
/// The stack of a `SIGEV_THREAD` thread whose attributes give none.
const SIGEV_STACK: u64 = 256 * 1024;

/// Deliver a `SIGEV_THREAD` event: a new thread of `p` will run
/// `function(value)`, made by the `sigev_worker` kernel thread (delivery
/// can't map memory or create threads: it may be an interrupt's). As in
/// procnto, the thread is detached; it takes its stack, priority and
/// scheduling from `attributes` (a `pthread_attr_t` in `p`, may be null),
/// else a stack the kernel maps (and unmaps as the thread ends) and the
/// priority of the thread the event is for. Dropped silently if that
/// fails, as QNX documents.
pub fn sigev_thread(p: ProcessRef, function: u64, value: u64, attributes: u64, prio: u8) {
    sigev_request(SigevRequest::Thread(p, function, value, attributes, prio));
}

/// Deliver a `SIGEV_MEMORY` event from an interrupt: the `sigev_worker`
/// does the operation (it may have to fault the page in).
pub fn sigev_memory(p: ProcessRef, addr: u64, op: u32, value: u32) {
    sigev_request(SigevRequest::Memory(p, addr, op, value));
}

fn sigev_request(r: SigevRequest) {
    SIGEV_REQUESTS.lock().push_back(r);
    if let Some(t) = sched::thread(SIGEV_WORKER.load(Ordering::Acquire)) {
        sched::wake(&t);
    }
}

/// The kernel thread that creates `SIGEV_THREAD` threads and does
/// interrupts' `SIGEV_MEMORY` operations.
pub fn sigev_worker(_: usize) {
    let me = sched::current();
    SIGEV_WORKER.store(me.tid, Ordering::Release);
    loop {
        let next = SIGEV_REQUESTS.lock().pop_front();
        match next {
            None => {
                // Idle, it shows as RECEIVE in pidin, as a server would.
                sched::prepare_block(sched::State::Receive);
                if !SIGEV_REQUESTS.lock().is_empty() {
                    sched::wake(me);
                }
                sched::block();
            }
            Some(SigevRequest::Thread(p, function, value, attributes, prio)) => {
                if !p.is_zombie() {
                    let _ = sigev_start(&p, function, value, attributes, prio);
                }
            }
            Some(SigevRequest::Memory(p, addr, op, value)) => {
                if !p.is_zombie() {
                    super::event::memory(&p, addr, op, value);
                }
            }
        }
    }
}

fn sigev_start(p: &Process, function: u64, value: u64, attributes: u64, prio: u8) -> Result<(), u32> {
    // SAFETY: `p` is held by the caller for the duration.
    let space = unsafe { Space::user(p) };
    let mut a = [0u8; ATTR_SIZE];
    if attributes != 0 {
        umem::read(space, attributes, &mut a)?;
    }
    let u32_at = |o: usize| u32::from_le_bytes(a[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(a[o..o + 8].try_into().unwrap());
    let flags = u32_at(0);
    let prio = if flags & EXPLICIT_SCHED != 0 {
        super::ability::priority_for(p, u32_at(0x28) as i32 as i64)?
    } else {
        prio
    };
    // The stack, and the region the kernel mapped for it, if it did.
    let (stackaddr, stacksize, owned) = match (u64_at(0x10), u64_at(8)) {
        // As procnto (measured): a `MAP_STACK` region of the attribute's
        // stack size, and a guard page right below it, no access, so that
        // an overflow faults.
        (0, size) => {
            let size = if size == 0 { SIGEV_STACK } else { size };
            let guard = crate::vm::pmem::PAGE_SIZE;
            let at = crate::vm::map::map(
                p,
                0,
                guard + size,
                crate::vm::map::PROT_READ | crate::vm::map::PROT_WRITE,
                crate::vm::map::MAP_PRIVATE | crate::vm::map::MAP_ANON | crate::vm::map::MAP_STACK,
                0,
                &[],
                None,
            )?;
            crate::vm::map::make_guard(p, at, guard);
            (at + guard, size, Some((at, guard + size)))
        }
        (addr, size) => (addr, size, None),
    };
    let started = start_qnx(
        p,
        space,
        Start {
            stackaddr,
            stacksize,
            guard: 0,
            flags: flags & !GUARD,
            prio,
            sigmask: 0,
            func: function,
            arg: value,
        },
        |t| {
            t.detached.store(true, Ordering::Relaxed);
            if owned.is_some() {
                *t.owned_stack.lock() = owned;
            }
        },
    );
    if started.is_err()
        && let Some((at, len)) = owned
    {
        let _ = crate::vm::map::unmap(p, at, len);
    }
    started.map(|_| ())
}

// _NTO_TCTL_* commands (<sys/neutrino.h>)
const TCTL_IO_PRIV: u32 = 1;
const TCTL_THREADS_HOLD: u32 = 2;
const TCTL_THREADS_CONT: u32 = 3;
const TCTL_ALIGN_FAULT: u32 = 5;
const TCTL_ONE_THREAD_HOLD: u32 = 8;
const TCTL_ONE_THREAD_CONT: u32 = 9;
const TCTL_ADD_EXIT_EVENT: u32 = 17;
const TCTL_DEL_EXIT_EVENT: u32 = 18;
const TCTL_HYP_VCPU: u32 = 25;
const TCTL_RUNMASK: u32 = 4;
const TCTL_RUNMASK_GET_AND_SET: u32 = 6;
const TCTL_RUNMASK_GET_AND_SET_64: u32 = 7;
const TCTL_RUNMASK_GET_AND_SET_INHERIT: u32 = 10;
const TCTL_NAME: u32 = 11;
const TCTL_SIGSTACK: u32 = 23;
const TCTL_TLS_SET_TP: u32 = 24;
const TCTL_IO: u32 = 14;
const TCTL_IO_LEVEL: u32 = 19;
/// `_NTO_THREAD_NAME_MAX`
const NAME_MAX: usize = 100;

/// The thread a `ThreadDetach()` or `ThreadCtlExt()` names in the
/// caller's process (`tid` 0: the caller).
fn own_thread(tid: u32) -> Result<sched::ThreadRef, u32> {
    let me = sched::current();
    if tid == 0 {
        return Ok(me.arc());
    }
    sched::thread_of(&me.process, tid).ok_or(ESRCH)
}

/// `ThreadCtl(_NTO_TCTL_SIGSTACK, stack)` (`sigaltstack()`): `stack` is a
/// `stack_t` { ss_sp, ss_size, ss_flags } in and out. In: `SS_DISABLE`
/// drops the alternate stack; a size sets it (at least `MINSIGSTKSZ`,
/// else ENOMEM; an end that overflows the address arithmetic is EFAULT);
/// neither leaves it. Out: the one before, `SS_ONSTACK` if
/// the thread runs on it (then it can't be changed: EPERM), `SS_DISABLE`
/// if there was none.
fn sigstack(t: &sched::Thread, data: u64) -> KerResult {
    const SS_ONSTACK: u32 = 1;
    const SS_DISABLE: u32 = 2;
    const MINSIGSTKSZ: u64 = 3 * 1024;
    let space = Space::current();
    let mut b = [0u8; 24];
    umem::read(space, data, &mut b)?;
    let (sp, size) = (
        u64::from_le_bytes(b[0..8].try_into().unwrap()),
        u64::from_le_bytes(b[8..16].try_into().unwrap()),
    );
    let flags = u32::from_le_bytes(b[16..20].try_into().unwrap());
    // The caller's stack pointer: SP_EL0 during its kernel call.
    let user_sp: u64;
    // SAFETY: reads a system register.
    unsafe { core::arch::asm!("mrs {}, sp_el0", out(reg) user_sp, options(nomem, nostack)) };
    let mut alt = t.altstack.lock();
    let old = *alt;
    let on = old.1 != 0 && old.0.checked_add(old.1).is_some_and(|end| (old.0..end).contains(&user_sp));
    let change = flags & SS_DISABLE != 0 || size != 0;
    if change && on {
        return Err(EPERM);
    }
    if flags & SS_DISABLE != 0 {
        *alt = (0, 0);
    } else if size != 0 {
        if size < MINSIGSTKSZ {
            return Err(ENOMEM);
        }
        // A stack whose end overflows the 64-bit address: EFAULT, as
        // procnto (measured with ss_sp near the top). One that is merely
        // unmapped, or above the user address space, is taken (not
        // measured for the latter), and the first signal on it kills the
        // thread, as there.
        if sp.checked_add(size).is_none() {
            return Err(EFAULT);
        }
        *alt = (sp, size);
    }
    drop(alt);
    let old_flags = if old.1 == 0 {
        SS_DISABLE
    } else if on {
        SS_ONSTACK
    } else {
        0
    };
    let mut out = [0u8; 24];
    out[0..8].copy_from_slice(&old.0.to_le_bytes());
    out[8..16].copy_from_slice(&old.1.to_le_bytes());
    out[16..20].copy_from_slice(&old_flags.to_le_bytes());
    umem::write(space, data, &out)?;
    Ok(0)
}

/// `ThreadDetach(tid)`.
pub fn thread_detach(tid: u32) -> KerResult {
    let me = sched::current();
    let t = match tid {
        0 => me.arc(),
        tid => sched::any_thread_of(&me.process, tid).ok_or(ESRCH)?,
    };
    // Under the lock `sched::exit` marks it dead with: one of the two
    // frees its ID.
    let g = sched::JOIN_LOCK.lock();
    // Joined already (a join that took it after this call looked it up):
    // gone, as a later detach finds it.
    if t.joined.load(Ordering::Acquire) {
        return Err(ESRCH);
    }
    if t.detached.swap(true, Ordering::AcqRel) {
        return Err(EINVAL);
    }
    // A thread waiting to join it gives up at once, with EINVAL (procnto,
    // measured): it will not take the thread or its ID.
    if let Some(j) = t.joiner.lock().take() {
        j.wake_error.store(EINVAL, Ordering::Relaxed);
        sched::wake(&j);
    }
    // Ended already: nobody will join it, it can go.
    if t.state() == sched::State::Dead {
        drop(g);
        t.process.release_ptid(t.ptid);
        sched::forget(&t);
    }
    Ok(0)
}

/// `ThreadCtl(cmd, data)` (`pid` and `tid` 0) and `ThreadCtlExt(pid, tid,
/// cmd, data)`.
pub fn thread_ctl(pid: i32, tid: u32, cmd: u32, data: u64) -> KerResult {
    let me = sched::current();
    if tid == 0 && pid != 0 {
        return Err(EINVAL);
    }
    let own = pid == 0 || pid == me.process.pid;
    // Runmasks, as procnto answers (measured): `_NTO_TCTL_RUNMASK` with 0
    // is EINVAL; another thread's runmask may be read, never set: ENOTSUP,
    // before the thread is looked up (so for a thread there isn't too).
    if own
        && matches!(
            cmd,
            TCTL_RUNMASK | TCTL_RUNMASK_GET_AND_SET | TCTL_RUNMASK_GET_AND_SET_64 | TCTL_RUNMASK_GET_AND_SET_INHERIT
        )
    {
        let space = Space::current();
        let mut b = [0u8; 8];
        let new = match cmd {
            TCTL_RUNMASK if data == 0 => return Err(EINVAL),
            TCTL_RUNMASK => data,
            TCTL_RUNMASK_GET_AND_SET_64 => {
                umem::read(space, data, &mut b)?;
                u64::from_le_bytes(b)
            }
            // GET_AND_SET: the mask; INHERIT: runmask[0], after the size.
            _ => {
                let at = if cmd == TCTL_RUNMASK_GET_AND_SET { data } else { data + 4 };
                umem::read(space, at, &mut b[..4])?;
                u32::from_le_bytes(b[..4].try_into().unwrap()) as u64
            }
        };
        if new != 0 && tid != 0 && tid != me.ptid {
            return Err(ENOTSUP);
        }
    }
    let t = if own {
        own_thread(tid)?
    } else {
        let p = process::lookup(pid).ok_or(ESRCH)?;
        sched::thread_of(&p, tid).ok_or(ESRCH)?
    };
    let space = Space::current();
    match cmd {
        TCTL_NAME => thread_name(&t, own, data),
        // Another process's runmask may be read (a mask of 0), by the same
        // user or with PROCMGR_AID_XPROCESS_QUERY (as procnto).
        TCTL_RUNMASK_GET_AND_SET | TCTL_RUNMASK_GET_AND_SET_64 if !own => {
            if !super::ability::may_query(process::current(), &t.process) {
                return Err(EPERM);
            }
            let n = if cmd == TCTL_RUNMASK_GET_AND_SET { 4 } else { 8 };
            let mut b = [0u8; 8];
            umem::read(space, data, &mut b[..n])?;
            if u64::from_le_bytes(b) != 0 {
                return Err(EPERM);
            }
            let old = set_runmask(&t, 0)?;
            umem::write(space, data, &old.to_le_bytes()[..n])?;
            Ok(0)
        }
        _ if !own => Err(EPERM),
        TCTL_SIGSTACK => sigstack(&t, data),
        // Nothing to grant on AArch64 (device access is by mapping its
        // registers), but only with the IO ability, as procnto.
        TCTL_IO | TCTL_IO_PRIV | TCTL_IO_LEVEL => super::ability::check(super::ability::IO, None).map(|()| 0),
        // The caller's own thread pointer (`data`), for a TLS of its own.
        TCTL_TLS_SET_TP if core::ptr::eq(&*t, me) => {
            crate::arch::context::set_user_tp(data);
            Ok(0)
        }
        TCTL_TLS_SET_TP => Err(EINVAL),
        TCTL_RUNMASK => set_runmask(&t, data).map(|_| 0),
        TCTL_RUNMASK_GET_AND_SET => {
            let mut b = [0u8; 4];
            umem::read(space, data, &mut b)?;
            let old = set_runmask(&t, u32::from_le_bytes(b) as u64)?;
            umem::write(space, data, &(old as u32).to_le_bytes())?;
            Ok(0)
        }
        TCTL_RUNMASK_GET_AND_SET_64 => {
            let mut b = [0u8; 8];
            umem::read(space, data, &mut b)?;
            let old = set_runmask(&t, u64::from_le_bytes(b))?;
            umem::write(space, data, &old.to_le_bytes())?;
            Ok(0)
        }
        TCTL_RUNMASK_GET_AND_SET_INHERIT => {
            let mut b = [0u8; 4];
            umem::read(space, data, &mut b)?;
            // procnto takes 1 or 2 words (measured with 4 CPUs: 2 is
            // `RMSK_SIZE` of its 64-CPU maximum), EINVAL otherwise: the
            // size is the user's and sizes the reply.
            let words = i32::from_le_bytes(b);
            if !(1..=RUNMASK_WORDS_MAX).contains(&words) {
                return Err(EINVAL);
            }
            // runmask[] then inherit_mask[]: the first word of runmask[]
            // (CPUs 0-31) is applied; the old masks are returned, the
            // inherit mask as every CPU.
            umem::read(space, data + 4, &mut b)?;
            let old = set_runmask(&t, u32::from_le_bytes(b) as u64)?;
            let mut masks = alloc::vec![0u8; 8 * words as usize];
            masks[..4].copy_from_slice(&(old as u32).to_le_bytes());
            let all = (crate::smp::online_mask() as u32).to_le_bytes();
            masks[4 * words as usize..4 * words as usize + 4].copy_from_slice(&all);
            umem::write(space, data + 4, &masks)?;
            Ok(0)
        }
        // The process's other threads, or one (data: its ID; the caller
        // itself or 0: nothing).
        TCTL_THREADS_HOLD | TCTL_THREADS_CONT => {
            for o in sched::threads_of(&t.process).iter().filter(|o| o.ptid != t.ptid) {
                hold(o, cmd == TCTL_THREADS_HOLD);
            }
            Ok(0)
        }
        TCTL_ONE_THREAD_HOLD | TCTL_ONE_THREAD_CONT => {
            let tid = data as u32;
            if tid != 0 && tid != t.ptid {
                let o = sched::thread_of(&t.process, tid).ok_or(ESRCH)?;
                hold(&o, cmd == TCTL_ONE_THREAD_HOLD);
            }
            Ok(0)
        }
        // `*data` > 0: misaligned accesses fault; < 0: they don't (the
        // hardware does them); 0: unchanged. The old setting (1 or -1)
        // is written back.
        TCTL_ALIGN_FAULT => {
            let mut b = [0u8; 4];
            umem::read(space, data, &mut b)?;
            let v = i32::from_le_bytes(b);
            let old = if v == 0 {
                t.align_fault.load(Ordering::Relaxed)
            } else {
                t.align_fault.swap(v > 0, Ordering::Relaxed)
            };
            umem::write(space, data, &(if old { 1i32 } else { -1 }).to_le_bytes())?;
            Ok(0)
        }
        TCTL_ADD_EXIT_EVENT => {
            let ev = super::event::read(space, data)?;
            *t.exit_event.lock() = Some((me.process.pid, ev));
            Ok(0)
        }
        TCTL_DEL_EXIT_EVENT => {
            *t.exit_event.lock() = None;
            Ok(0)
        }
        // No hypervisor: nothing to do (procnto answers 0 too).
        TCTL_HYP_VCPU => Ok(0),
        // No such command (procnto: EINVAL).
        _ => Err(EINVAL),
    }
}

/// Hold thread `t` or continue it.
fn hold(t: &sched::Thread, on: bool) {
    if on {
        super::debug::thread_hold(t);
    } else {
        super::debug::thread_continue(t);
    }
}

/// Thread `t` ends: the event added for that (`_NTO_TCTL_ADD_EXIT_EVENT`)
/// goes to the process that added it.
pub fn exit_event(t: &sched::Thread) {
    let Some((pid, ev)) = t.exit_event.lock().take() else {
        return;
    };
    if let Some(p) = process::lookup(pid) {
        let _ = super::event::deliver(&p, None, &ev, 0);
    }
}

/// Set thread `t`'s runmask to `mask` (0: leave it); returns the old one
/// (as the online CPUs it has). The caller moves at once if it may no
/// longer run where it is; no other thread's mask is set (`thread_ctl`).
pub fn set_runmask(t: &sched::Thread, mask: u64) -> Result<u64, u32> {
    let online = crate::smp::online_mask();
    let old = t.runmask.load(Ordering::Relaxed) & online;
    if mask != 0 {
        if mask & online == 0 {
            return Err(EINVAL);
        }
        t.runmask.store(mask, Ordering::Relaxed);
        if core::ptr::eq(t, sched::current()) && !t.may_run_on(crate::percpu::this().cpu) {
            sched::yield_now();
        }
    }
    Ok(old)
}

/// `_NTO_TCTL_NAME` on thread `t` (`own`: of the caller's process).
fn thread_name(t: &sched::Thread, own: bool, data: u64) -> KerResult {
    let space = Space::current();
    let mut hdr = [0u8; 8];
    umem::read(space, data, &mut hdr)?;
    let new_len = i32::from_le_bytes(hdr[0..4].try_into().unwrap());
    let buf_len = i32::from_le_bytes(hdr[4..8].try_into().unwrap());
    if buf_len < 0 || new_len > buf_len {
        return Err(EINVAL);
    }
    let old = t.qnx_name.lock().clone();
    if new_len >= 0 {
        if !own {
            return Err(EPERM);
        }
        if new_len as usize > NAME_MAX {
            return Err(E2BIG);
        }
        let mut name = alloc::vec![0u8; new_len as usize];
        umem::read(space, data + 8, &mut name)?;
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        *t.qnx_name.lock() = alloc::string::String::from_utf8_lossy(&name[..end]).into();
        crate::ker::trace::thread_name(t, &t.qnx_name.lock().clone());
    }
    let mut out = old.into_bytes();
    out.push(0);
    out.truncate(buf_len as usize);
    umem::write(space, data + 8, &out)?;
    Ok(0)
}

/// `ThreadJoin(tid, status)`.
pub fn thread_join(tid: u32, status: u64) -> KerResult {
    let me = sched::current();
    if tid == me.ptid {
        return Err(EDEADLK);
    }
    let t = sched::any_thread_of(&me.process, tid).ok_or(ESRCH)?;
    loop {
        // Detached or taken by another join: decided under the lock that
        // `ThreadDetach()` and `sched::exit` take, so that exactly one of
        // them frees the thread's ID.
        let g = sched::JOIN_LOCK.lock();
        if t.detached.load(Ordering::Acquire) {
            return Err(EINVAL);
        }
        if t.state() == sched::State::Dead {
            if t.joined.swap(true, Ordering::AcqRel) {
                return Err(ESRCH);
            }
            drop(g);
            me.process.release_ptid(t.ptid);
            sched::forget(&t);
            if status != 0 {
                let v = t.exit_status.load(Ordering::Relaxed);
                umem::write(Space::current(), status, &v.to_le_bytes())?;
            }
            return Ok(0);
        }
        {
            let mut j = t.joiner.lock();
            if j.as_ref().is_some_and(|j| !core::ptr::eq(&**j, me)) {
                return Err(EBUSY); // another thread joins it already
            }
            *j = Some(me.arc());
        }
        me.wake_error.store(0, Ordering::Relaxed);
        sched::prepare_block(sched::State::Join);
        let deadline = crate::timeout::arm(sched::State::Join);
        drop(g);
        sched::block();
        crate::timeout::disarm(deadline);
        let err = me.wake_error.swap(0, Ordering::Relaxed);
        if err != 0 {
            let mut j = t.joiner.lock();
            if j.as_ref().is_some_and(|j| core::ptr::eq(&**j, me)) {
                *j = None;
            }
            return Err(err);
        }
    }
}

/// `struct _thread_local_storage.__flags`: its offset, and the bits of
/// `<pthread.h>`.
const TLS_FLAGS_OFF: u64 = 0x1c;
const CANCEL_DISABLE: u32 = 0x10;
const CANCEL_PENDING: u32 = 0x40;

/// The address of the TLS `__flags` of thread `t`, of the caller's process
/// (its thread pointer's second word is the TLS address).
fn tls_flags_addr(t: &sched::Thread) -> Result<u64, u32> {
    let tp = t.tls.load(Ordering::Relaxed);
    let mut b = [0u8; 8];
    umem::read(Space::current(), tp + 8, &mut b)?;
    Ok(u64::from_le_bytes(b) + TLS_FLAGS_OFF)
}

/// `ThreadCancel(tid, canstub)`: cancel thread `tid` of the caller's
/// process. Its TLS gets `PTHREAD_CANCEL_PENDING` (libc's
/// `pthread_testcancel()` reads it); unless it disabled cancellation, a
/// thread blocked at a cancellation point is woken, and at its next
/// cancellation point (`cancellation_point`) it goes to `canstub`, which
/// ends it. As QNX 8 does, the asynchronous type is no different: a
/// thread that never reaches a cancellation point goes on.
pub fn thread_cancel(tid: u32, canstub: u64) -> KerResult {
    let me = sched::current();
    let t = sched::thread_of(&me.process, tid)
        .filter(|t| t.state() != sched::State::Dead)
        .ok_or(ESRCH)?;
    let addr = tls_flags_addr(&t)?;
    t.cancel_stub.store(canstub, Ordering::Release);
    let flags = umem::with_atomic_u32(Space::current(), addr, |a| {
        a.fetch_or(CANCEL_PENDING, Ordering::AcqRel)
    })?;
    if flags & CANCEL_DISABLE == 0 && !core::ptr::eq(&*t, me) {
        use sched::State;
        if matches!(
            t.state(),
            State::Send
                | State::Receive
                | State::Reply
                | State::Nanosleep
                | State::Sigsuspend
                | State::Sigwaitinfo
                | State::Condvar
                | State::Sem
                | State::Join
                | State::Intr
                | State::Barrier
                | State::MqSend
                | State::MqReceive
        ) {
            super::unblock(&t, super::errno::EINTR);
        }
    }
    Ok(0)
}

/// The calling thread is at a cancellation point (a blocking kernel call
/// other than `MsgSendvnc()` and `SyncMutexLock()`): if it was cancelled
/// and has cancellation enabled, it goes to its cancel stub instead of
/// returning (the stub, libc's, calls `pthread_exit()`). Returns whether
/// it does.
pub fn cancellation_point(frame: &mut TrapFrame) -> bool {
    let me = sched::current();
    let stub = me.cancel_stub.load(Ordering::Acquire);
    if stub == 0 {
        return false;
    }
    let enabled = tls_flags_addr(me)
        .and_then(|a| umem::with_atomic_u32(Space::current(), a, |f| f.load(Ordering::Acquire)))
        .is_ok_and(|f| f & CANCEL_DISABLE == 0);
    if !enabled {
        return false;
    }
    // Once: the cleanup handlers may make blocking calls themselves.
    me.cancel_stub.store(0, Ordering::Release);
    frame.elr = stub;
    true
}
