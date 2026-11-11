//! `struct sigevent` (`<sys/siginfo.h>`): what a server or the kernel
//! delivers when something happens, a pulse or a signal.

use crate::pod::pod;

/// `struct sigevent`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Sigevent {
    /// `SIGEV_*`, with `SIGEV_FLAG_*`.
    pub notify: i32,
    pub zero: i32,
    /// The signal, the connection, or a registered event's handle.
    pub signo: i32,
    pub zero2: i32,
    /// `union sigval`.
    pub value: u64,
    /// The code and priority, two shorts, for a pulse or a signal with
    /// a code.
    pub code: i16,
    pub priority: i16,
    pub zero3: u32,
}

pod!(Sigevent = 32);

/// `SIGEV_*`: what the event is.
pub const SIGEV_NONE: i32 = 0;
pub const SIGEV_SIGNAL: i32 = 1;
pub const SIGEV_SIGNAL_CODE: i32 = 2;
pub const SIGEV_SIGNAL_THREAD: i32 = 3;
pub const SIGEV_PULSE: i32 = 4;
pub const SIGEV_UNBLOCK: i32 = 5;
pub const SIGEV_INTR: i32 = 6;
pub const SIGEV_THREAD: i32 = 7;
pub const SIGEV_MEMORY: i32 = 8;
pub const SIGEV_SEM: i32 = 9;

/// `SIGEV_FLAG_*`.
pub const SIGEV_FLAG_UPDATEABLE: i32 = 0x400;
pub const SIGEV_FLAG_SIVAL_INT: i32 = 0x800;
pub const SIGEV_FLAG_HANDLE: i32 = 0x2000;
pub const SIGEV_FLAG_CODE_UPDATEABLE: i32 = 0x4000;

/// `SI_NOTIFY`: the code of a notification's signal.
pub const SI_NOTIFY: i16 = -6;

/// `struct _pulse`: a pulse as a receiver gets it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Pulse {
    pub r#type: u16,
    pub subtype: u16,
    pub code: i8,
    pub zero: [u8; 3],
    /// `union sigval`.
    pub value: u64,
    pub scoid: i32,
    pub zero2: u32,
}

pod!(Pulse = 24);

/// The kernel's pulse codes.
pub const PULSE_CODE_UNBLOCK: i8 = -32;
pub const PULSE_CODE_DISCONNECT: i8 = -33;
