//! Boot-time self-tests of kernel subsystems, with results on the console.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

use crate::ker::{self, errno, number};
use crate::sched::{self, Policy, ThreadRef};
use crate::sync::InitOnce;
use crate::{arch, kprintln, percpu, smp};

/// A user page in a fresh address space: mapped, seen by the MMU as EL0
/// readable once the space is active, and reachable from the kernel.
pub fn uspace() {
    use crate::arch::uspace::{self, AddressSpace, UserProt};
    use crate::vm::{kmem, pmem};
    const VA: u64 = 0x10_0000;
    let aspace = AddressSpace::new().expect("address space");
    let pa = pmem::alloc_page().expect("frame");
    aspace
        .map(
            VA,
            pa,
            UserProt {
                read: true,
                write: true,
                exec: false,
            },
        )
        .expect("map");
    let translated = aspace.translate(VA + 8, true) == Some(pa + 8);
    let kva = kmem::map_frame(pa).expect("kernel mapping");
    let daif = arch::irq_save();
    uspace::activate(aspace.ttbr0());
    let el0 = uspace::el0_readable(VA);
    // SAFETY: VA is mapped read/write in the active user space (no PAN).
    unsafe { (VA as *mut u64).write_volatile(0x5e1f_0000_cafe) };
    uspace::activate(crate::ker::process::KERNEL.aspace().ttbr0());
    arch::irq_restore(daif);
    // SAFETY: kva maps the same frame.
    let seen = unsafe { (kva as *const u64).read_volatile() } == 0x5e1f_0000_cafe;
    let unmapped = aspace.unmap(VA) == Some(pa) && aspace.translate(VA, false).is_none();
    kprintln!(
        "uspace: ASID {} user page {VA:#x} -> {pa:#x}: translate {translated}, EL0-readable {el0}, written via TTBR0 and seen via kernel map {seen}, unmapped {unmapped}",
        aspace.asid()
    );
}

/// Two user processes running hand-assembled code at EL0: one yields and
/// ends itself with ThreadDestroy, the other dereferences a null pointer
/// and must be terminated without disturbing the kernel.
pub fn user_smoke() {
    use crate::arch::uspace::UserProt;
    use crate::ker::process;
    use crate::vm::{kmem, pmem};
    const CODE: u64 = 0x40_0000;
    const STACK: u64 = 0x7f_ffff_0000;
    const YIELD_AND_EXIT: [u32; 6] = [
        0x5280_0b48, // mov w8, #90 (__KER_SCHED_YIELD)
        0xd400_0a21, // svc #0x51
        0xd280_0000, // mov x0, #0
        0x5280_05e8, // mov w8, #47 (__KER_THREAD_DESTROY)
        0xd400_0a21, // svc #0x51
        0x1400_0000, // b .
    ];
    const NULL_LOAD: [u32; 2] = [
        0xf940_0020, // ldr x0, [x1]   (x1 = 0)
        0x1400_0000, // b .
    ];
    kprintln!(
        "user: the second test process loads from address 0 on purpose: the fault dump below is expected"
    );
    let mut procs = Vec::new();
    for code in [&YIELD_AND_EXIT[..], &NULL_LOAD[..]] {
        let p = process::create().expect("process");
        let (cpa, spa) = (pmem::alloc_page().unwrap(), pmem::alloc_page().unwrap());
        let ckva = kmem::map_frame(cpa).unwrap();
        // SAFETY: freshly mapped frame of ours.
        unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), ckva as *mut u32, code.len()) };
        crate::arch::mmu::sync_icache(ckva, (code.len() * 4) as u64);
        p.aspace()
            .map(
                CODE,
                cpa,
                UserProt {
                    read: true,
                    write: false,
                    exec: true,
                },
            )
            .unwrap();
        p.aspace()
            .map(
                STACK,
                spa,
                UserProt {
                    read: true,
                    write: true,
                    exec: false,
                },
            )
            .unwrap();
        sched::spawn_user(&p, "user", 10, CODE, STACK + 4096, 0, 0).expect("user thread");
        procs.push(p);
    }
    while procs.iter().any(|p| p.thread_count() != 0) {
        sched::yield_now();
    }
    kprintln!(
        "user: processes {} and {} ran at EL0 and ended",
        procs[0].pid,
        procs[1].pid
    );
    crate::proc::bootscript::START.store(true, Ordering::Release);
}

/// Small blocks (size classes, reused after free) and a page-sized block.
pub fn heap() {
    let small: Vec<Box<u64>> = (0..1000).map(Box::new).collect();
    // Freed in order, so the free list (LIFO) hands out the last one first.
    let last = &*small[999] as *const u64;
    let sum: u64 = small.iter().map(|b| **b).sum();
    drop(small);
    let again = Box::new(7u64);
    let big = Box::new([1u8; 10000]);
    kprintln!(
        "heap: 1000 small boxes (sum {sum}), block reused after free: {}, {} bytes at {:p}",
        core::ptr::eq(&*again, last),
        big.len(),
        &*big
    );
}

const WORKERS: usize = 8;
const RUN_MS: u64 = 300;

/// One cache line per counter, so that counting does not contend.
#[repr(align(64))]
struct Counter(AtomicU64);

static WORK: [Counter; WORKERS] = [const { Counter(AtomicU64::new(0)) }; WORKERS];
static WORKER_THREADS: InitOnce<Vec<ThreadRef>> = InitOnce::new();

/// Round-robin workers at one priority, more than CPUs: each must get a
/// fair share and they move between CPUs. A lower-priority reporter only
/// runs once CPUs become free, and prints the result.
pub fn sched() {
    let threads = (0..WORKERS)
        .map(|i| sched::spawn("worker", 10, Policy::RoundRobin, worker, i).expect("spawn"))
        .collect();
    let _ = WORKER_THREADS.set(threads);
    sched::spawn("reporter", 5, Policy::RoundRobin, reporter, 0).expect("spawn");
}

fn worker(i: usize) {
    let end = arch::timer::counter() + arch::timer::frequency() * RUN_MS / 1000;
    while arch::timer::counter() < end {
        WORK[i].0.fetch_add(1, Ordering::Relaxed);
    }
}

fn reporter(_: usize) {
    let threads = WORKER_THREADS.get().unwrap();
    while threads.iter().any(|t| t.state() != sched::State::Dead) {
        sched::yield_now();
    }
    let counts: Vec<u64> = WORK.iter().map(|w| w.0.load(Ordering::Relaxed)).collect();
    let (min, max) = (counts.iter().min().unwrap(), counts.iter().max().unwrap());
    kprintln!(
        "sched: {WORKERS} round-robin workers on {} CPUs for {RUN_MS} ms:",
        smp::online_count()
    );
    for (t, c) in threads.iter().zip(&counts) {
        kprintln!(
            "  tid {:>2} {} {:>8} iterations, ran on CPUs {:#06b}",
            t.tid,
            t.name,
            c,
            t.cpus_seen.load(Ordering::Relaxed)
        );
    }
    kprintln!(
        "sched: smallest share {}% of the largest; reporter (priority 5) ran on CPU {}",
        min * 100 / max,
        percpu::this().cpu
    );
    kercalls();
}

/// Kernel calls from a kernel thread through the libc-style stub: a valid
/// call, an unknown one (must fail with ENOSYS through the error path) and
/// the round-trip cost of SchedYield with nothing else to run.
fn kercalls() {
    use arch::kercall::call;
    let nop = call(number::__KER_NOP, [0; 6]);
    let unknown = call(200, [0; 6]);
    const N: u64 = 100_000;
    let t0 = arch::timer::counter();
    for _ in 0..N {
        call(number::__KER_SCHED_YIELD, [0; 6]).expect("SchedYield");
    }
    let ns = (arch::timer::counter() - t0) * 1_000_000_000 / arch::timer::frequency() / N;
    kprintln!(
        "ker: NOP -> {nop:?}, call 200 -> {unknown:?} (ENOSYS = {}), SchedYield {ns} ns per call (QEMU), {} calls",
        errno::ENOSYS,
        ker::calls()
    );
    ipc();
}

// ---- message passing ----

const MSGS: u64 = 100;
const BENCH: u64 = 10_000;
static CHID: AtomicI32 = AtomicI32::new(-1);
/// Server's effective priority when it received each client's last message.
static SEEN_PRIO: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
static ERRORS: AtomicU64 = AtomicU64::new(0);
static PULSE: AtomicU64 = AtomicU64::new(0);
static CLIENTS_DONE: AtomicU32 = AtomicU32::new(0);
static SERVER_DONE: AtomicBool = AtomicBool::new(false);
static SERVER: InitOnce<ThreadRef> = InitOnce::new();
const CLIENT_PRIO: [u8; 2] = [30, 8];
const SERVER_PRIO: u8 = 20;
const PULSE_QUIT: i32 = 1;
const PULSE_TEST: i32 = 5;

fn k(n: u32, args: [u64; 6]) -> Result<u64, u32> {
    arch::kercall::call(n, args)
}

/// A server and two clients at priorities above and below it, talking only
/// through kernel calls with libc's calling sequence.
fn ipc() {
    let server = sched::spawn("server", SERVER_PRIO, Policy::Fifo, server, 0).expect("spawn");
    let _ = SERVER.set(server);
    for (i, &p) in CLIENT_PRIO.iter().enumerate() {
        sched::spawn("client", p, Policy::Fifo, client, i).expect("spawn");
    }
    sched::spawn("ipc-report", 5, Policy::RoundRobin, ipc_report, 0).expect("spawn");
}

fn server(_: usize) {
    let chid = k(number::__KER_CHANNEL_CREATE, [0; 6]).expect("ChannelCreate");
    CHID.store(chid as i32, Ordering::Release);
    // Room for a whole `struct _pulse` (24 bytes): a shorter buffer can't
    // take a pulse at all (EFAULT, as procnto).
    let mut msg = [0u64; 3];
    let mut info = [0u8; 64];
    loop {
        let buf = msg.as_mut_ptr() as u64;
        let rcvid = k(
            number::__KER_MSG_RECEIVEV,
            [chid, buf, (-24i64) as u64, info.as_mut_ptr() as u64, 0, 0],
        )
        .expect("MsgReceive");
        if rcvid == 0 {
            // struct _pulse: code at 4, value at 8.
            let code = (msg[0] >> 32) as i8 as i32;
            if code == PULSE_QUIT {
                break;
            }
            PULSE.store(((code as u64) << 32) | msg[1], Ordering::Relaxed);
            continue;
        }
        let (n, client) = (msg[0], msg[1] as usize);
        SEEN_PRIO[client].store(sched::current().priority() as u32, Ordering::Relaxed);
        if n == 0 {
            k(
                number::__KER_MSG_ERROR,
                [rcvid, errno::EINVAL as u64, 0, 0, 0, 0],
            )
            .expect("MsgError");
            continue;
        }
        let reply = n + 1;
        k(
            number::__KER_MSG_REPLYV,
            [
                rcvid,
                2 * n,
                &reply as *const u64 as u64,
                (-8i64) as u64,
                0,
                0,
            ],
        )
        .expect("MsgReply");
    }
    SERVER_DONE.store(true, Ordering::Release);
}

fn send(coid: u64, n: u64, client: usize) -> (Result<u64, u32>, u64) {
    let msg = [n, client as u64];
    let mut reply = 0u64;
    let r = k(
        number::__KER_MSG_SENDV,
        [
            coid,
            msg.as_ptr() as u64,
            (-16i64) as u64,
            &mut reply as *mut u64 as u64,
            (-8i64) as u64,
            0,
        ],
    );
    (r, reply)
}

fn client(i: usize) {
    while CHID.load(Ordering::Acquire) < 0 {
        sched::yield_now();
    }
    let chid = CHID.load(Ordering::Acquire) as u64;
    let coid = k(number::__KER_CONNECT_ATTACH, [0, 0, chid, 0, 0, 0]).expect("ConnectAttach");
    for n in 1..=MSGS {
        let (r, reply) = send(coid, n, i);
        if r != Ok(2 * n) || reply != n + 1 {
            ERRORS.fetch_add(1, Ordering::Relaxed);
        }
    }
    if send(coid, 0, i).0 != Err(errno::EINVAL) {
        ERRORS.fetch_add(1, Ordering::Relaxed);
    }
    if i == 0 {
        k(
            number::__KER_MSG_SEND_PULSE,
            [coid, u64::MAX, PULSE_TEST as u64, 42, 0, 0],
        )
        .expect("MsgSendPulse");
    }
    CLIENTS_DONE.fetch_add(1, Ordering::Release);
}

fn ipc_report(_: usize) {
    while CLIENTS_DONE.load(Ordering::Acquire) < CLIENT_PRIO.len() as u32 {
        sched::yield_now();
    }
    let chid = CHID.load(Ordering::Acquire) as u64;
    let coid = k(number::__KER_CONNECT_ATTACH, [0, 0, chid, 0, 0, 0]).expect("ConnectAttach");
    // Round trips from this low-priority thread (the server follows our priority).
    let t0 = arch::timer::counter();
    for n in 1..=BENCH {
        if send(coid, n, 2).0 != Ok(2 * n) {
            ERRORS.fetch_add(1, Ordering::Relaxed);
        }
    }
    let ns = (arch::timer::counter() - t0) * 1_000_000_000 / arch::timer::frequency() / BENCH;
    let pulse = PULSE.load(Ordering::Relaxed);
    k(
        number::__KER_MSG_SEND_PULSE,
        [coid, u64::MAX, PULSE_QUIT as u64, 0, 0, 0],
    )
    .expect("MsgSendPulse");
    while !SERVER_DONE.load(Ordering::Acquire) {
        sched::yield_now();
    }
    let server = SERVER.get().unwrap();
    kprintln!(
        "ipc: {} messages from clients at priorities {:?} and {BENCH} timed ones, {} wrong replies or errors",
        2 * (MSGS + 1),
        CLIENT_PRIO,
        ERRORS.load(Ordering::Relaxed)
    );
    kprintln!(
        "ipc: server (base priority {}) ran at {} / {} / {} for clients at {} / {} / 5; pulse code {} value {}",
        server.base_priority(),
        SEEN_PRIO[0].load(Ordering::Relaxed),
        SEEN_PRIO[1].load(Ordering::Relaxed),
        SEEN_PRIO[2].load(Ordering::Relaxed),
        CLIENT_PRIO[0],
        CLIENT_PRIO[1],
        pulse >> 32,
        pulse & 0xffff_ffff
    );
    kprintln!(
        "ipc: send/receive/reply round trip {ns} ns (QEMU), {} kernel calls",
        ker::calls()
    );
    user_smoke();
}

/// A thread sleeping for 10 ms, three times: it must wake after its
/// deadline, within a tick or two.
pub fn sleep() {
    sched::spawn("sleeper", 15, Policy::Fifo, sleeper, 0).expect("spawn");
}

fn sleeper(_: usize) {
    const NS: u64 = 10_000_000;
    let mut late = [0u64; 3];
    for l in &mut late {
        let deadline = crate::clock::now_ns() + NS;
        crate::timeout::sleep_until(deadline);
        *l = crate::clock::now_ns().saturating_sub(deadline) / 1000;
    }
    kprintln!("timeout: slept 3 x 10 ms, woken {late:?} us after the deadlines");
}
