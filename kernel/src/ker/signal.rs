//! Signals (QNX `ker/nto_signal.c`): per-process actions, per-thread masks.
//!
//! The kernel's signals are FreeBSD's: its numbers, 128-bit sets, 62
//! real-time signals (65 to 126), `struct sigaction`, `siginfo_t` and
//! `ucontext_t`. A QNX process is the one translated, at the boundary
//! (`personality::qnx`): its numbers, sets and `sa_flags` on the way in
//! and out, its order of delivery and its queuing rule. QNX's own signals
//! (`SIGSELECT` and the seven above it, `SIGPWR`) have numbers FreeBSD
//! doesn't use, 34 to 64, and take nothing from it.
//!
//! `SignalAction(pid, sigstub, signo, act, oact)` and
//! `SignalProcmask(pid, tid, how, set, oldset)`. `sigstub` is the
//! trampoline that calls the handler.
//!
//! Signals are posted to a process (`post`, `SignalKill`) and delivered on
//! the way back to user mode after a kernel call (`deliver`): ignored ones
//! are dropped, a default action ends the process (or is nothing, for the
//! signals ignored by default), and a handler is entered through the
//! trampoline. For that the kernel puts the context on the user stack and
//! a frame below it, and calls the trampoline with x0 at the frame, which
//! ends with `SignalReturn(frame)`: `enter_handler()` for FreeBSD's
//! layouts, `enter_handler_qnx()` for QNX's (`ucontext_t` with the registers at
//! +0x30, FP/SIMD at +0x140, `uc_sigmask` at +8; `struct _sighandler_info`
//! { siginfo (48 bytes), handler, context } and the mask to restore).
//!
//! A handler with `SA_ONSTACK` runs on the thread's alternate stack
//! (`sigaltstack()`, `ThreadCtl(_NTO_TCTL_SIGSTACK)`), unless the thread
//! is on it already.
//!
//! Not yet: other processes' actions; `si_code`s are still QNX's inside.

use alloc::collections::VecDeque;
use core::sync::atomic::Ordering;

use super::KerResult;
use super::errno::{EINTR, EINVAL, ENOSYS, EPERM, ESRCH};
use super::process;
use super::umem::{self, Space};
use crate::arch::context::{FpState, QNX_FPU_SIZE, fp_restore, fp_save};
use crate::arch::exception::TrapFrame;
use crate::personality::{Personality, qnx};
use crate::sched;

/// Signals are 1..NSIG-1: up to `_SIGMAX` (64). Applications have 1 to
/// `SIGRTMAX` (56, `_NSIG` - 1); QNX's libraries use the ones above
/// (`SIGSELECT`, 57, for `poll()` and `select()`).
pub const NSIG: usize = 128;
pub const SIGKILL: usize = 9;
pub const SIGCHLD: usize = 20;
const SIGSTOP: usize = 17;
const SIGTSTP: usize = 18;
pub const SIGCONT: usize = 19;
const SIGTTIN: usize = 21;
const SIGTTOU: usize = 22;
const SIGURG: usize = 16;
const SIGIO: usize = 23;
const SIGWINCH: usize = 28;
const SIGINFO: usize = 29;
const SIGTHR: usize = 32;
const SIGLIBRT: usize = 33;
const SIGSEGV: usize = 11;
/// QNX's `SIGPWR`, in the numbers FreeBSD doesn't use.
const QNX_SIGPWR: usize = 47;
/// FreeBSD's first real-time signal.
const SIGRTMIN: usize = 65;
/// The first of the top 8 signals, which are always masked and queued.
const SIGSELECT: usize = 57;
/// Signals whose default action is to do nothing (SIGCHLD, SIGPWR,
/// SIGWINCH, SIGURG, SIGCONT), and the libraries' signals above
/// `SIGRTMAX`. SIGSTOP stops the process (`debug::job_stop`), and so do
/// `DEFAULT_STOP`'s by default.
const DEFAULT_IGNORED: SigSet = bit(SIGCHLD) | bit(SIGWINCH) | bit(SIGURG) | bit(SIGCONT) | bit(SIGSTOP) | SPECIAL;
/// SIGTSTP, SIGTTIN, SIGTTOU: their default action stops the process, as
/// SIGSTOP does, with the signal as the stop signal (`WSTOPSIG`); unlike
/// SIGSTOP they can be caught, ignored and blocked (procnto, measured).
const DEFAULT_STOP: SigSet = bit(SIGTSTP) | bit(SIGTTIN) | bit(SIGTTOU);

/// The signals process `p` ignores by default.
fn default_ignored(p: &process::Process) -> SigSet {
    match p.personality() {
        Personality::Qnx => DEFAULT_IGNORED | bit(QNX_SIGPWR),
        Personality::Native => DEFAULT_IGNORED | bit(SIGIO) | bit(SIGINFO) | bit(SIGTHR) | bit(SIGLIBRT),
    }
}

/// A set in the calling process's memory, in its form.
fn read_set(space: Space, at: u64) -> Result<SigSet, u32> {
    if current_is_qnx() {
        let mut b = [0u8; 8];
        umem::read(space, at, &mut b)?;
        Ok(qnx::set_in(u64::from_le_bytes(b)))
    } else {
        let mut b = [0u8; 16];
        umem::read(space, at, &mut b)?;
        Ok(SigSet::from_le_bytes(b))
    }
}

/// Write a set to the calling process's memory, in its form.
fn write_set(space: Space, at: u64, set: SigSet) -> Result<(), u32> {
    if current_is_qnx() {
        umem::write(space, at, &qnx::set_out(set).to_le_bytes())
    } else {
        umem::write(space, at, &set.to_le_bytes())
    }
}

fn current_is_qnx() -> bool {
    process::current().personality() == Personality::Qnx
}
const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;
// sa_flags
const SA_SIGINFO: u32 = 0x40;
const SA_RESETHAND: u32 = 0x4;
const SA_ONSTACK: u32 = 0x1;
const SA_NODEFER: u32 = 0x10;
/// FreeBSD's `struct sigaction`: the handler, `sa_flags`, `sa_mask`.
const BSD_SIGACTION_SIZE: usize = 32;
/// FreeBSD's `siginfo_t`, and `ucontext_t` for AArch64: the mask, x0 to
/// x29, lr, sp, elr, spsr, then q0 to q31, fpsr, fpcr.
const BSD_SIGINFO_SIZE: usize = 80;
const BSD_UC_SIZE: usize = 944;
const BSD_UC_GPR: usize = 16;
const BSD_UC_FPU: usize = 288;
/// The frame for `SignalReturn()`: `siginfo_t`, the handler, the context.
const BSD_FRAME_SIZE: usize = 96;
/// `si_code` of a signal sent with kill() (`SI_USER`).
const SI_USER: i32 = 0x10001;
/// `ucontext_t`: uc_sigmask at 8, the registers at 0x30, FP at 0x140;
/// 848 bytes.
const UC_SIZE: usize = 848;
const UC_SIGMASK: usize = 8;
const UC_GPR: usize = 0x30;
const UC_FPU: usize = 0x140;
/// Our frame for SignalReturn: `struct _sighandler_info` (64 bytes) and
/// the mask to restore.
const FRAME_SIZE: usize = 80;

/// A set of signals: bit n-1 for signal n, 128 bits as FreeBSD's
/// `sigset_t`.
pub type SigSet = u128;

/// A signal set one thread changes while others read it. There is no
/// stable 128-bit atomic, so a lock; the callers' orderings are as they
/// were for the 64-bit atomic this replaces, and the lock gives more.
pub struct AtomicSigSet(crate::sync::IrqSpinLock<SigSet>);

impl AtomicSigSet {
    pub const fn new(set: SigSet) -> Self {
        Self(crate::sync::IrqSpinLock::new(set))
    }

    pub fn load(&self, _: Ordering) -> SigSet {
        *self.0.lock()
    }

    pub fn store(&self, set: SigSet, _: Ordering) {
        *self.0.lock() = set;
    }

    pub fn swap(&self, set: SigSet, _: Ordering) -> SigSet {
        core::mem::replace(&mut *self.0.lock(), set)
    }
}

/// The 64 bits QNX's `sigset_t` has.
pub const QNX_SET: SigSet = u64::MAX as SigSet;

const fn bit(signo: usize) -> SigSet {
    1 << (signo - 1)
}
/// Signals that cannot be caught, ignored or blocked.
const UNBLOCKABLE: SigSet = (1 << (SIGKILL - 1)) | (1 << (SIGSTOP - 1));
/// QNX's special signals, 57 to 64 (`SIGSELECT` & co.): always blocked
/// (unblocking them is quietly ignored), never caught or ignored, so they
/// stay pending until a `sigwaitinfo()` takes them. A server's
/// notification that comes before its client waits isn't lost.
pub const SPECIAL: SigSet = QNX_SET & !((1 << 56) - 1);

/// A thread signal mask as it can be: SIGKILL and SIGSTOP unblocked, the
/// special signals blocked.
pub fn valid_mask(mask: SigSet) -> SigSet {
    (mask & !UNBLOCKABLE) | SPECIAL
}
const SIGACTION_SIZE: usize = 24;
const SIG_BLOCK: u32 = 0;
const SIG_UNBLOCK: u32 = 1;
const SIG_SETMASK: u32 = 2;
/// `SIG_PENDING`: `SignalProcmask()` reads the pending blocked signals.
const SIG_PENDING: u32 = 5;

#[derive(Debug, Clone, Copy, Default)]
pub struct Action {
    pub handler: u64,
    pub flags: u32,
    pub mask: SigSet,
}

/// What `siginfo_t` (`<sys/siginfo.h>`, 48 bytes) says about a signal.
#[derive(Debug, Clone, Copy, Default)]
pub struct SigInfo {
    pub signo: i32,
    pub code: i32,
    pub pid: i32,
    pub uid: i32,
    /// `si_status` (SIGCHLD) or the low half of `si_value`.
    pub status: i32,
    pub value: u64,
    /// A fault's signal (SIGSEGV, SIGILL, SIGBUS, SIGTRAP...): `si_fltno`,
    /// `si_fltip`, `si_addr`.
    pub fault: Option<(i32, u64, u64)>,
    /// The timer that sent it (`ker::timer`'s key; 0: none).
    pub timer: u64,
}

pub const SIGINFO_SIZE: usize = 48;

impl SigInfo {
    /// signo at 0, code at 4, errno at 8, pid at 16, uid at 24, then
    /// `si_status` (28) for SIGCHLD or `si_value` (32) otherwise; a fault's
    /// fltno (16), fltip (24) and addr (32).
    /// FreeBSD's `siginfo_t`. A code only QNX has (a server's
    /// notification) is `SI_LWP`'s neighbour, `SI_UNDEFINED`-free: 0x10006.
    pub fn to_bytes(&self) -> [u8; BSD_SIGINFO_SIZE] {
        let code = if self.code < 0 { 0x10006 } else { self.code };
        let mut b = [0u8; BSD_SIGINFO_SIZE];
        b[0..4].copy_from_slice(&self.signo.to_le_bytes());
        b[8..12].copy_from_slice(&(code as i32).to_le_bytes());
        if let Some((fltno, _, addr)) = self.fault {
            b[24..32].copy_from_slice(&addr.to_le_bytes());
            b[40..44].copy_from_slice(&fltno.to_le_bytes());
        } else {
            b[12..16].copy_from_slice(&self.pid.to_le_bytes());
            b[16..20].copy_from_slice(&self.uid.to_le_bytes());
            if self.signo == SIGCHLD as i32 {
                b[20..24].copy_from_slice(&self.status.to_le_bytes());
            } else {
                b[32..40].copy_from_slice(&self.value.to_le_bytes());
            }
        }
        b
    }

    /// QNX's `siginfo_t`, whose numbers the caller has made QNX's
    /// (`qnx::view`).
    pub fn to_qnx_bytes(&self) -> [u8; SIGINFO_SIZE] {
        let mut b = [0u8; SIGINFO_SIZE];
        b[0..4].copy_from_slice(&self.signo.to_le_bytes());
        b[4..8].copy_from_slice(&self.code.to_le_bytes());
        b[16..20].copy_from_slice(&self.pid.to_le_bytes());
        b[24..28].copy_from_slice(&self.uid.to_le_bytes());
        if let Some((fltno, ip, addr)) = self.fault {
            // `__data.__fault`: fltno at 16, fltip at 24, addr at 32.
            b[16..20].copy_from_slice(&fltno.to_le_bytes());
            b[24..32].copy_from_slice(&ip.to_le_bytes());
            b[32..40].copy_from_slice(&addr.to_le_bytes());
        } else if self.signo == 18 {
            b[28..32].copy_from_slice(&self.status.to_le_bytes());
        } else {
            b[32..40].copy_from_slice(&self.value.to_le_bytes());
        }
        b
    }
}

#[derive(Clone)]
pub struct Signals {
    /// libc's signal trampoline (`__signalstub`), from SignalAction.
    pub stub: u64,
    /// Indexed by signal number.
    pub actions: alloc::vec::Vec<Action>,
    /// Signals sent to the process and not yet delivered.
    pub pending: SigSet,
    /// The information of each pending signal (by number; kept apart from
    /// the tables, so a process stays small): the latest, or with
    /// `SA_SIGINFO` each one sent, in order (procnto queues those).
    pub info: alloc::collections::BTreeMap<u8, VecDeque<SigInfo>>,
    /// Signals sent to one thread (`pthread_kill()`, `SIGEV_SIGNAL_THREAD`)
    /// and not yet delivered, with their information, by thread ID.
    pub threads: alloc::collections::BTreeMap<u32, ThreadSignals>,
}

/// A thread's own pending signals.
#[derive(Clone, Default)]
pub struct ThreadSignals {
    pub pending: SigSet,
    pub info: alloc::collections::BTreeMap<u8, VecDeque<SigInfo>>,
}

impl Signals {
    pub const fn new() -> Self {
        Self {
            stub: 0,
            actions: alloc::vec::Vec::new(),
            pending: 0,
            info: alloc::collections::BTreeMap::new(),
            threads: alloc::collections::BTreeMap::new(),
        }
    }

    /// The signals pending for thread `tid`: the process's and its own.
    pub fn pending_for(&self, tid: u32) -> SigSet {
        self.pending | self.threads.get(&tid).map_or(0, |t| t.pending)
    }

    /// Take pending signal `signo` for thread `tid` (its own first, as
    /// procnto, whatever order they came in: measured in sigqtest) and its
    /// information; a queued one stays pending while more are queued.
    pub fn take(&mut self, tid: u32, signo: usize) -> SigInfo {
        fn pop(pending: &mut SigSet, info: &mut alloc::collections::BTreeMap<u8, VecDeque<SigInfo>>, signo: usize) -> SigInfo {
            let q = info.entry(signo as u8).or_default();
            let i = q.pop_front().unwrap_or_default();
            if q.is_empty() {
                info.remove(&(signo as u8));
                *pending &= !bit(signo);
            }
            i
        }
        if let Some(t) = self.threads.get_mut(&tid)
            && t.pending & bit(signo) != 0
        {
            let info = pop(&mut t.pending, &mut t.info, signo);
            if t.pending == 0 {
                self.threads.remove(&tid);
            }
            return info;
        }
        pop(&mut self.pending, &mut self.info, signo)
    }

    /// Is a signal of timer `key` (`SigInfo::timer`) still pending?
    pub fn from_timer(&self, key: u64) -> bool {
        let pending = |pending: SigSet, info: &alloc::collections::BTreeMap<u8, VecDeque<SigInfo>>| {
            info.iter().any(|(&n, q)| pending & bit(n as usize) != 0 && q.iter().any(|i| i.timer == key))
        };
        pending(self.pending, &self.info) || self.threads.values().any(|t| pending(t.pending, &t.info))
    }

    /// Forget the pending instances of process signal `signo` sent by
    /// process `pid` (procnto's `nto_signal_remove`).
    pub fn remove_from(&mut self, signo: usize, pid: i32) {
        if let Some(q) = self.info.get_mut(&(signo as u8)) {
            q.retain(|i| i.pid != pid);
            if q.is_empty() {
                self.discard(signo);
            }
        }
    }

    /// Forget pending signal `signo` of the process, all instances.
    pub fn discard(&mut self, signo: usize) {
        self.pending &= !bit(signo);
        self.info.remove(&(signo as u8));
    }
}

fn own_process(pid: i32) -> Result<crate::ker::process::ProcessRef, u32> {
    let me = process::current();
    if pid == 0 || pid == me.pid {
        Ok(me.arc())
    } else if process::lookup(pid).is_some() {
        Err(ENOSYS) // other processes' signals: later
    } else {
        Err(ESRCH)
    }
}

/// `SignalAction(pid, sigstub, signo, act, oact)`.
pub fn signal_action(pid: i32, stub: u64, signo: i32, act: u64, oact: u64) -> KerResult {
    // As procnto's `ker_signal_action`: signal 57 with nothing else
    // flushes queued `SIGEV` pulses (none are queued here); other than
    // that, signals 1 to 56 only (EINVAL). Flags 0x40 and 0x80 are
    // ENOTSUP, and SIGKILL and SIGSTOP take no handler (EINVAL; SIG_DFL
    // is fine).
    const FLUSH_SIGEV: i32 = 57;
    const FLAGS_UNSUPPORTED: u32 = 0xc0;
    let is_qnx = current_is_qnx();
    if is_qnx && signo == FLUSH_SIGEV && pid == 0 && stub == 0 && act == 0 && oact == 0 {
        return Ok(0);
    }
    let signo = usize::try_from(signo).map_err(|_| EINVAL)?;
    let signo = if is_qnx {
        if !(1..=56).contains(&signo) {
            return Err(EINVAL);
        }
        qnx::signo_in(signo)
    } else {
        if !(1..=33).contains(&signo) && !(SIGRTMIN..NSIG - 1).contains(&signo) {
            return Err(EINVAL);
        }
        signo
    };
    let p = own_process(pid)?;
    let space = Space::current();
    let new = if act != 0 {
        let a = if is_qnx {
            let mut b = [0u8; SIGACTION_SIZE];
            umem::read(space, act, &mut b)?;
            let flags = u32::from_le_bytes(b[8..12].try_into().unwrap());
            if flags & FLAGS_UNSUPPORTED != 0 {
                return Err(super::errno::ENOTSUP);
            }
            Action {
                handler: u64::from_le_bytes(b[0..8].try_into().unwrap()),
                flags: qnx::sa_flags_in(flags),
                mask: qnx::set_in(u64::from_le_bytes(b[16..24].try_into().unwrap())) & !UNBLOCKABLE,
            }
        } else {
            let mut b = [0u8; BSD_SIGACTION_SIZE];
            umem::read(space, act, &mut b)?;
            Action {
                handler: u64::from_le_bytes(b[0..8].try_into().unwrap()),
                flags: u32::from_le_bytes(b[8..12].try_into().unwrap()),
                mask: SigSet::from_le_bytes(b[12..28].try_into().unwrap()) & !UNBLOCKABLE,
            }
        };
        if a.handler != SIG_DFL && bit(signo as usize) & UNBLOCKABLE != 0 {
            return Err(EINVAL);
        }
        Some(a)
    } else {
        None
    };
    let old = {
        let mut sig = p.signals.lock();
        let old = sig.actions[signo];
        if let Some(a) = new {
            sig.actions[signo] = a;
            sig.stub = stub;
        }
        old
    };
    if oact != 0 {
        if is_qnx {
            let mut b = [0u8; SIGACTION_SIZE];
            b[0..8].copy_from_slice(&old.handler.to_le_bytes());
            b[8..12].copy_from_slice(&qnx::sa_flags_out(old.flags).to_le_bytes());
            b[16..24].copy_from_slice(&qnx::set_out(old.mask).to_le_bytes());
            umem::write(space, oact, &b)?;
        } else {
            let mut b = [0u8; BSD_SIGACTION_SIZE];
            b[0..8].copy_from_slice(&old.handler.to_le_bytes());
            b[8..12].copy_from_slice(&old.flags.to_le_bytes());
            b[12..28].copy_from_slice(&old.mask.to_le_bytes());
            umem::write(space, oact, &b)?;
        }
    }
    Ok(0)
}

/// `SignalProcmask(pid, tid, how, set, oldset)`: tid 0 is the caller.
pub fn signal_procmask(pid: i32, tid: i32, how: u32, set: u64, oldset: u64) -> KerResult {
    let p = own_process(pid)?;
    let me = sched::current();
    // Thread `tid` of the process (0: the caller; ESRCH if none).
    let t = match tid {
        0 => me.arc(),
        tid => sched::threads_of(&p)
            .into_iter()
            .find(|t| t.ptid == tid as u32)
            .ok_or(ESRCH)?,
    };
    let space = Space::current();
    let old = t.sigmask.load(Ordering::Relaxed);
    // `SIG_PENDING` (`sigpending()`): the pending signals it blocks, its
    // own and the process's.
    if how == SIG_PENDING {
        let pending = {
            let s = p.signals.lock();
            s.pending | s.threads.get(&t.ptid).map_or(0, |ts| ts.pending)
        } & old;
        if oldset != 0 {
            write_set(space, oldset, pending)?;
        }
        return Ok(0);
    }
    if set != 0 {
        let set = read_set(space, set)? & !UNBLOCKABLE;
        // FreeBSD's `how` is QNX's plus one.
        let how = if current_is_qnx() { how } else { how.wrapping_sub(1) };
        let new = match how {
            SIG_BLOCK => old | set,
            SIG_UNBLOCK => old & !set,
            SIG_SETMASK => set,
            _ => return Err(EINVAL),
        };
        t.sigmask.store(valid_mask(new), Ordering::Relaxed);
    }
    if oldset != 0 {
        write_set(space, oldset, old)?;
    }
    Ok(0)
}

/// Does a thread of `p` (`only` that one, if given) wait for signal
/// `signo` (`sigwaitinfo()`)?
pub fn waited_for(p: &process::Process, signo: usize, only: Option<&sched::Thread>) -> bool {
    (1..NSIG).contains(&signo)
        && sched::threads_of(p).iter().any(|t| {
            only.is_none_or(|o| o.ptid == t.ptid)
                && t.state() == sched::State::Sigwaitinfo
                && t.sigwait_set.load(Ordering::Relaxed) & bit(signo) != 0
        })
}

/// Send signal `signo` to process `p` (SIGCHLD, kill()). Ignored signals
/// are dropped at once. A thread of `p` waiting for signals is woken.
pub fn post(p: &process::Process, signo: usize, info: SigInfo) {
    let _ = post_to(p, None, signo, info, false);
}

/// Send signal `signo` to thread `t` of process `p` (`pthread_kill()`,
/// `SIGEV_SIGNAL_THREAD`): only `t` takes it.
pub fn post_thread(p: &process::Process, t: &sched::Thread, signo: usize, info: SigInfo) {
    let _ = post_to(p, Some(t), signo, info, false);
}

/// `user`: sent by `SignalKill()`, which fails with ENOMEM rather than
/// let a signal queue grow into the kernel's reserve (review SCHED-8).
/// procnto queues signals without a count limit, about 40 bytes each,
/// until memory runs out and the call fails with ENOMEM (measured: 50
/// million). A signal the kernel sends whose queue can't grow at all is
/// dropped.
fn post_to(
    p: &process::Process,
    only: Option<&sched::Thread>,
    signo: usize,
    info: SigInfo,
    user: bool,
) -> Result<(), u32> {
    if !(1..NSIG).contains(&signo) {
        return Ok(());
    }
    // Job control (see `debug`): SIGSTOP stops the process, SIGCONT lets
    // it go on; SIGKILL gets its stopped threads going, to end.
    match signo {
        SIGSTOP => {
            super::debug::job_stop(p, true, SIGSTOP as u16);
            return Ok(());
        }
        SIGCONT => {
            // Pending stop signals go (POSIX).
            let mut sig = p.signals.lock();
            for s in [SIGTSTP, SIGTTIN, SIGTTOU] {
                sig.discard(s);
            }
            drop(sig);
            super::debug::job_continue(p)
        }
        _ => {}
    }
    {
        let mut sig = p.signals.lock();
        let handler = sig.actions[signo].handler;
        // An ignored signal is discarded, unless a thread blocks it or
        // waits for it (libc's poll() waits for SIGSELECT so).
        let wants = |t: &sched::Thread| {
            (t.sigmask.load(Ordering::Relaxed) | t.sigwait_set.load(Ordering::Relaxed)) & bit(signo)
                != 0
        };
        let wanted = || match only {
            Some(t) => wants(t),
            None => sched::threads_of(p).iter().any(|t| wants(t)),
        };
        if (handler == SIG_IGN || (handler == SIG_DFL && bit(signo) & default_ignored(p) != 0))
            && !wanted()
        {
            return Ok(());
        }
        // A stop signal nobody blocks or waits for, at its default action:
        // the process stops now (a blocked one stops it when delivered).
        if handler == SIG_DFL && bit(signo) & DEFAULT_STOP != 0 && !wanted() {
            drop(sig);
            super::debug::job_stop(p, true, signo as u16);
            return Ok(());
        }
        let info = SigInfo {
            signo: signo as i32,
            ..info
        };
        p.sig_hint.store(true, Ordering::Release);
        // With SA_SIGINFO each signal sent is queued, and so are the top 8
        // (SIGSELECT on: libc's poll() takes one per armed descriptor);
        // else the first one stays pending and the others are dropped (as
        // measured on procnto; its documentation says they replace it).
        let queued = match p.personality() {
            Personality::Qnx => sig.actions[signo].flags & SA_SIGINFO != 0 || bit(signo) & SPECIAL != 0,
            Personality::Native => signo >= SIGRTMIN || bit(signo) & SPECIAL != 0,
        };
        let put = |pending: &mut SigSet, map: &mut alloc::collections::BTreeMap<u8, VecDeque<SigInfo>>| {
            let q = map.entry(signo as u8).or_default();
            if *pending & bit(signo) == 0 {
                q.clear();
            } else if !queued {
                return Ok(());
            }
            if q.len() == q.capacity() {
                // A full queue doubles: from a user, only above the
                // kernel's reserve; any, only if the heap has the room.
                let grow = q.capacity().max(4) * core::mem::size_of::<SigInfo>();
                if user && !crate::vm::pmem::user_may_take(grow.div_ceil(crate::vm::pmem::PAGE_SIZE as usize)) {
                    return Err(super::errno::ENOMEM);
                }
                q.try_reserve(1).map_err(|_| super::errno::ENOMEM)?;
            }
            q.push_back(info);
            *pending |= bit(signo);
            Ok(())
        };
        match only {
            Some(t) => {
                let ts = sig.threads.entry(t.ptid).or_default();
                put(&mut ts.pending, &mut ts.info)?;
            }
            None => {
                let s = &mut *sig;
                put(&mut s.pending, &mut s.info)?;
            }
        }
    }
    // Interrupt the threads that can take it and are waiting for signals
    // or sleeping; the others see it on their next return to user mode.
    // A signal kept only because a thread blocks it (its action ignores
    // it) interrupts nobody: it is discarded when delivered, and would
    // end a wait of a thread that doesn't block it for nothing.
    let ignored = {
        let sig = p.signals.lock();
        let handler = sig.actions[signo].handler;
        handler == SIG_IGN || (handler == SIG_DFL && bit(signo) & default_ignored(p) != 0)
    };
    let threads = sched::threads_of(p);
    let mut taken = false;
    for t in &threads {
        if only.is_some_and(|o| o.ptid != t.ptid) {
            continue;
        }
        let waits_for_it = t.state() == sched::State::Sigwaitinfo
            && t.sigwait_set.load(Ordering::Relaxed) & bit(signo) != 0;
        let can_take_it = t.sigmask.load(Ordering::Relaxed) & bit(signo) == 0
            && matches!(
                t.state(),
                sched::State::Sigsuspend
                    | sched::State::Nanosleep
                    | sched::State::Send
                    | sched::State::Receive
                    | sched::State::Reply
                    | sched::State::Condvar
                    | sched::State::Sem
                    | sched::State::Barrier
                    | sched::State::MqSend
                    | sched::State::MqReceive
                    | sched::State::Sigwaitinfo
                    | sched::State::Intr
                    | sched::State::TraceBuffer
            );
        if waits_for_it {
            sched::wake(t);
            taken = true;
            break;
        }
        if signo == SIGKILL && t.state() == sched::State::Stopped {
            sched::wake(t);
            continue;
        }
        // A thread that can take the signal leaves a blocking call: it
        // fails with EINTR (or its server is asked to let it go).
        if can_take_it && !ignored && crate::ker::unblock(t, EINTR) {
            taken = true;
            break;
        }
    }
    // None blocked takes it: one busy in user code on another CPU is
    // interrupted, so that it takes it now, not at that CPU's next tick
    // (review SCHED-10; procnto sends an IPI).
    if !taken
        && !ignored
        && let Some(t) = threads.iter().find(|t| {
            only.is_none_or(|o| o.ptid == t.ptid)
                && t.state() == sched::State::Running
                && t.sigmask.load(Ordering::Relaxed) & bit(signo) == 0
        })
    {
        sched::interrupt(t);
    }
    Ok(())
}

/// Thread `tid` of `p` has ended: its own pending signals go.
pub fn forget_thread(p: &process::Process, tid: u32) {
    p.signals.lock().threads.remove(&tid);
}

/// Would the calling thread take signal `signo` in a handler now?
pub fn caught(signo: usize) -> bool {
    let me = sched::current();
    let sig = me.process.signals.lock();
    (1..NSIG).contains(&signo)
        && sig.actions[signo].handler > SIG_IGN
        && me.sigmask.load(Ordering::Relaxed) & bit(signo) == 0
}

/// A call the calling thread's signal interrupted is made again after the
/// handler, as FreeBSD's kernel does: its next signal has a handler with
/// `SA_RESTART`. QNX's `SA_*` has no such flag: never for a QNX process.
pub fn restarts() -> bool {
    const SA_RESTART: u32 = 0x2;
    let me = sched::current();
    let sig = me.process.signals.lock();
    let ready = sig.pending_for(me.ptid) & !me.sigmask.load(Ordering::Relaxed);
    ready != 0 && {
        let a = sig.actions[me.process.personality().next_signal(ready)];
        a.handler > SIG_IGN && a.flags & SA_RESTART != 0
    }
}

/// A fault of the calling thread that nobody debugs: as in QNX, its
/// signal (`info`). A handler takes it; ignored, blocked or not caught, it
/// ends the process (killed by it). Returns only if a handler will run.
pub fn fault(info: SigInfo) {
    let me = sched::current();
    let p = &*me.process;
    let signo = info.signo as usize;
    if !caught(signo) {
        process::exit_current(CLD_KILLED, signo as i32);
    }
    post(&p, signo, info);
}

/// Send signal `signo` to every process of process group `pgid` (the
/// terminal's foreground group, for the keys of `ISIG`).
pub fn post_group(pgid: i32, signo: usize) {
    let info = SigInfo {
        code: SI_USER,
        ..Default::default()
    };
    for p in process::all() {
        if p.pid != process::KERNEL_PID && p.pgid() == pgid && !p.is_zombie() {
            post(&p, signo, info);
        }
    }
}

/// `SignalWaitinfo(set, info)`: wait until a signal of `set` is pending
/// (blocked or not), take it, fill in `info` (if set) and return its
/// number.
///
/// `SignalWaitinfoMask(set, info, mask)` (`SignalWaitinfo()` is it with no
/// mask; libc's `poll()` waits so): with a mask, the thread's signal mask
/// is `mask` while it waits, as with `SignalSuspend()`. Either way a
/// signal outside `set` that the thread doesn't block ends the wait with
/// EINTR, and is delivered (then the old mask is back).
pub fn signal_waitinfo(set: u64, info: u64, mask: u64) -> KerResult {
    let me = sched::current();
    let p = &*me.process;
    let space = Space::current();
    let set = read_set(space, set)? & !UNBLOCKABLE;
    let old = if mask != 0 {
        let new = read_set(space, mask)?;
        let old = me.sigmask.swap(valid_mask(new), Ordering::Relaxed);
        me.suspend_mask.store(old, Ordering::Relaxed);
        Some(old)
    } else {
        None
    };
    let mut timeout = None;
    loop {
        let taken = {
            let mut sig = p.signals.lock();
            let pending = sig.pending_for(me.ptid);
            let ready = pending & set;
            if ready == 0 && pending & !set & !me.sigmask.load(Ordering::Relaxed) != 0 {
                // Another signal to take: delivered on the way out.
                me.sigwait_set.store(0, Ordering::Relaxed);
                return Err(EINTR);
            }
            if ready == 0 {
                me.sigwait_set.store(set, Ordering::Relaxed);
                me.wake_error.store(0, Ordering::Relaxed);
                sched::prepare_block(sched::State::Sigwaitinfo);
                timeout = crate::timeout::arm(sched::State::Sigwaitinfo);
                None
            } else {
                let signo = p.personality().next_signal(ready);
                Some((signo, sig.take(me.ptid, signo)))
            }
        };
        match taken {
            Some((signo, si)) => {
                me.sigwait_set.store(0, Ordering::Relaxed);
                if let Some(old) = old {
                    me.sigmask.store(old, Ordering::Relaxed);
                    me.suspend_mask
                        .store(sched::SUSPEND_NONE, Ordering::Relaxed);
                }
                if info != 0 {
                    if current_is_qnx() {
                        umem::write(space, info, &qnx::view(si).to_qnx_bytes())?;
                    } else {
                        umem::write(space, info, &si.to_bytes())?;
                    }
                }
                return Ok(p.personality().signo_out(signo) as u64);
            }
            None => {
                sched::block();
                crate::timeout::disarm(timeout.take());
                if let e @ 1.. = me.wake_error.swap(0, Ordering::Relaxed) {
                    me.sigwait_set.store(0, Ordering::Relaxed);
                    return Err(e);
                }
            }
        }
    }
}

/// Deliver the calling thread's pending, unblocked signals before it
/// returns to user mode with `frame`.
pub fn deliver(frame: &mut TrapFrame) {
    let me = sched::current();
    let p = &*me.process;
    if p.pid == process::KERNEL_PID {
        return;
    }
    while p.sig_hint.load(Ordering::Acquire) {
        let mask = me.sigmask.load(Ordering::Relaxed);
        // A signal its debugger traces stops the process first (it stays
        // pending, unless the debugger clears it).
        let next = {
            let ready = p.signals.lock().pending_for(me.ptid) & !mask;
            (ready != 0).then(|| p.personality().next_signal(ready))
        };
        if let Some(signo) = next
            && signo != SIGKILL
            && super::debug::trace_signal(&p, signo)
        {
            super::debug::park_if_stopped(me);
            if me.exit_on_return.load(Ordering::Relaxed) {
                sched::exit();
            }
            continue;
        }
        let (signo, action, info, stub) = {
            let mut sig = p.signals.lock();
            let ready = sig.pending_for(me.ptid) & !mask;
            if ready == 0 {
                if sig.pending == 0 && sig.threads.is_empty() {
                    p.sig_hint.store(false, Ordering::Release);
                }
                break;
            }
            let signo = p.personality().next_signal(ready);
            let info = sig.take(me.ptid, signo);
            let action = sig.actions[signo];
            // SIG_DFL again; the flags stay, but for SA_SIGINFO (procnto).
            if action.handler > SIG_IGN && action.flags & SA_RESETHAND != 0 {
                sig.actions[signo] = Action {
                    flags: action.flags & !SA_SIGINFO,
                    ..Action::default()
                };
            }
            (signo, action, info, sig.stub)
        };
        // Delivered: its timer may send it again (`sigwaitinfo()` leaves
        // the timer to find out, see `ker::timer`).
        if info.timer != 0 {
            super::timer::signal_taken(info.timer);
        }
        match action.handler {
            SIG_IGN => continue,
            SIG_DFL if bit(signo) & default_ignored(p) != 0 => continue,
            SIG_DFL if bit(signo) & DEFAULT_STOP != 0 => {
                super::debug::job_stop(p, true, signo as u16);
                super::debug::park_if_stopped(me);
                continue;
            }
            SIG_DFL => process::exit_current(CLD_KILLED, signo as i32),
            handler => {
                let entered = match p.personality() {
                    Personality::Qnx => enter_handler_qnx(frame, signo, handler, &action, info, stub),
                    Personality::Native => enter_handler(frame, signo, handler, &action, info, stub),
                };
                if entered.is_err() {
                    // No room for the frame on the user stack: the process
                    // ends with SIGDOOM, "target thread unable to process
                    // signal", as on procnto (measured with a stack
                    // pointer of 64; review SCHED-30).
                    let doom = match p.personality() {
                        Personality::Qnx => qnx::signo_in(SIGDOOM),
                        Personality::Native => SIGSEGV,
                    };
                    process::exit_current(CLD_KILLED, doom as i32);
                }
                return;
            }
        }
    }
    // A SignalSuspend that delivered nothing to a handler puts its mask back.
    let saved = me.suspend_mask.swap(sched::SUSPEND_NONE, Ordering::Relaxed);
    if saved != sched::SUSPEND_NONE {
        me.sigmask.store(saved, Ordering::Relaxed);
    }
}

const SIGDOOM: usize = 32;
/// `si_code` values of SIGCHLD (`<sys/siginfo.h>`).
pub const CLD_EXITED: i32 = 1;
pub const CLD_KILLED: i32 = 2;
pub const CLD_STOPPED: i32 = 5;
pub const CLD_CONTINUED: i32 = 6;

/// Make the thread run `handler` through the trampoline when it returns
/// to user mode, saving its state on its stack.
fn enter_handler_qnx(
    frame: &mut TrapFrame,
    signo: usize,
    handler: u64,
    action: &Action,
    info: SigInfo,
    stub: u64,
) -> Result<(), u32> {
    let me = sched::current();
    let space = Space::current();
    // The mask the handler's return restores: the one SignalSuspend
    // replaced, if any.
    let saved = me.suspend_mask.swap(sched::SUSPEND_NONE, Ordering::Relaxed);
    let old_mask = if saved != sched::SUSPEND_NONE {
        saved
    } else {
        me.sigmask.load(Ordering::Relaxed)
    };
    // SA_ONSTACK: from the top of the alternate stack, unless on it.
    let (alt, alt_size) = *me.altstack.lock();
    // User values: an alternate stack whose end overflows is as good as
    // none (`sigaltstack` refuses one, but the check is here too), and a
    // stack pointer with no room below it makes the push fault, as
    // writing below address 0 would.
    let alt_end = alt.checked_add(alt_size).filter(|_| alt_size != 0);
    let on_alt = alt_end.is_some_and(|end| (alt..end).contains(&frame.sp_el0));
    let sp = match alt_end {
        Some(end) if action.flags & SA_ONSTACK != 0 && !on_alt => end,
        _ => frame.sp_el0,
    };
    let uc_at = sp.saturating_sub(UC_SIZE as u64) & !15;
    let frame_at = uc_at.saturating_sub(FRAME_SIZE as u64) & !15;
    let mut uc = [0u8; UC_SIZE];
    uc[UC_SIGMASK..UC_SIGMASK + 8].copy_from_slice(&qnx::set_out(old_mask).to_le_bytes());
    let mut gpr = [0u64; 34];
    gpr[..31].copy_from_slice(&frame.x);
    gpr[31] = frame.sp_el0;
    gpr[32] = frame.elr;
    gpr[33] = frame.spsr;
    for (i, r) in gpr.iter().enumerate() {
        uc[UC_GPR + i * 8..UC_GPR + i * 8 + 8].copy_from_slice(&r.to_le_bytes());
    }
    let mut fp = FpState::new();
    sched::fp_make_live();
    fp_save(&mut fp);
    uc[UC_FPU..UC_FPU + QNX_FPU_SIZE].copy_from_slice(&fp.to_qnx());
    let mut f = [0u8; FRAME_SIZE];
    f[..SIGINFO_SIZE].copy_from_slice(&qnx::view(info).to_qnx_bytes());
    f[48..56].copy_from_slice(&handler.to_le_bytes());
    f[56..64].copy_from_slice(&uc_at.to_le_bytes());
    f[64..72].copy_from_slice(&qnx::set_out(old_mask).to_le_bytes());
    umem::write(space, uc_at, &uc)?;
    umem::write(space, frame_at, &f)?;
    let mut mask = old_mask | action.mask;
    if action.flags & SA_NODEFER == 0 {
        mask |= bit(signo);
    }
    me.sigmask.store(valid_mask(mask), Ordering::Relaxed);
    frame.x[0] = frame_at;
    frame.sp_el0 = frame_at;
    frame.elr = stub;
    Ok(())
}

/// `enter_handler()` for a FreeBSD process: the frame is FreeBSD's
/// `siginfo_t`, then the handler and the context, a FreeBSD `ucontext_t`,
/// and libsys's trampoline calls the handler.
fn enter_handler(
    frame: &mut TrapFrame,
    signo: usize,
    handler: u64,
    action: &Action,
    info: SigInfo,
    stub: u64,
) -> Result<(), u32> {
    let me = sched::current();
    let space = Space::current();
    let saved = me.suspend_mask.swap(sched::SUSPEND_NONE, Ordering::Relaxed);
    let old_mask = if saved != sched::SUSPEND_NONE {
        saved
    } else {
        me.sigmask.load(Ordering::Relaxed)
    };
    let (alt, alt_size) = *me.altstack.lock();
    let alt_end = alt.checked_add(alt_size).filter(|_| alt_size != 0);
    let on_alt = alt_end.is_some_and(|end| (alt..end).contains(&frame.sp_el0));
    let sp = match alt_end {
        Some(end) if action.flags & SA_ONSTACK != 0 && !on_alt => end,
        _ => frame.sp_el0,
    };
    let uc_at = sp.saturating_sub(BSD_UC_SIZE as u64) & !15;
    let frame_at = uc_at.saturating_sub(BSD_FRAME_SIZE as u64) & !15;
    let mut uc = [0u8; BSD_UC_SIZE];
    uc[..16].copy_from_slice(&old_mask.to_le_bytes());
    let mut gpr = [0u64; 34];
    gpr[..31].copy_from_slice(&frame.x);
    gpr[31] = frame.sp_el0;
    gpr[32] = frame.elr;
    gpr[33] = frame.spsr;
    for (i, r) in gpr.iter().enumerate() {
        uc[BSD_UC_GPR + i * 8..BSD_UC_GPR + i * 8 + 8].copy_from_slice(&r.to_le_bytes());
    }
    let mut fp = FpState::new();
    sched::fp_make_live();
    fp_save(&mut fp);
    uc[BSD_UC_FPU..BSD_UC_FPU + QNX_FPU_SIZE].copy_from_slice(&fp.to_qnx());
    // `mc_flags`: _MC_FP_VALID.
    uc[BSD_UC_FPU + 528..BSD_UC_FPU + 532].copy_from_slice(&1u32.to_le_bytes());
    let mut f = [0u8; BSD_FRAME_SIZE];
    f[..BSD_SIGINFO_SIZE].copy_from_slice(&info.to_bytes());
    f[80..88].copy_from_slice(&handler.to_le_bytes());
    f[88..96].copy_from_slice(&uc_at.to_le_bytes());
    umem::write(space, uc_at, &uc)?;
    umem::write(space, frame_at, &f)?;
    let mut mask = old_mask | action.mask;
    if action.flags & SA_NODEFER == 0 {
        mask |= bit(signo);
    }
    me.sigmask.store(valid_mask(mask), Ordering::Relaxed);
    frame.x[0] = frame_at;
    frame.sp_el0 = frame_at;
    frame.elr = stub;
    Ok(())
}

// ANCHOR: signal_return_native
/// `SignalReturn()` for a FreeBSD process: the context, which the handler
/// may have changed, mask and all.
fn signal_return_native(frame: &mut TrapFrame, at: u64) -> Result<(), u32> {
    let me = sched::current();
    let space = Space::current();
    let mut f = [0u8; BSD_FRAME_SIZE];
    umem::read(space, at, &mut f)?;
    let uc_at = u64::from_le_bytes(f[88..96].try_into().unwrap());
    let mut uc = [0u8; BSD_UC_SIZE];
    umem::read(space, uc_at, &mut uc)?;
    let r = |i: usize| {
        u64::from_le_bytes(uc[BSD_UC_GPR + i * 8..BSD_UC_GPR + i * 8 + 8].try_into().unwrap())
    };
    for i in 0..31 {
        frame.x[i] = r(i);
    }
    frame.sp_el0 = r(31);
    frame.elr = r(32);
    frame.spsr = r(33) & 0xf000_0000;
    let fpu: &[u8; QNX_FPU_SIZE] = uc[BSD_UC_FPU..BSD_UC_FPU + QNX_FPU_SIZE].try_into().unwrap();
    sched::fp_make_live();
    fp_restore(&FpState::from_qnx(fpu));
    let mask = SigSet::from_le_bytes(uc[..16].try_into().unwrap());
    me.sigmask.store(valid_mask(mask), Ordering::Relaxed);
    Ok(())
}
// ANCHOR_END: signal_return_native

/// `SignalReturn(frame)`: back from a handler to the saved context. Sets
/// the whole user state, so the caller must not complete it as a call.
pub fn signal_return(frame: &mut TrapFrame, at: u64) -> Result<(), u32> {
    if !current_is_qnx() {
        return signal_return_native(frame, at);
    }
    let me = sched::current();
    let space = Space::current();
    let mut f = [0u8; FRAME_SIZE];
    umem::read(space, at, &mut f)?;
    let uc_at = u64::from_le_bytes(f[56..64].try_into().unwrap());
    let mask = qnx::set_in(u64::from_le_bytes(f[64..72].try_into().unwrap()));
    let mut uc = [0u8; UC_SIZE];
    umem::read(space, uc_at, &mut uc)?;
    let r =
        |i: usize| u64::from_le_bytes(uc[UC_GPR + i * 8..UC_GPR + i * 8 + 8].try_into().unwrap());
    for i in 0..31 {
        frame.x[i] = r(i);
    }
    frame.sp_el0 = r(31);
    frame.elr = r(32);
    // Only the condition flags come from user memory: it stays EL0t.
    frame.spsr = r(33) & 0xf000_0000;
    let fpu: &[u8; QNX_FPU_SIZE] = uc[UC_FPU..UC_FPU + QNX_FPU_SIZE].try_into().unwrap();
    sched::fp_make_live();
    fp_restore(&FpState::from_qnx(fpu));
    me.sigmask.store(valid_mask(mask), Ordering::Relaxed);
    Ok(())
}

/// `SignalSuspend(set)`: wait with blocked signals `set` until a signal is
/// delivered; always fails with EINTR, after the handler has run.
pub fn signal_suspend(set: u64) -> KerResult {
    let me = sched::current();
    let p = &*me.process;
    let new = read_set(Space::current(), set)?;
    let old = me.sigmask.swap(valid_mask(new), Ordering::Relaxed);
    me.suspend_mask.store(old, Ordering::Relaxed);
    let wait = {
        let sig = p.signals.lock();
        let wait = sig.pending_for(me.ptid) & !me.sigmask.load(Ordering::Relaxed) == 0;
        let t = if wait {
            sched::prepare_block(sched::State::Sigsuspend);
            crate::timeout::arm(sched::State::Sigsuspend)
        } else {
            None
        };
        (wait, t)
    };
    if wait.0 {
        sched::block();
        crate::timeout::disarm(wait.1);
    }
    match me.wake_error.swap(0, Ordering::Relaxed) {
        0 => Err(EINTR),
        e => Err(e),
    }
}

/// `SignalKillSigval(nd, pid, tid, signo, code, &value)` (`sigqueue()`):
/// `SignalKill` with the value read from `union sigval`.
pub fn signal_kill_sigval(
    nd: u32,
    pid: i32,
    tid: i32,
    signo: i32,
    code: i32,
    value: u64,
) -> KerResult {
    let mut v = [0u8; 8];
    if value != 0 {
        umem::read(Space::current(), value, &mut v)?;
    }
    signal_kill(nd, pid, tid, signo, code, u64::from_le_bytes(v))
}

/// `SignalKill(nd, pid, tid, signo, code, value)`: pid > 0 is a process,
/// 0 the caller's process group, < 0 the process group -pid; `tid` a
/// thread of process `pid`, which alone takes the signal. Signal 0 only
/// checks that the target exists.
pub fn signal_kill(nd: u32, pid: i32, tid: i32, signo: i32, code: i32, value: u64) -> KerResult {
    if nd != 0 {
        return Err(super::errno::ENOREMOTE);
    }
    // Each personality's own signals: QNX's 1 to 64; FreeBSD's, which
    // leave 34 to 64 out, up to `SIGRTMAX`.
    let personality = process::current().personality();
    let signo = usize::try_from(signo)
        .ok()
        .filter(|&s| match personality {
            Personality::Qnx => s <= 64,
            Personality::Native => s <= 33 || (SIGRTMIN..NSIG - 1).contains(&s),
        })
        .ok_or(EINVAL)?;
    let signo = personality.signo_in(signo);
    // As procnto: no kernel codes from users; procnto takes no signals.
    let code = if current_is_qnx() {
        if code > 0 {
            return Err(EINVAL);
        }
        qnx::code_in(code)
    } else {
        if code != 0 && !(SI_USER..=0x10005).contains(&code) {
            return Err(EINVAL);
        }
        code
    };
    if pid == process::KERNEL_PID {
        return Err(EPERM);
    }
    let me = process::current();
    let targets: alloc::vec::Vec<process::ProcessRef> = match pid {
        p if p > 0 => alloc::vec![process::lookup(p).ok_or(ESRCH)?],
        p => {
            let pgid = if p == 0 { me.pgid() } else { -p };
            process::all()
                .into_iter()
                .filter(|q| q.pid != process::KERNEL_PID && q.pgid() == pgid && !q.is_zombie())
                .collect()
        }
    };
    if targets.is_empty() {
        return Err(ESRCH);
    }
    // POSIX: the sender's real or effective user ID is the target's real
    // or saved one, or the sender has PROCMGR_AID_SIGNAL (for the signal).
    // A process group gets the signal where permitted (EPERM if nowhere).
    let (ruid, euid) = {
        let c = me.cred.lock();
        (c.ruid, c.euid)
    };
    let may = |q: &process::Process| {
        let c = q.cred.lock();
        let (tr, ts) = (c.ruid, c.suid);
        drop(c);
        [ruid, euid].iter().any(|&u| u == tr || u == ts)
            || super::ability::allowed_as(me, euid, super::ability::SIGNAL, Some(signo as u64))
    };
    let targets: alloc::vec::Vec<process::ProcessRef> = targets.into_iter().filter(|q| may(q)).collect();
    if targets.is_empty() {
        return Err(EPERM);
    }
    let info = SigInfo {
        code: if code == 0 { SI_USER } else { code },
        pid: me.pid,
        value,
        ..Default::default()
    };
    // A thread of one process (a process group's ignore `tid`).
    if tid > 0 && pid > 0 {
        let p = &targets[0];
        let t = sched::threads_of(p)
            .into_iter()
            .find(|t| t.ptid == tid as u32 && t.state() != sched::State::Dead)
            .ok_or(ESRCH)?;
        if signo != 0 {
            post_to(p, Some(&t), signo, info, true)?;
        }
        return Ok(0);
    }
    if signo != 0 {
        for t in targets {
            post_to(&t, None, signo, info, true)?;
        }
    }
    Ok(0)
}
