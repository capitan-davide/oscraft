//! Kernel tracing (QNX's instrumented kernel and `TraceEvent()`, System
//! Analysis Toolkit): the kernel writes events in QNX's format into
//! buffers that QNX's `tracelogger` (`libtracelog`) reads.
//!
//! Contract (from `<sys/trace.h>`, `<sys/sysmsg.h>`, the SAT guide, and
//! what libc's `TraceEvent()`, `libtracelog` and `traceprinter` do):
//! - `TraceEvent(code, ...)` is kernel call 1 with x0 pointing to the
//!   array { code, arguments... } (the top four bits of `code` count its
//!   words). `_NTO_TRACE_ALLOCBUFFER`, `_DEALLOCBUFFER` and
//!   `_LOGGER_DETACH` are instead `_SYS_TRACE` messages to procnto
//!   (subtypes `_SYS_TRACE_ALLOCBUF`, `_DEALLOCBUF`, `_DETACH`).
//! - Buffers (`tracebuf_t`) are 16 KB: a 208-byte header { reserved1,
//!   reserved2, flags, num_events, seq_buff_num, ... } and 1011 16-byte
//!   event slots. `ALLOCBUF` (`bufs_per_cpu`) allocates that many per CPU,
//!   contiguous, CPU 0's first, mapped into the logger (the reply's
//!   `buff_base`). Each CPU writes its own buffers.
//! - A slot is { header, timestamp (low 32 bits of the cycle counter),
//!   data[2] }. The header: bits 30-31 the structure (simple, or begin,
//!   continue and end of a combine event whose payload is two words per
//!   slot, each slot with the same timestamp), 24-29 the CPU, 10-14 the
//!   internal class, 0-9 the event.
//! - A buffer starts with CONTROL TIME { msb, lsb } (tracelogger sorts
//!   ring-mode buffers by it) and CONTROL BUFFER { sequence, count } (the
//!   logger writes the count), and ends with CONTROL BUFFER_END { COOKIE,
//!   COOKIE }; its header then holds the number of slots used. Linear mode:
//!   full buffers are handed out by `_NTO_TRACE_WAITBUFFER`, which a
//!   logger thread bound to each CPU calls (returning the buffer's index,
//!   or `_TRACE_WAITBUFFER_DONE` once tracing has stopped and all are
//!   out); the logger frees a buffer by clearing its `flags` and
//!   `num_events`. With no free buffer, events are dropped. Ring mode:
//!   each CPU reuses its buffers in turn; the logger reads them all after
//!   stopping.
//! - `_NTO_TRACE_LOGGER_ATTACH(start, stop)`: the logger's events for
//!   tracing started and stopped by others.
//! - `_NTO_TRACE_START` reports the processes and threads there are
//!   (names, states) first, unless `_STARTNOSTATE`.
//! - Filters: external classes map to internal ones (`_NTO_TRACE_GET*()`
//!   in the library reference): kernel call exits are the call + 128,
//!   interrupted calls + 256, 64-bit variants have bit 9 (0x200); process
//!   events are (bit index + 1) << 6. Everything is traced, in fast mode,
//!   until a filter says otherwise.
//!
//! Not yet: per-pid/tid filters (accepted), event handlers, PMU events,
//! `_NTO_TRACE_SETBUFFER` (reserved memory), IPIs to report the other
//! CPUs' running threads at start.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use super::KerResult;
use super::errno::{EBUSY, EEXIST, EINVAL, ENOMEM, ENOTSUP, EOVERFLOW, EPERM};
use super::event::{self, SigEvent};
use super::process::{self, Process};
use super::umem::{self, Space};
use crate::arch::{irq_restore, irq_save};
use crate::percpu::{self, MAX_CPUS};
use crate::sched::{self, State, Thread, ThreadRef};
use crate::sync::IrqSpinLock;

pub const BUF_SIZE: u64 = 16 * 1024;
const HDR_SIZE: u64 = 208;
const SLOTS: usize = ((BUF_SIZE - HDR_SIZE) / 16) as usize;
const COOKIE: u32 = 0xe6e6_e6e6;
// tracebuf_t header fields
const H_FLAGS: u64 = 16;
const H_NUM: u64 = 20;
const H_SEQ: u64 = 24;
const FLAGS_RING: u32 = 0x1_0000;
const FLAGS_WRITING: u32 = 0x2_0000;
/// A full buffer, not yet freed by the logger.
const FLAGS_FULL: u32 = 0x1;

// Internal classes (`_TRACE_*_C` >> 10).
pub const C_CONTROL: u32 = 1;
pub const C_KERCALL: u32 = 2;
pub const C_INT: u32 = 3;
pub const C_PR_TH: u32 = 4;
pub const C_SYSTEM: u32 = 5;
pub const C_USER: u32 = 6;
pub const C_COMM: u32 = 7;
const CLASSES: usize = 12;
// Control events
const CONTROL_TIME: u32 = 1;
const CONTROL_BUFFER: u32 = 2;
const CONTROL_BUFFER_END: u32 = 4;
// Process and thread events (`_TRACE_PR_TH_*`)
pub const TH_CREATE: u32 = 24;
pub const TH_DESTROY: u32 = 25;
pub const PROC_CREATE: u32 = 1 << 6;
pub const PROC_DESTROY: u32 = 2 << 6;
pub const PROC_CREATE_NAME: u32 = 3 << 6;
pub const PROC_DESTROY_NAME: u32 = 4 << 6;
pub const PROC_THREAD_NAME: u32 = 5 << 6;
/// `_TRACE_INT_DELIVER`.
pub const INT_DELIVER: u32 = 7;
/// Kernel call event offsets.
pub const KERCALL_EXIT: u32 = 128;
pub const KERCALL64: u32 = 0x200;
// Structure bits
const STRUCT_CB: u32 = 1 << 30;
const STRUCT_CC: u32 = 2 << 30;
const STRUCT_CE: u32 = 3 << 30;

/// Tracing runs.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static RING: AtomicBool = AtomicBool::new(false);
/// Enabled events and wide events: a bit per (internal class, event).
static ENABLED: [AtomicU64; CLASSES * 16] = [const { AtomicU64::new(u64::MAX) }; CLASSES * 16];
static WIDE: [AtomicU64; CLASSES * 16] = [const { AtomicU64::new(0) }; CLASSES * 16];
/// The buffers as the CPUs write them: their kernel address and how many
/// per CPU (0: none, nothing is written).
static BUF_KVA: AtomicU64 = AtomicU64::new(0);
static PER_CPU: AtomicUsize = AtomicUsize::new(0);

/// The buffers' memory, as procnto's `trace_masks` records it: physical
/// and kernel address, bytes, and the logger's address for them.
#[derive(Clone, Copy)]
struct Buffers {
    pa: u64,
    kva: u64,
    len: u64,
    user: u64,
}

/// The buffers there are (procnto's `buff_0_ptr`; one set at a time).
/// Held only to decide; the memory is mapped and freed with it let go.
static BUFFERS: IrqSpinLock<Option<Buffers>> = IrqSpinLock::new(None);
/// A buffer filled, or tracing stopped, since the logger threads were
/// last looked at (see `kick`).
static PENDING: AtomicBool = AtomicBool::new(false);

/// A CPU's writing state.
struct CpuTrace {
    /// The buffer being written (global index) and its next slot.
    cur: Option<usize>,
    slot: usize,
    /// Buffers filled since tracing started (the next one's number).
    seq: u32,
    /// The next of its buffers to try (index among the CPU's).
    next: usize,
    /// The high half of the clock in the last TIME event.
    msb: u32,
    /// Full buffers for the logger (linear mode).
    full: VecDeque<usize>,
    /// The logger thread waiting for one.
    waiter: Option<ThreadRef>,
    /// `IDLE`, `ON` or `STOPPED` (procnto's `trc_state`).
    state: u8,
}

/// A CPU's tracing state: not tracing; tracing; stopped, with buffers the
/// logger hasn't been told the end of. As procnto's `trace_act_start`,
/// `trace_act_stop` and `nto_trace_waitbuffer`: a stop in linear mode
/// leaves the CPU `STOPPED` until its logger thread's WAITBUFFER has taken
/// the full buffers and returned the end; the buffers can't be freed
/// before every CPU is `IDLE` again (EBUSY).
const IDLE: u8 = 0;
const ON: u8 = 1;
const STOPPED: u8 = 2;

impl CpuTrace {
    const fn new() -> Self {
        Self {
            cur: None,
            slot: 0,
            seq: 1,
            next: 0,
            msb: 0,
            full: VecDeque::new(),
            waiter: None,
            state: IDLE,
        }
    }
}

static CPUS: [IrqSpinLock<CpuTrace>; MAX_CPUS] =
    [const { IrqSpinLock::new(CpuTrace::new()) }; MAX_CPUS];

/// The logger (`_NTO_TRACE_LOGGER_ATTACH`): its pid and its start and stop
/// events.
struct Logger {
    pid: i32,
    start: Option<SigEvent>,
    stop: Option<SigEvent>,
}

static LOGGER: IrqSpinLock<Option<Logger>> = IrqSpinLock::new(None);

fn bit_index(class: u32, event: u32) -> usize {
    class as usize * 1024 + (event & 0x3ff) as usize
}

fn get(map: &[AtomicU64], class: u32, event: u32) -> bool {
    let i = bit_index(class, event);
    (class as usize) < CLASSES && map[i / 64].load(Ordering::Relaxed) & (1 << (i % 64)) != 0
}

fn set(map: &[AtomicU64], class: u32, event: u32, on: bool) {
    if class as usize >= CLASSES {
        return;
    }
    let i = bit_index(class, event);
    if on {
        map[i / 64].fetch_or(1 << (i % 64), Ordering::Relaxed);
    } else {
        map[i / 64].fetch_and(!(1 << (i % 64)), Ordering::Relaxed);
    }
}

// ANCHOR: trace_on
/// Is event `event` of internal class `class` traced now?
pub fn on(class: u32, event: u32) -> bool {
    ACTIVE.load(Ordering::Relaxed) && get(&ENABLED, class, event)
}

/// Is it traced in wide mode?
pub fn wide(class: u32, event: u32) -> bool {
    get(&WIDE, class, event)
}
// ANCHOR_END: trace_on

fn timestamp() -> u64 {
    crate::arch::timer::counter()
}

fn buf_addr(idx: usize) -> u64 {
    BUF_KVA.load(Ordering::Relaxed) + idx as u64 * BUF_SIZE
}

fn put32(addr: u64, v: u32) {
    // SAFETY: `addr` lies in the trace buffers, mapped while PER_CPU != 0.
    unsafe { core::ptr::write_volatile(addr as *mut u32, v) }
}

fn get32(addr: u64) -> u32 {
    // SAFETY: as for `put32`.
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn put_slot(t: &mut CpuTrace, idx: usize, header: u32, ts: u32, d0: u32, d1: u32) {
    let a = buf_addr(idx) + HDR_SIZE + t.slot as u64 * 16;
    put32(a, header);
    put32(a + 4, ts);
    put32(a + 8, d0);
    put32(a + 12, d1);
    t.slot += 1;
}

fn header(cpu: usize, class: u32, event: u32) -> u32 {
    ((cpu as u32 & 0x3f) << 24) | (class << 10) | (event & 0x3ff)
}

/// Start a new buffer on CPU `cpu`; false if none is free (linear mode).
fn start_buffer(t: &mut CpuTrace, cpu: usize, ts: u64) -> bool {
    let per = PER_CPU.load(Ordering::Relaxed);
    if per == 0 {
        return false;
    }
    let ring = RING.load(Ordering::Relaxed);
    let idx = (0..per)
        .map(|i| (t.next + i) % per)
        .find(|&i| ring || get32(buf_addr(cpu * per + i) + H_FLAGS) == 0);
    let Some(i) = idx else {
        return false;
    };
    t.next = (i + 1) % per;
    let idx = cpu * per + i;
    let b = buf_addr(idx);
    put32(
        b + H_FLAGS,
        FLAGS_WRITING | if ring { FLAGS_RING } else { 0 },
    );
    put32(b + H_NUM, 0);
    put32(b + H_SEQ, t.seq);
    t.cur = Some(idx);
    t.slot = 0;
    let (msb, lsb) = ((ts >> 32) as u32, ts as u32);
    put_slot(t, idx, header(cpu, C_CONTROL, CONTROL_TIME), lsb, msb, lsb);
    let seq = t.seq;
    put_slot(t, idx, header(cpu, C_CONTROL, CONTROL_BUFFER), lsb, seq, 0);
    t.msb = msb;
    t.seq += 1;
    true
}

/// End CPU `cpu`'s buffer: BUFFER_END, the slot count; in linear mode the
/// logger gets it.
fn finish_buffer(t: &mut CpuTrace, cpu: usize) {
    let Some(idx) = t.cur.take() else {
        return;
    };
    let lsb = timestamp() as u32;
    put_slot(
        t,
        idx,
        header(cpu, C_CONTROL, CONTROL_BUFFER_END),
        lsb,
        COOKIE,
        COOKIE,
    );
    let b = buf_addr(idx);
    put32(b + H_NUM, t.slot as u32);
    let ring = RING.load(Ordering::Relaxed);
    put32(b + H_FLAGS, FLAGS_FULL | if ring { FLAGS_RING } else { 0 });
    if !ring {
        t.full.push_back(idx);
        PENDING.store(true, Ordering::Release);
    }
}

/// Write an event of `data` words (beyond the timestamp) on CPU `cpu`.
fn write_event(t: &mut CpuTrace, cpu: usize, class: u32, event: u32, data: &[u32]) {
    // A buffer holds TIME, BUFFER, a possible TIME on the clock's
    // rollover and BUFFER_END besides the event.
    let data = &data[..data.len().min(2 * (SLOTS - 4))];
    let slots = data.len().div_ceil(2).max(1);
    let ts = timestamp();
    if t.cur.is_some_and(|_| t.slot + slots + 2 > SLOTS) {
        finish_buffer(t, cpu);
    }
    if t.cur.is_none() && !start_buffer(t, cpu, ts) {
        return; // dropped
    }
    let idx = t.cur.unwrap();
    let (msb, lsb) = ((ts >> 32) as u32, ts as u32);
    if msb != t.msb {
        put_slot(t, idx, header(cpu, C_CONTROL, CONTROL_TIME), lsb, msb, lsb);
        t.msb = msb;
    }
    let h = header(cpu, class, event);
    let word = |i: usize| data.get(i).copied().unwrap_or(0);
    if slots == 1 {
        put_slot(t, idx, h, lsb, word(0), word(1));
        return;
    }
    for s in 0..slots {
        let st = match s {
            0 => STRUCT_CB,
            _ if s == slots - 1 => STRUCT_CE,
            _ => STRUCT_CC,
        };
        put_slot(t, idx, h | st, lsb, word(2 * s), word(2 * s + 1));
    }
}

// ANCHOR: emit
/// Trace event `event` of internal class `class` with `data` (words after
/// the timestamp: up to two make a simple event, more a combine event),
/// on the calling CPU, if it is traced.
pub fn emit(class: u32, event: u32, data: &[u32]) {
    if !on(class, event) {
        return;
    }
    let daif = irq_save();
    let cpu = percpu::this().cpu as usize;
    write_event(&mut CPUS[cpu].lock(), cpu, class, event, data);
    irq_restore(daif);
}
// ANCHOR_END: emit

/// A string as event words: its bytes and a NUL, zero-padded.
pub fn str_words(s: &str, out: &mut Vec<u32>) {
    let mut b: Vec<u8> = s.bytes().collect();
    b.push(0);
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    out.extend(
        b.chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])),
    );
}

/// A thread state change (`_NTO_TRACE_THREAD`): `state`, pid, tid.
pub fn thread_state(t: &Thread, state: State) {
    emit(C_PR_TH, state.qnx() as u32, &[t.process.pid as u32, t.ptid]);
}

/// A thread created (THCREATE, then its name if it has one and READY).
pub fn thread_created(t: &Thread) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    emit(C_PR_TH, TH_CREATE, &[t.process.pid as u32, t.ptid]);
}

/// A thread's name (PROCTHREAD_NAME): pid, tid, name.
pub fn thread_name(t: &Thread, name: &str) {
    if !on(C_PR_TH, PROC_THREAD_NAME) {
        return;
    }
    let mut d = alloc::vec![t.process.pid as u32, t.ptid];
    str_words(name, &mut d);
    emit(C_PR_TH, PROC_THREAD_NAME, &d);
}

/// A process created or replaced by a program (PROCCREATE_NAME): ppid,
/// pid, the program's path.
pub fn process_named(p: &Process) {
    process_event(p, PROC_CREATE_NAME);
}

/// A process created (PROCCREATE): ppid, pid.
pub fn process_created(p: &Process) {
    emit(
        C_PR_TH,
        PROC_CREATE,
        &[p.ppid.load(Ordering::Relaxed) as u32, p.pid as u32],
    );
}

/// A process ended (PROCDESTROY, PROCDESTROY_NAME).
pub fn process_ended(p: &Process) {
    emit(
        C_PR_TH,
        PROC_DESTROY,
        &[p.ppid.load(Ordering::Relaxed) as u32, p.pid as u32],
    );
    process_event(p, PROC_DESTROY_NAME);
}

fn process_event(p: &Process, event: u32) {
    if !on(C_PR_TH, event) {
        return;
    }
    let mut d = alloc::vec![p.ppid.load(Ordering::Relaxed) as u32, p.pid as u32];
    let name = p.name.lock().clone();
    str_words(&alloc::format!("/{name}"), &mut d);
    emit(C_PR_TH, event, &d);
}

/// Report the processes and threads there are (at start).
fn report_state() {
    for p in process::all() {
        process_named(&p);
        for t in sched::threads_of(&p) {
            emit(C_PR_TH, TH_CREATE, &[p.pid as u32, t.ptid]);
            let name = t.qnx_name.lock().clone();
            if !name.is_empty() {
                thread_name(&t, &name);
            }
            thread_state(&t, t.state());
        }
    }
}

/// Wake the logger threads that have a buffer to take or are done (from
/// places where waking is safe: the end of a kernel call, the clock).
pub fn kick() {
    if !PENDING.swap(false, Ordering::Acquire) {
        return;
    }
    for cpu in 0..crate::smp::online_count() as usize {
        let w = {
            let mut t = CPUS[cpu].lock();
            if t.waiter.is_some() && (!t.full.is_empty() || t.state != ON) {
                t.waiter.take()
            } else {
                None
            }
        };
        if let Some(w) = w {
            sched::wake(&w);
        }
    }
}

/// Deliver the logger's start or stop event, if another process changed
/// tracing.
fn notify_logger(start: bool) {
    let me = process::current().pid;
    let target = {
        let l = LOGGER.lock();
        l.as_ref()
            .filter(|l| l.pid != me)
            .and_then(|l| Some((l.pid, if start { l.start } else { l.stop }?)))
    };
    if let Some((pid, ev)) = target
        && let Some(p) = process::lookup(pid)
    {
        let _ = event::deliver(&p, None, &ev, 0);
    }
}

fn start(report: bool) -> KerResult {
    if PER_CPU.load(Ordering::Relaxed) == 0 {
        return Err(EINVAL);
    }
    for cpu in &CPUS[..crate::smp::online_count() as usize] {
        let mut t = cpu.lock();
        t.cur = None;
        t.seq = 1;
        t.next = 0;
        t.full.clear();
        t.state = ON;
    }
    ACTIVE.store(true, Ordering::Relaxed);
    if report {
        report_state();
    }
    notify_logger(true);
    Ok(0)
}

/// Stop tracing; the buffers being written go to the logger, which (in
/// linear mode) is then told the end on each CPU.
pub fn stop() {
    stop_as(false);
}

/// Stop tracing; `forced` (the logger detaches): no CPU waits for its
/// logger thread any more (procnto's `force_stop`).
fn stop_as(forced: bool) {
    ACTIVE.store(false, Ordering::Relaxed);
    PENDING.store(true, Ordering::Release);
    let ring = RING.load(Ordering::Relaxed);
    for (cpu, t) in CPUS.iter().enumerate().take(crate::smp::online_count() as usize) {
        let mut t = t.lock();
        finish_buffer(&mut t, cpu);
        t.state = if ring || forced { IDLE } else { STOPPED };
    }
    kick();
}

fn flush() {
    for (cpu, t) in CPUS.iter().enumerate() {
        finish_buffer(&mut t.lock(), cpu);
    }
    kick();
}

/// The CPU whose buffers a logger thread takes: the one its runmask names,
/// else the one it runs on.
fn home_cpu(t: &Thread) -> usize {
    let mask = t.runmask.load(Ordering::Relaxed) & crate::smp::online_mask();
    if mask.count_ones() == 1 {
        mask.trailing_zeros() as usize
    } else {
        percpu::this().cpu as usize
    }
}

/// `_NTO_TRACE_WAITBUFFER(&idx)`.
fn wait_buffer(idx_out: u64) -> KerResult {
    let me = sched::current();
    let cpu = home_cpu(me);
    loop {
        {
            let mut t = CPUS[cpu].lock();
            if let Some(i) = t.full.pop_front() {
                drop(t);
                umem::write(Space::current(), idx_out, &(i as u32).to_le_bytes())?;
                return Ok(0);
            }
            // The end: on a CPU not tracing, in ring mode, or once a stopped
            // CPU's buffers are all taken (then it is idle).
            if t.state == IDLE || RING.load(Ordering::Relaxed) {
                return Ok(1); // _TRACE_WAITBUFFER_DONE
            }
            if t.state == STOPPED {
                t.state = IDLE;
                return Ok(1);
            }
            t.waiter = Some(me.arc());
            me.wake_error.store(0, Ordering::Relaxed);
        }
        // Marked blocked without the lock (that traces the state change,
        // which takes it); a buffer that came meanwhile is checked for
        // again, and the thread wakes itself.
        sched::prepare_block(State::TraceBuffer);
        {
            let t = CPUS[cpu].lock();
            if !t.full.is_empty() || t.state != ON {
                drop(t);
                sched::wake(me);
            }
        }
        sched::block();
        if let e @ 1.. = me.wake_error.swap(0, Ordering::Relaxed) {
            CPUS[cpu].lock().waiter = None;
            return Err(e);
        }
    }
}

// External classes (`_NTO_TRACE_*`)
const X_CONTROL: u32 = 1;
const X_KERCALL: u32 = 2;
const X_KERCALLENTER: u32 = 3;
const X_KERCALLEXIT: u32 = 4;
const X_KERCALLINT: u32 = 5;
const X_INT: u32 = 6;
const X_PROCESS: u32 = 9;
const X_THREAD: u32 = 10;
const X_USER: u32 = 12;
const X_SYSTEM: u32 = 13;
const X_COMM: u32 = 14;
const X_QUIP: u32 = 17;
const X_SEC: u32 = 18;
const X_QVM: u32 = 19;
const X_PMU: u32 = 20;

/// The internal (class, events) an external class and event (None: all of
/// the class) stand for.
fn internal(class: u32, event: Option<u32>) -> Option<(u32, Vec<u32>)> {
    let all = |c: u32| Some((c, (0..1024).collect()));
    let range = |lo: u32| {
        (lo..lo + 128)
            .chain(lo + KERCALL64..lo + KERCALL64 + 128)
            .collect()
    };
    let kercall = |off: u32| match event {
        None => Some((C_KERCALL, range(off))),
        Some(e) => Some((C_KERCALL, alloc::vec![e + off, (e + off) | KERCALL64])),
    };
    match class {
        X_CONTROL => all(C_CONTROL),
        X_KERCALL => match event {
            None => all(C_KERCALL),
            Some(e) => Some((
                C_KERCALL,
                [0, KERCALL_EXIT, 2 * KERCALL_EXIT]
                    .iter()
                    .flat_map(|o| [e + o, (e + o) | KERCALL64])
                    .collect(),
            )),
        },
        X_KERCALLENTER => kercall(0),
        X_KERCALLEXIT => kercall(KERCALL_EXIT),
        X_KERCALLINT => kercall(2 * KERCALL_EXIT),
        // Per-interrupt masks: the whole class.
        X_INT => all(C_INT),
        X_PROCESS => match event {
            None => Some((C_PR_TH, (1..=5).map(|b| b << 6).collect())),
            Some(e) if e != 0 => Some((C_PR_TH, alloc::vec![(e.trailing_zeros() + 1) << 6])),
            Some(_) => None,
        },
        X_THREAD => match event {
            None => Some((C_PR_TH, (0..64).collect())),
            Some(e) => Some((C_PR_TH, alloc::vec![e])),
        },
        X_USER | X_COMM | X_QUIP | X_SEC | X_QVM | X_PMU => {
            let c = match class {
                X_USER => C_USER,
                X_COMM => C_COMM,
                X_QUIP => 8,
                X_SEC => 9,
                X_QVM => 10,
                _ => 11,
            };
            match event {
                None => all(c),
                Some(e) => Some((c, alloc::vec![e])),
            }
        }
        X_SYSTEM => match event {
            None => all(C_SYSTEM),
            Some(e) => Some((C_SYSTEM, alloc::vec![e, e | KERCALL64])),
        },
        _ => None,
    }
}

fn set_filter(map: &[AtomicU64], class: u32, event: Option<u32>, on: bool) -> KerResult {
    let (c, events) = internal(class, event).ok_or(EINVAL)?;
    for e in events {
        set(map, c, e, on);
    }
    Ok(0)
}

fn set_all(map: &[AtomicU64], on: bool) {
    for w in map {
        w.store(if on { u64::MAX } else { 0 }, Ordering::Relaxed);
    }
}

/// An event the caller inserts (user events, `trace_logf()` & co.).
fn insert(class: u32, event: u32, data: &[u32]) -> KerResult {
    if class as usize >= CLASSES || event > 0x3ff {
        return Err(EINVAL);
    }
    emit(class, event, data);
    Ok(0)
}

/// Words of user memory at `addr`.
fn user_words(addr: u64, words: u64) -> Result<Vec<u32>, u32> {
    let n = (words as usize).min(2 * SLOTS);
    let mut b = alloc::vec![0u8; 4 * n];
    umem::read(Space::current(), addr, &mut b)?;
    Ok(b.chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// A NUL-terminated user string as event words.
fn user_str(addr: u64) -> Result<Vec<u32>, u32> {
    let mut s = Vec::new();
    let space = Space::current();
    while s.len() < 4 * SLOTS {
        let mut c = [0u8];
        umem::read(space, addr + s.len() as u64, &mut c)?;
        if c[0] == 0 {
            break;
        }
        s.push(c[0]);
    }
    let mut w = Vec::new();
    str_words(core::str::from_utf8(&s).unwrap_or(""), &mut w);
    Ok(w)
}

/// `TraceEvent(code, ...)`: `args` points to { code, arguments... }.
pub fn trace_event(args: u64) -> KerResult {
    let mut b = [0u8; 8 * 6];
    umem::read(Space::current(), args, &mut b[..8])?;
    let code = u64::from_le_bytes(b[..8].try_into().unwrap()) as u32;
    let n = ((code >> 28) & 7).min(6) as usize;
    if n > 1 {
        umem::read(Space::current(), args + 8, &mut b[8..8 * n])?;
    }
    let a = |i: usize| u64::from_le_bytes(b[8 * i..8 * i + 8].try_into().unwrap());
    let a32 = |i: usize| a(i) as u32;
    // Controlling tracing takes PROCMGR_AID_TRACE (as procnto); inserting
    // one's own user events doesn't.
    if !matches!(code & 0x0fff_ffff, 30..=32) {
        super::ability::check(super::ability::TRACE, None)?;
    }
    match code & 0x0fff_ffff {
        1 => {
            set_all(&ENABLED, true);
            Ok(0)
        }
        2 => {
            set_all(&ENABLED, false);
            Ok(0)
        }
        3 => set_filter(&ENABLED, a32(1), None, true),
        4 => set_filter(&ENABLED, a32(1), None, false),
        5 => set_filter(&ENABLED, a32(1), Some(a32(2)), true),
        6 => set_filter(&ENABLED, a32(1), Some(a32(2)), false),
        // Event handlers: not supported.
        7 | 8 | 36 | 37 => Err(ENOTSUP),
        // INSERTEVENT(head, d0, d1): a raw event.
        9 => {
            let h = a32(1);
            insert((h >> 10) & 0x1f, h & 0x3ff, &[a32(2), a32(3)])
        }
        10 => start(true),
        33 => start(false),
        11 => {
            stop();
            notify_logger(false);
            Ok(0)
        }
        14 => {
            flush();
            Ok(0)
        }
        15 => Ok(0),
        16 => {
            set_all(&WIDE, false);
            Ok(0)
        }
        17 => {
            set_all(&WIDE, true);
            Ok(0)
        }
        18 => set_filter(&WIDE, a32(1), None, false),
        19 => set_filter(&WIDE, a32(1), None, true),
        20 => set_filter(&WIDE, a32(1), Some(a32(2)), false),
        21 => set_filter(&WIDE, a32(1), Some(a32(2)), true),
        // Per-pid and per-tid filters: accepted.
        22..=29 => Ok(0),
        // INSERTSUSEREVENT(event, d0, d1), INSERTCUSEREVENT(event, buf,
        // words), INSERTUSRSTREVENT(event, string).
        30 => insert(C_USER, a32(1), &[a32(2), a32(3)]),
        31 => insert(C_USER, a32(1), &user_words(a(2), a(3))?),
        32 => insert(C_USER, a32(1), &user_str(a(2))?),
        34 => {
            RING.store(true, Ordering::Relaxed);
            Ok(0)
        }
        35 => {
            RING.store(false, Ordering::Relaxed);
            Ok(0)
        }
        38 | 39 => Ok(0),
        40 => Ok(0),
        // _NTO_TRACE_SUPPINSTR
        41 => Ok(1),
        // INSERTSCLASSEVENT(class, event, d0, d1), INSERTCCLASSEVENT(class,
        // event, buf, words): the class is an internal one.
        42 => insert(a32(1), a32(2), &[a32(3), a32(4)]),
        43 => insert(a32(1), a32(2), &user_words(a(3), a(4))?),
        45 => logger_attach(a(1), a(2)),
        46 => wait_buffer(a(1)),
        _ => Err(ENOTSUP),
    }
}

/// `_NTO_TRACE_LOGGER_ATTACH(start, stop)`: the caller is the logger.
fn logger_attach(start_ev: u64, stop_ev: u64) -> KerResult {
    let me = process::current().pid;
    let space = Space::current();
    let read = |at: u64| (at != 0).then(|| event::read(space, at)).transpose();
    let (start, stop) = (read(start_ev)?, read(stop_ev)?);
    let mut l = LOGGER.lock();
    if l.as_ref()
        .is_some_and(|l| l.pid != me && process::lookup(l.pid).is_some())
    {
        return Err(EBUSY);
    }
    *l = Some(Logger {
        pid: me,
        start,
        stop,
    });
    Ok(0)
}

// _SYS_TRACE subtypes
pub const SYS_TRACE: u16 = 6;
const SYS_TRACE_ALLOCBUF: u16 = 0;
const SYS_TRACE_DEALLOCBUF: u16 = 1;
const SYS_TRACE_DETACH: u16 = 2;

/// A `_SYS_TRACE` message from process `pid`; the reply's data. As
/// procnto's `procmgr_trace`: `PROCMGR_AID_TRACE` first, then the
/// subtype (ENOSYS). Only the logger (`_NTO_TRACE_LOGGER_ATTACH`) may
/// have buffers or free them (EPERM).
pub fn sys_trace(pid: i32, subtype: u16, msg: &[u8]) -> Result<Vec<u8>, u32> {
    let p = process::lookup(pid).ok_or(EINVAL)?;
    if !super::ability::allowed(&p, super::ability::TRACE, None) {
        return Err(EPERM);
    }
    match subtype {
        SYS_TRACE_ALLOCBUF if msg.len() >= 8 => {
            let per = u32::from_le_bytes(msg[4..8].try_into().unwrap());
            let base = alloc_buffers(&p, per)?;
            Ok(base.to_le_bytes().to_vec())
        }
        SYS_TRACE_DEALLOCBUF => {
            if !is_logger(pid) {
                return Err(EPERM);
            }
            if ACTIVE.load(Ordering::Relaxed) || CPUS.iter().any(|t| t.lock().state != IDLE) {
                return Err(EBUSY);
            }
            if let Some(b) = take_buffers() {
                free_buffers(b);
            }
            Ok(Vec::new())
        }
        SYS_TRACE_DETACH => {
            logger_gone(pid);
            Ok(Vec::new())
        }
        SYS_TRACE_ALLOCBUF => Err(EINVAL),
        _ => Err(super::errno::ENOSYS),
    }
}

fn is_logger(pid: i32) -> bool {
    LOGGER.lock().as_ref().is_some_and(|l| l.pid == pid)
}

/// The logger `pid` detaches (`_SYS_TRACE_DETACH`) or has ended (procnto's
/// `cleanup`): tracing stops and its buffers are freed, as procnto's
/// `nto_trace_logger_detach`. Anyone else's detach does nothing.
pub fn logger_gone(pid: i32) {
    {
        let mut l = LOGGER.lock();
        if l.as_ref().is_none_or(|l| l.pid != pid) {
            return;
        }
        *l = None;
    }
    stop_as(true);
    if let Some(b) = take_buffers() {
        free_buffers(b);
    }
}

/// `_SYS_TRACE_ALLOCBUF`: `per` buffers for each CPU, in contiguous memory
/// mapped into the kernel and, shared, into the logger `p`; returns their
/// address there. As procnto's `procmgr_trace`: the memory comes first
/// (EINVAL for none, EOVERFLOW past 32 bits of buffers, ENOMEM), then
/// whether `p` is the logger (EPERM) and has none yet (EEXIST); buffers
/// are never replaced, only freed.
fn alloc_buffers(p: &Process, per: u32) -> Result<u64, u32> {
    use crate::arch::mmu::Prot;
    use crate::vm::{kmem, map, pmem};
    if per == 0 {
        return Err(EINVAL);
    }
    let ncpu = crate::smp::online_count() as u64;
    let n = u32::try_from(per as u64 * ncpu).map_err(|_| EOVERFLOW)?;
    let len = n as u64 * BUF_SIZE;
    let pages = len / pmem::PAGE_SIZE;
    let pa = pmem::alloc_contig(pages, 0, u64::MAX).ok_or(ENOMEM)?;
    let free = |pa: u64| (0..pages).for_each(|i| pmem::free_page(pa + i * pmem::PAGE_SIZE));
    let Some(kva) = kmem::map_phys(pa, len, Prot::Data) else {
        free(pa);
        return Err(ENOMEM);
    };
    // SAFETY: the buffers are mapped at `kva` for `len` bytes, and no CPU
    // writes them yet.
    unsafe { core::ptr::write_bytes(kva as *mut u8, 0, len as usize) };
    // Not inherited by a fork, as procnto maps them.
    let rw = map::PROT_READ | map::PROT_WRITE;
    let user = map::map_phys(p, 0, len, rw, map::MAP_SHARED | map::MAP_NOINHERIT, pa);
    let b = Buffers {
        pa,
        kva,
        len,
        user: *user.as_ref().unwrap_or(&0),
    };
    let r = user.and_then(|_| {
        let mut bufs = BUFFERS.lock();
        if !is_logger(p.pid) {
            return Err(EPERM);
        }
        if bufs.is_some() {
            return Err(EEXIST);
        }
        *bufs = Some(b);
        BUF_KVA.store(kva, Ordering::Relaxed);
        PER_CPU.store(per as usize, Ordering::Release);
        Ok(b.user)
    });
    if r.is_err() {
        if b.user != 0 {
            let _ = map::unmap(p, b.user, len);
        }
        kmem::unmap_phys(kva, len);
        free(pa);
    }
    r
}

/// Take the buffers out of use: once this returns, no CPU writes into
/// them. A CPU writes under its own lock and starts no buffer while
/// `PER_CPU` is 0, so going through each lock waits out a write at work,
/// and dropping the current buffer stops a CPU that started one after
/// `stop()` went past it.
fn take_buffers() -> Option<Buffers> {
    let b = BUFFERS.lock().take()?;
    PER_CPU.store(0, Ordering::SeqCst);
    for t in CPUS.iter() {
        let mut t = t.lock();
        t.cur = None;
        t.full.clear();
    }
    Some(b)
}

/// Unmap buffers taken out of use and free their memory: the logger's
/// view (procnto's `unmap_trace_buffers`) and any copy of it a fork gave
/// a child, by the frames they map (review IPC-20: none keeps a window
/// onto freed frames), then the kernel's.
fn free_buffers(b: Buffers) {
    use crate::vm::{kmem, map, pmem};
    for p in process::all() {
        map::unmap_phys(&p, b.pa, b.len);
    }
    kmem::unmap_phys(b.kva, b.len);
    for off in (0..b.len).step_by(pmem::PAGE_SIZE as usize) {
        pmem::free_page(b.pa + off);
    }
}
