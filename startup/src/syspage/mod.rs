//! The system page builder.
//!
//! Startup fills in the sections one by one while it learns about the
//! board ([`Syspage`]'s fields), and lays them out in their final memory
//! at the end ([`Syspage::layout`], [`Syspage::write`]). Sections refer to
//! each other only by offsets within a section (asinfo owners, string
//! names), so they can be placed in any order.
//!
//! The memory holding the system page, as the kernel finds it:
//!
//! ```text
//! +-------------------+ <- syspage (page aligned)
//! | syspage_entry     |    the header: offset and size of every section
//! | sections          |    total_size bytes in all
//! | (spare asinfo)    |
//! +-------------------+ <- page aligned
//! | callout code      |    patched copies of the callouts
//! +-------------------+ <- page aligned
//! | cpupage entries   |    one per CPU
//! +-------------------+
//! ```

pub mod asinfo;
pub mod hwinfo;
pub mod strings;

use crate::abi::syspage::*;
use crate::callout;
use asinfo::Asinfo;
use core::mem::size_of;
use hwinfo::Hwinfo;
use strings::{StringPool, TypedStrings};

/// The most CPUs startup describes.
pub const MAX_CPUS: usize = 8;

/// Size of the strings and typed_strings sections, at most.
pub const STRINGS_SIZE: usize = 1024;
pub const TYPED_STRINGS_SIZE: usize = 256;

/// A section that is an array, with room for `N` elements.
#[derive(Clone, Debug)]
pub struct List<T: Copy + Default, const N: usize> {
    items: [T; N],
    len: usize,
}

impl<T: Copy + Default, const N: usize> Default for List<T, N> {
    fn default() -> Self {
        Self {
            items: [T::default(); N],
            len: 0,
        }
    }
}

impl<T: Copy + Default, const N: usize> List<T, N> {
    /// Add an element, returning its index.
    pub fn push(&mut self, item: T) -> usize {
        assert!(self.len < N, "syspage array full");
        self.items[self.len] = item;
        self.len += 1;
        self.len - 1
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len]
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.items[..self.len]
    }
}

/// The system page while startup builds it.
pub struct Syspage {
    pub num_cpu: u16,
    pub system_private: SystemPrivateEntry,
    pub qtime: QtimeEntry,
    pub callout: CalloutEntry,
    /// One entry per CPU (the first `num_cpu` are used).
    pub cpuinfo: [CpuinfoEntry; MAX_CPUS],
    pub cacheattr: List<CacheattrEntry, 8>,
    pub intrinfo: List<IntrinfoEntry, 8>,
    pub smp: SmpEntry,
    pub hypinfo: HypinfoEntry,
    pub cluster: List<ClusterEntry, 8>,
    pub asinfo: Asinfo,
    pub hwinfo: Hwinfo,
    pub strings: StringPool<STRINGS_SIZE>,
    pub typed_strings: TypedStrings<TYPED_STRINGS_SIZE>,
    /// AArch64: the value that targets each CPU with a GIC SGI.
    pub gic_map: [u64; MAX_CPUS],
    /// AArch64: each CPU's GICv3 redistributor.
    pub gicr_map: [u32; MAX_CPUS],
    /// AArch64: each CPU's ID registers.
    pub idreg_dump: [[u64; AARCH64_IDDUMP_ENTRIES]; MAX_CPUS],
    /// AArch64: virtual address of CPU 0's level 0 page table.
    pub l0_vaddr: u64,
    /// The callouts the sections refer to.
    pub callouts: callout::Table,
}

/// Where each section goes, as offsets from the start of the system page.
#[derive(Clone, Copy, Debug, Default)]
pub struct Layout {
    pub header: SyspageEntry,
    /// Size of the header and the sections.
    pub total_size: usize,
}

impl Default for SyspageEntry {
    fn default() -> Self {
        // SAFETY: the header is plain integers; all zeros is valid.
        unsafe { core::mem::zeroed() }
    }
}

/// How much room the kernel may need to add asinfo entries after the
/// sections, as QNX's startup library leaves.
pub const ASINFO_SPARE: usize = 4 * size_of::<AsinfoEntry>();

impl Default for Syspage {
    fn default() -> Self {
        Self::new()
    }
}

impl Syspage {
    pub fn new() -> Self {
        Self {
            num_cpu: 1,
            system_private: SystemPrivateEntry::default(),
            qtime: QtimeEntry::default(),
            callout: CalloutEntry::default(),
            cpuinfo: [CpuinfoEntry::default(); MAX_CPUS],
            cacheattr: List::default(),
            intrinfo: List::default(),
            smp: SmpEntry::default(),
            hypinfo: HypinfoEntry::default(),
            cluster: List::default(),
            asinfo: Asinfo::new(),
            hwinfo: Hwinfo::new(),
            strings: StringPool::new(),
            typed_strings: TypedStrings::new(),
            gic_map: [0; MAX_CPUS],
            gicr_map: [0; MAX_CPUS],
            idreg_dump: [[0; AARCH64_IDDUMP_ENTRIES]; MAX_CPUS],
            l0_vaddr: 0,
            callouts: callout::Table::new(),
        }
    }

    /// Add an asinfo entry (see [`Asinfo::add`]).
    pub fn as_add(&mut self, start: u64, end: u64, name: &str, owner: u16) -> u16 {
        self.asinfo
            .add(&mut self.strings, start, end, 0, name, owner)
    }

    /// Add an asinfo entry in pieces (see [`Asinfo::add_containing`]).
    pub fn as_add_containing(
        &mut self,
        start: u64,
        end: u64,
        name: &str,
        container: &str,
    ) -> Option<u16> {
        self.asinfo
            .add_containing(&mut self.strings, start, end, 0, name, container)
    }

    /// The first asinfo entry named `name`.
    pub fn as_find(&self, name: &str) -> Option<u16> {
        self.asinfo.find(&self.strings, name)
    }

    /// Set a `confstr()` value.
    pub fn add_typed_string(&mut self, type_: u32, s: &str) {
        self.typed_strings.set(type_, s);
    }

    /// Use a callout; store the result in a section's callout field.
    pub fn callout(&mut self, callout: &'static callout::Callout, values: &[u64]) -> u64 {
        self.callouts.add(callout, values)
    }

    // ANCHOR: layout
    /// Place the sections. Each one starts 8-byte aligned.
    pub fn layout(&self) -> Layout {
        let mut h = SyspageEntry::default();
        let n = usize::from(self.num_cpu);
        let mut off = size_of::<SyspageEntry>();
        let mut place = |size: usize| {
            let at = off;
            off = (off + size).next_multiple_of(8);
            EntryInfo {
                entry_off: at as u16,
                entry_size: size as u16,
            }
        };
        let array = |e: EntryInfo, elem: usize| ArrayInfo {
            entry_off: e.entry_off,
            entry_size: e.entry_size,
            element_size: elem as u16,
        };
        let mut aarch64 = Aarch64SyspageEntry {
            l0_vaddr: self.l0_vaddr,
            ..Default::default()
        };
        aarch64.gic_map = place(n * size_of::<u64>());
        aarch64.gicr_map = place(n * size_of::<u32>());
        aarch64.idreg_dump = place(n * AARCH64_IDDUMP_ENTRIES * size_of::<u64>());
        h.un = SyspageUnion { aarch64 };
        h.callout = place(size_of::<CalloutEntry>());
        h.system_private = place(size_of::<SystemPrivateEntry>());
        h.qtime = place(size_of::<QtimeEntry>());
        h.cpuinfo = array(
            place(n * size_of::<CpuinfoEntry>()),
            size_of::<CpuinfoEntry>(),
        );
        h.cacheattr = array(
            place(core::mem::size_of_val(self.cacheattr.as_slice())),
            size_of::<CacheattrEntry>(),
        );
        h.intrinfo = array(
            place(core::mem::size_of_val(self.intrinfo.as_slice())),
            size_of::<IntrinfoEntry>(),
        );
        // A single CPU system has no smp section.
        h.smp = if n > 1 {
            place(size_of::<SmpEntry>())
        } else {
            EntryInfo::default()
        };
        h.hypinfo = place(size_of::<HypinfoEntry>());
        h.typed_strings = place(self.typed_strings.size());
        h.strings = place(self.strings.as_slice().len());
        h.hwinfo = place(self.hwinfo.as_slice().len());
        h.asinfo = array(
            place(core::mem::size_of_val(self.asinfo.entries())),
            size_of::<AsinfoEntry>(),
        );
        h.cluster = array(
            place(size_of_val(self.cluster.as_slice())),
            size_of::<ClusterEntry>(),
        );
        h.size = size_of::<SyspageEntry>() as u16;
        h.total_size = off as u16;
        h.type_ = SYSPAGE_AARCH64;
        h.num_cpu = self.num_cpu;
        h.version_major = SYSPAGE_VERSION_MAJOR;
        h.version_minor = SYSPAGE_VERSION_MINOR;
        Layout {
            header: h,
            total_size: off,
        }
    }
    // ANCHOR_END: layout

    /// Replace the callout references in the sections by the callouts'
    /// addresses (see [`callout::Table::output`]), and set the sizes of
    /// the interrupt callouts the kernel copies.
    pub fn resolve_callouts(&mut self, addrs: &[u64; callout::MAX_CALLOUTS]) {
        let t = &self.callouts;
        let r = |v: &mut u64| *v = t.resolve(*v, addrs);
        let c = &mut self.callout;
        r(&mut c.reboot);
        r(&mut c.debug_watchdog);
        r(&mut c.custom);
        for d in &mut c.debug {
            r(&mut d.display_char);
            r(&mut d.poll_key);
            r(&mut d.break_detect);
        }
        for i in self.intrinfo.as_mut_slice() {
            for g in [&mut i.id, &mut i.eoi] {
                if let Some((_, u)) = t.get(g.rtn) {
                    g.size = u.callout.code().len() as u16;
                }
                r(&mut g.rtn);
            }
            r(&mut i.mask);
            r(&mut i.unmask);
            r(&mut i.config);
        }
        for ca in self.cacheattr.as_mut_slice() {
            r(&mut ca.control);
        }
        r(&mut self.smp.send_ipi);
    }

    /// Write the header and the sections into `out`, as laid out by
    /// `layout`. Returns the number of bytes written.
    pub fn write(&self, layout: &Layout, out: &mut [u8]) -> usize {
        let h = &layout.header;
        let n = usize::from(self.num_cpu);
        out[..layout.total_size].fill(0);
        put(out, 0, as_bytes(h));
        // SAFETY: layout() set the AArch64 member.
        let a = unsafe { h.un.aarch64 };
        put(out, a.gic_map.entry_off, slice_bytes(&self.gic_map[..n]));
        put(out, a.gicr_map.entry_off, slice_bytes(&self.gicr_map[..n]));
        put(
            out,
            a.idreg_dump.entry_off,
            slice_bytes(&self.idreg_dump[..n]),
        );
        put(out, h.callout.entry_off, as_bytes(&self.callout));
        put(
            out,
            h.system_private.entry_off,
            as_bytes(&self.system_private),
        );
        put(out, h.qtime.entry_off, as_bytes(&self.qtime));
        put(out, h.cpuinfo.entry_off, slice_bytes(&self.cpuinfo[..n]));
        put(
            out,
            h.cacheattr.entry_off,
            slice_bytes(self.cacheattr.as_slice()),
        );
        put(
            out,
            h.intrinfo.entry_off,
            slice_bytes(self.intrinfo.as_slice()),
        );
        if n > 1 {
            put(out, h.smp.entry_off, as_bytes(&self.smp));
        }
        put(out, h.hypinfo.entry_off, as_bytes(&self.hypinfo));
        let ts = usize::from(h.typed_strings.entry_off);
        self.typed_strings.write(&mut out[ts..]);
        put(out, h.strings.entry_off, self.strings.as_slice());
        put(out, h.hwinfo.entry_off, self.hwinfo.as_slice());
        put(out, h.asinfo.entry_off, slice_bytes(self.asinfo.entries()));
        put(
            out,
            h.cluster.entry_off,
            slice_bytes(self.cluster.as_slice()),
        );
        layout.total_size
    }
}

fn put(out: &mut [u8], off: u16, bytes: &[u8]) {
    let off = usize::from(off);
    out[off..off + bytes.len()].copy_from_slice(bytes);
}

/// The bytes of a plain-data value.
fn as_bytes<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: the ABI types are #[repr(C)] integers; any padding bytes
    // are only copied, never interpreted.
    unsafe { core::slice::from_raw_parts((v as *const T).cast(), size_of::<T>()) }
}

fn slice_bytes<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: as in as_bytes().
    unsafe { core::slice::from_raw_parts(v.as_ptr().cast(), size_of_val(v)) }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;

    #[test]
    fn layout_and_write() {
        let mut sp = Syspage::new();
        sp.num_cpu = 4;
        let mem = sp.as_add(0, 0xfff_ffff_ffff, "memory", AS_NULL_OFF);
        sp.as_add(0, 0x7fff_ffff, "ram", mem);
        hwinfo::add_default(&mut sp.hwinfo, &mut sp.strings);
        sp.hwinfo.finish();
        sp.add_typed_string(strings::CS_HOSTNAME, "localhost");
        sp.cpuinfo[3].smp_hwcoreid = 0x8000_0003;
        let layout = sp.layout();
        let h = layout.header;
        assert_eq!(h.size, 240);
        assert_eq!(h.num_cpu, 4);
        assert_eq!(usize::from(h.total_size), layout.total_size);
        // Every section is 8-aligned and inside the system page.
        for e in [
            h.callout,
            h.qtime,
            h.system_private,
            h.smp,
            h.hwinfo,
            h.strings,
        ] {
            assert_eq!(e.entry_off % 8, 0);
            assert!(usize::from(e.entry_off) + usize::from(e.entry_size) <= layout.total_size);
        }
        assert_eq!(h.cpuinfo.entry_size, 4 * 32);
        assert_eq!(h.asinfo.entry_size, 2 * 24);

        let mut out = vec![0xaa; 4096];
        let n = sp.write(&layout, &mut out);
        assert_eq!(n, layout.total_size);
        // The header round-trips, and CPU 3's entry is where it says.
        let back: SyspageEntry = unsafe { core::ptr::read_unaligned(out.as_ptr().cast()) };
        assert_eq!(back.total_size, h.total_size);
        let cpu3 = usize::from(h.cpuinfo.entry_off) + 3 * 32 + 16;
        assert_eq!(out[cpu3..cpu3 + 8], 0x8000_0003u64.to_le_bytes());
        let s = usize::from(h.strings.entry_off);
        assert_eq!(&out[s..s + 7], b"memory\0");
    }
}
