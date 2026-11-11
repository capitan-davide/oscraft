//! The kernel's command line: the `struct bootargs_entry` that mkifs writes
//! into `boot_args` (see `arch/aarch64/boot.s`).

unsafe extern "C" {
    /// Buffer in the kernel's data, filled in by mkifs.
    static boot_args: [u8; 1024];
}

pub struct BootArgs {
    bytes: &'static [u8],
    argc: usize,
    envc: usize,
    /// Physical address of the startup header.
    pub shdr_addr: u32,
}

impl BootArgs {
    /// `None` if mkifs left the buffer empty.
    pub fn get() -> Option<Self> {
        // SAFETY: boot_args is written once at image build time and only read.
        let buf: &'static [u8] = unsafe { &*core::ptr::addr_of!(boot_args) };
        let size = (buf[0] as usize | (buf[1] as usize) << 8).min(buf.len());
        if size < 8 {
            return None;
        }
        Some(Self {
            bytes: &buf[8..size],
            argc: buf[2] as usize,
            envc: buf[3] as usize,
            shdr_addr: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        })
    }

    fn strings(&self) -> impl Iterator<Item = &'static str> {
        self.bytes
            .split(|&b| b == 0)
            .map(|s| core::str::from_utf8(s).unwrap_or("?"))
    }

    pub fn argv(&self) -> impl Iterator<Item = &'static str> {
        self.strings().take(self.argc)
    }

    pub fn env(&self) -> impl Iterator<Item = &'static str> {
        self.strings().skip(self.argc).take(self.envc)
    }

    /// Whether to run OsCraft's own self-tests at boot: `OSCRAFT_SELFTEST=1`
    /// in the kernel's environment (the image's `.boot` line, or
    /// `make ... SELFTEST=1`). Off by default: procnto has none, and starts
    /// the boot script at once (review CORE-18). A variable, not an
    /// option letter, so that it can't collide with one of procnto's.
    pub fn selftests(&self) -> bool {
        self.env().any(|e| e == "OSCRAFT_SELFTEST=1")
    }

    /// Whether procnto's option `opt` (e.g. "-U") is given.
    pub fn flag(&self, opt: &str) -> bool {
        self.argv().any(|a| a == opt)
    }

    /// The value of procnto's option `letter` (`-Cvalue` or `-C value`),
    /// its last one if given more than once.
    pub fn option(&self, letter: char) -> Option<&'static str> {
        let mut found = None;
        let mut args = self.argv().skip(1);
        while let Some(a) = args.next() {
            let Some(rest) = a.strip_prefix('-').and_then(|r| r.strip_prefix(letter)) else {
                continue;
            };
            found = if rest.is_empty() { args.next() } else { Some(rest) };
        }
        found
    }

    /// procnto's `-C tick_frequency[,clock_IST_priority]`: the tick in Hz
    /// (0: 1000, at most 100000) and the priority of the clock interrupt
    /// thread (0: 254, at most 254). Numbers as `strtoul(.., 0)` reads
    /// them. `Err` with procnto's complaint for a value out of range or a
    /// malformed string.
    pub fn clock(&self) -> Result<(u32, u8), &'static str> {
        let Some(s) = self.option('C') else {
            return Ok((1000, 254));
        };
        let (freq, rest) = strtoul(s);
        let hz = match freq {
            0 => 1000,
            f if f > 100_000 => return Err("requested clock frequency too fast (max 100000 ticks/s)"),
            f => f as u32,
        };
        let mut prio = 254;
        let mut rest = rest;
        if let Some(r) = rest.strip_prefix(',') {
            let (p, r) = strtoul(r);
            prio = match p {
                0 => 254,
                p if p > 254 => return Err("requested clock IST priority too high (max 254)"),
                p => p as u8,
            };
            rest = r.strip_prefix(',').unwrap_or(r);
        }
        if !rest.is_empty() {
            return Err("invalid clock (-C) configuration string");
        }
        Ok((hz, prio))
    }

    /// A memory-manager aspect of procnto's `-m` option: `Some(true)` if
    /// the last `-m` string enables `letter`, `Some(false)` if it disables
    /// it (`~letter`), `None` if it doesn't mention it. As in procnto, only
    /// the last `-m` counts, and within it the last mention of a letter.
    pub fn memmgr(&self, letter: char) -> Option<bool> {
        let config = self.argv().filter_map(|a| a.strip_prefix("-m")).last()?;
        let mut found = None;
        let mut negate = false;
        for c in config.chars() {
            if c == '~' {
                negate = true;
                continue;
            }
            if c == letter {
                found = Some(!negate);
            }
            negate = false;
        }
        found
    }
}

/// `strtoul(s, &end, 0)`: a decimal, `0x` hexadecimal or `0` octal
/// number at the start of `s` (0 if none), and the rest of `s`.
fn strtoul(s: &str) -> (u64, &str) {
    let (radix, digits) = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (16, h)
    } else if s.len() > 1 && s.starts_with('0') {
        (8, &s[1..])
    } else {
        (10, s)
    };
    let end = digits.find(|c: char| !c.is_digit(radix)).unwrap_or(digits.len());
    if end == 0 {
        // "0x" without hex digits reads as the 0; nothing at all as 0.
        return if s.starts_with('0') { (0, &s[1..]) } else { (0, s) };
    }
    let v = digits[..end]
        .chars()
        .fold(0u64, |v, c| v.saturating_mul(radix as u64).saturating_add(c.to_digit(radix).unwrap() as u64));
    (v, &digits[end..])
}
