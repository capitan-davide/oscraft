//! Test of the page arena: pages come backed, growth is on demand, freed
//! pages return, alignment holds, and contiguous runs stay whole.

#[cfg(target_os = "freebsd")]
fn main() {
    use bsdkern::mem::*;
    const PAGE: usize = 4096;
    let mut ok = true;
    let mut check = |what: &str, good: bool| {
        println!("bsdkern-mem: {} {what}", if good { "ok" } else { "FAIL" });
        ok &= good;
    };
    bsdkern_mem_init(104);
    check("nothing backed at first", backed() == 0);
    let a = bsdkern_pages_alloc(1, 1);
    check("one page", a != 0 && backed() == 512);
    // SAFETY: the pages are this test's.
    unsafe { (a as *mut u64).write(7) };
    let b = bsdkern_pages_alloc(3, 16);
    check("aligned", b != 0 && (b - a).is_multiple_of(16 * PAGE));
    bsdkern_pages_free(a, 1);
    check("a freed page returns", bsdkern_pages_alloc(1, 1) == a);
    let big = bsdkern_pages_alloc(2000, 1);
    check("a large run grows it", big != 0 && backed() == 2048);
    // SAFETY: as above.
    unsafe {
        core::ptr::write_bytes(big as *mut u8, 0x5a, 2000 * PAGE);
        check("all of it is there", *((big + 1999 * PAGE) as *const u8) == 0x5a);
    }
    let mut table = 0;
    let base = bsdkern_mem_base(&raw mut table);
    // SAFETY: the page's vm_page is backed with the page.
    unsafe { ((table + 104 * ((big - base) / PAGE)) as *mut u64).write(1) };
    check("its vm_page is there", true);
    println!("bsdkern-mem: {}", if ok { "passed" } else { "FAILED" });
}

#[cfg(not(target_os = "freebsd"))]
fn main() {}
