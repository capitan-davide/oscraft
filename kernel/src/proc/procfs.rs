//! The process filesystem at `/proc` (QNX `procfs_*` in procnto).
//!
//! Contract, from procnto's `procfs_open` and its `procfs_pid_files`
//! table: `/proc` lists the processes (and what is attached below it,
//! such as the image filesystem at `/proc/boot`); `/proc/<pid>` and
//! `/proc/self` (the opener) are directories of the files `cmdline`,
//! `exefile`, `vmstat`, `pmap`, `mappings`, `as` and `ctl`. The data files
//! are a snapshot taken at open; `stat()` gives their size (pidin sizes
//! its read of `cmdline` that way). `/proc`'s size is the free memory.
//!
//! As procnto's, the directories list no "." and "..", and a `stat()` of
//! a data file's path gives a size of 0 (an open one's `fstat()` the size
//! of its contents).
//!
//! Implemented: the directories, `/proc/config`, `cmdline` (the
//! arguments, each ending in a NUL), `exefile` (the program's path and a
//! NUL), OsCraft's own `personality` (`freebsd` or `qnx`, a line: which
//! ABI the process was branded with, see `ker::process`), and `as` and
//! `ctl` with these
//! `DCMD_PROC_*` devctls (`<sys/procfs.h>`; the command is the low byte),
//! which give what the introspection messages give (see `debug` and
//! `memmgr`):
//! - `DCMD_PROC_INFO`: the process's `debug_process_t`.
//! - `DCMD_PROC_MAPINFO`, `DCMD_PROC_PAGEDATA`: its regions as
//!   `procfs_mapinfo`s, as many as fit; `ret_val` is how many there are.
//! - `DCMD_PROC_MAPDEBUG` (in: an address) and `DCMD_PROC_MAPDEBUG_BASE`
//!   (the program): the `procfs_debuginfo` (lowest address, path) of the
//!   object mapped there.
//! - `DCMD_PROC_STATUS` (the first thread) and `DCMD_PROC_TIDSTATUS` (in:
//!   `tid` at offset 4): the `procfs_status` (`debug_thread_t`) of the
//!   first thread whose ID is at least that; ESRCH past the last.
//!
//! top and slay find processes this way. pidin's views also use
//! `DCMD_PROC_TIMERS` (`debug_timer64_t`s, or 32-bit ones by the command's
//! size), `DCMD_PROC_IRQS` (`debug_irq64_t`s) and `DCMD_PROC_CHANNELS`
//! (`debug_channel_t`s): as many as fit, `ret_val` how many there are
//! (procnto's `dcmd_TIMERS` & co.); and `DCMD_PROC_THREADCTL` with
//! `_NTO_TCTL_RUNMASK_GET_AND_SET_INHERIT`. A debugger (`pdebug`) also uses
//! (see `ker::debug`):
//! - `DCMD_PROC_STOP`, `DCMD_PROC_RUN` (`debug_run_t`: trace and fault
//!   sets, step, current thread, `_DEBUG_RUN_ARM`), `DCMD_PROC_EVENT` (the
//!   event to deliver at a stop), `DCMD_PROC_CURTHREAD`;
//! - `DCMD_PROC_GETGREG`/`SETGREG` (`AARCH64_CPU_REGISTERS`, 272 bytes,
//!   `ret_val` the size), `GETFPREG`/`SETFPREG`, `GETALTREG` (none on
//!   AArch64);
//! - `DCMD_PROC_BREAK` (software execution breakpoints),
//!   `DCMD_PROC_SET_FLAG`/`CLEAR_FLAG` (`_DEBUG_FLAG_KLC` & co.),
//!   `DCMD_PROC_THREADCTL` (thread names, run masks);
//! - reads and writes of `as`: the process's memory at the file position,
//!   code included, reads showing the instructions breakpoints replaced.
//!
//! A debugged process that has ended still answers `DCMD_PROC_STATUS`
//! (`_DEBUG_WHY_TERMINATED`, its exit status).
//!
//! `vmstat`, `pmap` and `mappings` are procnto's text reports of the
//! address space (`memmgr::vmstat`, `pmap`, `mappings`): counts in pages,
//! a line per region, a line per page. A read of `pmap` or `mappings`
//! stops at the end of the header, and in `mappings` at each region's
//! end (`read_end`). Not yet: hardware breakpoints and watchpoints.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::arch::context::{FpState, QNX_FPU_SIZE};
use crate::ker::debug as kdebug;
use crate::ker::errno::{EACCES, EBUSY, EFAULT, EINVAL, ENOENT, ENOSYS, ENOTSUP, ESRCH, ESRVRFAULT};
use crate::ker::event::{SIGEV_SIZE, SigEvent};
use crate::ker::process::{self, Process};
use crate::ker::umem;
use crate::vm::pmem::PAGE_SIZE;
use crate::sched;
use core::sync::atomic::Ordering;

use super::imagefs::{self, Entry};
use super::{debug, memmgr, pathmgr};

/// The files of `/proc/<pid>` implemented so far, in procnto's order,
/// their modes and their number in procnto's table (as, cmdline, exefile,
/// vmstat, pmap, mappings, ctl: the low bits of their inode). `as` and
/// `ctl` are opened for their devctls (and `as` to read and write the
/// memory).
const PID_FILES: [(&str, u32, u64); 8] = [
    ("as", 0o600, 1),
    ("cmdline", 0o644, 2),
    ("exefile", 0o600, 3),
    ("vmstat", 0o600, 4),
    ("pmap", 0o600, 5),
    ("mappings", 0o600, 6),
    ("ctl", 0o644, 7),
    // OsCraft's own: the process's personality (`ker::process`), a line.
    ("personality", 0o444, 8),
];
const AS: usize = 0;
const PMAP: usize = 4;
const MAPPINGS: usize = 5;
const CTL: usize = 6;
/// `/proc/config`'s inode, and procnto's for it (on its own device).
const CONFIG_INO: u32 = ROOT_INO + 3;
const CONFIG_INO64: u64 = 0xc;
/// `st_dev` of `/proc` itself, of the processes' directories and files,
/// and of `/proc/config` (measured).
const ROOT_DEV: u32 = 0x1;
const PID_DEV: u32 = 0x800;
const CONFIG_DEV: u32 = 0x409;
/// `DCMD_PROC_*` commands (`__PROC_SUBCMD_PROCFS + n`).
const PROC_INFO: u32 = 1;
const PROC_MAPINFO: u32 = 2;
const PROC_MAPDEBUG: u32 = 3;
const PROC_STOP: u32 = 5;
const PROC_STATUS: u32 = 7;
const PROC_CURTHREAD: u32 = 8;
const PROC_RUN: u32 = 9;
const PROC_GETGREG: u32 = 10;
const PROC_SETGREG: u32 = 11;
const PROC_GETFPREG: u32 = 12;
const PROC_SETFPREG: u32 = 13;
const PROC_BREAK: u32 = 14;
const PROC_EVENT: u32 = 17;
const PROC_SET_FLAG: u32 = 18;
const PROC_CLEAR_FLAG: u32 = 19;
const PROC_PAGEDATA: u32 = 20;
const PROC_GETALTREG: u32 = 21;
const PROC_TIMERS: u32 = 23;
const PROC_IRQS: u32 = 24;
const PROC_THREADCTL: u32 = 27;
const PROC_CHANNELS: u32 = 29;
/// `sizeof(debug_timer64_t)`, `sizeof(debug_timer32_t)`, `sizeof(debug_channel_t)`.
const DEBUG_TIMER64_SIZE: usize = 184;
const DEBUG_TIMER32_SIZE: usize = 96;
const DEBUG_CHANNEL_SIZE: usize = 48;
/// `_NTO_TCTL_RUNMASK_GET_AND_SET_INHERIT`, `_NTO_TCTL_NAME`
const TCTL_RUNMASK_GET_AND_SET_INHERIT: u32 = 10;
const TCTL_NAME: u32 = 11;
use super::debug::GREG_SIZE;
/// `_DCMD_PROC` (the class byte of a devctl command).
const DCMD_PROC: u32 = 0x08;
/// `__DIOT` | `__DIOF`: data goes to the server and comes back.
const DIOTF: u32 = 0xc000_0000;
const PROCFS_MAPINFO_SIZE: usize = 48;
/// Inodes: `/proc`, then per process its directory and files.
const ROOT_INO: u32 = 0x3000_0000;
/// Bounds on what `cmdline` reads of a process's arguments.
const ARGS_MAX: usize = 64 * 1024;
const ARGC_MAX: u64 = 4096;

/// What an open of the process filesystem refers to.
#[derive(Clone)]
pub enum Node {
    Root,
    Dir(i32),
    /// File `PID_FILES[index]` of a process, with its contents (taken at
    /// open, or at the first read).
    File {
        pid: i32,
        index: usize,
        data: Option<Vec<u8>>,
    },
    /// `/proc/config`: procnto's settings, one `name:value` line each.
    Config,
    /// `/proc/vm` or `/proc/ker` (`STATS_DIRS[index]`).
    StatsDir(usize),
    /// A file of those, `STATS_FILES[index]`: procnto's statistics.
    Stats(usize),
}

/// `/proc/vm` and `/proc/ker`, procnto's statistics.
const STATS_DIRS: [&str; 2] = ["vm", "ker"];
/// Their files, in their order: (directory, name, procnto's inode).
const STATS_FILES: [(usize, &str, u64); 3] = [(0, "stats", 0xa), (1, "intr", 0xe), (1, "stats", 0xd)];

fn is_debug_file(index: usize) -> bool {
    index == AS || index == CTL
}

fn pid_ino(pid: i32) -> u32 {
    ROOT_INO | ((pid as u32) << 4)
}

/// The NUL-terminated string at `addr` in `space`, at most `max` bytes.
fn read_cstr(space: umem::Space, addr: u64, max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    while out.len() < max {
        // Stop at page ends: the next page may not be mapped.
        let at = addr + out.len() as u64;
        let page = crate::vm::pmem::PAGE_SIZE;
        let n = buf.len().min((page - (at % page)) as usize);
        umem::read(space, at, &mut buf[..n]).ok()?;
        match buf[..n].iter().position(|&b| b == 0) {
            Some(i) => {
                out.extend_from_slice(&buf[..i]);
                return Some(out);
            }
            None => out.extend_from_slice(&buf[..n]),
        }
    }
    Some(out)
}

/// The arguments of `p`, from its initial stack (argc, then the argv
/// pointers), each followed by a NUL.
fn cmdline(p: &Process) -> Vec<u8> {
    let sp = p.initial_stack.load(Ordering::Relaxed);
    // SAFETY: `p` is borrowed for the whole call.
    let space = unsafe { umem::Space::user(p) };
    let mut word = [0u8; 8];
    let mut out = Vec::new();
    if sp == 0 || umem::read(space, sp, &mut word).is_err() {
        return out;
    }
    let argc = u64::from_le_bytes(word).min(ARGC_MAX);
    for i in 0..argc {
        if umem::read(space, sp + 8 * (1 + i), &mut word).is_err() {
            break;
        }
        let Some(arg) = read_cstr(space, u64::from_le_bytes(word), ARGS_MAX - out.len()) else {
            break;
        };
        out.extend_from_slice(&arg);
        out.push(0);
        if out.len() >= ARGS_MAX {
            break;
        }
    }
    out
}

/// Open `path` (below `/proc`) for process `opener`. A data file's
/// contents are taken now unless the open only serves a `stat()` of the
/// path (`snapshot` false), which so reads a size of 0 (as procnto's).
pub fn open(opener: i32, path: &str, write: bool, snapshot: bool) -> Result<Node, u32> {
    let mut parts = path.split('/').filter(|s| !s.is_empty());
    let Some(first) = parts.next() else {
        return if write { Err(EACCES) } else { Ok(Node::Root) };
    };
    if let Some(d) = STATS_DIRS.iter().position(|d| *d == first) {
        let node = match parts.next() {
            None => Node::StatsDir(d),
            Some(name) => Node::Stats(
                STATS_FILES
                    .iter()
                    .position(|f| f.0 == d && f.1 == name)
                    .ok_or(ENOENT)?,
            ),
        };
        return match (parts.next(), write) {
            (Some(_), _) => Err(ENOENT),
            (None, true) => Err(EACCES),
            (None, false) => Ok(node),
        };
    }
    if first == "config" {
        return match (parts.next(), write) {
            (Some(_), _) => Err(ENOENT),
            (None, true) => Err(EACCES),
            (None, false) => Ok(Node::Config),
        };
    }
    let pid = match first {
        "self" => opener,
        _ => first.parse::<i32>().map_err(|_| ENOENT)?,
    };
    let target = process::lookup(pid)
        .filter(|p| !p.is_zombie())
        .ok_or(ENOENT)?;
    let node = match parts.next() {
        None => Node::Dir(pid),
        Some(name) => {
            let index = PID_FILES.iter().position(|f| f.0 == name).ok_or(ENOENT)?;
            if write && !is_debug_file(index) {
                return Err(EACCES);
            }
            // The files are the process's own (its user and group, the
            // modes of procfs_umask 066 and procfs_ctl_umask 022): an
            // opener who isn't root and isn't that user gets EPERM for
            // what the mode doesn't give (measured, PROCMGR-6: uid 100
            // may read root's ctl, nothing else of root's; root may open
            // anyone's).
            may_open(opener, &target, PID_FILES[index].1, write)?;
            if parts.next().is_some() {
                return Err(ENOENT);
            }
            let data = snapshot.then(|| contents(pid, index));
            return Ok(Node::File { pid, index, data });
        }
    };
    if parts.next().is_some() {
        return Err(ENOENT);
    }
    if write {
        return Err(EACCES);
    }
    Ok(node)
}

/// Whether `opener` may open a file of `target` with `mode` for
/// reading (always) and, if `write`, writing: root may; the file's
/// owner by the owner bits, its group by the group bits, anyone else by
/// the other bits; EPERM otherwise (as procnto's `procfs_open`).
fn may_open(opener: i32, target: &process::Process, mode: u32, write: bool) -> Result<(), u32> {
    let opener = process::lookup(opener).ok_or(ESRCH)?;
    let (euid, egid) = {
        let c = opener.cred.lock();
        (c.euid, c.egid)
    };
    if euid == 0 {
        return Ok(());
    }
    let (uid, gid) = owner(target);
    let bits = if euid == uid {
        mode >> 6
    } else if egid == gid {
        mode >> 3
    } else {
        mode
    } & 0o7;
    let need = if write { 0o6 } else { 0o4 };
    if bits & need == need {
        Ok(())
    } else {
        Err(crate::ker::errno::EPERM)
    }
}

/// The user and group `/proc/<pid>` and its files belong to: the
/// process's effective ones (measured: a `setuid(100)` child's are
/// uid 100, gid 0).
fn owner(p: &process::Process) -> (u32, u32) {
    let c = p.cred.lock();
    (c.euid, c.egid)
}

/// Process `pid`'s file `name`, as `sysctl(KERN_PROC_ARGS)` reads it.
pub fn file_of(pid: i32, name: &str) -> Vec<u8> {
    let pid = if pid == -1 { 0 } else { pid };
    PID_FILES.iter().position(|f| f.0 == name).map_or(Vec::new(), |i| contents(pid, i))
}

/// What file `index` of process `pid` reads (`cmdline`: the arguments,
/// each ending in a NUL; `exefile`: the program's path and a NUL). procnto
/// itself is the image's kernel file, with no arguments but its name.
fn contents(pid: i32, index: usize) -> Vec<u8> {
    let Some(p) = process::lookup(pid).filter(|p| !p.is_zombie()) else {
        return Vec::new();
    };
    let kernel = || crate::bootargs::BootArgs::get().and_then(|b| b.argv().next()).unwrap_or("procnto");
    let mut out = match (PID_FILES[index].0, pid) {
        // All of its boot arguments, as procnto's (measured).
        ("cmdline", process::KERNEL_PID) => {
            let args = crate::bootargs::BootArgs::get().map(|b| b.argv().collect::<Vec<_>>());
            args.unwrap_or_default().join("\0").into_bytes()
        }
        ("cmdline", _) => return cmdline(&p),
        ("exefile", process::KERNEL_PID) => format!("/proc/boot/{}", kernel()).into_bytes(),
        ("exefile", _) => format!("/{}", p.name.lock().as_str()).into_bytes(),
        ("vmstat", _) => return memmgr::vmstat(&p),
        ("pmap", _) => return memmgr::pmap(&p),
        ("mappings", _) => return memmgr::mappings(&p),
        ("personality", _) => return p.personality().name().to_vec(),
        _ => return Vec::new(),
    };
    out.push(0);
    out
}

/// `/proc/config`, as procnto's (`procnto -v` options it was given and
/// the settings it works with; OsCraft's are procnto's defaults).
fn config() -> Vec<u8> {
    let clock_freq = format!("clock_freq:{}", crate::clock::tick_hz());
    let clock_prio = format!("clock_prio:{}", crate::clock::ist_priority());
    let lines = [
        "align_fault:off",
        "fd_close_timeout:30",
        "ker_verbose:1",
        "maxfds:1000",
        "nohalt:false",
        "pregrow_size:524288",
        "priv_prio.prio:64",
        "priv_prio.saturate:0",
        "procfs_ctl_umask:022",
        "procfs_umask:066",
        "shutdown_stack_len:128/0",
        "pathtrust:0",
        "small_msg_max:1200",
        "prp_load_profile:0",
        &clock_freq,
        &clock_prio,
        "ignore_unexpected_interrupts:true",
        "safety_state:on",
        "default_thr_stacksize:0",
        "proc_channel_width:1",
        "proc_thread_pool:16,1024,1024",
        "mm_cleanup_prio:10",
        "mmflags:0x205 (BACKWARDS_COMPAT,RANDOMIZE,V4)",
        "virtualization:disabled,EL1-Host",
        "thread_reaper_threshold:0",
        "thread_reaper_prio:10",
        "intrctlr:N/A",
    ];
    let mut out = Vec::new();
    for l in lines {
        out.extend_from_slice(l.as_bytes());
        out.push(b'\n');
    }
    out
}

/// The contents of a file node (taken now if the open didn't).
pub fn load(node: &mut Node) -> Vec<u8> {
    match node {
        Node::File { pid, index, data } => data.get_or_insert_with(|| contents(*pid, *index)).clone(),
        Node::Config => config(),
        Node::Stats(i) => stats(*i),
        _ => Vec::new(),
    }
}

/// Where a read of `node`'s `data` from `pos` stops, as procnto's: the
/// header of `pmap` and `mappings` comes alone, then the rest of `pmap`,
/// and `mappings` a region at a time (the lines of consecutive pages
/// with the same flags, protection and object).
pub fn read_end(node: &Node, data: &[u8], pos: usize) -> usize {
    let Node::File { index, .. } = node else {
        return data.len();
    };
    if *index != PMAP && *index != MAPPINGS {
        return data.len();
    }
    let line_end = |from: usize| data[from..].iter().position(|&b| b == b'\n').map_or(data.len(), |i| from + i + 1);
    let header = line_end(0);
    if pos < header {
        return header;
    }
    if *index == PMAP {
        return data.len();
    }
    // vaddr, and the fields that tell a region (all but the address,
    // the physical address and the offset).
    let key = |line: &[u8]| -> (u64, Vec<u8>) {
        let s = core::str::from_utf8(line).unwrap_or("");
        let f: Vec<&str> = s.trim_end().split(',').collect();
        let va = f.first().and_then(|v| u64::from_str_radix(v.trim_start_matches("0x"), 16).ok()).unwrap_or(0);
        let kept: Vec<&str> = f.iter().enumerate().filter(|(i, _)| ![0, 7, 13].contains(i)).map(|(_, v)| *v).collect();
        (va, kept.join(",").into_bytes())
    };
    let mut at = pos;
    let mut prev: Option<(u64, Vec<u8>)> = None;
    while at < data.len() {
        let end = line_end(at);
        let (va, k) = key(&data[at..end]);
        if let Some((pva, pk)) = &prev
            && (va != pva + crate::vm::pmem::PAGE_SIZE || &k != pk)
        {
            return at;
        }
        prev = Some((va, k));
        at = end;
    }
    data.len()
}

/// The path of a node.
pub fn path(node: &Node) -> String {
    match node {
        Node::Root => "/proc".into(),
        Node::Dir(pid) => format!("/proc/{pid}"),
        Node::File { pid, index, .. } => format!("/proc/{pid}/{}", PID_FILES[*index].0),
        Node::Config => "/proc/config".into(),
        Node::StatsDir(d) => format!("/proc/{}", STATS_DIRS[*d]),
        Node::Stats(i) => format!("/proc/{}/{}", STATS_DIRS[STATS_FILES[*i].0], STATS_FILES[*i].1),
    }
}

/// What `stat()` says of a node, and its size.
pub fn entry(node: &Node) -> (Entry, u64) {
    let dir = |ino| Entry {
        ino,
        mode: imagefs::S_IFDIR | 0o555,
        uid: 0,
        gid: 0,
        mtime: 0,
        data: &[],
    };
    match node {
        // As in QNX, /proc's size is the free memory (pidin info).
        Node::Root => (
            dir(ROOT_INO),
            (crate::vm::pmem::free_pages() as u64) * crate::vm::pmem::PAGE_SIZE,
        ),
        Node::Dir(pid) => {
            let (uid, gid) = process::lookup(*pid).map_or((0, 0), |p| owner(&p));
            (Entry { uid, gid, ..dir(pid_ino(*pid)) }, 0)
        }
        Node::File { pid, index, data } => {
            let (uid, gid) = process::lookup(*pid).map_or((0, 0), |p| owner(&p));
            (
                Entry {
                    ino: pid_ino(*pid) + 1 + *index as u32,
                    mode: imagefs::S_IFREG | PID_FILES[*index].1,
                    uid,
                    gid,
                    ..dir(0)
                },
                data.as_ref().map_or(0, |d| d.len() as u64),
            )
        }
        Node::Config => (
            Entry {
                ino: CONFIG_INO,
                mode: imagefs::S_IFREG | 0o400,
                ..dir(0)
            },
            config().len() as u64,
        ),
        Node::StatsDir(d) => (dir(ROOT_INO + 0x10 + *d as u32), 0),
        Node::Stats(i) => (
            Entry {
                ino: STATS_FILES[*i].2 as u32,
                mode: imagefs::S_IFREG | 0o444,
                ..dir(0)
            },
            stats(*i).len() as u64,
        ),
    }
}

/// `/proc/vm/stats`, `/proc/ker/intr`, `/proc/ker/stats`: procnto's
/// formats (measured) with OsCraft's own counts, 0 where it has nothing
/// of the kind. Pages: all RAM; free; what processes have mapped (the
/// image's pages mapped in place left out); the kernel's (the rest of
/// what is in use); the RAM it doesn't hand out (`ram` less `sysram`).
/// `ker/stats` has no `Slab:` lines: OsCraft has no slab caches.
fn stats(index: usize) -> Vec<u8> {
    use crate::vm::pmem::{free_pages, total_bytes, PAGE_SIZE};
    let size = memmgr::pages_size;
    let procs: Vec<process::ProcessRef> = process::all().into_iter().filter(|p| !p.is_zombie()).collect();
    match index {
        0 => {
            let sp = crate::syspage::get();
            let ram: u64 = sp.asinfo().filter(|a| sp.string(a.name) == "ram").map(|a| a.end + 1 - a.start).sum();
            let ram = ram.max(total_bytes()) / PAGE_SIZE;
            let managed = total_bytes() / PAGE_SIZE;
            let free = free_pages() as u64;
            let user: u64 = procs
                .iter()
                .map(|p| {
                    p.mem
                        .lock()
                        .regions()
                        .filter(|r| !r.image && r.phys.is_none() && !r.shared && r.prot != 0)
                        .map(|r| (r.end - r.start) / PAGE_SIZE)
                        .sum::<u64>()
                })
                .sum();
            let used = managed.saturating_sub(free);
            let user = user.min(used);
            format!(
                "vm_aspace={}\nvm_object=0\nvm_shmem={}\nvm_tymem=0\nvm_physobj=0\nvmem_avail={}\npmem_xdom_page=0\n\
                 pmem_xdom_contig=0\nfd_pend_close=0\nfd_free_objs=0\nkmem_vavail={}\npage_count={}\nanon_count={}\n\
                 cache_object={}\ncache_shmem={}\npages_allocated={}\npages_mapped={}\npages_free={}\npages_kernel={}\n\
                 pages_reserved={}\n",
                procs.len(),
                super::shmem::count(),
                size(free),
                size(0),
                size(ram),
                size(0),
                size(0),
                size(0),
                size(user),
                size(0),
                size(free),
                size(used - user),
                size(ram - managed),
            )
            .into_bytes()
        }
        1 => b"intrctlr:N/A\n".to_vec(),
        _ => {
            let threads: usize = procs.iter().map(|p| crate::sched::threads_of(p).len()).sum();
            format!("Processes:{}\nThreads,used:{},total:{}\nheap_pregrow,free:0\n", procs.len(), threads, threads)
                .into_bytes()
        }
    }
}

/// The inode of a process's directory (`file` 0) or file, as procnto's:
/// the pid in the upper 32 bits, the file's number below.
fn pid_ino64(pid: i32, file: u64) -> u64 {
    (pid as u32 as u64) << 32 | file
}

/// The `struct stat` of `node` for process `caller`, as procnto's (see the
/// module comment): `/proc` on device 1 with 512-byte blocks; a process's
/// directory and files on device 0x800 with their inodes from the pid, no
/// block size; a directory of size 1 and two links (four for the caller's
/// own, procnto's reference to it while it answers); `as` the size of
/// the process's readable mappings but the loader's read-only ones.
pub fn stat(node: &Node, caller: i32) -> super::resmgr::StatReply {
    let (e, size) = entry(node);
    let mut s = super::resmgr::stat(&e);
    let (ino, size, dev, nlink, blksize) = match node {
        Node::Root => (ROOT_INO as u64, size, ROOT_DEV, 2, 512),
        Node::Dir(pid) => (pid_ino64(*pid, 0), 1, PID_DEV, if *pid == caller { 4 } else { 2 }, 0),
        Node::File { pid, index, .. } => {
            let size = if *index == AS { mapped_bytes(*pid) } else { size };
            (pid_ino64(*pid, PID_FILES[*index].2), size, PID_DEV, 1, 0)
        }
        Node::Config => (CONFIG_INO64, size, CONFIG_DEV, 1, 0),
        // A directory of size 0; `/proc/vm/stats` of size 1 (measured).
        Node::StatsDir(d) => (ROOT_INO as u64 + 0x10 + *d as u64, 0, ROOT_DEV, 2, 0),
        Node::Stats(i) => {
            let size = if *i == 0 { 1 } else { stats(*i).len() as u64 };
            (STATS_FILES[*i].2, size, ROOT_DEV, 1, 0)
        }
    };
    s.st.st_ino = ino;
    s.st.st_size = size as i64;
    s.st.st_dev = u64::from(dev);
    s.st.st_nlink = u64::from(nlink as u32);
    s.st.st_blocks = if blksize == 0 { 0 } else { size.div_ceil(512) as i64 };
    s.st.st_blksize = blksize as i32;
    s
}

/// The size of process `pid`'s `as` file, as procnto's: the bytes of its
/// readable regions but the read-only ones the loader made (`MAP_ELF`:
/// code, read-only data, relocated data made read-only); guard pages
/// don't count either, other memory does, executable or not (measured).
fn mapped_bytes(pid: i32) -> u64 {
    use crate::vm::map::{MAP_ELF, PROT_READ, PROT_WRITE};
    process::lookup(pid).map_or(0, |p| {
        p.mem
            .lock()
            .regions()
            .filter(|r| r.prot & PROT_READ != 0)
            .filter(|r| r.flags & MAP_ELF == 0 || r.prot & PROT_WRITE != 0)
            .map(|r| r.end - r.start)
            .sum()
    })
}

/// What a seek to the end of `node` starts from: the end of user space
/// for `/proc` and `as`, 0 for `/proc/config`, the size (a directory's
/// 1) otherwise.
pub fn seek_end(node: &Node) -> u64 {
    match node {
        Node::Root => crate::arch::uspace::USER_END,
        Node::File { index, .. } if *index == AS => crate::arch::uspace::USER_END,
        Node::Config => 0,
        Node::Dir(_) => 1,
        _ => entry(node).1,
    }
}

/// The names in a directory node, with their inodes, as process `caller`
/// sees them.
pub fn names(node: &Node, caller: i32) -> Vec<(u64, String)> {
    // As procnto's: no "." and "..".
    let mut out: Vec<(u64, String)> = Vec::new();
    match node {
        Node::Root => {
            // The reader's own directory, the processes, what is attached
            // or stored below /proc (the image filesystem's /proc/boot),
            // procnto's files.
            out.push((pid_ino64(caller, 0), "self".into()));
            for p in process::all().into_iter().filter(|p| !p.is_zombie()) {
                out.push((pid_ino64(p.pid, 0), format!("{}", p.pid)));
            }
            // What is attached or stored below /proc (the image
            // filesystem's /proc/boot), then the processes.
            let mut below: Vec<String> = pathmgr::mount_names("proc");
            if let Some(fs) = imagefs::get() {
                for (_, name) in fs.children("proc") {
                    if !below.iter().any(|n| n == name) {
                        below.push(name.into());
                    }
                }
            }
            // The image filesystem's root has inode 1.
            out.extend(below.into_iter().map(|n| (1, n)));
            out.push((CONFIG_INO64, "config".into()));
            for (d, name) in STATS_DIRS.iter().enumerate() {
                out.push((ROOT_INO as u64 + 0x10 + d as u64, (*name).into()));
            }
        }
        Node::StatsDir(d) => {
            for f in STATS_FILES.iter().filter(|f| f.0 == *d) {
                out.push((f.2, f.1.into()));
            }
        }
        Node::Dir(pid) => {
            for f in PID_FILES.iter() {
                out.push((pid_ino64(*pid, f.2), f.0.into()));
            }
        }
        Node::File { .. } | Node::Config | Node::Stats(_) => {}
    }
    out
}

/// A devctl on `node` (see the module comment): `ret_val` and the data to
/// reply with, at most `room` bytes.
pub fn devctl(
    node: &Node,
    client: i32,
    dcmd: u32,
    data: &[u8],
    room: usize,
) -> Result<(i32, Vec<u8>), u32> {
    // Not a debugger's devctl: ENOSYS, as procnto's other files
    // (measured, review PATHMEM-17).
    let Node::File { pid, index, .. } = node else {
        return Err(ENOSYS);
    };
    if !is_debug_file(*index) || (dcmd >> 8) & 0xff != DCMD_PROC {
        return Err(ENOSYS);
    }
    let p = process::lookup(*pid).ok_or(ESRCH)?;
    if p.is_zombie() {
        // Ended: its debugger still learns how (`_DEBUG_WHY_TERMINATED`).
        if dcmd & 0xff != PROC_STATUS || p.debug.lock().debugger.is_none() {
            return Err(ESRCH);
        }
        return Ok((0, terminated_status(&p)));
    }
    let word = |at: usize| {
        data.get(at..at + 8)
            .map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap()))
    };
    let int = |at: usize| {
        data.get(at..at + 4)
            .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let cur = || {
        let st = p.debug.lock();
        kdebug::cur_thread(&p, &st).ok_or(ESRCH)
    };
    let (ret, mut out) = match dcmd & 0xff {
        PROC_INFO => (0, debug::process_info(&p).to_vec()),
        PROC_MAPINFO | PROC_PAGEDATA => {
            let mut out = memmgr::map_entries(&p, 0, u64::MAX);
            let count = out.len() / PROCFS_MAPINFO_SIZE;
            out.truncate(room / PROCFS_MAPINFO_SIZE * PROCFS_MAPINFO_SIZE);
            (count as i32, out)
        }
        PROC_MAPDEBUG => {
            let addr = if dcmd & DIOTF == DIOTF {
                word(0)
            } else {
                p.base_address.load(Ordering::Relaxed)
            };
            (0, memmgr::object_at(&p, addr)?)
        }
        PROC_STATUS => {
            let tid = match dcmd & DIOTF == DIOTF {
                true => int(4),
                false => cur()?.ptid,
            };
            (0, debug::thread_status(&p, tid).ok_or(ESRCH)?.to_vec())
        }
        PROC_STOP => {
            kdebug::request_stop(&p);
            (
                0,
                debug::thread_status(&p, cur()?.ptid).ok_or(ESRCH)?.to_vec(),
            )
        }
        PROC_CURTHREAD => {
            let tid = int(0);
            sched::thread_of(&p, tid).ok_or(ESRCH)?;
            p.debug.lock().cur_tid = tid;
            (0, Vec::new())
        }
        PROC_RUN => {
            let r = kdebug::Run {
                flags: int(0),
                tid: int(4),
                trace: word(8),
                fault: word(24),
                ip: word(40),
            };
            kdebug::run(&p, &r)?;
            (0, Vec::new())
        }
        PROC_GETGREG => (GREG_SIZE as i32, debug::gregs(&*cur()?)?),
        PROC_SETGREG => {
            let t = cur()?;
            let regs = data.get(..GREG_SIZE).ok_or(EINVAL)?;
            let reg = |i: usize| u64::from_le_bytes(regs[i * 8..i * 8 + 8].try_into().unwrap());
            // SAFETY: changed only while `t` is blocked (checked).
            unsafe {
                t.with_user_frame(|f| {
                    for (i, x) in f.x.iter_mut().enumerate() {
                        *x = reg(i);
                    }
                    f.sp_el0 = reg(31);
                    f.elr = reg(32);
                    // The flags only (NZCV); the mode stays EL0.
                    f.spsr = (f.spsr & !0xf000_0000) | (reg(33) & 0xf000_0000);
                })
            }
            .ok_or(EBUSY)?;
            (0, Vec::new())
        }
        PROC_GETFPREG => {
            let t = cur()?;
            // SAFETY: read only while `t` is blocked (checked).
            let (_, _, fp) = unsafe { t.user_state() }.ok_or(EBUSY)?;
            (QNX_FPU_SIZE as i32, fp.to_qnx().to_vec())
        }
        PROC_SETFPREG => {
            let t = cur()?;
            let b: &[u8; QNX_FPU_SIZE] =
                data.get(..QNX_FPU_SIZE).ok_or(EINVAL)?.try_into().unwrap();
            // SAFETY: changed only while `t` is blocked (checked).
            unsafe { t.with_user_fp(|fp| *fp = FpState::from_qnx(b)) }.ok_or(EBUSY)?;
            (0, Vec::new())
        }
        // AArch64 has no alternate registers (`AARCH64_ALT_REGISTERS`).
        PROC_GETALTREG => (8, alloc::vec![0u8; 8]),
        PROC_BREAK => {
            let kind = u16::from_le_bytes([
                data.first().copied().unwrap_or(0),
                data.get(1).copied().unwrap_or(0),
            ]);
            let size = i16::from_le_bytes([
                data.get(2).copied().unwrap_or(0),
                data.get(3).copied().unwrap_or(0),
            ]);
            let kind = kdebug::set_break(&p, kind, size, word(8))?;
            let mut out = data.get(..16).ok_or(EINVAL)?.to_vec();
            out[0..2].copy_from_slice(&kind.to_le_bytes());
            (0, out)
        }
        PROC_EVENT => {
            let b: &[u8; SIGEV_SIZE] = data.get(..SIGEV_SIZE).ok_or(EINVAL)?.try_into().unwrap();
            kdebug::set_event(&p, client, SigEvent::from_bytes(b));
            (0, Vec::new())
        }
        PROC_SET_FLAG | PROC_CLEAR_FLAG => {
            let flags = int(0) & kdebug::FLAG_MASK;
            let mut st = p.debug.lock();
            if dcmd & 0xff == PROC_SET_FLAG {
                st.flags |= flags;
            } else {
                st.flags &= !flags;
            }
            (0, Vec::new())
        }
        // As many as fit; `ret_val` counts them all.
        PROC_TIMERS => {
            // `debug_timer64_t` (or `debug_timer32_t`, by the command's
            // size): the id, then the `struct _timer_info` (in `info`).
            let size = match (dcmd >> 16) as usize & 0x3fff {
                DEBUG_TIMER32_SIZE => DEBUG_TIMER32_SIZE,
                _ => DEBUG_TIMER64_SIZE,
            };
            let timers = crate::ker::timer::timers_of(&p);
            let mut out = Vec::new();
            for (id, info) in &timers {
                let at = out.len();
                out.resize(at + size, 0);
                out[at..at + 4].copy_from_slice(&id.to_le_bytes());
                let i = at + size - crate::ker::timer::TIMER_INFO_SIZE;
                out[i..i + info.len()].copy_from_slice(info);
            }
            out.truncate(room / size * size);
            (timers.len() as i32, out)
        }
        PROC_IRQS => {
            let irqs = crate::ker::interrupt::attached_of(&p);
            let size = crate::ker::interrupt::DEBUG_IRQ_SIZE;
            let mut out: Vec<u8> = irqs.iter().flatten().copied().collect();
            out.truncate(room / size * size);
            (irqs.len() as i32, out)
        }
        PROC_CHANNELS => {
            // `debug_channel_t`: chid, type, flags, the queues' depths.
            let channels = p.channel_list();
            let mut out = Vec::new();
            for c in &channels {
                let mut d = [0u8; DEBUG_CHANNEL_SIZE];
                d[0..4].copy_from_slice(&c.chid.to_le_bytes());
                d[6..8].copy_from_slice(&(c.flags as u16).to_le_bytes());
                let s = c.state.lock();
                let depths = [
                    s.senders.len(),
                    s.pulses.len(),
                    s.receivers.len() + s.pulse_receivers.len(),
                    s.repliers.len(),
                ];
                for (i, n) in depths.into_iter().enumerate() {
                    d[8 + 4 * i..12 + 4 * i].copy_from_slice(&(n as u32).to_le_bytes());
                }
                out.extend_from_slice(&d);
            }
            out.truncate(room / DEBUG_CHANNEL_SIZE * DEBUG_CHANNEL_SIZE);
            (channels.len() as i32, out)
        }
        PROC_THREADCTL if int(4) == TCTL_RUNMASK_GET_AND_SET_INHERIT => {
            // { size, runmask[size], inherit_mask[size] }: the old masks
            // back (the inherit mask as every CPU); a nonzero runmask[0]
            // sets it.
            let t = sched::thread_of(&p, int(0)).ok_or(ESRCH)?;
            let words = int(8) as usize;
            if words < 1 || data.len() < 12 + 8 * words {
                return Err(EINVAL);
            }
            let old = crate::ker::thread::set_runmask(&t, int(12) as u64)?;
            let mut out = data[..12 + 8 * words].to_vec();
            out[12..12 + 8 * words].fill(0);
            out[12..16].copy_from_slice(&(old as u32).to_le_bytes());
            let all = crate::smp::online_mask() as u32;
            out[12 + 4 * words..16 + 4 * words].copy_from_slice(&all.to_le_bytes());
            (0, out)
        }
        PROC_THREADCTL => {
            // `procfs_threadctl` { tid, cmd, data }: thread names and
            // runmasks only.
            let t = sched::thread_of(&p, int(0)).ok_or(ESRCH)?;
            if int(4) != TCTL_NAME {
                return Err(ENOTSUP);
            }
            let buf_len = (int(12) as usize).min(data.len().saturating_sub(16));
            let mut name = t.qnx_name.lock().clone().into_bytes();
            name.push(0);
            name.truncate(buf_len);
            let mut out = data.get(..16).ok_or(EINVAL)?.to_vec();
            out.extend_from_slice(&name);
            (0, out)
        }
        // One it doesn't know: ENOSYS (measured, review PATHMEM-17).
        _ => return Err(ENOSYS),
    };
    out.truncate(room);
    Ok((ret, out))
}

/// The `procfs_status` of a debugged process that has ended: its pid,
/// `_DEBUG_FLAG_ISTOP`, `_DEBUG_WHY_TERMINATED` and the exit status.
fn terminated_status(p: &process::Process) -> Vec<u8> {
    let stop = p.debug.lock().stop;
    let mut d = alloc::vec![0u8; 280];
    d[0..4].copy_from_slice(&p.pid.to_le_bytes());
    d[8..12].copy_from_slice(&(kdebug::FLAG_STOPPED | kdebug::FLAG_ISTOP).to_le_bytes());
    d[12..14].copy_from_slice(&kdebug::WHY_TERMINATED.to_le_bytes());
    d[14..16].copy_from_slice(&stop.what.to_le_bytes());
    d
}

/// Read up to `buf.len()` bytes of process `pid`'s memory at `addr` (a
/// read of `/proc/<pid>/as`): as procnto, up to the first unmapped byte
/// (measured, review PATHMEM-14), and ESRVRFAULT if that is the first.
/// Returns how many bytes were read.
pub fn as_read(pid: i32, addr: u64, buf: &mut [u8]) -> Result<usize, u32> {
    let p = process::lookup(pid)
        .filter(|p| !p.is_zombie())
        .ok_or(ESRCH)?;
    // SAFETY: `p` is held meanwhile.
    let space = || unsafe { umem::Space::user(&p) };
    let mut n = buf.len();
    if umem::read(space(), addr, buf).is_err() {
        // A hole somewhere: the mapped pages before it, one at a time.
        n = 0;
        // (Any address may be asked: the sums wrap, and fault.)
        while n < buf.len() {
            let at = addr.wrapping_add(n as u64);
            let end = buf.len().min(n + (PAGE_SIZE - at % PAGE_SIZE) as usize);
            if umem::read(space(), at, &mut buf[n..end]).is_err() {
                break;
            }
            n = end;
        }
        if n == 0 {
            return Err(ESRVRFAULT);
        }
    }
    kdebug::hide_breaks(&p, addr, &mut buf[..n]);
    Ok(n)
}

/// Write `data` into process `pid`'s memory at `addr`, code included (a
/// write of `/proc/<pid>/as`).
pub fn as_write(pid: i32, addr: u64, data: &[u8]) -> Result<(), u32> {
    let p = process::lookup(pid)
        .filter(|p| !p.is_zombie())
        .ok_or(ESRCH)?;
    kdebug::write_mem(&p, addr, data, None).map_err(|e| if e == EFAULT { ESRVRFAULT } else { e })
}

/// Is `node` a process's `ctl`?
pub fn is_ctl_file(node: &Node) -> bool {
    matches!(node, Node::File { index, .. } if *index == CTL)
}

/// Is `node` a process's `as` or `ctl` (the debugger's files)?
pub fn is_ctl(node: &Node) -> Option<i32> {
    match node {
        Node::File { pid, index, .. } if is_debug_file(*index) => Some(*pid),
        _ => None,
    }
}

/// The `as` file (memory), as opposed to `ctl`.
pub fn is_as(node: &Node) -> Option<i32> {
    match node {
        Node::File { pid, index, .. } if *index == AS => Some(*pid),
        _ => None,
    }
}
