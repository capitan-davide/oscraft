//! The server: what `libsys` (`user/libsys/src/socket`) and QNX's `libsocket`
//! send to `io-sock`, for FreeBSD's sockets. A socket is a connection to
//! `/dev/socket/<domain>`, and each open has an OCB holding a `struct socket`.
//!
//! A call that waits holds its thread until it's done, and the framework's
//! OCB lock is off so that other clients don't wait with it: threads are
//! added when none is left to receive.

use core::ffi::{c_int, c_long, c_void};
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use kercall::abi::Iov;
use kercall::neutrino::msg::{MsgDeliverEvent, MsgRead, MsgReply, MsgReplyv};
use proto::connect::Connect;
use proto::io::{self, Devctl, DevctlReply, Notify, Openfd, ReadWrite};
use resmgr::abi::{ConnectFuncs, Context, IoFuncs, IofuncAttr, IofuncOcb, RESMGR_NOREPLY, ResmgrAttr};
use resmgr::dispatch::{dispatch_block, dispatch_context_alloc, dispatch_create, dispatch_handler, Dispatch};
use resmgr::iofunc::{iofunc_attr_init, iofunc_close_ocb_default, iofunc_devctl_default, iofunc_func_init, iofunc_ocb_attach};
use resmgr::mem::{calloc_bytes, free};
use resmgr::resmgr::{resmgr_attach, resmgr_msgread};

use bsdkern::privilege::{self, Table};

use crate::unix::{Name, Right, Unix};

const S_IFSOCK: u32 = 0o140000;
const MAX_THREADS: u32 = 128;
const NAME_ROOM: usize = 128;

// The I/O functions' indexes: the message type from `_IO_READ`.
const F_READ: usize = 0x00;
const F_WRITE: usize = 0x01;
const F_CLOSE_OCB: usize = 0x02;
const F_NOTIFY: usize = 0x04;
const F_DEVCTL: usize = 0x05;
const F_OPENFD: usize = 0x0c;
const F_UNBLOCK: usize = 0x06;
const F_MSG: usize = 0x12;
const F_LOCK_OCB: usize = 0x16;
const F_UNLOCK_OCB: usize = 0x17;

const EINVAL: c_int = 22;
const ENOTTY: c_int = 25;
const ENOMEM: c_int = 12;
const ENOSYS: c_int = 78;
const EMSGSIZE: c_int = 40;
const XTYPE_NONE: u32 = 0;
const XTYPE_TCPIP: u32 = 3;
const XTYPE_TCPIP_MSG: u32 = 4;
const XTYPE_TCPIP_MSG2: u32 = 7;
const XTYPE_MASK: u32 = 0xff;
const MSG_CTRUNC: c_int = 0x20;
const OPENFD_ACCEPT: u16 = 3;
const O_NONBLOCK: u32 = 0x4;
const DCMD_ALL_SETFLAGS: i32 = 0x8004_0102u32 as i32;
const POLLIN: c_int = 0x1;
const POLLPRI: c_int = 0x2;
const POLLOUT: c_int = 0x4;
const AF_UNIX: c_int = 1;
const EOPNOTSUPP: c_int = 45;
const ENOPROTOOPT: c_int = 42;
const SOL_SOCKET: u32 = 0xffff;
const SO_TYPE: u32 = 0x1008;
const SO_ERROR: u32 = 0x1007;
const SO_SNDBUF: u32 = 0x1001;
const SO_RCVBUF: u32 = 0x1002;
const SCM_RIGHTS: i32 = 1;
const CMSG_HDR: usize = 16;
const WIRE_FD: usize = 16;
/// `sigev_value` in a `struct sigevent`.
const EVENT_VALUE: usize = 16;
const COND_OBAND: u32 = 0x4000_0000;
const COND_OUTPUT: u32 = 0x2000_0000;
const COND_INPUT: u32 = 0x1000_0000;

// `DCMD_*` of `io-sock`, by their low half.
const DCMD_GETSOCKNAME: u32 = 0x0600;
const DCMD_BIND: u32 = 0x0601;
const DCMD_GETPEERNAME: u32 = 0x0602;
const DCMD_CONNECT: u32 = 0x0603;
const DCMD_LISTEN: u32 = 0x0604;
const DCMD_SHUTDOWN: u32 = 0x0605;
const FIONREAD: u32 = 0x667f;

type So = *mut c_void;

unsafe extern "C" {
    fn bsdnet_socket(d: c_int, t: c_int, p: c_int, so: *mut So) -> c_int;
    fn bsdnet_close(so: So) -> c_int;
    fn bsdnet_bind(so: So, a: *const u8) -> c_int;
    fn bsdnet_connect(so: So, a: *const u8) -> c_int;
    fn bsdnet_listen(so: So, backlog: c_int) -> c_int;
    fn bsdnet_accept(so: So, out: *mut So, a: *mut u8) -> c_int;
    fn bsdnet_send(so: So, b: *const u8, n: *mut usize, to: *const u8, f: c_int) -> c_int;
    fn bsdnet_recv(so: So, b: *mut u8, n: *mut usize, from: *mut u8, f: *mut c_int) -> c_int;
    fn bsdnet_sockaddr(so: So, a: *mut u8, peer: c_int) -> c_int;
    fn bsdnet_shutdown(so: So, how: c_int) -> c_int;
    fn bsdnet_setopt(so: So, level: c_int, name: c_int, v: *mut u8, n: usize) -> c_int;
    fn bsdnet_getopt(so: So, level: c_int, name: c_int, v: *mut u8, n: *mut usize) -> c_int;
    fn bsdnet_nbio(so: So, on: c_int);
    fn bsdnet_watch(so: So, write: c_int) -> *mut c_void;
    fn bsdnet_poll(so: So, events: c_int) -> c_int;
    fn bsdnet_ioctl(so: So, cmd: u64, data: *mut u8) -> c_int;
    fn bsdnet_nread(so: So) -> c_int;
    fn bsdnet_self() -> *mut c_void;
    fn bsdnet_client(uid: u32, gid: u32);
    fn bsdnet_interrupt(td: *mut c_void) -> c_int;
    fn bsdnet_sysctl(name: *mut c_int, n: u32, old: *mut u8, oldlen: *mut usize, new: *mut u8, newlen: usize) -> c_int;
}

/// An open socket.
#[repr(C)]
struct SockOcb {
    ocb: IofuncOcb,
    so: So,
    unix: Option<Arc<Unix>>,
}

/// A client's `poll()`, waiting on one side of a socket.
struct Armed {
    sel: usize,
    /// The client process's connection to this server, and its descriptor:
    /// a `poll()` of several sockets has an entry for each.
    scoid: c_int,
    coid: c_int,
    /// The `poll()` it is for: QNX's `_poll` numbers an event by the call's
    /// serial plus the entry's index, and a socket can be in two entries.
    call: u32,
    value: u32,
    rcvid: i64,
    event: [u8; 32],
    cond: u32,
}

// SAFETY: plain data.
unsafe impl Send for Armed {}

static ARMED: Mutex<Vec<Armed>> = Mutex::new(Vec::new());
static IDLE: AtomicU32 = AtomicU32::new(0);
static THREADS: AtomicU32 = AtomicU32::new(0);
static mut CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
static mut IO: IoFuncs = unsafe { core::mem::zeroed() };
static mut ATTR: IofuncAttr = unsafe { core::mem::zeroed() };

/// FreeBSD's `selwakeuppri()`: `bsdkern_sel_wakeup` in `kern/data.c`.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_sel_wakeup(sel: *mut c_void) {
    let mut armed = ARMED.lock().unwrap();
    armed.retain(|a| {
        if a.sel != sel as usize {
            return true;
        }
        // QNX's `_poll` takes the conditions from the event's value: its
        // top bits, above the descriptor's index (read with REA).
        let mut event = a.event;
        let at = EVENT_VALUE;
        let value = u32::from_le_bytes(event[at..at + 4].try_into().unwrap()) | a.cond;
        event[at..at + 4].copy_from_slice(&value.to_le_bytes());
        MsgDeliverEvent(a.rcvid, event.as_ptr().cast());
        false
    });
}

/// FreeBSD's privileges as `io-sock`'s abilities (`qnx_priv_init`,
/// `qnx_priv_grant`, read with REA); any other is `network/admin`.
static PRIVILEGES: Table = Table {
    abilities: &[
        c"network/privport",
        c"network/rawsocket",
        c"network/reuseport",
        c"network/ipsec",
        c"network/admin",
        c"network/jailroot",
    ],
    rules: &[(490, 0), (395, 1), (497, 1), (504, 2), (503, 3), (113, 5)],
    // `PRIV_NET_BPF`, and what FreeBSD itself lets anyone do.
    free: &[394],
    other: 4,
};

/// The messages being served, and the thread on each, for an unblock.
static SERVING: Mutex<Vec<(i64, usize)>> = Mutex::new(Vec::new());

/// A message being served, while it lasts.
struct Serving(i64);

impl Serving {
    fn new(ctp: *mut Context) -> Serving {
        // SAFETY: the context is the framework's; FreeBSD's call.
        let (rcvid, td, scoid) = unsafe { ((*ctp).rcvid, bsdnet_self(), (*ctp).info.scoid) };
        SERVING.lock().unwrap().push((rcvid, td as usize));
        privilege::serve(Some(scoid));
        if let Some((uid, gid)) = privilege::identity(scoid) {
            // SAFETY: FreeBSD's call, for this thread.
            unsafe { bsdnet_client(uid, gid) };
        }
        Serving(rcvid)
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        SERVING.lock().unwrap().retain(|s| s.0 != self.0);
        privilege::serve(None);
    }
}

/// `accept()` blocked: its new connection has no OCB yet.
unsafe extern "C" fn unblock_unbound(ctp: *mut Context) -> c_int {
    interrupt(ctp, None)
}

/// The client of message `rcvid` wants out: `_PULSE_CODE_UNBLOCK`.
unsafe extern "C" fn io_unblock(ctp: *mut Context, _msg: *mut u8, ocb: *mut c_void) -> c_int {
    interrupt(ctp, ocb_unix(ocb))
}

/// The thread on the message may not be asleep yet, so this tries until the
/// message is done.
fn interrupt(ctp: *mut Context, unix: Option<Arc<Unix>>) -> c_int {
    // SAFETY: the framework sets the context to the message concerned.
    let rcvid = unsafe { (*ctp).rcvid };
    for _ in 0..1000 {
        let td = SERVING.lock().unwrap().iter().find(|s| s.0 == rcvid).map(|s| s.1);
        let Some(td) = td else {
            break;
        };
        if let Some(u) = &unix {
            u.interrupt();
        } else {
            // SAFETY: the thread is alive: `SERVING` lists it.
            unsafe { bsdnet_interrupt(td as *mut c_void) };
        }
        std::thread::sleep(core::time::Duration::from_millis(1));
    }
    RESMGR_NOREPLY
}

fn ocb_unix(ocb: *mut c_void) -> Option<Arc<Unix>> {
    // SAFETY: an OCB of this server.
    unsafe { (*ocb.cast::<SockOcb>()).unix.clone() }
}

/// An `AF_UNIX` end changed: its waiters in `poll()` (keyed by the end).
pub fn wake(end: &Arc<Unix>, write: bool) {
    bsdkern_sel_wakeup((Arc::as_ptr(end) as usize + usize::from(write)) as *mut c_void);
}

/// A socket of FreeBSD's for what a `AF_LOCAL` one can't do.
fn helper() -> So {
    static HELPER: Mutex<usize> = Mutex::new(0);
    let mut h = HELPER.lock().unwrap();
    if *h == 0 {
        let mut so = null_mut();
        // SAFETY: FreeBSD's call.
        unsafe { bsdnet_socket(2, 2, 0, &raw mut so) };
        *h = so as usize;
    }
    *h as So
}

/// What a wake-up of one side of a socket is known by.
fn ocb_sel(ocb: *mut c_void, write: c_int) -> usize {
    match ocb_unix(ocb) {
        Some(u) => Arc::as_ptr(&u) as usize + write as usize,
        // SAFETY: FreeBSD's structure.
        None => unsafe { bsdnet_watch(ocb_socket(ocb), write) as usize },
    }
}

fn ocb_socket(ocb: *mut c_void) -> So {
    // SAFETY: an OCB of this server.
    unsafe { (*ocb.cast::<SockOcb>()).so }
}

/// Binds a new OCB for `so` to the client's connection.
///
/// # Safety
///
/// `open` is a connect message in the framework's buffer or on the stack.
unsafe fn bind_socket(ctp: *mut Context, open: *mut u8, so: So, unix: Option<Arc<Unix>>) -> c_int {
    let p = calloc_bytes(size_of::<SockOcb>()).cast::<SockOcb>();
    if p.is_null() {
        return ENOMEM;
    }
    // SAFETY: allocated above; the framework's function.
    unsafe {
        (&raw mut (*p).so).write(so);
        (&raw mut (*p).unix).write(unix);
        let e = iofunc_ocb_attach(ctp, open, p.cast(), &raw mut ATTR, null());
        if e != 0 {
            core::ptr::drop_in_place(&raw mut (*p).unix);
            free(p.cast());
        }
        e
    }
}

/// A client's socket address, before FreeBSD's code reads it: its `sa_len`
/// is the length the call gave, whatever the client wrote there, as the
/// kernel's `getsockaddr()` sets it. Programs may leave it zero (Rust's
/// standard library does), and the protocols check it (`EINVAL`).
fn own_length(address: &mut [u8]) {
    if let Ok(len) = u8::try_from(address.len())
        && len >= 2
    {
        address[0] = len;
    }
}

/// `socket()`: an open of `/dev/socket/<domain>`, the type and protocol as
/// its extra data; the domain is in the attachment.
unsafe extern "C" fn io_open(ctp: *mut Context, msg: *mut u8, handle: *mut c_void, extra: *mut c_void) -> c_int {
    let _serving = Serving::new(ctp);
    let _ = handle;
    let domain = domain_of(ctp);
    let (mut kind, mut protocol) = (0, 0);
    if !extra.is_null() {
        // SAFETY: `SocketExtra`, the extra data of a socket's open.
        unsafe {
            kind = extra.cast::<c_int>().read_unaligned();
            protocol = extra.cast::<c_int>().add(1).read_unaligned();
        }
    }
    let mut so = null_mut();
    if domain == AF_UNIX {
        // SAFETY: a connect message.
        let m = unsafe { msg.cast::<Connect>().read_unaligned() };
        // SAFETY: as above.
        unsafe { msg.cast::<Connect>().write_unaligned(Connect { access: 3, ..m }) };
        // SAFETY: the framework's buffer.
        return unsafe { bind_socket(ctp, msg, null_mut(), Some(Unix::new(kind))) };
    }
    // SAFETY: FreeBSD's call.
    let e = unsafe { bsdnet_socket(domain, kind, protocol, &raw mut so) };
    if e != 0 {
        return e;
    }
    // SAFETY: a connect message.
    let m = unsafe { msg.cast::<Connect>().read_unaligned() };
    // SAFETY: as above.
    unsafe { msg.cast::<Connect>().write_unaligned(Connect { access: 3, ..m }) };
    // SAFETY: the framework's buffer.
    let e = unsafe { bind_socket(ctp, msg, so, None) };
    if e != 0 {
        // SAFETY: never bound.
        unsafe { bsdnet_close(so) };
    }
    e
}

static DOMAINS: Mutex<Vec<(c_int, c_int)>> = Mutex::new(Vec::new());

fn domain_of(ctp: *mut Context) -> c_int {
    // SAFETY: the context is the framework's.
    let id = unsafe { (*ctp).id };
    DOMAINS.lock().unwrap().iter().find(|d| d.0 == id).map_or(0, |d| d.1)
}

unsafe extern "C" fn io_close_ocb(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let so = ocb_socket(ocb);
    let watched = [ocb_sel(ocb, 0), ocb_sel(ocb, 1)];
    ARMED.lock().unwrap().retain(|a| !watched.contains(&a.sel));
    // SAFETY: this was the last user.
    unsafe {
        if let Some(u) = (*ocb.cast::<SockOcb>()).unix.take() {
            u.shutdown(2);
        } else {
            bsdnet_close(so);
        }
        iofunc_close_ocb_default(ctp, msg, ocb.cast())
    }
}

/// The whole message from `offset`, `len` bytes.
///
/// # Safety
///
/// The context is the framework's.
unsafe fn read_all(ctp: *mut Context, offset: usize, len: usize) -> Result<Vec<u8>, c_int> {
    let mut v = vec![0u8; len];
    // SAFETY: the buffer is `len` bytes.
    let got = unsafe { resmgr_msgread(ctp, v.as_mut_ptr().cast(), len, offset) };
    if got < 0 {
        return Err(EINVAL);
    }
    v.truncate(got as usize);
    Ok(v)
}

fn reply(ctp: *mut Context, status: c_long, data: &[u8]) -> c_int {
    // SAFETY: the context is the framework's.
    MsgReply(unsafe { (*ctp).rcvid }, status, data.as_ptr().cast(), data.len());
    RESMGR_NOREPLY
}

/// `write()`, `sendto()` and `sendmsg()`.
unsafe extern "C" fn io_write(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let _serving = Serving::new(ctp);
    let so = ocb_socket(ocb);
    // SAFETY: a write message, and more by its xtype.
    let w = unsafe { msg.cast::<ReadWrite>().read_unaligned() };
    let nbytes = (w.nbytes_hi as usize) << 32 | w.nbytes as usize;
    let (head, flags, namelen, control) = match w.xtype & XTYPE_MASK {
        XTYPE_NONE => (size_of::<ReadWrite>(), 0, 0, 0),
        XTYPE_TCPIP => {
            // SAFETY: `flags`, `addrlen` after the header.
            let (f, a) = unsafe { two(msg, size_of::<ReadWrite>()) };
            (size_of::<ReadWrite>() + 8, f, a as usize, 0)
        }
        XTYPE_TCPIP_MSG => {
            // SAFETY: `flags`, `addrlen`, `controllen` after the header.
            let (f, l) = unsafe { two(msg, size_of::<ReadWrite>()) };
            (size_of::<ReadWrite>() + 8, f, (l & 0xffff) as usize, (l as usize >> 16) & 0xffff)
        }
        _ => return ENOSYS,
    };
    let unix = ocb_unix(ocb);
    if control != 0 && unix.is_none() {
        return unsupported();
    }
    // SAFETY: the context is the framework's.
    let mut body = match unsafe { read_all(ctp, head, namelen + control + nbytes) } {
        Ok(b) => b,
        Err(e) => return e,
    };
    if body.len() < namelen + control {
        return EMSGSIZE;
    }
    if unix.is_none() && namelen != 0 {
        own_length(&mut body[..namelen]);
    }
    if let Some(u) = unix {
        let rights = match rights_of(&body[namelen..namelen + control]) {
            Ok(r) => r,
            Err(e) => return e,
        };
        return match u.send(&body[namelen + control..], rights, flags) {
            Ok(n) => reply(ctp, n as c_long, &[]),
            Err(e) => e,
        };
    }
    let to = if namelen == 0 { null() } else { body.as_ptr() };
    let mut n = body.len() - namelen;
    // SAFETY: FreeBSD's call, over this buffer.
    let e = unsafe { bsdnet_send(so, body[namelen..].as_ptr(), &raw mut n, to, flags) };
    if e != 0 && n == 0 {
        return e;
    }
    reply(ctp, n as c_long, &[])
}

/// Control data on a socket of FreeBSD's stack: not yet.
const fn unsupported() -> c_int {
    EOPNOTSUPP
}

/// The descriptors in the wire form of `SCM_RIGHTS` going out, 8 bytes each,
/// imported.
fn rights_of(mut control: &[u8]) -> Result<Vec<Right>, c_int> {
    let mut out = Vec::new();
    while control.len() >= CMSG_HDR {
        let len = u32::from_le_bytes(control[..4].try_into().unwrap()) as usize;
        let level = i32::from_le_bytes(control[4..8].try_into().unwrap());
        let kind = i32::from_le_bytes(control[8..12].try_into().unwrap());
        if len < CMSG_HDR || len > control.len() {
            break;
        }
        if level == SOL_SOCKET as i32 && kind == SCM_RIGHTS {
            for h in control[CMSG_HDR..len].chunks_exact(8) {
                let fd = crate::rights::import(u64::from_le_bytes(h.try_into().unwrap()))?;
                out.push(Right { fd, handle: 0 });
            }
        }
        control = &control[((len + 7) & !7).min(control.len())..];
    }
    Ok(out)
}

/// The wire form of `SCM_RIGHTS` coming in: 16 bytes each, the handle last.
fn rights_wire(handles: &[u64]) -> Vec<u8> {
    if handles.is_empty() {
        return Vec::new();
    }
    let len = CMSG_HDR + handles.len() * WIRE_FD;
    let mut w = vec![0u8; len];
    w[..4].copy_from_slice(&(len as u32).to_le_bytes());
    w[4..8].copy_from_slice(&(SOL_SOCKET as i32).to_le_bytes());
    w[8..12].copy_from_slice(&SCM_RIGHTS.to_le_bytes());
    for (i, h) in handles.iter().enumerate() {
        w[CMSG_HDR + i * WIRE_FD + 8..CMSG_HDR + (i + 1) * WIRE_FD].copy_from_slice(&h.to_le_bytes());
    }
    w
}

/// `read()` and the rest on an `AF_UNIX` end. In `recvmsg()`'s form the
/// reply's parts are the reply, the room for control data, then the data.
fn unix_read(ctp: *mut Context, msg: *mut u8, u: &Unix, kind: u32, nbytes: usize, flags: c_int) -> c_int {
    let msg2 = kind == XTYPE_TCPIP_MSG2;
    // SAFETY: `controllen` and on, in a `recvmsg()`'s request.
    let room = if msg2 { unsafe { msg.add(size_of::<ReadWrite>() + 8).cast::<u32>().read_unaligned() as usize } } else { 0 };
    // SAFETY: the context is the framework's.
    let pid = unsafe { (*ctp).info.pid };
    let got = match u.recv(nbytes, flags, pid) {
        Ok(g) => g,
        Err(e) => return e,
    };
    let wire = rights_wire(&got.handles);
    let lead = match kind {
        XTYPE_TCPIP => 4,
        XTYPE_TCPIP_MSG2 => 20,
        _ => 0,
    };
    let mut out = vec![0u8; lead + room];
    let fits = wire.len().min(room);
    out[lead..lead + fits].copy_from_slice(&wire[..fits]);
    if msg2 {
        let cut = if wire.len() > room { MSG_CTRUNC } else { 0 };
        out[..4].copy_from_slice(&cut.to_le_bytes());
        out[8..12].copy_from_slice(&(fits as u32).to_le_bytes());
        out[12..16].copy_from_slice(&(wire.len() as u32).to_le_bytes());
    }
    out.extend_from_slice(&got.data);
    reply(ctp, got.data.len() as c_long, &out)
}

/// Two 32-bit fields at `at`.
///
/// # Safety
///
/// The message holds them.
unsafe fn two(msg: *mut u8, at: usize) -> (c_int, u32) {
    // SAFETY: the caller's guarantee.
    unsafe { (msg.add(at).cast::<c_int>().read_unaligned(), msg.add(at + 4).cast::<u32>().read_unaligned()) }
}

/// `read()`, `recvfrom()` and `recvmsg()`.
unsafe extern "C" fn io_read(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let _serving = Serving::new(ctp);
    let so = ocb_socket(ocb);
    // SAFETY: a read message, and more by its xtype.
    let r = unsafe { msg.cast::<ReadWrite>().read_unaligned() };
    let nbytes = (r.nbytes_hi as usize) << 32 | r.nbytes as usize;
    let kind = r.xtype & XTYPE_MASK;
    let (mut flags, room) = match kind {
        XTYPE_NONE => (0, 0usize),
        XTYPE_TCPIP | XTYPE_TCPIP_MSG2 => {
            // SAFETY: `flags`, `addrlen` after the header.
            let (f, a) = unsafe { two(msg, size_of::<ReadWrite>()) };
            (f, a as usize)
        }
        _ => return ENOSYS,
    };
    if let Some(u) = ocb_unix(ocb) {
        return unix_read(ctp, msg, &u, kind, nbytes, flags);
    }
    let lead = match kind {
        XTYPE_NONE => 0,
        XTYPE_TCPIP => 4,
        _ => 20,
    };
    let mut out = vec![0u8; lead + room + nbytes];
    let mut name = [0u8; NAME_ROOM];
    let mut n = nbytes;
    let from = if room == 0 { null_mut() } else { name.as_mut_ptr() };
    // SAFETY: FreeBSD's call, over these buffers.
    let e = unsafe { bsdnet_recv(so, out[lead + room..].as_mut_ptr(), &raw mut n, from, &raw mut flags) };
    if e != 0 && n == 0 {
        return e;
    }
    let got = (name[0] as usize).min(room).min(NAME_ROOM);
    out[lead + room..].copy_within(0..0, 0);
    if room != 0 {
        out[lead..lead + got].copy_from_slice(&name[..got]);
    }
    if kind == XTYPE_TCPIP {
        out[..4].copy_from_slice(&(got as u32).to_le_bytes());
    } else if kind == XTYPE_TCPIP_MSG2 {
        out[..4].copy_from_slice(&flags.to_le_bytes());
        out[4..8].copy_from_slice(&(got as u32).to_le_bytes());
    }
    out.truncate(lead + room + n);
    reply(ctp, n as c_long, &out)
}

/// A device command's reply: `ret_val`, then `data`.
fn dev_reply(ctp: *mut Context, ret: i32, data: &[u8]) -> c_int {
    let head = DevctlReply { zero: 0, ret_val: ret, nbytes: data.len() as u32, zero2: 0 };
    let parts = [Iov::raw((&raw const head).cast(), size_of::<DevctlReply>()), Iov::raw(data.as_ptr().cast(), data.len())];
    // SAFETY: the context is the framework's.
    MsgReplyv(unsafe { (*ctp).rcvid }, 0, parts.as_ptr().cast(), 2);
    RESMGR_NOREPLY
}

unsafe extern "C" fn io_devctl(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let _serving = Serving::new(ctp);
    let so = ocb_socket(ocb);
    let unix = ocb_unix(ocb);
    // SAFETY: a devctl message.
    let d = unsafe { msg.cast::<Devctl>().read_unaligned() };
    let low = d.dcmd as u32 & 0xffff;
    if d.dcmd == DCMD_ALL_SETFLAGS || (d.dcmd as u32 >> 8) & 0xff == 1 {
        // SAFETY: the framework's default with its own arguments.
        let r = unsafe { iofunc_devctl_default(ctp, msg, ocb.cast()) };
        if r == 0 && d.dcmd == DCMD_ALL_SETFLAGS {
            // SAFETY: the flags follow the header, checked by the default.
            let flags = unsafe { msg.add(size_of::<Devctl>()).cast::<u32>().read_unaligned() };
            if let Some(u) = &unix {
                u.set_nonblock(flags & O_NONBLOCK != 0);
            } else {
                // SAFETY: FreeBSD's call.
                unsafe { bsdnet_nbio(so, c_int::from(flags & O_NONBLOCK != 0)) };
            }
        }
        return r;
    }
    let size = (d.dcmd as u32 >> 16) as usize & 0x1fff;
    // SAFETY: the context is the framework's.
    let mut data = match unsafe { read_all(ctp, size_of::<Devctl>(), size.max(d.nbytes as usize)) } {
        Ok(b) => b,
        Err(e) => return e,
    };
    data.resize(size.max(NAME_ROOM), 0);
    let int = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    // SAFETY: an `ifioctl()` for any socket's sake.
    if let Some(u) = &unix { unsafe {
        return match low {
            DCMD_SHUTDOWN => {
                u.shutdown(int);
                dev_reply(ctp, 0, &[])
            }
            FIONREAD => dev_reply(ctp, 0, &0i32.to_le_bytes()),
            DCMD_BIND | DCMD_CONNECT => {
                let done = Name::parse(&data).and_then(|n| if low == DCMD_BIND { u.bind(n) } else { u.connect(&n) });
                match done {
                    Ok(()) => dev_reply(ctp, 0, &[]),
                    Err(e) => e,
                }
            }
            DCMD_LISTEN => match u.listen(int) {
                Ok(()) => dev_reply(ctp, 0, &[]),
                Err(e) => e,
            },
            // The family alone: a name is the file system's.
            DCMD_GETSOCKNAME | DCMD_GETPEERNAME => dev_reply(ctp, 2, &[2, AF_UNIX as u8]),
            // ifconfig makes an AF_LOCAL socket for its interface requests.
            _ => {
                let e = bsdnet_ioctl(helper(), d.dcmd as u32 as u64, data.as_mut_ptr());
                if e == 0 { dev_reply(ctp, 0, &data[..size]) } else { e }
            }
        };
    } }
    if matches!(low, DCMD_BIND | DCMD_CONNECT) {
        // The command's size is the address's length (libsys's `name_cmd()`).
        own_length(&mut data[..size]);
    }
    // SAFETY: FreeBSD's calls, over these buffers.
    let e = unsafe {
        match low {
            DCMD_BIND => bsdnet_bind(so, data.as_ptr()),
            DCMD_CONNECT => bsdnet_connect(so, data.as_ptr()),
            DCMD_LISTEN => bsdnet_listen(so, int),
            DCMD_SHUTDOWN => bsdnet_shutdown(so, int),
            DCMD_GETSOCKNAME | DCMD_GETPEERNAME => {
                let e = bsdnet_sockaddr(so, data.as_mut_ptr(), c_int::from(low == DCMD_GETPEERNAME));
                if e != 0 {
                    return e;
                }
                let n = (data[0] as usize).min(size);
                return dev_reply(ctp, n as i32, &data[..n]);
            }
            FIONREAD => return dev_reply(ctp, 0, &bsdnet_nread(so).to_le_bytes()),
            _ => {
                let e = bsdnet_ioctl(so, d.dcmd as u32 as u64, data.as_mut_ptr());
                if e == ENOTTY {
                    return ENOTTY;
                }
                if e == 0 {
                    return dev_reply(ctp, 0, &data[..size]);
                }
                e
            }
        }
    };
    if e != 0 {
        return e;
    }
    dev_reply(ctp, 0, &[])
}

/// `getsockopt()`, `setsockopt()` and `sysctl()`.
unsafe extern "C" fn io_msg(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let _serving = Serving::new(ctp);
    let so = ocb_socket(ocb);
    // SAFETY: a `_IO_MSG`, whose first fields hold in the buffer.
    let (subtype, a, b, c) = unsafe {
        let m = msg.cast::<io::Msg>().read_unaligned();
        let at = size_of::<io::Msg>();
        (
            m.subtype,
            msg.add(at).cast::<u32>().read_unaligned(),
            msg.add(at + 4).cast::<u32>().read_unaligned(),
            msg.add(at + 8).cast::<u32>().read_unaligned(),
        )
    };
    const HEAD: usize = size_of::<io::Msg>() + 16;
    if let Some(u) = ocb_unix(ocb) {
        if subtype == 3 {
            return sysctl(ctp, msg);
        }
        if subtype != 1 || a != SOL_SOCKET {
            return if subtype == 0 { reply(ctp, 0, &[]) } else { ENOPROTOOPT };
        }
        let value: i32 = match b {
            SO_TYPE => u.kind,
            SO_ERROR => 0,
            SO_SNDBUF | SO_RCVBUF => 64 * 1024,
            _ => return ENOPROTOOPT,
        };
        let mut v = vec![0u8; HEAD + 4];
        // SAFETY: the header, for the client's `len`.
        unsafe { core::ptr::copy_nonoverlapping(msg, v.as_mut_ptr(), HEAD) };
        v[size_of::<io::Msg>() + 8..size_of::<io::Msg>() + 12].copy_from_slice(&4u32.to_le_bytes());
        v[HEAD..].copy_from_slice(&value.to_le_bytes());
        return reply(ctp, 0, &v);
    }
    match subtype {
        0 => {
            // SAFETY: the context is the framework's.
            let mut v = match unsafe { read_all(ctp, HEAD, c as usize) } {
                Ok(v) => v,
                Err(e) => return e,
            };
            // SAFETY: FreeBSD's call.
            let e = unsafe { bsdnet_setopt(so, a as c_int, b as c_int, v.as_mut_ptr(), v.len()) };
            if e != 0 { e } else { reply(ctp, 0, &[]) }
        }
        1 => {
            let mut v = vec![0u8; HEAD + c as usize];
            let mut n = c as usize;
            // What the client's buffer holds: FreeBSD's handlers may read it
            // first (ipfw's IP_FW3 requests come so); libsys sends it,
            // QNX's libsocket may send only the header (the rest stays 0).
            // SAFETY: the context is the framework's.
            let rcvid = unsafe { (*ctp).rcvid };
            let _ = MsgRead(rcvid, v[HEAD..].as_mut_ptr().cast(), c as usize, HEAD);
            // SAFETY: FreeBSD's call, over this buffer.
            let e = unsafe { bsdnet_getopt(so, a as c_int, b as c_int, v[HEAD..].as_mut_ptr(), &raw mut n) };
            if e != 0 {
                return e;
            }
            // SAFETY: the header, for the client's `len`.
            unsafe { core::ptr::copy_nonoverlapping(msg, v.as_mut_ptr(), HEAD) };
            v[size_of::<io::Msg>() + 8..size_of::<io::Msg>() + 12].copy_from_slice(&(n as u32).to_le_bytes());
            v.truncate(HEAD + n);
            reply(ctp, 0, &v)
        }
        3 => sysctl(ctp, msg),
        _ => ENOSYS,
    }
}

/// `io_sock_sysctl_t`: names after the header, then the new value.
fn sysctl(ctp: *mut Context, msg: *mut u8) -> c_int {
    const HEAD: usize = size_of::<io::Msg>() + 24;
    let at = size_of::<io::Msg>();
    // SAFETY: the header is in the buffer.
    let (count, oldlen, newlen) = unsafe {
        (
            msg.add(at).cast::<u32>().read_unaligned() as usize,
            msg.add(at + 8).cast::<usize>().read_unaligned(),
            msg.add(at + 16).cast::<usize>().read_unaligned(),
        )
    };
    // SAFETY: the context is the framework's.
    let mut body = match unsafe { read_all(ctp, HEAD, count * 4 + newlen) } {
        Ok(b) => b,
        Err(e) => return e,
    };
    let mut names = vec![0 as c_int; count];
    // SAFETY: both are `count` ints.
    unsafe { core::ptr::copy_nonoverlapping(body.as_ptr(), names.as_mut_ptr().cast(), count * 4) };
    let mut out = vec![0u8; HEAD + oldlen];
    let mut len = oldlen;
    let new = if newlen == 0 { null_mut() } else { body[count * 4..].as_mut_ptr() };
    let old = if oldlen == 0 { null_mut() } else { out[HEAD..].as_mut_ptr() };
    // SAFETY: FreeBSD's call, over these buffers.
    let e = unsafe { bsdnet_sysctl(names.as_mut_ptr(), count as u32, old, &raw mut len, new, newlen) };
    if e != 0 && e != ENOMEM {
        return e;
    }
    // SAFETY: the header, for the client's `oldlen`.
    unsafe { core::ptr::copy_nonoverlapping(msg, out.as_mut_ptr(), HEAD) };
    out[at + 8..at + 16].copy_from_slice(&len.to_le_bytes());
    out.truncate(HEAD + len.min(oldlen));
    reply(ctp, c_long::from(e == ENOMEM), &out)
}

/// `accept()`: on a new connection, the OCB the listening socket's.
unsafe extern "C" fn io_openfd(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let _serving = Serving::new(ctp);
    // SAFETY: an openfd message.
    let m = unsafe { msg.cast::<Openfd>().read_unaligned() };
    if m.xtype == io::IO_OPENFD_PIPE {
        // The second end of `socketpair()`.
        let Some(first) = ocb_unix(ocb) else {
            return EOPNOTSUPP;
        };
        let second = Unix::new(first.kind);
        Unix::pair(&first, &second);
        let mut open = Connect { ioflag: m.ioflag, sflag: m.sflag, access: 3, ..Default::default() };
        // SAFETY: a connect message on this stack.
        return unsafe { bind_socket(ctp, (&raw mut open).cast(), null_mut(), Some(second)) };
    }
    if m.xtype != OPENFD_ACCEPT {
        return ENOSYS;
    }
    let tail = size_of::<Openfd>();
    // SAFETY: `io_sock_accept_t`: the type, the flags, `anamelen`.
    let room = unsafe { msg.add(tail + 8).cast::<u32>().read_unaligned() } as usize;
    let mut head = [0u8; 256];
    // SAFETY: the whole request is in the buffer.
    unsafe { core::ptr::copy_nonoverlapping(msg, head.as_mut_ptr(), tail + 16) };
    let mut name = [0u8; NAME_ROOM];
    let mut so = null_mut();
    let mut end = None;
    if let Some(u) = ocb_unix(ocb) {
        match u.accept() {
            Ok(e) => end = Some(e),
            Err(e) => return e,
        }
        name[0] = 2;
        name[1] = AF_UNIX as u8;
    } else {
        // SAFETY: FreeBSD's call.
        let e = unsafe { bsdnet_accept(ocb_socket(ocb), &raw mut so, name.as_mut_ptr()) };
        if e != 0 {
            return e;
        }
    }
    let unix = end.is_some();
    let mut open = Connect { ioflag: m.ioflag, sflag: m.sflag, access: 3, ..Default::default() };
    // SAFETY: a connect message on this stack.
    let e = unsafe { bind_socket(ctp, (&raw mut open).cast(), so, end) };
    if e != 0 {
        if !unix {
            // SAFETY: never bound.
            unsafe { bsdnet_close(so) };
        }
        return e;
    }
    let got = (name[0] as usize).min(room).min(NAME_ROOM);
    head[tail + 8..tail + 12].copy_from_slice(&(got as u32).to_le_bytes());
    let parts = [Iov::raw(head.as_ptr().cast(), tail + 16), Iov::raw(name.as_ptr().cast(), got)];
    // SAFETY: the context is the framework's.
    MsgReplyv(unsafe { (*ctp).rcvid }, 0, parts.as_ptr().cast(), 2);
    RESMGR_NOREPLY
}

/// `ionotify()` for `poll()`.
unsafe extern "C" fn io_notify(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let so = ocb_socket(ocb);
    let unix = ocb_unix(ocb);
    // SAFETY: a notify message.
    let m = unsafe { msg.cast::<Notify>().read_unaligned() };
    let wanted = m.flags;
    // SAFETY: the context is the framework's.
    let (scoid, coid, rcvid) = unsafe { ((*ctp).info.scoid, (*ctp).info.coid, (*ctp).rcvid) };
    let mut events = 0;
    if wanted & COND_INPUT != 0 {
        events |= POLLIN;
    }
    if wanted & COND_OUTPUT != 0 {
        events |= POLLOUT;
    }
    if wanted & COND_OBAND != 0 {
        events |= POLLPRI;
    }
    // SAFETY: the event's value, in the message.
    let value = unsafe { msg.add(core::mem::offset_of!(Notify, event) + EVENT_VALUE).cast::<u32>().read_unaligned() };
    let call = value.wrapping_sub(m.fd_first as u32);
    ARMED.lock().unwrap().retain(|a| a.scoid != scoid || a.coid != coid || (a.call == call && a.value != value));
    if m.action == io::NOTIFY_ACTION_POLLARM {
        // Before looking, so that what comes after the look isn't lost.
        let mut event = [0u8; 32];
        // SAFETY: the event's 32 bytes, in the message.
        unsafe { core::ptr::copy_nonoverlapping(msg.add(core::mem::offset_of!(Notify, event)), event.as_mut_ptr(), 32) };
        let mut armed = ARMED.lock().unwrap();
        for (bit, write) in [(POLLIN | POLLPRI, 0), (POLLOUT, 1)] {
            if events & bit != 0 {
                let sel = ocb_sel(ocb, write);
                let cond = if write == 0 { COND_INPUT } else { COND_OUTPUT };
                armed.push(Armed { sel, scoid, coid, call, value, rcvid, event, cond });
            }
        }
    }
    let got = match &unix {
        Some(u) => {
            let (r, w) = u.ready();
            (if r { POLLIN } else { 0 } | if w { POLLOUT } else { 0 }) & events
        }
        // SAFETY: FreeBSD's call.
        None => unsafe { bsdnet_poll(so, events) },
    };
    let mut held = 0;
    if got & POLLIN != 0 {
        held |= COND_INPUT;
    }
    if got & POLLOUT != 0 {
        held |= COND_OUTPUT;
    }
    if got & POLLPRI != 0 {
        held |= COND_OBAND;
    }
    if got & !(POLLIN | POLLOUT | POLLPRI) != 0 {
        held |= wanted & (COND_INPUT | COND_OUTPUT);
    }
    if held != 0 {
        ARMED.lock().unwrap().retain(|a| a.scoid != scoid || a.coid != coid);
    }
    // Only the first two fields: QNX's `_poll` (read with REA) sends one
    // message at a time with the whole `poll()` in it and receives the reply
    // over it, so a longer reply of zeros would end its loop over the
    // descriptors at `nfds`, and the others would never be armed.
    let mut r = [0u8; 8];
    r[4..8].copy_from_slice(&held.to_le_bytes());
    reply(ctp, 0, &r)
}

// ANCHOR: serve
fn serve(dpp: *mut Dispatch) {
    // SAFETY: the framework's, one context per thread.
    unsafe {
        let ctp = dispatch_context_alloc(dpp);
        assert!(!ctp.is_null(), "netd: no context");
        loop {
            IDLE.fetch_add(1, Ordering::SeqCst);
            let got = dispatch_block(ctp);
            if got.is_null() {
                // A signal, say: there is nothing to serve.
                IDLE.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            // The last idle thread starts another before this one may wait.
            if IDLE.fetch_sub(1, Ordering::SeqCst) == 1 && THREADS.load(Ordering::SeqCst) < MAX_THREADS {
                spawn(dpp);
            }
            dispatch_handler(ctp);
        }
    }
}
// ANCHOR_END: serve

struct Handle(*mut Dispatch);
// SAFETY: the dispatch handle is for all threads.
unsafe impl Send for Handle {}

fn spawn(dpp: *mut Dispatch) {
    THREADS.fetch_add(1, Ordering::SeqCst);
    let h = Handle(dpp);
    std::thread::spawn(move || {
        let h = h;
        serve(h.0);
    });
}

/// Serves `<prefix>/dev/socket/<n>` of each domain for ever. `io-sock` takes
/// a prefix too, for several stacks, and QNX's `libsocket` follows `SOCK`.
/// `/dev/netd`'s command: kldload(2) and its family, the call's FreeBSD
/// number, two integers and a buffer (`bsdkern_kld()`, `kern/kld.c`).
const DCMD_KLD: u32 = 0x4e01;
const KLD_ROOM: usize = 4080;

unsafe extern "C" {
    fn bsdkern_kld(number: c_int, a0: c_int, a1: c_int, buf: *mut u8, ret: *mut i64) -> c_int;
}

// SAFETY: all zeroes is a valid start for these; `run` fills them.
static mut KLD_CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
// SAFETY: as above.
static mut KLD_IO: IoFuncs = unsafe { core::mem::zeroed() };
// SAFETY: as above.
static mut KLD_ATTR: IofuncAttr = unsafe { core::mem::zeroed() };

/// The modules' calls on `/dev/netd`, with the client's credentials, so
/// that FreeBSD's checks (`PRIV_KLD_LOAD`, `PRIV_KLD_UNLOAD`) see it.
unsafe extern "C" fn io_kld(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let _serving = Serving::new(ctp);
    // SAFETY: a devctl message.
    let d = unsafe { msg.cast::<Devctl>().read_unaligned() };
    if d.dcmd as u32 & 0xffff != DCMD_KLD {
        // SAFETY: the framework's default with its own arguments.
        return unsafe { iofunc_devctl_default(ctp, msg, ocb.cast()) };
    }
    // SAFETY: the context is the framework's.
    let mut data = match unsafe { read_all(ctp, size_of::<Devctl>(), 16 + KLD_ROOM) } {
        Ok(b) => b,
        Err(e) => return e,
    };
    data.resize(16 + KLD_ROOM, 0);
    let int = |at: usize| i32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let (number, a0, a1) = (int(0), int(4), int(8));
    // A string argument ends inside the buffer.
    data[16 + KLD_ROOM - 1] = 0;
    let mut ret = 0i64;
    // SAFETY: bsdkern's call, over this buffer.
    let e = unsafe { bsdkern_kld(number, a0, a1, data[16..].as_mut_ptr(), &raw mut ret) };
    if e != 0 {
        return e;
    }
    // The return, then the buffer where it came: the reply lands on the
    // request in the client's memory.
    let mut out = ret.to_le_bytes().to_vec();
    out.resize(16, 0);
    out.extend_from_slice(&data[16..]);
    dev_reply(ctp, 0, &out)
}

pub fn run(prefix: &str) -> ! {
    // SAFETY: the framework's functions, with statics of this program.
    unsafe {
        let dpp = dispatch_create();
        assert!(!dpp.is_null(), "netd: dispatch_create");
        resmgr::resmgr::unblock_unbound(unblock_unbound);
        privilege::register(&PRIVILEGES).expect("netd: abilities");
        iofunc_func_init(9, &raw mut CONNECT, 33, &raw mut IO);
        (*(&raw mut CONNECT)).funcs[0] = Some(io_open);
        let f = &mut *(&raw mut IO);
        f.funcs[F_READ] = Some(io_read);
        f.funcs[F_WRITE] = Some(io_write);
        f.funcs[F_CLOSE_OCB] = Some(io_close_ocb);
        f.funcs[F_NOTIFY] = Some(io_notify);
        f.funcs[F_UNBLOCK] = Some(io_unblock);
        f.funcs[F_DEVCTL] = Some(io_devctl);
        f.funcs[F_OPENFD] = Some(io_openfd);
        f.funcs[F_MSG] = Some(io_msg);
        // A call that waits must not hold other clients of the same attribute.
        f.funcs[F_LOCK_OCB] = None;
        f.funcs[F_UNLOCK_OCB] = None;
        iofunc_attr_init(&raw mut ATTR, S_IFSOCK | 0o777, null(), null());
        let attr = ResmgrAttr { flags: 0, nparts_max: 2, msg_max_size: 4096, other_func: None, reserved: [0; 4] };
        for domain in [2, 28, 1, 17, 38] {
            let path = std::ffi::CString::new(format!("{prefix}/dev/socket/{domain}")).unwrap();
            let id = resmgr_attach(dpp, &attr, path.as_ptr(), proto::connect::FTYPE_SOCKET as u32, 0, &raw const CONNECT, &raw const IO, (&raw mut ATTR).cast());
            assert!(id >= 0, "netd: resmgr_attach: {}", std::io::Error::last_os_error());
            DOMAINS.lock().unwrap().push((id, domain));
        }
        // `/dev/netd`: the modules (kldload(2)).
        iofunc_func_init(9, &raw mut KLD_CONNECT, 33, &raw mut KLD_IO);
        (*(&raw mut KLD_IO)).funcs[F_DEVCTL] = Some(io_kld);
        iofunc_attr_init(&raw mut KLD_ATTR, 0o020600, null(), null());
        let path = std::ffi::CString::new(format!("{prefix}/dev/netd")).unwrap();
        let id = resmgr_attach(dpp, &attr, path.as_ptr(), 0, 0, &raw const KLD_CONNECT, &raw const KLD_IO, (&raw mut KLD_ATTR).cast());
        assert!(id >= 0, "netd: resmgr_attach /dev/netd: {}", std::io::Error::last_os_error());
        THREADS.store(1, Ordering::SeqCst);
        serve(dpp);
    }
    unreachable!()
}
