//! What the kernel says about a message and its sender
//! (`<sys/neutrino.h>`), and what a server says about an open file
//! (`struct _fdinfo`).

use crate::pod::pod;

/// `struct _msg_info`: a received message, and who sent it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MsgInfo {
    pub nd: u32,
    pub srcnd: u32,
    pub pid: i32,
    pub tid: i32,
    pub chid: i32,
    pub scoid: i32,
    pub coid: i32,
    pub priority: i16,
    pub flags: i16,
    /// `_Ssizet`s, as the kernel fills them.
    pub msglen: isize,
    pub srcmsglen: isize,
    pub dstmsglen: isize,
    pub type_id: u32,
    pub reserved: u32,
}

/// `_NTO_MI_UNBLOCK_REQ`: the client asked to be unblocked.
pub const MI_UNBLOCK_REQ: i16 = 0x100;

/// `struct _msg_info_hdr`: the first words of it, in a dup request.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MsgInfoHdr {
    pub nd: u32,
    pub srcnd: u32,
    pub pid: i32,
    pub tid: i32,
    pub chid: i32,
    pub scoid: i32,
    pub coid: i32,
}

/// `struct _msg_info32`, in a map request.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MsgInfo32 {
    pub nd: u32,
    pub srcnd: u32,
    pub pid: i32,
    pub tid: i32,
    pub chid: i32,
    pub scoid: i32,
    pub coid: i32,
    pub msglen: i32,
    pub srcmsglen: i32,
    pub dstmsglen: i32,
    pub priority: i16,
    pub flags: i16,
    pub type_id: u32,
}

/// `struct _client_info`: a client's credentials, with room for its
/// first eight supplementary groups; more follow when asked for.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ClientInfo {
    pub nd: u32,
    pub pid: i32,
    pub sid: i32,
    pub flags: u32,
    pub ruid: u32,
    pub euid: u32,
    pub suid: u32,
    pub rgid: u32,
    pub egid: u32,
    pub sgid: u32,
    pub ngroups: u32,
    pub grouplist: [u32; 8],
}

/// `struct _fdinfo`: an open file, as its server describes it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Fdinfo {
    pub mode: u32,
    pub ioflag: u32,
    pub offset: u64,
    pub size: u64,
    pub flags: u32,
    pub sflag: u16,
    pub count: u16,
    pub rcount: u16,
    pub wcount: u16,
    pub rlocks: u16,
    pub wlocks: u16,
    pub filehandle: u64,
    pub inodegen: u32,
    pub zero: [u32; 3],
}

pod!(
    MsgInfo = 64,
    MsgInfoHdr = 28,
    MsgInfo32 = 48,
    ClientInfo = 76,
    Fdinfo = 64
);
