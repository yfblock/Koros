//! Console output: the `Console` trait, the installed-console registry, and the
//! `print!`/`println!` macros.  Per-arch `Console` impls (under `kor-arch`)
//! call into the shared NS16550A / PL011 register drivers.

use core::fmt;
use spin::Mutex;

use spin::Once;

/// One-byte console output.  Implemented per-arch for the platform UART.
pub trait Console: Send + Sync {
    fn putc(&self, c: u8);
}

static CONSOLE: Once<&'static dyn Console> = Once::new();

/// Install the console.  Call once, before any `println!`.
pub fn install_console(c: &'static dyn Console) {
    CONSOLE.call_once(|| c);
}

/// Emit one byte to the installed console, or no-op if none installed yet.
pub fn putc(c: u8) {
    if let Some(con) = CONSOLE.get() {
        con.putc(c);
    }
}

/// Emit a byte string, translating `'\n'` to `"'\r\n'"`.
pub fn puts(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            putc(b'\r');
        }
        putc(b);
    }
}

struct ConsoleWriter;

impl fmt::Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        puts(s);
        Ok(())
    }
}

/// Serialises console output across CPUs so concurrent prints don't interleave.
static PRINT_LOCK: Mutex<()> = Mutex::new(());

pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    // Hold the lock with interrupts disabled: otherwise a task could be
    // preempted mid-print while holding the lock, and the next task's print
    // would spin forever waiting for it (single-CPU deadlock).
    crate::irq::without(|| {
        let _guard = PRINT_LOCK.lock();
        ConsoleWriter.write_fmt(args).ok();
    });
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::console::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {
        $crate::print!("{}\n", format_args!($($arg)*))
    };
}
