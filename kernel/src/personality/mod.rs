//! The personalities: which ABI a process speaks. Everything that differs
//! between a FreeBSD process and a QNX process is asked of its
//! `Personality` here; the rest of the kernel, the kernel calls' core, is
//! shared. FreeBSD is the native language; QNX is the compatibility one.

pub mod native;
pub mod qnx;

/// Which user-space ABI a process's program was built for. FreeBSD is
/// OsCraft's native personality: its userland is FreeBSD's, built from
/// source over a libsys that speaks OsCraft's kernel calls and message
/// protocols. QNX is the compatibility personality, for the SDP's
/// unmodified binaries (its libc, drivers and services), which the kernel
/// and its servers go on serving as procnto does. The kernel calls are
/// the same for both; what differs is how a program is started (its
/// initial stack, auxiliary vector and thread pointer: `proc::loader`)
/// and the message layouts it uses with servers, which see a FreeBSD
/// client in `struct _msg_info.flags` (`Personality::msg_info_flags`).
/// Chosen at load time from the ELF header's OS ABI byte (`EI_OSABI`), inherited by a fork.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum Personality {
    /// FreeBSD: `ELFOSABI_FREEBSD` (9), what FreeBSD's toolchain brands
    /// every binary with. The default.
    #[default]
    Native = 0,
    /// QNX 8.0: `ELFOSABI_NONE` or `ELFOSABI_GNU`, what the SDP's
    /// toolchain emits.
    Qnx = 1,
}

impl Personality {
    // ANCHOR: from_osabi
    /// From an ELF header's `EI_OSABI` byte. FreeBSD's toolchain brands
    /// every binary `ELFOSABI_FREEBSD`; QNX's leaves `ELFOSABI_NONE`, or
    /// `ELFOSABI_GNU` for a static link (measured: the SDP's `-static`
    /// `hello`), so anything not branded FreeBSD is QNX.
    pub fn from_osabi(osabi: u8) -> Self {
        match osabi {
            9 => Self::Native,
            _ => Self::Qnx,
        }
    }
    // ANCHOR_END: from_osabi

    /// Its bits in `struct _msg_info.flags`, which tell a server what
    /// a client speaks.
    pub fn msg_info_flags(self) -> u16 {
        match self {
            Self::Native => native::MI_NATIVE,
            Self::Qnx => 0,
        }
    }

    /// The number a process of this personality sees for the kernel's
    /// error `e`, which is FreeBSD's. A QNX-only error is EIO to FreeBSD.
    pub fn errno_out(self, e: u32) -> u32 {
        match self {
            Self::Native if e & crate::ker::errno::QNX_ONLY != 0 => crate::ker::errno::EIO,
            Self::Native => e,
            Self::Qnx => qnx::errno_out(e),
        }
    }

    /// The kernel's number for error `e` of a process of this personality.
    pub fn errno_in(self, e: u32) -> u32 {
        match self {
            Self::Native => e,
            Self::Qnx => qnx::errno_in(e),
        }
    }

    /// The kernel's number for signal `s` of a process of this personality.
    pub fn signo_in(self, s: usize) -> usize {
        match self {
            Self::Native => s,
            Self::Qnx => qnx::signo_in(s),
        }
    }

    /// The number a process of this personality knows signal `s` by.
    pub fn signo_out(self, s: usize) -> usize {
        match self {
            Self::Native => s,
            Self::Qnx => qnx::signo_out(s).unwrap_or(0),
        }
    }

    /// Which signal of `ready` it takes first: the lowest number it knows.
    pub fn next_signal(self, ready: crate::ker::signal::SigSet) -> usize {
        match self {
            Self::Native => ready.trailing_zeros() as usize + 1,
            Self::Qnx => qnx::next(ready),
        }
    }

    /// Its line in `/proc/<pid>/personality`.
    pub fn name(self) -> &'static [u8] {
        match self {
            Self::Native => b"freebsd\n",
            Self::Qnx => b"qnx\n",
        }
    }

    /// Whether a forked child's `MsgSend()` of `_PROC_FORK` returns 0 and
    /// the kernel fixes up its thread-local block, as QNX's libc wants;
    /// else it returns the child's ID for libsys (`native`).
    pub fn fork_uses_tls(self) -> bool {
        self == Self::Qnx
    }
}
