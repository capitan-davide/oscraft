//! The system manager's messages (`<sys/sysmsg.h>`), on
//! [`SYSMGR_COID`](crate::SYSMGR_COID): the configuration.

use crate::pod::pod;

/// `_SYS_CONF` and its subtypes.
pub const SYS_CONF: u16 = 0;
pub const SUB_GET: u16 = 0;
pub const SUB_SET: u16 = 1;

/// `_CONF_STR` and `_CONF_NUM` (`<sys/conf.h>`): what is asked for.
pub const CONF_STR: i32 = 0x10_0000;
pub const CONF_NUM: i32 = 0x20_0000;

/// `struct _sys_conf`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Conf {
    pub r#type: u16,
    pub subtype: u16,
    pub cmd: i32,
    pub name: i32,
    pub spare: i32,
    pub value: i64,
}

/// `struct _sys_conf_reply`, over the request.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ConfReply {
    pub zero: [u32; 3],
    pub r#match: i32,
    pub value: i64,
}

pod!(Conf = 24, ConfReply = 24);
