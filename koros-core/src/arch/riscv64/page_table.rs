//! RISC-V Sv39 runtime page-table operations.

use crate::mm::addr::{PhysAddr, VirtAddr};
use crate::mm::{alloc_page, MapError, MapSize, MappingFlags};

/// First unused 1 GiB slot in the high-half root (boot leaves use 256–259).
pub const TEST_VA_4K: usize = 0xFFFF_FFC4_1000_0000;
/// Separate 1 GiB slot (root index 261) for 2 MiB mapping tests.
pub const TEST_VA_2M: usize = 0xFFFF_FFC5_0000_0000;

const PTE_V: usize = 1 << 0;
const PTE_R: usize = 1 << 1;
const PTE_W: usize = 1 << 2;
const PTE_X: usize = 1 << 3;
const PTE_A: usize = 1 << 6;
const PTE_D: usize = 1 << 7;
const PTE_RW: usize = PTE_R | PTE_W | PTE_X;

const PTE_NUM: usize = 512;

use core::cell::UnsafeCell;

struct PageTableRoot(UnsafeCell<PhysAddr>);
unsafe impl Sync for PageTableRoot {}

static ROOT: PageTableRoot = PageTableRoot(UnsafeCell::new(PhysAddr::new(0)));

pub fn init() {
    unsafe {
        let satp: usize;
        core::arch::asm!("csrr {}, satp", out(reg) satp);
        let ppn = satp & ((1 << 44) - 1);
        *ROOT.0.get() = PhysAddr::new(ppn << 12);
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
        for level in (1..=2).rev() {
            let idx = vaddr.pn_index(level);
            let pte = table.page_slice_mut::<usize>(PTE_NUM)[idx];
            if !is_valid(pte) {
                return None;
            }
        if is_leaf(pte) {
            if level == 1 {
                return Some(megapage_2m_address(pte, vaddr));
            }
            let off = vaddr.page_offset(level);
            return Some(pte_address(pte) + off);
        }
            table = PhysAddr::new(pte_address(pte));
        }
        let pte = table.page_slice_mut::<usize>(PTE_NUM)[vaddr.pn_index(0)];
        if !is_valid(pte) || !is_leaf(pte) {
            return None;
        }
        Some(pte_address(pte) + vaddr.page_offset(0))
    }
}

fn map_4k(vaddr: VirtAddr, paddr: PhysAddr, flags: MappingFlags) -> Result<(), MapError> {
    unsafe {
        let mut root = *ROOT.0.get();
        let l1 = walk_table(&mut root, vaddr, 2)?;
        *ROOT.0.get() = root;
        let mut l1 = l1;
        let l0 = walk_table(&mut l1, vaddr, 1)?;
        let idx = vaddr.pn_index(0);
        let entry = &mut l0.page_slice_mut::<usize>(PTE_NUM)[idx];
        if is_valid(*entry) {
            return Err(MapError::BlockedByExistingMapping);
        }
        *entry = leaf_pte(paddr.raw(), flags);
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
        let l1 = walk_table(&mut root, vaddr, 2)?;
        *ROOT.0.get() = root;
        let idx = vaddr.pn_index(1);
        let entry = &mut l1.page_slice_mut::<usize>(PTE_NUM)[idx];
        if is_valid(*entry) {
            return Err(MapError::BlockedByExistingMapping);
        }
        *entry = megapage_2m_pte(paddr.raw(), flags);
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
    if is_valid(*entry) && is_leaf(*entry) {
        return Err(MapError::BlockedByExistingMapping);
    }
    if !is_valid(*entry) {
        let frame = alloc_page().ok_or(MapError::OutOfMemory)?;
        PhysAddr::new(frame).clear_page();
        *entry = table_pte(frame);
    }
    Ok(PhysAddr::new(pte_address(*entry)))
}

fn flags_to_hw(flags: MappingFlags) -> usize {
    let mut hw = PTE_V;
    if flags.contains(MappingFlags::R) {
        hw |= PTE_R | PTE_A;
    }
    if flags.contains(MappingFlags::W) {
        hw |= PTE_W | PTE_D;
    }
    if flags.contains(MappingFlags::X) {
        hw |= PTE_X;
    }
    if flags.contains(MappingFlags::U) {
        hw |= 1 << 4;
    }
    hw
}

fn leaf_pte(paddr: usize, flags: MappingFlags) -> usize {
    (paddr >> 2) | flags_to_hw(flags)
}

/// Sv39 level-1 megapage: PTE[19:10] must be zero; PA[20:12] comes from VA[20:12].
fn megapage_2m_pte(paddr: usize, flags: MappingFlags) -> usize {
    ((paddr >> 2) & !0xFFC00) | flags_to_hw(flags)
}

fn megapage_2m_address(pte: usize, vaddr: VirtAddr) -> usize {
    ((pte << 2) & !0x1F_FFFF) | (vaddr.raw() & 0x1F_FFFF)
}

fn table_pte(paddr: usize) -> usize {
    (paddr >> 2) | PTE_V
}

fn is_valid(pte: usize) -> bool {
    pte & PTE_V != 0
}

fn is_leaf(pte: usize) -> bool {
    is_valid(pte) && pte & PTE_RW != 0
}

fn pte_address(pte: usize) -> usize {
    (pte << 2) & 0xFFFF_FFFF_F000
}

fn flush_vaddr(vaddr: VirtAddr) {
    unsafe {
        core::arch::asm!("sfence.vma {}, zero", in(reg) vaddr.raw());
    }
}
