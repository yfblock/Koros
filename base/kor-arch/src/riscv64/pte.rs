//! Sv39 PTE flag bits and the generic↔hardware flag encoders.

use kor::mapping::MappingFlags;

/// Valid/present.
pub const PTE_V: usize = 1 << 0;
/// Read permission.
pub const PTE_R: usize = 1 << 1;
/// Write permission.
pub const PTE_W: usize = 1 << 2;
/// Execute permission.
pub const PTE_X: usize = 1 << 3;
/// User-mode access (U-bit).
pub const PTE_U: usize = 1 << 4;
/// Accessed (set by hardware or eagerly by software).
pub const PTE_A: usize = 1 << 6;
/// Dirty (written).
pub const PTE_D: usize = 1 << 7;
/// Any of R/W/X set — the PTE is a leaf at every level.
pub const PTE_RW: usize = PTE_R | PTE_W | PTE_X;

/// Encode generic mapping flags into Sv39 PTE flag bits
/// (V | A | D eagerly; W implies D, R implies A).
pub fn flags_to_hw(flags: MappingFlags) -> usize {
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
        hw |= PTE_U;
    }
    hw
}

/// Decode Sv39 PTE flag bits back into generic mapping flags.
pub fn mapping_flags(entry: usize) -> MappingFlags {
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
