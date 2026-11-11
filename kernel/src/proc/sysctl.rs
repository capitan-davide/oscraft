//! `sysctl()`, process 1's half: the message QNX's `libsocket` sends to
//! `io-sock` (`_IO_MSG`, subtype `IO_SOCK_SYSCTL`, `io_sock_sysctl_t`),
//! here on process 1's own channel, for the variables that are the
//! kernel's. libsys asks here first and `io-sock` for anything else, so
//! `ENOENT` is an answer. Each variable has FreeBSD's exact type: its
//! programs leave on a size that differs. No QNX program sends this.

use alloc::string::String;
use alloc::vec::Vec;

use crate::ker::errno::{EINVAL, ENOENT, EPERM};
use crate::ker::number::*;
use crate::arch::kercall::call;
use crate::vm::pmem;

use super::kinfo;

/// `struct _io_msg`'s `subtype`, and where `io_sock_sysctl_t`'s fields are.
pub const MSG_SYSCTL: u16 = 3;
const NAMELEN_AT: usize = 8;
const OLDLEN_AT: usize = 16;
const NEWLEN_AT: usize = 24;
pub const HEADER: usize = 32;
/// The reply's status when `old` was too small: `ENOMEM` with what fit.
const TRUNCATED: u64 = 1;

const CTL_SYSCTL: i32 = 0;
const SYSCTL_NAME2OID: i32 = 3;
const CTL_KERN: i32 = 1;
const KERN_PROC: i32 = 14;
const KERN_BOOTTIME: i32 = 21;
const CPUSTATES: usize = 5;
const CP_USER: usize = 0;
const CP_IDLE: usize = 4;
const PAGE: u64 = 4096;
/// Variables with no number of FreeBSD's fixed ones: numbers of ours, a block
/// from 0x7000 (libsys sends the rest to the network stack, whose numbers
/// made at run time count down from 0x7fffffff).
const OURS: i32 = 0x7000;

/// (name, what it is).
const NAMES: &[(&str, Var)] = &[
    ("kern.smp.active", Var::SmpActive),
    ("kern.smp.maxcpus", Var::Cpus),
    ("kern.lastpid", Var::LastPid),
    ("kern.cp_times", Var::CpTimes),
    ("kern.cp_time", Var::CpTime),
    ("kern.msgbuf", Var::Msgbuf),
    ("kern.boottime", Var::BootTime),
    ("kern.devname", Var::DevName),
    ("vm.loadavg", Var::LoadAverage),
    ("vm.dmmax", Var::Int(32)),
    // Devices are at the next level of the name: there are none.
    ("vm.swap_info", Var::Int(0)),
    ("vm.stats.vm.v_free_count", Var::FreePages),
    ("vm.stats.vm.v_wire_count", Var::WiredPages),
    // None of these exist here: no paging out, no swap, no buffer space.
    ("vm.stats.vm.v_active_count", Var::Int(0)),
    ("vm.stats.vm.v_inactive_count", Var::Int(0)),
    ("vm.stats.vm.v_laundry_count", Var::Int(0)),
    ("vm.stats.vm.v_swappgsin", Var::Long(0)),
    ("vm.stats.vm.v_swappgsout", Var::Long(0)),
    ("vfs.bufspace", Var::Long(0)),
];

#[derive(Clone, Copy)]
enum Var {
    Int(u32),
    Long(u64),
    SmpActive,
    Cpus,
    LastPid,
    CpTimes,
    CpTime,
    Msgbuf,
    BootTime,
    DevName,
    LoadAverage,
    FreePages,
    WiredPages,
}

fn busy_idle() -> Vec<(u64, u64)> {
    kinfo::cpu_times().into_iter().collect()
}

fn longs(values: &[u64]) -> Vec<u8> {
    values.iter().flat_map(|v: &u64| v.to_le_bytes()).collect()
}

fn value(v: Var) -> Vec<u8> {
    let total = pmem::total_bytes() / PAGE;
    let free = pmem::free_pages() as u64;
    match v {
        Var::DevName => Vec::new(),
        Var::Int(n) => n.to_le_bytes().to_vec(),
        Var::Long(n) => n.to_le_bytes().to_vec(),
        Var::SmpActive => u32::from(busy_idle().len() > 1).to_le_bytes().to_vec(),
        Var::Cpus => (busy_idle().len() as u32).to_le_bytes().to_vec(),
        Var::LastPid => kinfo::last_pid().to_le_bytes().to_vec(),
        Var::FreePages => (free as u32).to_le_bytes().to_vec(),
        // Mappings are fully backed: all that is in use is wired.
        Var::WiredPages => (total.saturating_sub(free) as u32).to_le_bytes().to_vec(),
        Var::BootTime => longs(&[kinfo::boot_seconds(), 0]),
        // `struct loadavg`: three averages, none kept yet, and `fscale`.
        Var::LoadAverage => longs(&[0, 0, 2048]),
        Var::CpTimes => {
            let mut t = alloc::vec![0u64; busy_idle().len() * CPUSTATES];
            for (c, (busy, idle)) in busy_idle().into_iter().enumerate() {
                t[c * CPUSTATES + CP_USER] = busy;
                t[c * CPUSTATES + CP_IDLE] = idle;
            }
            longs(&t)
        }
        Var::CpTime => {
            let mut t = [0u64; CPUSTATES];
            for (busy, idle) in busy_idle() {
                t[CP_USER] += busy;
                t[CP_IDLE] += idle;
            }
            longs(&t)
        }
        Var::Msgbuf => {
            let mut out = alloc::vec![0u8; crate::msgbuf::CAPACITY];
            let n = crate::msgbuf::snapshot(&mut out);
            out.truncate(n);
            out
        }
    }
}

/// What a request asks, as bytes.
fn answer(name: &[i32], new: &[u8]) -> Result<Vec<u8>, u32> {
    match *name {
        [CTL_SYSCTL, SYSCTL_NAME2OID] => {
            let text = String::from_utf8_lossy(new);
            let text = text.trim_end_matches('\0');
            let i = NAMES.iter().position(|n| n.0 == text).ok_or(ENOENT)?;
            Ok([CTL_KERN, OURS + i as i32].iter().flat_map(|v: &i32| v.to_le_bytes()).collect())
        }
        [CTL_KERN, KERN_BOOTTIME] => Ok(value(Var::BootTime)),
        [CTL_KERN, n] if n >= OURS => {
            let v = NAMES.get((n - OURS) as usize).ok_or(ENOENT)?.1;
            // The one variable with an input: the `dev_t`.
            if matches!(v, Var::DevName) {
                let dev = new.get(..8).ok_or(EINVAL)?;
                let dev = u64::from_le_bytes(dev.try_into().map_err(|_| EINVAL)?);
                let mut name = kinfo::dev_name(dev).ok_or(ENOENT)?.into_bytes();
                name.push(0);
                return Ok(name);
            }
            if new.is_empty() { Ok(value(v)) } else { Err(EPERM) }
        }
        [CTL_KERN, KERN_PROC, what, ref rest @ ..] => kinfo::kern_proc(what, rest.first().copied().unwrap_or(0)),
        _ => Err(ENOENT),
    }
}

/// A request; `room` is its reply buffer.
pub fn handle(rcvid: u64, msg: &[u8], room: usize) {
    let u32_at = |at: usize| u32::from_le_bytes(msg[at..at + 4].try_into().unwrap()) as usize;
    let u64_at = |at: usize| u64::from_le_bytes(msg[at..at + 8].try_into().unwrap()) as usize;
    let fail = |e: u32| {
        let _ = call(__KER_MSG_ERROR, [rcvid, e as usize as u64, 0, 0, 0, 0]);
    };
    let (namelen, oldlen, newlen) = (u32_at(NAMELEN_AT), u64_at(OLDLEN_AT), u64_at(NEWLEN_AT));
    let names_end = HEADER + namelen * 4;
    if namelen > 24 || msg.len() < names_end || msg.len() < names_end + newlen {
        return fail(EINVAL);
    }
    let name: Vec<i32> =
        msg[HEADER..names_end].chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
    match answer(&name, &msg[names_end..names_end + newlen]) {
        Err(e) => fail(e),
        Ok(data) => {
            let want = oldlen.min(room.saturating_sub(HEADER));
            let n = if oldlen == 0 { 0 } else { data.len().min(want) };
            let mut out = alloc::vec![0u8; HEADER + n];
            out[..HEADER].copy_from_slice(&msg[..HEADER]);
            // What the value's size is, with no buffer; what came, with one.
            let told = if oldlen == 0 { data.len() } else { n };
            out[OLDLEN_AT..OLDLEN_AT + 8].copy_from_slice(&(told as u64).to_le_bytes());
            out[HEADER..].copy_from_slice(&data[..n]);
            let status = if oldlen != 0 && n < data.len() { TRUNCATED } else { 0 };
            super::resmgr::reply(rcvid, status as u32, &out);
        }
    }
}
