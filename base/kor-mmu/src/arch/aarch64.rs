//! AArch64 runtime page-table operations (4 KiB pages + 2 MiB blocks).

use kor::addr::PhysAddr;
use kor::frame::alloc_frame;
use crate::AddrSpace;
use kor::mapping::MapError;
use kor::mapping::MappingFlags;

/// Kernel direct-map offset (matches the boot table built by the trampoline).
const KERNEL_OFFSET: usize = 0xFFFF_0000_0000_0000;

/// Zero one page at `pa` through the kernel direct map.
///
/// # Safety
/// `pa` must be a page-aligned frame available for exclusive write.
unsafe fn zero_page(pa: usize) {
    let va = phys_to_virt(pa) as *mut u64;
    // SAFETY: caller guarantees exclusive access to the frame.
    unsafe { core::ptr::write_bytes(va, 0, 512) };
}

/// Physical -> kernel direct-map virtual address.
fn phys_to_virt(pa: usize) -> usize {
    pa + KERNEL_OFFSET
}


/// 512 GiB slot — L0 index 1 is unused by the boot 1 GiB block map at L0[0].

pub(crate) const PTE_VALID: usize = 1 << 0;

/// Entries per page-table page (512 × 64-bit PTEs).
const PTE_NUM: usize = 512;
pub(crate) const PTE_TABLE: usize = 1 << 1;

/// Build the default boot tables: L0[0] -> L1, L1[i] = 1 GiB identity
/// block (covers the first 512 GiB).  TTBR0 and TTBR1 share the root, so
/// the same physical layout is reachable both at the low window and at
/// `0xFFFF_0000_0000_0000` (the returned offset).
///
/// Pure and static-free on purpose: it executes at a physical PC with the
/// MMU off, called from the kor-arch boot trampoline.  `pages` is the
/// two-page buffer (L0 then L1).
pub fn boot_table_identity_512g(pages: &mut [usize; PTE_NUM * 2]) -> usize {
    let l1_pa = pages.as_ptr() as usize + PTE_NUM * 8;
    let (l0, l1) = pages.split_at_mut(PTE_NUM);
    // Table descriptor: VALID | TABLE, pointing at the second page.
    l0[0] = l1_pa | PTE_VALID | PTE_TABLE;
    for gib in 0..PTE_NUM {
        l1[gib] = block_pte(gib << 30, MappingFlags::KERNEL_RWX);
    }
    0xFFFF_0000_0000_0000
}

/// Boot-trampoline hook (called with the MMU off at a physical PC): fill
/// the two-page buffer with the default mapping and report the offset.
///
/// # Safety
/// Must stay pure and static-free: paging is off and only the
/// trampoline's physical stack exists.  Applications wanting a different
/// boot mapping write their own trampoline (raw `global_asm!`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_mmu_build(table_pa: usize, _table_len: usize) -> usize {
    // SAFETY: the trampoline passes the physical address of its aligned
    // BSS buffer, owned for the duration of the call.
    let pages = unsafe { &mut *(table_pa as *mut [usize; PTE_NUM * 2]) };
    boot_table_identity_512g(pages)
}
const PTE_AF: usize = 1 << 10;
const PTE_ATTR: usize = 1 << 2; // MAIR index 1 (Normal WB), matches boot.S

pub fn init() {}

/// Read TTBR0_EL1 on the current CPU instead of caching another CPU's root.
pub fn root() -> usize {
    let ttbr: u64;
    unsafe {
        core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr);
    }
    (ttbr as usize) & 0xFFFF_FFFF_F000
}

/// Activate a TTBR0_EL1 page-table root.
///
/// # Safety
/// `root` must be a valid, page-aligned root containing live kernel mappings.
pub unsafe fn activate(root: usize) {
    unsafe {
        core::arch::asm!(
            "msr ttbr0_el1, {0}",
            "isb",
            "tlbi vmalle1is",
            "dsb sy",
            "isb",
            in(reg) root,
        );
    }
}
pub fn clone_active() -> Result<usize, MapError> {
    unsafe { clone_table(root(), 3) }
}

pub fn new_user() -> Result<usize, MapError> {
    let source = root();
    let cloned = alloc_frame().ok_or(MapError::OutOfMemory)?;
    unsafe { zero_page(cloned) };
    let src = unsafe { PhysAddr::new(source).page_slice_at_mut::<usize>(phys_to_virt(source), PTE_NUM) };
    let dst = unsafe { PhysAddr::new(cloned).page_slice_at_mut::<usize>(phys_to_virt(cloned), PTE_NUM) };
    dst[256..].copy_from_slice(&src[256..]);
    Ok(cloned)
}

unsafe fn clone_table(source: usize, level: usize) -> Result<usize, MapError> {
    let cloned = alloc_frame().ok_or(MapError::OutOfMemory)?;
    unsafe { zero_page(cloned) };
    let src = unsafe { PhysAddr::new(source).page_slice_at_mut::<usize>(phys_to_virt(source), PTE_NUM) };
    let dst = unsafe { PhysAddr::new(cloned).page_slice_at_mut::<usize>(phys_to_virt(cloned), PTE_NUM) };
    for index in 0..PTE_NUM {
        let entry = src[index];
        if !is_valid(entry) || (level == 3 && index >= 256) {
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
                    phys_to_virt(pte_address(entry)) as *const u8,
                    phys_to_virt(page) as *mut u8,
                    4096,
                );
            }
            dst[index] = page | (entry & !0x0000_FFFF_FFFF_F000);
        } else if is_block(entry) {
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
            dst[index] = child | (entry & !0x0000_FFFF_FFFF_F000);
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
    let entries = unsafe { PhysAddr::new(table).page_slice_at_mut::<usize>(phys_to_virt(table), PTE_NUM) };
    for &entry in &entries[..256] {
        if !is_valid(entry) {
            continue;
        }
        if level == 0 {
            kor::frame::free_frames(pte_address(entry), 1);
        } else if !is_block(entry) {
            unsafe {
                destroy_table(pte_address(entry), level - 1);
            }
        }
    }
    kor::frame::free_frames(table, 1);
}

/// Split L1[0] (first 1 GiB identity-mapped block) into an L2 table
/// with 512 × 2 MiB blocks, so user pages can be mapped in low address space.
pub fn split_identity() {
    unsafe {
        let root = PhysAddr::new(root());
        let l0_table = root.page_slice_at_mut::<usize>(phys_to_virt(root.raw()), PTE_NUM);
        if !is_table(l0_table[0]) {
            return; // already split or not a table
        }
        let l1_pa = pte_address(l0_table[0]);
        let l1_table = PhysAddr::new(l1_pa).page_slice_at_mut::<usize>(phys_to_virt(l1_pa), PTE_NUM);
        // Check if L1[0] is a 1 GiB block (needs splitting)
        if !is_block(l1_table[0]) {
            return; // already split
        }
        // Allocate L2 table
        let l2_paddr = alloc_frame().expect("OOM: L2 table");
        zero_page(l2_paddr);
        let l2_table = PhysAddr::new(l2_paddr).page_slice_at_mut::<usize>(phys_to_virt(l2_paddr), PTE_NUM);
        // Fill L2 with 512 × 2 MiB identity-mapped blocks (no U bit)
        for i in 0..512 {
            let pa_2m = i * (2 * 1024 * 1024);
            l2_table[i] = pa_2m | PTE_VALID | PTE_AF | PTE_ATTR; // block, no U
        }
        // Replace L1[0] with table descriptor pointing to L2
        l1_table[0] = l2_paddr | PTE_VALID | PTE_TABLE;
        // Flush TLB
        core::arch::asm!("tlbi vmalle1is", "dsb sy", "isb");
    }
}

pub(crate) fn page_pte(paddr: usize, flags: MappingFlags) -> usize {
    paddr | PTE_VALID | PTE_TABLE | PTE_AF | PTE_ATTR | ap_flags(flags)
}

pub(crate) fn block_pte(paddr: usize, flags: MappingFlags) -> usize {
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

pub(crate) fn is_valid(pte: usize) -> bool {
    pte & PTE_VALID != 0
}

pub(crate) fn is_table(pte: usize) -> bool {
    is_valid(pte) && pte & PTE_TABLE != 0
}

pub(crate) fn is_block(pte: usize) -> bool {
    is_valid(pte) && pte & PTE_TABLE == 0
}

pub(crate) fn pte_address(pte: usize) -> usize {
    pte & 0xFFFF_FFFF_F000
}



/// PTE encoding contract for the generic engine (see `page_table`).
impl AddrSpace {
    /// Physical -> kernel direct-map virtual address (inherent; the generic
    /// engine calls this for table-page access).
    pub fn phys_to_virt(pa: usize) -> usize {
        phys_to_virt(pa)
    }

    /// Zero one page at `pa` through the direct map.
    ///
    /// # Safety
    /// `pa` must be a page-aligned frame available for exclusive write.
    pub unsafe fn zero_page(pa: usize) {
        unsafe { zero_page(pa) }
    }


    /// Number of radix levels (Sv39-style: four).
    pub const LEVELS: usize = 4;

    /// This architecture drives real page tables.
    pub const PAGING_SUPPORTED: bool = true;

                pub fn table_entry( table_phys: usize) -> usize {
        table_phys | PTE_VALID | PTE_TABLE
    }

    pub fn leaf_entry( phys: usize, flags: MappingFlags, level: usize) -> usize {
        if level == 0 {
            page_pte(phys, flags)
        } else {
            block_pte(phys, flags)
        }
    }

    pub fn entry_present( entry: usize) -> bool {
        is_valid(entry)
    }

    pub fn entry_is_leaf( entry: usize, level: usize) -> bool {
        if level == 0 {
            // L3 valid entries are always 4 KiB page descriptors.
            is_valid(entry)
        } else {
            is_block(entry)
        }
    }

    pub fn entry_phys( entry: usize, _level: usize) -> usize {
        pte_address(entry)
    }

    pub fn entry_flags( entry: usize) -> MappingFlags {
        // Present mappings are readable; mirror of `ap_flags`.
        let mut flags = MappingFlags::R;
        // AP[2] (bit 7) set means read-only.
        if entry & (1 << 7) == 0 {
            flags = flags | MappingFlags::W;
        }
        // AP[1] (bit 6) set means EL0-accessible.
        if entry & (1 << 6) != 0 {
            flags = flags | MappingFlags::U;
        }
        // UXN (bit 54) clear means executable.
        if entry & (1 << 54) == 0 {
            flags = flags | MappingFlags::X;
        }
        flags
    }

    pub fn flush_tlb( va: Option<usize>) {
        // SAFETY: `tlbi vae1is` invalidates one VA (all ASIDs, inner
        // shareable); `tlbi vmalle1is` flushes everything.
        unsafe {
            match va {
                Some(address) => core::arch::asm!(
                    "tlbi vae1is, {}",
                    in(reg) address,
                ),
                None => core::arch::asm!("tlbi vmalle1is"),
            }
        }
    }
}
