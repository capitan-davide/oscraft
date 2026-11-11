//! The process's own credentials, as a server reads them to compare
//! with a client's.

use core::ffi::c_int;

use crate::neutrino::connect::ConnectClientInfo_r;

/// `struct _client_info`, with room for the groups it holds by default.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ClientInfo {
    pub nd: u32,
    pub pid: c_int,
    pub sid: c_int,
    pub flags: u32,
    pub ruid: u32,
    pub euid: u32,
    pub suid: u32,
    pub rgid: u32,
    pub egid: u32,
    pub sgid: u32,
    pub ngroups: u32,
    grouplist: [u32; 9],
}

/// The calling process's credentials (`ConnectClientInfo(-1)`); on
/// failure the zeros stay, root's IDs, as QNX's library returns
/// whatever is in its buffer.
pub fn credentials() -> ClientInfo {
    let mut info = ClientInfo::default();
    let _ = ConnectClientInfo_r(-1, (&raw mut info).cast(), 0);
    info
}
