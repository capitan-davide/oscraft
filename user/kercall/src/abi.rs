//! QNX's C types and structures, with the layouts of the QNX SDP 8.0's
//! headers. The constant assertions at the end of each structure check
//! the offsets the library relies on.

use core::ffi::{c_int, c_long, c_uint, c_void};

/// `ssize_t`, `<sys/types.h>`.
pub type SsizeT = c_long;
/// `pid_t`: a process ID.
pub type PidT = i32;
/// `rcvid_t`: a receive ID, which names a message's sender to a server.
pub type RcvidT = i64;
/// `clockid_t`: a clock's ID.
pub type ClockidT = c_int;
/// `timer_t`: a timer's ID.
pub type TimerT = c_int;

/// The thread's local storage, `struct _thread_local_storage`
/// (`<sys/storage.h>`). The kernel builds it at the top of each thread's
/// stack and fills in the identity fields before the thread runs.
#[repr(C)]
pub struct ThreadLocalStorage {
    pub stacksize: usize,
    pub arg: *mut c_void,
    /// Where this thread's `errno` is. The kernel points it at `errval`
    /// below; the library points the main thread's at the global `errno`.
    pub errptr: *mut c_int,
    pub errval: c_int,
    pub flags: c_uint,
    pub pid: c_int,
    pub tid: c_int,
    /// The value the thread stores in the mutexes it locks.
    pub owner: c_uint,
    pub stackaddr: *mut c_void,
    pub guardsize: c_uint,
    pub numkeys: c_uint,
    pub keydata: *mut *mut c_void,
    pub cleanup: *mut c_void,
    pub pt_tsd: *mut c_void,
    /// The dynamic loader's own data for the thread.
    pub ldd_data: *mut c_void,
    pub sigaction: *mut c_void,
    pub proc_thread_chid: c_int,
    pub reserved: c_int,
    pub cxa_atexit_data: *mut c_void,
}

const _: () = {
    use core::mem::{offset_of, size_of};
    assert!(offset_of!(ThreadLocalStorage, errptr) == 0x10);
    assert!(offset_of!(ThreadLocalStorage, pid) == 0x20);
    assert!(offset_of!(ThreadLocalStorage, tid) == 0x24);
    assert!(offset_of!(ThreadLocalStorage, stackaddr) == 0x30);
    assert!(offset_of!(ThreadLocalStorage, ldd_data) == 0x58);
    assert!(size_of::<ThreadLocalStorage>() == 0x78);
};

/// The thread control block, the 16 bytes the thread pointer (TPIDR_EL0)
/// points to: AArch64's ELF TLS layout, with QNX's thread local storage
/// in its second word.
#[repr(C)]
pub struct Tcb {
    /// Reserved for the dynamic thread vector (dynamic TLS).
    pub dtv: *mut c_void,
    pub tls: *mut ThreadLocalStorage,
}

/// One part of a message, `iov_t` (`<sys/neutrino.h>`).
#[repr(C)]
pub struct Iov {
    pub base: *const c_void,
    pub len: usize,
}

impl Iov {
    /// A part that is the bytes of `v`, a message or its header.
    pub fn of<T: proto::pod::Pod>(v: &T) -> Self {
        Iov {
            base: (v as *const T).cast(),
            len: size_of::<T>(),
        }
    }

    /// A part for a reply into `v`.
    pub fn of_mut<T: proto::pod::Pod>(v: &mut T) -> Self {
        Iov {
            base: (v as *mut T).cast_const().cast(),
            len: size_of::<T>(),
        }
    }

    /// A part of `len` bytes at `base`.
    pub fn raw(base: *const c_void, len: usize) -> Self {
        Iov { base, len }
    }
}

/// Error numbers (`<errno.h>`).
pub mod errno {
    pub use proto::errno::*;
}

/// `SYSMGR_COID`: every process's connection to the process manager,
/// which the memory and system managers answer on too.
pub use proto::SYSMGR_COID;
