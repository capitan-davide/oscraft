//! What `syslogd` decides: pure logic, tested on the host. The behaviour
//! is FreeBSD's `usr.sbin/syslogd/syslogd.c`, function by function as named
//! in each module.

pub mod conf;
pub mod message;
pub mod repeat;
