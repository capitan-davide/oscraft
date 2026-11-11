//! I/O.
//!
//! In QNX, a file descriptor is a connection to the resource manager that
//! owns the file: the connection ID is the descriptor. An I/O function is
//! a message on that connection, defined by `<sys/iomsg.h>`, and the
//! server's reply status is the function's result.

use core::ffi::{c_int, c_uint, c_void};

use proto::errno::{EBADF, EBADMSG, EINTR, EINVAL};
use proto::io::{self as m, Header, ReadWrite};
use proto::pod::Pod;

use crate::abi::Iov;

/// For a mapping of the message's parts (`<sys/mman.h>`).
const PROT_READ: core::ffi::c_int = 0x100;
const PROT_WRITE: core::ffi::c_int = 0x200;
const MAP_PRIVATE: core::ffi::c_int = 0x2;
const MAP_ANON: core::ffi::c_int = 0x8_0000;
use crate::neutrino::{msg_sendv, send_nc as send_pod_nc};

/// Writes `nbytes` bytes from `buf` to file descriptor `fd`: an
/// `_IO_WRITE` message, its header and the data as two parts, so that the
/// data is not copied. Returns the number of bytes written, the server's
/// reply status. The header's count is 32 bits: a larger write is cut to
/// what it can say, and returns that many, as a short write may.
///
/// # Safety
///
/// `buf` is valid for reads of `nbytes` bytes.
pub unsafe fn write(fd: c_int, buf: *const c_void, nbytes: usize) -> Result<usize, c_int> {
    let nbytes = nbytes.min(u32::MAX as usize);
    let header = ReadWrite {
        r#type: m::IO_WRITE,
        combine_len: size_of::<ReadWrite>() as u16,
        nbytes: nbytes as u32,
        xtype: m::IO_XTYPE_NONE,
        nbytes_hi: 0,
    };
    let message = [Iov::of(&header), Iov::raw(buf, nbytes)];
    // SAFETY: both parts are valid for their lengths: the header on this
    // stack, the data by the caller's guarantee.
    let written = unsafe { msg_sendv(fd, &message, &[]) }?;
    Ok(written as usize)
}

/// Reads up to `nbytes` bytes from `fd` into `buf`, at the file's offset
/// or, with `offset`, at that offset (`pread()`). The server replies with
/// the data, straight into `buf`, and its count as the status.
///
/// # Safety
///
/// `buf` is valid for writes of `nbytes` bytes.
pub unsafe fn read(
    fd: c_int,
    buf: *mut c_void,
    nbytes: usize,
    offset: Option<i64>,
) -> Result<usize, c_int> {
    let into = [Iov::raw(buf.cast_const(), nbytes)];
    // SAFETY: the caller's guarantee.
    unsafe { transfer(fd, m::IO_READ, m::IO_READ64, nbytes, offset, &[], &into) }
}

/// Moves `fd`'s offset; returns the new offset, which the server replies.
pub fn lseek(fd: c_int, offset: i64, whence: c_int) -> Result<i64, c_int> {
    let message = m::Lseek {
        r#type: m::IO_LSEEK,
        combine_len: size_of::<m::Lseek>() as u16,
        whence: whence as i16,
        flags: 0,
        offset: offset as u64,
    };
    let (_, result) = crate::neutrino::ask::<_, u64>(fd, &message)?;
    Ok(result as i64)
}

/// Closes `fd`: tells its server, with `_IO_CLOSE`, then detaches the
/// connection. If the server's answer is interrupted, the descriptor is
/// left open, so that the close can be tried again; any other failure
/// still detaches it. Returns the server's error, or the detach's.
pub fn close(fd: c_int) -> Result<(), c_int> {
    let message = Header {
        r#type: m::IO_CLOSE,
        combine_len: size_of::<Header>() as u16,
    };
    let send = [Iov::of(&message)];
    // SAFETY: the message is on this stack; there is no reply data.
    let sent = unsafe { msg_sendv(fd, &send, &[]) };
    if sent == Err(EINTR) {
        return Err(EINTR);
    }
    let detached = crate::neutrino::connect::ConnectDetach_r(fd);
    sent?;
    if detached != 0 { Err(detached) } else { Ok(()) }
}

/// `struct stat`, as QNX OS 8.0's `<sys/stat.h>` defines it: form 3 of
/// the forms servers reply in, 120 bytes.
pub const STAT_SIZE: usize = 0x78;
/// The stat forms (`_STAT_FORM_*`): what a server may reply in.
const STAT_FORM_UNSET: c_int = 0;
const STAT_FORM_32BIT: c_int = 1;
const STAT_FORM_64BIT: c_int = 2;
const STAT_FORM_PREFERRED: c_int = 3;

/// The `_IO_STAT` message, asking for the preferred form.
pub fn stat_message() -> m::Stat {
    m::Stat {
        r#type: m::IO_STAT,
        combine_len: size_of::<m::Stat>() as u16,
        format: STAT_FORM_PREFERRED as u32,
    }
}

/// Converts a `struct stat` a server replied in `form` to the preferred
/// form, in place: the first 0x48 bytes are the same in every form; the
/// times differ. Older servers reply 32-bit seconds (form 1), or 32-bit
/// seconds and nanoseconds (form 2); the preferred form has a 64-bit
/// `struct timespec` for each of the access, modification and change
/// times.
pub fn convert_stat(form: c_int, st: &mut [u8; STAT_SIZE]) -> Result<(), c_int> {
    use proto::stat::QnxStat;
    const OLD: usize = core::mem::offset_of!(QnxStat, old_times);
    const MTIM: usize = core::mem::offset_of!(QnxStat, st_mtim);
    let mut v = QnxStat::read_from(st).ok_or(EBADMSG)?;
    let times: [[i64; 2]; 3] = match form {
        STAT_FORM_PREFERRED => return Ok(()),
        STAT_FORM_UNSET | STAT_FORM_32BIT => {
            let old: [u32; 3] = Pod::read_from(&st[OLD..]).ok_or(EBADMSG)?;
            old.map(|s| [i64::from(s), 0])
        }
        STAT_FORM_64BIT => {
            // Seconds are unsigned, nanoseconds signed.
            let t: [u32; 6] = Pod::read_from(&st[MTIM..]).ok_or(EBADMSG)?;
            core::array::from_fn(|k| [i64::from(t[k * 2]), i64::from(t[k * 2 + 1] as i32)])
        }
        _ => return Err(EBADMSG),
    };
    [v.st_mtim, v.st_atim, v.st_ctim] = times;
    st.copy_from_slice(v.as_bytes());
    Ok(())
}

/// The device and inode of `fd`. Process 1 answers a FreeBSD process
/// with FreeBSD's `struct stat`, in form 0x4642; QNX's servers with QNX's.
pub fn dev_ino(fd: c_int) -> Result<(u32, u64), c_int> {
    const FORM_FREEBSD: i64 = 0x4642;
    let message = stat_message();
    let mut st = [0u8; 224];
    // Not a cancellation point, as in QNX's library.
    // SAFETY: both buffers are on this stack.
    let form = unsafe {
        crate::neutrino::msg_sendvnc(
            fd,
            (&raw const message).cast(),
            size_of::<m::Stat>(),
            st.as_mut_ptr().cast(),
            st.len(),
        )
    }? as i64;
    let u32_at = |at: usize| u32::from_le_bytes([st[at], st[at + 1], st[at + 2], st[at + 3]]);
    let u64_at = |at: usize| u64::from(u32_at(at)) | u64::from(u32_at(at + 4)) << 32;
    Ok(if form == FORM_FREEBSD { (u32_at(0), u64_at(8)) } else { (u32_at(0x10), u64_at(0)) })
}

/// The most parts `readv()` and `writev()` take, `UIO_MAXIOV`.
pub const IOV_MAX: usize = 1024;

/// The total length of `parts`, or `EINVAL` if it does not fit an
/// `ssize_t`.
fn total(parts: &[Iov]) -> Result<usize, c_int> {
    parts
        .iter()
        .try_fold(0usize, |sum, part| sum.checked_add(part.len))
        .filter(|&sum| sum <= isize::MAX as usize)
        .ok_or(EINVAL)
}

/// Writes the parts of `parts` to `fd`, in one message, at the file's
/// offset or at `offset`: `writev()`, `pwrite()`. The header is the
/// message's first part, and the caller's parts follow it as they are.
///
/// # Safety
///
/// Each part is valid for reads of its length.
pub unsafe fn writev(fd: c_int, parts: &[Iov], offset: Option<i64>) -> Result<usize, c_int> {
    if parts.is_empty() || parts.len() > IOV_MAX {
        return Err(EINVAL);
    }
    let nbytes = total(parts)?;
    // SAFETY: the caller's guarantee.
    unsafe { transfer(fd, m::IO_WRITE, m::IO_WRITE64, nbytes, offset, parts, &[]) }
}

/// Reads into the parts of `parts` from `fd`, at the file's offset or at
/// `offset`: `readv()`. The server's reply is scattered into them.
///
/// # Safety
///
/// Each part is valid for writes of its length.
pub unsafe fn readv(fd: c_int, parts: &[Iov], offset: Option<i64>) -> Result<usize, c_int> {
    if parts.is_empty() || parts.len() > IOV_MAX {
        return Err(EINVAL);
    }
    let nbytes = total(parts)?;
    // SAFETY: the caller's guarantee.
    unsafe { transfer(fd, m::IO_READ, m::IO_READ64, nbytes, offset, &[], parts) }
}

/// One read or write message of `nbytes`: the header, with the offset
/// after it if there is one, then `data` to send or `into` to receive.
///
/// # Safety
///
/// The parts are valid for their lengths.
unsafe fn transfer(
    fd: c_int,
    small: u16,
    large: u16,
    nbytes: usize,
    offset: Option<i64>,
    data: &[Iov],
    into: &[Iov],
) -> Result<usize, c_int> {
    /// The header, and the offset of `_IO_XTYPE_OFFSET` after it.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WithOffset {
        header: ReadWrite,
        offset: i64,
    }
    let header = WithOffset {
        header: ReadWrite {
            r#type: if nbytes > u32::MAX as usize {
                large
            } else {
                small
            },
            combine_len: if offset.is_some() {
                size_of::<WithOffset>() as u16
            } else {
                0
            },
            nbytes: nbytes as u32,
            xtype: if offset.is_some() {
                m::IO_XTYPE_OFFSET
            } else {
                m::IO_XTYPE_NONE
            },
            nbytes_hi: (nbytes as u64 >> 32) as u32,
        },
        offset: offset.unwrap_or(0),
    };
    let len = if offset.is_some() {
        size_of::<WithOffset>()
    } else {
        size_of::<ReadWrite>()
    };
    let head = Iov {
        base: (&raw const header).cast(),
        len,
    };
    // The header, then the data's parts: on the stack for up to SMALL of
    // them, else in a mapping of its own for the call. A thread's stack
    // may be small (a block driver's are), so it never takes IOV_MAX's.
    const SMALL: usize = 16;
    let count = if data.len() < SMALL {
        let mut send = [const {
            Iov {
                base: core::ptr::null(),
                len: 0,
            }
        }; SMALL];
        send[0] = head;
        for (slot, part) in send[1..].iter_mut().zip(data) {
            *slot = Iov {
                base: part.base,
                len: part.len,
            };
        }
        // SAFETY: the caller's guarantee; the header is on this stack.
        unsafe { msg_sendv(fd, &send[..=data.len()], into) }?
    } else {
        let bytes = (data.len() + 1) * size_of::<Iov>();
        // SAFETY: a new private mapping, for this call's parts.
        let at = unsafe {
            crate::mem::map(
                core::ptr::null_mut(),
                bytes,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANON,
                -1,
                0,
            )
        }?
        .cast::<Iov>();
        // SAFETY: the mapping has room for the header and every part.
        let send = unsafe { core::slice::from_raw_parts_mut(at, data.len() + 1) };
        send[0] = head;
        for (slot, part) in send[1..].iter_mut().zip(data) {
            *slot = Iov {
                base: part.base,
                len: part.len,
            };
        }
        // SAFETY: the caller's guarantee; the header is on this stack.
        let sent = unsafe { msg_sendv(fd, send, into) };
        // SAFETY: the mapping made above, no longer used.
        let _ = unsafe { crate::mem::unmap(at.cast(), bytes, 0) };
        sent?
    };
    if count < 0 || count as usize > nbytes {
        return Err(EBADMSG);
    }
    Ok(count as usize)
}

/// `DEVDIR_TO` and `DEVDIR_FROM` (`<devctl.h>`): the command sends data
/// to the server, gets data from it, or both.
const DEVDIR_TO: c_int = 0x8000_0000_u32 as c_int;
const DEVDIR_FROM: c_int = 0x4000_0000;

/// Sends device command `dcmd` to `fd`'s server: `nbytes` of `data` go
/// to the server if the command says so, and come back into it if it
/// says so. Returns the server's value for the command
/// (`_io_devctl_reply`'s `ret_val`). With `cancellable` false, the
/// message is not a cancellation point.
///
/// # Safety
///
/// `data` is valid for `nbytes` bytes, in the directions `dcmd` says.
pub unsafe fn devctl(
    fd: c_int,
    dcmd: c_int,
    data: *mut c_void,
    nbytes: usize,
    cancellable: bool,
) -> Result<c_int, c_int> {
    let msg = m::Devctl {
        r#type: m::IO_DEVCTL,
        combine_len: size_of::<m::Devctl>() as u16,
        dcmd,
        nbytes: nbytes as u32,
        zero: 0,
    };
    let mut answer = m::DevctlReply::default();
    let send = [
        Iov::of(&msg),
        Iov {
            base: data,
            len: if dcmd & DEVDIR_TO != 0 { nbytes } else { 0 },
        },
    ];
    let reply = [
        Iov::of_mut(&mut answer),
        Iov {
            base: data,
            len: if dcmd & DEVDIR_FROM != 0 { nbytes } else { 0 },
        },
    ];
    let status = if cancellable {
        crate::neutrino::msg::MsgSendv_r(fd, send.as_ptr().cast(), 2, reply.as_ptr().cast(), 2)
    } else {
        crate::neutrino::msg::MsgSendvnc_r(fd, send.as_ptr().cast(), 2, reply.as_ptr().cast(), 2)
    };
    if status < 0 {
        return Err(-status as c_int);
    }
    Ok(answer.ret_val)
}

/// Sends an `_IO_ACL` message on `fd`: `subtype` with the `len` bytes of
/// an ACL at `acl`; for `_IO_ACL_GET`, the header asks for at most `len`
/// bytes, which the reply's ACL replaces. The server's status.
///
/// # Safety
///
/// `acl` is valid for `len` bytes, and for a get writable.
pub unsafe fn acl(fd: c_int, subtype: u32, acl: *mut c_void, len: usize) -> Result<c_int, c_int> {
    let msg = m::Acl {
        r#type: m::IO_ACL,
        combine_len: size_of::<m::Acl>() as u16,
        subtype,
        zero: [0; 2],
    };
    let status = if subtype == m::IO_ACL_GET {
        let ask = m::AclHeader {
            size: len as u32,
            ..Default::default()
        };
        let mut answer = m::AclReply::default();
        let send = [Iov::of(&msg), Iov::of(&ask)];
        let reply = [Iov::of_mut(&mut answer), Iov { base: acl, len }];
        crate::neutrino::msg::MsgSendvnc_r(fd, send.as_ptr().cast(), 2, reply.as_ptr().cast(), 2)
    } else {
        let send = [Iov::of(&msg), Iov { base: acl, len }];
        crate::neutrino::msg::MsgSendvnc_r(fd, send.as_ptr().cast(), 2, core::ptr::null(), 0)
    };
    if status < 0 {
        return Err(-status as c_int);
    }
    Ok(status as c_int)
}

/// `struct _server_info` (`<sys/neutrino.h>`): who serves a connection.
#[repr(C)]
#[derive(Default)]
struct ServerInfo {
    nd: u32,
    srcnd: u32,
    pid: i32,
    tid: i32,
    chid: i32,
    scoid: i32,
    coid: i32,
    priority: i16,
    flags: u16,
    rest: [u32; 16],
}

/// `_NTO_COF_CLOEXEC`, `_NTO_COF_NOSHARE`, `_NTO_COF_DUP2`.
const COF_CLOEXEC: c_int = 0x1;
const COF_NOSHARE: u16 = 0x40;
const COF_DUP2: c_int = 0x8000;

/// Duplicates `fd`: a new connection to the same server, as the lowest
/// descriptor from `min`, or as exactly `target` (closing what it was).
/// The server learns of it with `_IO_DUP`, naming the old connection, so
/// that both share the open file. The new descriptor is attached
/// close-on-exec, and keeps that only if `cloexec`.
pub fn dup(fd: c_int, min: c_int, target: Option<c_int>, cloexec: bool) -> Result<c_int, c_int> {
    use crate::neutrino::connect::{
        ConnectAttach_r, ConnectDetach_r, ConnectFlags_r, ConnectServerInfo_r,
    };
    let mut info = ServerInfo::default();
    if fd < 0 || fd & SIDE_CHANNEL != 0 || ConnectServerInfo_r(0, fd, (&raw mut info).cast()) != fd
    {
        return Err(EBADF);
    }
    let flags = c_int::from(info.flags & COF_NOSHARE) | COF_CLOEXEC;
    let new = match target {
        Some(target) if target == fd => return Ok(fd),
        Some(target) => {
            // Close what `target` was, as close() would, but keep the
            // connection's slot: the kernel replaces it.
            let close = Header {
                r#type: m::IO_CLOSE,
                combine_len: size_of::<Header>() as u16,
            };
            let closed = send_pod_nc(target, &close);
            if closed == Err(EINTR) {
                return Err(EINTR);
            }
            ConnectAttach_r(0, 0, fd, target as c_uint, flags | COF_DUP2)
        }
        None => ConnectAttach_r(info.nd, info.pid, info.chid, min as c_uint, flags),
    };
    if new < 0 {
        return Err(EBADF);
    }
    let msg = m::Dup {
        r#type: m::IO_DUP,
        combine_len: size_of::<m::Dup>() as u16,
        info: client_of(&info, fd),
        handle: 0,
    };
    let sent = send_pod_nc(new, &msg);
    if let Err(error) = sent {
        ConnectDetach_r(new);
        return Err(error);
    }
    if !cloexec {
        ConnectFlags_r(0, new, COF_CLOEXEC as c_uint, 0);
    }
    Ok(new)
}

/// The client side of `struct _msg_info_hdr` that `_IO_DUP` and
/// `_IO_OPENFD` send: this process, and its connection `coid` to the
/// server `info` describes.
fn client_of(info: &ServerInfo, coid: c_int) -> proto::info::MsgInfoHdr {
    proto::info::MsgInfoHdr {
        pid: crate::tls::pid(),
        chid: info.chid,
        scoid: info.scoid,
        coid,
        ..Default::default()
    }
}

/// `_NTO_SIDE_CHANNEL`: connection IDs from here are not descriptors.
const SIDE_CHANNEL: c_int = 0x4000_0000;

/// The `_IO_CHMOD` message: a file's new permissions.
pub fn chmod_message(mode: u32) -> m::Chmod {
    m::Chmod {
        r#type: m::IO_CHMOD,
        combine_len: size_of::<m::Chmod>() as u16,
        mode,
    }
}

/// The `_IO_CHOWN` message: a file's new owner and group.
pub fn chown_message(uid: u32, gid: u32) -> m::Chown {
    m::Chown {
        r#type: m::IO_CHOWN,
        combine_len: size_of::<m::Chown>() as u16,
        gid: gid as i32,
        uid: uid as i32,
    }
}

/// `UTIME_NOW`: the time is now.
const UTIME_NOW: i64 = 0x4000_0001;

/// The message that sets a file's times: the 32-bit `_IO_UTIME`, or
/// `_IO_UTIME64`.
#[derive(Clone, Copy)]
pub enum UtimeMessage {
    Short(m::Utime),
    Long(m::Utime64),
}

impl UtimeMessage {
    /// The message's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            UtimeMessage::Short(u) => u.as_bytes(),
            UtimeMessage::Long(u) => u.as_bytes(),
        }
    }
}

/// The message that sets a file's access and modification times, from
/// `times` (null: now), as QNX's library builds it: the 32-bit
/// `_IO_UTIME` when it can carry them, `_IO_UTIME64` otherwise, with
/// `UTIME_NOW` and `UTIME_OMIT` in the nanoseconds.
///
/// # Safety
///
/// `times` is null or two `timespec`s.
pub unsafe fn utime_message(times: *const [i64; 4]) -> UtimeMessage {
    let short = |cur_flag, actime, modtime| {
        UtimeMessage::Short(m::Utime {
            r#type: m::IO_UTIME,
            combine_len: size_of::<m::Utime>() as u16,
            cur_flag,
            actime,
            modtime,
        })
    };
    let fits = |v: i64| (0..1 << 32).contains(&v);
    // SAFETY: the caller's guarantee.
    match unsafe { times.as_ref() } {
        None => short(1, 0, 0),
        Some(&[_, an, _, mn]) if an == UTIME_NOW && mn == UTIME_NOW => short(1, 0, 0),
        Some(&[a, 0, mt, 0]) if fits(a) && fits(mt) => short(0, a as u32, mt as u32),
        Some(&[a, an, mt, mn]) => UtimeMessage::Long(m::Utime64 {
            r#type: m::IO_UTIME64,
            combine_len: size_of::<m::Utime64>() as u16,
            zero: 0,
            atime_s: a,
            atime_ns: an,
            mtime_s: mt,
            mtime_ns: mn,
        }),
    }
}

/// Sets `fd`'s times.
///
/// # Safety
///
/// As [`utime_message`].
pub unsafe fn futimens(fd: c_int, times: *const [i64; 4]) -> Result<c_int, c_int> {
    // SAFETY: the caller's guarantee.
    let message = unsafe { utime_message(times) };
    send_nc(fd, message.as_bytes())
}

/// Sends a message of `msg` with no reply data on `fd`, not as a
/// cancellation point; the server's status.
fn send_nc(fd: c_int, msg: &[u8]) -> Result<c_int, c_int> {
    // SAFETY: the message is the caller's slice.
    unsafe {
        crate::neutrino::msg_sendvnc(fd, msg.as_ptr().cast(), msg.len(), core::ptr::null_mut(), 0)
    }
    .map(|status| status as c_int)
}

/// Changes `fd`'s permissions.
pub fn fchmod(fd: c_int, mode: u32) -> Result<c_int, c_int> {
    send_nc(fd, chmod_message(mode).as_bytes())
}

/// Changes `fd`'s owner and group.
pub fn fchown(fd: c_int, uid: u32, gid: u32) -> Result<c_int, c_int> {
    send_nc(fd, chown_message(uid, gid).as_bytes())
}

/// `O_SYNC` and `O_DSYNC`: what `_IO_SYNC` asks for.
pub const SYNC_FILE: u32 = 0x20;
pub const SYNC_DATA: u32 = 0x10;

/// Writes `fd`'s data, and with [`SYNC_FILE`] its status, to storage.
pub fn sync(fd: c_int, flag: u32) -> Result<c_int, c_int> {
    let msg = m::Sync {
        r#type: m::IO_SYNC,
        combine_len: size_of::<m::Sync>() as u16,
        flag,
    };
    let send = [Iov::of(&msg)];
    // SAFETY: the message is on this stack.
    unsafe { msg_sendv(fd, &send, &[]) }.map(|status| status as c_int)
}

/// `_IO_SPACE`'s subtypes (`<fcntl.h>`'s `F_*SP64`): free, allocate or
/// grow a file's space.
const F_GROWSP64: u16 = 0x6d;
const F_ALLOCSP64: u16 = 0x6e;
const F_FREESP64: u16 = 0x6f;

/// One `_IO_SPACE` request; the reply is the file's new size.
fn space(fd: c_int, subtype: u16, start: u64, len: u64) -> Result<u64, c_int> {
    space_from(fd, subtype, 0, start, len)
}

/// `space()` from `whence` (`SEEK_SET`, `SEEK_CUR`, `SEEK_END`).
fn space_from(fd: c_int, subtype: u16, whence: i16, start: u64, len: u64) -> Result<u64, c_int> {
    let msg = m::Space {
        r#type: m::IO_SPACE,
        combine_len: size_of::<m::Space>() as u16,
        subtype,
        whence,
        start,
        len,
    };
    let (_, size) = crate::neutrino::ask_nc::<_, u64>(fd, &msg)?;
    Ok(size)
}

/// Sets `fd`'s size to `length`: frees everything from it on, and if the
/// file was shorter, grows it, as a hole or else allocated.
pub fn truncate(fd: c_int, length: i64) -> Result<(), c_int> {
    use proto::errno::ESPIPE;
    let fix = |error| if error == ESPIPE { EINVAL } else { error };
    let length = u64::try_from(length).map_err(|_| EINVAL)?;
    let size = space(fd, F_FREESP64, length, 0).map_err(fix)?;
    if size < length {
        match space(fd, F_GROWSP64, size, length - size) {
            Err(EINVAL) => space(fd, F_ALLOCSP64, size, length - size).map_err(fix)?,
            other => other.map_err(fix)?,
        };
    }
    Ok(())
}

/// Frees everything of `fd` from `offset`, from `whence` (`ltrunc()`):
/// the file's new size.
pub fn free_from(fd: c_int, offset: i64, whence: c_int) -> Result<u64, c_int> {
    space_from(fd, F_FREESP64, whence as i16, offset as u64, 0)
}

/// Allocates `len` bytes of `fd` from `offset` (`posix_fallocate()`).
pub fn allocate(fd: c_int, offset: i64, len: i64) -> Result<(), c_int> {
    space(fd, F_ALLOCSP64, offset as u64, len as u64).map(|_| ())
}

/// Opens the file `fd` is open on again, with `oflag`, as a new
/// descriptor: `_IO_OPENFD`, which names the old connection. A pipe's
/// second end is opened this way.
pub fn openfd(fd: c_int, oflag: u32, sflag: u16, pipe: bool) -> Result<c_int, c_int> {
    use crate::neutrino::connect::{
        ConnectAttach_r, ConnectDetach_r, ConnectFlags_r, ConnectServerInfo_r,
    };
    const COF_PIPE: u16 = 0x400;
    const O_CLOEXEC: u32 = 0x2000;
    let mut info = ServerInfo::default();
    if fd == -1 || ConnectServerInfo_r(0, fd, (&raw mut info).cast()) != fd {
        return Err(EBADF);
    }
    let flags = if pipe || info.flags & COF_PIPE != 0 {
        0x41
    } else {
        COF_CLOEXEC
    };
    let new = ConnectAttach_r(info.nd, info.pid, info.chid, 0, flags);
    if new < 0 {
        return Err(-new);
    }
    let msg = m::Openfd {
        r#type: m::IO_OPENFD,
        combine_len: size_of::<m::Openfd>() as u16,
        // The access mode becomes _IO_FLAG_*, as in a connect request.
        ioflag: ((oflag + 1) & 7) | (oflag & !0x2007) | 0x8000,
        sflag,
        xtype: if pipe {
            m::IO_OPENFD_PIPE
        } else {
            m::IO_OPENFD_NONE
        },
        info: client_of(&info, fd),
        handle: 0,
    };
    let send = [Iov::of(&msg)];
    let sent = if pipe {
        send_pod_nc(new, &msg)
    } else {
        // SAFETY: as above.
        unsafe { msg_sendv(new, &send, &[]) }
    };
    if let Err(error) = sent {
        ConnectDetach_r(new);
        return Err(error);
    }
    if oflag & O_CLOEXEC == 0 {
        ConnectFlags_r(0, new, COF_CLOEXEC as c_uint, 0);
    }
    Ok(new)
}

/// Reads up to `nbytes` from `fd` with `_IO_READ` of extended type
/// `xtype`, as `_readx()`: a directory's entries, with
/// `_IO_XTYPE_READDIR`.
///
/// # Safety
///
/// `buf` is valid for writes of `nbytes` bytes.
pub unsafe fn readx(
    fd: c_int,
    buf: *mut c_void,
    nbytes: usize,
    xtype: u32,
) -> Result<usize, c_int> {
    let header = ReadWrite {
        r#type: m::IO_READ,
        combine_len: size_of::<ReadWrite>() as u16,
        nbytes: nbytes as u32,
        xtype,
        nbytes_hi: 0,
    };
    let send = [Iov::of(&header)];
    let reply = [Iov::raw(buf.cast_const(), nbytes)];
    // SAFETY: the header is on this stack; the reply buffer the caller's.
    let count = unsafe { msg_sendv(fd, &send, &reply) }?;
    if count as usize > nbytes {
        return Err(EBADMSG);
    }
    Ok(count as usize)
}

/// Asks `fd`'s server for its `struct _fdinfo`, into `info` (64 bytes),
/// and the file's path, into `path` (`maxlen` bytes, cut and terminated
/// if it does not fit). Returns the server's status: the path's length.
///
/// # Safety
///
/// `info` is writable for 64 bytes; `path` for `maxlen`.
pub unsafe fn fdinfo(
    fd: c_int,
    flags: u32,
    info: *mut u8,
    path: *mut u8,
    maxlen: usize,
) -> Result<c_int, c_int> {
    let msg = m::FdinfoReq {
        r#type: m::IO_FDINFO,
        combine_len: size_of::<m::FdinfoReq>() as u16,
        flags,
        path_len: maxlen as u32,
        reserved: 0,
    };
    let mut zero = [0u32; 2];
    let send = [Iov::of(&msg)];
    let reply = [
        Iov::raw((&raw mut zero).cast_const().cast(), 8),
        Iov {
            base: info.cast(),
            len: 64,
        },
        Iov {
            base: path.cast(),
            len: maxlen,
        },
    ];
    // SAFETY: the caller's guarantee, and the message.
    let n = unsafe { msg_sendv(fd, &send, &reply) }?;
    if maxlen > 0 {
        // SAFETY: as above.
        unsafe { path.wrapping_add(maxlen - 1).write(0) };
    }
    Ok(n as c_int)
}


/// Sends lock command `cmd` (`F_SETLK`, `F_SETLKW`, `F_GETLK`) with the
/// `flock_t` `fl`; for `F_GETLK`, the server's answer replaces it.
/// Returns the server's status.
pub fn lock(fd: c_int, cmd: u32, fl: &mut m::Flock) -> Result<c_int, c_int> {
    let msg = m::Lock {
        r#type: m::IO_LOCK,
        combine_len: size_of::<m::Lock>() as u16,
        subtype: cmd,
        nbytes: size_of::<m::Flock>() as u32,
    };
    let mut reply = m::LockReply::default();
    let send = [Iov::of(&msg), Iov::of(fl)];
    let back = [Iov::of_mut(&mut reply), Iov::of_mut(fl)];
    let parts = if cmd == m::F_GETLK {
        &back[..]
    } else {
        &back[..1]
    };
    // SAFETY: the message and the caller's lock.
    unsafe { msg_sendv(fd, &send, parts) }.map(|status| status as c_int)
}
