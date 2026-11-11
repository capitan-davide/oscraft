//! Message passing: sending, receiving and replying; reading and writing
//! the parts of a message; pulses and events.

use core::ffi::*;

use super::nr;
use crate::abi::*;
use proto::event::SIGEV_FLAG_HANDLE;

kercall! {
    /// `int MsgCurrent(rcvid_t __rcvid)`
    errno MsgCurrent = nr::MSG_CURRENT;
    fn(rcvid: RcvidT) -> c_int
}

kercall! {
    /// `int MsgCurrent_r(rcvid_t __rcvid)`
    status MsgCurrent_r = nr::MSG_CURRENT;
    fn(rcvid: RcvidT) -> c_int
}

kercall! {
    /// `int MsgDeliverEvent(rcvid_t __rcvid, const struct sigevent *__event)`
    errno MsgDeliverEvent = nr::MSG_DELIVER_EVENT;
    fn(rcvid: RcvidT, event: *const c_void) -> c_int
}

kercall! {
    /// `int MsgDeliverEvent_r(rcvid_t __rcvid, const struct sigevent *__event)`
    status MsgDeliverEvent_r = nr::MSG_DELIVER_EVENT;
    fn(rcvid: RcvidT, event: *const c_void) -> c_int
}

kercall! {
    /// `int MsgError(rcvid_t __rcvid, int __err)`
    errno MsgError = nr::MSG_ERROR;
    fn(rcvid: RcvidT, err: c_int) -> c_int
}

kercall! {
    /// `int MsgError_r(rcvid_t __rcvid, int __err)`
    status MsgError_r = nr::MSG_ERROR;
    fn(rcvid: RcvidT, err: c_int) -> c_int
}

kercall! {
    /// `int MsgInfo(rcvid_t __rcvid, struct _msg_info *__info)`
    errno MsgInfo = nr::MSG_INFO;
    fn(rcvid: RcvidT, info: *mut c_void) -> c_int
}

kercall! {
    /// `int MsgInfo_r(rcvid_t __rcvid, struct _msg_info *__info)`
    status MsgInfo_r = nr::MSG_INFO;
    fn(rcvid: RcvidT, info: *mut c_void) -> c_int
}

kercall! {
    /// `int MsgPause(rcvid_t __rcvid, unsigned __cookie)`
    errno MsgPause = nr::MSG_PAUSE;
    fn(rcvid: RcvidT, cookie: c_uint) -> c_int
}

kercall! {
    /// `int MsgPause_r(rcvid_t __rcvid, unsigned __cookie)`
    status MsgPause_r = nr::MSG_PAUSE;
    fn(rcvid: RcvidT, cookie: c_uint) -> c_int
}

kercall! {
    /// `int MsgQueueClose(rcvid_t __rcvid, int __queueid)`
    errno MsgQueueClose = nr::MSG_QUEUE_CLOSE;
    fn(rcvid: RcvidT, queueid: c_int) -> c_int
}

kercall! {
    /// `int MsgQueueCtl(int __fd, int __cmd, void *__data)`
    errno MsgQueueCtl = nr::MSG_QUEUE_CTL;
    fn(fd: c_int, cmd: c_int, data: *mut c_void) -> c_int
}

kercall! {
    /// `int MsgQueueOpen(rcvid_t __rcvid, int __queueid, _Uint32t __msgsize, _Uint32t __maxmsg, unsigned __oflag)`
    errno MsgQueueOpen = nr::MSG_QUEUE_OPEN;
    fn(rcvid: RcvidT, queueid: c_int, msgsize: u32, maxmsg: u32, oflag: c_uint) -> c_int
}

kercall! {
    /// `int MsgQueueReceive(int __fd, char *__msg, size_t __msglen, unsigned *priority)`
    errno MsgQueueReceive = nr::MSG_QUEUE_RECEIVE;
    fn(fd: c_int, msg: *mut c_char, msglen: usize, priority: *mut c_uint) -> c_int
}

kercall! {
    /// `int MsgQueueSend(int __fd, char const *__msg, size_t __msglen, unsigned __priority)`
    errno MsgQueueSend = nr::MSG_QUEUE_SEND;
    fn(fd: c_int, msg: *const c_char, msglen: usize, priority: c_uint) -> c_int
}

kercall! {
    /// `ssize_t MsgRead(rcvid_t __rcvid, void *__msg, _Sizet __bytes, _Sizet __offset)`
    errno MsgRead = nr::MSG_READV, neg(2);
    fn(rcvid: RcvidT, msg: *mut c_void, bytes: usize, offset: usize) -> SsizeT
}

kercall! {
    /// `ssize_t MsgRead_r(rcvid_t __rcvid, void *__msg, _Sizet __bytes, _Sizet __offset)`
    neg MsgRead_r = nr::MSG_READV, neg(2);
    fn(rcvid: RcvidT, msg: *mut c_void, bytes: usize, offset: usize) -> SsizeT
}

kercall! {
    /// `ssize_t MsgReadv(rcvid_t __rcvid, const struct iovec *__iov, _Sizet __parts, _Sizet __offset)`
    errno MsgReadv = nr::MSG_READV;
    fn(rcvid: RcvidT, iov: *const c_void, parts: usize, offset: usize) -> SsizeT
}

kercall! {
    /// `ssize_t MsgReadv_r(rcvid_t __rcvid, const struct iovec *__iov, _Sizet __parts, _Sizet __offset)`
    neg MsgReadv_r = nr::MSG_READV;
    fn(rcvid: RcvidT, iov: *const c_void, parts: usize, offset: usize) -> SsizeT
}

kercall! {
    /// `rcvid_t MsgReceive(int __chid, void *__msg, _Sizet __bytes, struct _msg_info *__info)`
    errno MsgReceive = nr::MSG_RECEIVEV, neg(2);
    fn(chid: c_int, msg: *mut c_void, bytes: usize, info: *mut c_void) -> RcvidT
}

kercall! {
    /// `rcvid_t MsgReceive_r(int __chid, void *__msg, _Sizet __bytes, struct _msg_info *__info)`
    neg MsgReceive_r = nr::MSG_RECEIVEV, neg(2);
    fn(chid: c_int, msg: *mut c_void, bytes: usize, info: *mut c_void) -> RcvidT
}

kercall! {
    /// `int MsgReceivePulse(int __chid, void *__pulse, _Sizet __bytes, struct _msg_info *__info)`
    errno MsgReceivePulse = nr::MSG_RECEIVEPULSEV, neg(2);
    fn(chid: c_int, pulse: *mut c_void, bytes: usize, info: *mut c_void) -> c_int
}

kercall! {
    /// `int MsgReceivePulse_r(int __chid, void *__pulse, _Sizet __bytes, struct _msg_info *__info)`
    neg MsgReceivePulse_r = nr::MSG_RECEIVEPULSEV, neg(2);
    fn(chid: c_int, pulse: *mut c_void, bytes: usize, info: *mut c_void) -> c_int
}

kercall! {
    /// `rcvid_t MsgReceivev(int __chid, const struct iovec *__iov, _Sizet __parts, struct _msg_info *__info)`
    errno MsgReceivev = nr::MSG_RECEIVEV;
    fn(chid: c_int, iov: *const c_void, parts: usize, info: *mut c_void) -> RcvidT
}

kercall! {
    /// `rcvid_t MsgReceivev_r(int __chid, const struct iovec *__iov, _Sizet __parts, struct _msg_info *__info)`
    neg MsgReceivev_r = nr::MSG_RECEIVEV;
    fn(chid: c_int, iov: *const c_void, parts: usize, info: *mut c_void) -> RcvidT
}

kercall! {
    /// `int MsgReply(rcvid_t __rcvid, long __status, const void *__msg, _Sizet __bytes)`
    errno MsgReply = nr::MSG_REPLYV, neg(3);
    fn(rcvid: RcvidT, status: c_long, msg: *const c_void, bytes: usize) -> c_int
}

kercall! {
    /// `int MsgReply_r(rcvid_t __rcvid, long __status, const void *__msg, _Sizet __bytes)`
    neg MsgReply_r = nr::MSG_REPLYV, neg(3);
    fn(rcvid: RcvidT, status: c_long, msg: *const c_void, bytes: usize) -> c_int
}

kercall! {
    /// `int MsgReplyv(rcvid_t __rcvid, long __status, const struct iovec *__iov, _Sizet __parts)`
    errno MsgReplyv = nr::MSG_REPLYV;
    fn(rcvid: RcvidT, status: c_long, iov: *const c_void, parts: usize) -> c_int
}

kercall! {
    /// `int MsgReplyv_r(rcvid_t __rcvid, long __status, const struct iovec *__iov, _Sizet __parts)`
    neg MsgReplyv_r = nr::MSG_REPLYV;
    fn(rcvid: RcvidT, status: c_long, iov: *const c_void, parts: usize) -> c_int
}

kercall! {
    /// `long MsgSend(int __coid, const void *__smsg, _Sizet __sbytes, void *__rmsg, _Sizet __rbytes)`
    errno MsgSend = nr::MSG_SENDV, neg(2, 4);
    fn(coid: c_int, smsg: *const c_void, sbytes: usize, rmsg: *mut c_void, rbytes: usize) -> c_long
}

kercall! {
    /// `long MsgSend_r(int __coid, const void *__smsg, _Sizet __sbytes, void *__rmsg, _Sizet __rbytes)`
    neg MsgSend_r = nr::MSG_SENDV, neg(2, 4);
    fn(coid: c_int, smsg: *const c_void, sbytes: usize, rmsg: *mut c_void, rbytes: usize) -> c_long
}

kercall! {
    /// `int MsgSendPulse(int __coid, int __priority, int __code, long __value)`
    errno MsgSendPulse = nr::MSG_SEND_PULSE;
    fn(coid: c_int, priority: c_int, code: c_int, value: c_long) -> c_int
}

kercall! {
    /// `int MsgSendPulse_r(int __coid, int __priority, int __code, long __value)`
    status MsgSendPulse_r = nr::MSG_SEND_PULSE;
    fn(coid: c_int, priority: c_int, code: c_int, value: c_long) -> c_int
}

kercall! {
    /// `int MsgSendPulsePtr(int __coid, int __priority, int __code, void * __value)`
    errno MsgSendPulsePtr = nr::MSG_SEND_PULSEPTR;
    fn(coid: c_int, priority: c_int, code: c_int, value: *mut c_void) -> c_int
}

kercall! {
    /// `int MsgSendPulsePtr_r(int __coid, int __priority, int __code, void * __value)`
    status MsgSendPulsePtr_r = nr::MSG_SEND_PULSEPTR;
    fn(coid: c_int, priority: c_int, code: c_int, value: *mut c_void) -> c_int
}

kercall! {
    /// `long MsgSendnc(int __coid, const void *__smsg, _Sizet __sbytes, void *__rmsg, _Sizet __rbytes)`
    errno MsgSendnc = nr::MSG_SENDVNC, neg(2, 4);
    fn(coid: c_int, smsg: *const c_void, sbytes: usize, rmsg: *mut c_void, rbytes: usize) -> c_long
}

kercall! {
    /// `long MsgSendnc_r(int __coid, const void *__smsg, _Sizet __sbytes, void *__rmsg, _Sizet __rbytes)`
    neg MsgSendnc_r = nr::MSG_SENDVNC, neg(2, 4);
    fn(coid: c_int, smsg: *const c_void, sbytes: usize, rmsg: *mut c_void, rbytes: usize) -> c_long
}

kercall! {
    /// `long MsgSendsv(int __coid, const void *__smsg, _Sizet __sbytes, const struct iovec *__riov, _Sizet __rparts)`
    errno MsgSendsv = nr::MSG_SENDV, neg(2);
    fn(coid: c_int, smsg: *const c_void, sbytes: usize, riov: *const c_void, rparts: usize) -> c_long
}

kercall! {
    /// `long MsgSendsv_r(int __coid, const void *__smsg, _Sizet __sbytes, const struct iovec *__riov, _Sizet __rparts)`
    neg MsgSendsv_r = nr::MSG_SENDV, neg(2);
    fn(coid: c_int, smsg: *const c_void, sbytes: usize, riov: *const c_void, rparts: usize) -> c_long
}

kercall! {
    /// `long MsgSendsvnc(int __coid, const void *__smsg, _Sizet __sbytes, const struct iovec *__riov, _Sizet __rparts)`
    errno MsgSendsvnc = nr::MSG_SENDVNC, neg(2);
    fn(coid: c_int, smsg: *const c_void, sbytes: usize, riov: *const c_void, rparts: usize) -> c_long
}

kercall! {
    /// `long MsgSendsvnc_r(int __coid, const void *__smsg, _Sizet __sbytes, const struct iovec *__riov, _Sizet __rparts)`
    neg MsgSendsvnc_r = nr::MSG_SENDVNC, neg(2);
    fn(coid: c_int, smsg: *const c_void, sbytes: usize, riov: *const c_void, rparts: usize) -> c_long
}

kercall! {
    /// `long MsgSendv(int __coid, const struct iovec *__siov, _Sizet __sparts, const struct iovec *__riov, _Sizet __rparts)`
    errno MsgSendv = nr::MSG_SENDV;
    fn(coid: c_int, siov: *const c_void, sparts: usize, riov: *const c_void, rparts: usize) -> c_long
}

kercall! {
    /// `long MsgSendv_r(int __coid, const struct iovec *__siov, _Sizet __sparts, const struct iovec *__riov, _Sizet __rparts)`
    neg MsgSendv_r = nr::MSG_SENDV;
    fn(coid: c_int, siov: *const c_void, sparts: usize, riov: *const c_void, rparts: usize) -> c_long
}

kercall! {
    /// `long MsgSendvnc(int __coid, const struct iovec *__siov, _Sizet __sparts, const struct iovec *__riov, _Sizet __rparts)`
    errno MsgSendvnc = nr::MSG_SENDVNC;
    fn(coid: c_int, siov: *const c_void, sparts: usize, riov: *const c_void, rparts: usize) -> c_long
}

kercall! {
    /// `long MsgSendvnc_r(int __coid, const struct iovec *__siov, _Sizet __sparts, const struct iovec *__riov, _Sizet __rparts)`
    neg MsgSendvnc_r = nr::MSG_SENDVNC;
    fn(coid: c_int, siov: *const c_void, sparts: usize, riov: *const c_void, rparts: usize) -> c_long
}

kercall! {
    /// `long MsgSendvs(int __coid, const struct iovec *__siov, _Sizet __sparts, void *__rmsg, _Sizet __rbytes)`
    errno MsgSendvs = nr::MSG_SENDV, neg(4);
    fn(coid: c_int, siov: *const c_void, sparts: usize, rmsg: *mut c_void, rbytes: usize) -> c_long
}

kercall! {
    /// `long MsgSendvs_r(int __coid, const struct iovec *__siov, _Sizet __sparts, void *__rmsg, _Sizet __rbytes)`
    neg MsgSendvs_r = nr::MSG_SENDV, neg(4);
    fn(coid: c_int, siov: *const c_void, sparts: usize, rmsg: *mut c_void, rbytes: usize) -> c_long
}

kercall! {
    /// `long MsgSendvsnc(int __coid, const struct iovec *__siov, _Sizet __sparts, void *__rmsg, _Sizet __rbytes)`
    errno MsgSendvsnc = nr::MSG_SENDVNC, neg(4);
    fn(coid: c_int, siov: *const c_void, sparts: usize, rmsg: *mut c_void, rbytes: usize) -> c_long
}

kercall! {
    /// `long MsgSendvsnc_r(int __coid, const struct iovec *__siov, _Sizet __sparts, void *__rmsg, _Sizet __rbytes)`
    neg MsgSendvsnc_r = nr::MSG_SENDVNC, neg(4);
    fn(coid: c_int, siov: *const c_void, sparts: usize, rmsg: *mut c_void, rbytes: usize) -> c_long
}

kercall! {
    /// `int MsgVerifyEvent(rcvid_t __rcvid, const struct sigevent *__event)`
    errno MsgVerifyEvent = nr::MSG_VERIFY_EVENT;
    fn(rcvid: RcvidT, event: *const c_void) -> c_int
}

kercall! {
    /// `int MsgVerifyEvent_r(rcvid_t __rcvid, const struct sigevent *__event)`
    status MsgVerifyEvent_r = nr::MSG_VERIFY_EVENT;
    fn(rcvid: RcvidT, event: *const c_void) -> c_int
}

kercall! {
    /// `ssize_t MsgWrite(rcvid_t __rcvid, const void *__msg, _Sizet __bytes, _Sizet __offset)`
    errno MsgWrite = nr::MSG_WRITEV, neg(2);
    fn(rcvid: RcvidT, msg: *const c_void, bytes: usize, offset: usize) -> SsizeT
}

kercall! {
    /// `ssize_t MsgWrite_r(rcvid_t __rcvid, const void *__msg, _Sizet __bytes, _Sizet __offset)`
    neg MsgWrite_r = nr::MSG_WRITEV, neg(2);
    fn(rcvid: RcvidT, msg: *const c_void, bytes: usize, offset: usize) -> SsizeT
}

kercall! {
    /// `ssize_t MsgWritev(rcvid_t __rcvid, const struct iovec *__iov, _Sizet __parts, _Sizet __offset)`
    errno MsgWritev = nr::MSG_WRITEV;
    fn(rcvid: RcvidT, iov: *const c_void, parts: usize, offset: usize) -> SsizeT
}

kercall! {
    /// `ssize_t MsgWritev_r(rcvid_t __rcvid, const struct iovec *__iov, _Sizet __parts, _Sizet __offset)`
    neg MsgWritev_r = nr::MSG_WRITEV;
    fn(rcvid: RcvidT, iov: *const c_void, parts: usize, offset: usize) -> SsizeT
}

kercall! {
    /// `int __MsgRegisterEvent_r(struct sigevent *__event, int __coid, unsigned *__handle)`:
    /// registers `event` for the server on `coid`, or with a null event,
    /// forgets the registration `*handle`.
    status __MsgRegisterEvent_r = nr::MSG_REGISTER_EVENT;
    fn(event: *mut c_void, coid: c_int, handle: *mut c_uint) -> c_int
}


/// Registers `event` so that the server on `coid` can deliver it: the
/// kernel keeps the event, and `event` becomes its handle. Returns 0 or
/// the error number.
///
/// # Safety
///
/// `event` is null or a writable `struct sigevent`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn MsgRegisterEvent_r(event: *mut c_void, coid: c_int) -> c_int {
    if event.is_null() {
        return 22;
    }
    let mut handle: c_uint = 0;
    let error = __MsgRegisterEvent_r(event, coid, &mut handle);
    if error == 0 {
        let words = event.cast::<c_int>();
        // SAFETY: the caller's guarantee: sigev_notify is the first word,
        // sigev_value the third.
        let notify = unsafe { words.read() };
        // SAFETY: as above.
        unsafe { words.write(notify | SIGEV_FLAG_HANDLE) };
        // SAFETY: as above.
        unsafe { words.wrapping_add(2).write(handle as c_int) };
    }
    error
}

/// `MsgRegisterEvent_r()`, with `errno`.
///
/// # Safety
///
/// As `MsgRegisterEvent_r()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn MsgRegisterEvent(event: *mut c_void, coid: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    let error = unsafe { MsgRegisterEvent_r(event, coid) };
    if error != 0 {
        crate::tls::set_errno(error);
        return -1;
    }
    0
}

/// Forgets the registration `event` holds. Returns 0 or the error
/// number: `EINVAL` if it holds none.
///
/// # Safety
///
/// `event` is a `struct sigevent`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn MsgUnregisterEvent_r(event: *const c_void) -> c_int {
    let words = event.cast::<c_int>();
    // SAFETY: the caller's guarantee.
    if unsafe { words.read() } & SIGEV_FLAG_HANDLE == 0 {
        return 22;
    }
    // SAFETY: as above.
    let mut handle = unsafe { words.wrapping_add(2).read() } as c_uint;
    __MsgRegisterEvent_r(core::ptr::null_mut(), 0, &mut handle)
}

/// `MsgUnregisterEvent_r()`, with `errno`.
///
/// # Safety
///
/// As `MsgUnregisterEvent_r()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn MsgUnregisterEvent(event: *const c_void) -> c_int {
    // SAFETY: the caller's guarantee.
    let error = unsafe { MsgUnregisterEvent_r(event) };
    if error != 0 {
        crate::tls::set_errno(error);
        return -1;
    }
    0
}

kercall! {
    /// `int MsgQueueOpen_r(...)`: `MsgQueueOpen()`, the error negated.
    neg MsgQueueOpen_r = nr::MSG_QUEUE_OPEN;
    fn(rcvid: RcvidT, queueid: c_int, msgsize: u32, maxmsg: u32, oflag: c_uint) -> c_int
}

kercall! {
    /// `int MsgQueueClose_r(rcvid_t __rcvid, int __queueid)`
    neg MsgQueueClose_r = nr::MSG_QUEUE_CLOSE;
    fn(rcvid: RcvidT, queueid: c_int) -> c_int
}

kercall! {
    /// `int MsgQueueCtl_r(int __fd, int __cmd, void *__data)`
    neg MsgQueueCtl_r = nr::MSG_QUEUE_CTL;
    fn(fd: c_int, cmd: c_int, data: *mut c_void) -> c_int
}

kercall! {
    /// `int MsgQueueSend_r(int __fd, char const *__msg, size_t __msglen, unsigned __priority)`
    neg MsgQueueSend_r = nr::MSG_QUEUE_SEND;
    fn(fd: c_int, msg: *const c_char, msglen: usize, priority: c_uint) -> c_int
}

kercall! {
    /// `int MsgQueueReceive_r(int __fd, char *__msg, size_t __msglen, unsigned *priority)`
    neg MsgQueueReceive_r = nr::MSG_QUEUE_RECEIVE;
    fn(fd: c_int, msg: *mut c_char, msglen: usize, priority: *mut c_uint) -> c_int
}

kercall! {
    /// `KerTest()`: the kernel's test call, its arguments passed through.
    errno KerTest = nr::KER_TEST;
    fn(a: usize, b: usize, c: usize, d: usize) -> isize
}

kercall! {
    /// `KerTest_r()`
    neg KerTest_r = nr::KER_TEST;
    fn(a: usize, b: usize, c: usize, d: usize) -> isize
}

kercall! {
    /// `SysCustom()`: a call a kernel module adds, its arguments passed
    /// through.
    errno SysCustom = nr::SYS_CUSTOM;
    fn(a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize
}

kercall! {
    /// `SysCustom_r()`
    neg SysCustom_r = nr::SYS_CUSTOM;
    fn(a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize
}

kercall! {
    /// `MsgForwardv(rcvid, smsg, sparts, rmsg, rparts, replied)`: OsCraft's:
    /// `MsgSendv()` for a client; negative part counts: one buffer.
    errno MsgForwardv = nr::OSCRAFT_MSG_FORWARDV;
    fn(rcvid: RcvidT, smsg: *const c_void, sparts: i64, rmsg: *mut c_void, rparts: i64, replied: *mut u64) -> SsizeT
}
