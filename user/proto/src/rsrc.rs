//! The resource database (`<sys/rsrcdbmsg.h>`), which the process manager
//! keeps: ranges of memory, interrupts, I/O ports and the like that
//! drivers create, attach and detach, and device numbers.

/// `RSRCDBMGR_RSRC_CMD`: the message type.
pub const RSRC_CMD: u16 = 0xd0;
/// `RSRCDBMGR_REQ_*`: the subtype's request, above the resource type.
pub const REQ_CREATE: u16 = 2 << 8;
pub const REQ_DESTROY: u16 = 4 << 8;
pub const REQ_ATTACH: u16 = 6 << 8;
pub const REQ_DETACH: u16 = 8 << 8;
pub const REQ_QUERY: u16 = 10 << 8;
pub const REQ_RESERVED: u16 = 12 << 8;
pub const REQ_QUERY_NAME: u16 = 14 << 8;
/// The resource type of device numbers, in a request's subtype.
pub const DEVNO: u16 = 8;

/// `struct _rsrc_cmd`: then the elements, then their names.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct RsrcCmd {
    pub r#type: u16,
    pub subtype: u16,
    pub pid: u32,
    pub nbytes: u32,
    pub count: u32,
    pub index: u32,
}

/// `msg_rsrc_minor_request_t`: a device number's request and reply.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MinorRequest {
    pub major: i32,
    pub minor: i32,
    pub flags: i32,
    pub reserved: u32,
}

crate::pod::pod!(RsrcCmd = 20, MinorRequest = 16);

/// Offsets in `struct syspage_entry` (`<sys/syspage.h>`): the hardware
/// information's and the strings' sections, and the address space's
/// array.
pub const SYSPAGE_HWINFO: usize = 16;
pub const SYSPAGE_STRINGS: usize = 32;
pub const SYSPAGE_ASINFO: usize = 200;
/// `struct asinfo_entry`'s size, and its name's offset.
pub const ASINFO_SIZE: usize = 24;
pub const ASINFO_NAME: usize = 18;
