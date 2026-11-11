//! Arm's PL011 UART, as a debug device: startup's output, and the
//! kernel's debug callouts. A board names its PL011s in its list of
//! [`DebugDriver`]s with [`driver()`].
//!
//! [`DebugDriver`]: crate::console::DebugDriver

use crate::console::{DebugDriver, DebugPort};

const DR: u64 = 0x00;
const FR: u64 = 0x18;
const IBRD: u64 = 0x24;
const FBRD: u64 = 0x28;
const LCRH: u64 = 0x2c;
const CR: u64 = 0x30;
const FR_BUSY: u32 = 1 << 3;
const FR_TXFF: u32 = 1 << 5;
/// 8 data bits, no parity, 1 stop bit, FIFOs on.
const LCRH_8N1_FIFO: u32 = (3 << 5) | (1 << 4);
/// UART, transmitter and receiver enabled.
const CR_ENABLE: u32 = (1 << 0) | (1 << 8) | (1 << 9);

fn read(port: &DebugPort, reg: u64) -> u32 {
    // SAFETY: the PL011's registers, mapped 1:1 while the MMU is off.
    unsafe { core::ptr::read_volatile((port.base + reg) as *const u32) }
}

fn write(port: &DebugPort, reg: u64, value: u32) {
    // SAFETY: as in read().
    unsafe { core::ptr::write_volatile((port.base + reg) as *mut u32, value) }
}

/// Set the baud rate, if the port has one and a clock: the divisor is
/// `clk / (16 * baud)`, in 16.6 fixed point, rounded.
pub fn init(port: &DebugPort) {
    if port.baud == 0 || port.clk == 0 {
        return;
    }
    while read(port, FR) & FR_BUSY != 0 {}
    write(port, CR, 0);
    let div = ((port.clk * 4 + port.baud / 2) / port.baud) as u32;
    write(port, IBRD, div >> 6);
    write(port, FBRD, div & 0x3f);
    write(port, LCRH, LCRH_8N1_FIFO);
    write(port, CR, CR_ENABLE);
}

/// Write one byte, waiting for room in the FIFO.
pub fn putc(port: &DebugPort, byte: u8) {
    while read(port, FR) & FR_TXFF != 0 {}
    write(port, DR, u32::from(byte));
}

// ANCHOR: pl011_callouts
// display_char(syspage, c), poll_key(syspage), break_detect(syspage),
// patched with the registers' virtual address.
crate::callout!(pub static DISPLAY_CHAR, "oscraft_pl011_display_char", patches = 1,
    "movz    x7, #0", "movk x7, #0, lsl #16", "movk x7, #0, lsl #32", "movk x7, #0, lsl #48",
    "0: ldr  w2, [x7, #0x18]",      // FR: transmit FIFO full?
    "tst     w2, #0x20",
    "b.ne    0b",
    "and     w0, w1, #0xff",
    "str     w0, [x7]",             // DR
    "ret",
);

crate::callout!(pub static POLL_KEY, "oscraft_pl011_poll_key", patches = 1,
    "movz    x7, #0", "movk x7, #0, lsl #16", "movk x7, #0, lsl #32", "movk x7, #0, lsl #48",
    "ldr     w1, [x7, #0x18]",      // FR: receive FIFO empty?
    "tst     w1, #0x10",
    "b.ne    0f",
    "ldr     w0, [x7]",             // DR
    "and     w0, w0, #0xff",
    "ret",
    "0: mov  w0, #-1",
    "ret",
);

crate::callout!(pub static BREAK_DETECT, "oscraft_pl011_break_detect", patches = 0,
    "mov     w0, #0",
    "ret",
);
// ANCHOR_END: pl011_callouts

/// A PL011 named `name`, with its default port.
pub const fn driver(name: &'static str, defaults: DebugPort) -> DebugDriver {
    DebugDriver {
        name,
        defaults,
        init,
        putc,
        size: 0x1000,
        display_char: &DISPLAY_CHAR,
        poll_key: &POLL_KEY,
        break_detect: &BREAK_DETECT,
    }
}
