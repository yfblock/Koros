//! Panic handler.

use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kor::println!("");
    kor::println!("!!! HV PANIC !!!");
    kor::println!("  {}", info.message());
    if let Some(loc) = info.location() {
        kor::println!("  at {}:{}", loc.file(), loc.line());
    }
    loop {
        core::hint::spin_loop();
    }
}
