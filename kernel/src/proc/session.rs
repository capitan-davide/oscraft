//! Sessions and their controlling terminals: `_PROC_SESSION`
//! (`procmgr_session()`, `<sys/procmsg.h>`, `<sys/procmgr.h>`) and
//! `/dev/tty`.
//!
//! Contract, from procnto's `procmgr_msg_session` and `tty_open`: the
//! message is { type, xid, sid, id, event }; the events are
//! - `PROCMGR_SESSION_SETSID` (0): the sender, unless it leads a process
//!   group, leads a new session; the reply status is its pid.
//! - `PROCMGR_SESSION_TCSETSID` (1): the session leader `sid` makes its
//!   descriptor `id` the session's terminal; procnto keeps a connection to
//!   it and tells the terminal's server with `DCMD_CHR_TCSETSID`.
//! - `PROCMGR_SESSION_SETPGRP` (2): the terminal's server reports `id` as
//!   the session's foreground process group.
//! - `PROCMGR_SESSION_SIGNAL_LEADER` (3), `_SIGNAL_PGRP` (4) and
//!   `_SIGNAL_PID` (5): signal `id` to the leader of session `sid`, its
//!   foreground process group, or the process group of process `sid` (a
//!   terminal's signal keys, `SIGHUP`, `SIGTTOU`...). They take
//!   `PROCMGR_AID_SESSION` (EPERM, checked first). As measured on procnto:
//!   `SIGNAL_LEADER` needs a session with a terminal (EINVAL), and signal
//!   0 sends nothing; `SIGNAL_PGRP` needs a session (EINVAL), with a
//!   terminal its foreground group, else the group `sid`; `SIGNAL_PID`
//!   needs a process and a valid signal (EINVAL), and is EIO if the
//!   process ignores the signal or thread `xid` (if not 0) blocks it.
//! - Opening `/dev/tty` opens the session's terminal; without one, ENXIO.
//!
//! Here `/dev/tty` answers with a symbolic link to the terminal's name in
//! the path space (procnto sends an `_IO_OPENFD` on its own connection to
//! the terminal instead).

use alloc::collections::BTreeMap;
use alloc::string::String;

use crate::arch::kercall::call;
use crate::ker::ability;
use crate::ker::errno::{EBADF, EINVAL, EIO, ENOSYS, EPERM, ESRCH};
use crate::ker::number::*;
use crate::ker::process;
use crate::ker::signal::{self, SigInfo};
use crate::sync::IrqSpinLock;

use super::pathmgr;

pub const PROC_SESSION: u16 = 0x0017;
const SETSID: u32 = 0;
const TCSETSID: u32 = 1;
const SETPGRP: u32 = 2;
const SIGNAL_LEADER: u32 = 3;
const SIGNAL_PGRP: u32 = 4;
const SIGNAL_PID: u32 = 5;
/// `DCMD_CHR_TCSETSID`: `__DIOT(_DCMD_CHR, 8, pid_t)`.
const DCMD_CHR_TCSETSID: u32 = 0x8004_0308;
const IO_DEVCTL: u16 = 0x106;

/// A session's controlling terminal.
struct Terminal {
    /// The terminal's server and channel.
    pid: i32,
    chid: i32,
    /// The foreground process group, as its server reported it.
    pgrp: i32,
    /// Its path, as its server tells (`_IO_FDINFO`), if known.
    path: Option<String>,
    /// procnto's connection to it (`tcsetsid`), closed when the session
    /// leader ends: that tells the server the session is gone, and frees
    /// the terminal for another session (procnto's
    /// `procmgr_sleader_detach`).
    coid: i32,
}

static TERMINALS: IrqSpinLock<BTreeMap<i32, Terminal>> = IrqSpinLock::new(BTreeMap::new());

fn i32_at(msg: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(msg[at..at + 4].try_into().unwrap())
}

/// Make procnto's connection `coid` (an open terminal) the controlling
/// terminal of the session `sid` leads.
pub fn set_terminal(sid: i32, coid: i32) -> Result<(), u32> {
    let conn = process::KERNEL.connection(coid).ok_or(EINVAL)?;
    // struct _io_devctl { type, combine_len, dcmd, nbytes, zero }, the data.
    let mut m = [0u8; 20];
    m[0..2].copy_from_slice(&IO_DEVCTL.to_le_bytes());
    m[2..4].copy_from_slice(&16u16.to_le_bytes());
    m[4..8].copy_from_slice(&DCMD_CHR_TCSETSID.to_le_bytes());
    m[8..12].copy_from_slice(&4u32.to_le_bytes());
    m[16..20].copy_from_slice(&sid.to_le_bytes());
    let mut reply = [0u8; 16];
    call(
        __KER_MSG_SENDV,
        [
            coid as u64,
            m.as_ptr() as u64,
            (-(m.len() as i64)) as u64,
            reply.as_mut_ptr() as u64,
            (-(reply.len() as i64)) as u64,
            0,
        ],
    )?;
    let path = fd_path(coid);
    let old = TERMINALS.lock().insert(
        sid,
        Terminal {
            pid: conn.channel.owner.pid,
            chid: conn.channel.chid,
            pgrp: sid,
            path,
            coid,
        },
    );
    if let Some(old) = old {
        release(old);
    }
    Ok(())
}

/// Close procnto's connection to a terminal its session no longer has
/// (on the worker thread: the terminal's server may be sending to us).
fn release(t: Terminal) {
    super::worker::defer(move || super::client::close(t.coid));
}

/// The path of procnto's connection `coid`, as its server tells
/// (`_IO_FDINFO`, as `iofdinfo()`): one server (devc-pty) serves many
/// terminals on one channel.
fn fd_path(coid: i32) -> Option<String> {
    const IO_FDINFO: u16 = 0x10e;
    // struct _io_fdinfo { type, combine_len, flags, path_len, reserved };
    // the reply: struct _io_fdinfo_reply { zero[2], struct _fdinfo info }
    // (72 bytes), then the path.
    let mut m = [0u8; 16];
    m[0..2].copy_from_slice(&IO_FDINFO.to_le_bytes());
    m[2..4].copy_from_slice(&16u16.to_le_bytes());
    m[8..12].copy_from_slice(&256u32.to_le_bytes());
    let mut reply = [0u8; 72 + 256];
    call(
        __KER_MSG_SENDV,
        [
            coid as u64,
            m.as_ptr() as u64,
            (-(m.len() as i64)) as u64,
            reply.as_mut_ptr() as u64,
            (-(reply.len() as i64)) as u64,
            0,
        ],
    )
    .ok()?;
    let path = reply[72..].split(|&b| b == 0).next()?;
    (!path.is_empty()).then(|| String::from_utf8_lossy(path).into_owned())
}

/// Session `sid`'s foreground process group, if it has a terminal whose
/// server has reported one.
pub fn foreground(sid: i32) -> Option<i32> {
    TERMINALS.lock().get(&sid).map(|t| t.pgrp).filter(|&g| g > 0)
}

/// The name `/dev/tty` stands for in process `pid`: its session's
/// terminal (the console if the terminal's server has no name); none for
/// a session without a terminal (ENXIO, as procnto: sshd checks that a
/// new session has none).
pub fn tty_name(pid: i32) -> Option<String> {
    let sid = process::lookup(pid).map_or(0, |p| p.sid());
    let server = TERMINALS.lock().get(&sid).map(|t| (t.pid, t.chid, t.path.clone()))?;
    Some(match server {
        (_, _, Some(path)) => path,
        (pid, chid, None) => pathmgr::name_of(pid, chid)
            .map_or_else(|| "/dev/text".into(), |n| alloc::format!("/{n}")),
    })
}

/// The session's controlling terminal, for `struct kinfo_proc`: its name
/// and foreground process group.
pub fn terminal_of(sid: i32) -> Option<(String, i32)> {
    let pgrp = TERMINALS.lock().get(&sid).map(|t| t.pgrp)?;
    Some((tty_name(sid)?, pgrp))
}

/// `tcsetsid()`: the leader `sender` of session `sid` makes its descriptor
/// `fd` the session's terminal. procnto opens it too (a connection of its
/// own and an `_IO_OPENFD` naming the leader's), then `set_terminal`. EPERM unless `sender`
/// leads session `sid`, EBADF for no such descriptor.
fn tcsetsid(sender: i32, sid: i32, fd: i32) -> Result<(), u32> {
    const IO_OPENFD: u16 = 0x10d;
    const IO_FLAG_RD_WR: u32 = 3;
    let p = process::lookup(sender).ok_or(ESRCH)?;
    if sid != sender || p.sid() != sender {
        return Err(EPERM);
    }
    let c = p.connection(fd).ok_or(EBADF)?;
    let (server, chid, scoid) = (c.channel.owner.pid, c.channel.chid, c.scoid);
    let coid = call(
        __KER_CONNECT_ATTACH,
        [0, server as u64, chid as u64, process::SIDE_CHANNEL as u64, 0, 0],
    )? as i32;
    // struct _io_openfd { type, combine_len, ioflag, sflag, xtype, struct
    // _msg_info_hdr info, handle }: the info names the leader's connection.
    // A new open, not a dup: the session is procnto's open's, and goes
    // with it, though others keep the terminal open.
    let mut m = [0u8; 48];
    m[0..2].copy_from_slice(&IO_OPENFD.to_le_bytes());
    m[2..4].copy_from_slice(&48u16.to_le_bytes());
    m[4..8].copy_from_slice(&IO_FLAG_RD_WR.to_le_bytes());
    m[20..24].copy_from_slice(&sender.to_le_bytes()); // info.pid
    m[28..32].copy_from_slice(&chid.to_le_bytes()); // info.chid
    m[32..36].copy_from_slice(&scoid.to_le_bytes()); // info.scoid
    m[36..40].copy_from_slice(&fd.to_le_bytes()); // info.coid
    let open = call(
        __KER_MSG_SENDV,
        [coid as u64, m.as_ptr() as u64, (-(m.len() as i64)) as u64, 0, 0, 0],
    );
    let r = open.and_then(|_| set_terminal(sid, coid));
    if r.is_err() {
        let _ = call(__KER_CONNECT_DETACH, [coid as u64, 0, 0, 0, 0, 0]);
    }
    r
}

/// Handle `_PROC_SESSION` from process `sender`.
pub fn handle(rcvid: u64, sender: i32, msg: &[u8]) -> Result<(), u32> {
    if msg.len() < 16 {
        return Err(EINVAL);
    }
    let (sid, id, event) = (i32_at(msg, 4), i32_at(msg, 8), i32_at(msg, 12) as u32);
    let info = |signo| SigInfo {
        signo,
        code: 0, // SI_USER
        pid: sender,
        ..Default::default()
    };
    let status = match event {
        SETSID => {
            let p = process::lookup(sender).ok_or(ESRCH)?;
            if p.pgid() == p.pid {
                return Err(EPERM);
            }
            p.set_new_session();
            sender
        }
        SETPGRP => {
            let mut terminals = TERMINALS.lock();
            let t = terminals.get_mut(&sid).ok_or(EINVAL)?;
            if t.pid != sender {
                return Err(EPERM);
            }
            t.pgrp = id;
            0
        }
        SIGNAL_LEADER | SIGNAL_PGRP | SIGNAL_PID
            if !process::lookup(sender).is_some_and(|p| ability::allowed(&p, ability::SESSION, None)) =>
        {
            return Err(EPERM);
        }
        SIGNAL_LEADER => {
            if !TERMINALS.lock().contains_key(&sid) {
                return Err(EINVAL);
            }
            let p = process::lookup(sid).ok_or(EINVAL)?;
            if id != 0 {
                let id = crate::personality::qnx::signo_in(id as usize) as i32;
                signal::post(&p, id as usize, info(id));
            }
            0
        }
        // As procnto's `procmgr_msg_session`: EIO if process `sid`
        // ignores the signal (`SIG_IGN`), or, with a thread ID in `xid`,
        // that thread blocks it (a terminal's server asks so before
        // sending SIGTTOU or SIGTTIN); otherwise its process group gets it.
        SIGNAL_PID => {
            // Terminal drivers here are QNX's: its numbers.
            let id = crate::personality::qnx::signo_in(id as usize) as i32;
            let p = process::lookup(sid).ok_or(EINVAL)?;
            if !(1..signal::NSIG as i32).contains(&id) {
                return Err(EINVAL);
            }
            if p.signals.lock().actions[id as usize].handler == signal::SIG_IGN {
                return Err(EIO);
            }
            let xid = u16::from_le_bytes([msg[2], msg[3]]);
            if xid != 0 {
                let t = crate::sched::threads_of(&p)
                    .into_iter()
                    .find(|t| t.ptid == xid as u32)
                    .ok_or(ESRCH)?;
                if t.sigmask.load(core::sync::atomic::Ordering::Relaxed) & (1 << (id - 1)) != 0 {
                    return Err(EIO);
                }
            }
            signal::post_group(p.pgid(), id as usize);
            0
        }
        SIGNAL_PGRP => {
            // Terminal drivers here are QNX's: its numbers.
            let id = crate::personality::qnx::signo_in(id as usize) as i32;
            let pgrp = match TERMINALS.lock().get(&sid).map(|t| t.pgrp) {
                Some(pgrp) => pgrp,
                None if process::lookup(sid).is_some_and(|p| p.sid() == sid) => sid,
                None => return Err(EINVAL),
            };
            signal::post_group(pgrp, id as usize);
            0
        }
        // Messages to the terminal's server, which may ask procnto in
        // turn: on the worker thread, which answers.
        TCSETSID => {
            super::worker::defer(move || {
                let status = tcsetsid(sender, sid, id);
                let _ = match status {
                    Ok(()) => call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]),
                    Err(e) => call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]),
                };
            });
            return Ok(());
        }
        _ => return Err(ENOSYS),
    };
    let _ = call(__KER_MSG_REPLYV, [rcvid, status as u64, 0, 0, 0, 0]);
    Ok(())
}

/// A session leader has ended: its session loses its terminal, and the
/// terminal's foreground process group gets SIGHUP (procnto's
/// `procmgr_sleader_detach`).
pub fn process_exited(pid: i32) {
    const SIGHUP: usize = 1;
    let gone = TERMINALS.lock().remove(&pid);
    if let Some(t) = gone {
        if t.pgrp > 0 {
            signal::post_group(t.pgrp, SIGHUP);
        }
        release(t);
    }
}
