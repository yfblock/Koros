//! `Console` implementation for riscv64 — NS16550A over MMIO.

use kor::Console;

pub struct Ns16550aMmio {
    pub base: usize,
}

pub static CONSOLE: Ns16550aMmio = Ns16550aMmio { base: 0x1000_0000 };

impl Console for Ns16550aMmio {
    fn putc(&self, c: u8) {
        crate::uart::ns16550a::putchar_mmio(self.base, c);
    }
}
