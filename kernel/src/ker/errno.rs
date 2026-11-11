//! The kernel's error numbers: FreeBSD's (`<sys/errno.h>`), the system's
//! native ones, as `proto` has them from that header, unsigned. `qnxabi`
//! converts them for a QNX process; those only QNX has are marked here.

use proto::errno as b;

/// The mark on an error only QNX has: its QNX number under it.
pub const QNX_ONLY: u32 = 0x1_0000;

pub const EPERM: u32 = b::EPERM as u32;
pub const ENOENT: u32 = b::ENOENT as u32;
pub const ESRCH: u32 = b::ESRCH as u32;
pub const EINTR: u32 = b::EINTR as u32;
pub const EIO: u32 = b::EIO as u32;
pub const ENXIO: u32 = b::ENXIO as u32;
pub const E2BIG: u32 = b::E2BIG as u32;
pub const ENOEXEC: u32 = b::ENOEXEC as u32;
/// Can't access shared library (an executable's interpreter).
pub const ELIBACC: u32 = QNX_ONLY | 83;
pub const EBADF: u32 = b::EBADF as u32;
pub const ECHILD: u32 = b::ECHILD as u32;
pub const EAGAIN: u32 = b::EAGAIN as u32;
pub const ENOMEM: u32 = b::ENOMEM as u32;
pub const EACCES: u32 = b::EACCES as u32;
pub const EFAULT: u32 = b::EFAULT as u32;
pub const EBUSY: u32 = b::EBUSY as u32;
pub const EEXIST: u32 = b::EEXIST as u32;
pub const ENODEV: u32 = b::ENODEV as u32;
pub const ENOTDIR: u32 = b::ENOTDIR as u32;
pub const EISDIR: u32 = b::EISDIR as u32;
pub const EINVAL: u32 = b::EINVAL as u32;
/// Too many open files in the process (`RLIMIT_NOFILE`).
pub const EMFILE: u32 = b::EMFILE as u32;
pub const ENOTTY: u32 = b::ENOTTY as u32;
pub const EFBIG: u32 = b::EFBIG as u32;
pub const ENOSPC: u32 = b::ENOSPC as u32;
pub const EROFS: u32 = b::EROFS as u32;
pub const ERANGE: u32 = b::ERANGE as u32;
pub const EDEADLK: u32 = b::EDEADLK as u32;
pub const ENOTSUP: u32 = b::ENOTSUP as u32;
pub const EOWNERDEAD: u32 = b::EOWNERDEAD as u32;
pub const EBADMSG: u32 = b::EBADMSG as u32;
pub const ESPIPE: u32 = b::ESPIPE as u32;
pub const ENAMETOOLONG: u32 = b::ENAMETOOLONG as u32;
pub const EOVERFLOW: u32 = b::EOVERFLOW as u32;
pub const ENOSYS: u32 = b::ENOSYS as u32;
pub const ENOTEMPTY: u32 = b::ENOTEMPTY as u32;
pub const ENOTRECOVERABLE: u32 = b::ENOTRECOVERABLE as u32;
pub const ETIMEDOUT: u32 = b::ETIMEDOUT as u32;
pub const ENOREMOTE: u32 = QNX_ONLY | 300;
pub const ESRVRFAULT: u32 = QNX_ONLY | 312;
pub const ENOBUFS: u32 = b::ENOBUFS as u32;
pub const EOPNOTSUPP: u32 = b::EOPNOTSUPP as u32;
pub const EPIPE: u32 = b::EPIPE as u32;
pub const EMSGSIZE: u32 = b::EMSGSIZE as u32;
