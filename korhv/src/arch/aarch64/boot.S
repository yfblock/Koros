// aarch64 EL2 boot -- identity map at EL2, enable MMU, call rust_entry.
//
// Non-VHE EL2 has a single translation regime (TTBR0_EL2 only, no TTBR1), so
// the high-half trick used at EL1 does not work: we run identity-mapped
// (VA == PA) and link the kernel at its physical load address (0x40080000).

.set PTE_VALID,      (1 << 0)
.set PTE_TYPE_TABLE, (1 << 1)
.set PTE_AF,         (1 << 10)
.set PTE_ATTR_INDX,  (1 << 2)       // MAIR index 1 = Normal WB

.set L1_BLOCK_FLAGS, (PTE_VALID | PTE_AF | PTE_ATTR_INDX)
.set L0_TABLE_FLAGS, (PTE_VALID | PTE_TYPE_TABLE)

.section .data.boot_page_table
.balign 4096
_boot_page_table:
    .zero 8192                       // L0 (4K) + L1 (4K)

.section .text.entry
.global _start
_start:
    mov     x19, x0                  // save DTB pointer (x0 on entry)
    mrs     x20, mpidr_el1           // save CPU id

    // --- Clear BSS (identity / physical addresses, MMU off) ---
    ldr     x0, =_sbss
    ldr     x1, =_ebss
    cmp     x0, x1
    b.ge    2f
1:
    str     xzr, [x0], #8
    cmp     x0, x1
    b.lt    1b
2:

    // --- Set stack pointer (identity address) ---
    ldr     x0, =bstack_top
    mov     sp, x0

    // --- Build page tables: L0[0] -> L1; L1[0..2] = 1 GiB blocks (0, 0x40000000) ---
    ldr     x0, =_boot_page_table
    add     x1, x0, #0x1000          // L1 table
    orr     x1, x1, #L0_TABLE_FLAGS
    str     x1, [x0]                 // L0[0] = L1 | table
    add     x2, x0, #0x1000          // L1 base
    movz    x3, #L1_BLOCK_FLAGS
    str     x3, [x2]                 // L1[0] = 0x00000000 block (devices, low)
    movz    x4, #0x0000
    movk    x4, #0x4000, lsl #16     // x4 = 0x40000000
    orr     x5, x4, x3
    str     x5, [x2, #8]             // L1[1] = 0x40000000 block (RAM)

    // --- MAIR_EL2: Attr0=Device(0x00), Attr1=Normal-WB(0xFF) ---
    movz    x0, #0xFF00
    movk    x0, #0x0044, lsl #16
    msr     mair_el2, x0

    // --- TCR_EL2: T0SZ=16, TG0=4K, SH0=inner, ORGN0=WB, IRGN0=WB, PS=40-bit ---
    ldr     x0, =0x23510
    msr     tcr_el2, x0
    isb

    // --- TTBR0_EL2 ---
    ldr     x0, =_boot_page_table
    msr     ttbr0_el2, x0
    tlbi    alle2
    dsb     sy
    isb

    // --- Enable EL2 MMU + I/D cache ---
    mrs     x0, sctlr_el2
    movz    x1, #0x1005              // M | C | I
    orr     x0, x0, x1
    msr     sctlr_el2, x0
    isb

    // Identity-mapped: PC stays valid, no high-half jump needed.
    and     x0, x20, #0xffffff       // hart_id from mpidr_el1
    mov     x1, x19                  // DTB pointer
    ldr     x8, =rust_entry
    blr     x8
    b       .
