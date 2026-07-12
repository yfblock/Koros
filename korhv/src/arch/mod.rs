//! Architecture selector.  korhv supports x86_64 (AMD SVM) and aarch64 (EL2);
//! the other targets are rejected at compile time.  Each arch module exports
//! `boot`, `console`, `mm`, `provider` and `hyp` with a uniform surface, so the
//! shared `main.rs` calls `crate::arch::hyp::init/run` without cfg branches.

#[cfg(target_arch = "x86_64")]
pub mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use x86_64::*;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;
#[cfg(target_arch = "aarch64")]
pub use aarch64::*;

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("korhv supports only x86_64 and aarch64");
