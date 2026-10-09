//! ARM PL011 UART register-level driver.
//!
//! The base address is supplied by the platform configuration.

use kor::mmio::{self, Readable, Writeable, register_bitfields};

const UART_DR: usize = 0x00; // Data Register
const UART_FR: usize = 0x18; // Flag Register

register_bitfields!(u8, FR [TXFF 5, RXFE 4]);

/// Write one byte to an MMIO-mapped PL011 at `base`.
pub fn putchar(base: usize, c: u8) {
    while mmio::rof::<u8, FR::Register>(base + UART_FR).is_set(FR::TXFF) {}
    mmio::wo::<u8>(base + UART_DR).set(c);
}

/// Read one byte from an MMIO-mapped PL011 at `base`.
/// Returns `None` if no data is available (non-blocking).
pub fn getchar(base: usize) -> Option<u8> {
    if !mmio::rof::<u8, FR::Register>(base + UART_FR).is_set(FR::RXFE) {
        Some(mmio::ro::<u8>(base + UART_DR).get())
    } else {
        None
    }
}
