//! The startup header and the boot arguments (`<sys/startup.h>`).

// ANCHOR: startup_header
/// `struct startup_header`: written by mkifs in front of startup, it
/// describes the image: where it is, how big its parts are and how it is
/// stored.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StartupHeader {
    pub signature: u32,
    pub version: u16,
    pub flags1: u8,
    pub flags2: u8,
    pub header_size: u16,
    pub machine: u16,
    /// Where the boot loader jumps to: startup's entry point.
    pub startup_vaddr: u32,
    pub paddr_bias: u32,
    /// Where the image is (the startup header's own address).
    pub image_paddr: u32,
    /// Where the image runs from, if the boot loader copies it.
    pub ram_paddr: u32,
    /// RAM used by startup and by the files of the image file system.
    pub ram_size: u32,
    /// Size of the startup header and startup itself (never compressed).
    pub startup_size: u32,
    /// Size of the whole image, as stored.
    pub stored_size: u32,
    pub imagefs_paddr: u32,
    /// Size of the image file system, uncompressed.
    pub imagefs_size: u32,
    pub preboot_size: u16,
    pub zero0: u16,
    pub zero: [u32; 1],
    /// Added to `startup_vaddr`, `image_paddr`, `ram_paddr` and
    /// `imagefs_paddr`, for images above 4 GB.
    pub addr_off: u64,
    pub info: [u32; 48],
}
// ANCHOR_END: startup_header

/// `STARTUP_HDR_SIGNATURE`.
pub const STARTUP_HDR_SIGNATURE: u32 = 0x00ff_7eeb;
/// The image runs with virtual memory: startup sets up the MMU.
pub const STARTUP_HDR_FLAGS1_VIRTUAL: u8 = 0x01;
/// The image file system is compressed (one of the `COMPRESS_*` methods).
pub const STARTUP_HDR_FLAGS1_COMPRESS_MASK: u8 = 0x1c;

/// `struct bootargs_entry`: startup's command line, written by mkifs over
/// the area with the `ddpvbskr` signature (see [`crate::bootargs`]). The
/// `argc` argument strings, then the `envc` environment strings, follow it,
/// each terminated by a NUL.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BootArgsEntry {
    /// Size of the whole entry, strings included.
    pub size_lo: u8,
    pub size_hi: u8,
    pub argc: i8,
    pub envc: i8,
    /// Address of the startup header, or [`SHDR_ADDR_64`].
    pub shdr_addr: u32,
}

/// `shdr_addr` value for a startup header above 4 GB: its 64-bit address is
/// then in the last 8 bytes of the entry.
pub const SHDR_ADDR_64: u32 = !0;
