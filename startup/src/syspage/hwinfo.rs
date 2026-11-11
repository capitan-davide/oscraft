//! The `hwinfo` section: devices, for drivers to find their defaults.
//!
//! hwinfo is a flattened tree of *items* (a group, a bus, a device) and
//! *tags* (a device's registers, interrupt, clock). Each entry starts with
//! a `hwi_prefix`: its size in 32-bit words and the offset of its name in
//! the strings section. An item also records its own size including its
//! tags, its item name, its owner (by offset) and how many children it
//! has. Item names start with a capital letter, tag names do not.
//!
//! The kernel itself needs only the root item, the group `hw`. Drivers
//! read the rest through the C library's `hwi_find_*()` functions.

use super::strings::{Bytes, StringPool};

/// `HWI_NULL_OFF`: no owner (a root item).
pub const HWI_NULL_OFF: u16 = 0xffff;

/// Room for the section, in bytes.
pub const HWINFO_SIZE: usize = 1024;

/// The hwinfo section being built.
#[derive(Clone, Debug, Default)]
pub struct Hwinfo {
    bytes: Bytes<HWINFO_SIZE>,
    /// Offset of the item that the next tags belong to.
    last_item: usize,
    finished: bool,
}

impl Hwinfo {
    pub const fn new() -> Self {
        Self {
            bytes: Bytes::new(),
            last_item: 0,
            finished: false,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    fn word(&mut self, off: usize) -> &mut [u8] {
        &mut self.bytes.as_mut_slice()[off..off + 2]
    }

    fn add_u16(&mut self, off: usize, add: u16) {
        let w = self.word(off);
        let v = u16::from_le_bytes([w[0], w[1]]) + add;
        w.copy_from_slice(&v.to_le_bytes());
    }

    /// Add a tag named `name` whose fields after the prefix are `body`
    /// (a multiple of 4 bytes), to the last item. `align` is the alignment
    /// the tag must start at (8 if it has a 64-bit field); a `pad` tag
    /// fills the gap.
    pub fn add_tag<const S: usize>(
        &mut self,
        strings: &mut StringPool<S>,
        name: &str,
        body: &[u8],
        align: usize,
    ) -> u16 {
        assert!(
            body.len().is_multiple_of(4),
            "hwinfo tag body not in 32-bit words"
        );
        if !self.bytes.len().is_multiple_of(align) {
            self.add_tag(strings, "pad", &[], 4);
        }
        let off = self.bytes.len();
        let words = (4 + body.len()) / 4;
        let name = strings.add(name);
        self.bytes.extend(&(words as u16).to_le_bytes());
        self.bytes.extend(&name.to_le_bytes());
        self.bytes.extend(body);
        if off != self.last_item {
            // The item grows by the tag: offset of itemsize is 4.
            self.add_u16(self.last_item + 4, words as u16);
        }
        off as u16
    }

    /// Add an item: `type_name` is its kind (`Group`, `Bus`, `Device`),
    /// `item_name` its name, `owner` its parent's offset (or
    /// [`HWI_NULL_OFF`]) and `extra` the fields after the common item
    /// fields (a device's `pnpid`). Returns its offset.
    pub fn add_item<const S: usize>(
        &mut self,
        strings: &mut StringPool<S>,
        type_name: &str,
        item_name: &str,
        owner: u16,
        extra: &[u8],
    ) -> u16 {
        let item_name = strings.add(item_name);
        if !self.bytes.len().is_multiple_of(4) {
            self.bytes.pad_to(4);
        }
        let off = self.bytes.len();
        self.last_item = off;
        let words = (12 + extra.len()) as u16 / 4;
        let mut body = [0u8; 32];
        // itemsize, itemname, owner, kids.
        body[..2].copy_from_slice(&words.to_le_bytes());
        body[2..4].copy_from_slice(&item_name.to_le_bytes());
        body[4..6].copy_from_slice(&owner.to_le_bytes());
        body[8..8 + extra.len()].copy_from_slice(extra);
        self.add_tag(strings, type_name, &body[..8 + extra.len()], 4);
        if owner != HWI_NULL_OFF {
            // The owner's kids count is at offset 10.
            self.add_u16(usize::from(owner) + 10, 1);
        }
        off as u16
    }

    /// The offset of the item named `item_name` (an item's own name, such
    /// as `unknown`, not its kind), if there is one.
    pub fn find_item<const S: usize>(
        &self,
        strings: &StringPool<S>,
        item_name: &str,
    ) -> Option<u16> {
        let name = strings.find(item_name)?;
        let b = self.bytes.as_slice();
        let u16_at = |off: usize| u16::from_le_bytes([b[off], b[off + 1]]);
        let mut off = 0;
        while off + 4 <= b.len() {
            let words = usize::from(u16_at(off));
            if words == 0 {
                break;
            }
            // Items' kinds start with a capital letter; their own name is
            // at offset 6.
            let kind = strings.get(u16_at(off + 2));
            if kind.starts_with(|c: char| c.is_ascii_uppercase()) && u16_at(off + 6) == name {
                return Some(off as u16);
            }
            off += words * 4;
        }
        None
    }

    /// The section as the kernel reads it: the entries, then a zero
    /// prefix that ends them, padded to 8 bytes. [`crate::Startup::boot`]
    /// calls it.
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.bytes.extend(&[0; 4]);
        self.bytes.pad_to(8);
    }
}

// ANCHOR: hwinfo_tags
/// The tags drivers read most, as `<hw/sysinfo.h>` lays them out. Each
/// goes to the last item added.
impl Hwinfo {
    /// `location`: a register block of `len` bytes at `base`, registers
    /// `1 << regshift` bytes apart, in the asinfo entry at `addrspace`.
    pub fn add_location<const S: usize>(
        &mut self,
        strings: &mut StringPool<S>,
        base: u64,
        len: u32,
        regshift: u16,
        addrspace: u16,
    ) -> u16 {
        let mut b = [0u8; 20];
        b[0..4].copy_from_slice(&len.to_le_bytes());
        b[4..12].copy_from_slice(&base.to_le_bytes());
        b[12..14].copy_from_slice(&regshift.to_le_bytes());
        b[14..16].copy_from_slice(&addrspace.to_le_bytes());
        self.add_tag(strings, "location", &b, 8)
    }

    /// `irq`: an interrupt vector.
    pub fn add_irq<const S: usize>(&mut self, strings: &mut StringPool<S>, vector: u32) -> u16 {
        self.add_tag(strings, "irq", &vector.to_le_bytes(), 4)
    }

    /// `inputclk`: an input clock of `clk` Hz, divided by `div`.
    pub fn add_inputclk<const S: usize>(
        &mut self,
        strings: &mut StringPool<S>,
        clk: u32,
        div: u32,
    ) -> u16 {
        let mut b = [0u8; 8];
        b[0..4].copy_from_slice(&clk.to_le_bytes());
        b[4..8].copy_from_slice(&div.to_le_bytes());
        self.add_tag(strings, "inputclk", &b, 4)
    }

    /// `nicaddr`: a network interface's MAC address.
    pub fn add_nicaddr<const S: usize>(
        &mut self,
        strings: &mut StringPool<S>,
        mac: &[u8; 6],
    ) -> u16 {
        let mut b = [0u8; 12];
        b[0..4].copy_from_slice(&6u32.to_le_bytes());
        b[4..10].copy_from_slice(mac);
        self.add_tag(strings, "nicaddr", &b, 4)
    }

    /// `regname`: a named value, such as a register's width.
    pub fn add_regname<const S: usize>(
        &mut self,
        strings: &mut StringPool<S>,
        name: &str,
        offset: u16,
    ) -> u16 {
        let name = strings.add(name);
        let mut b = [0u8; 4];
        b[0..2].copy_from_slice(&name.to_le_bytes());
        b[2..4].copy_from_slice(&offset.to_le_bytes());
        self.add_tag(strings, "regname", &b, 4)
    }
}
// ANCHOR_END: hwinfo_tags

/// Add the items every system has: the root group `hw` and, under it, the
/// bus `unknown` for devices on no particular bus. Returns the bus's
/// offset.
pub fn add_default<const S: usize>(hw: &mut Hwinfo, strings: &mut StringPool<S>) -> u16 {
    let root = hw.add_item(strings, "Group", "hw", HWI_NULL_OFF, &[]);
    hw.add_item(strings, "Bus", "unknown", root, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(b: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([b[off], b[off + 1]])
    }

    #[test]
    fn items_tags_and_alignment() {
        let mut s = StringPool::<256>::new();
        let mut hw = Hwinfo::new();
        let bus = add_default(&mut hw, &mut s);
        assert_eq!(bus, 12);
        let dev = hw.add_item(&mut s, "Device", "wdog", bus, &7u32.to_le_bytes());
        assert_eq!(dev, 24);
        // A location (len, base, regshift, addrspace) must start 8-aligned:
        // the device ends at 40, so it starts there with no pad.
        let mut loc = [0u8; 20];
        loc[0..4].copy_from_slice(&0x114u32.to_le_bytes());
        loc[8..16].copy_from_slice(&0xfe10_0000u64.to_le_bytes());
        assert_eq!(hw.add_tag(&mut s, "location", &loc, 8), 40);
        let b = hw.as_slice();
        // The root has 1 kid, the bus 1; the device grew by the tag.
        assert_eq!(u16_at(b, 10), 1);
        assert_eq!(u16_at(b, 12 + 10), 1);
        assert_eq!(u16_at(b, 24 + 4), 4 + 6);
        // A 12-byte tag (64..76) puts the next 8-aligned tag after a pad.
        hw.add_tag(&mut s, "inputclk", &[0; 8], 4);
        assert_eq!(hw.add_tag(&mut s, "location", &loc, 8), 80);
        assert_eq!(s.get(u16_at(hw.as_slice(), 76 + 2)), "pad");
        assert_eq!(hw.find_item(&s, "unknown"), Some(12));
        assert_eq!(hw.find_item(&s, "wdog"), Some(24));
        assert_eq!(hw.find_item(&s, "location"), None);
    }
}
