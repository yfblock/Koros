//! aarch64 arch layer for korhv: an EL2 boot into the hypervisor (identity
//! mapped, no high-half -- non-VHE EL2 has a single translation regime), a
//! PL011 console, a minimal ArchProvider, and the EL2 hypervisor (hyp).

pub mod boot;
pub mod console;
pub mod hyp;
pub mod mm;
pub mod provider;
