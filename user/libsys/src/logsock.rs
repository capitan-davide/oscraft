//! `syslog(3)`'s socket. libc connects a datagram socket to `/var/run/log`
//! and sends each message; here that is `syslogd`'s ring (`logring`),
//! written with no kernel call. `connect()` opens `/var/run/log`, whose
//! read names this process's ring, maps it, and puts that connection at
//! the socket's descriptor; its `send()`s go to the ring, with a pulse
//! when `syslogd` sleeps, which never blocks the sender. Without `syslogd`
//! the `connect()` fails and libc goes on as FreeBSD's would: the message
//! is lost, and the next `syslog()` tries again. A process with a ring
//! goes on writing to it across a restart of `syslogd`.

use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

use logring::Ring;
use proto::errno::ECONNREFUSED;
use proto::fcntl::O_RDWR;

use crate::errno::fail;
use crate::fd::{__sys_dup2, __sys_fcntl};
use crate::fs::{__sys_close, __sys_open};
use crate::io::__sys_read;
use crate::kercall;
use crate::mem::{__sys_mmap, __sys_munmap};

const AF_UNIX: u8 = 1;
const F_SETFD: c_int = 2;
const FD_CLOEXEC: isize = 1;
const PROT_RW: c_int = 3;
const MAP_SHARED: c_int = 1;
/// `syslogd`'s wake-up (`PULSE_WAKE`).
const PULSE_WAKE: i32 = 1;

/// The descriptor that stands for the socket, and the ring's mapping.
static FD: AtomicI32 = AtomicI32::new(-1);
static MAP: AtomicUsize = AtomicUsize::new(0);
static SIZE: AtomicUsize = AtomicUsize::new(0);

/// Whether `name` is `syslogd`'s socket: `sa_len`, `sa_family`, path.
///
/// # Safety
///
/// `name` holds `len` bytes.
unsafe fn is_log_socket(name: *const c_void, len: u32) -> bool {
    const PATH: &[u8] = b"/var/run/log";
    let len = len as usize;
    if len < 2 + PATH.len() {
        return false;
    }
    // SAFETY: the caller's guarantee.
    let b = unsafe { core::slice::from_raw_parts(name.cast::<u8>(), len) };
    // By hand: a slice comparison calls `memcmp`, which rtld hasn't got.
    b[1] == AF_UNIX
        && PATH.iter().zip(&b[2..]).all(|(a, c)| a == c)
        && b.get(2 + PATH.len()).is_none_or(|&c| c == 0 || c == b'p')
}

/// A stand-in for an `AF_UNIX` datagram socket when the socket server
/// isn't there (it starts late in the boot, and the base image has none):
/// `syslog()` needs no network stack, and `connect()` replaces this.
pub fn placeholder(domain: c_int, kind: c_int) -> Option<c_int> {
    const SOCK_DGRAM: c_int = 2;
    const SOCK_TYPE_MASK: c_int = 0xf;
    if domain != c_int::from(AF_UNIX) || kind & SOCK_TYPE_MASK != SOCK_DGRAM {
        return None;
    }
    // SAFETY: a string.
    let fd = unsafe { __sys_open(c"/dev/null".as_ptr().cast::<c_char>(), O_RDWR as c_int, 0) };
    (fd >= 0).then_some(fd)
}

/// `<path> <size>`, as `syslogd` answers.
fn parse_reply(reply: &[u8]) -> Option<(&[u8], usize)> {
    let space = reply.iter().position(|&c| c == b' ')?;
    let size = core::str::from_utf8(&reply[space + 1..]).ok()?.parse().ok()?;
    Some((&reply[..space], size))
}

/// Registers, and puts the connection at descriptor `s`. False without
/// `syslogd`.
fn register(s: c_int) -> bool {
    // SAFETY: a string.
    let fd = unsafe { __sys_open(c"/var/run/log".as_ptr().cast::<c_char>(), O_RDWR as c_int, 0) };
    if fd < 0 {
        return false;
    }
    let mut reply = [0u8; 128];
    // SAFETY: a buffer on this stack, one byte kept for the NUL.
    let n = unsafe { __sys_read(fd, reply.as_mut_ptr().cast(), reply.len() - 1) };
    let Some((path, size)) = usize::try_from(n).ok().and_then(|n| parse_reply(&reply[..n])) else {
        __sys_close(fd);
        return false;
    };
    let mut name = [0u8; 128];
    name[..path.len()].copy_from_slice(path);
    // SAFETY: NUL-terminated: the reply is shorter than the array.
    let ring = unsafe { __sys_open(name.as_ptr().cast(), O_RDWR as c_int, 0) };
    if ring < 0 {
        __sys_close(fd);
        return false;
    }
    // SAFETY: a new shared mapping of the file.
    let map = unsafe { __sys_mmap(core::ptr::null_mut(), size, PROT_RW, MAP_SHARED, ring, 0) };
    __sys_close(ring);
    closing(s);
    if map as isize == -1 || __sys_dup2(fd, s) < 0 {
        __sys_close(fd);
        return false;
    }
    __sys_close(fd);
    __sys_fcntl(s, F_SETFD, FD_CLOEXEC);
    MAP.store(map as usize, Ordering::Release);
    SIZE.store(size, Ordering::Release);
    FD.store(s, Ordering::Release);
    true
}

/// Connects `s` to the ring. `None` unless it is `syslogd`'s socket.
///
/// # Safety
///
/// As `connect()`.
pub unsafe fn connect(s: c_int, name: *const c_void, len: u32) -> Option<c_int> {
    // SAFETY: the caller's guarantee.
    if !unsafe { is_log_socket(name, len) } {
        return None;
    }
    Some(if register(s) { 0 } else { fail(ECONNREFUSED) as c_int })
}

/// `send()` on the socket: `None` for any other descriptor.
///
/// # Safety
///
/// `buf` holds `len` bytes.
pub unsafe fn send(fd: c_int, buf: *const c_void, len: usize) -> Option<isize> {
    if fd < 0 || FD.load(Ordering::Acquire) != fd {
        return None;
    }
    let (map, size) = (MAP.load(Ordering::Acquire), SIZE.load(Ordering::Acquire));
    // SAFETY: the mapping `connect()` made, of that size.
    let ring = unsafe { Ring::attach(map as *mut u8, size) }?;
    // SAFETY: the caller's guarantee.
    let message = unsafe { core::slice::from_raw_parts(buf.cast::<u8>(), len) };
    let now = kercall::clock_time(0).unwrap_or(0);
    if ring.write(0, now, message) && kercall::msg_send_pulse(fd, -1, PULSE_WAKE, 0).is_err() {
        // `syslogd` has gone, or gone and come back. The message is in the
        // ring, which a new one adopts: only a new registration is due,
        // not a second send.
        if !register(fd) {
            // Not back yet: the next message tries again.
            ring.rearm();
        }
    }
    Some(len as isize)
}

/// `close()` of a descriptor, which may be the socket.
pub fn closing(fd: c_int) {
    if fd >= 0 && FD.compare_exchange(fd, -1, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
        let map = MAP.swap(0, Ordering::AcqRel);
        __sys_munmap(map as *mut c_void, SIZE.load(Ordering::Acquire));
    }
}
