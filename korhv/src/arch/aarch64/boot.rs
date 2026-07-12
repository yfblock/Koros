//! aarch64 EL2 boot: identity-map the kernel + RAM at EL2 (TTBR0_EL2), enable
//! the EL2 MMU, and call the hypervisor kernel_main.  Non-VHE EL2 has a single
//! translation regime, so there is no high-half (VA == PA).

use core::arch::global_asm;

global_asm!(include_str!("boot.S"));

#[unsafe(no_mangle)]
extern "C" fn rust_entry(_hart_id: usize, dtb: usize) {
    super::mm::set_dtb_ptr(dtb);
    // kernel_main is provided by the korhv binary crate and never returns.
    crate::kernel_main()
}
