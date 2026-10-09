//! ARM PL011 UART register-level driver.
//!
//! The base address is supplied by the platform configuration.

const UART_DR: usize = 0x00; // Data Register
const UART_FR: usize = 0x18; // Flag Register
const FR_TXFF: u8 = 1 << 5; // Transmit FIFO full
const FR_RXFE: u8 = 1 << 4; // Receive FIFO empty

/// Write one byte to an MMIO-mapped PL011 at `base`.
pub fn putchar(base: usize, c: u8) {
    let p = crate::phys_to_virt(base) as *mut u8;
    unsafe {
        while p.add(UART_FR).read_volatile() & FR_TXFF != 0 {}
        p.add(UART_DR).write_volatile(c);
    }
}

/// Read one byte from an MMIO-mapped PL011 at `base`.
/// Returns `None` if no data is available (non-blocking).
pub fn getchar(base: usize) -> Option<u8> {
    let p = crate::phys_to_virt(base) as *mut u8;
    unsafe {
        if p.add(UART_FR).read_volatile() & FR_RXFE == 0 {
            Some(p.add(UART_DR).read_volatile())
        } else {
            None
        }
    }
}
