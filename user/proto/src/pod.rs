//! Plain data: types that are their bytes, and so can be sent as a
//! message and read back from a reply.

/// A type with no padding that matters and no invalid bit patterns: any
/// bytes are a value of it, and its value is its bytes.
///
/// # Safety
///
/// The type is `#[repr(C)]`, `Copy`, and every bit pattern of its size
/// is a valid value.
pub unsafe trait Pod: Copy + 'static {
    /// The all-zero value.
    fn zeroed() -> Self {
        // SAFETY: every bit pattern is valid (the trait's contract).
        unsafe { core::mem::zeroed() }
    }

    /// The value's bytes.
    fn as_bytes(&self) -> &[u8] {
        // SAFETY: the value is its bytes, all initialised for a Pod
        // built from zeroed() or from fields.
        unsafe { core::slice::from_raw_parts((self as *const Self).cast(), size_of::<Self>()) }
    }

    /// The value's bytes, writable, for a reply.
    fn as_mut_bytes(&mut self) -> &mut [u8] {
        // SAFETY: as above; any bytes written are a valid value.
        unsafe { core::slice::from_raw_parts_mut((self as *mut Self).cast(), size_of::<Self>()) }
    }

    /// Reads a value from the start of `bytes`, if it is long enough.
    fn read_from(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < size_of::<Self>() {
            return None;
        }
        // SAFETY: enough bytes, and any are a valid value.
        Some(unsafe { bytes.as_ptr().cast::<Self>().read_unaligned() })
    }
}

/// Declares types plain data, checking each one's size.
macro_rules! pod {
    ($($t:ty = $size:expr),* $(,)?) => {
        $(
            // SAFETY: an integer, or a #[repr(C)] struct of integers and
            // arrays of them whose size is checked below.
            unsafe impl $crate::pod::Pod for $t {}
            const _: () = assert!(size_of::<$t>() == $size);
        )*
    };
}
pub(crate) use pod;

pod!(
    u8 = 1,
    u16 = 2,
    u32 = 4,
    u64 = 8,
    i8 = 1,
    i16 = 2,
    i32 = 4,
    i64 = 8
);

// SAFETY: an array of plain data is plain data, with no padding.
unsafe impl<T: Pod, const N: usize> Pod for [T; N] {}
