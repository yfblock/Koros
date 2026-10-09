//! riscv64 trap vector fragments — per-application composition.
//!
//! Two self-contained vectors (the dual-mode one has internal branching
//! that prevents composition from linear fragments):
//!
//! - [`trap_kernel_vec_asm!`]: kernel-only — no sscratch, no user-mode
//!   entry/exit.  Sufficient for apps that never enter U-mode (koros).
//! - [`trap_user_vec_asm!`]: dual-mode (kernel + user) with the sscratch
//!   dance.  Requires `RISCV_USER_TRAP_SCRATCH` (from `prepare_user_entry`).
//!
//! Both call the application contract
//! `#[unsafe(no_mangle)] extern "C" fn trap_dispatch(tf: &mut TrapFrame)`
//! (link-time resolved, like `secondary_entry`).
//!
//! Composition:
//! ```ignore
//! core::arch::global_asm!(
//!     kor_arch::trap_kernel_vec_asm!(),      // or trap_user_vec_asm!()
//!     tf_size = const core::mem::size_of::<kor_arch::TrapFrame>(),
//! );
//! ```

/// The kernel-mode trap vector (`kernelvec`): pushes the frame, calls
/// `trap_dispatch`, pops the frame, and `sret`s.  No sscratch swap —
/// traps always originate from S-mode.
#[macro_export]
macro_rules! trap_kernel_vec_asm {
    () => {
        r####"# riscv64 kernel-mode trap entry (no user mode, no sscratch).

.section .text
.balign 4
.global kernelvec
kernelvec:
    addi    sp, sp, -{tf_size}

    sd      x1,   1 * 8(sp)
    sd      x2,   2 * 8(sp)
    sd      x3,   3 * 8(sp)
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
    call    trap_dispatch

    ld      t0, 32 * 8(sp)
    ld      t1, 33 * 8(sp)
    csrw    sstatus, t0
    csrw    sepc, t1

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

    addi    sp, sp, {tf_size}
    sret
"####
    };
}

/// The dual-mode trap vector (`kernelvec`): detects user vs kernel entry
/// via `sscratch`, saves the full frame (including user tp/sp from the
/// per-hart scratch), calls `trap_dispatch`, and returns via the
/// appropriate path.  Requires `RISCV_USER_TRAP_SCRATCH`.
#[macro_export]
macro_rules! trap_user_vec_asm {
    () => {
        r####"# riscv64 dual-mode trap entry (kernel + user via sscratch).
#
# Convention:
#   Kernel mode: sscratch = 0, sp = kernel stack
#   User mode:   sscratch = kernel sp, sp = user sp

.section .text
.balign 4
.global kernelvec
kernelvec:
    csrrw   sp, sscratch, sp
    beqz    sp, 2f

    # ---- From user mode: sp = scratch pointer, sscratch = user sp ----
    sd      tp, 16(sp)
    csrr    tp, sscratch
    sd      tp, 24(sp)
    csrw    sscratch, x0
    ld      tp, 8(sp)
    ld      sp, 0(sp)
    addi    sp, sp, -{tf_size}

    sd      x1,   1 * 8(sp)
    sd      x2,   2 * 8(sp)
    sd      x3,   3 * 8(sp)
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

    csrr    t0, sstatus
    sd      t0, 32 * 8(sp)
    csrr    t1, sepc
    sd      t1, 33 * 8(sp)

    mv      a0, sp
    call    trap_dispatch

    ld      t0, 32 * 8(sp)
    ld      t1, 33 * 8(sp)
    csrw    sstatus, t0
    csrw    sepc, t1

    # Check SPP: 0 = user, 1 = kernel.
    andi    t0, t0, (1 << 8)
    bnez    t0, 3f

    # Refresh the per-hart scratch from this task's frame (another user
    # task may have run on this hart while we were blocked).
    la      t1, RISCV_USER_TRAP_SCRATCH
    slli    t0, tp, 5
    add     t1, t1, t0
    ld      t0,   2 * 8(sp)
    sd      t0, 24(t1)
    ld      t0,   4 * 8(sp)
    sd      t0, 16(t1)

    # ---- User return ----
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
    csrrw   sp, sscratch, sp
    addi    sp, sp, -{tf_size}

    sd      x1,   1 * 8(sp)
    sd      x2,   2 * 8(sp)
    sd      x3,   3 * 8(sp)
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
    call    trap_dispatch

    ld      t0, 32 * 8(sp)
    ld      t1, 33 * 8(sp)
    csrw    sstatus, t0
    csrw    sepc, t1

3:
    # ---- Kernel return ----
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

    csrw    sscratch, x0
    addi    sp, sp, {tf_size}
    sret
"####
    };
}
