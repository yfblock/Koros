//! Block device abstraction.
//!
//! Provides the [`BlockDevice`] trait that all block-storage backends
//! (virtio-blk, NVMe, …) must implement, and a [`BlockError`] type for
//! fallible I/O operations.

pub mod cache;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors returned by block-device operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    /// Underlying I/O transfer failed.
    IoError,
    /// The requested block number is out of range.
    InvalidBlock,
    /// Attempted to write a read-only device.
    ReadOnly,
    /// Device-level error (initialisation failure, transport error, …).
    DeviceError,
}

// ---------------------------------------------------------------------------
// BlockDevice trait
// ---------------------------------------------------------------------------

/// A fixed-size, randomly-addressable block storage device.
///
/// All I/O is expressed in units of [`BlockDevice::block_size`] bytes.
/// Implementations must be `Send + Sync` so the device can be shared
/// across threads via `Arc<dyn BlockDevice>`.
pub trait BlockDevice: Send + Sync {
    /// Read the block at `block_id` into `buf`.
    ///
    /// `buf` must be exactly [`block_size`](BlockDevice::block_size) bytes.
    fn read_block(&self, block_id: usize, buf: &mut [u8]) -> Result<(), BlockError>;

    /// Write `buf` to the block at `block_id`.
    ///
    /// `buf` must be exactly [`block_size`](BlockDevice::block_size) bytes.
    fn write_block(&self, block_id: usize, buf: &[u8]) -> Result<(), BlockError>;

    /// Size of one block in bytes (typically 512).
    fn block_size(&self) -> usize;

    /// Total number of addressable blocks on this device.
    fn total_blocks(&self) -> usize;
}
