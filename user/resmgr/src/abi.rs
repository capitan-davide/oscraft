//! The framework's public structures, as `<sys/dispatch.h>`,
//! `<sys/resmgr.h>` and `<sys/iofunc.h>` define them: servers allocate
//! them, fill them and read them, so their layout is QNX's.

use core::ffi::{c_char, c_int, c_long, c_uint, c_void};

use kercall::abi::Iov;
pub use proto::connect::{
    CONNECT_COMBINE, CONNECT_COMBINE_CLOSE, CONNECT_MOUNT, CONNECT_OPEN, CONNECT_RSVD_UNBLOCK,
};
pub use proto::errno::{
    EACCES, EAGAIN, EBADF, EBADMSG, EBUSY, EEXIST, EFBIG, EINVAL, EISDIR, ELOOP, ENFILE, ENOENT,
    ENOMEM, ENOSYS, ENOTTY, EOVERFLOW, EPERM, EROFS, ESRCH,
};
pub use proto::event::{PULSE_CODE_DISCONNECT, PULSE_CODE_UNBLOCK};
pub use proto::io::{
    IO_CLOSE, IO_COMBINE_FLAG, IO_CONNECT, IO_DEVCTL, IO_DUP, IO_MAX, IO_MMAP, IO_MSG, IO_NOTIFY,
    IO_OPENFD, IO_PATHCONF, IO_READ, IO_READ64, IO_RSVD_CLOSE_OCB, IO_RSVD_LOCK_OCB,
    IO_RSVD_UNBLOCK, IO_RSVD_UNLOCK_OCB, IO_STAT, IO_UTIME, IO_UTIME64, IO_WRITE64,
};

/// `struct _msg_info`, what the kernel says about a message, and
/// `_NTO_MI_UNBLOCK_REQ`, the client asked to be unblocked.
pub use proto::info::{MI_UNBLOCK_REQ, MsgInfo};

/// `resmgr_context_t`, which is also `message_context_t` and the start of
/// `dispatch_context_t`: one thread's message, its sender and its reply.
/// The context is followed by `nparts_max` more parts of `iov`, then by
/// the receive buffer `msg` points to.
#[repr(C)]
pub struct Context {
    pub rcvid: i64,
    pub info: MsgInfo,
    pub msg: *mut u8,
    pub dpp: *mut crate::dispatch::Dispatch,
    /// The attachment the message is for, as `resmgr_attach()` returned.
    pub id: c_int,
    pub extra: *mut ExtendedContext,
    pub msg_max_size: usize,
    /// The reply's status, for a reply with parts.
    pub status: c_long,
    /// Where in `msg` the message being handled starts (a combined
    /// message's second part is further in), and its size.
    pub offset: usize,
    pub size: usize,
    pub iov: [Iov; 1],
}

/// `struct _extended_context`, which the context points to.
#[repr(C)]
pub struct ExtendedContext {
    pub length: usize,
    pub xendian: [u32; 2],
    pub data: *mut c_void,
}

/// A connect function: `open`, `unlink` and the others.
pub type ConnectFn =
    Option<unsafe extern "C" fn(*mut Context, *mut u8, *mut c_void, *mut c_void) -> c_int>;
/// An I/O function: `read`, `write` and the others.
pub type IoFn = Option<unsafe extern "C" fn(*mut Context, *mut u8, *mut c_void) -> c_int>;
/// `check_msg`, whose third argument is whether the message is combined.
pub type CheckFn =
    Option<unsafe extern "C" fn(*const Context, *const u8, bool, *const c_void) -> c_int>;

/// `resmgr_connect_funcs_t`: what a server does with a path: open,
/// unlink, rename, mknod, readlink, link, unblock, mount, and an open
/// combined with an I/O message.
#[repr(C)]
pub struct ConnectFuncs {
    pub nfuncs: c_uint,
    pub funcs: [ConnectFn; CONNECT_NFUNCS],
}
pub const CONNECT_NFUNCS: usize = 9;

/// `resmgr_io_funcs_t`: what a server does with an open file, one
/// function per I/O message, in the order of their types from `_IO_READ`.
#[repr(C)]
pub struct IoFuncs {
    pub nfuncs: c_uint,
    pub funcs: [IoFn; IO_NFUNCS],
}
pub const IO_NFUNCS: usize = 33;

/// `resmgr_attr_t`.
#[repr(C)]
pub struct ResmgrAttr {
    pub flags: c_uint,
    pub nparts_max: c_uint,
    pub msg_max_size: usize,
    pub other_func: Option<unsafe extern "C" fn(*mut Context, *mut c_void) -> c_int>,
    pub reserved: [c_uint; 4],
}

/// `message_attr_t`.
#[repr(C)]
pub struct MessageAttr {
    pub flags: c_uint,
    pub nparts_max: c_uint,
    pub msg_max_size: usize,
    pub reserved: [c_uint; 5],
}

/// A message or pulse handler: `(ctp, code or type, flags, handle)`.
pub type MessageFn =
    Option<unsafe extern "C" fn(*mut Context, c_int, c_uint, *mut c_void) -> c_int>;

/// `iofunc_mount_t`: what the files of one mount share.
#[repr(C)]
pub struct IofuncMount {
    pub flags: u32,
    pub conf: u32,
    pub dev: u32,
    pub blocksize: i32,
    pub funcs: *mut IofuncFuncs,
    pub power: *mut c_void,
    pub size: usize,
    pub ext_flags: u32,
    pub timeres: u32,
    pub reserved: [u64; 4],
}

/// `iofunc_funcs_t`: how a mount allocates OCBs and locks attributes.
#[repr(C)]
pub struct IofuncFuncs {
    pub nfuncs: c_uint,
    pub ocb_calloc: Option<unsafe extern "C" fn(*mut Context, *mut IofuncAttr) -> *mut IofuncOcb>,
    pub ocb_free: Option<unsafe extern "C" fn(*mut IofuncOcb)>,
    pub attr_lock: Option<unsafe extern "C" fn(*mut IofuncAttr) -> c_int>,
    pub attr_unlock: Option<unsafe extern "C" fn(*mut IofuncAttr) -> c_int>,
    pub attr_trylock: Option<unsafe extern "C" fn(*mut IofuncAttr) -> c_int>,
}

/// `iofunc_attr_t`: a file, as the server keeps it.
#[repr(C)]
pub struct IofuncAttr {
    pub mount: *mut IofuncMount,
    pub mmap_list: *mut c_void,
    pub lock_list: *mut c_void,
    pub acl: *mut c_void,
    /// A `pthread_mutex_t`: the attribute's lock.
    pub lock: [u32; 2],
    pub flags: u32,
    pub count: u16,
    pub rcount: u16,
    pub wcount: u16,
    pub rlocks: u16,
    pub wlocks: u16,
    pub reserved: u16,
    pub nbytes: i64,
    pub inode: u64,
    pub uid: u32,
    pub gid: u32,
    pub mtime: i64,
    pub atime: i64,
    pub ctime: i64,
    pub mode: u32,
    pub nlink: u32,
    pub rdev: u32,
    pub mtime_ns: u32,
    pub atime_ns: u32,
    pub ctime_ns: u32,
}

/// `iofunc_ocb_t`: an open file: one client's view of it.
#[repr(C)]
pub struct IofuncOcb {
    pub attr: *mut IofuncAttr,
    /// The open flags plus one: `_IO_FLAG_RD` and `_WR` in the low bits.
    pub ioflag: i32,
    pub sflag: u16,
    pub flags: u16,
    pub offset: i64,
    pub reserved: *mut c_void,
}

/// `iofunc_notify_event_t`: one client's armed event.
#[repr(C)]
pub struct NotifyEvent {
    pub next: *mut NotifyEvent,
    pub rcvid: i64,
    pub scoid: c_int,
    pub cnt: c_int,
    pub event: [u64; 4],
    pub flags: c_uint,
    pub coid: c_int,
}

/// `iofunc_notify_t`: the events armed for one condition, and the
/// smallest count among them.
#[repr(C)]
pub struct Notify {
    pub cnt: c_int,
    pub list: *mut NotifyEvent,
}

/// `struct _client_info`: a client's credentials.
pub use proto::info::ClientInfo;

/// `thread_pool_attr_t`.
#[repr(C)]
pub struct ThreadPoolAttr {
    pub handle: *mut c_void,
    pub block_func: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
    pub unblock_func: Option<unsafe extern "C" fn(*mut c_void)>,
    pub handler_func: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
    pub context_alloc: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
    pub context_free: Option<unsafe extern "C" fn(*mut c_void)>,
    pub attr: *mut c_void,
    pub lo_water: u16,
    pub increment: u16,
    pub hi_water: u16,
    pub maximum: u16,
    pub tid_name: *const c_char,
    pub error_func: Option<unsafe extern "C" fn(c_uint, c_int)>,
    pub reserved: [c_uint; 5],
}

/// `thread_pool_t`: the attributes, then the pool's counts.
#[repr(C)]
pub struct ThreadPool {
    pub pool_attr: ThreadPoolAttr,
    pub created: c_uint,
    pub waiting: c_uint,
    pub flags: c_uint,
    /// The library's own state: [`crate::pool::State`].
    pub props: *mut c_void,
    pub reserved: [c_uint; 2],
}

/// `name_attach_t`.
#[repr(C)]
pub struct NameAttach {
    pub dpp: *mut crate::dispatch::Dispatch,
    pub chid: c_int,
    pub mntid: c_int,
    pub zero: [c_int; 2],
}

/// The return values of handlers (`<sys/resmgr.h>`).
pub const RESMGR_NOREPLY: c_int = c_int::MIN;
pub const RESMGR_DEFAULT: c_int = c_int::MIN + 1;

const _: () = {
    use core::mem::offset_of;
    assert!(size_of::<MsgInfo>() == 0x40);
    assert!(size_of::<Context>() == 0x98);
    assert!(offset_of!(Context, msg) == 0x48);
    assert!(offset_of!(Context, iov) == 0x88);
    assert!(size_of::<ConnectFuncs>() == 0x50);
    assert!(size_of::<IoFuncs>() == 0x110);
    assert!(size_of::<ResmgrAttr>() == 0x28);
    assert!(size_of::<IofuncMount>() == 0x50);
    assert!(size_of::<IofuncFuncs>() == 0x30);
    assert!(size_of::<IofuncAttr>() == 0x80);
    assert!(offset_of!(IofuncAttr, nbytes) == 0x38);
    assert!(offset_of!(IofuncAttr, mode) == 0x68);
    assert!(size_of::<IofuncOcb>() == 0x20);
    assert!(size_of::<NotifyEvent>() == 0x40);
    assert!(size_of::<ClientInfo>() == 0x4c);
    assert!(size_of::<ThreadPoolAttr>() == 0x68);
    assert!(size_of::<ThreadPool>() == 0x88);
    assert!(size_of::<NameAttach>() == 0x18);
};
