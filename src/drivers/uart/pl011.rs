//! PL011 UART driver.
//!
//! Used by aarch64 (MMIO 0x0900_0000).

const UART_BASE: usize = 0x0900_0000;
const UART_DR: usize = 0x00;
const UART_FR: usize = 0x18;
const FR_TXFF: u8 = 1 << 5;

pub fn putchar(c: u8) {
    let base = UART_BASE as *mut u8;
    unsafe {
        while base.add(UART_FR).read_volatile() & FR_TXFF != 0 {}
        base.add(UART_DR).write_volatile(c);
    }
}
