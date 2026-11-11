//! Shared memory handles (`shm_create_handle()`, `shm_open_handle()`,
//! `mmap_handle()`, `shm_delete_handle()`; `_MEM_SHMEM_HANDLE`), as
//! procnto's `memmgr_shmem_handle`.
//!
//! The message is `struct _mem_shmem_handle` { type, subtype, options,
//! handle, pid, perm, sub_offset, sub_length } (40 bytes); the reply is the
//! message back, with the handle for a new one.
//! - Subtype 0, on a descriptor of the object: a handle for process `pid`,
//!   with access `perm` (`O_RDONLY`/`O_WRONLY`/`O_RDWR`, which the
//!   descriptor must allow: EACCES). Options `SHM_CREATE_HANDLE_OPT_NOFD`
//!   (it can only be mapped) and `_SUBRANGE` (only `sub_length` bytes from
//!   `sub_offset` on, page-aligned and in the object; EINVAL otherwise).
//! - Subtype 1, on a new connection to procnto: open the handle there
//!   (`shm_open_handle()`); `pid`, if not 0, must be its creator.
//! - Subtype 2: delete it (its creator or the process it is for).
//! - `mmap()` with `MAP_HANDLE` maps one (see `memmgr`).
//! A handle is used once: opening or mapping it, even when that fails for
//! want of access, removes it; only the process it is for may (ESRCH
//! otherwise). The handles of a process that ends go.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::shmem::{Open, ShmFile};
use crate::ker::errno::{EACCES, EINVAL, EPERM, ESRCH};
use crate::sync::IrqSpinLock;
use crate::vm::pmem::PAGE_SIZE;

pub const MEM_SHMEM_HANDLE: u16 = 0x004b;
pub const MEM_SHMEM_REVOKE: u16 = 0x004c;
pub const MSG_SIZE: usize = 40;

const OPT_NOFD: u32 = 0x1;
const OPT_SUBRANGE: u32 = 0x2;

struct Handle {
    id: u64,
    file: Arc<ShmFile>,
    /// Its creator and the process it is for.
    src: i32,
    dst: i32,
    /// Bit 0 read, bit 1 write (`_IO_FLAG_RD`/`_WR`).
    perm: u32,
    options: u32,
    /// `_SUBRANGE`: the offset and length.
    sub: (u64, u64),
}

static HANDLES: IrqSpinLock<Vec<Handle>> = IrqSpinLock::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn u32_at(m: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(m[i..i + 4].try_into().unwrap())
}

fn u64_at(m: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(m[i..i + 8].try_into().unwrap())
}

/// The access an open flag (`O_ACCMODE`) asks for, as `_IO_FLAG_*` bits.
fn perm_of(flags: u32) -> u32 {
    (flags & 7) + 1
}

/// Subtype 0 on object descriptor `o` of process `caller`: the reply
/// (the message with the new handle).
pub fn create(caller: i32, o: &Open, msg: &[u8]) -> Result<Vec<u8>, u32> {
    let options = u32_at(msg, 4);
    let (dst, perm) = (u32_at(msg, 16) as i32, perm_of(u32_at(msg, 20)));
    let (sub_off, sub_len) = (u64_at(msg, 24), u64_at(msg, 32));
    if options > OPT_NOFD | OPT_SUBRANGE {
        return Err(EINVAL);
    }
    if options & OPT_SUBRANGE != 0
        && ((sub_off as i64) < 0
            || !sub_off.is_multiple_of(PAGE_SIZE)
            || !sub_len.is_multiple_of(PAGE_SIZE)
            || sub_off.checked_add(sub_len).is_none())
    {
        return Err(EINVAL);
    }
    crate::ker::process::lookup(dst)
        .filter(|p| !p.is_zombie())
        .ok_or(ESRCH)?;
    let Open::File { file, ioflag, .. } = o else {
        return Err(crate::ker::errno::EBADF);
    };
    if perm & !*ioflag & 3 != 0 {
        return Err(EACCES);
    }
    if options & OPT_SUBRANGE != 0 && sub_off + sub_len > file.obj.size() {
        return Err(EINVAL);
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    HANDLES.lock().push(Handle {
        id,
        file: file.clone(),
        src: caller,
        dst,
        perm,
        options,
        sub: if options & OPT_SUBRANGE != 0 { (sub_off, sub_len) } else { (0, 0) },
    });
    let mut r = msg[..MSG_SIZE].to_vec();
    r[8..16].copy_from_slice(&id.to_le_bytes());
    Ok(r)
}

/// Take handle `id` for its use by `caller`, if it is for `caller` and,
/// unless `src` is 0, made by `src`.
fn take(id: u64, caller: i32, src: i32) -> Option<Handle> {
    let mut handles = HANDLES.lock();
    let i = handles
        .iter()
        .position(|h| h.id == id && (src == 0 || h.src == src) && h.dst == caller)?;
    Some(handles.swap_remove(i))
}

/// Subtype 1 (`shm_open_handle()`) from `caller`: what its new descriptor
/// opens.
pub fn open(caller: i32, msg: &[u8]) -> Result<Open, u32> {
    let h = take(u64_at(msg, 8), caller, u32_at(msg, 16) as i32).ok_or(ESRCH)?;
    if h.options & OPT_NOFD != 0 {
        return Err(EPERM);
    }
    let perm = perm_of(u32_at(msg, 20));
    if perm & !h.perm != 0 {
        return Err(EACCES);
    }
    Ok(Open::File {
        name: None,
        file: h.file,
        pos: 0,
        ioflag: perm,
    })
}

/// Subtype 2 (`shm_delete_handle()`) from `caller`.
pub fn delete(caller: i32, msg: &[u8]) -> Result<(), u32> {
    let id = u64_at(msg, 8);
    let mut handles = HANDLES.lock();
    let i = handles
        .iter()
        .position(|h| h.id == id && (h.src == caller || h.dst == caller))
        .ok_or(ESRCH)?;
    handles.swap_remove(i);
    Ok(())
}

/// `mmap()` of handle `id` by `caller` (`MAP_HANDLE`; `src` its creator,
/// or 0): the object and the offset in it to map `len` bytes at `offset`
/// with `prot` (`PROT_READ` 0x100, `PROT_WRITE` 0x200, `PROT_EXEC` 0x400).
pub fn for_mmap(id: u64, caller: i32, src: i32, prot: u32, offset: u64, len: u64) -> Result<(Arc<ShmFile>, u64), u32> {
    let h = take(id, caller, src).ok_or(ESRCH)?;
    let (sub_off, sub_len) = h.sub;
    if h.options & OPT_SUBRANGE != 0 {
        let end = (offset as i64 >= 0)
            .then(|| offset.checked_add(sub_off))
            .flatten()
            .and_then(|start| start.checked_add(len))
            .and_then(|e| e.checked_next_multiple_of(PAGE_SIZE))
            .filter(|&e| len != 0 && e != 0);
        if end.is_none_or(|e| e > sub_off + sub_len) {
            return Err(EINVAL);
        }
    }
    // No handle allows execution: PROT_EXEC is EACCES whatever the
    // handle's access (procnto, measured with an O_RDWR handle; the docs
    // don't say).
    const PROT_EXEC: u32 = 0x400;
    if prot & PROT_EXEC != 0 || (prot >> 8) & 3 & !h.perm != 0 {
        return Err(EACCES);
    }
    Ok((h.file, offset + sub_off))
}

/// `shm_revoke()` (`_MEM_SHMEM_REVOKE` { type, subtype, pid }: subtype 0
/// for every process, 1 for `pid`) on object descriptor `o` from
/// `caller`: only the creator of a revocable object (EINVAL, EPERM). The
/// object's handles for those processes go, and so do their mappings of
/// it made while it was revocable (`vm::map::revoke`).
pub fn revoke(caller: i32, o: &Open, msg: &[u8]) -> Result<(), u32> {
    let Open::File { file, .. } = o else {
        return Err(crate::ker::errno::EBADF);
    };
    if !file.revocable() {
        return Err(EINVAL);
    }
    if caller != file.creator() {
        return Err(EPERM);
    }
    let pid = match u16::from_le_bytes([msg[2], msg[3]]) {
        0 => 0,
        _ => u32_at(msg, 4) as i32,
    };
    HANDLES
        .lock()
        .retain(|h| !(Arc::ptr_eq(&h.file, file) && (pid == 0 || h.dst == pid)));
    let targets = if pid == 0 {
        crate::ker::process::all()
    } else {
        alloc::vec![crate::ker::process::lookup(pid).ok_or(ESRCH)?]
    };
    for p in targets {
        crate::vm::map::revoke(&p, &file.obj);
    }
    Ok(())
}

/// Process `pid` has ended: the handles it made or was given go.
pub fn process_exited(pid: i32) {
    HANDLES.lock().retain(|h| h.src != pid && h.dst != pid);
}
