//! procnto's resource database manager (`<sys/rsrcdbmsg.h>`,
//! `<sys/rsrcdbmgr.h>`), received on the system-manager channel as
//! `RSRCDBMGR_RSRC_CMD` messages. So far only device numbers:
//! `rsrcdbmgr_devno_attach()` and `rsrcdbmgr_devno_detach()`, which
//! resource managers use to give their devices a `dev_t` (`st_rdev`).
//!
//! Contract, from the SDP headers and procnto's `rsrcdbmgr_handler`,
//! `rsrcmgr_handle_minor` and `_rsrc_minor`:
//! - The message is a 20-byte `struct _rsrc_cmd` (`type`, `subtype`, `pid`,
//!   `nbytes`, `count`, `index`). The subtype's top bits are the request
//!   (`RSRCDBMGR_REQ_*`), its low byte the resource type; device numbers
//!   are type 8.
//! - A device-number request follows with a `msg_rsrc_minor_request_t`
//!   (`major`, `minor`, `flags`, reserved) and, for an attach, the major's
//!   name (nul-terminated).
//! - Attach: the name selects a major (0..64); an unknown name gets the
//!   lowest free one. A minor of -1 means the lowest free minor; a minor
//!   in use is EINVAL, as is one above 1023. The reply is the 16-byte
//!   minor request with `major` and `minor` filled in.
//! - Detach: frees the (`major`, `minor`) the process attached (else
//!   EINVAL); a major with no minors left loses its name.
//! - A process's device numbers are freed when it ends.
//!
//! The other resources (memory, interrupts, I/O ports, DMA channels, PCI
//! memory: types 0 to 4, named "memory", "irq", "io", "dma",
//! "pcimemory"), as procnto (measured; message formats from libc's
//! `rsrcdbmgr_*()`):
//! - The database starts with memory free from 0 to the end of the CPU's
//!   physical address space, and each interrupt controller's vectors in
//!   the system page (`rsrcdbmgr_seed`); nothing is in use.
//! - Create (`msg_rsrc_alloc_t` items: start, end, flags) adds a range to
//!   the free list of its type; free ranges that overlap or touch merge.
//!   Destroy takes the free parts of a range away (in-use parts stay but
//!   can't be detached any more). Both always succeed.
//! - Attach (`msg_rsrc_request_t` items: length, align, start, end,
//!   flags) takes `length` from the free list: the lowest place that fits
//!   (the highest with `TOPDOWN`), within start..end with `RANGE`, aligned
//!   with `ALIGN` (an alignment of 0 or 1: none). `SHARE` may take a range
//!   another process holds shared, exactly. All items or none: EINVAL if
//!   one can't be had. The reply is the items with start and end filled in
//!   (and `RANGE` set). An item with `FLAG_NAME` names its type.
//! - Detach gives back (part of) a range the caller holds: to the free
//!   list, as a block of its own (released ranges don't merge). EINVAL if
//!   the caller doesn't hold it.
//! - These four take `PROCMGR_AID_RSRCDBMGR` (EPERM).
//! - Query by name (`RSRCDBMGR_REQ_QUERY_NAME` with the list, `RESERVED`
//!   for the count only): pid 0 the free blocks, -1 the blocks in use, a
//!   pid those it holds; from `index` (EINVAL past the end), at most
//!   `count`; the status is how many. An unknown name has none. The flags
//!   returned are the block's (`SHARE`), without the type.
//! - A process's ranges go back to the free list when it ends, but those
//!   attached with `NOREMOVE`.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::arch::kercall::call;
use crate::ker::errno::{EBADMSG, EINVAL, ENOSYS, EPERM};
use crate::ker::number::*;
use crate::sync::IrqSpinLock;

pub const RSRCDBMGR_RSRC_CMD: u16 = 0x00d0;
const REQ_MASK: u16 = 0xfe00;
const REQ_CREATE: u16 = 2 << 8;
const REQ_DESTROY: u16 = 4 << 8;
const REQ_ATTACH: u16 = 6 << 8;
const REQ_DETACH: u16 = 8 << 8;
/// `RSRCDBMGR_RESERVED`: libc's query by name without a list (the count).
const REQ_COUNT: u16 = 12 << 8;
const REQ_QUERY_NAME: u16 = 14 << 8;
// RSRCDBMGR_FLAG_* (<sys/rsrcdbmgr.h>)
const FLAG_ALIGN: u32 = 0x200;
const FLAG_RANGE: u32 = 0x400;
const FLAG_SHARE: u32 = 0x800;
const FLAG_TOPDOWN: u32 = 0x1000;
const FLAG_NAME: u32 = 0x2000;
const FLAG_NOREMOVE: u32 = 0x8000;
/// `RSRCDBMGR_TYPE_COUNT`, and the names of the types procnto knows.
const TYPES: usize = 8;
const TYPE_NAMES: [&str; 5] = ["memory", "irq", "io", "dma", "pcimemory"];
const ALLOC_SIZE: usize = 24;
const REQUEST_SIZE: usize = 48;
/// The resource type of device numbers.
const TYPE_DEVNO: u16 = 8;
const CMD_SIZE: usize = 20;
const MINOR_REQUEST_SIZE: usize = 16;
const MAJORS: usize = 64;
const MINOR_MAX: i32 = 0x3ff;

struct Major {
    name: String,
    /// Minor number -> the process that attached it.
    minors: BTreeMap<i32, i32>,
}

static DEVNOS: IrqSpinLock<Vec<Option<Major>>> = IrqSpinLock::new(Vec::new());

/// A range in use.
#[derive(Clone, Copy)]
struct Held {
    start: u64,
    end: u64,
    pid: i32,
    /// `SHARE`, `NOREMOVE`.
    flags: u32,
}

/// One type's resources: the ranges created (and not destroyed), the free
/// blocks, and the ranges in use (in the order attached).
#[derive(Clone, Default)]
struct Pool {
    created: Vec<(u64, u64)>,
    free: Vec<(u64, u64)>,
    held: Vec<Held>,
}

static POOLS: IrqSpinLock<Vec<Pool>> = IrqSpinLock::new(Vec::new());

fn u32_at(msg: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(msg[at..at + 4].try_into().unwrap())
}
fn u64_at(msg: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(msg[at..at + 8].try_into().unwrap())
}

/// `ranges` with `(s, e)` added, overlapping and touching ones merged.
fn merge(ranges: &mut Vec<(u64, u64)>, (s, e): (u64, u64)) {
    let (mut s, mut e) = (s, e);
    ranges.retain(|&(a, b)| {
        let joins = a <= e.saturating_add(1) && s <= b.saturating_add(1);
        if joins {
            (s, e) = (s.min(a), e.max(b));
        }
        !joins
    });
    insert(ranges, (s, e));
}

/// `ranges` with `(s, e)` added as a block of its own, in order.
fn insert(ranges: &mut Vec<(u64, u64)>, r: (u64, u64)) {
    let i = ranges.partition_point(|&(a, _)| a < r.0);
    ranges.insert(i, r);
}

/// `ranges` less `(s, e)`.
fn remove(ranges: &mut Vec<(u64, u64)>, (s, e): (u64, u64)) {
    let mut out = Vec::with_capacity(ranges.len() + 1);
    for &(a, b) in ranges.iter() {
        if b < s || a > e {
            out.push((a, b));
            continue;
        }
        if a < s {
            out.push((a, s - 1));
        }
        if b > e {
            out.push((e + 1, b));
        }
    }
    *ranges = out;
}

/// Record range `h` in use; one the same process holds next to it, with
/// the same flags, grows instead (procnto lists them as one).
fn hold(pool: &mut Pool, h: Held) {
    let next = |o: &&mut Held| {
        o.pid == h.pid && o.flags == h.flags && (o.end.checked_add(1) == Some(h.start) || h.end.checked_add(1) == Some(o.start))
    };
    match pool.held.iter_mut().find(|o| next(o)) {
        Some(o) => (o.start, o.end) = (o.start.min(h.start), o.end.max(h.end)),
        None => pool.held.push(h),
    }
}

fn covered(ranges: &[(u64, u64)], (s, e): (u64, u64)) -> bool {
    ranges.iter().any(|&(a, b)| a <= s && e <= b)
}

/// Seed the database (procnto's `rsrcdbmgr_seed`): memory from 0 to the
/// end of the physical address space, each interrupt controller's vectors.
pub fn init(sp: &crate::syspage::Syspage) {
    let mut pools = POOLS.lock();
    pools.resize_with(TYPES, Pool::default);
    let top = (1u64 << crate::arch::mmu::pa_bits()) - 1;
    pools[0].created.push((0, top));
    pools[0].free.push((0, top));
    for i in sp.intrinfo() {
        if i.num_vectors == 0 {
            continue;
        }
        let r = (i.vector_base as u64, (i.vector_base + i.num_vectors - 1) as u64);
        merge(&mut pools[1].created, r);
        merge(&mut pools[1].free, r);
    }
}

/// The type an item names: its flags' low byte, or with `FLAG_NAME` the
/// name (up to a '/').
fn type_of(flags: u32, name: &str) -> Result<usize, u32> {
    let t = if flags & FLAG_NAME != 0 {
        let top = name.split('/').next().unwrap_or("");
        TYPE_NAMES.iter().position(|&n| n == top).ok_or(EINVAL)?
    } else {
        (flags & 0xff) as usize
    };
    if t < TYPES { Ok(t) } else { Err(EINVAL) }
}

/// The names that follow `count` items of `size` bytes.
fn names(msg: &[u8], count: usize, size: usize) -> Vec<&str> {
    let mut rest = msg.get(CMD_SIZE + count * size..).unwrap_or(&[]);
    (0..count)
        .map(|_| {
            let n = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            let name = core::str::from_utf8(&rest[..n]).unwrap_or("");
            rest = rest.get(n + 1..).unwrap_or(&[]);
            name
        })
        .collect()
}

fn reply(rcvid: u64, status: u64, data: &[u8]) {
    let _ = call(
        __KER_MSG_REPLYV,
        [rcvid, status, data.as_ptr() as u64, (-(data.len() as i64)) as u64, 0, 0],
    );
}

/// Take `len` bytes from `pool`'s free blocks within `lo..=hi`, aligned to
/// `align`, the lowest place (or the highest with `top`).
fn find(pool: &Pool, len: u64, align: u64, lo: u64, hi: u64, top: bool) -> Option<u64> {
    let fits = |&(a, b): &(u64, u64)| -> Option<u64> {
        let (a, b) = (a.max(lo), b.min(hi));
        if a > b || b - a < len - 1 {
            return None;
        }
        if top {
            let at = (b - (len - 1)) / align * align;
            (at >= a).then_some(at)
        } else {
            let at = a.checked_next_multiple_of(align)?;
            (at <= b && b - at >= len - 1).then_some(at)
        }
    };
    if top { pool.free.iter().rev().find_map(fits) } else { pool.free.iter().find_map(fits) }
}

/// The resource requests (create, destroy, attach, detach, query) from
/// process `sender`.
fn resources(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    if msg.len() < CMD_SIZE {
        return Err(EBADMSG);
    }
    let req = subtype & REQ_MASK;
    let count = u32_at(msg, 12) as usize;
    if matches!(req, REQ_CREATE | REQ_DESTROY | REQ_ATTACH | REQ_DETACH)
        && !crate::ker::process::lookup(sender)
            .is_some_and(|p| crate::ker::ability::allowed(&p, crate::ker::ability::RSRCDBMGR, None))
    {
        return Err(EPERM);
    }
    let mut pools = POOLS.lock();
    match req {
        REQ_CREATE | REQ_DESTROY => {
            let items = msg.get(CMD_SIZE..CMD_SIZE + count * ALLOC_SIZE).ok_or(EBADMSG)?;
            let names = names(msg, count, ALLOC_SIZE);
            let mut new = pools.clone();
            for (i, it) in items.chunks(ALLOC_SIZE).enumerate() {
                let (s, e, flags) = (u64_at(it, 0), u64_at(it, 8), u32_at(it, 16));
                if s > e {
                    return Err(EINVAL);
                }
                let pool = &mut new[type_of(flags, names[i])?];
                if req == REQ_CREATE {
                    merge(&mut pool.created, (s, e));
                    // Its free part: less what is in use.
                    let mut add = alloc::vec![(s, e)];
                    for h in &pool.held {
                        remove(&mut add, (h.start, h.end));
                    }
                    for r in add {
                        merge(&mut pool.free, r);
                    }
                } else {
                    remove(&mut pool.created, (s, e));
                    remove(&mut pool.free, (s, e));
                }
            }
            *pools = new;
            drop(pools);
            reply(rcvid, 0, &[]);
            Ok(())
        }
        REQ_ATTACH | REQ_DETACH => {
            let items = msg.get(CMD_SIZE..CMD_SIZE + count * REQUEST_SIZE).ok_or(EBADMSG)?;
            let names = names(msg, count, REQUEST_SIZE);
            let mut new = pools.clone();
            let mut out = items.to_vec();
            for (i, it) in out.chunks_mut(REQUEST_SIZE).enumerate() {
                let (len, align, s, e, flags) =
                    (u64_at(it, 0), u64_at(it, 8), u64_at(it, 16), u64_at(it, 24), u32_at(it, 32));
                let pool = &mut new[type_of(flags, names[i])?];
                if req == REQ_DETACH {
                    let held = pool
                        .held
                        .iter()
                        .position(|h| h.pid == sender && h.start <= s && e <= h.end)
                        .ok_or(EINVAL)?;
                    if s > e || !covered(&pool.created, (s, e)) {
                        return Err(EINVAL);
                    }
                    let h = pool.held.remove(held);
                    if h.start < s {
                        pool.held.push(Held { end: s - 1, ..h });
                    }
                    if e < h.end {
                        pool.held.push(Held { start: e + 1, ..h });
                    }
                    insert(&mut pool.free, (s, e));
                    continue;
                }
                if len == 0 {
                    return Err(EINVAL);
                }
                let (lo, hi) = if flags & FLAG_RANGE != 0 { (s, e) } else { (0, u64::MAX) };
                let align = if flags & FLAG_ALIGN != 0 { align.max(1) } else { 1 };
                let keep = flags & (FLAG_SHARE | FLAG_NOREMOVE);
                // The range's last address: a length of 0, or one that
                // runs past the end of the arithmetic, is EINVAL.
                let last = lo.checked_add(len).and_then(|e| e.checked_sub(1)).filter(|&e| e >= lo).ok_or(EINVAL)?;
                // A range another process holds shared, exactly.
                let shared = (flags & (FLAG_SHARE | FLAG_RANGE) == FLAG_SHARE | FLAG_RANGE)
                    .then(|| pool.held.iter().filter(|h| h.start <= last && lo <= h.end).copied().collect::<Vec<_>>())
                    .filter(|o| !o.is_empty());
                let at = match shared {
                    Some(o) => {
                        if o.iter().any(|h| h.flags & FLAG_SHARE == 0 || h.pid == sender || h.start != lo || h.end != last) {
                            return Err(EINVAL);
                        }
                        lo
                    }
                    None => {
                        let at = find(pool, len, align, lo, hi, flags & FLAG_TOPDOWN != 0).ok_or(EINVAL)?;
                        remove(&mut pool.free, (at, at + len - 1));
                        at
                    }
                };
                hold(pool, Held { start: at, end: at + len - 1, pid: sender, flags: keep });
                it[16..24].copy_from_slice(&at.to_le_bytes());
                it[24..32].copy_from_slice(&(at + len - 1).to_le_bytes());
                it[32..36].copy_from_slice(&(flags | FLAG_RANGE).to_le_bytes());
            }
            *pools = new;
            drop(pools);
            reply(rcvid, 0, if req == REQ_ATTACH { &out } else { &[] });
            Ok(())
        }
        REQ_QUERY_NAME | REQ_COUNT => {
            let pid = i32_at(msg, 4);
            let nlen = u32_at(msg, 8) as usize;
            let name = msg.get(CMD_SIZE..CMD_SIZE + nlen).unwrap_or(&[]);
            let name = core::str::from_utf8(&name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())]).unwrap_or("");
            let blocks: Vec<(u64, u64, u32)> = match TYPE_NAMES.iter().position(|&n| !name.is_empty() && n == name) {
                None => Vec::new(),
                Some(t) => {
                    let pool = &pools[t];
                    match pid {
                        0 => pool.free.iter().map(|&(a, b)| (a, b, 0)).collect(),
                        _ => {
                            let mut v: Vec<(u64, u64, u32)> = pool
                                .held
                                .iter()
                                .filter(|h| pid == -1 || h.pid == pid)
                                .map(|h| (h.start, h.end, h.flags & FLAG_SHARE))
                                .collect();
                            v.sort_by_key(|b| b.0);
                            v
                        }
                    }
                }
            };
            drop(pools);
            if req == REQ_COUNT {
                reply(rcvid, blocks.len() as u64, &[]);
                return Ok(());
            }
            let index = u32_at(msg, 16) as usize;
            if index > blocks.len() {
                return Err(EINVAL);
            }
            let mut out = Vec::new();
            for &(a, b, f) in blocks.iter().skip(index).take(count) {
                out.extend_from_slice(&a.to_le_bytes());
                out.extend_from_slice(&b.to_le_bytes());
                out.extend_from_slice(&f.to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
            }
            reply(rcvid, (out.len() / ALLOC_SIZE) as u64, &out);
            Ok(())
        }
        _ => {
            crate::kprintln!("[pid {sender}] rsrcdbmgr: request {subtype:#06x} not implemented");
            Err(ENOSYS)
        }
    }
}

fn i32_at(msg: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(msg[at..at + 4].try_into().unwrap())
}

/// Handle an `RSRCDBMGR_RSRC_CMD` message from process `sender`.
pub fn handle(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) {
    let req = subtype & REQ_MASK;
    let result = if subtype & 0xff == TYPE_DEVNO && matches!(req, REQ_ATTACH | REQ_DETACH) {
        devno(rcvid, sender, req, msg)
    } else {
        resources(rcvid, sender, subtype, msg)
    };
    if let Err(e) = result {
        let _ = call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]);
    }
}

fn devno(rcvid: u64, sender: i32, req: u16, msg: &[u8]) -> Result<(), u32> {
    if msg.len() < CMD_SIZE + MINOR_REQUEST_SIZE {
        return Err(EBADMSG);
    }
    let pid = match i32_at(msg, 4) {
        0 => sender,
        pid => pid,
    };
    let data = &msg[CMD_SIZE..CMD_SIZE + MINOR_REQUEST_SIZE];
    let (major, minor) = (i32_at(data, 0), i32_at(data, 4));
    let mut table = DEVNOS.lock();
    if table.is_empty() {
        table.resize_with(MAJORS, || None);
    }
    match req {
        REQ_ATTACH => {
            let name = &msg[CMD_SIZE + MINOR_REQUEST_SIZE..];
            let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
            let name = core::str::from_utf8(name).map_err(|_| EINVAL)?;
            let major = match table
                .iter()
                .position(|m| m.as_ref().is_some_and(|m| m.name == name))
            {
                Some(i) => i,
                None => {
                    let i = table.iter().position(Option::is_none).ok_or(EINVAL)?;
                    table[i] = Some(Major {
                        name: String::from(name),
                        minors: BTreeMap::new(),
                    });
                    i
                }
            };
            let m = table[major].as_mut().unwrap();
            let minor = if minor < 0 {
                (0..=MINOR_MAX)
                    .find(|n| !m.minors.contains_key(n))
                    .ok_or(EINVAL)?
            } else if minor > MINOR_MAX || m.minors.contains_key(&minor) {
                return Err(EINVAL);
            } else {
                minor
            };
            m.minors.insert(minor, pid);
            drop(table);
            let mut reply = [0u8; MINOR_REQUEST_SIZE];
            reply.copy_from_slice(data);
            reply[0..4].copy_from_slice(&(major as i32).to_le_bytes());
            reply[4..8].copy_from_slice(&minor.to_le_bytes());
            let _ = call(
                __KER_MSG_REPLYV,
                [
                    rcvid,
                    0,
                    reply.as_ptr() as u64,
                    (-(MINOR_REQUEST_SIZE as i64)) as u64,
                    0,
                    0,
                ],
            );
            Ok(())
        }
        REQ_DETACH => {
            let m = usize::try_from(major)
                .ok()
                .and_then(|i| table.get_mut(i))
                .and_then(Option::as_mut)
                .ok_or(EINVAL)?;
            if m.minors.get(&minor) != Some(&pid) {
                return Err(EINVAL);
            }
            m.minors.remove(&minor);
            if m.minors.is_empty() {
                table[major as usize] = None;
            }
            drop(table);
            let _ = call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]);
            Ok(())
        }
        _ => Err(EINVAL),
    }
}

/// Free the device numbers and give back the resources (but those attached
/// with `NOREMOVE`) of a process that has ended.
pub fn process_exited(pid: i32) {
    for pool in POOLS.lock().iter_mut() {
        let mut gone = Vec::new();
        pool.held.retain(|h| {
            let go = h.pid == pid && h.flags & FLAG_NOREMOVE == 0;
            if go {
                gone.push((h.start, h.end));
            }
            !go
        });
        for r in gone {
            if covered(&pool.created, r) {
                insert(&mut pool.free, r);
            }
        }
    }
    for slot in DEVNOS.lock().iter_mut() {
        if let Some(m) = slot {
            m.minors.retain(|_, &mut p| p != pid);
            if m.minors.is_empty() {
                *slot = None;
            }
        }
    }
}
