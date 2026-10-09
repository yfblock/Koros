//! `Console` implementation for aarch64 — ARM PL011 MMIO, with the base
//! discovered from the device tree (`console_init`) and a QEMU-`virt`
//! fallback.

use core::sync::atomic::{AtomicUsize, Ordering};

use kor::Console;

/// QEMU `virt` fallback when the FDT lookup fails or no DTB was provided.
const DEFAULT_BASE: usize = 0x0900_0000;

/// PL011 MMIO console; the base is patched by [`console_init`].
pub struct Pl011Mmio {
    pub base: AtomicUsize,
}

pub static CONSOLE: Pl011Mmio = Pl011Mmio {
    base: AtomicUsize::new(DEFAULT_BASE),
};

/// Discover the UART base from the FDT (compatible `arm,pl011`), falling
/// back to the QEMU-`virt` default.  Call before `install_console`.
pub fn console_init(dtb: usize) {
    let base = if dtb != 0 {
        // SAFETY: `dtb` is the firmware-provided FDT blob pointer.
        unsafe { kor::fdt::find_compatible_reg(dtb, &["arm,pl011"], 0) }.unwrap_or(DEFAULT_BASE)
    } else {
        DEFAULT_BASE
    };
    CONSOLE.base.store(base, Ordering::Relaxed);
}

impl Console for Pl011Mmio {
    fn putc(&self, c: u8) {
        crate::uart::pl011::putchar(self.base.load(Ordering::Relaxed), c);
    }

    fn getchar(&self) -> Option<u8> {
        crate::uart::pl011::getchar(self.base.load(Ordering::Relaxed))
    }
}
