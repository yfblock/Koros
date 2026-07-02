//! Architecture-specific modules.
//!
//! Each subdirectory provides the boot entry point and platform
//! initialisation for one target architecture.

cfg_if::cfg_if! {
    if #[cfg(target_arch = "riscv64")]      { pub mod riscv64; }
    else if #[cfg(target_arch = "x86_64")]  { pub mod x86_64; }
    else if #[cfg(target_arch = "aarch64")] { pub mod aarch64; }
    else if #[cfg(target_arch = "loongarch64")] { pub mod loongarch64; }
    else { compile_error!("unsupported target_arch"); }
}
