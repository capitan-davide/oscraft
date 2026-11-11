//! FreeBSD's `priv(9)` answered by QNX's abilities, for any FreeBSD kernel
//! code in a server (`kern/priv.c`). FreeBSD's checks stay: each names what it
//! is about to do, and here that name maps to an ability of the client on
//! whose behalf a thread is working. So the default is FreeBSD's, root may
//! and others may not, and a single ability can be given to a program without
//! making it root. A server gives its table to [`register()`]; `netd`'s has
//! the names of `io-sock`'s (`qnx_priv_init`, `qnx_priv_grant`).

use core::cell::Cell;
use core::ffi::{CStr, c_int, c_void};
use core::ptr::null_mut;
use std::sync::OnceLock;

use kercall::neutrino::msg::MsgSend;
use resmgr::compat::ConnectClientInfoAble;

/// `SYSMGR_COID`, `_PROC_RESOURCE`, `_PROC_RESOURCE_CREATEABILITY`.
const SYSMGR_COID: c_int = 0x4000_0000;
const PROC_RESOURCE: u16 = 0x1a;
const LOOKUP_ABILITY: u16 = 4;
const CREATE_ABILITY: u16 = 5;
/// `PROCMGR_ADN_ROOT`: root has it, others don't, by default.
const ADN_ROOT: u32 = 0x1000_0000;
const NAME_MAX: usize = 128;
const GETGROUPS: c_int = 1;
const EPERM: c_int = 1;
/// `struct _client_info`: words of the effective user and group IDs.
const INFO_EUID: usize = 5;
const INFO_EGID: usize = 8;

/// What a server's privileges map to.
pub struct Table {
    /// Every ability it uses.
    pub abilities: &'static [&'static CStr],
    /// A FreeBSD privilege, and the index of its ability.
    pub rules: &'static [(c_int, usize)],
    /// Privileges granted to all (`PRIV_*` a client needn't hold).
    pub free: &'static [c_int],
    /// The index of the ability for any privilege not listed.
    pub other: usize,
}

struct Registered {
    table: &'static Table,
    ids: Vec<u32>,
}

static REGISTERED: OnceLock<Registered> = OnceLock::new();

thread_local! {
    /// The client this thread works for; -1: none, FreeBSD's own threads.
    static CLIENT: Cell<c_int> = const { Cell::new(-1) };
}

/// `name`'s ability, made now or by another server sharing it (`io-sock` has
/// the same names: an ability then means one thing for both).
fn create(name: &CStr) -> Result<u32, c_int> {
    resource(name, CREATE_ABILITY, ADN_ROOT).or_else(|_| resource(name, LOOKUP_ABILITY, 0))
}

fn resource(name: &CStr, subtype: u16, flags: u32) -> Result<u32, c_int> {
    let bytes = name.to_bytes_with_nul();
    assert!(bytes.len() <= NAME_MAX, "bsdkern: ability name");
    let mut m = [0u8; 16 + NAME_MAX];
    m[0..2].copy_from_slice(&PROC_RESOURCE.to_le_bytes());
    m[2..4].copy_from_slice(&subtype.to_le_bytes());
    m[8..12].copy_from_slice(&flags.to_le_bytes());
    m[12..16].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    m[16..16 + bytes.len()].copy_from_slice(bytes);
    let r = MsgSend(SYSMGR_COID, m.as_ptr().cast(), 16 + bytes.len(), null_mut(), 0);
    if r < 0 { Err(EPERM) } else { Ok(r as u32) }
}

/// Creates `table`'s abilities: a server's first act, while it has the right.
pub fn register(table: &'static Table) -> Result<(), c_int> {
    let mut ids = Vec::new();
    for name in table.abilities {
        ids.push(create(name)?);
    }
    assert!(REGISTERED.set(Registered { table, ids }).is_ok(), "bsdkern: table twice");
    Ok(())
}

/// `struct _client_able`.
#[repr(C)]
struct Able {
    ability: u32,
    flags: u32,
    lo: u64,
    hi: u64,
}

/// Who a client is: its effective user and group IDs.
pub fn identity(scoid: c_int) -> Option<(u32, u32)> {
    let mut info = null_mut::<c_void>();
    // SAFETY: `info` is on this stack.
    if unsafe { ConnectClientInfoAble(scoid, &raw mut info, GETGROUPS, null_mut(), 0) } != 0 {
        return None;
    }
    let p = info.cast::<u32>();
    // SAFETY: the header is there; allocated with `malloc()`.
    unsafe {
        let id = (p.add(INFO_EUID).read(), p.add(INFO_EGID).read());
        resmgr::mem::free(info);
        Some(id)
    }
}

/// This thread now works for the client on `scoid`, or for nobody.
pub fn serve(scoid: Option<c_int>) {
    CLIENT.set(scoid.unwrap_or(-1));
}

/// `priv_check_cred()`, from `kern/priv.c`: 0 or an error. With no client,
/// FreeBSD's own threads may do anything, as `qnx_priv_grant`.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_priv_denied(privilege: c_int) -> c_int {
    let scoid = CLIENT.get();
    if scoid < 0 {
        return 0;
    }
    let Some(r) = REGISTERED.get() else {
        return EPERM;
    };
    if r.table.free.contains(&privilege) {
        return 0;
    }
    let at = r.table.rules.iter().find(|x| x.0 == privilege).map_or(r.table.other, |x| x.1);
    let mut able = Able { ability: r.ids[at], flags: 0, lo: 0, hi: 0 };
    // SAFETY: one entry, on this stack.
    let e = unsafe { ConnectClientInfoAble(scoid, null_mut(), 0, (&raw mut able).cast(), 1) };
    if e == 0 && able.flags != 0 { 0 } else { EPERM }
}
