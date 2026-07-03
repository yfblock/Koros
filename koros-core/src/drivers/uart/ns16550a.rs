//! NS16550A UART register-level driver.
//!
//! The base address is supplied by the platform configuration; this module
//! only knows the register layout.

// Register offsets (same for MMIO and port-I/O variants).
const THR: usize = 0; // Transmit Holding Register
const LSR: usize = 5; // Line Status Register
const LSR_THRE: u8 = 1 << 5; // Transmit Hold Register Empty

/// Write one byte to an MMIO-mapped NS16550A at `base`.
pub fn putchar_mmio(base: usize, c: u8) {
    let p = base as *mut u8;
    // SAFETY: `base` is the console UART MMIO region from the platform config.
    unsafe {
        while p.add(LSR).read_volatile() & LSR_THRE == 0 {}
        p.add(THR).write_volatile(c);
    }
}

/// Write one byte to a port-I/O NS16550A at `base` (x86_64 legacy COM port).
#[cfg(target_arch = "x86_64")]
pub fn putchar_port(base: u16, c: u8) {
    use x86_64::instructions::port::Port;
    // SAFETY: `base` is the console UART I/O port from the platform config.
    unsafe {
        while Port::<u8>::new(base + LSR as u16).read() & LSR_THRE == 0 {}
        Port::<u8>::new(base + THR as u16).write(c);
    }
}
