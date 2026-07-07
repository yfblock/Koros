//! ARM PL011 UART register-level driver.
//!
//! The base address is supplied by the platform configuration.

const UART_DR: usize = 0x00; // Data Register
const UART_FR: usize = 0x18; // Flag Register
const FR_TXFF: u8 = 1 << 5; // Transmit FIFO full

/// Write one byte to an MMIO-mapped PL011 at `base`.
pub fn putchar(base: usize, c: u8) {
    let p = base as *mut u8;
    // SAFETY: `base` is the console UART MMIO region from the platform config.
    unsafe {
        while p.add(UART_FR).read_volatile() & FR_TXFF != 0 {}
        p.add(UART_DR).write_volatile(c);
    }
}
