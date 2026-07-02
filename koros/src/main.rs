#![no_std]
#![no_main]

use koros_core::{cmdline, mm, trap};

/// Kernel entry point, called by the architecture boot code
/// (`koros_core::arch::<arch>::boot::rust_entry`) after early setup.
#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    trap::init();
    mm::init(); // captures the boot command line early (see mm::init)
    koros_core::println!("Hello, world!");
    koros_core::println!("cmdline: {:?}", cmdline::raw());

    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64"))]
    koros_core::ext2_test();

    loop {
        core::hint::spin_loop();
    }
}
