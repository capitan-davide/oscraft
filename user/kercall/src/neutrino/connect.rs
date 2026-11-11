//! Connections, a client's end of a channel, and what a server can learn
//! about the client at the other end.

use core::ffi::*;

use super::nr;
use crate::abi::*;
use proto::errno::EINVAL;

kercall! {
    /// `int ConnectAttach(_Uint32t __nd, pid_t __pid, int __chid, unsigned __index, int __flags)`
    errno ConnectAttach = nr::CONNECT_ATTACH;
    fn(nd: u32, pid: PidT, chid: c_int, index: c_uint, flags: c_int) -> c_int
}

kercall! {
    /// `int ConnectAttach_r(_Uint32t __nd, pid_t __pid, int __chid, unsigned __index, int __flags)`
    neg ConnectAttach_r = nr::CONNECT_ATTACH;
    fn(nd: u32, pid: PidT, chid: c_int, index: c_uint, flags: c_int) -> c_int
}

kercall! {
    /// `int ConnectClientInfoExtraArgs(int __scoid, struct _client_info *__info_pp, int __ngroups, struct _client_able *__abilities, int __nable, unsigned *__type_id)`
    errno ConnectClientInfoExtraArgs = nr::CONNECT_CLIENT_INFO;
    fn(scoid: c_int, info_pp: *mut c_void, ngroups: c_int, abilities: *mut c_void, nable: c_int, type_id: *mut c_uint) -> c_int
}

kercall! {
    /// `int ConnectClientInfoExtraArgs_r(int __scoid, struct _client_info *__info_pp, int __ngroups, struct _client_able *__abilities, int __nable, unsigned *__type_id)`
    status ConnectClientInfoExtraArgs_r = nr::CONNECT_CLIENT_INFO;
    fn(scoid: c_int, info_pp: *mut c_void, ngroups: c_int, abilities: *mut c_void, nable: c_int, type_id: *mut c_uint) -> c_int
}

kercall! {
    /// `int ConnectDetach(int __coid)`
    errno ConnectDetach = nr::CONNECT_DETACH;
    fn(coid: c_int) -> c_int
}

kercall! {
    /// `int ConnectDetach_r(int __coid)`
    status ConnectDetach_r = nr::CONNECT_DETACH;
    fn(coid: c_int) -> c_int
}

kercall! {
    /// `int ConnectDupFd(int __op, pid64_t *__dstpid, dupfd_handle_t *__handle, pid_t *__srvpid, struct _msg_info_hdr *__info)`
    errno ConnectDupFd = nr::CONNECT_DUPFD;
    fn(op: c_int, dstpid: *mut i64, handle: *mut u64, srvpid: *mut PidT, info: *mut c_void) -> c_int
}

kercall! {
    /// `int ConnectDupFd_r(int __op, pid64_t *__dstpid, dupfd_handle_t *__handle, pid_t *__srvpid, struct _msg_info_hdr *__info)`
    status ConnectDupFd_r = nr::CONNECT_DUPFD;
    fn(op: c_int, dstpid: *mut i64, handle: *mut u64, srvpid: *mut PidT, info: *mut c_void) -> c_int
}

kercall! {
    /// `int ConnectFlags(pid_t __pid, int __coid, unsigned __mask, unsigned __bits)`
    errno ConnectFlags = nr::CONNECT_FLAGS;
    fn(pid: PidT, coid: c_int, mask: c_uint, bits: c_uint) -> c_int
}

kercall! {
    /// `int ConnectFlags_r(pid_t __pid, int __coid, unsigned __mask, unsigned __bits)`
    neg ConnectFlags_r = nr::CONNECT_FLAGS;
    fn(pid: PidT, coid: c_int, mask: c_uint, bits: c_uint) -> c_int
}

kercall! {
    /// `int ConnectServerInfo(pid_t __pid, int __coid, struct _server_info *__info)`
    errno ConnectServerInfo = nr::CONNECT_SERVER_INFO;
    fn(pid: PidT, coid: c_int, info: *mut c_void) -> c_int
}

kercall! {
    /// `int ConnectServerInfo_r(pid_t __pid, int __coid, struct _server_info *__info)`
    neg ConnectServerInfo_r = nr::CONNECT_SERVER_INFO;
    fn(pid: PidT, coid: c_int, info: *mut c_void) -> c_int
}

// Functions built on the calls above.

/// `int ConnectClientInfo(int scoid, struct _client_info *info, int ngroups)`:
/// what a server can learn about the client on `scoid`, with room for
/// `ngroups` supplementary groups. A negative `ngroups` is `EINVAL`.
#[unsafe(no_mangle)]
pub extern "C" fn ConnectClientInfo(scoid: c_int, info: *mut c_void, ngroups: c_int) -> c_int {
    if ngroups < 0 {
        crate::tls::set_errno(EINVAL);
        return -1;
    }
    ConnectClientInfoExtraArgs(
        scoid,
        info,
        ngroups,
        core::ptr::null_mut(),
        0,
        core::ptr::null_mut(),
    )
}

/// `ConnectClientInfo()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn ConnectClientInfo_r(scoid: c_int, info: *mut c_void, ngroups: c_int) -> c_int {
    if ngroups < 0 {
        return EINVAL;
    }
    ConnectClientInfoExtraArgs_r(
        scoid,
        info,
        ngroups,
        core::ptr::null_mut(),
        0,
        core::ptr::null_mut(),
    )
}

unsafe extern "C" {
    /// The ISO C layer's allocator.
    fn free(p: *mut c_void);
}

/// Frees what `ConnectClientInfoExt()` allocated, and clears `*info`.
///
/// # Safety
///
/// `info` points to null or to `ConnectClientInfoExt()`'s result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ClientInfoExtFree(info: *mut *mut c_void) -> c_int {
    // SAFETY: the caller's guarantee.
    let p = unsafe { info.read() };
    if !p.is_null() {
        // SAFETY: as above.
        unsafe { free(p) };
        // SAFETY: as above.
        unsafe { info.write(core::ptr::null_mut()) };
    }
    0
}
