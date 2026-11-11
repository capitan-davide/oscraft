# Appendix C: Glossary

**AP (application processor)**: a CPU other than the boot CPU; also
called a secondary CPU.

**ASID**: an Address Space Identifier, which tags TLB entries with the
address space they belong to.

**asinfo**: the system page section that describes the physical address
space as a tree of named ranges.

**Boot CPU**: the CPU the boot loader enters startup on.

**Bootstrap executable**: a program in the image that startup loads and
starts; the kernel is the last one.

**Callout**: a piece of board code that startup copies into the system
page and the kernel calls.

**Cpupage**: a small per-CPU structure next to the system page.

**Device tree (FDT)**: a description of the hardware that a boot loader
passes, as a flattened binary blob (`.dtb`).

**EL0-EL3**: AArch64's exception levels, from applications (EL0) to the
secure monitor (EL3). QNX's kernel runs at EL1.

**GIC**: Arm's Generic Interrupt Controller; GICD is its distributor,
GICC a CPU interface.

**hwinfo**: the system page section that describes devices for drivers.

**IFS (image file system)**: the file system inside the boot image, which
the kernel serves from memory.

**IPI**: an inter-processor interrupt, which one CPU sends another; on a
GIC, a software-generated interrupt (SGI).

**Keeplinked**: a bootstrap executable that mkifs links at its final
address, so that startup maps it in place.

**mkifs**: QNX's tool that builds a boot image from a build file.

**PPI, SGI, SPI**: a GIC's private per-CPU, software-generated, and
shared peripheral interrupts.

**procnto**: QNX's kernel, with the process manager in the same program.

**Spin table**: a word per CPU in memory, polled by a waiting CPU for the
address to start at.

**Startup**: the program that runs first from a QNX image and prepares
the board for the kernel.

**Startup header**: the structure mkifs puts in front of startup, which
describes the image.

**Recursive entry**: a translation table entry that points back at its
own table, so that the tables appear as data in the address space.

**sysram**: asinfo entries for the RAM the kernel may use.

**System page**: the block of memory where startup describes the board
to the kernel.

**TLB**: the Translation Lookaside Buffer, the CPU's cache of recent
address translations.

**Translation table**: the in-memory tree that the MMU walks to translate
a virtual address to a physical one; also called a page table.

**TTBR0, TTBR1**: the two translation table base registers, for the
lower and upper halves of the address space.
