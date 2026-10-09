//! `Console` implementation for riscv64 — NS16550a-family UART over MMIO,
//! with base *and register geometry* discovered from the device tree
//! (`console_init`); falls back to the QEMU-`virt` defaults.

use core::sync::atomic::{AtomicUsize, Ordering};

use kor::Console;

/// QEMU `virt` fallback when the FDT lookup fails or no DTB was provided
/// (byte-wide, byte-spaced registers).
/// QEMU `virt` fallback (byte-wide, byte-spaced).
const DEFAULT_INFO: kor::fdt::UartInfo = kor::fdt::UartInfo {
    base: 0x1000_0000,
    reg_shift: 0,
    reg_io_width: 1,
};

/// NS16550a-family MMIO console; patched by [`console_init`].
pub struct Ns16550aMmio {
    pub base: AtomicUsize,
    pub reg_shift: AtomicUsize,
    pub reg_io_width: AtomicUsize,
}

pub static CONSOLE: Ns16550aMmio = Ns16550aMmio {
    base: AtomicUsize::new(DEFAULT_INFO.base),
    reg_shift: AtomicUsize::new(DEFAULT_INFO.reg_shift as usize),
    reg_io_width: AtomicUsize::new(DEFAULT_INFO.reg_io_width as usize),
};

/// Discover the UART (base, `reg-shift`, `reg-io-width`) from the FDT,
/// falling back to the QEMU-`virt` default.  Call before
/// `install_console`.
pub fn console_init(dtb: usize) {
    let info = if dtb != 0 {
        // SAFETY: `dtb` is the firmware-provided FDT blob pointer.
        unsafe { kor::fdt::find_uart(dtb) }.unwrap_or(DEFAULT_INFO)
    } else {
        DEFAULT_INFO
    };
    CONSOLE.base.store(info.base, Ordering::Relaxed);
    CONSOLE.reg_shift.store(info.reg_shift as usize, Ordering::Relaxed);
    CONSOLE.reg_io_width.store(info.reg_io_width as usize, Ordering::Relaxed);
}

impl Ns16550aMmio {
    fn geometry(&self) -> (usize, u32, u32) {
        (
            self.base.load(Ordering::Relaxed),
            self.reg_shift.load(Ordering::Relaxed) as u32,
            self.reg_io_width.load(Ordering::Relaxed) as u32,
        )
    }
}

impl Console for Ns16550aMmio {
    fn putc(&self, c: u8) {
        let (base, shift, width) = self.geometry();
        crate::uart::ns16550a::putchar_mmio(base, shift, width, c);
    }

    fn getchar(&self) -> Option<u8> {
        let (base, shift, width) = self.geometry();
        crate::uart::ns16550a::getchar_mmio(base, shift, width)
    }
}
