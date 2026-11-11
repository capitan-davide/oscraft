//! Interrupt handling, driven by the system page.
//!
//! The controllers, their logical vector ranges and their callouts come
//! from `intrinfo`; the kernel itself knows no interrupt controller. A
//! logical vector is `vector_base + id`, plus `cpu * local_stride` for
//! controllers of per-CPU interrupts.

use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::arch;
use crate::arch::intrgen::{CodeBuffer, FragmentState};
use crate::percpu;
use crate::sync::InitOnce;
use crate::syspage::{Syspage, SyspageEntry};

/// `intrinfo.cascade_vector` of a controller that is not cascaded.
const NO_CASCADE: u32 = 0x7fff_ffff;
const MAX_CONTROLLERS: usize = 8;
const MAX_HANDLERS: usize = 32;

type Callout = unsafe extern "C" fn(*mut SyspageEntry, i32) -> i32;

#[derive(Debug, Clone, Copy)]
struct Controller {
    vector_base: u32,
    num_vectors: u32,
    local_stride: u32,
    id: u64,
    eoi: u64,
    mask: Option<Callout>,
    unmask: Option<Callout>,
}

struct Controllers {
    syspage: Syspage,
    list: [Option<Controller>; MAX_CONTROLLERS],
}

static CONTROLLERS: InitOnce<Controllers> = InitOnce::new();

/// Handler slots: (vector + 1, handler); 0 = free.
static HANDLER_VECTOR: [AtomicU32; MAX_HANDLERS] = [const { AtomicU32::new(0) }; MAX_HANDLERS];
static HANDLER_FN: [AtomicUsize; MAX_HANDLERS] = [const { AtomicUsize::new(0) }; MAX_HANDLERS];
/// Per handler slot: interrupts taken, and the counter value at the last.
static HANDLER_COUNT: [AtomicU64; MAX_HANDLERS] = [const { AtomicU64::new(0) }; MAX_HANDLERS];
static HANDLER_TIME: [AtomicU64; MAX_HANDLERS] = [const { AtomicU64::new(0) }; MAX_HANDLERS];
static SPURIOUS: AtomicU64 = AtomicU64::new(0);
/// CPU 0's interprocessor-interrupt vector (`u32::MAX`: none): the per-CPU
/// interrupt whose `config` callout says `INTR_CONFIG_FLAG_IPI`.
static IPI_VECTOR: AtomicU32 = AtomicU32::new(u32::MAX);
/// `INTR_CONFIG_FLAG_IPI`.
const INTR_CONFIG_FLAG_IPI: u32 = 0x4;

type ConfigCallout = unsafe extern "C" fn(*mut SyspageEntry, *const crate::syspage::IntrinfoEntry, i32) -> u32;

#[derive(Debug)]
pub struct InitInfo {
    pub controllers: usize,
    pub cascaded_skipped: usize,
}

/// Generate the interrupt code for the controllers in the system page.
pub fn init(sp: &Syspage) -> Result<InitInfo, &'static str> {
    let mut code = CodeBuffer::new().ok_or("no memory for interrupt code")?;
    let mut list = [None; MAX_CONTROLLERS];
    let (mut n, mut skipped) = (0, 0);
    for i in sp.intrinfo() {
        if i.cascade_vector != NO_CASCADE {
            // Cascaded controllers (an MSI block behind the GIC) need dispatch from
            // their parent vector; not supported yet.
            skipped += 1;
            continue;
        }
        if n == MAX_CONTROLLERS {
            return Err("too many interrupt controllers");
        }
        let id = code
            .add_fragment(i.id.rtn as u64, i.id.size)
            .ok_or("interrupt code too large")?;
        let eoi = code
            .add_fragment(i.eoi.rtn as u64, i.eoi.size)
            .ok_or("interrupt code too large")?;
        if i.local_stride != 0 && i.config != 0 && IPI_VECTOR.load(Ordering::Relaxed) == u32::MAX {
            // SAFETY: a C-ABI callout of the system page taking it, this
            // intrinfo entry and a controller-relative interrupt number.
            let config: ConfigCallout = unsafe { core::mem::transmute(i.config) };
            if let Some(id) = (0..i.num_vectors)
                // SAFETY: as above.
                .find(|&id| unsafe { config(sp.as_ptr(), i, id as i32) } & INTR_CONFIG_FLAG_IPI != 0)
            {
                IPI_VECTOR.store(i.vector_base + id, Ordering::Relaxed);
            }
        }
        list[n] = Some(Controller {
            vector_base: i.vector_base,
            num_vectors: i.num_vectors,
            local_stride: i.local_stride,
            id,
            eoi,
            mask: i.mask,
            unmask: i.unmask,
        });
        n += 1;
    }
    code.finish()
        .ok_or("cannot make interrupt code executable")?;
    CONTROLLERS
        .set(Controllers { syspage: *sp, list })
        .map_err(|_| "interrupts already initialised")?;
    Ok(InitInfo {
        controllers: n,
        cascaded_skipped: skipped,
    })
}

/// The controller and controller-relative id of a logical vector on `cpu`.
fn locate(vector: u32, cpu: u32) -> Option<(Controller, u32)> {
    let c = CONTROLLERS.get()?;
    c.list.iter().flatten().find_map(|ctl| {
        let base = ctl.vector_base + cpu * ctl.local_stride;
        (vector >= base && vector < base + ctl.num_vectors).then(|| (*ctl, vector - base))
    })
}

/// The calling CPU's instance of a per-CPU interrupt given by its CPU 0
/// vector (e.g. `qtime.intr`); other vectors are returned unchanged.
pub fn local_vector(vector0: u32) -> Option<u32> {
    let (ctl, id) = locate(vector0, 0)?;
    Some(ctl.vector_base + percpu::this().cpu * ctl.local_stride + id)
}

/// A slot claimed but not yet published (`attach`).
const CLAIMED: u32 = u32::MAX;

/// Attach `handler` to a logical vector and unmask it on this CPU. One
/// handler per vector: a second attach is an error. Callers don't race
/// on one vector (the kernel's attaches are at boot and per CPU, the
/// user ones serialize in `ker::interrupt`): the duplicate check and
/// the claim are two steps.
pub fn attach(vector: u32, handler: fn(u32)) -> Result<(), &'static str> {
    let (ctl, id) = locate(vector, percpu::this().cpu).ok_or("no controller for this vector")?;
    if HANDLER_VECTOR.iter().any(|v| v.load(Ordering::Acquire) == vector + 1) {
        return Err("vector already has a handler");
    }
    // The handler is in place before the vector is published, so that an
    // interrupt that arrives in between (the vector may be unmasked
    // already, by another CPU or a user attachment) finds it rather than
    // a slot with no function, which would be taken as unwanted.
    let slot = HANDLER_VECTOR
        .iter()
        .position(|v| {
            v.compare_exchange(0, CLAIMED, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        })
        .ok_or("too many interrupt handlers")?;
    HANDLER_FN[slot].store(handler as usize, Ordering::Release);
    HANDLER_VECTOR[slot].store(vector + 1, Ordering::Release);
    set_masked(&ctl, id, false)
}

/// The logical vector of CPU `cpu`'s instance of `vector0`, a vector as
/// numbered on CPU 0 (itself for an interrupt that isn't per-CPU); `None`
/// if there is no such vector or CPU.
pub fn vector_on(vector0: u32, cpu: u32) -> Option<u32> {
    if cpu >= crate::smp::online_count() as u32 {
        return None;
    }
    let (ctl, id) = locate(vector0, 0)?;
    Some(ctl.vector_base + cpu * ctl.local_stride + id)
}

/// Whether a logical vector belongs to an interrupt controller.
pub fn exists(vector: u32) -> bool {
    // Any CPU's: a per-CPU interrupt's vectors are one per CPU, and a
    // caller may name another CPU's (the clock's `qtime.intr` is CPU 0's)
    // from wherever it runs (review SCHED-13: EINVAL from CPU 1).
    (0..crate::smp::online_count()).any(|cpu| locate(vector, cpu).is_some())
}

/// Whether `vector` has a handler installed already (the kernel's own, or
/// the user attachments' one).
pub fn handled(vector: u32) -> bool {
    HANDLER_VECTOR.iter().any(|v| v.load(Ordering::Acquire) == vector + 1)
}

/// Mask or unmask a logical vector (on this CPU, for per-CPU interrupts).
pub fn mask(vector: u32, masked: bool) -> Result<(), &'static str> {
    let (ctl, id) = locate(vector, percpu::this().cpu).ok_or("no controller for this vector")?;
    set_masked(&ctl, id, masked)
}

fn set_masked(ctl: &Controller, id: u32, masked: bool) -> Result<(), &'static str> {
    let sp = CONTROLLERS
        .get()
        .ok_or("interrupts not initialised")?
        .syspage;
    let callout =
        if masked { ctl.mask } else { ctl.unmask }.ok_or("controller has no mask callout")?;
    // SAFETY: mask/unmask are C-ABI callouts taking the system page and a
    // controller-relative interrupt number.
    unsafe { callout(sp.as_ptr(), id as i32) };
    Ok(())
}

/// The handler slot of a vector and its handler.
fn handler_for(vector: u32) -> Option<(usize, fn(u32))> {
    HANDLER_VECTOR
        .iter()
        .zip(&HANDLER_FN)
        .enumerate()
        .find_map(|(i, (v, f))| {
            let f = f.load(Ordering::Acquire);
            // SAFETY: only `fn(u32)` values are stored in HANDLER_FN.
            (v.load(Ordering::Acquire) == vector + 1 && f != 0)
                .then(|| (i, unsafe { core::mem::transmute::<usize, fn(u32)>(f) }))
        })
}

/// A vector's deliveries: (the counter value at the last, how many), and
/// whether the kernel handles it (`InterruptQuery()`).
pub fn stats(vector: u32) -> (u64, u64, bool) {
    match handler_for(vector) {
        Some((i, _)) => (
            HANDLER_TIME[i].load(Ordering::Relaxed),
            HANDLER_COUNT[i].load(Ordering::Relaxed),
            true,
        ),
        None => (0, 0, false),
    }
}

// ANCHOR: interrupt_handle
/// IRQ entry: identify the interrupt, run its handler, end it.
pub fn handle() {
    let Some(c) = CONTROLLERS.get() else { return };
    let cpu = percpu::this().cpu;
    let mut st = FragmentState::new(c.syspage.as_ptr() as u64);
    for ctl in c.list.iter().flatten() {
        let id = arch::intrgen::run(ctl.id, &mut st);
        if id as i64 == -1 {
            continue;
        }
        let id = id as u32;
        let vector = ctl.vector_base + cpu * ctl.local_stride + id;
        crate::ker::entropy::add_interrupt(vector);
        match handler_for(vector) {
            Some((slot, h)) => {
                HANDLER_COUNT[slot].fetch_add(1, Ordering::Relaxed);
                HANDLER_TIME[slot].store(arch::timer::counter(), Ordering::Relaxed);
                h(vector)
            }
            None => {
                // Nobody wants it: keep it from firing again.
                let _ = set_masked(ctl, id, true);
            }
        }
        st.set_id(id as u64);
        arch::intrgen::run(ctl.eoi, &mut st);
        return;
    }
    SPURIOUS.fetch_add(1, Ordering::Relaxed);
}
// ANCHOR_END: interrupt_handle

/// CPU 0's interprocessor-interrupt vector, if the system page has one.
pub fn ipi_vector() -> Option<u32> {
    Some(IPI_VECTOR.load(Ordering::Relaxed)).filter(|&v| v != u32::MAX)
}

pub fn spurious_count() -> u64 {
    SPURIOUS.load(Ordering::Relaxed)
}
