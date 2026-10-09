//! Memory-mapped device-register bit-field definitions.
//!
//! Register *access* goes through the address newtypes
//! (`PhysAddr::read::<T>()` / `write::<T>()` — see [`crate::addr`],
//! translated through the installed application pair); this module only
//! re-exports [`register_bitfields!`] so drivers can declare named fields
//! (e.g. `LSR [THRE 5, DR 0]`) and use `FIELD.mask` / field values instead
//! of hand-written mask constants.

pub use tock_registers::RegisterLongName;
pub use tock_registers::fields::Field;
pub use tock_registers::register_bitfields;

/// Test whether a named single-bit field is set in `value`.
///
/// A tock [`Field`] carries its width mask *unshifted* (`THRE 5` has
/// `mask == 1`, `shift == 5`), so raw `value & field.mask` tests the wrong
/// bit — always go through this helper.
pub fn bit_set<T, R: RegisterLongName>(value: T, field: Field<T, R>) -> bool
where
    T: tock_registers::UIntLike,
{
    (value & (field.mask << field.shift)) != T::zero()
}
