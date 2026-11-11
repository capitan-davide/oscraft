//! `sysctl()`: FreeBSD's tree of kernel values. OsCraft has no such tree;
//! the values programs and libc ask for at start are answered here from
//! what the kernel put in the auxiliary vector (`_elf_aux_info()`, FreeBSD's
//! `auxv.c` in libsys). Anything else is ENOENT, reported once.

use core::ffi::{c_char, c_int, c_void};

use proto::errno::{EINVAL, ENOENT, ENOMEM};

use crate::errno::{fail, not_implemented};

unsafe extern "C" {
    /// FreeBSD's `auxv.c`, built into libsys with this crate (and into
    /// the dynamic loader's private copy of it).
    pub fn _elf_aux_info(aux: c_int, buf: *mut c_void, buflen: c_int) -> c_int;
}

const CTL_KERN: c_int = 1;
const CTL_HW: c_int = 6;
const KERN_OSRELDATE: c_int = 24;
const KERN_USRSTACK: c_int = 33;
const HW_NCPU: c_int = 3;
const HW_PAGESIZE: c_int = 7;
const AT_PAGESZ: c_int = 6;
const AT_OSRELDATE: c_int = 18;
const AT_NCPUS: c_int = 19;
const AT_USRSTACKBASE: c_int = 35;

fn aux_int(tag: c_int) -> Option<c_int> {
    let mut v: c_int = 0;
    // SAFETY: an int's buffer, its size given.
    (unsafe { _elf_aux_info(tag, (&raw mut v).cast(), size_of::<c_int>() as c_int) } == 0).then_some(v)
}

/// The number of CPUs, from the auxiliary vector.
pub fn ncpus() -> Option<c_int> {
    aux_int(AT_NCPUS)
}

/// The kernel's free memory in bytes: the size `stat("/proc")` reports.
fn free_memory() -> Option<u64> {
    let mut st = crate::stat::Stat::default();
    // SAFETY: a literal path and a stat on this stack.
    (unsafe { crate::fs::__sys_fstatat(-100, c"/proc".as_ptr(), &mut st, 0) } == 0 && st.st_size > 0).then_some(st.st_size as u64)
}

/// A value: an int, or a long (`kern.usrstack`).
enum Value {
    Int(c_int),
    Long(u64),
    Str(&'static [u8]),
}

/// What the system says of itself (`uname`): the kernel is OsCraft's,
/// the userland FreeBSD's.
const OSTYPE: &[u8] = b"OsCraft\0";
const OSRELEASE: &[u8] = b"0.1-FREEBSD15\0";
const VERSION: &[u8] = b"OsCraft kernel, FreeBSD 15 userland\0";
const HOSTNAME: &[u8] = b"oscraft\0";
const MACHINE: &[u8] = b"arm64\0";
const MACHINE_ARCH: &[u8] = b"aarch64\0";
const KERN_OSTYPE: c_int = 1;
const KERN_OSRELEASE: c_int = 2;
const KERN_VERSION: c_int = 4;
const KERN_HOSTNAME: c_int = 10;
const HW_MACHINE: c_int = 1;
const HW_PHYSMEM: c_int = 5;
const HW_USERMEM: c_int = 6;
const HW_REALMEM: c_int = 18;
const HW_MACHINE_ARCH: c_int = 11;

fn aux_long(tag: c_int) -> Option<u64> {
    let mut v: u64 = 0;
    // SAFETY: a long's buffer, its size given.
    (unsafe { _elf_aux_info(tag, (&raw mut v).cast(), size_of::<u64>() as c_int) } == 0).then_some(v)
}

/// The value of a name, or None for one not known here.
fn lookup(name: &[c_int]) -> Option<Value> {
    match *name {
        [CTL_HW, HW_NCPU] => aux_int(AT_NCPUS).map(Value::Int),
        [CTL_HW, HW_PAGESIZE] => aux_int(AT_PAGESZ).map(Value::Int),
        [CTL_KERN, KERN_OSRELDATE] => aux_int(AT_OSRELDATE).map(Value::Int),
        [CTL_KERN, KERN_USRSTACK] => aux_long(AT_USRSTACKBASE).map(Value::Long),
        // `ARG_MAX` of `<sys/syslimits.h>`.
        [CTL_KERN, KERN_ARGMAX] => Some(Value::Int(2 * 256 * 1024)),
        [CTL_KERN, KERN_OSTYPE] => Some(Value::Str(OSTYPE)),
        [CTL_KERN, KERN_OSRELEASE] => Some(Value::Str(OSRELEASE)),
        [CTL_KERN, KERN_VERSION] => Some(Value::Str(VERSION)),
        [CTL_KERN, KERN_HOSTNAME] => Some(Value::Str(HOSTNAME)),
        [CTL_HW, HW_MACHINE] => Some(Value::Str(MACHINE)),
        // The memory: what the kernel has free, which `/proc`'s size
        // tells (the system page's total is not read yet).
        [CTL_HW, HW_PHYSMEM | HW_USERMEM | HW_REALMEM] => free_memory().map(Value::Long),
        [CTL_HW, HW_MACHINE_ARCH] => Some(Value::Str(MACHINE_ARCH)),
        _ => None,
    }
}

/// Whether `a` and `b` are the same bytes (by hand: a slice comparison
/// would call `bcmp`, which the dynamic loader's libc lacks).
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
}

fn by_name(name: &[u8]) -> Option<Value> {
    if same(name, b"hw.ncpu") {
        lookup(&[CTL_HW, HW_NCPU])
    } else if same(name, b"hw.pagesize") {
        lookup(&[CTL_HW, HW_PAGESIZE])
    } else if same(name, b"kern.fscale") {
        Some(Value::Int(2048))
    } else if same(name, b"kern.ccpu") {
        Some(Value::Int(0))
    } else if same(name, b"kern.pid_max") {
        Some(Value::Int(0x7fff_ffff))
    } else if same(name, b"hw.availpages") {
        free_memory().map(|m| Value::Long(m / 4096))
    } else if same(name, b"kern.osreldate") {
        lookup(&[CTL_KERN, KERN_OSRELDATE])
    } else if same(name, b"kern.usrstack") {
        lookup(&[CTL_KERN, KERN_USRSTACK])
    } else if same(name, b"kern.ostype") {
        lookup(&[CTL_KERN, KERN_OSTYPE])
    } else if same(name, b"kern.osrelease") {
        lookup(&[CTL_KERN, KERN_OSRELEASE])
    } else if same(name, b"kern.version") {
        lookup(&[CTL_KERN, KERN_VERSION])
    } else if same(name, b"kern.hostname") {
        lookup(&[CTL_KERN, KERN_HOSTNAME])
    } else if same(name, b"kern.sched.cpusetsize") {
        // sizeof(cpuset_t), 256 CPUs in bits: what libthr sizes affinity sets with.
        Some(Value::Int(32))
    } else if same(name, b"hw.availpages") {
        // `sysconf(_SC_PHYS_PAGES)`: a long of pages.
        free_memory().zip(aux_int(AT_PAGESZ)).map(|(m, p)| Value::Long(m / p.max(1) as u64))
    } else if same(name, b"hw.physmem") {
        lookup(&[CTL_HW, HW_PHYSMEM])
    } else if same(name, b"hw.usermem") {
        lookup(&[CTL_HW, HW_USERMEM])
    } else if same(name, b"hw.realmem") {
        lookup(&[CTL_HW, HW_REALMEM])
    } else if same(name, b"hw.machine") {
        lookup(&[CTL_HW, HW_MACHINE])
    } else if same(name, b"hw.machine_arch") {
        lookup(&[CTL_HW, HW_MACHINE_ARCH])
    } else {
        None
    }
}

/// Answers `value` into the caller's `old` buffer of `*oldlenp` bytes.
///
/// # Safety
///
/// `old` and `oldlenp` are as `sysctl()` takes them.
unsafe fn answer(value: Option<Value>, old: *mut c_void, oldlenp: *mut usize, what: &str) -> c_int {
    let Some(v) = value else {
        not_implemented(what);
        return fail(ENOENT) as c_int;
    };
    let (small, n, bytes): ([u8; 8], usize, &[u8]) = match v {
        Value::Int(i) => {
            let mut b = [0; 8];
            b[..4].copy_from_slice(&i.to_ne_bytes());
            (b, 4, &[])
        }
        Value::Long(l) => (l.to_ne_bytes(), 8, &[]),
        Value::Str(s) => ([0; 8], s.len(), s),
    };
    let bytes: &[u8] = if bytes.is_empty() { &small[..n] } else { bytes };
    if oldlenp.is_null() {
        return 0;
    }
    // SAFETY: the caller's guarantee.
    let len = unsafe { *oldlenp };
    if !old.is_null() {
        if len < n {
            return fail(ENOMEM) as c_int;
        }
        // SAFETY: `old` has at least `n` bytes' room.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), old.cast::<u8>(), n) };
    }
    // SAFETY: as above.
    unsafe { *oldlenp = n };
    0
}

/// `__sysctl(name, namelen, old, oldlenp, new, newlen)`.
///
/// # Safety
///
const KERN_ARGMAX: c_int = 8;

/// A variable of process 1's, or `None` for `io-sock` to try: the kernel's
/// are in the same `sysctl` message, on its channel.
///
/// # Safety
///
/// As `sysctl()`.
unsafe fn via_kernel(name: &[c_int], old: *mut c_void, oldlenp: *mut usize, new: *const c_void, newlen: usize) -> Option<c_int> {
    // SAFETY: the caller's guarantee.
    let oldlen = if oldlenp.is_null() { 0 } else { unsafe { *oldlenp } };
    let fd = crate::kercall::SYSMGR_COID;
    // SAFETY: as above.
    match unsafe { crate::socket::sysctl::ask(fd, name, old, oldlen, new, newlen) } {
        Ok(found) => {
            if !oldlenp.is_null() {
                // SAFETY: as above.
                unsafe { *oldlenp = found.len };
            }
            Some(if found.cut { fail(proto::errno::ENOMEM) as c_int } else { 0 })
        }
        // Under a number of process 1's (its own block from 0x7000, a
        // few), it is the answer; any other is the network stack's, whose
        // numbers made at run time count down from 0x7fffffff (a module's).
        Err(proto::errno::ENOENT) if name.get(1).is_none_or(|&n| !(0x7000..0x7400).contains(&n)) => None,
        Err(e) => Some(fail(e) as c_int),
    }
}

/// As `sysctl()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys___sysctl(
    name: *const c_int,
    namelen: u32,
    old: *mut c_void,
    oldlenp: *mut usize,
    new: *const c_void,
    newlen: usize,
) -> c_int {
    if name.is_null() || namelen == 0 || namelen > 24 {
        return fail(EINVAL) as c_int;
    }
    // SAFETY: the caller's guarantee: `namelen` ints at `name`.
    let name = unsafe { core::slice::from_raw_parts(name, namelen as usize) };
    if let Some(v) = lookup(name) {
        // SAFETY: the caller's guarantee.
        return unsafe { answer(Some(v), old, oldlenp, "sysctl (by number)") };
    }
    // SAFETY: the caller's guarantee.
    if let Some(r) = unsafe { via_kernel(name, old, oldlenp, new, newlen) } {
        return r;
    }
    // SAFETY: the caller's guarantee.
    unsafe { via_socket(name, old, oldlenp, new, newlen) }
}

/// A variable the kernel has not: `io-sock`'s (`socket::sysctl`).
///
/// # Safety
///
/// As `sysctl()`.
unsafe fn via_socket(name: &[c_int], old: *mut c_void, oldlenp: *mut usize, new: *const c_void, newlen: usize) -> c_int {
    // SAFETY: the caller's guarantee.
    let oldlen = if oldlenp.is_null() { 0 } else { unsafe { *oldlenp } };
    // SAFETY: as above.
    match unsafe { crate::socket::sysctl::via_socket(name, old, oldlen, new, newlen) } {
        Ok(n) => {
            if !oldlenp.is_null() {
                // SAFETY: as above.
                unsafe { *oldlenp = n };
            }
            0
        }
        Err(e) => fail(e) as c_int,
    }
}


/// `__sysctlbyname(name, namelen, old, oldlenp, new, newlen)`.
///
/// # Safety
///
/// As `sysctlbyname()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys___sysctlbyname(
    name: *const c_char,
    namelen: usize,
    old: *mut c_void,
    oldlenp: *mut usize,
    new: *const c_void,
    newlen: usize,
) -> c_int {
    if name.is_null() {
        return fail(EINVAL) as c_int;
    }
    // SAFETY: the caller's guarantee: `namelen` bytes at `name`.
    let bytes = unsafe { core::slice::from_raw_parts(name.cast::<u8>(), namelen) };
    if let Some(v) = by_name(bytes) {
        let what = core::str::from_utf8(bytes).unwrap_or("sysctl (by name)");
        // SAFETY: the caller's guarantee.
        return unsafe { answer(Some(v), old, oldlenp, what) };
    }
    // The numbers of the name (`sysctl.name2oid`, FreeBSD's
    // sysctlnametomib()), then the variable: process 1's, or io-sock's.
    const NAME2OID: [c_int; 2] = [0, 3];
    const CTL_MAXNAME: usize = 24;
    let mut oid = [0 as c_int; CTL_MAXNAME];
    let mut oidlen = size_of_val(&oid);
    // SAFETY: the numbers' buffer, its size, and the name's bytes.
    if unsafe { via_kernel(&NAME2OID, oid.as_mut_ptr().cast(), &raw mut oidlen, name.cast(), namelen) } == Some(0) {
        let n = oidlen / size_of::<c_int>();
        // SAFETY: the caller's guarantee.
        return unsafe { __sys___sysctl(oid.as_ptr(), n as u32, old, oldlenp, new, newlen) };
    }
    // SAFETY: the numbers' buffer and the name's bytes.
    let n = match unsafe { crate::socket::sysctl::via_socket(&NAME2OID, oid.as_mut_ptr().cast(), size_of_val(&oid), name.cast(), namelen) } {
        Ok(n) => n / size_of::<c_int>(),
        Err(e) => return fail(e) as c_int,
    };
    // SAFETY: the caller's guarantee.
    unsafe { via_socket(&oid[..n.min(CTL_MAXNAME)], old, oldlenp, new, newlen) }
}
