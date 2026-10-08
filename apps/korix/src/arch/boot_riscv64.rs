//! riscv64 boot — generic MMU trampoline (see `boot.S`).
//!
//! The application provides `boot_mmu_build`, which fills the boot table
//! buffer and returns the phys→virt offset; this module only wires the
//! trampoline and hands control to `kernel_main`.

use core::arch::global_asm;

global_asm!(include_str!("boot_riscv64.S"));

#[unsafe(no_mangle)]
extern "C" fn rust_entry(hart_id: usize, dtb: usize) {
    // Whichever hart SBI hands control to is the boot hart (not necessarily
    // hart 0).  Record its id and run the kernel; secondaries are started
    // later via `smp::boot_secondaries`.
    kor_arch::riscv64::smp::set_cpu_id(hart_id);
    kor_arch::riscv64::mm::set_dtb_ptr(dtb);
    // `kernel_main` is this crate's own entry; it never returns.
    crate::kernel_main()
}
