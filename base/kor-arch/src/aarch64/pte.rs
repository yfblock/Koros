//! AArch64 stage-1 PTE flag bits and the generic↔hardware flag encoders
//! (attribute indices match MAIR_EL1 as programmed by the boot fragments:
//! index 1 = Normal WB).

use kor::mapping::MappingFlags;

/// Valid/present.
pub const PTE_VALID: usize = 1 << 0;
/// Descriptor is a table pointer (not a block/page leaf).
pub const PTE_TABLE: usize = 1 << 1;
/// MAIR attribute index 1 (Normal WB).
pub const PTE_ATTR: usize = 1 << 2;
/// Accessed — software-set (QEMU/hardware may require it).
pub const PTE_AF: usize = 1 << 10;
/// AP[2]: read-only.
pub const PTE_AP2_RO: usize = 1 << 7;
/// AP[1]: EL0 access allowed.
pub const PTE_AP1_EL0: usize = 1 << 6;
/// UXN: unprivileged execute-never.
pub const PTE_UXN: usize = 1 << 54;

/// Encode generic mapping flags into AP/UXN permission bits.
pub fn ap_flags(flags: MappingFlags) -> usize {
    let mut extra = 0;
    if !flags.contains(MappingFlags::W) {
        extra |= PTE_AP2_RO;
    }
    if flags.contains(MappingFlags::U) {
        extra |= PTE_AP1_EL0;
    }
    if !flags.contains(MappingFlags::X) {
        extra |= PTE_UXN;
    }
    extra
}

/// Decode PTE permission bits back into generic mapping flags.
pub fn mapping_flags(entry: usize) -> MappingFlags {
    let mut flags = MappingFlags::R;
    if entry & PTE_AP2_RO == 0 {
        flags = flags | MappingFlags::W;
    }
    if entry & PTE_AP1_EL0 != 0 {
        flags = flags | MappingFlags::U;
    }
    if entry & PTE_UXN == 0 {
        flags = flags | MappingFlags::X;
    }
    flags
}
