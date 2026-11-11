//! `pipe`: OsCraft's pipe manager, a native resource manager over the
//! kernel's pipes (`PipeOpen()`, `PipeCtl()`, `PipeClose()`: QNX 8's
//! `<sys/kpipe.h>`, which QNX's `kpipe` uses), in place of QNX's `pipe`.
//!
//! `pipe()` in a client (FreeBSD's through libsys, QNX's through its
//! libc) opens "/" for the file type `_FTYPE_PIPE`, then `_IO_OPENFD`
//! with `_IO_OPENFD_PIPE` on a new connection for the other end. This
//! manager answers both by binding the client's connection to a kernel
//! pipe: the kernel then does the reads and writes, the waits, SIGPIPE
//! and end of file, and `ionotify()`'s arming, without a message to the
//! manager. What still comes here: the opens, `_IO_DUP` (a `dup()`, or
//! a fork's inherited descriptors: the new connection is bound to the
//! same pipe), `_IO_NOTIFY` (passed on as `_NTO_PIPE_CTL_NOTIFY`),
//! `DCMD_ALL_SETFLAGS` (`O_NONBLOCK`, passed on as `_SETFLAGS`), `stat`
//! (a FIFO, mode 0777, as `kpipe`'s), `lseek` (ESPIPE) and the closes:
//! the pipe is forgotten (`PipeClose()`) with its last OCB, since no
//! further end or dup can be opened on it; the kernel frees it with its
//! last connection.
//!
//! 64 KB of buffer, as FreeBSD's pipes grow to, and `PIPE_BUF` by who
//! creates the pipe. Named pipes (`mkfifo()`) are not served yet.
//!
//! `/dev/pipe` is a device of nothing: it says the manager is
//! up (`waitfor`).

use core::ffi::{c_char, c_int, c_void};
use core::ptr::{null, null_mut};
use std::collections::HashMap;
use std::sync::Mutex;

use kercall::abi::Iov;
use kercall::neutrino::misc::{PipeClose, PipeCtl, PipeOpen};
use proto::connect::Connect;
use proto::io::{self, IO_OPENFD_PIPE};
use resmgr::abi::{
    ConnectFuncs, Context, IoFuncs, IofuncAttr, IofuncOcb, RESMGR_DEFAULT, ResmgrAttr,
};
use resmgr::dispatch::{
    dispatch_block, dispatch_context_alloc, dispatch_create, dispatch_handler,
};
use resmgr::iofunc::{
    iofunc_attr_init, iofunc_close_ocb_default, iofunc_pathconf_default, iofunc_stat_default, pathconf_reply, iofunc_devctl_default, iofunc_func_init,
    iofunc_ocb_attach, iofunc_unblock_default,
};
use resmgr::mem::{calloc_bytes, free};
use resmgr::resmgr::resmgr_attach;

const S_IFIFO: u32 = 0o010000;
const S_IFCHR: u32 = 0o020000;
/// `_FTYPE_PIPE` and `_RESMGR_FLAG_FTYPEONLY`: the manager of all
/// anonymous pipes.
const FTYPE_PIPE: u32 = 4;
const RESMGR_FLAG_FTYPEONLY: u32 = 0x8;
/// What `PipeOpen()` takes of an open's flags: `_IO_FLAG_RD`, `_WR`,
/// `O_NONBLOCK`.
const OPEN_FLAGS: u32 = 0x7;
const DCMD_ALL_SETFLAGS: i32 = 0x8004_0102u32 as i32;

/// The I/O functions' indexes (the message type from `_IO_READ`).
const IO_CLOSE_OCB: usize = 0x02;
const IO_STAT: usize = 0x03;
const IO_NOTIFY: usize = 0x04;
const IO_DEVCTL: usize = 0x05;
const IO_UNBLOCK: usize = 0x06;
const IO_PATHCONF: usize = 0x07;
const IO_LSEEK: usize = 0x08;
const IO_OPENFD: usize = 0x0c;
const IO_DUP: usize = 0x14;

/// `struct nto_pipe_cfg`: the buffer, the atomic write, the write
/// notification threshold (0: the atomic size), the type (bit 2:
/// duplex), the source buffer (0 or 1); 64 bytes.
#[repr(C)]
struct PipeCfg {
    bufsz: u32,
    chunksz: u32,
    wnotify: u32,
    ty: u32,
    source: u32,
    reserved: [u32; 11],
}

const BUFFER_SIZE: u32 = 65536;
/// What a write of at most this much is all or nothing: `PIPE_BUF`, by
/// who creates the pipe: FreeBSD's for a native program, QNX's for a QNX
/// one, which reads its own `PIPE_BUF` from its headers.
const ATOMIC_SIZE: u32 = 512;
const ATOMIC_SIZE_QNX: u32 = 5120;
/// `MI_NATIVE` in `_msg_info.flags`: the client is a native program.
const MI_NATIVE: u16 = 0x8000;

fn cfg(source: u32, ctp: *mut Context) -> PipeCfg {
    // SAFETY: the context is the framework's.
    let native = unsafe { (*ctp).info.flags } as u16 & MI_NATIVE != 0;
    PipeCfg {
        bufsz: BUFFER_SIZE,
        chunksz: if native { ATOMIC_SIZE } else { ATOMIC_SIZE_QNX },
        wnotify: 0,
        ty: 0,
        source,
        reserved: [0; 11],
    }
}

/// `union nto_pipe_ctl`'s controls: `{ id, len }` then the arguments.
const CTL_NOTIFY: u32 = 1;
const CTL_SETFLAGS: u32 = 3;
const CTL_UNBLOCK: u32 = 5;
const CTL_INFO: u32 = 0x100;

/// An open end of a pipe: the framework's OCB, then the kernel pipe's
/// ID and the end's source buffer (what a dup of this end is opened
/// with).
#[repr(C)]
struct PipeOcb {
    ocb: IofuncOcb,
    id: c_int,
    source: u32,
}

/// The open ends (OCBs) of each pipe: `PipeClose()` with the last.
static OPEN: Mutex<Option<HashMap<c_int, u32>>> = Mutex::new(None);

fn ends() -> std::sync::MutexGuard<'static, Option<HashMap<c_int, u32>>> {
    let mut g = OPEN.lock().unwrap();
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    g
}

/// Binds the client's connection (`scoid`) to pipe `id` (-1: a new
/// one) as `source` with `flags`, and an OCB for it to the connection,
/// through the connect message `open` (the open's flags). 0 or the
/// error number.
unsafe fn bind(
    ctp: *mut Context,
    open: *mut u8,
    scoid: c_int,
    id: c_int,
    source: u32,
    flags: u32,
) -> c_int {
    let mut c = cfg(source, ctp);
    let r = PipeOpen(scoid, id, (&raw mut c).cast(), flags);
    if r < 0 {
        return kercall::tls::get_errno();
    }
    let id = if id < 0 { r } else { id };
    let p = calloc_bytes(size_of::<PipeOcb>()).cast::<PipeOcb>();
    if p.is_null() {
        if r >= 0 && id == r {
            PipeClose(id);
        }
        return proto::errno::ENOMEM as c_int;
    }
    // SAFETY: allocated above, zeroed.
    unsafe {
        (*p).id = id;
        (*p).source = source;
    }
    *ends().as_mut().unwrap().entry(id).or_insert(0) += 1;
    // SAFETY: the framework's function with a connect message and the
    // shared attribute.
    let e = unsafe { iofunc_ocb_attach(ctp, open, p.cast(), &raw mut ATTR, null()) };
    if e != 0 {
        // The connection's binding goes with the connection; the OCB
        // did not get bound.
        drop_end(id);
        // SAFETY: allocated above, never bound.
        unsafe { free(p.cast()) };
    }
    e
}

/// One OCB of pipe `id` closed: the last forgets the pipe.
fn drop_end(id: c_int) {
    let mut g = ends();
    let m = g.as_mut().unwrap();
    if let Some(n) = m.get_mut(&id) {
        *n -= 1;
        if *n == 0 {
            m.remove(&id);
            PipeClose(id);
        }
    }
}

/// `_IO_CONNECT` of "/" for `_FTYPE_PIPE`: a new pipe, and this
/// connection bound to it as source 0.
unsafe extern "C" fn io_open(
    ctp: *mut Context,
    msg: *mut u8,
    _handle: *mut c_void,
    _extra: *mut c_void,
) -> c_int {
    // SAFETY: a connect message.
    let m = unsafe { msg.cast::<Connect>().read_unaligned() };
    if m.file_type != FTYPE_PIPE || m.path_len > 1 {
        return proto::errno::ENOENT as c_int;
    }
    // The access the OCB gets is the open's (the message's `access` is
    // what the path manager granted, nothing for a pipe).
    // SAFETY: the message's buffer holds the request.
    unsafe {
        msg.cast::<Connect>().write_unaligned(Connect {
            access: (m.ioflag & 3) as u16,
            ..m
        })
    };
    // SAFETY: the context is the framework's.
    let scoid = unsafe { (*ctp).info.scoid };
    // SAFETY: as above.
    unsafe { bind(ctp, msg, scoid, -1, 0, m.ioflag & OPEN_FLAGS) }
}

/// `_IO_OPENFD` with `_IO_OPENFD_PIPE`: the other end, on the new
/// connection the message came on, with the opposite source buffer.
unsafe extern "C" fn io_openfd(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an openfd message.
    let m = unsafe { msg.cast::<io::Openfd>().read_unaligned() };
    if m.xtype != IO_OPENFD_PIPE {
        return proto::errno::ENOSYS as c_int;
    }
    // SAFETY: an OCB this manager bound.
    let o = unsafe { &*ocb.cast::<PipeOcb>() };
    let mut open = Connect {
        ioflag: m.ioflag,
        sflag: m.sflag,
        access: (m.ioflag & 3) as u16,
        ..Default::default()
    };
    // SAFETY: the context is the framework's.
    let scoid = unsafe { (*ctp).info.scoid };
    // SAFETY: as above, with a connect message of the open's flags.
    unsafe {
        bind(
            ctp,
            (&raw mut open).cast(),
            scoid,
            o.id,
            1 - o.source,
            m.ioflag & OPEN_FLAGS,
        )
    }
}

/// `_IO_DUP`: the new connection is bound to the same pipe as the same
/// end (a fork's or `dup2()`'s copy comes bound: the kernel copies the
/// binding, and says so); the framework then shares the OCB.
unsafe extern "C" fn io_dup(ctp: *mut Context, _msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an OCB this manager bound.
    let o = unsafe { &*ocb.cast::<PipeOcb>() };
    let mut c = cfg(o.source, ctp);
    // SAFETY: the context is the framework's.
    let scoid = unsafe { (*ctp).info.scoid };
    if PipeOpen(
        scoid,
        o.id,
        (&raw mut c).cast(),
        o.ocb.ioflag as u32 & OPEN_FLAGS,
    ) < 0
    {
        return kercall::tls::get_errno();
    }
    0
}

/// `_IO_LSEEK`: a pipe has no position.
unsafe extern "C" fn io_lseek(_ctp: *mut Context, _msg: *mut u8, _ocb: *mut c_void) -> c_int {
    proto::errno::ESPIPE as c_int
}

/// The last close of an OCB: one end fewer.
unsafe extern "C" fn io_close_ocb(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an OCB this manager bound.
    let id = unsafe { (*ocb.cast::<PipeOcb>()).id };
    drop_end(id);
    // SAFETY: the framework's default with its own arguments.
    unsafe { iofunc_close_ocb_default(ctp, msg, ocb.cast()) }
}

/// `_IO_NOTIFY`: the kernel arms the client's event on the pipe
/// (`_NTO_PIPE_CTL_NOTIFY`: the action as the kernel's command, the
/// client, the conditions, the event, and where the ready conditions
/// go), and replies as the framework's `iofunc_notify()` would.
unsafe extern "C" fn io_notify(ctp: *mut Context, msg: *mut u8, _ocb: *mut c_void) -> c_int {
    // SAFETY: a notify message.
    let mut m = unsafe { msg.cast::<io::Notify>().read_unaligned() };
    let command: u32 = match m.action {
        io::NOTIFY_ACTION_TRANARM | io::NOTIFY_ACTION_EDGEARM => 1,
        io::NOTIFY_ACTION_CONDARM | io::NOTIFY_ACTION_POLLARM => 3,
        io::NOTIFY_ACTION_POLL => 6,
        _ => return proto::errno::EINVAL as c_int,
    };
    // SAFETY: the context is the framework's.
    let info = unsafe { &(*ctp).info };
    let mut ready: u32 = 0;
    let mut ctl = [0u8; 0x50];
    ctl[0..4].copy_from_slice(&CTL_NOTIFY.to_le_bytes());
    ctl[4..8].copy_from_slice(&0x50u32.to_le_bytes());
    ctl[8..12].copy_from_slice(&command.to_le_bytes());
    ctl[0x10..0x18].copy_from_slice(&(info.pid as u32 as u64).to_le_bytes());
    ctl[0x18..0x1c].copy_from_slice(&(info.tid as u32).to_le_bytes());
    ctl[0x1c..0x20].copy_from_slice(&m.flags.to_le_bytes());
    ctl[0x20..0x24].copy_from_slice(&m.flags_extra_mask.to_le_bytes());
    // SAFETY: the event's 32 bytes, in the message.
    let event = unsafe {
        core::slice::from_raw_parts(msg.add(core::mem::offset_of!(io::Notify, event)), 32)
    };
    ctl[0x28..0x48].copy_from_slice(event);
    ctl[0x48..0x50].copy_from_slice(&((&raw mut ready) as u64).to_le_bytes());
    if PipeCtl(info.scoid, ctl.as_mut_ptr().cast()) < 0 {
        return kercall::tls::get_errno();
    }
    m.action = ready as i32;
    // SAFETY: the message's buffer holds the request.
    unsafe { msg.cast::<io::Notify>().write_unaligned(m) };
    // SAFETY: the context is the framework's.
    unsafe { (*ctp).iov[0] = Iov::raw(msg.cast(), size_of::<io::Notify>()) };
    -1
}

/// The client's end: the bytes it can read, and the atomic size.
fn info(ctp: *mut Context) -> Result<(u32, u32), c_int> {
    let mut ctl = [0u32; 4];
    (ctl[0], ctl[1]) = (CTL_INFO, 16);
    // SAFETY: the context is the framework's.
    let scoid = unsafe { (*ctp).info.scoid };
    if PipeCtl(scoid, ctl.as_mut_ptr().cast()) < 0 {
        return Err(kercall::tls::get_errno());
    }
    Ok((ctl[2], ctl[3]))
}

/// `_IO_STAT`: the size is what can be read.
unsafe extern "C" fn io_stat(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let bytes = match info(ctp) {
        Ok((bytes, _)) => bytes,
        Err(e) => return e,
    };
    // SAFETY: one thread serves; the attribute is its alone.
    unsafe { (*(&raw mut ATTR)).nbytes = i64::from(bytes) };
    // SAFETY: the framework's default with its own arguments.
    unsafe { iofunc_stat_default(ctp, msg, ocb.cast()) }
}

/// `_IO_PATHCONF`: `_PC_PIPE_BUF` is this pipe's.
unsafe extern "C" fn io_pathconf(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    const PC_PIPE_BUF: i32 = 6;
    // SAFETY: a pathconf message.
    if unsafe { msg.cast::<io::Pathconf>().read_unaligned() }.name as i32 == PC_PIPE_BUF {
        return match info(ctp) {
            // SAFETY: the current message.
            Ok((_, atomic)) => unsafe { pathconf_reply(ctp, msg, i64::from(atomic)) },
            Err(e) => e,
        };
    }
    // SAFETY: the framework's default with its own arguments.
    unsafe { iofunc_pathconf_default(ctp, msg, ocb.cast()) }
}

/// `FIONREAD`'s reply.
unsafe fn fionread(ctp: *mut Context, msg: *mut u8) -> c_int {
    let bytes = match info(ctp) {
        Ok((bytes, _)) => bytes,
        Err(e) => return e,
    };
    // SAFETY: the message's buffer holds a devctl header and more.
    unsafe {
        let reply = msg.cast::<io::DevctlReply>();
        reply.write_unaligned(io::DevctlReply { zero: 0, ret_val: 0, nbytes: 4, zero2: 0 });
        msg.add(size_of::<io::DevctlReply>()).cast::<u32>().write_unaligned(bytes);
        (*ctp).iov[0] = Iov::raw(msg.cast(), size_of::<io::DevctlReply>() + 4);
    }
    -1
}

/// `_IO_DEVCTL`: the framework keeps the flags; `O_NONBLOCK` also goes
/// to the kernel's binding (`_NTO_PIPE_CTL_SETFLAGS`).
unsafe extern "C" fn io_devctl(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a devctl message.
    let m = unsafe { msg.cast::<io::Devctl>().read_unaligned() };
    if m.dcmd & 0xffff == 0x667f {
        // SAFETY: the context and message are the framework's.
        return unsafe { fionread(ctp, msg) };
    }
    // SAFETY: the framework's default with its own arguments.
    let r = unsafe { iofunc_devctl_default(ctp, msg, ocb.cast()) };
    if r == 0 && m.dcmd == DCMD_ALL_SETFLAGS {
        // SAFETY: the flags follow the header, checked by the default.
        let flags = unsafe {
            msg.add(size_of::<io::Devctl>())
                .cast::<u32>()
                .read_unaligned()
        };
        let mut ctl = [0u8; 0x10];
        ctl[0..4].copy_from_slice(&CTL_SETFLAGS.to_le_bytes());
        ctl[4..8].copy_from_slice(&0x10u32.to_le_bytes());
        ctl[8..12].copy_from_slice(&flags.to_le_bytes());
        // SAFETY: the context is the framework's.
        let scoid = unsafe { (*ctp).info.scoid };
        if PipeCtl(scoid, ctl.as_mut_ptr().cast()) < 0 {
            return kercall::tls::get_errno();
        }
    }
    r
}

/// `_IO_UNBLOCK`: the client's threads blocked in the pipe leave it.
unsafe extern "C" fn io_unblock(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let mut ctl = [0u8; 8];
    ctl[0..4].copy_from_slice(&CTL_UNBLOCK.to_le_bytes());
    ctl[4..8].copy_from_slice(&8u32.to_le_bytes());
    // SAFETY: the context is the framework's.
    let scoid = unsafe { (*ctp).info.scoid };
    PipeCtl(scoid, ctl.as_mut_ptr().cast());
    // SAFETY: the framework's default with its own arguments.
    unsafe { iofunc_unblock_default(ctp, msg, ocb.cast()) }
}

static mut CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
static mut IO: IoFuncs = unsafe { core::mem::zeroed() };
static mut ATTR: IofuncAttr = unsafe { core::mem::zeroed() };
static mut DEV_CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
static mut DEV_IO: IoFuncs = unsafe { core::mem::zeroed() };
static mut DEV_ATTR: IofuncAttr = unsafe { core::mem::zeroed() };

fn main() {
    // SAFETY: the framework's functions, with statics of this program.
    unsafe {
        let dpp = dispatch_create();
        if dpp.is_null() {
            eprintln!("pipe: dispatch_create failed");
            std::process::exit(1);
        }
        iofunc_func_init(9, &raw mut CONNECT, 33, &raw mut IO);
        (*(&raw mut CONNECT)).funcs[0] = Some(io_open);
        let io = &mut *(&raw mut IO);
        io.funcs[IO_CLOSE_OCB] = Some(io_close_ocb);
        io.funcs[IO_NOTIFY] = Some(io_notify);
        io.funcs[IO_DEVCTL] = Some(io_devctl);
        io.funcs[IO_UNBLOCK] = Some(io_unblock);
        io.funcs[IO_STAT] = Some(io_stat);
        io.funcs[IO_PATHCONF] = Some(io_pathconf);
        io.funcs[IO_LSEEK] = Some(io_lseek);
        io.funcs[IO_OPENFD] = Some(io_openfd);
        io.funcs[IO_DUP] = Some(io_dup);
        iofunc_attr_init(&raw mut ATTR, S_IFIFO | 0o777, null(), null());
        let attr = ResmgrAttr {
            flags: 0,
            nparts_max: 1,
            msg_max_size: 4096,
            other_func: None,
            reserved: [0; 4],
        };
        let id = resmgr_attach(
            dpp,
            &attr,
            null::<c_char>(),
            FTYPE_PIPE,
            RESMGR_FLAG_FTYPEONLY,
            &raw const CONNECT,
            &raw const IO,
            (&raw mut ATTR).cast::<c_void>(),
        );
        if id < 0 {
            eprintln!(
                "pipe: resmgr_attach(pipes) failed: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
        iofunc_func_init(9, &raw mut DEV_CONNECT, 33, &raw mut DEV_IO);
        iofunc_attr_init(&raw mut DEV_ATTR, S_IFCHR | 0o444, null(), null());
        let id = resmgr_attach(
            dpp,
            &attr,
            c"/dev/pipe".as_ptr(),
            0,
            0,
            &raw const DEV_CONNECT,
            &raw const DEV_IO,
            (&raw mut DEV_ATTR).cast::<c_void>(),
        );
        if id < 0 {
            eprintln!(
                "pipe: resmgr_attach(/dev/pipe) failed: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
        let ctp = dispatch_context_alloc(dpp);
        if ctp.is_null() {
            eprintln!("pipe: dispatch_context_alloc failed");
            std::process::exit(1);
        }
        loop {
            if dispatch_block(ctp).is_null() {
                eprintln!(
                    "pipe: dispatch_block failed: {}",
                    std::io::Error::last_os_error()
                );
                std::process::exit(1);
            }
            dispatch_handler(ctp);
        }
    }
    #[allow(unreachable_code)]
    {
        let _ = (RESMGR_DEFAULT, null_mut::<c_void>());
    }
}
