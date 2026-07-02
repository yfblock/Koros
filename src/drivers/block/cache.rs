//! LRU block cache.
//!
//! Caches recently-accessed blocks in memory and evicts the least-recently-used
//! entry when the cache reaches capacity. Dirty blocks are flushed to the
//! underlying device before eviction or on explicit [`BlockCache::sync`].

extern crate alloc;

use alloc::{sync::Arc, vec::Vec, collections::BTreeMap};
use super::{BlockDevice, BlockError};

// ---------------------------------------------------------------------------
// Cached block
// ---------------------------------------------------------------------------

/// A single cached block held in memory.
struct CachedBlock {
    /// Logical block number on the device.
    block_id: usize,
    /// Raw block data (length == `device.block_size()`).
    data: Vec<u8>,
    /// `true` when the data has been written through the cache but not yet
    /// flushed to the underlying device.
    dirty: bool,
}

// ---------------------------------------------------------------------------
// Block cache
// ---------------------------------------------------------------------------

/// A fixed-capacity, write-back block cache with LRU eviction.
///
/// Reads are served from the cache when possible; writes mark the cached block
/// as dirty.  Call [`sync`](BlockCache::sync) to flush dirty blocks to the
/// backing device.
pub struct BlockCache {
    /// The underlying storage device.
    device: Arc<dyn BlockDevice>,
    /// Map from block id → cached data.
    cache: BTreeMap<usize, CachedBlock>,
    /// Maximum number of blocks the cache may hold.
    max_blocks: usize,
    /// LRU ordering: least-recently-used at the front, most-recently-used at
    /// the back.
    access_order: Vec<usize>,
}

impl BlockCache {
    /// Create a new cache wrapping `device` with capacity for `max_blocks`.
    pub fn new(device: Arc<dyn BlockDevice>, max_blocks: usize) -> Self {
        Self {
            device,
            cache: BTreeMap::new(),
            max_blocks,
            access_order: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Read the block at `block_id` into `buf`.
    ///
    /// On a cache hit the data is copied directly from memory; on a miss the
    /// block is fetched from the device, inserted into the cache (possibly
    /// evicting the LRU entry), and then copied to `buf`.
    pub fn read_block_cached(
        &mut self,
        block_id: usize,
        buf: &mut [u8],
    ) -> Result<(), BlockError> {
        let bs = self.device.block_size();
        if buf.len() != bs {
            return Err(BlockError::InvalidBlock);
        }

        if let Some(entry) = self.cache.get_mut(&block_id) {
            // Cache hit — promote in LRU order and copy.
            buf.copy_from_slice(&entry.data);
            self.touch(block_id);
            return Ok(());
        }

        // Cache miss — fetch from device.
        self.device.read_block(block_id, buf)?;

        // Insert into cache (may evict).
        let data = Vec::from(buf);
        self.cache.insert(block_id, CachedBlock { block_id, data, dirty: false });
        self.access_order.push(block_id);
        self.evict_if_needed();

        Ok(())
    }

    /// Write `buf` to the block at `block_id` through the cache.
    ///
    /// The cached copy is updated and marked dirty.  The data is **not**
    /// written to the device until [`sync`](BlockCache::sync) is called (or the
    /// block is evicted).
    pub fn write_block_cached(
        &mut self,
        block_id: usize,
        buf: &[u8],
    ) -> Result<(), BlockError> {
        let bs = self.device.block_size();
        if buf.len() != bs {
            return Err(BlockError::InvalidBlock);
        }

        if let Some(entry) = self.cache.get_mut(&block_id) {
            // Cache hit — update in place.
            entry.data.copy_from_slice(buf);
            entry.dirty = true;
            self.touch(block_id);
            return Ok(());
        }

        // Cache miss — insert new dirty entry.
        let data = Vec::from(buf);
        self.cache.insert(block_id, CachedBlock { block_id, data, dirty: true });
        self.access_order.push(block_id);
        self.evict_if_needed();

        Ok(())
    }

    /// Flush every dirty block to the underlying device and clear the dirty
    /// flags.
    pub fn sync(&mut self) -> Result<(), BlockError> {
        for entry in self.cache.values_mut() {
            if entry.dirty {
                self.device.write_block(entry.block_id, &entry.data)?;
                entry.dirty = false;
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    /// Move `block_id` to the back (most-recently-used end) of `access_order`.
    fn touch(&mut self, block_id: usize) {
        if let Some(pos) = self.access_order.iter().position(|&id| id == block_id) {
            self.access_order.remove(pos);
        }
        self.access_order.push(block_id);
    }

    /// Evict the least-recently-used entry when the cache exceeds capacity.
    ///
    /// If the evicted block is dirty it is written back to the device first.
    fn evict_if_needed(&mut self) {
        while self.cache.len() > self.max_blocks {
            // The front of access_order is the LRU entry.
            let victim_id = match self.access_order.first().copied() {
                Some(id) => id,
                None => break,
            };

            // Remove from access_order.
            self.access_order.remove(0);

            // Remove from cache, flushing if dirty.
            if let Some(entry) = self.cache.remove(&victim_id)
                && entry.dirty
            {
                // Best-effort flush — ignore error during eviction to avoid
                // panicking in a non-recoverable path.  A real kernel would
                // log this.
                let _ = self.device.write_block(entry.block_id, &entry.data);
            }
        }
    }
}
