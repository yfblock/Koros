//! aarch64 trap vector fragments (see `crate::boot` for the composition
//! model).  The vector table + save/restore path is pure mechanism; it calls
//! the application dispatch contract
//! `#[unsafe(no_mangle)] extern "C" fn trap_dispatch(tf: &mut TrapFrame, kind: u64, source: u64)`
//! (link-time resolved).
//!
//! Composition: `core::arch::global_asm!(kor_arch::trap_kernel_vec_asm!());`

/// The 16-entry EL1 exception vector table (`exception_vector_base`) plus
/// the register save/restore paths.  `kind`: 0 = sync, 1 = IRQ, 2 = FIQ,
/// 3 = SError; `source`: 0 = current-EL SP0, 1 = current-EL SPx,
/// 2 = lower-EL AArch64, 3 = lower-EL AArch32.
#[macro_export]
macro_rules! trap_kernel_vec_asm {
    () => {
        r####"// AArch64 exception vector table.
//
// Hardware assigns exactly 128 bytes to each of the 16 entries.  Keep the
// table itself to branch stubs and put the full register-save paths after it.
//
// TrapFrame layout (the final 8 bytes are ABI-alignment padding):
//   offset 0:   x0-x30   (31 * 8 = 248 bytes)
//   offset 248: SP_EL0
//   offset 256: ELR_EL1
//   offset 264: SPSR_EL1
//   offset 272: TPIDR_EL0
//   offset 280: padding
//   total:      288 bytes

.equ TF_REGS,   0
.equ TF_SP,     248
.equ TF_ELR,    256
.equ TF_SPSR,   264
.equ TF_TPIDR,  272
.equ TF_SIZE,   288

.macro VECTOR target
    b       \target
    .space  124
.endm

.macro SAVE_AND_CALL kind, source
    msr     daifset, #2
    sub     sp, sp, TF_SIZE

    stp     x0,  x1,  [sp, #(TF_REGS + 0 * 8)]
    stp     x2,  x3,  [sp, #(TF_REGS + 2 * 8)]
    stp     x4,  x5,  [sp, #(TF_REGS + 4 * 8)]
    stp     x6,  x7,  [sp, #(TF_REGS + 6 * 8)]
    stp     x8,  x9,  [sp, #(TF_REGS + 8 * 8)]
    stp     x10, x11, [sp, #(TF_REGS + 10 * 8)]
    stp     x12, x13, [sp, #(TF_REGS + 12 * 8)]
    stp     x14, x15, [sp, #(TF_REGS + 14 * 8)]
    stp     x16, x17, [sp, #(TF_REGS + 16 * 8)]
    stp     x18, x19, [sp, #(TF_REGS + 18 * 8)]
    stp     x20, x21, [sp, #(TF_REGS + 20 * 8)]
    stp     x22, x23, [sp, #(TF_REGS + 22 * 8)]
    stp     x24, x25, [sp, #(TF_REGS + 24 * 8)]
    stp     x26, x27, [sp, #(TF_REGS + 26 * 8)]
    stp     x28, x29, [sp, #(TF_REGS + 28 * 8)]
    str     x30,      [sp, #(TF_REGS + 30 * 8)]

    mrs     x9,  sp_el0
    mrs     x10, elr_el1
    mrs     x11, spsr_el1
    mrs     x12, tpidr_el0
    str     x9,  [sp, #TF_SP]
    str     x10, [sp, #TF_ELR]
    str     x11, [sp, #TF_SPSR]
    str     x12, [sp, #TF_TPIDR]

    mov     x0, sp
    mov     x1, #\kind
    mov     x2, #\source
    bl      trap_dispatch
    b       .Lexception_return
.endm

.section .text
.balign 2048
.global exception_vector_base
exception_vector_base:
    // Current EL using SP_EL0.
    VECTOR  .Lcurrent_sp0_sync
    VECTOR  .Lcurrent_sp0_irq
    VECTOR  .Lcurrent_sp0_fiq
    VECTOR  .Lcurrent_sp0_serror

    // Current EL using SP_ELx.
    VECTOR  .Lcurrent_spx_sync
    VECTOR  .Lcurrent_spx_irq
    VECTOR  .Lcurrent_spx_fiq
    VECTOR  .Lcurrent_spx_serror

    // Lower EL using AArch64.
    VECTOR  .Llower_a64_sync
    VECTOR  .Llower_a64_irq
    VECTOR  .Llower_a64_fiq
    VECTOR  .Llower_a64_serror

    // Lower EL using AArch32 is unsupported.
    VECTOR  .Llower_a32
    VECTOR  .Llower_a32
    VECTOR  .Llower_a32
    VECTOR  .Llower_a32


.Lcurrent_sp0_sync:
    SAVE_AND_CALL 0, 0
.Lcurrent_sp0_irq:
    SAVE_AND_CALL 1, 0
.Lcurrent_sp0_fiq:
    SAVE_AND_CALL 2, 0
.Lcurrent_sp0_serror:
    SAVE_AND_CALL 3, 0

.Lcurrent_spx_sync:
    SAVE_AND_CALL 0, 1
.Lcurrent_spx_irq:
    SAVE_AND_CALL 1, 1
.Lcurrent_spx_fiq:
    SAVE_AND_CALL 2, 1
.Lcurrent_spx_serror:
    SAVE_AND_CALL 3, 1

.Llower_a64_sync:
    SAVE_AND_CALL 0, 2
.Llower_a64_irq:
    SAVE_AND_CALL 1, 2
.Llower_a64_fiq:
    SAVE_AND_CALL 2, 2
.Llower_a64_serror:
    SAVE_AND_CALL 3, 2

.Llower_a32:
    b       .Llower_a32

.Lexception_return:
    ldr     x9,  [sp, #TF_SP]
    ldr     x10, [sp, #TF_ELR]
    ldr     x11, [sp, #TF_SPSR]
    ldr     x12, [sp, #TF_TPIDR]
    msr     sp_el0, x9
    msr     elr_el1, x10
    msr     spsr_el1, x11
    msr     tpidr_el0, x12

    ldp     x0,  x1,  [sp, #(TF_REGS + 0 * 8)]
    ldp     x2,  x3,  [sp, #(TF_REGS + 2 * 8)]
    ldp     x4,  x5,  [sp, #(TF_REGS + 4 * 8)]
    ldp     x6,  x7,  [sp, #(TF_REGS + 6 * 8)]
    ldp     x8,  x9,  [sp, #(TF_REGS + 8 * 8)]
    ldp     x10, x11, [sp, #(TF_REGS + 10 * 8)]
    ldp     x12, x13, [sp, #(TF_REGS + 12 * 8)]
    ldp     x14, x15, [sp, #(TF_REGS + 14 * 8)]
    ldp     x16, x17, [sp, #(TF_REGS + 16 * 8)]
    ldp     x18, x19, [sp, #(TF_REGS + 18 * 8)]
    ldp     x20, x21, [sp, #(TF_REGS + 20 * 8)]
    ldp     x22, x23, [sp, #(TF_REGS + 22 * 8)]
    ldp     x24, x25, [sp, #(TF_REGS + 24 * 8)]
    ldp     x26, x27, [sp, #(TF_REGS + 26 * 8)]
    ldp     x28, x29, [sp, #(TF_REGS + 28 * 8)]
    ldr     x30,      [sp, #(TF_REGS + 30 * 8)]
    add     sp, sp, TF_SIZE
    eret
"####
    };
}
