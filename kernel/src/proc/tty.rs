//! A terminal line discipline (what QNX's `io-char` library does for
//! character devices), as far as the text console needs one: `struct
//! termios` settings, canonical input with line editing and echo, and raw
//! input for programs that edit lines themselves (ksh).
//!
//! The settings are FreeBSD's `struct termios` (`<sys/_termios.h>`): flags,
//! `c_cc[20]` at 16, `c_ispeed` at 36, `c_ospeed` at 40; 44 bytes.
//! `personality::qnx` lays them out for a QNX process.
//!
//! Not yet: flow control, output
//! processing (the console always turns "\n" into "\r\n").

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::tprint;

pub const TERMIOS_SIZE: usize = 44;
const NCCS: usize = 20;

// c_iflag
const BRKINT: u32 = 0x2;
const INLCR: u32 = 0x40;
const IGNCR: u32 = 0x80;
const ICRNL: u32 = 0x100;
const IXON: u32 = 0x200;
const IMAXBEL: u32 = 0x2000;
// c_oflag
const OPOST: u32 = 0x1;
const ONLCR: u32 = 0x2;
// c_cflag
const CS8: u32 = 0x300;
const CREAD: u32 = 0x800;
// c_lflag
const ISIG: u32 = 0x80;
const ICANON: u32 = 0x100;
const ECHO: u32 = 0x8;
const ECHOE: u32 = 0x2;
const ECHOK: u32 = 0x4;
const ECHONL: u32 = 0x10;
const ECHOCTL: u32 = 0x40;
const IEXTEN: u32 = 0x400;
// c_cc indices
const VEOF: usize = 0;
const VEOL: usize = 1;
const VERASE: usize = 3;
const VWERASE: usize = 4;
const VKILL: usize = 5;
const VINTR: usize = 8;
const VQUIT: usize = 9;
const VSUSP: usize = 10;
const VMIN: usize = 16;
const VTIME: usize = 17;
const B115200: u32 = 115200;
// <signal.h>
const SIGINT: usize = 2;
const SIGQUIT: usize = 3;
const SIGTSTP: usize = 18;
/// `_POSIX_VDISABLE`: a control character set to this is disabled.
const VDISABLE: u8 = 0;

/// io-char's buffer sizes (the defaults of its `-C` and `-I` options, in
/// the usage text of any `devc-*` driver): the line being edited holds at
/// most `CANON_MAX` bytes, its terminator included; input waiting to be
/// read at most `INPUT_MAX`. A byte that doesn't fit is dropped (a bell
/// echoed under IMAXBEL), so that typing can't grow the kernel's heap
/// without bound (review PROCMGR-16).
const CANON_MAX: usize = 256;
pub(super) const INPUT_MAX: usize = 2048;

pub struct Tty {
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    cc: [u8; NCCS],
    speed: u32,
    /// The line being edited (canonical mode).
    line: Vec<u8>,
    /// Input ready to be read: complete lines, or raw bytes.
    ready: VecDeque<u8>,
    /// Lines completed in `ready` (an end-of-file is an empty line).
    lines: VecDeque<usize>,
    /// Foreground process group (`tcsetpgrp()`).
    pub pgrp: i32,
    /// QNX's last settings: the characters FreeBSD has no slot for.
    pub qnx_base: [u8; 76],
}

impl Tty {
    /// A terminal in the usual cooked mode.
    pub fn new() -> Self {
        let mut cc = [VDISABLE; NCCS];
        cc[VINTR] = 0x03; // ^C
        cc[VQUIT] = 0x1c; // ^\
        cc[VERASE] = 0x7f; // DEL
        cc[VKILL] = 0x15; // ^U
        cc[VEOF] = 0x04; // ^D
        cc[VSUSP] = 0x1a; // ^Z
        cc[VWERASE] = 0x17; // ^W
        cc[VMIN] = 1;
        cc[VTIME] = 0;
        Self {
            iflag: BRKINT | ICRNL | IXON,
            oflag: OPOST | ONLCR,
            cflag: CS8 | CREAD,
            lflag: ISIG | ICANON | ECHO | ECHOE | ECHOK | ECHOCTL | IEXTEN,
            cc,
            speed: B115200,
            line: Vec::new(),
            ready: VecDeque::new(),
            lines: VecDeque::new(),
            pgrp: 0,
            qnx_base: [0; 76],
        }
    }

    pub fn get_attr(&self) -> [u8; TERMIOS_SIZE] {
        let mut t = [0u8; TERMIOS_SIZE];
        for (o, v) in [
            (0, self.iflag),
            (4, self.oflag),
            (8, self.cflag),
            (12, self.lflag),
        ] {
            t[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        t[16..16 + NCCS].copy_from_slice(&self.cc);
        t[36..40].copy_from_slice(&self.speed.to_le_bytes());
        t[40..44].copy_from_slice(&self.speed.to_le_bytes());
        t
    }

    /// `tcsetattr()`. `flush` drops pending input (TCSAFLUSH).
    pub fn set_attr(&mut self, t: &[u8], flush: bool) {
        if t.len() < TERMIOS_SIZE {
            return;
        }
        let u = |o: usize| u32::from_le_bytes(t[o..o + 4].try_into().unwrap());
        let was_canon = self.canonical();
        (self.iflag, self.oflag, self.cflag, self.lflag) = (u(0), u(4), u(8), u(12));
        self.cc.copy_from_slice(&t[16..16 + NCCS]);
        if flush {
            self.line.clear();
            self.ready.clear();
            self.lines.clear();
        } else if was_canon && !self.canonical() {
            // A partial line becomes readable as it is.
            self.ready.extend(self.line.drain(..));
            self.lines.clear();
        }
    }

    fn canonical(&self) -> bool {
        self.lflag & ICANON != 0
    }

    fn echo(&self, b: u8) {
        if self.lflag & ECHO == 0 {
            return;
        }
        if b < 0x20 && b != b'\n' && b != b'\t' && self.lflag & ECHOCTL != 0 {
            tprint!("^{}", (b + b'@') as char);
        } else {
            tprint!("{}", b as char);
        }
    }

    fn is(&self, b: u8, i: usize) -> bool {
        self.cc[i] != VDISABLE && b == self.cc[i]
    }

    /// Erase the last character of the line being edited.
    fn erase(&mut self) -> bool {
        if self.line.pop().is_none() {
            return false;
        }
        if self.lflag & ECHO != 0 && self.lflag & ECHOE != 0 {
            tprint!("\x08 \x08");
        }
        true
    }

    /// Whether `n` more bytes fit in the input; if not, the byte is
    /// dropped, with a bell under IMAXBEL.
    fn room(&self, n: usize) -> bool {
        if self.ready.len() + self.line.len() + n <= INPUT_MAX {
            return true;
        }
        if self.iflag & IMAXBEL != 0 {
            tprint!("\x07");
        }
        false
    }

    fn end_line(&mut self) {
        self.lines.push_back(self.line.len());
        self.ready.extend(self.line.drain(..));
    }

    /// A byte typed on the terminal. Returns the signal it raises for the
    /// foreground process group, if any (ISIG: VINTR is SIGINT, VQUIT
    /// SIGQUIT, VSUSP SIGTSTP).
    pub fn input(&mut self, mut b: u8) -> Option<usize> {
        if self.lflag & ISIG != 0 {
            let signal = [(VINTR, SIGINT), (VQUIT, SIGQUIT), (VSUSP, SIGTSTP)]
                .into_iter()
                .find(|&(c, _)| self.is(b, c))
                .map(|(_, sig)| sig);
            if let Some(sig) = signal {
                // Pending input is flushed, as without NOFLSH.
                self.echo(b);
                tprint!("\n");
                self.line.clear();
                self.ready.clear();
                self.lines.clear();
                return Some(sig);
            }
        }
        if b == b'\r' {
            if self.iflag & IGNCR != 0 {
                return None;
            }
            if self.iflag & ICRNL != 0 {
                b = b'\n';
            }
        } else if b == b'\n' && self.iflag & INLCR != 0 {
            b = b'\r';
        }
        if !self.canonical() {
            if !self.room(1) {
                return None;
            }
            self.echo(b);
            self.ready.push_back(b);
            return None;
        }
        if self.is(b, VERASE) || b == 0x08 {
            self.erase();
        } else if self.is(b, VKILL) {
            while self.erase() {}
            if self.lflag & ECHOK != 0 && self.lflag & ECHOE == 0 {
                tprint!("\n");
            }
        } else if self.lflag & IEXTEN != 0 && self.is(b, VWERASE) {
            while self.line.last() == Some(&b' ') && self.erase() {}
            while self.line.last().is_some_and(|&c| c != b' ') && self.erase() {}
        } else if self.is(b, VEOF) {
            self.end_line();
        } else if b == b'\n' || self.is(b, VEOL) {
            if !self.room(1) {
                return None;
            }
            if self.lflag & (ECHO | ECHONL) != 0 {
                tprint!("\n");
            }
            self.line.push(b);
            self.end_line();
        } else {
            // One place is kept for the line's terminator.
            if self.line.len() + 2 > CANON_MAX {
                if self.iflag & IMAXBEL != 0 {
                    tprint!("\x07");
                }
                return None;
            }
            if !self.room(2) {
                return None;
            }
            self.echo(b);
            self.line.push(b);
        }
        None
    }

    /// Give back input a read did not take after all (its reader left).
    pub fn unread(&mut self, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        if self.canonical() {
            self.lines.push_front(data.len());
        }
        for b in data.into_iter().rev() {
            self.ready.push_front(b);
        }
    }

    /// Whether `read()` would return now.
    pub fn readable(&self) -> bool {
        if self.canonical() {
            !self.lines.is_empty()
        } else {
            !self.ready.is_empty() || self.cc[VMIN] == 0
        }
    }

    /// Up to `nbytes` of input for a read, or None if the read must wait.
    /// Canonical mode returns at most one line; raw mode what is there
    /// (with `VMIN` 0, possibly nothing).
    pub fn read(&mut self, nbytes: usize) -> Option<Vec<u8>> {
        if self.canonical() {
            let line = self.lines.front_mut()?;
            let n = nbytes.min(*line);
            *line -= n;
            if *line == 0 {
                self.lines.pop_front();
            }
            return Some(self.ready.drain(..n).collect());
        }
        if self.ready.is_empty() && self.cc[VMIN] > 0 {
            return None;
        }
        let n = nbytes.min(self.ready.len());
        Some(self.ready.drain(..n).collect())
    }
}
