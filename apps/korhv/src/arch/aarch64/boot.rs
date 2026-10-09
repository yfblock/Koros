//! aarch64 EL2 boot for this hypervisor.
//!
//! Trampoline composed from kor-arch fragments (uniform names) plus this
//! app's EL2 mapping policy: non-VHE EL2 has a single translation regime
//! (TTBR0_EL2 only, no TTBR1), so the high-half trick does not work — the
//! host runs identity-mapped (VA == PA, link address == load address) and
//! kor-arch's setup fragment degenerates cleanly (its load-delta `x24`
//! computes 0).  `x23` (the offset register `boot_call_asm!` tests) is
//! zeroed by the policy to select the physical call path.  `rust_entry`
//! is kor-arch's (it stashes the DTB pointer, then tail-calls the entry).

/// L0 entry pointing at the L1 table (valid + table type).
const L0_TABLE_FLAGS: usize = (1 << 0) | (1 << 1);
/// L1 block entry for low MMIO (valid + accessed, MAIR attr 0 = Device).
const L1_DEVICE_FLAGS: usize = (1 << 0) | (1 << 10);
/// L1 block entry for RAM (valid + accessed, MAIR attr 1 = Normal-WB).
const L1_NORMAL_FLAGS: usize = (1 << 0) | (1 << 10) | (1 << 2);
/// TCR_EL2: T0SZ=16, TG0=4K, SH0=inner, ORGN0/IRGN0=WB, PS=40-bit.
const TCR_EL2_VAL: usize = 0x2_3510;
/// SCTLR_EL2 enable bits: M (MMU) | C (D-cache) | I (I-cache).
const SCTLR_EL2_ENABLE: usize = 0x1005;

core::arch::global_asm!(
    kor_arch::boot_stack_asm!(0x80000),
    kor_arch::boot_setup_asm!(),
    // --- App policy: EL2 identity mapping (VA == PA) ---
    // Contract: x19/x20/x24 and sp preserved; x23 = offset (0 here).
    "
    .section .data.boot_page_table
    .balign 4096
    .global _boot_page_table
_boot_page_table:
    .zero 8192                       // L0 (4K) + L1 (4K)

    .section .text.entry
    ldr     x0, =_boot_page_table
    add     x2, x0, #0x1000          // clean L1 base pointer
    orr     x1, x2, #{l0_table_flags}
    str     x1, [x0]                 // L0[0] = L1 | table
    movz    x3, #{l1_device_flags}
    str     x3, [x2]                 // L1[0] = low MMIO, Device-nGnRnE
    movz    x4, #0x0000
    movk    x4, #0x4000, lsl #16     // x4 = 0x40000000
    movz    x5, #{l1_normal_flags}
    orr     x5, x4, x5
    str     x5, [x2, #8]             // L1[1] = RAM, Normal-WB

    // MAIR_EL2: Attr0 = Device (0x00), Attr1 = Normal-WB (0xFF)
    movz    x0, #0xFF00
    movk    x0, #0x0044, lsl #16
    msr     mair_el2, x0

    // TCR_EL2
    ldr     x0, ={tcr_el2}
    msr     tcr_el2, x0
    isb

    ldr     x0, =_boot_page_table
    msr     ttbr0_el2, x0
    tlbi    alle2
    dsb     sy
    isb

    // Enable the EL2 MMU + I/D cache
    mrs     x0, sctlr_el2
    movz    x1, #{sctlr_el2_enable}
    orr     x0, x0, x1
    msr     sctlr_el2, x0
    isb

    mov     x23, xzr                 // offset = 0: identity, stay physical
    and     x0, x20, #0xffffff       // a0 = hart id (mpidr)
    mov     x1, x19                  // a1 = DTB pointer
    ",
    kor_arch::boot_call_asm!(kernel_main),
    l0_table_flags  = const L0_TABLE_FLAGS,
    l1_device_flags = const L1_DEVICE_FLAGS,
    l1_normal_flags = const L1_NORMAL_FLAGS,
    tcr_el2         = const TCR_EL2_VAL,
    sctlr_el2_enable = const SCTLR_EL2_ENABLE,
);
