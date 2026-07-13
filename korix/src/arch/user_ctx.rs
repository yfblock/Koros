//! User-mode context management.

pub struct UserContext {
    pub entry: usize,
    pub sp: usize,
}

impl UserContext {
    pub fn new(entry: usize, sp: usize) -> Self {
        Self { entry, sp }
    }
}

/// Switch to user mode. Zeros all GP registers (except sp) to prevent
/// kernel values leaking into user space.
pub unsafe fn enter_user(ctx: &UserContext) -> ! {
    #[cfg(target_arch = "riscv64")]
    {
        use core::arch::asm;

        asm!(
            // 1. Save kernel sp to sscratch.
            "csrw    sscratch, sp",
            // 2. Set sepc and sp BEFORE zeroing (entry/usp might be in any reg).
            "csrw    sepc, {entry}",
            "mv      sp, {usp}",
            // 3. Zero all GP registers except x0 and sp.
            "mv      ra, zero",
            "mv      gp, zero",
            "mv      tp, zero",
            "mv      t0, zero",
            "mv      t1, zero",
            "mv      t2, zero",
            "mv      s0, zero",
            "mv      s1, zero",
            "mv      a0, zero",
            "mv      a1, zero",
            "mv      a2, zero",
            "mv      a3, zero",
            "mv      a4, zero",
            "mv      a5, zero",
            "mv      a6, zero",
            "mv      a7, zero",
            "mv      s2, zero",
            "mv      s3, zero",
            "mv      s4, zero",
            "mv      s5, zero",
            "mv      s6, zero",
            "mv      s7, zero",
            "mv      s8, zero",
            "mv      s9, zero",
            "mv      s10, zero",
            "mv      s11, zero",
            "mv      t3, zero",
            "mv      t4, zero",
            "mv      t5, zero",
            "mv      t6, zero",
            // 4. Set SPP=0 (U-mode) and SPIE=1 (interrupts on).
            "li      {tmp}, (1 << 5)",
            "csrs    sstatus, {tmp}",
            "li      {tmp}, (1 << 8)",
            "csrc    sstatus, {tmp}",
            // 5. Flush TLB and enter user mode.
            "sfence.vma",
            "sret",
            entry = in(reg) ctx.entry,
            usp = in(reg) ctx.sp,
            tmp = out(reg) _,
        );
        unreachable!();
    }
    #[cfg(target_arch = "aarch64")]
    {
        use core::arch::asm;
        // Set ELR_EL1 to entry, SP_EL0 to user sp, SPSR_EL1 to EL0h
        // SPSR_EL1 for EL0h: M=0 (EL0t), all interrupts unmasked
        const SPSR_EL0H: usize = 0; // M[3:0]=0 → EL0t
        asm!(
            "msr     elr_el1, {entry}",
            "msr     sp_el0, {usp}",
            "msr     spsr_el1, {spsr}",
            // Zero all GP registers
            "mov     x0, xzr",
            "mov     x1, xzr",
            "mov     x2, xzr",
            "mov     x3, xzr",
            "mov     x4, xzr",
            "mov     x5, xzr",
            "mov     x6, xzr",
            "mov     x7, xzr",
            "mov     x8, xzr",
            "mov     x9, xzr",
            "mov     x10, xzr",
            "mov     x11, xzr",
            "mov     x12, xzr",
            "mov     x13, xzr",
            "mov     x14, xzr",
            "mov     x15, xzr",
            "mov     x16, xzr",
            "mov     x17, xzr",
            "mov     x18, xzr",
            "mov     x19, xzr",
            "mov     x20, xzr",
            "mov     x21, xzr",
            "mov     x22, xzr",
            "mov     x23, xzr",
            "mov     x24, xzr",
            "mov     x25, xzr",
            "mov     x26, xzr",
            "mov     x27, xzr",
            "mov     x28, xzr",
            "mov     x29, xzr",
            "mov     x30, xzr",
            "tlbi    vmalle1is",
            "dsb     sy",
            "isb",
            "eret",
            entry = in(reg) ctx.entry,
            usp = in(reg) ctx.sp,
            spsr = in(reg) SPSR_EL0H,
        );
        unreachable!();
    }

    #[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64")))]
    {
        let _ = ctx;
        loop { core::hint::spin_loop(); }
    }
}
