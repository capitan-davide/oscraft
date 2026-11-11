//! A file system in memory, as FreeBSD's tmpfs (`sys/fs/tmpfs`): every
//! kind of node `fsd` can make (directories, regular files, sockets'
//! nodes), with owners, modes and times, and limits on the bytes and the
//! nodes it may hold. Nothing outlives the process.

use std::collections::BTreeMap;

use fsd::{FileSystem, Node, Result, S_IFDIR, S_IFMT, S_IFREG, S_IFSOCK, Space, Who, rules};
use proto::errno as e;

const NAME_MAX: usize = 255;
const BLOCK: u64 = 4096;

struct Inode {
    node: Node,
    data: Vec<u8>,
    children: BTreeMap<String, u64>,
}

/// One mount's tree.
pub struct Tmpfs {
    nodes: BTreeMap<u64, Inode>,
    next: u64,
    used: u64,
    max_bytes: u64,
    max_nodes: u64,
}

impl Tmpfs {
    /// An empty tree, its root of `mode` owned by `uid`, `gid`; at most
    /// `max_bytes` of data and `max_nodes` nodes.
    pub fn new(mode: u32, uid: u32, gid: u32, max_bytes: u64, max_nodes: u64, now: i64) -> Tmpfs {
        let root = Node { ino: 2, mode: S_IFDIR | (mode & 0o7777), uid, gid, nlink: 2, size: 0, atime: now, mtime: now, ctime: now };
        let mut nodes = BTreeMap::new();
        nodes.insert(2, Inode { node: root, data: Vec::new(), children: BTreeMap::new() });
        Tmpfs { nodes, next: 3, used: 0, max_bytes, max_nodes }
    }

    fn walk(&self, path: &str) -> Result<u64> {
        let mut at = 2;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let dir = &self.nodes[&at];
            if !dir.node.is_dir() {
                return Err(e::ENOTDIR);
            }
            if part.len() > NAME_MAX {
                return Err(e::ENAMETOOLONG);
            }
            at = match part {
                "." => at,
                ".." => return Err(e::EINVAL),
                _ => *dir.children.get(part).ok_or(e::ENOENT)?,
            };
        }
        Ok(at)
    }

    /// The directory a path's last name is in, and the name.
    fn split<'a>(&self, path: &'a str) -> Result<(u64, &'a str)> {
        let path = path.trim_end_matches('/');
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        if name.is_empty() || name == "." || name == ".." {
            return Err(e::EINVAL);
        }
        if name.len() > NAME_MAX {
            return Err(e::ENAMETOOLONG);
        }
        let d = self.walk(dir)?;
        if !self.nodes[&d].node.is_dir() {
            return Err(e::ENOTDIR);
        }
        Ok((d, name))
    }

    fn get(&mut self, path: &str) -> Result<&mut Inode> {
        let ino = self.walk(path)?;
        Ok(self.nodes.get_mut(&ino).unwrap())
    }

    /// A directory's size, as tmpfs counts it: its entries.
    fn dir_size(n: &Inode) -> u64 {
        (n.children.len() as u64 + 2) * 32
    }

    fn touch_dir(&mut self, ino: u64, now: i64) {
        let d = self.nodes.get_mut(&ino).unwrap();
        d.node.size = Self::dir_size(d);
        d.node.mtime = now;
        d.node.ctime = now;
    }

    fn resize(&mut self, path: &str, size: u64, now: i64) -> Result<()> {
        let ino = self.walk(path)?;
        let old = self.nodes[&ino].data.len() as u64;
        if self.nodes[&ino].node.mode & S_IFMT != S_IFREG {
            return Err(if self.nodes[&ino].node.is_dir() { e::EISDIR } else { e::EINVAL });
        }
        if size > old && self.max_bytes != 0 && self.used + (size - old) > self.max_bytes {
            return Err(e::ENOSPC);
        }
        self.used = self.used + size - old;
        let n = self.nodes.get_mut(&ino).unwrap();
        n.data.resize(size as usize, 0);
        n.node.size = size;
        n.node.mtime = now;
        n.node.ctime = now;
        Ok(())
    }
}

impl FileSystem for Tmpfs {
    fn kind(&self) -> &'static str {
        "tmpfs"
    }

    fn lookup(&mut self, path: &str) -> Result<Node> {
        Ok(self.get(path)?.node)
    }

    fn make(&mut self, path: &str, mode: u32, who: &Who, now: i64) -> Result<Node> {
        let (dir, name) = self.split(path)?;
        if self.nodes[&dir].children.contains_key(name) {
            return Err(e::EEXIST);
        }
        if !matches!(mode & S_IFMT, S_IFREG | S_IFDIR | S_IFSOCK) {
            return Err(e::ENOTSUP);
        }
        if self.max_nodes != 0 && self.nodes.len() as u64 >= self.max_nodes {
            return Err(e::ENOSPC);
        }
        // The group is the directory's, as on FreeBSD.
        let gid = self.nodes[&dir].node.gid;
        let ino = self.next;
        self.next += 1;
        let is_dir = mode & S_IFMT == S_IFDIR;
        let perm = rules::new_mode(mode & 0o7777, gid, who);
        let node = Node {
            ino,
            mode: (mode & S_IFMT) | perm,
            uid: who.uid,
            gid,
            nlink: if is_dir { 2 } else { 1 },
            size: if is_dir { 64 } else { 0 },
            atime: now,
            mtime: now,
            ctime: now,
        };
        self.nodes.insert(ino, Inode { node, data: Vec::new(), children: BTreeMap::new() });
        self.nodes.get_mut(&dir).unwrap().children.insert(name.into(), ino);
        if is_dir {
            self.nodes.get_mut(&dir).unwrap().node.nlink += 1;
        }
        self.touch_dir(dir, now);
        Ok(node)
    }

    fn read(&mut self, path: &str, at: u64, buf: &mut [u8]) -> Result<usize> {
        let n = self.get(path)?;
        if n.node.is_dir() {
            return Err(e::EISDIR);
        }
        let len = n.data.len() as u64;
        if at >= len {
            return Ok(0);
        }
        let k = buf.len().min((len - at) as usize);
        buf[..k].copy_from_slice(&n.data[at as usize..at as usize + k]);
        Ok(k)
    }

    fn write(&mut self, path: &str, at: Option<u64>, data: &[u8], now: i64) -> Result<(u64, u64)> {
        let n = self.get(path)?;
        if n.node.is_dir() {
            return Err(e::EISDIR);
        }
        let at = at.unwrap_or(n.data.len() as u64);
        let end = at.checked_add(data.len() as u64).ok_or(e::EFBIG)?;
        if end > n.data.len() as u64 {
            self.resize(path, end, now)?;
        }
        let n = self.get(path)?;
        n.data[at as usize..end as usize].copy_from_slice(data);
        n.node.mtime = now;
        n.node.ctime = now;
        Ok((at, n.data.len() as u64))
    }

    fn truncate(&mut self, path: &str, size: u64, now: i64) -> Result<u64> {
        self.resize(path, size, now)?;
        Ok(size)
    }

    fn list(&mut self, path: &str) -> Result<Vec<(String, Node)>> {
        let ino = self.walk(path)?;
        let d = &self.nodes[&ino];
        if !d.node.is_dir() {
            return Err(e::ENOTDIR);
        }
        Ok(d.children.iter().map(|(k, v)| (k.clone(), self.nodes[v].node)).collect())
    }

    fn remove(&mut self, path: &str, dir: bool, now: i64) -> Result<()> {
        let (parent, name) = self.split(path)?;
        let ino = *self.nodes[&parent].children.get(name).ok_or(e::ENOENT)?;
        let n = &self.nodes[&ino];
        match (dir, n.node.is_dir()) {
            (true, false) => return Err(e::ENOTDIR),
            // unlink() of a directory: `kern_funlinkat()`.
            (false, true) => return Err(e::EPERM),
            (true, true) if !n.children.is_empty() => return Err(e::ENOTEMPTY),
            _ => {}
        }
        self.nodes.get_mut(&parent).unwrap().children.remove(name);
        if dir {
            self.nodes.get_mut(&parent).unwrap().node.nlink -= 1;
        }
        let gone = self.nodes.remove(&ino).unwrap();
        self.used -= gone.data.len() as u64;
        self.touch_dir(parent, now);
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str, now: i64) -> Result<()> {
        let (fdir, fname) = self.split(from)?;
        let ino = *self.nodes[&fdir].children.get(fname).ok_or(e::ENOENT)?;
        let (tdir, tname) = self.split(to)?;
        let moving_dir = self.nodes[&ino].node.is_dir();
        if moving_dir {
            // Not into itself: no ancestor of the target is the directory.
            let mut at = to.trim_end_matches('/').to_string();
            while let Some((up, _)) = at.rsplit_once('/') {
                at = up.to_string();
                if self.walk(&at).ok() == Some(ino) {
                    return Err(e::EINVAL);
                }
            }
        }
        if let Some(&old) = self.nodes[&tdir].children.get(tname) {
            if old == ino {
                return Ok(());
            }
            let o = &self.nodes[&old];
            match (moving_dir, o.node.is_dir()) {
                (true, false) => return Err(e::ENOTDIR),
                (false, true) => return Err(e::EISDIR),
                (true, true) if !o.children.is_empty() => return Err(e::ENOTEMPTY),
                _ => {}
            }
            self.remove(to, moving_dir, now)?;
        }
        self.nodes.get_mut(&fdir).unwrap().children.remove(fname);
        self.nodes.get_mut(&tdir).unwrap().children.insert(tname.into(), ino);
        if moving_dir && fdir != tdir {
            self.nodes.get_mut(&fdir).unwrap().node.nlink -= 1;
            self.nodes.get_mut(&tdir).unwrap().node.nlink += 1;
        }
        self.nodes.get_mut(&ino).unwrap().node.ctime = now;
        self.touch_dir(fdir, now);
        self.touch_dir(tdir, now);
        Ok(())
    }

    fn set_mode(&mut self, path: &str, mode: u32, who: &Who, now: i64) -> Result<()> {
        let n = self.get(path)?;
        let perm = rules::chmod(&n.node, mode, who)?;
        n.node.mode = (n.node.mode & S_IFMT) | perm;
        n.node.ctime = now;
        Ok(())
    }

    fn set_owner(&mut self, path: &str, uid: u32, gid: u32, who: &Who, now: i64) -> Result<()> {
        let n = self.get(path)?;
        let (uid, gid) = rules::chown(&n.node, uid, gid, who)?;
        if (uid, gid) != (n.node.uid, n.node.gid) && !who.is_root() {
            // A change by a non-root clears the set-ID bits.
            n.node.mode &= !0o6000;
        }
        n.node.uid = uid;
        n.node.gid = gid;
        n.node.ctime = now;
        Ok(())
    }

    fn set_times(&mut self, path: &str, atime: Option<i64>, mtime: Option<i64>, who: &Who, to_now: bool) -> Result<()> {
        let n = self.get(path)?;
        rules::utimes(&n.node, who, to_now)?;
        if let Some(t) = atime {
            n.node.atime = t;
        }
        if let Some(t) = mtime {
            n.node.mtime = t;
        }
        Ok(())
    }

    fn space(&mut self) -> Result<Space> {
        let used = self.used.div_ceil(BLOCK) + self.nodes.len() as u64;
        let total = if self.max_bytes != 0 { self.max_bytes / BLOCK } else { used.max(1) * 2 + 65536 };
        let files = if self.max_nodes != 0 { self.max_nodes } else { u64::from(u32::MAX) };
        Ok(Space {
            block_size: BLOCK as u32,
            blocks: total,
            free: total.saturating_sub(used),
            files,
            free_files: files - self.nodes.len() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> Who {
        Who::default()
    }

    #[test]
    fn files_dirs_and_sockets() {
        let mut t = Tmpfs::new(0o1777, 0, 0, 0, 0, 1);
        let me = Who { uid: 100, gid: 100, groups: vec![] };
        t.make("/d", S_IFDIR | 0o755, &root(), 2).unwrap();
        assert_eq!(t.lookup("/").unwrap().nlink, 3);
        t.make("/d/f", S_IFREG | 0o644, &root(), 2).unwrap();
        assert_eq!(t.write("/d/f", Some(3), b"abc", 3).unwrap(), (3, 6));
        let mut b = [9u8; 8];
        assert_eq!(t.read("/d/f", 0, &mut b).unwrap(), 6);
        assert_eq!(&b[..6], b"\0\0\0abc");
        let s = t.make("/s", S_IFSOCK | 0o666, &me, 4).unwrap();
        assert_eq!((s.mode & S_IFMT, s.uid), (S_IFSOCK, 100));
        assert_eq!(t.remove("/d", true, 5), Err(e::ENOTEMPTY));
        assert_eq!(t.remove("/d", false, 5), Err(e::EPERM));
        t.rename("/d/f", "/g", 6).unwrap();
        assert_eq!(t.lookup("/d/f"), Err(e::ENOENT));
        assert_eq!(t.rename("/d", "/d/x", 6), Err(e::EINVAL));
        t.remove("/d", true, 7).unwrap();
        assert_eq!(t.lookup("/").unwrap().nlink, 2);
        assert_eq!(t.make("/g/x", S_IFREG, &root(), 8), Err(e::ENOTDIR));
    }

    #[test]
    fn owners_and_limits() {
        let mut t = Tmpfs::new(0o755, 0, 0, 8192, 4, 1);
        let me = Who { uid: 100, gid: 100, groups: vec![] };
        t.make("/f", S_IFREG | 0o644, &me, 1).unwrap();
        assert_eq!(t.set_mode("/f", 0o600, &Who { uid: 7, gid: 7, groups: vec![] }, 2), Err(e::EPERM));
        t.set_mode("/f", 0o600, &me, 2).unwrap();
        assert_eq!(t.set_owner("/f", 0, u32::MAX, &me, 2), Err(e::EPERM));
        t.set_owner("/f", 7, 7, &root(), 2).unwrap();
        assert_eq!(t.write("/f", Some(0), &[0; 9000], 3), Err(e::ENOSPC));
        t.make("/a", S_IFREG, &me, 3).unwrap();
        t.make("/b", S_IFREG, &me, 3).unwrap();
        assert_eq!(t.make("/c", S_IFREG, &me, 3), Err(e::ENOSPC));
    }
}
