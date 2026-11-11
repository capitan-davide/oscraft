//! Describing a CPU: cpuinfo, its caches and its ID registers.
//!
//! Every CPU runs [`init_cpuinfo`] for itself, since its ID registers can
//! only be read on it: the boot CPU early, the others as they start.

use crate::abi::syspage::*;
use crate::callout;
use crate::global::Global;
use crate::syspage::Syspage;
use core::arch::{asm, global_asm};

macro_rules! read {
    ($reg:literal) => {{
        let v: u64;
        // SAFETY: reading an ID or system register has no side effect.
        unsafe { asm!(concat!("mrs {}, ", $reg), out(reg) v) };
        v
    }};
}

/// `MIDR_EL1`: the CPU's implementer, part and revision.
pub fn midr() -> u64 {
    read!("midr_el1")
}

/// `MPIDR_EL1`: the CPU's place in the system (its affinity levels).
pub fn mpidr() -> u64 {
    read!("mpidr_el1")
}

/// `ID_AA64MMFR0_EL1`: the MMU's features (address and ASID sizes).
pub fn mmfr0() -> u64 {
    read!("id_aa64mmfr0_el1")
}

/// `CNTFRQ_EL0`: the generic timer's frequency.
pub fn cntfrq() -> u32 {
    read!("cntfrq_el0") as u32
}

/// Names of Arm's own cores, by `MIDR_EL1` part number.
const ARM_CORES: [(u64, &str); 12] = [
    (0xd03, "Cortex-A53"),
    (0xd04, "Cortex-A35"),
    (0xd05, "Cortex-A55"),
    (0xd07, "Cortex-A57"),
    (0xd08, "Cortex-A72"),
    (0xd09, "Cortex-A73"),
    (0xd0a, "Cortex-A75"),
    (0xd0b, "Cortex-A76"),
    (0xd0c, "Neoverse-N1"),
    (0xd0d, "Cortex-A77"),
    (0xd41, "Cortex-A78"),
    (0xd44, "Cortex-X1"),
];

/// The CPU's name, from its `MIDR_EL1`.
pub fn name(midr: u64) -> &'static str {
    if (midr >> 24) & 0xff != 0x41 {
        return "Unknown";
    }
    let part = (midr >> 4) & 0xfff;
    ARM_CORES
        .iter()
        .find(|c| c.0 == part)
        .map_or("Unknown", |c| c.1)
}

// AArch64 cpuinfo flags (<aarch64/syspage.h>), including the ARMv7 ones
// set for AArch32 programs.
const FLAG_V6: u64 = 0x0002;
const FLAG_SMP: u64 = 0x0008;
const FLAG_V7_MP: u64 = 0x0010;
const FLAG_V7: u64 = 0x0020;
const FLAG_SIMD: u64 = 0x0040;
const FLAG_IDIV: u64 = 0x0400;
const FLAG_VFP_D32: u64 = 0x0800;
const FLAG_LSE: u64 = 0x8000;
const FLAG_AES: u64 = 0x10_0000;
const FLAG_PMULL: u64 = 0x20_0000;
const FLAG_SHA1: u64 = 0x40_0000;
const FLAG_SHA256: u64 = 0x80_0000;
const FLAG_SHA512: u64 = 0x100_0000;
const FLAG_SHA3: u64 = 0x200_0000;
const FLAG_RNDR: u64 = 0x400_0000;
const FLAG_ICACHE_COHERENT: u64 = 0x800_0000;
const FLAG_SPECTRE_V2_FIX: u64 = 0x1000;
const FLAG_SPECTRE_V2_MMU: u64 = 0x2000;
const FLAG_PAUTH: u64 = 0x2_0000;
const FLAG_SSBS: u64 = 0x8_0000;
const FLAG_SVE: u64 = 0x1000_0000;

/// What the options ask of every CPU's description, beyond what its ID
/// registers say.
#[derive(Clone, Copy, Debug, Default)]
pub struct Features {
    /// `-e sve`: let the kernel use SVE, if the CPU has it.
    pub sve: bool,
    /// `-U`: let the kernel use pointer authentication, if the CPU has it.
    pub pauth: bool,
    /// `-E spectrev2`: the kernel's Spectre variant 2 mitigation.
    pub spectre_v2: bool,
    /// `-E ssbs`: Speculative Store Bypass Safe.
    pub ssbs: bool,
    /// `-X cachesnooped`: the data caches are kept coherent with DMA.
    pub cache_snooped: bool,
    /// `-F`: flags to set and to clear, after all others.
    pub flags_on: u64,
    pub flags_off: u64,
}

static FEATURES: Global<Features> = Global::new(Features {
    sve: false,
    pauth: false,
    spectre_v2: false,
    ssbs: false,
    cache_snooped: false,
    flags_on: 0,
    flags_off: 0,
});

/// Use `features` for the CPUs described from now on.
pub fn set_features(features: Features) {
    // SAFETY: the boot CPU sets them before any CPU is described.
    unsafe { *FEATURES.get() = features };
}

fn features() -> Features {
    // SAFETY: read-only once the CPUs are being described.
    unsafe { *FEATURES.get() }
}

/// The cpuinfo flags of this CPU, from its ID registers.
pub fn flags(smp: bool) -> u64 {
    let field = |reg: u64, shift: u32| (reg >> shift) & 0xf;
    // The kernel runs with the MMU on; this crate requires an FPU.
    let mut f = CPU_FLAG_MMU | FLAG_V6 | FLAG_V7 | FLAG_IDIV;
    if smp {
        f |= FLAG_SMP | FLAG_V7_MP;
    }
    let pfr0 = read!("id_aa64pfr0_el1");
    if field(pfr0, 16) != 0xf || field(pfr0, 20) != 0xf {
        f |= CPU_FLAG_FPU | FLAG_VFP_D32;
        if field(pfr0, 20) != 0xf {
            f |= FLAG_SIMD;
        }
    }
    let isar0 = read!("id_aa64isar0_el1");
    for (shift, value, flag) in [
        (20, 2, FLAG_LSE),
        (4, 1, FLAG_AES),
        (4, 2, FLAG_PMULL | FLAG_AES),
        (8, 1, FLAG_SHA1),
        (12, 1, FLAG_SHA256),
        (12, 2, FLAG_SHA512 | FLAG_SHA256),
        (32, 1, FLAG_SHA3),
        (60, 1, FLAG_RNDR),
    ] {
        if field(isar0, shift) == value {
            f |= flag;
        }
    }
    // CTR_EL0.DIC: no instruction cache maintenance needed.
    if read!("ctr_el0") & (1 << 29) != 0 {
        f |= FLAG_ICACHE_COHERENT;
    }
    // What the options ask for, where the CPU has it.
    let opt = features();
    if opt.sve && field(pfr0, 32) == 1 {
        f |= FLAG_SVE;
    }
    // ID_AA64ISAR1_EL1.{APA, API}: address authentication.
    if opt.pauth && read!("id_aa64isar1_el1") & (0xff << 4) != 0 {
        f |= FLAG_PAUTH;
    }
    if opt.spectre_v2 {
        // The Cortex-A57 and A72 need the MMU-based mitigation.
        let part = (read!("midr_el1") >> 4) & 0xfff;
        f |= if part == 0xd07 || part == 0xd08 {
            FLAG_SPECTRE_V2_MMU
        } else {
            FLAG_SPECTRE_V2_FIX
        };
    }
    if opt.ssbs {
        f |= FLAG_SSBS;
    }
    (f | opt.flags_on) & !opt.flags_off
}

// ANCHOR: cache_callouts
// The cache maintenance callouts: control(base, num_lines, flags, cache,
// syspage) does up to 32 lines from the virtual address base and returns
// how many it did; 0 means the whole cache was done. MS_INVALIDATE (4) in
// flags asks for clean and invalidate, not just clean.
crate::callout!(pub static CACHE_ICACHE, "oscraft_cache_armv8_icache", patches = 0,
    // A huge request: invalidate all of it.
    "cmp     x1, #1024",
    "b.lo    0f",
    "ic      ialluis",
    "dsb     sy",
    "isb",
    "mov     x0, #0",
    "ret",
    "0: ldr  w3, [x3, #4]",     // cache->line_size
    "cmp     x1, #32",
    "mov     x4, #32",
    "csel    x1, x1, x4, ls",
    "mov     x2, x0",
    "mov     x0, x1",
    "1: ic   ivau, x2",
    "add     x2, x2, x3",
    "subs    x1, x1, #1",
    "b.ne    1b",
    "dsb     sy",
    "isb",
    "ret",
);

crate::callout!(pub static CACHE_DCACHE, "oscraft_cache_armv8_dcache", patches = 0,
    "ldr     w3, [x3, #4]",     // cache->line_size
    "cmp     x1, #32",
    "mov     x4, #32",
    "csel    x1, x1, x4, ls",
    "tst     x2, #4",           // MS_INVALIDATE
    "mov     x2, x0",
    "mov     x0, x1",
    "b.ne    1f",
    "0: dc   cvac, x2",         // clean to the point of coherency
    "add     x2, x2, x3",
    "subs    x1, x1, #1",
    "b.ne    0b",
    "dsb     sy",
    "isb",
    "ret",
    "1: dc   civac, x2",        // clean and invalidate
    "add     x2, x2, x3",
    "subs    x1, x1, #1",
    "b.ne    1b",
    "dsb     sy",
    "isb",
    "ret",
);
// ANCHOR_END: cache_callouts

/// The geometry of the cache selected by `csselr`: (line size, sets, ways).
fn cache_geometry(csselr: u64) -> (u32, u32, u16) {
    let ccsidr: u64;
    // SAFETY: selecting a cache and reading its size only changes
    // CSSELR_EL1, which nothing else relies on.
    unsafe {
        asm!("msr csselr_el1, {}", "isb", "mrs {}, ccsidr_el1", in(reg) csselr, out(reg) ccsidr)
    };
    let line = 1 << ((ccsidr & 7) + 4);
    let ways = ((ccsidr >> 3) & 0x3ff) as u16 + 1;
    let sets = ((ccsidr >> 13) & 0x7fff) as u32 + 1;
    (line, sets, ways)
}

/// Add a cacheattr entry, or find the same one another CPU added.
fn add_cache(
    sp: &mut Syspage,
    flags: u32,
    (line, sets, ways): (u32, u32, u16),
    control: &'static callout::Callout,
) -> u8 {
    let control = sp.callout(control, &[]);
    let e = CacheattrEntry {
        next: u32::from(CACHE_LIST_END),
        line_size: line,
        num_lines: sets * u32::from(ways),
        flags,
        ways,
        control,
        ..Default::default()
    };
    let same = |c: &CacheattrEntry| {
        (c.next, c.line_size, c.num_lines, c.flags, c.ways, c.control)
            == (e.next, e.line_size, e.num_lines, e.flags, e.ways, e.control)
    };
    match sp.cacheattr.as_slice().iter().position(same) {
        Some(i) => i as u8,
        None => sp.cacheattr.push(e) as u8,
    }
}

// ANCHOR: init_cpuinfo
/// Fill in cpuinfo for CPU `cpu`, running on it: its identity, speed and
/// flags, its first-level caches (the maintenance instructions act on all
/// levels, so the kernel needs only these) and its ID registers.
pub fn init_cpuinfo(sp: &mut Syspage, cpu: usize, speed_mhz: u32) {
    let midr = midr();
    let name = sp.strings.add(name(midr));
    let mut info = CpuinfoEntry {
        cpu: midr as u32,
        speed: speed_mhz,
        flags: flags(sp.num_cpu > 1),
        smp_hwcoreid: mpidr(),
        name,
        ins_cache: CACHE_LIST_END,
        data_cache: CACHE_LIST_END,
        ..Default::default()
    };
    let clidr = read!("clidr_el1");
    let snooped = if features().cache_snooped {
        CACHE_FLAG_SNOOPED
    } else {
        0
    };
    if clidr & 4 != 0 {
        info.data_cache = add_cache(
            sp,
            CACHE_FLAG_UNIFIED | snooped,
            cache_geometry(0),
            &CACHE_DCACHE,
        );
    } else {
        if clidr & 1 != 0 {
            info.ins_cache = add_cache(sp, CACHE_FLAG_INSTR, cache_geometry(1), &CACHE_ICACHE);
        }
        if clidr & 2 != 0 {
            info.data_cache = add_cache(
                sp,
                CACHE_FLAG_DATA | snooped,
                cache_geometry(0),
                &CACHE_DCACHE,
            );
        }
    }
    sp.cpuinfo[cpu] = info;
    // SAFETY: the dump is 56 u64s, which is what the routine writes.
    unsafe { __oscraft_read_idregs(sp.idreg_dump[cpu].as_mut_ptr()) };
}
// ANCHOR_END: init_cpuinfo

// Store the 56 ID registers S3_0_C0_C1_0 .. S3_0_C0_C7_7 at x0, in order.
// The unallocated ones read as zero.
global_asm!(
    ".section .text.__oscraft_read_idregs, \"ax\"",
    ".global __oscraft_read_idregs",
    "__oscraft_read_idregs:",
    ".irp m, 1, 2, 3, 4, 5, 6, 7",
    ".irp o, 0, 1, 2, 3, 4, 5, 6, 7",
    "    mrs     x9, S3_0_C0_C\\m\\()_\\o",
    "    str     x9, [x0], #8",
    ".endr",
    ".endr",
    "    ret",
);

unsafe extern "C" {
    fn __oscraft_read_idregs(dst: *mut u64);
}

/// The generic timer as the kernel needs it on this CPU: `ClockCycles()`
/// reads the virtual count from EL0, and the virtual timer is off until
/// the kernel programs it.
pub fn init_timer() {
    // SAFETY: CNTKCTL_EL1.EL0VCTEN only grants EL0 reads of the count;
    // CNTV_CTL_EL0 = 0 stops the virtual timer.
    unsafe { asm!("msr cntkctl_el1, {}", "msr cntv_ctl_el0, xzr", "isb", in(reg) 1u64 << 1) };
}
