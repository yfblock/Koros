//! NS16550A port-I/O console at 0x3F8 (COM1), implementing kor::Console.

use core::arch::asm;
use kor::Console;

const PORT: u16 = 0x3F8;

/// The singleton console installed by kernel_main.
pub struct Ns16550a;

pub static CONSOLE: Ns16550a = Ns16550a;

impl Console for Ns16550a {
    fn putc(&self, c: u8) {
        // SAFETY: outb to the legacy COM1 port is side-effect-only I/O.
        unsafe {
            asm!("out dx, al", in("al") c, in("dx") PORT);
        }
    }
}
