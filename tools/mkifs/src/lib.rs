//! Clean-room `mkifs`/`dumpifs` for QNX OS 8.0 boot images (aarch64le).
//!
//! Built from the published interface only: the SDP 8.0.5 headers
//! (`<sys/startup.h>`, `<sys/image.h>`), the QNX 8.0 documentation of
//! `mkifs`, the QNX OS for Safety startup specification, and the open-source
//! startup library in the RPi4 BSP (which consumes the image). See
//! the book's boot chapter (`docs/book/src/04-boot.md`).

pub mod buildfile;
pub mod dump;
pub mod elf;
pub mod format;
pub mod image;
pub mod script;
