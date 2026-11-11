//! QNX's binary interface for startup: the structures startup reads (the
//! startup header, the boot arguments, the image file system) and the ones
//! it builds for the kernel (the system page).
//!
//! Each type mirrors a C structure of the QNX SDP 8.0 headers, field by
//! field, with `#[repr(C)]`. Pointers in the C structures are `u64` here:
//! they hold virtual addresses of the kernel's address space, which startup
//! computes but never dereferences. That also makes the types the same on
//! the host, where the layout tests run (see `abi/layout.txt`).

pub mod elf;
pub mod image;
pub mod startup;
pub mod syspage;

#[cfg(test)]
mod layout_tests;
