//! procnto as a client of other servers: open a name the path space
//! resolves to another server, stat, read and close it, as libc would
//! (`<sys/iomsg.h>`). For the boot script's opens and the loader's reads
//! of programs on other servers' filesystems. Not from the system
//! manager's thread when the server may ask procnto in turn (see
//! `worker`).

use alloc::vec::Vec;

use crate::arch::kercall::call;
use crate::ker::errno::{EIO, ENOENT};
use crate::ker::number::*;

use super::pathmgr;

/// `ioflag` of `open()`: `O_RDONLY` and `O_RDWR` (`_IO_FLAG_RD`,
/// `_IO_FLAG_WR`).
pub const READ: u32 = 1;
pub const READ_WRITE: u32 = 3;
pub const O_LARGEFILE: u32 = 0x1000_0000;

const IO_READ: u16 = 0x101;
const IO_STAT: u16 = 0x104;
const IO_CLOSE: u16 = 0x116;
/// `_STAT_FORM_T64_2008`: QNX's `struct stat`, with `st_size` at 8,
/// `st_mode` at 44; what a QNX binary answers.
const STAT_FORM: u32 = 3;
const STAT_SIZE: usize = 224;
/// Most bytes one `_IO_READ` asks for.
const READ_CHUNK: usize = 64 * 1024;

fn send(coid: i32, msg: &[u8], reply: &mut [u8]) -> Result<u64, u32> {
    call(
        __KER_MSG_SENDV,
        [
            coid as u64,
            msg.as_ptr() as u64,
            (-(msg.len() as i64)) as u64,
            reply.as_mut_ptr() as u64,
            (-(reply.len() as i64)) as u64,
            0,
        ],
    )
}

/// The `_IO_CONNECT` (`_IO_CONNECT_OPEN`) libc's `open()` sends for
/// `rest`, the path below the server's mount point `handle`, with `ioflag`
/// and `mode`.
pub fn connect_msg(handle: u32, ioflag: u32, mode: u32, rest: &str) -> Vec<u8> {
    const IO_CONNECT_OPEN: u16 = 2;
    const SH_DENYNO: u16 = 0x40;
    // As libc's open() sends them.
    const ACCESS: u16 = 7;
    const REPLY_MAX: u16 = 0x614;
    const ENTRY_MAX: u16 = 0x30;
    let path_len = rest.len() + 1;
    let mut m = alloc::vec![0u8; pathmgr::CONNECT_HDR + path_len];
    m[0..2].copy_from_slice(&pathmgr::IO_CONNECT.to_le_bytes());
    m[2..4].copy_from_slice(&IO_CONNECT_OPEN.to_le_bytes());
    m[8..10].copy_from_slice(&REPLY_MAX.to_le_bytes());
    m[10..12].copy_from_slice(&ENTRY_MAX.to_le_bytes());
    m[16..20].copy_from_slice(&handle.to_le_bytes());
    m[20..24].copy_from_slice(&ioflag.to_le_bytes());
    m[24..28].copy_from_slice(&mode.to_le_bytes());
    m[28..30].copy_from_slice(&SH_DENYNO.to_le_bytes());
    m[30..32].copy_from_slice(&ACCESS.to_le_bytes());
    m[34..36].copy_from_slice(&(path_len as u16).to_le_bytes());
    m[pathmgr::CONNECT_HDR..pathmgr::CONNECT_HDR + rest.len()].copy_from_slice(rest.as_bytes());
    m
}

/// Open the server's file `name` (absolute) as procnto, with `ioflag`:
/// its connection ID.
pub fn open(name: &str, ioflag: u32) -> Result<i32, u32> {
    let (pid, chid, handle, rest) = pathmgr::lookup(name).ok_or(ENOENT)?;
    let coid = call(__KER_CONNECT_ATTACH, [0, pid as u64, chid as u64, 0, 0, 0])? as i32;
    // Process 1 speaks FreeBSD's flags, but to a QNX binary.
    let qnx_server = crate::ker::process::lookup(pid)
        .is_some_and(|p| p.personality() == crate::ker::process::Personality::Qnx);
    let ioflag = if qnx_server { crate::personality::qnx::ioflag_out(ioflag) } else { ioflag };
    let m = connect_msg(handle, ioflag, 0, &rest);
    if let Err(e) = send(coid, &m, &mut []) {
        let _ = call(__KER_CONNECT_DETACH, [coid as u64, 0, 0, 0, 0, 0]);
        return Err(e);
    }
    Ok(coid)
}

/// The size and mode (`st_mode`) of an open file.
pub fn stat(coid: i32) -> Result<(u64, u32), u32> {
    let mut m = [0u8; 8];
    m[0..2].copy_from_slice(&IO_STAT.to_le_bytes());
    m[2..4].copy_from_slice(&8u16.to_le_bytes());
    m[4..8].copy_from_slice(&STAT_FORM.to_le_bytes());
    // Zeroed: a server that replies short leaves the mode 0, which no
    // caller takes for a regular file (the reply's length isn't known
    // here: MsgSend() returns the server's status; review PROCMGR-15).
    let mut s = [0u8; STAT_SIZE];
    let form = send(coid, &m, &mut s)?;
    // FreeBSD's `struct stat`, from a native server: `st_size` at 112,
    // `st_mode`, 16 bits, at 24.
    if form as u32 == crate::personality::native::STAT_FORM_FREEBSD {
        return Ok((
            u64::from_le_bytes(s[112..120].try_into().unwrap()),
            u32::from(u16::from_le_bytes(s[24..26].try_into().unwrap())),
        ));
    }
    Ok((
        u64::from_le_bytes(s[8..16].try_into().unwrap()),
        u32::from_le_bytes(s[44..48].try_into().unwrap()),
    ))
}

/// The first `size` bytes of an open file, from its current position
/// (fewer if it ends first). No more is asked for or kept: a server that
/// never says the end can't grow procnto's heap past `size`, reserved
/// up front (review PROCMGR-15).
pub fn read_all(coid: i32, size: u64) -> Result<Vec<u8>, u32> {
    let size = usize::try_from(size).map_err(|_| EIO)?;
    let mut data = Vec::new();
    data.try_reserve(size).map_err(|_| EIO)?;
    let mut buf = alloc::vec![0u8; READ_CHUNK];
    while data.len() < size {
        let want = (size - data.len()).min(READ_CHUNK);
        // struct _io_read { type, combine_len, nbytes, xtype, zero }.
        let mut m = [0u8; 16];
        m[0..2].copy_from_slice(&IO_READ.to_le_bytes());
        m[2..4].copy_from_slice(&16u16.to_le_bytes());
        m[4..8].copy_from_slice(&(want as u32).to_le_bytes());
        let n = send(coid, &m, &mut buf[..want])? as usize;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n.min(want)]);
    }
    Ok(data)
}

/// Close an open file and procnto's connection.
pub fn close(coid: i32) {
    let mut m = [0u8; 4];
    m[0..2].copy_from_slice(&IO_CLOSE.to_le_bytes());
    m[2..4].copy_from_slice(&4u16.to_le_bytes());
    let _ = send(coid, &m, &mut []);
    let _ = call(__KER_CONNECT_DETACH, [coid as u64, 0, 0, 0, 0, 0]);
}
