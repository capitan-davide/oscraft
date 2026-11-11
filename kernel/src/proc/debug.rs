//! Introspection: the process manager messages behind QNX 8's
//! `introspection_*()` functions (pidin, top, slay...): `_PROC_GETNAME`,
//! `_PROC_GET_ALL_PID` and `_PROC_DEBUG`.
//!
//! Contract, from the SDP's procnto (the messages go through
//! `procmgr_handler`'s `procmgr_msgs` table; `nto_debug_process`,
//! `nto_debug_thread_info`) and `<sys/debug.h>`:
//! - `_PROC_GETNAME` { type, zero, reserved, `pid64_t` pid }: the reply's
//!   status is the length of the process's name, the data the name and a
//!   NUL (only the NUL if it doesn't fit; status 0 without a name).
//! - `_PROC_GET_ALL_PID` { type, zero }: every process's `pid64_t`, the
//!   status their size in bytes (the caller retries with a larger buffer
//!   if that is more than it gave).
//! - `_PROC_DEBUG` { type, subtype, tid, `pid64_t` pid }, pid 0 being the
//!   sender. Subtype 0: the process's `debug_process_t` (272 bytes); a
//!   zombie's has only its pid, parent and `_NTO_PF_ZOMBIE`. Subtype 3:
//!   the `debug_thread64_t` (280 bytes) of its first thread whose ID is at
//!   least `tid`, ESRCH past the last. Subtypes 1 and 2: thread `tid`'s
//!   registers (`AARCH64_CPU_REGISTERS`, `AARCH64_FPU_REGISTERS`), the
//!   status their size (procnto's `debug_registers_read`). A thread running
//!   on another CPU has none to show (EBUSY); process 1's own threads show
//!   zeros.
//!
//! A `pid64_t` here is the plain pid (procnto accepts those, with zero
//! upper bits, everywhere). The CPU times are all user time (`utime`,
//! `cutime`, `sutime`). The process flags (`_NTO_PF_*`) given are
//! `ZOMBIE`, `LOADING`, `NOZOMBIE`, `SLEADER`, `64BIT` and, for programs,
//! `NOEXEC_STACK`. A thread's stack is the region its stack pointer is
//! in. Not filled in yet: `sig_queue`.

use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use crate::arch::kercall::call;
use crate::ker::errno::{EINVAL, ENOSYS, ESRCH};
use crate::ker::number::*;
use crate::ker::process::{self, Process};
use crate::sched::{self, State, Thread, ThreadRef};

pub const PROC_GETNAME: u16 = 0x0023;
pub const PROC_GET_ALL_PID: u16 = 0x0024;
pub const PROC_DEBUG: u16 = 0x0025;
pub const PROC_GET_ALL_CHILD_PID: u16 = 0x0027;
const DEBUG_PROC_INFO: u16 = 0;
const DEBUG_GREGS: u16 = 1;
const DEBUG_FPREGS: u16 = 2;
const DEBUG_THREAD_STATUS: u16 = 3;
/// `sizeof(AARCH64_CPU_REGISTERS)`: x0-x30, sp, elr, pstate.
pub const GREG_SIZE: usize = 34 * 8;
// `_NTO_PF_*`
const PF_LOADING: u32 = 0x2;
const PF_TERMING: u32 = 0x4;
const PF_DEBUG_STOPPED: u32 = 0x100;
const PF_NOZOMBIE: u32 = 0x10;
const PF_SLEADER: u32 = 0x1_0000;
const PF_64BIT: u32 = 0x0800_0000;
const PF_NOEXEC_STACK: u32 = 0x4000_0000;
/// `sizeof(debug_process_t)` and `sizeof(debug_thread64_t)`.
const PROCESS_SIZE: usize = 272;
const THREAD_SIZE: usize = 280;
/// `_NTO_PF_ZOMBIE`
const PF_ZOMBIE: u32 = 0x8;

fn reply(rcvid: u64, status: usize, data: &[u8]) {
    let _ = call(
        __KER_MSG_REPLYV,
        [
            rcvid,
            status as u64,
            data.as_ptr() as u64,
            (-(data.len() as i64)) as u64,
            0,
            0,
        ],
    );
}

/// The process a `pid64_t` names (0: the sender).
fn target(sender: i32, pid: u64) -> Result<crate::ker::process::ProcessRef, u32> {
    let pid = if pid == 0 { sender as u64 } else { pid };
    let p = process::lookup(pid as u32 as i32).ok_or(ESRCH)?;
    // The plain pid, or its 64-bit form exactly.
    if pid >> 32 != 0 && pid != pid64(p.pid) {
        return Err(ESRCH);
    }
    Ok(p)
}

/// The 64-bit pid of process `pid` (`pid64_t`): as procnto, 1 above the
/// pid for every process but procnto itself.
pub fn pid64(pid: i32) -> u64 {
    if pid == process::KERNEL_PID { 1 } else { 1 << 32 | pid as u32 as u64 }
}

fn put(b: &mut [u8], at: usize, v: &[u8]) {
    b[at..at + v.len()].copy_from_slice(v);
}

/// `debug_process_t` of `p`.
pub fn process_info(p: &Process) -> [u8; PROCESS_SIZE] {
    let mut d = [0u8; PROCESS_SIZE];
    let parent = p.ppid.load(Ordering::Relaxed);
    put(&mut d, 0, &p.pid.to_le_bytes());
    put(&mut d, 4, &parent.to_le_bytes());
    if p.is_zombie() {
        put(&mut d, 8, &PF_ZOMBIE.to_le_bytes());
        return d;
    }
    let threads = sched::threads_of(p);
    let mut flags = PF_64BIT;
    if p.pid != process::KERNEL_PID {
        flags |= PF_NOEXEC_STACK;
    }
    if p.loading.load(Ordering::Relaxed) {
        flags |= PF_LOADING;
    }
    if p.nozombie.load(Ordering::Relaxed) {
        flags |= PF_NOZOMBIE;
    }
    // Stopped by its debugger at a point of interest (until a run); parked
    // at its end for it: ending.
    {
        let st = p.debug.lock();
        if st.held && st.stop.flags & crate::ker::debug::FLAG_ISTOP != 0 {
            flags |= PF_DEBUG_STOPPED;
        }
        if st.exiting {
            flags |= PF_TERMING;
        }
    }
    if p.sid() == p.pid {
        flags |= PF_SLEADER;
    }
    put(&mut d, 8, &flags.to_le_bytes());
    put(&mut d, 12, &p.umask.load(Ordering::Relaxed).to_le_bytes());
    put(&mut d, 24, &p.pgid().to_le_bytes());
    put(&mut d, 28, &p.sid().to_le_bytes());
    put(
        &mut d,
        32,
        &p.base_address.load(Ordering::Relaxed).to_le_bytes(),
    );
    put(
        &mut d,
        40,
        &p.initial_stack.load(Ordering::Relaxed).to_le_bytes(),
    );
    {
        let c = p.cred.lock();
        for (at, id) in [(48, c.ruid), (52, c.rgid), (56, c.euid), (60, c.egid), (64, c.suid), (68, c.sgid)] {
            put(&mut d, at, &id.to_le_bytes());
        }
    }
    {
        // sig_ignore, sig_pending (bit n - 1 for signal n).
        let s = p.signals.lock();
        let ignored = s
            .actions
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, a)| a.handler == crate::ker::signal::SIG_IGN)
            .fold(0 as crate::ker::signal::SigSet, |m, (n, _)| m | 1 << (n - 1));
        put(&mut d, 72, &crate::personality::qnx::set_out(ignored).to_le_bytes());
        put(&mut d, 88, &crate::personality::qnx::set_out(s.pending).to_le_bytes());
    }
    put(&mut d, 96, &(p.channel_list().len() as u32).to_le_bytes()); // num_chancons
    put(&mut d, 100, &(p.fds().len() as u32).to_le_bytes()); // num_fdcons
    put(&mut d, 104, &(threads.len() as u32).to_le_bytes()); // num_threads
    put(&mut d, 108, &(crate::ker::timer::timers_of(p).len() as u32).to_le_bytes());
    put(
        &mut d,
        112,
        &p.start_time.load(Ordering::Relaxed).to_le_bytes(),
    );
    // OsCraft doesn't tell user from system time: all is user time.
    put(&mut d, 120, &p.cpu_time().to_le_bytes()); // utime
    put(&mut d, 136, &p.child_cpu_time().to_le_bytes()); // cutime
    let prio = threads.first().map_or(0, |t| t.base_priority());
    d[152] = prio; // priority
    put(&mut d, 168, &p.pls.load(Ordering::Relaxed).to_le_bytes());
    d
}

/// Thread `t`'s `AARCH64_CPU_REGISTERS` (x0-x30, sp, elr, pstate) as it
/// left user mode: EBUSY while it runs; zeros for process 1's threads.
pub fn gregs(t: &Thread) -> Result<Vec<u8>, u32> {
    if t.process.pid == process::KERNEL_PID {
        return Ok(alloc::vec![0; GREG_SIZE]);
    }
    // SAFETY: read only while `t` is blocked (checked).
    let (frame, _, _) = unsafe { t.user_state() }.ok_or(crate::ker::errno::EBUSY)?;
    let mut out = Vec::with_capacity(GREG_SIZE);
    for r in frame.x.iter().chain([&frame.sp_el0, &frame.elr, &frame.spsr]) {
        out.extend_from_slice(&r.to_le_bytes());
    }
    Ok(out)
}

/// Thread `t`'s `AARCH64_FPU_REGISTERS`, as `gregs`.
pub fn fpregs(t: &Thread) -> Result<Vec<u8>, u32> {
    use crate::arch::context::QNX_FPU_SIZE;
    if t.process.pid == process::KERNEL_PID {
        return Ok(alloc::vec![0; QNX_FPU_SIZE]);
    }
    // SAFETY: read only while `t` is blocked (checked).
    let (_, _, fp) = unsafe { t.user_state() }.ok_or(crate::ker::errno::EBUSY)?;
    Ok(fp.to_qnx().to_vec())
}

/// `debug_thread64_t` of thread `t` of `p`.
fn thread_info(p: &Process, t: &Thread) -> [u8; THREAD_SIZE] {
    let mut d = [0u8; THREAD_SIZE];
    let state = t.state();
    put(&mut d, 0, &p.pid.to_le_bytes());
    put(&mut d, 4, &t.ptid.to_le_bytes());
    d[56] = t.priority();
    d[57] = t.base_priority();
    d[58] = crate::ker::policy_number(t.policy()) as u8;
    d[59] = state.qnx();
    // What a debugger needs: why it stopped, where, its TLS.
    let v = crate::ker::debug::thread_view(p, t);
    put(&mut d, 8, &v.flags.to_le_bytes());
    put(&mut d, 12, &v.why.to_le_bytes());
    put(&mut d, 14, &v.what.to_le_bytes());
    put(&mut d, 16, &v.ip.to_le_bytes());
    put(&mut d, 24, &v.sp.to_le_bytes());
    put(&mut d, 40, &v.tls.to_le_bytes());
    // The stack: the region its stack pointer is in, with the guard page
    // right below it (an inaccessible stack region), as procnto counts it.
    if v.sp != 0 {
        let mem = p.mem.lock();
        if let Some(r) = mem.regions().find(|r| r.start < v.sp && v.sp - 1 < r.end) {
            let guard = mem.regions().find(|g| {
                g.end == r.start && g.prot == crate::vm::map::PROT_NONE && g.flags & crate::vm::map::MAP_STACK != 0
            });
            let base = guard.map_or(r.start, |g| g.start);
            put(&mut d, 32, &base.to_le_bytes()); // stkbase
            put(&mut d, 48, &((r.end - base) as u32).to_le_bytes()); // stksize
        }
    }
    if let Some(info) = v.info {
        // `info` at 200; `__info32` at 88 for older readers.
        put(&mut d, 88, &crate::personality::qnx::view(info).to_qnx_bytes());
        put(&mut d, 200, &crate::personality::qnx::view(info).to_qnx_bytes());
    }
    put(&mut d, 62, &(t.last_cpu() as u16).to_le_bytes());
    put(&mut d, 68, &(-1i32).to_le_bytes()); // last_chid
    put(&mut d, 72, &crate::personality::qnx::set_out(t.sigmask.load(Ordering::Relaxed)).to_le_bytes());
    // `blocked`: the channel of a RECEIVE-blocked thread; the server and
    // channel of a SEND- or REPLY-blocked one.
    if let Some(ch) = t.ipc.blocked_on() {
        match state {
            State::Receive => put(&mut d, 136, &ch.chid.to_le_bytes()),
            State::Send | State::Reply => {
                put(&mut d, 140, &ch.owner.pid.to_le_bytes());
                put(&mut d, 148, &ch.chid.to_le_bytes());
            }
            _ => {}
        }
    }
    put(&mut d, 168, &t.start_time.to_le_bytes());
    put(&mut d, 176, &t.cpu_time().to_le_bytes()); // sutime
    d
}

/// The `debug_thread64_t` of the first thread of `p` whose ID is at
/// least `tid`.
pub fn thread_status(p: &Process, tid: u32) -> Option<[u8; THREAD_SIZE]> {
    let t = threads_by_id(p).into_iter().find(|t| t.ptid >= tid)?;
    Some(thread_info(p, &t))
}

/// The threads of `p` by QNX thread ID.
fn threads_by_id(p: &Process) -> Vec<ThreadRef> {
    let mut threads = sched::threads_of(p);
    threads.sort_by_key(|t| t.ptid);
    threads
}

fn u64_at(msg: &[u8], at: usize) -> u64 {
    msg.get(at..at + 8)
        .map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap()))
}

/// Handle one of the introspection messages from process `sender`, whose
/// reply buffer holds `room` bytes; replies to it, or returns the error to
/// reply with.
pub fn handle(rcvid: u64, sender: i32, ty: u16, msg: &[u8], room: usize) -> Result<(), u32> {
    match ty {
        // As procnto: pid 0 is no process here.
        PROC_GETNAME => {
            let pid = u64_at(msg, 8);
            if pid == 0 {
                return Err(ESRCH);
            }
            let p = target(sender, pid)?;
            let mut name = p.name.lock().clone().into_bytes();
            let len = name.len();
            name.push(0);
            // A name and NUL that don't fit: only the NUL, the status still
            // the name's length (review PROCMGR-17), not a truncated name
            // without its end.
            if name.len() > room {
                name.clear();
                name.push(0);
            }
            reply(rcvid, len, &name);
        }
        PROC_GET_ALL_PID => {
            let mut pids: Vec<u8> = Vec::new();
            for p in process::all() {
                pids.extend_from_slice(&pid64(p.pid).to_le_bytes());
            }
            reply(rcvid, pids.len(), &pids);
        }
        // struct proc_get_all_child_pid { type, zero, reserved, pid }: the
        // 64-bit pids of process `pid`'s children (0: the sender's); the
        // status is their size, whatever the reply buffer holds.
        PROC_GET_ALL_CHILD_PID => {
            let p = target(sender, u64_at(msg, 8))?;
            let mut pids: Vec<u8> = Vec::new();
            for c in process::all() {
                if c.ppid.load(core::sync::atomic::Ordering::Relaxed) == p.pid && c.pid != p.pid {
                    pids.extend_from_slice(&pid64(c.pid).to_le_bytes());
                }
            }
            reply(rcvid, pids.len(), &pids);
        }
        PROC_DEBUG => {
            if msg.len() < 16 {
                return Err(EINVAL);
            }
            let subtype = u16::from_le_bytes([msg[2], msg[3]]);
            let tid = u32::from_le_bytes(msg[4..8].try_into().unwrap());
            let p = target(sender, u64_at(msg, 8))?;
            match subtype {
                DEBUG_PROC_INFO => reply(rcvid, 0, &process_info(&p)),
                DEBUG_THREAD_STATUS => reply(rcvid, 0, &thread_status(&p, tid).ok_or(ESRCH)?),
                // Thread 0 is none (as procnto).
                DEBUG_GREGS | DEBUG_FPREGS if tid == 0 => return Err(EINVAL),
                DEBUG_GREGS | DEBUG_FPREGS => {
                    let t = sched::thread_of(&p, tid).ok_or(ESRCH)?;
                    let regs = if subtype == DEBUG_GREGS { gregs(&t)? } else { fpregs(&t)? };
                    reply(rcvid, regs.len(), &regs);
                }
                _ => return Err(ENOSYS),
            }
        }
        _ => return Err(ENOSYS),
    }
    Ok(())
}
