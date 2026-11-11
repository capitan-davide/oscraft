//! The kernel's entropy pool and `SysSrandom()`.
//!
//! Every interrupt mixes the counter's value (its timing) into a 64-bit
//! pool, as do the seeds callers give (`SysSrandom(seedp, ...)`). Entropy
//! asked for comes out of the pool through SplitMix64, mixed with the
//! counter again. This is not a cryptographic generator: its consumer,
//! QNX's `random` service, feeds what it gets to qcrypto's DRBG.

use core::sync::atomic::{AtomicU64, Ordering};

use super::KerResult;
use super::errno::EFAULT;
use super::umem::{self, Space};
use crate::arch::timer;

static POOL: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

fn mix(v: u64) {
    let _ = POOL.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |p| {
        Some((p ^ v).rotate_left(23).wrapping_mul(0x9e37_79b9_7f4a_7c15))
    });
}

/// An interrupt (vector `vector`) came: its timing is entropy.
pub fn add_interrupt(vector: u32) {
    mix(timer::counter() ^ (vector as u64) << 48);
}

/// A word out of the pool (SplitMix64).
fn next() -> u64 {
    let s = POOL.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed) ^ timer::counter();
    let mut z = s.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `SysSrandom(seedp, entropy, elements)`: mix in the 64-bit seed at
/// `seedp` (if not 0), give `elements` words of entropy at `entropy`.
/// Everyone may (every process has every ability).
pub fn sys_srandom(seedp: u64, entropy: u64, elements: u64) -> KerResult {
    // Taking entropy out needs SRANDOM (as procnto); seeding doesn't.
    if elements != 0 {
        super::ability::check(super::ability::SRANDOM, None)?;
    }
    let space = Space::current();
    if seedp != 0 {
        let mut b = [0u8; 8];
        umem::read(space, seedp, &mut b)?;
        mix(u64::from_le_bytes(b));
    }
    if elements != 0 {
        if entropy == 0 || elements > 4096 {
            return Err(EFAULT);
        }
        let mut out = alloc::vec::Vec::with_capacity(8 * elements as usize);
        for _ in 0..elements {
            out.extend_from_slice(&next().to_le_bytes());
        }
        umem::write(space, entropy, &out)?;
    }
    Ok(0)
}
