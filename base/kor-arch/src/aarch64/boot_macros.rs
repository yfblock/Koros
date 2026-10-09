//! aarch64 boot fragments (uniform names — see `crate::boot`).
//!
//! Composition order: `boot_stack_asm!` → `boot_pt_buffer_asm!` →
//! `boot_setup_asm!` → *mapping policy* (result in `x23`) →
//! `boot_cross_asm!` → `boot_call_asm!(entry)` → `boot_secondary_asm!`.

/// BSS symbols for the kor-mmu `boot_mmu_build` hook ABI: an 8 KiB
/// page-table buffer plus the offset cell the secondary stub reads.
#[macro_export]
macro_rules! boot_pt_buffer_asm {
    () => {
        concat!(
            ".section .bss.boot_page_table,\"aw\",%nobits\n",
            ".balign 4096\n",
            ".global _boot_pt_buffer\n",
            "_boot_pt_buffer:\n",
            "    .zero 8192\n",
            ".section .bss.boot_mmu_offset,\"aw\",%nobits\n",
            ".balign 8\n",
            "_boot_mmu_offset:\n",
            "    .zero 8\n",
        )
    };
}

/// `_start` prologue: save the DTB pointer and CPU id, compute the runtime
/// load delta `x24` (link VA − physical PC), clear BSS, switch to the boot
/// stack.  Falls through in the physical world.
#[macro_export]
macro_rules! boot_setup_asm {
    () => {
        concat!(
            ".section .text.entry\n",
            ".global _start\n",
            "_start:\n",
            "    mov     x19, x0                 // DTB pointer (PA)\n",
            "    mrs     x20, mpidr_el1          // CPU ID\n",
            "\n",
            "    // x24 = link VA here - physical PC here\n",
            "    adr     x0, 9f\n",
            "    ldr     x1, =9f\n",
            "    sub     x24, x1, x0\n",
            "9:\n",
            "\n",
            "    ldr     x0, =_sbss\n",
            "    sub     x0, x0, x24\n",
            "    ldr     x1, =_ebss\n",
            "    sub     x1, x1, x24\n",
            "    cmp     x0, x1\n",
            "    b.ge    2f\n",
            "1:\n",
            "    str     xzr, [x0], #8\n",
            "    cmp     x0, x1\n",
            "    b.lt    1b\n",
            "2:\n",
            "    ldr     x0, =bstack_top\n",
            "    sub     x0, x0, x24\n",
            "    mov     sp, x0\n",
        )
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
        concat!(
            "    ldr     x1, =_boot_mmu_offset\n",
            "    sub     x1, x1, x24\n",
            "    str     x23, [x1]\n",
            "    cbz     x23, .Lkor_boot_phys\n",
            "\n",
            "    movz    x0, #0xFF00             // MAIR: normal RW + device\n",
            "    movk    x0, #0x0044, lsl #16\n",
            "    msr     mair_el1, x0\n",
            "    ldr     x0, =0x0000000575103510 // TCR_EL1\n",
            "    msr     tcr_el1, x0\n",
            "    isb\n",
            "    ldr     x0, =_boot_pt_buffer\n",
            "    sub     x0, x0, x24\n",
            "    msr     ttbr0_el1, x0\n",
            "    msr     ttbr1_el1, x0\n",
            "    tlbi    vmalle1\n",
            "    dsb     sy\n",
            "    isb\n",
            "    mrs     x0, sctlr_el1\n",
            "    movz    x1, #0x1005             // M | A | C | I | TEX remap\n",
            "    orr     x0, x0, x1\n",
            "    msr     sctlr_el1, x0\n",
            "    isb\n",
            "\n",
            "    adr     x0, .Lkor_boot_virt\n",
            "    add     x0, x0, x23\n",
            "    br      x0\n",
            ".Lkor_boot_virt:\n",
            "    add     sp, sp, x23\n",
            "    add     x19, x19, x23\n",
            ".Lkor_boot_phys:\n",
            "    and     x0, x20, #0xffffff\n",
            "    mov     x1, x19\n",
        )
    };
}

/// Call the application entry.  The offset (`x23`) and load delta (`x24`)
/// left live by [`boot_cross_asm!`] select the entry's virtual or physical
/// address; parks if the entry ever returns.
#[macro_export]
macro_rules! boot_call_asm {
    ($entry:ident) => {
        concat!(
            "    ldr     x8, =",
            stringify!($entry),
            "\n",
            "    cbz     x23, .Lkor_call_phys\n",
            "    blr     x8\n",
            "    b       .\n",
            ".Lkor_call_phys:\n",
            "    sub     x8, x8, x24\n",
            "    blr     x8\n",
            "    b       .\n",
        )
    };
}

/// Secondary-core stub (entered physically via PSCI `CPU_ON`): re-install
/// the boot root, cross the offset, and enter `rust_entry_secondary`.
/// Requires the [`boot_pt_buffer_asm!`] symbols.
#[macro_export]
macro_rules! boot_secondary_asm {
    () => {
        concat!(
            ".global _secondary_start\n",
            "_secondary_start:\n",
            "    mov     x20, x0                 // stack top (post-offset)\n",
            "    mrs     x19, mpidr_el1\n",
            "    and     x19, x19, #0xffffff\n",
            "\n",
            "    adr     x0, 8f\n",
            "    ldr     x1, =8f\n",
            "    sub     x24, x1, x0\n",
            "8:\n",
            "    ldr     x0, =_boot_mmu_offset\n",
            "    sub     x0, x0, x24\n",
            "    ldr     x23, [x0]\n",
            "    cbz     x23, .Lkor_sec_phys\n",
            "\n",
            "    movz    x0, #0xFF00\n",
            "    movk    x0, #0x0044, lsl #16\n",
            "    msr     mair_el1, x0\n",
            "    ldr     x0, =0x0000000575103510\n",
            "    msr     tcr_el1, x0\n",
            "    isb\n",
            "    ldr     x0, =_boot_pt_buffer\n",
            "    sub     x0, x0, x24\n",
            "    msr     ttbr0_el1, x0\n",
            "    msr     ttbr1_el1, x0\n",
            "    tlbi    vmalle1\n",
            "    dsb     sy\n",
            "    isb\n",
            "    mrs     x0, sctlr_el1\n",
            "    movz    x1, #0x1005\n",
            "    orr     x0, x0, x1\n",
            "    msr     sctlr_el1, x0\n",
            "    isb\n",
            "\n",
            "    adr     x0, .Lkor_sec_virt\n",
            "    add     x0, x0, x23\n",
            "    br      x0\n",
            ".Lkor_sec_virt:\n",
            "    mov     sp, x20\n",
            "    mov     x0, x19\n",
            "    ldr     x8, =rust_entry_secondary\n",
            "    blr     x8\n",
            "    b       .\n",
            ".Lkor_sec_phys:\n",
            "    mov     sp, x20\n",
            "    mov     x0, x19\n",
            "    ldr     x8, =rust_entry_secondary\n",
            "    sub     x8, x8, x24\n",
            "    blr     x8\n",
            "    b       .\n",
        )
    };
}

/// Emit the `rust_entry` wrapper: records the DTB pointer through kor-arch,
/// then tail-calls the app's `kernel_main`.
///
/// # Safety
/// `kernel_main` must exist in the invoking crate and never return.
#[macro_export]
macro_rules! boot_entry {
    () => {
        #[unsafe(no_mangle)]
        extern "C" fn rust_entry(_hart_id: usize, dtb: usize) {
            kor_arch::aarch64::mm::set_dtb_ptr(dtb);
            crate::kernel_main()
        }
    };
}
