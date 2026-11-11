//! `nmount()`, `unmount()` and `getfsstat()`. A file system here is a
//! server, so a mount is a request to the device's server: it attaches
//! the path itself. There is no kernel mount table: the servers keep
//! `/var/run/mounts`, a line `from on fstype` for each mount.

use core::ffi::{c_char, c_int, c_void};

use proto::errno::{EINVAL, ENODEV, EFAULT};
use proto::fcntl::O_RDWR;

use crate::errno::fail;
use crate::fs::{__sys_close, __sys_open};
use crate::io::__sys_read;
use crate::misc2::{__sys_statfs, Statfs};
use crate::tty::{DEVDIR_NONE, DEVDIR_TO, dcmd, devctl};

/// `diskd`'s commands: 'M', 1 and 2.
const GROUP: u32 = b'M' as u32;
const MOUNT: u32 = 1;
const UNMOUNT: u32 = 2;
const MNT_FORCE: u64 = 0x0000_0000_0008_0000;
const PATH_MAX: usize = 1024;
const TABLE: usize = 8192;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Iovec {
    base: *const u8,
    len: usize,
}

fn string(i: &Iovec) -> &[u8] {
    if i.base.is_null() || i.len == 0 {
        return &[];
    }
    // SAFETY: `nmount()`'s caller's guarantee: `len` bytes, the last a NUL.
    unsafe { core::slice::from_raw_parts(i.base, i.len - 1) }
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
}

/// `nmount(iov, niov, flags)`: name and value pairs. Only `fstype`,
/// `from` and `fspath` matter here.
///
/// # Safety
///
/// `iov` holds `niov` strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_nmount(iov: *const Iovec, niov: u32, _flags: u64) -> c_int {
    if iov.is_null() || niov % 2 != 0 {
        return fail(EINVAL) as c_int;
    }
    // SAFETY: the caller's guarantee.
    let pairs = unsafe { core::slice::from_raw_parts(iov, niov as usize) };
    let (mut kind, mut from, mut path): (&[u8], &[u8], &[u8]) = (&[], &[], &[]);
    for pair in pairs.chunks_exact(2) {
        let name = string(&pair[0]);
        if same(name, b"fstype") {
            kind = string(&pair[1]);
        } else if same(name, b"from") {
            from = string(&pair[1]);
        } else if same(name, b"fspath") {
            path = string(&pair[1]);
        }
    }
    if !same(kind, b"msdosfs") {
        return fail(ENODEV) as c_int;
    }
    let (mut device, mut target) = ([0u8; PATH_MAX], [0u8; PATH_MAX]);
    if from.is_empty() || path.is_empty() || from.len() >= PATH_MAX || path.len() >= PATH_MAX {
        return fail(EINVAL) as c_int;
    }
    device[..from.len()].copy_from_slice(from);
    target[..path.len()].copy_from_slice(path);
    // SAFETY: NUL-terminated: shorter than the array.
    let fd = unsafe { __sys_open(device.as_ptr().cast(), O_RDWR as c_int, 0) };
    if fd < 0 {
        return -1;
    }
    let size = path.len() + 1;
    // SAFETY: the array, of that many bytes.
    let r = unsafe { devctl(fd, dcmd(DEVDIR_TO, GROUP, MOUNT, size), target.as_mut_ptr().cast(), size) };
    __sys_close(fd);
    r.map_or_else(|e| fail(e) as c_int, |_| 0)
}

/// `unmount(path, flags)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_unmount(path: *const c_char, flags: u64) -> c_int {
    // SAFETY: the caller's guarantee.
    let fd = unsafe { __sys_open(path, 0, 0) };
    if fd < 0 {
        return -1;
    }
    let mut force = c_int::from(flags & MNT_FORCE != 0);
    // SAFETY: an int, on this stack.
    let r = unsafe { devctl(fd, dcmd(DEVDIR_TO, GROUP, UNMOUNT, 4), (&raw mut force).cast::<c_void>(), 4) };
    __sys_close(fd);
    let _ = DEVDIR_NONE;
    r.map_or_else(|e| fail(e) as c_int, |_| 0)
}

/// `getfsstat(buf, size, flags)`: the table, with each mount's numbers.
///
/// # Safety
///
/// `buf` is null, or holds `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getfsstat(buf: *mut Statfs, size: i64, _flags: c_int) -> c_int {
    let mut table = [0u8; TABLE];
    // The image's root is always there, as a system's root is.
    let root = b"/dev/ifs / imagefs\n";
    table[..root.len()].copy_from_slice(root);
    let mut n = root.len();
    // SAFETY: a string.
    let fd = unsafe { __sys_open(c"/var/run/mounts".as_ptr().cast(), 0, 0) };
    if fd >= 0 {
        // SAFETY: the rest of the array.
        let got = unsafe { __sys_read(fd, table[n..].as_mut_ptr().cast(), TABLE - n) };
        __sys_close(fd);
        n += got.max(0) as usize;
    }
    let room = usize::try_from(size).unwrap_or(0) / size_of::<Statfs>();
    let mut count = 0;
    for line in table[..n].split(|&c| c == b'\n') {
        let mut words = line.split(|&c| c == b' ');
        let (Some(from), Some(on), Some(kind)) = (words.next(), words.next(), words.next()) else { continue };
        if !buf.is_null() && count < room {
            let mut path = [0u8; PATH_MAX];
            if on.len() >= PATH_MAX {
                return fail(EFAULT) as c_int;
            }
            path[..on.len()].copy_from_slice(on);
            // SAFETY: the caller's room for one more.
            let out = unsafe { buf.add(count) };
            // SAFETY: a NUL-terminated path; a statfs. A failure leaves
            // the names to fill.
            let _ = unsafe { __sys_statfs(path.as_ptr().cast(), out) };
            // SAFETY: as above.
            unsafe { (*out).name(from, on, kind) };
        }
        count += 1;
    }
    count.min(if buf.is_null() { usize::MAX } else { room }) as c_int
}
