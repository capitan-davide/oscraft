//! A process's log ring: the shared memory between libsys's `syslog()`
//! path and `syslogd`, which read and write it without a kernel call.
//!
//! The idea is slogger2's (`slog2_commit_to_buffer`): writers reserve
//! room with an atomic operation and copy, and old records are overwritten.
//! One thing differs: slogger2 publishes in order, so a writer can spin on
//! an earlier one that was preempted. Here each record has its own commit
//! mark, so no writer ever waits; a reader stops at a record not finished
//! yet. Positions count bytes for ever, so a reader can tell how far
//! behind it is, and offset 0 of the area always starts a record, which
//! is where a reader overrun by writers starts again.
//!
//! All of this is plain atomics on memory the caller maps; it knows
//! nothing of processes, and runs on the host for its tests.

#![no_std]

use core::ptr::{addr_of, addr_of_mut, copy_nonoverlapping};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

/// "OSLOGRNG"
pub const MAGIC: u64 = 0x474e_5247_4f4c_534f;
/// The largest message kept, as FreeBSD's `MAXLINE` limits a line.
pub const MAX_MESSAGE: usize = 8192;

const COMMITTED: u64 = 1;
const PADDING: u64 = 2;
const RECORD: usize = size_of::<Record>();

/// The start of the shared memory; the area follows.
#[repr(C)]
pub struct Header {
    magic: u64,
    capacity: u64,
    /// The next position to reserve.
    head: AtomicU64,
    /// 1: the reader sleeps, and the next writer must wake it.
    waiting: AtomicU32,
    pub pid: u32,
    /// How far the reader got: a new reader goes on from here, and
    /// nothing is read twice across a restart.
    read: AtomicU64,
}

/// One message: `stamp` is written last.
#[repr(C)]
struct Record {
    /// The record's position and `COMMITTED`, or `PADDING`.
    stamp: AtomicU64,
    fields: Fields,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Fields {
    time_ns: u64,
    len: u32,
    priority: u32,
}

const fn align8(n: usize) -> usize {
    (n + 7) & !7
}

/// The bytes a ring of `capacity` bytes of area maps.
pub const fn mapped_size(capacity: usize) -> usize {
    size_of::<Header>() + capacity
}

/// A ring in mapped memory.
#[derive(Clone, Copy)]
pub struct Ring {
    header: *mut Header,
    capacity: u64,
}

// SAFETY: everything shared goes through atomics; message bytes are only
// read once their record's commit mark says they are complete.
unsafe impl Send for Ring {}
// SAFETY: as above.
unsafe impl Sync for Ring {}

impl Ring {
    /// Starts a ring in zeroed memory.
    ///
    /// # Safety
    ///
    /// `at` is 8-byte aligned, zeroed, and maps `mapped_size(capacity)`
    /// bytes for as long as the ring is used.
    pub unsafe fn init(at: *mut u8, capacity: usize, pid: u32) -> Ring {
        let header = at.cast::<Header>();
        let fixed = Header {
            magic: MAGIC,
            capacity: capacity as u64,
            head: AtomicU64::new(0),
            waiting: AtomicU32::new(0),
            pid,
            read: AtomicU64::new(0),
        };
        // SAFETY: the caller's guarantee.
        unsafe { header.write(fixed) };
        Ring { header, capacity: capacity as u64 }
    }

    /// A ring another process started, if `at` holds one.
    ///
    /// # Safety
    ///
    /// `at` is 8-byte aligned and maps the whole ring, `mapped` bytes,
    /// for as long as it is used.
    pub unsafe fn attach(at: *mut u8, mapped: usize) -> Option<Ring> {
        let header = at.cast::<Header>();
        if mapped < size_of::<Header>() {
            return None;
        }
        // SAFETY: the caller's guarantee, and a header's room is there.
        let fixed = unsafe { &*header };
        let (magic, capacity) = (fixed.magic, fixed.capacity);
        let fits = (capacity as usize).checked_add(size_of::<Header>()).is_some_and(|n| n <= mapped);
        (magic == MAGIC && fits && capacity % 8 == 0 && capacity != 0).then_some(Ring { header, capacity })
    }

    fn header(&self) -> &Header {
        // SAFETY: mapped for as long as the ring is used; `Ring`'s
        // constructors' contract.
        unsafe { &*self.header }
    }

    fn area(&self) -> *mut u8 {
        // SAFETY: the area follows the header in the mapping.
        unsafe { self.header.cast::<u8>().add(size_of::<Header>()) }
    }

    /// The record at `pos`, which is 8-byte aligned, with `RECORD` bytes
    /// of room before the end of the area.
    fn record(&self, pos: u64) -> *mut Record {
        // SAFETY: inside the area, by the callers' reservation.
        unsafe { self.area().add((pos % self.capacity) as usize).cast() }
    }

    /// Logs `message`, cut to what a record can hold. Never waits. True
    /// when the reader sleeps and must be woken.
    pub fn write(&self, priority: u32, time_ns: u64, message: &[u8]) -> bool {
        let room = (self.capacity as usize / 4).min(MAX_MESSAGE).saturating_sub(RECORD);
        let message = &message[..message.len().min(room)];
        let total = align8(RECORD + message.len()) as u64;
        let h = self.header();
        let mut head = h.head.load(Ordering::Relaxed);
        let (start, end_of_area) = loop {
            let left = self.capacity - head % self.capacity;
            // A record stays in one piece: the rest of the area is padding.
            let pad = if left < total { left } else { 0 };
            match h.head.compare_exchange_weak(head, head + pad + total, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => break (head + pad, (pad != 0).then_some(head)),
                Err(now) => head = now,
            }
        };
        if let Some(pad) = end_of_area {
            // SAFETY: reserved above; at least 8 bytes, the stamp.
            unsafe { &(*self.record(pad)).stamp }.store(pad | PADDING, Ordering::Release);
        }
        let r = self.record(start);
        // SAFETY: reserved above, in one piece. The stamp goes first, so
        // that a reader sees this record unfinished, not an old one.
        let stamp = unsafe { &(*r).stamp };
        stamp.store(0, Ordering::Release);
        let fields = Fields { time_ns, len: message.len() as u32, priority };
        // SAFETY: as above.
        let at = unsafe { addr_of_mut!((*r).fields) };
        // SAFETY: as above.
        unsafe { at.write(fields) };
        // SAFETY: the room was reserved for the message.
        let text = unsafe { r.cast::<u8>().add(RECORD) };
        // SAFETY: as above.
        unsafe { copy_nonoverlapping(message.as_ptr(), text, message.len()) };
        stamp.store(start | COMMITTED, Ordering::Release);
        h.waiting.swap(0, Ordering::AcqRel) == 1
    }

    /// Says the reader is going to sleep. False when there is something
    /// new after `reader`, and it should read on instead.
    pub fn prepare_sleep(&self, reader: &Reader) -> bool {
        let h = self.header();
        h.waiting.store(1, Ordering::SeqCst);
        if h.head.load(Ordering::SeqCst) != reader.tail {
            h.waiting.store(0, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// A writer's wake-up didn't get through: the next write tries again.
    pub fn rearm(&self) {
        self.header().waiting.store(1, Ordering::SeqCst);
    }

    /// A reader that goes on where the last one stopped.
    pub fn reader(&self) -> Reader {
        Reader { tail: self.header().read.load(Ordering::Acquire) }
    }

    /// The process the ring was made for.
    pub fn pid(&self) -> u32 {
        self.header().pid
    }
}

/// What a read found.
#[derive(Debug, PartialEq, Eq)]
pub enum Read {
    /// A message of this many bytes, copied to the buffer.
    Message { priority: u32, time_ns: u64, len: usize },
    /// Nothing, or only a message not finished yet.
    Empty,
    /// The writers went round over messages not read: some are lost.
    Lost,
}

/// A reader's place; one reader per ring.
#[derive(Clone, Copy, Default)]
pub struct Reader {
    tail: u64,
}

impl Reader {
    /// The next message, into `buf`, at most `MAX_MESSAGE` bytes.
    pub fn read(&mut self, ring: &Ring, buf: &mut [u8]) -> Read {
        loop {
            let head = ring.header().head.load(Ordering::Acquire);
            if self.tail == head {
                return Read::Empty;
            }
            if head - self.tail > ring.capacity {
                return self.lost(ring, head);
            }
            let r = ring.record(self.tail);
            // SAFETY: inside the area; the stamp is atomic.
            let stamp = unsafe { &(*r).stamp }.load(Ordering::Acquire);
            if stamp == self.tail | PADDING {
                self.tail += ring.capacity - self.tail % ring.capacity;
                continue;
            }
            if stamp != self.tail | COMMITTED {
                // Unfinished, or being overwritten: the next read tells.
                let head = ring.header().head.load(Ordering::Acquire);
                return if head - self.tail > ring.capacity { self.lost(ring, head) } else { Read::Empty };
            }
            // SAFETY: committed at this position; a writer overwriting it
            // meanwhile is seen below.
            let fields = unsafe { addr_of!((*r).fields) };
            // SAFETY: as above.
            let Fields { time_ns, len: stored, priority } = unsafe { fields.read() };
            let stored = (stored as usize).min(MAX_MESSAGE);
            let len = stored.min(buf.len());
            // SAFETY: inside the area.
            let text = unsafe { r.cast::<u8>().add(RECORD) };
            // SAFETY: as above, and bounded by `buf`.
            unsafe { copy_nonoverlapping(text, buf.as_mut_ptr(), len) };
            // Writers reserve before they write: if none has reserved this
            // far, the copy above is whole. A new record needn't start where
            // this one did, so its stamp alone couldn't tell.
            fence(Ordering::Acquire);
            let head = ring.header().head.load(Ordering::Relaxed);
            if head - self.tail > ring.capacity {
                return self.lost(ring, head);
            }
            self.tail += align8(RECORD + stored) as u64;
            ring.header().read.store(self.tail, Ordering::Release);
            return Read::Message { priority, time_ns, len };
        }
    }

    /// Starts again at the oldest place certain to start a record.
    fn lost(&mut self, ring: &Ring, head: u64) -> Read {
        let oldest = head.saturating_sub(ring.capacity);
        self.tail = oldest.next_multiple_of(ring.capacity).min(head);
        ring.header().read.store(self.tail, Ordering::Release);
        Read::Lost
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    use std::vec::Vec;

    fn ring(capacity: usize) -> (Ring, Vec<u64>) {
        let mut mem = vec![0u64; mapped_size(capacity) / 8 + 1];
        // SAFETY: aligned, zeroed and big enough; the vector is returned.
        let r = unsafe { Ring::init(mem.as_mut_ptr().cast(), capacity, 7) };
        (r, mem)
    }

    fn text<'a>(r: &Ring, rd: &mut Reader, b: &'a mut [u8]) -> Option<(u32, &'a [u8])> {
        match rd.read(r, b) {
            Read::Message { priority, len, .. } => Some((priority, &b[..len])),
            _ => None,
        }
    }

    #[test]
    fn a_message_goes_through() {
        let (r, _m) = ring(4096);
        let mut rd = Reader::default();
        let mut b = [0u8; 256];
        assert_eq!(rd.read(&r, &mut b), Read::Empty);
        r.write(14, 5, b"hello");
        assert_eq!(text(&r, &mut rd, &mut b), Some((14, &b"hello"[..])));
        assert_eq!(rd.read(&r, &mut b), Read::Empty);
    }

    #[test]
    fn records_wrap_in_one_piece() {
        let (r, _m) = ring(1024);
        let mut rd = Reader::default();
        let mut b = [0u8; 256];
        for i in 0..200u32 {
            let msg = [i as u8; 100];
            r.write(i, 0, &msg);
            assert_eq!(text(&r, &mut rd, &mut b), Some((i, &msg[..])), "{i}");
        }
    }

    #[test]
    fn an_overrun_reader_says_so_and_goes_on() {
        let (r, _m) = ring(1024);
        let mut rd = Reader::default();
        let mut b = [0u8; 256];
        for i in 0..100u32 {
            r.write(i, 0, &[1u8; 100]);
        }
        assert_eq!(rd.read(&r, &mut b), Read::Lost);
        let mut last = None;
        while let Some((p, _)) = text(&r, &mut rd, &mut b) {
            last = Some(p);
        }
        assert_eq!(last, Some(99));
    }

    #[test]
    fn a_long_message_is_cut() {
        let (r, _m) = ring(1 << 16);
        let mut rd = Reader::default();
        let mut b = vec![0u8; MAX_MESSAGE];
        r.write(1, 0, &vec![b'x'; 100_000]);
        let (_, t) = text(&r, &mut rd, &mut b).unwrap();
        assert_eq!(t.len(), MAX_MESSAGE - RECORD);
    }

    #[test]
    fn an_unfinished_message_holds_the_reader_only() {
        let (r, _m) = ring(4096);
        let mut rd = Reader::default();
        let mut b = [0u8; 64];
        r.write(1, 0, b"one");
        // A writer stopped between its reservation and its commit.
        let at = r.header().head.fetch_add(64, Ordering::AcqRel);
        r.write(3, 0, b"three");
        assert_eq!(text(&r, &mut rd, &mut b), Some((1, &b"one"[..])));
        assert_eq!(rd.read(&r, &mut b), Read::Empty);
        // SAFETY: the reservation above.
        unsafe { &(*r.record(at)).stamp }.store(at | PADDING, Ordering::Release);
    }

    #[test]
    fn the_sleeping_reader_is_woken_once() {
        let (r, _m) = ring(4096);
        let rd = Reader::default();
        assert!(!r.write(1, 0, b"a"));
        let mut b = [0u8; 16];
        let mut rd2 = rd;
        rd2.read(&r, &mut b);
        assert!(r.prepare_sleep(&rd2));
        assert!(r.write(1, 0, b"b"));
        assert!(!r.write(1, 0, b"c"));
        assert!(!r.prepare_sleep(&rd2), "something to read");
    }

    #[test]
    fn a_new_reader_goes_on_where_the_last_stopped() {
        let (r, _m) = ring(4096);
        let mut rd = r.reader();
        let mut b = [0u8; 16];
        r.write(1, 0, b"old");
        assert!(text(&r, &mut rd, &mut b).is_some());
        r.write(2, 0, b"new");
        let mut next = r.reader();
        assert_eq!(text(&r, &mut next, &mut b), Some((2, &b"new"[..])));
        assert_eq!(next.read(&r, &mut b), Read::Empty);
    }

    #[test]
    fn a_failed_wake_up_is_tried_again() {
        let (r, _m) = ring(4096);
        let rd = r.reader();
        assert!(r.prepare_sleep(&rd));
        assert!(r.write(1, 0, b"a"));
        assert!(!r.write(1, 0, b"b"));
        r.rearm();
        assert!(r.write(1, 0, b"c"));
    }

    #[test]
    fn writers_together_never_corrupt_a_message() {
        let (r, _m) = ring(1 << 16);
        let done = std::sync::atomic::AtomicU32::new(0);
        std::thread::scope(|s| {
            for w in 0..4u8 {
                let done = &done;
                s.spawn(move || {
                    for i in 0..50_000usize {
                        r.write(u32::from(w), 0, &vec![w + 1; 8 + i % 64]);
                    }
                    done.fetch_add(1, Ordering::SeqCst);
                });
            }
            let mut rd = Reader::default();
            let mut b = [0u8; 256];
            let (mut got, mut idle) = (0u64, 0u32);
            while done.load(Ordering::SeqCst) < 4 || idle < 2 {
                match rd.read(&r, &mut b) {
                    Read::Message { priority, len, .. } => {
                        idle = 0;
                        got += 1;
                        assert!(b[..len].iter().all(|&c| u32::from(c) == priority + 1), "torn");
                    }
                    Read::Lost => {}
                    Read::Empty => idle += 1,
                }
            }
            assert!(got > 0);
        });
    }

    #[test]
    fn a_bad_mapping_is_refused() {
        let mut mem = [0u64; 16];
        // SAFETY: aligned and that big.
        assert!(unsafe { Ring::attach(mem.as_mut_ptr().cast(), 128) }.is_none());
    }
}
