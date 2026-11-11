//! Process creation from a program file: `_PROC_SPAWN` (`<sys/procmsg.h>`),
//! which libc sends for `spawn()`, `posix_spawn()` and the `exec*()`
//! functions, and `_PROC_POSIX_SPAWNP`, which procnto handles the same way
//! (its `procmgr_handler` sends both to `procmgr_spawn`).
//!
//! The request (subtype `_PROC_SPAWN_START`) is a `struct
//! _proc_posix_spawn` (not the older `struct _proc_spawn` the header
//! documents next to the subtype): pathlen at 4, nargv at 6, narge at 8,
//! searchlen at 10, filact_bytes at 16, filact_obytes at 20, argenv_bytes
//! at 24 and attr_bytes at 28; 32 bytes. Then come libc's private
//! `_posix_spawnattr_t`, the file actions and their open area, the search
//! path (PATH, for `POSIX_SPAWN_SEARCH_PATH` or `posix_spawnp()`), the
//! program's path, and the
//! argument and environment strings, NUL-terminated.
//!
//! The attributes, as libc's `posix_spawnattr_set*()` fill them (and
//! procnto's `procmgr_create_child` reads them): a magic number, flags at
//! 4, process group at 8, stack size at 12, then `sigset_t`s: signals to
//! reset to their default action at 16, the signal mask at 24, signals to
//! ignore at 32; the `sched_param` at 40 (priority first), the policy at
//! 80, the CPU runmask at 88, uid and gid at 96, the working directory's
//! descriptor at 112. Each applies only with its `POSIX_SPAWN_*` flag.
//!
//! The file actions, as libc's `posix_spawn_file_actions_add*()` fill them
//! (QNX's `spawn()` turns its fd map into them too, for `popen()`): a
//! magic number, the open area's size at 16, the number of actions at 20,
//! then 12-byte actions { type, a, b } from 24: 1 close(a), 2 dup2(a, b),
//! 3 open (`a`: the offset of its entry in the open area). An open entry
//! is { size, mode, oflags, fd, path }. The child starts with the parent's
//! descriptors, the actions apply in order, and then those with
//! close-on-exec close (a dup2() target has it cleared). procnto opens the
//! files it serves itself (the image filesystem, /dev/null, /dev/zero,
//! /dev/shmem and so /tmp). Another server's file the child opens itself
//! as it starts, as procnto's loader thread does (`ker::StartupOpen`); the
//! spawn fails with the server's errno, and leaves no child, if that
//! fails.
//!
//! `POSIX_SPAWN_SETCRED` gives the child the attributes' uid and gid (real,
//! effective and saved) and needs `PROCMGR_AID_SPAWN_SETUID` (and
//! `_SETGID`), else EPERM; `POSIX_SPAWN_RESETIDS` sets the effective IDs to
//! the real ones. `POSIX_SPAWN_SETCWD` makes a directory opened with
//! `O_DIRECTORY` the working directory (ENOTDIR for any other descriptor,
//! as procnto). With `POSIX_SPAWN_EXEC` these apply to the exec()ing
//! process.
//!
//! Like a forked child, a spawned one tells the other servers about the
//! descriptors it inherits (`_IO_DUP`, see `ker::fork_startup`) before its
//! program runs, and the parent's reply waits for that: the parent may
//! close its copies as soon as it has it (`popen()` does).
//!
//! With `POSIX_SPAWN_EXEC` the sending process is replaced: the
//! close-on-exec file descriptors are closed, caught signals go back to
//! their default action, the memory is emptied and the program loaded, and
//! a new thread starts it (with the old thread's signal mask) while the
//! sending thread ends. Otherwise a child process is created and its pid
//! is the reply.
//!
//! `POSIX_SPAWN_SETSTACKMAX` sets the size of the main thread's stack
//! (512 KB by default). With `POSIX_SPAWN_EXEC`, file actions (spawn()'s
//! fd map) are ignored, as procnto ignores them.
//!
//! OsCraft doesn't bind threads to CPUs, so the runmask is accepted and
//! has no effect. `POSIX_SPAWN_NOZOMBIE` severs the new program from its
//! parent, so it leaves no zombie (on's `-d`). A child spawned so is
//! procnto's from the start. With `POSIX_SPAWN_EXEC` (procnto's
//! `procmgr_nozombie`), the exec()ing process's parent sees it exit with
//! status 0: its waits report that and it gets a SIGCHLD, while the
//! program runs on. Programs and `#!` scripts come from the image
//! filesystem, through its links (`/tmp`) from procnto's `/dev/shmem`,
//! or from other servers' filesystems (read as procnto, on its worker
//! thread); all but the image's must be executable (EACCES). An exec()ing
//! process's other threads end first (as procnto destroys them), and
//! those that ended unjoined are forgotten.

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use super::resmgr::{self, Client};
use super::{imagefs, loader, pathmgr, procmgr};
use crate::arch::context::FpState;
use crate::arch::exception::TrapFrame;
use crate::arch::kercall::call;
use crate::ker::errno::{E2BIG, EACCES, ENOTDIR, EPERM, EBADF, EBADMSG, EINVAL, ENOENT, ENOEXEC, ENOMEM, ENOSYS, ESRCH};
use crate::ker::number::*;
use crate::ker::StartupOpen;
use crate::ker::process::{self, Process, ProcessRef};
use crate::kprintln;
use crate::sched;
use crate::vm::map;

pub const PROC_SPAWN: u16 = 0x0010;
pub const PROC_POSIX_SPAWNP: u16 = 0x001c;
const SPAWN_START: u16 = 6;
const HDR: usize = 32;
/// Offsets in the attributes.
const ATTR_FLAGS: usize = 4;
const ATTR_STACK_MAX: usize = 12;
const ATTR_PGROUP: usize = 8;
const ATTR_SIGDEFAULT: usize = 16;
const ATTR_SIGMASK: usize = 24;
const ATTR_SIGIGNORE: usize = 32;
const ATTR_PRIORITY: usize = 40;
const ATTR_POLICY: usize = 80;
const ATTR_UID: usize = 96;
const ATTR_GID: usize = 100;
const ATTR_CWD: usize = 112;
/// Smallest attributes procnto accepts.
const ATTR_MIN: usize = 0x6c;
// _posix_spawnattr_t flags (<spawn.h>)
const SETPGROUP: u32 = 0x1;
const SETSIGMASK: u32 = 0x2;
const SETSIGDEF: u32 = 0x4;
const SETSIGIGN: u32 = 0x8;
const SETSCHEDULER: u32 = 0x40;
const SETSID: u32 = 0x200;
const SETSCHEDPARAM: u32 = 0x400;
const EXPLICIT_CPU: u32 = 0x800;
const NOZOMBIE: u32 = 0x2000;
const HOLD: u32 = 0x8000;
const CHECK_SCRIPT: u32 = 0x4_0000;
const EXEC: u32 = 0x1_0000;
const SEARCH_PATH: u32 = 0x2_0000;
const RESETIDS: u32 = 0x10;
const SETCRED: u32 = 0x8_0000;
const SETCWD: u32 = 0x400_0000;
const SETSTACKMAX: u32 = 0x1000;
/// `POSIX_SPAWN_NEWAPP`: a new application ID for the child; it takes
/// `PROCMGR_AID_CHILD_NEWAPP` (application IDs aren't kept otherwise).
const NEWAPP: u32 = 0x1000_0000;
/// The flags this implementation acts on, or that need nothing.
const SUPPORTED: u32 = SETPGROUP
    | SETSIGMASK
    | SETSIGDEF
    | SETSIGIGN
    | SETSCHEDULER
    | SETSID
    | SETSCHEDPARAM
    | EXPLICIT_CPU
    | NOZOMBIE
    | HOLD
    | CHECK_SCRIPT
    | EXEC
    | SEARCH_PATH
    | RESETIDS
    | SETCRED
    | SETCWD
    | SETSTACKMAX
    | NEWAPP;
// Scheduling policies (<sched.h>).
const SCHED_FIFO: i32 = 1;
const SCHED_RR: i32 = 2;
const SCHED_OTHER: i32 = 3;
const SIG_IGN: u64 = 1;
/// File actions: header size, action size, open entry header size.
const FA_HDR: usize = 24;
const FA_SIZE: usize = 12;
const FA_OPEN_HDR: usize = 16;
// File action types.
const FA_CLOSE: u32 = 1;
const FA_DUP2: u32 = 2;
const FA_OPEN: u32 = 3;
/// QNX's `O_CLOEXEC`: spawn requests are QNX's message, and only QNX
/// callers send file actions.
const O_CLOEXEC: u32 = 0o20000;
const O_ACCMODE: u32 = 3;
const S_IFREG: u32 = 0o100000;

/// A file action.
enum FileAction<'a> {
    Close(i32),
    Dup2(i32, i32),
    Open {
        fd: i32,
        path: &'a str,
        oflags: u32,
        mode: u32,
    },
}

struct Request<'a> {
    flags: u32,
    pgroup: i32,
    sigdefault: crate::ker::signal::SigSet,
    sigmask: crate::ker::signal::SigSet,
    sigignore: crate::ker::signal::SigSet,
    priority: i32,
    policy: i32,
    uid: u32,
    gid: u32,
    cwd_fd: i32,
    /// The main thread's stack size (`POSIX_SPAWN_SETSTACKMAX`).
    stack_size: u64,
    search: &'a str,
    path: &'a str,
    argv: Vec<&'a [u8]>,
    envp: Vec<&'a [u8]>,
    actions: Vec<FileAction<'a>>,
    /// Sent as `_PROC_SPAWN` (libc's `spawn()`), not `_PROC_POSIX_SPAWN`:
    /// file actions are then the child's whole fd map.
    fd_map: bool,
}

fn u16_at(b: &[u8], o: usize) -> usize {
    u16::from_le_bytes([b[o], b[o + 1]]) as usize
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// A NUL-terminated string, without the NUL.
fn cstr(b: &[u8]) -> &str {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..end]).unwrap_or("")
}

/// The file actions in `acts`, with their open area `opens`.
fn file_actions<'a>(acts: &'a [u8], opens: &'a [u8]) -> Result<Vec<FileAction<'a>>, u32> {
    if acts.is_empty() {
        return Ok(Vec::new());
    }
    if acts.len() < FA_HDR {
        return Err(EBADMSG);
    }
    let count = u32_at(acts, 20) as usize;
    if acts.len() != FA_HDR + count * FA_SIZE {
        return Err(EBADMSG);
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = FA_HDR + i * FA_SIZE;
        let (a, b) = (u32_at(acts, at + 4), u32_at(acts, at + 8));
        out.push(match u32_at(acts, at) {
            FA_CLOSE => FileAction::Close(a as i32),
            FA_DUP2 => FileAction::Dup2(a as i32, b as i32),
            FA_OPEN => {
                let e = opens.get(a as usize..).ok_or(EBADMSG)?;
                if e.len() < FA_OPEN_HDR {
                    return Err(EBADMSG);
                }
                let size = (u32_at(e, 0) as usize).min(e.len());
                FileAction::Open {
                    fd: u32_at(e, 12) as i32,
                    path: cstr(e.get(FA_OPEN_HDR..size).ok_or(EBADMSG)?),
                    oflags: u32_at(e, 8),
                    mode: u32_at(e, 4),
                }
            }
            _ => return Err(EINVAL),
        });
    }
    Ok(out)
}

fn parse(msg: &[u8]) -> Result<Request<'_>, u32> {
    if msg.len() < HDR {
        return Err(EINVAL);
    }
    let (pathlen, nargv, narge, searchlen) = (
        u16_at(msg, 4),
        u16_at(msg, 6),
        u16_at(msg, 8),
        u16_at(msg, 10),
    );
    let (filact, filact_open) = (u32_at(msg, 16) as usize, u32_at(msg, 20) as usize);
    let (argenv, attr_len) = (u32_at(msg, 24) as usize, u32_at(msg, 28) as usize);
    if attr_len < ATTR_MIN {
        return Err(EBADMSG);
    }
    let attr = msg.get(HDR..HDR + attr_len).ok_or(EBADMSG)?;
    let acts_at = HDR + attr_len;
    let actions = file_actions(
        msg.get(acts_at..acts_at + filact).ok_or(E2BIG)?,
        msg.get(acts_at + filact..acts_at + filact + filact_open)
            .ok_or(E2BIG)?,
    )?;
    let search_at = HDR + attr_len + filact + filact_open;
    let path_at = search_at + searchlen;
    let strings_at = path_at + pathlen;
    // The whole request must have arrived (no MsgRead() of the rest yet).
    let strings = msg.get(strings_at..strings_at + argenv).ok_or(E2BIG)?;
    let mut s = strings.split(|&c| c == 0);
    let argv: Vec<&[u8]> = s.by_ref().take(nargv).collect();
    let envp: Vec<&[u8]> = s.take(narge).collect();
    if argv.len() != nargv || envp.len() != narge {
        return Err(EINVAL);
    }
    let flags = u32_at(attr, ATTR_FLAGS);
    if flags & !SUPPORTED != 0 {
        kprintln!(
            "spawn: attribute flags {:#x} not implemented, ignored",
            flags & !SUPPORTED
        );
    }
    Ok(Request {
        flags,
        pgroup: u32_at(attr, ATTR_PGROUP) as i32,
        sigdefault: crate::personality::qnx::set_in(u64_at(attr, ATTR_SIGDEFAULT)),
        sigmask: crate::personality::qnx::set_in(u64_at(attr, ATTR_SIGMASK)),
        sigignore: crate::personality::qnx::set_in(u64_at(attr, ATTR_SIGIGNORE)),
        priority: u32_at(attr, ATTR_PRIORITY) as i32,
        policy: u32_at(attr, ATTR_POLICY) as i32,
        uid: u32_at(attr, ATTR_UID),
        gid: u32_at(attr, ATTR_GID),
        cwd_fd: attr.get(ATTR_CWD..ATTR_CWD + 4).map_or(-1, |b| u32_at(b, 0) as i32),
        stack_size: match u32_at(attr, ATTR_STACK_MAX) {
            n if flags & SETSTACKMAX != 0 && n != 0 => n as u64,
            _ => loader::STACK_SIZE,
        },
        search: cstr(&msg[search_at..path_at]),
        path: cstr(&msg[path_at..strings_at]),
        argv,
        envp,
        actions,
        fd_map: u16_at(msg, 0) as u16 == PROC_SPAWN,
    })
}

/// Where a child's descriptor comes from.
#[derive(Clone)]
enum Source {
    /// A copy of the parent's descriptor.
    Parent(i32),
    /// A file process 1 opened for it.
    Opened(resmgr::Opened),
    /// Another server's file, which the child opens as it starts: the
    /// index of its open action.
    External(usize),
}

/// The child's descriptors: the parent's, changed by the file actions,
/// without those that close on exec. The files of other servers the child
/// is to open (`Source::External`) are those of `opens`, by index; their
/// `fd` and `copies` are left for the caller.
fn child_fds(parent: &Process, req: &Request, opens: &mut Vec<StartupOpen>) -> Result<BTreeMap<i32, Source>, u32> {
    // `spawn()` with an fd map (sent as file actions): the child gets the
    // descriptors it maps and no others; without one, as `posix_spawn()`,
    // all those not closing on exec (measured).
    let mut fds: BTreeMap<i32, (Source, bool)> = if req.fd_map && !req.actions.is_empty() {
        BTreeMap::new()
    } else {
        parent
            .fds()
            .into_iter()
            .map(|(fd, cloexec)| (fd, (Source::Parent(fd), cloexec)))
            .collect()
    };
    // Every descriptor an action names in the child is checked against
    // the limit (EBADF, as procnto: a close of 0x7fffffff fails the spawn
    // with it); the child's table is sized by the largest one.
    let nofile = parent.nofile() as i32;
    let in_range = |fd: i32| if (0..nofile).contains(&fd) { Ok(fd) } else { Err(EBADF) };
    for a in &req.actions {
        match *a {
            FileAction::Close(fd) => {
                fds.remove(&in_range(fd)?);
            }
            FileAction::Dup2(fd, to) => {
                let to = in_range(to)?;
                // A map names the parent's descriptors, whatever the child
                // has at that number so far.
                let src = if req.fd_map {
                    parent.connection(fd).ok_or(EBADF)?;
                    Source::Parent(fd)
                } else {
                    fds.get(&fd).ok_or(EBADF)?.0.clone()
                };
                fds.insert(to, (src, false));
            }
            FileAction::Open { fd, path, oflags, mode } => {
                let fd = in_range(fd)?;
                let src = match resmgr::open_local(parent.pid, path, oflags, mode) {
                    Ok(opened) => Source::Opened(opened),
                    Err(ENOSYS) => {
                        let abs = pathmgr::absolute(parent.pid, path);
                        let abs = imagefs::get().map_or(abs.clone(), |fs| fs.resolve(&abs));
                        let (pid, chid, handle, rest) = pathmgr::lookup(&abs).ok_or(ENOENT)?;
                        if pid == process::KERNEL_PID {
                            return Err(ENOSYS);
                        }
                        opens.push(StartupOpen {
                            fd: -1,
                            copies: Vec::new(),
                            pid,
                            chid,
                            handle,
                            path: rest,
                            ioflag: (oflags & !O_ACCMODE) | ((oflags & O_ACCMODE) + 1),
                            mode: (mode & 0o7777) | S_IFREG,
                        });
                        Source::External(opens.len() - 1)
                    }
                    Err(e) => return Err(e),
                };
                fds.insert(fd, (src, oflags & O_CLOEXEC != 0));
            }
        }
    }
    Ok(fds
        .into_iter()
        .filter(|(_, (_, cloexec))| !cloexec)
        .map(|(fd, (src, _))| (fd, src))
        .collect())
}

/// The program to run: (ELF image, absolute path). The path is the one
/// searched, links not resolved: procnto's loader names the process after
/// it (`set_debug_name()`), so a toybox command keeps its own name.
/// A program found: its ELF image and path, and for a `#!` script, the
/// arguments to start its interpreter with.
struct Found {
    elf: Cow<'static, [u8]>,
    path: String,
    script_argv: Option<Vec<Vec<u8>>>,
}

/// Find the program `req` names (along the search path if asked). A file
/// that starts with `#!interpreter [argument]` is a script: the
/// interpreter (an ELF program) runs with `interpreter [argument] script`
/// followed by the arguments after the first.
fn find(sender: i32, req: &Request, external: bool) -> Result<Found, u32> {
    let (data, path) = find_file(sender, req, external)?;
    if !data.starts_with(b"#!") {
        if !data.starts_with(b"\x7fELF") {
            return Err(ENOEXEC);
        }
        return Ok(Found {
            elf: data,
            path,
            script_argv: None,
        });
    }
    let line = data[2..].split(|&c| c == b'\n').next().unwrap_or(&[]);
    let line = core::str::from_utf8(line).map_err(|_| ENOEXEC)?.trim();
    let (interp, arg) = match line.split_once([' ', '\t']) {
        Some((i, a)) => (i, Some(a.trim())),
        None => (line, None),
    };
    if interp.is_empty() {
        return Err(ENOEXEC);
    }
    let abs = pathmgr::absolute(sender, interp);
    let interp_elf = program(&abs, external).ok_or(ENOENT)??;
    if !interp_elf.starts_with(b"\x7fELF") {
        return Err(ENOEXEC);
    }
    let mut argv: Vec<Vec<u8>> = alloc::vec![interp.as_bytes().to_vec()];
    argv.extend(arg.filter(|a| !a.is_empty()).map(|a| a.as_bytes().to_vec()));
    argv.push(path.as_bytes().to_vec());
    argv.extend(req.argv.iter().skip(1).map(|a| a.to_vec()));
    Ok(Found {
        elf: interp_elf,
        path: alloc::format!("/{abs}"),
        script_argv: Some(argv),
    })
}

/// "Run this spawn again on the worker thread": its program is another
/// server's (see `program`).
const DEFER: u32 = u32::MAX;

/// The program file at absolute path `abs`: in the image filesystem;
/// through the image's links (`/tmp`), a file of procnto's `/dev/shmem`;
/// or a file of another server's (a filesystem's), read as procnto would
/// (`client`), only with `external` (on the worker thread: the server may
/// ask procnto in turn), else DEFER. Others than the image's must be
/// executable regular files (EACCES). None if there is no such file.
fn program(abs: &str, external: bool) -> Option<Result<Cow<'static, [u8]>, u32>> {
    const S_IXALL: u32 = 0o111;
    const S_IFMT: u32 = 0o170000;
    const S_IFREG: u32 = 0o100000;
    let fs = imagefs::get()?;
    // A name that isn't a regular file (a directory, procnto's devices):
    // there, but not executable (procnto: EACCES).
    if let Some((entry, _)) = fs.follow(abs) {
        if entry.mode & imagefs::S_IFMT != imagefs::S_IFREG {
            return Some(Err(EACCES));
        }
        return Some(Ok(Cow::Borrowed(entry.data)));
    }
    let resolved = fs.resolve(abs);
    if let Some(name) = resolved.strip_prefix("dev/shmem/") {
        let (data, mode) = super::shmem::contents(name)?;
        return Some(if mode & S_IXALL == 0 { Err(EACCES) } else { Ok(Cow::Owned(data)) });
    }
    let (server, _, _, rest) = pathmgr::lookup(&resolved)?;
    if server == process::KERNEL_PID {
        return rest.is_empty().then_some(Err(EACCES));
    }
    if !external {
        return Some(Err(DEFER));
    }
    let coid = super::client::open(&resolved, super::client::READ).ok()?;
    let r = super::client::stat(coid).and_then(|(size, mode)| {
        if mode & S_IFMT != S_IFREG || mode & S_IXALL == 0 {
            return Err(EACCES);
        }
        super::client::read_all(coid, size)
    });
    super::client::close(coid);
    Some(r.map(Cow::Owned))
}

/// The file `req` names: its contents and absolute path.
fn find_file(sender: i32, req: &Request, external: bool) -> Result<(Cow<'static, [u8]>, String), u32> {
    // posix_spawnp() sends the search path without `POSIX_SPAWN_SEARCH_PATH`.
    let search = req.flags & SEARCH_PATH != 0 || !req.search.is_empty();
    let candidates: Vec<String> = if search && !req.path.contains('/') {
        req.search
            .split(':')
            .map(|dir| alloc::format!("{}/{}", if dir.is_empty() { "." } else { dir }, req.path))
            .collect()
    } else {
        alloc::vec![req.path.into()]
    };
    // Not executable: EACCES, unless another candidate is.
    let mut err = ENOENT;
    for c in candidates {
        let abs = pathmgr::absolute(sender, &c);
        match program(&abs, external) {
            Some(Ok(data)) => return Ok((data, alloc::format!("/{abs}"))),
            Some(Err(DEFER)) => return Err(DEFER),
            Some(Err(e)) => err = e,
            None => {}
        }
    }
    Err(err)
}

/// Signal dispositions across exec()/spawn(): caught signals go back to
/// their default action; ignored ones stay ignored. Then those the
/// attributes name are reset (`sigdefault`) or ignored (`sigignore`).
fn reset_signals(p: &Process, req: &Request) {
    let mut signals = p.signals.lock();
    for (n, a) in signals.actions.iter_mut().enumerate() {
        // Bit n - 1 is signal n.
        let named = |set: crate::ker::signal::SigSet, flag: u32| req.flags & flag != 0 && n > 0 && set >> (n - 1) & 1 != 0;
        if a.handler > SIG_IGN || named(req.sigdefault, SETSIGDEF) {
            *a = Default::default();
        }
        if named(req.sigignore, SETSIGIGN) {
            a.handler = SIG_IGN;
        }
    }
}

/// The new program's credentials and working directory (None: the
/// parent's), as the attributes set them:
/// - `POSIX_SPAWN_RESETIDS`: the effective IDs become the real ones;
/// - `POSIX_SPAWN_SETCRED`: all its user and group IDs; other than the
///   parent's effective ones, they take `PROCMGR_AID_SPAWN_SETUID` and
///   `_SETGID` (for the ID) (EPERM);
/// - `POSIX_SPAWN_SETCWD`: the directory of a descriptor opened with
///   `O_DIRECTORY` (else ENOTDIR).
///
/// As procnto (spawn2test).
fn identity(parent: &Process, req: &Request) -> Result<(process::Cred, Option<String>), u32> {
    let mut c = parent.cred.lock().clone();
    if req.flags & RESETIDS != 0 {
        (c.euid, c.suid, c.egid, c.sgid) = (c.ruid, c.ruid, c.rgid, c.rgid);
    }
    if req.flags & SETCRED != 0 {
        use crate::ker::ability;
        let euid = c.euid;
        if req.uid != c.euid && !ability::allowed_as(parent, euid, ability::SPAWN_SETUID, Some(req.uid as u64)) {
            return Err(EPERM);
        }
        if req.gid != c.egid && !ability::allowed_as(parent, euid, ability::SPAWN_SETGID, Some(req.gid as u64)) {
            return Err(EPERM);
        }
        (c.ruid, c.euid, c.suid) = (req.uid, req.uid, req.uid);
        (c.rgid, c.egid, c.sgid) = (req.gid, req.gid, req.gid);
    }
    let cwd = if req.flags & SETCWD != 0 {
        Some(resmgr::opendir_path(parent.pid, req.cwd_fd).ok_or(ENOTDIR)?)
    } else {
        None
    };
    Ok((c, cwd))
}

/// The child's scheduling: the priority and policy the attributes set, or
/// those of the thread that spawns it.
fn scheduling(req: &Request, thread: &sched::Thread) -> Result<(u8, sched::Policy), u32> {
    let prio = if req.flags & SETSCHEDPARAM != 0 {
        crate::ker::ability::priority_for(&thread.process, req.priority as i64)?
    } else {
        thread.base_priority()
    };
    let policy = if req.flags & SETSCHEDULER != 0 {
        match req.policy {
            SCHED_FIFO => sched::Policy::Fifo,
            SCHED_RR => sched::Policy::RoundRobin,
            SCHED_OTHER => sched::Policy::Other,
            _ => return Err(EINVAL),
        }
    } else {
        thread.policy()
    };
    Ok((prio, policy))
}

/// The threads of `p` other than `keep` end (an exec): each as it next
/// returns to EL0, woken if blocked, interrupted if running; this waits
/// until they have, and forgets those that ended unjoined.
fn end_other_threads(p: &Process, keep: &sched::Thread) {
    let others = || {
        sched::threads_of(p)
            .into_iter()
            .filter(|t| !core::ptr::eq(&**t, keep))
            .collect::<Vec<_>>()
    };
    for t in others() {
        t.exit_on_return.store(true, Ordering::Relaxed);
        if t.state().is_blocked() {
            crate::ker::unblock(&t, crate::ker::errno::EINTR);
            // One REPLY-blocked on procnto's own channel (a held wait)
            // only got the unblock pulse, and the thread that serves that
            // pulse is this one, which is waiting here: answer it now,
            // as the pulse would (review item PROCMGR-2: a thread in
            // waitpid() while another called exec() hung procnto).
            // So for one held in a fork or a spawn until its child has
            // started (review item PROCMGR-26): the child's start-up
            // pulse would come to this thread too.
            if t.state() == sched::State::Reply && !procmgr::abandon_startup(t.tid as u64) {
                procmgr::unblocked(t.tid as u64);
            }
        } else {
            sched::interrupt(&t);
        }
    }
    // A short sleep, not a yield: a lower-priority thread must get to run.
    while !others().is_empty() {
        crate::timeout::sleep_until(crate::clock::now_ns() + 100_000);
    }
    sched::wait_off_cpu_but(p, keep);
    sched::forget_process(p);
}

/// Load the program into `p` and start its first thread.
fn start(
    p: &Process,
    req: &Request,
    argv: &[&[u8]],
    elf: &[u8],
    path: &str,
    (prio, policy): (u8, sched::Policy),
    sigmask: crate::ker::signal::SigSet,
    dups: Option<Vec<[i32; 5]>>,
    opens: Option<Vec<StartupOpen>>,
    closes: Option<Vec<i32>>,
) -> Result<(), u32> {
    let s = loader::load(p, elf, path, argv, &req.envp, req.stack_size).map_err(|e| {
        kprintln!("spawn: {path}: {e}");
        loader::errno_of(e)
    })?;
    let frame = TrapFrame::new_user(s.entry, s.sp, s.x0);
    sched::spawn_user_frame_init(
        p,
        "main",
        prio,
        frame,
        s.tp,
        FpState::new(),
        sigmask,
        1,
        |t| {
            t.set_policy(policy);
            *t.pending_dups.lock() = dups;
            *t.pending_opens.lock() = opens;
            *t.pending_closes.lock() = closes;
        },
    )
    .ok_or(ENOMEM)?;
    Ok(())
}

/// A child that never ran ends: it leaves no zombie and sends its parent no
/// SIGCHLD, but sends `PROCESS_DEATH` (`procmgr::process_exited`), which
/// frees what it holds and takes it out of the process table. Also for a
/// boot script's program that fails to start (`bootscript::external`).
pub(super) fn abandon(child: &Process) {
    child.nozombie.store(true, Ordering::Relaxed);
    process::last_thread_exited(child);
}

/// A spawn that fails once its message has been read: procnto makes the
/// child before it looks at the program (`procmgr_create_child()`), so the
/// failure still uses up a pid and sends `PROCESS_DEATH`, without
/// `PROCESS_CREATE` (measured). The child, if made already, or one made
/// for that, ends. An exec has no child to make.
fn failed(parent: &Process, req: &Request, child: Option<ProcessRef>, err: u32) -> u32 {
    if req.flags & EXEC == 0
        && err != DEFER
        && let Some(child) = child.or_else(|| Some(process::spawn_child(parent, Some(&[]))?.0))
    {
        abandon(&child);
    }
    err
}

fn spawn(rcvid: u64, sender: i32, tid: u32, msg: &[u8], external: bool) -> Result<(), u32> {
    let req = parse(msg)?;
    let parent = process::lookup(sender).ok_or(ESRCH)?;
    let thread = sched::thread_of(&parent, tid).ok_or(ESRCH)?;
    let mut child = None;
    spawn_as(rcvid, sender, &req, &parent, &thread, external, &mut child)
        .map_err(|e| failed(&parent, &req, child, e))
}

/// `spawn`, once the message is read; the child, once made, in `made`.
fn spawn_as(
    rcvid: u64,
    sender: i32,
    req: &Request,
    parent: &Process,
    thread: &sched::Thread,
    external: bool,
    made: &mut Option<ProcessRef>,
) -> Result<(), u32> {
    // Spawning and exec take PROCMGR_AID_SPAWN, a new application ID
    // PROCMGR_AID_CHILD_NEWAPP (EPERM; a spawn's child is made first, so
    // the pid is used up, as procnto).
    use crate::ker::ability;
    if !ability::allowed(parent, ability::SPAWN, None)
        || req.flags & NEWAPP != 0 && !ability::allowed(parent, ability::CHILD_NEWAPP, None)
    {
        return Err(EPERM);
    }
    let found = find(sender, req, external)?;
    let (elf, path) = (&*found.elf, found.path);
    let argv: Vec<&[u8]> = match &found.script_argv {
        Some(v) => v.iter().map(|a| a.as_slice()).collect(),
        None => req.argv.clone(),
    };
    let sched = scheduling(req, thread)?;
    let (cred, cwd) = identity(parent, req)?;
    // A group to join must exist in the parent's session (setpgid()'s
    // rule): EPERM, before anything is made, as procnto (measured with a
    // group 99999; review PROCMGR-23).
    if req.flags & SETPGROUP != 0
        && req.flags & SETSID == 0
        && req.pgroup != 0
        && !process::all()
            .iter()
            .any(|q| q.pgid() == req.pgroup && q.sid() == parent.sid() && !q.is_zombie())
    {
        return Err(EPERM);
    }
    let sigmask = if req.flags & SETSIGMASK != 0 {
        req.sigmask
    } else {
        thread.sigmask.load(Ordering::Relaxed)
    };
    if req.flags & EXEC != 0 {
        // File actions (spawn()'s fd map) are ignored, as procnto does:
        // the descriptors stay but the close-on-exec ones.
        // What can be known of the new image is checked first: a load that
        // can't succeed fails the exec here, the process as it was
        // (procnto, measured, review PROCMGR-7).
        loader::check(elf, req.stack_size).map_err(|e| {
            kprintln!("spawn: {path}: {e}");
            loader::errno_of(e)
        })?;
        // The point of no return: from here on a failure ends the process.
        end_other_threads(parent, thread);
        let (closed, deferred) = parent.close_on_exec();
        for coid in closed {
            resmgr::detached(Client { pid: sender, coid });
        }
        reset_signals(parent, req);
        *parent.cred.lock() = cred;
        if let Some(cwd) = cwd {
            *parent.cwd.lock() = cwd;
        }
        map::clear(parent);
        // The old image's process local storage went with its memory: the
        // new image starts with none, until its libc registers its own, as
        // procnto (measured, plsexectest). Its loader reads it first.
        parent.pls.store(0, Ordering::Relaxed);
        parent.execed.store(true, Ordering::Relaxed);
        parent.reset_ptids();
        let closes = (!deferred.is_empty()).then_some(deferred);
        if let Err(e) = start(parent, req, &argv, elf, &path, sched, sigmask, None, None, closes) {
            // Nothing is left to return to: the process ends as if killed
            // (not with exit status 0).
            kprintln!("[pid {sender}] exec {path} failed (errno {e}); the process ends");
            *parent.exit.lock() = Some((crate::ker::signal::CLD_KILLED, crate::ker::signal::SIGKILL as i32));
        }
        if req.flags & NOZOMBIE != 0 {
            procmgr::detach(parent);
        }
        // The calling thread ends as its MsgSend() returns.
        thread.exit_on_return.store(true, Ordering::Relaxed);
        let _ = call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]);
        return Ok(());
    }
    let mut opens = Vec::new();
    let fds = child_fds(parent, req, &mut opens)?;
    // Each open at the first descriptor it ends up at, copied to the others.
    for (&fd, src) in &fds {
        if let Source::External(i) = *src {
            let o = &mut opens[i];
            if o.fd < 0 {
                o.fd = fd;
            } else {
                o.copies.push(fd);
            }
        }
    }
    // Those that end up nowhere (closed, or closing on exec) aren't opened.
    opens.retain(|o| o.fd >= 0);
    let len = fds.keys().last().map_or(0, |&fd| fd as usize + 1);
    let mut fd_map = alloc::vec![-1; len];
    for (&fd, src) in &fds {
        if let Source::Parent(from) = src {
            fd_map[fd as usize] = *from;
        }
    }
    let (child, copied) = process::spawn_child(parent, Some(&fd_map)).ok_or(ENOMEM)?;
    child.execed.store(true, Ordering::Relaxed);
    *made = Some(child.clone());
    // The copies: procnto's open-file contexts are copied here, the other
    // servers learn of theirs from the child (`_IO_DUP`).
    let mut dups = Vec::new();
    for (from, to) in copied {
        let c = parent.connection(from).ok_or(EBADF)?;
        if c.channel.owner.pid == process::KERNEL_PID {
            let client = |pid, coid| Client { pid, coid };
            resmgr::inherit(client(sender, from), client(child.pid, to));
        } else {
            dups.push([to, sender, c.channel.chid, c.scoid, from]);
        }
    }
    for (fd, src) in fds {
        if let Source::Opened(opened) = src {
            child.connect_kernel(fd).ok_or(ENOMEM)?;
            resmgr::bind(
                Client {
                    pid: child.pid,
                    coid: fd,
                },
                opened,
            );
        }
    }
    if req.flags & SETSID != 0 {
        child.set_new_session();
    } else if req.flags & SETPGROUP != 0 {
        // 0: a new group, led by the child.
        child.set_pgid(if req.pgroup == 0 {
            child.pid
        } else {
            req.pgroup
        });
    }
    *child.signals.lock() = parent.signals.lock().clone();
    reset_signals(&child, req);
    *child.cred.lock() = cred;
    if let Some(cwd) = cwd {
        *child.cwd.lock() = cwd;
    }
    if req.flags & NOZOMBIE != 0 {
        // Spawned severed: never the parent's to wait for.
        child.nozombie.store(true, Ordering::Relaxed);
        child.ppid.store(process::KERNEL_PID, Ordering::Relaxed);
    }
    if req.flags & HOLD != 0 {
        // Created stopped, for a debugger: it runs on SIGCONT.
        crate::ker::debug::job_stop(&child, false, 23); // SIGSTOP
    }
    if dups.is_empty() && opens.is_empty() {
        start(&child, req, &argv, elf, &path, sched, sigmask, None, None, None)?;
        let _ = call(__KER_MSG_REPLYV, [rcvid, child.pid as u64, 0, 0, 0, 0]);
    } else {
        // The parent's answer is held until the child has told the other
        // servers about its descriptors (PULSE_FORK_DONE). Registered
        // before the child runs: started first, a child at a higher
        // priority on another CPU could send the pulse before the
        // registration, and `fork_done` would drop it (review item
        // PROCMGR-3). A start that fails takes the registration back.
        let opens = (!opens.is_empty()).then_some(opens);
        procmgr::reply_after_startup(child.pid, rcvid);
        if let Err(e) = start(&child, req, &argv, elf, &path, sched, sigmask, Some(dups), opens, None) {
            procmgr::forget_startup(child.pid);
            return Err(e);
        }
    }
    super::procevent::trigger(super::procevent::PROCESS_CREATE, child.pid as u32);
    Ok(())
}

// ANCHOR: spawn_handle
/// Handle a `_PROC_SPAWN` message from thread `tid` of process `sender`.
pub fn handle(rcvid: u64, sender: i32, tid: u32, subtype: u16, msg: &[u8]) {
    let r = match subtype {
        SPAWN_START => match spawn(rcvid, sender, tid, msg, false) {
            // Another server's program: on the worker thread, which
            // answers.
            Err(DEFER) => {
                let msg = msg.to_vec();
                super::worker::defer(move || {
                    if let Err(e) = spawn(rcvid, sender, tid, &msg, true) {
                        let _ = call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]);
                    }
                });
                Ok(())
            }
            r => r,
        },
        _ => {
            kprintln!("[pid {sender}] spawn: subtype {subtype} not implemented");
            Err(ENOSYS)
        }
    };
    if let Err(e) = r {
        let _ = call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]);
    }
}
// ANCHOR_END: spawn_handle
