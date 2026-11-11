//! Against real tools: what `mkfs.fat` and `mtools` write, this reads, and
//! what this writes, `fsck.fat` and `mtools` accept.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use bufcache::{BlockDevice, Cache};
use msdosfs::{DosTime, Error, Volume};

struct ImageFile(fs::File, u64);

impl BlockDevice for ImageFile {
    fn block_size(&self) -> usize {
        512
    }
    fn blocks(&self) -> u64 {
        self.1 / 512
    }
    fn read(&mut self, block: u64, buf: &mut [u8]) -> io::Result<()> {
        self.0.seek(SeekFrom::Start(block * 512))?;
        self.0.read_exact(buf)
    }
    fn write(&mut self, block: u64, buf: &[u8]) -> io::Result<()> {
        self.0.seek(SeekFrom::Start(block * 512))?;
        self.0.write_all(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

const T: DosTime = DosTime { date: 0x5b66, time: 0x4800 };

struct Image {
    path: PathBuf,
}

impl Drop for Image {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn run(cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd).args(args).env("MTOOLS_SKIP_CHECK", "1").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{cmd} {args:?}: {text}");
    text
}

fn image(name: &str, bits: u32, mb: u64) -> Image {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("msdosfs-{name}-{}-{n}.img", std::process::id()));
    let f = fs::File::create(&path).unwrap();
    f.set_len(mb << 20).unwrap();
    let p = path.to_str().unwrap();
    run("mkfs.fat", &["-F", &bits.to_string(), "-S", "512", "-s", "1", "-n", "OSCRAFT", p]);
    Image { path }
}

fn open(p: &Path) -> Volume<ImageFile> {
    let f = fs::OpenOptions::new().read(true).write(true).open(p).unwrap();
    let len = f.metadata().unwrap().len();
    Volume::mount(Cache::new(ImageFile(f, len), 256)).unwrap()
}

fn fsck(p: &Path) {
    let text = run("fsck.fat", &["-n", "-v", p.to_str().unwrap()]);
    assert!(!text.contains("Dirty") && !text.contains("Free cluster summary wrong"), "{text}");
}

fn each_kind(test: impl Fn(&Image)) {
    test(&image("a", 12, 2));
    test(&image("b", 16, 16));
    test(&image("c", 32, 64));
}

fn mcopy_in(img: &Image, data: &[u8], name: &str) {
    let host = img.path.with_extension("in");
    fs::write(&host, data).unwrap();
    run("mcopy", &["-i", img.path.to_str().unwrap(), host.to_str().unwrap(), &format!("::{name}")]);
    let _ = fs::remove_file(host);
}

fn mcopy_out(img: &Image, name: &str) -> Vec<u8> {
    let host = img.path.with_extension("out");
    run("mcopy", &["-n", "-i", img.path.to_str().unwrap(), &format!("::{name}"), host.to_str().unwrap()]);
    let d = fs::read(&host).unwrap();
    let _ = fs::remove_file(host);
    d
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

#[test]
fn reads_what_real_tools_wrote() {
    each_kind(|img| {
        let data = pattern(70_000);
        mcopy_in(img, &data, "/SHORT.BIN");
        mcopy_in(img, b"long", "/a rather long File Name.text");
        run("mmd", &["-i", img.path.to_str().unwrap(), "::/sub"]);
        mcopy_in(img, b"inside", "/sub/in.txt");
        let mut v = open(&img.path);
        let e = v.resolve("/short.bin").unwrap();
        assert_eq!(e.size, 70_000);
        let mut buf = vec![0; 70_000];
        assert_eq!(v.read_at(&e, 0, &mut buf).unwrap(), 70_000);
        assert_eq!(buf, data);
        let root = v.root_entry();
        let names: Vec<String> = v.read_dir(&root).unwrap().into_iter().map(|e| e.name).collect();
        assert!(names.iter().any(|n| n == "a rather long File Name.text"), "{names:?}");
        let e = v.resolve("/SUB/IN.TXT").unwrap();
        let mut b = [0; 16];
        assert_eq!(v.read_at(&e, 0, &mut b).unwrap(), 6);
        assert_eq!(&b[..6], b"inside");
        assert_eq!(v.resolve("/nothing").unwrap_err(), Error::NotFound);
    });
}

#[test]
fn what_it_writes_real_tools_accept() {
    each_kind(|img| {
        let mut v = open(&img.path);
        let data = pattern(70_000);
        let mut e = v.create("/data.bin", T).unwrap();
        v.write_at(&mut e, 0, &data, T).unwrap();
        v.mkdir("/a directory", T).unwrap();
        let mut f = v.create("/a directory/Mixed Case Name.TxT", T).unwrap();
        v.write_at(&mut f, 0, b"hello", T).unwrap();
        v.create("/UPPER.TXT", T).unwrap();
        v.unmount().unwrap();
        fsck(&img.path);
        assert_eq!(mcopy_out(img, "/data.bin"), data);
        assert_eq!(mcopy_out(img, "/a directory/Mixed Case Name.TxT"), b"hello");
        let list = run("mdir", &["-i", img.path.to_str().unwrap(), "-b", "::/a directory"]);
        assert!(list.contains("Mixed Case Name.TxT"), "{list}");
    });
}

#[test]
fn many_names_grow_a_directory() {
    let img = image("d", 32, 64);
    let mut v = open(&img.path);
    v.mkdir("/many", T).unwrap();
    for i in 0..600 {
        v.create(&format!("/many/a file with a long name {i}.text"), T).unwrap();
    }
    v.unmount().unwrap();
    fsck(&img.path);
    let mut v = open(&img.path);
    let d = v.resolve("/many").unwrap();
    assert_eq!(v.read_dir(&d).unwrap().len(), 602);
    assert!(v.resolve("/many/a file with a long name 599.text").is_ok());
    let list = run("mdir", &["-i", img.path.to_str().unwrap(), "-b", "::/many"]);
    assert_eq!(list.lines().filter(|l| !l.ends_with('.') ).count(), 600, "{list}");
}

#[test]
fn removal_gives_the_space_back() {
    each_kind(|img| {
        let mut v = open(&img.path);
        let (_, _, free) = v.space().unwrap();
        let mut e = v.create("/big", T).unwrap();
        v.write_at(&mut e, 0, &pattern(50_000), T).unwrap();
        v.mkdir("/d", T).unwrap();
        v.create("/d/f", T).unwrap();
        assert_eq!(v.rmdir("/d").unwrap_err(), Error::NotEmpty);
        v.unlink("/d/f").unwrap();
        v.rmdir("/d").unwrap();
        v.unlink("/big").unwrap();
        assert_eq!(v.space().unwrap().2, free);
        v.unmount().unwrap();
        fsck(&img.path);
    });
}

#[test]
fn truncate_both_ways_and_gaps() {
    each_kind(|img| {
        let mut v = open(&img.path);
        let mut e = v.create("/f", T).unwrap();
        v.write_at(&mut e, 0, &pattern(10_000), T).unwrap();
        v.truncate(&mut e, 100, T).unwrap();
        v.write_at(&mut e, 5000, b"x", T).unwrap();
        v.unmount().unwrap();
        fsck(&img.path);
        let d = mcopy_out(img, "/f");
        assert_eq!(d.len(), 5001);
        assert_eq!(&d[..100], &pattern(100)[..]);
        assert!(d[100..5000].iter().all(|&b| b == 0), "a gap reads as zeros");
        assert_eq!(d[5000], b'x');
        let mut v = open(&img.path);
        let mut e = v.resolve("/f").unwrap();
        v.truncate(&mut e, 0, T).unwrap();
        assert_eq!(e.first_cluster, 0);
        v.unmount().unwrap();
        fsck(&img.path);
    });
}

#[test]
fn rename_files_and_directories() {
    each_kind(|img| {
        let mut v = open(&img.path);
        v.mkdir("/a", T).unwrap();
        v.mkdir("/b", T).unwrap();
        v.mkdir("/a/inner", T).unwrap();
        let mut f = v.create("/a/inner/file", T).unwrap();
        v.write_at(&mut f, 0, b"data", T).unwrap();
        v.rename("/a/inner", "/b/moved", T).unwrap();
        assert_eq!(v.rename("/b", "/b/moved/x", T).unwrap_err(), Error::Invalid);
        v.rename("/b/moved/file", "/top with a long name", T).unwrap();
        let mut g = v.create("/victim", T).unwrap();
        v.write_at(&mut g, 0, b"old", T).unwrap();
        v.rename("/top with a long name", "/victim", T).unwrap();
        v.rename("/victim", "/VICTIM", T).unwrap();
        v.unmount().unwrap();
        fsck(&img.path);
        assert_eq!(mcopy_out(img, "/VICTIM"), b"data");
        let list = run("mdir", &["-i", img.path.to_str().unwrap(), "-/", "-b", "::/"]);
        assert!(list.contains("moved"), "{list}");
    });
}

#[test]
fn modes_and_times() {
    each_kind(|img| {
        let mut v = open(&img.path);
        v.create("/f", T).unwrap();
        v.mkdir("/d", T).unwrap();
        v.set_mode("/f", 0o444).unwrap();
        assert_ne!(v.resolve("/f").unwrap().attr & msdosfs::ATTR_READONLY, 0);
        v.set_mode("/f", 0o644).unwrap();
        assert_eq!(v.resolve("/f").unwrap().attr & msdosfs::ATTR_READONLY, 0);
        v.set_mode("/d", 0o444).unwrap();
        assert_eq!(v.resolve("/d").unwrap().attr & msdosfs::ATTR_READONLY, 0, "a directory stays");
        assert_eq!(v.set_mode("/", 0o755).unwrap_err(), Error::Invalid);
        let when = DosTime { date: 0x5b67, time: 0x1234 };
        v.set_mtime("/f", when).unwrap();
        assert_eq!(v.resolve("/f").unwrap().mtime, when);
        assert_eq!(v.set_mtime("/", when).unwrap_err(), Error::Invalid);
        v.unmount().unwrap();
        fsck(&img.path);
    });
}

#[test]
fn the_disk_full_and_names_refused() {
    let img = image("e", 12, 1);
    let mut v = open(&img.path);
    assert_eq!(v.create("/a?b", T).unwrap_err(), Error::Invalid);
    assert_eq!(v.create(&format!("/{}", "x".repeat(256)), T).unwrap_err(), Error::NameTooLong);
    v.create("/x", T).unwrap();
    assert_eq!(v.create("/X", T).unwrap_err(), Error::Exists);
    // `X.TXT` free as a name, taken as another's hidden short name.
    v.create("/lower~1.txt", T).unwrap();
    v.create("/lower one.txt", T).unwrap();
    v.create("/LOWERO~1.TXT", T).unwrap();
    let mut e = v.create("/full", T).unwrap();
    assert_eq!(v.write_at(&mut e, 0, &vec![1; 4 << 20], T).unwrap_err(), Error::NoSpace);
    v.unmount().unwrap();
    run("fsck.fat", &["-n", img.path.to_str().unwrap()]);
}
