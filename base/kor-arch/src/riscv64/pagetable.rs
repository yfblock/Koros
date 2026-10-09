//! RISC-V Sv39 runtime page-table operations.

use kor::addr::PhysAddr;
use kor::frame::alloc_frame;
use crate::MapError;
use crate::MappingFlags;



/// First unused 1 GiB slot in the high-half root (boot leaves use 256–259).

/// Zero one page at `pa` through the kernel mapping.
///
/// # Safety
/// `pa` must be a page-aligned frame available for exclusive write.
pub unsafe fn zero_page(pa: usize) {
    let va = kor::PhysAddr::new(pa).to_ka().raw() as *mut u64;
    // SAFETY: caller guarantees exclusive access to the frame.
    unsafe { core::ptr::write_bytes(va, 0, 512) };
}

use crate::pte::{
    PTE_A, PTE_D, PTE_R, PTE_RW, PTE_V, PTE_W, PTE_X, flags_to_hw, mapping_flags,
};

/// Entries per page-table page (512 × 64-bit PTEs).
const PTE_NUM: usize = 512;


/// Build the default boot root: 1 GiB leaves mapping the low 4 GiB both
/// identity (entries 0-3) and at the high-half window (entries 256-259).
/// Returns the phys→virt offset of that window.
///
/// Pure and static-free on purpose: it executes at a physical PC with the
/// MMU off, called from the kor-arch boot trampoline.
pub fn boot_table_identity_high(table: &mut [usize; PTE_NUM]) -> usize {
    for gib in 0..4usize {
        let pa = gib << 30;
        let entry = leaf_pte(pa, MappingFlags::KERNEL_RWX);
        table[gib] = entry;
        table[256 + gib] = entry;
    }
    0xFFFF_FFC0_0000_0000
}

/// Boot-trampoline hook (called with the MMU off at a physical PC): fill
/// the table buffer with the default mapping and report the offset.
///
/// # Safety
/// Must stay pure and static-free (no panics, no globals): paging is off
/// and only the trampoline's physical stack exists.  An application that
/// wants a different boot mapping writes its own trampoline (raw
/// `global_asm!`) instead of the kor-arch macro.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_mmu_build(table_pa: usize, _table_len: usize) -> usize {
    // SAFETY: the trampoline passes the physical address of its aligned
    // BSS buffer, owned for the duration of the call.
    let table = unsafe { &mut *(table_pa as *mut [usize; PTE_NUM]) };
    boot_table_identity_high(table)
}

/// Kernel (boot) root captured during init. Shutdown switches back to it
/// before touching the identity-mapped syscon, because user address spaces
/// only carry the high half.
static BOOT_ROOT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Install the runtime page-table services and capture the boot root.
pub fn init() {
    BOOT_ROOT.store(root(), core::sync::atomic::Ordering::Relaxed);
}

/// Reactivate the boot root (used on the shutdown path, which never returns).
///
/// # Safety
/// Callers must not resume normal execution afterwards on a CPU whose current
/// root differed — the previous root's mappings vanish until reactivated.
pub unsafe fn activate_boot_root() {
    let boot = BOOT_ROOT.load(core::sync::atomic::Ordering::Relaxed);
    if boot != 0 {
        unsafe { activate(boot) };
    }
}

/// Link-time hook mirroring [`boot_mmu_build`]: kor-arch's shutdown fallback
/// calls this to return to the boot root before the identity-mapped syscon
/// write. Declared `extern "C"` in kor-arch and resolved here at link time,
/// so neither crate depends on the other for it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_mmu_activate() {
    // SAFETY: the shutdown path never resumes on the previous root.
    unsafe { activate_boot_root() };
}

/// Read the root installed on this CPU. Address spaces are task state, so a
/// process running on another CPU must never observe a shared cached root.
pub fn root() -> usize {
    let satp: usize;
    unsafe {
        core::arch::asm!("csrr {}, satp", out(reg) satp);
    }
    (satp & ((1usize << 44) - 1)) << 12
}

/// Activate an Sv39 root page table.
///
/// # Safety
/// `root` must be a valid, page-aligned Sv39 root containing live kernel mappings.
pub unsafe fn activate(root: usize) {
    let satp = (8usize << 60) | (root >> 12);
    unsafe {
        core::arch::asm!("csrw satp, {}", "sfence.vma", in(reg) satp);
    }
}
pub fn clone_active() -> Result<usize, MapError> {
    let source = root();
    let cloned = unsafe { clone_table(source, 2)? };
    Ok(cloned)
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
        if !is_valid(entry) || (level == 2 && index >= 256) {
            dst[index] = entry;
        } else if is_leaf(entry) {
            if level == 0 {
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
                dst[index] = leaf_pte(page, mapping_flags(entry));
            } else {
                dst[index] = entry;
            }
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
            dst[index] = table_pte(child);
        }
    }
    Ok(cloned)
}

pub unsafe fn destroy(root: usize) {
    unsafe {
        destroy_table(root, 2);
    }
}

unsafe fn destroy_table(table: usize, level: usize) {
    let entries = unsafe { PhysAddr::new(table).page_slice_at_mut::<usize>(PTE_NUM) };
    for &entry in &entries[..256] {
        if !is_valid(entry) {
            continue;
        }
        if is_leaf(entry) {
            if level == 0 {
                kor::frame::free_frames(pte_address(entry), 1);
            }
        } else if level > 0 {
            unsafe {
                destroy_table(pte_address(entry), level - 1);
            }
        }
    }
    kor::frame::free_frames(table, 1);
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
pub fn split_identity() {
    // Sv39 boot maps the low half with 1 GiB L2 leaves; split entry 0.
    unsafe {
        let root = PhysAddr::new(root());
        let l2_table = root.page_slice_at_mut::<usize>(PTE_NUM);
        // Check if entry 0 is already split (non-leaf)
        if l2_table[0] != 0 && !is_leaf(l2_table[0]) {
            return; // Already split
        }

        // Allocate L1 table
        let l1_paddr = alloc_frame().expect("OOM: L1 table");
        zero_page(l1_paddr);
        let l1_table = PhysAddr::new(l1_paddr).page_slice_at_mut::<usize>(PTE_NUM);

        // Fill L1 entries 1-511 with 2 MiB identity-mapped leaves
        let leaf_flags = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D;
        for i in 1..512 {
            let pa_2m = i * (2 * 1024 * 1024);
            l1_table[i] = (pa_2m >> 2) | leaf_flags;
        }

        // Allocate L0 table for the first 2 MiB (L1 entry 0)
        let l0_paddr = alloc_frame().expect("OOM: L0 table");
        zero_page(l0_paddr);
        let l0_table = PhysAddr::new(l0_paddr).page_slice_at_mut::<usize>(PTE_NUM);

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

pub(crate) fn leaf_pte(paddr: usize, flags: MappingFlags) -> usize {
    (paddr >> 2) | flags_to_hw(flags)
}

/// Sv39 level-1 megapage: PTE[19:10] must be zero; PA[20:12] comes from VA[20:12].
pub(crate) fn megapage_2m_pte(paddr: usize, flags: MappingFlags) -> usize {
    ((paddr >> 2) & !0xFFC00) | flags_to_hw(flags)
}

pub(crate) fn table_pte(paddr: usize) -> usize {
    (paddr >> 2) | PTE_V
}

pub(crate) fn is_valid(pte: usize) -> bool {
    pte & PTE_V != 0
}

pub(crate) fn is_leaf(pte: usize) -> bool {
    is_valid(pte) && pte & PTE_RW != 0
}

pub(crate) fn pte_address(pte: usize) -> usize {
    (pte << 2) & 0xFFFF_FFFF_F000
}



/// PTE encoding contract for the generic engine (see `page_table`).

/// Number of radix levels (Sv39-style: three).
pub const LEVELS: usize = 3;

/// This architecture drives real page tables.
pub const PAGING_SUPPORTED: bool = true;

pub fn table_entry( table_phys: usize) -> usize {
    table_pte(table_phys)
}

pub fn leaf_entry( phys: usize, flags: MappingFlags, level: usize) -> usize {
    match level {
        0 => leaf_pte(phys, flags),
        1 => megapage_2m_pte(phys, flags),
        _ => leaf_pte(phys, flags),
    }
}

pub fn entry_present( entry: usize) -> bool {
    is_valid(entry)
}

pub fn entry_is_leaf( entry: usize, _level: usize) -> bool {
    // A PTE with any of R/W/X set is a leaf at every level; V with
    // R=W=X=0 points to the next-level table.
    is_leaf(entry)
}

pub fn entry_phys( entry: usize, _level: usize) -> usize {
    pte_address(entry)
}

pub fn entry_flags( entry: usize) -> MappingFlags {
    mapping_flags(entry)
}

pub fn flush_tlb( va: Option<usize>) {
    // SAFETY: `sfence.vma` is always executable in S-mode; with an
    // address it flushes that page's translations, else everything.
    unsafe {
        match va {
            Some(address) => core::arch::asm!(
                "sfence.vma {}",
                in(reg) address,
            ),
            None => core::arch::asm!("sfence.vma"),
        }
    }
}
