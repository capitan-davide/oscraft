//! The two string sections: `strings`, a pool of names that other
//! sections refer to by offset, and `typed_strings`, the values behind
//! `confstr()` (`_CS_MACHINE`, `_CS_HOSTNAME`, ...).

/// A buffer of bytes with a fixed capacity.
#[derive(Clone, Debug)]
pub struct Bytes<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> Bytes<N> {
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn extend(&mut self, bytes: &[u8]) {
        let end = self.len + bytes.len();
        assert!(end <= N, "syspage section full ({N} bytes)");
        self.buf[self.len..end].copy_from_slice(bytes);
        self.len = end;
    }

    /// Append zeros up to a multiple of `align`.
    pub fn pad_to(&mut self, align: usize) {
        while !self.len.is_multiple_of(align) {
            self.extend(&[0]);
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf[..self.len]
    }
}

impl<const N: usize> Default for Bytes<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// The `strings` section: NUL-terminated names, each stored once.
#[derive(Clone, Debug, Default)]
pub struct StringPool<const N: usize> {
    bytes: Bytes<N>,
}

impl<const N: usize> StringPool<N> {
    pub const fn new() -> Self {
        Self {
            bytes: Bytes::new(),
        }
    }

    /// The offset of `s` in the pool, adding it if it is not there yet.
    pub fn add(&mut self, s: &str) -> u16 {
        if let Some(off) = self.find(s) {
            return off;
        }
        let off = self.bytes.len();
        self.bytes.extend(s.as_bytes());
        self.bytes.extend(&[0]);
        off as u16
    }

    /// The offset of `s`, if it is in the pool.
    pub fn find(&self, s: &str) -> Option<u16> {
        let mut off = 0;
        for name in self.bytes.as_slice().split_inclusive(|&b| b == 0) {
            if &name[..name.len() - 1] == s.as_bytes() {
                return Some(off as u16);
            }
            off += name.len();
        }
        None
    }

    /// The string at offset `off`.
    pub fn get(&self, off: u16) -> &str {
        let rest = &self.bytes.as_slice()[usize::from(off)..];
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        core::str::from_utf8(&rest[..end]).unwrap_or("?")
    }

    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

/// The `typed_strings` section: entries of a `u32` type (a `_CS_*`
/// constant) followed by a NUL-terminated string, each padded to 4 bytes,
/// and a 0 type that ends the list.
#[derive(Clone, Debug, Default)]
pub struct TypedStrings<const N: usize> {
    bytes: Bytes<N>,
}

impl<const N: usize> TypedStrings<N> {
    pub const fn new() -> Self {
        Self {
            bytes: Bytes::new(),
        }
    }

    /// Set the string of `type_`, replacing an earlier one.
    pub fn set(&mut self, type_: u32, s: &str) {
        if self.find(type_).is_some() {
            self.remove(type_);
        }
        self.bytes.extend(&type_.to_le_bytes());
        self.bytes.extend(s.as_bytes());
        self.bytes.extend(&[0]);
        self.bytes.pad_to(4);
    }

    /// The string of `type_`, if set.
    pub fn find(&self, type_: u32) -> Option<&str> {
        self.entries().find(|e| e.0 == type_).map(|e| e.1)
    }

    fn remove(&mut self, type_: u32) {
        let mut kept = Bytes::<N>::new();
        for (t, s) in self.entries() {
            if t != type_ {
                kept.extend(&t.to_le_bytes());
                kept.extend(s.as_bytes());
                kept.extend(&[0]);
                kept.pad_to(4);
            }
        }
        self.bytes = kept;
    }

    /// The entries, as (type, string).
    pub fn entries(&self) -> impl Iterator<Item = (u32, &str)> {
        let mut rest = self.bytes.as_slice();
        core::iter::from_fn(move || {
            if rest.len() < 4 {
                return None;
            }
            let t = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
            let body = &rest[4..];
            let end = body.iter().position(|&b| b == 0)?;
            let s = core::str::from_utf8(&body[..end]).unwrap_or("?");
            let size = (4 + end + 1).next_multiple_of(4);
            rest = &rest[size.min(rest.len())..];
            Some((t, s))
        })
    }

    /// The section as the kernel reads it: the entries and the end marker.
    pub fn write(&self, out: &mut [u8]) -> usize {
        let len = self.bytes.len();
        out[..len].copy_from_slice(self.bytes.as_slice());
        out[len..len + 4].fill(0);
        len + 4
    }

    pub fn size(&self) -> usize {
        self.bytes.len() + 4
    }
}

/// `_CS_*` values of `<confname.h>` that startup sets.
pub const CS_HOSTNAME: u32 = 2;
pub const CS_MACHINE: u32 = 5;
pub const CS_ARCHITECTURE: u32 = 6;
pub const CS_HW_SERIAL: u32 = 7;
pub const CS_HW_PROVIDER: u32 = 8;

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    #[test]
    fn pool_stores_each_string_once() {
        let mut p = StringPool::<64>::new();
        assert_eq!(p.add("hw"), 0);
        assert_eq!(p.add("memory"), 3);
        assert_eq!(p.add("hw"), 0);
        assert_eq!(p.get(3), "memory");
        assert_eq!(p.as_slice(), b"hw\0memory\0");
    }

    #[test]
    fn typed_strings_layout_and_replace() {
        let mut t = TypedStrings::<64>::new();
        t.set(CS_MACHINE, "RaspberryPi4B");
        t.set(CS_HOSTNAME, "x");
        t.set(CS_HOSTNAME, "localhost");
        let e: Vec<_> = t.entries().collect();
        assert_eq!(
            e,
            [(CS_MACHINE, "RaspberryPi4B"), (CS_HOSTNAME, "localhost")]
        );
        // 4 + 14 rounded to 20, 4 + 10 rounded to 16, and the end marker.
        assert_eq!(t.size(), 20 + 16 + 4);
    }
}
