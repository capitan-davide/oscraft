//! Access control lists on the server side (`<sys/iofunc.h>`): an
//! attribute's ACL, and the `iofunc` functions' hooks that ask it.
//!
//! An attribute's `acl` points to an `iofunc_acl_t`, made by a control
//! (`struct _iofunc_acl_ctrl`) whose function table keeps the ACL in one
//! format. The library has one control, `iofunc_acl_posix_ctrl`, for
//! POSIX ACLs (Chapter 24's format): the `_IO_ACL` handler gets and sets
//! them, and the access check reads them. A file whose ACL has only the
//! three entries the permission bits already say keeps no ACL: a get
//! makes it from the mode.
//!
//! QNX's library can also keep ACLs apart from their attributes, in a
//! table by attribute, for servers that make attributes on demand; this
//! one keeps each attribute's ACL in the attribute only.

use core::ffi::{c_int, c_void};
use core::mem::offset_of;
use core::ptr::null_mut;

use kercall::abi::Iov;
use kercall::tls;
use proto::errno::{EBADMSG, ENOSPC, ENOTSUP};
use proto::io::{self, AclHeader, IO_ACL_CHK, IO_ACL_GET, IO_ACL_SET, IO_CHMOD};
use proto::stat::QnxStat;

use crate::abi::*;
use crate::iofunc::{_iofunc_chmod, iofunc_client_info_ext, iofunc_client_info_ext_free};
use crate::mem::{calloc, free};
use crate::resmgr::{resmgr_msgget, resmgr_msgreplyv};

unsafe extern "C" {
    #[link_name = "realloc"]
    fn c_realloc(p: *mut c_void, size: usize) -> *mut c_void;
}

/// Whether the entries are a valid ACL (QNX's `acl_posix_check_np()`,
/// which oscraft-libc has in its ISO C layer): here only the header's
/// counts are checked, which `check_acl` did already; the entries'
/// order and tags are taken as given, until ACLs are served natively.
///
/// # Safety
///
/// `hdr` points to an ACL header with its entries.
unsafe fn acl_posix_check_np(_hdr: *mut AclHeader) -> c_int {
    0
}

/// `IOFUNC_PC_ACL`: the mount's file system has ACLs.
const PC_ACL: u32 = 0x10;
/// `IOFUNC_ATTR_DIRTY_ACL`: the ACL changed.
const ATTR_DIRTY_ACL: u32 = 0x400;
/// `_IOFUNC_ACL_BUFF_ALLOCED`: the ACL's buffer is the library's.
const BUFF_ALLOCED: u32 = 1;
/// `_S_ACL_EXT`: `st_mode`'s flag for an ACL beyond the permission bits.
const S_ACL_EXT: u32 = 0x1_0000;
/// `_NTO_CI_FULL_GROUPS` and `_NTO_CI_GROUPS_INFO_ONLY`: the client's
/// groups are all there; only their number is.
pub const CI_FULL_GROUPS: u32 = 0x8000_0000;
const CI_GROUPS_INFO_ONLY: u32 = 0x4000_0000;

/// The entries' tags (`ACL_USER_OBJ` to `ACL_OTHER`).
const USER_OBJ: u32 = 1;
const USER: u32 = 2;
const GROUP_OBJ: u32 = 3;
const GROUP: u32 = 4;
const MASK: u32 = 5;
const OTHER: u32 = 6;

/// The most entries an ACL may have, as QNX's control says.
const MAX_ENTS: u16 = 0x1004;

/// `struct _iofunc_acl_funcs`: a control's functions.
#[repr(C)]
pub struct AclFuncs {
    pub nfuncs: u32,
    pub init: unsafe extern "C" fn(*mut IofuncAttr, *mut AclCtrl) -> c_int,
    pub fini: unsafe extern "C" fn(*mut IofuncAttr),
    pub acl: unsafe extern "C" fn(*mut Context, *mut u8, *mut IofuncOcb, *mut IofuncAttr) -> c_int,
    pub access: unsafe extern "C" fn(
        *mut Context,
        *const IofuncAttr,
        *mut IofuncAcl,
        u32,
        *const ClientInfo,
    ) -> c_int,
    pub stat: unsafe extern "C" fn(*mut Context, *const IofuncAttr, *mut u8) -> c_int,
    pub set: unsafe extern "C" fn(*mut IofuncAttr, *const c_void, c_int) -> c_int,
    pub size: unsafe extern "C" fn(*const c_void, *mut usize) -> c_int,
    pub endianise: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    pub update: unsafe extern "C" fn(*mut IofuncAttr),
}

/// `struct _iofunc_acl_ctrl`: a kind of ACL, its functions and limits.
#[repr(C)]
pub struct AclCtrl {
    pub funcs: *const AclFuncs,
    pub max_ents: c_int,
    pub max_buffsize: c_int,
    pub reserved: [c_int; 5],
}

/// `iofunc_acl_t`, and what the POSIX functions add after it: an
/// attribute's ACL, in `buff`, and its numbers of named users and groups
/// and the owning group's entry.
#[repr(C)]
pub struct IofuncAcl {
    pub funcs: *const AclFuncs,
    pub ctrl: *mut AclCtrl,
    pub handle: *mut c_void,
    pub flags: u32,
    pub buffsize: c_int,
    pub buff: *mut AclHeader,
    nuser: c_int,
    group_obj: c_int,
    ngroup: c_int,
}

static mut POSIX_FUNCS: AclFuncs = AclFuncs {
    nfuncs: 9,
    init: posix_init,
    fini: posix_fini,
    acl: posix_acl,
    access: posix_access,
    stat: posix_stat,
    set: posix_set,
    size: posix_size,
    endianise: posix_endianise,
    update: posix_update,
};

static mut POSIX_CTRL: AclCtrl = AclCtrl {
    funcs: &raw const POSIX_FUNCS,
    max_ents: MAX_ENTS as c_int,
    max_buffsize: 0,
    reserved: [0; 5],
};

/// The POSIX ACL control, which servers give `iofunc_acl_init()`.
#[unsafe(no_mangle)]
pub static mut iofunc_acl_posix_ctrl: *mut AclCtrl = &raw mut POSIX_CTRL;

/// An attribute's ACL, if it has one.
fn acl_of(attr: *const IofuncAttr) -> *mut IofuncAcl {
    // SAFETY: the callers' attributes are initialised.
    unsafe { (*attr).acl }.cast()
}

/// An ACL's functions.
///
/// # Safety
///
/// `acl` is an ACL.
unsafe fn funcs<'a>(acl: *const IofuncAcl) -> &'a AclFuncs {
    // SAFETY: the caller's guarantee.
    let f = unsafe { (*acl).funcs };
    // SAFETY: an ACL's functions are its control's, static.
    unsafe { &*f }
}

/// An ACL's buffer, if it holds more than the permission bits say.
///
/// # Safety
///
/// `acl` is null or an ACL.
unsafe fn extended(acl: *const IofuncAcl) -> Option<*mut AclHeader> {
    // SAFETY: the caller's guarantee.
    let hdr = unsafe { acl.as_ref() }?.buff;
    // SAFETY: an ACL's buffer holds its header.
    (!hdr.is_null() && unsafe { (*hdr).nents } >= 4).then_some(hdr)
}

/// The entries after an ACL's header.
///
/// # Safety
///
/// `hdr` is an ACL of `nents` entries.
unsafe fn entries<'a>(hdr: *const AclHeader) -> &'a [io::AclEntry] {
    // SAFETY: the caller's guarantee.
    let n = usize::from(unsafe { (*hdr).nents });
    // SAFETY: as above: the entries follow the header.
    unsafe { core::slice::from_raw_parts(hdr.wrapping_add(1).cast(), n) }
}

/// Whether an ACL fits the control's limits and is valid.
///
/// # Safety
///
/// `ctrl` is a control; `hdr` an ACL whose header is readable.
unsafe fn check(ctrl: *const AclCtrl, hdr: *mut AclHeader) -> c_int {
    // SAFETY: the caller's guarantee.
    let h = unsafe { hdr.read_unaligned() };
    if h.r#type != 0 {
        return ENOTSUP;
    }
    // SAFETY: as above.
    let c = unsafe { &*ctrl };
    if c.max_ents != 0 && c_int::from(h.nents) > c.max_ents {
        return ENOSPC;
    }
    if c.max_buffsize != 0 && h.size as c_int > c.max_buffsize {
        return ENOSPC;
    }
    if (usize::from(h.nents) + 1) * 8 > h.size as usize {
        return EINVAL;
    }
    // SAFETY: the header says its entries are there.
    unsafe { acl_posix_check_np(hdr) }
}

/// Gives the attribute an empty POSIX ACL.
unsafe extern "C" fn posix_init(attr: *mut IofuncAttr, ctrl: *mut AclCtrl) -> c_int {
    let acl = calloc::<IofuncAcl>();
    if acl.is_null() {
        return ENOMEM;
    }
    // SAFETY: the caller's control.
    let funcs = unsafe { (*ctrl).funcs };
    // SAFETY: just allocated, zeroed.
    let a = unsafe { &mut *acl };
    a.funcs = funcs;
    a.ctrl = ctrl;
    // SAFETY: the caller's attribute.
    unsafe { (*attr).acl = acl.cast() };
    0
}

/// Drops the attribute's ACL.
unsafe extern "C" fn posix_fini(attr: *mut IofuncAttr) {
    let acl = acl_of(attr);
    if acl.is_null() {
        return;
    }
    // SAFETY: the caller's attribute.
    unsafe { (*attr).acl = null_mut() };
    // SAFETY: the attribute's ACL.
    let a = unsafe { &mut *acl };
    if !a.buff.is_null() && a.flags & BUFF_ALLOCED != 0 {
        // SAFETY: the library's buffer.
        unsafe { free(a.buff.cast()) };
    }
    // SAFETY: allocated by posix_init().
    unsafe { free(acl.cast()) };
}

/// Sets the owner's, the group class's and the others' entries of an
/// ACL from the mode; the mask, if any, is the group class.
fn mode_to_acl(hdr: *mut AclHeader, mode: u32) {
    // SAFETY: the callers' ACL.
    let n = usize::from(unsafe { (*hdr).nents });
    // SAFETY: as above: its entries.
    let ents = hdr.wrapping_add(1).cast::<io::AclEntry>();
    // SAFETY: as above: its entries.
    let ents = unsafe { core::slice::from_raw_parts_mut(ents, n) };
    let group = mode >> 3 & 7;
    let mut group_obj = None;
    let mut masked = false;
    for (k, e) in ents.iter_mut().enumerate() {
        match e.tag_permset >> 24 {
            USER_OBJ => e.tag_permset = USER_OBJ << 24 | (mode >> 6 & 7),
            GROUP_OBJ => group_obj = Some(k),
            MASK => {
                e.tag_permset = MASK << 24 | group;
                masked = true;
            }
            OTHER => e.tag_permset = OTHER << 24 | (mode & 7),
            _ => {}
        }
    }
    // With a mask, the group class is the mask, and the owning group
    // keeps its own permissions.
    if let Some(k) = group_obj.filter(|_| !masked) {
        ents[k].tag_permset = GROUP_OBJ << 24 | group;
    }
}

/// `_IO_ACL`: gets the attribute's ACL (made from the mode if it has
/// none), checks one, or sets it, with the permission bits following it.
unsafe extern "C" fn posix_acl(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    let acl = acl_of(attr);
    let subtype = crate::iofunc::hdr::<io::Acl>(msg).subtype;
    if subtype == IO_ACL_GET {
        let room = if c.rcvid == 0 {
            c.size
        } else {
            c.msg_max_size.saturating_sub(c.offset)
        };
        // SAFETY: the attribute's ACL, if any.
        let stored = unsafe { extended(acl) };
        let hdr = match stored {
            Some(b) => b,
            None => {
                if room.max(16) - 16 < 32 {
                    return ENOMEM;
                }
                let hdr = msg.wrapping_add(16).cast::<AclHeader>();
                let minimal = [(USER_OBJ, -1), (GROUP_OBJ, -1), (OTHER, -1)];
                crate::iofunc::put_at(
                    msg,
                    16,
                    AclHeader {
                        size: 32,
                        r#type: 0,
                        nents: 3,
                    },
                );
                for (k, (tag, id)) in minimal.into_iter().enumerate() {
                    let e = io::AclEntry {
                        tag_permset: tag << 24,
                        id,
                    };
                    crate::iofunc::put_at(msg, 24 + k * 8, e);
                }
                // SAFETY: the caller's attribute.
                mode_to_acl(hdr, unsafe { (*attr).mode });
                hdr
            }
        };
        let iov = [
            Iov {
                base: msg.cast(),
                len: 16,
            },
            Iov {
                base: hdr.cast(),
                // SAFETY: an ACL's header.
                len: unsafe { (*hdr).size } as usize,
            },
        ];
        // SAFETY: the context, and the two parts.
        if unsafe { resmgr_msgreplyv(ctp, iov.as_ptr(), 2) } < 0 {
            return tls::get_errno();
        }
        return RESMGR_NOREPLY;
    }
    if subtype != IO_ACL_SET && subtype != IO_ACL_CHK {
        return ENOSYS;
    }
    let have = c.size.saturating_sub(16);
    if have < 8 {
        return EBADMSG;
    }
    let nents = crate::iofunc::at::<AclHeader>(msg, 16).nents;
    let bytes = usize::from(nents) * 8 + 8;
    // The ACL, in the message, or read into a buffer of its own.
    let mut owned = null_mut::<u8>();
    let hdr = if bytes <= have {
        msg.wrapping_add(16).cast::<AclHeader>()
    } else {
        if (c.info.srcmsglen as usize) < c.offset + 16 + bytes {
            return EBADMSG;
        }
        if nents > MAX_ENTS {
            return ENOSPC;
        }
        owned = crate::mem::calloc_bytes(bytes);
        if owned.is_null() {
            return ENOMEM;
        }
        // SAFETY: the context; room for the ACL.
        if unsafe { resmgr_msgget(ctp, owned.cast(), bytes, 16) } < 0 {
            // SAFETY: just allocated.
            unsafe { free(owned.cast()) };
            return tls::get_errno();
        }
        owned.cast::<AclHeader>()
    };
    // SAFETY: the attribute's ACL.
    let ctrl = unsafe { (*acl).ctrl };
    // SAFETY: the ACL's control; the ACL read.
    let mut e = unsafe { check(ctrl, hdr) };
    if e == 0 && subtype == IO_ACL_SET {
        e = set_with_mode(ctp, ocb, attr, acl, hdr);
    }
    // SAFETY: null, or allocated above.
    unsafe { free(owned.cast()) };
    e
}

/// Sets a checked ACL, and the mode its entries say, as a chmod by the
/// client would.
fn set_with_mode(
    ctp: *mut Context,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
    acl: *mut IofuncAcl,
    hdr: *mut AclHeader,
) -> c_int {
    // SAFETY: the caller's attribute.
    let a = unsafe { &mut *attr };
    let (old_mode, old_flags) = (a.mode, a.flags);
    let (mut nuser, mut ngroup, mut group_obj) = (0, 0, 0);
    let mut mode = a.mode;
    // SAFETY: a checked ACL.
    for (k, e) in unsafe { entries(hdr) }.iter().enumerate() {
        let perms = e.tag_permset & 7;
        match e.tag_permset >> 24 {
            USER_OBJ => mode = mode & !0o700 | perms << 6,
            USER => nuser += 1,
            GROUP_OBJ => {
                mode = mode & !0o070 | perms << 3;
                group_obj = k as c_int;
            }
            GROUP => ngroup += 1,
            MASK => mode = mode & !0o070 | perms << 3,
            OTHER => mode = mode & !0o007 | perms,
            _ => {}
        }
    }
    let chmod = io::Chmod {
        r#type: IO_CHMOD,
        combine_len: size_of::<io::Chmod>() as u16,
        mode,
    };
    let mut info = null_mut();
    // SAFETY: the caller's OCB.
    let ioflag = unsafe { (*ocb).ioflag };
    // SAFETY: the caller's context.
    let e = unsafe { iofunc_client_info_ext(ctp, ioflag, &mut info, 1) };
    if e != 0 {
        return e;
    }
    let msg = (&raw const chmod).cast_mut().cast();
    // SAFETY: a chmod message, the caller's OCB and attribute, the info.
    let e = unsafe { _iofunc_chmod(ctp, msg, ocb, attr, info) };
    // SAFETY: allocated above.
    unsafe { iofunc_client_info_ext_free(&mut info) };
    if e != 0 {
        return e;
    }
    // SAFETY: a checked ACL.
    let e = unsafe { posix_set(attr, hdr.cast(), 0) };
    if e != 0 {
        a.flags = old_flags;
        a.mode = old_mode;
        return e;
    }
    // SAFETY: the attribute's ACL.
    let acl = unsafe { &mut *acl };
    (acl.nuser, acl.group_obj, acl.ngroup) = (nuser, group_obj, ngroup);
    a.flags |= ATTR_DIRTY_ACL;
    0
}

/// Whether the ACL lets the client do `check` (the owner's permission
/// bits): the first entry that matches decides, the named users and
/// groups within the group class. `ENOSYS` leaves it to the permission
/// bits.
unsafe extern "C" fn posix_access(
    _ctp: *mut Context,
    attr: *const IofuncAttr,
    acl: *mut IofuncAcl,
    check: u32,
    info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's ACL.
    let hdr = unsafe { (*acl).buff };
    // SAFETY: as above: its header.
    if hdr.is_null() || unsafe { (*hdr).nents } < 4 {
        return ENOSYS;
    }
    // SAFETY: the caller's attribute and credentials.
    let a = unsafe { &*attr };
    // SAFETY: as above.
    let i = unsafe { &*info };
    let ngroups = if i.flags & CI_GROUPS_INFO_ONLY != 0 {
        0
    } else {
        i.ngroups as usize
    };
    // The client's groups: the list may go past the struct's eight.
    let groups = i.grouplist.as_ptr();
    // SAFETY: the credentials hold ngroups groups.
    let in_groups = |g: u32| unsafe { core::slice::from_raw_parts(groups, ngroups) }.contains(&g);
    let verdict = |ok: bool| if ok { 0 } else { EACCES };
    let mode = a.mode;
    let mut allowed = 0;
    let mut other = mode << 6;
    // SAFETY: the ACL's entries.
    for e in unsafe { entries(hdr) } {
        let perms = (e.tag_permset & 7) << 6;
        match e.tag_permset >> 24 {
            USER_OBJ if i.euid == a.uid => return verdict(check & !mode == 0),
            USER if i.euid == e.id as u32 => {
                return verdict(check == mode << 3 & check & perms);
            }
            GROUP_OBJ if i.egid == a.gid || in_groups(a.gid) => {
                allowed = mode << 3 & perms;
                other = 0;
            }
            GROUP if i.egid == e.id as u32 || in_groups(e.id as u32) => {
                allowed = mode << 3 & perms;
                other = 0;
            }
            OTHER => {
                if i.flags & CI_FULL_GROUPS == 0 {
                    return ENOSYS;
                }
                return verdict(check & !other == 0);
            }
            _ => {}
        }
        if check & !allowed == 0 {
            return 0;
        }
    }
    EACCES
}

/// Flags a file whose ACL goes beyond the permission bits.
unsafe extern "C" fn posix_stat(_ctp: *mut Context, attr: *const IofuncAttr, st: *mut u8) -> c_int {
    // SAFETY: the attribute's ACL, if any.
    let extended = unsafe { extended(acl_of(attr)) }.is_some();
    let at = offset_of!(QnxStat, st_mode);
    let mode: u32 = crate::iofunc::at(st, at);
    let mode = if extended {
        mode | S_ACL_EXT
    } else {
        mode & !S_ACL_EXT
    };
    crate::iofunc::put_at(st, at, mode);
    0
}

/// Keeps a valid ACL for the attribute: a copy of it, or none if it says
/// no more than the permission bits. `update_stats` recounts its named
/// users and groups.
unsafe extern "C" fn posix_set(
    attr: *mut IofuncAttr,
    data: *const c_void,
    update_stats: c_int,
) -> c_int {
    // SAFETY: the caller's attribute has an ACL.
    let acl = unsafe { &mut *acl_of(attr) };
    let hdr = data.cast::<AclHeader>().cast_mut();
    // SAFETY: the caller's ACL.
    let e = unsafe { check(acl.ctrl, hdr) };
    if e != 0 {
        return e;
    }
    // SAFETY: as above.
    let h = unsafe { hdr.read_unaligned() };
    if h.nents < 4 {
        if acl.flags & BUFF_ALLOCED != 0 {
            // SAFETY: the library's buffer.
            unsafe { free(acl.buff.cast()) };
            acl.buff = null_mut();
            acl.flags &= !BUFF_ALLOCED;
            acl.buffsize = 0;
        } else if !acl.buff.is_null() && acl.buffsize >= 8 {
            // The server's own buffer: empty it.
            // SAFETY: the server's buffer holds a header.
            unsafe { (*acl.buff).nents = 0 };
        }
    } else {
        if acl.buffsize < h.size as c_int {
            if !acl.buff.is_null() && acl.flags & BUFF_ALLOCED == 0 {
                return ENOMEM;
            }
            // SAFETY: null or the library's buffer.
            let p = unsafe { c_realloc(acl.buff.cast(), h.size as usize) };
            if p.is_null() {
                return ENOMEM;
            }
            acl.buff = p.cast();
            acl.flags |= BUFF_ALLOCED;
            acl.buffsize = h.size as c_int;
        }
        // SAFETY: room for the ACL's size, from a valid ACL.
        unsafe {
            core::ptr::copy_nonoverlapping(data.cast::<u8>(), acl.buff.cast(), h.size as usize)
        };
    }
    if update_stats != 0 {
        (acl.nuser, acl.group_obj, acl.ngroup) = (0, 0, 0);
        // SAFETY: a checked ACL.
        for (k, e) in unsafe { entries(hdr) }.iter().enumerate() {
            match e.tag_permset >> 24 {
                USER => acl.nuser += 1,
                GROUP_OBJ => acl.group_obj = k as c_int,
                GROUP => acl.ngroup += 1,
                _ => {}
            }
        }
    }
    0
}

/// An ACL's size in bytes, from its header.
unsafe extern "C" fn posix_size(data: *const c_void, size: *mut usize) -> c_int {
    // SAFETY: the caller's ACL and result.
    let h = unsafe { data.cast::<AclHeader>().read_unaligned() };
    // SAFETY: as above.
    unsafe { size.write(h.size as usize) };
    0
}

/// Swaps an ACL's bytes, to or from the other byte order.
unsafe extern "C" fn posix_endianise(data: *mut c_void, from_non_native: c_int) -> c_int {
    let hdr = data.cast::<AclHeader>();
    // SAFETY: the caller's ACL.
    let h = unsafe { hdr.read_unaligned() };
    let swapped = AclHeader {
        size: h.size.swap_bytes(),
        r#type: h.r#type.swap_bytes(),
        nents: h.nents.swap_bytes(),
    };
    let n = usize::from(if from_non_native != 0 {
        swapped.nents
    } else {
        h.nents
    });
    // SAFETY: as above.
    unsafe { hdr.write_unaligned(swapped) };
    let ents = data.cast::<u32>().wrapping_add(2);
    for k in 0..n * 2 {
        let p = ents.wrapping_add(k);
        // SAFETY: the ACL's entries, two words each.
        let w = unsafe { p.read_unaligned() };
        // SAFETY: as above.
        unsafe { p.write_unaligned(w.swap_bytes()) };
    }
    0
}

/// After a chmod: the ACL's entries follow the new permission bits.
unsafe extern "C" fn posix_update(attr: *mut IofuncAttr) {
    let acl = acl_of(attr);
    // SAFETY: the attribute's ACL.
    let hdr = unsafe { (*acl).buff };
    if hdr.is_null() {
        return;
    }
    // SAFETY: the caller's attribute.
    let a = unsafe { &mut *attr };
    mode_to_acl(hdr, a.mode);
    a.flags |= ATTR_DIRTY_ACL;
}

/// Whether the attribute's mount (if any) has ACLs.
fn mount_has_acls(attr: *const IofuncAttr) -> bool {
    // SAFETY: the callers' attribute, and its mount if any.
    let mount = unsafe { (*attr).mount };
    // SAFETY: as above.
    unsafe { mount.as_ref() }.is_none_or(|m| m.conf & PC_ACL != 0)
}

/// Gives the attribute an ACL of `ctrl`'s kind: `EINVAL` without a
/// control or with an ACL of another kind, `ENOSYS` if it has one.
///
/// # Safety
///
/// `attr` is initialised; `ctrl` is null or a control.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_acl_init(
    attr: *mut IofuncAttr,
    ctrl: *mut AclCtrl,
    _buff: *mut c_void,
) -> c_int {
    let acl = acl_of(attr);
    if ctrl.is_null() {
        return if acl.is_null() { EINVAL } else { ENOSYS };
    }
    if acl.is_null() {
        // SAFETY: the caller's control.
        let f = unsafe { (*ctrl).funcs };
        // SAFETY: a control's functions, static.
        let init = unsafe { (*f).init };
        // SAFETY: the caller's attribute and control.
        return unsafe { init(attr, ctrl) };
    }
    // SAFETY: the attribute's ACL.
    if unsafe { (*acl).ctrl } == ctrl {
        ENOSYS
    } else {
        EINVAL
    }
}

/// Drops the attribute's ACL.
///
/// # Safety
///
/// `attr` is initialised.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_acl_fini(attr: *mut IofuncAttr) {
    let acl = acl_of(attr);
    if !acl.is_null() {
        // SAFETY: the attribute's ACL's functions.
        let fini = unsafe { funcs(acl) }.fini;
        // SAFETY: the caller's attribute.
        unsafe { fini(attr) };
    }
}

/// The attribute's ACL: null if its mount has none.
///
/// # Safety
///
/// `attr` is initialised.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_acl_get(attr: *const IofuncAttr) -> *mut IofuncAcl {
    if mount_has_acls(attr) {
        acl_of(attr)
    } else {
        null_mut()
    }
}

/// The attribute's ACL, whatever its mount: this library keeps it in the
/// attribute, where QNX's tracks it.
///
/// # Safety
///
/// `attr` is initialised.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_acl_track_get(attr: *const IofuncAttr) -> *mut IofuncAcl {
    acl_of(attr)
}

/// Drops the attribute's ACL: `iofunc_acl_fini()`.
///
/// # Safety
///
/// `attr` is initialised.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_acl_track_destroy(attr: *mut IofuncAttr) {
    // SAFETY: the caller's guarantee.
    unsafe { iofunc_acl_fini(attr) }
}

/// Makes sure the attribute has an ACL of `ctrl`'s kind, if its mount has
/// ACLs.
///
/// # Safety
///
/// As `iofunc_acl_init()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_acl_track(attr: *mut IofuncAttr, ctrl: *mut AclCtrl) -> c_int {
    if !mount_has_acls(attr) {
        return ENOSYS;
    }
    if !acl_of(attr).is_null() {
        return 0;
    }
    // SAFETY: the caller's guarantee.
    unsafe { iofunc_acl_init(attr, ctrl, null_mut()) }
}

/// `_IO_ACL` with the attribute's ACL's functions: `ENOSYS` without one.
///
/// # Safety
///
/// `ctp` is a context with the `_IO_ACL` `msg`; `ocb` is attached to
/// `attr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_acl(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    let acl = acl_of(attr);
    if acl.is_null() {
        return ENOSYS;
    }
    // SAFETY: the attribute's ACL's functions; the caller's guarantee.
    let f = unsafe { funcs(acl) }.acl;
    // SAFETY: the caller's guarantee.
    unsafe { f(ctp, msg, ocb, attr) }
}

/// The default `_IO_ACL`: POSIX ACLs, made when first asked for.
///
/// # Safety
///
/// `ctp` is a context with the `_IO_ACL` `msg`; `ocb` is an OCB.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_acl_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above; the library's control.
    let ctrl = unsafe { iofunc_acl_posix_ctrl };
    // SAFETY: as above.
    let e = unsafe { iofunc_acl_track(attr, ctrl) };
    if e != 0 {
        return e;
    }
    // SAFETY: as above.
    unsafe { iofunc_acl(ctp, msg, ocb, attr) }
}

/// `struct _iofunc_acl_hooks`: where the `iofunc` functions ask an
/// attribute's ACL. A server may replace them.
#[repr(C)]
pub struct AclHooks {
    pub nfuncs: u32,
    pub stat: unsafe extern "C" fn(*mut Context, *mut IofuncAttr, *mut u8) -> c_int,
    pub check_access:
        unsafe extern "C" fn(*mut Context, *const IofuncAttr, u32, *const ClientInfo) -> c_int,
    pub unlink: unsafe extern "C" fn(*mut IofuncAttr) -> c_int,
    pub ocb_detach: unsafe extern "C" fn(*mut IofuncAttr) -> c_int,
}

/// Adds the ACL's flag to a file's status.
unsafe extern "C" fn hook_stat(ctp: *mut Context, attr: *mut IofuncAttr, st: *mut u8) -> c_int {
    // SAFETY: the caller's attribute.
    let acl = unsafe { iofunc_acl_get(attr) };
    if acl.is_null() {
        return 0;
    }
    // SAFETY: the attribute's ACL's functions.
    let f = unsafe { funcs(acl) }.stat;
    // SAFETY: the caller's guarantee.
    unsafe { f(ctp, attr, st) }
}

/// The ACL's verdict, `ENOSYS` without one.
unsafe extern "C" fn hook_check_access(
    ctp: *mut Context,
    attr: *const IofuncAttr,
    check: u32,
    info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's attribute.
    let acl = unsafe { iofunc_acl_get(attr) };
    if acl.is_null() {
        return ENOSYS;
    }
    // SAFETY: as above.
    let f = unsafe { funcs(acl) }.access;
    // SAFETY: the caller's guarantee.
    unsafe { f(ctp, attr, acl, check, info) }
}

/// Nothing kept apart from the attribute to drop with a link or an open.
unsafe extern "C" fn hook_release(_attr: *mut IofuncAttr) -> c_int {
    0
}

/// The ACL hooks.
#[unsafe(no_mangle)]
pub static mut acl_hooks: AclHooks = AclHooks {
    nfuncs: 4,
    stat: hook_stat,
    check_access: hook_check_access,
    unlink: hook_release,
    ocb_detach: hook_release,
};

/// After a chmod: the attribute's ACL follows its new mode, if its
/// functions have `update`.
///
/// # Safety
///
/// `attr` is initialised.
pub unsafe fn chmod_update(attr: *mut IofuncAttr) {
    let acl = acl_of(attr);
    if acl.is_null() {
        return;
    }
    // SAFETY: the attribute's ACL's functions.
    let f = unsafe { funcs(acl) };
    if f.nfuncs > 8 {
        // SAFETY: the caller's attribute.
        unsafe { (f.update)(attr) };
    }
}

/// `acl_posix_check_np()`'s error number for an unknown type is
/// `ENOTSUP`, which `check()` returns first.
const _: () = assert!(ENOTSUP == 45);
