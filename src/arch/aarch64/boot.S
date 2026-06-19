// AArch64 boot — page table setup, MMU enable, jump to high-half VA.
//
// Reference: polyhal/polyhal-boot/src/arch/aarch64.rs
// VIRT_ADDR_START = 0xFFFF000000000000

// PTE flags
.set PTE_VALID,      (1 << 0)
.set PTE_TYPE_TABLE, (1 << 1)   // set for table/page, clear for block
.set PTE_AF,         (1 << 10)
.set PTE_ATTR_INDX,  (1 << 2)   // MAIR index 1 = Normal, Write-Back cacheable

// L1 1 GiB block descriptor: VALID | AF | AttrIndx=1
.set L1_BLOCK_FLAGS, (PTE_VALID | PTE_AF | PTE_ATTR_INDX)

// L0 table descriptor: VALID | TYPE_TABLE
.set L0_TABLE_FLAGS, (PTE_VALID | PTE_TYPE_TABLE)

// Kernel virtual-to-physical offset
.set KERNEL_OFFSET, 0xFFFF000000000000

.section .data.boot_page_table
.balign 4096

// BOOT_PT: two pages
// Page 0: L0 table (512 entries)
// Page 1: L1 table (512 entries of 1 GiB blocks)
_boot_page_table:
    .zero 8192

.section .text.entry
.global _start
_start:
    mov     x19, x0

    // --- Clear BSS (physical addresses, MMU off) ---
    ldr     x0, =_sbss
    movz    x1, #0xFFFF, lsl #48   // x1 = KERNEL_OFFSET
    sub     x0, x0, x1              // physical _sbss
    ldr     x1, =_ebss
    movz    x2, #0xFFFF, lsl #48   // x2 = KERNEL_OFFSET
    sub     x1, x1, x2              // physical _ebss
    cmp     x0, x1
    b.ge    2f
1:
    str     xzr, [x0], #8
    cmp     x0, x1
    b.lt    1b
2:

    // --- Set stack pointer (physical address) ---
    ldr     x0, =bstack_top
    movz    x1, #0xFFFF, lsl #48   // x1 = KERNEL_OFFSET
    sub     x0, x0, x1
    mov     sp, x0

    // --- Build page tables ---
    // L0[0] -> L1 table (at BOOT_PT + 4096)
    ldr     x0, =_boot_page_table
    sub     x0, x0, x1
    add     x1, x0, #0x1000
    orr     x1, x1, #L0_TABLE_FLAGS
    str     x1, [x0]

    // L1[i] = 1 GiB block for i * 0x40000000
    add     x2, x0, #0x1000
    mov     x3, #0
    mov     x4, #0x200
    movz    x6, #L1_BLOCK_FLAGS
    movz    x7, #0x0000, lsl #0
    movk    x7, #0x4000, lsl #16
1:
    orr     x5, x3, x6
    str     x5, [x2], #8
    add     x3, x3, x7
    subs    x4, x4, #1
    b.ne    1b

    // --- Init MMU ---
    // MAIR_EL1: Attr0=Device(0x00), Attr1=Normal-WB(0xFF), Attr2=Normal-NonCache(0x44)
    movz    x0, #0xFF00
    movk    x0, #0x0044, lsl #16
    msr     mair_el1, x0

    // TCR_EL1: IPS=48, T0SZ=16, T1SZ=16, inner-shareable, cacheable
    ldr     x0, =0x0000000535103510
    msr     tcr_el1, x0
    isb

    // TTBR0_EL1 and TTBR1_EL1 point to same root table
    ldr     x0, =_boot_page_table
    movz    x1, #0xFFFF, lsl #48   // x1 = KERNEL_OFFSET
    sub     x0, x0, x1
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x0

    // Flush TLB
    tlbi    vmalle1
    dsb     sy
    isb

    // Enable MMU + I-cache + D-cache
    mrs     x0, sctlr_el1
    movz    x1, #0x1005
    orr     x0, x0, x1
    msr     sctlr_el1, x0
    isb

    // --- Jump to high-half VA ---
    adr     x0, 3f
    movz    x1, #0xFFFF, lsl #48   // x1 = KERNEL_OFFSET
    add     x0, x0, x1
    br      x0
3:
    // Now running at high-half VA
    mov     x0, x19
    ldr     x8, =rust_entry
    blr     x8
    b       .
