//! Traffic through the stack inside `netd`, before anything is served.

use core::ffi::{c_int, c_void};
use core::ptr::{null_mut, read_unaligned};

const AF_INET: c_int = 2;
const SOCK_STREAM: c_int = 1;
const SOCK_DGRAM: c_int = 2;

unsafe extern "C" {
    fn bsdnet_socket(d: c_int, t: c_int, p: c_int, so: *mut *mut c_void) -> c_int;
    fn bsdnet_bind(so: *mut c_void, a: *const Sin) -> c_int;
    fn bsdnet_connect(so: *mut c_void, a: *const Sin) -> c_int;
    fn bsdnet_listen(so: *mut c_void, backlog: c_int) -> c_int;
    fn bsdnet_accept(so: *mut c_void, out: *mut *mut c_void, a: *mut Sin) -> c_int;
    fn bsdnet_send(so: *mut c_void, b: *const u8, n: *mut usize, to: *const Sin, f: c_int) -> c_int;
    fn bsdnet_recv(so: *mut c_void, b: *mut u8, n: *mut usize, from: *mut Sin, f: *mut c_int) -> c_int;
    fn bsdnet_close(so: *mut c_void) -> c_int;
}

/// `struct sockaddr_in`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Sin {
    len: u8,
    family: u8,
    port: u16,
    addr: [u8; 4],
    zero: [u8; 8],
    room: [u8; 112],
}

fn local(port: u16) -> Sin {
    Sin { len: 16, family: AF_INET as u8, port: port.to_be(), addr: [127, 0, 0, 1], zero: [0; 8], room: [0; 112] }
}

fn check(what: &str, e: c_int) {
    println!("netd: {what}: {e}");
    assert_eq!(e, 0, "netd: {what}");
}

struct Ptr(*mut c_void);
// SAFETY: a socket may be used from any thread.
unsafe impl Send for Ptr {}

/// UDP and TCP to itself over `lo0`.
pub fn run() {
    // SAFETY: the pointers are all on this stack, the sockets FreeBSD's.
    unsafe {
        let (mut a, mut b) = (null_mut(), null_mut());
        check("udp socket", bsdnet_socket(AF_INET, SOCK_DGRAM, 0, &raw mut a));
        check("udp socket", bsdnet_socket(AF_INET, SOCK_DGRAM, 0, &raw mut b));
        check("udp bind", bsdnet_bind(a, &local(7000)));
        let mut n = 5;
        check("udp send", bsdnet_send(b, b"hello".as_ptr(), &raw mut n, &local(7000), 0));
        let mut buf = [0u8; 16];
        let mut n = 16;
        let mut from = local(0);
        let mut flags = 0;
        check("udp recv", bsdnet_recv(a, buf.as_mut_ptr(), &raw mut n, &raw mut from, &raw mut flags));
        println!("netd: udp got {:?} from port {}", core::str::from_utf8(&buf[..n]), u16::from_be(read_unaligned(&raw const from.port)));
        bsdnet_close(a);
        bsdnet_close(b);

        let (mut l, mut c, mut s) = (null_mut(), null_mut(), null_mut());
        check("tcp socket", bsdnet_socket(AF_INET, SOCK_STREAM, 0, &raw mut l));
        check("tcp bind", bsdnet_bind(l, &local(7001)));
        check("tcp listen", bsdnet_listen(l, 5));
        let server = Ptr(l);
        let t = std::thread::spawn(move || {
            let server = server;
            let mut s = null_mut();
            let mut peer = local(0);
            let e = bsdnet_accept(server.0, &raw mut s, &raw mut peer);
            (e, Ptr(s))
        });
        check("tcp socket", bsdnet_socket(AF_INET, SOCK_STREAM, 0, &raw mut c));
        check("tcp connect", bsdnet_connect(c, &local(7001)));
        let (e, acc) = t.join().unwrap();
        check("tcp accept", e);
        s = acc.0.cast::<c_void>().cast();
        let mut n = 11;
        check("tcp send", bsdnet_send(c, b"hello world".as_ptr(), &raw mut n, core::ptr::null(), 0));
        let mut buf = [0u8; 32];
        let mut n = 32;
        let mut flags = 0;
        check("tcp recv", bsdnet_recv(s, buf.as_mut_ptr(), &raw mut n, null_mut(), &raw mut flags));
        println!("netd: tcp got {:?}", core::str::from_utf8(&buf[..n]));
        bsdnet_close(c);
        bsdnet_close(s);
        bsdnet_close(l);
    }
    println!("netd: traffic passed");
}
