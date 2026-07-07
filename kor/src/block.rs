//! Block device abstraction — the `BlockDevice` trait and `BlockError`.
//!
//! The block-device *registry* (register/first) is owned by the composition
//! layer (`koros::registries`); this module holds only the trait.

/// Errors returned by block-device operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    /// Underlying I/O transfer failed.
    IoError,
    /// The requested block number is out of range.
    InvalidBlock,
    /// Attempted to write a read-only device.
    ReadOnly,
    /// Device-level error (initialisation failure, transport error, ...).
    DeviceError,
}

/// A fixed-size, randomly-addressable block storage device.
///
/// All I/O is expressed in units of [`BlockDevice::block_size`] bytes.
/// Implementations must be `Send + Sync` so the device can be shared
/// across threads via `Arc<dyn BlockDevice>`.
pub trait BlockDevice: Send + Sync {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) -> Result<(), BlockError>;
    fn write_block(&self, block_id: usize, buf: &[u8]) -> Result<(), BlockError>;
    fn block_size(&self) -> usize;
    fn total_blocks(&self) -> usize;

    fn read_blocks(&self, start_block: usize, buf: &mut [u8]) -> Result<(), BlockError> {
        let bs = self.block_size();
        for (i, chunk) in buf.chunks_mut(bs).enumerate() {
            self.read_block(start_block + i, chunk)?;
        }
        Ok(())
    }

    fn write_blocks(&self, start_block: usize, buf: &[u8]) -> Result<(), BlockError> {
        let bs = self.block_size();
        for (i, chunk) in buf.chunks(bs).enumerate() {
            self.write_block(start_block + i, chunk)?;
        }
        Ok(())
    }
}
