//! x86_64 runtime page-table operations (4-level, 4 KiB pages + 2 MiB huge pages).

use kor::addr::{PhysAddr, VirtAddr};
use kor::{MapError, MapSize, MappingFlags};
use kor_frame::alloc_page;
use x86_64::registers::control::Cr3;

/// First PML4 index after the boot high-half 512 GiB slot (index 256).
pub const TEST_VA_4K: usize = 0xFFFF_8080_0000_0000;
pub const TEST_VA_2M: usize = 0xFFFF_8080_0020_0000;

const PTE_P: usize = 1 << 0;
const PTE_RW: usize = 1 << 1;
const PTE_US: usize = 1 << 2;
const PTE_PS: usize = 1 << 7;
const PTE_NUM: usize = 512;

use core::cell::UnsafeCell;

struct PageTableRoot(UnsafeCell<PhysAddr>);
unsafe impl Sync for PageTableRoot {}

static ROOT: PageTableRoot = PageTableRoot(UnsafeCell::new(PhysAddr::new(0)));

pub fn init() {
    unsafe {
        *ROOT.0.get() = PhysAddr::new(Cr3::read().0.start_address().as_u64() as usize);
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
            if !is_present(pte) {
                return None;
            }
            if is_huge(pte) {
                let off = vaddr.page_offset(level);
                return Some(pte_address(pte) + off);
            }
            table = PhysAddr::new(pte_address(pte));
        }
        let pte = table.page_slice_mut::<usize>(PTE_NUM)[vaddr.pn_index(0)];
        if !is_present(pte) || is_huge(pte) {
            return None;
        }
        Some(pte_address(pte) + vaddr.page_offset(0))
    }
}

fn map_4k(vaddr: VirtAddr, paddr: PhysAddr, flags: MappingFlags) -> Result<(), MapError> {
    unsafe {
        let mut root = *ROOT.0.get();
        let pdpt = walk_table(&mut root, vaddr, 3)?;
        *ROOT.0.get() = root;
        let mut pdpt = pdpt;
        let pd = walk_table(&mut pdpt, vaddr, 2)?;
        let mut pd = pd;
        let pt = walk_table(&mut pd, vaddr, 1)?;
        let idx = vaddr.pn_index(0);
        let entry = &mut pt.page_slice_mut::<usize>(PTE_NUM)[idx];
        if is_present(*entry) {
            return Err(MapError::BlockedByExistingMapping);
        }
        *entry = leaf_pte_4k(paddr.raw(), flags);
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
        let pdpt = walk_table(&mut root, vaddr, 3)?;
        *ROOT.0.get() = root;
        let mut pdpt = pdpt;
        let pd = walk_table(&mut pdpt, vaddr, 2)?;
        let idx = vaddr.pn_index(1);
        let entry = &mut pd.page_slice_mut::<usize>(PTE_NUM)[idx];
        if is_present(*entry) {
            return Err(MapError::BlockedByExistingMapping);
        }
        *entry = leaf_pte_2m(paddr.raw(), flags);
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
    if is_present(*entry) && is_huge(*entry) {
        return Err(MapError::BlockedByExistingMapping);
    }
    if !is_present(*entry) {
        let frame = alloc_page().ok_or(MapError::OutOfMemory)?;
        PhysAddr::new(frame).clear_page();
        *entry = table_pte(frame);
    }
    Ok(PhysAddr::new(pte_address(*entry)))
}

fn leaf_pte_4k(paddr: usize, flags: MappingFlags) -> usize {
    paddr | hw_flags(flags)
}

fn leaf_pte_2m(paddr: usize, flags: MappingFlags) -> usize {
    paddr | hw_flags(flags) | PTE_PS
}

fn table_pte(paddr: usize) -> usize {
    paddr | PTE_P | PTE_RW | PTE_US
}

fn hw_flags(flags: MappingFlags) -> usize {
    let mut hw = PTE_P;
    if flags.contains(MappingFlags::W) {
        hw |= PTE_RW;
    }
    if flags.contains(MappingFlags::U) {
        hw |= PTE_US;
    }
    hw
}

fn is_present(pte: usize) -> bool {
    pte & PTE_P != 0
}

fn is_huge(pte: usize) -> bool {
    pte & PTE_PS != 0
}

fn pte_address(pte: usize) -> usize {
    pte & 0xFFFF_FFFF_F000
}

fn flush_vaddr(vaddr: VirtAddr) {
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) vaddr.raw(), options(nostack));
    }
}
