//! Early-boot MMU policy hook for the kor-arch trampoline.
//!
//! `boot.S` calls [`boot_mmu_build`] with the MMU off at a physical PC;
//! this builds the default Sv39 boot root (via kor-mmu's pure encoder) and
//! reports the phys→virt offset.  A different application policy — or a
//! no-MMU build returning 0 — replaces this function.

/// Boot-table policy: identity-low + high-half 1 GiB leaves (Sv39).
///
/// # Safety
/// Runs with the MMU off on the trampoline's physical stack: must stay
/// pure and static-free (no panics, no globals) until paging is enabled.
#[cfg(target_arch = "riscv64")]
#[unsafe(no_mangle)]
unsafe extern "C" fn boot_mmu_build(table_pa: usize, _table_len: usize) -> usize {
    // SAFETY: the trampoline passes the physical address of the aligned
    // 8 KiB BSS buffer it owns for exactly this call.
    let table = unsafe { &mut *(table_pa as *mut [usize; 512]) };
    kor_mmu::arch::riscv64::boot_table_identity_high(table)
}
