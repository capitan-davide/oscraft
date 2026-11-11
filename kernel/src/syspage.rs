//! The system page: the hardware description built by the startup program.
//!
//! Layouts mirror the QNX 8.0 ABI (SDP 8.0.5 `<sys/syspage.h>`,
//! `<aarch64/syspage.h>`); sizes and offsets are checked at compile time.
//! Sections are located by offsets from the start of the page.

use core::mem::{offset_of, size_of};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EntryInfo {
    pub entry_off: u16,
    pub entry_size: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ArrayInfo {
    pub entry_off: u16,
    pub entry_size: u16,
    pub element_size: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}

/// `struct syspage_entry`.
#[repr(C)]
#[derive(Debug)]
pub struct SyspageEntry {
    pub size: u16,
    pub total_size: u16,
    pub type_: u16,
    pub num_cpu: u16,
    pub system_private: EntryInfo,
    pub version: Version,
    pub hwinfo: EntryInfo,
    pub qtime: EntryInfo,
    pub callout: EntryInfo,
    pub typed_strings: EntryInfo,
    pub strings: EntryInfo,
    pub smp: EntryInfo,
    /// CPU-specific part (`struct aarch64_syspage_entry`), in a 160-byte union.
    pub un: [u64; 20],
    pub asinfo: ArrayInfo,
    pub cpuinfo: ArrayInfo,
    pub cacheattr: ArrayInfo,
    pub intrinfo: ArrayInfo,
    pub hypinfo: EntryInfo,
    pub cluster: ArrayInfo,
}

pub const SYSPAGE_AARCH64: u16 = 0x1101; // SYSPAGE_VERSIONED | SYSPAGE_64BIT, + 1
/// The layout this kernel reads (`SYSPAGE_VERSION_MAJOR` of QNX 8's
/// `<sys/syspage.h>`).
pub const SYSPAGE_VERSION_MAJOR: u16 = 2;

const _: () = {
    assert!(size_of::<SyspageEntry>() == 0xf0);
    assert!(offset_of!(SyspageEntry, un) == 40);
    assert!(offset_of!(SyspageEntry, asinfo) == 200);
    assert!(offset_of!(SyspageEntry, cluster) == 228);
};

/// Output of the debug callouts: `void display_char(struct syspage_entry *, char)`.
pub type DisplayChar = unsafe extern "C" fn(*mut SyspageEntry, u8);

/// `struct debug_callout`.
#[repr(C)]
#[derive(Debug)]
pub struct DebugCallout {
    pub display_char: Option<DisplayChar>,
    pub poll_key: Option<unsafe extern "C" fn(*mut SyspageEntry) -> i32>,
    pub break_detect: Option<unsafe extern "C" fn(*mut SyspageEntry) -> i32>,
    pub spare: usize,
}

/// `struct callout_entry`.
#[repr(C)]
#[derive(Debug)]
pub struct CalloutEntry {
    pub reboot: Option<unsafe extern "C" fn(*mut SyspageEntry, i32)>,
    pub debug: [DebugCallout; 2],
    pub debug_watchdog: usize,
    pub custom: usize,
    pub spare: [u64; 2],
}

const _: () = assert!(size_of::<CalloutEntry>() == 0x68);

/// `struct system_private_entry`.
#[repr(C)]
#[derive(Debug)]
pub struct SystemPrivateEntry {
    pub cpupage_spacing: u32,
    pub private_flags: u32,
    pub pagesize: u32,
    pub num_ready: u32,
    pub traced_classes: u32,
    pub spare: u32,
    pub kdump_info: u64,
    pub user_cpupageptr: usize,
    pub user_syspageptr: usize,
    pub kern_cpupageptr: usize,
    pub kern_syspageptr: usize,
    pub kdebug_info: usize,
    pub kdebug_call: usize,
    pub entropy: [u64; 2],
}

/// `struct qtime_entry`.
#[repr(C)]
#[derive(Debug)]
pub struct QtimeEntry {
    pub cycles_per_sec: u64,
    pub nsec_tod_adjust: u64,
    pub nsec_inc: u32,
    pub boot_time: u32,
    pub adjust_tick_count: u32,
    pub adjust_tick_nsec_inc: i32,
    pub timer_period: u32,
    pub timer_scale: i32,
    pub intr: i32,
    pub epoch: u32,
    pub flags: u32,
    pub rr_interval_mul: u32,
    pub timer_load_max: u64,
    pub boot_cc: u64,
    pub tick_period_cc: u64,
    pub spare: [u64; 3],
}

/// `struct cpuinfo_entry`.
#[repr(C)]
#[derive(Debug)]
pub struct CpuinfoEntry {
    pub cpu: u32,
    pub speed: u32,
    pub flags: u64,
    pub smp_hwcoreid: u64,
    pub spare: u32,
    pub name: u16,
    pub ins_cache: u8,
    pub data_cache: u8,
}

pub const CPU_FLAG_FPU: u64 = 1 << 31;
pub const CPU_FLAG_MMU: u64 = 1 << 30;

/// `struct asinfo_entry`: one node of the physical address-space tree.
#[repr(C)]
#[derive(Debug)]
pub struct AsinfoEntry {
    pub start: u64,
    pub end: u64,
    /// Offset of the parent entry in the asinfo section, or `AS_NULL_OFF`.
    pub owner: u16,
    /// Offset of the name in the strings section.
    pub name: u16,
    pub attr: u16,
    pub priority: u16,
}

pub const AS_NULL_OFF: u16 = 0xffff;

/// `struct cacheattr_entry`.
#[repr(C)]
#[derive(Debug)]
pub struct CacheattrEntry {
    pub next: u32,
    pub line_size: u32,
    pub num_lines: u32,
    pub flags: u32,
    pub ways: u16,
    pub spare0: [u16; 3],
    pub control: usize,
}

pub const CACHE_LIST_END: u32 = 0xff;

/// `struct __intrgen_data`: a code fragment of an interrupt callout.
#[repr(C)]
#[derive(Debug)]
pub struct IntrgenData {
    pub genflags: u16,
    pub size: u16,
    pub spare: u32,
    pub rtn: usize,
}

/// `struct intrinfo_entry`: one interrupt controller.
#[repr(C)]
#[derive(Debug)]
pub struct IntrinfoEntry {
    pub vector_base: u32,
    pub num_vectors: u32,
    pub cascade_vector: u32,
    pub cpu_intr_base: u32,
    pub cpu_intr_stride: u16,
    pub flags: u16,
    pub id: IntrgenData,
    pub eoi: IntrgenData,
    pub mask: Option<unsafe extern "C" fn(*mut SyspageEntry, i32) -> i32>,
    pub unmask: Option<unsafe extern "C" fn(*mut SyspageEntry, i32) -> i32>,
    pub config: usize,
    pub local_stride: u32,
    pub spare: [u32; 3],
}

/// `struct smp_entry`.
#[repr(C)]
#[derive(Debug)]
pub struct SmpEntry {
    pub pending: u32,
    pub spare: u32,
    pub send_ipi: usize,
    pub start_addr: usize,
}

const _: () = {
    assert!(size_of::<SystemPrivateEntry>() == 0x60);
    assert!(size_of::<QtimeEntry>() == 0x68);
    assert!(offset_of!(QtimeEntry, timer_load_max) == 56);
    assert!(size_of::<CpuinfoEntry>() == 0x20);
    assert!(size_of::<AsinfoEntry>() == 0x18);
    assert!(size_of::<CacheattrEntry>() == 0x20);
    assert!(size_of::<IntrinfoEntry>() == 96);
    assert!(offset_of!(IntrinfoEntry, mask) == 56);
};

/// The system page, at the kernel virtual address startup passed in `x0`.
#[derive(Debug, Clone, Copy)]
pub struct Syspage(*mut SyspageEntry);

// SAFETY: the system page is mapped for the kernel's whole lifetime and is
// only read during boot; callouts are serialised by their users.
unsafe impl Send for Syspage {}
// SAFETY: as above.
unsafe impl Sync for Syspage {}

impl Syspage {
    /// # Safety
    ///
    /// `ptr` must be the system page passed by startup, mapped and valid for
    /// the rest of the kernel's life.
    pub unsafe fn new(ptr: *mut SyspageEntry) -> Self {
        Self(ptr)
    }

    pub fn as_ptr(&self) -> *mut SyspageEntry {
        self.0
    }

    pub fn header(&self) -> &SyspageEntry {
        // SAFETY: guaranteed by the contract of `new`.
        unsafe { &*self.0 }
    }

    /// A section described by `info`, as a `T`.
    ///
    /// # Safety
    ///
    /// `T` must be the type of that section.
    pub unsafe fn section<T>(&self, info: EntryInfo) -> &T {
        let p = self.0.cast::<u8>().wrapping_add(info.entry_off as usize).cast::<T>();
        // SAFETY: startup places sections inside the page, suitably aligned.
        unsafe { &*p }
    }

    pub fn callout(&self) -> &CalloutEntry {
        // SAFETY: the callout section is a `struct callout_entry`.
        unsafe { self.section(self.header().callout) }
    }

    /// The `struct system_private_entry`, for the fields the kernel sets
    /// (`kdebug_info`), written with volatile accesses.
    pub fn system_private_ptr(&self) -> *mut SystemPrivateEntry {
        // SAFETY: as for `section`; startup maps the page writable.
        unsafe { self.0.cast::<u8>().add(self.header().system_private.entry_off as usize).cast() }
    }

    pub fn system_private(&self) -> &SystemPrivateEntry {
        // SAFETY: the section is a `struct system_private_entry`.
        unsafe { self.section(self.header().system_private) }
    }

    pub fn qtime(&self) -> &QtimeEntry {
        // SAFETY: the section is a `struct qtime_entry`.
        unsafe { self.section(self.header().qtime) }
    }

    /// The `struct qtime_entry`, for the fields the kernel changes
    /// (`clock::set_tod_adjust`), read and written with volatile accesses.
    pub fn qtime_ptr(&self) -> *mut QtimeEntry {
        // SAFETY: as for `section`; startup maps the page writable.
        unsafe { self.0.cast::<u8>().add(self.header().qtime.entry_off as usize).cast() }
    }

    pub fn smp(&self) -> &SmpEntry {
        // SAFETY: the section is a `struct smp_entry`.
        unsafe { self.section(self.header().smp) }
    }

    /// The `struct smp_entry`, for the field the kernel writes
    /// (`start_addr`, releasing a CPU), with volatile accesses: a pointer
    /// derived from the page, not from a shared reference (review CORE-10).
    pub fn smp_ptr(&self) -> *mut SmpEntry {
        // SAFETY: as for `section`; startup maps the page writable.
        unsafe { self.0.cast::<u8>().add(self.header().smp.entry_off as usize).cast() }
    }

    /// Elements of an array section, stepping by its recorded element size.
    ///
    /// # Safety
    ///
    /// `T` must be the element type of that section.
    pub unsafe fn array<'a, T: 'a>(&'a self, info: ArrayInfo) -> impl Iterator<Item = &'a T> + 'a {
        let step = info.element_size as usize;
        let count = if step == 0 {
            0
        } else {
            info.entry_size as usize / step
        };
        let base = self.0.cast::<u8>().wrapping_add(info.entry_off as usize);
        // SAFETY: elements lie inside the section, inside the page.
        (0..count).map(move |i| unsafe { &*base.wrapping_add(i * step).cast::<T>() })
    }

    pub fn cpuinfo(&self) -> impl Iterator<Item = &CpuinfoEntry> {
        // SAFETY: the cpuinfo section holds `struct cpuinfo_entry` elements.
        unsafe { self.array(self.header().cpuinfo) }
    }

    pub fn asinfo(&self) -> impl Iterator<Item = &AsinfoEntry> {
        // SAFETY: the asinfo section holds `struct asinfo_entry` elements.
        unsafe { self.array(self.header().asinfo) }
    }

    pub fn cacheattr(&self) -> impl Iterator<Item = &CacheattrEntry> {
        // SAFETY: the cacheattr section holds `struct cacheattr_entry` elements.
        unsafe { self.array(self.header().cacheattr) }
    }

    pub fn intrinfo(&self) -> impl Iterator<Item = &IntrinfoEntry> {
        // SAFETY: the intrinfo section holds `struct intrinfo_entry` elements.
        unsafe { self.array(self.header().intrinfo) }
    }

    /// A NUL-terminated name from the strings section.
    pub fn string(&self, offset: u16) -> &str {
        let info = self.header().strings;
        if offset >= info.entry_size {
            return "?";
        }
        // SAFETY: the strings section lies inside the page.
        let bytes = unsafe {
            core::slice::from_raw_parts(
                self.0.cast::<u8>().wrapping_add((info.entry_off + offset) as usize),
                (info.entry_size - offset) as usize,
            )
        };
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        core::str::from_utf8(&bytes[..end]).unwrap_or("?")
    }

    /// Check what the kernel will read before it reads it: the version
    /// (`SYSPAGE_VERSION_MAJOR`), every section it uses inside the page,
    /// each fixed section at least as large as its `struct`, each array's
    /// elements at least as large as theirs (review CORE-12). A system page
    /// from another startup is refused here, rather than misread later.
    pub fn check(&self) -> Result<(), &'static str> {
        let h = self.header();
        if h.version.major != SYSPAGE_VERSION_MAJOR {
            return Err("another version (SYSPAGE_VERSION_MAJOR)");
        }
        let total = h.total_size as usize;
        let inside = |off: u16, size: u16| off as usize + size as usize <= total;
        let fixed = |info: EntryInfo, need: usize| {
            inside(info.entry_off, info.entry_size) && info.entry_size as usize >= need
        };
        let array = |info: ArrayInfo, need: usize| {
            inside(info.entry_off, info.entry_size) && (info.entry_size == 0 || info.element_size as usize >= need)
        };
        let checks = [
            ("system_private section out of the page or too small", fixed(h.system_private, size_of::<SystemPrivateEntry>())),
            ("qtime section out of the page or too small", fixed(h.qtime, size_of::<QtimeEntry>())),
            ("callout section out of the page or too small", fixed(h.callout, size_of::<CalloutEntry>())),
            ("smp section out of the page or too small", fixed(h.smp, size_of::<SmpEntry>())),
            ("strings section out of the page", inside(h.strings.entry_off, h.strings.entry_size)),
            ("asinfo out of the page or its elements too small", array(h.asinfo, size_of::<AsinfoEntry>())),
            ("cpuinfo out of the page or its elements too small", array(h.cpuinfo, size_of::<CpuinfoEntry>())),
            ("cacheattr out of the page or its elements too small", array(h.cacheattr, size_of::<CacheattrEntry>())),
            ("intrinfo out of the page or its elements too small", array(h.intrinfo, size_of::<IntrinfoEntry>())),
        ];
        match checks.iter().find(|(_, ok)| !ok) {
            Some((what, _)) => Err(what),
            None => Ok(()),
        }
    }

    /// Index of an asinfo entry from its byte offset in the section.
    pub fn asinfo_index(&self, offset: u16) -> Option<usize> {
        let size = self.header().asinfo.element_size;
        (offset != AS_NULL_OFF && size != 0).then(|| (offset / size) as usize)
    }
}

static SYSPAGE: crate::sync::InitOnce<Syspage> = crate::sync::InitOnce::new();

/// Remember the system page for code that is not handed it.
pub fn set(sp: Syspage) {
    let _ = SYSPAGE.set(sp);
}

pub fn get() -> Syspage {
    *SYSPAGE.get().expect("system page not set")
}

impl Syspage {
    /// A typed string (`typed_strings` section: { u32 type; char data[] },
    /// NUL-terminated, entries 4-byte aligned), e.g. `_CS_MACHINE`.
    pub fn typed_string(&self, ty: u32) -> Option<&str> {
        let info = self.header().typed_strings;
        // SAFETY: the section lies inside the system page.
        let b = unsafe {
            core::slice::from_raw_parts(
                self.0.cast::<u8>().wrapping_add(info.entry_off as usize),
                info.entry_size as usize,
            )
        };
        let mut off = 0;
        while off + 4 < b.len() {
            let t = u32::from_le_bytes(b[off..off + 4].try_into().unwrap());
            let s = &b[off + 4..];
            let n = s.iter().position(|&c| c == 0)?;
            if t == ty {
                return core::str::from_utf8(&s[..n]).ok();
            }
            off = (off + 4 + n + 1).next_multiple_of(4);
        }
        None
    }
}
