//! x86-64 long-mode PTE flag bits and the generic↔hardware flag encoders.

use kor::mapping::MappingFlags;

/// Present.
pub const PTE_P: usize = 1 << 0;
/// Read/write (clear = read-only).
pub const PTE_RW: usize = 1 << 1;
/// User/supervisor (clear = kernel-only).
pub const PTE_US: usize = 1 << 2;
/// Page size (huge-page leaf at this level).
pub const PTE_PS: usize = 1 << 7;

/// Encode generic mapping flags into x86-64 PTE flag bits (present implies
/// readable and executable — the encoders never set NX).
pub fn hw_flags(flags: MappingFlags) -> usize {
    let mut hw = PTE_P;
    if flags.contains(MappingFlags::W) {
        hw |= PTE_RW;
    }
    if flags.contains(MappingFlags::U) {
        hw |= PTE_US;
    }
    hw
}

/// Decode x86-64 PTE flag bits back into generic mapping flags (mirror of
/// [`hw_flags`]: present implies readable and executable).
pub fn mapping_flags(entry: usize) -> MappingFlags {
    let mut flags = MappingFlags::R | MappingFlags::X;
    if entry & PTE_RW != 0 {
        flags = flags | MappingFlags::W;
    }
    if entry & PTE_US != 0 {
        flags = flags | MappingFlags::U;
    }
    flags
}
