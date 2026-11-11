//! Kernel console.
//!
//! Output and input go through the system page's debug callouts
//! (`callout.debug[0].display_char` and `poll_key`): board-specific code that the startup
//! program supplies and patches with the UART's mapped address. The kernel
//! itself knows nothing about the console device.

use core::fmt;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::sync::IrqSpinLock;
use crate::syspage::{Syspage, SyspageEntry};

static SYSPAGE: AtomicPtr<SyspageEntry> = AtomicPtr::new(core::ptr::null_mut());

/// Start using the system page's debug callout. Output before this is lost.
pub fn init(syspage: Syspage) {
    SYSPAGE.store(syspage.as_ptr(), Ordering::Release);
}

// ANCHOR: putc
fn putc(byte: u8) {
    let ptr = SYSPAGE.load(Ordering::Acquire);
    if ptr.is_null() {
        return;
    }
    // SAFETY: only valid system page pointers are stored by `init`.
    let syspage = unsafe { Syspage::new(ptr) };
    if let Some(display_char) = syspage.callout().debug[0].display_char {
        // SAFETY: startup's callouts follow the C calling convention and
        // only use the system page and their own patched-in device address.
        unsafe { display_char(ptr, byte) };
    }
}
// ANCHOR_END: putc

/// A byte from the console, if one has arrived
/// (`callout.debug[0].poll_key`, which returns -1 when there is none).
pub fn poll_key() -> Option<u8> {
    let ptr = SYSPAGE.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }
    // SAFETY: as in `putc`.
    let syspage = unsafe { Syspage::new(ptr) };
    let poll_key = syspage.callout().debug[0].poll_key?;
    let _guard = LOCK.lock();
    // SAFETY: as for display_char; the lock keeps it off the UART while
    // another CPU prints.
    let c = unsafe { poll_key(ptr) };
    u8::try_from(c).ok()
}

struct Console;

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                putc(b'\r');
            }
            putc(byte);
        }
        Ok(())
    }
}

/// Serialises output so that messages from different CPUs do not mix.
static LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

/// The console and the message buffer: what the kernel says.
struct Logged;

impl fmt::Write for Logged {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        crate::msgbuf::write(s.as_bytes());
        Console.write_str(s)
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    let _guard = LOCK.lock();
    let _ = Logged.write_fmt(args);
}

/// The console alone: what programs write to `/dev/text`, and its echo,
/// isn't the kernel's to keep.
#[doc(hidden)]
pub fn _print_terminal(args: fmt::Arguments) {
    use fmt::Write;
    let _guard = LOCK.lock();
    let _ = Console.write_fmt(args);
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => ($crate::console::_print(format_args!($($arg)*)));
}

/// Terminal output, not a kernel message (`_print_terminal`).
#[macro_export]
macro_rules! tprint {
    ($($arg:tt)*) => ($crate::console::_print_terminal(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! kprintln {
    () => ($crate::kprint!("\n"));
    ($($arg:tt)*) => ($crate::kprint!("{}\n", format_args!($($arg)*)));
}
