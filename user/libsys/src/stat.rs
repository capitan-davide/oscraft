//! `fstat()`: an `_IO_STAT` message. Process 1 answers a FreeBSD process
//! with FreeBSD's `struct stat`, as it is; QNX's servers with QNX's, which
//! is laid out again here for as long as those servers are around. The
//! reply's status says which.

use core::ffi::c_int;

use proto::io::{self as m};
use proto::pod::Pod;
use proto::stat::{self, QnxStat};

use crate::errno::fail;
use crate::kercall::{Iov, msg_sendv};

pub use proto::stat::{Stat, FORM_FREEBSD};

/// Room for either kind of reply.
pub type Reply = [u8; size_of::<Stat>()];

/// FreeBSD's stat from a reply in form `form`.
pub fn decode(form: u32, reply: &Reply) -> Stat {
    if form == FORM_FREEBSD {
        // SAFETY: a reply of that form is such a struct, of plain data.
        return unsafe { reply.as_ptr().cast::<Stat>().read_unaligned() };
    }
    let mut st = QnxStat::read_from(reply).unwrap_or_default();
    if form != stat::FORM_PREFERRED && form != stat::FORM_UNSET {
        // Older forms: 32-bit times (oscraft-platform's `convert_stat`).
        let secs = st.old_times.map(i64::from);
        [st.st_mtim, st.st_atim, st.st_ctim] = secs.map(|s| [s, 0]);
    }
    qnxabi::stat::from_qnx(&st)
}

/// `fstat(fd, sb)`.
///
/// # Safety
///
/// `sb` points to a FreeBSD `struct stat`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_fstat(fd: c_int, sb: *mut Stat) -> c_int {
    let msg = m::Stat {
        r#type: m::IO_STAT,
        combine_len: size_of::<m::Stat>() as u16,
        format: stat::FORM_PREFERRED,
    };
    let mut reply: Reply = [0; size_of::<Stat>()];
    let send = [Iov::of(&msg)];
    let recv = [Iov::raw(reply.as_mut_ptr().cast(), reply.len())];
    // SAFETY: both are on this stack, valid for their sizes.
    match unsafe { msg_sendv(fd, &send, &recv) } {
        Ok(form) => {
            // SAFETY: the caller's guarantee.
            unsafe { sb.write(decode(form as u32, &reply)) };
            0
        }
        Err(e) => fail(e) as c_int,
    }
}
