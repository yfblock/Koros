//! Per-architecture MMU back-ends.
//!
//! Each paging architecture owns one module here: its PTE flags and
//! encoders, the [`crate::format::MmuFormat`] implementation for the
//! generic engine, the active-address-space operations (`root`/`activate`),
//! fork-style cloning (`clone_active`/`new_user`/`destroy_address_space`),
//! and boot-time identity-map surgery (`split_identity`).  loongarch64 runs
//! unpaged and provides stubs only.
//!
//! This is the one `cfg`-selected module inside `kor-mmu` (the same narrow
//! exception `kor-arch` uses); everything above it is arch-neutral.

cfg_if::cfg_if! {
    if #[cfg(target_arch = "riscv64")]      { pub mod riscv64; pub use riscv64 as current; }
    else if #[cfg(target_arch = "x86_64")]  { pub mod x86_64; pub use x86_64 as current; }
    else if #[cfg(target_arch = "aarch64")] { pub mod aarch64; pub use aarch64 as current; }
    else if #[cfg(target_arch = "loongarch64")] { pub mod loongarch64; pub use loongarch64 as current; }
    else { compile_error!("kor-mmu: unsupported target_arch"); }
}
