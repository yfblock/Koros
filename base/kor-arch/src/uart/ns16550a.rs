//! NS16550A UART register-level driver.
//!
//! The base address is supplied by the platform configuration; this module
//! only knows the register layout.

use kor::mmio::{self, Readable, Writeable, register_bitfields};

// Register offsets (same for MMIO and port-I/O variants).
const THR: usize = 0; // Transmit Holding Register (write)
const RBR: usize = 0; // Receive Buffer Register (read)
const LSR: usize = 5; // Line Status Register

register_bitfields!(u8, LSR [THRE 5, DR 0]);

/// Write one byte to an MMIO-mapped NS16550A at `base`.
pub fn putchar_mmio(base: usize, c: u8) {
    while !mmio::rof::<u8, LSR::Register>(base + LSR).is_set(LSR::THRE) {}
    mmio::wo::<u8>(base + THR).set(c);
}

/// Read one byte from an MMIO-mapped NS16550A at `base`.
/// Returns `None` if no data is available (non-blocking).
pub fn getchar_mmio(base: usize) -> Option<u8> {
    if mmio::rof::<u8, LSR::Register>(base + LSR).is_set(LSR::DR) {
        Some(mmio::ro::<u8>(base + RBR).get())
    } else {
        None
    }
}

/// Write one byte to a port-I/O NS16550A at `base` (x86_64 legacy COM port).
#[cfg(target_arch = "x86_64")]
pub fn putchar_port(base: u16, c: u8) {
    use x86_64::instructions::port::Port;
    // SAFETY: `base` is the console UART I/O port from the platform config.
    unsafe {
        while Port::<u8>::new(base + LSR as u16).read() & LSR::THRE.mask == 0 {}
        Port::<u8>::new(base + THR as u16).write(c);
    }
}

/// Read one byte from a port-I/O NS16550A at `base` (x86_64 legacy COM port).
/// Returns `None` if no data is available (non-blocking).
#[cfg(target_arch = "x86_64")]
pub fn getchar_port(base: u16) -> Option<u8> {
    use x86_64::instructions::port::Port;
    // SAFETY: `base` is the console UART I/O port from the platform config.
    unsafe {
        if Port::<u8>::new(base + LSR as u16).read() & LSR::DR.mask != 0 {
            Some(Port::<u8>::new(base + RBR as u16).read())
        } else {
            None
        }
    }
}
