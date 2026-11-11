//! The system page, read-only in every process: what startup found of the
//! board, here the address-space entries (`asinfo`), such as the device
//! tree startup leaves in memory (`"fdt"`).

use core::ffi::{CStr, c_char, c_int};

use kercall::neutrino::misc::__SysCpupageGet;

/// `CPUPAGE_SYSPAGE` (`<sys/syspage.h>`): the system page's address.
const CPUPAGE_SYSPAGE: c_int = 2;
/// Offsets in `struct syspage_entry` (QNX 8 `<sys/syspage.h>`).
const STRINGS: usize = 32;
const ASINFO: usize = 200;
/// `struct asinfo_entry`: start, end (u64), owner, name, attr, priority (u16).
const AS_NAME: usize = 18;

fn u16_at(base: *const u8, off: usize) -> u16 {
    // SAFETY: inside the system page, which is mapped read-only for good.
    unsafe { base.add(off).cast::<u16>().read_unaligned() }
}

fn u64_at(base: *const u8, off: usize) -> u64 {
    // SAFETY: as above.
    unsafe { base.add(off).cast::<u64>().read_unaligned() }
}

/// The first address-space entry named `name`: its first and last
/// physical addresses in `*start` and `*end`. False if there is none.
///
/// # Safety
///
/// `name` is a C string; `start` and `end` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_syspage_as(name: *const c_char, start: *mut u64, end: *mut u64) -> bool {
    let sp = __SysCpupageGet(CPUPAGE_SYSPAGE) as *const u8;
    if sp.is_null() {
        return false;
    }
    let strings = sp.wrapping_add(usize::from(u16_at(sp, STRINGS)));
    let (off, size, step) = (u16_at(sp, ASINFO), u16_at(sp, ASINFO + 2), u16_at(sp, ASINFO + 4));
    if step == 0 {
        return false;
    }
    // SAFETY: the caller's guarantee.
    let want = unsafe { CStr::from_ptr(name) };
    for i in 0..usize::from(size / step) {
        let e = sp.wrapping_add(usize::from(off) + i * usize::from(step));
        let n = strings.wrapping_add(usize::from(u16_at(e, AS_NAME)));
        // SAFETY: names in the strings section are NUL-terminated.
        if unsafe { CStr::from_ptr(n.cast()) } == want {
            // SAFETY: the caller's guarantee.
            unsafe {
                start.write(u64_at(e, 0));
                end.write(u64_at(e, 8));
            }
            return true;
        }
    }
    false
}
