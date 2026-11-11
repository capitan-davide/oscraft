//! The dispatch layer, `<sys/dispatch.h>`: a channel, the handlers
//! attached to it, and the loop that receives a message and calls the
//! handler for it.
//!
//! A server makes a dispatch handle (`dispatch_create()`), attaches
//! handlers to it (`message_attach()` for ranges of message types,
//! `pulse_attach()` for pulse codes, `resmgr_attach()` for paths, which
//! attaches the resource manager's own handler to every I/O message
//! type), allocates a context per thread, and loops: `dispatch_block()`
//! receives into the context, `dispatch_handler()` calls the handler.

use core::ffi::{c_int, c_uint, c_void};
use core::ptr::null_mut;

use kercall::neutrino::channel::{ChannelCreate, ChannelDestroy_r};
use kercall::neutrino::connect::{ConnectAttach_r, ConnectDetach_r};
use kercall::neutrino::msg::{MsgError_r, MsgReceive_r, MsgSendPulse_r};
use kercall::neutrino::timer::TimerTimeout_r;
use crate::compat::Mutex;
use kercall::tls;

use crate::abi::*;
use crate::mem::{calloc, free};
use proto::errno::ETIMEDOUT;
use proto::event::Pulse;
use proto::io::Header;
use proto::pod::Pod;

/// Reads a received message's header, a `T` of `proto`'s.
fn rd<T: Pod>(msg: *const u8) -> T {
    // SAFETY: the receive buffer holds at least the header; any bytes
    // are a T.
    unsafe { msg.cast::<T>().read_unaligned() }
}

/// `dispatch_t`: a channel and its handlers. Its layout is the library's
/// own: servers only hold pointers to it.
pub struct Dispatch {
    pub chid: c_int,
    /// The size of the receive buffer, and the reply parts, each context
    /// gets: the largest any attachment asked for.
    msg_max_size: usize,
    nparts_max: c_uint,
    /// Whether a context was allocated: the sizes may no longer grow.
    allocated: bool,
    /// `dispatch_timeout()`'s timeout, in nanoseconds, if any.
    timeout: Option<u64>,
    lock: Mutex,
    handlers: *mut Handler,
    /// Whether `resmgr_attach()` attached the resource manager's handlers.
    pub resmgr: bool,
    /// `resmgr_attr_t`'s `other_func`, for messages nothing else handles.
    pub other_func: Option<unsafe extern "C" fn(*mut Context, *mut c_void) -> c_int>,
    /// The descriptors `select_attach()` watches.
    pub select: *mut crate::select::Selects,
}

/// One attached handler.
struct Handler {
    next: *mut Handler,
    kind: Kind,
    lo: c_int,
    hi: c_int,
    func: MessageFn,
    handle: *mut c_void,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Message,
    Pulse,
    /// `MSG_FLAG_TIMEOUT_FUNC`: called when `dispatch_block()` times out.
    Timeout,
    /// `MSG_FLAG_DEFAULT_FUNC`: called for what no other handler takes.
    Default,
}

/// `MSG_FLAG_*` (`<sys/dispatch.h>`).
const MSG_FLAG_TYPE_PULSE: c_uint = 0x1;
const MSG_FLAG_ALLOC_PULSE: c_uint = 0x2;
const MSG_FLAG_TIMEOUT_FUNC: c_uint = 0x80;
const MSG_FLAG_DEFAULT_FUNC: c_uint = 0x100;
const MSG_FLAG_SIDE_CHANNEL: c_int = 0x200;
/// `_NTO_CHF_UNBLOCK | _NTO_CHF_DISCONNECT | _NTO_CHF_COID_DISCONNECT`:
/// the kernel tells the server when a client wants to be unblocked and
/// when one goes away.
const CHANNEL_FLAGS: c_uint = 0x4a;
/// `_NTO_SIDE_CHANNEL`: a connection that is not a file descriptor.
const SIDE_CHANNEL: c_uint = 0x4000_0000;
/// The default receive buffer: QNX's.
const DEFAULT_MSG_SIZE: usize = 0x828;
/// `CLOCK_MONOTONIC`, `_NTO_TIMEOUT_RECEIVE`, `ETIMEDOUT`.
const CLOCK_MONOTONIC: c_int = 2;
const TIMEOUT_RECEIVE: u32 = 1 << 5;

/// Stores `error` in `errno`; null.
fn null_with<T>(error: c_int) -> *mut T {
    tls::set_errno(error);
    null_mut()
}

/// Stores `error` in `errno`; -1.
pub(crate) fn fail(error: c_int) -> c_int {
    tls::set_errno(error);
    -1
}

/// A dispatch handle on channel `chid`, or on a new channel if it is -1.
/// Null, with `errno` set, on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dispatch_create_channel(chid: c_int, _flags: c_uint) -> *mut Dispatch {
    let chid = if chid == -1 {
        ChannelCreate(CHANNEL_FLAGS)
    } else {
        chid
    };
    if chid == -1 {
        return null_mut();
    }
    let dpp = calloc::<Dispatch>();
    if dpp.is_null() {
        return null_with(ENOMEM);
    }
    // SAFETY: allocated for a Dispatch.
    unsafe {
        dpp.write(Dispatch {
            chid,
            msg_max_size: DEFAULT_MSG_SIZE,
            nparts_max: 1,
            allocated: false,
            timeout: None,
            lock: Mutex::new(),
            handlers: null_mut(),
            resmgr: false,
            other_func: None,
            select: null_mut(),
        });
    }
    dpp
}

/// A dispatch handle on a new channel.
#[unsafe(no_mangle)]
pub extern "C" fn dispatch_create() -> *mut Dispatch {
    dispatch_create_channel(-1, 0)
}

/// QNX's internal name for `dispatch_create_channel()`.
#[unsafe(no_mangle)]
pub extern "C" fn _dispatch_create(chid: c_int, flags: c_uint) -> *mut Dispatch {
    dispatch_create_channel(chid, flags)
}

/// Destroys the handle and its channel.
///
/// # Safety
///
/// `dpp` is a dispatch handle no thread is using.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dispatch_destroy(dpp: *mut Dispatch) -> c_int {
    if dpp.is_null() {
        return fail(EINVAL);
    }
    // SAFETY: the caller's guarantee.
    let d = unsafe { &mut *dpp };
    let _ = ChannelDestroy_r(d.chid);
    let mut h = d.handlers;
    while !h.is_null() {
        // SAFETY: a handler of the list, allocated by attach().
        let next = unsafe { (*h).next };
        // SAFETY: as above.
        unsafe { free(h.cast()) };
        h = next;
    }
    // SAFETY: allocated by dispatch_create_channel().
    unsafe { free(dpp.cast()) };
    0
}

/// QNX's hash-table size for bindings; this library's table is fixed.
#[unsafe(no_mangle)]
pub extern "C" fn dispatch_set_hash_size(_dpp: *mut Dispatch, _size: c_uint) -> c_int {
    0
}

impl Dispatch {
    /// Grows the contexts' receive buffer and reply parts to at least
    /// `size` and `nparts`. Once a context exists they may not grow.
    pub(crate) fn set_context_size(&mut self, size: usize, nparts: c_uint) -> Result<(), c_int> {
        if nparts > 0x8_0000 {
            return Err(EINVAL);
        }
        let _guard = self.lock.lock();
        if self.allocated && (size > self.msg_max_size || nparts > self.nparts_max) {
            return Err(EINVAL);
        }
        self.msg_max_size = self.msg_max_size.max(size);
        self.nparts_max = self.nparts_max.max(nparts);
        Ok(())
    }

    /// Whether a handler of `kind` overlaps `lo..=hi`.
    fn overlaps(&self, kind: Kind, lo: c_int, hi: c_int) -> bool {
        let mut h = self.handlers;
        while !h.is_null() {
            // SAFETY: a handler of the list, under the lock.
            let e = unsafe { &*h };
            let same = e.kind == kind;
            let ranged = matches!(kind, Kind::Message | Kind::Pulse);
            if same && (!ranged || (e.lo <= hi && lo <= e.hi)) {
                return true;
            }
            h = e.next;
        }
        false
    }

    /// The lowest pulse code from 0 that no handler takes, for
    /// `MSG_FLAG_ALLOC_PULSE`.
    fn free_pulse_code(&self) -> Option<c_int> {
        (0..=127).find(|&c| !self.overlaps(Kind::Pulse, c, c))
    }

    /// The handler for what `ctp` received: its function, handle, and the
    /// code or type it is called with.
    fn find(
        &self,
        ctp: &Context,
    ) -> Option<(
        unsafe extern "C" fn(*mut Context, c_int, c_uint, *mut c_void) -> c_int,
        *mut c_void,
        c_int,
    )> {
        let _guard = self.lock.lock();
        let (kind, value) = match ctp.rcvid {
            -1 => (Kind::Timeout, 0),
            // A pulse is in the buffer, its code at byte 4; or a message
            // of at least two bytes, its type.
            0 => (Kind::Pulse, c_int::from(rd::<Pulse>(ctp.msg).code)),
            _ => (Kind::Message, c_int::from(rd::<Header>(ctp.msg).r#type)),
        };
        let mut fallback = None;
        let mut h = self.handlers;
        while !h.is_null() {
            // SAFETY: a handler of the list, under the lock.
            let e = unsafe { &*h };
            if let Some(func) = e.func {
                if e.kind == Kind::Default && kind != Kind::Timeout {
                    fallback = Some((func, e.handle, value));
                } else if e.kind == kind
                    && (kind == Kind::Timeout || (e.lo <= value && value <= e.hi))
                {
                    return Some((func, e.handle, value));
                }
            }
            h = e.next;
        }
        fallback
    }
}

/// Attaches `func` for the message types `low` to `high`, or with
/// `MSG_FLAG_TYPE_PULSE` for the pulse codes; `MSG_FLAG_ALLOC_PULSE`
/// picks a free code and returns it. Returns 0, the code, or -1 with
/// `errno` set.
///
/// # Safety
///
/// `dpp` is a dispatch handle; `attr` is null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn message_attach(
    dpp: *mut Dispatch,
    attr: *const MessageAttr,
    low: c_int,
    high: c_int,
    func: MessageFn,
    handle: *mut c_void,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { attr.as_ref() };
    let flags = attr.map_or(0, |a| a.flags);
    let kind = if flags & MSG_FLAG_TIMEOUT_FUNC != 0 {
        Kind::Timeout
    } else if flags & MSG_FLAG_DEFAULT_FUNC != 0 {
        Kind::Default
    } else if flags & MSG_FLAG_TYPE_PULSE != 0 {
        Kind::Pulse
    } else {
        Kind::Message
    };
    let alloc = flags & MSG_FLAG_ALLOC_PULSE != 0;
    if alloc && kind != Kind::Pulse {
        return fail(EINVAL);
    }
    match kind {
        Kind::Message if low > high || low < 0 || high > 0xffff => return fail(EINVAL),
        Kind::Pulse if !alloc && (low > high || low < -128 || high > 127) => return fail(EINVAL),
        _ => {}
    }
    // SAFETY: the caller's guarantee.
    let d = unsafe { &mut *dpp };
    if let Some(a) = attr
        && let Err(error) = d.set_context_size(a.msg_max_size, a.nparts_max)
    {
        return fail(error);
    }
    let _guard = d.lock.lock();
    let (lo, hi) = if alloc {
        match d.free_pulse_code() {
            Some(c) => (c, c),
            None => return fail(EAGAIN),
        }
    } else {
        (low, high)
    };
    if d.overlaps(kind, lo, hi) {
        return fail(EINVAL);
    }
    let h = calloc::<Handler>();
    if h.is_null() {
        return fail(ENOMEM);
    }
    // SAFETY: allocated for a Handler; the list is under the lock.
    unsafe {
        h.write(Handler {
            next: d.handlers,
            kind,
            lo,
            hi,
            func,
            handle,
        });
    }
    d.handlers = h;
    // A pulse's code is returned, as QNX's library does; 0 for messages.
    if kind == Kind::Pulse { lo } else { 0 }
}

/// Attaches `func` for pulse `code`; `MSG_FLAG_ALLOC_PULSE` in `flags`
/// picks a free code and returns it.
///
/// # Safety
///
/// As `message_attach()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pulse_attach(
    dpp: *mut Dispatch,
    flags: c_int,
    code: c_int,
    func: MessageFn,
    handle: *mut c_void,
) -> c_int {
    let attr = MessageAttr {
        flags: (flags as c_uint & MSG_FLAG_ALLOC_PULSE) | MSG_FLAG_TYPE_PULSE,
        nparts_max: 0,
        msg_max_size: 0,
        reserved: [0; 5],
    };
    // SAFETY: the caller's guarantee.
    unsafe { message_attach(dpp, &attr, code, code, func, handle) }
}

/// Detaches the handler of `kind` that starts at `lo`.
///
/// # Safety
///
/// `dpp` is a dispatch handle.
unsafe fn detach(dpp: *mut Dispatch, kind: Kind, lo: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    let d = unsafe { &mut *dpp };
    let _guard = d.lock.lock();
    let mut link = &raw mut d.handlers;
    loop {
        // SAFETY: link is the list's head or a handler's next.
        let h = unsafe { link.read() };
        if h.is_null() {
            return fail(EINVAL);
        }
        // SAFETY: a handler of the list.
        let e = unsafe { &mut *h };
        if e.kind == kind && e.lo == lo {
            // SAFETY: as above.
            unsafe { link.write(e.next) };
            // SAFETY: allocated by message_attach().
            unsafe { free(h.cast()) };
            return 0;
        }
        link = &raw mut e.next;
    }
}

/// Detaches the message handler for `low` to `high`.
///
/// # Safety
///
/// `dpp` is a dispatch handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn message_detach(
    dpp: *mut Dispatch,
    low: c_int,
    _high: c_int,
    flags: c_int,
) -> c_int {
    let kind = if flags as c_uint & MSG_FLAG_TYPE_PULSE != 0 {
        Kind::Pulse
    } else {
        Kind::Message
    };
    // SAFETY: the caller's guarantee.
    unsafe { detach(dpp, kind, low) }
}

/// Detaches the handler for pulse `code`.
///
/// # Safety
///
/// `dpp` is a dispatch handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pulse_detach(dpp: *mut Dispatch, code: c_int, _flags: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { detach(dpp, Kind::Pulse, code) }
}

/// A connection to the handle's own channel, for sending it pulses:
/// a side channel with `MSG_FLAG_SIDE_CHANNEL`.
///
/// # Safety
///
/// `dpp` is a dispatch handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn message_connect(dpp: *mut Dispatch, flags: c_int) -> c_int {
    let index = if flags & MSG_FLAG_SIDE_CHANNEL != 0 {
        SIDE_CHANNEL
    } else {
        0
    };
    // SAFETY: the caller's guarantee.
    let chid = unsafe { (*dpp).chid };
    let coid = ConnectAttach_r(0, 0, chid, index, 0);
    if coid < 0 { fail(-coid) } else { coid }
}

/// A context for one thread: the context, `nparts_max` more reply parts,
/// and the receive buffer. Null with `errno` set if nothing is attached
/// yet, or there is no memory.
///
/// # Safety
///
/// `dpp` is a dispatch handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dispatch_context_alloc(dpp: *mut Dispatch) -> *mut Context {
    // SAFETY: the caller's guarantee.
    let d = unsafe { &mut *dpp };
    let _guard = d.lock.lock();
    if d.handlers.is_null() {
        return null_with(EINVAL);
    }
    let head =
        size_of::<Context>() + d.nparts_max as usize * size_of::<kercall::abi::Iov>();
    let ctp = crate::mem::calloc_bytes(head + d.msg_max_size).cast::<Context>();
    if ctp.is_null() {
        return null_with(ENOMEM);
    }
    let extra = calloc::<ExtendedContext>();
    if extra.is_null() {
        // SAFETY: allocated above.
        unsafe { free(ctp.cast()) };
        return null_with(ENOMEM);
    }
    // SAFETY: allocated, zeroed, for a context and an extended context.
    let c = unsafe { &mut *ctp };
    // SAFETY: as above.
    unsafe { (*extra).length = size_of::<ExtendedContext>() };
    c.extra = extra;
    c.msg = ctp.cast::<u8>().wrapping_add(head);
    c.dpp = dpp;
    c.msg_max_size = d.msg_max_size;
    c.id = -1;
    d.allocated = true;
    ctp
}

/// Frees a context.
///
/// # Safety
///
/// `ctp` is a context `dispatch_context_alloc()` returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dispatch_context_free(ctp: *mut Context) {
    if ctp.is_null() {
        return;
    }
    // SAFETY: the caller's guarantee.
    let extra = unsafe { (*ctp).extra };
    // SAFETY: allocated by dispatch_context_alloc().
    unsafe { free(extra.cast()) };
    // SAFETY: as above.
    unsafe { free(ctp.cast()) };
}

/// Waits for a message or pulse on channel `chid`, into `ctp`. Returns
/// `ctp`, with `rcvid` -1 if `dispatch_timeout()`'s time passed, or null
/// with `errno` set.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dispatch_block_chid(ctp: *mut Context, chid: c_int) -> *mut Context {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    c.info.msglen = 0;
    c.id = -1;
    // SAFETY: the context's dispatch handle.
    let timeout = unsafe { (*c.dpp).timeout };
    if let Some(ns) = timeout {
        let r = TimerTimeout_r(
            CLOCK_MONOTONIC,
            TIMEOUT_RECEIVE,
            core::ptr::null(),
            &ns,
            null_mut(),
        );
        if r < 0 {
            return null_with(-r);
        }
    }
    let rcvid = MsgReceive_r(chid, c.msg.cast(), c.msg_max_size, (&raw mut c.info).cast());
    if rcvid < 0 {
        c.rcvid = -1;
        if -rcvid as c_int != ETIMEDOUT {
            return null_with(-rcvid as c_int);
        }
        tls::set_errno(ETIMEDOUT);
        return ctp;
    }
    c.rcvid = rcvid;
    ctp
}

/// Waits for a message or pulse on the handle's channel.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dispatch_block(ctp: *mut Context) -> *mut Context {
    // SAFETY: the caller's guarantee: the context and its dispatch handle.
    let dpp = unsafe { (*ctp).dpp };
    // SAFETY: as above.
    let chid = unsafe { (*dpp).chid };
    // SAFETY: the caller's guarantee.
    unsafe { dispatch_block_chid(ctp, chid) }
}

/// Wakes a thread blocked on the handle's channel: a pulse to it.
///
/// # Safety
///
/// `ctp` is a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dispatch_unblock(ctp: *mut Context) {
    // SAFETY: the caller's guarantee.
    let dpp = unsafe { (*ctp).dpp };
    // SAFETY: as above.
    let chid = unsafe { (*dpp).chid };
    let coid = ConnectAttach_r(0, 0, chid, SIDE_CHANNEL, 0);
    if coid >= 0 {
        let _ = MsgSendPulse_r(coid, -1, c_int::from(PULSE_CODE_UNBLOCK), 0);
        let _ = ConnectDetach_r(coid);
    }
}

/// Sets how long `dispatch_block()` waits, from each call; null for no
/// limit.
///
/// # Safety
///
/// `dpp` is a dispatch handle; `reltime` is null or a `timespec`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dispatch_timeout(dpp: *mut Dispatch, reltime: *const [i64; 2]) -> c_int {
    // SAFETY: the caller's guarantee.
    let t = unsafe { reltime.as_ref() };
    let ns = t.map(|t| {
        (t[0] as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(t[1] as u64)
    });
    // SAFETY: as above.
    let d = unsafe { &mut *dpp };
    let _guard = d.lock.lock();
    d.timeout = ns;
    0
}

/// Calls the handler for what `ctp` received: the message handler for
/// its type, the pulse handler for its code, the timeout handler. A
/// message nothing takes gets `ENOSYS`. Returns the handler's result, or
/// -1.
///
/// # Safety
///
/// `ctp` is a context `dispatch_block()` filled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dispatch_handler(ctp: *mut Context) -> c_int {
    if ctp.is_null() {
        return -1;
    }
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    if c.rcvid > 0 && c.info.msglen < 2 {
        let _ = MsgError_r(c.rcvid, EBADMSG);
        return -1;
    }
    // SAFETY: the context's dispatch handle.
    let found = unsafe { (*c.dpp).find(c) };
    match found {
        Some((func, handle, value)) => {
            c.offset = 0;
            c.size = c.info.msglen as usize;
            // SAFETY: an attached handler, called as attached.
            unsafe { func(ctp, value, 0, handle) }
        }
        None => {
            if c.rcvid > 0 {
                let _ = MsgError_r(c.rcvid, ENOSYS);
            }
            -1
        }
    }
}
