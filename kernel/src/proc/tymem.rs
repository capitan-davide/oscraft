//! Typed memory (POSIX `posix_typed_mem_open()`; QNX `memmgr/tymem.c`): a
//! named pool of physical memory, which drivers allocate DMA buffers from
//! (a driver in a bsdkern server asks for `below4G`: `user/bsdkern/src/dma.rs`).
//!
//! Contract, from libc (`posix_typed_mem_open`, `mem_offset`) and the
//! QNX 8.0 library reference:
//! - `posix_typed_mem_open(name, oflag, tflag)` is an `_IO_CONNECT` of
//!   `name` with file type `_FTYPE_TYMEM` and `tflag` (4 bytes) as the
//!   extra data (`_IO_CONNECT_EXTRA_TYMEM`). The names are those of the
//!   system page's address-space entries (`sysram`, `below4G`,
//!   `below1G`...); `a&b` is the memory that is in both. ENOENT for a name
//!   that isn't there.
//! - `mmap()` of the object with `POSIX_TYPED_MEM_ALLOCATE_CONTIG` (or
//!   `_ALLOCATE`) allocates RAM from the pool (here always contiguous); the
//!   offset is ignored. Without them it maps the pool's physical memory at
//!   that offset.
//! - `mem_offset()` (`_MEM_OFFSET`, see `memmgr`) gives a mapping's physical
//!   address, `posix_typed_mem_get_info()` (`_MEM_INFO`) the most the pool
//!   can still allocate.

use alloc::string::String;

use super::pathmgr::Connect;
use crate::ker::errno::ENOENT;
use crate::syspage;

// `posix_typed_mem_open()` flags (<sys/mman.h>)
pub const ALLOCATE: u32 = 0x1;
pub const ALLOCATE_CONTIG: u32 = 0x2;

/// An open typed memory object.
#[derive(Clone)]
pub struct Open {
    pub name: String,
    /// The pool's physical bounds (inclusive).
    pub lo: u64,
    pub hi: u64,
    /// `POSIX_TYPED_MEM_*`.
    pub tflag: u32,
    pub ioflag: u32,
}

/// The bounds of the address-space entries named `name` (the smallest
/// range holding them all). `name` may be a path of names, `memory/below4G`:
/// then only the entries of the last name whose ancestors, in the system
/// page's tree, include the earlier names in order count (procnto opens
/// `/memory/below4G`, measured: review PATHMEM-26).
fn bounds(name: &str) -> Option<(u64, u64)> {
    let sp = syspage::get();
    let mut parts: alloc::vec::Vec<&str> = name.split('/').filter(|s| !s.is_empty()).collect();
    let last = parts.pop()?;
    let entries: alloc::vec::Vec<&syspage::AsinfoEntry> = sp.asinfo().collect();
    let under = |a: &syspage::AsinfoEntry| {
        // Walk up the owners, matching the earlier names from the nearest.
        let mut want = parts.iter().rev().peekable();
        let mut owner = a.owner;
        while want.peek().is_some() {
            let Some(e) = sp.asinfo_index(owner).and_then(|i| entries.get(i)) else {
                break;
            };
            if Some(&sp.string(e.name)) == want.peek().map(|s| **s).as_ref() {
                want.next();
            }
            owner = e.owner;
        }
        want.peek().is_none()
    };
    let mut found: Option<(u64, u64)> = None;
    for a in sp.asinfo().filter(|a| sp.string(a.name) == last && under(a)) {
        found = Some(match found {
            Some((lo, hi)) => (lo.min(a.start), hi.max(a.end)),
            None => (a.start, a.end),
        });
    }
    found
}

/// Open `c.path` (the pool's name).
pub fn open(c: &Connect) -> Result<Open, u32> {
    let name = c.path.trim_matches('/');
    let mut range: Option<(u64, u64)> = None;
    for part in name.split('&') {
        let (lo, hi) = bounds(part).ok_or(ENOENT)?;
        range = Some(match range {
            Some((l, h)) => (l.max(lo), h.min(hi)),
            None => (lo, hi),
        });
    }
    let (lo, hi) = range.filter(|(lo, hi)| lo <= hi).ok_or(ENOENT)?;
    let tflag = c
        .combined
        .get(..4)
        .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
    Ok(Open {
        name: name.into(),
        lo,
        hi,
        tflag,
        ioflag: c.ioflag,
    })
}
