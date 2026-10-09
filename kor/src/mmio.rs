//! Memory-mapped device-register access.
//!
//! [`Reg`] wraps one device register and exposes safe volatile reads and
//! writes.  The address-validity contract is concentrated at construction:
//! a `Reg` must only be built from a platform device address whose register
//! file really contains a `T` at that offset (the per-arch driver constants
//! satisfy this by construction).  Addresses translate through the
//! application-installed [`crate::addr`] pair, so the same driver works
//! identity-mapped and through a high-half window.

use core::marker::PhantomData;

use crate::addr::PhysAddr;

/// One memory-mapped device register of width `T`.
pub struct Reg<T: Copy> {
    addr: usize,
    _width: PhantomData<T>,
}

impl<T: Copy> Reg<T> {
    /// Register at a physical device address (`pa` must point at a real
    /// `T`-wide register of a mapped device).
    pub fn at_pa(pa: usize) -> Self {
        Self {
            addr: PhysAddr::new(pa).to_va().raw(),
            _width: PhantomData,
        }
    }

    /// Volatile read of the register.
    pub fn read(&self) -> T {
        // SAFETY: constructed from a valid device register address.
        unsafe { (self.addr as *const T).read_volatile() }
    }

    /// Volatile write of the register.
    pub fn write(&self, value: T) {
        // SAFETY: constructed from a valid device register address.
        unsafe { (self.addr as *mut T).write_volatile(value) }
    }

    /// Register at a fixed byte offset from this one.
    pub fn at_offset(&self, off: usize) -> Self {
        Self {
            addr: self.addr + off,
            _width: PhantomData,
        }
    }
}
