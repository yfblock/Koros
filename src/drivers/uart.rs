//! Minimal UART driver for QEMU.
//!
//! NS16550A (riscv64, x86_64, loongarch64) and PL011 (aarch64) supported.

use core::fmt;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "aarch64")] {
        // PL011 UART at 0x09000000 on QEMU virt machine.
        const UART_BASE: usize = 0x0900_0000;
        const UART_DR: usize = 0x00;   // Data Register
        const UART_FR: usize = 0x18;   // Flag Register
        const FR_TXFF: u8 = 1 << 5;    // TX FIFO Full
    } else {
        // NS16550A UART.
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "riscv64")] {
                const UART_BASE: usize = 0x1000_0000;
            } else if #[cfg(target_arch = "x86_64")] {
                const UART_BASE: u16 = 0x3F8;
            } else if #[cfg(target_arch = "loongarch64")] {
                const UART_BASE: usize = 0x1FE0_01E0;
            }
        }
        const THR_OFFSET: usize = 0x00;
        const LSR_OFFSET: usize = 0x05;
        const LSR_THRE: u8 = 1 << 5;   // THR Empty
    }
}

fn putchar(c: u8) {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "aarch64")] {
            let base = UART_BASE as *mut u8;
            unsafe {
                while base.add(UART_FR).read_volatile() & FR_TXFF != 0 {}
                base.add(UART_DR).write_volatile(c);
            }
        } else if #[cfg(target_arch = "x86_64")] {
            while unsafe { x86_64::instructions::port::Port::<u8>::new(UART_BASE + LSR_OFFSET as u16).read() } & LSR_THRE == 0 {}
            unsafe { x86_64::instructions::port::Port::<u8>::new(UART_BASE + THR_OFFSET as u16).write(c); }
        } else {
            let base = UART_BASE as *mut u8;
            unsafe {
                while base.add(LSR_OFFSET).read_volatile() & LSR_THRE == 0 {}
                base.add(THR_OFFSET).write_volatile(c);
            }
        }
    }
}

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
