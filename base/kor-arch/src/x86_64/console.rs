//! `Console` implementation for x86_64 — NS16550A over legacy port I/O.
//!
//! The COM1 console sits at port `0x3F8`; this installs a [`kor::Console`]
//! that writes through the shared NS16550A port driver.

use kor::Console;

/// NS16550A console over legacy port I/O at `base`.
pub struct Ns16550aPort {
    pub base: u16,
}

/// Singleton instance installed by the binary crate.
pub static CONSOLE: Ns16550aPort = Ns16550aPort { base: 0x3F8 };

/// Legacy port-I/O has no device tree; a no-op for interface uniformity.
pub fn console_init(_dtb: usize) {}

impl Console for Ns16550aPort {
    fn putc(&self, c: u8) {
        crate::uart::ns16550a::putchar_port(self.base, c);
    }

    fn getchar(&self) -> Option<u8> {
        crate::uart::ns16550a::getchar_port(self.base)
    }
}
