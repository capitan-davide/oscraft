//! Startup's command line: the arguments in the boot-arguments area, and a
//! `getopt()`-style option parser over them.

use crate::abi::startup::BootArgsEntry;
use core::mem::size_of;

/// The most arguments (and environment strings) startup keeps.
pub const MAX_ARGS: usize = 32;

/// The parsed boot arguments.
#[derive(Debug)]
pub struct CmdLine<'a> {
    /// Address of the startup header.
    pub shdr_addr: u64,
    pub argv: Strings<'a>,
    pub envv: Strings<'a>,
}

/// A list of strings, borrowed from the boot-arguments area.
#[derive(Clone, Copy, Debug)]
pub struct Strings<'a> {
    items: [&'a str; MAX_ARGS],
    len: usize,
}

impl<'a> Strings<'a> {
    pub const fn new() -> Self {
        Self {
            items: [""; MAX_ARGS],
            len: 0,
        }
    }

    pub fn as_slice(&self) -> &[&'a str] {
        &self.items[..self.len]
    }

    /// Add a string; beyond [`MAX_ARGS`] strings, it is dropped.
    pub fn push(&mut self, s: &'a str) {
        if self.len < MAX_ARGS {
            self.items[self.len] = s;
            self.len += 1;
        }
    }
}

impl Default for Strings<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Why the boot arguments could not be read.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// mkifs did not fill in the area (the signature is still there).
    NotFilled,
    /// The entry claims more bytes than the area has.
    BadSize,
    /// A string is not terminated, or not UTF-8.
    BadString,
}

impl<'a> CmdLine<'a> {
    /// Parse the boot-arguments area that mkifs filled in.
    pub fn parse(area: &'a [u8]) -> Result<Self, Error> {
        if area.starts_with(b"ddpvbskr") {
            return Err(Error::NotFilled);
        }
        let hdr_size = size_of::<BootArgsEntry>();
        let size = usize::from(area[0]) | usize::from(area[1]) << 8;
        if size < hdr_size || size > area.len() {
            return Err(Error::BadSize);
        }
        let argc = area[2] as i8;
        let envc = area[3] as i8;
        let shdr = u32::from_le_bytes([area[4], area[5], area[6], area[7]]);
        let shdr_addr = if shdr == crate::abi::startup::SHDR_ADDR_64 {
            // The real address is in the entry's last 8 bytes.
            let mut b = [0; 8];
            b.copy_from_slice(&area[size - 8..size]);
            u64::from_le_bytes(b)
        } else {
            u64::from(shdr)
        };
        let mut strings = area[hdr_size..size].split(|&b| b == 0);
        let mut take = |count: i8| -> Result<Strings<'a>, Error> {
            let mut list = Strings::new();
            for _ in 0..count.max(0) {
                let s = strings.next().ok_or(Error::BadString)?;
                list.push(core::str::from_utf8(s).map_err(|_| Error::BadString)?);
            }
            Ok(list)
        };
        let argv = take(argc)?;
        let envv = take(envc)?;
        Ok(Self {
            shdr_addr,
            argv,
            envv,
        })
    }
}

/// One option found by [`Getopt`].
#[derive(Debug, PartialEq, Eq)]
pub enum Opt<'a> {
    /// An option letter, with its argument if `optstring` says it has one.
    Flag(char, Option<&'a str>),
    /// An option letter not in `optstring`, or missing its argument.
    Unknown(char),
}

/// Options in `getopt()` style: `-v`, `-vvv`, `-D miniuart` or
/// `-Dminiuart`. The option strings list the option letters; a letter
/// followed by `:` takes an argument. Parsing stops at the first argument
/// that is not an option. `argv[0]` is the program's name and is skipped.
pub struct Getopt<'a> {
    args: Strings<'a>,
    optstrings: [&'static str; 2],
    /// Index of the argument being parsed, and position in it.
    index: usize,
    pos: usize,
}

impl<'a> Getopt<'a> {
    /// Parse `args` for the letters of both option strings (the
    /// library's common options and a board's own).
    pub fn new(args: Strings<'a>, optstrings: [&'static str; 2]) -> Self {
        Self {
            args,
            optstrings,
            index: 1,
            pos: 0,
        }
    }

    fn takes_argument(&self, c: char) -> Option<bool> {
        self.optstrings.iter().find_map(|o| {
            let i = o.find(c)?;
            Some(o[i + c.len_utf8()..].starts_with(':'))
        })
    }
}

impl<'a> Iterator for Getopt<'a> {
    type Item = Opt<'a>;

    fn next(&mut self) -> Option<Opt<'a>> {
        let args = self.args.as_slice();
        let arg = *args.get(self.index)?;
        if self.pos == 0 {
            if arg.len() < 2 || !arg.starts_with('-') {
                return None;
            }
            self.pos = 1;
        }
        let c = arg[self.pos..].chars().next()?;
        self.pos += c.len_utf8();
        let rest = &arg[self.pos..];
        let opt = match self.takes_argument(c) {
            None => Opt::Unknown(c),
            Some(false) => Opt::Flag(c, None),
            Some(true) if !rest.is_empty() => {
                self.pos = arg.len();
                Opt::Flag(c, Some(rest))
            }
            Some(true) => match args.get(self.index + 1) {
                Some(next) => {
                    self.index += 1;
                    Opt::Flag(c, Some(next))
                }
                None => Opt::Unknown(c),
            },
        };
        if self.pos >= arg.len() {
            self.index += 1;
            self.pos = 0;
        }
        Some(opt)
    }
}

/// Parse a number as `strtoul(.., 0)` does: decimal, `0x` hex or `0`
/// octal. Returns the value and the rest of the string.
pub fn parse_number(s: &str) -> (u64, &str) {
    let (radix, digits) = if let Some(hex) = s.strip_prefix("0x").or(s.strip_prefix("0X")) {
        (16, hex)
    } else if s.len() > 1 && s.starts_with('0') {
        (8, &s[1..])
    } else {
        (10, s)
    };
    let end = digits
        .find(|c: char| !c.is_digit(radix))
        .unwrap_or(digits.len());
    let value = u64::from_str_radix(&digits[..end], radix).unwrap_or(0);
    (value, &digits[end..])
}

/// Parse a size as startup's `getsize()` does: a number with an optional
/// `k`, `m` or `g` suffix (powers of 1024).
pub fn parse_size(s: &str) -> (u64, &str) {
    let (value, rest) = parse_number(s);
    let shift = match rest.chars().next() {
        Some('k' | 'K') => 10,
        Some('m' | 'M') => 20,
        Some('g' | 'G') => 30,
        _ => return (value, rest),
    };
    (value << shift, &rest[1..])
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// A boot-arguments entry as mkifs writes it.
    fn entry(shdr: u32, args: &[&str], env: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&[0, 0, args.len() as u8, env.len() as u8]);
        v.extend_from_slice(&shdr.to_le_bytes());
        for s in args.iter().chain(env) {
            v.extend_from_slice(s.as_bytes());
            v.push(0);
        }
        let size = v.len() as u16;
        v[..2].copy_from_slice(&size.to_le_bytes());
        v.resize(256, 0);
        v
    }

    #[test]
    fn parses_args_and_env() {
        let area = entry(0x80fa0, &["startup-x", "-vv", "-D", "miniuart"], &["A=1"]);
        let cmd = CmdLine::parse(&area).unwrap();
        assert_eq!(cmd.shdr_addr, 0x80fa0);
        assert_eq!(cmd.argv.as_slice(), ["startup-x", "-vv", "-D", "miniuart"]);
        assert_eq!(cmd.envv.as_slice(), ["A=1"]);
    }

    #[test]
    fn rejects_an_unfilled_area() {
        assert_eq!(
            CmdLine::parse(b"ddpvbskr\0\0\0\0").unwrap_err(),
            Error::NotFilled
        );
    }

    #[test]
    fn getopt_like_c() {
        let mut argv = Strings::new();
        for a in [
            "startup", "-vv", "-D", "miniuart", "-Wx", "-u", "reg", "-q", "rest",
        ] {
            argv.push(a);
        }
        let opts: Vec<_> = Getopt::new(argv, ["vD:u:", "W:"]).collect();
        assert_eq!(
            opts,
            [
                Opt::Flag('v', None),
                Opt::Flag('v', None),
                Opt::Flag('D', Some("miniuart")),
                Opt::Flag('W', Some("x")),
                Opt::Flag('u', Some("reg")),
                Opt::Unknown('q'),
            ]
        );
    }

    #[test]
    fn numbers_and_sizes() {
        assert_eq!(parse_number("0x1f000000,"), (0x1f00_0000, ","));
        assert_eq!(parse_number("2500"), (2500, ""));
        assert_eq!(parse_number("010"), (8, ""));
        assert_eq!(parse_size("4k,1"), (4096, ",1"));
        assert_eq!(parse_size("16M"), (16 << 20, ""));
    }
}
