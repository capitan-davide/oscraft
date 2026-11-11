//! First OsCraft user program. Runs at EL0 in its own process and talks to
//! procnto only through QNX kernel calls on `SYSMGR_COID`.

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::fmt::Write;

/// Connection to procnto every process starts with (`<sys/neutrino.h>`).
const SYSMGR_COID: u64 = 0x4000_0000;
const __KER_MSG_SENDV: u32 = 11;
const __KER_THREAD_DESTROY: u32 = 47;
/// OsCraft bring-up message understood by procnto's sysmgr: print text.
const OSCRAFT_DEBUG_PRINT: u16 = 0xf000;

global_asm!(
    ".section .text.start, \"ax\"",
    ".global _start",
    "_start:",
    "    bl main",
    "    mov x0, #0",
    "    mov w8, #47", // __KER_THREAD_DESTROY
    "    svc #0x51",
    "    b .",
);

/// A kernel call with libc's convention: on failure the kernel resumes one
/// instruction later (skipping the branch) with the errno in x0.
fn kercall(n: u32, a: [u64; 5]) -> Result<u64, u64> {
    let (r, failed): (u64, u64);
    // SAFETY: traps into the kernel; only x0-x8 are involved.
    unsafe {
        asm!(
            "mov x9, #0",
            "svc #0x51",
            "b 1f",
            "mov x9, #1",
            "1:",
            in("w8") n,
            inlateout("x0") a[0] => r,
            in("x1") a[1], in("x2") a[2], in("x3") a[3], in("x4") a[4],
            out("x9") failed,
        );
    }
    if failed == 0 { Ok(r) } else { Err(r) }
}

/// Send `text` to procnto as an `_OSCRAFT_DEBUG_PRINT` message.
fn print(text: &str) -> Result<u64, u64> {
    let mut msg = [0u8; 256];
    msg[..2].copy_from_slice(&OSCRAFT_DEBUG_PRINT.to_le_bytes());
    let n = text.len().min(msg.len() - 2);
    msg[2..2 + n].copy_from_slice(&text.as_bytes()[..n]);
    kercall(
        __KER_MSG_SENDV,
        [
            SYSMGR_COID,
            msg.as_ptr() as u64,
            (-((2 + n) as i64)) as u64,
            0,
            0,
        ],
    )
}

/// A fixed-capacity string for formatting without a heap.
struct Line {
    buf: [u8; 200],
    len: usize,
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

fn fib(n: u64) -> u64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ =
        print("Hello from user space: an OsCraft process at EL0, talking to procnto by MsgSend");
    let mut line = Line {
        buf: [0; 200],
        len: 0,
    };
    let sp: u64;
    // SAFETY: reads the stack pointer only.
    unsafe { asm!("mov {}, sp", out(reg) sp) };
    let _ = write!(
        line,
        "fib(25) = {} computed at EL0, stack at {sp:#x}",
        fib(25)
    );
    let _ = print(core::str::from_utf8(&line.buf[..line.len]).unwrap_or("?"));
    // An unknown request must fail with ENOSYS (89) through the error path.
    let msg = 0x1234u16.to_le_bytes();
    let r = kercall(
        __KER_MSG_SENDV,
        [SYSMGR_COID, msg.as_ptr() as u64, (-2i64) as u64, 0, 0],
    );
    let mut line = Line {
        buf: [0; 200],
        len: 0,
    };
    let _ = write!(line, "unknown request -> {r:?} (ENOSYS is 89)");
    let _ = print(core::str::from_utf8(&line.buf[..line.len]).unwrap_or("?"));
    let _ = __KER_THREAD_DESTROY;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    let _ = print("hello: panic");
    let _ = kercall(__KER_THREAD_DESTROY, [0; 5]);
    loop {}
}
