//! `fsd`: a file system's server, QNX's shape. A file system is a process
//! that attaches a path and answers the messages under it; this is the
//! part every such server has, generic over [`FileSystem`]: open files,
//! reads and writes, directories, `stat()`, the owner, mode and time
//! rules, `rename()`, `unlink()`, `mknod()`, and `statfs()`. `diskd` serves
//! FAT with it, `tmpfs` memory. An open file is a path, looked up again
//! on each operation, so a file system needs no notion of an open file.

use core::ffi::c_int;

#[cfg(target_os = "freebsd")]
mod server;

#[cfg(target_os = "freebsd")]
pub use server::{attach, detach, dispatch, serve, set_unmount};

/// FreeBSD's error numbers.
pub type Result<T> = core::result::Result<T, c_int>;

/// The file types, as `st_mode`'s.
pub const S_IFMT: u32 = 0o170000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFSOCK: u32 = 0o140000;
pub const S_ISVTX: u32 = 0o1000;

/// What `stat()` says of a file.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Node {
    pub ino: u64,
    /// The type and the permissions.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub size: u64,
    /// Seconds since the Epoch.
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }
}

/// Who asks: the effective IDs and the groups.
#[derive(Clone, Debug, Default)]
pub struct Who {
    pub uid: u32,
    pub gid: u32,
    pub groups: Vec<u32>,
}

impl Who {
    pub fn is_root(&self) -> bool {
        self.uid == 0
    }

    pub fn in_group(&self, gid: u32) -> bool {
        self.gid == gid || self.groups.contains(&gid)
    }

    /// May read (4), write (2) or execute (1) `node`, as `vaccess()`.
    pub fn may(&self, node: &Node, want: u32) -> bool {
        if self.is_root() {
            // Root executes only what someone may.
            return want & 1 == 0 || node.is_dir() || node.mode & 0o111 != 0;
        }
        let bits = if self.uid == node.uid {
            node.mode >> 6
        } else if self.in_group(node.gid) {
            node.mode >> 3
        } else {
            node.mode
        };
        bits & want & 7 == want & 7
    }
}

/// A file system's size: `statfs()`'s.
#[derive(Clone, Copy, Debug, Default)]
pub struct Space {
    pub block_size: u32,
    pub blocks: u64,
    pub free: u64,
    pub files: u64,
    pub free_files: u64,
}

// ANCHOR: file_system
/// A file system, by path: `"/"` is its root, `"/a/b"` below it.
pub trait FileSystem: Send + core::any::Any {
    /// Its type, as `statfs()` names it: `"msdosfs"`, `"tmpfs"`.
    fn kind(&self) -> &'static str;
    fn lookup(&mut self, path: &str) -> Result<Node>;
    /// Makes `path` of `mode` (its type in it: a regular file, a directory,
    /// a socket, ...), owned by `who`.
    fn make(&mut self, path: &str, mode: u32, who: &Who, now: i64) -> Result<Node>;
    fn read(&mut self, path: &str, at: u64, buf: &mut [u8]) -> Result<usize>;
    /// Writes at `at`, or at the end: where it went, and the new size.
    fn write(&mut self, path: &str, at: Option<u64>, data: &[u8], now: i64) -> Result<(u64, u64)>;
    /// The new size.
    fn truncate(&mut self, path: &str, size: u64, now: i64) -> Result<u64>;
    /// A directory's entries, without `.` and `..`.
    fn list(&mut self, path: &str) -> Result<Vec<(String, Node)>>;
    /// `unlink()`, or `rmdir()` if `dir`.
    fn remove(&mut self, path: &str, dir: bool, now: i64) -> Result<()>;
    fn rename(&mut self, from: &str, to: &str, now: i64) -> Result<()>;
    /// `chmod()`: the permission bits, the caller's rights checked.
    fn set_mode(&mut self, path: &str, mode: u32, who: &Who, now: i64) -> Result<()>;
    /// `chown()`: `u32::MAX` leaves an ID as it is.
    fn set_owner(&mut self, path: &str, uid: u32, gid: u32, who: &Who, now: i64) -> Result<()>;
    /// `utimes()`: `None` leaves a time; `now` says the client gave none.
    fn set_times(&mut self, path: &str, atime: Option<i64>, mtime: Option<i64>, who: &Who, to_now: bool) -> Result<()>;
    fn space(&mut self) -> Result<Space>;
    fn sync(&mut self) -> Result<()> {
        Ok(())
    }
}
// ANCHOR_END: file_system

/// POSIX's rules for `chmod()`, `chown()` and `utimes()`, as FreeBSD's
/// `setattr` functions apply them (`ufs_chmod()`, `ufs_chown()`,
/// `vn_utimes_perm()`): for a file system that keeps owners.
pub mod rules {
    use super::{Node, Result, S_ISVTX, Who};
    use proto::errno as e;

    /// The new mode's permission bits, or why not: the owner or root; a
    /// non-root loses the sticky bit on a file and set-group-ID outside
    /// its groups.
    pub fn chmod(node: &Node, mode: u32, who: &Who) -> Result<u32> {
        if !who.is_root() && who.uid != node.uid {
            return Err(e::EPERM);
        }
        let mut m = mode & 0o7777;
        if !who.is_root() {
            if !node.is_dir() && m & S_ISVTX != 0 {
                return Err(e::EFTYPE);
            }
            if !who.in_group(node.gid) && m & 0o2000 != 0 {
                return Err(e::EPERM);
            }
            m &= 0o7777;
        }
        Ok(m)
    }

    /// The new owner and group: root anything; the owner only its own
    /// group, to one of its groups.
    pub fn chown(node: &Node, uid: u32, gid: u32, who: &Who) -> Result<(u32, u32)> {
        let uid = if uid == u32::MAX { node.uid } else { uid };
        let gid = if gid == u32::MAX { node.gid } else { gid };
        if !who.is_root() && (uid != node.uid || who.uid != node.uid || (gid != node.gid && !who.in_group(gid))) {
            return Err(e::EPERM);
        }
        Ok((uid, gid))
    }

    /// May set the times: the owner or root, or, to now, whoever may write.
    pub fn utimes(node: &Node, who: &Who, to_now: bool) -> Result<()> {
        if who.is_root() || who.uid == node.uid || (to_now && who.may(node, 2)) {
            Ok(())
        } else if to_now {
            Err(e::EACCES)
        } else {
            Err(e::EPERM)
        }
    }

    /// The mode bits a new file gets: set-group-ID only for a member.
    pub fn new_mode(mode: u32, gid: u32, who: &Who) -> u32 {
        if !who.is_root() && !who.in_group(gid) { mode & !0o2000 } else { mode }
    }
}
