//! NS16550A UART register-level driver.
//!
//! The base address is supplied by the platform configuration; this module
//! only knows the register layout.

use kor::mmio::{bit_set, register_bitfields};

// Register offsets (same for MMIO and port-I/O variants).
const THR: usize = 0; // Transmit Holding Register (write)
const RBR: usize = 0; // Receive Buffer Register (read)
const LSR: usize = 5; // Line Status Register

register_bitfields!(u8, LSR [THRE 5, DR 0]);

/// Write one byte to an MMIO-mapped NS16550A with the given register
/// geometry (`reg_shift`: log2 of the register spacing; `reg_width`: 1 or 4
/// bytes — DesignWare APB UARTs use 4-byte-wide, word-spaced registers).
pub fn putchar_mmio(base: usize, reg_shift: u32, reg_width: u32, c: u8) {
    let lsr = reg_addr(base, LSR, reg_shift);
    let thr = reg_addr(base, THR, reg_shift);
    if reg_width == 4 {
        while !bit_set(lsr.read_u32() as u8, LSR::THRE) {}
        thr.write_u32(c as u32);
    } else {
        while !bit_set(lsr.read_u8(), LSR::THRE) {}
        thr.write_u8(c);
    }
}

/// Read one byte from an MMIO-mapped NS16550A with the given register
/// geometry.  Returns `None` if no data is available (non-blocking).
pub fn getchar_mmio(base: usize, reg_shift: u32, reg_width: u32) -> Option<u8> {
    let lsr = reg_addr(base, LSR, reg_shift);
    let rbr = reg_addr(base, RBR, reg_shift);
    let ready = if reg_width == 4 {
        bit_set(lsr.read_u32() as u8, LSR::DR)
    } else {
        bit_set(lsr.read_u8(), LSR::DR)
    };
    if ready {
        Some(if reg_width == 4 {
            rbr.read_u32() as u8
        } else {
            rbr.read_u8()
        })
    } else {
        None
    }
}

/// Address of register `offset` under the geometry (`base + (offset << shift)`).
fn reg_addr(base: usize, offset: usize, reg_shift: u32) -> kor::KernelAddr {
    kor::PhysAddr::new(base + (offset << reg_shift)).to_ka()
}

/// Write one byte to a port-I/O NS16550A at `base` (x86_64 legacy COM port).
#[cfg(target_arch = "x86_64")]
pub fn putchar_port(base: u16, c: u8) {
    use x86_64::instructions::port::Port;
    // SAFETY: `base` is the console UART I/O port from the platform config.
    unsafe {
        while !bit_set(Port::<u8>::new(base + LSR as u16).read(), LSR::THRE) {}
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
        if bit_set(Port::<u8>::new(base + LSR as u16).read(), LSR::DR) {
            Some(Port::<u8>::new(base + RBR as u16).read())
        } else {
            None
        }
    }
}
