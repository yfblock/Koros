//! Per-process user address spaces.

extern crate alloc;

use kor::{MapError, MapSize, MappingFlags};

const PAGE_SIZE: usize = 4096;

/// An exclusively owned user page-table root.
///
/// `AddressSpace` is deliberately neither `Clone` nor reference counted: one
/// live process owns one root, and destroying that process destroys that root.
pub struct AddressSpace {
    root: usize,
    owned: bool,
}

impl AddressSpace {
    /// Borrow the boot root retained by every CPU idle task. It remains
    /// kernel-owned for the machine lifetime and is never process-destroyed.
    pub fn adopt_active() -> Self {
        Self { root: kor::arch::current().address_space_root(), owned: false }
    }

    /// Create a root containing kernel mappings but no user mappings.
    pub fn empty() -> Result<Self, MapError> {
        Ok(Self { root: kor::arch::current().new_address_space()?, owned: true })
    }

    /// Deep-copy this address space for `fork`.
    pub fn fork(&self) -> Result<Self, MapError> {
        self.with_active(|| {
            kor::arch::current()
                .clone_address_space()
                .map(|root| Self { root, owned: true })
        })
    }

    pub(crate) fn root(&self) -> usize {
        self.root
    }

    /// Activate this process's address space on the current CPU.
    pub fn activate(&self) {
        if !self.is_active() {
            unsafe { kor::arch::current().activate_address_space(self.root) }
        }
    }

    pub fn is_active(&self) -> bool {
        kor::arch::current().address_space_root() == self.root
    }

    /// Run an operation with this root active, restoring the caller's root
    /// before returning. Interrupts are held off so the current task cannot be
    /// migrated while the temporary root is installed.
    pub fn with_active<T>(&self, operation: impl FnOnce() -> T) -> T {
        let interrupts_enabled = kor::irq::is_enabled();
        kor::irq::disable();
        let arch = kor::arch::current();
        let previous = arch.address_space_root();
        if previous != self.root {
            unsafe { arch.activate_address_space(self.root) };
        }
        let result = operation();
        if previous != self.root {
            unsafe { arch.activate_address_space(previous) };
        }
        if interrupts_enabled {
            kor::irq::enable();
        }
        result
    }

    pub fn translate(&self, vaddr: usize) -> Option<usize> {
        self.with_active(|| kor::arch::current().translate(vaddr))
    }

    pub fn map_zeroed(&self, vaddr: usize, flags: MappingFlags) -> Result<usize, MapError> {
        if vaddr & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::BlockedByExistingMapping);
        }
        self.with_active(|| {
            let arch = kor::arch::current();
            let paddr = kor_frame::alloc_page().ok_or(MapError::OutOfMemory)?;
            match arch.map(vaddr, paddr, flags, MapSize::Page4K) {
                Ok(()) => {
                    unsafe {
                        core::ptr::write_bytes(
                            arch.phys_to_virt(paddr) as *mut u8,
                            0,
                            PAGE_SIZE,
                        );
                    }
                    Ok(paddr)
                }
                Err(error) => {
                    kor_frame::free_frames(paddr, 1);
                    Err(error)
                }
            }
        })
    }

    /// Copy bytes into this address space without requiring it to remain
    /// active after the call.
    pub fn write(&self, vaddr: usize, bytes: &[u8]) -> Result<(), MapError> {
        self.with_active(|| {
            let arch = kor::arch::current();
            let mut written = 0;
            while written < bytes.len() {
                let address = vaddr.checked_add(written).ok_or(MapError::BlockedByExistingMapping)?;
                let paddr = arch.translate(address).ok_or(MapError::BlockedByExistingMapping)?;
                let chunk = core::cmp::min(PAGE_SIZE - (address & (PAGE_SIZE - 1)), bytes.len() - written);
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        bytes.as_ptr().add(written),
                        arch.phys_to_virt(paddr) as *mut u8,
                        chunk,
                    );
                }
                written += chunk;
            }
            Ok(())
        })
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        let arch = kor::arch::current();
        if self.owned {
            assert!(!self.is_active(), "dropping active address space");
            unsafe { arch.destroy_address_space(self.root) }
        }
    }
}
