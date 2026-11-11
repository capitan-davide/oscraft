//! Interrupts for user-level drivers: `InterruptAttachEvent()`,
//! `InterruptAttachThread()`, `InterruptDetach()`, `InterruptMask()`,
//! `InterruptUnmask()`, `InterruptWait()`, `InterruptUnblock()` and
//! `InterruptQuery()`.
//!
//! Contract, from the QNX 8.0 library reference and procnto's
//! `ker_interrupt_attach`, `ker_interrupt_mask` and `ker_interrupt_unmask`:
//! - `InterruptAttachEvent(intr, event, flags)` reaches the kernel as
//!   `InterruptAttach(intr, NULL, event, sizeof(*event), flags)`. A
//!   `SIGEV_INTR` event (or none) attaches the calling thread, which then
//!   waits with `InterruptWait()`; other events are delivered to the
//!   process. `intr` is a logical vector from the system page's
//!   `intrinfo`; synthetic vectors (from 0x7fff0000) are not supported.
//! - When the interrupt fires, each unmasked attachment is masked and its
//!   event delivered; the vector stays masked at the controller until all
//!   of its attachments are unmasked again (`InterruptUnmask(intr, id)`,
//!   or `InterruptWait()` with `_NTO_INTR_WAIT_FLAGS_UNMASK`). Masking and
//!   unmasking take the attachment's id (`intr` is ignored): an
//!   attachment already in that state is EAGAIN, id -1 ENOTSUP and an id
//!   the process doesn't own ESRCH.
//! - An attachment starts unmasked unless `_NTO_INTR_FLAGS_NO_UNMASK`.
//! - `InterruptAttachThread(intr, flags)` is libc's
//!   `InterruptAttach(intr, NULL, NULL, 0, flags)`: the thread attaches
//!   itself, as with a `SIGEV_INTR` event.
//! - `InterruptQuery(type, id, extra, data)`: see `interrupt_query`.
//! - `_NTO_INTR_FLAGS_EXCLUSIVE`: the vector's only attachment (EBUSY
//!   otherwise, either way round).
//! - `InterruptUnblock(id)` posts the process's attachment `id` as if its
//!   interrupt had come (its thread's `InterruptWait()` returns, or its
//!   event is delivered), without masking anything: a way to get an
//!   interrupt thread out of its wait, e.g. to detach.
//! - A process's attachments go away when it ends.
//!
//! The kernel delivers the events from its interrupt handler; QNX runs an
//! interrupt service thread per event instead. QNX 8 has no handler
//! functions: `InterruptAttach()` with one is EINVAL, as on procnto.
//! `InterruptWait()` times out with `TimerTimeout()` (its own timeout
//! argument is unused, as documented). Not yet: CPU-local interrupts.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, Ordering};

use super::KerResult;
use super::errno::{EAGAIN, EBUSY, EINVAL, ENODEV, ENOSYS, ENOTSUP, EPERM, ESRCH};
use super::event::{self, SigEvent};
use super::process::{self, Process, ProcessRef};
use super::umem::{self, Space};
use crate::sched::{self, State, ThreadRef};
use crate::sync::IrqSpinLock;

const FLAGS_NO_UNMASK: u32 = 0x2;
/// `_NTO_INTR_FLAGS_TRK_MSK`: procnto sets it on every attachment.
const FLAGS_TRK_MSK: u32 = 0x8;
const FLAGS_EXCLUSIVE: u32 = 0x20;
pub const WAIT_FLAGS_UNMASK: u32 = 0x2;
const SYNTHETIC: u32 = 0x7fff_0000;
/// `si_code` of a plain `SIGEV_SIGNAL` sent for an interrupt.
const SI_USER: i32 = 0;

struct Attachment {
    id: i32,
    vector: u32,
    process: ProcessRef,
    thread: ThreadRef,
    /// `None`: wake `thread` (`SIGEV_INTR`).
    event: Option<SigEvent>,
    /// The `struct sigevent` given, as given (zeros for a thread).
    event_raw: [u8; event::SIGEV_SIZE],
    /// `_NTO_INTR_FLAGS_*` given, with `_NTO_INTR_FLAGS_TRK_MSK`.
    flags: u32,
    masked: bool,
}

static ATTACHED: IrqSpinLock<Vec<Attachment>> = IrqSpinLock::new(Vec::new());
/// Vectors whose kernel handler is installed.
static HOOKED: IrqSpinLock<Vec<u32>> = IrqSpinLock::new(Vec::new());
static NEXT_ID: AtomicI32 = AtomicI32::new(1);

/// `sizeof(debug_irq64_t)`.
pub const DEBUG_IRQ_SIZE: usize = 0x70;

/// The interrupts `p` has attached, as `debug_irq64_t`s
/// (`DCMD_PROC_IRQS`): pid, tid, flags, mask count, id, vector, event.
pub fn attached_of(p: &Process) -> alloc::vec::Vec<[u8; DEBUG_IRQ_SIZE]> {
    ATTACHED
        .lock()
        .iter()
        .filter(|a| core::ptr::eq(&*a.process, p))
        .map(|a| {
            let mut d = [0u8; DEBUG_IRQ_SIZE];
            d[0..4].copy_from_slice(&a.process.pid.to_le_bytes());
            d[4..8].copy_from_slice(&a.thread.ptid.to_le_bytes());
            d[0x10..0x14].copy_from_slice(&a.flags.to_le_bytes());
            d[0x18..0x1c].copy_from_slice(&(a.masked as u32).to_le_bytes());
            d[0x1c..0x20].copy_from_slice(&a.id.to_le_bytes());
            d[0x20..0x24].copy_from_slice(&a.vector.to_le_bytes());
            d[0x50..0x70].copy_from_slice(&a.event_raw);
            d
        })
        .collect()
}

/// Is `t` an interrupt thread (attached itself, `InterruptAttachThread()`
/// or a `SIGEV_INTR` event)?
pub fn is_ist(t: &crate::sched::Thread) -> bool {
    ATTACHED
        .lock()
        .iter()
        .any(|a| a.event.is_none() && core::ptr::eq(&*a.thread, t))
}

/// `InterruptAttach(intr, handler, area, size, flags)`.
pub fn interrupt_attach(intr: u32, handler: u64, area: u64, size: u64, flags: u32) -> KerResult {
    super::ability::check(super::ability::INTERRUPT, Some(intr as u64))?;
    // As procnto: the synthetic vectors (_NTO_INTR_CLASS_SYNTHETIC: the
    // clock and spare ones) are not supported, others beyond them invalid;
    // QNX 8 has no interrupt handler functions (EINVAL).
    if intr >= SYNTHETIC {
        return Err(if intr & !0x8000 <= SYNTHETIC + 2 { ENOSYS } else { EINVAL });
    }
    if handler != 0 {
        return Err(EINVAL);
    }
    if !crate::interrupt::exists(intr) {
        return Err(EINVAL);
    }
    let mut event_raw = [0u8; event::SIGEV_SIZE];
    let event = if area == 0 {
        None
    } else {
        if size < event::SIGEV_SIZE as u64 {
            return Err(EINVAL);
        }
        umem::read(Space::current(), area, &mut event_raw)?;
        let ev = SigEvent::from_bytes(&event_raw);
        ev.validate()?;
        match ev.kind() {
            event::SIGEV_INTR => None,
            event::SIGEV_NONE | event::SIGEV_UNBLOCK | event::SIGEV_SIGNAL_THREAD => {
                return Err(EINVAL);
            }
            _ => Some(ev),
        }
    };
    let me = sched::current();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let masked = flags & FLAGS_NO_UNMASK != 0;
    {
        let mut attached = ATTACHED.lock();
        if event.is_none()
            && attached
                .iter()
                .any(|a| a.event.is_none() && core::ptr::eq(&*a.thread, me))
        {
            return Err(EBUSY);
        }
        // An exclusive attachment is the vector's only one.
        if attached
            .iter()
            .any(|a| a.vector == intr && (a.flags | flags) & FLAGS_EXCLUSIVE != 0)
        {
            return Err(EBUSY);
        }
        attached.push(Attachment {
            id,
            vector: intr,
            process: me.process.clone(),
            thread: me.arc(),
            event,
            event_raw,
            flags: flags | FLAGS_TRK_MSK,
            masked,
        });
    }
    let hook = {
        let mut hooked = HOOKED.lock();
        let new = !hooked.contains(&intr);
        if new {
            hooked.push(intr);
        }
        new
    };
    if hook {
        // Installing the handler unmasks the vector. A vector the kernel
        // handles itself (the clock's) is EBUSY, as on procnto (measured,
        // review SCHED-13); the attachment and the hook go again, so that
        // nothing is left half made.
        let busy = crate::interrupt::handled(intr);
        if busy || crate::interrupt::attach(intr, irq).is_err() {
            HOOKED.lock().retain(|&v| v != intr);
            ATTACHED.lock().retain(|a| a.id != id);
            return Err(if busy { EBUSY } else { EAGAIN });
        }
    }
    update_vector(intr);
    Ok(id as u64)
}

// ANCHOR: irq
/// The kernel's handler for vectors with user attachments.
fn irq(vector: u32) {
    let mut fired: Vec<(ProcessRef, ThreadRef, Option<SigEvent>)> = Vec::new();
    {
        let mut attached = ATTACHED.lock();
        for a in attached
            .iter_mut()
            .filter(|a| a.vector == vector && !a.masked)
        {
            a.masked = true;
            fired.push((a.process.clone(), a.thread.clone(), a.event));
        }
    }
    // Keep it from firing again until the drivers have dealt with it.
    let _ = crate::interrupt::mask(vector, true);
    if !fired.is_empty() {
        crate::ker::trace::emit(
            crate::ker::trace::C_INT,
            crate::ker::trace::INT_DELIVER,
            &[vector, 0],
        );
    }
    for (p, t, ev) in fired {
        let ev = ev.unwrap_or(SigEvent {
            notify: event::SIGEV_INTR,
            ..Default::default()
        });
        let _ = event::deliver(&p, Some(t), &ev, SI_USER);
    }
}
// ANCHOR_END: irq

/// Mask the vector at its controller while any attachment is masked (or
/// none is left).
fn update_vector(vector: u32) {
    let attached = ATTACHED.lock();
    let mut on_vector = attached.iter().filter(|a| a.vector == vector).peekable();
    let masked = on_vector.peek().is_none() || on_vector.any(|a| a.masked);
    let _ = crate::interrupt::mask(vector, masked);
}

/// Set the mask of the calling process's attachment `id`.
fn set_masked(id: i32, masked: bool) -> KerResult {
    if id == -1 {
        return Err(ENOTSUP);
    }
    let me = process::current();
    let vector = {
        let mut attached = ATTACHED.lock();
        let a = attached
            .iter_mut()
            .find(|a| a.id == id && core::ptr::eq(&*a.process, me))
            .ok_or(ESRCH)?;
        if a.masked == masked {
            return Err(EAGAIN);
        }
        a.masked = masked;
        a.vector
    };
    update_vector(vector);
    Ok(0)
}

/// `InterruptMask(intr, id)`.
pub fn interrupt_mask(_intr: u32, id: i32) -> KerResult {
    set_masked(id, true)
}

/// `InterruptUnmask(intr, id)`.
pub fn interrupt_unmask(_intr: u32, id: i32) -> KerResult {
    set_masked(id, false)
}

/// `InterruptDetach(id)`.
pub fn interrupt_detach(id: i32) -> KerResult {
    let me = process::current();
    let vector = {
        let mut attached = ATTACHED.lock();
        let i = attached
            .iter()
            .position(|a| a.id == id && core::ptr::eq(&*a.process, me))
            .ok_or(EINVAL)?;
        attached.remove(i).vector
    };
    update_vector(vector);
    Ok(0)
}

/// `InterruptUnblock(id)`.
pub fn interrupt_unblock(id: i32) -> KerResult {
    let me = process::current();
    let (p, t, ev) = ATTACHED
        .lock()
        .iter()
        .find(|a| a.id == id && core::ptr::eq(&*a.process, me))
        .map(|a| (a.process.clone(), a.thread.clone(), a.event))
        .ok_or(ESRCH)?;
    let ev = ev.unwrap_or(SigEvent {
        notify: event::SIGEV_INTR,
        ..Default::default()
    });
    event::deliver(&p, Some(t), &ev, SI_USER)?;
    Ok(0)
}

const QUERY_HANDLER: u32 = 0;
const QUERY_SOURCE: u32 = 1;
/// `sizeof(struct interrupt_query_data)` (`<sys/interrupts.h>`).
const QUERY_SIZE: usize = 88;

/// `InterruptQuery(type, id, extra, data)`: a `struct interrupt_query_data`
/// about the caller's attachment `id` (`_NTO_INTR_QUERY_HANDLER`) or about
/// vector `id` (`_NTO_INTR_QUERY_SOURCE`, root only). As procnto: the
/// vector's deliveries (the kernel's own vectors, such as the clock's,
/// included), the handler's process, thread, runmask, flags (with
/// `_NTO_INTR_FLAGS_TRK_MSK`), mask count (1 masked, 0 not) and event;
/// the source's number of handlers (1 for a kernel vector) and of masked
/// ones. ESRCH for an attachment that isn't the caller's, EINVAL for a
/// vector that doesn't exist or another type. A source is numbered as on
/// CPU 0; `extra` is the CPU of a per-CPU source's instance.
pub fn interrupt_query(ty: u32, id: i32, extra: u64, data: u64) -> KerResult {
    let me = process::current();
    let mut d = [0u8; QUERY_SIZE];
    let put32 = |d: &mut [u8; QUERY_SIZE], at: usize, v: u32| {
        d[at..at + 4].copy_from_slice(&v.to_le_bytes());
    };
    let put64 = |d: &mut [u8; QUERY_SIZE], at: usize, v: u64| {
        d[at..at + 8].copy_from_slice(&v.to_le_bytes());
    };
    let vector = match ty {
        QUERY_HANDLER => {
            let attached = ATTACHED.lock();
            let a = attached
                .iter()
                .find(|a| a.id == id && core::ptr::eq(&*a.process, me))
                .ok_or(ESRCH)?;
            put32(&mut d, 24, a.process.pid as u32);
            put32(&mut d, 28, a.thread.ptid);
            put64(&mut d, 32, a.thread.runmask.load(Ordering::Relaxed));
            put32(&mut d, 40, a.id as u32);
            put32(&mut d, 44, a.flags);
            put32(&mut d, 48, a.masked as u32);
            d[56..].copy_from_slice(&a.event_raw);
            a.vector
        }
        QUERY_SOURCE => {
            if !super::ability::allowed(me, super::ability::INTERRUPT, None) {
                return Err(EPERM);
            }
            // Numbered as on CPU 0; a per-CPU source's instance on CPU
            // `extra`.
            let vector0 = u32::try_from(id).map_err(|_| EINVAL)?;
            let cpu = u32::try_from(extra).map_err(|_| EINVAL)?;
            let vector = crate::interrupt::vector_on(vector0, cpu).ok_or(EINVAL)?;
            let attached = ATTACHED.lock();
            let on_vector = attached.iter().filter(|a| a.vector == vector);
            let handlers = on_vector.clone().count() as u32;
            let masked = on_vector.filter(|a| a.masked).count() as u32;
            let kernel = crate::interrupt::stats(vector).2;
            put32(&mut d, 28, if handlers == 0 && kernel { 1 } else { handlers });
            put32(&mut d, 32, masked);
            vector
        }
        _ => return Err(EINVAL),
    };
    let (time, count, _) = crate::interrupt::stats(vector);
    put32(&mut d, 0, ty);
    put32(&mut d, 4, vector);
    put64(&mut d, 8, time);
    put64(&mut d, 16, count);
    umem::write(Space::current(), data, &d)?;
    Ok(0)
}

/// `InterruptWait(flags, timeout)`: wait for a `SIGEV_INTR` event.
pub fn interrupt_wait(flags: u32, _timeout: u64) -> KerResult {
    let me = sched::current();
    // A thread with no interrupt attached has none to wait for (procnto:
    // ENODEV).
    if !ATTACHED
        .lock()
        .iter()
        .any(|a| a.event.is_none() && core::ptr::eq(&*a.thread, me))
    {
        return Err(ENODEV);
    }
    if flags & WAIT_FLAGS_UNMASK != 0 {
        let ids: Vec<i32> = ATTACHED
            .lock()
            .iter()
            .filter(|a| a.event.is_none() && core::ptr::eq(&*a.thread, me) && a.masked)
            .map(|a| a.id)
            .collect();
        for id in ids {
            let _ = set_masked(id, false);
        }
    }
    let timeout = {
        let _g = event::INTR_LOCK.lock();
        if me.intr_pending.swap(false, Ordering::AcqRel) {
            return Ok(0);
        }
        me.wake_error.store(0, Ordering::Relaxed);
        sched::prepare_block(State::Intr);
        crate::timeout::arm(State::Intr)
    };
    sched::block();
    crate::timeout::disarm(timeout);
    match me.wake_error.swap(0, Ordering::Relaxed) {
        0 => {
            me.intr_pending.store(false, Ordering::Release);
            Ok(0)
        }
        e => Err(e),
    }
}

/// Detach the interrupts of a process that has ended.
pub fn process_exited(p: &Process) {
    let vectors: Vec<u32> = {
        let mut attached = ATTACHED.lock();
        let gone = attached
            .iter()
            .filter(|a| core::ptr::eq(&*a.process, p))
            .map(|a| a.vector)
            .collect();
        attached.retain(|a| !core::ptr::eq(&*a.process, p));
        gone
    };
    for v in vectors {
        update_vector(v);
    }
}
