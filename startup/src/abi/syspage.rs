//! The system page (`<sys/syspage.h>`, `<aarch64/syspage.h>`).
//!
//! The system page is one block of memory: a [`SyspageEntry`] header,
//! then the sections it points to by offset and size, then the callouts'
//! read/write data and code, then one [`CpupageEntry`] per CPU.

/// `syspage_entry_info`: where a section is, relative to the system page.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntryInfo {
    pub entry_off: u16,
    pub entry_size: u16,
}

/// `syspage_array_info`: a section that is an array of elements.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArrayInfo {
    pub entry_off: u16,
    pub entry_size: u16,
    pub element_size: u16,
}

/// The system page layout version.
pub const SYSPAGE_VERSION_MAJOR: u16 = 2;
pub const SYSPAGE_VERSION_MINOR: u16 = 0;

/// `SYSPAGE_AARCH64`: `SYSPAGE_VERSIONED | SYSPAGE_64BIT`, plus one.
pub const SYSPAGE_AARCH64: u16 = 0x1101;

// ANCHOR: aarch64_syspage_entry
/// `struct aarch64_syspage_entry`: the AArch64 part of the header.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Aarch64SyspageEntry {
    /// Virtual address of CPU 0's level 0 page table; CPU n's is 64 bytes
    /// per CPU further.
    pub l0_vaddr: u64,
    /// One `u64` per CPU: the value that targets it with a GIC SGI.
    pub gic_map: EntryInfo,
    /// One `u32` per CPU: its GICv3 redistributor.
    pub gicr_map: EntryInfo,
    /// The CPUs' ID registers, 56 `u64` per CPU.
    pub idreg_dump: EntryInfo,
    pub rsvd: [u64; 6],
}
// ANCHOR_END: aarch64_syspage_entry

/// Number of ID registers per CPU in `idreg_dump`: `S3_0_C0_C1_0` through
/// `S3_0_C0_C7_7`.
pub const AARCH64_IDDUMP_ENTRIES: usize = 7 * 8;

// ANCHOR: syspage_entry
/// `struct syspage_entry`: the system page's header.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SyspageEntry {
    /// Size of this header.
    pub size: u16,
    /// Size of the header and all the sections.
    pub total_size: u16,
    /// [`SYSPAGE_AARCH64`].
    pub type_: u16,
    pub num_cpu: u16,
    pub system_private: EntryInfo,
    pub version_major: u16,
    pub version_minor: u16,
    pub hwinfo: EntryInfo,
    pub qtime: EntryInfo,
    pub callout: EntryInfo,
    pub typed_strings: EntryInfo,
    pub strings: EntryInfo,
    pub smp: EntryInfo,
    /// `un`: the architecture's part, padded to 20 `u64`.
    pub un: SyspageUnion,
    pub asinfo: ArrayInfo,
    pub cpuinfo: ArrayInfo,
    pub cacheattr: ArrayInfo,
    pub intrinfo: ArrayInfo,
    pub hypinfo: EntryInfo,
    pub cluster: ArrayInfo,
}
// ANCHOR_END: syspage_entry

/// The `un` union of [`SyspageEntry`].
#[repr(C)]
#[derive(Clone, Copy)]
pub union SyspageUnion {
    pub aarch64: Aarch64SyspageEntry,
    pub filler: [u64; 20],
}

impl core::fmt::Debug for SyspageUnion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // SAFETY: both members are plain data; the AArch64 one is the one in
        // use.
        unsafe { self.aarch64.fmt(f) }
    }
}

/// `SYSTEM_PRIVATE_FLAG_ABNORMAL_REBOOT`: the last reboot was abnormal.
pub const SYSTEM_PRIVATE_FLAG_ABNORMAL_REBOOT: u32 = 0x0000_0001;

/// `struct system_private_entry`: what the kernel needs about the system
/// page itself and the boot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemPrivateEntry {
    /// Distance between two CPUs' cpupage entries.
    pub cpupage_spacing: u32,
    pub private_flags: u32,
    pub pagesize: u32,
    pub num_ready: u32,
    pub traced_classes: u32,
    pub spare: u32,
    pub kdump_info: u64,
    /// The cpupage and system page, as user code sees them (read-only).
    pub user_cpupageptr: u64,
    pub user_syspageptr: u64,
    /// The same, as the kernel sees them.
    pub kern_cpupageptr: u64,
    pub kern_syspageptr: u64,
    pub kdebug_info: u64,
    pub kdebug_call: u64,
    pub entropy: [u64; 2],
}

// ANCHOR: qtime_entry
/// `struct qtime_entry`: the clocks and the system tick.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct QtimeEntry {
    /// Frequency of `ClockCycles()`: the generic timer's counter.
    pub cycles_per_sec: u64,
    pub nsec_tod_adjust: u64,
    pub nsec_inc: u32,
    /// Seconds since the epoch at boot, if known.
    pub boot_time: u32,
    /// `struct _clockadjust`: `tick_count`, `tick_nsec_inc`.
    pub adjust_tick_count: u32,
    pub adjust_tick_nsec_inc: i32,
    pub timer_period: u32,
    pub timer_scale: i32,
    /// The system tick's interrupt vector.
    pub intr: i32,
    pub epoch: u32,
    pub flags: u32,
    pub rr_interval_mul: u32,
    pub timer_load_max: u64,
    pub boot_cc: u64,
    pub tick_period_cc: u64,
    pub spare: [u64; 3],
}
// ANCHOR_END: qtime_entry

/// `ClockCycles()` is the same on all CPUs.
pub const QTIME_FLAG_GLOBAL_CLOCKCYCLES: u32 = 0x0000_0008;
/// The kernel runs without a periodic tick.
pub const QTIME_FLAG_TICKLESS: u32 = 0x0000_0004;

/// `struct debug_callout`: one debug channel's callouts.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct DebugCallout {
    pub display_char: u64,
    pub poll_key: u64,
    pub break_detect: u64,
    pub spare: u64,
}

// ANCHOR: callout_entry
/// `struct callout_entry`: the kernel's entry points into board code that
/// do not belong to another section.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CalloutEntry {
    pub reboot: u64,
    pub debug: [DebugCallout; 2],
    pub debug_watchdog: u64,
    pub custom: u64,
    pub spare: [u64; 2],
}
// ANCHOR_END: callout_entry

/// `struct asinfo_entry`: one range of the physical address space.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AsinfoEntry {
    pub start: u64,
    /// Last byte of the range (inclusive).
    pub end: u64,
    /// Offset of the parent entry in the section, or [`AS_NULL_OFF`].
    pub owner: u16,
    /// Offset of the name in the strings section.
    pub name: u16,
    pub attr: u16,
    pub priority: u16,
}

pub const AS_NULL_OFF: u16 = 0xffff;
/// The entry has children. The only other attribute, [`AS_ATTR_CONTINUED`],
/// is for ranges split over several entries; QNX 8.0 defines no
/// readable/writable/cacheable bits (its startup library writes 0 for RAM,
/// ROM and devices alike).
pub const AS_ATTR_KIDS: u16 = 0x0010;
pub const AS_ATTR_CONTINUED: u16 = 0x0020;
pub const AS_PRIORITY_DEFAULT: u16 = 100;

/// `struct cpuinfo_entry`: one CPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuinfoEntry {
    /// The CPU's `MIDR_EL1`.
    pub cpu: u32,
    /// Speed in MHz.
    pub speed: u32,
    pub flags: u64,
    /// The CPU's `MPIDR_EL1`.
    pub smp_hwcoreid: u64,
    pub spare: u32,
    /// Offset of the name in the strings section.
    pub name: u16,
    /// First instruction and data cache: indexes into cacheattr.
    pub ins_cache: u8,
    pub data_cache: u8,
}

pub const CPU_FLAG_FPU: u64 = 1 << 31;
pub const CPU_FLAG_MMU: u64 = 1 << 30;

/// `struct cacheattr_entry`: one cache.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheattrEntry {
    /// Index of the next cache level, or [`CACHE_LIST_END`].
    pub next: u32,
    pub line_size: u32,
    pub num_lines: u32,
    pub flags: u32,
    pub ways: u16,
    pub spare0: [u16; 3],
    /// The cache control callout.
    pub control: u64,
}

pub const CACHE_LIST_END: u8 = 0xff;
pub const CACHE_FLAG_INSTR: u32 = 0x0001;
pub const CACHE_FLAG_DATA: u32 = 0x0002;
pub const CACHE_FLAG_UNIFIED: u32 = 0x0003;
pub const CACHE_FLAG_SHARED: u32 = 0x0004;
pub const CACHE_FLAG_SNOOPED: u32 = 0x0008;
pub const CACHE_FLAG_WRITEBACK: u32 = 0x0020;
pub const CACHE_FLAG_CTRL_PHYS64: u32 = 0x1000;

// ANCHOR: intrinfo_entry
/// `struct __intrgen_data`: an interrupt callout that the kernel copies
/// into its own interrupt entry code, rather than calling it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct IntrgenData {
    /// `INTR_GENFLAG_*`: which registers the kernel loads for the code.
    pub genflags: u16,
    /// Size of the code, in bytes.
    pub size: u16,
    pub spare: u32,
    pub rtn: u64,
}

/// `struct intrinfo_entry`: one interrupt controller, or one range of its
/// interrupts.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct IntrinfoEntry {
    /// First logical interrupt vector of the range.
    pub vector_base: u32,
    pub num_vectors: u32,
    /// Vector this controller cascades into, or `0x7fffffff`.
    pub cascade_vector: u32,
    pub cpu_intr_base: u32,
    pub cpu_intr_stride: u16,
    pub flags: u16,
    /// Find the interrupt that is pending.
    pub id: IntrgenData,
    /// Acknowledge it.
    pub eoi: IntrgenData,
    pub mask: u64,
    pub unmask: u64,
    pub config: u64,
    pub local_stride: u32,
    pub spare: [u32; 3],
}
// ANCHOR_END: intrinfo_entry

/// `cascade_vector` of a controller that is not cascaded.
pub const INTR_NOT_CASCADED: u32 = 0x7fff_ffff;
pub const INTR_GENFLAG_LOAD_SYSPAGE: u16 = 0x8000;
pub const INTR_GENFLAG_LOAD_INTRINFO: u16 = 0x4000;
pub const INTR_GENFLAG_LOAD_LEVEL: u16 = 0x2000;
pub const INTR_GENFLAG_LOAD_INTRMASK: u16 = 0x1000;
pub const INTR_GENFLAG_NOGLITCH: u16 = 0x0800;
pub const INTR_GENFLAG_LOAD_CPUNUM: u16 = 0x0400;
pub const INTR_GENFLAG_ID_LOOP: u16 = 0x0200;
pub const INTR_FLAG_MSI: u16 = 0x0400;
pub const INTR_CONFIG_FLAG_PREATTACH: u32 = 0x0001;
pub const INTR_CONFIG_FLAG_DISALLOWED: u32 = 0x0002;
pub const INTR_CONFIG_FLAG_IPI: u32 = 0x0004;
pub const INTR_CONFIG_FLAG_SYS: u32 = 0x0008;

/// `struct smp_entry`: how the CPUs talk to each other.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SmpEntry {
    /// Set to 1 by startup and cleared by each secondary CPU as it reaches
    /// the `smp_spin` callout.
    pub pending: u32,
    pub spare: u32,
    /// The callout that sends an inter-processor interrupt.
    pub send_ipi: u64,
    /// Written by the kernel: where a secondary CPU in `smp_spin` goes next.
    pub start_addr: u64,
}

/// `struct hypinfo_entry`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct HypinfoEntry {
    pub flags: u64,
}

/// `struct cluster_entry`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ClusterEntry {
    pub cpumask: u64,
    pub name: u16,
    pub spare: [u16; 3],
}

// ANCHOR: cpupage_entry
/// `struct cpupage_entry`: one CPU's page, where the kernel keeps per-CPU
/// data that user code may read.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CpupageEntry {
    pub cpu: u32,
    pub state: u32,
    /// `un`: `struct aarch64_cpupage_entry` (`gic_cpu`), padded to 8 `u32`.
    pub un: [u32; 8],
    pub pls: u64,
    pub spare: [u32; 8],
}
// ANCHOR_END: cpupage_entry
