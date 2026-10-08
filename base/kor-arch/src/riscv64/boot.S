# RISC-V boot trampoline — MMU bring-up delegated to the application.
#
# The app links in `boot_mmu_build(table_pa, table_len) -> offset` (an
# extern "C" symbol): it fills the table buffer and returns the phys->virt
# offset of the mapping it built, or 0 to run unmapped.  This trampoline is
# policy-free: BSS clear, physical stack, call the builder, enable Sv39 if
# asked, and jump to the post-offset entry — so one trampoline serves both
# MMU and no-MMU applications.
#
# Pre-MMU addressing note: with PC still physical, every `la` below yields
# the *physical* address of its symbol (PC-relative auipc), which is what
# makes the BSS clear, the stack, the buffer pointer, and the builder call
# work before any offset is known.

# SATP mode Sv39
.set SATP_MODE_SV39, (8 << 60)

# Boot page-table buffer (root level + one spare page for app policies
# that want finer granularity at boot).  Lives in BSS: the trampoline
# clears BSS before calling the builder.
.section .bss.boot_page_table,"aw",@nobits
.balign 4096
.global _boot_pt_buffer
_boot_pt_buffer:
    .space 4096 * 2

# Phys->virt offset published by the boot hart; the secondary trampoline
# reads it before enabling paging.
.section .bss.boot_mmu_offset,"aw",@nobits
.balign 8
_boot_mmu_offset:
    .space 8

.section .text.entry
.global _start
_start:
    # Save OpenSBI-provided hart_id (a0) and DTB pointer (a1)
    mv      s0, a0
    mv      s1, a1

    # --- Clear BSS (physical addresses — PC is physical before MMU) ---
    la      t0, _sbss
    la      t1, _ebss
    bgeu    t0, t1, 2f
1:
    sd      zero, (t0)
    addi    t0, t0, 8
    bltu    t0, t1, 1b
2:
    # --- Early stack (physical) ---
    la      sp, bstack_top

    # --- Ask the application to build the boot page tables ---
    la      a0, _boot_pt_buffer       # physical (no MMU yet)
    li      a1, 4096 * 2
    call    boot_mmu_build            # a0 = phys->virt offset (0 = unmapped)

    # Publish the offset for the secondary-hart trampoline.
    la      t2, _boot_mmu_offset
    sd      a0, (t2)

    beqz    a0, 5f                    # no-MMU app: stay on physical PCs
    # --- Enable Sv39 paging ---
    la      t0, _boot_pt_buffer
    srli    t0, t0, 12                # PPN for satp
    li      t1, SATP_MODE_SV39
    or      t0, t0, t1
    csrw    satp, t0
    sfence.vma

    # --- Jump to the post-offset virtual address ---
    la      t0, 3f
    add     t0, t0, a0
    jr      t0
3:
    # Now running on virtual PCs — move sp across the same offset.
    add     sp, sp, a0
    mv      a0, s0                    # restore hart_id
    mv      a1, s1                    # restore DTB pointer
    call    rust_entry
    j       .
5:
    # Unmapped path: enter Rust on physical PCs (the app must be linked
    # for a physical/identity model, like korhv does on aarch64).
    mv      a0, s0
    mv      a1, s1
    call    rust_entry
    j       .

# --- Secondary hart entry (via SBI hart_start) --------------------------
# SBI enters here in S-mode with the MMU off: a0 = hart_id, a1 = opaque
# (per-hart stack top, in the coordinate system the app published).
# Enable paging with the tables the boot hart's builder produced, switch
# across the published offset, set the stack, and call rust_entry_secondary.
.global _secondary_start
_secondary_start:
    mv      s0, a0                  # hart_id
    mv      s1, a1                  # stack top (VA after paging)

    la      t0, _boot_mmu_offset    # physical (no MMU yet)
    ld      t2, (t0)                # offset published by the boot hart
    beqz    t2, 4f                  # no-MMU app: skip enable

    la      t0, _boot_pt_buffer
    srli    t0, t0, 12
    li      t1, SATP_MODE_SV39
    or      t0, t0, t1
    csrw    satp, t0
    sfence.vma

    la      t0, 4f
    add     t0, t0, t2
    jr      t0
4:
    mv      sp, s1              # per-hart stack top (post-offset address)
    mv      tp, s0              # tp = hart_id
    mv      a0, s0
    call    rust_entry_secondary
    j       .
