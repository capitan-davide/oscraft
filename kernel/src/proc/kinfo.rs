//! A FreeBSD `struct kinfo_proc` per process, FreeBSD's
//! `fill_kinfo_proc()`: `sysctl(KERN_PROC)`, for `ps` and `top` (`libkvm`).
//! The offsets are `<sys/user.h>`'s, measured with `offsetof` for arm64.

use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use crate::ker::process::{self, Process};
use crate::sched::{self, State, Thread};

const SIZE: usize = 1088;
const KI_STRUCTSIZE: usize = 0;
const KI_PID: usize = 72;
const KI_PPID: usize = 76;
const KI_PGID: usize = 80;
const KI_TPGID: usize = 84;
const KI_SID: usize = 88;
const KI_TSID: usize = 92;
const KI_UID: usize = 168;
const KI_RUID: usize = 172;
const KI_SVUID: usize = 176;
const KI_RGID: usize = 180;
const KI_SVGID: usize = 184;
const KI_SIZE: usize = 256;
const KI_RSSIZE: usize = 264;
const KI_RUNTIME: usize = 328;
const KI_START: usize = 336;
const KI_CHILDTIME: usize = 352;
const KI_FLAG: usize = 368;
const KI_KIFLAG: usize = 376;
const KI_STAT: usize = 388;
const KI_ONCPU_OLD: usize = 392;
const KI_LASTCPU_OLD: usize = 393;
const KI_TDNAME: usize = 394;
const KI_WMESG: usize = 411;
const KI_COMM: usize = 447;
const KI_TDEV: usize = 560;
const KI_ONCPU: usize = 568;
const KI_LASTCPU: usize = 572;
const KI_NUMTHREADS: usize = 596;
const KI_TID: usize = 600;
const KI_PRI_CLASS: usize = 604;
const KI_PRI_LEVEL: usize = 605;
const KI_RUSAGE: usize = 608;
const KI_RUSAGE_CH: usize = 752;
const KI_TDFLAGS: usize = 1080;
const COMM_MAX: usize = 19;
const TDNAME_MAX: usize = 16;
const WMESG_MAX: usize = 8;
/// `<sys/proc.h>`: the process states.
const SRUN: u8 = 2;
const SSLEEP: u8 = 3;
const SSTOP: u8 = 4;
const SZOMB: u8 = 5;
/// `PRI_TIMESHARE`.
const PRI_CLASS: u8 = 3;
const NODEV: u64 = u64::MAX;
/// `NOCPU`: not on a CPU now.
const NOCPU: i32 = -1;
/// `<sys/proc.h>`, `<sys/user.h>`: what `ps` shows as `S`, `L`, `s`.
const P_INMEM: u64 = 0x1000_0000;
const P_SYSTEM: u64 = 0x200;
const TDF_SINTR: u64 = 0x8;
const KI_SLEADER: u64 = 0x2;
const KI_CTTY: u64 = 0x1;
const P_CONTROLT: u64 = 0x2;
/// A session's terminal as a `dev_t`: its number, with this above it.
const TERMINAL_DEV: u64 = 0x5000_0000;
const PAGE: u64 = 4096;

fn put(r: &mut [u8], at: usize, b: &[u8]) {
    r[at..at + b.len()].copy_from_slice(b);
}

fn text(r: &mut [u8], at: usize, max: usize, s: &str) {
    let n = s.len().min(max);
    put(r, at, &s.as_bytes()[..n]);
}

/// What a thread's state is called, as `ki_wmesg`.
fn wait_name(s: State) -> &'static str {
    match s {
        State::Reply | State::Send => "msg",
        State::Receive => "recv",
        State::Nanosleep => "nanslp",
        State::Mutex => "mutex",
        State::Condvar => "cond",
        State::Sem => "sem",
        State::Sigsuspend | State::Sigwaitinfo => "sigwait",
        _ => "wait",
    }
}

/// A timeval from nanoseconds.
fn timeval(r: &mut [u8], at: usize, ns: u64) {
    put(r, at, &(ns / 1_000_000_000).to_le_bytes());
    put(r, at + 8, &(ns / 1000 % 1_000_000).to_le_bytes());
}

/// `fill_kinfo_proc()`, and with `only` `fill_kinfo_thread()`: the record
/// of that thread rather than of the whole process.
fn record(p: &Process, only: Option<&Thread>, now_ns: u64, wall_ns: u64, out: &mut Vec<u8>) {
    let mut r = [0u8; SIZE];
    put(&mut r, KI_STRUCTSIZE, &(SIZE as u32).to_le_bytes());
    put(&mut r, KI_PID, &p.pid.to_le_bytes());
    put(&mut r, KI_PPID, &p.ppid.load(Ordering::Relaxed).to_le_bytes());
    put(&mut r, KI_PGID, &p.pgid().to_le_bytes());
    put(&mut r, KI_SID, &p.sid().to_le_bytes());
    let terminal = super::session::terminal_of(p.sid());
    let dev = if terminal.is_some() { TERMINAL_DEV | p.sid() as u64 } else { NODEV };
    put(&mut r, KI_TDEV, &dev.to_le_bytes());
    if let Some((_, pgrp)) = &terminal {
        put(&mut r, KI_TPGID, &pgrp.to_le_bytes());
        put(&mut r, KI_TSID, &p.sid().to_le_bytes());
    }
    {
        let c = p.cred.lock();
        put(&mut r, KI_UID, &c.euid.to_le_bytes());
        put(&mut r, KI_RUID, &c.ruid.to_le_bytes());
        put(&mut r, KI_SVUID, &c.suid.to_le_bytes());
        put(&mut r, KI_RGID, &c.rgid.to_le_bytes());
        put(&mut r, KI_SVGID, &c.sgid.to_le_bytes());
    }
    let name = p.name.lock();
    text(&mut r, KI_COMM, COMM_MAX, name.rsplit('/').next().unwrap_or(""));
    drop(name);
    let threads = sched::threads_of(p);
    put(&mut r, KI_NUMTHREADS, &(threads.len() as u32).to_le_bytes());
    // A process's record speaks for a thread on a CPU if it has one, as
    // `fill_kinfo_proc()` picks one.
    let thread = only.or_else(|| {
        threads.iter().find(|t| t.state() == State::Running).or(threads.first()).map(|t| &**t)
    });
    if let Some(t) = thread {
        put(&mut r, KI_TID, &t.tid.to_le_bytes());
        r[KI_PRI_LEVEL] = 255 - t.priority();
        let last = t.last_cpu();
        let on = if t.state() == State::Running { i32::from(last) } else { NOCPU };
        put(&mut r, KI_LASTCPU, &i32::from(last).to_le_bytes());
        put(&mut r, KI_ONCPU, &on.to_le_bytes());
        r[KI_LASTCPU_OLD] = last;
        r[KI_ONCPU_OLD] = on as u8;
    }
    if let Some(t) = only {
        text(&mut r, KI_TDNAME, TDNAME_MAX, &t.qnx_name.lock());
    }
    r[KI_PRI_CLASS] = PRI_CLASS;
    let shown: Vec<State> = match only {
        Some(t) => alloc::vec![t.state()],
        None => threads.iter().map(|t| t.state()).collect(),
    };
    r[KI_STAT] = if p.is_zombie() {
        SZOMB
    } else if shown.iter().any(|s| matches!(s, State::Ready | State::Running)) {
        SRUN
    } else if shown.contains(&State::Stopped) {
        SSTOP
    } else {
        SSLEEP
    };
    if let Some(t) = thread.filter(|_| r[KI_STAT] == SSLEEP) {
        text(&mut r, KI_WMESG, WMESG_MAX, wait_name(t.state()));
    }
    let pages: u64 = if p.pid == process::KERNEL_PID || p.is_zombie() {
        0
    } else {
        p.mem.lock().regions().map(|m| (m.end - m.start) / PAGE).sum()
    };
    put(&mut r, KI_SIZE, &(pages * PAGE).to_le_bytes());
    // Mappings are fully backed here: all of it is resident.
    put(&mut r, KI_RSSIZE, &pages.to_le_bytes());
    // Signals end every wait here but a few, as `S` says.
    put(&mut r, KI_TDFLAGS, &TDF_SINTR.to_le_bytes());
    let system = if p.pid == process::KERNEL_PID { P_SYSTEM } else { 0 };
    let controlling = if terminal.is_some() { P_CONTROLT } else { 0 };
    put(&mut r, KI_FLAG, &(P_INMEM | system | controlling).to_le_bytes());
    let leader = if p.sid() == p.pid { KI_SLEADER } else { 0 };
    let tty = if terminal.is_some() { KI_CTTY } else { 0 };
    put(&mut r, KI_KIFLAG, &(leader | tty).to_le_bytes());
    let cpu = only.map_or_else(|| p.cpu_time(), Thread::cpu_time);
    put(&mut r, KI_RUNTIME, &(cpu / 1000).to_le_bytes());
    timeval(&mut r, KI_RUSAGE, cpu);
    // Only the total is kept: user and system time are not told apart.
    let children = p.child_cpu_time();
    timeval(&mut r, KI_CHILDTIME, children);
    timeval(&mut r, KI_RUSAGE_CH, children);
    let age = now_ns.saturating_sub(p.start_time.load(Ordering::Relaxed));
    let started = wall_ns.saturating_sub(age);
    timeval(&mut r, KI_START, started);
    out.extend_from_slice(&r);
}

/// `sysctl(KERN_PROC, what, arg)`: the records asked for, or a process's
/// arguments or path.
pub fn kern_proc(what: i32, arg: i32) -> Result<Vec<u8>, u32> {
    const PID: i32 = 1;
    const PGRP: i32 = 2;
    const SESSION: i32 = 3;
    const UID: i32 = 5;
    const RUID: i32 = 6;
    const ARGS: i32 = 7;
    const ALL_PROCESSES: i32 = 8;
    const PATHNAME: i32 = 12;
    const INC_THREAD: i32 = 0x10;
    // `KERN_PROC_ALL`, 0, has threads too (`sysctl_kern_proc()`).
    let threads = what == 0 || what & INC_THREAD != 0;
    let what = what & !INC_THREAD;
    if what == ARGS || what == PATHNAME {
        let file = if what == ARGS { "cmdline" } else { "exefile" };
        return Ok(super::procfs::file_of(arg, file));
    }
    if !matches!(what, 0 | PID | PGRP | SESSION | UID | RUID | ALL_PROCESSES) {
        return Err(crate::ker::errno::EOPNOTSUPP);
    }
    let (now, wall) = (crate::clock::now_ns(), crate::clock::tod_ns());
    let mut out = Vec::new();
    for p in process::all() {
        let keep = match what {
            PID => p.pid == arg,
            PGRP => p.pgid() == arg,
            SESSION => p.sid() == arg,
            UID => p.cred.lock().euid as i32 == arg,
            RUID => p.cred.lock().ruid as i32 == arg,
            _ => true,
        };
        if !keep {
            continue;
        }
        let each = if threads { sched::threads_of(&p) } else { Vec::new() };
        for t in &each {
            record(&p, Some(t), now, wall, &mut out);
        }
        if each.is_empty() {
            record(&p, None, now, wall, &mut out);
        }
    }
    if what == PID && out.is_empty() {
        return Err(crate::ker::errno::ESRCH);
    }
    Ok(out)
}

/// Each CPU's busy and idle milliseconds, from its idle thread's CPU time.
pub fn cpu_times() -> Vec<(u64, u64)> {
    let cpus = usize::from(crate::syspage::get().header().num_cpu);
    let up = crate::clock::now_ns() / 1_000_000;
    sched::threads_of(&process::KERNEL)
        .iter()
        .take(cpus)
        .map(|t| {
            let idle = t.cpu_time() / 1_000_000;
            (up.saturating_sub(idle), idle)
        })
        .collect()
}

/// The highest process ID in use: `kern.lastpid`, near enough for `top`.
pub fn last_pid() -> i32 {
    process::all().iter().map(|p| p.pid).max().unwrap_or(1)
}

/// `devname()`: a terminal's name, below `/dev`, for `ps`'s `TT`.
pub fn dev_name(dev: u64) -> Option<alloc::string::String> {
    let sid = dev.checked_sub(TERMINAL_DEV)? as i32;
    let (name, _) = super::session::terminal_of(sid)?;
    Some(name.strip_prefix("/dev/").unwrap_or(&name).into())
}

/// When the system started, seconds since the Epoch.
pub fn boot_seconds() -> u64 {
    crate::clock::tod_ns().saturating_sub(crate::clock::now_ns()) / 1_000_000_000
}
