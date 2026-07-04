//! Console UART output.
//!
//! The console device (NS16550A / PL011, MMIO or port I/O, and its base
//! address) is selected at runtime from the board's [`PlatformConfig`]
//! ([`crate::platform`]), so no addresses are hardcoded here.  Register-level
//! drivers live in `ns16550a.rs` / `pl011.rs`.

use core::fmt;

use crate::platform::{self, Console};

mod ns16550a;
#[cfg(target_arch = "aarch64")]
mod pl011;

/// Emit one byte to the configured console.  A no-op until the platform
/// configuration is installed (see [`crate::platform::init`]).
fn putchar(c: u8) {
    match platform::console() {
        Some(Console::Ns16550aMmio { base }) => ns16550a::putchar_mmio(base, c),
        #[cfg(target_arch = "x86_64")]
        Some(Console::Ns16550aPort { base }) => ns16550a::putchar_port(base, c),
        #[cfg(not(target_arch = "x86_64"))]
        Some(Console::Ns16550aPort { .. }) => {}
        #[cfg(target_arch = "aarch64")]
        Some(Console::Pl011Mmio { base }) => pl011::putchar(base, c),
        #[cfg(not(target_arch = "aarch64"))]
        Some(Console::Pl011Mmio { .. }) => {}
        None => {}
    }
}

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

/// Serialises console output across CPUs so concurrent prints don't interleave.
static PRINT_LOCK: spin::Mutex<()> = spin::Mutex::new(());

pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    // Hold the lock with interrupts disabled: otherwise a task could be
    // preempted mid-print while holding the lock, and the next task's print
    // would spin forever waiting for it (single-CPU deadlock).
    crate::irq::without(|| {
        let _guard = PRINT_LOCK.lock();
        UartWriter.write_fmt(args).ok();
    });
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
