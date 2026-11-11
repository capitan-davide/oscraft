//! A server's devices as files: each `struct cdev` its drivers make
//! (`make_dev()`, `kern/cdev.c`) is served at `/dev/<name>` with QNX's
//! resource-manager framework, its opens, closes and `ioctl()`s calling the
//! driver's functions as devfs would. An `ioctl()` arrives as a devctl
//! whose command is FreeBSD's own request (QNX's `_DCMD` encoding is
//! `_IOC`'s: direction, size, group, number), with its data; the data,
//! changed or not, is the reply, landing where the client's was. Requests
//! whose data holds pointers into the client are packed by libsys and
//! unpacked by the server (`bsdkern_cdev_ioctl_packed()`).

use core::ffi::{c_char, c_int, c_void};
use core::ptr::{null, null_mut};
use std::sync::Mutex;

use kercall::abi::Iov;
use kercall::neutrino::msg::MsgReplyv;
use proto::io::{Devctl, DevctlReply};
use resmgr::abi::{ConnectFuncs, Context, IoFuncs, IofuncAttr, IofuncOcb, RESMGR_NOREPLY, ResmgrAttr};
use resmgr::dispatch::{Dispatch, dispatch_block, dispatch_context_alloc, dispatch_create, dispatch_handler};
use resmgr::iofunc::{iofunc_attr_init, iofunc_close_ocb_default, iofunc_devctl_default, iofunc_func_init, iofunc_ocb_attach};
use resmgr::mem::{calloc_bytes, free};
use resmgr::resmgr::{resmgr_attach, resmgr_msgread};

const S_IFCHR: u32 = 0o020000;
const F_CLOSE_OCB: usize = 2;
const F_DEVCTL: usize = 5;
const ENOMEM: c_int = 12;
const EINVAL: c_int = 22;
/// `IOC_VOID`, `IOC_OUT`, `IOC_IN` (`<sys/ioccom.h>`).
const IOC_VOID: u32 = 0x2000_0000;
const IOC_DIRMASK: u32 = 0xe000_0000;
/// The largest `ioctl()` data, and a packed request with its buffers.
const ROOM: usize = 64 * 1024;

unsafe extern "C" {
    fn bsdkern_cdev_name(i: c_int, buf: *mut c_char, len: usize) -> c_int;
    fn bsdkern_cdev_open(name: *const c_char, fflags: c_int, op: *mut *mut c_void) -> c_int;
    fn bsdkern_cdev_ioctl(o: *mut c_void, cmd: u64, data: *mut c_void) -> c_int;
    fn bsdkern_cdev_close(o: *mut c_void);
    /// A server's unpacking of requests with pointers (`kern/cdev.c`'s
    /// weak default: none): -1 if `cmd` isn't one, else the error, with
    /// the reply's length in `*out`.
    fn bsdkern_cdev_ioctl_packed(o: *mut c_void, cmd: u64, buf: *mut u8, len: usize, out: *mut usize) -> c_int;
}

/// An open: the framework's OCB, then the device's.
#[repr(C)]
struct DevOcb {
    ocb: IofuncOcb,
    open: *mut c_void,
}

/// Each device's attribute, with its name: the open's handle.
#[repr(C)]
struct DevAttr {
    attr: IofuncAttr,
    name: std::ffi::CString,
}

// SAFETY: all zeroes is a valid start for these; `serve` fills them.
static mut CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
// SAFETY: as above.
static mut IO: IoFuncs = unsafe { core::mem::zeroed() };

struct Served(*mut Dispatch, Vec<String>);
// SAFETY: the dispatch handle is the framework's, usable from any thread.
unsafe impl Send for Served {}
static SERVED: Mutex<Served> = Mutex::new(Served(null_mut(), Vec::new()));

unsafe extern "C" fn io_open(ctp: *mut Context, msg: *mut u8, handle: *mut c_void, _extra: *mut c_void) -> c_int {
    let a = handle.cast::<DevAttr>();
    let p = calloc_bytes(size_of::<DevOcb>()).cast::<DevOcb>();
    if p.is_null() {
        return ENOMEM;
    }
    // SAFETY: allocated above; the attribute is one of `serve`'s; the
    // framework's function checks the open against the attribute.
    unsafe {
        let e = iofunc_ocb_attach(ctp, msg, p.cast(), a.cast(), null());
        if e != 0 {
            free(p.cast());
            return e;
        }
        // The open's FREAD and FWRITE: the framework's flags, less one.
        let fflags = (*p).ocb.ioflag & 3;
        let e = bsdkern_cdev_open((*a).name.as_ptr(), fflags, &raw mut (*p).open);
        if e != 0 {
            (*p).open = null_mut();
        }
        e
    }
}

unsafe extern "C" fn io_close_ocb(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an OCB of `io_open`'s, its last user.
    unsafe {
        let o = (*ocb.cast::<DevOcb>()).open;
        if !o.is_null() {
            bsdkern_cdev_close(o);
        }
        iofunc_close_ocb_default(ctp, msg, ocb.cast())
    }
}

unsafe extern "C" fn io_devctl(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a devctl message.
    let d = unsafe { msg.cast::<Devctl>().read_unaligned() };
    let dcmd = d.dcmd as u32;
    // QNX's own commands (DCMD_ALL_*: class 1) are the framework's.
    if (dcmd >> 8) & 0xff == 1 {
        // SAFETY: the framework's default with its own arguments.
        return unsafe { iofunc_devctl_default(ctp, msg, ocb.cast()) };
    }
    // SAFETY: an OCB of `io_open`'s.
    let o = unsafe { (*ocb.cast::<DevOcb>()).open };
    let len = (d.nbytes as usize).min(ROOM);
    let mut buf = vec![0u8; len.max(((dcmd >> 16) & 0x1fff) as usize)];
    // SAFETY: the buffer holds `len` bytes; the data follows the header.
    let got = unsafe { resmgr_msgread(ctp, buf.as_mut_ptr().cast(), len, size_of::<Devctl>()) };
    if got < 0 {
        return EINVAL;
    }
    let cmd = u64::from(if dcmd & IOC_DIRMASK == 0 { dcmd | IOC_VOID } else { dcmd });
    let mut out = buf.len();
    // SAFETY: the server's unpacking, over this buffer.
    let mut e = unsafe { bsdkern_cdev_ioctl_packed(o, cmd, buf.as_mut_ptr(), buf.len(), &raw mut out) };
    if e < 0 {
        // SAFETY: the driver's function, over a buffer of the request's size.
        e = unsafe { bsdkern_cdev_ioctl(o, cmd, buf.as_mut_ptr().cast()) };
        out = len;
    }
    if e != 0 {
        return e;
    }
    let head = DevctlReply { zero: 0, ret_val: 0, nbytes: out as u32, zero2: 0 };
    let parts = [Iov::raw((&raw const head).cast(), size_of::<DevctlReply>()), Iov::raw(buf.as_ptr().cast(), out)];
    // SAFETY: the context is the framework's.
    MsgReplyv(unsafe { (*ctp).rcvid }, 0, parts.as_ptr().cast(), 2);
    RESMGR_NOREPLY
}

/// Serves `/dev/<name>` for device `name`, mode `mode`.
fn attach(dpp: *mut Dispatch, name: &str, mode: u32) {
    let a = Box::leak(Box::new(DevAttr {
        // SAFETY: initialized just below, by the framework.
        attr: unsafe { core::mem::zeroed() },
        name: std::ffi::CString::new(name).unwrap_or_default(),
    }));
    let path = std::ffi::CString::new(format!("/dev/{name}")).unwrap_or_default();
    let rattr = ResmgrAttr { flags: 0, nparts_max: 2, msg_max_size: 4096, other_func: None, reserved: [0; 4] };
    // SAFETY: the framework's functions, with a leaked attribute.
    unsafe {
        iofunc_attr_init(&raw mut a.attr, S_IFCHR | mode, null(), null());
        let id = resmgr_attach(dpp, &rattr, path.as_ptr(), 0, 0, &raw const CONNECT, &raw const IO, (a as *mut DevAttr).cast());
        if id < 0 {
            eprintln!("bsdkern: /dev/{name}: {}", std::io::Error::last_os_error());
        }
    }
}

/// A device made after `serve()` started (`kern/cdev.c`'s hook).
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_cdev_added(_dev: *mut c_void) {
    // The names are looked up again: the new one is the one not served.
    let mut s = SERVED.lock().unwrap();
    if s.0.is_null() {
        return;
    }
    let dpp = s.0;
    for name in names() {
        if !s.1.contains(&name) {
            attach(dpp, &name, 0o600);
            s.1.push(name);
        }
    }
}

fn names() -> Vec<String> {
    let mut v = Vec::new();
    let mut buf = [0u8; 64];
    // SAFETY: the buffer is 64 bytes.
    while unsafe { bsdkern_cdev_name(v.len() as c_int, buf.as_mut_ptr().cast(), buf.len()) } == 0 {
        let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
        v.push(String::from_utf8_lossy(&buf[..n]).into_owned());
    }
    v
}

/// Serves the server's devices, those made so far and those to come, with
/// `threads` threads; returns at once.
pub fn serve(threads: usize) {
    // SAFETY: the framework's functions, with statics of this module.
    let dpp = unsafe {
        let dpp = dispatch_create();
        assert!(!dpp.is_null(), "bsdkern: dispatch_create");
        iofunc_func_init(9, &raw mut CONNECT, 33, &raw mut IO);
        (*(&raw mut CONNECT)).funcs[0] = Some(io_open);
        let f = &mut *(&raw mut IO);
        f.funcs[F_CLOSE_OCB] = Some(io_close_ocb);
        f.funcs[F_DEVCTL] = Some(io_devctl);
        dpp
    };
    {
        let mut s = SERVED.lock().unwrap();
        for name in names() {
            attach(dpp, &name, 0o600);
            s.1.push(name);
        }
        s.0 = dpp;
    }
    for _ in 0..threads.max(1) {
        let h = Served(dpp, Vec::new());
        std::thread::spawn(move || {
            let h = h;
            // SAFETY: the framework's functions.
            unsafe {
                let ctp = dispatch_context_alloc(h.0);
                assert!(!ctp.is_null(), "bsdkern: dispatch_context_alloc");
                loop {
                    if dispatch_block(ctp).is_null() {
                        continue;
                    }
                    dispatch_handler(ctp);
                }
            }
        });
    }
}
