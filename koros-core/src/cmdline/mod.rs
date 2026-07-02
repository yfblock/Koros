//! Kernel command-line storage and parsing.
//!
//! The raw command line is obtained from the bootloader in an
//! architecture-specific way (FDT `/chosen/bootargs` on riscv64/aarch64/
//! loongarch64, the Multiboot cmdline on x86_64) through
//! [`crate::mm::boot_cmdline`], captured once at boot, and then queried as
//! whitespace-separated `key=value` pairs and bare flags.

use alloc::string::String;
use spin::Once;

/// The command line, captured once during boot.
static CMDLINE: Once<String> = Once::new();

/// Capture the boot command line.  Call once, after `mm::init()`.
pub fn init() {
    CMDLINE.call_once(|| crate::mm::boot_cmdline().unwrap_or_default());
}

/// The full command-line string (empty if none was provided).
pub fn raw() -> &'static str {
    CMDLINE.get().map(String::as_str).unwrap_or("")
}

/// Iterate over whitespace-separated command-line arguments.
pub fn args() -> impl Iterator<Item = &'static str> {
    raw().split_whitespace()
}

/// Return the value of the first `key=value` argument matching `key`.
///
/// # Examples
///
/// For a command line `"console=ttyS0 root=/dev/vda ro"`, `get("root")`
/// returns `Some("/dev/vda")`.
pub fn get(key: &str) -> Option<&'static str> {
    args().find_map(|arg| {
        let (k, v) = arg.split_once('=')?;
        (k == key).then_some(v)
    })
}

/// Return `true` if the bare flag `flag` (an argument with no `=`) is present.
pub fn has_flag(flag: &str) -> bool {
    args().any(|arg| arg == flag)
}
