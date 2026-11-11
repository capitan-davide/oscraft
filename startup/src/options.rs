//! The common options: what the letters of [`COMMON_OPTIONS`] mean.
//!
//! Parsing only records what each option asks for, in [`Options`];
//! [`crate::Startup`] applies it when the time comes. Some options take
//! effect at once (the debug console), most at the end of the boot (RAM
//! to reserve, system page flags).
//!
//! [`COMMON_OPTIONS`]: crate::COMMON_OPTIONS

use crate::cmdline::{Opt, parse_number, parse_size};

/// Room for this many of each repeatable option.
pub const MAX_REPEAT: usize = 8;

/// A fixed-capacity list, for repeatable options.
#[derive(Clone, Copy, Debug)]
pub struct Repeat<T: Copy + Default> {
    items: [T; MAX_REPEAT],
    len: usize,
}

impl<T: Copy + Default> Default for Repeat<T> {
    fn default() -> Self {
        Self {
            items: [T::default(); MAX_REPEAT],
            len: 0,
        }
    }
}

impl<T: Copy + Default> Repeat<T> {
    fn push(&mut self, item: T) -> bool {
        if self.len == MAX_REPEAT {
            return false;
        }
        self.items[self.len] = item;
        self.len += 1;
        true
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len]
    }
}

/// `-r addr,size[,keep]`: RAM to keep from the kernel, cleared unless
/// `keep` is 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reserved {
    pub addr: u64,
    pub size: u64,
    pub clear: bool,
}

/// `-R size[,align[,name[,low]]]`: RAM to set aside under a name, from the
/// top of RAM (or the bottom, with `low` not 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Named {
    pub size: u64,
    pub align: u64,
    pub name: &'static str,
    pub low: bool,
}

/// `-c name:mask`: a CPU cluster for the scheduler.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cluster {
    pub name: &'static str,
    pub cpumask: u64,
}

/// What the common options asked for.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// `-v`, repeated.
    pub debug_level: u32,
    /// `-P n`: start at most n CPUs.
    pub max_cpus: Option<usize>,
    /// `-N name`: the host name.
    pub hostname: Option<&'static str>,
    /// `-D`, `-K`: the debug channels' devices (`name[.options]`).
    pub debug: [Option<&'static str>; 2],
    /// `-u reg|arg`: where the device tree's address is.
    pub fdt: Option<&'static str>,
    /// `-r`: RAM to keep from the kernel.
    pub reserved: Repeat<Reserved>,
    /// `-R`: RAM to set aside under a name.
    pub named: Repeat<Named>,
    /// `-M addr,size`: more RAM.
    pub extra_ram: Repeat<(u64, u64)>,
    /// `-c`: CPU clusters.
    pub clusters: Repeat<Cluster>,
    /// `-f cpu[,cycles[,timer]]`: frequencies in Hz, instead of the ones
    /// the board finds.
    pub cpu_freq: Option<u64>,
    pub timer_freq: Option<u64>,
    /// `-j addr`: where to store the system page's physical address, for a
    /// JTAG debugger.
    pub jtag: Option<u64>,
    /// `-o loop,overhead`: nanospin calibration, for the C library.
    pub nanospin: Option<(u32, u32)>,
    /// `-A`: tell the kernel the last reboot was abnormal.
    pub abnormal_reboot: bool,
    /// `-Z`: the kernel runs without a periodic tick.
    pub tickless: bool,
    /// `-E meltdown`: no global mappings.
    pub meltdown: bool,
    /// `-E`, `-e`, `-U`, `-X cachesnooped`, `-F`: see
    /// [`crate::arch::aarch64::cpu::Features`].
    pub sve: bool,
    pub pauth: bool,
    pub spectre_v2: bool,
    pub ssbs: bool,
    pub cache_snooped: bool,
    pub flags_on: u64,
    pub flags_off: u64,
}

/// Why an option was not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The option has a value this startup does not accept.
    BadValue(char, &'static str),
    /// The option asks for something this startup does not do.
    Unsupported(char, &'static str),
    /// More of a repeatable option than there is room for.
    TooMany(char),
    /// The letter is not an option.
    Unknown(char),
}

impl core::fmt::Display for Problem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Problem::BadValue(c, v) => write!(f, "-{c}: bad value '{v}'"),
            Problem::Unsupported(c, v) => write!(f, "-{c} {v}: not supported, ignored"),
            Problem::TooMany(c) => write!(f, "-{c}: too many, ignored"),
            Problem::Unknown(c) => write!(f, "unknown option -{c}"),
        }
    }
}

impl Options {
    /// Record one option. Returns what was wrong with it, if anything.
    pub fn apply(&mut self, opt: &Opt<'static>) -> Result<(), Problem> {
        let (c, arg) = match *opt {
            Opt::Flag(c, arg) => (c, arg.unwrap_or("")),
            Opt::Unknown(c) => return Err(Problem::Unknown(c)),
        };
        match c {
            'v' => self.debug_level += 1,
            'P' => self.max_cpus = Some((parse_size(arg).0 as usize).max(1)),
            'N' => self.hostname = Some(arg),
            'D' => self.debug[0] = Some(arg),
            'K' => self.debug[1] = Some(arg),
            'u' => self.fdt = Some(arg),
            'r' => {
                let (addr, rest) = parse_size(arg);
                let Some(rest) = rest.strip_prefix(',') else {
                    return Err(Problem::BadValue(c, arg));
                };
                let (size, rest) = parse_size(rest);
                let keep = rest
                    .strip_prefix(',')
                    .is_some_and(|k| parse_number(k).0 == 1);
                if !self.reserved.push(Reserved {
                    addr,
                    size,
                    clear: !keep,
                }) {
                    return Err(Problem::TooMany(c));
                }
            }
            'R' => {
                let mut fields = arg.split(',');
                let size = parse_size(fields.next().unwrap_or("")).0;
                let align = fields.next().map_or(0, |a| parse_size(a).0);
                let name = fields
                    .next()
                    .filter(|n| !n.is_empty())
                    .unwrap_or("reserved");
                let low = fields.next().is_some_and(|l| parse_number(l).0 != 0);
                if !self.named.push(Named {
                    size,
                    align,
                    name,
                    low,
                }) {
                    return Err(Problem::TooMany(c));
                }
            }
            'M' => {
                let (addr, rest) = parse_size(arg);
                let Some(size) = rest.strip_prefix(',') else {
                    return Err(Problem::BadValue(c, arg));
                };
                if !self.extra_ram.push((addr, parse_size(size).0)) {
                    return Err(Problem::TooMany(c));
                }
            }
            'c' => {
                for cluster in arg.split(',') {
                    let Some((name, mask)) = cluster.split_once(':') else {
                        return Err(Problem::BadValue(c, arg));
                    };
                    let cpumask = parse_number(mask).0;
                    // Names starting with '_' are the kernel's own.
                    if name.starts_with('_') || cpumask == 0 {
                        return Err(Problem::BadValue(c, arg));
                    }
                    if !self.clusters.push(Cluster { name, cpumask }) {
                        return Err(Problem::TooMany(c));
                    }
                }
            }
            'f' => {
                let mut fields = arg.split(',');
                self.cpu_freq = fields.next().and_then(parse_freq).or(self.cpu_freq);
                // The cycles frequency is the timer's on AArch64.
                let _cycles = fields.next();
                self.timer_freq = fields.next().and_then(parse_freq).or(self.timer_freq);
            }
            'j' => self.jtag = Some(parse_size(arg).0),
            'o' => {
                let (lp, rest) = parse_number(arg);
                let Some(overhead) = rest.strip_prefix(',') else {
                    return Err(Problem::BadValue(c, arg));
                };
                self.nanospin = Some((lp as u32, parse_number(overhead).0 as u32));
            }
            'A' => self.abnormal_reboot = true,
            'Z' => self.tickless = true,
            'U' => self.pauth = true,
            'F' => match arg.strip_prefix('~') {
                Some(off) => {
                    let f = parse_number(off).0;
                    self.flags_off |= f;
                    self.flags_on &= !f;
                }
                None => {
                    let f = parse_number(arg).0;
                    self.flags_on |= f;
                    self.flags_off &= !f;
                }
            },
            'E' => {
                for errata in arg.split(',') {
                    match errata {
                        "meltdown" => self.meltdown = true,
                        "~meltdown" => self.meltdown = false,
                        "spectrev2" => self.spectre_v2 = true,
                        "~spectrev2" => self.spectre_v2 = false,
                        "ssbs" => self.ssbs = true,
                        "~ssbs" => self.ssbs = false,
                        _ => return Err(Problem::BadValue(c, errata)),
                    }
                }
            }
            'e' => match arg {
                "sve" => self.sve = true,
                _ => return Err(Problem::BadValue(c, arg)),
            },
            'X' => {
                for x in arg.split(',') {
                    match x.split_once('=') {
                        None if x == "cachesnooped" => self.cache_snooped = true,
                        // Only 4 KB pages.
                        Some(("pagesize", p))
                            if u64::from_str_radix(p.trim_start_matches("0x"), 16)
                                == Ok(0x1000) => {}
                        Some(("pagesize", _)) => return Err(Problem::Unsupported(c, x)),
                        _ => return Err(Problem::BadValue(c, x)),
                    }
                }
            }
            // QNX Hypervisor, and an image file system kept across a warm
            // reset: not supported.
            'Q' if arg == "disable" => {}
            'Q' => return Err(Problem::Unsupported(c, arg)),
            'I' | 'i' => return Err(Problem::Unsupported(c, arg)),
            // -C (secure system), -H (running under a hypervisor, for its
            // PSCI reboot), -S, -T (no boot time: there is none without a
            // clock) change nothing here.
            'C' | 'H' | 'S' | 'T' => {}
            _ => return Err(Problem::Unknown(c)),
        }
        Ok(())
    }
}

/// A frequency as QNX's startup reads it: Hz, or with a `k`, `m` (or `h`)
/// suffix; a plain number below 1000 is MHz.
fn parse_freq(s: &str) -> Option<u64> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let n: u64 = s[..end].parse().ok()?;
    Some(match s[end..].chars().next() {
        Some('m' | 'M') => n * 1_000_000,
        Some('k' | 'K') => n * 1_000,
        Some('h' | 'H') => n,
        _ if n < 1000 => n * 1_000_000,
        _ => n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[(char, &'static str)]) -> (Options, Option<Problem>) {
        let mut o = Options::default();
        let mut problem = None;
        for &(c, arg) in args {
            if let Err(p) = o.apply(&Opt::Flag(c, Some(arg))) {
                problem = Some(p);
            }
        }
        (o, problem)
    }

    #[test]
    fn ram_options() {
        let (o, p) = parse(&[
            ('r', "0,4096,1"),
            ('r', "0x1000000,1m"),
            ('R', "2m,0x200000,frame_buffer,1"),
            ('M', "0x100000000,4g"),
        ]);
        assert_eq!(p, None);
        assert_eq!(
            o.reserved.as_slice(),
            [
                Reserved {
                    addr: 0,
                    size: 4096,
                    clear: false
                },
                Reserved {
                    addr: 0x100_0000,
                    size: 1 << 20,
                    clear: true
                }
            ]
        );
        assert_eq!(
            o.named.as_slice(),
            [Named {
                size: 2 << 20,
                align: 0x20_0000,
                name: "frame_buffer",
                low: true
            }]
        );
        assert_eq!(o.extra_ram.as_slice(), [(1 << 32, 4 << 30)]);
        assert_eq!(parse(&[('R', "4k")]).0.named.as_slice()[0].name, "reserved");
    }

    #[test]
    fn cpu_options() {
        let (o, p) = parse(&[
            ('E', "spectrev2,ssbs"),
            ('E', "~ssbs"),
            ('e', "sve"),
            ('X', "cachesnooped,pagesize=1000"),
            ('F', "0x100"),
            ('F', "~0x40"),
        ]);
        assert_eq!(p, None);
        assert!(o.spectre_v2 && !o.ssbs && o.sve && o.cache_snooped);
        assert_eq!((o.flags_on, o.flags_off), (0x100, 0x40));
        assert_eq!(
            parse(&[('X', "pagesize=10000")]).1,
            Some(Problem::Unsupported('X', "pagesize=10000"))
        );
    }

    #[test]
    fn frequencies_clusters_and_more() {
        let (o, p) = parse(&[
            ('f', "700,,54m"),
            ('c', "big:0xc,little:3"),
            ('j', "0x1000"),
            ('o', "500,20"),
            ('A', ""),
            ('Z', ""),
        ]);
        assert_eq!(p, None);
        assert_eq!(
            (o.cpu_freq, o.timer_freq),
            (Some(700_000_000), Some(54_000_000))
        );
        assert_eq!(
            o.clusters.as_slice(),
            [
                Cluster {
                    name: "big",
                    cpumask: 0xc
                },
                Cluster {
                    name: "little",
                    cpumask: 3
                }
            ]
        );
        assert_eq!(
            (o.jtag, o.nanospin, o.abnormal_reboot, o.tickless),
            (Some(0x1000), Some((500, 20)), true, true)
        );
        assert_eq!(
            parse(&[('c', "_kernel:1")]).1,
            Some(Problem::BadValue('c', "_kernel:1"))
        );
        assert_eq!(
            parse(&[('Q', "enable")]).1,
            Some(Problem::Unsupported('Q', "enable"))
        );
    }
}
