//! A program marked `ELFOSABI_FREEBSD` (the Makefile patches the ELF
//! header after the link): the kernel starts it as a FreeBSD process
//! (`proc::loader::initial_stack`). It prints what it finds on its
//! initial stack: argc, argv, envp, the auxiliary vector, x0 at entry,
//! the thread pointer and `struct ps_strings`. It still talks to procnto
//! with QNX kernel calls: those are the same for both personalities.

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::fmt::Write;

const SYSMGR_COID: u64 = 0x4000_0000;
const __KER_MSG_SENDV: u32 = 11;
const __KER_THREAD_DESTROY: u32 = 47;
const OSCRAFT_DEBUG_PRINT: u16 = 0xf000;

global_asm!(
    ".section .text.start, \"ax\"",
    ".global _start",
    "_start:",
    "    mov x1, x0", // x0 at entry (FreeBSD: the stack pointer)
    "    mov x0, sp",
    "    bl main",
    "    mov x0, #0",
    "    mov w8, #47", // __KER_THREAD_DESTROY
    "    svc #0x51",
    "    b .",
);

fn kercall(n: u32, a: [u64; 5]) -> Result<u64, u64> {
    let (r, failed): (u64, u64);
    // SAFETY: traps into the kernel; only x0-x9 are involved.
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

fn print(text: &str) {
    let mut msg = [0u8; 256];
    msg[..2].copy_from_slice(&OSCRAFT_DEBUG_PRINT.to_le_bytes());
    let n = text.len().min(msg.len() - 2);
    msg[2..2 + n].copy_from_slice(&text.as_bytes()[..n]);
    let _ = kercall(
        __KER_MSG_SENDV,
        [SYSMGR_COID, msg.as_ptr() as u64, (-((2 + n) as i64)) as u64, 0, 0],
    );
}

struct Line {
    buf: [u8; 240],
    len: usize,
}

impl Line {
    fn new() -> Self {
        Line { buf: [0; 240], len: 0 }
    }
    fn flush(&self) {
        print(core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?"));
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// A NUL-terminated string in memory, for printing.
fn cstr<'a>(p: u64) -> &'a str {
    // SAFETY: the kernel wrote the string; it is NUL-terminated.
    unsafe {
        let mut n = 0;
        while *((p + n) as *const u8) != 0 {
            n += 1;
        }
        core::str::from_utf8(core::slice::from_raw_parts(p as *const u8, n as usize)).unwrap_or("?")
    }
}

fn word(p: u64) -> u64 {
    // SAFETY: an address inside the initial stack the kernel built.
    unsafe { *(p as *const u64) }
}

#[unsafe(no_mangle)]
extern "C" fn main(sp: u64, x0: u64) {
    let tp: u64;
    // SAFETY: reads the thread pointer only.
    unsafe { asm!("mrs {}, tpidr_el0", out(reg) tp) };
    let argc = word(sp);
    let mut l = Line::new();
    let _ = write!(l, "hello: argc={argc} x0==sp:{} sp%16={} tpidr_el0={tp:#x}", x0 == sp, sp % 16);
    l.flush();
    let mut at = sp + 8;
    let mut l = Line::new();
    let _ = write!(l, "hello: argv:");
    while word(at) != 0 {
        let _ = write!(l, " {}", cstr(word(at)));
        at += 8;
    }
    l.flush();
    let argv_at = sp + 8;
    at += 8;
    let envp_at = at;
    let mut envc = 0;
    let mut l = Line::new();
    let _ = write!(l, "hello: envp:");
    while word(at) != 0 {
        if envc < 3 {
            let _ = write!(l, " {}", cstr(word(at)));
        }
        envc += 1;
        at += 8;
    }
    let _ = write!(l, " (envc={envc})");
    l.flush();
    at += 8;
    // auxv
    let mut ps_strings = 0;
    loop {
        let (t, v) = (word(at), word(at + 8));
        at += 16;
        if t == 0 {
            break;
        }
        let mut l = Line::new();
        let _ = match t {
            15 => write!(l, "hello: AT_EXECPATH {}", cstr(v)),
            16 => write!(l, "hello: AT_CANARY {:#x}..", word(v)),
            28 => write!(l, "hello: AT_ARGC {v}"),
            29 => write!(l, "hello: AT_ARGV ok:{}", v == argv_at),
            30 => write!(l, "hello: AT_ENVC {v}"),
            31 => write!(l, "hello: AT_ENVV ok:{}", v == envp_at),
            20 => write!(l, "hello: AT_PAGESIZES [{}]", word(v)),
            32 => {
                ps_strings = v;
                write!(l, "hello: AT_PS_STRINGS {v:#x}")
            }
            _ => write!(l, "hello: AT_{t} {v:#x}"),
        };
        l.flush();
    }
    if ps_strings != 0 {
        let mut l = Line::new();
        let _ = write!(
            l,
            "hello: ps_strings argv ok:{} nargv={} envp ok:{} nenv={}",
            word(ps_strings) == argv_at,
            word(ps_strings + 8) as u32,
            word(ps_strings + 16) == envp_at,
            word(ps_strings + 24) as u32
        );
        l.flush();
    }
    let _ = __KER_THREAD_DESTROY;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    print("hello: panic");
    let _ = kercall(__KER_THREAD_DESTROY, [0; 5]);
    loop {}
}
