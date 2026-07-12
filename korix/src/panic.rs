//! Panic handler for korix.

use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kor::println!("\n!!! KORIX PANIC !!!");
    if let Some(loc) = info.location() {
        kor::println!("  at {}:{}", loc.file(), loc.line());
    }
    kor::println!("  {}", info);
    loop {
        core::hint::spin_loop();
    }
}
