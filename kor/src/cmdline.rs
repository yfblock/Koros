//! Kernel command-line storage and parsing.
//!
//! The raw command line is supplied by the composition layer (which obtains it
//! from the `ArchProvider`); it is captured once via [`init_from`] and then
//! queried as whitespace-separated `key=value` pairs and bare flags.

use alloc::string::String;
use spin::Once;

/// The command line, captured once during boot.
static CMDLINE: Once<String> = Once::new();

/// Capture the boot command line.  Call once, early in `kernel_main`.
pub fn init_from(cmdline: String) {
    CMDLINE.call_once(|| cmdline);
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
