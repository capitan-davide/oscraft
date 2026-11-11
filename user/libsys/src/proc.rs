//! The process calls that go to the process manager: `fork()`,
//! `execve()` (a spawn that replaces the caller) and `wait4()`
//! (`<sys/procmsg.h>`), as QNX's libc makes them.

use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicI32, Ordering};

use proto::errno::{E2BIG, EINTR, EINVAL};
use proto::proc::{self as p, Fork, SpawnHeader, Wait};

use crate::errno::fail;
use crate::kercall::{Iov, SYSMGR_COID, msg_sendv, send};

/// `_FORK_ASPACE`: a separate address space, a real fork.
const FORK_ASPACE: u32 = 0x1;
/// OsCraft: the kernel sets it above the ID in a FreeBSD child's fork
/// reply (`proc::procmgr::fork_into`).
const FORK_CHILD: i64 = 1 << 30;
const RETRIES: u32 = 3;

/// The process's ID, once known: from the fork that made it, else the
/// auxiliary vector (`AT_OSCRAFT_PID`).
static PID: AtomicI32 = AtomicI32::new(0);

/// The calling process's ID.
pub fn pid() -> c_int {
    match PID.load(Ordering::Relaxed) {
        0 => {
            let id = crate::process::aux_pid();
            PID.store(id, Ordering::Relaxed);
            id
        }
        id => id,
    }
}

// ANCHOR: sys_fork
/// `fork()`: `_PROC_FORK`; the child's reply carries its ID.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_fork() -> c_int {
    let request = Fork {
        r#type: p::PROC_FORK,
        zero: 0,
        flags: FORK_ASPACE,
        frame: 0,
    };
    let mut tries = RETRIES;
    loop {
        match send(SYSMGR_COID, &request) {
            Ok(r) if r & FORK_CHILD != 0 => {
                PID.store((r & (FORK_CHILD - 1)) as c_int, Ordering::Relaxed);
                return 0;
            }
            Ok(r) => return r as c_int,
            Err(EINTR) if tries > 1 => tries -= 1,
            Err(e) => return fail(e) as c_int,
        }
    }
}
// ANCHOR_END: sys_fork

/// `_posix_spawnattr_t` (`<sys/posix_spawn.h>`), as the process manager
/// reads it; here always the defaults with the exec flag.
#[repr(C)]
#[derive(Clone, Copy)]
struct SpawnAttr {
    cookie: u32,
    flags: u32,
    pgrp: c_int,
    stack_max: u32,
    sig_default: u64,
    sig_mask: u64,
    sig_ignore: u64,
    param: [u32; 10],
    policy: i32,
    runmask: u64,
    uid: u32,
    gid: u32,
    type_id: u32,
    asid: u32,
    cwd_dirfd: c_int,
    aslr: u32,
    cid: c_int,
    reserved: [u32; 2],
}

// SAFETY: plain data, every bit pattern valid.
unsafe impl proto::pod::Pod for SpawnAttr {}

/// `SPAWN_EXEC`: the program replaces the caller.
const SPAWN_EXEC: u32 = 0x1_0000;
const ATTR_COOKIE: u32 = 0x7073_6163;
/// The most parts a request has: six fixed, then one per string.
const MAX_PARTS: usize = 1024;

/// The strings of a null-terminated array, each with its NUL.
///
/// # Safety
///
/// `array` is a null-terminated array of strings.
unsafe fn strings<'a>(array: *const *const c_char) -> impl Iterator<Item = &'a [u8]> {
    let mut at = array;
    core::iter::from_fn(move || {
        if at.is_null() {
            return None;
        }
        // SAFETY: the caller's guarantee.
        let s = unsafe { at.read() };
        if s.is_null() {
            return None;
        }
        at = at.wrapping_add(1);
        let mut n = 0;
        // SAFETY: a string: up to and with its NUL.
        while unsafe { s.add(n).read() } != 0 {
            n += 1;
        }
        // SAFETY: as above.
        Some(unsafe { core::slice::from_raw_parts(s.cast::<u8>(), n + 1) })
    })
}

/// `execve(path, argv, envp)`: `_PROC_SPAWN` with `SPAWN_EXEC`, one
/// message part per string; returns only on failure.
///
/// # Safety
///
/// `path` is a string; `argv` and `envp` are null-terminated arrays of
/// strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char) -> c_int {
    if path.is_null() {
        return fail(EINVAL) as c_int;
    }
    let attr = SpawnAttr {
        cookie: ATTR_COOKIE,
        flags: SPAWN_EXEC,
        pgrp: 0,
        stack_max: 0,
        sig_default: 0,
        sig_mask: 0,
        sig_ignore: 0,
        param: [1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        policy: 2,
        runmask: u64::MAX,
        uid: 0,
        gid: 0,
        type_id: 0,
        asid: 0,
        cwd_dirfd: -2,
        aslr: 0,
        cid: 0,
        reserved: [0; 2],
    };
    // SAFETY: the caller's guarantee.
    let path_bytes = unsafe { strings(&raw const path) }.next().unwrap_or(b"\0");
    let mut header = SpawnHeader {
        r#type: p::PROC_SPAWN,
        subtype: p::SPAWN_START,
        pathlen: path_bytes.len() as u16,
        attr_bytes: size_of::<SpawnAttr>() as u32,
        ..SpawnHeader::default()
    };
    let mut parts = [Iov::raw(core::ptr::null(), 0); MAX_PARTS];
    let mut n = 0;
    let mut push = |part: Iov| -> bool {
        if n == MAX_PARTS {
            return false;
        }
        parts[n] = part;
        n += 1;
        true
    };
    push(Iov::of(&header));
    push(Iov::of(&attr));
    push(Iov::raw(path_bytes.as_ptr().cast(), path_bytes.len()));
    let mut bytes: u64 = 0;
    // SAFETY: the caller's guarantee.
    for s in unsafe { strings(argv) } {
        header.nargv += 1;
        bytes += s.len() as u64;
        if !push(Iov::raw(s.as_ptr().cast(), s.len())) {
            return fail(E2BIG) as c_int;
        }
    }
    // SAFETY: the caller's guarantee.
    for s in unsafe { strings(envp) } {
        header.narge += 1;
        bytes += s.len() as u64;
        if !push(Iov::raw(s.as_ptr().cast(), s.len())) {
            return fail(E2BIG) as c_int;
        }
    }
    if bytes > 0xffff_f6c3 {
        return fail(E2BIG) as c_int;
    }
    header.argenv_bytes = bytes as u32;
    // The header's counts are final now: its part points at it.
    parts[0] = Iov::of(&header);
    let mut tries = RETRIES;
    loop {
        // SAFETY: every part is valid for its length while the call lasts.
        match unsafe { msg_sendv(SYSMGR_COID, &parts[..n], &[]) } {
            Ok(_) => return 0, // not reached: the image was replaced
            Err(EINTR) if tries > 1 => tries -= 1,
            Err(e) => return fail(e) as c_int,
        }
    }
}

/// FreeBSD's `siginfo_t`, which the process manager answers a wait with.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ChildInfo {
    signo: c_int,
    errno: c_int,
    code: c_int,
    pid: c_int,
    uid: u32,
    status: c_int,
    rest: [u64; 7],
}

const _: () = assert!(size_of::<ChildInfo>() == 80);

// SAFETY: plain data, every bit pattern valid.
unsafe impl proto::pod::Pod for ChildInfo {}

const SIGCHLD: c_int = 20;
const CLD_EXITED: c_int = 1;
const CLD_KILLED: c_int = 2;
const CLD_DUMPED: c_int = 3;
const CLD_TRAPPED: c_int = 4;
const CLD_STOPPED: c_int = 5;
const CLD_CONTINUED: c_int = 6;
// FreeBSD's wait options, and QNX's.
const BSD_WNOHANG: c_int = 1;
const BSD_WUNTRACED: c_int = 2;
const BSD_WCONTINUED: c_int = 4;
const BSD_WNOWAIT: c_int = 8;
const BSD_WEXITED: c_int = 16;
const BSD_WTRAPPED: c_int = 32;
const QNX_WEXITED: c_int = 0x1;
const QNX_WTRAPPED: c_int = 0x2;
const QNX_WUNTRACED: c_int = 0x4;
const QNX_WCONTINUED: c_int = 0x8;
const QNX_WNOHANG: c_int = 0x40;
const QNX_WNOWAIT: c_int = 0x80;
const P_ALL: i16 = 0;
const P_PID: i16 = 1;
const P_PGID: i16 = 2;
/// FreeBSD's status word for a continued child (`_W_CONTINUED`).
const BSD_W_CONTINUED: c_int = 0x13;
/// `struct rusage`: 18 longs.
const RUSAGE_WORDS: usize = 18;

/// `wait4(pid, status, options, rusage)`: `_PROC_WAIT`, its reply turned
/// into FreeBSD's status word, the signal numbers mapped. The times are
/// not reported yet (zeros).
///
/// # Safety
///
/// `status` and `rusage` are writable or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_wait4(pid: c_int, status: *mut c_int, options: c_int, rusage: *mut c_void) -> c_int {
    let (idtype, id) = match pid {
        -1 => (P_ALL, 0),
        0 => (P_PGID, crate::cred::pgrp()),
        p if p < -1 => (P_PGID, -p),
        p => (P_PID, p),
    };
    let mut q = QNX_WEXITED | QNX_WTRAPPED;
    for (b, qo) in [
        (BSD_WNOHANG, QNX_WNOHANG),
        (BSD_WUNTRACED, QNX_WUNTRACED),
        (BSD_WCONTINUED, QNX_WCONTINUED),
        (BSD_WNOWAIT, QNX_WNOWAIT),
        (BSD_WEXITED, QNX_WEXITED),
        (BSD_WTRAPPED, QNX_WTRAPPED),
    ] {
        if options & b != 0 {
            q |= qo;
        }
    }
    let request = Wait {
        r#type: p::PROC_WAIT,
        idtype,
        options: q,
        id,
    };
    let mut info = ChildInfo::default();
    let sendv = [Iov::of(&request)];
    let recv = [Iov::raw((&raw mut info).cast(), size_of::<ChildInfo>())];
    // SAFETY: both are on this stack.
    if let Err(e) = unsafe { msg_sendv(SYSMGR_COID, &sendv, &recv) } {
        return fail(e) as c_int;
    }
    if info.signo != SIGCHLD {
        return 0;
    }
    let sig = |s: c_int| s & 0x7f;
    let word = match info.code {
        CLD_EXITED => (info.status & 0xff) << 8,
        CLD_KILLED => sig(info.status),
        CLD_DUMPED => sig(info.status) | 0x80,
        CLD_TRAPPED | CLD_STOPPED => (sig(info.status) << 8) | 0x7f,
        CLD_CONTINUED => BSD_W_CONTINUED,
        _ => return fail(EINVAL) as c_int,
    };
    if !status.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { status.write(word) };
    }
    if !rusage.is_null() {
        // SAFETY: the caller's guarantee: a struct rusage.
        unsafe { core::ptr::write_bytes(rusage.cast::<i64>(), 0, RUSAGE_WORDS) };
    }
    info.pid
}
