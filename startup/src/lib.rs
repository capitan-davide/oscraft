//! A startup library in Rust, compatible with QNX OS 8.0.
//!
//! A QNX system boots in two steps. The *startup* program runs first: it
//! finds out what the hardware is, describes it in the *system page*,
//! prepares the MMU and the CPUs, and starts the kernel (`procnto`), which
//! never sees the hardware the way startup does. This crate is the generic
//! part of a startup program, the counterpart of QNX's `libstartup.a`. A
//! board crate adds what is specific to one board: it defines `main()`,
//! which receives the [`Startup`] context, and names it with [`entry!`].
//!
//! The crate has no board code.

#![no_std]

pub mod abi;
pub mod arch;
pub mod bootargs;
pub mod callout;
pub mod cmdline;
pub mod console;
pub mod decompress;
pub mod elf;
pub mod fdt;
mod global;
#[cfg(target_arch = "aarch64")]
pub mod ifs;
pub mod options;
#[cfg(target_arch = "aarch64")]
pub mod pl011;
pub mod qtime;
pub mod ram;
#[cfg(target_arch = "aarch64")]
mod startup;
pub mod syspage;

#[cfg(target_arch = "aarch64")]
pub use startup::{COMMON_OPTIONS, Startup};

// ANCHOR: entry_macro
/// Name the board's `main(&mut Startup) -> !`: once `_start` has set up
/// the CPU, the library builds the [`Startup`] context and calls it.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        extern "C" fn __oscraft_startup_main(boot_regs: &[u64; 4]) -> ! {
            $main($crate::Startup::init(*boot_regs))
        }
    };
}
// ANCHOR_END: entry_macro

/// Stop with a message on the debug console: startup cannot go on.
#[macro_export]
macro_rules! crash {
    ($($arg:tt)*) => {{
        $crate::kprintln!("startup: {}", format_args!($($arg)*));
        $crate::halt()
    }};
}

/// Stop this CPU for good.
pub fn halt() -> ! {
    loop {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: waiting for an event has no effect besides the wait.
        unsafe {
            core::arch::asm!("wfe")
        };
        #[cfg(not(target_arch = "aarch64"))]
        core::hint::spin_loop();
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    crash!("panic: {info}")
}
