//! `Console` implementation for aarch64 — ARM PL011 over MMIO.
//!
//! The QEMU `virt` console UART sits at `0x0900_0000`; this installs a
//! [`kor::Console`] that writes through the shared PL011 driver.

use kor::Console;

/// ARM PL011 console over MMIO at `base`.
pub struct Pl011Mmio {
    pub base: usize,
}

/// Singleton instance installed by the binary crate.
pub static CONSOLE: Pl011Mmio = Pl011Mmio { base: 0x0900_0000 };

impl Console for Pl011Mmio {
    fn putc(&self, c: u8) {
        crate::uart::pl011::putchar(self.base, c);
    }
}
