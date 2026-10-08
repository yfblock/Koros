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
    # sscratch points to UserTrapScratch on user entry and is zero in kernel.
    csrrw   sp, sscratch, sp
    beqz    sp, 2f

    # ---- From user mode: sp = scratch pointer, sscratch = user sp ----
    sd      tp, 16(sp)
    csrr    tp, sscratch
    sd      tp, 24(sp)
    # Nested kernel traps must observe the kernel-mode marker while Rust runs.
    csrw    sscratch, x0
    ld      tp, 8(sp)
    ld      sp, 0(sp)
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

    # Save user sp and tp from the per-hart scratch.
    la      t1, RISCV_USER_TRAP_SCRATCH
    slli    t0, tp, 5
    add     t1, t1, t0
    ld      t0, 24(t1)
    sd      t0,   2 * 8(sp)
    ld      t0, 16(t1)
    sd      t0,   4 * 8(sp)

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

    # A task may have blocked in its trap handler while another user task ran
    # on this hart. Refresh the per-hart scratch from this task's trap frame
    # before restoring registers so the final user sp/tp loads cannot observe
    # the other task's values.
    la      t1, RISCV_USER_TRAP_SCRATCH
    slli    t0, tp, 5
    add     t1, t1, t0
    ld      t0,   2 * 8(sp)
    sd      t0, 24(t1)
    ld      t0,   4 * 8(sp)
    sd      t0, 16(t1)

    # ---- User return: restore ALL registers except sp ----
    ld      x1,   1 * 8(sp)
    ld      x3,   3 * 8(sp)
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

    # Only sp and tp are used as scratch because all other user registers live.
    csrw    sscratch, sp
    slli    sp, tp, 5
    la      tp, RISCV_USER_TRAP_SCRATCH
    add     tp, tp, sp
    csrr    sp, sscratch
    addi    sp, sp, {tf_size}
    sd      sp, 0(tp)
    csrw    sscratch, tp
    ld      sp, 24(tp)
    ld      tp, 16(tp)
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

# Resume an owned UserTrapContext. Offsets follow kor_arch::UserTrapContext:
# regs[32], pc, status, sp, tls. a0 points to the context.
.global riscv_resume_user
riscv_resume_user:
    mv      t6, a0
    ld      t0, 32 * 8(t6)
    csrw    sepc, t0
    ld      t0, 33 * 8(t6)
    li      t1, ((1 << 5) | (3 << 13))
    or      t0, t0, t1
    li      t1, (1 << 8)
    not     t1, t1
    and     t0, t0, t1
    csrw    sstatus, t0
    # `prepare_user_entry` left sscratch pointing at this hart's trap record.
    ld      ra,   1 * 8(t6)
    ld      gp,   3 * 8(t6)
    ld      tp,  35 * 8(t6)
    ld      t0,   5 * 8(t6)
    ld      t1,   6 * 8(t6)
    ld      t2,   7 * 8(t6)
    ld      s0,   8 * 8(t6)
    ld      s1,   9 * 8(t6)
    ld      a0,  10 * 8(t6)
    ld      a1,  11 * 8(t6)
    ld      a2,  12 * 8(t6)
    ld      a3,  13 * 8(t6)
    ld      a4,  14 * 8(t6)
    ld      a5,  15 * 8(t6)
    ld      a6,  16 * 8(t6)
    ld      a7,  17 * 8(t6)
    ld      s2,  18 * 8(t6)
    ld      s3,  19 * 8(t6)
    ld      s4,  20 * 8(t6)
    ld      s5,  21 * 8(t6)
    ld      s6,  22 * 8(t6)
    ld      s7,  23 * 8(t6)
    ld      s8,  24 * 8(t6)
    ld      s9,  25 * 8(t6)
    ld      s10, 26 * 8(t6)
    ld      s11, 27 * 8(t6)
    ld      t3,  28 * 8(t6)
    ld      t4,  29 * 8(t6)
    ld      t5,  30 * 8(t6)
    ld      sp,  34 * 8(t6)
    ld      t6,  31 * 8(t6)
    fence.i
    sret
