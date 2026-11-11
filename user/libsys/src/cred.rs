//! The process's IDs and credentials, from the process manager
//! (`_PROC_GETSETID`, `<sys/procmsg.h>`).

use core::ffi::c_int;

use proto::proc::{self as p, GetSetId};

use crate::errno::fail;
use crate::kercall::{Iov, SYSMGR_COID, msg_sendv};

/// `struct _proc_getsetid_reply`: the IDs, then `struct _cred_info`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Reply {
    pub zero: u32,
    pub pgrp: i32,
    pub ppid: i32,
    pub sid: i32,
    pub orig_ppid: i32,
    pub reserved: [u32; 2],
    pub ruid: u32,
    pub euid: u32,
    pub suid: u32,
    pub rgid: u32,
    pub egid: u32,
    pub sgid: u32,
    pub ngroups: u32,
    pub grouplist: [u32; 8],
}

const _: () = assert!(size_of::<Reply>() == p::GETSETID_REPLY_SIZE);

/// The calling process's IDs and credentials.
pub(crate) fn ids() -> Result<Reply, c_int> {
    let request = GetSetId {
        r#type: p::PROC_GETSETID,
        subtype: p::ID_GETID,
        pid: 0,
        ..GetSetId::default()
    };
    let mut reply = Reply::default();
    let send = [Iov::of(&request)];
    let recv = [Iov::raw((&raw mut reply).cast(), size_of::<Reply>())];
    // SAFETY: both are on this stack, valid for their sizes.
    unsafe { msg_sendv(SYSMGR_COID, &send, &recv) }?;
    Ok(reply)
}

fn id(f: fn(&Reply) -> u32) -> u32 {
    match ids() {
        Ok(r) => f(&r),
        Err(e) => fail(e) as u32,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_getuid() -> u32 {
    id(|r| r.ruid)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_geteuid() -> u32 {
    id(|r| r.euid)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_getgid() -> u32 {
    id(|r| r.rgid)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_getegid() -> u32 {
    id(|r| r.egid)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_getppid() -> c_int {
    id(|r| r.ppid as u32) as c_int
}

/// The effective user and group IDs, for permission checks.
pub fn euid_egid() -> (u32, u32) {
    ids().map_or((0, 0), |r| (r.euid, r.egid))
}

/// The process group, for `wait4(0, ...)`.
pub fn pgrp() -> c_int {
    id(|r| r.pgrp as u32) as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_getpgrp() -> c_int {
    pgrp()
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_getsid(_pid: c_int) -> c_int {
    id(|r| r.sid as u32) as c_int
}
