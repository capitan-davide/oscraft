#![allow(non_snake_case)]
//! What the framework took from oscraft-libc's own POSIX layer, over
//! FreeBSD's libc and libthr here: a mutex, thread keys, the process
//! ID, and the few client-side calls a server makes (`ionotify()`,
//! `devctlv()`, `iofdinfo()`, `fchown()`, the client-info calls).

use core::cell::UnsafeCell;
use core::ffi::{c_int, c_long, c_uint, c_void};
use core::ptr::null_mut;

use kercall::abi::Iov;
use kercall::neutrino::ask_nc;
use kercall::neutrino::connect::ConnectClientInfoExtraArgs;
use kercall::neutrino::msg::MsgSendv_r;
use kercall::tls;
use proto::errno::{EINVAL, ENOMEM, ENOSYS, ENOTTY};
use proto::event::Sigevent;
use proto::io::{IO_NOTIFY, Notify};

unsafe extern "C" {
    fn pthread_mutex_lock(m: *mut c_void) -> c_int;
    fn pthread_mutex_unlock(m: *mut c_void) -> c_int;
    #[link_name = "pthread_key_create"]
    fn c_pthread_key_create(key: *mut c_int, destructor: *const c_void) -> c_int;
    #[link_name = "pthread_getspecific"]
    fn c_pthread_getspecific(key: c_int) -> *mut c_void;
    #[link_name = "pthread_setspecific"]
    fn c_pthread_setspecific(key: c_int, value: *const c_void) -> c_int;
    #[link_name = "getpid"]
    fn c_getpid() -> c_int;
    #[link_name = "fchown"]
    fn c_fchown(fd: c_int, uid: c_uint, gid: c_uint) -> c_int;
    fn malloc(n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// libthr's thread keys, with the key as an int (oscraft-libc's signature).
///
/// # Safety
///
/// `key` is writable; `destructor` is a function or null.
pub unsafe fn pthread_key_create(key: *mut c_int, destructor: *const c_void) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { c_pthread_key_create(key, destructor) }
}

/// The calling thread's value for `key`.
pub fn pthread_getspecific(key: c_int) -> *mut c_void {
    // SAFETY: a key libthr made, or an error.
    unsafe { c_pthread_getspecific(key) }
}

/// Sets the calling thread's value for `key`.
///
/// # Safety
///
/// `value` is what the key's users expect.
pub unsafe fn pthread_setspecific(key: c_int, value: *const c_void) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { c_pthread_setspecific(key, value) }
}

/// The calling process's ID.
pub fn getpid() -> c_int {
    // SAFETY: a call without preconditions.
    unsafe { c_getpid() }
}

/// `fchown(fd, uid, gid)` from libc.
pub fn fchown(fd: c_int, uid: u32, gid: u32) -> c_int {
    // SAFETY: a call without memory preconditions.
    unsafe { c_fchown(fd, uid, gid) }
}

/// The framework's lock: libthr's mutex, a pointer libthr allocates on
/// first use (`PTHREAD_MUTEX_INITIALIZER` is null), so a `const` one is
/// a null word.
pub struct Mutex {
    inner: UnsafeCell<*mut c_void>,
}

// SAFETY: a pthread mutex may be used from any thread.
unsafe impl Sync for Mutex {}
// SAFETY: as above.
unsafe impl Send for Mutex {}

/// The mutex, held while the guard lives.
pub struct Guard<'a> {
    mutex: &'a Mutex,
}

impl Mutex {
    pub const fn new() -> Self {
        Mutex { inner: UnsafeCell::new(null_mut()) }
    }

    /// Takes the mutex.
    pub fn lock(&self) -> Guard<'_> {
        // SAFETY: libthr initialises a null mutex on first use.
        let r = unsafe { pthread_mutex_lock(self.inner.get().cast()) };
        assert!(r == 0, "pthread_mutex_lock failed: {r}");
        Guard { mutex: self }
    }

    /// Takes the mutex with no guard, for C's lock functions; release it
    /// with [`Mutex::raw_unlock`].
    pub fn raw_lock(&self) {
        core::mem::forget(self.lock());
    }

    /// Releases a mutex [`Mutex::raw_lock`] took.
    ///
    /// # Safety
    ///
    /// The calling thread holds the mutex, with no guard.
    pub unsafe fn raw_unlock(&self) {
        // SAFETY: the caller's guarantee.
        unsafe { pthread_mutex_unlock(self.inner.get().cast()) };
    }
}

impl Default for Mutex {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        // SAFETY: the guard holds the mutex.
        unsafe { pthread_mutex_unlock(self.mutex.inner.get().cast()) };
    }
}

/// `ionotify(fd, action, flags, event)`: the conditions that hold.
///
/// # Safety
///
/// `event` is null or a `struct sigevent`.
pub unsafe fn ionotify(fd: c_int, action: c_int, flags: c_int, event: *const c_void) -> c_int {
    let msg = Notify {
        r#type: IO_NOTIFY,
        action,
        flags: flags as u32,
        // SAFETY: the caller's guarantee.
        event: unsafe { event.cast::<Sigevent>().as_ref() }.copied().unwrap_or_default(),
        ..Notify::default()
    };
    // The reply's flags, the conditions that hold, are where the action was.
    match ask_nc::<_, Notify>(fd, &msg) {
        Ok((_, reply)) => reply.action,
        Err(e) => {
            tls::set_errno(e);
            -1
        }
    }
}

/// `devctlv(fd, dcmd, sparts, rparts, sv, rv, info)`: an `_IO_DEVCTL`
/// with its data in parts; an error number, or 0.
///
/// # Safety
///
/// `sv` holds `sparts` parts, `rv` `rparts`; `info` is null or writable.
pub unsafe fn devctlv(fd: c_int, dcmd: c_int, sparts: c_int, rparts: c_int, sv: *const Iov, rv: *const Iov, info: *mut c_int) -> c_int {
    let (ns, nr) = (sparts.max(0) as usize, rparts.max(0) as usize);
    // SAFETY: a new array of parts.
    let parts = unsafe { malloc((ns + nr + 2) * size_of::<Iov>()) }.cast::<Iov>();
    if parts.is_null() {
        return ENOMEM;
    }
    let mut msg = [0u8; 16];
    let (mut sbytes, mut rbytes) = (0usize, 0usize);
    let put = |k: usize, part: Iov| {
        // SAFETY: k is within the array's room.
        unsafe { parts.wrapping_add(k).write(part) };
    };
    put(0, Iov { base: msg.as_ptr().cast(), len: 16 });
    for k in 0..ns {
        // SAFETY: the caller's parts.
        let p = unsafe { sv.wrapping_add(k).read() };
        sbytes += p.len;
        put(1 + k, p);
    }
    put(1 + ns, Iov { base: msg.as_ptr().cast(), len: 16 });
    for k in 0..nr {
        // SAFETY: as above.
        let p = unsafe { rv.wrapping_add(k).read() };
        rbytes += p.len;
        put(2 + ns + k, p);
    }
    msg[..4].copy_from_slice(&0x10_0106u32.to_ne_bytes());
    msg[4..8].copy_from_slice(&dcmd.to_ne_bytes());
    msg[8..12].copy_from_slice(&(sbytes.max(rbytes) as u32).to_ne_bytes());
    let r = MsgSendv_r(fd, parts.cast(), ns + 1, parts.wrapping_add(1 + ns).cast(), nr + 1);
    // SAFETY: the array.
    unsafe { free(parts.cast()) };
    if r == 0 {
        if !info.is_null() {
            // SAFETY: the caller's guarantee; the reply's value.
            unsafe { *info = i32::from_ne_bytes(msg[4..8].try_into().unwrap_or_default()) };
        }
        return 0;
    }
    if r == -(ENOSYS as c_long) { ENOTTY } else { -r as c_int }
}

/// `iofdinfo(fd, flags, info, path, maxlen)`: the server's `_io_fdinfo`
/// and the file's path.
///
/// # Safety
///
/// `info` is null or writable for 64 bytes; `path` null or writable for
/// `maxlen`.
pub unsafe fn iofdinfo(fd: c_int, flags: c_uint, info: *mut c_void, path: *mut core::ffi::c_char, maxlen: c_int) -> c_int {
    let mut scratch = [0u8; 64];
    let info = if info.is_null() { scratch.as_mut_ptr() } else { info.cast() };
    let (path, maxlen) = if path.is_null() { (null_mut(), 0) } else { (path.cast(), maxlen.max(0) as usize) };
    // SAFETY: the caller's guarantee; scratch is 64 bytes.
    match unsafe { kercall::io::fdinfo(fd, flags, info, path, maxlen) } {
        Ok(n) => n,
        Err(error) => {
            tls::set_errno(error);
            -1
        }
    }
}

/// `struct _client_able`: an ability to check the client for, and the
/// answer.
#[repr(C)]
pub struct ClientAble {
    pub ability: c_uint,
    pub flags: c_uint,
    pub range_lo: u64,
    pub range_hi: u64,
}

/// `_NTO_CLIENTINFO_GETGROUPS`, `_NTO_CLIENTINFO_GETTYPEID`.
const GETGROUPS: c_int = 1;
const GETTYPEID: c_int = 2;
/// `struct _client_info` before its groups, in words, and the most groups.
const INFO_WORDS: usize = 11;
const GROUPS_MAX: usize = 128;
/// An ability number the library looked up before it was created: not
/// kept here (a server asking for one gets EINVAL).
const AID_UNCREATED: c_uint = 0x8000_0000;

/// What a server can learn about the client on `scoid`
/// (`ConnectClientInfoExt()`), and whether it has each of `abilities`:
/// in `*info_pp`, allocated, with its groups (`_NTO_CLIENTINFO_GETGROUPS`)
/// and its type ID after them (`_NTO_CLIENTINFO_GETTYPEID`).
///
/// # Safety
///
/// `info_pp` is null or writable; `abilities` holds `nable` entries.
pub unsafe fn ConnectClientInfoAble(scoid: c_int, info_pp: *mut *mut c_void, flags: c_int, abilities: *mut ClientAble, nable: c_int) -> c_int {
    if info_pp.is_null() {
        if flags & (GETGROUPS | GETTYPEID) != 0 {
            tls::set_errno(EINVAL);
            return -1;
        }
    } else {
        // SAFETY: the caller's guarantee.
        unsafe { *info_pp = null_mut() };
    }
    for k in 0..nable.max(0) as usize {
        // SAFETY: the caller's guarantee.
        let a = unsafe { &mut *abilities.wrapping_add(k) };
        if a.ability & AID_UNCREATED != 0 {
            tls::set_errno(EINVAL);
            return -1;
        }
    }
    let ngroups = if flags & GETGROUPS != 0 { GROUPS_MAX } else { 0 };
    let mut info = [0u32; INFO_WORDS + GROUPS_MAX];
    let mut type_id: c_uint = 0;
    let r = ConnectClientInfoExtraArgs(scoid, info.as_mut_ptr().cast(), (ngroups as u32 | 0x8000_0000) as c_int, abilities.cast(), nable, &mut type_id);
    if r == -1 {
        return -1;
    }
    if info_pp.is_null() {
        return 0;
    }
    let n = if flags & GETGROUPS != 0 { (info[10] as usize).min(GROUPS_MAX) } else { 0 };
    let typed = usize::from(flags & GETTYPEID != 0);
    // SAFETY: a new buffer.
    let p = unsafe { malloc((INFO_WORDS + n + typed) * 4) }.cast::<u32>();
    if p.is_null() {
        tls::set_errno(ENOMEM);
        return -1;
    }
    // SAFETY: room for the header, the groups and the type ID.
    unsafe { core::ptr::copy_nonoverlapping(info.as_ptr(), p, INFO_WORDS + n) };
    if typed != 0 {
        // SAFETY: as above.
        unsafe { p.wrapping_add(INFO_WORDS + n).write(type_id) };
    }
    if flags & GETGROUPS != 0 {
        // SAFETY: as above, the header's flags word: the group list is complete.
        unsafe { *p.wrapping_add(3) |= 0x8000_0000 };
    }
    // SAFETY: the caller's guarantee.
    unsafe { *info_pp = p.cast() };
    0
}

/// [`ConnectClientInfoAble`] without abilities, as QNX's
/// `ConnectClientInfoExt()`.
///
/// # Safety
///
/// `info` is writable.
pub unsafe fn ConnectClientInfoExt(scoid: c_int, info: *mut *mut c_void, flags: c_int) -> c_int {
    // SAFETY: the caller's guarantee; no abilities.
    unsafe { ConnectClientInfoAble(scoid, info, flags, null_mut(), 0) }
}

/// Frees what [`ConnectClientInfoExt`] allocated.
///
/// # Safety
///
/// `info` is its result, or null.
pub unsafe fn ClientInfoExtFree(info: *mut *mut c_void) -> c_int {
    if !info.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe {
            free(*info);
            *info = null_mut();
        }
    }
    0
}
