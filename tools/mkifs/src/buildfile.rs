//! Buildfile parsing.
//!
//! A buildfile is a list of lines. Each line holds attribute blocks
//! (`[name=value +flag -flag]`) and optionally one file specification:
//!
//! ```text
//! [attrs] target                 # host file found by searching for `target`
//! [attrs] target=host            # host file `host`
//! [attrs] target = { ... }       # inline contents, up to a line starting with `}`
//! [type=link] target=link_text   # symbolic link
//! ```
//!
//! Attribute blocks on a line of their own change the defaults for all
//! following lines; blocks in front of a file specification apply to that
//! file only. See the `mkifs` utility reference in the QNX 8.0 documentation.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};

/// Address-space specification of the `image` attribute:
/// `[start][-end][,maxsize][=totalsize][%align]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddrSpace {
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub max_size: Option<u64>,
    pub total_size: Option<u64>,
    pub align: Option<u64>,
}

impl AddrSpace {
    pub fn parse(s: &str) -> Result<Self> {
        let mut a = Self::default();
        let mut rest = s;
        let take = |rest: &mut &str| -> Result<u64> {
            let n = rest.find(['-', ',', '=', '%']).unwrap_or(rest.len());
            let (num, tail) = rest.split_at(n);
            *rest = tail;
            parse_size(num)
        };
        if !rest.is_empty() && !rest.starts_with(['-', ',', '=', '%']) {
            a.start = Some(take(&mut rest)?);
        }
        while let Some(c) = rest.chars().next() {
            rest = &rest[1..];
            let v = Some(take(&mut rest)?);
            match c {
                '-' => a.end = v,
                ',' => a.max_size = v,
                '=' => a.total_size = v,
                '%' => a.align = v,
                _ => unreachable!(),
            }
        }
        Ok(a)
    }
}

/// Parse a number in C syntax (decimal, `0x` hex, leading-0 octal) with an
/// optional `k`, `m` or `g` suffix.
pub fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim();
    let (digits, mult) = match s.chars().last().map(|c| c.to_ascii_lowercase()) {
        Some('k') => (&s[..s.len() - 1], 1u64 << 10),
        Some('m') => (&s[..s.len() - 1], 1 << 20),
        Some('g') => (&s[..s.len() - 1], 1 << 30),
        _ => (s, 1),
    };
    let v = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16)
    } else if digits.len() > 1 && digits.starts_with('0') {
        u64::from_str_radix(&digits[1..], 8)
    } else {
        digits.parse()
    }
    .map_err(|_| anyhow!("invalid number '{s}'"))?;
    v.checked_mul(mult)
        .ok_or_else(|| anyhow!("number '{s}' too large"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileType {
    #[default]
    File,
    Link,
    Dir,
}

/// Permission specification: an octal mode, or chmod-style clauses such as
/// `+x`, `a=rx` or `u+w,go-w` applied to the host file's mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Perms {
    Host,
    Spec(String),
}

impl Perms {
    pub fn apply(&self, base: u32) -> Result<u32> {
        let Perms::Spec(spec) = self else {
            return Ok(base & 0o7777);
        };
        if spec == "*" {
            return Ok(base & 0o7777);
        }
        if spec.chars().all(|c| c.is_digit(8)) {
            return Ok(u32::from_str_radix(spec, 8)?);
        }
        let mut mode = base & 0o7777;
        for clause in spec.split(',') {
            let op_at = clause
                .find(['+', '-', '='])
                .ok_or_else(|| anyhow!("bad perms '{spec}'"))?;
            let (who, rest) = clause.split_at(op_at);
            let (op, what) = rest.split_at(1);
            let mut who_mask = 0;
            for c in who.chars() {
                who_mask |= match c {
                    'u' => 0o4700,
                    'g' => 0o2070,
                    'o' => 0o1007,
                    'a' => 0o7777,
                    _ => bail!("bad perms '{spec}'"),
                };
            }
            if who.is_empty() {
                who_mask = 0o7777;
            }
            let mut bits = 0;
            for c in what.chars() {
                bits |= match c {
                    'r' => 0o444,
                    'w' => 0o222,
                    'x' => 0o111,
                    's' => 0o6000,
                    't' => 0o1000,
                    _ => bail!("bad perms '{spec}'"),
                };
            }
            let bits = bits & who_mask;
            mode = match op {
                "+" => mode | bits,
                "-" => mode & !bits,
                _ => (mode & !(who_mask & 0o777)) | bits,
            };
        }
        Ok(mode)
    }
}

/// The attributes in effect for one buildfile entry.
#[derive(Debug, Clone)]
pub struct Attrs {
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub perms: Perms,
    pub dperms: Perms,
    pub file_type: FileType,
    pub script: bool,
    pub keeplinked: bool,
    pub optional: bool,
    pub autolink: bool,
    pub search: Option<Vec<PathBuf>>,
    pub prefix: String,
    pub cd: Option<PathBuf>,
    pub mtime: Option<u32>,
    /// `virtual=[cpu,]bootfile`: marks the bootstrap file.
    pub virtual_: Option<(String, String)>,
    pub image: Option<AddrSpace>,
    /// `+dupignore`: a later entry for a target already in the image is
    /// ignored, the earlier one kept (else it is an error).
    pub dupignore: bool,
    /// The attributes set so far (by name), for the `?` prefix: `?name...`
    /// applies only if `name` hasn't been set (QNX's mkifs, measured).
    pub set: std::collections::BTreeSet<String>,
}

impl Default for Attrs {
    fn default() -> Self {
        Self {
            uid: None,
            gid: None,
            perms: Perms::Host,
            dperms: Perms::Spec("0755".into()),
            file_type: FileType::File,
            script: false,
            keeplinked: false,
            // As QNX's mkifs: a file not found is an error, unless the
            // entry says `[+optional]` (review TOOLS-1).
            optional: false,
            autolink: true,
            search: None,
            prefix: "proc/boot".into(),
            cd: None,
            mtime: None,
            virtual_: None,
            image: None,
            dupignore: false,
            set: Default::default(),
        }
    }
}

/// Attributes that are accepted but have no effect in this implementation.
const IGNORED_ATTRS: &[&str] = &[
    "big_pages",
    "compress",
    "followlink",
    "page_align",
    "pagesizes",
    "raw",
    "ram",
    "keepsection",
    "phys_align",
];

impl Attrs {
    /// Apply one attribute token (`+name`, `-name`, `name=value`).
    pub fn apply(&mut self, token: &str, warn: &mut dyn FnMut(String)) -> Result<()> {
        let conditional = token.starts_with('?');
        let token = token.strip_prefix('?').unwrap_or(token);
        let (name, value, flag) = if let Some(n) = token.strip_prefix('+') {
            (n, None, true)
        } else if let Some(n) = token.strip_prefix('-') {
            (n, None, false)
        } else if let Some((n, v)) = token.split_once('=') {
            (n, Some(expand_vars(&unquote(v))?), true)
        } else {
            bail!("attribute '{token}' needs a value or a +/- prefix");
        };
        let need = |v: &Option<String>| {
            v.clone()
                .ok_or_else(|| anyhow!("attribute '{name}' needs a value"))
        };
        // `?`: only if not set before (review TOOLS-3: it was applied
        // regardless).
        if conditional && self.set.contains(name) {
            return Ok(());
        }
        self.set.insert(name.to_string());
        match name {
            "uid" => self.uid = id_spec(&need(&value)?)?,
            "gid" => self.gid = id_spec(&need(&value)?)?,
            "perms" => self.perms = Perms::Spec(need(&value)?),
            "dperms" => self.dperms = Perms::Spec(need(&value)?),
            "type" => {
                self.file_type = match need(&value)?.as_str() {
                    "file" => FileType::File,
                    "link" => FileType::Link,
                    "dir" => FileType::Dir,
                    t => bail!("unsupported file type '{t}'"),
                }
            }
            "script" => self.script = flag,
            "keeplinked" => self.keeplinked = flag,
            "optional" => self.optional = flag,
            "dupignore" => self.dupignore = flag,
            "autolink" => self.autolink = flag,
            "search" => {
                self.search = Some(
                    need(&value)?
                        .split(':')
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .collect(),
                )
            }
            "prefix" => self.prefix = need(&value)?.trim_matches('/').to_string(),
            "cd" => self.cd = Some(PathBuf::from(need(&value)?)),
            "mtime" => {
                let v = need(&value)?;
                self.mtime = if v == "*" {
                    None
                } else {
                    Some(parse_size(&v)? as u32)
                }
            }
            "virtual" => {
                let v = need(&value)?;
                let (cpu, boot) = match v.split_once([',', '/']) {
                    Some((c, b)) => (c.to_string(), b.to_string()),
                    None => (
                        std::env::var("PROCESSOR").unwrap_or_else(|_| "x86_64".into()),
                        v,
                    ),
                };
                let boot = boot.split_whitespace().next().unwrap_or("").to_string();
                self.virtual_ = Some((cpu, boot));
            }
            "physical" => bail!("physical (non-MMU) images are not supported"),
            "image" => self.image = Some(AddrSpace::parse(&need(&value)?)?),
            n if IGNORED_ATTRS.contains(&n) => {}
            n => warn(format!("ignoring unsupported attribute '{n}'")),
        }
        Ok(())
    }
}

fn id_spec(s: &str) -> Result<Option<u32>> {
    if s == "*" {
        Ok(None)
    } else {
        Ok(Some(parse_size(s)? as u32))
    }
}

/// Expand `${NAME}` references from the environment.
pub fn expand_vars(s: &str) -> Result<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let end = rest[i..]
            .find('}')
            .ok_or_else(|| anyhow!("unterminated '${{' in '{s}'"))?;
        let name = &rest[i + 2..i + end];
        let value = match name {
            "PFS" => ":".to_string(),
            _ => std::env::var(name).unwrap_or_default(),
        };
        out.push_str(&value);
        rest = &rest[i + end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Remove one level of double or single quotes and backslash escapes.
pub fn unquote(s: &str) -> String {
    let mut out = String::new();
    let mut quote = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', Some('"')) | ('\\', None) => out.extend(chars.next()),
            ('"' | '\'', None) => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            _ => out.push(c),
        }
    }
    out
}

/// Split a line into whitespace-separated words, keeping quoted parts and
/// `[...]` blocks together; quotes are kept in the words.
pub fn split_words(line: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quote = None;
    let mut depth = 0;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', _) => {
                cur.push(c);
                cur.extend(chars.next());
            }
            ('"' | '\'', None) => {
                quote = Some(c);
                cur.push(c);
            }
            (q, Some(open)) if q == open => {
                quote = None;
                cur.push(c);
            }
            (_, Some(_)) => cur.push(c),
            ('[', None) => {
                depth += 1;
                cur.push(c);
            }
            (']', None) if depth > 0 => {
                depth -= 1;
                cur.push(c);
            }
            (c, None) if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if quote.is_some() {
        bail!("unterminated quote");
    }
    if depth > 0 {
        bail!("unterminated '['");
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    Ok(words)
}

/// Remove a `#` comment (at the start or after whitespace, outside quotes).
pub fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    let mut prev_ws = true;
    for (i, c) in line.char_indices() {
        match (c, quote) {
            ('"' | '\'', None) => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            ('#', None) if prev_ws => return &line[..i],
            _ => {}
        }
        prev_ws = c.is_whitespace();
    }
    line
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// Host file name (searched for) or link text, as written.
    Host(String),
    Inline(String),
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub line: usize,
    pub attrs: Attrs,
    pub target: String,
    /// `None` for a bare `target`: the host file is found by its name.
    pub content: Option<Content>,
}

/// Parse a buildfile. `pre_lines` are processed first (the `-l` option).
pub fn parse(text: &str, pre_lines: &[String], warn: &mut dyn FnMut(String)) -> Result<Vec<Entry>> {
    let mut global = Attrs::default();
    let mut entries = Vec::new();
    let all: Vec<(usize, &str)> = pre_lines
        .iter()
        .map(|l| (0, l.as_str()))
        .chain(text.lines().enumerate().map(|(i, l)| (i + 1, l)))
        .collect();
    let mut i = 0;
    while i < all.len() {
        let (lineno, raw) = all[i];
        i += 1;
        let ctx = || format!("buildfile line {lineno}");
        let line = strip_comment(raw);
        let mut rest = line.trim_start();
        let mut local: Option<Attrs> = None;
        while rest.starts_with('[') {
            let end = block_end(rest).with_context(ctx)?;
            let words = split_words(&rest[1..end]).with_context(ctx)?;
            let attrs = local.get_or_insert_with(|| global.clone());
            for w in words {
                attrs.apply(&w, warn).with_context(ctx)?;
            }
            rest = rest[end + 1..].trim_start();
        }
        let rest = rest.trim_end();
        if rest.is_empty() {
            if let Some(a) = local {
                global = a;
            }
            continue;
        }
        let attrs = local.unwrap_or_else(|| global.clone());
        let (target, content) = match split_spec(rest) {
            Some((t, v)) if v.starts_with('{') => {
                let body = &v[1..];
                let inline = if let Some(single) = body.trim_end().strip_suffix('}') {
                    single.trim().to_string() + "\n"
                } else {
                    let mut s = String::new();
                    if !body.trim().is_empty() {
                        s.push_str(body.trim_start());
                        s.push('\n');
                    }
                    loop {
                        let Some(&(_, l)) = all.get(i) else {
                            bail!("{}: inline file '{t}' is not closed with '}}'", ctx());
                        };
                        i += 1;
                        if l.trim_start().starts_with('}') {
                            break;
                        }
                        s.push_str(l);
                        s.push('\n');
                    }
                    s
                };
                (t, Some(Content::Inline(inline)))
            }
            Some((t, v)) => (
                t,
                Some(Content::Host(expand_vars(&unquote(v)).with_context(ctx)?)),
            ),
            None => (unquote(rest), None),
        };
        entries.push(Entry {
            line: lineno,
            attrs,
            target: expand_vars(&target)?,
            content,
        });
    }
    Ok(entries)
}

/// Index of the `]` closing the block that starts at `s[0]`.
fn block_end(s: &str) -> Result<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices().skip(1) {
        match (c, quote) {
            ('"', None) => quote = Some(c),
            ('"', Some(_)) => quote = None,
            (']', None) => return Ok(i),
            _ => {}
        }
    }
    bail!("unterminated '['")
}

/// Split `target=value` / `target = {` at the first unquoted `=`.
fn split_spec(s: &str) -> Option<(String, &str)> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (c, quote) {
            ('"', None) => quote = Some(c),
            ('"', Some(_)) => quote = None,
            ('=', None) => return Some((unquote(s[..i].trim()), s[i + 1..].trim())),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(text: &str) -> Vec<Entry> {
        parse(text, &[], &mut |w| panic!("warning: {w}")).unwrap()
    }

    /// `?name...` applies only if `name` hasn't been set: globally, or on
    /// an entry against the global attributes (QNX's mkifs, measured).
    #[test]
    fn conditional_attributes() {
        let e = parse_ok("[uid=3]\n[?uid=5] /a\n[?gid=7] /b\n");
        assert_eq!((e[0].attrs.uid, e[0].attrs.gid), (Some(3), None));
        assert_eq!((e[1].attrs.uid, e[1].attrs.gid), (Some(3), Some(7)));
        let e = parse_ok("[?uid=9]\n/c\n");
        assert_eq!(e[0].attrs.uid, Some(9));
    }

    #[test]
    fn files_required_unless_optional() {
        let e = parse_ok("/a\n[+optional] /b\n[+dupignore] /c\n");
        assert!(!e[0].attrs.optional && !e[0].attrs.dupignore);
        assert!(e[1].attrs.optional);
        assert!(e[2].attrs.dupignore);
    }

    #[test]
    fn sizes_and_address_spaces() {
        assert_eq!(parse_size("0x80000").unwrap(), 0x80000);
        assert_eq!(parse_size("4m").unwrap(), 4 << 20);
        assert_eq!(parse_size("0755").unwrap(), 0o755);
        let a = AddrSpace::parse("0x80000-0x100000,1m=2m%4k").unwrap();
        assert_eq!(
            a,
            AddrSpace {
                start: Some(0x80000),
                end: Some(0x100000),
                max_size: Some(1 << 20),
                total_size: Some(2 << 20),
                align: Some(4096)
            }
        );
    }

    #[test]
    fn perms() {
        assert_eq!(Perms::Spec("0644".into()).apply(0o100755).unwrap(), 0o644);
        assert_eq!(Perms::Spec("+x".into()).apply(0o644).unwrap(), 0o755);
        assert_eq!(Perms::Spec("go-w".into()).apply(0o666).unwrap(), 0o644);
        assert_eq!(Perms::Host.apply(0o100744).unwrap(), 0o744);
    }

    #[test]
    fn entries_and_attribute_scope() {
        let e = parse_ok(
            "[image=0x80000]\n\
             [virtual=aarch64le,raw] .boot = {\n  startup -v\n  procnto\n}\n\
             [uid=0 gid=0]\n\
             [type=link] /bin/sh=/bin/ksh\n\
             /bin/wdtkick=wdtkick   # comment\n\
             libc.so\n\
             [perms=0644] /etc/motd = { hello }\n",
        );
        assert_eq!(e.len(), 5);
        assert_eq!(e[0].target, ".boot");
        assert_eq!(
            e[0].attrs.virtual_,
            Some(("aarch64le".into(), "raw".into()))
        );
        assert_eq!(e[0].attrs.image.as_ref().unwrap().start, Some(0x80000));
        assert_eq!(
            e[0].content,
            Some(Content::Inline("  startup -v\n  procnto\n".into()))
        );
        assert_eq!(e[1].attrs.file_type, FileType::Link);
        assert_eq!(e[1].content, Some(Content::Host("/bin/ksh".into())));
        assert_eq!(
            e[2].attrs.file_type,
            FileType::File,
            "[type=link] was local"
        );
        assert_eq!(e[2].attrs.uid, Some(0), "[uid=0] is global");
        assert_eq!(e[2].content, Some(Content::Host("wdtkick".into())));
        assert_eq!(e[3].target, "libc.so");
        assert_eq!(e[3].content, None);
        assert_eq!(e[4].content, Some(Content::Inline("hello\n".into())));
    }

    #[test]
    fn words_keep_quotes_and_blocks() {
        assert_eq!(
            split_words(r#"[+session] ksh -c "echo a b" &"#).unwrap(),
            vec!["[+session]", "ksh", "-c", "\"echo a b\"", "&"]
        );
        assert_eq!(unquote(r#""echo a b""#), "echo a b");
    }
}
