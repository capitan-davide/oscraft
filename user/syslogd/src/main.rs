//! `syslogd`: FreeBSD's system logger, rewritten for OsCraft. To its users
//! it is the same: `syslog(3)`, `logger`, `syslog.conf`, log files. Inside,
//! a process logs to its own ring (`logring`) with no kernel call: this
//! server hands the rings out through `/var/run/log`, reads them when woken
//! by a pulse, and keeps a ring after its process ended until it is read.
//!
//! A read of `/var/run/log` is the registration: the reply names the
//! ring. libsys's `connect()` to `/var/run/log` does it, and the
//! descriptor stands for the socket from then on; a pulse on it wakes
//! this server, and its close says the process is done. Rings outlive
//! this server: a new one adopts them, and a client whose wake-up fails
//! registers again without sending its message twice.

use core::ffi::{c_char, c_int, c_long, c_void};
use core::ptr::{null, null_mut};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write};
use std::net::UdpSocket;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, fchown};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use kercall::neutrino::connect::ConnectClientInfo;
use kercall::neutrino::msg::MsgReply;
use logring::{Read, Reader, Ring};
use proto::info::ClientInfo;
use resmgr::abi::{ConnectFuncs, Context, IoFuncs, IofuncAttr, ResmgrAttr, RESMGR_NOREPLY};
use resmgr::dispatch::{dispatch_block, dispatch_context_alloc, dispatch_create, dispatch_handler, pulse_attach};
use resmgr::iofunc::{iofunc_attr_init, iofunc_close_ocb_default, iofunc_func_init};
use resmgr::resmgr::resmgr_attach;
use syslogd::conf::{self, Action, Rule};
use syslogd::message;
use syslogd::repeat::Repeat;

unsafe extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    fn gethostname(name: *mut c_char, len: usize) -> c_int;
    fn regcomp(re: *mut c_void, pattern: *const c_char, flags: c_int) -> c_int;
    fn regexec(re: *const c_void, s: *const c_char, n: usize, m: *mut c_void, flags: c_int) -> c_int;
    fn regfree(re: *mut c_void);
    fn sigemptyset(set: *mut u32) -> c_int;
    fn sigaddset(set: *mut u32, sig: c_int) -> c_int;
    fn pthread_sigmask(how: c_int, set: *const u32, old: *mut u32) -> c_int;
    fn sigwait(set: *const u32, sig: *mut c_int) -> c_int;
}

const PROT_RW: c_int = 3;
const O_NONBLOCK: c_int = 4;
const SIGHUP: c_int = 1;
const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;
const SIG_BLOCK: c_int = 1;
/// `TIMERINTVL`.
const TICK_SECS: u64 = 30;
const MAP_SHARED: c_int = 1;
const S_IFCHR: u32 = 0o020000;
/// The area of a ring.
const CAPACITY: usize = 60 * 1024;
const RING_DIR: &str = "/dev/shmem/logring";
const CONF: &str = "/etc/syslog.conf";
/// FreeBSD's `_PATH_LOGPID`.
const PID_FILE: &str = "/var/run/syslog.pid";
/// The pulse a sender wakes this server with (`libsys::logsock`).
const PULSE_WAKE: c_int = 1;
const IO_READ: usize = 0;
const IO_CLOSE_OCB: usize = 2;

struct Client {
    path: String,
    map: *mut u8,
    ring: Ring,
    reader: Reader,
    /// Its process closed it; it goes once read.
    done: bool,
    /// Left by an earlier `syslogd`: it goes when its process has a new
    /// ring or has ended.
    adopted: bool,
    /// The connection's OCB.
    ocb: usize,
}

// SAFETY: one thread serves; the mapping is this process's.
unsafe impl Send for Client {}

struct Output {
    rule: Rule,
    file: Option<File>,
    repeat: Repeat,
    /// `@host`: resolved at the first message, the network coming late.
    socket: Option<UdpSocket>,
    /// `|command`, started at the first message and again if it ends.
    child: Option<Child>,
}

struct State {
    clients: Vec<Client>,
    outputs: Vec<Output>,
    next: u64,
    /// Seconds between marks (0: none), and since the last.
    mark_interval: u64,
    mark_clock: u64,
    conf: String,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// `LocalHostName`: the system's name, as `gethostname()` says at start.
fn host() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let mut b = [0u8; 256];
        // SAFETY: a buffer of that size, the last byte kept for the NUL.
        if unsafe { gethostname(b.as_mut_ptr().cast(), b.len() - 1) } != 0 {
            return "localhost".into();
        }
        let end = b.iter().position(|&c| c == 0).unwrap_or(0);
        // The first label, as FreeBSD trims the domain.
        String::from_utf8_lossy(&b[..end]).split('.').next().unwrap_or("").to_string()
    })
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn write_line(out: &mut Output, console: &mut Option<File>, line: &str) {
    let file = match out.rule.action {
        Action::Console | Action::Everyone => console.as_mut(),
        _ => out.file.as_mut(),
    };
    if let Some(f) = file {
        let _ = f.write_all(line.as_bytes());
        if matches!(out.rule.action, Action::File { sync: true, .. }) {
            let _ = f.sync_data();
        }
    }
}

/// To a program's standard input, as `p_open()`: one that ended is
/// started again for the next line.
fn pipe_line(out: &mut Output, command: &str, line: &str) {
    if out.child.as_mut().is_some_and(|c| c.try_wait().map_or(true, |s| s.is_some())) {
        out.child = None;
    }
    if out.child.is_none() {
        out.child = Command::new("/bin/sh").arg("-c").arg(command).stdin(Stdio::piped()).spawn().ok();
    }
    let sent = out.child.as_mut().and_then(|c| c.stdin.as_mut()).map(|i| i.write_all(line.as_bytes()));
    if !matches!(sent, Some(Ok(()))) {
        out.child = None;
    }
}

/// To another `syslogd`, a datagram as `fprintlog_write()`.
fn forward(out: &mut Output, host: &str, port: u16, datagram: &str) {
    if out.socket.is_none() {
        out.socket = UdpSocket::bind(("0.0.0.0", 0)).ok().filter(|s| s.connect((host, port)).is_ok());
    }
    if let Some(s) = &out.socket
        && s.send(datagram.as_bytes()).is_err()
    {
        out.socket = None;
    }
}

/// A line to an output, whatever kind.
fn emit(out: &mut Output, console: &mut Option<File>, line: &str, datagram: &str) {
    match out.rule.action.clone() {
        Action::Pipe(command) => pipe_line(out, &command, line),
        Action::Forward { host, port } => forward(out, &host, port, datagram),
        _ => write_line(out, console, line),
    }
}

/// A line of `syslogd`'s own, as the time and host of now.
fn own_line(text: &str, t: u64) -> String {
    format!("{} {} {text}\n", message::format_time(t), host())
}

/// `evaluate_prop_filter()`'s regular expressions, libc's. A pattern
/// that doesn't compile matches nothing.
fn regex_matches(pattern: &str, extended: bool, insensitive: bool, subject: &str) -> bool {
    const REG_EXTENDED: c_int = 1;
    const REG_ICASE: c_int = 2;
    const REG_NOSUB: c_int = 4;
    let (Ok(pattern), Ok(subject)) = (CString::new(pattern), CString::new(subject)) else {
        return false;
    };
    let mut flags = REG_NOSUB;
    if extended {
        flags |= REG_EXTENDED;
    }
    if insensitive {
        flags |= REG_ICASE;
    }
    // `regex_t`: more room than FreeBSD's needs.
    let mut re = [0u64; 16];
    // SAFETY: strings and room for a `regex_t`.
    if unsafe { regcomp(re.as_mut_ptr().cast(), pattern.as_ptr(), flags) } != 0 {
        return false;
    }
    // SAFETY: compiled above; no match offsets are asked for.
    let hit = unsafe { regexec(re.as_ptr().cast(), subject.as_ptr(), 0, null_mut(), 0) } == 0;
    // SAFETY: compiled above.
    unsafe { regfree(re.as_mut_ptr().cast()) };
    hit
}

/// One message to the rules, as `logmsg()`.
fn deliver(outputs: &mut [Output], raw: &[u8], console: &mut Option<File>) {
    let t = now();
    route(outputs, &message::parse(raw, t), t, console);
}

fn route(outputs: &mut [Output], m: &message::Message, t: u64, console: &mut Option<File>) {
    let line = message::file_line(m, host());
    let datagram = message::forward_line(m, host());
    let selected = conf::Selected {
        facility: m.facility,
        level: m.level,
        program: &m.program,
        host: host(),
        text: &m.body,
    };
    for out in outputs.iter_mut().filter(|o| o.rule.accepts(&selected, &regex_matches)) {
        let writes = out.repeat.message(&m.text, t);
        if let Some(count) = writes.count {
            let own = own_line(&count, t);
            emit(out, console, &own, own.trim_end());
        }
        if writes.line {
            emit(out, console, &line, &datagram);
        }
    }
}

/// The timer, as `markit()`: `-- MARK --` lines, and counts held too long.
fn tick(st: &mut State, console: &mut Option<File>) {
    let t = now();
    st.mark_clock += TICK_SECS;
    let mark = st.mark_interval != 0 && st.mark_clock >= st.mark_interval;
    if mark {
        st.mark_clock = 0;
    }
    let interval = st.mark_interval;
    for out in &mut st.outputs {
        if mark && out.rule.matches(conf::MARK, 6) && out.repeat.takes_mark(t, interval) {
            let own = own_line("-- MARK --", t);
            emit(out, console, &own, own.trim_end());
            out.repeat.written = t;
        }
        if let Some(count) = out.repeat.tick(t) {
            let own = own_line(&count, t);
            emit(out, console, &own, own.trim_end());
        }
    }
}

/// What is held comes out, before the files close.
fn flush_counts(st: &mut State, console: &mut Option<File>) {
    let t = now();
    for out in &mut st.outputs {
        if let Some(count) = out.repeat.finish(t) {
            let own = own_line(&count, t);
            emit(out, console, &own, own.trim_end());
        }
    }
}

/// Reads every ring dry, and goes to sleep on them.
fn drain(st: &mut State, console: &mut Option<File>) {
    let mut buf = vec![0u8; logring::MAX_MESSAGE];
    loop {
        let mut any = false;
        for c in &mut st.clients {
            loop {
                match c.reader.read(&c.ring, &mut buf) {
                    Read::Message { len, .. } => {
                        any = true;
                        deliver(&mut st.outputs, &buf[..len], console);
                    }
                    Read::Lost => {
                        let text = format!("<46>syslogd: pid {}: messages lost", c.ring.pid());
                        deliver(&mut st.outputs, text.as_bytes(), console);
                    }
                    Read::Empty => break,
                }
            }
        }
        if !any && st.clients.iter().all(|c| c.ring.prepare_sleep(&c.reader)) {
            break;
        }
    }
    st.clients.retain_mut(|c| {
        // SAFETY: no signal is sent; it asks if the process exists.
        if c.adopted && unsafe { kill(c.ring.pid() as c_int, 0) } != 0 {
            c.done = true;
        }
        if c.done {
            // SAFETY: mapped in `register`, and read for the last time.
            unsafe { munmap(c.map.cast(), logring::mapped_size(CAPACITY)) };
            let _ = fs::remove_file(&c.path);
        }
        !c.done
    });
}

/// A new ring, named for the process.
fn new_ring(st: &mut State, pid: u32, uid: u32, gid: u32, ocb: usize) -> Option<String> {
    for c in st.clients.iter_mut().filter(|c| c.adopted && c.ring.pid() == pid) {
        c.done = true;
    }
    let mut path = format!("{RING_DIR}/{pid}.{}", st.next);
    while fs::exists(&path).unwrap_or(false) {
        st.next += 1;
        path = format!("{RING_DIR}/{pid}.{}", st.next);
    }
    st.next += 1;
    // The client's alone: another user can't touch its messages.
    let f = OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path).ok()?;
    fchown(&f, Some(uid), Some(gid)).ok()?;
    let size = logring::mapped_size(CAPACITY);
    f.set_len(size as u64).ok()?;
    // SAFETY: a new mapping of the file just sized.
    let map = unsafe { mmap(null_mut(), size, PROT_RW, MAP_SHARED, f.as_raw_fd(), 0) }.cast::<u8>();
    if map as isize == -1 {
        return None;
    }
    // SAFETY: page-aligned, zero and that big; it stays mapped.
    let ring = unsafe { Ring::init(map, CAPACITY, pid) };
    let reader = ring.reader();
    // Asleep from the start: the first message must wake this server.
    ring.prepare_sleep(&reader);
    st.clients.push(Client { path: path.clone(), map, ring, reader, done: false, adopted: false, ocb });
    Some(path)
}

/// `read()`: a new client's registration.
unsafe extern "C" fn io_read(ctp: *mut Context, _msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: the context is the framework's.
    let (rcvid, pid, scoid) = unsafe { ((*ctp).rcvid, (*ctp).info.pid, (*ctp).info.scoid) };
    let mut who = ClientInfo::default();
    if ConnectClientInfo(scoid, (&raw mut who).cast(), 0) < 0 {
        return kercall::tls::get_errno();
    }
    let mut g = STATE.lock().unwrap();
    let st = g.as_mut().expect("started");
    let Some(path) = new_ring(st, pid as u32, who.euid, who.egid, ocb as usize) else {
        return 12; // ENOMEM
    };
    let mut console = console();
    drain(st, &mut console);
    let reply = format!("{path} {}", logring::mapped_size(CAPACITY));
    MsgReply(rcvid, reply.len() as c_long, reply.as_ptr().cast(), reply.len());
    RESMGR_NOREPLY
}

/// The last close: the process is done with its ring.
unsafe extern "C" fn io_close_ocb(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    {
        let mut g = STATE.lock().unwrap();
        let st = g.as_mut().expect("started");
        for c in st.clients.iter_mut().filter(|c| c.ocb == ocb as usize) {
            c.done = true;
        }
        let mut console = console();
        drain(st, &mut console);
    }
    // SAFETY: the framework's default with its own arguments.
    unsafe { iofunc_close_ocb_default(ctp, msg, ocb.cast()) }
}

/// A sender's wake-up.
unsafe extern "C" fn wake(_ctp: *mut Context, _type: c_int, _flags: u32, _handle: *mut c_void) -> c_int {
    let mut g = STATE.lock().unwrap();
    let st = g.as_mut().expect("started");
    let mut console = console();
    drain(st, &mut console);
    0
}

fn console() -> Option<File> {
    OpenOptions::new().write(true).open("/dev/console").ok()
}

/// An `include` directory's `*.conf` files, in order, as `configfiles()`.
fn included(dir: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<_> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".conf") && !n.starts_with('.'))
        .collect();
    names.sort();
    names.iter().filter_map(|n| fs::read_to_string(format!("{dir}/{n}")).ok()).collect()
}

/// `init()`: reads `syslog.conf` and opens the files.
fn read_conf(path: &str) -> Vec<Output> {
    let text = fs::read_to_string(path).unwrap_or_else(|_| "*.err;kern.*;auth.notice /dev/console\n".into());
    let (rules, errors) = conf::parse_full(&text, host(), &included);
    for e in errors {
        eprintln!("syslogd: {path}: {e}");
    }
    let mut outputs = Vec::new();
    for rule in rules {
        let file = match &rule.action {
            Action::File { path, .. } => {
                if let Some(dir) = std::path::Path::new(path).parent() {
                    let _ = fs::create_dir_all(dir);
                }
                let f = OpenOptions::new().append(true).create(true).open(path);
                if let Err(e) = &f {
                    eprintln!("syslogd: {path}: {e}");
                }
                f.ok()
            }
            Action::Console | Action::Forward { .. } | Action::Pipe(_) | Action::Everyone => None,
            Action::Unsupported(a) => {
                eprintln!("syslogd: action not served yet: {a}");
                None
            }
        };
        outputs.push(Output { rule, file, repeat: Repeat::default(), socket: None, child: None });
    }
    outputs
}

/// `-f conf` and `-m minutes`, FreeBSD's flags that apply here.
fn arguments() -> (String, u64) {
    let (mut conf, mut mark) = (CONF.to_string(), 20 * 60);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-f" => conf = args.next().unwrap_or(conf),
            "-m" => mark = args.next().and_then(|m| m.parse::<u64>().ok()).map_or(mark, |m| m * 60),
            _ => eprintln!("syslogd: unknown argument {a}"),
        }
    }
    (conf, mark)
}

/// `/dev/klog`, once a second as FreeBSD's `log_wakeups_per_second`: a
/// kernel message can come from anywhere, so nothing wakes this.
fn read_klog() {
    let Ok(mut klog) = OpenOptions::new().read(true).custom_flags(O_NONBLOCK).open("/dev/klog") else {
        return;
    };
    let mut partial = Vec::new();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        while let Ok(n @ 1..) = klog.read(&mut buf) {
            partial.extend_from_slice(&buf[..n]);
            while let Some(end) = partial.iter().position(|&c| c == b'\n') {
                let line: Vec<u8> = partial.drain(..=end).collect();
                let t = now();
                let m = message::parse_kernel(&line[..end], t);
                let mut g = STATE.lock().unwrap();
                let mut console = None;
                route(&mut g.as_mut().expect("started").outputs, &m, t, &mut console);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// The timer's thread, the one that takes `SIGHUP` and `SIGTERM`, and the
/// one that reads the kernel's messages.
fn start_helpers() {
    let mut set = [0u32; 4];
    // SAFETY: the set is on this stack; signals are blocked before any
    // thread starts, so `sigwait()` gets them.
    unsafe {
        sigemptyset(set.as_mut_ptr());
        sigaddset(set.as_mut_ptr(), SIGHUP);
        sigaddset(set.as_mut_ptr(), SIGINT);
        sigaddset(set.as_mut_ptr(), SIGTERM);
        pthread_sigmask(SIG_BLOCK, set.as_ptr(), null_mut());
    }
    std::thread::spawn(read_klog);
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(TICK_SECS));
            let mut g = STATE.lock().unwrap();
            let mut console = console();
            tick(g.as_mut().expect("started"), &mut console);
        }
    });
    std::thread::spawn(move || {
        loop {
            let mut sig: c_int = 0;
            // SAFETY: both are on this stack.
            if unsafe { sigwait(set.as_ptr(), &raw mut sig) } != 0 {
                continue;
            }
            let mut g = STATE.lock().unwrap();
            let st = g.as_mut().expect("started");
            let mut console = console();
            drain(st, &mut console);
            flush_counts(st, &mut console);
            if sig == SIGHUP {
                // `init()`: the file again, and the logs reopened, which
                // is how `newsyslog` rotates them.
                st.outputs = read_conf(&st.conf.clone());
                deliver(&mut st.outputs, b"<46>syslogd: restart", &mut console);
            } else {
                let text = format!("<46>syslogd: exiting on signal {sig}");
                deliver(&mut st.outputs, text.as_bytes(), &mut console);
                let _ = fs::remove_file(PID_FILE);
                std::process::exit(0);
            }
        }
    });
}

/// The rings an earlier `syslogd` left: what is in them isn't lost, and
/// their processes write on until a wake-up fails and they register again.
fn adopt() -> Vec<Client> {
    let mut clients = Vec::new();
    let Ok(dir) = fs::read_dir(RING_DIR) else { return clients };
    for e in dir.flatten() {
        let path = e.path();
        let Ok(f) = OpenOptions::new().read(true).write(true).open(&path) else { continue };
        let size = logring::mapped_size(CAPACITY);
        // SAFETY: a new mapping of a ring's file.
        let map = unsafe { mmap(null_mut(), size, PROT_RW, MAP_SHARED, f.as_raw_fd(), 0) }.cast::<u8>();
        if map as isize == -1 {
            continue;
        }
        // SAFETY: page-aligned, and it stays mapped.
        let Some(ring) = (unsafe { Ring::attach(map, size) }) else {
            // SAFETY: mapped above.
            unsafe { munmap(map.cast(), size) };
            let _ = fs::remove_file(&path);
            continue;
        };
        let path = path.display().to_string();
        let reader = ring.reader();
        clients.push(Client { path, map, ring, reader, done: false, adopted: true, ocb: 0 });
    }
    clients
}

// SAFETY: all zeroes is a valid start for these; `main` fills them.
static mut CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
// SAFETY: as above.
static mut IO: IoFuncs = unsafe { core::mem::zeroed() };
// SAFETY: as above.
static mut ATTR: IofuncAttr = unsafe { core::mem::zeroed() };

fn main() {
    let _ = fs::create_dir_all(RING_DIR);
    let _ = fs::create_dir_all("/var/run");
    let _ = fs::write(PID_FILE, format!("{}\n", std::process::id()));
    let (conf_path, mark_interval) = arguments();
    let outputs = read_conf(&conf_path);
    let mut state = State { clients: adopt(), outputs, next: 0, mark_interval, mark_clock: 0, conf: conf_path };
    let mut console_file = console();
    drain(&mut state, &mut console_file);
    *STATE.lock().unwrap() = Some(state);
    start_helpers();
    // SAFETY: the framework's functions, with statics of this program.
    unsafe {
        let dpp = dispatch_create();
        if dpp.is_null() {
            eprintln!("syslogd: dispatch_create failed");
            std::process::exit(1);
        }
        iofunc_func_init(9, &raw mut CONNECT, 33, &raw mut IO);
        (*(&raw mut IO)).funcs[IO_READ] = Some(io_read);
        (*(&raw mut IO)).funcs[IO_CLOSE_OCB] = Some(io_close_ocb);
        iofunc_attr_init(&raw mut ATTR, S_IFCHR | 0o666, null(), null());
        let attr = ResmgrAttr { flags: 0, nparts_max: 1, msg_max_size: 4096, other_func: None, reserved: [0; 4] };
        pulse_attach(dpp, 0, PULSE_WAKE, Some(wake), null_mut());
        let path = c"/var/run/log".as_ptr().cast::<c_char>();
        if resmgr_attach(dpp, &attr, path, 0, 0, &raw const CONNECT, &raw const IO, (&raw mut ATTR).cast()) < 0 {
            eprintln!("syslogd: can't attach /var/run/log: {}", std::io::Error::last_os_error());
            std::process::exit(1);
        }
        let ctp = dispatch_context_alloc(dpp);
        loop {
            if dispatch_block(ctp).is_null() {
                eprintln!("syslogd: {}", std::io::Error::last_os_error());
                std::process::exit(1);
            }
            dispatch_handler(ctp);
        }
    }
}
