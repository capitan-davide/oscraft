//! Interrupt code generation.
//!
//! The system page describes each interrupt controller with code fragments
//! (`intrinfo.id`, `intrinfo.eoi`) that are meant to be inlined into the
//! kernel's interrupt path: they have no `ret` and pass values in fixed
//! callee-saved registers (BSP `callout_interrupt_gic_v2.S`):
//!
//! - `x20` = system page, `x22` = mask count (inputs);
//! - `x19` = controller-relative interrupt id, or -1 for "not mine" (id output,
//!   eoi input);
//! - `x24` = state the GICv2 fragments share within one interrupt (the
//!   value read from GICC_IAR).
//!
//! Each fragment is copied into a kernel code page with a `ret` appended and
//! called through `intr_fragment`, which keeps x19..x28 in a
//! `FragmentState` between the calls that handle one interrupt.

use core::arch::global_asm;

use super::mmu::{self, Prot};
use crate::vm::kmem;
use crate::vm::pmem::PAGE_SIZE;

global_asm!(include_str!("intr_bridge.s"));

unsafe extern "C" {
    fn intr_fragment(code: u64, state: *mut FragmentState) -> u64;
}

/// x19..x28 as seen by the fragments.
#[repr(C)]
#[derive(Debug, Default, Clone)]
pub struct FragmentState {
    x: [u64; 10],
}

impl FragmentState {
    /// State at the start of one interrupt.
    pub fn new(syspage: u64) -> Self {
        let mut s = Self::default();
        s.x[1] = syspage; // x20
        s
    }

    pub fn set_id(&mut self, id: u64) {
        self.x[0] = id; // x19
    }
}

/// Run a generated fragment; returns x19.
pub fn run(code: u64, state: &mut FragmentState) -> u64 {
    // SAFETY: `code` is a fragment copied by `CodeBuffer`, ending in `ret`,
    // that only uses registers the bridge saves and its patched devices.
    unsafe { intr_fragment(code, state) }
}

const RET: u32 = 0xd65f_03c0;

/// One page of generated code: writable while fragments are added, then
/// made executable by `finish`.
pub struct CodeBuffer {
    base: u64,
    len: u64,
}

impl CodeBuffer {
    pub fn new() -> Option<Self> {
        Some(Self {
            base: kmem::map_new(1, Prot::Data)?,
            len: 0,
        })
    }

    /// Copy a fragment of `size` bytes at `rtn` and append `ret`; returns
    /// the address of the copy. A missing fragment becomes a bare `ret`.
    pub fn add_fragment(&mut self, rtn: u64, size: u16) -> Option<u64> {
        let size = if rtn == 0 { 0 } else { size as u64 };
        if self.len + size + 4 > PAGE_SIZE {
            return None;
        }
        let at = self.base + self.len;
        // SAFETY: the fragment lies in the mapped callout area; the buffer
        // page is mapped writable and has room (checked above).
        unsafe { core::ptr::copy_nonoverlapping(rtn as *const u8, at as *mut u8, size as usize) };
        // SAFETY: the room checked above includes the `ret` after it.
        unsafe { ((at + size) as *mut u32).write_unaligned(RET) };
        self.len = (self.len + size + 4).next_multiple_of(4);
        Some(at)
    }

    /// Make the code executable (and no longer writable).
    pub fn finish(self) -> Option<()> {
        mmu::sync_icache(self.base, self.len);
        mmu::protect(self.base, Prot::Code).ok()
    }
}
