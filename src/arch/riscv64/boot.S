# RISC-V Sv39 boot — page table setup and MMU enable.
#
# Uses 1 GiB superpages at L2 level to map the low 4 GiB at both
# identity and high-half addresses. This covers the kernel (0x80000000)
# and UART (0x10000000).

# Page table flags
.set PTE_V, (1 << 0)
.set PTE_R, (1 << 1)
.set PTE_W, (1 << 2)
.set PTE_X, (1 << 3)
.set PTE_A, (1 << 6)
.set PTE_D, (1 << 7)
.set LEAF_FLAGS, (PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D)

# SATP mode Sv39
.set SATP_MODE_SV39, (8 << 60)

.section .data.boot_page_table
.balign 4096

# Root page table (L2) — using 1 GiB leaf entries
_boot_root_table:
    # Entry 0: 0x00000000-0x3FFFFFFF
    .quad (0x00000000 >> 2) | LEAF_FLAGS
    # Entry 1: 0x40000000-0x7FFFFFFF
    .quad (0x40000000 >> 2) | LEAF_FLAGS
    # Entry 2: 0x80000000-0xBFFFFFFF (kernel)
    .quad (0x80000000 >> 2) | LEAF_FLAGS
    # Entry 3: 0xC0000000-0xFFFFFFFF
    .quad (0xC0000000 >> 2) | LEAF_FLAGS
    # Entries 4-255: unused
    .zero (252 * 8)
    # Entry 256: 0xFFFFFFC000000000-0xFFFFFFC03FFFFFFF
    .quad (0x00000000 >> 2) | LEAF_FLAGS
    # Entry 257: 0xFFFFFFC040000000-0xFFFFFFC07FFFFFFF
    .quad (0x40000000 >> 2) | LEAF_FLAGS
    # Entry 258: 0xFFFFFFC080000000-0xFFFFFFC0BFFFFFFF (kernel high-half)
    .quad (0x80000000 >> 2) | LEAF_FLAGS
    # Entry 259: 0xFFFFFFC0C0000000-0xFFFFFFC0FFFFFFFF
    .quad (0xC0000000 >> 2) | LEAF_FLAGS
    # Entries 260-511: unused
    .zero (252 * 8)

.section .text.entry
.global _start
_start:
    # --- Clear BSS (physical addresses — PC is physical before MMU) ---
    la      t0, _sbss               # physical addr: auipc uses physical PC
    la      t1, _ebss
    bgeu    t0, t1, 2f
1:
    sd      zero, (t0)
    addi    t0, t0, 8
    bltu    t0, t1, 1b
2:

    # --- Set stack pointer (physical address) ---
    la      sp, bstack_top

    # --- Enable Sv39 paging ---
    la      t0, _boot_root_table    # physical address (no MMU yet)
    srli    t0, t0, 12              # PPN for satp
    li      t1, SATP_MODE_SV39
    or      t0, t0, t1
    csrw    satp, t0
    sfence.vma

    # --- Jump to high-half virtual address ---
    la      t0, 3f                  # physical address of label 3
    li      t1, {kernel_offset}
    add     t0, t0, t1              # virtual address
    jr      t0
3:
    # Now running at high-half VA
    call    rust_entry
    j       .
