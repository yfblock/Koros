//! Memory-mapped device-register bit-field definitions.
//!
//! Register access goes through the address newtypes
//! (`PhysAddr::to_ka().read_u32()` — see [`crate::addr`]); this module
//! re-exports [`register_bitfields!`] so drivers can declare named fields
//! (e.g. `LSR [THRE 5, DR 0]`) and use the tock-registers native
//! `FIELD.is_set(value)` / `field.read(value)` helpers instead of
//! hand-written mask constants.

pub use tock_registers::RegisterLongName;
pub use tock_registers::fields::Field;
pub use tock_registers::register_bitfields;
