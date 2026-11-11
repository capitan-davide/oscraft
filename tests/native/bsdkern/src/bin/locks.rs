//! Test of `mtx`, `rw` and `sx`, called as FreeBSD's code calls them.

#[cfg(target_os = "freebsd")]
fn main() {
    use bsdkern::lock::*;
    use core::ptr::{null, null_mut};
    use std::sync::Arc;

    unsafe extern "C" {
        fn bsdkern_set_curthread(td: *mut core::ffi::c_void);
    }

    #[repr(C, align(8))]
    struct Lock([usize; 4]);
    struct Shared(std::cell::UnsafeCell<Lock>, std::cell::UnsafeCell<u64>);
    // SAFETY: the test's locks guard the cell.
    unsafe impl Sync for Shared {}
    // SAFETY: as above.
    unsafe impl Send for Shared {}

    const THREADS: usize = 6;
    const ROUNDS: u64 = 50_000;
    let new = || Arc::new(Shared(std::cell::UnsafeCell::new(Lock([0; 4])), std::cell::UnsafeCell::new(0)));
    let word = |s: &Shared| s.0.get().cast::<usize>().wrapping_add(3);
    let run = |s: Arc<Shared>, body: fn(*mut usize)| {
        let threads: Vec<_> = (0..THREADS)
            .map(|i| {
                let s = s.clone();
                std::thread::spawn(move || {
                    // SAFETY: any distinct non-null value serves as a thread.
                    unsafe { bsdkern_set_curthread((0x1000 + i * 16) as *mut _) };
                    let c = s.0.get().cast::<usize>().wrapping_add(3);
                    for _ in 0..ROUNDS {
                        body(c);
                        // SAFETY: `body` left the lock held by this thread.
                        unsafe {
                            *s.1.get() += 1;
                            *s.1.get() -= 1;
                            *s.1.get() += 1;
                        }
                        // The second half of the pair, keyed on the word.
                        release(c);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        // SAFETY: all have ended.
        unsafe { *s.1.get() }
    };
    static mut KIND: u8 = 0;
    fn release(c: *mut usize) {
        // SAFETY: this thread holds the lock.
        unsafe {
            match KIND {
                0 => __mtx_unlock_flags(c, 0, core::ptr::null(), 0),
                1 => _rw_wunlock_cookie(c, core::ptr::null(), 0),
                _ => _sx_xunlock(c.wrapping_sub(3).cast(), core::ptr::null(), 0),
            }
        }
    }
    let want = THREADS as u64 * ROUNDS;
    let mut ok = true;
    init();

    // SAFETY: the words are fresh, in the structures' places.
    unsafe {
        let m = new();
        _mtx_init(word(&m), c"m".as_ptr(), null(), 0);
        let n = run(m, |c| __mtx_lock_flags(c, 0, core::ptr::null(), 0));
        println!("bsdkern-locks: mtx {n}");
        ok &= n == want;

        KIND = 1;
        let r = new();
        _rw_init_flags(word(&r), c"r".as_ptr(), 0);
        let n = run(r, |c| _rw_wlock_cookie(c, core::ptr::null(), 0));
        println!("bsdkern-locks: rw {n}");
        ok &= n == want;

        KIND = 2;
        let s = new();
        sx_init_flags(s.0.get().cast(), c"s".as_ptr(), 0);
        let n = run(s, |c| {
            _sx_xlock(c.wrapping_sub(3).cast(), 0, core::ptr::null(), 0);
        });
        println!("bsdkern-locks: sx {n}");
        ok &= n == want;

        // Recursion, ownership, trylock, upgrade and downgrade.
        bsdkern_set_curthread(0x2000 as *mut _);
        let l = new();
        let c = word(&l);
        _mtx_init(c, c"r".as_ptr(), null(), 4);
        __mtx_lock_flags(c, 0, null(), 0);
        __mtx_lock_flags(c, 0, null(), 0);
        let held = c.read() == 0x2000;
        __mtx_unlock_flags(c, 0, null(), 0);
        let still = c.read() == 0x2000;
        __mtx_unlock_flags(c, 0, null(), 0);
        println!("bsdkern-locks: recursive {held} {still} {}", c.read());
        ok &= held && still && c.read() == 0;

        let u = new();
        let c = word(&u);
        _rw_init_flags(c, c"u".as_ptr(), 0);
        __rw_rlock(c, null(), 0);
        let one = __rw_try_upgrade(c, null(), 0);
        let owned = _rw_wowned(c);
        __rw_downgrade(c, null(), 0);
        let other_reader = __rw_try_rlock(c, null(), 0);
        let blocked = __rw_try_upgrade(c, null(), 0);
        let no_writer = __rw_try_wlock(c, null(), 0);
        _rw_runlock_cookie(c, null(), 0);
        _rw_runlock_cookie(c, null(), 0);
        let free = __rw_try_wlock(c, null(), 0);
        _rw_wunlock_cookie(c, null(), 0);
        println!("bsdkern-locks: rw {one} {owned} {other_reader} {blocked} {no_writer} {free}");
        ok &= (one, owned, other_reader, blocked, no_writer, free) == (1, 1, 1, 0, 0, 1);
    }
    let _ = null_mut::<u8>();
    println!("bsdkern-locks: {}", if ok { "passed" } else { "FAIL" });
}

#[cfg(not(target_os = "freebsd"))]
fn main() {}
