//! Error numbers between QNX's and FreeBSD's. The numbers themselves are
//! `proto`'s: FreeBSD's generated from `<sys/errno.h>`, QNX's from its
//! `<errno.h>`. This is only which is which.

use proto::{errno as b, qnx_errno as q};

/// (QNX, FreeBSD): the errors both have. Where several QNX errors are one
/// of FreeBSD's, the first listed is the one going to QNX.
const ERRNOS: &[(i32, i32)] = &[
    (q::EBADFSYS, b::EINTEGRITY),
    (q::EPERM, b::EPERM),
    (q::ENOENT, b::ENOENT),
    (q::ESRCH, b::ESRCH),
    (q::EINTR, b::EINTR),
    (q::EIO, b::EIO),
    (q::ENXIO, b::ENXIO),
    (q::E2BIG, b::E2BIG),
    (q::ENOEXEC, b::ENOEXEC),
    (q::EBADF, b::EBADF),
    (q::ECHILD, b::ECHILD),
    (q::EAGAIN, b::EAGAIN),
    (q::ENOMEM, b::ENOMEM),
    (q::EACCES, b::EACCES),
    (q::EFAULT, b::EFAULT),
    (q::ENOTBLK, b::ENOTBLK),
    (q::EBUSY, b::EBUSY),
    (q::EEXIST, b::EEXIST),
    (q::EXDEV, b::EXDEV),
    (q::ENODEV, b::ENODEV),
    (q::ENOTDIR, b::ENOTDIR),
    (q::EISDIR, b::EISDIR),
    (q::EINVAL, b::EINVAL),
    (q::ENFILE, b::ENFILE),
    (q::EMFILE, b::EMFILE),
    (q::ENOTTY, b::ENOTTY),
    (q::ETXTBSY, b::ETXTBSY),
    (q::EFBIG, b::EFBIG),
    (q::ENOSPC, b::ENOSPC),
    (q::ESPIPE, b::ESPIPE),
    (q::EROFS, b::EROFS),
    (q::EMLINK, b::EMLINK),
    (q::EPIPE, b::EPIPE),
    (q::EDOM, b::EDOM),
    (q::ERANGE, b::ERANGE),
    (q::ENOMSG, b::ENOMSG),
    (q::EIDRM, b::EIDRM),
    (q::EDEADLK, b::EDEADLK),
    (q::ENOLCK, b::ENOLCK),
    (q::ECANCELED, b::ECANCELED),
    (q::ENOTSUP, b::ENOTSUP),
    (q::EDQUOT, b::EDQUOT),
    (q::EDEADLOCK, b::EDEADLK),
    (q::EOWNERDEAD, b::EOWNERDEAD),
    (q::EREMOTE, b::EREMOTE),
    (q::ENOLINK, b::ENOLINK),
    (q::EPROTO, b::EPROTO),
    (q::EMULTIHOP, b::EMULTIHOP),
    (q::EBADMSG, b::EBADMSG),
    (q::ENAMETOOLONG, b::ENAMETOOLONG),
    (q::EOVERFLOW, b::EOVERFLOW),
    (q::EILSEQ, b::EILSEQ),
    (q::ENOSYS, b::ENOSYS),
    (q::ELOOP, b::ELOOP),
    (q::ERESTART, b::EINTR),
    (q::ENOTEMPTY, b::ENOTEMPTY),
    (q::EUSERS, b::EUSERS),
    (q::ENOTRECOVERABLE, b::ENOTRECOVERABLE),
    (q::EOPNOTSUPP, b::EOPNOTSUPP),
    (q::ESTALE, b::ESTALE),
    (q::EINPROGRESS, b::EINPROGRESS),
    (q::EALREADY, b::EALREADY),
    (q::ENOTSOCK, b::ENOTSOCK),
    (q::EDESTADDRREQ, b::EDESTADDRREQ),
    (q::EMSGSIZE, b::EMSGSIZE),
    (q::EPROTOTYPE, b::EPROTOTYPE),
    (q::ENOPROTOOPT, b::ENOPROTOOPT),
    (q::EPROTONOSUPPORT, b::EPROTONOSUPPORT),
    (q::ESOCKTNOSUPPORT, b::ESOCKTNOSUPPORT),
    (q::EPFNOSUPPORT, b::EPFNOSUPPORT),
    (q::EAFNOSUPPORT, b::EAFNOSUPPORT),
    (q::EADDRINUSE, b::EADDRINUSE),
    (q::EADDRNOTAVAIL, b::EADDRNOTAVAIL),
    (q::ENETDOWN, b::ENETDOWN),
    (q::ENETUNREACH, b::ENETUNREACH),
    (q::ENETRESET, b::ENETRESET),
    (q::ECONNABORTED, b::ECONNABORTED),
    (q::ECONNRESET, b::ECONNRESET),
    (q::ENOBUFS, b::ENOBUFS),
    (q::EISCONN, b::EISCONN),
    (q::ENOTCONN, b::ENOTCONN),
    (q::ESHUTDOWN, b::ESHUTDOWN),
    (q::ETOOMANYREFS, b::ETOOMANYREFS),
    (q::ETIMEDOUT, b::ETIMEDOUT),
    (q::ECONNREFUSED, b::ECONNREFUSED),
    (q::EHOSTDOWN, b::EHOSTDOWN),
    (q::EHOSTUNREACH, b::EHOSTUNREACH),
    (q::EBADRPC, b::EBADRPC),
    (q::ERPCMISMATCH, b::ERPCMISMATCH),
    (q::EPROGUNAVAIL, b::EPROGUNAVAIL),
    (q::EPROGMISMATCH, b::EPROGMISMATCH),
    (q::EPROCUNAVAIL, b::EPROCUNAVAIL),
];

/// FreeBSD's number for QNX's error `e`, if FreeBSD has it.
pub fn from_qnx(e: i32) -> Option<i32> {
    ERRNOS.iter().find(|p| p.0 == e).map(|p| p.1)
}

/// QNX's number for FreeBSD's error `e`, if QNX has it.
pub fn to_qnx(e: i32) -> Option<i32> {
    ERRNOS.iter().find(|p| p.1 == e).map(|p| p.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_numbers_that_differ() {
        assert_eq!(from_qnx(q::EAGAIN), Some(b::EAGAIN));
        assert_eq!(b::EAGAIN, 35);
        assert_eq!(to_qnx(b::ENOSYS), Some(q::ENOSYS));
        assert_eq!(to_qnx(b::ETIMEDOUT), Some(q::ETIMEDOUT));
    }

    #[test]
    fn each_goes_there_and_back() {
        for &(qnx, bsd) in ERRNOS {
            assert_eq!(from_qnx(to_qnx(bsd).unwrap()), Some(bsd), "{qnx}");
            assert_eq!(to_qnx(from_qnx(qnx).unwrap()).map(from_qnx), Some(Some(bsd)), "{qnx}");
        }
    }

    #[test]
    fn qnx_alone_has_these() {
        assert_eq!(from_qnx(q::ESRVRFAULT), None);
        assert_eq!(from_qnx(q::ENOREMOTE), None);
    }
}
