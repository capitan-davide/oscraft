//! The `asinfo` section: the physical address space, as a tree.
//!
//! Each entry names a range (`memory`, `ram`, `sysram`, `imagefs`, a
//! device's registers) and points to its parent by offset, so the array
//! is a flattened tree. The root is `memory`, the whole physical address
//! space. The kernel takes its free RAM from the `sysram` entries;
//! `posix_typed_mem_open()` finds memory by these names.

use super::strings::StringPool;
use crate::abi::syspage::{
    AS_ATTR_CONTINUED, AS_ATTR_KIDS, AS_NULL_OFF, AS_PRIORITY_DEFAULT, AsinfoEntry,
};
use core::mem::size_of;

/// Room for this many entries.
pub const MAX_ASINFO: usize = 64;

const ENTRY_SIZE: u16 = size_of::<AsinfoEntry>() as u16;

/// The asinfo array. Offsets (`owner`, and what [`Asinfo::add`] returns)
/// are byte offsets in the section, as the kernel expects.
#[derive(Clone, Debug)]
pub struct Asinfo {
    entries: [AsinfoEntry; MAX_ASINFO],
    len: usize,
}

impl Default for Asinfo {
    fn default() -> Self {
        Self::new()
    }
}

impl Asinfo {
    pub const fn new() -> Self {
        const EMPTY: AsinfoEntry = AsinfoEntry {
            start: 0,
            end: 0,
            owner: 0,
            name: 0,
            attr: 0,
            priority: 0,
        };
        Self {
            entries: [EMPTY; MAX_ASINFO],
            len: 0,
        }
    }

    pub fn entries(&self) -> &[AsinfoEntry] {
        &self.entries[..self.len]
    }

    pub fn entries_mut(&mut self) -> &mut [AsinfoEntry] {
        &mut self.entries[..self.len]
    }

    fn at(&self, off: u16) -> &AsinfoEntry {
        &self.entries[usize::from(off / ENTRY_SIZE)]
    }

    /// Add `[start, end]` (`end` inclusive) named `name`, under the entry
    /// at offset `owner` (or [`AS_NULL_OFF`] for a root). Returns the new
    /// entry's offset.
    pub fn add<const S: usize>(
        &mut self,
        strings: &mut StringPool<S>,
        start: u64,
        end: u64,
        attr: u16,
        name: &str,
        owner: u16,
    ) -> u16 {
        assert!(self.len < MAX_ASINFO, "asinfo full");
        let name = strings.add(name);
        self.entries[self.len] = AsinfoEntry {
            start,
            end,
            owner,
            name,
            attr,
            priority: AS_PRIORITY_DEFAULT,
        };
        if owner != AS_NULL_OFF {
            self.entries[usize::from(owner / ENTRY_SIZE)].attr |= AS_ATTR_KIDS;
        }
        self.len += 1;
        (self.len as u16 - 1) * ENTRY_SIZE
    }

    /// The first entry named `name`, if any.
    pub fn find<const S: usize>(&self, strings: &StringPool<S>, name: &str) -> Option<u16> {
        let name = strings.find(name)?;
        let i = self.entries().iter().position(|e| e.name == name)?;
        Some(i as u16 * ENTRY_SIZE)
    }

    /// Whether the entry at `off`, or one of its ancestors, is named
    /// `name` (a string offset).
    fn is_within(&self, mut off: u16, name: u16) -> bool {
        loop {
            let e = self.at(off);
            if e.name == name {
                return true;
            }
            if e.owner == AS_NULL_OFF {
                return false;
            }
            off = e.owner;
        }
    }

    // ANCHOR: add_containing
    /// Add `[start, end]` named `name`, in pieces: each piece goes under
    /// the most specific entry that contains it and is (or descends from)
    /// an entry named `container`. Parts outside any such entry are left
    /// out. All pieces but the last are marked [`AS_ATTR_CONTINUED`].
    /// Returns the offset of the first piece, if any.
    ///
    /// For example, free RAM below 1 GB on a Raspberry Pi goes under
    /// `below1G`, a child of `ram`, and the rest directly under `ram`.
    pub fn add_containing<const S: usize>(
        &mut self,
        strings: &mut StringPool<S>,
        start: u64,
        end: u64,
        attr: u16,
        name: &str,
        container: &str,
    ) -> Option<u16> {
        let container = strings.find(container)?;
        // Pieces of [start, end] with their owners. Entries come after
        // their parents in the array, so painting the matching entries in
        // array order leaves each piece with its deepest owner.
        let mut pieces = Pieces::new();
        for (i, e) in self.entries().iter().enumerate() {
            let off = i as u16 * ENTRY_SIZE;
            if e.end < start || e.start > end || !self.is_within(off, container) {
                continue;
            }
            pieces.paint(start.max(e.start), end.min(e.end), off);
        }
        let mut first = None;
        let n = pieces.len;
        for (k, &(s, e, owner)) in pieces.items[..n].iter().enumerate() {
            let a = if k + 1 < n {
                attr | AS_ATTR_CONTINUED
            } else {
                attr
            };
            let off = self.add(strings, s, e, a, name, owner);
            first.get_or_insert(off);
        }
        first
    }
    // ANCHOR_END: add_containing
}

/// Ranges `[start, end]` with an owner, sorted and disjoint.
struct Pieces {
    items: [(u64, u64, u16); 16],
    len: usize,
}

impl Pieces {
    fn new() -> Self {
        Self {
            items: [(0, 0, 0); 16],
            len: 0,
        }
    }

    /// Give `[s, e]` to `owner`, over whatever owned it before.
    fn paint(&mut self, s: u64, e: u64, owner: u16) {
        let mut out = [(0, 0, 0); 16];
        let mut n = 0;
        let mut push = |item: (u64, u64, u16)| {
            assert!(n < out.len(), "asinfo: range in too many pieces");
            out[n] = item;
            n += 1;
        };
        let mut placed = false;
        for &(ps, pe, po) in &self.items[..self.len] {
            if pe < s || ps > e {
                if ps > e && !placed {
                    push((s, e, owner));
                    placed = true;
                }
                push((ps, pe, po));
                continue;
            }
            if ps < s {
                push((ps, s - 1, po));
            }
            if !placed {
                push((s, e, owner));
                placed = true;
            }
            if pe > e {
                push((e + 1, pe, po));
            }
        }
        if !placed {
            push((s, e, owner));
        }
        self.items = out;
        self.len = n;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    #[test]
    fn tree_and_containing_pieces() {
        let mut s = StringPool::<256>::new();
        let mut a = Asinfo::new();
        let mem = a.add(&mut s, 0, 0xfff_ffff_ffff, 0, "memory", AS_NULL_OFF);
        let b4g = a.add(&mut s, 0, 0xffff_ffff, 0, "below4G", mem);
        let ram = a.add(&mut s, 0, 0x7fff_ffff, 0, "ram", b4g);
        a.add(&mut s, 0x3c00_0000, 0x3fff_ffff, 0, "vcram", ram);
        let b1g = a.add(&mut s, 0, 0x3bff_ffff, 0, "below1G", ram);
        assert_eq!((mem, b4g, ram, b1g), (0, 0x18, 0x30, 0x60));
        assert_ne!(a.entries()[0].attr & AS_ATTR_KIDS, 0);

        // Free RAM that spans below1G and the rest of ram.
        let first = a.add_containing(&mut s, 0x3a00_0000, 0x7fff_ffff, 0, "sysram", "ram");
        let sysram: Vec<_> = a.entries()[usize::from(first.unwrap() / 24)..]
            .iter()
            .map(|e| (e.start, e.end, e.owner, e.attr & AS_ATTR_CONTINUED != 0))
            .collect();
        assert_eq!(
            sysram,
            [
                (0x3a00_0000, 0x3bff_ffff, b1g, true),
                // vcram is under ram too, so it is a container; the board
                // takes it out of the free RAM, so it does not get sysram
                // in practice.
                (0x3c00_0000, 0x3fff_ffff, 0x48, true),
                (0x4000_0000, 0x7fff_ffff, ram, false),
            ]
        );
    }
}
