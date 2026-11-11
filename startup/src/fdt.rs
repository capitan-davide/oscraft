//! A reader for flattened device trees (FDT, the `.dtb` format).
//!
//! A device tree describes the hardware as a tree of nodes with
//! properties. In the blob, a header leads to the *structure block*, a
//! sequence of big-endian 32-bit tokens: `BEGIN_NODE` (followed by the
//! node's name), `PROP` (length, name offset in the *strings block*, value),
//! `END_NODE`, `NOP`, `END`. Nodes nest by their BEGIN/END pairs.
//!
//! Startup needs very little of it, so this reader only walks the nodes
//! and finds properties; it never allocates.

const MAGIC: u32 = 0xd00d_feed;
const BEGIN_NODE: u32 = 1;
const END_NODE: u32 = 2;
const PROP: u32 = 3;
const NOP: u32 = 4;
const END: u32 = 9;

/// A device tree blob.
#[derive(Clone, Copy)]
pub struct Fdt<'a> {
    blob: &'a [u8],
    structs: usize,
    strings: usize,
}

/// A node, by the offset of its `BEGIN_NODE` token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node(usize);

fn be32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

impl<'a> Fdt<'a> {
    /// Check the header of `blob` and read where its blocks are.
    pub fn new(blob: &'a [u8]) -> Option<Self> {
        if be32(blob, 0)? != MAGIC {
            return None;
        }
        let size = be32(blob, 4)? as usize;
        let blob = blob.get(..size)?;
        Some(Self {
            blob,
            structs: be32(blob, 8)? as usize,
            strings: be32(blob, 12)? as usize,
        })
    }

    /// The blob at physical address `addr`, if there is one.
    ///
    /// # Safety
    ///
    /// `addr` must be readable, with the MMU off, for the size the header
    /// claims; the blob must not change while the result is used.
    pub unsafe fn at(addr: u64) -> Option<Fdt<'static>> {
        // SAFETY: as the caller guarantees, for the 8-byte header first.
        let hdr = unsafe { core::slice::from_raw_parts(addr as *const u8, 8) };
        let size = be32(hdr, 4)? as usize;
        // SAFETY: and for the whole blob.
        Fdt::new(unsafe { core::slice::from_raw_parts(addr as *const u8, size) })
    }

    fn cstr(&self, off: usize) -> Option<&'a [u8]> {
        let rest = self.blob.get(off..)?;
        Some(&rest[..rest.iter().position(|&b| b == 0)?])
    }

    /// Every node with its depth (the root is 0), in order.
    pub fn nodes(&self) -> impl Iterator<Item = (Node, usize)> + '_ {
        let mut off = self.structs;
        let mut depth = 0usize;
        core::iter::from_fn(move || {
            loop {
                let token = be32(self.blob, off)?;
                match token {
                    BEGIN_NODE => {
                        let node = Node(off);
                        let name = self.cstr(off + 4)?;
                        off = (off + 4 + name.len() + 1).next_multiple_of(4);
                        depth += 1;
                        return Some((node, depth - 1));
                    }
                    END_NODE => {
                        depth = depth.checked_sub(1)?;
                        off += 4;
                    }
                    PROP => {
                        let len = be32(self.blob, off + 4)? as usize;
                        off = (off + 12 + len).next_multiple_of(4);
                    }
                    NOP => off += 4,
                    END => return None,
                    _ => return None,
                }
            }
        })
    }

    /// The value of `node`'s property `name`.
    pub fn property(&self, node: Node, name: &str) -> Option<&'a [u8]> {
        let name_len = self.cstr(node.0 + 4)?.len();
        let mut off = (node.0 + 4 + name_len + 1).next_multiple_of(4);
        // Properties come first in a node, before its children.
        while let Some(token) = be32(self.blob, off) {
            match token {
                PROP => {
                    let len = be32(self.blob, off + 4)? as usize;
                    let nameoff = be32(self.blob, off + 8)? as usize;
                    if self.cstr(self.strings + nameoff)? == name.as_bytes() {
                        return self.blob.get(off + 12..off + 12 + len);
                    }
                    off = (off + 12 + len).next_multiple_of(4);
                }
                NOP => off += 4,
                _ => return None,
            }
        }
        None
    }

    /// The first node whose `compatible` list has `compatible`.
    pub fn find_compatible(&self, compatible: &str) -> Option<Node> {
        self.nodes().map(|n| n.0).find(|&n| {
            self.property(n, "compatible")
                .is_some_and(|v| v.split(|&b| b == 0).any(|c| c == compatible.as_bytes()))
        })
    }

    /// The parent of `node`.
    pub fn parent(&self, node: Node) -> Option<Node> {
        let mut path = [Node(0); 32];
        for (n, depth) in self.nodes() {
            if depth >= path.len() {
                return None;
            }
            path[depth] = n;
            if n == node {
                return (depth > 0).then(|| path[depth - 1]);
            }
        }
        None
    }
}

/// Big-endian 32-bit cells of a property value.
pub fn cells(value: &[u8]) -> impl Iterator<Item = u32> + '_ {
    value
        .chunks_exact(4)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// A blob builder, enough for the tests.
    struct Builder {
        structs: Vec<u8>,
        strings: Vec<u8>,
    }

    impl Builder {
        fn new() -> Self {
            Self {
                structs: Vec::new(),
                strings: Vec::new(),
            }
        }
        fn word(&mut self, w: u32) {
            self.structs.extend_from_slice(&w.to_be_bytes());
        }
        fn pad(&mut self) {
            while !self.structs.len().is_multiple_of(4) {
                self.structs.push(0);
            }
        }
        fn begin(&mut self, name: &str) -> &mut Self {
            self.word(BEGIN_NODE);
            self.structs.extend_from_slice(name.as_bytes());
            self.structs.push(0);
            self.pad();
            self
        }
        fn prop(&mut self, name: &str, value: &[u8]) -> &mut Self {
            let off = self.strings.len() as u32;
            self.strings.extend_from_slice(name.as_bytes());
            self.strings.push(0);
            self.word(PROP);
            self.word(value.len() as u32);
            self.word(off);
            self.structs.extend_from_slice(value);
            self.pad();
            self
        }
        fn end(&mut self) -> &mut Self {
            self.word(END_NODE);
            self
        }
        fn finish(&mut self) -> Vec<u8> {
            self.word(END);
            let structs = 40;
            let strings = structs + self.structs.len();
            let total = strings + self.strings.len();
            let mut b = Vec::new();
            for w in [MAGIC, total as u32, structs as u32, strings as u32] {
                b.extend_from_slice(&w.to_be_bytes());
            }
            b.resize(structs, 0);
            b.extend_from_slice(&self.structs);
            b.extend_from_slice(&self.strings);
            b
        }
    }

    fn be(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn finds_a_node_its_parent_and_a_property() {
        // The shape of the Raspberry Pi 4's emmc2bus.
        let blob = Builder::new()
            .begin("")
            .prop("compatible", b"raspberrypi,4-model-b\0brcm,bcm2711\0")
            .begin("emmc2bus")
            .prop("dma-ranges", &be(&[0, 0xc000_0000, 0, 0, 0x4000_0000]))
            .begin("mmc@7e340000")
            .prop("compatible", b"brcm,bcm2711-emmc2\0")
            .end()
            .end()
            .end()
            .finish();
        let fdt = Fdt::new(&blob).unwrap();
        let mmc = fdt.find_compatible("brcm,bcm2711-emmc2").unwrap();
        let bus = fdt.parent(mmc).unwrap();
        let ranges: Vec<_> = cells(fdt.property(bus, "dma-ranges").unwrap()).collect();
        assert_eq!(ranges, [0, 0xc000_0000, 0, 0, 0x4000_0000]);
        assert!(fdt.find_compatible("brcm,bcm2711").is_some());
        assert!(fdt.find_compatible("none").is_none());
        assert!(Fdt::new(b"not a device tree").is_none());
    }
}
