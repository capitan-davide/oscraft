//! process 1's system-manager channel (chid 1). Every process is connected to
//! it as `SYSMGR_COID`; QNX's libc sends it `_SYS_*`, `_PROC_*` and `_MEM_*`
//! messages (`<sys/sysmsg.h>`, `<sys/procmsg.h>`, `<sys/memmsg.h>`).
//!
//! Implemented: `_SYS_CONF` gets (`confstr()`, `sysconf()` via procnto)
//! and sets (privileged: e.g. QNX's mqueue publishes `_SC_MQ_OPEN_MAX`,
//! `hostname` sets `_CS_HOSTNAME`; a set value overrides the default),
//! `_SYS_CMD` `_SYS_CMD_REBOOT` (`sysmgr_reboot()`, QNX's `shutdown`) and
//! `_SYS_CMD_CPUMODE` (`sysmgr_cpumode()`), `_SYS_GETPAGESIZES`,
//! `_PROC_GETSETID` (see `procmgr`), `_MEM_MAP` and `_MEM_CTRL` (see
//! `memmgr`), and, as process 1's channel is
//! also the path manager's and its resource managers', `_IO_CONNECT` (see
//! `pathmgr`) and I/O messages (see `resmgr`).
//! For bring-up there is also one OsCraft-private message,
//! `_OSCRAFT_DEBUG_PRINT` (type 0xf000, then text), which prints the text,
//! and `_OSCRAFT_DEBUG_CRASH` (type 0xf001, privileged), which crashes the
//! kernel on purpose (a data abort in procnto), to test kernel dumps.

use alloc::string::String;

use crate::arch::kercall::call;
use crate::ker::errno::{EBADMSG, ENOSYS};
use crate::ker::number::*;
use crate::ker::process::SYSMGR_CHID;
use crate::kprintln;

use super::{memmgr, pathmgr, procmgr, resmgr, rsrcdb, shmhandle, spawn};

/// The shortest message of type `ty` (and `subtype`) procnto takes: a
/// shorter one is EBADMSG, before anything else. Measured on procnto for
/// each type and subtypes 0 to 7 (each type's fixed part, `struct _sys_*`,
/// `_proc_*`, `_mem_*`); types it doesn't handle have none (ENOSYS).
fn min_len(ty: u16, subtype: u16) -> Option<usize> {
    Some(match ty {
        0x00 => 24,                      // _SYS_CONF
        0x01 | 0x06 => 8,                // _SYS_CMD, _SYS_TRACE
        0x11 | 0x14 | 0x15 => 12,        // _PROC_WAIT, _SETPGID, _UMASK
        0x13 => 20,                      // _PROC_GETSETID
        0x17 | 0x1e | 0x23 | 0x25 | 0x27 => 16, // _SESSION, _TIMER_TOLERANCE, _GETNAME, _DEBUG, _GET_ALL_CHILD_PID
        0x1a if subtype == 3 => 16,      // _PROC_RESOURCE_SETABILITIES
        0x19 | 0x1a | 0x1d | 0x24 => 4,  // _PROC_EVENT, _RESOURCE, _VALUE, _GET_ALL_PID
        0x21 => 48,                      // _PROC_SERVMON
        0x22 | 0x4d => 8,                // _PROC_GETPID64, _MEM_AS_INFO
        0x40 => 72,                      // _MEM_MAP
        0x41 | 0x43 | 0x46 | 0x4a => 24, // _MEM_CTRL, _OFFSET, _PMEM_ADD, _OBJ_INFO
        0x42 | 0x47 => 16,               // _MEM_INFO, _PEER
        0x44 => 48,                      // _MEM_DEBUG_INFO
        0x49 => 64,                      // _MEM_DEBUG_INFO64
        0x4b | 0x4c => 40,               // _MEM_SHMEM_HANDLE, _REVOKE
        0x4e | 0x4f => 32,               // _MEM_MAP_INFO, _READ_MEM
        _ => return None,
    })
}

/// `_SYS_CONF` (`<sys/sysmsg.h>`).
const SYS_CONF: u16 = 0x0000;
/// `_SYS_CMD`: `struct _sys_cmd` { type, cmd, mode }.
const SYS_CMD: u16 = 0x0001;
const SYS_CMD_REBOOT: u16 = 0;
const SYS_CMD_CPUMODE: u16 = 1;
/// `_SYS_GETPAGESIZES` (`getpagesizes()`).
const SYS_GETPAGESIZES: u16 = 0x0005;
/// `struct _sys_conf.cmd` bits (`<sys/conf.h>`).
const CONF_STR: u32 = 0x10_0000;
const CONF_NUM: u32 = 0x20_0000;
/// Message type outside QNX's `_SYS_`/`_PROC_`/`_MEM_` ranges.
pub const OSCRAFT_DEBUG_PRINT: u16 = 0xf000;
pub const OSCRAFT_DEBUG_CRASH: u16 = 0xf001;

// confstr names (<confname.h>)
const CS_PATH: u32 = 1;
const CS_HOSTNAME: u32 = 2;
const CS_RELEASE: u32 = 3;
const CS_VERSION: u32 = 4;
const CS_MACHINE: u32 = 5;
const CS_ARCHITECTURE: u32 = 6;
const CS_SYSNAME: u32 = 11;
const CS_LIBPATH: u32 = 200;

// sysconf names (<confname.h>)
const SC_ARG_MAX: u32 = 1;
const SC_CHILD_MAX: u32 = 2;
const SC_CLK_TCK: u32 = 3;
const SC_NGROUPS_MAX: u32 = 4;
const SC_OPEN_MAX: u32 = 5;
const SC_SAVED_IDS: u32 = 7;
const SC_VERSION: u32 = 8;
const SC_PAGESIZE: u32 = 11;
const SC_DELAYTIMER_MAX: u32 = 18;
const SC_RTSIG_MAX: u32 = 21;
const SC_THREAD_STACK_MIN: u32 = 45;
const SC_THREAD_THREADS_MAX: u32 = 46;
const SC_NZERO: u32 = 92;
const SC_CONTAINER_MAX_ID: u32 = 182;
const SC_NPROCESSORS_CONF: u32 = 90;
const SC_NPROCESSORS_ONLN: u32 = 91;
const SC_AVPHYS_PAGES: u32 = 67;

/// Value of a numeric configuration name.
/// Configuration values set with `_SYS_SUB_SET`, by name.
static SET_NUMBERS: crate::sync::IrqSpinLock<alloc::collections::BTreeMap<u32, i64>> =
    crate::sync::IrqSpinLock::new(alloc::collections::BTreeMap::new());
static SET_STRINGS: crate::sync::IrqSpinLock<alloc::collections::BTreeMap<u32, String>> =
    crate::sync::IrqSpinLock::new(alloc::collections::BTreeMap::new());

fn conf_number(name: u32) -> Option<i64> {
    if let Some(&v) = SET_NUMBERS.lock().get(&name) {
        return Some(v);
    }
    // procnto's values (its `sysmgr_conf_table` and `kernel_conf_table`).
    Some(match name {
        SC_ARG_MAX => 61440,
        SC_CHILD_MAX | SC_OPEN_MAX => 1000,
        SC_CLK_TCK => 100,
        SC_NGROUPS_MAX => 384,
        SC_SAVED_IDS => 1,
        SC_VERSION => 200809,
        SC_DELAYTIMER_MAX => 0x10_0000,
        SC_RTSIG_MAX => 16,
        SC_THREAD_STACK_MIN => 4096,
        SC_THREAD_THREADS_MAX => 0x7fff,
        SC_NZERO => 10,
        SC_CONTAINER_MAX_ID => 0xfff,
        SC_PAGESIZE => crate::vm::pmem::PAGE_SIZE as i64,
        SC_AVPHYS_PAGES => crate::vm::pmem::free_pages() as i64,
        SC_NPROCESSORS_CONF | SC_NPROCESSORS_ONLN => crate::syspage::get().header().num_cpu as i64,
        _ => return None,
    })
}

/// Value of a configuration string: the system page's typed strings first
/// (startup sets e.g. `_CS_MACHINE`), then OsCraft's defaults.
fn conf_string(name: u32) -> Option<String> {
    if let Some(s) = SET_STRINGS.lock().get(&name) {
        return Some(s.clone());
    }
    if let Some(s) = crate::syspage::get().typed_string(name) {
        return Some(s.into());
    }
    let s = match name {
        // OsCraft's layout: QNX's tools and libraries under /compat/qnx, FreeBSD's
        // at the usual places (procnto's own strings have no /compat/qnx).
        CS_PATH => "/proc/boot:/compat/qnx/bin:/compat/qnx/usr/bin:/compat/qnx/sbin:/compat/qnx/usr/sbin:/bin:/usr/bin:/sbin:/usr/sbin",
        CS_HOSTNAME => "localhost",
        CS_RELEASE => "8.0.0",
        CS_VERSION => concat!("OsCraft ", env!("CARGO_PKG_VERSION")),
        CS_MACHINE | CS_ARCHITECTURE => "aarch64le",
        CS_SYSNAME => "OsCraft",
        CS_LIBPATH => "/proc/boot:/compat/qnx/lib:/compat/qnx/usr/lib:/compat/qnx/lib/dll:/lib:/usr/lib",
        _ => return None,
    };
    Some(s.into())
}

/// Reset the machine through the system page's reboot callout (startup's
/// board code); returns only if there is none.
fn reboot() {
    let sp = crate::syspage::get();
    let Some(callout) = sp.callout().reboot else {
        kprintln!("sysmgr: no reboot callout");
        return;
    };
    // Let the console drain (the reply's output, our message).
    for _ in 0..1000 {
        core::hint::spin_loop();
    }
    crate::arch::irq_save();
    // SAFETY: startup's callouts follow the C calling convention and only
    // use the system page and their own patched-in device addresses; 0:
    // a normal (not abnormal) reboot.
    unsafe { callout(sp.as_ptr(), 0) };
    kprintln!("sysmgr: the reboot callout returned");
}

/// A `_SYS_CONF` set (`struct _sys_conf` { type, subtype, cmd, name,
/// spare, value }): a number is `value`; a string follows the header
/// (`set_info`). Privileged (EPERM).
fn sys_conf_set(rcvid: u64, pid: i32, msg: &[u8]) {
    let cmd = u32::from_le_bytes(msg[4..8].try_into().unwrap());
    let name = u32::from_le_bytes(msg[8..12].try_into().unwrap());
    let value = i64::from_le_bytes(msg[16..24].try_into().unwrap());
    let privileged = crate::ker::process::lookup(pid)
        .is_some_and(|p| crate::ker::ability::allowed(&p, crate::ker::ability::CONFSET, Some(name as u64)));
    let r = if !privileged {
        Err(crate::ker::errno::EPERM)
    } else if cmd & CONF_STR != 0 {
        let s = msg[24..].split(|&b| b == 0).next().unwrap_or(&[]);
        SET_STRINGS
            .lock()
            .insert(name, String::from_utf8_lossy(s).into_owned());
        Ok(())
    } else if cmd & CONF_NUM != 0 {
        SET_NUMBERS.lock().insert(name, value);
        Ok(())
    } else {
        Err(crate::ker::errno::EINVAL)
    };
    if r.is_ok() {
        let which = if cmd & CONF_STR != 0 { super::procevent::CONFSTR } else { super::procevent::SYSCONF };
        super::procevent::trigger(which, name);
    }
    let _ = match r {
        Ok(()) => call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]),
        Err(e) => call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]),
    };
}

/// Reply to a `_SYS_CONF` get: `struct _sys_conf_reply` { zero[3], match,
/// value } (match -1 = unknown name), followed by the string for
/// `_CONF_STR` queries, as much of it as the client asked for.
fn sys_conf(rcvid: u64, msg: &[u8]) {
    let cmd = u32::from_le_bytes(msg[4..8].try_into().unwrap());
    let name = u32::from_le_bytes(msg[8..12].try_into().unwrap());
    let room = i64::from_le_bytes(msg[16..24].try_into().unwrap()).max(0) as usize;
    // The header, then as much of the string as the client has room for,
    // however long it is (review PROCMGR-20: 256 bytes at most before).
    let mut reply = alloc::vec![0u8; 24];
    let mut len = 24;
    let (matched, value): (i32, i64) = if cmd & CONF_STR != 0 {
        match conf_string(name) {
            Some(s) => {
                let bytes = s.as_bytes();
                // Length including the NUL, as confstr() reports it.
                let total = bytes.len() + 1;
                let n = total.min(room);
                reply.resize(24 + n, 0);
                reply[24..24 + n.min(bytes.len())].copy_from_slice(&bytes[..n.min(bytes.len())]);
                len += n;
                (0, total as i64)
            }
            None => (-1, 0),
        }
    } else if cmd & CONF_NUM != 0 {
        match conf_number(name) {
            Some(v) => (0, v),
            // Unknown: the value asked with goes back (procnto), so
            // sysconf() gives its -1.
            None => (-1, i64::from_le_bytes(msg[16..24].try_into().unwrap())),
        }
    } else {
        let _ = call(
            __KER_MSG_ERROR,
            [rcvid, crate::ker::errno::EINVAL as u64, 0, 0, 0, 0],
        );
        return;
    };
    reply[12..16].copy_from_slice(&matched.to_le_bytes());
    reply[16..24].copy_from_slice(&value.to_le_bytes());
    let _ = call(
        __KER_MSG_REPLYV,
        [
            rcvid,
            0,
            reply.as_ptr() as u64,
            (-(len as i64)) as u64,
            0,
            0,
        ],
    );
}

/// The longest I/O message other than a write that process 1 takes whole
/// (a write is read as it is written, `resmgr::write_in`).
const IO_MSG_MAX: usize = 4 * 1024 * 1024;

/// How much of a message procnto's process manager receives: a client's
/// buffer that faults within it fails its send (EFAULT, the message never
/// delivered), past it the message arrives and the fault comes when the
/// manager reads that far (measured, issue #288: a `write()` to
/// `/dev/shmem` whose buffer faults 2071 bytes into the data fails the
/// send, 2072 bytes in sizes the file, then EFAULT).
const RECEIVE_SIZE: usize = 2088;

/// `_IO_MSG`, and whether it is `sysctl()`'s: its `mgrid`, then subtype.
const IO_MSG: u16 = 0x113;
const IOMGR_NETWORK: u16 = 6;

fn subtype_sysctl(msg: &[u8]) -> bool {
    u16::from_le_bytes([msg[4], msg[5]]) == IOMGR_NETWORK
        && u16::from_le_bytes([msg[6], msg[7]]) == super::sysctl::MSG_SYSCTL
}

pub fn server(_: usize) {
    // Room for a spawn request with ARG_MAX bytes of arguments (and a
    // connect message with a PATH_MAX path). Only `RECEIVE_SIZE` of it is
    // received; the rest is read (`MsgRead()`).
    let mut msg = alloc::vec![0u8; 64 * 1024];
    let mut info = [0u8; 64];
    loop {
        let Ok(rcvid) = call(
            __KER_MSG_RECEIVEV,
            [
                SYSMGR_CHID as u64,
                msg.as_mut_ptr() as u64,
                (-(RECEIVE_SIZE as i64)) as u64,
                info.as_mut_ptr() as u64,
                0,
                0,
            ],
        ) else {
            continue;
        };
        if rcvid == 0 {
            // struct _pulse: code at 4, value at 8.
            let pid = i32::from_le_bytes(msg[8..12].try_into().unwrap());
            const PULSE_CODE_UNBLOCK: i32 = -32;
            match msg[4] as i8 as i32 {
                // A client a signal or timeout wants out: the value its
                // rcvid.
                PULSE_CODE_UNBLOCK => procmgr::unblocked(pid as u32 as u64),
                crate::ker::process::PULSE_PROCESS_EXIT => procmgr::process_exited(pid),
                crate::ker::process::PULSE_JOB_CHANGED => procmgr::job_changed(),
                crate::ker::process::PULSE_DETACHED => {
                    procmgr::detached(pid, i32::from_le_bytes(msg[12..16].try_into().unwrap()))
                }
                crate::ker::process::PULSE_FORK_DONE => {
                    procmgr::fork_done(pid, u32::from_le_bytes(msg[12..16].try_into().unwrap()))
                }
                _ => {}
            }
            continue;
        }
        let pid = i32::from_le_bytes(info[8..12].try_into().unwrap());
        let tid = u32::from_le_bytes(info[12..16].try_into().unwrap());
        let coid = i32::from_le_bytes(info[24..28].try_into().unwrap());
        let client = resmgr::Client { pid, coid };
        let mut len = (i64::from_le_bytes(info[32..40].try_into().unwrap()) as usize).min(RECEIVE_SIZE);
        // The whole message the client sent (`srcmsglen`).
        let srclen = i64::from_le_bytes(info[40..48].try_into().unwrap()).max(0) as usize;
        // The rest of a longer message, up to the buffer's size, but for
        // an I/O message's (a write's data is read as it is written, the
        // others whole, below). A fault in it fails this message, not the
        // client's send (`RECEIVE_SIZE`).
        if srclen > len && len >= 2 && !(0x0101..=0x01ff).contains(&u16::from_le_bytes([msg[0], msg[1]])) {
            let rest = &mut msg[len..srclen.min(64 * 1024)];
            match call(
                __KER_MSG_READV,
                [rcvid, rest.as_mut_ptr() as u64, (-(rest.len() as i64)) as u64, len as u64, 0, 0],
            ) {
                Ok(got) => len += got as usize,
                Err(e) => {
                    let _ = call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]);
                    continue;
                }
            }
        }
        // The header as the client sent it: the buffer still holds the
        // previous message's bytes past `len` (review PROCMGR-11). Under 2
        // bytes there is no type: EINVAL; 2 or 3 bytes are dispatched on
        // the type, the subtype 0, and are too short for any (EBADMSG),
        // as procnto answers them (measured).
        if len < 2 {
            let _ = call(__KER_MSG_ERROR, [rcvid, crate::ker::errno::EINVAL as u64, 0, 0, 0, 0]);
            continue;
        }
        msg[len.min(4)..4].fill(0);
        let (ty, subtype) = (
            u16::from_le_bytes([msg[0], msg[1]]),
            u16::from_le_bytes([msg[2], msg[3]]),
        );
        if min_len(ty, subtype).is_some_and(|min| len < min) {
            let _ = call(__KER_MSG_ERROR, [rcvid, EBADMSG as u64, 0, 0, 0, 0]);
            continue;
        }
        match ty {
            IO_MSG if len >= super::sysctl::HEADER && subtype_sysctl(&msg[..len]) => {
                // The client's reply buffer size (`dstmsglen`).
                let room = i64::from_le_bytes(info[48..56].try_into().unwrap()).max(0) as usize;
                super::sysctl::handle(rcvid, &msg[..len], room);
            }
            SYS_CONF if subtype == 0 && len >= 24 => sys_conf(rcvid, &msg[..len]),
            SYS_CONF if subtype == 1 && len >= 24 => sys_conf_set(rcvid, pid, &msg[..len]),
            // Rebooting takes PROCMGR_AID_REBOOT (as procnto).
            SYS_CMD
                if subtype == SYS_CMD_REBOOT
                    && !crate::ker::process::lookup(pid).is_some_and(|p| crate::ker::ability::allowed(&p, crate::ker::ability::REBOOT, None)) =>
            {
                let _ = call(__KER_MSG_ERROR, [rcvid, crate::ker::errno::EPERM as u64, 0, 0, 0, 0]);
            }
            SYS_CMD if subtype == SYS_CMD_REBOOT => {
                kprintln!("[pid {pid}] sysmgr: reboot");
                let _ = call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]);
                reboot();
            }
            // As procnto's `sysmgr_cmd`: a power mode takes
            // PROCMGR_AID_CPUMODE (the mode its subrange value), and then
            // there is nothing to do (`SysCpumode()` does nothing either).
            SYS_CMD if subtype == SYS_CMD_CPUMODE => {
                let mode = msg.get(4..8).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
                let ok = crate::ker::process::lookup(pid)
                    .is_some_and(|p| crate::ker::ability::allowed(&p, crate::ker::ability::CPUMODE, Some(mode as u64)));
                if ok {
                    let _ = call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]);
                } else {
                    let _ = call(__KER_MSG_ERROR, [rcvid, crate::ker::errno::EPERM as u64, 0, 0, 0, 0]);
                }
            }
            // As procnto's `sysmgr_getpagesizes`: the page sizes (one, 4 KB);
            // the status is how many, whatever the reply buffer holds (libc
            // asks with none first).
            SYS_GETPAGESIZES => {
                let sizes: &[u64] = &[crate::vm::pmem::PAGE_SIZE];
                let _ = call(
                    __KER_MSG_REPLYV,
                    [rcvid, sizes.len() as u64, sizes.as_ptr() as u64, (-(8 * sizes.len() as i64)) as u64, 0, 0],
                );
            }
            // Other commands: ENOSYS, quietly (as procnto).
            SYS_CMD => {
                let _ = call(__KER_MSG_ERROR, [rcvid, ENOSYS as u64, 0, 0, 0, 0]);
            }
            crate::ker::trace::SYS_TRACE => {
                match crate::ker::trace::sys_trace(pid, subtype, &msg[..len]) {
                    Ok(r) => {
                        let _ = call(
                            __KER_MSG_REPLYV,
                            [
                                rcvid,
                                0,
                                r.as_ptr() as u64,
                                (-(r.len() as i64)) as u64,
                                0,
                                0,
                            ],
                        );
                    }
                    Err(e) => {
                        let _ = call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]);
                    }
                }
            }
            spawn::PROC_SPAWN | spawn::PROC_POSIX_SPAWNP => {
                spawn::handle(rcvid, pid, tid, subtype, &msg[..len])
            }
            // The other _PROC_* messages (_PROCMGR_BASE 0x10 to _MEMMGR_BASE).
            0x0011..=0x003f => {
                let room = i64::from_le_bytes(info[48..56].try_into().unwrap()).max(0) as usize;
                procmgr::handle(rcvid, pid, tid, ty, subtype, &msg[..len], room)
            }
            pathmgr::PATH_OPENDIR => pathmgr::opendir(rcvid, client, &msg[..len]),
            pathmgr::PATH_CHDIR => pathmgr::chdir(rcvid, client, &msg[..len]),
            pathmgr::PATH_CHROOT => pathmgr::chroot(rcvid, client, &msg[..len]),
            pathmgr::IO_CONNECT => pathmgr::connect(rcvid, client, &msg[..len]),
            // A write longer than the receive buffer (sftp-server writes up
            // to 255 KB at once): its manager reads the rest from the
            // client as it writes it, at any length (`resmgr::write_in`).
            resmgr::IO_WRITE | resmgr::IO_WRITE64 if srclen > len => {
                resmgr::io_from(rcvid, client, ty, &msg[..len], srclen)
            }
            // Another I/O message longer than what was received: read
            // whole, up to `IO_MSG_MAX`; a fault in the client's buffer
            // fails it (`RECEIVE_SIZE`).
            0x0101..=0x01ff if srclen > len => {
                let mut whole = alloc::vec![0u8; srclen.min(IO_MSG_MAX)];
                whole[..len].copy_from_slice(&msg[..len]);
                let rest = &mut whole[len..];
                match call(
                    __KER_MSG_READV,
                    [
                        rcvid,
                        rest.as_mut_ptr() as u64,
                        (-(rest.len() as i64)) as u64,
                        len as u64,
                        0,
                        0,
                    ],
                ) {
                    Ok(got) => resmgr::io(rcvid, client, ty, &whole[..len + got as usize]),
                    Err(e) => {
                        let _ = call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]);
                    }
                }
            }
            0x0101..=0x01ff => resmgr::io(rcvid, client, ty, &msg[..len]),
            // Shared memory handles: made and revoked on a descriptor of
            // the object, opened on a new connection, deleted on ours.
            shmhandle::MEM_SHMEM_HANDLE | shmhandle::MEM_SHMEM_REVOKE if len >= shmhandle::MSG_SIZE => {
                match (ty, subtype) {
                    // As procnto's `memmgr_shmem_handle`: subtypes above 2
                    // are EBADMSG; a handle for no such process ESRCH,
                    // before the object is looked at.
                    (shmhandle::MEM_SHMEM_HANDLE, 3..) => {
                        let _ = call(__KER_MSG_ERROR, [rcvid, EBADMSG as u64, 0, 0, 0, 0]);
                    }
                    (shmhandle::MEM_SHMEM_HANDLE, 0)
                        if !Some(i32::from_le_bytes(msg[16..20].try_into().unwrap()))
                            .filter(|&pid| pid != 0)
                            .and_then(crate::ker::process::lookup)
                            .is_some_and(|p| !p.is_zombie()) =>
                    {
                        let _ = call(__KER_MSG_ERROR, [rcvid, crate::ker::errno::ESRCH as u64, 0, 0, 0, 0]);
                    }
                    (shmhandle::MEM_SHMEM_HANDLE, 1) => resmgr::shm_handle_open(rcvid, client, &msg[..len]),
                    (shmhandle::MEM_SHMEM_HANDLE, 2) => match shmhandle::delete(pid, &msg[..len]) {
                        Ok(()) => {
                            let _ = call(
                                __KER_MSG_REPLYV,
                                [rcvid, 0, msg.as_ptr() as u64, (-(shmhandle::MSG_SIZE as i64)) as u64, 0, 0],
                            );
                        }
                        Err(e) => {
                            let _ = call(__KER_MSG_ERROR, [rcvid, e as u64, 0, 0, 0, 0]);
                        }
                    },
                    _ => resmgr::io(rcvid, client, ty, &msg[..len]),
                }
            }
            rsrcdb::RSRCDBMGR_RSRC_CMD if len >= 4 => {
                rsrcdb::handle(rcvid, pid, subtype, &msg[..len])
            }
            memmgr::MEM_MAP
            | memmgr::MEM_CTRL
            | memmgr::MEM_INFO
            | memmgr::MEM_OFFSET
            | memmgr::MEM_PEER
            | memmgr::MEM_MAP_INFO
            | memmgr::MEM_READ_MEM
            | memmgr::MEM_AS_INFO
            | memmgr::MEM_DEBUG_INFO
            | memmgr::MEM_DEBUG_INFO64
            | memmgr::MEM_PMEM_ADD
            | memmgr::MEM_OBJ_INFO => {
                // The client's reply buffer size (`dstmsglen`).
                let room = i64::from_le_bytes(info[48..56].try_into().unwrap()).max(0) as usize;
                memmgr::handle(rcvid, pid, ty, subtype, &msg[..len], room)
            }
            OSCRAFT_DEBUG_CRASH
                if crate::ker::process::lookup(pid).is_some_and(|p| p.cred.lock().euid == 0) =>
            {
                kprintln!("[pid {pid}] sysmgr: crashing the kernel on request");
                // SAFETY: deliberately not: a read of an unmapped address.
                let v = unsafe { core::ptr::read_volatile(core::ptr::dangling::<u64>()) };
                kprintln!("sysmgr: the crash read {v}");
            }
            OSCRAFT_DEBUG_PRINT if len >= 2 => {
                kprintln!(
                    "[pid {pid}] {}",
                    String::from_utf8_lossy(&msg[2..len]).trim_end()
                );
                let _ = call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]);
            }
            _ => {
                // A message shorter than a header (type and subtype) is no
                // request at all: answered ENOSYS silently, as procnto does.
                // Longer unknown ones are worth a line: something OsCraft
                // may lack (review TOOLS-24).
                if len >= 4 {
                    kprintln!(
                        "[pid {pid}] sysmgr: message type {ty:#06x} subtype {subtype:#06x} ({len} bytes) not implemented"
                    );
                }
                let _ = call(__KER_MSG_ERROR, [rcvid, ENOSYS as u64, 0, 0, 0, 0]);
            }
        }
    }
}
