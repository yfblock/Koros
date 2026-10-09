//! x86_64 runtime page-table operations (4-level, 4 KiB pages + 2 MiB huge pages).

use kor::addr::PhysAddr;
use kor::frame::alloc_frame;
use crate::MapError;
use crate::MappingFlags;
use x86_64::registers::control::Cr3;



/// First PML4 index after the boot high-half 512 GiB slot (index 256).

/// Zero one page at `pa` through the kernel mapping.
///
/// # Safety
/// `pa` must be a page-aligned frame available for exclusive write.
pub unsafe fn zero_page(pa: usize) {
    let va = kor::PhysAddr::new(pa).to_ka().raw() as *mut u64;
    // SAFETY: caller guarantees exclusive access to the frame.
    unsafe { core::ptr::write_bytes(va, 0, 512) };
}

use crate::pte::{PTE_P, PTE_PS, PTE_RW, PTE_US, hw_flags};
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
    let cloned = alloc_frame().ok_or(MapError::OutOfMemory)?;
    unsafe { zero_page(cloned) };
    let src = unsafe { PhysAddr::new(source).page_slice_at_mut::<usize>(PTE_NUM) };
    let dst = unsafe { PhysAddr::new(cloned).page_slice_at_mut::<usize>(PTE_NUM) };
    dst[256..].copy_from_slice(&src[256..]);
    Ok(cloned)
}

unsafe fn clone_table(source: usize, level: usize) -> Result<usize, MapError> {
    let cloned = alloc_frame().ok_or(MapError::OutOfMemory)?;
    unsafe { zero_page(cloned) };
    let src = unsafe { PhysAddr::new(source).page_slice_at_mut::<usize>(PTE_NUM) };
    let dst = unsafe { PhysAddr::new(cloned).page_slice_at_mut::<usize>(PTE_NUM) };
    for index in 0..PTE_NUM {
        let entry = src[index];
        if !is_present(entry) || (level == 3 && index >= 256) {
            dst[index] = entry;
        } else if level == 0 {
            let page = match alloc_frame() {
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
                    kor::PhysAddr::new(pte_address(entry)).to_ka().raw() as *const u8,
                    kor::PhysAddr::new(page).to_ka().raw() as *mut u8,
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
    let entries = unsafe { PhysAddr::new(table).page_slice_at_mut::<usize>(PTE_NUM) };
    for &entry in &entries[..256] {
        if !is_present(entry) {
            continue;
        }
        if level == 0 {
            kor::frame::free_frames(pte_address(entry), 1);
        } else if !is_huge(entry) {
            unsafe {
                destroy_table(pte_address(entry), level - 1);
            }
        }
    }
    kor::frame::free_frames(table, 1);
}

/// Detach the low identity map from the high-half direct map.
/// Boot maps PML4[0] and PML4[256] through the same PDPT and PD pages;
/// user mappings must not mutate the shared high-half tables.
pub fn split_identity() {
    unsafe {
        let root = PhysAddr::new(root());
        let pml4 = root.page_slice_at_mut::<usize>(PTE_NUM);
        let low = pml4[0];
        if !is_present(low) || is_huge(low) {
            return;
        }

        let old_pdpt = PhysAddr::new(pte_address(low));
        let new_pdpt_addr = alloc_frame().expect("x86: OOM cloning low PDPT");
        zero_page(new_pdpt_addr);
        let old_entries = old_pdpt.page_slice_at_mut::<usize>(PTE_NUM);
        let new_entries = PhysAddr::new(new_pdpt_addr).page_slice_at_mut::<usize>(PTE_NUM);

        for (index, old_entry) in old_entries.iter().copied().enumerate() {
            if !is_present(old_entry) || is_huge(old_entry) {
                new_entries[index] = old_entry;
                continue;
            }

            let old_pd = PhysAddr::new(pte_address(old_entry));
            let new_pd_addr = alloc_frame().expect("x86: OOM cloning low PD");
            zero_page(new_pd_addr);
            let old_pd_entries = old_pd.page_slice_at_mut::<usize>(PTE_NUM);
            let new_pd_entries = PhysAddr::new(new_pd_addr).page_slice_at_mut::<usize>(PTE_NUM);
            new_pd_entries.copy_from_slice(old_pd_entries);
            new_entries[index] = new_pd_addr | (old_entry & 0xFFF);
        }

        pml4[0] = new_pdpt_addr | (low & 0xFFF);
        let cr3 = root.raw();
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
    }
}

pub(crate) fn leaf_pte_4k(paddr: usize, flags: MappingFlags) -> usize {
    paddr | hw_flags(flags)
}

pub(crate) fn leaf_pte_2m(paddr: usize, flags: MappingFlags) -> usize {
    paddr | hw_flags(flags) | PTE_PS
}

pub(crate) fn table_pte(paddr: usize) -> usize {
    paddr | PTE_P | PTE_RW | PTE_US
}

pub(crate) fn is_present(pte: usize) -> bool {
    pte & PTE_P != 0
}

pub(crate) fn is_huge(pte: usize) -> bool {
    pte & PTE_PS != 0
}

pub(crate) fn pte_address(pte: usize) -> usize {
    pte & 0xFFFF_FFFF_F000
}



/// PTE encoding contract for the generic engine (see `page_table`).

/// Number of radix levels (Sv39-style: four).
pub const LEVELS: usize = 4;

/// This architecture drives real page tables.
pub const PAGING_SUPPORTED: bool = true;

            pub fn table_entry( table_phys: usize) -> usize {
    table_pte(table_phys)
}

pub fn leaf_entry( phys: usize, flags: MappingFlags, level: usize) -> usize {
    match level {
        0 => leaf_pte_4k(phys, flags),
        // 2 MiB and 1 GiB leaves set the PS bit in the entry.
        _ => leaf_pte_2m(phys, flags),
    }
}

pub fn entry_present( entry: usize) -> bool {
    is_present(entry)
}

pub fn entry_is_leaf( entry: usize, level: usize) -> bool {
    if level == 0 {
        // PT entries are 4 KiB leaves whenever present.
        is_present(entry)
    } else {
        is_huge(entry)
    }
}

pub fn entry_phys( entry: usize, _level: usize) -> usize {
    pte_address(entry)
}

pub fn entry_flags( entry: usize) -> MappingFlags {
    // Mirror of `hw_flags`: present implies readable (and executable,
    // since the encoders never set NX).
    let mut flags = MappingFlags::R | MappingFlags::X;
    if entry & (1 << 1) != 0 {
        flags = flags | MappingFlags::W;
    }
    if entry & (1 << 2) != 0 {
        flags = flags | MappingFlags::U;
    }
    flags
}

pub fn flush_tlb( va: Option<usize>) {
    // SAFETY: `invlpg` invalidates one page's translations; reloading
    // CR3 flushes non-global entries.
    unsafe {
        match va {
            Some(address) => core::arch::asm!(
                "invlpg [{}]",
                in(reg) address,
                options(nostack),
            ),
            None => {
                let cr3: usize;
                // SAFETY: reading CR3 is side-effect free.
                core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack));
                // SAFETY: writing CR3 back flushes non-global entries.
                core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
            }
        }
    }
}
