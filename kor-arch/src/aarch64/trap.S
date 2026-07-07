// AArch64 exception vector table.
//
// 16 entries × 128 bytes each = 2 KiB, aligned to 2 KiB.
//
// Each entry saves the full context (31 GPRs + SP_EL0 + ELR + SPSR + TPIDR)
// and calls handle_trap(frame, kind, source).
//
// TrapFrame layout:
//   offset 0:   x0 – x30  (31 × 8 = 248 bytes)
//   offset 248: sp        (8 bytes)
//   offset 256: elr       (8 bytes)
//   offset 264: spsr      (8 bytes)
//   offset 272: tpidr     (8 bytes)
//   total: 280 bytes

.equ TF_REGS,   0
.equ TF_SP,     248
.equ TF_ELR,    256
.equ TF_SPSR,   264
.equ TF_TPIDR,  272
.equ TF_SIZE,   280

// Macro to handle a trap at current EL (kernel).
// kind: 0=Sync, 1=IRQ, 2=FIQ, 3=SError
// source: 0=SP_EL0, 1=SP_ELx
.macro TRAP_CURRENT kind, source
    .p2align 7
    // Disable interrupts
    msr     daifset, #2

    // Allocate TrapFrame on stack
    sub     sp, sp, TF_SIZE

    // Save x0 – x30
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

    // Save SP_EL0, ELR_EL1, SPSR_EL1, TPIDR_EL0
    mrs     x9,  sp_el0
    mrs     x10, elr_el1
    mrs     x11, spsr_el1
    mrs     x12, tpidr_el0
    stp     x30, x9,  [sp, #TF_SP]      // sp slot: store x30 as placeholder first
    stp     x10, x11, [sp, #TF_ELR]
    str     x12,      [sp, #TF_TPIDR]
    // Fix sp slot - actually store SP_EL0
    stp     x9,  x10, [sp, #TF_SP]      // SP, ELR
    // Store SPSR
    str     x11,      [sp, #TF_SPSR]

    // Call handle_trap(tf, kind, source)
    mov     x0, sp
    mov     x1, #\kind
    mov     x2, #\source
    bl      handle_trap

    // Restore and return
    b       .Lexception_return
.endm

// Macro for lower-EL (user) traps — currently just panics.
// We don't have userspace yet.
.macro TRAP_LOWER kind
    .p2align 7
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
    stp     x9,  x10, [sp, #TF_SP]
    str     x11,      [sp, #TF_SPSR]
    str     x12,      [sp, #TF_TPIDR]
    mov     x0, sp
    mov     x1, #\kind
    mov     x2, #2           // source = LowerEL AArch64
    bl      handle_trap
    b       .Lexception_return
.endm

.section .text
.p2align 12
.global exception_vector_base
exception_vector_base:
    // Current EL with SP_EL0
    TRAP_CURRENT 0 0        // Synchronous
    TRAP_CURRENT 1 0        // IRQ
    TRAP_CURRENT 2 0        // FIQ
    TRAP_CURRENT 3 0        // SError

    // Current EL with SP_ELx
    TRAP_CURRENT 0 1        // Synchronous
    TRAP_CURRENT 1 1        // IRQ
    TRAP_CURRENT 2 1        // FIQ
    TRAP_CURRENT 3 1        // SError

    // Lower EL, AArch64
    TRAP_LOWER 0            // Synchronous
    TRAP_LOWER 1            // IRQ
    TRAP_LOWER 2            // FIQ
    TRAP_LOWER 3            // SError

    // Lower EL, AArch32
    .p2align 7
    b   .                    // Sync  (AArch32 — not supported)
    .p2align 7
    b   .                    // IRQ
    .p2align 7
    b   .                    // FIQ
    .p2align 7
    b   .                    // SError

.Lexception_return:
    // Restore TPIDR, SPSR, ELR, SP
    ldr     x12, [sp, #TF_TPIDR]
    ldr     x11, [sp, #TF_SPSR]
    ldp     x10, x9,  [sp, #TF_ELR]    // ELR, SP (note: sp stored in elr+8 slot)
    // Fix: sp is at TF_SP, elr at TF_ELR
    ldp     x9,  x10, [sp, #TF_SP]     // x9=SP, x10=ELR
    ldr     x11,      [sp, #TF_SPSR]
    ldr     x12,      [sp, #TF_TPIDR]
    msr     sp_el0, x9
    msr     elr_el1, x10
    msr     spsr_el1, x11
    msr     tpidr_el0, x12

    // Restore x0 – x30
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
