// AArch64 boot trampoline — MMU bring-up delegated to the application.
//
// The app links in `boot_mmu_build(table_pa, table_len) -> offset` (an
// extern "C" symbol): it fills the table buffer and returns the phys->virt
// offset of the mapping it built, or 0 to run unmapped.  This trampoline is
// policy-free: BSS clear, physical stack, call the builder, program
// MAIR/TCR/TTBR and enable the MMU if asked, then jump across the offset —
// so one trampoline serves both MMU and no-MMU applications.
//
// Pre-MMU addressing note: the image is linked for high-half VAs, so every
// absolute `ldr =sym` below is converted to its physical address by
// subtracting the runtime load delta (link VA - PC, computed once via
// `adr`); no offset constant is baked in.

// L0 table descriptor: VALID | TYPE_TABLE
.set L0_TABLE_FLAGS, ((1 << 0) | (1 << 1))

.section .bss.boot_page_table,"aw",%nobits
.balign 4096
.global _boot_pt_buffer
_boot_pt_buffer:
    .zero 8192                  // L0 root + one L1 table

.section .bss.boot_mmu_offset,"aw",%nobits
.balign 8
_boot_mmu_offset:
    .zero 8

.section .text.entry
.global _start
_start:
    mov     x19, x0             // save DTB pointer (PA from firmware)
    mrs     x20, mpidr_el1      // save CPU ID

    // --- Runtime load delta: x24 = link VA here - physical PC here ---
    adr     x0, 9f
    ldr     x1, =9f
    sub     x24, x1, x0
9:

    // --- Clear BSS (physical addresses, MMU off) ---
    ldr     x0, =_sbss
    sub     x0, x0, x24
    ldr     x1, =_ebss
    sub     x1, x1, x24
    cmp     x0, x1
    b.ge    2f
1:
    str     xzr, [x0], #8
    cmp     x0, x1
    b.lt    1b
2:
    // --- Set stack pointer (physical address) ---
    ldr     x0, =bstack_top
    sub     x0, x0, x24
    mov     sp, x0

    // --- Ask the application to build the boot page tables ---
    ldr     x0, =_boot_pt_buffer
    sub     x0, x0, x24         // physical buffer address
    mov     x1, #8192
    ldr     x8, =boot_mmu_build
    sub     x8, x8, x24         // physical function address
    blr     x8                  // x0 = phys->virt offset (0 = unmapped)
    mov     x23, x0

    // Publish the offset for the secondary-core trampoline.
    ldr     x1, =_boot_mmu_offset
    sub     x1, x1, x24
    str     x23, [x1]

    cbz     x23, 5f             // no-MMU app: stay on physical PCs

    // --- Init MMU (mechanism, policy-free) ---
    // MAIR_EL1: Attr0=Device(0x00), Attr1=Normal-WB(0xFF), Attr2=Normal-NonCache(0x44)
    movz    x0, #0xFF00
    movk    x0, #0x0044, lsl #16
    msr     mair_el1, x0

    // TCR_EL1: IPS=48, T0SZ=16, T1SZ=16, 4KB granule both, inner-shareable, cacheable
    ldr     x0, =0x0000000575103510
    msr     tcr_el1, x0
    isb

    // TTBR0_EL1 and TTBR1_EL1 point to the app-built root
    ldr     x0, =_boot_pt_buffer
    sub     x0, x0, x24
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x0

    tlbi    vmalle1
    dsb     sy
    isb

    // Enable MMU + I-cache + D-cache
    mrs     x0, sctlr_el1
    movz    x1, #0x1005
    orr     x0, x0, x1
    msr     sctlr_el1, x0
    isb

    // --- Jump across the offset to the virtual PC ---
    adr     x0, 3f
    add     x0, x0, x23
    br      x0
3:
    // Now running on virtual PCs — move sp and the DTB pointer across.
    add     sp, sp, x23
    add     x19, x19, x23

    and     x0, x20, #0xffffff  // hart_id from mpidr_el1
    mov     x1, x19             // DTB pointer (VA)
    ldr     x8, =rust_entry
    blr     x8
    b       .
5:
    // Unmapped path: enter Rust on physical PCs (the app must be linked
    // for a physical/identity model).
    and     x0, x20, #0xffffff
    mov     x1, x19             // DTB stays a physical pointer
    ldr     x8, =rust_entry
    sub     x8, x8, x24
    blr     x8
    b       .

// --- Secondary core entry (via PSCI CPU_ON) -----------------------------
// PSCI enters here at EL1 with the MMU off; x0 = context id (our per-CPU
// stack top VA).  The boot core's builder already produced the tables and
// published the offset, so this path re-enables the MMU with them, crosses
// the offset, sets the stack, and calls rust_entry_secondary.
.global _secondary_start
_secondary_start:
    mov     x20, x0                 // per-CPU stack top (VA), used after MMU on
    mrs     x19, mpidr_el1
    and     x19, x19, #0xffffff     // cpu id

    // Runtime load delta (this core's PC is physical here).
    adr     x0, 9f
    ldr     x1, =9f
    sub     x24, x1, x0
9:

    // Offset published by the boot core.
    ldr     x0, =_boot_mmu_offset
    sub     x0, x0, x24
    ldr     x23, [x0]
    cbz     x23, 5f                 // no-MMU app: skip enable

    // MAIR / TCR (same config as the boot core)
    movz    x0, #0xFF00
    movk    x0, #0x0044, lsl #16
    msr     mair_el1, x0
    ldr     x0, =0x0000000575103510
    msr     tcr_el1, x0
    isb

    // TTBR0/1 = physical base of the app-built tables
    ldr     x0, =_boot_pt_buffer
    sub     x0, x0, x24
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x0
    tlbi    vmalle1
    dsb     sy
    isb

    // Enable MMU + I/D cache
    mrs     x0, sctlr_el1
    movz    x1, #0x1005
    orr     x0, x0, x1
    msr     sctlr_el1, x0
    isb

    // Jump to the high-half VA
    adr     x0, 4f
    add     x0, x0, x23
    br      x0
4:
    mov     sp, x20                 // per-CPU stack top (post-offset address)
    mov     x0, x19                 // cpu id
    ldr     x8, =rust_entry_secondary
    blr     x8
    b       .
5:
    mov     sp, x20
    mov     x0, x19
    ldr     x8, =rust_entry_secondary
    sub     x8, x8, x24
    blr     x8
    b       .
