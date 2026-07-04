# riscv64 trap entry.
#
# Saves all 32 GPRs + sstatus + sepc on the stack,
# calls handle_trap(tf), restores, and sret.
#
# TrapFrame layout (must match trap.rs):
#   offset 0:   x0   – x31  (32 × 8 = 256 bytes)
#   offset 256: sstatus     (8 bytes)
#   offset 264: sepc        (8 bytes)
#   total: 272 bytes

.section .text
.balign 4
.global kernelvec
kernelvec:
    # Allocate TrapFrame on stack
    addi    sp, sp, -{tf_size}

    # Save x1 – x31 (skip x0, it's always 0)
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

    # Save sstatus and sepc
    csrr    t0, sstatus
    sd      t0, 32 * 8(sp)
    csrr    t1, sepc
    sd      t1, 33 * 8(sp)

    # Call the Rust handler
    mv      a0, sp
    call    handle_trap

    # Restore sstatus and sepc
    ld      t0, 32 * 8(sp)
    ld      t1, 33 * 8(sp)
    csrw    sstatus, t0
    csrw    sepc, t1

    # Restore x1 – x31.  NOTE: x4 (tp) is deliberately NOT restored — it holds
    # this hart's CPU id (per-CPU, fixed in kernel mode).  A task preempted on
    # one hart may resume on another after migration; restoring the saved tp
    # would carry the old hart's id and corrupt cpu_id()/per-CPU state.
    ld      x1,   1 * 8(sp)
    ld      x2,   2 * 8(sp)
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

    # Deallocate TrapFrame
    addi    sp, sp, {tf_size}

    sret
