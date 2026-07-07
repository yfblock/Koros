//! Register-level UART drivers (NS16550A, PL011) used by the per-arch
//! `Console` implementations.

pub mod ns16550a;
#[cfg(target_arch = "aarch64")]
pub mod pl011;
