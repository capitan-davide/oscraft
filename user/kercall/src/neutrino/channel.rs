//! Channels: where a server receives messages and pulses.

use core::ffi::*;

use super::nr;

kercall! {
    /// `int ChannelCreate(unsigned __flags)`
    errno ChannelCreate = nr::CHANNEL_CREATE;
    fn(flags: c_uint) -> c_int
}

kercall! {
    /// `int ChannelCreate_r(unsigned __flags)`
    neg ChannelCreate_r = nr::CHANNEL_CREATE;
    fn(flags: c_uint) -> c_int
}

kercall! {
    /// `int ChannelCtl(int __chid, int __cmd, void *data)`
    errno ChannelCtl = nr::CHANNEL_CTL;
    fn(chid: c_int, cmd: c_int, data: *mut c_void) -> c_int
}

kercall! {
    /// `int ChannelCtl_r(int __chid, int __cmd, void *data)`
    status ChannelCtl_r = nr::CHANNEL_CTL;
    fn(chid: c_int, cmd: c_int, data: *mut c_void) -> c_int
}

kercall! {
    /// `int ChannelDestroy(int __chid)`
    errno ChannelDestroy = nr::CHANNEL_DESTROY;
    fn(chid: c_int) -> c_int
}

kercall! {
    /// `int ChannelDestroy_r(int __chid)`
    status ChannelDestroy_r = nr::CHANNEL_DESTROY;
    fn(chid: c_int) -> c_int
}

/// `_NTO_CHF_PULSE_POOL`, and `_NTO_CHANNEL_CTL_SET_PULSE_POOL`.
const CHF_PULSE_POOL: c_uint = 0x100;
const CTL_SET_PULSE_POOL: c_int = 1;

/// `int ChannelCreatePulsePool(unsigned flags, const struct nto_channel_config *config)`:
/// a channel with its own pool of pulses, as `config` sets it up.
#[unsafe(no_mangle)]
pub extern "C" fn ChannelCreatePulsePool(flags: c_uint, config: *const c_void) -> c_int {
    let chid = ChannelCreate(flags | CHF_PULSE_POOL);
    if chid == -1 {
        return -1;
    }
    if ChannelCtl(chid, CTL_SET_PULSE_POOL, config.cast_mut()) == -1 {
        ChannelDestroy(chid);
        return -1;
    }
    chid
}
