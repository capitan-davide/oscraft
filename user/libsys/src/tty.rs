//! `ioctl()`: FreeBSD's requests on QNX's servers. The terminal requests
//! are `_IO_DEVCTL` commands (`<sys/dcmd_chr.h>`), which QNX encodes as
//! BSD does (group, number, direction, size), so a request becomes the
//! command with QNX's payload size and payload: `struct termios` laid
//! out again (`<termios.h>` of each: flags, control characters and
//! speeds differ), `struct winsize` as it is. A server of OsCraft's own
//! will take FreeBSD's layouts directly; this is for the SDP's drivers.

use core::ffi::{c_int, c_void};

use proto::errno::{EINVAL, ENOTTY};
use proto::io::{self as m, Devctl, DevctlReply};

use proto::termios::{Termios, QnxTermios};
use qnxabi::termios::{from_qnx, to_qnx};

use crate::errno::{fail, not_implemented};
use crate::kercall::{self, Iov, msg_sendv};

// FreeBSD's `_IOC` encoding (`<sys/ioccom.h>`); QNX's `__DIO*` is the same
// but for the void bit and the size.
const IOC_VOID: u64 = 0x2000_0000;
const IOC_OUT: u64 = 0x4000_0000;
const IOC_IN: u64 = 0x8000_0000;
const IOCPARM_MASK: u64 = 0x1fff;
pub(crate) const DEVDIR_TO: u32 = 0x8000_0000;
pub(crate) const DEVDIR_FROM: u32 = 0x4000_0000;
pub(crate) const DEVDIR_NONE: u32 = 0;
const DCMD_CHR: u32 = 3;
const DCMD_ALL_GETFLAGS: u32 = 0x4004_0101;
const DCMD_ALL_SETFLAGS: u32 = 0x8004_0102;

/// A device command: class or group, number, direction and payload size.
pub(crate) fn dcmd(dir: u32, group: u32, num: u32, size: usize) -> u32 {
    dir | ((size as u32) << 16) | (group << 8) | num
}

/// One `_IO_DEVCTL` on `fd` with `data` of `size` bytes in, out or both
/// as `dcmd` says: the server's answer.
///
/// # Safety
///
/// `data` is valid for `size` bytes, writable if the command replies.
pub(crate) unsafe fn devctl(fd: c_int, dcmd: u32, data: *mut c_void, size: usize) -> Result<c_int, c_int> {
    let msg = Devctl {
        r#type: m::IO_DEVCTL,
        combine_len: size_of::<Devctl>() as u16,
        dcmd: dcmd as i32,
        nbytes: size as u32,
        zero: 0,
    };
    let mut answer = DevctlReply::default();
    let sendv = [Iov::of(&msg), Iov::raw(data, if dcmd & DEVDIR_TO != 0 { size } else { 0 })];
    let recv = [
        Iov::raw((&raw mut answer).cast(), size_of::<DevctlReply>()),
        Iov::raw(data, if dcmd & DEVDIR_FROM != 0 { size } else { 0 }),
    ];
    // SAFETY: the message and the reply are on this stack; the data the
    // caller's.
    unsafe { msg_sendv(fd, &sendv, &recv) }?;
    Ok(answer.ret_val)
}

/// `ioctl(fd, request, data)`.
///
/// # Safety
///
/// `data` is what `request` says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_ioctl(fd: c_int, request: u64, data: *mut c_void) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { ioctl(fd, request, data) }.unwrap_or_else(|e| fail(e) as c_int)
}

/// # Safety
///
/// As `__sys_ioctl`.
unsafe fn ioctl(fd: c_int, request: u64, data: *mut c_void) -> Result<c_int, c_int> {
    let (group, num) = (((request >> 8) & 0xff) as u32, (request & 0xff) as u32);
    let len = ((request >> 16) & IOCPARM_MASK) as usize;
    let dir = (request & (IOC_IN | IOC_OUT)) as u32;
    // SAFETY: throughout, `data` is as the request says.
    unsafe {
        match (group as u8, num) {
            // TIOCGETA, TIOCSETA and the like: FreeBSD's struct, as it is;
            // QNX's drivers are laid out below.
            (b't', 19) if crate::fd::native(fd) => devctl(fd, dcmd(DEVDIR_FROM, b't' as u32, 19, len), data, len).map(|_| 0),
            (b't', 20..=22) if crate::fd::native(fd) => devctl(fd, dcmd(DEVDIR_TO, b't' as u32, num, len), data, len).map(|_| 0),
            // TIOCGETA.
            (b't', 19) => {
                let mut q = QnxTermios::default();
                devctl(fd, dcmd(DEVDIR_FROM, b't' as u32, 19, size_of::<QnxTermios>()), (&raw mut q).cast(), size_of::<QnxTermios>())
                    .map(|_| {
                        data.cast::<Termios>().write_unaligned(from_qnx(&q));
                        0
                    })
            }
            // TIOCSETA, TIOCSETAW, TIOCSETAF: the current settings first,
            // for the characters FreeBSD has no slot for.
            (b't', 20..=22) => {
                let mut old = QnxTermios::default();
                devctl(fd, dcmd(DEVDIR_FROM, b't' as u32, 19, size_of::<QnxTermios>()), (&raw mut old).cast(), size_of::<QnxTermios>())?;
                let mut q = to_qnx(&data.cast::<Termios>().read_unaligned(), &old);
                devctl(fd, dcmd(DEVDIR_TO, b't' as u32, num, size_of::<QnxTermios>()), (&raw mut q).cast(), size_of::<QnxTermios>()).map(|_| 0)
            }
            // TIOCGWINSZ, TIOCSWINSZ, TIOCGPGRP, TIOCSPGRP, TIOCDRAIN,
            // TIOCOUTQ: the same command and payload on both.
            (b't', 103 | 104 | 118 | 119 | 94 | 115) => devctl(fd, dcmd(dir, b't' as u32, num, len), data, len).map(|_| 0),
            // TIOCFLUSH: FreeBSD's FREAD and FWRITE bits become QNX's
            // TCIFLUSH 0, TCOFLUSH 1, TCIOFLUSH 2.
            (b't', 16) => {
                let which = if data.is_null() { 3 } else { data.cast::<c_int>().read_unaligned() & 3 };
                let mut q: c_int = match which {
                    1 => 0,
                    2 => 1,
                    _ => 2,
                };
                devctl(fd, dcmd(DEVDIR_TO, b't' as u32, 16, 4), (&raw mut q).cast(), 4).map(|_| 0)
            }
            // TIOCGSID: QNX's DCMD_CHR_TCGETSID.
            (b't', 99) => devctl(fd, dcmd(DEVDIR_FROM, DCMD_CHR, 7, 4), data, 4).map(|_| 0),
            // TIOCSCTTY: DCMD_CHR_TCSETSID with the caller's own ID.
            (b't', 97) => {
                let mut pid = crate::proc::pid();
                devctl(fd, dcmd(DEVDIR_TO, DCMD_CHR, 8, 4), (&raw mut pid).cast(), 4).map(|_| 0)
            }
            // TIOCPTMASTER: the pty manager's answer.
            (b't', 28) => devctl(fd, dcmd(0, b't' as u32, 28, 0), core::ptr::null_mut(), 0).map(|_| 0),
            // FIODGNAME (`struct fiodgname_arg` { len, buf }): the device's
            // name under /dev.
            (b'f', 120) => {
                let want = data.cast::<c_int>().read_unaligned() as usize;
                let buf = data.cast::<*mut u8>().add(1).read_unaligned();
                let mut name = [0u8; 32];
                devctl(fd, dcmd(DEVDIR_FROM, b'f' as u32, 120, 32), name.as_mut_ptr().cast(), 32)?;
                let n = name.iter().position(|&b| b == 0).unwrap_or(32);
                if n + 1 > want {
                    return Err(EINVAL);
                }
                core::ptr::copy_nonoverlapping(name.as_ptr(), buf, n + 1);
                Ok(0)
            }
            // FIONREAD: DCMD_CHR_ISCHARS, the same encoding.
            (b'f', 127) => devctl(fd, dcmd(DEVDIR_FROM, b'f' as u32, 127, 4), data, 4).map(|_| 0),
            // FIONBIO: O_NONBLOCK in the server's status flags.
            (b'f', 126) => {
                let on = data.cast::<c_int>().read_unaligned() != 0;
                let mut flags: c_int = 0;
                devctl(fd, DCMD_ALL_GETFLAGS, (&raw mut flags).cast(), 4)?;
                let nonblock = proto::fcntl::O_NONBLOCK as c_int;
                flags = if on { flags | nonblock } else { flags & !nonblock };
                devctl(fd, DCMD_ALL_SETFLAGS, (&raw mut flags).cast(), 4).map(|_| 0)
            }
            // FIOCLEX, FIONCLEX: the connection's flag.
            (b'f', 1) => kercall::connect_flags(fd, 1, 1).map(|_| 0),
            (b'f', 2) => kercall::connect_flags(fd, 1, 0).map(|_| 0),
            // The network's: the interfaces ('i'), bpf ('B') and pf ('D').
            (b'i' | b'B' | b'D', _) => crate::socket::netioctl::ioctl(fd, request as u32, data),
            // pci(4): to pcid as they are, PCIOCGETCONF packed.
            (b'p', 10) => crate::pci::getconf(fd, num, data.cast(), len),
            (b'p', _) => devctl(fd, dcmd(dir, group, num, len), data, len).map(|_| 0),
            // usbd's own (bsdkern_usb.h): to usbd as they are.
            (b'U', 200..=203) => devctl(fd, dcmd(dir, group, num, len), data, len).map(|_| 0),
            // Disks' DIOCG*: to their server as they are.
            (b'd', _) => devctl(fd, dcmd(DEVDIR_FROM, group, num, len), data, len).map(|_| 0),
            // FD_GTYPE: no floppy here; newfs_msdos goes on.
            (b'F', _) => Err(ENOTTY),
            _ => {
                let _ = IOC_VOID;
                let _ = EINVAL;
                not_implemented("ioctl request");
                Err(ENOTTY)
            }
        }
    }
}

/// `posix_openpt(flags)`: the first free controller, as `openpty()` looks
/// for one on QNX: `/dev/pty[p-zP-T][0-9a-f]`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_posix_openpt(flags: c_int) -> c_int {
    const LETTERS: &[u8] = b"pqrstuvwxyzPQRST";
    const HEX: &[u8] = b"0123456789abcdef";
    let mut path = *b"/dev/ptyp0\0";
    for index in 0..256 {
        path[8] = LETTERS[index / 16];
        path[9] = HEX[index % 16];
        // SAFETY: a string.
        let fd = unsafe { crate::fs::__sys_open(path.as_ptr().cast(), flags, 0) };
        if fd >= 0 {
            return fd;
        }
        if crate::errno::last() != proto::errno::EBUSY {
            break;
        }
    }
    fail(proto::errno::EAGAIN) as c_int
}
