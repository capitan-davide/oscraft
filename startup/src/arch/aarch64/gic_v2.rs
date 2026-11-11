//! The Arm GICv2 interrupt controller (such as the GIC-400).
//!
//! A GICv2 has a *distributor* (GICD), shared by all CPUs, which enables
//! interrupts, sets their priorities and routes them to CPUs, and one
//! *CPU interface* (GICC) per CPU, banked at the same address, through
//! which a CPU acknowledges (IAR) and ends (EOIR) an interrupt.
//!
//! Interrupt IDs 0-15 are software-generated (SGIs, used as IPIs), 16-31
//! private to each CPU (PPIs, such as the timer), 32 and up shared (SPIs,
//! the devices). The kernel sees them as two intrinfo ranges: vectors
//! 0-31 and 32 up, each with its callouts.

use crate::abi::syspage::*;
use crate::global::Global;
use crate::{Startup, kprintln};

// Distributor registers.
const GICD_CTLR: u64 = 0x000;
const GICD_TYPER: u64 = 0x004;
const GICD_ISENABLER: u64 = 0x100;
const GICD_ICENABLER: u64 = 0x180;
const GICD_ICPENDR: u64 = 0x280;
const GICD_IPRIORITYR: u64 = 0x400;
const GICD_ITARGETSR: u64 = 0x800;
const GICD_ICFGR: u64 = 0xc00;
// CPU interface registers.
const GICC_CTLR: u64 = 0x000;
const GICC_PMR: u64 = 0x004;

fn read(addr: u64) -> u32 {
    // SAFETY: GIC registers, mapped 1:1 while the MMU is off.
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn write(addr: u64, value: u32) {
    // SAFETY: as in read().
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

/// Where the GIC is, for the CPUs that start later.
#[derive(Clone, Copy)]
struct Gic {
    gicd: u64,
    gicc: u64,
}

static GIC: Global<Option<Gic>> = Global::new(None);

// ANCHOR: gic_callouts
// The interrupt callouts (see <aarch64/callout.ah>). The id and eoi ones
// are copied into the kernel's interrupt entry code, so they do not
// return: they fall through their end. They get the syspage in x20 and
// return the controller-relative interrupt in x19, -1 if none; the IAR
// value travels in x24 from the PPI id to the SPI id and to eoi.
crate::callout!(pub static ID_PPI, "oscraft_gic_v2_id_ppi", patches = 1,
    "movz    x6, #0", "movk x6, #0, lsl #16", "movk x6, #0, lsl #32", "movk x6, #0, lsl #48",
    "ldr     w0, [x6, #0xc]",       // GICC_IAR
    "and     w19, w0, #0x3ff",
    "cmp     w19, #31",
    "mov     x1, #-1",
    "mov     w24, w19",
    "csel    x19, x1, x19, hi",     // not an SGI/PPI: -1
    "cbnz    x19, 2f",
    // SGI 0, the IPI: end it now, with its source CPU.
    "str     w0, [x6, #0x10]",      // GICC_EOIR
    "2:",
);

crate::callout!(pub static ID_SPI, "oscraft_gic_v2_id_spi", patches = 0,
    "sub     x0, x24, #32",
    "cmp     x24, #1021",
    "mov     x1, #-1",
    "csel    x19, x1, x0, hi",      // 1022, 1023: spurious
);

crate::callout!(pub static EOI, "oscraft_gic_v2_eoi", patches = 1,
    "movz    x6, #0", "movk x6, #0, lsl #16", "movk x6, #0, lsl #32", "movk x6, #0, lsl #48",
    "cbz     w24, 0f",              // the IPI was ended in id
    "str     w24, [x6, #0x10]",     // GICC_EOIR
    "0:",
);

// mask/unmask(syspage, vector): set the vector's bit in ICENABLER /
// ISENABLER; SPI vectors are relative to 32.
macro_rules! enable_callout {
    ($name:ident, $sym:literal, $reg:literal, $add:literal) => {
        crate::callout!(pub static $name, $sym, patches = 1,
            "movz    x5, #0", "movk x5, #0, lsl #16", "movk x5, #0, lsl #32", "movk x5, #0, lsl #48",
            concat!("add     x5, x5, #", $reg),
            concat!("add     w1, w1, #", $add),
            "and     w0, w1, #0x1f",
            "mov     w2, #1",
            "lsl     w2, w2, w0",
            "lsr     w0, w1, #5",
            "str     w2, [x5, x0, lsl #2]",
            "mov     x0, #0",
            "ret",
        );
    };
}
enable_callout!(MASK_PPI, "oscraft_gic_v2_mask_ppi", "0x180", "0");
enable_callout!(UNMASK_PPI, "oscraft_gic_v2_unmask_ppi", "0x100", "0");
enable_callout!(MASK_SPI, "oscraft_gic_v2_mask_spi", "0x180", "32");
enable_callout!(UNMASK_SPI, "oscraft_gic_v2_unmask_spi", "0x100", "32");

// config(syspage, intrinfo, vector): vector 0 (SGI 0) is the IPI.
crate::callout!(pub static CONFIG_PPI, "oscraft_gic_v2_config_ppi", patches = 0,
    "cmp     x2, #0",
    "mov     w0, #4",               // INTR_CONFIG_FLAG_IPI
    "csel    w0, w0, wzr, eq",
    "ret",
);

// send_ipi(syspage, target, cmd, &ipicmds[cpu]): record the command and,
// if none was pending, send SGI 0 to the CPU. The kernel takes target from
// gic_map: the CPU's GIC interface number.
crate::callout!(pub static SEND_IPI, "oscraft_gic_v2_send_ipi", patches = 1,
    "movz    x5, #0", "movk x5, #0, lsl #16", "movk x5, #0, lsl #32", "movk x5, #0, lsl #48",
    "mrs     x0, daif",
    "msr     daifset, #2",
    "0: ldxr w7, [x3]",
    "orr     w6, w7, w2",
    "stxr    w4, w6, [x3]",
    "cbnz    w4, 0b",
    "dsb     sy",
    "cbnz    w7, 1f",
    "mov     w7, #0x10000",         // GICD_SGIR target list: bit 16 + cpu
    "lsl     w7, w7, w1",
    "str     w7, [x5, #0xf00]",     // GICD_SGIR
    "1: msr  daif, x0",
    "ret",
);
// ANCHOR_END: gic_callouts

/// The CPU interface number of the CPU running this: the bit set in
/// `GICD_ITARGETSR0`, which reads as the reading CPU's own interface.
fn cpu_interface(gicd: u64, num_cpu: u16) -> u32 {
    let mask = read(gicd + GICD_ITARGETSR) & 0xff;
    if mask == 0 && num_cpu == 1 {
        return 0;
    }
    if mask.count_ones() != 1 {
        crate::crash!("GICv2: no CPU interface for this CPU (ITARGETSR0 {mask:#x})");
    }
    mask.trailing_zeros()
}

// ANCHOR: gic_init
/// Set up the GIC whose distributor is at `gicd` and CPU interfaces at
/// `gicc`, describe it in the system page, and register [`init_cpu`] as
/// each CPU's part of the setup. All shared interrupts go to the boot CPU, disabled, level
/// triggered, at the same priority; the kernel takes it from there.
pub fn init(s: &mut Startup, gicd: u64, gicc: u64) {
    let mem = s.syspage.as_find("memory").unwrap_or(AS_NULL_OFF);
    s.syspage.as_add(gicd, gicd + 0xfff, "gicd", mem);
    s.syspage.as_add(gicc, gicc + 0x1fff, "gicc", mem);
    let gicd_v = s.map_device(gicd, 0x1000);
    let gicc_v = s.map_device(gicc, 0x2000);
    // SAFETY: the boot CPU owns GIC until the other CPUs start.
    unsafe { *GIC.get() = Some(Gic { gicd, gicc }) };

    let cpu = cpu_interface(gicd, s.syspage.num_cpu);
    write(gicd + GICD_CTLR, 0);
    let lines = ((read(gicd + GICD_TYPER) & 0x1f) + 1) * 32;
    let spis = lines.min(1020) - 32;
    if s.debug_level > 0 {
        kprintln!("GICv2: {lines} interrupts, SPIs to CPU interface {cpu}");
    }
    for i in (32..lines).step_by(32) {
        write(gicd + GICD_ICENABLER + u64::from(i / 8), !0);
        write(gicd + GICD_ICPENDR + u64::from(i / 8), !0);
    }
    for i in (32..32 + spis).step_by(4) {
        write(gicd + GICD_IPRIORITYR + u64::from(i), 0xa0a0_a0a0);
        write(gicd + GICD_ITARGETSR + u64::from(i), 0x0101_0101 << cpu);
    }
    for i in (32..lines).step_by(16) {
        write(gicd + GICD_ICFGR + u64::from(i / 4), 0);
    }
    write(gicd + GICD_CTLR, 1);

    let sp = &mut s.syspage;
    let ppi = IntrinfoEntry {
        vector_base: 0,
        num_vectors: 32,
        cascade_vector: INTR_NOT_CASCADED,
        id: IntrgenData {
            rtn: sp.callout(&ID_PPI, &[gicc_v]),
            ..Default::default()
        },
        eoi: IntrgenData {
            rtn: sp.callout(&EOI, &[gicc_v]),
            ..Default::default()
        },
        mask: sp.callout(&MASK_PPI, &[gicd_v]),
        unmask: sp.callout(&UNMASK_PPI, &[gicd_v]),
        config: sp.callout(&CONFIG_PPI, &[]),
        // The SGIs and PPIs are banked per CPU.
        local_stride: 0x8000,
        ..Default::default()
    };
    let spi = IntrinfoEntry {
        vector_base: 32,
        num_vectors: spis,
        cascade_vector: INTR_NOT_CASCADED,
        id: IntrgenData {
            rtn: sp.callout(&ID_SPI, &[]),
            ..Default::default()
        },
        eoi: IntrgenData {
            rtn: sp.callout(&EOI, &[gicc_v]),
            ..Default::default()
        },
        mask: sp.callout(&MASK_SPI, &[gicd_v]),
        unmask: sp.callout(&UNMASK_SPI, &[gicd_v]),
        ..Default::default()
    };
    sp.intrinfo.push(ppi);
    sp.intrinfo.push(spi);
    sp.smp.send_ipi = sp.callout(&SEND_IPI, &[gicd_v]);
    s.set_cpu_intr_init(init_cpu);
}
// ANCHOR_END: gic_init

// ANCHOR: gic_cpu_init
/// Set up the CPU interface of the CPU running this, logical CPU `cpu`:
/// record its interface number in `gic_map`, give its SGIs and PPIs a
/// priority, enable the SGIs and disable the PPIs, and open the interface
/// to interrupts of a higher priority than 0xf0.
pub fn init_cpu(gic_map: &mut [u64], cpu: usize, num_cpu: u16) {
    // SAFETY: written once by init(), before any CPU calls this.
    let Some(gic) = (unsafe { *GIC.get() }) else {
        crate::crash!("GICv2 not set up");
    };
    gic_map[cpu] = u64::from(cpu_interface(gic.gicd, num_cpu));
    // SGI 0 (the IPI) at 0xc0, the other SGIs and the PPIs at 0xa0.
    write(gic.gicd + GICD_IPRIORITYR, 0xa0a0_a0c0);
    for i in 1..8 {
        write(gic.gicd + GICD_IPRIORITYR + i * 4, 0xa0a0_a0a0);
    }
    write(gic.gicd + GICD_ICENABLER, 0xffff_0000);
    write(gic.gicd + GICD_ISENABLER, 0x0000_ffff);
    write(gic.gicc + GICC_PMR, 0xf0);
    write(gic.gicc + GICC_CTLR, 1);
}
// ANCHOR_END: gic_cpu_init
