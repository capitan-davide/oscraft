//! `AF_UNIX`: `socketpair()` and named sockets, with descriptors passed as
//! `dupfd` handles, opaque here. FreeBSD's `uipc_usrreq.c` stands on vnodes
//! and its file table, which `netd` hasn't got, so this domain is its own.
//!
//! A path never comes here, as in `io-sock` (`uipc_bindat`, `verify_handle`,
//! read with REA): the client makes or opens the socket's node in a file
//! system and sends a handle to it, and a socket is known by the node, so
//! permissions, `rename()` and `unlink()` are the file system's. A name
//! starting with NUL has no node, and is kept here as it is.

use core::ffi::c_int;
use std::collections::VecDeque;
use std::os::fd::FromRawFd;
use std::os::freebsd::fs::MetadataExt;
use std::sync::{Arc, Condvar, Mutex, Weak};

const EWOULDBLOCK: c_int = 35;
const EPIPE: c_int = 32;
const EINTR: c_int = 4;
const ENOTCONN: c_int = 57;
const EMSGSIZE: c_int = 40;
const EOPNOTSUPP: c_int = 45;
const EINVAL: c_int = 22;
const EISCONN: c_int = 56;
const EADDRINUSE: c_int = 48;
const ECONNREFUSED: c_int = 61;
const S_IFMT: u32 = 0o170000;
const S_IFSOCK: u32 = 0o140000;
const MSG_PEEK: c_int = 2;
const MSG_DONTWAIT: c_int = 0x80;
/// The most a side holds, as FreeBSD's `net.local.stream.sendspace`.
const ROOM: usize = 64 * 1024;
const SOCK_STREAM: c_int = 1;

/// What one send left: bytes, and the descriptors with them.
struct Packet {
    data: Vec<u8>,
    at: usize,
    rights: Vec<Right>,
}

/// A descriptor on its way: `netd` holds a copy of it, which it registers
/// for the receiver when that one looks. `fd` closes when it is delivered.
pub struct Right {
    pub fd: c_int,
    pub handle: u64,
}

impl Drop for Right {
    fn drop(&mut self) {
        crate::rights::close(self.fd);
    }
}

/// What a socket is called.
#[derive(Clone, PartialEq)]
pub enum Name {
    /// The node's device and inode.
    Node(u64, u64),
    /// A name with no node.
    Abstract(Vec<u8>),
}

impl Name {
    /// From a client's address: `{ len, AF_UNIX, 1, handle }`, as `libsocket`
    /// and `libsys` send it, or `{ len, AF_UNIX, 0, name... }`.
    pub fn parse(address: &[u8]) -> Result<Name, c_int> {
        let len = usize::from(*address.first().ok_or(EINVAL)?).min(address.len());
        if len < 3 || address[1] != 1 {
            return Err(EINVAL);
        }
        if address[2] == 0 {
            return Ok(Name::Abstract(address[2..len].to_vec()));
        }
        let handle = address.get(3..11).ok_or(EINVAL)?;
        let fd = crate::rights::import(u64::from_le_bytes(handle.try_into().unwrap()))?;
        // SAFETY: the copy `import()` just made, this `File`'s alone.
        let node = unsafe { std::fs::File::from_raw_fd(fd) };
        let stat = node.metadata().map_err(|e| e.raw_os_error().unwrap_or(EINVAL))?;
        if stat.st_mode() as u32 & S_IFMT != S_IFSOCK {
            return Err(ECONNREFUSED);
        }
        Ok(Name::Node(stat.st_dev(), stat.st_ino()))
    }
}

/// The bound sockets. A closed one's entry is dead, and is replaced.
static NAMES: Mutex<Vec<(Name, Weak<Unix>)>> = Mutex::new(Vec::new());

#[derive(Default)]
struct Side {
    queue: VecDeque<Packet>,
    held: usize,
    /// This side reads no more, or writes no more.
    no_read: bool,
    no_write: bool,
    nonblock: bool,
    /// A client waiting here wants out.
    interrupted: bool,
    /// Listening: the most waiting, and the ends for `accept()`.
    limit: Option<usize>,
    waiting: VecDeque<Arc<Unix>>,
    bound: bool,
}

/// One end of a pair.
pub struct Unix {
    pub kind: c_int,
    side: Mutex<Side>,
    changed: Condvar,
    peer: Mutex<Weak<Unix>>,
}

/// What a read gives.
pub struct Got {
    pub data: Vec<u8>,
    pub handles: Vec<u64>,
}

impl Unix {
    pub fn new(kind: c_int) -> Arc<Unix> {
        Arc::new(Unix { kind, side: Mutex::default(), changed: Condvar::new(), peer: Mutex::new(Weak::new()) })
    }

    /// Joins a pair.
    pub fn pair(a: &Arc<Unix>, b: &Arc<Unix>) {
        *a.peer.lock().unwrap() = Arc::downgrade(b);
        *b.peer.lock().unwrap() = Arc::downgrade(a);
    }

    fn peer(&self) -> Option<Arc<Unix>> {
        self.peer.lock().unwrap().upgrade()
    }

    pub fn set_nonblock(&self, on: bool) {
        self.side.lock().unwrap().nonblock = on;
    }

    /// `bind()`.
    pub fn bind(self: &Arc<Unix>, name: Name) -> Result<(), c_int> {
        let mut names = NAMES.lock().unwrap();
        names.retain(|n| n.1.strong_count() != 0);
        if names.iter().any(|n| n.0 == name) {
            return Err(EADDRINUSE);
        }
        let mut side = self.side.lock().unwrap();
        if side.bound {
            return Err(EINVAL);
        }
        side.bound = true;
        names.push((name, Arc::downgrade(self)));
        Ok(())
    }

    /// `listen()`.
    pub fn listen(&self, backlog: c_int) -> Result<(), c_int> {
        let mut side = self.side.lock().unwrap();
        if self.kind != SOCK_STREAM {
            return Err(EOPNOTSUPP);
        }
        if !side.bound {
            return Err(EINVAL);
        }
        side.limit = Some(backlog.max(1) as usize);
        Ok(())
    }

    /// `connect()`: a stream's end for its listener's `accept()`, or a
    /// datagram socket's destination.
    pub fn connect(self: &Arc<Unix>, name: &Name) -> Result<(), c_int> {
        let found = NAMES.lock().unwrap().iter().find(|n| n.0 == *name).and_then(|n| n.1.upgrade());
        let target = found.ok_or(ECONNREFUSED)?;
        if target.kind != self.kind {
            return Err(ECONNREFUSED);
        }
        if self.kind != SOCK_STREAM {
            *self.peer.lock().unwrap() = Arc::downgrade(&target);
            return Ok(());
        }
        if self.peer().is_some() {
            return Err(EISCONN);
        }
        let mut side = target.side.lock().unwrap();
        let limit = side.limit.ok_or(ECONNREFUSED)?;
        if side.waiting.len() >= limit {
            return Err(ECONNREFUSED);
        }
        let end = Unix::new(self.kind);
        Unix::pair(self, &end);
        side.waiting.push_back(end);
        drop(side);
        target.changed.notify_all();
        crate::server::wake(&target, false);
        Ok(())
    }

    /// `accept()`: the next end.
    pub fn accept(&self) -> Result<Arc<Unix>, c_int> {
        let mut side = self.side.lock().unwrap();
        if side.limit.is_none() {
            return Err(EINVAL);
        }
        loop {
            if let Some(end) = side.waiting.pop_front() {
                return Ok(end);
            }
            if side.nonblock {
                return Err(EWOULDBLOCK);
            }
            if core::mem::take(&mut side.interrupted) {
                return Err(EINTR);
            }
            side = self.changed.wait(side).unwrap();
        }
    }

    /// `send()`.
    pub fn send(&self, data: &[u8], rights: Vec<Right>, flags: c_int) -> Result<usize, c_int> {
        if self.side.lock().unwrap().no_write {
            return Err(EPIPE);
        }
        let nonblock = self.side.lock().unwrap().nonblock || flags & MSG_DONTWAIT != 0;
        let Some(peer) = self.peer() else {
            return Err(if self.kind == SOCK_STREAM { EPIPE } else { ENOTCONN });
        };
        if self.kind != SOCK_STREAM && data.len() > ROOM {
            return Err(EMSGSIZE);
        }
        let mut sent = 0;
        let mut rights = Some(rights);
        loop {
            let mut side = peer.side.lock().unwrap();
            while side.held >= ROOM && !side.no_read {
                if nonblock {
                    return if sent == 0 { Err(EWOULDBLOCK) } else { Ok(sent) };
                }
                if core::mem::take(&mut side.interrupted) {
                    return if sent == 0 { Err(EINTR) } else { Ok(sent) };
                }
                side = peer.changed.wait(side).unwrap();
            }
            if side.no_read {
                return if sent == 0 { Err(EPIPE) } else { Ok(sent) };
            }
            let n = (data.len() - sent).min(ROOM - side.held.min(ROOM));
            side.queue.push_back(Packet {
                data: data[sent..sent + n].to_vec(),
                at: 0,
                rights: rights.take().unwrap_or_default(),
            });
            side.held += n;
            sent += n;
            peer.changed.notify_all();
            drop(side);
            crate::server::wake(&peer, false);
            if sent == data.len() {
                return Ok(sent);
            }
        }
    }

    /// `recv()`.
    pub fn recv(&self, room: usize, flags: c_int, pid: c_int) -> Result<Got, c_int> {
        let peek = flags & MSG_PEEK != 0;
        let mut side = self.side.lock().unwrap();
        let nonblock = side.nonblock || flags & MSG_DONTWAIT != 0;
        while side.queue.is_empty() {
            if side.no_read || (self.kind == SOCK_STREAM && self.peer().is_none()) {
                return Ok(Got { data: Vec::new(), handles: Vec::new() });
            }
            if nonblock {
                return Err(EWOULDBLOCK);
            }
            if core::mem::take(&mut side.interrupted) {
                return Err(EINTR);
            }
            side = self.changed.wait(side).unwrap();
        }
        let mut data = Vec::new();
        let mut handles = Vec::new();
        let stream = self.kind == SOCK_STREAM;
        let mut i = 0;
        while data.len() < room && i < side.queue.len() {
            let p = &mut side.queue[i];
            if !p.rights.is_empty() && !data.is_empty() {
                break;
            }
            let n = (p.data.len() - p.at).min(room - data.len());
            data.extend_from_slice(&p.data[p.at..p.at + n]);
            for r in &mut p.rights {
                if r.handle == 0 {
                    r.handle = crate::rights::register(pid, r.fd);
                }
                handles.push(r.handle);
            }
            if peek {
                i += 1;
            } else {
                p.at += n;
                p.rights.clear();
                if p.at == p.data.len() || !stream {
                    let left = p.data.len() - p.at + n;
                    side.queue.remove(i);
                    side.held -= left.min(side.held);
                } else {
                    side.held -= n;
                }
            }
            if !stream || !handles.is_empty() {
                break;
            }
        }
        drop(side);
        if !peek {
            self.changed.notify_all();
            if let Some(peer) = self.peer() {
                crate::server::wake(&peer, true);
            }
        }
        Ok(Got { data, handles })
    }

    /// A client waiting on this end wants out.
    pub fn interrupt(&self) {
        self.side.lock().unwrap().interrupted = true;
        self.changed.notify_all();
        if let Some(p) = self.peer() {
            p.side.lock().unwrap().interrupted = true;
            p.changed.notify_all();
        }
    }

    /// What `poll()` finds: readable, writable.
    pub fn ready(&self) -> (bool, bool) {
        let mine = self.side.lock().unwrap();
        if mine.limit.is_some() {
            return (!mine.waiting.is_empty(), false);
        }
        let readable = !mine.queue.is_empty() || mine.no_read;
        drop(mine);
        let writable = match self.peer() {
            Some(p) => {
                let s = p.side.lock().unwrap();
                s.held < ROOM || s.no_read
            }
            None => true,
        };
        (readable || (self.kind == SOCK_STREAM && self.peer().is_none()), writable)
    }

    /// `shutdown()`: how 0, 1 or 2.
    pub fn shutdown(&self, how: c_int) {
        let mut mine = self.side.lock().unwrap();
        if how != 1 {
            mine.no_read = true;
        }
        if how != 0 {
            mine.no_write = true;
        }
        drop(mine);
        self.changed.notify_all();
        if let Some(p) = self.peer() {
            if how != 0 {
                p.side.lock().unwrap().no_read = true;
            }
            p.changed.notify_all();
            crate::server::wake(&p, false);
            crate::server::wake(&p, true);
        }
    }
}
