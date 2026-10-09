//! riscv64 boot for this application.
//!
//! Trampoline composed from kor-arch fragments (uniform names); the only
//! arch-specific text is the mapping policy below — this app asks
//! kor-mmu's `boot_mmu_build` hook for the default boot mapping.

core::arch::global_asm!(
    kor_arch::boot_stack_asm!(0x80000),
    kor_arch::boot_pt_buffer_asm!(),
    kor_arch::boot_setup_asm!(),
    // --- App policy: no MMU — stay physical (a0 = 0 = unmapped). ---
    "
    mv      a0, zero
    ",
    kor_arch::boot_cross_asm!(),
    kor_arch::boot_call_asm!(kernel_main),
    kor_arch::boot_secondary_asm!(),
);
