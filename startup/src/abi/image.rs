//! The image file system, IFS (`<sys/image.h>`).
//!
//! The IFS follows startup in the image: an [`ImageHeader`], then a
//! directory of variable-sized entries, then the files' data. The kernel
//! serves it as `/proc/boot` and the rest of the image's paths; startup
//! only walks the directory to find the *bootstrap executables*, the
//! programs it loads and starts (the kernel, `procnto`, is the last one).

/// `struct image_header`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ImageHeader {
    /// "imagefs".
    pub signature: [u8; 7],
    pub flags: u8,
    /// Size from the header to the end of the trailer.
    pub image_size: u32,
    /// Size from the header to the last directory entry.
    pub hdr_dir_size: u32,
    /// Offset from the header to the first directory entry.
    pub dir_offset: u32,
    pub boot_ino: [u32; 4],
    pub script_ino: u32,
    pub chain_paddr: u32,
    pub spare: [u32; 10],
    pub mountflags: u32,
    // The mount point string follows.
}

pub const IMAGE_SIGNATURE: [u8; 7] = *b"imagefs";
/// The directory entries' `ino` fields carry the `IFS_INO_*` bits.
pub const IMAGE_FLAGS_INO_BITS: u8 = 0x04;

/// `struct image_attr`: the part common to all directory entries.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ImageAttr {
    /// Size of the entry; 0 ends the directory.
    pub size: u16,
    pub extattr_offset: u16,
    pub ino: u32,
    pub mode: u32,
    pub gid: u32,
    pub uid: u32,
    pub mtime: u32,
}

/// `struct image_file`: a directory entry for a regular file.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ImageFile {
    pub attr: ImageAttr,
    /// Offset of the data from the image header.
    pub offset: u32,
    pub size: u32,
    // The path follows, NUL-terminated, without a leading '/'.
}

/// The file is a bootstrap executable (set in `ino` by mkifs).
pub const IFS_INO_BOOTSTRAP_EXE: u32 = 0x2000_0000;

// ANCHOR: bootstrap_structs
/// `IFS_BOOTSTRAP_SIGNATURE`: "BSHD".
pub const IFS_BOOTSTRAP_SIGNATURE: u32 =
    (b'B' as u32) << 24 | (b'S' as u32) << 16 | (b'H' as u32) << 8 | b'D' as u32;

/// `struct ifs_bootstrap_head`: right before a bootstrap executable's entry
/// point, it leads startup to the executable's [`IfsBootstrapData`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IfsBootstrapHead {
    pub signature: u32,
    /// Virtual address of the executable's `IfsBootstrapData`.
    pub bootstrap: u64,
}

/// `struct ifs_bootstrap_data`: a bootstrap executable's link to the next
/// one, filled in by startup.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IfsBootstrapData {
    pub size: u32,
    /// Entry point of the next bootstrap executable (0 for the kernel).
    pub next_entry: u64,
    /// Virtual address of the executable's own `bootargs_entry`.
    pub args: u64,
    pub spare: u32,
}
// ANCHOR_END: bootstrap_structs
