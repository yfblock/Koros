//! Trap and interrupt handling.
//!
//! Architecture-specific implementations are in `src/arch/<arch>/trap.rs`.
//! Each arch provides a `pub fn init()` that installs trap vectors.

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::trap;

#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::trap;

#[cfg(target_arch = "aarch64")]
use crate::arch::aarch64::trap;

#[cfg(target_arch = "loongarch64")]
use crate::arch::loongarch64::trap;

/// Install trap handlers for the current architecture.
pub fn init() {
    trap::init();
}
