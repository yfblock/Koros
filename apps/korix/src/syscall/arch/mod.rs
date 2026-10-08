//! Per-ABI syscall adapters — the only architecture-specific part of the
//! syscall layer.
//!
//! The portable layer ([`crate::syscall`]) speaks one canonical numbering:
//! the Linux `asm-generic` table, native to riscv64 and aarch64. An adapter
//! translates an incoming `(nr, args)` from its own ABI into that canonical
//! form: renumbering, argument-vector reshuffling, and native-only calls
//! (routed through the internal pseudo-number block at 500+).
//!
//! Porting checklist for a new architecture:
//!   1. add `arch/<name>.rs` implementing `translate` (+ `native_syscall` if
//!      the ABI has calls with no generic equivalent);
//!   2. select it as `current` below;
//!   3. kor-arch's trap glue extracts `(nr, args[0..6])` from the frame,
//!      calls the handler installed via `kor::install_syscall_handler`
//!      (`syscall::dispatch`), writes the return value back, and applies the
//!      PC-advance rule — nothing else changes.

#[cfg(target_arch = "aarch64")]
pub mod aarch64;
#[cfg(target_arch = "riscv64")]
pub mod riscv64;
#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "aarch64")]
pub use aarch64 as current;
#[cfg(target_arch = "riscv64")]
pub use riscv64 as current;
#[cfg(target_arch = "x86_64")]
pub use x86_64 as current;
