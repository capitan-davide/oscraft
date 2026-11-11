//! The boot script (`[+script]` in the buildfile), compiled by mkifs into
//! `union script_cmd` records (`<sys/image.h>`) and run by procnto.
//!
//! External commands are looked up in the image filesystem (directly by
//! path, or along PATH), loaded into a new process and started; the script
//! waits for foreground commands to end. `[+session]` makes the command a
//! session leader. `display_msg` prints.
//!
//! A `[+session]` command's session gets the reopened device as its
//! controlling terminal.
//!
//! `waitfor name [secs]` waits until a server has attached `name` (polled
//! every `SCRIPT_CHECKS_MS`, 100 ms; mkifs's default is 5 s), printing
//! "Unable to access" if it doesn't come. `reopen name` waits the same
//! way, then opens `name` (read-write, as procnto, the script's process):
//! the commands after it get it as their standard input, output and
//! error. Like a forked child's, their descriptors are dups of procnto's,
//! announced to the server with `_IO_DUP` before they first run.
//!
//! Not yet: `procmgr_symlink` (skipped).

use alloc::string::String;
use alloc::vec::Vec;

use super::{imagefs, loader, pathmgr};
use crate::arch::kercall::call;
use crate::ker::number::*;
use crate::ker::process::{self, KERNEL, KERNEL_PID};
use crate::{clock, kprintln, sched, timeout};

const SCRIPT_TYPE_EXTERNAL: u8 = 0;
const SCRIPT_TYPE_WAITFOR: u8 = 1;
const SCRIPT_TYPE_REOPEN: u8 = 2;
const SCRIPT_TYPE_DISPLAY_MSG: u8 = 3;
const SCRIPT_FLAGS_SESSION: u8 = 0x02;
const SCRIPT_FLAGS_SCHED_SET: u8 = 0x04;
const SCRIPT_FLAGS_BACKGROUND: u8 = 0x20;
const DEFAULT_PRIO: u8 = 10;

/// Set when the boot script may start: at once without the kernel's
/// self-tests, else when they are done, so that their output does not
/// interleave with the script's.
pub static START: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// How long the boot script waits for the self-tests: a self-test that
/// hangs is reported, and doesn't keep the system from starting.
const SELFTEST_WAIT_NS: u64 = 30_000_000_000;

pub fn run(_: usize) {
    let give_up = crate::clock::now_ns() + SELFTEST_WAIT_NS;
    while !START.load(core::sync::atomic::Ordering::Acquire) {
        if crate::clock::now_ns() >= give_up {
            kprintln!("bootscript: the kernel's self-tests haven't finished after 30 s: starting anyway");
            break;
        }
        sched::yield_now();
    }
    let Some(script) = imagefs::get().and_then(|fs| fs.script()) else {
        kprintln!("bootscript: no boot script in the image");
        return;
    };
    // procnto's connection to the `reopen`ed device, if any.
    let mut console: Option<i32> = None;
    let mut b = script.data;
    while b.len() >= 4 {
        let size = u16::from_le_bytes([b[0], b[1]]) as usize;
        if size == 0 || size > b.len() {
            break;
        }
        let rec = &b[..size];
        match rec[2] {
            SCRIPT_TYPE_EXTERNAL => external(rec, console),
            SCRIPT_TYPE_WAITFOR => {
                waitfor(rec);
            }
            SCRIPT_TYPE_REOPEN => {
                if let Some(name) = waitfor(rec) {
                    // As procnto's, with O_LARGEFILE: the console's F_GETFL
                    // says so in every process the script starts (measured,
                    // issue #291).
                    match super::client::open(&name, super::client::READ_WRITE | super::client::O_LARGEFILE) {
                        Ok(coid) => {
                            if let Some(old) = console.replace(coid) {
                                let _ = call(__KER_CONNECT_DETACH, [old as u64, 0, 0, 0, 0, 0]);
                            }
                        }
                        Err(e) => kprintln!("bootscript: reopen {name}: error {e}"),
                    }
                }
            }
            SCRIPT_TYPE_DISPLAY_MSG => {
                kprintln!(
                    "{}",
                    String::from_utf8_lossy(strings(&rec[4..]).next().unwrap_or(b"")).trim_end()
                )
            }
            t => kprintln!("bootscript: skipping command of type {t} (not supported yet)"),
        }
        b = &b[size.next_multiple_of(4).min(b.len())..];
    }
    kprintln!("bootscript: done");
}

/// `waitfor` (and the wait of `reopen`): the name, once it exists.
fn waitfor(rec: &[u8]) -> Option<String> {
    const CHECK_NS: u64 = 100_000_000; // SCRIPT_CHECKS_MS
    let checks = u16::from_le_bytes([rec[4], rec[5]]);
    let name = String::from_utf8_lossy(strings(&rec[6..]).next().unwrap_or(b"")).into_owned();
    let exists = |n: &str| {
        pathmgr::lookup(n).is_some() || imagefs::get().is_some_and(|fs| fs.lookup(n).is_some())
    };
    for _ in 0..=checks {
        if exists(&name) {
            return Some(name);
        }
        timeout::sleep_until(clock::now_ns() + CHECK_NS);
    }
    kprintln!("Unable to access {name}");
    None
}

fn strings(b: &[u8]) -> impl Iterator<Item = &[u8]> {
    b.split(|&c| c == 0)
}

/// Value of `name` in a list of `NAME=value` strings.
fn env_value<'a>(env: impl Iterator<Item = &'a [u8]>, name: &str) -> Option<&'a [u8]> {
    env.filter_map(|e| e.strip_prefix(name.as_bytes())?.strip_prefix(b"="))
        .next()
}

fn external(rec: &[u8], console: Option<i32>) {
    let (flags, prio, argc, envc) = (rec[5], rec[9], rec[10] as usize, rec[11] as usize);
    let mut s = strings(&rec[12..]);
    let cmd = String::from_utf8_lossy(s.next().unwrap_or(b"")).into_owned();
    let argv: alloc::vec::Vec<&[u8]> = s.by_ref().take(argc).collect();
    let envp: alloc::vec::Vec<&[u8]> = s.take(envc).collect();
    let path =
        env_value(envp.iter().copied(), "PATH").map(|p| String::from_utf8_lossy(p).into_owned());
    let fs = imagefs::get().expect("imagefs");
    let found = if cmd.contains('/') {
        fs.lookup(&cmd).map(|f| (f, cmd.clone()))
    } else {
        path.as_deref()
            .unwrap_or("/proc/boot")
            .split(':')
            .find_map(|dir| {
                let full = alloc::format!("{dir}/{cmd}");
                fs.lookup(&full).map(|f| (f, full))
            })
    };
    let Some((file, full_path)) = found else {
        kprintln!("bootscript: {cmd}: not found");
        return;
    };
    let Some(p) = process::create() else {
        kprintln!("bootscript: {cmd}: cannot create a process");
        return;
    };
    // Standard input, output and error: the reopened device, as dups of
    // procnto's connection, or the console.
    let mut dups: Vec<[i32; 5]> = Vec::new();
    if let Some((coid, conn)) = console.and_then(|coid| Some((coid, KERNEL.connection(coid)?))) {
        for fd in 0..3 {
            drop(p.remove_connection(fd));
            let _ = p.add_connection(fd as u32, conn.copy_for(p.pid)); // 0..3 fit
            dups.push([fd, KERNEL_PID, conn.channel.chid, conn.scoid, coid]);
        }
    }
    if flags & SCRIPT_FLAGS_SESSION != 0 {
        p.set_new_session();
        // Its terminal: the reopened device.
        if let Some(coid) = console
            && let Err(e) = super::session::set_terminal(p.pid, coid)
        {
            kprintln!("bootscript: {cmd}: cannot set its terminal: error {e}");
        }
    }
    let start = match loader::load(&p, file.data, &full_path, &argv, &envp, loader::STACK_SIZE) {
        Ok(s) => s,
        Err(e) => {
            kprintln!("bootscript: {cmd}: {e}");
            super::spawn::abandon(&p);
            return;
        }
    };
    let prio = if flags & SCRIPT_FLAGS_SCHED_SET != 0 {
        prio
    } else {
        DEFAULT_PRIO
    };
    let frame = crate::arch::exception::TrapFrame::new_user(start.entry, start.sp, start.x0);
    let fp = crate::arch::context::FpState::new();
    let init = |t: &sched::Thread| {
        if !dups.is_empty() {
            *t.pending_dups.lock() = Some(dups);
        }
    };
    if sched::spawn_user_frame_init(&p, "main", prio, frame, start.tp, fp, 0, 1, init).is_none() {
        kprintln!("bootscript: {cmd}: cannot create its thread");
        super::spawn::abandon(&p);
        return;
    }
    kprintln!("bootscript: started {cmd} as process {}", p.pid);
    if flags & SCRIPT_FLAGS_BACKGROUND == 0 {
        // Done when it ends, or turns into a daemon (`procmgr_daemon()`).
        while p.thread_count() != 0 && !p.nozombie.load(core::sync::atomic::Ordering::Relaxed) {
            sched::yield_now();
        }
    }
}
