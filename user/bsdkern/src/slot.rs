//! CPUs, as slots. FreeBSD's per-CPU data is safe because a critical
//! section stops preemption; a user process cannot do that, and on a
//! real-time kernel should not. As in `io-sock` (`__critical_enter`,
//! `cpu_pcpu_init`), a "CPU" here is a `struct pcpu` with a lock: entering
//! a critical section takes a slot, the one last used if it is free, else
//! any free one, else waits. So as many threads as there are CPUs are in
//! critical sections at once, each alone with its slot's data, and the
//! thread may still be preempted, with nobody else touching that slot.
//!
//! Unlike `io-sock`, the mutexes inherit priority by request, so that a
//! low-priority holder cannot keep a high-priority thread waiting behind
//! a third.

use core::cell::Cell;
use core::ffi::{c_int, c_void};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::OnceLock;

/// `struct pcpu`'s room: `sys/arm64/include/pcpu.h`'s `PCPU_MD_FIELDS`
/// pads it to a page's fraction; a page is ample, as in `io-sock`.
const PCPU_SIZE: usize = 4096;
/// `PTHREAD_PRIO_INHERIT`.
const PRIO_INHERIT: c_int = 1;

type Mutex = *mut c_void;

unsafe extern "C" {
    fn pthread_mutexattr_init(attr: *mut *mut c_void) -> c_int;
    fn pthread_mutexattr_setprotocol(attr: *mut *mut c_void, protocol: c_int) -> c_int;
    fn pthread_mutexattr_destroy(attr: *mut *mut c_void) -> c_int;
    fn pthread_mutex_init(m: *mut Mutex, attr: *const *mut c_void) -> c_int;
    fn pthread_mutex_trylock(m: *mut Mutex) -> c_int;
    fn pthread_mutex_lock(m: *mut Mutex) -> c_int;
    fn pthread_mutex_unlock(m: *mut Mutex) -> c_int;
    fn bsdkern_set_curpcpu(pc: *mut c_void);
    fn bsdkern_get_curpcpu() -> *mut c_void;
}

#[repr(C, align(4096))]
struct Pcpu([u8; PCPU_SIZE]);

struct Slot {
    pcpu: *mut Pcpu,
    lock: *mut Mutex,
}

// SAFETY: both point to memory that lives as long as the process; the
// lock guards what is behind `pcpu`.
unsafe impl Sync for Slot {}
// SAFETY: as above.
unsafe impl Send for Slot {}

static SLOTS: OnceLock<Box<[Slot]>> = OnceLock::new();

thread_local! {
    /// How deep in critical sections this thread is.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    /// The slot it holds or last held.
    static HOME: Cell<usize> = const { Cell::new(0) };
    /// The slot held now: `sched_bind()` can move `HOME` meanwhile.
    static HELD: Cell<usize> = const { Cell::new(0) };
    /// Tied to `HOME` by `sched_bind()`.
    static BOUND: Cell<bool> = const { Cell::new(false) };
}

/// Makes `count` slots; call once, before any critical section.
pub fn init(count: usize) {
    let slots = (0..count.max(1)).map(|_| {
        // SAFETY: zeroed memory is a valid array of bytes.
        let pcpu = Box::into_raw(unsafe { Box::<Pcpu>::new_zeroed().assume_init() });
        let lock = Box::into_raw(Box::new(null_mut::<c_void>()));
        let mut attr = null_mut();
        // SAFETY: `attr` and `lock` are valid for these calls.
        unsafe {
            assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
            assert_eq!(pthread_mutexattr_setprotocol(&raw mut attr, PRIO_INHERIT), 0);
            assert_eq!(pthread_mutex_init(lock, &raw const attr), 0);
            pthread_mutexattr_destroy(&raw mut attr);
        }
        Slot { pcpu, lock }
    });
    assert!(SLOTS.set(slots.collect()).is_ok(), "bsdkern: slots made twice");
}

fn slots() -> &'static [Slot] {
    SLOTS.get().expect("bsdkern: no slots")
}

/// Tells whether this thread is pinned: `kern/sched.c` reads `td_pinned`.
static PIN_QUERY: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

/// Sets the function that says whether this thread is pinned.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_slot_pin_query(query: extern "C" fn() -> c_int) {
    PIN_QUERY.store(query as *mut c_void, Ordering::Relaxed);
}

fn pinned() -> bool {
    let query = PIN_QUERY.load(Ordering::Relaxed);
    if query.is_null() {
        return false;
    }
    // SAFETY: only `bsdkern_slot_pin_query()` stores it, a function so typed.
    let query: extern "C" fn() -> c_int = unsafe { core::mem::transmute(query) };
    query() != 0
}

fn take() -> usize {
    let all = slots();
    let home = HOME.get();
    // A pinned thread keeps its slot, or per-CPU data would change under it
    // (`__critical_enter` looks for another only if `td_pinned` is 0).
    if BOUND.get() || pinned() {
        // SAFETY: an initialised mutex.
        assert_eq!(unsafe { pthread_mutex_lock(all[home].lock) }, 0);
        return home;
    }
    for i in 0..all.len() {
        let at = (home + i) % all.len();
        // SAFETY: an initialised mutex.
        if unsafe { pthread_mutex_trylock(all[at].lock) } == 0 {
            return at;
        }
    }
    // SAFETY: an initialised mutex.
    assert_eq!(unsafe { pthread_mutex_lock(all[home].lock) }, 0);
    home
}

/// FreeBSD's `critical_enter()`.
#[unsafe(no_mangle)]
pub extern "C" fn critical_enter_KBI() {
    let depth = DEPTH.get();
    DEPTH.set(depth + 1);
    if depth == 0 {
        let at = take();
        HOME.set(at);
        HELD.set(at);
        // SAFETY: a slot's `struct pcpu`, which this thread now holds.
        unsafe { bsdkern_set_curpcpu(slots()[at].pcpu.cast()) };
    }
}

/// FreeBSD's `critical_exit()`.
#[unsafe(no_mangle)]
pub extern "C" fn critical_exit_KBI() {
    let depth = DEPTH.get();
    if depth == 0 {
        let caller: usize;
        // SAFETY: reads a register.
        unsafe { core::arch::asm!("mov {}, x29", out(reg) caller) };
        // SAFETY: frame pointers are kept in FreeBSD's code; for the report.
        let (a, b) = unsafe { (*((caller + 8) as *const usize), *(*(caller as *const usize) as *const usize).add(1)) };
        panic!("bsdkern: critical_exit outside a critical section, from {a:#x} {b:#x}");
    }
    DEPTH.set(depth - 1);
    if depth == 1 {
        // SAFETY: the mutex this thread took.
        assert_eq!(unsafe { pthread_mutex_unlock(slots()[HELD.get()].lock) }, 0);
    }
}

/// This thread's `struct pcpu`, as `get_pcpu()` reads it.
pub fn current() -> *mut u8 {
    // SAFETY: reads a thread-local pointer.
    unsafe { bsdkern_get_curpcpu() }.cast()
}

/// `sched_bind()`: this thread's critical sections use slot `cpu`, or any
/// again if negative, as `io-sock`'s. FreeBSD's callers hold the thread's
/// lock, so a critical section: it counts from the next one.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_bind(cpu: c_int) {
    BOUND.set(cpu >= 0);
    if cpu >= 0 {
        HOME.set(cpu as usize % slots().len());
    }
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// `bsdkern_pcpus_init()`: the slots, for the C part.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_slots_init(cpus: c_int) {
    init(cpus as usize);
}

/// Slot `cpu`'s `struct pcpu`.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_slot_pcpu(cpu: c_int) -> *mut c_void {
    slots()[cpu as usize].pcpu.cast()
}

/// Gives a new thread a home slot, spread over the CPUs, so that its
/// `curpcpu` is valid from the start.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_slot_attach() {
    let home = NEXT.fetch_add(1, Ordering::Relaxed) % slots().len();
    HOME.set(home);
    // SAFETY: a slot's `struct pcpu`; reading it outside a critical section
    // is as safe as on FreeBSD.
    unsafe { bsdkern_set_curpcpu(slots()[home].pcpu.cast()) };
}
