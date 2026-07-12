//! PL011 MMIO console at 0x0900_0000 (QEMU virt), implementing kor::Console.

use core::arch::asm;
use kor::Console;

const BASE: usize = 0x0900_0000;

/// The singleton console installed by kernel_main.
pub struct Pl011;

pub static CONSOLE: Pl011 = Pl011;

impl Console for Pl011 {
    fn putc(&self, c: u8) {
        // SAFETY: write to the PL011 data register (offset 0); poll the flag
        // register (offset 0x18, bit 5 = TXFF) so bytes are not dropped.
        const UARTFR: usize = BASE + 0x18;
        unsafe {
            loop {
                let fr: u32;
                asm!("ldr {}, [{}]", out(reg) fr, in(reg) UARTFR);
                if fr & (1 << 5) == 0 {
                    break;
                }
            }
            let dr: usize = BASE;
            let c32: u32 = c as u32;
            asm!("str {}, [{}]", in(reg) c32, in(reg) dr);
        }
    }
}
