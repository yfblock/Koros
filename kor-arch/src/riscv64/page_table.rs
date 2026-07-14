//! RISC-V Sv39 runtime page-table operations.

use kor::addr::{PhysAddr, VirtAddr};
use kor::{MapError, MapSize, MappingFlags};
use kor_frame::alloc_page;

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

pub fn init() {}

pub fn dynamic_maps_supported() -> bool {
    true
}

/// Read the root installed on this CPU. Address spaces are task state, so a
/// process running on another CPU must never observe a shared cached root.
pub fn root() -> usize {
    let satp: usize;
    unsafe { core::arch::asm!("csrr {}, satp", out(reg) satp); }
    (satp & ((1usize << 44) - 1)) << 12
}

/// Activate an Sv39 root page table.
///
/// # Safety
/// `root` must be a valid, page-aligned Sv39 root containing live kernel mappings.
pub unsafe fn activate(root: usize) {
    let satp = (8usize << 60) | (root >> 12);
    unsafe { core::arch::asm!("csrw satp, {}", "sfence.vma", in(reg) satp); }
}
pub fn clone_active() -> Result<usize, MapError> {
    let source = root();
    let cloned = unsafe { clone_table(source, 2)? };
    Ok(cloned)
}

pub fn new_user() -> Result<usize, MapError> {
    let source = root();
    let cloned = alloc_page().ok_or(MapError::OutOfMemory)?;
    PhysAddr::new(cloned).clear_page();
    unsafe {
        let src = PhysAddr::new(source).page_slice_mut::<usize>(PTE_NUM);
        let dst = PhysAddr::new(cloned).page_slice_mut::<usize>(PTE_NUM);
        dst[256..].copy_from_slice(&src[256..]);
    }
    Ok(cloned)
}

unsafe fn clone_table(source: usize, level: usize) -> Result<usize, MapError> {
    let cloned = alloc_page().ok_or(MapError::OutOfMemory)?;
    PhysAddr::new(cloned).clear_page();
    let src = unsafe { PhysAddr::new(source).page_slice_mut::<usize>(PTE_NUM) };
    let dst = unsafe { PhysAddr::new(cloned).page_slice_mut::<usize>(PTE_NUM) };
    for index in 0..PTE_NUM {
        let entry = src[index];
        if !is_valid(entry) || (level == 2 && index >= 256) {
            dst[index] = entry;
        } else if is_leaf(entry) {
            if level == 0 {
                let page = match alloc_page() {
                    Some(page) => page,
                    None => {
                        unsafe { destroy_table(cloned, level); }
                        return Err(MapError::OutOfMemory);
                    }
                };
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        kor::arch::phys_to_virt(pte_address(entry)) as *const u8,
                        kor::arch::phys_to_virt(page) as *mut u8,
                        4096,
                    );
                }
                dst[index] = leaf_pte(page, mapping_flags(entry));
            } else {
                dst[index] = entry;
            }
        } else {
            let child = match unsafe { clone_table(pte_address(entry), level - 1) } {
                Ok(child) => child,
                Err(error) => {
                    unsafe { destroy_table(cloned, level); }
                    return Err(error);
                }
            };
            dst[index] = table_pte(child);
        }
    }
    Ok(cloned)
}

fn mapping_flags(entry: usize) -> MappingFlags {
    let mut flags = MappingFlags::U;
    if entry & PTE_R != 0 { flags = flags | MappingFlags::R; }
    if entry & PTE_W != 0 { flags = flags | MappingFlags::W; }
    if entry & PTE_X != 0 { flags = flags | MappingFlags::X; }
    flags
}

pub unsafe fn destroy(root: usize) {
    unsafe { destroy_table(root, 2); }
}

unsafe fn destroy_table(table: usize, level: usize) {
    let entries = unsafe { PhysAddr::new(table).page_slice_mut::<usize>(PTE_NUM) };
    for &entry in &entries[..256] {
        if !is_valid(entry) { continue; }
        if is_leaf(entry) {
            if level == 0 { kor_frame::free_frames(pte_address(entry), 1); }
        } else if level > 0 {
            unsafe { destroy_table(pte_address(entry), level - 1); }
        }
    }
    kor_frame::free_frames(table, 1);
}

/// Split L2 entry 0 (first 1 GiB) from a 1 GiB leaf into L1 + L0 tables.
///
/// The boot page table uses 1 GiB L2 leaf entries for identity mapping.
/// This prevents mapping individual 4 KiB user pages in the low address
/// space.  This function splits entry 0 into:
///   - An L1 table with 512 × 2 MiB identity-mapped leaves (entries 1-511)
///   - An L0 table for the first 2 MiB (L1 entry 0) with 512 × 4 KiB leaves
/// After splitting, the identity mapping is preserved at 4 KiB granularity
/// for the first 2 MiB and 2 MiB granularity for the rest, and individual
/// user pages can be mapped by overwriting L0 entries.
pub fn split_l2_identity() {
    unsafe {
        let root = PhysAddr::new(root());
        let l2_table = root.page_slice_mut::<usize>(PTE_NUM);
        // Check if entry 0 is already split (non-leaf)
        if l2_table[0] != 0 && !is_leaf(l2_table[0]) {
            return; // Already split
        }

        // Allocate L1 table
        let l1_paddr = alloc_page().expect("OOM: L1 table");
        PhysAddr::new(l1_paddr).clear_page();
        let l1_table = PhysAddr::new(l1_paddr).page_slice_mut::<usize>(PTE_NUM);

        // Fill L1 entries 1-511 with 2 MiB identity-mapped leaves
        let leaf_flags = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D;
        for i in 1..512 {
            let pa_2m = i * (2 * 1024 * 1024);
            l1_table[i] = (pa_2m >> 2) | leaf_flags;
        }

        // Allocate L0 table for the first 2 MiB (L1 entry 0)
        let l0_paddr = alloc_page().expect("OOM: L0 table");
        PhysAddr::new(l0_paddr).clear_page();
        let l0_table = PhysAddr::new(l0_paddr).page_slice_mut::<usize>(PTE_NUM);

        // Fill L0 entries with 4 KiB identity-mapped leaves
        for i in 0..512 {
            let pa_4k = i * 4096;
            l0_table[i] = (pa_4k >> 2) | leaf_flags;
        }

        // Point L1 entry 0 to the L0 table (non-leaf PTE)
        l1_table[0] = table_pte(l0_paddr);

        // Replace L2 entry 0 with pointer to L1 table (non-leaf PTE)
        l2_table[0] = table_pte(l1_paddr);

        core::arch::asm!("sfence.vma");
    }
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
        let mut table = PhysAddr::new(root());
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
        let mut root = PhysAddr::new(root());
        let l1 = walk_table(&mut root, vaddr, 2)?;
        let mut l1 = l1;
        let l0 = walk_table(&mut l1, vaddr, 1)?;
        let idx = vaddr.pn_index(0);
        let entry = &mut l0.page_slice_mut::<usize>(PTE_NUM)[idx];
        // Allow user mappings to overwrite identity-mapped boot leaves.
        if is_valid(*entry) && !flags.contains(MappingFlags::U) {
            return Err(MapError::BlockedByExistingMapping);
        }
        *entry = leaf_pte(paddr.raw(), flags);
        flush_vaddr(vaddr);
    }
    Ok(())
}
pub fn unmap(vaddr: usize, size: MapSize) -> Result<usize, MapError> {
    let vaddr = VirtAddr::new(vaddr);
    unsafe {
        let mut table = PhysAddr::new(root());
        let target_level = if size == MapSize::Page2M { 1 } else { 0 };
        for level in (target_level + 1..=2).rev() {
            let entry = table.page_slice_mut::<usize>(PTE_NUM)[vaddr.pn_index(level)];
            if !is_valid(entry) || is_leaf(entry) { return Err(MapError::BlockedByExistingMapping); }
            table = PhysAddr::new(pte_address(entry));
        }
        let entry = &mut table.page_slice_mut::<usize>(PTE_NUM)[vaddr.pn_index(target_level)];
        if !is_valid(*entry) || !is_leaf(*entry) { return Err(MapError::BlockedByExistingMapping); }
        let paddr = pte_address(*entry);
        *entry = 0;
        flush_vaddr(vaddr);
        Ok(paddr)
    }
}


fn map_2m(vaddr: VirtAddr, paddr: PhysAddr, flags: MappingFlags) -> Result<(), MapError> {
    if paddr.raw() % (2 * 1024 * 1024) != 0 {
        return Err(MapError::BlockedByExistingMapping);
    }
    unsafe {
        let mut root = PhysAddr::new(root());
        let l1 = walk_table(&mut root, vaddr, 2)?;
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
