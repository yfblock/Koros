//! Page-mapping types used by the `ArchProvider` page-table operations.

/// Mapping permissions for `map`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingFlags(u8);

impl MappingFlags {
    pub const R: Self = Self(1 << 0);
    pub const W: Self = Self(1 << 1);
    pub const X: Self = Self(1 << 2);
    pub const U: Self = Self(1 << 3);

    pub const KERNEL_RWX: Self = Self(Self::R.0 | Self::W.0 | Self::X.0);

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// Page size for a mapping operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapSize {
    Page4K,
    Page2M,
}

/// Errors from `map`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    OutOfMemory,
    BlockedByExistingMapping,
    Unsupported,
}
