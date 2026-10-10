//! Per-process user address spaces and their authoritative virtual-memory map.

extern crate alloc;

use alloc::collections::BTreeMap;
use kor_arch::{MapError, MapSize, MappingFlags};

pub const PAGE_SIZE: usize = 4096;
pub use crate::user::layout::ADDRESS_LIMIT as USER_ADDRESS_LIMIT;

#[derive(Clone, Copy)]
struct VmPage {
    phys: usize,
    flags: MappingFlags,
    /// Registry-owned shared frame (MAP_SHARED): never copied on fork,
    /// never freed by this space — the shm registry owns it.
    shared: bool,
}

/// An exclusively owned user page-table root plus the authoritative set of
/// user mappings. Kernel mappings inherited by a new root never appear here.
pub struct AddressSpace {
    root: usize,
    owned: bool,
    pages: BTreeMap<usize, VmPage>,
}

impl AddressSpace {
    /// Borrow the boot root retained by CPU idle tasks.
    pub fn adopt_active() -> Self {
        Self {
            root: kor_mmu::address_space_root(),
            owned: false,
            pages: BTreeMap::new(),
        }
    }

    /// Create a root containing kernel mappings but no user mappings.
    pub fn empty() -> Result<Self, MapError> {
        Ok(Self {
            root: kor_mmu::new_address_space()?,
            owned: true,
            pages: BTreeMap::new(),
        })
    }

    /// Deep-copy this address space for fork. The architecture clones physical
    /// contents; the VM map preserves virtual ownership and permissions.
    pub fn fork(&self) -> Result<Self, MapError> {
        let mut child = Self::empty()?;
        for (&vaddr, &page) in &self.pages {
            if page.shared {
                // Shared mappings alias the registry-owned frames in the
                // child: no copy, no transfer of ownership.
                child.map_shared_frame(vaddr, page.phys, page.flags)?;
                continue;
            }
            let child_phys = child.map_zeroed(vaddr, page.flags)?;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    kor_arch::phys_to_virt(page.phys) as *const u8,
                    kor_arch::phys_to_virt(child_phys) as *mut u8,
                    PAGE_SIZE,
                );
            }
        }
        Ok(child)
    }

    pub(crate) fn root(&self) -> usize {
        self.root
    }

    pub fn activate(&self) {
        if !self.is_active() {
            unsafe { kor_mmu::activate_address_space(self.root) }
        }
    }

    pub fn is_active(&self) -> bool {
        kor_mmu::address_space_root() == self.root
    }

    pub fn with_active<T>(&self, operation: impl FnOnce() -> T) -> T {
        let interrupts_enabled = kor_arch::irq_is_enabled();
        kor_arch::irq_disable();
        let previous = kor_mmu::address_space_root();
        if previous != self.root {
            // SAFETY: same contract as `activate`.
            unsafe { kor_mmu::AddrSpace::from_root(self.root).activate() };
        }
        let result = operation();
        if previous != self.root {
            // SAFETY: restoring the previously active root.
            unsafe { kor_mmu::activate_address_space(previous) };
        }
        if interrupts_enabled {
            kor_arch::irq_enable();
        }
        result
    }

    pub fn translate(&self, vaddr: usize) -> Option<usize> {
        let page = vaddr & !(PAGE_SIZE - 1);
        let mapping = self.pages.get(&page)?;
        Some(mapping.phys + (vaddr & (PAGE_SIZE - 1)))
    }

    pub fn is_mapped(&self, vaddr: usize) -> bool {
        self.pages.contains_key(&(vaddr & !(PAGE_SIZE - 1)))
    }

    pub fn validate_range(&self, address: usize, length: usize, write: bool) -> bool {
        if length == 0 {
            return address < USER_ADDRESS_LIMIT;
        }
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        if address >= USER_ADDRESS_LIMIT || end > USER_ADDRESS_LIMIT {
            return false;
        }
        let mut page = address & !(PAGE_SIZE - 1);
        let last = (end - 1) & !(PAGE_SIZE - 1);
        loop {
            let Some(mapping) = self.pages.get(&page) else {
                return false;
            };
            if write && !mapping.flags.contains(MappingFlags::W) {
                return false;
            }
            if page == last {
                return true;
            }
            let Some(next) = page.checked_add(PAGE_SIZE) else {
                return false;
            };
            page = next;
        }
    }

    /// Return whether every page in the aligned range is currently unmapped.
    pub fn range_is_free(&self, start: usize, length: usize) -> bool {
        if start & (PAGE_SIZE - 1) != 0 || length == 0 || length & (PAGE_SIZE - 1) != 0 {
            return false;
        }
        let Some(end) = start.checked_add(length) else {
            return false;
        };
        end <= USER_ADDRESS_LIMIT
            && (start..end)
                .step_by(PAGE_SIZE)
                .all(|address| !self.pages.contains_key(&address))
    }

    pub fn map_zeroed(&mut self, vaddr: usize, flags: MappingFlags) -> Result<usize, MapError> {
        if vaddr & (PAGE_SIZE - 1) != 0
            || vaddr >= USER_ADDRESS_LIMIT
            || !flags.contains(MappingFlags::U)
        {
            return Err(MapError::BlockedByExistingMapping);
        }
        if self.pages.contains_key(&vaddr) {
            return Err(MapError::BlockedByExistingMapping);
        }
        self.with_active(|| {
            let paddr = kor::frame::alloc_frame().ok_or(MapError::OutOfMemory)?;
            match kor_mmu::active().map(vaddr, paddr, flags, MapSize::Page4K) {
                Ok(()) => {
                    unsafe {
                        core::ptr::write_bytes(
                            kor_arch::phys_to_virt(paddr) as *mut u8,
                            0,
                            PAGE_SIZE,
                        );
                    }
                    Ok(paddr)
                }
                Err(error) => {
                    kor::frame::free_frames(paddr, 1);
                    Err(error)
                }
            }
        })
        .inspect(|&phys| {
            self.pages.insert(
                vaddr,
                VmPage {
                    phys,
                    flags,
                    shared: false,
                },
            );
        })
    }

    /// Map a registry-owned shared frame at `vaddr` without taking ownership:
    /// the frame is aliased into this space, not copied and never freed by it.
    pub fn map_shared_frame(
        &mut self,
        vaddr: usize,
        phys: usize,
        flags: MappingFlags,
    ) -> Result<(), MapError> {
        if vaddr & (PAGE_SIZE - 1) != 0
            || vaddr >= USER_ADDRESS_LIMIT
            || !flags.contains(MappingFlags::U)
            || self.pages.contains_key(&vaddr)
        {
            return Err(MapError::BlockedByExistingMapping);
        }
        self.with_active(|| kor_mmu::active().map(vaddr, phys, flags, MapSize::Page4K))?;
        self.pages.insert(
            vaddr,
            VmPage {
                phys,
                flags,
                shared: true,
            },
        );
        Ok(())
    }

    /// Replace a complete page-aligned range transactionally. Fresh frames are
    /// allocated before old mappings are removed; any failure restores the old
    /// page-table entries and VM metadata.
    pub fn replace_zeroed(
        &mut self,
        start: usize,
        length: usize,
        flags: MappingFlags,
    ) -> Result<(), MapError> {
        if start & (PAGE_SIZE - 1) != 0 || length == 0 || length & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::BlockedByExistingMapping);
        }
        let end = start
            .checked_add(length)
            .ok_or(MapError::BlockedByExistingMapping)?;
        if end > USER_ADDRESS_LIMIT || !flags.contains(MappingFlags::U) {
            return Err(MapError::BlockedByExistingMapping);
        }

        let count = length / PAGE_SIZE;
        let mut fresh = alloc::vec::Vec::with_capacity(count);
        for _ in 0..count {
            match kor::frame::alloc_frame() {
                Some(phys) => fresh.push(phys),
                None => {
                    for phys in fresh {
                        kor::frame::free_frames(phys, 1);
                    }
                    return Err(MapError::OutOfMemory);
                }
            }
        }

        self.with_active(|| {
            let old: alloc::vec::Vec<_> = (start..end)
                .step_by(PAGE_SIZE)
                .map(|vaddr| (vaddr, self.pages.get(&vaddr).copied()))
                .collect();
            for &(vaddr, mapping) in &old {
                if mapping.is_some() {
                    kor_mmu::active().unmap(vaddr, MapSize::Page4K)?;
                }
            }
            for (index, vaddr) in (start..end).step_by(PAGE_SIZE).enumerate() {
                if let Err(error) =
                    kor_mmu::active().map(vaddr, fresh[index], flags, MapSize::Page4K)
                {
                    for rollback in (start..vaddr).step_by(PAGE_SIZE) {
                        let _ = kor_mmu::active().unmap(rollback, MapSize::Page4K);
                    }
                    for &(old_vaddr, mapping) in &old {
                        if let Some(mapping) = mapping {
                            let _ = kor_mmu::active().map(
                                old_vaddr,
                                mapping.phys,
                                mapping.flags,
                                MapSize::Page4K,
                            );
                        }
                    }
                    return Err(error);
                }
                unsafe {
                    core::ptr::write_bytes(
                        kor_arch::phys_to_virt(fresh[index]) as *mut u8,
                        0,
                        PAGE_SIZE,
                    );
                }
            }
            for mapping in old.into_iter().filter_map(|(_, mapping)| mapping) {
                kor::frame::free_frames(mapping.phys, 1);
            }
            Ok(())
        })?;
        for (index, vaddr) in (start..end).step_by(PAGE_SIZE).enumerate() {
            self.pages.insert(
                vaddr,
                VmPage {
                    phys: fresh[index],
                    flags,
                    shared: false,
                },
            );
        }
        Ok(())
    }

    pub fn protect_range(
        &mut self,
        start: usize,
        length: usize,
        flags: MappingFlags,
    ) -> Result<(), MapError> {
        if start & (PAGE_SIZE - 1) != 0
            || length == 0
            || length & (PAGE_SIZE - 1) != 0
            || !flags.contains(MappingFlags::U)
        {
            return Err(MapError::BlockedByExistingMapping);
        }
        let end = start
            .checked_add(length)
            .ok_or(MapError::BlockedByExistingMapping)?;
        if end > USER_ADDRESS_LIMIT {
            return Err(MapError::BlockedByExistingMapping);
        }
        let old: alloc::vec::Vec<_> = (start..end)
            .step_by(PAGE_SIZE)
            .map(|vaddr| {
                self.pages
                    .get(&vaddr)
                    .copied()
                    .map(|page| (vaddr, page))
                    .ok_or(MapError::BlockedByExistingMapping)
            })
            .collect::<Result<_, _>>()?;
        self.with_active(|| {
            for &(vaddr, _) in &old {
                kor_mmu::active().unmap(vaddr, MapSize::Page4K)?;
            }
            for (index, &(vaddr, page)) in old.iter().enumerate() {
                if let Err(error) = kor_mmu::active().map(vaddr, page.phys, flags, MapSize::Page4K)
                {
                    for &(mapped, _) in &old[..index] {
                        let _ = kor_mmu::active().unmap(mapped, MapSize::Page4K);
                    }
                    for &(rollback, old_page) in &old {
                        let _ = kor_mmu::active().map(
                            rollback,
                            old_page.phys,
                            old_page.flags,
                            MapSize::Page4K,
                        );
                    }
                    return Err(error);
                }
            }
            Ok(())
        })?;
        for (vaddr, page) in old {
            self.pages.insert(vaddr, VmPage { flags, ..page });
        }
        Ok(())
    }

    pub fn unmap_range(&mut self, start: usize, length: usize) -> Result<(), MapError> {
        if start & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::BlockedByExistingMapping);
        }
        let end = start
            .checked_add(length)
            .ok_or(MapError::BlockedByExistingMapping)?;
        let pages: alloc::vec::Vec<_> = (start..end)
            .step_by(PAGE_SIZE)
            .filter_map(|vaddr| self.pages.get(&vaddr).copied().map(|page| (vaddr, page)))
            .collect();
        self.with_active(|| {
            for &(vaddr, _) in &pages {
                kor_mmu::active().unmap(vaddr, MapSize::Page4K)?;
            }
            Ok(())
        })?;
        for (vaddr, page) in pages {
            self.pages.remove(&vaddr);
            if !page.shared {
                kor::frame::free_frames(page.phys, 1);
            }
        }
        Ok(())
    }

    /// Copy bytes into mapped pages while constructing an image or mmap.
    ///
    /// Unlike [`AddressSpace::write`], this ignores user-visible write
    /// permissions; the kernel owns the pages and uses the direct map.
    pub fn populate(&self, vaddr: usize, bytes: &[u8]) -> Result<(), MapError> {
        if !self.validate_range(vaddr, bytes.len(), false) {
            return Err(MapError::BlockedByExistingMapping);
        }
        let mut written = 0;
        while written < bytes.len() {
            let address = vaddr
                .checked_add(written)
                .ok_or(MapError::BlockedByExistingMapping)?;
            let paddr = self
                .translate(address)
                .ok_or(MapError::BlockedByExistingMapping)?;
            let chunk = core::cmp::min(
                PAGE_SIZE - (address & (PAGE_SIZE - 1)),
                bytes.len() - written,
            );
            // SAFETY: `validate_range` and `translate` prove that each direct-
            // map destination is an owned mapped page for this address space.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    bytes.as_ptr().add(written),
                    kor_arch::phys_to_virt(paddr) as *mut u8,
                    chunk,
                );
            }
            written += chunk;
        }
        Ok(())
    }

    pub fn write(&self, vaddr: usize, bytes: &[u8]) -> Result<(), MapError> {
        if !self.validate_range(vaddr, bytes.len(), true) {
            return Err(MapError::BlockedByExistingMapping);
        }
        let mut written = 0;
        while written < bytes.len() {
            let address = vaddr
                .checked_add(written)
                .ok_or(MapError::BlockedByExistingMapping)?;
            let paddr = self
                .translate(address)
                .ok_or(MapError::BlockedByExistingMapping)?;
            let chunk = core::cmp::min(
                PAGE_SIZE - (address & (PAGE_SIZE - 1)),
                bytes.len() - written,
            );
            unsafe {
                core::ptr::copy_nonoverlapping(
                    bytes.as_ptr().add(written),
                    kor_arch::phys_to_virt(paddr) as *mut u8,
                    chunk,
                );
            }
            written += chunk;
        }
        Ok(())
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        if self.owned {
            assert!(!self.is_active(), "dropping active address space");
            // Shared frames are registry-owned: detach their PTEs first so
            // the arch walk below cannot return them to the frame allocator.
            let shared: alloc::vec::Vec<usize> = self
                .pages
                .iter()
                .filter(|(_, page)| page.shared)
                .map(|(vaddr, _)| *vaddr)
                .collect();
            let _ = self.with_active(|| {
                for vaddr in &shared {
                    let _ = kor_mmu::active().unmap(*vaddr, MapSize::Page4K);
                }
            });
            // kor-arch's destroy_table returns every remaining level-0 leaf
            // data frame along with the table pages.
            unsafe { kor_mmu::destroy_address_space(self.root) }
        }
    }
}
