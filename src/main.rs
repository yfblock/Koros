#![no_std]
#![no_main]
mod arch;
mod boot;
mod drivers;

use core::panic::PanicInfo;

#[unsafe(no_mangle)]
fn kernel_main() -> ! {
    println!("Hello, world!");
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
