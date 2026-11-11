//! Names, `name_attach()` and `name_open()`: a server's channel under
//! `/dev/name/local/`, which clients find by name and talk to with
//! messages of their own, without the resource manager's I/O protocol.

use core::ffi::{CStr, c_char, c_int, c_uint};
use core::ptr::null_mut;

use kercall::path::{self, Request};

use crate::abi::*;
use crate::dispatch::{Dispatch, dispatch_create, dispatch_destroy, fail};
use crate::mem::{calloc, free};
use proto::connect::{CONNECT_OPEN, FTYPE_NAME};
use proto::fcntl::{O_RDWR, SH_DENYNO};

/// Where names live in the path manager's namespace.
const PREFIX: &[u8] = b"/dev/name/local/";
/// `NAME_FLAG_DETACH_SAVEDPP`: keep the dispatch handle.
const DETACH_SAVEDPP: c_uint = 1;
/// The longest name.
const NAME_MAX: usize = 255;

/// `PREFIX` and `name`, as a string in `buf`.
fn full_path<'a>(name: &[u8], buf: &'a mut [u8; PREFIX.len() + NAME_MAX + 1]) -> Option<&'a CStr> {
    if name.len() > NAME_MAX {
        return None;
    }
    buf[..PREFIX.len()].copy_from_slice(PREFIX);
    buf[PREFIX.len()..PREFIX.len() + name.len()].copy_from_slice(name);
    buf[PREFIX.len() + name.len()] = 0;
    CStr::from_bytes_until_nul(buf).ok()
}

/// Attaches `name` (relative, without `..`) to the channel of `dpp`, or
/// of a new dispatch handle if it is null. Null with `errno` set on
/// failure.
///
/// # Safety
///
/// `dpp` is null or a dispatch handle; `name` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn name_attach(
    dpp: *mut Dispatch,
    name: *const c_char,
    flags: c_uint,
) -> *mut NameAttach {
    let bad = || {
        kercall::tls::set_errno(EINVAL);
        null_mut()
    };
    if name.is_null() {
        return bad();
    }
    // SAFETY: the caller's guarantee.
    let n = unsafe { CStr::from_ptr(name) }.to_bytes();
    if n.is_empty() || n[0] == b'/' || n.windows(3).any(|w| w == b"../") || n.ends_with(b"..") {
        return bad();
    }
    let mut buf = [0u8; PREFIX.len() + NAME_MAX + 1];
    let Some(path) = full_path(n, &mut buf) else {
        kercall::tls::set_errno(ENOMEM);
        return null_mut();
    };
    let own = dpp.is_null();
    let d = if own { dispatch_create() } else { dpp };
    if d.is_null() {
        return null_mut();
    }
    // SAFETY: a dispatch handle; the path is a string.
    let id =
        unsafe { crate::resmgr::resmgr_attach_name(d, path.as_ptr(), FTYPE_NAME, flags & 0x31f) };
    if id == -1 {
        if own {
            // SAFETY: created above.
            unsafe { dispatch_destroy(d) };
        }
        return null_mut();
    }
    let attach = calloc::<NameAttach>();
    if attach.is_null() {
        // SAFETY: the attachment just made.
        unsafe { crate::resmgr::resmgr_detach(d, id, 0) };
        if own {
            // SAFETY: as above.
            unsafe { dispatch_destroy(d) };
        }
        kercall::tls::set_errno(ENOMEM);
        return null_mut();
    }
    // SAFETY: d is a dispatch handle.
    let chid = unsafe { (*d).chid };
    // SAFETY: allocated for a NameAttach.
    unsafe {
        attach.write(NameAttach {
            dpp: d,
            chid,
            mntid: id,
            zero: [0; 2],
        });
    }
    attach
}

/// Detaches the name and, unless `NAME_FLAG_DETACH_SAVEDPP`, destroys
/// its dispatch handle.
///
/// # Safety
///
/// `attach` is null or what `name_attach()` returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn name_detach(attach: *mut NameAttach, flags: c_uint) -> c_int {
    if attach.is_null() {
        return 0;
    }
    // SAFETY: the caller's guarantee.
    let a = unsafe { &*attach };
    // SAFETY: as above.
    let r = unsafe { crate::resmgr::resmgr_detach(a.dpp, a.mntid, 0) };
    if flags & DETACH_SAVEDPP == 0 {
        // SAFETY: as above.
        unsafe { dispatch_destroy(a.dpp) };
    }
    // SAFETY: allocated by name_attach().
    unsafe { free(attach.cast()) };
    r
}

const SYSMGR_COID: c_uint = 0x4000_0000;

/// A connection to the server that attached `name`: relative names are
/// under `/dev/name/local/`. The connection is not a file descriptor.
/// Returns it, or -1 with `errno` set.
///
/// # Safety
///
/// `name` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn name_open(name: *const c_char, _flags: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    if name.is_null() || unsafe { name.read() } == 0 {
        return fail(EINVAL);
    }
    let mut buf = [0u8; PREFIX.len() + NAME_MAX + 1];
    // SAFETY: as above.
    let path = if unsafe { name.read() } == b'/' as c_char {
        name
    } else {
        // SAFETY: as above.
        match full_path(unsafe { CStr::from_ptr(name) }.to_bytes(), &mut buf) {
            Some(p) => p.as_ptr(),
            None => return fail(ENOMEM),
        }
    };
    let request = Request {
        subtype: CONNECT_OPEN,
        file_type: FTYPE_NAME,
        ioflag: O_RDWR,
        mode: 0,
        sflag: SH_DENYNO,
        access: 3,
        dirfd: None,
        extra_type: 0,
        extra: &[],
        base: SYSMGR_COID,
        cancellable: false,
        coflags: 0,
    };
    // SAFETY: path is a string.
    match unsafe { path::connect(&request, path, &mut []) } {
        Ok(c) => c.coid,
        Err(e) => fail(e),
    }
}

/// Closes a connection `name_open()` made.
#[unsafe(no_mangle)]
pub extern "C" fn name_close(coid: c_int) -> c_int {
    kercall::neutrino::connect::ConnectDetach(coid)
}
