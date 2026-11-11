//! `priochain`: a native test server for `priochaintest`, which measures
//! that a QNX client's priority reaches a native server through `qnxd`.
//! It says where its channel is in `/dev/shmem/priochain` and answers
//! each message with the priority it is running at, and the message
//! `0` ends it.

use core::ffi::{c_int, c_void};
use core::ptr::null_mut;

use kercall::neutrino::channel::ChannelCreate;
use kercall::neutrino::msg::{MsgReceive, MsgReply};
use kercall::neutrino::sched::SchedGet;

fn main() {
    let chid = ChannelCreate(0);
    let at = format!("{} {}\n", std::process::id(), chid);
    if chid < 0 || std::fs::write("/dev/shmem/priochain", at).is_err() {
        std::process::exit(1);
    }
    loop {
        let mut msg: u32 = 0;
        let rcvid = MsgReceive(chid, (&raw mut msg).cast(), 4, null_mut());
        if rcvid <= 0 {
            continue;
        }
        // `struct sched_param`: the priority, the current one, the rest.
        let mut param = [0 as c_int; 16];
        SchedGet(0, 0, param.as_mut_ptr().cast());
        MsgReply(rcvid, 0, (&raw const param[1]).cast::<c_void>(), 4);
        if msg == 0 {
            return;
        }
    }
}
