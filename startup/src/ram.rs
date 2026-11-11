//! Physical RAM: what there is, and what startup has handed out.
//!
//! The board reports its RAM with [`Ram::add`]. Startup then allocates
//! from it for the page tables, the system page and the bootstrap
//! executables' data, always from the lowest free address that fits.
//! Whatever is still free at the end becomes the kernel's `sysram`, the
//! memory it manages.
//!
//! Some ranges must be left alone only while startup runs, such as the
//! device tree the boot loader passed or a compressed image being
//! unpacked: [`Ram::avoid`] keeps allocations out of them without taking
//! them from the kernel.

/// A set of disjoint, half-open address ranges `[start, end)`, sorted and
/// merged, with room for `N` ranges.
#[derive(Clone, Debug)]
pub struct RangeSet<const N: usize> {
    ranges: [(u64, u64); N],
    len: usize,
}

impl<const N: usize> RangeSet<N> {
    pub const fn new() -> Self {
        Self {
            ranges: [(0, 0); N],
            len: 0,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.ranges[..self.len].iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Add `[start, end)`, merging it with the ranges it overlaps or
    /// touches.
    pub fn insert(&mut self, mut start: u64, mut end: u64) {
        if start >= end {
            return;
        }
        // The ranges before it stay, the ones it overlaps merge into it,
        // the ones after it move to make room.
        let first = self.ranges[..self.len].partition_point(|r| r.1 < start);
        let mut last = first;
        while last < self.len && self.ranges[last].0 <= end {
            start = start.min(self.ranges[last].0);
            end = end.max(self.ranges[last].1);
            last += 1;
        }
        let merged = last - first;
        if merged == 0 {
            assert!(self.len < N, "RangeSet: more than {N} ranges");
            self.ranges.copy_within(first..self.len, first + 1);
            self.len += 1;
        } else {
            self.ranges.copy_within(last..self.len, first + 1);
            self.len -= merged - 1;
        }
        self.ranges[first] = (start, end);
    }

    /// Remove `[start, end)`, splitting a range that contains it.
    pub fn remove(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        let mut i = 0;
        while i < self.len {
            let (s, e) = self.ranges[i];
            if e <= start || s >= end {
                i += 1;
                continue;
            }
            match (s < start, e > end) {
                // Both ends survive: split in two.
                (true, true) => {
                    assert!(self.len < N, "RangeSet: more than {N} ranges");
                    self.ranges.copy_within(i + 1..self.len, i + 2);
                    self.len += 1;
                    self.ranges[i] = (s, start);
                    self.ranges[i + 1] = (end, e);
                    return;
                }
                (true, false) => {
                    self.ranges[i].1 = start;
                    i += 1;
                }
                (false, true) => {
                    self.ranges[i].0 = end;
                    i += 1;
                }
                (false, false) => {
                    self.ranges.copy_within(i + 1..self.len, i);
                    self.len -= 1;
                }
            }
        }
    }
}

impl<const N: usize> Default for RangeSet<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Room for this many separate free ranges, and avoided ones.
const MAX_RANGES: usize = 32;

/// Startup's view of physical RAM.
#[derive(Debug, Default)]
pub struct Ram {
    /// RAM that is still free.
    free: RangeSet<MAX_RANGES>,
    /// RAM not to allocate from while startup runs.
    avoided: RangeSet<MAX_RANGES>,
}

impl Ram {
    pub const fn new() -> Self {
        Self {
            free: RangeSet::new(),
            avoided: RangeSet::new(),
        }
    }

    /// Add `size` bytes of RAM at `start`.
    pub fn add(&mut self, start: u64, size: u64) {
        self.free.insert(start, start + size);
    }

    /// Take `[start, start + size)` out of the free RAM for good: it is in
    /// use (by the image, or by a device that shares RAM).
    pub fn reserve(&mut self, start: u64, size: u64) {
        self.free.remove(start, start + size);
    }

    /// Keep allocations out of `[start, start + size)` while startup runs;
    /// it stays in the kernel's free RAM.
    pub fn avoid(&mut self, start: u64, size: u64) {
        self.avoided.insert(start, start + size);
    }

    // ANCHOR: ram_alloc
    /// Allocate `size` bytes aligned to `align` (a power of two), from the
    /// lowest free address that is not avoided.
    pub fn alloc(&mut self, size: u64, align: u64) -> Option<u64> {
        let addr = self.find(size, align)?;
        self.reserve(addr, size);
        Some(addr)
    }

    /// Where [`Ram::alloc`] would allocate, without allocating: for memory
    /// startup uses only for a moment, between two allocations.
    pub fn find(&self, size: u64, align: u64) -> Option<u64> {
        let fits = |s: u64, e: u64| {
            let a = s.checked_next_multiple_of(align)?;
            (a.checked_add(size)? <= e).then_some(a)
        };
        let mut found = None;
        'ranges: for (s, e) in self.free.iter() {
            // Walk the free range from the bottom, stepping over the
            // avoided ranges inside it.
            let mut lo = s;
            for (as_, ae) in self.avoided.iter() {
                if ae <= lo || as_ >= e {
                    continue;
                }
                if let Some(a) = fits(lo, as_.min(e)) {
                    found = Some(a);
                    break 'ranges;
                }
                lo = lo.max(ae);
            }
            if let Some(a) = fits(lo, e) {
                found = Some(a);
                break;
            }
        }
        found
    }
    // ANCHOR_END: ram_alloc

    /// Where the highest `size` bytes aligned to `align` that are free and
    /// not avoided are, without allocating them.
    pub fn find_top(&self, size: u64, align: u64) -> Option<u64> {
        let mut best = None;
        for (s, e) in self.free.iter() {
            // The gaps of [s, e) between avoided ranges.
            let mut lo = s;
            let mut gaps = self
                .avoided
                .iter()
                .filter(|a| a.1 > s && a.0 < e)
                .map(|a| (a.0, a.1));
            loop {
                let (hi, next_lo) = match gaps.next() {
                    Some((as_, ae)) => (as_.max(lo), Some(ae)),
                    None => (e, None),
                };
                if let Some(top) = hi.checked_sub(size).map(|t| t & !(align - 1))
                    && top >= lo
                {
                    best = best.max(Some(top));
                }
                match next_lo {
                    Some(n) => lo = lo.max(n),
                    None => break,
                }
            }
        }
        best
    }

    /// The free RAM ranges, `[start, end)`.
    pub fn free(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.free.iter()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn ranges<const N: usize>(s: &RangeSet<N>) -> Vec<(u64, u64)> {
        s.iter().collect()
    }

    #[test]
    fn insert_merges_and_sorts() {
        let mut s = RangeSet::<8>::new();
        s.insert(100, 200);
        s.insert(0, 10);
        s.insert(300, 400);
        assert_eq!(ranges(&s), [(0, 10), (100, 200), (300, 400)]);
        s.insert(150, 300);
        assert_eq!(ranges(&s), [(0, 10), (100, 400)]);
        s.insert(10, 100);
        assert_eq!(ranges(&s), [(0, 400)]);
    }

    #[test]
    fn remove_splits_and_trims() {
        let mut s = RangeSet::<8>::new();
        s.insert(0, 100);
        s.insert(200, 300);
        s.remove(40, 60);
        assert_eq!(ranges(&s), [(0, 40), (60, 100), (200, 300)]);
        s.remove(90, 250);
        assert_eq!(ranges(&s), [(0, 40), (60, 90), (250, 300)]);
        s.remove(0, 1000);
        assert!(s.is_empty());
    }

    #[test]
    fn alloc_takes_the_lowest_fit_and_skips_avoided() {
        let mut ram = Ram::new();
        ram.add(0, 0x20_0000);
        ram.reserve(0x8_0000, 0x1_0000); // the image
        ram.avoid(0x1000, 0x1000);
        assert_eq!(ram.alloc(0x1000, 0x1000), Some(0));
        // 0x1000..0x2000 is avoided.
        assert_eq!(ram.alloc(0x1000, 0x1000), Some(0x2000));
        assert_eq!(ram.alloc(0x10, 8), Some(0x3000));
        assert_eq!(ram.alloc(0x1000, 0x1000), Some(0x4000));
        // Too big below the image: above it.
        assert_eq!(ram.alloc(0x7_c000, 0x1000), Some(0x9_0000));
        assert_eq!(ram.alloc(0x100_0000, 0x1000), None);
        // The avoided range stays free for the kernel.
        assert!(ram.free().any(|(s, e)| s <= 0x1000 && e >= 0x2000));
    }

    #[test]
    fn find_top_takes_the_highest_fit() {
        let mut ram = Ram::new();
        ram.add(0, 0x10_0000);
        ram.add(0x20_0000, 0x10_0000);
        assert_eq!(ram.find_top(0x2000, 0x1000), Some(0x2f_e000));
        ram.avoid(0x2f_0000, 0x1_0000);
        assert_eq!(ram.find_top(0x2000, 0x1000), Some(0x2e_e000));
        assert_eq!(ram.find_top(0x20_0000, 0x1000), None);
    }
}
