//! The boot arguments: startup's command line, written into the image by
//! mkifs.
//!
//! mkifs finds a startup program's argument area by its signature,
//! `ddpvbskr`, and overwrites it with a `struct bootargs_entry`: its size,
//! the argument and environment counts, the address of the startup header
//! and the strings themselves. This module reserves the area.

/// Size of the argument area. `bootargs_entry` records its size in 16 bits;
/// the command lines of real build files take a few hundred bytes.
pub const BOOT_ARGS_SIZE: usize = 4096;

/// The argument area, before mkifs fills it in.
#[repr(C, align(8))]
pub struct BootArgsArea {
    signature: [u8; 8],
    rest: [u8; BOOT_ARGS_SIZE - 8],
}

/// The signature mkifs looks for: "ddpvbskr", the `__STACK_SIG` of
/// `<sys/startup.h>`.
#[unsafe(no_mangle)]
#[used]
#[unsafe(link_section = ".data.boot_args")]
static mut boot_args: BootArgsArea = BootArgsArea {
    signature: *b"ddpvbskr",
    rest: [0; BOOT_ARGS_SIZE - 8],
};

/// The argument area as mkifs left it.
pub fn area() -> &'static [u8; BOOT_ARGS_SIZE] {
    // SAFETY: mkifs wrote the area before startup ran, and nothing writes
    // it while startup runs. The read must not be folded at compile time,
    // which a shared reference to a `static mut` prevents.
    unsafe { &*(&raw const boot_args).cast() }
}
