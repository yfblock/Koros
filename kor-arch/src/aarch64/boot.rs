//! aarch64 boot — page table setup, MMU enable, jump to high-half VA.

use core::arch::global_asm;

global_asm!(include_str!("boot.S"));

#[unsafe(no_mangle)]
extern "C" fn rust_entry(_hart_id: usize, dtb: usize) {
    // The CPU that enters here is the boot CPU; secondaries (once PSCI
    // bring-up is implemented) will enter through a separate path.
    crate::aarch64::mm::set_dtb_ptr(dtb);
    // SAFETY: `kernel_main` is provided by the `koros` binary crate.
    unsafe { crate::kernel_main() }
}
