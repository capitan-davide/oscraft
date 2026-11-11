//! procnto's process manager messages (`<sys/procmsg.h>`), received on the
//! system-manager channel.
//!
//! `handle` dispatches them by type: fork and wait, umask, daemon,
//! `_PROC_GETSETID` (the IDs and their changes: `setuid()` & co.), process
//! groups and sessions, resources and abilities, events and values, server
//! monitors, introspection (`debug`), and the others named in its match.
//! Processes have credentials (`Process::cred`) and abilities; the boot
//! script's processes run as root and lead their own sessions.

use crate::arch::kercall::call;
use core::sync::atomic::Ordering;

use super::{debug, resmgr};
use alloc::vec::Vec;

use crate::ker::errno::{E2BIG, EACCES, EBADMSG, ECHILD, EEXIST, EINVAL, ENOENT, ENOMEM, ENOSPC, ENOSYS, ENOTSUP, ENXIO, EPERM, ESRCH};
use crate::ker::number::*;
use crate::ker::ability;
use crate::ker::process::Process;
use crate::ker::process::{self, KERNEL_PID};
use crate::ker::signal;
use crate::kprintln;
use crate::sched;
use crate::sync::IrqSpinLock;
use crate::vm::map;

pub const PROC_WAIT: u16 = 0x0011;
pub const PROC_FORK: u16 = 0x0012;
pub const PROC_GETSETID: u16 = 0x0013;
pub const PROC_SETPGID: u16 = 0x0014;
/// `_PROC_GETPID64`: { type, zero, pid }; the reply is a `pid64_t`.
const PROC_GETPID64: u16 = 0x0022;
pub const PROC_UMASK: u16 = 0x0015;
pub const PROC_DAEMON: u16 = 0x0018;
pub const PROC_RESOURCE: u16 = 0x001a;
pub const PROC_MAC: u16 = 0x0020;
/// `_PROC_TIMER_TOLERANCE` (`struct _proc_timer_tolerance` { type,
/// subtype (`_GET` 0, `_SET` 1), pid, tolerance }).
pub const PROC_TIMER_TOLERANCE: u16 = 0x001e;
const MAC_POLICY_ID: u16 = 1;
/// `_FORK_ASPACE`: the child gets its own address space.
const FORK_ASPACE: u32 = 0x1;
/// OsCraft: in a FreeBSD child's fork reply, set above its ID.
const FORK_CHILD: u64 = 1 << 30;
// _PROC_GETSETID subtypes
const PROC_ID_GETID: u16 = 0;
const PROC_ID_SETUID: u16 = 1;
const PROC_ID_SETGID: u16 = 2;
const PROC_ID_SETEUID: u16 = 3;
const PROC_ID_SETEGID: u16 = 4;
const PROC_ID_SETREUID: u16 = 5;
const PROC_ID_SETREGID: u16 = 6;
const PROC_ID_SETGROUPS: u16 = 7;
const PROC_ID_GETID_NO_CRED: u16 = 8;
/// `struct _proc_getsetid_reply`: zero, pgrp, ppid, sid, orig_ppid,
/// reserved[2], then `struct _cred_info` with all the supplementary
/// groups (as procnto, whatever `ngroups` the request has);
/// `_PROC_ID_GETID_NO_CRED` stops before the credentials.
const CRED_AT: usize = 28;

/// `_PROC_GETSETID` (`struct _proc_getsetid`: type, subtype, pid, eid,
/// rid, ngroups, then the groups): get a process's IDs, or set the
/// caller's (`setuid()` & co., POSIX rules: a process with effective user
/// ID 0 sets any, others only to their real or saved ones; -1 leaves one
/// unchanged where -1 means that). A query's reply is the IDs; a change
/// has none (as procnto).
fn getsetid(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    let id_at = |o: usize| {
        msg.get(o..o + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or(EINVAL)
    };
    if subtype != PROC_ID_GETID && subtype != PROC_ID_GETID_NO_CRED {
        let me = process::lookup(sender).ok_or(ESRCH)?;
        // Privilege (to set IDs other than one's own): the SETUID or
        // SETGID ability, for the ID asked for.
        let euid = me.cred.lock().euid;
        let may = |gid: bool, id: u32| {
            let aid = if gid { ability::SETGID } else { ability::SETUID };
            ability::allowed_as(&me, euid, aid, Some(id as u64))
        };
        set_ids(&mut me.cred.lock(), subtype, id_at(8)?, id_at(12)?, may, || {
            let n = id_at(16)? as usize;
            if n > process::NGROUPS_MAX {
                return Err(EINVAL);
            }
            (0..n).map(|i| id_at(20 + 4 * i)).collect()
        })?;
        reply_status(rcvid, 0, &[]);
        return Ok(());
    }
    let pid = match msg.get(4..8) {
        Some(b) => i32::from_le_bytes(b.try_into().unwrap()),
        None => 0,
    };
    let p = process::lookup(if pid == 0 { sender } else { pid }).ok_or(ESRCH)?;
    let mut r = alloc::vec![0u8; CRED_AT];
    r[4..8].copy_from_slice(&p.pgid().to_le_bytes());
    r[8..12].copy_from_slice(&p.ppid.load(Ordering::Relaxed).to_le_bytes());
    r[12..16].copy_from_slice(&p.sid().to_le_bytes());
    r[16..20].copy_from_slice(&KERNEL_PID.to_le_bytes()); // orig_ppid
    if subtype != PROC_ID_GETID_NO_CRED {
        r.extend(p.cred.lock().info(usize::MAX));
    }
    reply_status(rcvid, 0, &r);
    Ok(())
}

/// Change `c` as `_PROC_ID_SET*` subtype `subtype` asks, with IDs `eid`
/// and `rid` (`u32::MAX`, -1: unchanged) or the groups from `groups`.
fn set_ids(
    c: &mut process::Cred,
    subtype: u16,
    eid: u32,
    rid: u32,
    may: impl Fn(bool, u32) -> bool,
    groups: impl FnOnce() -> Result<Vec<u32>, u32>,
) -> Result<(), u32> {
    const NONE: u32 = u32::MAX;
    let gid = matches!(subtype, PROC_ID_SETGID | PROC_ID_SETEGID | PROC_ID_SETREGID | PROC_ID_SETGROUPS);
    let privileged = |id: u32| may(gid, id);
    // As procnto: -1 is no ID for these (EINVAL).
    if matches!(subtype, PROC_ID_SETUID | PROC_ID_SETGID | PROC_ID_SETEUID | PROC_ID_SETEGID) && eid == NONE {
        return Err(EINVAL);
    }
    match subtype {
        // setuid(): all three if privileged, else the effective one to the
        // real or saved one.
        PROC_ID_SETUID | PROC_ID_SETGID => {
            let uid = subtype == PROC_ID_SETUID;
            let (r, e, s) = if uid {
                (&mut c.ruid, &mut c.euid, &mut c.suid)
            } else {
                (&mut c.rgid, &mut c.egid, &mut c.sgid)
            };
            if privileged(eid) {
                (*r, *e, *s) = (eid, eid, eid);
            } else if eid == *r || eid == *s {
                *e = eid;
            } else {
                return Err(EPERM);
            }
        }
        PROC_ID_SETEUID | PROC_ID_SETEGID => {
            let (r, e, s) = if subtype == PROC_ID_SETEUID {
                (c.ruid, &mut c.euid, c.suid)
            } else {
                (c.rgid, &mut c.egid, c.sgid)
            };
            if !(privileged(eid) || eid == r || eid == s) {
                return Err(EPERM);
            }
            *e = eid;
        }
        // setreuid(): the real one to the real or effective one, the
        // effective one to any of the three; the saved one follows the
        // effective one when the real one is set or the effective one
        // becomes other than the real one.
        PROC_ID_SETREUID | PROC_ID_SETREGID => {
            let (r, e, s) = if subtype == PROC_ID_SETREUID {
                (&mut c.ruid, &mut c.euid, &mut c.suid)
            } else {
                (&mut c.rgid, &mut c.egid, &mut c.sgid)
            };
            let ok_r = rid == NONE || privileged(rid) || rid == *r || rid == *e;
            let ok_e = eid == NONE || privileged(eid) || eid == *r || eid == *e || eid == *s;
            if !(ok_r && ok_e) {
                return Err(EPERM);
            }
            let old_r = *r;
            if rid != NONE {
                *r = rid;
            }
            if eid != NONE {
                *e = eid;
            }
            if rid != NONE || (eid != NONE && eid != old_r) {
                *s = *e;
            }
        }
        // As procnto: the count is checked first (EINVAL); without the
        // privilege only the same groups again.
        PROC_ID_SETGROUPS => {
            let new = groups()?;
            if new != c.groups && !privileged(c.egid) {
                return Err(EPERM);
            }
            c.groups = new;
        }
        _ => return Err(ENOSYS),
    }
    Ok(())
}

/// `_PROC_FORK` (`struct _proc_fork` { type, zero, flags, frame }): a copy
/// of the sending process whose one thread is a copy of the sending thread,
/// returning 0 from the same `MsgSend()`; the parent's returns the child's
/// pid. As in procnto's `procmgr_fork`, `_FORK_ASPACE` (a separate address
/// space, i.e. a real fork) is required. `frame` is 0 from libc's fork().
// ANCHOR: procmgr_fork
fn fork(rcvid: u64, sender: i32, tid: u32, msg: &[u8]) -> Result<(), u32> {
    let flags = msg
        .get(4..8)
        .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
    if flags & FORK_ASPACE == 0 {
        return Err(EINVAL);
    }
    let parent = process::lookup(sender).ok_or(ESRCH)?;
    let thread = sched::thread_of(&parent, tid).ok_or(ESRCH)?;
    // SAFETY: the thread is REPLY-blocked on us until we reply.
    let (frame, tp, fp) = unsafe { thread.user_state() }.ok_or(EINVAL)?;
    let child = process::fork(&parent).ok_or(ENOMEM)?;
    // The parent's fork() returns once the child has inherited the
    // descriptors (`fork_done`): registered before the child can run
    // (PROCMGR-3, as for a spawn).
    FORKS.lock().push((child.pid, rcvid));
    if let Err(e) = fork_into(&parent, &child, &thread, frame, tp, fp) {
        // Undone whole: its memory (what the copy got to), its open
        // files, its pid, its registration. The parent sees only the
        // error.
        forget_startup(child.pid);
        resmgr::process_exited(child.pid);
        process::remove(child.pid);
        return Err(e);
    }
    super::procevent::trigger(super::procevent::PROCESS_CREATE, child.pid as u32);
    Ok(())
}
// ANCHOR_END: procmgr_fork

/// The child's memory, open files and first thread, from the parent's.
fn fork_into(
    parent: &Process,
    child: &Process,
    thread: &sched::Thread,
    mut frame: crate::arch::exception::TrapFrame,
    tp: u64,
    fp: crate::arch::context::FpState,
) -> Result<(), u32> {
    map::fork(parent, child)?;
    resmgr::fork(parent.pid, child.pid);
    if child.personality().fork_uses_tls() {
        fix_child_tls(child, tp)?;
        frame.x[0] = 0; // the child's MsgSend() returns 0
    } else {
        // A FreeBSD process has no thread-local block for the kernel to
        // put the child's ID in: the child's MsgSend() returns the ID
        // with `FORK_CHILD` set, and libsys's fork() keeps it and
        // returns 0 (personality::native).
        frame.x[0] = child.pid as u64 | FORK_CHILD;
    }
    let sigmask = thread.sigmask.load(Ordering::Relaxed);
    // Its first kernel call tells the other servers about its connections.
    let dups: Vec<[i32; 5]> = parent
        .external_connections()
        .into_iter()
        .map(|(coid, chid, scoid)| [coid, parent.pid, chid, scoid, coid])
        .collect();
    sched::spawn_user_frame_init(
        child,
        "forked",
        thread.base_priority(),
        frame,
        tp,
        fp,
        sigmask,
        1, // the child's only thread is its thread 1
        |t| *t.pending_dups.lock() = Some(dups),
    )
    .ok_or(ENOMEM)?;
    Ok(())
}

/// Answer `rcvid` with `child`'s pid once the child has told the servers
/// about its inherited descriptors (as for a fork).
pub fn reply_after_startup(child: i32, rcvid: u64) {
    FORKS.lock().push((child, rcvid));
}

/// A registration made before a start that then failed: taken back (the
/// caller answers the request with the error).
pub fn forget_startup(child: i32) {
    FORKS.lock().retain(|&(c, _)| c != child);
}

/// A thread held in a fork or spawn (`rcvid`, waiting for its child's
/// start-up) is ending with its process's exec: it is answered now, with
/// EINTR, and the child, which has not run yet, is abandoned: killed
/// before it runs a user instruction, with no zombie and no SIGCHLD.
/// procnto's exec completes and the child never runs (measured, review
/// PROCMGR-26); here the start-up's pulse would come to the thread that is
/// waiting for the held one to go, which hung procnto. Returns whether
/// `rcvid` was held so.
pub fn abandon_startup(rcvid: u64) -> bool {
    let held = {
        let mut forks = FORKS.lock();
        let i = forks.iter().position(|&(_, r)| r == rcvid);
        i.map(|i| forks.remove(i))
    };
    let Some((child, _)) = held else {
        return false;
    };
    if let Some(c) = process::lookup(child) {
        c.nozombie.store(true, Ordering::Relaxed);
        c.ppid.store(KERNEL_PID, Ordering::Relaxed);
        signal::post(&c, signal::SIGKILL, signal::SigInfo::default());
    }
    let _ = call(__KER_MSG_ERROR, [rcvid, crate::ker::errno::EINTR as u64, 0, 0, 0, 0]);
    true
}

/// Forks whose parent waits for the child's start-up: (child pid, rcvid).
static FORKS: IrqSpinLock<Vec<(i32, u64)>> = IrqSpinLock::new(Vec::new());

/// The forked or spawned child `pid` has inherited its descriptors:
/// answer its parent, with `err` if the child failed to start.
pub fn fork_done(pid: i32, err: u32) {
    let waiting = {
        let mut forks = FORKS.lock();
        let i = forks.iter().position(|&(c, _)| c == pid);
        i.map(|i| forks.remove(i))
    };
    if let Some((_, rcvid)) = waiting {
        if err != 0 {
            let _ = call(__KER_MSG_ERROR, [rcvid, err as u64, 0, 0, 0, 0]);
            return;
        }
        let _ = call(__KER_MSG_REPLYV, [rcvid, pid as u64, 0, 0, 0, 0]);
    }
}

/// The forked thread's TLS is a copy of its parent's: make it describe
/// the child's thread 1 (`__pid` at 0x20, `__tid` at 0x24, `__owner` at
/// 0x28, the value libc stores in the mutexes it locks). The TLS address
/// is word 1 of the ELF TLS block the thread pointer points at.
fn fix_child_tls(child: &Process, tp: u64) -> Result<(), u32> {
    use crate::ker::umem::{self, Space};
    // SAFETY: `child` is borrowed for the whole call.
    let space = unsafe { Space::user(child) };
    let mut b = [0u8; 8];
    umem::read(space, tp + 8, &mut b)?;
    let tls = u64::from_le_bytes(b);
    let mut ids = [0u8; 12];
    ids[0..4].copy_from_slice(&child.pid.to_le_bytes());
    ids[4..8].copy_from_slice(&1u32.to_le_bytes());
    ids[8..12].copy_from_slice(&(((child.pid as u32 & 0x7fff) << 16) | 1).to_le_bytes());
    umem::write(space, tls + 0x20, &ids)
}

/// `_PROC_SETPGID` (`struct _proc_setpgid` { type, zero, pid, pgid }),
/// with the rules of procnto's `procmgr_setpgid`: the target is the sender
/// (pid 0) or one of its children; a session leader cannot move; pgid 0
/// means the target's pid; any other group must exist in the target's
/// session.
fn setpgid(rcvid: u64, sender: i32, msg: &[u8]) -> Result<(), u32> {
    let int = |o: usize| {
        msg.get(o..o + 4)
            .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
    };
    let (pid, pgid) = (int(4).ok_or(EINVAL)?, int(8).ok_or(EINVAL)?);
    if pgid < 0 {
        return Err(EINVAL);
    }
    let me = process::lookup(sender).ok_or(ESRCH)?;
    if !ability::allowed(&me, ability::PGRP, Some(pgid as u64)) {
        return Err(EPERM);
    }
    let target = match pid {
        0 => process::lookup(sender),
        pid => process::lookup(pid)
            .filter(|p| p.pid == sender || p.ppid.load(Ordering::Relaxed) == sender),
    }
    .ok_or(ESRCH)?;
    // A child running a program of its own (spawned, or it has exec'ed)
    // stays where it is: EACCES (POSIX; procnto, measured).
    if target.pid != sender && target.execed.load(Ordering::Relaxed) {
        return Err(EACCES);
    }
    if target.sid() == target.pid {
        return Err(EPERM);
    }
    let pgid = if pgid == 0 { target.pid } else { pgid };
    if pgid != target.pid
        && !process::all()
            .iter()
            .any(|p| p.pgid() == pgid && p.sid() == target.sid())
    {
        return Err(EPERM);
    }
    target.set_pgid(pgid);
    let _ = call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]);
    Ok(())
}

// _PROC_WAIT (`struct _proc_wait` { type, idtype, options, id })
const P_ALL: i16 = 0;
const P_PID: i16 = 1;
const P_PGID: i16 = 2;
const WEXITED: i32 = 0x1;
const WUNTRACED: i32 = 0x4;
const WCONTINUED: i32 = 0x8;
const WNOHANG: i32 = 0x40;
const WNOWAIT: i32 = 0x80;
/// The options procnto accepts (`procmgr_wait`).
const WAIT_OPTIONS: i32 = 0xcf;

/// A held `_PROC_WAIT`.
#[derive(Clone, Copy)]
struct Wait {
    rcvid: u64,
    parent: i32,
    idtype: i16,
    id: i32,
    options: i32,
}

static WAITS: IrqSpinLock<Vec<Wait>> = IrqSpinLock::new(Vec::new());

/// The children of `w.parent` that `w` is about.
fn wait_candidates(w: &Wait) -> Vec<process::ProcessRef> {
    process::all()
        .into_iter()
        .filter(|p| p.pid != KERNEL_PID && p.ppid.load(Ordering::Relaxed) == w.parent)
        .filter(|p| match w.idtype {
            P_ALL => true,
            P_PID => p.pid == w.id,
            P_PGID => p.pgid() == w.id,
            _ => false,
        })
        .collect()
}

/// Reply to wait `w` with `info`, about `child`. As procnto
/// (`procmgr_wait_check_internal`), the child's SIGCHLD that the parent
/// hasn't taken yet goes: a parent whose wait a child's end answers sees
/// no SIGCHLD for it. A child severed from its parent
/// that runs on (`detach`) keeps it: procnto's parent gets that SIGCHLD
/// whether its wait comes before or after (measured).
fn answer_wait(w: &Wait, child: &Process, info: signal::SigInfo) {
    let severed = child.nozombie.load(Ordering::Relaxed) && !child.is_zombie();
    if !severed && let Some(parent) = process::lookup(w.parent) {
        parent.signals.lock().remove_from(signal::SIGCHLD, child.pid);
    }
    // In the waiter's numbers and layout.
    let waiter_qnx = process::lookup(w.parent)
        .is_some_and(|p| p.personality() == crate::ker::process::Personality::Qnx);
    let (qnx, bsd);
    let b: &[u8] = if waiter_qnx {
        qnx = crate::personality::qnx::view(info).to_qnx_bytes();
        &qnx
    } else {
        bsd = info.to_bytes();
        &bsd
    };
    let _ = call(__KER_MSG_REPLYV, [w.rcvid, 0, b.as_ptr() as u64, (-(b.len() as i64)) as u64, 0, 0]);
}

/// Answer `w` if one of its children has ended: reply with that child's
/// SIGCHLD `siginfo_t` and, unless WNOWAIT, forget the child. Returns
/// whether it was answered.
fn try_wait(w: &Wait) -> bool {
    use crate::ker::debug::JobEvent;
    let candidates = wait_candidates(w);
    let Some(child) = candidates
        .iter()
        // A zombie, or a severed child that runs on; not a child that ended
        // severed (or with SIGCHLD ignored), which procnto removes.
        .find(|p| {
            !p.unwaited.load(Ordering::Acquire)
                && (p.is_zombie() != p.nozombie.load(Ordering::Relaxed))
                && w.options & WEXITED != 0
        })
    else {
        // A child stopped (WUNTRACED) or continued (WCONTINUED) since the
        // last wait for it: CLD_STOPPED with the signal, CLD_CONTINUED.
        let job = candidates.iter().find_map(|p| {
            let st = p.debug.lock();
            match st.job_event {
                Some(JobEvent::Stopped(sig)) if w.options & WUNTRACED != 0 => {
                    Some((p.clone(), signal::CLD_STOPPED, sig as i32))
                }
                Some(JobEvent::Continued) if w.options & WCONTINUED != 0 => {
                    Some((p.clone(), signal::CLD_CONTINUED, signal::SIGCONT as i32))
                }
                _ => None,
            }
        });
        let Some((child, code, status)) = job else {
            return false;
        };
        let info = signal::SigInfo {
            signo: signal::SIGCHLD as i32,
            code,
            pid: child.pid,
            status,
            ..Default::default()
        };
        answer_wait(w, &child, info);
        if w.options & WNOWAIT == 0 {
            child.debug.lock().job_event = None;
        }
        return true;
    };
    let child = child.clone();
    // A severed child that still runs ended with status 0 for its parent.
    let (code, status) = match child.is_zombie() {
        true => child.exit_status(),
        false => (signal::CLD_EXITED, 0),
    };
    let info = signal::SigInfo {
        signo: signal::SIGCHLD as i32,
        code,
        pid: child.pid,
        status,
        ..Default::default()
    };
    answer_wait(w, &child, info);
    if w.options & WNOWAIT == 0 {
        if child.is_zombie() {
            process::remove(child.pid);
        } else {
            child.ppid.store(KERNEL_PID, Ordering::Relaxed);
        }
    }
    true
}

/// Client `rcvid` asks to leave its request (a signal for it, procnto's
/// channel having `_NTO_CHF_UNBLOCK`): a held wait ends with EINTR, unless
/// a child it takes has ended (the SIGCHLD comes as the child ends,
/// before procnto has heard of it): then procnto answers it with that. Other
/// requests are answered anyway, soon. Only while it still asks: the
/// pulse may come after an answer and a new request.
pub fn unblocked(rcvid: u64) {
    let Some(t) = u32::try_from(rcvid).ok().and_then(crate::sched::thread) else {
        return;
    };
    if !t.unblock_requested.load(Ordering::Relaxed) || t.state() != crate::sched::State::Reply {
        return;
    }
    let held = {
        let mut waits = WAITS.lock();
        let i = waits.iter().position(|w| w.rcvid == rcvid);
        i.map(|i| waits.remove(i))
    };
    let Some(w) = held else {
        return;
    };
    interrupt_wait(w);
}

/// A held wait whose client asked to leave: EINTR, unless a child it can
/// take has ended: procnto answers it as it handles the end
/// (`process_exited`), which may not have come yet.
fn interrupt_wait(w: Wait) {
    let ended = wait_candidates(&w)
        .iter()
        .any(|p| {
            p.is_zombie()
                && !p.nozombie.load(Ordering::Relaxed)
                && !p.unwaited.load(Ordering::Acquire)
                && w.options & WEXITED != 0
        });
    if ended {
        WAITS.lock().push(w);
        return;
    }
    let _ = call(__KER_MSG_ERROR, [w.rcvid, crate::ker::errno::EINTR as u64, 0, 0, 0, 0]);
}

/// A child stopped or continued: answer the waits that settles.
pub fn job_changed() {
    recheck_waits();
}

/// Answer the held waits that a child's end or departure settles: with
/// the child that ended, or ECHILD if there is no child left to wait for.
fn recheck_waits() {
    let waits: Vec<Wait> = WAITS.lock().clone();
    for w in waits {
        if try_wait(&w) {
            WAITS.lock().retain(|x| x.rcvid != w.rcvid);
        } else if wait_candidates(&w).is_empty() {
            WAITS.lock().retain(|x| x.rcvid != w.rcvid);
            let _ = call(__KER_MSG_ERROR, [w.rcvid, ECHILD as u64, 0, 0, 0, 0]);
        }
    }
}

/// Sever `p` from its parent (procnto's `procmgr_nozombie`): to the
/// parent it has exited with status 0 (its waits see that, and it gets a
/// SIGCHLD), while it runs on and will leave no zombie. The waits it
/// settles are answered, and the SIGCHLD sent, by procnto's system-manager
/// thread, which answers all waits (`PULSE_DETACHED`, `detached`): run
/// from the worker thread
/// (an exec of a program on disk), a second `recheck_waits` beside that
/// thread's could answer one wait twice, or miss one being registered.
pub fn detach(p: &Process) {
    if p.nozombie.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Some(sysmgr) = process::KERNEL.channel(process::SYSMGR_CHID) {
        let prio = sched::current().base_priority();
        // The parent too: a wait that answers for the child first hands
        // it over to procnto.
        let ppid = p.ppid.load(Ordering::Relaxed);
        let value = (p.pid as u32 as u64) | (ppid as u32 as u64) << 32;
        crate::ker::msg::channel_pulse(&sysmgr, prio, process::PULSE_DETACHED, value, 0);
    }
}

/// `PULSE_DETACHED` (`detach`), on the system-manager thread: answer the
/// waits that process `pid`'s departure from its parent `ppid` settles,
/// then send the parent its SIGCHLD. In that order: sent first, the signal
/// could take a parent blocked in a wait out of it (EINTR) before the
/// wait is answered.
pub fn detached(pid: i32, ppid: i32) {
    recheck_waits();
    if ppid != KERNEL_PID
        && let Some(parent) = process::lookup(ppid)
    {
        let info = signal::SigInfo {
            code: signal::CLD_EXITED,
            pid,
            ..Default::default()
        };
        signal::post(&parent, signal::SIGCHLD, info);
    }
}

/// `_PROC_WAIT` (`waitid()`, `waitpid()`): as procnto's `procmgr_wait`,
/// options beyond WEXITED, WTRAPPED, WUNTRACED, WCONTINUED, WNOHANG and
/// WNOWAIT are EINVAL, no matching child is ECHILD, and with WNOHANG and
/// nothing to report the reply is an all-zero `siginfo_t`. Otherwise the
/// request is held until a child ends.
fn wait(rcvid: u64, sender: i32, msg: &[u8]) -> Result<(), u32> {
    if msg.len() < 12 {
        return Err(EINVAL);
    }
    let w = Wait {
        rcvid,
        parent: sender,
        idtype: i16::from_le_bytes([msg[2], msg[3]]),
        options: i32::from_le_bytes(msg[4..8].try_into().unwrap()),
        id: i32::from_le_bytes(msg[8..12].try_into().unwrap()),
    };
    // An unknown idtype matches no child (procnto: ECHILD).
    if w.options & !WAIT_OPTIONS != 0 {
        return Err(EINVAL);
    }
    if wait_candidates(&w).is_empty() {
        return Err(ECHILD);
    }
    if try_wait(&w) {
        return Ok(());
    }
    if w.options & WNOHANG != 0 {
        let b = [0u8; signal::SIGINFO_SIZE];
        let _ = call(
            __KER_MSG_REPLYV,
            [
                rcvid,
                0,
                b.as_ptr() as u64,
                (-(b.len() as i64)) as u64,
                0,
                0,
            ],
        );
        return Ok(());
    }
    // A signal came while the request waited to be received (the kernel
    // leaves requests to procnto queued, see `ker::msg::unblock`): as if
    // it came now.
    let asked = u32::try_from(rcvid)
        .ok()
        .and_then(crate::sched::thread)
        .is_some_and(|t| t.unblock_requested.load(Ordering::Relaxed));
    if asked {
        interrupt_wait(w);
        return Ok(());
    }
    WAITS.lock().push(w);
    Ok(())
}

/// Process `pid` has ended (its last thread's pulse): answer the waits it
/// satisfies, forget its open files, and give its children to procnto.
pub fn process_exited(pid: i32) {
    // A debugger that ends lets its processes go.
    for p in process::all() {
        if p.debug.lock().debugger.is_some_and(|(d, _)| d == pid) {
            crate::ker::debug::detach(&p);
        }
    }
    let daemon = process::lookup(pid).is_some_and(|p| p.sid() == KERNEL_PID);
    super::procevent::process_exited(pid, daemon);
    crate::ker::servmon::process_exited(pid);
    // Its memory and page tables: no thread runs on them any more.
    if let Some(p) = process::lookup(pid) {
        map::clear(&p);
        // A copy that walked its tables before they emptied may still read
        // them (review IPC-2).
        map::wait_copies(&p);
        p.aspace().free_tables();
    }
    // A logger that ends: tracing stops, its buffers are freed (procnto's
    // `cleanup`), its view of them gone with its memory.
    crate::ker::trace::logger_gone(pid);
    resmgr::process_exited(pid);
    super::shmhandle::process_exited(pid);
    crate::ker::mqueue::process_exited(pid);
    crate::ker::kpipe::process_exited(pid);
    super::pathmgr::process_exited(pid);
    super::rsrcdb::process_exited(pid);
    super::session::process_exited(pid);
    if process::lookup(pid).is_some_and(|p| p.nozombie.load(Ordering::Relaxed)) {
        process::remove(pid);
    }
    recheck_waits();
    // Its children become procnto's. procnto waits for none of its
    // children: they leave no zombies.
    for q in process::all() {
        if q.ppid.load(Ordering::Relaxed) == pid {
            q.ppid.store(KERNEL_PID, Ordering::Relaxed);
            if q.is_zombie() {
                process::remove(q.pid);
            }
        }
    }
    if process::lookup(pid).is_some_and(|p| p.ppid.load(Ordering::Relaxed) == KERNEL_PID) {
        process::remove(pid);
    }
}

// _PROC_RESOURCE subtypes
const RESOURCE_USAGE: u16 = 0;
const RESOURCE_GETLIMIT: u16 = 1;
const RESOURCE_SETLIMIT: u16 = 2;
/// `RLIM_NLIMITS`, `RLIM_INFINITY` (`<sys/resource.h>`).
const RLIM_NLIMITS: u32 = 19;
const RLIM_INFINITY: u64 = u64::MAX - 2;
const RLIMIT_NOFILE: u32 = process::RLIMIT_NOFILE;
const RLIMIT_NPROC: u32 = 8;

/// Process `p`'s limit on `resource`: (current, maximum). Not enforced
/// yet; the defaults agree with `sysconf()`.
fn limit(p: &Process, resource: u32) -> (u64, u64) {
    if let Some(&l) = p.limits.lock().get(&resource) {
        return l;
    }
    match resource {
        RLIMIT_NOFILE | RLIMIT_NPROC => (process::DEFAULT_NOFILE, process::DEFAULT_NOFILE),
        _ => (RLIM_INFINITY, RLIM_INFINITY),
    }
}
const RESOURCE_SETABILITIES: u16 = 3;
const RESOURCE_LOOKUPABILITY: u16 = 4;
const RESOURCE_CREATEABILITY: u16 = 5;
const RESOURCE_ABILITY_NAME: u16 = 6;
/// `sizeof(struct rusage)` (64-bit).
const RUSAGE_SIZE: usize = 144;
/// Custom abilities get IDs after QNX's own (`PROCMGR_AID_MAX` is 75), in
/// the order they are made (as procnto's `nto_ability_create`).
pub const FIRST_CUSTOM_ABILITY: u32 = 76;
/// `PROCMGR_ABILITY_NAME_MAX`, with the NUL.
const ABILITY_NAME_MAX: usize = 128;
/// A custom ability's default domains (procnto's `ABLE_ALLOW_ROOT`,
/// `ABLE_ALLOW_NONROOT`), or, made by a lookup and not created yet, none
/// until it is (`PENDING`).
const ABLE_ROOT: u16 = 1;
const ABLE_NONROOT: u16 = 2;
const ABLE_PENDING: u16 = 0x600;

/// The IDs of the custom abilities made so far.
pub fn custom_abilities() -> impl Iterator<Item = u32> {
    FIRST_CUSTOM_ABILITY..FIRST_CUSTOM_ABILITY + customs().len() as u32
}

/// Does domain `dom` (0 root, 1 non-root) have custom ability `aid` by
/// default? None: no such ability.
pub fn custom_default(aid: u32, dom: usize) -> Option<bool> {
    let i = aid.checked_sub(FIRST_CUSTOM_ABILITY)? as usize;
    customs().get(i).map(|(_, flags)| flags & (1 << dom) != 0)
}

/// Custom abilities by ID order: name and default domains.
static ABILITIES: IrqSpinLock<Vec<(alloc::string::String, u16)>> = IrqSpinLock::new(Vec::new());

/// The custom abilities, starting with those procnto makes for QNX's
/// iofunc layer (`message_init`): `iofunc/chown`, `iofunc/read` and
/// `iofunc/dup` root's, `iofunc/exec` nobody's.
fn customs() -> crate::sync::IrqSpinGuard<'static, Vec<(alloc::string::String, u16)>> {
    let mut a = ABILITIES.lock();
    if a.is_empty() {
        for (name, flags) in [
            ("iofunc/chown", ABLE_ROOT),
            ("iofunc/read", ABLE_ROOT),
            ("iofunc/dup", ABLE_ROOT),
            ("iofunc/exec", 0),
        ] {
            a.push((name.into(), flags));
        }
    }
    a
}

/// Look up (`create` false) or create custom ability `name` for `caller`,
/// as procnto's `nto_ability_create`: `flags` may hold only
/// `PROCMGR_ADN_ROOT` and `PROCMGR_ADN_NONROOT` (EINVAL), the domains
/// that have it by default. A new name takes `PROCMGR_AID_ABLE_CREATE`
/// (EPERM); a lookup makes it with no domains until a creation gives them
/// (with ABLE_CREATE too). Creating an ability again with flags lacking
/// one of its domains is EEXIST; otherwise it is the same ID.
fn ability_create(caller: &Process, name: &str, flags: u32, create: bool) -> Result<u32, u32> {
    const ADN_ROOT: u32 = 0x1000_0000;
    const ADN_NONROOT: u32 = 0x2000_0000;
    if flags & !(ADN_ROOT | ADN_NONROOT) != 0 {
        return Err(EINVAL);
    }
    let doms = if flags & ADN_ROOT != 0 { ABLE_ROOT } else { 0 }
        | if flags & ADN_NONROOT != 0 { ABLE_NONROOT } else { 0 };
    let may_create = ability::allowed(caller, ability::ABLE_CREATE, None);
    let mut a = customs();
    if let Some(i) = a.iter().position(|(n, _)| n == name) {
        if create {
            let old = &mut a[i].1;
            if *old & ABLE_PENDING != 0 {
                if !may_create {
                    return Err(EPERM);
                }
                *old = doms;
            } else if *old & !doms != 0 {
                return Err(EEXIST);
            }
        }
        return Ok(FIRST_CUSTOM_ABILITY + i as u32);
    }
    if !may_create {
        return Err(EPERM);
    }
    let id = FIRST_CUSTOM_ABILITY + a.len() as u32;
    if id >= 0xffff {
        return Err(ENOSPC);
    }
    a.push((name.into(), if create { doms } else { ABLE_PENDING }));
    Ok(id)
}

fn reply_status(rcvid: u64, status: u64, data: &[u8]) {
    let _ = call(
        __KER_MSG_REPLYV,
        [
            rcvid,
            status,
            data.as_ptr() as u64,
            (-(data.len() as i64)) as u64,
            0,
            0,
        ],
    );
}

/// `_PROC_RESOURCE`. Abilities (QNX's privileges): set as `ability` says;
/// a custom ability's lookup or creation gives it an ID (the reply
/// status), the same for the same name. Resource usage is all zero
/// so far. Resource limits are kept (`getrlimit()`, `setrlimit()`) and
/// inherited, not enforced yet.
fn resource(rcvid: u64, sender: i32, subtype: u16, msg: &[u8], room: usize) -> Result<(), u32> {
    match subtype {
        RESOURCE_USAGE => reply_status(rcvid, 0, &[0; RUSAGE_SIZE]),
        // struct _proc_resource_getlimit { type, subtype, pid, count,
        // reserved, resource[count] }; the reply: struct rlimit[count].
        RESOURCE_GETLIMIT | RESOURCE_SETLIMIT => {
            let u32_at = |o: usize| {
                msg.get(o..o + 4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            };
            let pid = u32_at(4).ok_or(EINVAL)? as i32;
            let p = process::lookup(if pid == 0 { sender } else { pid }).ok_or(ESRCH)?;
            let count = u32_at(8).ok_or(EINVAL)? as usize;
            // As procnto: one limit per message (libc sends one).
            if count != 1 {
                return Err(ENOTSUP);
            }
            if subtype == RESOURCE_GETLIMIT {
                let mut out = Vec::with_capacity(16 * count);
                for i in 0..count {
                    let r = u32_at(16 + 4 * i).filter(|&r| r < RLIM_NLIMITS).ok_or(EINVAL)?;
                    let (cur, max) = limit(&p, r);
                    out.extend_from_slice(&cur.to_le_bytes());
                    out.extend_from_slice(&max.to_le_bytes());
                }
                reply_status(rcvid, 0, &out);
                return Ok(());
            }
            // struct _proc_resource_entry { resource, reserved, rlimit } each.
            let mut set = Vec::with_capacity(count);
            for i in 0..count {
                let e = msg.get(16 + 24 * i..16 + 24 * (i + 1)).ok_or(EINVAL)?;
                let r = u32::from_le_bytes(e[0..4].try_into().unwrap());
                let cur = u64::from_le_bytes(e[8..16].try_into().unwrap());
                let max = u64::from_le_bytes(e[16..24].try_into().unwrap());
                if r >= RLIM_NLIMITS || cur > max {
                    return Err(EINVAL);
                }
                // Raising a hard limit takes RLIMIT (for the new limit).
                if max > limit(&p, r).1
                    && !process::lookup(sender).is_some_and(|me| ability::allowed(&me, ability::RLIMIT, Some(max)))
                {
                    return Err(EPERM);
                }
                set.push((r, (cur, max)));
            }
            p.limits.lock().extend(set);
            reply_status(rcvid, 0, &[])
        }
        // struct _proc_resource_setabilities { type, subtype, pid, count,
        // entry[count] }.
        RESOURCE_SETABILITIES => {
            let word = |o: usize| msg.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
            let pid = word(4).ok_or(EINVAL)? as i32;
            // The entries are the rest of the message (procnto ignores
            // `count`).
            let words: Vec<u32> = (12..msg.len() & !3).step_by(4).map(|o| word(o).unwrap()).collect();
            let me = process::lookup(sender).ok_or(ESRCH)?;
            let target = if pid == 0 { me.clone() } else { process::lookup(pid).ok_or(ESRCH)? };
            if target.is_zombie() {
                return Err(ENXIO);
            }
            ability::set(&me, &target, &words)?;
            reply_status(rcvid, 0, &[])
        }
        // struct _proc_resource_lookupability { type, subtype, pid, flags,
        // namelen, name[namelen] }: the ID is the reply's status. As
        // procnto's `procmgr_msg_resource`: a name above
        // PROCMGR_ABILITY_NAME_MAX is E2BIG, one not ending in its NUL
        // EBADMSG; a lookup that may not make the ability is ENOENT (libc
        // then hands out a temporary ID).
        RESOURCE_LOOKUPABILITY | RESOURCE_CREATEABILITY => {
            let word = |o: usize| msg.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
            let namelen = word(12).ok_or(EBADMSG)? as usize;
            if namelen > ABILITY_NAME_MAX {
                return Err(E2BIG);
            }
            let name = msg.get(16..16 + namelen).filter(|_| namelen > 0).ok_or(EBADMSG)?;
            if name.iter().position(|&c| c == 0) != Some(namelen - 1) {
                return Err(EBADMSG);
            }
            let name = core::str::from_utf8(&name[..namelen - 1]).map_err(|_| EBADMSG)?;
            let me = process::lookup(sender).ok_or(ESRCH)?;
            let id = if subtype == RESOURCE_CREATEABILITY {
                ability_create(&me, name, word(8).unwrap_or(0), true)?
            } else {
                ability_create(&me, name, 0, false).map_err(|e| if e == EPERM { ENOENT } else { e })?
            };
            reply_status(rcvid, id as u64, &[]);
        }
        // struct _proc_resource_ability_name { type, subtype, pid, able }:
        // a custom ability's name (`procmgr_ability_name()`); EINVAL for
        // QNX's own and unknown IDs (procnto's `nto_ability_query_get_name`),
        // and error 255 if it doesn't fit the reply buffer (up to
        // PROCMGR_ABILITY_NAME_MAX) with its NUL.
        RESOURCE_ABILITY_NAME => {
            let aid = msg.get(8..12).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap())) & 0xffff;
            let mut name = {
                let a = customs();
                let i = aid.checked_sub(FIRST_CUSTOM_ABILITY).ok_or(EINVAL)? as usize;
                a.get(i).ok_or(EINVAL)?.0.clone().into_bytes()
            };
            name.push(0);
            if name.len() > room.min(ABILITY_NAME_MAX) {
                return Err(crate::ker::errno::ENOBUFS);
            }
            reply_status(rcvid, 0, &name);
        }
        _ => {
            kprintln!("[pid {sender}] procmgr: _PROC_RESOURCE subtype {subtype} not implemented");
            return Err(ENOSYS);
        }
    }
    Ok(())
}

/// `_PROC_TIMER_TOLERANCE` (`procmgr_timer_tolerance()`): get or set a
/// process's default timer tolerance; the reply is the one before.
/// Another process's needs `DEFAULT_TIMER_TOLERANCE`.
fn timer_tolerance(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    const SET: u16 = 1;
    if subtype > SET {
        return Err(ENOSYS);
    }
    let pid = msg.get(4..8).map(|b| i32::from_le_bytes(b.try_into().unwrap())).ok_or(EINVAL)?;
    let p = process::lookup(if pid == 0 { sender } else { pid }).ok_or(ESRCH)?;
    let old = p.timer_tolerance.load(Ordering::Relaxed);
    if subtype == SET {
        let new = msg.get(8..16).map(|b| u64::from_le_bytes(b.try_into().unwrap())).ok_or(EINVAL)?;
        if p.pid != sender
            && !process::lookup(sender).is_some_and(|me| ability::allowed(&me, ability::DEFAULT_TIMER_TOLERANCE, None))
        {
            return Err(EPERM);
        }
        p.timer_tolerance.store(new, Ordering::Relaxed);
    }
    reply_status(rcvid, 0, &old.to_le_bytes());
    Ok(())
}

/// `_PROC_SERVMON`.
const PROC_SERVMON: u16 = 0x0021;

/// `_PROC_SERVMON` (`struct _proc_servmon` { type, subtype, timeout,
/// target_pid, sigevent }): server monitors, see `ker::servmon`. As
/// procnto's `procmgr_servmonitor`: CONFIG and IGNORE take
/// `PROCMGR_AID_SERVER_MONITOR`; other subtypes are ENOSYS.
fn servmon(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    use crate::ker::servmon;
    const CONFIG: u16 = 0;
    const IGNORE: u16 = 1;
    const ALIVE: u16 = 2;
    let word = |o: usize| msg.get(o..o + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
    let able = || {
        let me = process::lookup(sender).ok_or(ESRCH)?;
        if ability::allowed(&me, ability::SERVER_MONITOR, None) { Ok(()) } else { Err(EPERM) }
    };
    match subtype {
        CONFIG => {
            able()?;
            let b: &[u8; crate::ker::event::SIGEV_SIZE] =
                msg.get(16..16 + crate::ker::event::SIGEV_SIZE).ok_or(EBADMSG)?.try_into().unwrap();
            servmon::config(sender, word(4), crate::ker::event::SigEvent::from_bytes(b))?;
        }
        IGNORE => {
            able()?;
            let target = word(8) as i32;
            process::lookup(target).filter(|_| target != 0).ok_or(ESRCH)?;
            servmon::ignore(sender, target)?;
        }
        ALIVE => servmon::alive()?,
        _ => return Err(ENOSYS),
    }
    reply_status(rcvid, 0, &[]);
    Ok(())
}

/// `_PROC_MAC` (mandatory access control): there is no security policy;
/// `_PROC_MAC_POLICY_ID` answers 0 (no policy loaded).
fn mac(rcvid: u64, sender: i32, subtype: u16) -> Result<(), u32> {
    match subtype {
        MAC_POLICY_ID => reply_status(rcvid, 0, &0u32.to_le_bytes()),
        _ => {
            kprintln!("[pid {sender}] procmgr: _PROC_MAC subtype {subtype} not implemented");
            return Err(ENOSYS);
        }
    }
    Ok(())
}

// _PROC_UMASK subtypes
const UMASK_SET: u16 = 0;
const UMASK_GET: u16 = 1;

/// `_PROC_UMASK` (`struct _proc_umask` { type, subtype, umask, pid }): set
/// or get a process's file creation mask; the reply (`struct
/// _proc_umask_reply` { zero1, umask }) has the previous one. As procnto's
/// `procmgr_umask`: anyone may read a mask; the value set is kept as given
/// (libc masks it); setting another process's takes `PROCMGR_AID_UMASK`
/// unless the target's real, effective and saved user IDs are all the
/// caller's effective one. Other subtypes are ENOSYS.
fn umask(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    let int = |o: usize| {
        msg.get(o..o + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    if subtype != UMASK_SET && subtype != UMASK_GET {
        return Err(ENOSYS);
    }
    let pid = int(8).unwrap_or(0) as i32;
    let p = process::lookup(if pid == 0 { sender } else { pid }).ok_or(ESRCH)?;
    let old = if subtype == UMASK_GET {
        p.umask.load(Ordering::Relaxed)
    } else {
        if p.pid != sender {
            let caller = process::lookup(sender).ok_or(ESRCH)?;
            let euid = caller.cred.lock().euid;
            let same = {
                let c = p.cred.lock();
                c.ruid == euid && c.euid == euid && c.suid == euid
            };
            if !same && !ability::allowed(&caller, ability::UMASK, None) {
                return Err(EPERM);
            }
        }
        p.umask.swap(int(4).ok_or(EINVAL)?, Ordering::Relaxed)
    };
    let mut r = [0u8; 8];
    r[4..8].copy_from_slice(&old.to_le_bytes());
    reply_status(rcvid, 0, &r);
    Ok(())
}

/// `_PROC_DAEMON` (`procmgr_daemon()`): the sender leaves its parent and
/// session, becoming a child of procnto and the leader of a new session.
/// Redirecting its standard descriptors to /dev/null is not done yet.
fn daemon(rcvid: u64, sender: i32) -> Result<(), u32> {
    let p = process::lookup(sender).ok_or(ESRCH)?;
    // As procnto's `procmgr_msg_daemon`: to its parent the process has
    // exited (`procmgr_nozombie`), and it is in session 1 (daemons'
    // deaths are `PROCMGR_EVENT_DAEMON_DEATH`).
    detach(&p);
    p.set_daemon_session();
    reply_status(rcvid, 0, &[]);
    Ok(())
}

/// `_PROC_GETPID64`: the 64-bit pid of process `pid`, which must exist (0
/// is none, as procnto).
fn getpid64(rcvid: u64, _sender: i32, msg: &[u8]) -> Result<(), u32> {
    let pid = msg
        .get(4..8)
        .map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()));
    let p = process::lookup(pid).filter(|_| pid != 0).ok_or(ESRCH)?;
    reply_status(rcvid, 0, &debug::pid64(p.pid).to_le_bytes());
    Ok(())
}

/// Handle a `_PROC_*` message from thread `tid` of process `sender`;
/// replies to it.
pub fn handle(rcvid: u64, sender: i32, tid: u32, ty: u16, subtype: u16, msg: &[u8], room: usize) {
    let r = match ty {
        PROC_GETSETID => getsetid(rcvid, sender, subtype, msg),
        PROC_FORK if !process::lookup(sender).is_some_and(|p| ability::allowed(&p, ability::FORK, None)) => {
            Err(EPERM)
        }
        PROC_FORK => fork(rcvid, sender, tid, msg),
        PROC_SETPGID => setpgid(rcvid, sender, msg),
        PROC_GETPID64 => getpid64(rcvid, sender, msg),
        PROC_WAIT => wait(rcvid, sender, msg),
        PROC_RESOURCE => resource(rcvid, sender, subtype, msg, room),
        PROC_UMASK => umask(rcvid, sender, subtype, msg),
        PROC_DAEMON => daemon(rcvid, sender),
        PROC_MAC => mac(rcvid, sender, subtype),
        PROC_TIMER_TOLERANCE => timer_tolerance(rcvid, sender, subtype, msg),
        super::procevent::PROC_EVENT => super::procevent::handle(rcvid, sender, subtype, msg),
        super::procevent::PROC_VALUE => super::procevent::value(rcvid, sender, subtype, msg),
        PROC_SERVMON => servmon(rcvid, sender, subtype, msg),
        super::session::PROC_SESSION => super::session::handle(rcvid, sender, msg),
        debug::PROC_GETNAME | debug::PROC_GET_ALL_PID | debug::PROC_DEBUG | debug::PROC_GET_ALL_CHILD_PID => {
            debug::handle(rcvid, sender, ty, msg, room)
        }
        _ => {
            kprintln!(
                "[pid {sender}] procmgr: message type {ty:#06x} subtype {subtype} ({} bytes) not implemented",
                msg.len()
            );
            Err(ENOSYS)
        }
    };
    if let Err(e) = r {
        let _ = call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]);
    }
}
