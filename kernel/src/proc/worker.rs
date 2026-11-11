//! A procnto thread for work that sends messages to other servers on a
//! client's behalf. The system manager's thread must not do that itself:
//! a server that, to answer, sends to procnto in turn (a terminal's server
//! reporting its foreground process group, for one) would wait for a
//! thread that waits for it. The work answers its client itself.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::sched;
use crate::sync::IrqSpinLock;

type Job = Box<dyn FnOnce() + Send>;

static JOBS: IrqSpinLock<VecDeque<Job>> = IrqSpinLock::new(VecDeque::new());
static WORKER: AtomicU32 = AtomicU32::new(0);

// ANCHOR: worker
/// Run `job` on the worker thread, after the jobs queued before it.
pub fn defer(job: impl FnOnce() + Send + 'static) {
    JOBS.lock().push_back(Box::new(job));
    if let Some(t) = sched::thread(WORKER.load(Ordering::Acquire)) {
        sched::wake(&t);
    }
}

/// The worker thread.
pub fn run(_: usize) {
    let me = sched::current();
    WORKER.store(me.tid, Ordering::Release);
    loop {
        let job = JOBS.lock().pop_front();
        match job {
            Some(job) => job(),
            None => {
                sched::prepare_block(sched::State::Receive);
                if !JOBS.lock().is_empty() {
                    sched::wake(me);
                }
                sched::block();
            }
        }
    }
}
// ANCHOR_END: worker
