//! The debug console: startup's own output, `kprintln!`, and the kernel's
//! debug channel.
//!
//! A board lists the UARTs it can use as [`DebugDriver`]s. The library
//! picks one, by the `-D` option or else the first, prints through its
//! driver, and gives the kernel the same device through the driver's
//! callouts (see [`crate::Startup::boot`]).

use crate::callout::Callout;
use crate::cmdline::parse_number;
use crate::global::Global;
use core::fmt;

// ANCHOR: debug_driver
/// Where a debug UART is and how to set it up: `base^shift.baud.clk`, the
/// options of `-D name.options`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugPort {
    /// Physical address of the registers.
    pub base: u64,
    /// Registers are `1 << shift` bytes apart (0: the device's own spacing).
    pub shift: u32,
    /// Baud rate; 0 leaves the device as the firmware set it.
    pub baud: u64,
    /// The UART's input clock, in Hz.
    pub clk: u64,
}

/// A UART startup and the kernel can use for debug output.
pub struct DebugDriver {
    /// The name `-D` selects it by.
    pub name: &'static str,
    pub defaults: DebugPort,
    /// Set the device up for startup's output.
    pub init: fn(&DebugPort),
    /// Write one byte, waiting until the device can take it.
    pub putc: fn(&DebugPort, u8),
    /// Size of the register block to map for the callouts.
    pub size: u64,
    /// The kernel's callouts. Each takes the registers' virtual address as
    /// its patch value, if it has one.
    pub display_char: &'static Callout,
    pub poll_key: &'static Callout,
    pub break_detect: &'static Callout,
}
// ANCHOR_END: debug_driver

/// The driver for `-D arg` (`name` or `name.options`), with its port: the
/// driver whose name `arg` starts with, followed by nothing or a `.`, its
/// defaults overridden by the options. With no `-D`, the first driver.
pub fn select<'d>(
    drivers: &'d [DebugDriver],
    arg: Option<&str>,
) -> Option<(&'d DebugDriver, DebugPort)> {
    let Some(arg) = arg else {
        return drivers.first().map(|d| (d, d.defaults));
    };
    drivers.iter().find_map(|d| {
        let rest = arg.strip_prefix(d.name)?;
        let options = match rest.strip_prefix('.') {
            Some(options) => options,
            None if rest.is_empty() => "",
            None => return None,
        };
        Some((d, parse_port(d.defaults, options)))
    })
}

/// Apply `base^shift.baud.clk` to `port`; an empty field keeps its value.
/// The base is hexadecimal, as in QNX's startup.
pub fn parse_port(mut port: DebugPort, options: &str) -> DebugPort {
    let mut fields = options.split('.');
    if let Some(f) = fields.next().filter(|f| !f.is_empty()) {
        let (base, shift) = f.split_once('^').unwrap_or((f, ""));
        let hex = base.trim_start_matches("0x");
        port.base = u64::from_str_radix(hex, 16).unwrap_or(port.base);
        if !shift.is_empty() {
            port.shift = parse_number(shift).0 as u32;
        }
    }
    if let Some(f) = fields.next().filter(|f| !f.is_empty()) {
        port.baud = parse_number(f).0;
    }
    if let Some(f) = fields.next().filter(|f| !f.is_empty()) {
        port.clk = parse_number(f).0;
    }
    port
}

static CONSOLE: Global<Option<(&'static DebugDriver, DebugPort)>> = Global::new(None);

/// Print on `driver`'s device at `port` from now on.
pub fn set(driver: &'static DebugDriver, port: DebugPort) {
    (driver.init)(&port);
    // SAFETY: only the boot CPU prints, so nothing else accesses CONSOLE.
    unsafe { *CONSOLE.get() = Some((driver, port)) };
}

/// The debug device in use.
pub fn current() -> Option<(&'static DebugDriver, DebugPort)> {
    // SAFETY: as in set().
    unsafe { *CONSOLE.get() }
}

struct Writer(&'static DebugDriver, DebugPort);

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                (self.0.putc)(&self.1, b'\r');
            }
            (self.0.putc)(&self.1, b);
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn print(args: fmt::Arguments) {
    if let Some((driver, port)) = current() {
        let _ = fmt::write(&mut Writer(driver, port), args);
    }
}

/// Print a line on the debug console, if there is one.
#[macro_export]
macro_rules! kprintln {
    () => { $crate::console::print(format_args!("\n")) };
    ($($arg:tt)*) => {
        $crate::console::print(format_args!("{}\n", format_args!($($arg)*)))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::callout!(static NOP, "test_nop", patches = 0, "ret");
    fn nothing(_: &DebugPort) {}
    fn put(_: &DebugPort, _: u8) {}

    const fn driver(name: &'static str, base: u64) -> DebugDriver {
        DebugDriver {
            name,
            defaults: DebugPort {
                base,
                shift: 0,
                baud: 115200,
                clk: 48_000_000,
            },
            init: nothing,
            putc: put,
            size: 0x1000,
            display_char: &NOP,
            poll_key: &NOP,
            break_detect: &NOP,
        }
    }

    static DRIVERS: [DebugDriver; 3] = [
        driver("miniuart", 0xfe21_5000),
        driver("pl011-3", 0xfe20_1600),
        driver("pl011", 0xfe20_1000),
    ];

    #[test]
    fn selects_by_name_and_options() {
        assert_eq!(select(&DRIVERS, None).unwrap().0.name, "miniuart");
        assert_eq!(select(&DRIVERS, Some("pl011")).unwrap().1.base, 0xfe20_1000);
        // "pl011-3" is not "pl011" followed by options.
        assert_eq!(select(&DRIVERS, Some("pl011-3")).unwrap().0.name, "pl011-3");
        let (d, port) = select(&DRIVERS, Some("pl011-3.fe201800^2.9600")).unwrap();
        assert_eq!(d.name, "pl011-3");
        assert_eq!(
            port,
            DebugPort {
                base: 0xfe20_1800,
                shift: 2,
                baud: 9600,
                clk: 48_000_000
            }
        );
        assert_eq!(
            select(&DRIVERS, Some("miniuart..57600")).unwrap().1.baud,
            57600
        );
        assert!(select(&DRIVERS, Some("8250")).is_none());
    }
}
