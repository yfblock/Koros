# riscv64 trap entry.
#
# Saves all 32 GPRs + sstatus + sepc on the stack,
# calls handle_trap(tf), restores, and sret.
#
# Convention:
#   Kernel mode: sscratch = 0, sp = kernel stack
#   User mode:   sscratch = kernel sp, sp = user sp
# On entry, csrrw atomically swaps sp and sscratch:
#   User:   sp → kernel sp, sscratch → user sp
#   Kernel: sp → 0, sscratch → kernel sp (then swap back)

.section .text
.balign 4
.global kernelvec
kernelvec:
    # Atomically swap sp and sscratch — no GPR clobbered.
    csrrw   sp, sscratch, sp
    # If from user: sp = kernel sp, sscratch = user sp
    # If from kernel: sp = 0, sscratch = kernel sp
    beqz    sp, 2f

    # ---- From user mode: sp = kernel sp, sscratch has user sp ----
    addi    sp, sp, -{tf_size}

    # Save x1 - x31 (all unmodified — no scratch used)
    sd      x1,   1 * 8(sp)
    sd      x2,   2 * 8(sp)
    sd      x3,   3 * 8(sp)
    sd      x4,   4 * 8(sp)
    sd      x5,   5 * 8(sp)
    sd      x6,   6 * 8(sp)
    sd      x7,   7 * 8(sp)
    sd      x8,   8 * 8(sp)
    sd      x9,   9 * 8(sp)
    sd      x10, 10 * 8(sp)
    sd      x11, 11 * 8(sp)
    sd      x12, 12 * 8(sp)
    sd      x13, 13 * 8(sp)
    sd      x14, 14 * 8(sp)
    sd      x15, 15 * 8(sp)
    sd      x16, 16 * 8(sp)
    sd      x17, 17 * 8(sp)
    sd      x18, 18 * 8(sp)
    sd      x19, 19 * 8(sp)
    sd      x20, 20 * 8(sp)
    sd      x21, 21 * 8(sp)
    sd      x22, 22 * 8(sp)
    sd      x23, 23 * 8(sp)
    sd      x24, 24 * 8(sp)
    sd      x25, 25 * 8(sp)
    sd      x26, 26 * 8(sp)
    sd      x27, 27 * 8(sp)
    sd      x28, 28 * 8(sp)
    sd      x29, 29 * 8(sp)
    sd      x30, 30 * 8(sp)
    sd      x31, 31 * 8(sp)

    # Overwrite x2 with the user sp (saved in sscratch during swap).
    csrr    t0, sscratch
    sd      t0,   2 * 8(sp)

    # Save sstatus and sepc.
    csrr    t0, sstatus
    sd      t0, 32 * 8(sp)
    csrr    t1, sepc
    sd      t1, 33 * 8(sp)

    mv      a0, sp
    call    handle_trap

    ld      t0, 32 * 8(sp)
    ld      t1, 33 * 8(sp)
    csrw    sstatus, t0
    csrw    sepc, t1

    # Check SPP: 0 = user, 1 = kernel.
    andi    t0, t0, (1 << 8)
    bnez    t0, 3f

    # ---- User return: restore ALL registers except sp ----
    ld      x1,   1 * 8(sp)
    ld      x3,   3 * 8(sp)
    ld      x4,   4 * 8(sp)
    ld      x5,   5 * 8(sp)
    ld      x6,   6 * 8(sp)
    ld      x7,   7 * 8(sp)
    ld      x8,   8 * 8(sp)
    ld      x9,   9 * 8(sp)
    ld      x10, 10 * 8(sp)
    ld      x11, 11 * 8(sp)
    ld      x12, 12 * 8(sp)
    ld      x13, 13 * 8(sp)
    ld      x14, 14 * 8(sp)
    ld      x15, 15 * 8(sp)
    ld      x16, 16 * 8(sp)
    ld      x17, 17 * 8(sp)
    ld      x18, 18 * 8(sp)
    ld      x19, 19 * 8(sp)
    ld      x20, 20 * 8(sp)
    ld      x21, 21 * 8(sp)
    ld      x22, 22 * 8(sp)
    ld      x23, 23 * 8(sp)
    ld      x24, 24 * 8(sp)
    ld      x25, 25 * 8(sp)
    ld      x26, 26 * 8(sp)
    ld      x27, 27 * 8(sp)
    ld      x28, 28 * 8(sp)
    ld      x29, 29 * 8(sp)
    ld      x30, 30 * 8(sp)
    ld      x31, 31 * 8(sp)

    # Switch sp without clobbering any GPR:
    #   1. sscratch = trapframe base (sp)
    #   2. sp = kernel sp (trapframe + tf_size)
    #   3. csrrw sp, sscratch, sp → sp = trapframe base, sscratch = kernel sp
    #   4. ld sp, 2*8(sp) → sp = user sp from trapframe
    csrw    sscratch, sp
    addi    sp, sp, {tf_size}
    csrrw   sp, sscratch, sp
    ld      sp,   2 * 8(sp)
    sret

2:
    # ---- From kernel mode: sp = 0, sscratch = kernel sp ----
    # Swap back: sp = kernel sp, sscratch = 0.
    csrrw   sp, sscratch, sp
    # Now sscratch = 0 (kernel mode marker), sp = kernel sp.

    addi    sp, sp, -{tf_size}

    sd      x1,   1 * 8(sp)
    sd      x2,   2 * 8(sp)
    sd      x3,   3 * 8(sp)
    # x4 (tp) NOT saved — holds hart id, never modified.
    sd      x5,   5 * 8(sp)
    sd      x6,   6 * 8(sp)
    sd      x7,   7 * 8(sp)
    sd      x8,   8 * 8(sp)
    sd      x9,   9 * 8(sp)
    sd      x10, 10 * 8(sp)
    sd      x11, 11 * 8(sp)
    sd      x12, 12 * 8(sp)
    sd      x13, 13 * 8(sp)
    sd      x14, 14 * 8(sp)
    sd      x15, 15 * 8(sp)
    sd      x16, 16 * 8(sp)
    sd      x17, 17 * 8(sp)
    sd      x18, 18 * 8(sp)
    sd      x19, 19 * 8(sp)
    sd      x20, 20 * 8(sp)
    sd      x21, 21 * 8(sp)
    sd      x22, 22 * 8(sp)
    sd      x23, 23 * 8(sp)
    sd      x24, 24 * 8(sp)
    sd      x25, 25 * 8(sp)
    sd      x26, 26 * 8(sp)
    sd      x27, 27 * 8(sp)
    sd      x28, 28 * 8(sp)
    sd      x29, 29 * 8(sp)
    sd      x30, 30 * 8(sp)
    sd      x31, 31 * 8(sp)

    csrr    t0, sstatus
    sd      t0, 32 * 8(sp)
    csrr    t1, sepc
    sd      t1, 33 * 8(sp)

    mv      a0, sp
    call    handle_trap

    ld      t0, 32 * 8(sp)
    ld      t1, 33 * 8(sp)
    csrw    sstatus, t0
    csrw    sepc, t1

3:
    # ---- Kernel return ----
    ld      x1,   1 * 8(sp)
    ld      x2,   2 * 8(sp)
    ld      x3,   3 * 8(sp)
    # x4 (tp) NOT restored.
    ld      x5,   5 * 8(sp)
    ld      x6,   6 * 8(sp)
    ld      x7,   7 * 8(sp)
    ld      x8,   8 * 8(sp)
    ld      x9,   9 * 8(sp)
    ld      x10, 10 * 8(sp)
    ld      x11, 11 * 8(sp)
    ld      x12, 12 * 8(sp)
    ld      x13, 13 * 8(sp)
    ld      x14, 14 * 8(sp)
    ld      x15, 15 * 8(sp)
    ld      x16, 16 * 8(sp)
    ld      x17, 17 * 8(sp)
    ld      x18, 18 * 8(sp)
    ld      x19, 19 * 8(sp)
    ld      x20, 20 * 8(sp)
    ld      x21, 21 * 8(sp)
    ld      x22, 22 * 8(sp)
    ld      x23, 23 * 8(sp)
    ld      x24, 24 * 8(sp)
    ld      x25, 25 * 8(sp)
    ld      x26, 26 * 8(sp)
    ld      x27, 27 * 8(sp)
    ld      x28, 28 * 8(sp)
    ld      x29, 29 * 8(sp)
    ld      x30, 30 * 8(sp)
    ld      x31, 31 * 8(sp)
    csrw    sscratch, x0
    addi    sp, sp, {tf_size}
    sret
