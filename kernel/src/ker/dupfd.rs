//! Passing file descriptors between processes: `ConnectDupFd(op, pid,
//! handle, srvpid, info)` (QNX 8.0 library reference).
//!
//! As procnto's `ker_connect_dupfd` (and measured):
//! - `_NTO_DUPFD_REGISTER(_DISC)`: the sender registers its descriptor
//!   `info->coid` for process `*pid` and gets a `*handle`. At most 200
//!   per process: then EAGAIN, or with `_DISC` its oldest one goes.
//! - `_NTO_DUPFD_UNREGISTER`: the sender or the receiver forgets
//!   `*handle`.
//! - `_NTO_DUPFD_LOOKUP`: the sender or the receiver gets, for `*handle`,
//!   the peer's pid (`*pid`), the server's (`*srvpid`) and the
//!   `struct _msg_info_hdr` of the descriptor's connection (`*info`), for
//!   the `_IO_DUP` the receiver then sends that server.
//! - `_NTO_DUPFD_VERIFY`: the server checks that the `_IO_DUP` (its info
//!   in `*info`) from process `*pid` has the sender's approval, and so
//!   uses the registration up: one import per registration. procnto's
//!   own resource managers ask `authorized()`, which does the same.
//!
//! A handle there isn't (never made, used up, forgotten) is ENOENT. A
//! process's registrations go when it ends, the sender or the receiver
//! (review IPC-22: those for a receiver that had ended stayed, and
//! counted against the sender's 200).

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::KerResult;
use super::errno::{EAGAIN, EBADF, EINVAL, ENOENT, ESRCH};
use super::process;
use super::umem::{self, Space};
use crate::sched;
use crate::sync::IrqSpinLock;

const REGISTER: u32 = 1;
const REGISTER_DISC: u32 = 2;
const UNREGISTER: u32 = 3;
const LOOKUP: u32 = 4;
const VERIFY: u32 = 5;
/// Registrations per process.
const MAX: usize = 200;
/// `struct _msg_info_hdr`: nd, srcnd, pid, tid, chid, scoid, coid.
const INFO_SIZE: usize = 28;

struct Reg {
    handle: u64,
    sender: i32,
    receiver: i32,
    server: i32,
    info: [u8; INFO_SIZE],
}

static REGS: IrqSpinLock<Vec<Reg>> = IrqSpinLock::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// `ConnectDupFd(op, pid, handle, srvpid, info)`.
pub fn connect_dupfd(op: u32, pidp: u64, handlep: u64, srvpidp: u64, infop: u64) -> KerResult {
    let me = sched::current();
    let mypid = me.process.pid;
    let space = Space::current();
    let read_u64 = |a: u64| -> Result<u64, u32> {
        let mut b = [0u8; 8];
        umem::read(space, a, &mut b)?;
        Ok(u64::from_le_bytes(b))
    };
    match op {
        REGISTER | REGISTER_DISC => {
            let receiver = read_u64(pidp)? as i32;
            process::lookup(receiver).ok_or(ESRCH)?;
            let mut hdr = [0u8; INFO_SIZE];
            umem::read(space, infop, &mut hdr)?;
            let coid = i32_at(&hdr, 24);
            let conn = me.process.connection(coid).ok_or(EBADF)?;
            let mut info = [0u8; INFO_SIZE];
            let mut put = |o: usize, v: i32| info[o..o + 4].copy_from_slice(&v.to_le_bytes());
            put(8, mypid);
            put(12, me.ptid as i32);
            put(16, conn.channel.chid);
            put(20, conn.scoid);
            put(24, coid);
            let handle = NEXT.fetch_add(1, Ordering::Relaxed);
            let mut regs = REGS.lock();
            if regs.iter().filter(|r| r.sender == mypid).count() >= MAX {
                if op == REGISTER {
                    return Err(EAGAIN);
                }
                let oldest = regs.iter().position(|r| r.sender == mypid).unwrap();
                regs.remove(oldest);
            }
            regs.push(Reg {
                handle,
                sender: mypid,
                receiver,
                server: conn.channel.owner.pid,
                info,
            });
            drop(regs);
            umem::write(space, handlep, &handle.to_le_bytes())?;
            Ok(0)
        }
        UNREGISTER => {
            let handle = read_u64(handlep)?;
            let mut regs = REGS.lock();
            let i = regs
                .iter()
                .position(|r| r.handle == handle && (r.sender == mypid || r.receiver == mypid))
                .ok_or(ENOENT)?;
            regs.remove(i);
            Ok(0)
        }
        LOOKUP => {
            let handle = read_u64(handlep)?;
            let (peer, server, info) = {
                let regs = REGS.lock();
                let r = regs.iter().find(|r| r.handle == handle).ok_or(ENOENT)?;
                let peer = match mypid {
                    p if p == r.sender => r.receiver,
                    p if p == r.receiver => r.sender,
                    _ => return Err(ENOENT),
                };
                (peer, r.server, r.info)
            };
            umem::write(space, pidp, &(peer as u64).to_le_bytes())?;
            if srvpidp != 0 {
                umem::write(space, srvpidp, &server.to_le_bytes())?;
            }
            umem::write(space, infop, &info)?;
            Ok(0)
        }
        VERIFY => {
            let requester = read_u64(pidp)? as i32;
            let mut info = [0u8; INFO_SIZE];
            umem::read(space, infop, &mut info)?;
            if authorized(i32_at(&info, 8), i32_at(&info, 24), requester, mypid) {
                Ok(0)
            } else {
                Err(ENOENT)
            }
        }
        _ => Err(EINVAL),
    }
}

/// May process `receiver` duplicate descriptor `coid` of process `sender`
/// (a connection to `server`)? A registration that says so is used up:
/// the next import of it fails (procnto, measured).
pub fn authorized(sender: i32, coid: i32, receiver: i32, server: i32) -> bool {
    let mut regs = REGS.lock();
    let found = regs.iter().position(|r| {
        r.sender == sender
            && i32_at(&r.info, 24) == coid
            && r.receiver == receiver
            && r.server == server
    });
    found.map(|i| regs.remove(i)).is_some()
}

/// Process `pid` has ended: the registrations it made, and those made for
/// it, go.
pub fn process_exited(pid: i32) {
    REGS.lock().retain(|r| r.sender != pid && r.receiver != pid);
}
