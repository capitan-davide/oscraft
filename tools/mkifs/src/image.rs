//! Image layout and writing.
//!
//! ```text
//! base ─► boot prefix (raw bootfile: jump to startup_vaddr)
//!   hdr ─► struct startup_header
//!          startup program (linked at hdr + 0x100, .bss included)
//!          struct startup_trailer_v2
//!   ifs ─► struct image_header, dirents
//!          bootstrap executables (page aligned, mapped in place)
//!          other ELF files (page aligned), then data files
//!          struct image_trailer_v2
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};

use crate::buildfile::{
    self, Attrs, Content, Entry, FileType, split_words, strip_comment, unquote,
};
use crate::elf::{self, ET_DYN, ET_EXEC, ET_REL, Elf, PAGE_SIZE};
use crate::format::*;
use crate::script;

/// Base virtual address for bootstrap executables (`vboot` of the QNX 8.0
/// aarch64le bootfiles).
pub const VBOOT: u64 = 0xffff_ff80_6000_0000;
/// Default image address when the buildfile has no `[image=]` (4 MB).
const DEFAULT_IMAGE_BASE: u64 = 4 << 20;
/// Space reserved for the `raw` bootfile's jump prefix.
const RAW_PREFIX_LEN: u64 = 0x100;
/// Linker script for relocatable bootstrap programs (startup, procnto),
/// written for ld.lld: every output section is aligned to its contents.
/// `.text` is placed with `--section-start`. procnto finds its built-in
/// modules between the `.rodata_mod_start` and `.rodata_mod_end` sections,
/// so those stay together and in order.
const LINK_SCRIPT: &str = "\
ENTRY(_start)
PHDRS { text PT_LOAD; data PT_LOAD; }
SECTIONS {
  .text : {
    *(.text.unlikely .text.*_unlikely) *(.text.exit .text.exit.*)
    *(.text.startup .text.startup.*) *(.text.hot .text.hot.*)
    *(.text .text.*)
  } :text
  .rodata ALIGN(0x10000) : {
    *(.rodata_mod_start) *(.rodata_mod) *(.rodata_mod_end)
    *(.rodata .rodata.*)
  } :data
  .data : {
    *(.data .data.*)
    . = ALIGN(0x100); *(.read_mostly)
    . = ALIGN(0x100); *(.eh_frame*)
  } :data
  .got : { *(.got) *(.got.plt) } :data
  .bss : { *(.bss .bss.*) *(COMMON) } :data
}
";
/// Host search directories under each root, per `mkifs` MKIFS_PATH defaults.
const SEARCH_SUBDIRS: &[&str] = &[
    "sbin", "usr/sbin", "boot/sys", "bin", "usr/bin", "lib", "lib/dll", "usr/lib",
];

#[derive(Debug, Clone)]
pub struct Options {
    pub buildfile: PathBuf,
    pub output: PathBuf,
    pub verbose: u8,
    /// `-r` root directories, searched before `$QNX_TARGET`.
    pub roots: Vec<PathBuf>,
    /// `-o`: directory for `.sym` files of `[+keeplinked]` executables.
    pub sym_dir: Option<PathBuf>,
    /// `-a`: suffix for `.sym` files.
    pub sym_suffix: Option<String>,
    /// `-l`: lines processed before the buildfile.
    pub pre_lines: Vec<String>,
    /// Linker for relocatable bootstrap executables.
    pub linker: PathBuf,
    /// Linker script overriding the built-in one.
    pub link_script: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bootfile {
    /// Image starts with a jump to the startup entry point.
    Raw,
    /// Image starts with the startup header; the loader jumps to `startup_vaddr`.
    Binary,
}

#[derive(Debug)]
struct BootProgram {
    host: PathBuf,
    argv: Vec<String>,
    env: Vec<String>,
    keeplinked: bool,
}

#[derive(Debug)]
enum NodeKind {
    File { data: Vec<u8>, elf: bool },
    Bootstrap(usize),
    Dir,
    Link(String),
}

#[derive(Debug)]
struct Node {
    mode: u32,
    uid: u32,
    gid: u32,
    mtime: u32,
    kind: NodeKind,
}

struct Builder<'a> {
    opts: &'a Options,
    cpu: String,
    build_time: u32,
    nodes: BTreeMap<String, Node>,
    explicit: BTreeSet<String>,
    script_path: Option<String>,
    autolink_candidates: Vec<String>,
    warnings: Vec<String>,
}

pub fn build(opts: &Options) -> Result<()> {
    let text = std::fs::read_to_string(&opts.buildfile)
        .with_context(|| format!("reading {}", opts.buildfile.display()))?;
    let mut warnings = Vec::new();
    let entries = buildfile::parse(&text, &opts.pre_lines, &mut |w| warnings.push(w))?;
    let image = build_image(opts, &entries, warnings)?;
    std::fs::write(&opts.output, &image)
        .with_context(|| format!("writing {}", opts.output.display()))?;
    Ok(())
}

fn build_image(opts: &Options, entries: &[Entry], warnings: Vec<String>) -> Result<Vec<u8>> {
    let boot_entry = entries
        .iter()
        .find(|e| e.attrs.virtual_.is_some())
        .ok_or_else(|| anyhow!("no bootstrap file (an entry with [virtual=cpu,bootfile])"))?;
    let (cpu, bootfile) = boot_entry.attrs.virtual_.clone().unwrap();
    let bootfile = match bootfile.as_str() {
        "raw" => Bootfile::Raw,
        "binary" => Bootfile::Binary,
        b => bail!("unsupported bootfile '{b}' (supported: raw, binary)"),
    };
    ensure!(
        cpu == "aarch64le",
        "unsupported CPU '{cpu}' (supported: aarch64le)"
    );
    let build_time = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as u32)
        });

    let mut b = Builder {
        opts,
        cpu,
        build_time,
        nodes: BTreeMap::new(),
        explicit: BTreeSet::new(),
        script_path: None,
        autolink_candidates: Vec::new(),
        warnings,
    };
    let image_spec = boot_entry.attrs.image.clone().unwrap_or_default();
    let programs = b.bootstrap_programs(boot_entry)?;
    ensure!(
        programs.len() >= 2,
        "the bootstrap file needs a startup program and procnto"
    );
    for (i, p) in programs.iter().enumerate().skip(1) {
        let name = p.host.file_name().unwrap().to_string_lossy();
        let path = format!("{}/{name}", boot_entry.attrs.prefix);
        b.explicit.insert(path.clone());
        b.nodes.insert(
            path,
            Node {
                mode: S_IFREG | 0o755,
                uid: 0,
                gid: 0,
                mtime: b.mtime_of(&p.host),
                kind: NodeKind::Bootstrap(i),
            },
        );
    }
    let boot_env = programs.last().unwrap().env.clone();
    for e in entries.iter().filter(|e| e.attrs.virtual_.is_none()) {
        b.add_entry(e, &boot_env)
            .with_context(|| format!("buildfile line {}", e.line))?;
    }
    b.add_autolinks()?;
    b.add_parent_dirs();
    for w in &b.warnings {
        eprintln!("mkifs: warning: {w}");
    }

    let base = image_spec.start.unwrap_or(DEFAULT_IMAGE_BASE);
    let prefix_len = match bootfile {
        Bootfile::Raw => RAW_PREFIX_LEN,
        Bootfile::Binary => 0,
    };
    let hdr = base + prefix_len;

    // Startup: linked right after its header, boot arguments filled in.
    let startup = &programs[0];
    let text = (hdr + STARTUP_HDR_SIZE as u64).next_multiple_of(PAGE_SIZE);
    let startup_elf = b.linked(startup, text)?;
    let mut sblob = elf::flatten(&startup_elf, hdr + STARTUP_HDR_SIZE as u64)?;
    let args = encode_bootargs(&startup.argv, &startup.env, hdr)?;
    let at = bootargs_area(&sblob)
        .context("startup has no boot-argument area ('ddpvbskr' followed by its empty room)")?;
    ensure!(
        at + args.len() <= sblob.len(),
        "startup arguments do not fit"
    );
    sblob[at..at + args.len()].copy_from_slice(&args);
    let startup_size = (STARTUP_HDR_SIZE + sblob.len()).next_multiple_of(4) + TRAILER_V2_SIZE;
    let entry = u32::try_from(startup_elf.e_entry).context("startup entry point above 4 GB")?;

    // Image filesystem.
    let ifs_addr = hdr + startup_size as u64;
    let ifs = b.image_fs(ifs_addr, base, hdr, &programs)?;

    // Startup header and trailer.
    let stored_size = (startup_size + ifs.len()) as u32;
    let sh = StartupHeader {
        signature: STARTUP_HDR_SIGNATURE,
        version: STARTUP_HDR_VERSION,
        flags1: STARTUP_HDR_FLAGS1_VIRTUAL | STARTUP_HDR_FLAGS1_TRAILER_V2,
        header_size: STARTUP_HDR_SIZE as u16,
        machine: EM_AARCH64,
        startup_vaddr: entry,
        image_paddr: u32::try_from(hdr)?,
        ram_paddr: u32::try_from(hdr)?,
        ram_size: stored_size,
        startup_size: startup_size as u32,
        stored_size,
        imagefs_size: ifs.len() as u32,
        preboot_size: prefix_len as u16,
        ..Default::default()
    };
    let mut startup_region = sh.to_bytes().to_vec();
    startup_region.extend_from_slice(&sblob);
    pad_to(&mut startup_region, 4);
    let trailer = trailer_v2(&startup_region);
    startup_region.extend_from_slice(&trailer);

    let mut image = match bootfile {
        Bootfile::Raw => raw_prefix(prefix_len as u32),
        Bootfile::Binary => Vec::new(),
    };
    image.extend_from_slice(&startup_region);
    image.extend_from_slice(&ifs);
    if let Some(total) = image_spec.total_size {
        ensure!(
            image.len() as u64 <= total,
            "image ({} bytes) exceeds {total} bytes",
            image.len()
        );
        image.resize(total as usize, 0);
    }
    pad_to(&mut image, image_spec.align.unwrap_or(4) as usize);
    if let Some(max) = image_spec.max_size {
        ensure!(
            image.len() as u64 <= max,
            "image ({} bytes) exceeds the maximum {max}",
            image.len()
        );
    }
    if let Some(end) = image_spec.end {
        ensure!(
            base + image.len() as u64 - 1 <= end,
            "image extends beyond {end:#x}"
        );
    }
    if opts.verbose > 0 {
        eprintln!(
            "mkifs: image {:#x}-{:#x}: startup header {hdr:#x}, entry {entry:#x}, imagefs {ifs_addr:#x} ({} bytes)",
            base,
            base + image.len() as u64,
            ifs.len()
        );
    }
    Ok(image)
}

/// `adr x19, <startup header>; ldr w19, [x19, #12]; br x19`, padded to `len`.
/// Offset 12 is `startup_header.startup_vaddr`.
fn raw_prefix(len: u32) -> Vec<u8> {
    let adr = 0x1000_0000 | ((len & 3) << 29) | (((len >> 2) & 0x7ffff) << 5) | 19;
    let mut p = Vec::new();
    for insn in [adr, 0xb940_0e73, 0xd61f_0260] {
        p.extend_from_slice(&u32::to_le_bytes(insn));
    }
    p.resize(len as usize, 0);
    p
}

/// Startup's boot-argument area: the signature followed by the area's
/// empty room. The signature can be in startup's read-only data too, a
/// startup that checks for it (OsCraft's does), which comes before its
/// data: the last copy with zeros after it is the area.
fn bootargs_area(blob: &[u8]) -> Option<usize> {
    const ROOM: usize = 64;
    let n = BOOTARGS_SIGNATURE.len();
    (0..blob.len().saturating_sub(n + ROOM)).rev().find(|&i| {
        &blob[i..i + n] == BOOTARGS_SIGNATURE && blob[i + n..i + n + ROOM].iter().all(|&b| b == 0)
    })
}

impl Builder<'_> {
    fn search_dirs(&self, attrs: &Attrs) -> Vec<PathBuf> {
        if let Some(s) = &attrs.search {
            return s.clone();
        }
        let mut dirs: Vec<PathBuf> = std::env::var("MKIFS_PATH")
            .map(|p| {
                p.split(':')
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        let roots = self
            .opts
            .roots
            .iter()
            .cloned()
            .chain(std::env::var_os("QNX_TARGET").map(PathBuf::from));
        for root in roots {
            dirs.extend(SEARCH_SUBDIRS.iter().map(|s| root.join(&self.cpu).join(s)));
        }
        dirs
    }

    fn find_host(&self, name: &str, attrs: &Attrs) -> Option<PathBuf> {
        let p = Path::new(name);
        if p.is_absolute() {
            return p.exists().then(|| p.to_path_buf());
        }
        if name.contains('/') {
            let base = attrs.cd.clone().unwrap_or_else(|| PathBuf::from("."));
            let cand = base.join(p);
            if cand.exists() {
                return Some(cand);
            }
        }
        self.search_dirs(attrs)
            .into_iter()
            .map(|d| d.join(p))
            .find(|c| c.is_file())
    }

    fn mtime_of(&self, host: &Path) -> u32 {
        std::fs::metadata(host)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(self.build_time, |d| d.as_secs() as u32)
    }

    /// The startup program and bootstrap executables of the bootstrap file.
    fn bootstrap_programs(&mut self, e: &Entry) -> Result<Vec<BootProgram>> {
        let Some(Content::Inline(text)) = &e.content else {
            bail!(
                "the bootstrap file must be inline ({} = {{ ... }})",
                e.target
            );
        };
        let mut programs = Vec::new();
        let mut global_env: Vec<String> = Vec::new();
        let mut global_keeplinked = e.attrs.keeplinked;
        for (n, raw) in text.lines().enumerate() {
            let ctx = || format!("bootstrap file, line {}", n + 1);
            let words = split_words(strip_comment(raw)).with_context(ctx)?;
            let mut words = words.as_slice();
            let mut keeplinked = None;
            while let Some(w) = words.first().filter(|w| w.starts_with('[')) {
                for t in split_words(&w[1..w.len() - 1])? {
                    match t.as_str() {
                        "+keeplinked" => keeplinked = Some(true),
                        "-keeplinked" => keeplinked = Some(false),
                        t => self
                            .warnings
                            .push(format!("{}: ignoring bootstrap attribute '{t}'", ctx())),
                    }
                }
                words = &words[1..];
            }
            let mut env = global_env.clone();
            let mut local = Vec::new();
            while let Some(w) = words
                .first()
                .filter(|w| w.contains('=') && !w.starts_with('-'))
            {
                local.push(unquote(w));
                words = &words[1..];
            }
            let Some((name, args)) = words.split_first() else {
                global_env.extend(local);
                if let Some(k) = keeplinked {
                    global_keeplinked = k;
                }
                continue;
            };
            env.extend(local);
            let name = buildfile::expand_vars(&unquote(name)).with_context(ctx)?;
            let host = self
                .find_host(&name, &e.attrs)
                .ok_or_else(|| anyhow!("{}: bootstrap program '{name}' not found", ctx()))?;
            let argv0 = Path::new(&name)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            programs.push(BootProgram {
                host,
                argv: std::iter::once(argv0)
                    .chain(args.iter().map(|a| unquote(a)))
                    .collect(),
                env,
                keeplinked: keeplinked.unwrap_or(global_keeplinked),
            });
        }
        Ok(programs)
    }

    /// The program's ELF, linked at `text` if it is a relocatable object.
    fn linked(&self, p: &BootProgram, text: u64) -> Result<Elf> {
        let data =
            std::fs::read(&p.host).with_context(|| format!("reading {}", p.host.display()))?;
        let elf = Elf::parse(data).with_context(|| p.host.display().to_string())?;
        let name = p.host.file_name().unwrap().to_string_lossy().into_owned();
        let linked = match elf.e_type {
            ET_EXEC => elf,
            ET_REL => self.link(&p.host, &name, text)?,
            t => bail!(
                "{}: unsupported ELF type {t} for a bootstrap program",
                p.host.display()
            ),
        };
        if p.keeplinked {
            let sym = match &self.opts.sym_suffix {
                Some(s) => format!("{name}.{s}.sym"),
                None => format!("{name}.sym"),
            };
            let dir = self
                .opts
                .sym_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from("."));
            std::fs::write(dir.join(&sym), &linked.data)
                .with_context(|| format!("writing {sym}"))?;
        }
        Ok(linked)
    }

    fn link(&self, input: &Path, name: &str, text: u64) -> Result<Elf> {
        // A directory of our own for the scratch files, gone however this
        // ends (review TOOLS-7: predictable names in the shared temp
        // directory, which a planted symlink could redirect, and files left
        // behind when the linker failed).
        let tmp = ScratchDir::new()?;
        let script = match &self.opts.link_script {
            Some(s) => s.clone(),
            None => {
                let p = tmp.0.join("bootstrap.ld");
                std::fs::write(&p, LINK_SCRIPT)?;
                p
            }
        };
        let out = tmp.0.join(name.replace('/', "_"));
        let status = Command::new(&self.opts.linker)
            .arg("-T")
            .arg(&script)
            .args(["--section-start", &format!(".text={text:#x}")])
            .args(["--no-relax", "-z", "max-page-size=4096", "-o"])
            .arg(&out)
            .arg(input)
            .status()
            .with_context(|| format!("running {}", self.opts.linker.display()))?;
        ensure!(status.success(), "linking {name} failed ({status})");
        let data = std::fs::read(&out)?;
        Elf::parse(data)
    }

    fn insert(&mut self, path: String, node: Node) {
        if self.nodes.insert(path.clone(), node).is_some() && self.opts.verbose > 0 {
            self.warnings.push(format!(
                "'/{path}' specified more than once; the last one is used"
            ));
        }
        self.explicit.insert(path);
    }

    fn add_entry(&mut self, e: &Entry, boot_env: &[String]) -> Result<()> {
        let a = &e.attrs;
        let path = if let Some(abs) = e.target.strip_prefix('/') {
            abs.trim_end_matches('/').to_string()
        } else {
            format!("{}/{}", a.prefix, e.target)
        };
        // A target listed twice is an error, unless `+dupignore` keeps
        // the earlier entry (QNX's mkifs, measured; review TOOLS-2), or
        // it is the boot script again: a second `[+script]` block's
        // commands follow the first's (a build file composed of parts,
        // e.g. the QNX personality's over the base image's).
        let script_again = a.script && self.script_path.as_deref() == Some(path.as_str());
        if self.explicit.contains(&path) && !script_again {
            if a.dupignore {
                return Ok(());
            }
            bail!("entry '{path}' redefined");
        }
        let uid = a.uid.unwrap_or(0);
        let gid = a.gid.unwrap_or(0);
        match a.file_type {
            FileType::Link => {
                let Some(Content::Host(target)) = &e.content else {
                    bail!("link '{}' needs a target (name=target)", e.target);
                };
                let mtime = a.mtime.unwrap_or(self.build_time);
                self.insert(
                    path,
                    Node {
                        mode: S_IFLNK | 0o777,
                        uid,
                        gid,
                        mtime,
                        kind: NodeKind::Link(target.clone()),
                    },
                );
            }
            FileType::Dir => {
                let mode = S_IFDIR | a.dperms.apply(0o755)?;
                let mtime = a.mtime.unwrap_or(self.build_time);
                self.insert(
                    path,
                    Node {
                        mode,
                        uid,
                        gid,
                        mtime,
                        kind: NodeKind::Dir,
                    },
                );
            }
            FileType::File => {
                let (data, host_mode, mtime) = match &e.content {
                    Some(Content::Inline(text)) => {
                        (text.clone().into_bytes(), 0o644, self.build_time)
                    }
                    other => {
                        let name = match other {
                            Some(Content::Host(h)) => h.clone(),
                            _ if e.target.starts_with('/') => Path::new(&e.target)
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .into_owned(),
                            _ => e.target.clone(),
                        };
                        let Some(host) = self.find_host(&name, a) else {
                            if a.optional {
                                self.warnings
                                    .push(format!("host file '{name}' not found; skipped"));
                                return Ok(());
                            }
                            bail!("host file '{name}' not found");
                        };
                        let meta = std::fs::metadata(&host)?;
                        let data = std::fs::read(&host)
                            .with_context(|| format!("reading {}", host.display()))?;
                        use std::os::unix::fs::PermissionsExt;
                        (data, meta.permissions().mode(), self.mtime_of(&host))
                    }
                };
                let mtime = a.mtime.unwrap_or(mtime);
                let (data, is_elf) = if a.script {
                    let text = String::from_utf8(data).context("script is not UTF-8")?;
                    let records = script::compile(&text, boot_env)?;
                    if script_again
                        && let Some(Node {
                            kind: NodeKind::File { data, .. },
                            ..
                        }) = self.nodes.get_mut(&path)
                    {
                        // The first block's end record (four zeros) goes:
                        // the second's commands follow its last.
                        data.truncate(data.len().saturating_sub(4));
                        data.extend(records);
                        return Ok(());
                    }
                    self.script_path = Some(path.clone());
                    (records, false)
                } else {
                    let is_elf = elf::is_elf64(&data);
                    (data, is_elf)
                };
                let mode = S_IFREG | a.perms.apply(host_mode)?;
                let autolink = a.autolink && is_elf;
                self.insert(
                    path.clone(),
                    Node {
                        mode,
                        uid,
                        gid,
                        mtime,
                        kind: NodeKind::File { data, elf: is_elf },
                    },
                );
                if autolink {
                    self.autolink_candidates.push(path);
                }
            }
        }
        Ok(())
    }

    /// Shared objects are stored under their `DT_SONAME`, with symbolic
    /// links for the requested and the unversioned names.
    fn add_autolinks(&mut self) -> Result<()> {
        for path in std::mem::take(&mut self.autolink_candidates) {
            let Some(Node {
                kind: NodeKind::File { data, .. },
                ..
            }) = self.nodes.get(&path)
            else {
                continue;
            };
            let Ok(elf) = Elf::parse(data.clone()) else {
                continue;
            };
            if elf.e_type != ET_DYN {
                continue;
            }
            let Some(soname) = elf.soname().filter(|s| s.contains(".so")) else {
                continue;
            };
            let (dir, file) = match path.rsplit_once('/') {
                Some((d, f)) => (format!("{d}/"), f.to_string()),
                None => (String::new(), path.clone()),
            };
            let unversioned = soname.split(".so").next().unwrap().to_string() + ".so";
            let real = format!("{dir}{soname}");
            if file != soname {
                let node = self.nodes.remove(&path).unwrap();
                self.explicit.remove(&path);
                if !self.nodes.contains_key(&real) {
                    self.nodes.insert(real.clone(), node);
                }
            }
            for name in [file, unversioned] {
                let link = format!("{dir}{name}");
                if name != soname
                    && !self.explicit.contains(&link)
                    && !self.nodes.contains_key(&link)
                {
                    let mtime = self.nodes[&real].mtime;
                    self.nodes.insert(
                        link,
                        Node {
                            mode: S_IFLNK | 0o777,
                            uid: 0,
                            gid: 0,
                            mtime,
                            kind: NodeKind::Link(soname.clone()),
                        },
                    );
                }
            }
        }
        Ok(())
    }

    fn add_parent_dirs(&mut self) {
        let mut dirs = BTreeSet::new();
        for path in self.nodes.keys() {
            let mut p = path.as_str();
            while let Some((parent, _)) = p.rsplit_once('/') {
                dirs.insert(parent.to_string());
                p = parent;
            }
        }
        dirs.insert(String::new());
        for d in dirs {
            self.nodes.entry(d).or_insert(Node {
                mode: S_IFDIR | 0o755,
                uid: 0,
                gid: 0,
                mtime: self.build_time,
                kind: NodeKind::Dir,
            });
        }
    }

    fn image_fs(
        &self,
        ifs_addr: u64,
        base: u64,
        hdr: u64,
        programs: &[BootProgram],
    ) -> Result<Vec<u8>> {
        // Order: root, bootstrap executables, then everything else by path.
        let mut order: Vec<&String> = Vec::new();
        order.push(self.nodes.keys().find(|k| k.is_empty()).unwrap());
        let mut boots: Vec<(&String, usize)> = self
            .nodes
            .iter()
            .filter_map(|(k, n)| match n.kind {
                NodeKind::Bootstrap(i) => Some((k, i)),
                _ => None,
            })
            .collect();
        boots.sort_by_key(|&(_, i)| i);
        order.extend(boots.iter().map(|(k, _)| *k));
        order.extend(
            self.nodes
                .iter()
                .filter(|(k, n)| !k.is_empty() && !matches!(n.kind, NodeKind::Bootstrap(_)))
                .map(|(k, _)| k),
        );

        let mut dirents: Vec<Dirent> = order
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let n = &self.nodes[*path];
                let mut ino = i as u32 + 1;
                if let NodeKind::Bootstrap(_) = n.kind {
                    ino |= IFS_INO_PROCESSED_ELF | IFS_INO_BOOTSTRAP_EXE;
                }
                let kind = match &n.kind {
                    NodeKind::File { .. } | NodeKind::Bootstrap(_) => {
                        DirentKind::File { offset: 0, size: 0 }
                    }
                    NodeKind::Dir => DirentKind::Dir,
                    NodeKind::Link(t) => DirentKind::Symlink { target: t.clone() },
                };
                Dirent {
                    ino,
                    mode: n.mode,
                    gid: n.gid,
                    uid: n.uid,
                    mtime: n.mtime,
                    path: (*path).clone(),
                    kind,
                }
            })
            .collect();

        let mut header = ImageHeader {
            flags: IMAGE_FLAGS_INO_BITS | IMAGE_FLAGS_SORTED | IMAGE_FLAGS_TRAILER_V2,
            mountpoint: "/".into(),
            ..Default::default()
        };
        let header_len = header.to_bytes().len();
        let dir_len: usize = dirents.iter().map(Dirent::encoded_len).sum::<usize>() + 4;
        header.dir_offset = header_len as u32;
        header.hdr_dir_size = (header_len + dir_len) as u32;

        // Place file contents: bootstraps, other ELF files, then data.
        let mut data: Vec<(usize, u64, Vec<u8>)> = Vec::new(); // (dirent index, paddr, bytes)
        let mut cursor = ifs_addr + header.hdr_dir_size as u64;
        let mut place = |idx: usize, bytes: Vec<u8>, align: u64, cursor: &mut u64| {
            let at = cursor.next_multiple_of(align);
            *cursor = at + bytes.len() as u64;
            data.push((idx, at, bytes));
        };
        for (idx, path) in order.iter().enumerate() {
            if let NodeKind::Bootstrap(i) = self.nodes[*path].kind {
                let p = &programs[i];
                let at = cursor.next_multiple_of(PAGE_SIZE);
                let text = VBOOT + (at - base) + PAGE_SIZE;
                let linked = self.linked(p, text)?;
                let args = encode_bootargs(&p.argv, &p.env, hdr)?;
                let bytes = elf::process_bootstrap(&linked, at, &args)
                    .with_context(|| format!("bootstrap executable /{path}"))?;
                place(idx, bytes, PAGE_SIZE, &mut cursor);
                if self.opts.verbose > 0 {
                    eprintln!(
                        "mkifs: bootstrap /{path} at {at:#x}, entry {:#x}",
                        linked.e_entry
                    );
                }
            }
        }
        for pass_elf in [true, false] {
            for (idx, path) in order.iter().enumerate() {
                if let NodeKind::File { data: d, elf } = &self.nodes[*path].kind
                    && *elf == pass_elf
                {
                    place(
                        idx,
                        d.clone(),
                        if pass_elf { PAGE_SIZE } else { 1 },
                        &mut cursor,
                    );
                }
            }
        }

        if let Some(sp) = &self.script_path {
            header.script_ino = dirents.iter().find(|d| &d.path == sp).unwrap().ino;
        }
        for (slot, (path, _)) in boots.iter().take(4).enumerate() {
            header.boot_ino[slot] = dirents.iter().find(|d| &d.path == *path).unwrap().ino;
        }
        for (idx, at, bytes) in &data {
            dirents[*idx].kind = DirentKind::File {
                offset: (at - ifs_addr) as u32,
                size: bytes.len() as u32,
            };
        }

        let body_len = (cursor - ifs_addr) as usize;
        header.image_size = (body_len.next_multiple_of(4) + TRAILER_V2_SIZE) as u32;
        let mut ifs = header.to_bytes();
        for d in &dirents {
            // A dirent's size is 16 bits (review TOOLS-6).
            ensure!(d.encoded_len() <= u16::MAX as usize, "directory entry too long ({} bytes)", d.encoded_len());
            ifs.extend_from_slice(&d.to_bytes());
        }
        ifs.extend_from_slice(&[0; 4]);
        debug_assert_eq!(ifs.len(), header.hdr_dir_size as usize);
        ifs.resize(body_len, 0);
        for (_, at, bytes) in &data {
            let o = (at - ifs_addr) as usize;
            ifs[o..o + bytes.len()].copy_from_slice(bytes);
        }
        pad_to(&mut ifs, 4);
        let trailer = trailer_v2(&ifs);
        ifs.extend_from_slice(&trailer);
        if self.opts.verbose > 1 {
            for d in &dirents {
                eprintln!("mkifs:   {:#010x} {:o} {}", d.ino, d.mode, d.path);
            }
        }
        Ok(ifs)
    }
}

/// A private directory in the system's temporary directory: made new
/// (never one that exists, so never through someone's symlink), readable by
/// us only, and removed with everything in it when dropped.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let base = std::env::temp_dir();
        for attempt in 0u32..100 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos());
            let dir = base.join(format!("mkifs-{}-{nanos:x}-{attempt}", std::process::id()));
            match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
            }
        }
        bail!("no scratch directory could be made in {}", base.display())
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
