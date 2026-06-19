#![no_std]
#![no_main]
// The assembly uses explicit .intel_syntax / .att_syntax directives because
// .altmacro conflicts with AT&T %-prefix, and multiboot.S transitions between
// .code32 and .code64 sections with different syntax requirements.
#![allow(bad_asm_style)]
mod arch;
mod boot;
mod drivers;
mod mm;
mod trap;

use core::panic::PanicInfo;

#[unsafe(no_mangle)]
fn kernel_main() -> ! {
    mm::init();
    trap::init();
    println!("Hello, world!");

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("");
    println!("!!! KERNEL PANIC !!!");
    println!("  {}", info.message());
    if let Some(loc) = info.location() {
        println!("  at {}:{}", loc.file(), loc.line());
    }
    loop {
        core::hint::spin_loop();
    }
}
