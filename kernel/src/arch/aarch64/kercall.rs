//! Making kernel calls from kernel code, with the same instruction sequence
//! as the SDP's libc `_r` stubs: success returns x0, failure returns
//! -errno (the kernel resumes at the `neg`).

use core::arch::global_asm;

global_asm!(
    ".text",
    ".global ker_call_r",
    "ker_call_r:",
    "    mov w8, w6",
    "    svc #0x51",
    "    ret",
    "    neg x0, x0",
    "    ret",
);

unsafe extern "C" {
    fn ker_call_r(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, number: u32) -> i64;
}

/// Kernel call `number` with up to six arguments; `Err(errno)` on failure.
pub fn call(number: u32, args: [u64; 6]) -> Result<u64, u32> {
    // SAFETY: the stub only traps into the kernel's own call dispatcher.
    let r = unsafe { ker_call_r(args[0], args[1], args[2], args[3], args[4], args[5], number) };
    if r < 0 {
        Err((-r) as u32)
    } else {
        Ok(r as u64)
    }
}
