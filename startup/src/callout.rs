//! Kernel callouts: small pieces of board code the kernel calls.
//!
//! The kernel knows nothing about the board's interrupt controller, UART
//! or reset logic. Where it needs them, it calls a *callout*: code that
//! startup provides, copies into the system page and patches for the
//! board (with the virtual address of a device's registers, for one). The
//! system page sections hold the callouts' addresses: `callout` (debug
//! output, reboot), `intrinfo` (interrupt controller), `cacheattr` (cache
//! maintenance), `smp` (inter-processor interrupts).
//!
//! A callout runs in the kernel's address space, long after startup has
//! gone, so it must be position-independent and may refer to nothing but
//! its arguments, the system page and the values patched into it.
//!
//! Callouts are written in assembly with [`callout!`]. Each one starts
//! with as many `movz`/`movk` sequences (4 instructions each, loading a
//! 64-bit register) as it has patch values; startup writes the values
//! into the copy.

/// The most values a callout can have patched in.
pub const MAX_PATCHES: usize = 4;

/// A callout's code, as linked into startup.
pub struct Callout {
    pub name: &'static str,
    start: *const u8,
    end: *const u8,
    /// Number of `movz`/`movk` sequences at the start to patch.
    pub patches: usize,
}

// SAFETY: the pointers are to code in startup's text, which is never
// written.
unsafe impl Sync for Callout {}

impl Callout {
    #[doc(hidden)]
    pub const fn new(name: &'static str, start: *const u8, end: *const u8, patches: usize) -> Self {
        Self {
            name,
            start,
            end,
            patches,
        }
    }

    /// The callout's code.
    pub fn code(&self) -> &'static [u8] {
        // SAFETY: start and end delimit the callout in startup's text.
        unsafe {
            core::slice::from_raw_parts(self.start, self.end.offset_from(self.start) as usize)
        }
    }
}

// ANCHOR: callout_macro
/// Define a callout from assembly lines:
///
/// ```ignore
/// callout!(pub static MY_CALLOUT, "my_callout", patches = 1,
///     "movz x5, #0", "movk x5, #0, lsl #16", "movk x5, #0, lsl #32",
///     "movk x5, #0, lsl #48",
///     "ret",
/// );
/// ```
///
/// The symbol name (`"my_callout"`) must be unique in the program.
#[macro_export]
macro_rules! callout {
    ($vis:vis static $name:ident, $sym:literal, patches = $patches:expr, $($asm:expr),+ $(,)?) => {
        core::arch::global_asm!(
            concat!(".section .text.callout.", $sym, ", \"ax\""),
            ".balign 8",
            concat!(".global ", $sym),
            concat!($sym, ":"),
            $($asm,)+
            concat!(".global ", $sym, "_end"),
            concat!($sym, "_end:"),
        );
        $vis static $name: $crate::callout::Callout = {
            unsafe extern "C" {
                #[link_name = $sym]
                static START: u8;
                #[link_name = concat!($sym, "_end")]
                static END: u8;
            }
            $crate::callout::Callout::new($sym, &raw const START, &raw const END, $patches)
        };
    };
}
// ANCHOR_END: callout_macro

/// A callout in use, with the values to patch into it: what the system
/// page sections refer to until the callouts are copied into the system
/// page.
#[derive(Clone, Copy)]
pub struct Use {
    pub callout: &'static Callout,
    pub values: [u64; MAX_PATCHES],
}

/// The most callouts in use.
pub const MAX_CALLOUTS: usize = 32;

/// The callouts in use.
pub struct Table {
    uses: [Option<Use>; MAX_CALLOUTS],
    len: usize,
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

/// The value a section holds in place of a callout's address until the
/// callouts are copied: a tag in the top bits, the index in the low ones.
/// A kernel address has its top 24 bits set, so no address looks like it.
const REF_TAG: u64 = 0x0ca1_1007 << 32;

impl Table {
    pub const fn new() -> Self {
        Self {
            uses: [None; MAX_CALLOUTS],
            len: 0,
        }
    }

    /// Use `callout`, patched with `values`. The same callout with the
    /// same values is copied once. Returns the reference to store in a
    /// section until [`Table::resolve`].
    pub fn add(&mut self, callout: &'static Callout, values: &[u64]) -> u64 {
        assert_eq!(
            values.len(),
            callout.patches,
            "{}: wrong number of patch values",
            callout.name
        );
        let mut v = [0; MAX_PATCHES];
        v[..values.len()].copy_from_slice(values);
        if let Some(i) = self.uses[..self.len]
            .iter()
            .position(|u| u.is_some_and(|u| core::ptr::eq(u.callout, callout) && u.values == v))
        {
            return REF_TAG | i as u64;
        }
        assert!(self.len < MAX_CALLOUTS, "too many callouts");
        self.uses[self.len] = Some(Use { callout, values: v });
        self.len += 1;
        REF_TAG | (self.len as u64 - 1)
    }

    pub fn uses(&self) -> impl Iterator<Item = &Use> {
        self.uses[..self.len].iter().flatten()
    }

    /// The use a section value refers to, if it is a callout reference.
    pub fn get(&self, value: u64) -> Option<(usize, &Use)> {
        if value & !0xffff_ffff != REF_TAG {
            return None;
        }
        let i = (value & 0xffff_ffff) as usize;
        self.uses[..self.len].get(i)?.as_ref().map(|u| (i, u))
    }

    /// Total size of the callouts' code, each 8-byte aligned.
    pub fn code_size(&self) -> usize {
        self.uses()
            .map(|u| u.callout.code().len().next_multiple_of(8))
            .sum()
    }

    // ANCHOR: callout_output
    /// Copy every callout into `area` (the system page's callout code,
    /// mapped at `vaddr` in the kernel), patch it, and return each use's
    /// virtual address, by index.
    pub fn output(&self, area: &mut [u8], vaddr: u64) -> [u64; MAX_CALLOUTS] {
        let mut addrs = [0; MAX_CALLOUTS];
        let mut off = 0;
        for (i, u) in self.uses().enumerate() {
            let code = u.callout.code();
            let dst = &mut area[off..off + code.len()];
            dst.copy_from_slice(code);
            for (k, &value) in u.values[..u.callout.patches].iter().enumerate() {
                patch_mov64(&mut dst[k * 16..k * 16 + 16], value, u.callout.name);
            }
            addrs[i] = vaddr + off as u64;
            off += code.len().next_multiple_of(8);
        }
        addrs
    }
    // ANCHOR_END: callout_output

    /// The address that replaces `value`, if it is a callout reference.
    pub fn resolve(&self, value: u64, addrs: &[u64; MAX_CALLOUTS]) -> u64 {
        match self.get(value) {
            Some((i, _)) => addrs[i],
            None => value,
        }
    }
}

// ANCHOR: patch_mov64
/// Patch `value` into a `movz`/`movk`/`movk`/`movk` sequence that loads a
/// 64-bit register, 16 bits per instruction (the imm16 field, bits 5-20).
fn patch_mov64(insns: &mut [u8], value: u64, name: &str) {
    // movz xN, #imm, lsl #0 / movk xN, #imm, lsl #16, #32, #48: opcode and
    // shift (hw) bits, with the immediate and the register masked out.
    const EXPECTED: [u32; 4] = [0xd280_0000, 0xf2a0_0000, 0xf2c0_0000, 0xf2e0_0000];
    for (k, insn) in insns.chunks_exact_mut(4).enumerate() {
        let mut op = u32::from_le_bytes([insn[0], insn[1], insn[2], insn[3]]);
        assert_eq!(
            op & 0xffe0_0000,
            EXPECTED[k],
            "{name}: patch site is not a movz/movk sequence"
        );
        let imm = ((value >> (16 * k)) & 0xffff) as u32;
        op = (op & !(0xffff << 5)) | (imm << 5);
        insn.copy_from_slice(&op.to_le_bytes());
    }
}
// ANCHOR_END: patch_mov64

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_a_mov64_sequence() {
        // movz x5, #0; movk x5, #0, lsl #16; ... lsl #32; ... lsl #48
        let mut code = [0u8; 16];
        for (k, op) in [0xd280_0005u32, 0xf2a0_0005, 0xf2c0_0005, 0xf2e0_0005]
            .iter()
            .enumerate()
        {
            code[k * 4..k * 4 + 4].copy_from_slice(&op.to_le_bytes());
        }
        patch_mov64(&mut code, 0xffff_ff80_4020_5000, "test");
        let ops: [u32; 4] = core::array::from_fn(|k| {
            u32::from_le_bytes(code[k * 4..k * 4 + 4].try_into().unwrap())
        });
        assert_eq!(ops, [0xd28a_0005, 0xf2a8_0405, 0xf2df_f005, 0xf2ff_ffe5]);
    }

    #[test]
    #[should_panic(expected = "not a movz/movk")]
    fn refuses_other_instructions() {
        patch_mov64(&mut [0; 16], 1, "test");
    }
}
