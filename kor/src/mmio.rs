//! Memory-mapped device-register access, built on `tock-registers`.
//!
//! The tock register cells ([`ReadOnly`]/[`WriteOnly`]/[`ReadWrite`]) are
//! placed over device memory by pointer cast; this module is where the
//! address-validity contract concentrates:
//!
//! * [`ro`]/[`wo`]/[`rw`] take a *physical* device address, translate it
//!   through the application-installed [`crate::addr`] pair (identity until
//!   installed), and cast — the caller must pass the address of a real
//!   register of width `T` on a mapped device (the per-arch driver constants
//!   satisfy this by construction).
//!
//! Named bit-fields come from [`register_bitfields!`]
//! (`reg.read(CTRL::ENABLE)`, `reg.modify(CTRL::MODE::PLL)`, …), replacing
//! raw mask arithmetic at the call sites.

use tock_registers::RegisterLongName;
use tock_registers::UIntLike;
use tock_registers::registers::{ReadOnly, ReadWrite, WriteOnly};

use crate::addr::PhysAddr;

/// Re-exported register types, access traits, and the bit-field macro so
/// drivers never depend on the tock crate directly.
pub use tock_registers::{
    interfaces::{Readable, Writeable},
    register_bitfields, registers,
};

/// Read-only register cell at physical device address `pa`.
pub fn ro<T: UIntLike>(pa: usize) -> &'static ReadOnly<T> {
    // SAFETY: `pa` names a real read-only device register of width `T`.
    unsafe { &*(PhysAddr::new(pa).to_va().raw() as *const ReadOnly<T>) }
}

/// Read-only register cell whose named bit-fields ([`register_bitfields!`])
/// are accessible via `read(field)`/`is_set(field)`.
pub fn rof<T: UIntLike, R: RegisterLongName>(pa: usize) -> &'static ReadOnly<T, R> {
    // SAFETY: `pa` names a real read-only device register of width `T`.
    unsafe { &*(PhysAddr::new(pa).to_va().raw() as *const ReadOnly<T, R>) }
}

/// Write-only register cell at physical device address `pa`.
pub fn wo<T: UIntLike>(pa: usize) -> &'static WriteOnly<T> {
    // SAFETY: `pa` names a real write-only device register of width `T`.
    unsafe { &*(PhysAddr::new(pa).to_va().raw() as *const WriteOnly<T>) }
}

/// Read-write register cell at physical device address `pa`.
pub fn rw<T: UIntLike>(pa: usize) -> &'static ReadWrite<T> {
    // SAFETY: `pa` names a real read-write device register of width `T`.
    unsafe { &*(PhysAddr::new(pa).to_va().raw() as *const ReadWrite<T>) }
}

/// Read-write cell with named bit-fields (`read`/`modify`/`is_set`).
pub fn rwf<T: UIntLike, R: RegisterLongName>(pa: usize) -> &'static ReadWrite<T, R> {
    // SAFETY: `pa` names a real read-write device register of width `T`.
    unsafe { &*(PhysAddr::new(pa).to_va().raw() as *const ReadWrite<T, R>) }
}
