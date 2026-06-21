//! AArch64 runtime page-table operations (4 KiB pages + 2 MiB blocks).

use crate::mm::addr::{PhysAddr, VirtAddr};
use crate::mm::{alloc_page, MapError, MapSize, MappingFlags};

/// 512 GiB slot — L0 index 1 is unused by the boot 1 GiB block map at L0[0].
pub const TEST_VA_4K: usize = 0x0080_0000_0000;
pub const TEST_VA_2M: usize = 0x0080_0020_0000;

const PTE_VALID: usize = 1 << 0;
const PTE_TABLE: usize = 1 << 1;
const PTE_AF: usize = 1 << 10;
const PTE_ATTR: usize = 1 << 2; // MAIR index 1 (Normal WB), matches boot.S
const PTE_NUM: usize = 512;

use core::cell::UnsafeCell;

struct PageTableRoot(UnsafeCell<PhysAddr>);
unsafe impl Sync for PageTableRoot {}

static ROOT: PageTableRoot = PageTableRoot(UnsafeCell::new(PhysAddr::new(0)));

pub fn init() {
    unsafe {
        let ttbr: u64;
        core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr);
        *ROOT.0.get() = PhysAddr::new((ttbr as usize) & 0xFFFF_FFFF_F000);
    }
}

pub fn dynamic_maps_supported() -> bool {
    true
}

pub fn map(vaddr: usize, paddr: usize, flags: MappingFlags, size: MapSize) -> Result<(), MapError> {
    let vaddr = VirtAddr::new(vaddr);
    let paddr = PhysAddr::new(paddr);
    match size {
        MapSize::Page4K => map_4k(vaddr, paddr, flags),
        MapSize::Page2M => map_2m(vaddr, paddr, flags),
    }
}

pub fn translate(vaddr: usize) -> Option<usize> {
    let vaddr = VirtAddr::new(vaddr);
    unsafe {
        let mut table = *ROOT.0.get();
        for level in (1..=3).rev() {
            let idx = vaddr.pn_index(level);
            let pte = table.page_slice_mut::<usize>(PTE_NUM)[idx];
            if !is_valid(pte) {
                return None;
            }
            if is_block(pte) {
                let off = vaddr.page_offset(level);
                return Some(pte_address(pte) + off);
            }
            if !is_table(pte) {
                return None;
            }
            table = PhysAddr::new(pte_address(pte));
        }
        let pte = table.page_slice_mut::<usize>(PTE_NUM)[vaddr.pn_index(0)];
        if !is_valid(pte) || is_block(pte) || !is_table(pte) {
            return None;
        }
        Some(pte_address(pte) + vaddr.page_offset(0))
    }
}

fn map_4k(vaddr: VirtAddr, paddr: PhysAddr, flags: MappingFlags) -> Result<(), MapError> {
    unsafe {
        let mut root = *ROOT.0.get();
        let l2 = walk_table(&mut root, vaddr, 3)?;
        *ROOT.0.get() = root;
        let mut l2 = l2;
        let l1 = walk_table(&mut l2, vaddr, 2)?;
        let mut l1 = l1;
        let l0 = walk_table(&mut l1, vaddr, 1)?;
        let idx = vaddr.pn_index(0);
        let entry = &mut l0.page_slice_mut::<usize>(PTE_NUM)[idx];
        if is_valid(*entry) {
            return Err(MapError::BlockedByExistingMapping);
        }
        *entry = page_pte(paddr.raw(), flags);
        flush_vaddr(vaddr);
    }
    Ok(())
}

fn map_2m(vaddr: VirtAddr, paddr: PhysAddr, flags: MappingFlags) -> Result<(), MapError> {
    if paddr.raw() % (2 * 1024 * 1024) != 0 {
        return Err(MapError::BlockedByExistingMapping);
    }
    unsafe {
        let mut root = *ROOT.0.get();
        let l2 = walk_table(&mut root, vaddr, 3)?;
        *ROOT.0.get() = root;
        let mut l2 = l2;
        let l1 = walk_table(&mut l2, vaddr, 2)?;
        let idx = vaddr.pn_index(1);
        let entry = &mut l1.page_slice_mut::<usize>(PTE_NUM)[idx];
        if is_valid(*entry) {
            return Err(MapError::BlockedByExistingMapping);
        }
        *entry = block_pte(paddr.raw(), flags);
        flush_vaddr(vaddr);
    }
    Ok(())
}

unsafe fn walk_table(
    parent: &mut PhysAddr,
    vaddr: VirtAddr,
    level: usize,
) -> Result<PhysAddr, MapError> {
    let idx = vaddr.pn_index(level);
    let entry = &mut parent.page_slice_mut::<usize>(PTE_NUM)[idx];
    if is_valid(*entry) && is_block(*entry) {
        return Err(MapError::BlockedByExistingMapping);
    }
    if !is_valid(*entry) {
        let frame = alloc_page().ok_or(MapError::OutOfMemory)?;
        PhysAddr::new(frame).clear_page();
        *entry = table_pte(frame);
    } else if !is_table(*entry) {
        return Err(MapError::BlockedByExistingMapping);
    }
    Ok(PhysAddr::new(pte_address(*entry)))
}

fn table_pte(paddr: usize) -> usize {
    paddr | PTE_VALID | PTE_TABLE
}

fn page_pte(paddr: usize, flags: MappingFlags) -> usize {
    paddr | PTE_VALID | PTE_TABLE | PTE_AF | PTE_ATTR | ap_flags(flags)
}

fn block_pte(paddr: usize, flags: MappingFlags) -> usize {
    paddr | PTE_VALID | PTE_AF | PTE_ATTR | ap_flags(flags)
}

fn ap_flags(flags: MappingFlags) -> usize {
    let mut extra = 0;
    if !flags.contains(MappingFlags::W) {
        extra |= 1 << 7; // AP[2]: read-only
    }
    if flags.contains(MappingFlags::U) {
        extra |= 1 << 6; // AP[1]: EL0
    }
    if !flags.contains(MappingFlags::X) {
        extra |= 1 << 54; // UXN
    }
    extra
}

fn is_valid(pte: usize) -> bool {
    pte & PTE_VALID != 0
}

fn is_table(pte: usize) -> bool {
    is_valid(pte) && pte & PTE_TABLE != 0
}

fn is_block(pte: usize) -> bool {
    is_valid(pte) && pte & PTE_TABLE == 0
}

fn pte_address(pte: usize) -> usize {
    pte & 0xFFFF_FFFF_F000
}

fn flush_vaddr(vaddr: VirtAddr) {
    unsafe {
        core::arch::asm!(
            "tlbi vaae1is, {0}",
            "dsb sy",
            "isb",
            in(reg) (vaddr.raw() >> 12) & 0xFFFF_FFFF_FFFF,
        );
    }
}
