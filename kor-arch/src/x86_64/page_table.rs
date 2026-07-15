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

pub fn init() {}

/// Read CR3 on the current CPU instead of caching another CPU's root.
pub fn root() -> usize {
    Cr3::read().0.start_address().as_u64() as usize
}

/// Activate a CR3 page-table root.
///
/// # Safety
/// `root` must be a valid, page-aligned PML4 containing live kernel mappings.
pub unsafe fn activate(root: usize) {
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) root, options(nostack));
    }
}
pub fn clone_active() -> Result<usize, MapError> {
    unsafe { clone_table(root(), 3) }
}

pub fn new_user() -> Result<usize, MapError> {
    let source = root();
    let cloned = alloc_page().ok_or(MapError::OutOfMemory)?;
    unsafe { PhysAddr::new(cloned).clear_page() };
    unsafe {
        let src = unsafe { PhysAddr::new(source).page_slice_mut::<usize>(PTE_NUM) };
        let dst = unsafe { PhysAddr::new(cloned).page_slice_mut::<usize>(PTE_NUM) };
        dst[256..].copy_from_slice(&src[256..]);
    }
    Ok(cloned)
}

unsafe fn clone_table(source: usize, level: usize) -> Result<usize, MapError> {
    let cloned = alloc_page().ok_or(MapError::OutOfMemory)?;
    unsafe { PhysAddr::new(cloned).clear_page() };
    let src = unsafe { unsafe { PhysAddr::new(source).page_slice_mut::<usize>(PTE_NUM) } };
    let dst = unsafe { unsafe { PhysAddr::new(cloned).page_slice_mut::<usize>(PTE_NUM) } };
    for index in 0..PTE_NUM {
        let entry = src[index];
        if !is_present(entry) || (level == 3 && index >= 256) {
            dst[index] = entry;
        } else if level == 0 {
            let page = match alloc_page() {
                Some(page) => page,
                None => {
                    unsafe {
                        destroy_table(cloned, level);
                    }
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
            dst[index] = page | (entry & !0x000F_FFFF_FFFF_F000);
        } else if is_huge(entry) {
            dst[index] = entry;
        } else {
            let child = match unsafe { clone_table(pte_address(entry), level - 1) } {
                Ok(child) => child,
                Err(error) => {
                    unsafe {
                        destroy_table(cloned, level);
                    }
                    return Err(error);
                }
            };
            dst[index] = child | (entry & !0x000F_FFFF_FFFF_F000);
        }
    }
    Ok(cloned)
}

pub unsafe fn destroy(root: usize) {
    unsafe {
        destroy_table(root, 3);
    }
}

unsafe fn destroy_table(table: usize, level: usize) {
    let entries = unsafe { unsafe { PhysAddr::new(table).page_slice_mut::<usize>(PTE_NUM) } };
    for &entry in &entries[..256] {
        if !is_present(entry) {
            continue;
        }
        if level == 0 {
            kor_frame::free_frames(pte_address(entry), 1);
        } else if !is_huge(entry) {
            unsafe {
                destroy_table(pte_address(entry), level - 1);
            }
        }
    }
    kor_frame::free_frames(table, 1);
}

/// Detach the low identity map from the high-half direct map.
/// Boot maps PML4[0] and PML4[256] through the same PDPT and PD pages;
/// user mappings must not mutate the shared high-half tables.
pub fn split_identity_map() {
    unsafe {
        let root = PhysAddr::new(root());
        let pml4 = unsafe { root.page_slice_mut::<usize>(PTE_NUM) };
        let low = pml4[0];
        if !is_present(low) || is_huge(low) {
            return;
        }

        let old_pdpt = PhysAddr::new(pte_address(low));
        let new_pdpt_addr = alloc_page().expect("x86: OOM cloning low PDPT");
        unsafe { PhysAddr::new(new_pdpt_addr).clear_page() };
        let old_entries = unsafe { old_pdpt.page_slice_mut::<usize>(PTE_NUM) };
        let new_entries = unsafe { PhysAddr::new(new_pdpt_addr).page_slice_mut::<usize>(PTE_NUM) };

        for (index, old_entry) in old_entries.iter().copied().enumerate() {
            if !is_present(old_entry) || is_huge(old_entry) {
                new_entries[index] = old_entry;
                continue;
            }

            let old_pd = PhysAddr::new(pte_address(old_entry));
            let new_pd_addr = alloc_page().expect("x86: OOM cloning low PD");
            unsafe { PhysAddr::new(new_pd_addr).clear_page() };
            let old_pd_entries = unsafe { old_pd.page_slice_mut::<usize>(PTE_NUM) };
            let new_pd_entries =
                unsafe { PhysAddr::new(new_pd_addr).page_slice_mut::<usize>(PTE_NUM) };
            new_pd_entries.copy_from_slice(old_pd_entries);
            new_entries[index] = new_pd_addr | (old_entry & 0xFFF);
        }

        pml4[0] = new_pdpt_addr | (low & 0xFFF);
        let cr3 = root.raw();
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
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
        let mut table = PhysAddr::new(root());
        for level in (1..=3).rev() {
            let idx = vaddr.pn_index(level);
            let pte = unsafe { table.page_slice_mut::<usize>(PTE_NUM) }[idx];
            if !is_present(pte) {
                return None;
            }
            if is_huge(pte) {
                let off = vaddr.page_offset(level);
                return Some(pte_address(pte) + off);
            }
            table = PhysAddr::new(pte_address(pte));
        }
        let pte = unsafe { table.page_slice_mut::<usize>(PTE_NUM) }[vaddr.pn_index(0)];
        if !is_present(pte) || is_huge(pte) {
            return None;
        }
        Some(pte_address(pte) + vaddr.page_offset(0))
    }
}

fn map_4k(vaddr: VirtAddr, paddr: PhysAddr, flags: MappingFlags) -> Result<(), MapError> {
    unsafe {
        let mut root = PhysAddr::new(root());
        let user = flags.contains(MappingFlags::U);
        let pdpt = walk_table(&mut root, vaddr, 3, user)?;
        let mut pdpt = pdpt;
        let pd = walk_table(&mut pdpt, vaddr, 2, user)?;

        let mut pd = pd;
        let pt = walk_table(&mut pd, vaddr, 1, user)?;
        let idx = vaddr.pn_index(0);
        let entry = &mut unsafe { pt.page_slice_mut::<usize>(PTE_NUM) }[idx];
        if is_present(*entry) && !flags.contains(MappingFlags::U) {
            return Err(MapError::BlockedByExistingMapping);
        }
        *entry = leaf_pte_4k(paddr.raw(), flags);
        flush_vaddr(vaddr);
    }
    Ok(())
}
pub fn unmap(vaddr: usize, size: MapSize) -> Result<usize, MapError> {
    let vaddr = VirtAddr::new(vaddr);
    unsafe {
        let mut table = PhysAddr::new(root());
        let target_level = if size == MapSize::Page2M { 1 } else { 0 };
        for level in (target_level + 1..=3).rev() {
            let entry = unsafe { table.page_slice_mut::<usize>(PTE_NUM) }[vaddr.pn_index(level)];
            if !is_present(entry) || is_huge(entry) {
                return Err(MapError::BlockedByExistingMapping);
            }
            table = PhysAddr::new(pte_address(entry));
        }
        let entry =
            &mut unsafe { table.page_slice_mut::<usize>(PTE_NUM) }[vaddr.pn_index(target_level)];
        let expected_leaf = if size == MapSize::Page2M {
            is_huge(*entry)
        } else {
            !is_huge(*entry)
        };
        if !is_present(*entry) || !expected_leaf {
            return Err(MapError::BlockedByExistingMapping);
        }
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
        let user = flags.contains(MappingFlags::U);
        let pdpt = walk_table(&mut root, vaddr, 3, user)?;
        let mut pdpt = pdpt;
        let pd = walk_table(&mut pdpt, vaddr, 2, user)?;
        let idx = vaddr.pn_index(1);
        let entry = &mut unsafe { pd.page_slice_mut::<usize>(PTE_NUM) }[idx];
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
    user: bool,
) -> Result<PhysAddr, MapError> {
    let idx = vaddr.pn_index(level);
    let entry = &mut unsafe { parent.page_slice_mut::<usize>(PTE_NUM) }[idx];
    if is_present(*entry) && is_huge(*entry) {
        let block_paddr = pte_address(*entry);
        let inherited_flags = *entry & (0xFFF | (1usize << 63)) & !PTE_PS;
        let new_table = alloc_page().ok_or(MapError::OutOfMemory)?;
        unsafe { PhysAddr::new(new_table).clear_page() };
        let table = unsafe { PhysAddr::new(new_table).page_slice_mut::<usize>(PTE_NUM) };
        let child_size = 1usize << (12 + 9 * (level - 1));
        for (i, child) in table.iter_mut().enumerate() {
            let huge = if level > 1 { PTE_PS } else { 0 };
            *child = block_paddr + i * child_size | inherited_flags | huge;
        }
        *entry = table_pte(new_table);
    }
    if user {
        *entry |= PTE_US;
    }
    if !is_present(*entry) {
        let frame = alloc_page().ok_or(MapError::OutOfMemory)?;
        unsafe { PhysAddr::new(frame).clear_page() };
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
