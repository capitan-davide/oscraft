//! The process manager's messages (`<sys/procmsg.h>`), on
//! [`SYSMGR_COID`](crate::SYSMGR_COID): processes, their IDs, sessions,
//! limits and children.

use crate::pod::pod;

/// The message types, from `_PROCMGR_BASE`.
pub const PROC_SPAWN: u16 = 0x10;
pub const PROC_WAIT: u16 = 0x11;
pub const PROC_FORK: u16 = 0x12;
pub const PROC_GETSETID: u16 = 0x13;
pub const PROC_SETPGID: u16 = 0x14;
pub const PROC_UMASK: u16 = 0x15;
pub const PROC_SESSION: u16 = 0x17;
pub const PROC_DAEMON: u16 = 0x18;
pub const PROC_EVENT: u16 = 0x19;
pub const PROC_RESOURCE: u16 = 0x1a;
pub const PROC_POSIX_SPAWN: u16 = 0x1b;
/// `_PROC_POSIX_SPAWNP`: the request QNX's library sends for both
/// `posix_spawn()` and `posix_spawnp()`, the search path empty for the
/// first.
pub const PROC_POSIX_SPAWNP: u16 = 0x1c;
pub const PROC_GETPID64: u16 = 0x22;

/// `_PROC_ID_*`: `_PROC_GETSETID`'s subtypes.
pub const ID_GETID: u16 = 0;
pub const ID_SETUID: u16 = 1;
pub const ID_SETGID: u16 = 2;
pub const ID_SETEUID: u16 = 3;
pub const ID_SETEGID: u16 = 4;
pub const ID_SETREUID: u16 = 5;
pub const ID_SETREGID: u16 = 6;
pub const ID_SETGROUPS: u16 = 7;
pub const ID_GETID_NO_CRED: u16 = 8;

/// `struct _proc_getsetid`, followed by the groups for
/// `_PROC_ID_SETGROUPS`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct GetSetId {
    pub r#type: u16,
    pub subtype: u16,
    pub pid: i32,
    pub eid: u32,
    pub rid: u32,
    pub ngroups: i32,
}

/// The first words of `struct _proc_getsetid_reply`; the credentials
/// follow.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct GetSetIdReply {
    pub zero: u32,
    pub pgrp: i32,
    pub ppid: i32,
    pub sid: i32,
    pub orig_ppid: i32,
    pub reserved: [u32; 2],
}

/// The room a `_PROC_GETSETID` reply needs.
pub const GETSETID_REPLY_SIZE: usize = 0x58;

/// `_PROC_EVENT_*`: `_PROC_EVENT`'s subtypes.
pub const EVENT_NOTIFY: u16 = 0;
pub const EVENT_TRIGGER: u16 = 1;
pub const EVENT_NOTIFY_ADD: u16 = 2;
pub const EVENT_NOTIFY_DEL: u16 = 3;
pub const EVENT_TRIGGER_UPDATEABLE: u16 = 4;

/// `struct _proc_event`: an event to deliver for `flags`
/// (`PROCMGR_EVENT_*`), or `flags` to trigger, with a value in the event's
/// for an updateable one.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Event {
    pub r#type: u16,
    pub subtype: u16,
    pub flags: u32,
    pub event: crate::event::Sigevent,
}

/// `struct _proc_event_del`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct EventDel {
    pub r#type: u16,
    pub subtype: u16,
    pub id: i32,
}

/// `_PROC_RESOURCE_*`: `_PROC_RESOURCE`'s subtypes.
pub const RESOURCE_USAGE: u16 = 0;
pub const RESOURCE_GETLIMIT: u16 = 1;
pub const RESOURCE_SETLIMIT: u16 = 2;
pub const RESOURCE_SETABILITIES: u16 = 3;
pub const RESOURCE_LOOKUPABILITY: u16 = 4;
pub const RESOURCE_CREATEABILITY: u16 = 5;
pub const RESOURCE_ABILITY_NAME: u16 = 6;

/// `struct _proc_resource_setabilities` up to its entries: `count`
/// words follow, abilities each with its subrange if it has one, then
/// the end of the list.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct SetAbilities {
    pub r#type: u16,
    pub subtype: u16,
    pub pid: i32,
    pub count: u32,
}

/// `struct _proc_resource_lookupability`, for a lookup or a creation,
/// followed by the name and its NUL.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct LookupAbility {
    pub r#type: u16,
    pub subtype: u16,
    pub pid: i32,
    pub flags: u32,
    pub namelen: u32,
}

/// `struct _proc_resource_ability_name`; the reply is the name.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct AbilityName {
    pub r#type: u16,
    pub subtype: u16,
    pub pid: i32,
    pub able: u32,
}

/// `struct _proc_resource_getlimit` with one resource; the reply is a
/// `struct rlimit`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct GetLimit {
    pub r#type: u16,
    pub subtype: u16,
    pub pid: i32,
    pub count: u32,
    pub reserved: u32,
    pub resource: u32,
}

/// `struct rlimit`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Rlimit {
    pub cur: u64,
    pub max: u64,
}

/// `struct _proc_resource_setlimit` with one entry.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct SetLimit {
    pub r#type: u16,
    pub subtype: u16,
    pub pid: i32,
    pub count: u32,
    pub reserved: u32,
    pub resource: u32,
    pub reserved2: u32,
    pub limit: Rlimit,
}

/// `struct _proc_session`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Session {
    pub r#type: u16,
    pub xid: u16,
    pub sid: i32,
    pub id: i32,
    pub event: u32,
}

/// `struct _proc_setpgid`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct SetPgid {
    pub r#type: u16,
    pub zero: u16,
    pub pid: i32,
    pub pgid: i32,
}

/// `struct _proc_umask`, and its reply.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Umask {
    pub r#type: u16,
    pub subtype: u16,
    pub umask: u32,
    pub pid: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct UmaskReply {
    pub zero: u32,
    pub umask: u32,
}

/// `_PROC_UMASK_SET` and `_PROC_UMASK_GET`.
pub const UMASK_SET: u16 = 0;
pub const UMASK_GET: u16 = 1;

/// `struct _proc_wait`; the reply is a `siginfo_t`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Wait {
    pub r#type: u16,
    pub idtype: i16,
    pub options: i32,
    pub id: i32,
}

/// `struct _proc_fork`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Fork {
    pub r#type: u16,
    pub zero: u16,
    pub flags: u32,
    pub frame: u64,
}

/// `struct _proc_daemon`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Daemon {
    pub r#type: u16,
    pub subtype: u16,
    pub status: i32,
    pub flags: u32,
    pub reserved: u32,
}

/// `struct _proc_getpid64`; the reply is the 64-bit ID.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct GetPid64 {
    pub r#type: u16,
    pub zero: u16,
    pub pid: i32,
}

/// `struct _proc_posix_spawn`, the header of both spawn requests: the
/// counts and sizes of the parts that follow it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct SpawnHeader {
    pub r#type: u16,
    pub subtype: u16,
    pub pathlen: u16,
    pub nargv: u16,
    pub narge: u16,
    pub searchlen: u16,
    pub reserved: [u16; 2],
    pub filact_bytes: u32,
    pub filact_obytes: u32,
    pub argenv_bytes: u32,
    pub attr_bytes: u32,
}

/// `_PROC_SPAWN_START`: the spawn requests' subtype.
pub const SPAWN_START: u16 = 6;

pod!(
    GetSetId = 20,
    GetSetIdReply = 28,
    GetLimit = 20,
    Rlimit = 16,
    SetLimit = 40,
    Session = 16,
    SetPgid = 12,
    Umask = 12,
    UmaskReply = 8,
    Wait = 12,
    Fork = 16,
    Daemon = 16,
    GetPid64 = 8,
    SpawnHeader = 32,
    SetAbilities = 12,
    LookupAbility = 16,
    AbilityName = 12,
    Event = 40,
    EventDel = 8,
);
