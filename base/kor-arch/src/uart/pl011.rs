//! ARM PL011 UART register-level driver.
//!
//! The base address is supplied by the platform configuration.

use kor::register_bitfields;

const UART_DR: usize = 0x00; // Data Register
const UART_FR: usize = 0x18; // Flag Register

register_bitfields!(u8, FR [TXFF 5, RXFE 4]);

/// Write one byte to an MMIO-mapped PL011 at `base`.
pub fn putchar(base: usize, c: u8) {
    let fr = kor::PhysAddr::new(base + UART_FR).to_ka().read_u8();
    while FR::TXFF.is_set(fr) {}
    kor::PhysAddr::new(base + UART_DR).to_ka().write_u8(c);
}

/// Read one byte from an MMIO-mapped PL011 at `base`.
/// Returns `None` if no data is available (non-blocking).
pub fn getchar(base: usize) -> Option<u8> {
    let fr = kor::PhysAddr::new(base + UART_FR).to_ka().read_u8();
    if !FR::RXFE.is_set(fr) {
        Some(kor::PhysAddr::new(base + UART_DR).to_ka().read_u8())
    } else {
        None
    }
}
