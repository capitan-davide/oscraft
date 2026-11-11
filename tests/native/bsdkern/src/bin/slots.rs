//! Test of the slots: threads count in unsynchronised per-slot memory
//! inside critical sections; no count may be lost, as many threads as
//! slots run at once, and sections nest.

#[cfg(target_os = "freebsd")]
fn main() {
    use bsdkern::slot::{critical_enter_KBI as enter, critical_exit_KBI as exit, current, init};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    const SLOTS: usize = 4;
    const THREADS: usize = 8;
    const ROUNDS: u64 = 100_000;
    init(SLOTS);

    let start = Arc::new(Barrier::new(THREADS));
    let counts: Vec<_> = (0..THREADS)
        .map(|_| {
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                for _ in 0..ROUNDS {
                    enter();
                    enter();
                    let at = current().cast::<u64>();
                    // SAFETY: the slot is this thread's until exit(); a
                    // non-atomic add shows any overlap as a lost count.
                    unsafe { at.write_volatile(at.read_volatile() + 1) };
                    exit();
                    exit();
                }
            })
        })
        .collect();
    for t in counts {
        t.join().unwrap();
    }
    // Each slot's page, in turn: taking all four shows every total.
    let mut seen = std::collections::BTreeMap::new();
    let holders: Vec<_> = (0..SLOTS).map(|_| Arc::new(Barrier::new(2))).collect();
    let mut joins = Vec::new();
    for hold in holders.iter().cloned() {
        joins.push(std::thread::spawn(move || {
            enter();
            let at = current() as usize;
            // SAFETY: as above.
            let n = unsafe { (at as *const u64).read_volatile() };
            hold.wait();
            exit();
            (at, n)
        }));
    }
    std::thread::sleep(Duration::from_millis(200));
    for h in &holders {
        h.wait();
    }
    for j in joins {
        let (at, n) = j.join().unwrap();
        seen.insert(at, n);
    }
    let total: u64 = seen.values().sum();
    println!("bsdkern-slots: {} slots, total {}", seen.len(), total);

    // A pinned thread waits for its own slot, though others are free.
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    static PIN: AtomicBool = AtomicBool::new(false);
    extern "C" fn query() -> core::ffi::c_int {
        core::ffi::c_int::from(PIN.load(SeqCst))
    }
    bsdkern::slot::bsdkern_slot_pin_query(query);
    enter();
    let first = current();
    exit();
    PIN.store(true, SeqCst);
    let held = Arc::new(Barrier::new(2));
    let other = {
        let held = held.clone();
        std::thread::spawn(move || {
            PIN.store(false, SeqCst);
            for _ in 0..SLOTS * 2 {
                enter();
                exit();
            }
            held.wait();
        })
    };
    held.wait();
    other.join().unwrap();
    PIN.store(true, SeqCst);
    enter();
    let again = current();
    exit();
    println!("bsdkern-slots: pinned stays {}", first == again);
    let want = THREADS as u64 * ROUNDS;
    let ok = seen.len() == SLOTS && total == want && first == again;
    println!("bsdkern-slots: {}", if ok { "passed" } else { "FAIL" });
}

#[cfg(not(target_os = "freebsd"))]
fn main() {}
