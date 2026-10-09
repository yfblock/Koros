//! riscv64 boot fragments (uniform names — see `crate::boot`).
//!
//! Composition order: `boot_stack_asm!` → `boot_pt_buffer_asm!` →
//! `boot_setup_asm!` → *mapping policy* (result in `a0`) →
//! `boot_cross_asm!` → `boot_call_asm!(entry)` → `boot_secondary_asm!`.

/// BSS symbols for the kor-mmu `boot_mmu_build` hook ABI: a two-page
/// page-table buffer plus the offset cell the secondary stub reads.
#[macro_export]
macro_rules! boot_pt_buffer_asm {
    () => {
        concat!(
            ".section .bss.boot_page_table,\"aw\",@nobits\n",
            ".balign 4096\n",
            ".global _boot_pt_buffer\n",
            "_boot_pt_buffer:\n",
            "    .space 4096 * 2\n",
            ".section .bss.boot_mmu_offset,\"aw\",@nobits\n",
            ".balign 8\n",
            "_boot_mmu_offset:\n",
            "    .space 8\n",
        )
    };
}

/// `_start` prologue: save firmware args, clear BSS, switch to the boot
/// stack.  Falls through in the physical world with `s0` = hart id and
/// `s1` = DTB pointer.
#[macro_export]
macro_rules! boot_setup_asm {
    () => {
        concat!(
            ".section .text.entry\n",
            ".global _start\n",
            "_start:\n",
            "    mv      s0, a0                 # hart_id\n",
            "    mv      s1, a1                 # DTB pointer\n",
            "\n",
            "    # la is physical while the PC is physical\n",
            "    la      t0, _sbss\n",
            "    la      t1, _ebss\n",
            "    bgeu    t0, t1, 2f\n",
            "1:\n",
            "    sd      zero, (t0)\n",
            "    addi    t0, t0, 8\n",
            "    bltu    t0, t1, 1b\n",
            "2:\n",
            "    la      sp, bstack_top\n",
        )
    };
}

/// Offset bookkeeping, Sv39 enable (tables at `_boot_pt_buffer`), and the
/// crossing to virtual PCs.  Expects the mapping-slot result in `a0`
/// (0 = stay unmapped); falls through with `a0`/`a1` restored and `sp`
/// offset-adjusted.
#[macro_export]
macro_rules! boot_cross_asm {
    () => {
        concat!(
            "    la      t2, _boot_mmu_offset\n",
            "    sd      a0, (t2)\n",
            "    beqz    a0, .Lkor_boot_phys\n",
            "\n",
            "    la      t0, _boot_pt_buffer\n",
            "    srli    t0, t0, 12\n",
            "    li      t1, (8 << 60)          # SATP_MODE_SV39\n",
            "    or      t0, t0, t1\n",
            "    csrw    satp, t0\n",
            "    sfence.vma\n",
            "\n",
            "    la      t0, .Lkor_boot_virt\n",
            "    add     t0, t0, a0\n",
            "    jr      t0\n",
            ".Lkor_boot_virt:\n",
            "    add     sp, sp, a0\n",
            ".Lkor_boot_phys:\n",
            "    mv      a0, s0\n",
            "    mv      a1, s1\n",
        )
    };
}

/// Call the application entry (`PC`-relative, valid in both the mapped and
/// unmapped configurations) and park if it ever returns.
#[macro_export]
macro_rules! boot_call_asm {
    ($entry:ident) => {
        concat!(
            "    call    ",
            stringify!($entry),
            "\n",
            "    j       .\n",
        )
    };
}

/// Secondary-hart stub (entered physically via SBI `hart_start`): re-install
/// the boot root, cross the offset, and enter `rust_entry_secondary`.
/// Requires the [`boot_pt_buffer_asm!`] symbols.
#[macro_export]
macro_rules! boot_secondary_asm {
    () => {
        concat!(
            ".global _secondary_start\n",
            "_secondary_start:\n",
            "    mv      s0, a0                 # hart_id\n",
            "    mv      s1, a1                 # stack top (post-offset)\n",
            "\n",
            "    la      t0, _boot_mmu_offset\n",
            "    ld      t2, (t0)\n",
            "    beqz    t2, .Lkor_sec_phys\n",
            "\n",
            "    la      t0, _boot_pt_buffer\n",
            "    srli    t0, t0, 12\n",
            "    li      t1, (8 << 60)          # SATP_MODE_SV39\n",
            "    or      t0, t0, t1\n",
            "    csrw    satp, t0\n",
            "    sfence.vma\n",
            "\n",
            "    la      t0, .Lkor_sec_virt\n",
            "    add     t0, t0, t2\n",
            "    jr      t0\n",
            ".Lkor_sec_virt:\n",
            ".Lkor_sec_phys:\n",
            "    mv      sp, s1\n",
            "    mv      tp, s0\n",
            "    mv      a0, s0\n",
            "    call    rust_entry_secondary\n",
            "    j       .\n",
        )
    };
}

/// Emit the `rust_entry` wrapper: records hart id and DTB pointer through
/// kor-arch, then tail-calls the app's `kernel_main`.
///
/// # Safety
/// `kernel_main` must exist in the invoking crate and never return.
#[macro_export]
macro_rules! boot_entry {
    () => {
        #[unsafe(no_mangle)]
        extern "C" fn rust_entry(hart_id: usize, dtb: usize) {
            kor_arch::riscv64::smp::set_cpu_id(hart_id);
            kor_arch::riscv64::mm::set_dtb_ptr(dtb);
            crate::kernel_main()
        }
    };
}
