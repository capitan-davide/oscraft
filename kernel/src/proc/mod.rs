//! procnto's process-manager side (QNX `proc/`, `procmgr/`, `pathmgr/`),
//! as far as it exists: the image filesystem, the ELF loader, the boot
//! script, the system-manager channel, the memory manager, the path
//! manager and procnto's own resource managers.

pub mod bootscript;
pub mod client;
pub mod debug;
pub mod devtext;
pub mod kinfo;
pub mod sysctl;
pub mod imagefs;
pub mod loader;
pub mod locks;
pub mod memmgr;
pub mod namedsem;
pub mod pathmgr;
pub mod procevent;
pub mod procfs;
pub mod procmgr;
pub mod resmgr;
pub mod rsrcdb;
pub mod session;
pub mod shmem;
pub mod shmhandle;
pub mod spawn;
pub mod sysmgr;
pub mod tty;
pub mod tymem;
pub mod worker;

use crate::sched::{self, Policy};
use crate::syspage::Syspage;

/// Priority of the thread that runs the boot script (QNX 8.0 `mkifs`:
/// "The script is run by a priority 10 thread").
const SCRIPT_PRIO: u8 = 10;

/// Start procnto's services and the boot script.
pub fn start(sp: &Syspage) {
    pathmgr::init();
    rsrcdb::init(sp);
    match imagefs::init(sp) {
        Some(fs) => crate::kprintln!("imagefs: {} entries, {} bytes", fs.entries(), fs.size()),
        None => crate::kprintln!("imagefs: not found"),
    }
    sched::spawn("sysmgr", SCRIPT_PRIO, Policy::RoundRobin, sysmgr::server, 0)
        .expect("sysmgr thread");
    sched::spawn(
        "devtext",
        SCRIPT_PRIO,
        Policy::RoundRobin,
        devtext::server,
        0,
    )
    .expect("devtext thread");
    sched::spawn(
        "devtext-poll",
        SCRIPT_PRIO,
        Policy::RoundRobin,
        devtext::poller,
        0,
    )
    .expect("devtext poller thread");
    sched::spawn("kernel-worker", SCRIPT_PRIO, Policy::RoundRobin, worker::run, 0)
        .expect("kernel worker thread");
    sched::spawn(
        "sigev-thread",
        SCRIPT_PRIO,
        Policy::RoundRobin,
        crate::ker::thread::sigev_worker,
        0,
    )
    .expect("SIGEV_THREAD worker");
    sched::spawn(
        "bootscript",
        SCRIPT_PRIO,
        Policy::RoundRobin,
        bootscript::run,
        0,
    )
    .expect("boot script thread");
}
