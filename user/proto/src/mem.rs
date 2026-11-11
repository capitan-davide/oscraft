//! The memory manager's messages (`<sys/memmsg.h>`), on
//! [`SYSMGR_COID`](crate::SYSMGR_COID) or on a shared memory object's
//! descriptor.

use crate::pod::pod;

/// The message types, from `_MEMMGR_BASE`.
pub const MEM_MAP: u16 = 0x40;
pub const MEM_CTRL: u16 = 0x41;
pub const MEM_INFO: u16 = 0x42;
pub const MEM_OFFSET: u16 = 0x43;
pub const MEM_PEER: u16 = 0x47;
pub const MEM_SHMEM_HANDLE: u16 = 0x4b;
pub const MEM_SHMEM_REVOKE: u16 = 0x4c;

/// `_MEM_CTRL_*`.
pub const CTRL_UNMAP: u16 = 0;
pub const CTRL_PROTECT: u16 = 1;
pub const CTRL_SYNC: u16 = 2;
pub const CTRL_LOCKALL: u16 = 3;
pub const CTRL_UNLOCKALL: u16 = 4;
pub const CTRL_LOCK: u16 = 5;
pub const CTRL_UNLOCK: u16 = 6;
pub const CTRL_ADVISE: u16 = 7;

/// `_MEM_OFFSET_*`.
pub const OFFSET_PHYS: u16 = 0;
pub const OFFSET_FD: u16 = 1;

/// `_MEM_SHMEM_*_HANDLE`.
pub const HANDLE_CREATE: u16 = 0;
pub const HANDLE_OPEN: u16 = 1;
pub const HANDLE_DELETE: u16 = 2;

/// `SHM_CREATE_HANDLE_OPT_SUBRANGE`: the handle is to part of the object.
pub const HANDLE_OPT_SUBRANGE: u32 = 2;

/// `struct _mem_map`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Map {
    pub r#type: u16,
    pub zero: u16,
    pub pid: i32,
    pub addr: u64,
    pub len: u64,
    pub prot: u32,
    pub flags: u32,
    pub fd: i32,
    pub preload: u32,
    pub align: u64,
    pub offset: i64,
    pub shmem_handle: u64,
    pub elf_pvaddr: u64,
}

/// `struct _mem_map_reply`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MapReply {
    pub real_size: u64,
    pub real_addr: u64,
    pub addr: u64,
}

/// `struct _mem_ctrl`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Ctrl {
    pub r#type: u16,
    pub subtype: u16,
    pub flags: u32,
    pub addr: u64,
    pub len: u64,
}

/// `struct _mem_info`; the reply's data is a `struct posix_typed_mem_info`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Info {
    pub r#type: u16,
    pub zero: u16,
    pub fd: i32,
    pub flags: u32,
    pub reserved: u32,
}

/// `struct _mem_offset`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Offset {
    pub r#type: u16,
    pub subtype: u16,
    pub reserved: i32,
    pub addr: u64,
    pub len: u64,
}

/// `struct _mem_offset_reply`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct OffsetReply {
    pub size: u64,
    pub offset: i64,
    pub fd: i32,
    pub reserved: u32,
}

/// `struct _mem_peer`: the header of a request on another process's
/// behalf; the request follows.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Peer {
    pub r#type: u16,
    pub peer_msg_len: u16,
    pub pid: u32,
    pub reserved1: u64,
}

/// `struct _mem_shmem_handle`, and its reply in place.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ShmemHandle {
    pub r#type: u16,
    pub subtype: u16,
    pub options: u32,
    pub handle: u64,
    pub pid: u32,
    pub perm: u32,
    pub sub_offset: i64,
    pub sub_length: u64,
}

/// `struct _mem_shmem_revoke`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ShmemRevoke {
    pub r#type: u16,
    pub subtype: u16,
    pub pid: i32,
    pub zero64: [u64; 4],
}

pod!(
    Map = 0x48,
    MapReply = 0x18,
    Ctrl = 0x18,
    Info = 16,
    Offset = 24,
    OffsetReply = 24,
    Peer = 16,
    ShmemHandle = 40,
    ShmemRevoke = 40,
);
