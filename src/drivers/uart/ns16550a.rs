//! NS16550A UART driver.
//!
//! Used by riscv64 (MMIO 0x1000_0000), x86_64 (port I/O 0x3F8), and
//! loongarch64 (MMIO 0x1FE0_01E0).

// Register offsets (same for MMIO and port I/O variants).
const THR: u16 = 0;  // Transmit Holding Register
const LSR: u16 = 5;  // Line Status Register
const LSR_THRE: u8 = 1 << 5;  // Transmit Hold Register Empty

pub fn putchar(c: u8) {
    // x86_64: port I/O
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use x86_64::instructions::port::Port;
        while Port::<u8>::new(0x3F8 + LSR).read() & LSR_THRE == 0 {}
        Port::<u8>::new(0x3F8 + THR).write(c);
    }

    // riscv64 / loongarch64: MMIO
    #[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
    {
        let base: usize = if cfg!(target_arch = "riscv64") {
            0x1000_0000
        } else {
            0x1FE0_01E0
        };
        let p = base as *mut u8;
        unsafe {
            while p.add(LSR as usize).read_volatile() & LSR_THRE == 0 {}
            p.add(THR as usize).write_volatile(c);
        }
    }
}
