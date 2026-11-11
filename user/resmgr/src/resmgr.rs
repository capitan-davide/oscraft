//! The resource manager layer, `<sys/resmgr.h>`: paths attached to the
//! path manager, open files bound to their clients' connections, and the
//! handler that turns each I/O message into a call of the server's
//! function for it, and the function's result into the reply.
//!
//! An attachment (a *link*) is a path in the path manager's namespace
//! that names this server's channel, with a handle the path manager
//! sends back in every connect message. A connect message (an open,
//! unlink, rename...) is matched to its link by that handle and goes to
//! the link's connect functions. An open that succeeds *binds* an OCB to
//! the client's connection, identified by the server connection, the
//! client's process and its connection (`scoid`, `pid`, `coid`); every
//! I/O message on that connection then goes to the binding's I/O
//! functions with its OCB.

use core::ffi::{c_char, c_int, c_long, c_uint, c_void};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicU32, Ordering};

use kercall::abi::Iov;
use kercall::neutrino::connect::ConnectDetach_r;
use kercall::neutrino::msg::{
    MsgError_r, MsgInfo_r, MsgRead, MsgReadv, MsgReplyv, MsgWrite, MsgWritev,
};
use kercall::path::{self, Request};
use crate::compat::Mutex;

use crate::abi::*;
use crate::dispatch::{Dispatch, fail, message_attach};
use crate::mem::{calloc, free};
use core::mem::offset_of;

use proto::connect::{CONNECT_LINK, Connect, EXTRA_RESMGR_LINK, FTYPE_LINK};
use proto::event::Pulse;
use proto::fcntl::{O_NOCTTY, SH_DENYNO};
use proto::io::{self, Header, Openfd};
use proto::pod::Pod;

/// Reads a received message's header, a `T` of `proto`'s, at `msg`.
fn msg_as<T: Pod>(msg: *const u8) -> T {
    // SAFETY: the framework reads only headers within the message it has
    // checked, or within its buffer; any bytes are a T.
    unsafe { msg.cast::<T>().read_unaligned() }
}

/// Reads the byte at `at` of a received message.
fn byte(msg: *const u8, at: usize) -> u8 {
    // SAFETY: as msg_as().
    unsafe { msg.wrapping_add(at).read() }
}

/// An attachment: a path, its functions and its handle.
struct Link {
    next: *mut Link,
    id: c_int,
    /// The connection to the path manager that holds the path: closing it
    /// removes the path. -1 for `RESMGR_FLAG_ATTACH_LOCAL`.
    link_fd: c_int,
    connect_funcs: *const ConnectFuncs,
    io_funcs: *const IoFuncs,
    handle: *mut c_void,
    /// Messages being handled for the link: a detach waits for them.
    count: c_uint,
    /// Not usable: still being attached, or being detached.
    busy: bool,
}

/// The attachments, by increasing ID, and their lock.
static mut LINKS: *mut Link = null_mut();
static LINKS_LOCK: Mutex = Mutex::new();

/// Runs `f` on the list's head, under its lock.
fn with_links<R>(f: impl FnOnce(*mut *mut Link) -> R) -> R {
    let _guard = LINKS_LOCK.lock();
    f(&raw mut LINKS)
}

/// A new link, with the lowest free ID, busy until it is published.
fn link_alloc() -> *mut Link {
    let link = calloc::<Link>();
    if link.is_null() {
        return link;
    }
    with_links(|head| {
        let mut prev = head;
        let mut id = 0;
        loop {
            // SAFETY: prev is the head or a link's next, under the lock.
            let p = unsafe { prev.read() };
            // SAFETY: a link of the list.
            if p.is_null() || unsafe { (*p).id } != id {
                break;
            }
            id += 1;
            // SAFETY: as above.
            prev = unsafe { &raw mut (*p).next };
        }
        // SAFETY: prev is under the lock.
        let next = unsafe { prev.read() };
        // SAFETY: allocated above.
        unsafe {
            link.write(Link {
                next,
                id,
                link_fd: -1,
                connect_funcs: core::ptr::null(),
                io_funcs: core::ptr::null(),
                handle: null_mut(),
                count: 0,
                busy: true,
            });
        }
        // SAFETY: as above.
        unsafe { prev.write(link) };
    });
    link
}

/// The usable link `id`, with one more message counted on it; release it
/// with [`link_return`].
fn link_query(id: c_int) -> Option<&'static mut Link> {
    with_links(|head| {
        // SAFETY: the head, under the lock.
        let mut p = unsafe { head.read() };
        while !p.is_null() {
            // SAFETY: a link of the list.
            let l = unsafe { &mut *p };
            if l.id == id {
                if l.busy {
                    return None;
                }
                l.count += 1;
                return Some(l);
            }
            p = l.next;
        }
        None
    })
}

/// Releases a link [`link_query`] returned.
fn link_return(link: &mut Link) {
    let _guard = LINKS_LOCK.lock();
    link.count = link.count.saturating_sub(1);
}

/// Removes link `id` from the list, and closes its path. Waits for the
/// messages being handled for it, other than the caller's own.
fn link_free(id: c_int, own: c_uint) -> Result<(), c_int> {
    let link = with_links(|head| {
        let mut prev = head;
        loop {
            // SAFETY: the head or a link's next, under the lock.
            let p = unsafe { prev.read() };
            if p.is_null() {
                return None;
            }
            // SAFETY: a link of the list.
            let l = unsafe { &mut *p };
            if l.id == id {
                l.busy = true;
                // SAFETY: as above.
                unsafe { prev.write(l.next) };
                return Some(p);
            }
            prev = &raw mut l.next;
        }
    });
    let Some(link) = link else {
        return Err(EINVAL);
    };
    // Wait for the messages in other threads to finish.
    loop {
        let busy = {
            let _guard = LINKS_LOCK.lock();
            // SAFETY: removed from the list; still allocated.
            unsafe { (*link).count > own }
        };
        if !busy {
            break;
        }
        // SAFETY: no preconditions.
        let _ = kercall::neutrino::thread::ThreadCtl_r(1, null_mut());
    }
    // SAFETY: as above.
    let fd = unsafe { (*link).link_fd };
    if fd != -1 {
        let _ = kercall::io::close(fd);
    }
    // SAFETY: allocated by link_alloc().
    unsafe { free(link.cast()) };
    Ok(())
}

/// An open file bound to one or more client connections: its OCB and
/// functions. Each connection holds one reference, and so does each
/// message being handled; the last one calls the close function.
pub struct Binding {
    pub ocb: *mut c_void,
    pub funcs: *const IoFuncs,
    pub id: c_int,
    refs: AtomicU32,
}

/// One connection's entry in the table.
struct Entry {
    next: *mut Entry,
    scoid: c_int,
    pid: c_int,
    coid: c_int,
    binding: *mut Binding,
}

/// The bindings, hashed by the client's process ID.
const BUCKETS: usize = 64;
static mut TABLE: [*mut Entry; BUCKETS] = [null_mut(); BUCKETS];
static TABLE_LOCK: Mutex = Mutex::new();

/// The bucket of `pid`'s connections.
fn bucket(pid: c_int) -> *mut *mut Entry {
    let i = (pid as u32 as usize).wrapping_mul(0x9e37_79b9) >> 7 & (BUCKETS - 1);
    // An element of the table; callers hold its lock.
    (&raw mut TABLE).cast::<*mut Entry>().wrapping_add(i)
}

/// The connection a message came on: its server connection, process
/// and connection.
#[derive(Clone, Copy)]
pub struct Key {
    pub scoid: c_int,
    pub pid: c_int,
    pub coid: c_int,
}

impl Key {
    pub fn of(info: &MsgInfo) -> Self {
        Key {
            scoid: info.scoid,
            pid: info.pid,
            coid: info.coid,
        }
    }

    /// The key in a `struct _msg_info_hdr` at `p`: `_IO_DUP`'s and
    /// `_IO_OPENFD`'s.
    ///
    /// # Safety
    ///
    /// `p` points to 28 readable bytes.
    unsafe fn at(p: *const u8) -> Self {
        let w = |at: usize| {
            // SAFETY: the caller's guarantee.
            unsafe { p.wrapping_add(at).cast::<c_int>().read_unaligned() }
        };
        Key {
            pid: w(8),
            scoid: w(0x14),
            coid: w(0x18),
        }
    }

    fn matches(&self, e: &Entry) -> bool {
        e.scoid == self.scoid && e.pid == self.pid && e.coid == self.coid
    }
}

/// The binding of `key`'s connection, with a reference.
pub fn get_binding(key: Key) -> Option<&'static Binding> {
    let _guard = TABLE_LOCK.lock();
    // SAFETY: the bucket, under the lock.
    let mut e = unsafe { bucket(key.pid).read() };
    while !e.is_null() {
        // SAFETY: an entry of the table.
        let entry = unsafe { &*e };
        if key.matches(entry) {
            // SAFETY: an entry's binding is live while it is in the table.
            let b = unsafe { &*entry.binding };
            b.refs.fetch_add(1, Ordering::Relaxed);
            return Some(b);
        }
        e = entry.next;
    }
    None
}

/// Drops a reference: the last calls the close function, then frees the
/// binding.
pub fn put_binding(ctp: &mut Context, binding: &Binding) {
    if binding.refs.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    if let Some(close) = io_func(binding.funcs, IO_RSVD_CLOSE_OCB) {
        let rcvid = ctp.rcvid;
        ctp.rcvid = 0;
        // SAFETY: the server's close function, called with its OCB.
        unsafe { close(ctp, null_mut(), binding.ocb) };
        ctp.rcvid = rcvid;
    }
    // SAFETY: allocated by resmgr_open_bind(); no reference remains.
    unsafe { free(core::ptr::from_ref(binding).cast_mut().cast()) };
}

/// Binds `key`'s connection to `binding`, which gains the reference the
/// caller holds. `EBUSY` if the connection is bound already.
fn add_binding(key: Key, binding: *mut Binding) -> Result<(), c_int> {
    let entry = calloc::<Entry>();
    if entry.is_null() {
        return Err(ENOMEM);
    }
    let _guard = TABLE_LOCK.lock();
    let head = bucket(key.pid);
    // SAFETY: the bucket, under the lock.
    let mut e = unsafe { head.read() };
    while !e.is_null() {
        // SAFETY: an entry of the table.
        let x = unsafe { &*e };
        if key.matches(x) {
            // SAFETY: allocated above.
            unsafe { free(entry.cast()) };
            return Err(EBUSY);
        }
        e = x.next;
    }
    // SAFETY: the bucket, under the lock.
    let next = unsafe { head.read() };
    // SAFETY: allocated above.
    unsafe {
        entry.write(Entry {
            next,
            scoid: key.scoid,
            pid: key.pid,
            coid: key.coid,
            binding,
        });
    }
    // SAFETY: as above.
    unsafe { head.write(entry) };
    Ok(())
}

/// Unbinds the first connection `pick` selects; its binding, with the
/// connection's reference.
fn remove_binding(mut pick: impl FnMut(&Entry) -> bool) -> Option<&'static Binding> {
    let _guard = TABLE_LOCK.lock();
    for i in 0..BUCKETS {
        // An element of the table, under the lock.
        let mut link = (&raw mut TABLE).cast::<*mut Entry>().wrapping_add(i);
        loop {
            // SAFETY: the bucket or an entry's next.
            let e = unsafe { link.read() };
            if e.is_null() {
                break;
            }
            // SAFETY: an entry of the table.
            let entry = unsafe { &mut *e };
            if pick(entry) {
                // SAFETY: as above.
                unsafe { link.write(entry.next) };
                let binding = entry.binding;
                // SAFETY: allocated by add_binding().
                unsafe { free(e.cast()) };
                // SAFETY: the connection's reference keeps it live.
                return Some(unsafe { &*binding });
            }
            link = &raw mut entry.next;
        }
    }
    None
}

/// The I/O function for message `type` in `funcs`, if there is one.
pub(crate) fn io_func(funcs: *const IoFuncs, r#type: u16) -> IoFn {
    // SAFETY: funcs is null or the server's table.
    let f = unsafe { funcs.as_ref() }?;
    let i = usize::from(r#type.checked_sub(IO_READ)?);
    if i < f.nfuncs as usize && i < IO_NFUNCS {
        f.funcs[i]
    } else {
        None
    }
}

/// Binds the connection the current message came on to `ocb`, with the
/// link's I/O functions or `funcs`: what an open function does when it
/// succeeds. 0, or -1 with `errno` set.
///
/// # Safety
///
/// `ctp` is the context of a connect message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_open_bind(
    ctp: *mut Context,
    ocb: *mut c_void,
    funcs: *const IoFuncs,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    let Some(link) = link_query(c.id) else {
        return fail(ENOENT);
    };
    let binding = calloc::<Binding>();
    if binding.is_null() {
        link_return(link);
        return fail(ENOMEM);
    }
    // SAFETY: allocated for a Binding.
    unsafe {
        binding.write(Binding {
            ocb,
            funcs: if funcs.is_null() {
                link.io_funcs
            } else {
                funcs
            },
            id: link.id,
            refs: AtomicU32::new(1),
        });
    }
    let result = add_binding(Key::of(&c.info), binding);
    link_return(link);
    match result {
        Ok(()) => 0,
        Err(error) => {
            // SAFETY: allocated above, never published.
            unsafe { free(binding.cast()) };
            fail(error)
        }
    }
}

/// The OCB the current message's connection is bound to, or null.
///
/// # Safety
///
/// `ctp` is a context; `info` is null or a message's `_msg_info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _resmgr_ocb(ctp: *mut Context, info: *const MsgInfo) -> *mut c_void {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    // SAFETY: as above.
    let key = Key::of(unsafe { info.as_ref() }.unwrap_or(&c.info));
    match get_binding(key) {
        Some(b) => {
            let ocb = b.ocb;
            put_binding(c, b);
            ocb
        }
        None => null_mut(),
    }
}

/// The OCB the current message's connection is bound to, or null.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_ocb(ctp: *mut Context) -> *mut c_void {
    // SAFETY: the caller's guarantee.
    unsafe { _resmgr_ocb(ctp, core::ptr::null()) }
}

/// The I/O functions of the current message's connection, or null.
///
/// # Safety
///
/// `ctp` is a context; `info` is null or a message's `_msg_info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _resmgr_iofuncs(
    ctp: *mut Context,
    info: *const MsgInfo,
) -> *const IoFuncs {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    // SAFETY: as above.
    let key = Key::of(unsafe { info.as_ref() }.unwrap_or(&c.info));
    match get_binding(key) {
        Some(b) => {
            let funcs = b.funcs;
            put_binding(c, b);
            funcs
        }
        None => core::ptr::null(),
    }
}

/// The I/O functions of the current message's connection, or null.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_iofuncs(ctp: *mut Context) -> *const IoFuncs {
    // SAFETY: the caller's guarantee.
    unsafe { _resmgr_iofuncs(ctp, core::ptr::null()) }
}

/// Both of the above: 0, or -1 if the connection is not bound.
///
/// # Safety
///
/// `ctp` is a context; `ocb` and `funcs` are null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_ocb_iofuncs(
    ctp: *mut Context,
    ocb: *mut *mut c_void,
    funcs: *mut *const IoFuncs,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    let Some(b) = get_binding(Key::of(&c.info)) else {
        return -1;
    };
    if !ocb.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { ocb.write(b.ocb) };
    }
    if !funcs.is_null() {
        // SAFETY: as above.
        unsafe { funcs.write(b.funcs) };
    }
    put_binding(c, b);
    0
}

/// Unbinds the connection `info` names, without calling the close
/// function for it. 0, or -1 if it was not bound.
///
/// # Safety
///
/// `info` is a message's `_msg_info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _resmgr_unbind(info: *const MsgInfo) -> c_int {
    // SAFETY: the caller's guarantee.
    let key = Key::of(unsafe { &*info });
    match remove_binding(|e| key.matches(e)) {
        Some(b) => {
            if b.refs.fetch_sub(1, Ordering::AcqRel) == 1 {
                // SAFETY: the last reference.
                unsafe { free(core::ptr::from_ref(b).cast_mut().cast()) };
            }
            0
        }
        None => -1,
    }
}

/// Unbinds the current message's connection.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_unbind(ctp: *mut Context) -> c_int {
    // SAFETY: the caller's guarantee.
    let info = unsafe { &raw const (*ctp).info };
    // SAFETY: as above.
    unsafe { _resmgr_unbind(info) }
}

/// Calls `func(dpp, ocb, info, data)` for each bound connection, until
/// it returns 0.
///
/// # Safety
///
/// `func` may be called so.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _resmgr_ocb_for_each(
    dpp: *mut Dispatch,
    func: unsafe extern "C" fn(*mut Dispatch, *mut c_void, *const MsgInfo, *mut c_void) -> c_int,
    data: *mut c_void,
) {
    let _guard = TABLE_LOCK.lock();
    for i in 0..BUCKETS {
        // SAFETY: an element of the table, under the lock.
        let mut e = unsafe {
            (&raw const TABLE)
                .cast::<*mut Entry>()
                .wrapping_add(i)
                .read()
        };
        while !e.is_null() {
            // SAFETY: an entry of the table.
            let entry = unsafe { &*e };
            let info = MsgInfo {
                pid: entry.pid,
                scoid: entry.scoid,
                coid: entry.coid,
                ..MsgInfo::default()
            };
            // SAFETY: an entry's binding is live.
            let ocb = unsafe { (*entry.binding).ocb };
            // SAFETY: the caller's guarantee.
            if unsafe { func(dpp, ocb, &info, data) } == 0 {
                return;
            }
            e = entry.next;
        }
    }
}

const FTYPE_ALL: u32 = u32::MAX;
/// `_RESMGR_FLAG_FTYPEONLY`, `_FTYPEALL`, and the flags the path manager
/// takes.
const RESMGR_FLAG_FTYPEONLY: c_uint = 0x8;
const RESMGR_FLAG_FTYPEALL: c_uint = 0x10;
const RESMGR_FLAG_MASK: c_uint = 0x31f;
/// `RESMGR_FLAG_ATTACH_LOCAL`: do not attach a path.
const ATTACH_LOCAL: c_uint = 0x2;
/// `SYSMGR_COID`: the path manager's connection, and the start of the
/// side channels.
const SYSMGR_COID: c_uint = 0x4000_0000;

/// `struct _io_resmgr_link_extra`: the channel and handle the path
/// manager's entry names.
#[repr(C)]
struct LinkExtra {
    nd: u32,
    pid: i32,
    chid: i32,
    handle: u32,
    flags: u32,
    file_type: u32,
    reserved: [u32; 2],
}

/// Attaches `path` in the path manager's namespace to channel `chid`,
/// with `handle`: the entry lasts while the returned connection is open.
///
/// # Safety
///
/// `path` is null or a string.
pub unsafe fn pathmgr_attach(
    path: *const c_char,
    chid: c_int,
    handle: c_uint,
    file_type: u32,
    mut flags: c_uint,
    name: bool,
) -> Result<c_int, c_int> {
    let file_type = if flags & RESMGR_FLAG_FTYPEALL != 0 {
        FTYPE_ALL
    } else {
        file_type
    };
    // SAFETY: the caller's guarantee.
    let path = if path.is_null() || unsafe { path.read() } == 0 {
        flags |= RESMGR_FLAG_FTYPEONLY;
        c"/".as_ptr()
    } else if flags & RESMGR_FLAG_FTYPEONLY != 0 {
        return Err(EINVAL);
    } else {
        path
    };
    let extra = LinkExtra {
        nd: 0,
        pid: crate::compat::getpid(),
        chid,
        handle,
        flags,
        file_type,
        reserved: [0; 2],
    };
    // SAFETY: a plain structure, as bytes.
    let bytes = unsafe {
        core::slice::from_raw_parts((&raw const extra).cast::<u8>(), size_of::<LinkExtra>())
    };
    let request = Request {
        subtype: CONNECT_LINK,
        // A name's request has the name's own type, as QNX's
        // name_attach() sends it.
        file_type: if name { file_type } else { FTYPE_LINK },
        ioflag: if name { 0 } else { O_NOCTTY },
        mode: 0,
        sflag: SH_DENYNO,
        access: 0,
        dirfd: None,
        extra_type: EXTRA_RESMGR_LINK,
        extra: bytes,
        base: SYSMGR_COID,
        cancellable: false,
        coflags: 0,
    };
    // SAFETY: path is a string; the extra data is the link's.
    let connection = unsafe { path::connect(&request, path, &mut []) }?;
    Ok(connection.coid)
}

/// Attaches `path` to the handle's channel, with the server's connect
/// and I/O functions and `handle`, which the connect functions get.
/// Returns the attachment's ID, or -1 with `errno` set.
///
/// # Safety
///
/// `dpp` is a dispatch handle; `attr` is null or valid; `path` is null
/// or a string; the function tables outlive the attachment.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_attach(
    dpp: *mut Dispatch,
    attr: *const ResmgrAttr,
    path: *const c_char,
    file_type: c_uint,
    flags: c_uint,
    connect_funcs: *const ConnectFuncs,
    io_funcs: *const IoFuncs,
    handle: *mut c_void,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { attr.as_ref() };
    let (aflags, size, nparts) =
        attr.map_or((0, 0, 0), |a| (a.flags, a.msg_max_size, a.nparts_max));
    // SAFETY: as above.
    let d = unsafe { &mut *dpp };
    if let Err(error) = d.set_context_size(size, nparts) {
        return fail(error);
    }
    if !d.resmgr {
        // The resource manager's handler takes every I/O message, and the
        // kernel's unblock and disconnect pulses.
        // SAFETY: dpp is a dispatch handle.
        if unsafe {
            message_attach(
                dpp,
                core::ptr::null(),
                0x100,
                0x1ff,
                Some(resmgr_message),
                null_mut(),
            )
        } == -1
        {
            return -1;
        }
        let pulses = MessageAttr {
            flags: 1,
            nparts_max: 0,
            msg_max_size: 0,
            reserved: [0; 5],
        };
        let (lo, hi) = (
            c_int::from(PULSE_CODE_DISCONNECT),
            c_int::from(PULSE_CODE_UNBLOCK),
        );
        // SAFETY: as above.
        if unsafe { message_attach(dpp, &pulses, lo, hi, Some(resmgr_message), null_mut()) } == -1 {
            return -1;
        }
        d.resmgr = true;
    }
    if path.is_null() && aflags & ATTACH_LOCAL == 0 && flags & RESMGR_FLAG_FTYPEONLY == 0 {
        return 0;
    }
    let link = link_alloc();
    if link.is_null() {
        return fail(ENOMEM);
    }
    // SAFETY: allocated, still busy: no other thread uses it.
    let l = unsafe { &mut *link };
    l.connect_funcs = connect_funcs;
    l.io_funcs = io_funcs;
    l.handle = handle;
    if aflags & ATTACH_LOCAL == 0 {
        // SAFETY: the caller's guarantee.
        match unsafe {
            pathmgr_attach(
                path,
                d.chid,
                l.id as c_uint,
                file_type,
                flags & RESMGR_FLAG_MASK,
                false,
            )
        } {
            Ok(fd) => l.link_fd = fd,
            Err(error) => {
                let _ = link_free(l.id, 0);
                return fail(error);
            }
        }
    }
    if let Some(a) = attr
        && a.other_func.is_some()
        && d.other_func.is_none()
    {
        d.other_func = a.other_func;
    }
    let id = l.id;
    let _guard = LINKS_LOCK.lock();
    l.busy = false;
    id
}

/// Attaches `path` for `name_attach()`: a link with no functions, whose
/// clients' messages the server receives itself.
///
/// # Safety
///
/// `dpp` is a dispatch handle; `path` is a string.
pub unsafe fn resmgr_attach_name(
    dpp: *mut Dispatch,
    path: *const c_char,
    file_type: u32,
    flags: c_uint,
) -> c_int {
    let link = link_alloc();
    if link.is_null() {
        return fail(ENOMEM);
    }
    // SAFETY: allocated, still busy.
    let l = unsafe { &mut *link };
    // SAFETY: the caller's guarantee.
    let chid = unsafe { (*dpp).chid };
    // SAFETY: as above.
    match unsafe { pathmgr_attach(path, chid, l.id as c_uint, file_type, flags, true) } {
        Ok(fd) => l.link_fd = fd,
        Err(error) => {
            let _ = link_free(l.id, 0);
            return fail(error);
        }
    }
    let id = l.id;
    let _guard = LINKS_LOCK.lock();
    l.busy = false;
    id
}

/// Detaches attachment `id`: removes its path and, unless `flags` has
/// `_RESMGR_DETACH_PATHNAME`, closes every connection bound through it.
///
/// # Safety
///
/// `dpp` is a dispatch handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_detach(dpp: *mut Dispatch, id: c_int, flags: c_uint) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { detach(dpp, id, flags) }
}

/// Lets user `uid` replace the path of attachment `id`: the path
/// manager's link becomes theirs. 0, or -1 with `errno`.
#[unsafe(no_mangle)]
pub extern "C" fn resmgr_overridable(id: c_int, uid: c_int) -> c_int {
    let fd = with_links(|head| {
        // SAFETY: the list, under its lock.
        let mut l = unsafe { *head };
        // SAFETY: the list's links.
        while let Some(link) = unsafe { l.as_ref() } {
            if link.id == id {
                return Some(link.link_fd);
            }
            l = link.next;
        }
        None
    });
    let Some(fd) = fd else {
        return fail(proto::errno::EINVAL);
    };
    crate::compat::fchown(fd, uid as u32, u32::MAX)
}

/// A message in the other byte order: this library takes none, as QNX's.
#[unsafe(no_mangle)]
pub extern "C" fn resmgr_endian(_ctp: *mut Context, _msg: *mut u8) -> c_int {
    0
}

/// Nothing to set for the other byte order.
#[unsafe(no_mangle)]
pub extern "C" fn resmgr_endian_context(
    _ctp: *mut Context,
    _kind: c_int,
    _mode: c_int,
    _hint: c_int,
) {
}

/// `resmgr_detach()`'s work.
///
/// # Safety
///
/// As `resmgr_detach()`.
unsafe fn detach(dpp: *mut Dispatch, id: c_int, flags: c_uint) -> c_int {
    // From a handler, the handler's own message is not waited for.
    // SAFETY: the caller's guarantee; the thread's context, if any.
    unsafe { resmgr_detach_ctp(dpp, current_context(), id, flags) }
}

/// `resmgr_detach()` from a handler: `ctp`'s own message is not waited
/// for.
///
/// # Safety
///
/// As `resmgr_detach()`; `ctp` is null or the caller's context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_detach_ctp(
    _dpp: *const Dispatch,
    ctp: *mut Context,
    id: c_int,
    flags: c_uint,
) -> c_int {
    const DETACH_PATHNAME: c_uint = 1;
    // SAFETY: the caller's guarantee.
    let own = unsafe { ctp.as_ref() }.map_or(0, |c| c_uint::from(c.id == id));
    if let Err(error) = link_free(id, own) {
        return fail(error);
    }
    if flags & DETACH_PATHNAME == 0 {
        // SAFETY: the caller's guarantee.
        unsafe { close_bindings(ctp, id) };
    }
    0
}

/// Asks the path manager, through link `link_id`, whether the client of
/// `ctp` may open the path of the open request `msg`: an error number, 0
/// if it may.
///
/// # Safety
///
/// `ctp` is a context; `msg` an open request, with its path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _resmgr_check_open(
    ctp: *const Context,
    link_id: c_int,
    msg: *const Connect,
) -> c_int {
    /// `DCMD_PATHMGR_CHECK_OPEN`'s value.
    const CHECK_OPEN: c_int = 0x8010_081e_u32 as c_int;
    // SAFETY: the caller's guarantee.
    let pid = unsafe { (*ctp).info.pid };
    // SAFETY: as above.
    let len = usize::from(unsafe { (*msg).path_len });
    let head: [u64; 2] = [pid as u32 as u64, len as u64];
    let parts = [
        Iov::of(&head),
        Iov::raw(
            msg.cast::<u8>().wrapping_add(size_of::<Connect>()).cast(),
            len,
        ),
    ];
    // SAFETY: both parts are valid for their lengths.
    unsafe {
        crate::compat::devctlv(
            link_id,
            CHECK_OPEN,
            2,
            0,
            parts.as_ptr(),
            core::ptr::null(),
            null_mut(),
        )
    }
}

/// Completes a close this library delayed: none is, so 0.
#[unsafe(no_mangle)]
pub extern "C" fn _resmgr_complete_delayed_close(_ctp: *mut Context) -> c_int {
    0
}

/// Closes every connection bound through attachment `id`: 0.
///
/// # Safety
///
/// `ctp` is null or the caller's context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _resmgr_detach_id(ctp: *mut Context, id: c_int, _flags: c_uint) -> c_int {
    if !ctp.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { close_bindings(ctp, id) };
    }
    0
}

/// Sends `_IO_CLOSE` to each connection bound through attachment `id`,
/// and frees its binding.
///
/// # Safety
///
/// `ctp` is null or the caller's context.
unsafe fn close_bindings(ctp: *mut Context, id: c_int) {
    // SAFETY: a zeroed context is valid; the close functions get it.
    let mut scratch = unsafe { core::mem::zeroed::<Context>() };
    let mut close = [0u8; 4];
    close[..2].copy_from_slice(&IO_CLOSE.to_le_bytes());
    close[2..].copy_from_slice(&4u16.to_le_bytes());
    scratch.msg = close.as_mut_ptr();
    scratch.rcvid = 0;
    scratch.id = id;
    scratch.size = 4;
    // SAFETY: as above.
    scratch.dpp = unsafe { ctp.as_ref() }.map_or(null_mut(), |c| c.dpp);
    while let Some(b) = remove_binding(|e| {
        // SAFETY: an entry's binding is live.
        unsafe { (*e.binding).id == id }
    }) {
        if let Some(f) = io_func(b.funcs, IO_CLOSE) {
            // SAFETY: the server's close function, with its OCB.
            unsafe { f(&mut scratch, scratch.msg, b.ocb) };
        }
        put_binding(&mut scratch, b);
    }
}

/// The path attachment `id` holds, as the path manager reports it,
/// into `path`. Its length, or -1 with `errno` set.
///
/// # Safety
///
/// `path` is writable for `maxbuf` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_pathname(
    id: c_int,
    flags: c_uint,
    path: *mut c_char,
    maxbuf: c_int,
) -> c_int {
    let Some(link) = link_query(id) else {
        return fail(EINVAL);
    };
    let fd = link.link_fd;
    link_return(link);
    // SAFETY: the caller's guarantee.
    let n = unsafe { crate::compat::iofdinfo(fd, flags, null_mut(), path, maxbuf) };
    if n == -1 {
        return fail(EINVAL);
    }
    n
}

/// The device and inode numbers of attachment `id`'s path.
///
/// # Safety
///
/// `dev` and `ino` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_devino(id: c_int, dev: *mut u32, ino: *mut u64) -> c_int {
    let Some(link) = link_query(id) else {
        return fail(EINVAL);
    };
    let fd = link.link_fd;
    link_return(link);
    match kercall::io::dev_ino(fd) {
        Ok((d, i)) => {
            // SAFETY: the caller's guarantee.
            unsafe { ino.write(i) };
            // SAFETY: as above.
            unsafe { dev.write(d) };
            0
        }
        Err(_) => fail(EINVAL),
    }
}

/// Replies to the current message with `parts` of `iov` and the
/// context's status.
///
/// # Safety
///
/// `ctp` is a context; `iov` holds `parts` valid parts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msgreplyv(
    ctp: *mut Context,
    iov: *const Iov,
    parts: c_int,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    MsgReplyv(c.rcvid, c.status, iov.cast(), parts as usize)
}

/// Replies with one buffer.
///
/// # Safety
///
/// `ctp` is a context; `ptr` is valid for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msgreply(
    ctp: *mut Context,
    ptr: *const c_void,
    len: usize,
) -> c_int {
    let iov = Iov { base: ptr, len };
    // SAFETY: the caller's guarantee.
    unsafe { resmgr_msgreplyv(ctp, &iov, 1) }
}

/// Reads `size` bytes of the client's message from `offset` into `msg`.
///
/// # Safety
///
/// `ctp` is a context; `msg` is writable for `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msgread(
    ctp: *mut Context,
    msg: *mut c_void,
    size: usize,
    offset: usize,
) -> isize {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    MsgRead(c.rcvid, msg, size, offset + c.offset) as isize
}

/// `resmgr_msgread()` into parts.
///
/// # Safety
///
/// As `resmgr_msgread()`, for the parts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msgreadv(
    ctp: *mut Context,
    iov: *const Iov,
    parts: c_int,
    offset: usize,
) -> isize {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    MsgReadv(c.rcvid, iov.cast(), parts as usize, offset + c.offset) as isize
}

/// Writes `size` bytes from `msg` into the client's reply buffer at
/// `offset`.
///
/// # Safety
///
/// `ctp` is a context; `msg` is valid for `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msgwrite(
    ctp: *mut Context,
    msg: *const c_void,
    size: usize,
    offset: usize,
) -> isize {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    MsgWrite(c.rcvid, msg, size, offset) as isize
}

/// `resmgr_msgwrite()` from parts.
///
/// # Safety
///
/// As `resmgr_msgwrite()`, for the parts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msgwritev(
    ctp: *mut Context,
    iov: *const Iov,
    parts: c_int,
    offset: usize,
) -> isize {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    MsgWritev(c.rcvid, iov.cast(), parts as usize, offset) as isize
}

/// Gets `size` bytes of the current message from `offset` (from the
/// current part's start): from the receive buffer as far as it holds the
/// message, and the rest from the client.
///
/// # Safety
///
/// `ctp` is a context; `msg` is writable for `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msgget(
    ctp: *mut Context,
    msg: *mut c_void,
    size: usize,
    offset: usize,
) -> isize {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    let have = c.size;
    let mut copied = 0;
    if offset < have {
        copied = size.min(have - offset);
        // SAFETY: the buffer holds `have` bytes of the part, from offset.
        unsafe {
            core::ptr::copy_nonoverlapping(
                c.msg.wrapping_add(c.offset + offset),
                msg.cast::<u8>(),
                copied,
            )
        };
    }
    if copied == size || c.rcvid <= 0 {
        return copied as isize;
    }
    let total = c.info.srcmsglen.max(0) as usize;
    let at = c.offset + offset + copied;
    if at >= total {
        return copied as isize;
    }
    let n = MsgRead(
        c.rcvid,
        msg.cast::<u8>().wrapping_add(copied).cast(),
        size - copied,
        at,
    );
    if n < 0 {
        -1
    } else {
        (copied + n as usize) as isize
    }
}

/// `resmgr_msgget()` into parts.
///
/// # Safety
///
/// As `resmgr_msgget()`, for the parts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msggetv(
    ctp: *mut Context,
    iov: *const Iov,
    parts: usize,
    mut offset: usize,
) -> isize {
    let mut total = 0isize;
    for i in 0..parts {
        // SAFETY: the caller's guarantee.
        let part = unsafe { &*iov.wrapping_add(i) };
        // SAFETY: as above.
        let n = unsafe { resmgr_msgget(ctp, part.base.cast_mut(), part.len, offset) };
        if n < 0 {
            return -1;
        }
        total += n;
        offset += n as usize;
        if (n as usize) < part.len {
            break;
        }
    }
    total
}

/// How much of the current message there is from `start`: in the buffer
/// (`*local`) and still at the client (`*remote`); the sum.
///
/// # Safety
///
/// `ctp` is a context; `local` and `remote` are null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msggetsize(
    ctp: *mut Context,
    start: usize,
    local: *mut usize,
    remote: *mut usize,
) -> usize {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    let have = c.size.saturating_sub(start);
    let rest = (c.info.srcmsglen - c.info.msglen).max(0) as usize;
    if !local.is_null() {
        // SAFETY: as above.
        unsafe { local.write(have) };
    }
    if !remote.is_null() {
        // SAFETY: as above.
        unsafe { remote.write(rest) };
    }
    have + rest
}

/// Handles message `rcvid` again, as if it had just arrived: for a
/// server that put a message aside.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_msg_again(ctp: *mut Context, rcvid: i64) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    if MsgInfo_r(rcvid, (&raw mut c.info).cast()) != 0 {
        return -1;
    }
    c.rcvid = rcvid;
    let n = MsgRead(rcvid, c.msg.cast(), c.msg_max_size, 0);
    if n < 0 {
        return -1;
    }
    c.info.msglen = n as isize;
    // SAFETY: as above.
    unsafe { handle(ctp) };
    0
}

// ANCHOR: reply
/// Replies to the message by the handler's result `r`: no reply, an
/// error, or a reply of `-r` parts with the context's status.
fn reply(ctp: &mut Context, mut r: c_int) {
    if r == RESMGR_DEFAULT {
        r = match ctp_other(ctp) {
            Some(r) => r,
            None => ENOSYS,
        };
    }
    if r == RESMGR_NOREPLY || r == RESMGR_DEFAULT || ctp.rcvid <= 0 {
        if r == RESMGR_DEFAULT && ctp.rcvid > 0 {
            let _ = MsgError_r(ctp.rcvid, ENOSYS);
        }
        return;
    }
    if r > 0 {
        let _ = MsgError_r(ctp.rcvid, r);
    } else {
        let _ = MsgReplyv(
            ctp.rcvid,
            ctp.status,
            ctp.iov.as_ptr().cast(),
            (-r) as usize,
        );
    }
}
// ANCHOR_END: reply

/// The handle's `other_func`, if it has one, called for the message.
fn ctp_other(ctp: &mut Context) -> Option<c_int> {
    // SAFETY: the context's dispatch handle.
    let f = unsafe { (*ctp.dpp).other_func }?;
    let msg = ctp.msg.cast();
    // SAFETY: the server's function, called as resmgr_attr_t says.
    Some(unsafe { f(ctp, msg) })
}

/// The minimum size of each I/O message, from `_IO_CONNECT`; 0 for those
/// a client may not send.
const MIN_SIZE: [u8; 34] = [
    0x28, 0x10, 0x10, 0, 0x8, 0x58, 0x10, 0, 0x8, 0x10, 0x8, 0xc, 0x10, 0x30, 0x10, 0xc, 0x18, 0x4,
    0x58, 0x8, 0, 0x28, 0x4, 0, 0, 0x8, 0x10, 0x10, 0, 0, 0x10, 0x10, 0x58, 0x28,
];

/// Whether the message at `msg`, with `len` bytes left, is long enough:
/// 0, `EBADMSG`, or `ENOSYS` for a type no client sends.
///
/// # Safety
///
/// `msg` is readable for `len` bytes.
unsafe fn check_length(msg: *const u8, len: isize) -> c_int {
    if len < 4 {
        return EBADMSG;
    }
    // SAFETY: the caller's guarantee.
    let Header {
        r#type: t,
        combine_len: sub,
    } = msg_as(msg);
    let mut len = len;
    if t != IO_CONNECT && sub & IO_COMBINE_FLAG != 0 {
        let part = isize::from((sub & !IO_COMBINE_FLAG) as i16);
        if len <= part {
            return EBADMSG;
        }
        len = part;
    }
    let Some(i) = t
        .checked_sub(IO_CONNECT)
        .map(usize::from)
        .filter(|&i| i < MIN_SIZE.len())
    else {
        return 0;
    };
    match MIN_SIZE[i] {
        0 => ENOSYS,
        m if len >= isize::from(m) => 0,
        _ => EBADMSG,
    }
}

/// The resource manager's message and pulse handler, which
/// `resmgr_attach()` attaches.
unsafe extern "C" fn resmgr_message(
    ctp: *mut Context,
    _code: c_int,
    _flags: c_uint,
    _h: *mut c_void,
) -> c_int {
    // SAFETY: the dispatch layer's context.
    unsafe { handle(ctp) };
    0
}

/// The key of each thread's context while it handles a message, for
/// `resmgr_detach()` from a handler, as QNX's library keeps it: -1 until
/// made.
static CURRENT_KEY: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(-1);
static CURRENT_KEY_LOCK: crate::compat::Mutex = crate::compat::Mutex::new();

/// The key, made on first use; -1 if it cannot be made.
fn current_key() -> c_int {
    use core::sync::atomic::Ordering;
    let key = CURRENT_KEY.load(Ordering::Acquire);
    if key != -1 {
        return key;
    }
    let _guard = CURRENT_KEY_LOCK.lock();
    let key = CURRENT_KEY.load(Ordering::Acquire);
    if key != -1 {
        return key;
    }
    let mut new = -1;
    // SAFETY: a key, no destructor.
    if unsafe { crate::compat::pthread_key_create(&mut new, null_mut()) } != 0 {
        return -1;
    }
    CURRENT_KEY.store(new, Ordering::Release);
    new
}

/// The context of the message the calling thread is handling, if any.
fn current_context() -> *mut Context {
    let key = CURRENT_KEY.load(core::sync::atomic::Ordering::Acquire);
    if key == -1 {
        return null_mut();
    }
    crate::compat::pthread_getspecific(key).cast()
}

/// Handles the message or pulse in `ctp`, and replies, with `ctp` as the
/// calling thread's current context meanwhile.
///
/// # Safety
///
/// `ctp` is a context with a message or pulse in it.
unsafe fn handle(ctp: *mut Context) {
    let key = current_key();
    let before = current_context();
    if key != -1 {
        // SAFETY: the key, and a pointer to keep.
        unsafe { crate::compat::pthread_setspecific(key, ctp.cast()) };
    }
    // SAFETY: the caller's guarantee.
    unsafe { handle_message(ctp) };
    if key != -1 {
        // SAFETY: as above.
        unsafe { crate::compat::pthread_setspecific(key, before.cast()) };
    }
}

/// [`handle`]'s work.
///
/// # Safety
///
/// As [`handle`].
unsafe fn handle_message(ctp: *mut Context) {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    c.status = 0;
    let msg = c.msg;
    if c.rcvid == 0 {
        let pulse: Pulse = msg_as(msg);
        let (code, value, scoid) = (pulse.code, pulse.value as i64, pulse.scoid);
        if code == PULSE_CODE_DISCONNECT {
            disconnect(c, scoid);
        } else if code == PULSE_CODE_UNBLOCK && value > 0 {
            let r = unblock(c, value);
            if r != RESMGR_DEFAULT {
                reply(c, r);
            }
        }
        return;
    }
    if c.rcvid < 0 {
        return;
    }
    // SAFETY: the buffer holds the message.
    let r = match unsafe { check_length(msg, c.info.msglen) } {
        0 => {
            // SAFETY: as above.
            let t = msg_as::<Header>(msg).r#type;
            match t {
                IO_CONNECT => connect(c),
                IO_DUP => dup(c),
                IO_OPENFD => {
                    // SAFETY: an _IO_OPENFD message: the original
                    // connection's info.
                    let key = unsafe { Key::at(msg.wrapping_add(offset_of!(Openfd, info))) };
                    with_binding(c, key, |c, b| io(c, msg, b))
                }
                IO_MMAP if c.info.pid != 1 => EACCES,
                IO_CLOSE => close(c),
                _ => with_binding(c, Key::of(&c.info), |c, b| io(c, msg, b)),
            }
        }
        e => e,
    };
    reply(c, r);
}

/// Runs `f` with `key`'s binding, or `EBADF`.
fn with_binding(
    c: &mut Context,
    key: Key,
    f: impl FnOnce(&mut Context, &Binding) -> c_int,
) -> c_int {
    match get_binding(key) {
        Some(b) => {
            let r = f(c, b);
            put_binding(c, b);
            r
        }
        None => EBADF,
    }
}

/// A connect message: an open, or another request about a path, for the
/// link the path manager's handle names.
fn connect(c: &mut Context) -> c_int {
    let msg = c.msg;
    // An _IO_CONNECT message, then its path.
    let m: Connect = msg_as(msg);
    let (subtype, handle_id, ioflag, path_len, extra_type, extra_len) = (
        m.subtype,
        m.handle as c_int,
        m.ioflag,
        usize::from(m.path_len),
        m.extra_type,
        usize::from(m.extra_len),
    );
    let head = size_of::<Connect>();
    let msglen = c.info.msglen.max(0) as usize;
    if path_len == 0 || msglen < head + path_len || byte(msg, head + path_len - 1) != 0 {
        return if subtype == CONNECT_RSVD_UNBLOCK {
            RESMGR_DEFAULT
        } else {
            EINVAL
        };
    }
    let Some(link) = link_query(handle_id) else {
        return if subtype == CONNECT_RSVD_UNBLOCK {
            RESMGR_DEFAULT
        } else {
            ENOENT
        };
    };
    c.id = link.id;
    // SAFETY: the link's table, if any, outlives it.
    let Some(funcs) = (unsafe { link.connect_funcs.as_ref() }) else {
        link_return(link);
        return RESMGR_DEFAULT;
    };
    c.offset = 0;
    let extra_at = (0x28 + path_len + 7) & !7;
    let extra = if extra_len == 0 || extra_at + extra_len > msglen {
        if extra_len != 0 {
            link_return(link);
            return EINVAL;
        }
        null_mut()
    } else {
        msg.wrapping_add(extra_at)
    };
    let func = |i: usize| {
        if (funcs.nfuncs as usize) > i {
            funcs.funcs[i]
        } else {
            None
        }
    };
    let r = if subtype <= CONNECT_COMBINE_CLOSE {
        combine(c, link, subtype, ioflag, extra, extra_len, func(0))
    } else if subtype == CONNECT_MOUNT {
        match func(usize::from(CONNECT_MOUNT) - 2) {
            Some(f) => {
                c.size = msglen;
                mount(c, link, extra_type, extra_at, extra_len, f)
            }
            None => RESMGR_DEFAULT,
        }
    } else if usize::from(subtype) < CONNECT_NFUNCS + 2 {
        match func(usize::from(subtype) - 2) {
            Some(f) => {
                c.size = msglen;
                let _ = extra_type;
                // SAFETY: the server's function, with the link's handle.
                unsafe { f(c, msg, link.handle, extra.cast()) }
            }
            None => RESMGR_DEFAULT,
        }
    } else {
        RESMGR_DEFAULT
    };
    link_return(link);
    r
}

/// `io_mount_extra_t`'s size before the data, and the offsets of its
/// server side's `ocb`, `data`, `type` and `special`; and of the client
/// side's process, server connection and connection.
const MOUNT_HEAD: usize = 0x50;
const MOUNT_SRV: usize = 0x10;
const MOUNT_PID: usize = 0x18;
const MOUNT_SCOID: usize = 0x24;
const MOUNT_COID: usize = 0x28;
/// `_IO_CONNECT_EXTRA_MOUNT` and `_IO_CONNECT_EXTRA_MOUNT_OCB`.
const EXTRA_MOUNT: u8 = 10;
const EXTRA_MOUNT_OCB: u8 = 11;

/// A mount request: the extra data, copied out of the message, gets its
/// server side, as QNX's framework gives it: pointers to the data, the
/// type and the special device; for `_IO_CONNECT_EXTRA_MOUNT_OCB`, the
/// OCB of the device the client opened on this server.
fn mount(
    c: &mut Context,
    link: &Link,
    extra_type: u8,
    at: usize,
    len: usize,
    f: unsafe extern "C" fn(*mut Context, *mut u8, *mut c_void, *mut c_void) -> c_int,
) -> c_int {
    if !matches!(extra_type, EXTRA_MOUNT | EXTRA_MOUNT_OCB) || len < MOUNT_HEAD {
        return EINVAL;
    }
    let mut head = [0u32; 4];
    // SAFETY: the context; room for the header.
    if unsafe { resmgr_msgget(c, head.as_mut_ptr().cast(), 16, at) } != 16 {
        return EINVAL;
    }
    let (nbytes, datalen) = (head[1] as usize, head[2] as usize);
    if datalen + MOUNT_HEAD >= nbytes {
        return EINVAL;
    }
    if nbytes > 0x4000 {
        return EBADMSG;
    }
    let buf = crate::mem::calloc_bytes(nbytes);
    if buf.is_null() {
        return ENOMEM;
    }
    // SAFETY: as above, with room for nbytes.
    let got = unsafe { resmgr_msgget(c, buf.cast(), nbytes, at) };
    let word = |off: usize| {
        // SAFETY: within the copy, which is longer than the header.
        unsafe { buf.wrapping_add(off).cast::<c_int>().read_unaligned() }
    };
    let binding = if extra_type == EXTRA_MOUNT_OCB {
        get_binding(Key {
            scoid: word(MOUNT_SCOID),
            pid: word(MOUNT_PID),
            coid: word(MOUNT_COID),
        })
    } else {
        None
    };
    let r = if got != nbytes as isize {
        EINVAL
    } else if extra_type == EXTRA_MOUNT_OCB && binding.is_none() {
        ENOENT
    } else {
        let end = buf.wrapping_add(nbytes);
        let ty = buf.wrapping_add(MOUNT_HEAD + datalen);
        // SAFETY: the copy, from the type to its end.
        let rest = unsafe { core::slice::from_raw_parts(ty, nbytes - MOUNT_HEAD - datalen) };
        match rest.iter().position(|&b| b == 0) {
            None => EINVAL,
            Some(nul) => {
                let special = ty.wrapping_add(nul + 1);
                let srv: [*mut u8; 8] = [
                    binding.map_or(null_mut(), |b| b.ocb.cast()),
                    if datalen != 0 {
                        buf.wrapping_add(MOUNT_HEAD)
                    } else {
                        null_mut()
                    },
                    ty,
                    if special < end { special } else { null_mut() },
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                ];
                // SAFETY: the server side's eight pointers, in the copy.
                unsafe {
                    buf.wrapping_add(MOUNT_SRV)
                        .cast::<[*mut u8; 8]>()
                        .write_unaligned(srv)
                };
                // SAFETY: the server's function, with the link's handle.
                unsafe { f(c, c.msg, link.handle, buf.cast()) }
            }
        }
    };
    if let Some(b) = binding {
        put_binding(c, b);
    }
    // SAFETY: the copy.
    unsafe { free(buf.cast()) };
    r
}

/// An open combined with an I/O message, which runs on the new binding:
/// `stat()` by path, say; with `_IO_CONNECT_COMBINE_CLOSE`, the binding
/// is closed after it.
fn combine(
    c: &mut Context,
    link: &Link,
    subtype: u16,
    _ioflag: u32,
    extra: *mut u8,
    extra_len: usize,
    open: ConnectFn,
) -> c_int {
    let Some(open) = open else {
        return ENOSYS;
    };
    let msg = c.msg;
    // SAFETY: the connect message's path, as the size says.
    c.size = size_of::<Connect>() + usize::from(msg_as::<Connect>(msg).path_len);
    // SAFETY: the server's open function, with the link's handle.
    let mut r = unsafe { open(c, msg, link.handle, null_mut()) };
    if r != 0 || c.status < 0 {
        return r;
    }
    let Some(b) = get_binding(Key::of(&c.info)) else {
        return EBADF;
    };
    if extra_len >= 4 {
        // SAFETY: the embedded message is within the received message.
        let e = unsafe { check_length(extra, extra_len as isize) };
        r = if e != 0 {
            e
        } else {
            c.status = 0;
            io(c, extra, b)
        };
    }
    if subtype == CONNECT_COMBINE_CLOSE {
        let (rcvid, saved_msg, status, msglen) = (c.rcvid, c.msg, c.status, c.info.msglen);
        let mut close_msg = [0u8; 4];
        close_msg[..2].copy_from_slice(&IO_CLOSE.to_le_bytes());
        close_msg[2..].copy_from_slice(&4u16.to_le_bytes());
        c.rcvid = 0;
        c.msg = close_msg.as_mut_ptr();
        c.info.msglen = 4;
        // As a client's `_IO_CLOSE`: the connection unbound too, or the
        // binding outlives the request and the connection's next open is
        // EBUSY (a side-channel `stat()` after `opendir()`'s).
        let _ = close(c);
        c.rcvid = rcvid;
        c.msg = saved_msg;
        c.status = status;
        c.info.msglen = msglen;
    }
    put_binding(c, b);
    r
}

/// Message types a client may not send, from `_IO_RSVD_CLOSE_OCB`.
fn reserved(t: u16) -> bool {
    matches!(
        t,
        IO_RSVD_CLOSE_OCB
            | IO_RSVD_UNBLOCK
            | 0x114
            | IO_RSVD_LOCK_OCB
            | IO_RSVD_UNLOCK_OCB
            | 0x11c
            | 0x11d
    )
}

/// Calls the binding's function for the I/O message at `msg`, and for
/// each message combined after it, under the OCB's lock. The result of
/// the last, or of the first that fails.
fn io(c: &mut Context, mut msg: *mut u8, b: &Binding) -> c_int {
    let mut locked = false;
    let r = loop {
        // SAFETY: a checked I/O message.
        let Header {
            r#type: t,
            combine_len: sub,
        } = msg_as(msg);
        let combined = sub & IO_COMBINE_FLAG != 0;
        let at = msg as usize - c.msg as usize;
        c.id = b.id;
        c.offset = at;
        c.size = if combined {
            usize::from(sub & !IO_COMBINE_FLAG)
        } else {
            (c.info.msglen.max(0) as usize).saturating_sub(at)
        };
        if !(IO_READ..=IO_UTIME64).contains(&t) || reserved(t) || (t == IO_NOTIFY && combined) {
            break EBADMSG;
        }
        // SAFETY: the binding's table outlives it.
        let check = unsafe { b.funcs.as_ref() }.and_then(|f| {
            if f.nfuncs as usize > 0x13 {
                *check_slot(f)
            } else {
                None
            }
        });
        let checked = match check {
            // SAFETY: the server's check function.
            Some(f) => unsafe { f(c, msg, combined, b.ocb) },
            None => crate::iofunc::check_msg(t, combined),
        };
        if checked != 0 {
            break checked;
        }
        let mut func = io_func(b.funcs, t);
        if t == IO_UTIME64 && func.is_none() {
            func = io_func(b.funcs, IO_UTIME);
        }
        let Some(f) = func else {
            break ENOSYS;
        };
        if !locked && let Some(lock) = io_func(b.funcs, IO_RSVD_LOCK_OCB) {
            // SAFETY: the server's lock function.
            let e = unsafe { lock(c, null_mut(), b.ocb) };
            if e != 0 {
                break e;
            }
            locked = true;
        }
        let rcvid = c.rcvid;
        if combined {
            c.rcvid = 0;
        }
        let r = if t == IO_STAT {
            stat(c, msg, b, f)
        } else {
            // SAFETY: the server's function, with the binding's OCB.
            unsafe { f(c, msg, b.ocb) }
        };
        c.rcvid = rcvid;
        if !combined || (r != 0 && r != RESMGR_NOREPLY) {
            break r;
        }
        msg = msg.wrapping_add(usize::from(sub & !IO_COMBINE_FLAG));
        c.status = 0;
        let left = c.info.msglen - (msg as isize - c.msg as isize);
        let e = if (msg as usize) & 3 != 0 {
            EBADMSG
        } else {
            // SAFETY: the next message is within the buffer, if left says so.
            unsafe { check_length(msg, left) }
        };
        if e != 0 {
            break e;
        }
    };
    if locked && let Some(unlock) = io_func(b.funcs, IO_RSVD_UNLOCK_OCB) {
        // SAFETY: the server's unlock function.
        unsafe { unlock(c, null_mut(), b.ocb) };
    }
    r
}

/// The `check_msg` slot of an I/O table, which has its own type.
fn check_slot(f: &IoFuncs) -> &CheckFn {
    // SAFETY: slot 0x13 of the table is check_msg, a function pointer of
    // the same size.
    unsafe { &*(&raw const f.funcs[0x13]).cast::<CheckFn>() }
}

/// `_IO_STAT`: the server's function, then its `struct stat` converted
/// to the form the client asked for.
fn stat(
    c: &mut Context,
    msg: *mut u8,
    b: &Binding,
    f: unsafe extern "C" fn(*mut Context, *mut u8, *mut c_void) -> c_int,
) -> c_int {
    let want = match msg_as::<io::Stat>(msg).format {
        0 => 3,
        w => w,
    };
    c.status = 0;
    // SAFETY: the server's function, with the binding's OCB.
    let r = unsafe { f(c, msg, b.ocb) };
    if r != -1 {
        return r;
    }
    let have = match (c.status, c.iov[0].len) {
        (0, 0x48) => 1,
        (0, 0x60) => 2,
        (0, 0x78) | (0, _) => 3,
        // FreeBSD's `struct stat`: for the client, or `qnxd`, as it is.
        (s, _) if s as u32 == proto::stat::FORM_FREEBSD => return r,
        (s, _) => s as u32,
    };
    let st = c.iov[0].base.cast_mut().cast::<u8>();
    // SAFETY: the reply buffer the server set: the message buffer, big
    // enough for the largest form.
    let buf = unsafe { core::slice::from_raw_parts_mut(st, 0x78) };
    match crate::iofunc::convert_stat(buf, have, want) {
        Ok(len) => {
            c.status = c_long::from(want as c_int);
            c.iov[0].len = len;
            -1
        }
        Err(e) => e,
    }
}

/// `_IO_CLOSE`: unbinds the connection, calls the close function for
/// it, and drops its reference, the last of which closes the OCB.
fn close(c: &mut Context) -> c_int {
    let key = Key::of(&c.info);
    let Some(b) = remove_binding(|e| key.matches(e)) else {
        return EBADF;
    };
    let r = match io_func(b.funcs, IO_CLOSE) {
        Some(f) => {
            c.offset = 0;
            c.size = c.info.msglen.max(0) as usize;
            c.id = b.id;
            let msg = c.msg;
            // SAFETY: the server's close function, with its OCB.
            unsafe { f(c, msg, b.ocb) }
        }
        None => 0,
    };
    put_binding(c, b);
    r
}

/// `_IO_DUP`: binds the message's connection to the binding of the
/// connection the message names.
fn dup(c: &mut Context) -> c_int {
    // SAFETY: an _IO_DUP message: the original connection's info at 4.
    let key = unsafe { Key::at(c.msg.wrapping_add(4)) };
    let Some(b) = get_binding(key) else {
        return EBADF;
    };
    c.id = b.id;
    if let Some(f) = io_func(b.funcs, IO_DUP) {
        let msg = c.msg;
        // SAFETY: the server's dup function, with its OCB.
        let r = unsafe { f(c, msg, b.ocb) };
        if r != 0 && r != RESMGR_DEFAULT {
            put_binding(c, b);
            return r;
        }
    }
    // The reference get_binding() took becomes the new connection's.
    match add_binding(Key::of(&c.info), core::ptr::from_ref(b).cast_mut()) {
        Ok(()) => 0,
        Err(e) => {
            put_binding(c, b);
            e
        }
    }
}

/// The disconnect pulse: a client process went away, or closed its last
/// connection on `scoid`. Every binding on it is closed.
fn disconnect(c: &mut Context, scoid: c_int) {
    c.info.scoid = scoid;
    while let Some(b) = remove_binding(|e| e.scoid == scoid) {
        if let Some(f) = io_func(b.funcs, IO_CLOSE) {
            let msg = c.msg;
            // SAFETY: the buffer holds at least a pulse's 24 bytes.
            unsafe { msg.cast::<u16>().write(IO_CLOSE) };
            // SAFETY: as above.
            unsafe { msg.wrapping_add(2).cast::<u16>().write(4) };
            c.info.msglen = 4;
            c.size = 4;
            c.offset = 0;
            c.id = b.id;
            // SAFETY: the server's close function, with its OCB.
            unsafe { f(c, msg, b.ocb) };
        }
        put_binding(c, b);
    }
    let _ = ConnectDetach_r(scoid);
}

/// The unblock pulse: the client of message `rcvid` wants to be
/// unblocked (a signal or a timeout). The binding's unblock function
/// decides, and its result replies to that client.
fn unblock(c: &mut Context, rcvid: i64) -> c_int {
    c.rcvid = rcvid;
    if MsgInfo_r(rcvid, (&raw mut c.info).cast()) != 0 || c.info.flags & MI_UNBLOCK_REQ == 0 {
        c.rcvid = 0;
        return RESMGR_DEFAULT;
    }
    let r = match get_binding(Key::of(&c.info)) {
        Some(b) => {
            let r = match io_func(b.funcs, IO_RSVD_UNBLOCK) {
                Some(f) => {
                    let msg = c.msg;
                    // SAFETY: the server's unblock function, with its OCB.
                    unsafe { f(c, msg, b.ocb) }
                }
                None => RESMGR_DEFAULT,
            };
            put_binding(c, b);
            r
        }
        None => match UNBLOCK_UNBOUND.load(Ordering::Relaxed) as usize {
            0 => RESMGR_DEFAULT,
            // SAFETY: only `unblock_unbound()` stores it, a function so typed.
            f => unsafe { core::mem::transmute::<usize, unsafe extern "C" fn(*mut Context) -> c_int>(f)(c) },
        },
    };
    if r == RESMGR_DEFAULT {
        c.rcvid = 0;
    }
    r
}

/// What to do for an unblock on a connection with no OCB: a message that
/// is making one, such as `accept()`'s `_IO_OPENFD`, and has blocked.
static UNBLOCK_UNBOUND: core::sync::atomic::AtomicPtr<c_void> = core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

/// Sets the function for an unblock on a connection with no OCB.
pub fn unblock_unbound(f: unsafe extern "C" fn(*mut Context) -> c_int) {
    UNBLOCK_UNBOUND.store(f as *mut c_void, Ordering::Relaxed);
}

/// A context for `resmgr_block()`: the dispatch layer's.
///
/// # Safety
///
/// `dpp` is a dispatch handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_context_alloc(dpp: *mut Dispatch) -> *mut Context {
    // SAFETY: the caller's guarantee.
    unsafe { crate::dispatch::dispatch_context_alloc(dpp) }
}

/// Frees a context.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_context_free(ctp: *mut Context) {
    // SAFETY: the caller's guarantee.
    unsafe { crate::dispatch::dispatch_context_free(ctp) }
}

/// Receives a message, as `dispatch_block()` does without its timeout.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_block(ctp: *mut Context) -> *mut Context {
    // SAFETY: the caller's guarantee.
    unsafe { crate::dispatch::dispatch_block(ctp) }
}

/// Wakes a thread blocked in `resmgr_block()`.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_unblock(ctp: *mut Context) {
    // SAFETY: the caller's guarantee.
    unsafe { crate::dispatch::dispatch_unblock(ctp) }
}

/// Handles the message `resmgr_block()` received.
///
/// # Safety
///
/// `ctp` is a context with a message in it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resmgr_handler(ctp: *mut Context) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { handle(ctp) };
    0
}

/// Pausing messages (`resmgr_pause()` and its family): not supported.
#[unsafe(no_mangle)]
pub extern "C" fn resmgr_pause(_ctp: *mut Context) -> c_int {
    fail(ENOSYS)
}

/// As `resmgr_pause()`.
#[unsafe(no_mangle)]
pub extern "C" fn resmgr_pause_queue(
    _ctp: *mut Context,
    _data: *const c_void,
    _size: usize,
) -> c_int {
    fail(ENOSYS)
}

/// As `resmgr_pause()`.
#[unsafe(no_mangle)]
pub extern "C" fn resmgr_pause_resume(
    _ctp: *mut Context,
    _ocb: *mut c_void,
    _f: *const c_void,
) -> c_int {
    fail(ENOSYS)
}
