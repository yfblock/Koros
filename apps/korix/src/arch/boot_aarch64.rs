//! aarch64 boot — generic MMU trampoline (see `boot.S`).

use core::arch::global_asm;

global_asm!(include_str!("boot_aarch64.S"));

#[unsafe(no_mangle)]
extern "C" fn rust_entry(_hart_id: usize, dtb: usize) {
    // The CPU that enters here is the boot CPU; secondaries (once PSCI
    // bring-up is implemented) will enter through a separate path.
    kor_arch::aarch64::mm::set_dtb_ptr(dtb);
    // `kernel_main` is this crate's own entry; it never returns.
    crate::kernel_main()
}
