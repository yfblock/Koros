//! Minimal UART driver for QEMU.
//!
//! Architecture-specific `putchar` implementations live in driver-named sibling
//! modules (`ns16550a.rs`, `pl011.rs`).  The `mod` declarations below select
//! the correct driver for each architecture; no `#[cfg]` is needed in the
//! generic helpers.

use core::fmt;

// ---------------------------------------------------------------------------
// Driver selection by architecture
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "riscv64", target_arch = "x86_64", target_arch = "loongarch64"))]
mod ns16550a;
#[cfg(target_arch = "aarch64")]
mod pl011;

// Re-export the selected driver's putchar.
#[cfg(any(target_arch = "riscv64", target_arch = "x86_64", target_arch = "loongarch64"))]
use ns16550a::putchar;
#[cfg(target_arch = "aarch64")]
use pl011::putchar;

// ---------------------------------------------------------------------------
// Generic helpers (architecture-neutral)
// ---------------------------------------------------------------------------

pub fn puts(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            putchar(b'\r');
        }
        putchar(b);
    }
}

struct UartWriter;

impl fmt::Write for UartWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        puts(s);
        Ok(())
    }
}

pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    UartWriter.write_fmt(args).ok();
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::drivers::uart::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {
        $crate::print!("{}\n", format_args!($($arg)*))
    };
}
