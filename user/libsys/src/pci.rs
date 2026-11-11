//! `pci(4)`'s requests on `/dev/pci`, served by pcid: as they are, but for
//! `PCIOCGETCONF`, whose `struct pci_conf_io` points to the caller's
//! pattern and match buffers. Those go in the same message after it,
//! `[pci_conf_io][patterns][room for matches]`, and the reply comes back
//! the same way, scattered into the caller's buffers (pcid's
//! `kern/glue.c` unpacks them).

use core::ffi::c_int;

use proto::io::{self as m, Devctl, DevctlReply};

use crate::kercall::{Iov, msg_sendv};
use crate::tty::{DEVDIR_FROM, DEVDIR_TO, dcmd};

/// `struct pci_conf_io` (`<sys/pciio.h>`): the offsets of its fields.
const PAT_BUF_LEN: usize = 0;
const PATTERNS: usize = 8;
const MATCH_BUF_LEN: usize = 16;
const MATCHES: usize = 24;
/// The most pcid takes in one request.
const ROOM: usize = 64 * 1024;

/// `ioctl(fd, PCIOCGETCONF, data)`; `len` is the struct's size.
///
/// # Safety
///
/// `data` is a `struct pci_conf_io` whose buffers are as it says.
pub unsafe fn getconf(fd: c_int, num: u32, data: *mut u8, len: usize) -> Result<c_int, c_int> {
    // SAFETY: the caller's guarantee, throughout.
    unsafe {
        let u32_at = |off: usize| data.add(off).cast::<u32>().read_unaligned() as usize;
        let ptr_at = |off: usize| data.add(off).cast::<*mut u8>().read_unaligned();
        let (pat_len, match_len) = (u32_at(PAT_BUF_LEN), u32_at(MATCH_BUF_LEN));
        let total = len + pat_len + match_len;
        if total > ROOM {
            return Err(proto::errno::ENOMEM);
        }
        let (patterns, matches) = (ptr_at(PATTERNS), ptr_at(MATCHES));
        let msg = Devctl {
            r#type: m::IO_DEVCTL,
            combine_len: size_of::<Devctl>() as u16,
            dcmd: dcmd(DEVDIR_TO | DEVDIR_FROM, b'p' as u32, num, len) as i32,
            nbytes: total as u32,
            zero: 0,
        };
        let mut answer = DevctlReply::default();
        // The match buffer's room is sent too, from the matches themselves:
        // its content doesn't matter, its length does.
        let sendv = [Iov::of(&msg), Iov::raw(data.cast(), len), Iov::raw(patterns.cast(), pat_len), Iov::raw(matches.cast(), match_len)];
        let recv = [
            Iov::raw((&raw mut answer).cast(), size_of::<DevctlReply>()),
            Iov::raw(data.cast(), len),
            Iov::raw(patterns.cast(), pat_len),
            Iov::raw(matches.cast(), match_len),
        ];
        msg_sendv(fd, &sendv, &recv)?;
        Ok(answer.ret_val)
    }
}
