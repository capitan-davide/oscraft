//! The process calls.

use core::ffi::c_int;

use crate::kercall::thread_destroy_all;


/// OsCraft's addition to a FreeBSD process's auxiliary vector: its
/// process ID (`AT_OSCRAFT_PID`, `kernel/src/proc/loader.rs`). QNX's
/// libc reads the ID from the thread-local block the kernel builds, which
/// a FreeBSD process does not have.
const AT_OSCRAFT_PID: c_int = 1001;

/// The process's ID from the auxiliary vector (FreeBSD's
/// `_elf_aux_info()` passes an unknown tag through as ENOENT, so the
/// vector is scanned here): the first image's; a forked child learns
/// its own from its fork reply (`proc`).
pub fn aux_pid() -> c_int {
    aux_value(AT_OSCRAFT_PID).map_or(1, |v| v as c_int)
}

/// `getpid()`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_getpid() -> c_int {
    crate::proc::pid()
}

/// `issetugid()`: no program on OsCraft runs set-user-ID yet.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_issetugid() -> c_int {
    0
}

unsafe extern "C" {
    /// FreeBSD's `auxv.c`: the vector, which a static libc finds after
    /// the environment on the first `_elf_aux_info()` and the dynamic
    /// loader sets in libc (`set_program_var`); null in the loader's own
    /// copy of libsys, which never asks for the pid.
    static __elf_aux_vector: *const [u64; 2];
}

/// The value of tag `tag` in the auxiliary vector, if present.
pub fn aux_value(tag: c_int) -> Option<u64> {
    // A known tag first, so that a static libc finds its vector.
    let _ = crate::sysctl::ncpus();
    // SAFETY: the vector the kernel built, ended by AT_NULL, or null.
    let mut p = unsafe { __elf_aux_vector };
    if p.is_null() {
        return None;
    }
    loop {
        // SAFETY: within the vector, which AT_NULL ends.
        let [t, v] = unsafe { *p };
        if t == 0 {
            return None;
        }
        if t == tag as u64 {
            return Some(v);
        }
        // SAFETY: the next entry exists, as this one was not AT_NULL.
        p = unsafe { p.add(1) };
    }
}

/// Ends the process with `status`: destroys all its threads
/// (`ThreadDestroy(-1, -1, status)`); the call returns only if it
/// failed, so it is repeated.
pub fn exit_now(status: c_int) -> ! {
    loop {
        thread_destroy_all(status);
    }
}

/// `_exit(status)`: the syscall is named `_exit`, so its stub is
/// `__sys__exit` and its aliases `__exit` and `_exit`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys__exit(status: c_int) -> ! {
    exit_now(status)
}


/// `struct rlimit`: the soft and the hard limit.
#[repr(C)]
pub struct Rlimit {
    pub cur: i64,
    pub max: i64,
}

const RLIMIT_STACK: c_int = 3;
const RLIMIT_NOFILE: c_int = 8;
const RLIM_NLIMITS: c_int = 16;
const RLIM_INFINITY: i64 = i64::MAX;
const AT_USRSTACKLIM: c_int = 36;
/// procnto's default open files limit.
const OPEN_MAX: i64 = 1000;

/// `getrlimit(which, rlp)`: the stack's limit is the main stack's size
/// (AT_USRSTACKLIM, what the kernel mapped: superlocked memory does not
/// grow), the open files' procnto's default, the rest unlimited. The
/// process manager's own limits are not asked yet.
///
/// # Safety
///
/// `rlp` points to a `struct rlimit`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getrlimit(which: c_int, rlp: *mut Rlimit) -> c_int {
    if !(0..RLIM_NLIMITS).contains(&which) {
        return crate::errno::fail(proto::errno::EINVAL) as c_int;
    }
    let v = match which {
        RLIMIT_STACK => aux_value(AT_USRSTACKLIM).map_or(RLIM_INFINITY, |l| l as i64),
        RLIMIT_NOFILE => OPEN_MAX,
        _ => RLIM_INFINITY,
    };
    // SAFETY: the caller's guarantee.
    unsafe { rlp.write(Rlimit { cur: v, max: v }) };
    0
}
