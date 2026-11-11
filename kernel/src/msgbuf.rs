//! The kernel's message buffer, FreeBSD's `msgbuf`: what the kernel prints,
//! kept in a ring from the first message, long before any program runs.
//! `/dev/klog` reads it for `syslogd`, once; `sysctl(kern.msgbuf)` is a
//! snapshot, for `dmesg`. Terminal output through `/dev/text` isn't kept here.

use crate::sync::IrqSpinLock;

pub const CAPACITY: usize = 64 * 1024;

struct Ring {
    data: [u8; CAPACITY],
    /// Bytes ever written.
    head: u64,
    /// What `/dev/klog` has read.
    read: u64,
}

static RING: IrqSpinLock<Ring> = IrqSpinLock::new(Ring { data: [0; CAPACITY], head: 0, read: 0 });

/// Called with the console's lock held, from any context.
pub fn write(bytes: &[u8]) {
    let mut r = RING.lock();
    for &b in bytes {
        let at = (r.head % CAPACITY as u64) as usize;
        r.data[at] = b;
        r.head += 1;
    }
}

/// Copies what is there from `from` into `out`: the bytes copied, the
/// position after them. A position overwritten moves up to the oldest.
fn copy(r: &Ring, from: u64, out: &mut [u8]) -> (usize, u64) {
    let from = from.max(r.head.saturating_sub(CAPACITY as u64));
    let n = ((r.head - from) as usize).min(out.len());
    for (i, slot) in out.iter_mut().take(n).enumerate() {
        *slot = r.data[((from + i as u64) % CAPACITY as u64) as usize];
    }
    (n, from + n as u64)
}

/// `/dev/klog`: what hasn't been read, once.
pub fn read(out: &mut [u8]) -> usize {
    let mut r = RING.lock();
    let (n, at) = copy(&r, r.read, out);
    r.read = at;
    n
}

/// `kern.msgbuf`: all there is, read or not.
pub fn snapshot(out: &mut [u8]) -> usize {
    copy(&RING.lock(), 0, out).0
}
