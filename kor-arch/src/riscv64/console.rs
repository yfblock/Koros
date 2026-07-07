//! `Console` implementation for riscv64 — NS16550A over MMIO.
//!
//! The QEMU `virt` console UART sits at `0x1000_0000`; this installs a
//! [`kor::Console`] that writes through the shared NS16550A driver.

use kor::Console;

/// NS16550A console over MMIO at `base`.
pub struct Ns16550aMmio {
    pub base: usize,
}

/// Singleton instance installed by the binary crate.
pub static CONSOLE: Ns16550aMmio = Ns16550aMmio { base: 0x1000_0000 };

impl Console for Ns16550aMmio {
    fn putc(&self, c: u8) {
        crate::uart::ns16550a::putchar_mmio(self.base, c);
    }
}
