//! aarch64 boot fragments (uniform names — see `crate::boot`).
//!
//! Composition order: `boot_stack_asm!` → `boot_pt_buffer_asm!` →
//! `boot_setup_asm!` → *mapping policy* (result in `x23`) →
//! `boot_cross_asm!` → `boot_call_asm!(entry)` → `boot_secondary_asm!`.
//!
//! Note: the load-delta labels use numeric locals (`9f`/`8f`) — LLVM's
//! integrated assembler rejects `.L` temporaries inside literal pools.
use crate::kernel_main;


/// BSS symbols for the kor-mmu `boot_mmu_build` hook ABI: an 8 KiB
/// page-table buffer plus the offset cell the secondary stub reads.
#[macro_export]
macro_rules! boot_pt_buffer_asm {
    () => {
        "
.section .bss.boot_page_table,\"aw\",%nobits
.balign 4096
.global _boot_pt_buffer
_boot_pt_buffer:
    .zero 8192
.section .bss.boot_mmu_offset,\"aw\",%nobits
.balign 8
_boot_mmu_offset:
    .zero 8
"
    };
}

/// `_start` prologue: save the DTB pointer and CPU id, compute the runtime
/// load delta `x24` (link VA − physical PC), clear BSS, switch to the boot
/// stack.  Falls through in the physical world.
#[macro_export]
macro_rules! boot_setup_asm {
    () => {
        "
.section .text.entry
.global _start
_start:
    mov     x19, x0                 // DTB pointer (PA)
    mrs     x20, mpidr_el1          // CPU ID

    // x24 = link VA here - physical PC here
    adr     x0, 9f
    ldr     x1, =9f
    sub     x24, x1, x0
9:

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
    ldr     x0, =bstack_top
    sub     x0, x0, x24
    mov     sp, x0
"
    };
}

/// Offset bookkeeping, MAIR/TCR/TTBR programming and MMU enable (tables at
/// `_boot_pt_buffer`), and the crossing to virtual PCs.  Expects the
/// mapping-slot result in `x23` (0 = stay unmapped); falls through with
/// `x0`/`x1` = (cpu id, DTB VA), `sp` offset-adjusted, and both `x23`
/// (offset) and `x24` (load delta) still live for [`boot_call_asm!`].
#[macro_export]
macro_rules! boot_cross_asm {
    () => {
        "
    ldr     x1, =_boot_mmu_offset
    sub     x1, x1, x24
    str     x23, [x1]
    cbz     x23, .Lkor_boot_phys

    movz    x0, #0xFF00             // MAIR: normal RW + device
    movk    x0, #0x0044, lsl #16
    msr     mair_el1, x0
    ldr     x0, =0x0000000575103510 // TCR_EL1
    msr     tcr_el1, x0
    isb
    ldr     x0, =_boot_pt_buffer
    sub     x0, x0, x24
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x0
    tlbi    vmalle1
    dsb     sy
    isb
    mrs     x0, sctlr_el1
    movz    x1, #0x1005             // M | A | C | I | TEX remap
    orr     x0, x0, x1
    msr     sctlr_el1, x0
    isb

    adr     x0, .Lkor_boot_virt
    add     x0, x0, x23
    br      x0
.Lkor_boot_virt:
    add     sp, sp, x23
    add     x19, x19, x23
.Lkor_boot_phys:
    and     x0, x20, #0xffffff
    mov     x1, x19
"
    };
}

/// Call the application entry.  The offset (`x23`) and load delta (`x24`)
/// left live by [`boot_cross_asm!`] select the entry's virtual or physical
/// address; parks if the entry ever returns.
#[macro_export]
macro_rules! boot_call_asm {
    ($entry:ident) => {
        concat!(
            "
    ldr     x8, =",
            stringify!($entry),
            "
    cbz     x23, .Lkor_call_phys
    blr     x8
    b       .
.Lkor_call_phys:
    sub     x8, x8, x24
    blr     x8
    b       .
",
        )
    };
}

/// Secondary-core stub (entered physically via PSCI `CPU_ON`): re-install
/// the boot root, cross the offset, and enter `rust_entry_secondary`.
/// Requires the [`boot_pt_buffer_asm!`] symbols.
#[macro_export]
macro_rules! boot_secondary_asm {
    () => {
        "
.global _secondary_start
_secondary_start:
    mov     x20, x0                 // stack top (post-offset)
    mrs     x19, mpidr_el1
    and     x19, x19, #0xffffff

    adr     x0, 8f
    ldr     x1, =8f
    sub     x24, x1, x0
8:
    ldr     x0, =_boot_mmu_offset
    sub     x0, x0, x24
    ldr     x23, [x0]
    cbz     x23, .Lkor_sec_phys

    movz    x0, #0xFF00
    movk    x0, #0x0044, lsl #16
    msr     mair_el1, x0
    ldr     x0, =0x0000000575103510
    msr     tcr_el1, x0
    isb
    ldr     x0, =_boot_pt_buffer
    sub     x0, x0, x24
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x0
    tlbi    vmalle1
    dsb     sy
    isb
    mrs     x0, sctlr_el1
    movz    x1, #0x1005
    orr     x0, x0, x1
    msr     sctlr_el1, x0
    isb

    adr     x0, .Lkor_sec_virt
    add     x0, x0, x23
    br      x0
.Lkor_sec_virt:
    mov     sp, x20
    mov     x0, x19
    ldr     x8, =rust_entry_secondary
    blr     x8
    b       .
.Lkor_sec_phys:
    mov     sp, x20
    mov     x0, x19
    ldr     x8, =rust_entry_secondary
    sub     x8, x8, x24
    blr     x8
    b       .
"
    };
}

/// First Rust function of the boot flow (`call rust_entry` from the app's
/// composed trampoline): stash the DTB pointer into kor-arch state, then
/// enter the application's `kernel_main` (link-time extern).
#[unsafe(no_mangle)]
extern "C" fn rust_entry(_hart_id: usize, dtb: usize) {
    super::mm::set_dtb_ptr(dtb);
    // SAFETY: the app binary provides kernel_main at link time; it never
    // returns.
    unsafe { kernel_main() }
}
