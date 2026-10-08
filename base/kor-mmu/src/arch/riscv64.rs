//! RISC-V Sv39 runtime page-table operations.

use kor::addr::PhysAddr;
use kor::frame::alloc_frame;
use kor::{MapError, MapSize, MappingFlags};

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

pub fn dynamic_maps_supported() -> bool {
    true
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
    unsafe { PhysAddr::new(cloned).clear_page() };
    let src = unsafe { PhysAddr::new(source).page_slice_mut::<usize>(PTE_NUM) };
    let dst = unsafe { PhysAddr::new(cloned).page_slice_mut::<usize>(PTE_NUM) };
    dst[256..].copy_from_slice(&src[256..]);
    Ok(cloned)
}

unsafe fn clone_table(source: usize, level: usize) -> Result<usize, MapError> {
    let cloned = alloc_frame().ok_or(MapError::OutOfMemory)?;
    unsafe { PhysAddr::new(cloned).clear_page() };
    let src = unsafe { PhysAddr::new(source).page_slice_mut::<usize>(PTE_NUM) };
    let dst = unsafe { PhysAddr::new(cloned).page_slice_mut::<usize>(PTE_NUM) };
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

pub(crate) fn mapping_flags(entry: usize) -> MappingFlags {
    let mut flags = MappingFlags::U;
    if entry & PTE_R != 0 {
        flags = flags | MappingFlags::R;
    }
    if entry & PTE_W != 0 {
        flags = flags | MappingFlags::W;
    }
    if entry & PTE_X != 0 {
        flags = flags | MappingFlags::X;
    }
    flags
}

pub unsafe fn destroy(root: usize) {
    unsafe {
        destroy_table(root, 2);
    }
}

unsafe fn destroy_table(table: usize, level: usize) {
    let entries = unsafe { PhysAddr::new(table).page_slice_mut::<usize>(PTE_NUM) };
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
        let l2_table = root.page_slice_mut::<usize>(PTE_NUM);
        // Check if entry 0 is already split (non-leaf)
        if l2_table[0] != 0 && !is_leaf(l2_table[0]) {
            return; // Already split
        }

        // Allocate L1 table
        let l1_paddr = alloc_frame().expect("OOM: L1 table");
        PhysAddr::new(l1_paddr).clear_page();
        let l1_table = PhysAddr::new(l1_paddr).page_slice_mut::<usize>(PTE_NUM);

        // Fill L1 entries 1-511 with 2 MiB identity-mapped leaves
        let leaf_flags = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D;
        for i in 1..512 {
            let pa_2m = i * (2 * 1024 * 1024);
            l1_table[i] = (pa_2m >> 2) | leaf_flags;
        }

        // Allocate L0 table for the first 2 MiB (L1 entry 0)
        let l0_paddr = alloc_frame().expect("OOM: L0 table");
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

/// Bytes covered by one mapping of `size`.
/// Size of one 2 MiB mapping.
const TWO_MIB: usize = 2 * 1024 * 1024;

fn map_size_bytes(size: MapSize) -> usize {
    match size {
        MapSize::Page4K => 4096,
        MapSize::Page2M => TWO_MIB,
    }
}

/// Map through the `kor-mmu` engine on the active root.
pub fn map(vaddr: usize, paddr: usize, flags: MappingFlags, size: MapSize) -> Result<(), MapError> {
    // SAFETY: the active root is live for the whole call; the transaction
    // serializes against other kor-mmu users.
    let space = unsafe { crate::AddrSpace::from_root(root()) };
    let mut txn = space.lock(vaddr..vaddr + map_size_bytes(size));
    txn.map(vaddr, paddr, flags, size)
}

/// Translate through the `kor-mmu` engine; the leaf granularity carries the
/// offset bits a megapage PTE does not hold (Sv39 PA[20:12] come from VA).
pub fn translate(vaddr: usize) -> Option<usize> {
    // SAFETY: read-only walk of the live root inside a transaction.
    let space = unsafe { crate::AddrSpace::from_root(root()) };
    let txn = space.lock(vaddr..vaddr + 4096);
    match txn.query(vaddr) {
        crate::PageStatus::Mapped {
            phys, granularity, ..
        } => Some(phys + vaddr % granularity),
        _ => None,
    }
}

/// Unmap through the `kor-mmu` engine; returns the mapped physical address.
pub fn unmap(vaddr: usize, size: MapSize) -> Result<usize, MapError> {
    let Some(paddr) = translate(vaddr) else {
        return Err(MapError::BlockedByExistingMapping);
    };
    // SAFETY: same contract as `map`.
    let space = unsafe { crate::AddrSpace::from_root(root()) };
    let mut txn = space.lock(vaddr..vaddr + map_size_bytes(size));
    txn.unmap(vaddr..vaddr + map_size_bytes(size))
        .map(|_| paddr)
}

pub(crate) fn flags_to_hw(flags: MappingFlags) -> usize {
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

// ---------------------------------------------------------------------------
// MmuFormat implementation (PTE encoding contract for the generic engine)
// ---------------------------------------------------------------------------

use crate::format::MmuFormat;

/// Sv39: 3 levels, 4 KiB / 2 MiB / 1 GiB leaves.
pub struct Sv39Format;

pub static FORMAT: Option<&'static dyn crate::format::MmuFormat> = Some(&Sv39Format);

/// Bits per page-table index level (9 for 512-entry tables).
const LEVEL_SHIFT: usize = 9;
const PAGE_SHIFT: usize = 12;

impl MmuFormat for Sv39Format {
    fn levels(&self) -> usize {
        3
    }

    fn table_index(&self, va: usize, level: usize) -> usize {
        (va >> (PAGE_SHIFT + LEVEL_SHIFT * level)) & ((1 << LEVEL_SHIFT) - 1)
    }

    fn page_offset(&self, va: usize, level: usize) -> usize {
        va & ((1usize << (PAGE_SHIFT + LEVEL_SHIFT * level)) - 1)
    }

    fn leaf_size(&self, level: usize) -> usize {
        1usize << (PAGE_SHIFT + LEVEL_SHIFT * level)
    }

    fn table_entry(&self, table_phys: usize) -> usize {
        table_pte(table_phys)
    }

    fn leaf_entry(&self, phys: usize, flags: MappingFlags, level: usize) -> usize {
        match level {
            0 => leaf_pte(phys, flags),
            1 => megapage_2m_pte(phys, flags),
            _ => leaf_pte(phys, flags),
        }
    }

    fn entry_present(&self, entry: usize) -> bool {
        is_valid(entry)
    }

    fn entry_is_leaf(&self, entry: usize, _level: usize) -> bool {
        // A PTE with any of R/W/X set is a leaf at every level; V with
        // R=W=X=0 points to the next-level table.
        is_leaf(entry)
    }

    fn entry_phys(&self, entry: usize, _level: usize) -> usize {
        pte_address(entry)
    }

    fn entry_flags(&self, entry: usize) -> MappingFlags {
        mapping_flags(entry)
    }

    fn flush_tlb(&self, va: Option<usize>) {
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
}
