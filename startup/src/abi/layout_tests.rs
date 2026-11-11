//! Check the Rust mirrors against the QNX headers' layout, as recorded in
//! `abi/layout.txt` (see `abi/layout.c`).

extern crate std;

use super::elf::*;
use super::image::*;
use super::startup::*;
use super::syspage::*;
use core::mem::{offset_of, size_of};
use std::collections::BTreeMap;
use std::string::{String, ToString};
use std::vec::Vec;

/// The layout of the Rust types, in `layout.txt`'s terms.
fn rust_layout() -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    macro_rules! size {
        ($t:ident) => {
            m.insert(
                concat!(stringify!($t), ".size").to_string(),
                size_of::<$t>(),
            );
        };
    }
    macro_rules! off {
        ($t:ident, $($f:ident),+) => {
            $(m.insert(concat!(stringify!($t), ".", stringify!($f)).to_string(), offset_of!($t, $f));)+
        };
    }
    size!(StartupHeader);
    off!(
        StartupHeader,
        image_paddr,
        startup_size,
        imagefs_size,
        addr_off,
        info
    );
    size!(BootArgsEntry);
    off!(BootArgsEntry, shdr_addr);
    size!(SyspageEntry);
    off!(
        SyspageEntry,
        system_private,
        version_major,
        hwinfo,
        qtime,
        callout
    );
    off!(
        SyspageEntry,
        typed_strings,
        strings,
        smp,
        un,
        asinfo,
        cpuinfo
    );
    off!(SyspageEntry, cacheattr, intrinfo, hypinfo, cluster);
    size!(Aarch64SyspageEntry);
    off!(Aarch64SyspageEntry, gic_map, gicr_map, idreg_dump, rsvd);
    size!(SystemPrivateEntry);
    off!(
        SystemPrivateEntry,
        kdump_info,
        user_cpupageptr,
        kern_syspageptr,
        entropy
    );
    size!(QtimeEntry);
    off!(
        QtimeEntry,
        nsec_inc,
        adjust_tick_count,
        timer_period,
        intr,
        flags
    );
    off!(QtimeEntry, timer_load_max, tick_period_cc);
    size!(CalloutEntry);
    off!(CalloutEntry, debug, debug_watchdog, custom);
    size!(DebugCallout);
    size!(AsinfoEntry);
    off!(AsinfoEntry, owner, priority);
    size!(CpuinfoEntry);
    off!(CpuinfoEntry, flags, smp_hwcoreid, name, data_cache);
    size!(CacheattrEntry);
    off!(CacheattrEntry, ways, control);
    size!(IntrgenData);
    off!(IntrgenData, rtn);
    size!(IntrinfoEntry);
    off!(
        IntrinfoEntry,
        cpu_intr_stride,
        id,
        eoi,
        mask,
        config,
        local_stride
    );
    size!(SmpEntry);
    off!(SmpEntry, send_ipi, start_addr);
    size!(HypinfoEntry);
    size!(ClusterEntry);
    size!(CpupageEntry);
    off!(CpupageEntry, un, pls);
    size!(ImageHeader);
    off!(ImageHeader, dir_offset, mountflags);
    size!(ImageAttr);
    size!(ImageFile);
    off!(ImageFile, offset);
    size!(IfsBootstrapHead);
    off!(IfsBootstrapHead, bootstrap);
    size!(IfsBootstrapData);
    off!(IfsBootstrapData, next_entry, args);
    size!(Elf64Ehdr);
    off!(Elf64Ehdr, e_entry, e_phnum);
    size!(Elf64Phdr);
    off!(Elf64Phdr, p_vaddr, p_memsz);
    m
}

/// The layout of the C types, from `layout.txt`.
fn c_layout() -> BTreeMap<String, usize> {
    include_str!("../../abi/layout.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| {
            let (name, value) = l.split_once(' ').expect("layout.txt: bad line");
            (
                name.to_string(),
                value.parse().expect("layout.txt: bad value"),
            )
        })
        .collect()
}

#[test]
fn layouts_match_the_qnx_headers() {
    let (rust, c) = (rust_layout(), c_layout());
    let mismatches: Vec<_> = c
        .iter()
        .filter(|(name, value)| rust.get(*name) != Some(value))
        .map(|(name, value)| (name, *value, rust.get(name).copied()))
        .collect();
    assert!(mismatches.is_empty(), "C vs Rust layout: {mismatches:#?}");
    let unchecked: Vec<_> = rust.keys().filter(|k| !c.contains_key(*k)).collect();
    assert!(unchecked.is_empty(), "not in layout.txt: {unchecked:?}");
}
