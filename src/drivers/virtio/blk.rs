//! Virtio block-device driver.
//!
//! Implements the [`BlockDevice`] trait over a virtio-blk transport
//! (MMIO or PCI) using a single request virtqueue with polling completion.
//!
//! References:
//! - virtio spec Section 5.2 (Block Device)

use alloc::boxed::Box;
use core::ptr;
use core::sync::atomic::{fence, Ordering};

use super::VirtioTransport;
use crate::drivers::block::{BlockDevice, BlockError};
use crate::mm;

// ---------------------------------------------------------------------------
// Cache maintenance for DMA (aarch64)
// ---------------------------------------------------------------------------

/// Clean (flush) the data cache for the given virtual address range.
///
/// On aarch64 this ensures CPU writes are visible to DMA devices by
/// executing `dc cvac` on each cache line followed by a `dsb sy`.
/// On other architectures this is a no-op.
#[inline]
fn cache_clean_range(va: usize, len: usize) {
    #[cfg(target_arch = "aarch64")]
    {
        const CACHE_LINE: usize = 64;
        let start = va & !(CACHE_LINE - 1);
        let end = (va + len + CACHE_LINE - 1) & !(CACHE_LINE - 1);
        let mut addr = start;
        while addr < end {
            unsafe {
                core::arch::asm!("dc cvac, {}", in(reg) addr);
            }
            addr += CACHE_LINE;
        }
        unsafe {
            core::arch::asm!("dsb sy");
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (va, len);
    }
}

/// Invalidate the data cache for the given virtual address range.
///
/// On aarch64 this ensures CPU reads see data written by DMA devices by
/// executing `dc civac` (clean-and-invalidate) on each cache line followed
/// by a `dsb sy`.  Clean-and-invalidate is used instead of plain invalidate
/// to safely handle partial cache lines that may contain dirty CPU data.
/// On other architectures this is a no-op.
#[inline]
fn cache_invalidate_range(va: usize, len: usize) {
    #[cfg(target_arch = "aarch64")]
    {
        const CACHE_LINE: usize = 64;
        let start = va & !(CACHE_LINE - 1);
        let end = (va + len + CACHE_LINE - 1) & !(CACHE_LINE - 1);
        let mut addr = start;
        while addr < end {
            unsafe {
                core::arch::asm!("dc civac, {}", in(reg) addr);
            }
            addr += CACHE_LINE;
        }
        unsafe {
            core::arch::asm!("dsb sy");
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (va, len);
    }
}

// ---------------------------------------------------------------------------
// Virtio-blk request types (virtio spec 5.2.3)
// ---------------------------------------------------------------------------

/// Read request.
const VIRTIO_BLK_T_IN: u32 = 0;
/// Write request.
const VIRTIO_BLK_T_OUT: u32 = 1;

// ---------------------------------------------------------------------------
// Virtio-blk status bytes (virtio spec 5.2.4)
// ---------------------------------------------------------------------------

const VIRTIO_BLK_S_OK: u8 = 0;

// ---------------------------------------------------------------------------
// Virtio-blk feature bits (virtio spec 5.2.1)
// ---------------------------------------------------------------------------

/// Maximum segment size hint from device.
#[allow(dead_code)]
const VIRTIO_BLK_F_SIZE_MAX: u64 = 1 << 1;
/// Maximum number of segments hint from device.
#[allow(dead_code)]
const VIRTIO_BLK_F_SEG_MAX: u64 = 1 << 2;
/// Device reports disk geometry.
#[allow(dead_code)]
const VIRTIO_BLK_F_GEOMETRY: u64 = 1 << 4;
/// Device is read-only.
const VIRTIO_BLK_F_RO: u64 = 1 << 5;
/// Device reports optimal block size.
const VIRTIO_BLK_F_BLK_SIZE: u64 = 1 << 6;
/// Device supports cache flush command.
#[allow(dead_code)]
const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;
/// Device reports topology info.
#[allow(dead_code)]
const VIRTIO_BLK_F_TOPOLOGY: u64 = 1 << 10;
/// Device supports write-cache configuration.
#[allow(dead_code)]
const VIRTIO_BLK_F_CONFIG_WCE: u64 = 1 << 11;

// ---------------------------------------------------------------------------
// Generic virtio feature bits (device-independent)
// ---------------------------------------------------------------------------

/// VIRTIO_F_VERSION_1 — modern (non-legacy) virtio.
const VIRTIO_F_VERSION_1: u64 = 1 << 32;

// ---------------------------------------------------------------------------
// Virtio status bits (virtio spec 2.1)
// ---------------------------------------------------------------------------

const STATUS_ACKNOWLEDGE: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_FEATURES_OK: u32 = 8;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FAILED: u32 = 128;

// ---------------------------------------------------------------------------
// Device config offsets (virtio spec 5.2.4)
// ---------------------------------------------------------------------------

/// Capacity low 32 bits (in 512-byte sectors).
const CONFIG_CAPACITY_LOW: usize = 0;
/// Capacity high 32 bits.
const CONFIG_CAPACITY_HIGH: usize = 4;
/// Block size in bytes (when VIRTIO_BLK_F_BLK_SIZE is negotiated).
const CONFIG_BLK_SIZE: usize = 20;

// ---------------------------------------------------------------------------
// Virtio-blk request header (virtio spec 5.2.3)
// ---------------------------------------------------------------------------

/// On-wire request header, 16 bytes, naturally aligned.
#[repr(C)]
struct VirtioBlkReq {
    type_: u32,
    reserved: u32,
    sector: u64,
}

// ---------------------------------------------------------------------------
// Virtqueue constants
// ---------------------------------------------------------------------------

/// Number of descriptors in the request queue.
const QUEUE_SIZE: u16 = 256;
/// End-of-chain sentinel for `next` field.
const DESC_NEXT_NONE: u16 = 0xFFFF;
/// Descriptor flag: this descriptor continues the chain.
const DESC_F_NEXT: u16 = 1;
/// Descriptor flag: device writes (buffer is device-writable).
const DESC_F_WRITE: u16 = 2;

// ---------------------------------------------------------------------------
// Virtqueue on-wire structures (virtio spec 2.6)
// ---------------------------------------------------------------------------

/// One descriptor in the descriptor table.
#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

/// Available ring header.
#[repr(C)]
struct VirtqAvail {
    flags: u16,
    idx: u16,
    // ring follows in memory; accessed via pointer arithmetic.
}

/// One element in the used ring.
#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqUsedElem {
    id: u32,
    len: u32,
}

/// Used ring header.
#[repr(C)]
struct VirtqUsed {
    flags: u16,
    idx: u16,
    // ring follows in memory; accessed via pointer arithmetic.
}

// ---------------------------------------------------------------------------
// VirtQueue — manages one split virtqueue in contiguous allocated pages.
// ---------------------------------------------------------------------------

struct VirtQueue {
    /// Virtual address of the contiguous allocation holding all structures.
    #[allow(dead_code)]
    base_va: usize,
    /// Physical address of the same allocation (for device DMA).
    base_pa: usize,
    /// Pointer to descriptor table.
    desc: *mut VirtqDesc,
    /// Pointer to available ring.
    avail: *mut VirtqAvail,
    /// Pointer to the `ring` array inside the available ring.
    avail_ring: *mut u16,
    /// Pointer to used ring.
    used: *mut VirtqUsed,
    /// Pointer to the `ring` array inside the used ring.
    used_ring: *mut VirtqUsedElem,
    /// Stack of free descriptor indices.
    free_list: [u16; QUEUE_SIZE as usize],
    /// Number of free descriptors.
    free_count: usize,
    /// Next index to poll in the used ring.
    last_used_idx: u16,
    /// Queue size.
    size: u16,
}

// SAFETY: VirtQueue only contains raw pointers into DMA-coherent memory
// and plain data.  All mutable access is serialised by the enclosing
// spin::Mutex in VirtioBlk, so the type is safe to send/share.
unsafe impl Send for VirtQueue {}
unsafe impl Sync for VirtQueue {}

impl VirtQueue {
    /// Layout constants (byte offsets within the contiguous allocation).
    ///   [0 .. 4096)                   — descriptor table (256 × 16 B)
    ///   [4096 .. 4096 + avail_sz)     — available ring
    ///   [4096 + avail_sz .. total)    — used ring (8-byte aligned)
    const DESC_SIZE: usize = QUEUE_SIZE as usize * core::mem::size_of::<VirtqDesc>();

    /// Allocate contiguous pages and initialise all ring structures.
    ///
    /// Returns `None` if physical memory allocation fails.
    fn new() -> Option<Self> {
        let avail_size = core::mem::size_of::<VirtqAvail>()
            + QUEUE_SIZE as usize * core::mem::size_of::<u16>();
        let used_size = core::mem::size_of::<VirtqUsed>()
            + QUEUE_SIZE as usize * core::mem::size_of::<VirtqUsedElem>();

        // Page-align the used ring for legacy virtio-mmio compatibility.
        let used_off = (Self::DESC_SIZE + avail_size + mm::PAGE_SIZE - 1)
            & !(mm::PAGE_SIZE - 1);
        let total = used_off + used_size;
        let num_pages = total.div_ceil(mm::PAGE_SIZE);

        let base_pa = mm::alloc_frames(num_pages)?;
        let base_va = mm::phys_to_virt(base_pa);

        // Zero the entire allocation.
        unsafe {
            ptr::write_bytes(base_va as *mut u8, 0, num_pages * mm::PAGE_SIZE);
        }

        // Descriptor table starts at offset 0.
        let desc = base_va as *mut VirtqDesc;

        // Available ring follows the descriptor table.
        let avail_off = Self::DESC_SIZE;
        let avail = (base_va + avail_off) as *mut VirtqAvail;
        let avail_ring = (base_va + avail_off + core::mem::size_of::<VirtqAvail>()) as *mut u16;

        // Used ring is page-aligned (see used_off calculation above).
        let used = (base_va + used_off) as *mut VirtqUsed;
        let used_ring =
            (base_va + used_off + core::mem::size_of::<VirtqUsed>()) as *mut VirtqUsedElem;

        // Build the free descriptor list.
        let mut free_list = [0u16; QUEUE_SIZE as usize];
        for (i, entry) in free_list.iter_mut().enumerate() {
            *entry = i as u16;
        }

        Some(Self {
            base_va,
            base_pa,
            desc,
            avail,
            avail_ring,
            used,
            used_ring,
            free_list,
            free_count: QUEUE_SIZE as usize,
            last_used_idx: 0,
            size: QUEUE_SIZE,
        })
    }

    /// Configure the transport's virtqueue registers to point at this queue.
    fn setup_transport(&self, transport: &mut dyn VirtioTransport) {
        transport.queue_select(0);
        transport.set_queue_size(QUEUE_SIZE);

        if transport.version() >= 2 {
            let desc_pa = self.base_pa;
            let avail_pa = desc_pa + Self::DESC_SIZE;
            let avail_size = core::mem::size_of::<VirtqAvail>()
                + QUEUE_SIZE as usize * core::mem::size_of::<u16>();
            let used_pa = (desc_pa + Self::DESC_SIZE + avail_size + mm::PAGE_SIZE - 1)
                & !(mm::PAGE_SIZE - 1);
            transport.queue_set_desc_addr(desc_pa as u64);
            transport.queue_set_avail_addr(avail_pa as u64);
            transport.queue_set_used_addr(used_pa as u64);
            transport.queue_enable();
        } else {
            transport.queue_set_pfn_legacy((self.base_pa / mm::PAGE_SIZE) as u32);
        }
    }

    /// Pop one descriptor index from the free list, or return `None`.
    fn alloc_desc(&mut self) -> Option<u16> {
        if self.free_count == 0 {
            return None;
        }
        self.free_count -= 1;
        Some(self.free_list[self.free_count])
    }

    /// Return a descriptor index to the free list.
    fn free_desc(&mut self, idx: u16) {
        self.free_list[self.free_count] = idx;
        self.free_count += 1;
    }

    /// Allocate a chain of `count` free descriptors.
    ///
    /// Returns the head index, or `None` if there are not enough free
    /// descriptors.  On success the descriptors are linked via `DESC_F_NEXT`
    /// and the last descriptor's `next` is `DESC_NEXT_NONE`.
    fn alloc_desc_chain(&mut self, count: usize) -> Option<u16> {
        if self.free_count < count {
            return None;
        }
        let mut indices = [0u16; 3]; // max 3 for virtio-blk
        for slot in indices.iter_mut().take(count) {
            *slot = self.alloc_desc()?;
        }
        for i in 0..count {
            let d = unsafe { &mut *self.desc.add(indices[i] as usize) };
            if i + 1 < count {
                d.flags = DESC_F_NEXT;
                d.next = indices[i + 1];
            } else {
                d.flags = 0;
                d.next = DESC_NEXT_NONE;
            }
        }
        Some(indices[0])
    }

    /// Free a chain of descriptors starting at `head`.
    fn free_desc_chain(&mut self, head: u16) {
        let mut idx = head;
        loop {
            let d = unsafe { &*self.desc.add(idx as usize) };
            let flags = d.flags;
            let next = d.next;
            self.free_desc(idx);
            if flags & DESC_F_NEXT == 0 {
                break;
            }
            idx = next;
        }
    }

    /// Fill descriptor `idx` with the given buffer.
    fn set_desc(&self, idx: u16, addr: u64, len: u32, flags: u16) {
        let d = unsafe { &mut *self.desc.add(idx as usize) };
        d.addr = addr;
        d.len = len;
        d.flags = flags;
    }

    /// Submit a descriptor chain to the available ring and notify the device.
    fn submit_and_notify(
        &mut self,
        head: u16,
        transport: &mut dyn VirtioTransport,
    ) {
        let idx = unsafe { (*self.avail).idx };
        let ring_slot = idx % self.size;
        unsafe {
            self.avail_ring.add(ring_slot as usize).write_volatile(head);
        }

        // Release fence: all descriptor/ring writes must be visible before
        // the device reads the updated avail index.
        fence(Ordering::Release);
        unsafe {
            (*self.avail).idx = idx.wrapping_add(1);
        }

        cache_clean_range(
            unsafe { self.avail_ring.add(ring_slot as usize) } as usize,
            core::mem::size_of::<u16>(),
        );
        cache_clean_range(
            unsafe { &(*self.avail).idx as *const u16 as usize },
            core::mem::size_of::<u16>(),
        );

        // Notify the device.
        transport.queue_notify(0);
    }

    /// Poll the used ring until descriptor `head` appears.
    ///
    /// Returns the number of bytes the device wrote, or `None` if the
    /// descriptor has not yet appeared.
    fn poll_used(&mut self, head: u16) -> Option<u32> {
        // Invalidate used index so we read the device's latest write.
        cache_invalidate_range(
            unsafe { &(*self.used).idx as *const u16 as usize },
            core::mem::size_of::<u16>(),
        );
        // Acquire fence: pair with the device's release on the used index.
        fence(Ordering::Acquire);
        let used_idx = unsafe { (*self.used).idx };
        while self.last_used_idx != used_idx {
            let slot = self.last_used_idx % self.size;
            // Invalidate used ring entry before reading (device wrote it).
            cache_invalidate_range(
                unsafe { self.used_ring.add(slot as usize) } as usize,
                core::mem::size_of::<VirtqUsedElem>(),
            );
            let elem = unsafe { *self.used_ring.add(slot as usize) };
            self.last_used_idx = self.last_used_idx.wrapping_add(1);
            if elem.id == u32::from(head) {
                return Some(elem.len);
            }
        }
        None
    }

    /// Busy-wait until descriptor `head` completes.
    fn wait_for(&mut self, head: u16) -> u32 {
        loop {
            if let Some(len) = self.poll_used(head) {
                return len;
            }
            core::hint::spin_loop();
        }
    }
}

// ---------------------------------------------------------------------------
// Inner state protected by the mutex
// ---------------------------------------------------------------------------

struct VirtioBlkInner {
    transport: Box<dyn VirtioTransport>,
    queue: VirtQueue,
}

// ---------------------------------------------------------------------------
// VirtioBlk — the block device driver
// ---------------------------------------------------------------------------

/// Virtio block device.
///
/// Owns a transport and a single request virtqueue.  All I/O is
/// synchronous (polling).  Interior mutability is provided by a
/// `spin::Mutex` so the `BlockDevice` trait can be implemented on `&self`.
pub struct VirtioBlk {
    inner: spin::Mutex<VirtioBlkInner>,
    /// Total number of 512-byte sectors.
    capacity: u64,
    /// Logical block size in bytes (typically 512).
    block_size: usize,
    /// `true` if the device reported `VIRTIO_BLK_F_RO`.
    read_only: bool,
}

impl VirtioBlk {
    /// Initialise a virtio-blk device over the given transport.
    ///
    /// Follows the standard virtio initialisation sequence:
    ///   1. Reset (status = 0)
    ///   2. ACKNOWLEDGE → DRIVER → FEATURES_OK → DRIVER_OK
    ///   3. Read device config (capacity, block size)
    ///   4. Set up request virtqueue
    pub fn new(mut transport: Box<dyn VirtioTransport>) -> Result<Self, BlockError> {
        transport.set_status(0);
        transport.add_status(STATUS_ACKNOWLEDGE);
        transport.add_status(STATUS_DRIVER);

        let ver = transport.version();
        transport.set_version(ver);

        let offered = transport.device_features();
        let read_only = offered & VIRTIO_BLK_F_RO != 0;
        let has_blk_size = offered & VIRTIO_BLK_F_BLK_SIZE != 0;

        let mut wanted = offered & (VIRTIO_BLK_F_RO | VIRTIO_BLK_F_BLK_SIZE);
        if ver >= 2 {
            wanted |= offered & VIRTIO_F_VERSION_1;
        }
        transport.set_driver_features(wanted);

        if ver >= 2 {
            transport.add_status(STATUS_FEATURES_OK);
            if transport.status() & STATUS_FEATURES_OK == 0 {
                transport.add_status(STATUS_FAILED);
                return Err(BlockError::DeviceError);
            }
        }

        let cap_lo = transport.read_device_config(CONFIG_CAPACITY_LOW);
        let cap_hi = transport.read_device_config(CONFIG_CAPACITY_HIGH);
        let capacity = (u64::from(cap_hi) << 32) | u64::from(cap_lo);

        let block_size = if has_blk_size {
            transport.read_device_config(CONFIG_BLK_SIZE) as usize
        } else {
            512
        };

        let queue = VirtQueue::new().ok_or(BlockError::DeviceError)?;
        queue.setup_transport(&mut *transport);

        crate::println!("virtio-blk: queue base_pa={:#x}", queue.base_pa);

        transport.add_status(STATUS_DRIVER_OK);

        Ok(Self {
            inner: spin::Mutex::new(VirtioBlkInner { transport, queue }),
            capacity,
            block_size,
            read_only,
        })
    }

    /// Perform a synchronous block I/O operation (read or write).
    ///
    /// Uses a 3-descriptor chain: header (ro) → data (ro/wo) → status (wo).
    fn do_block_io(
        &self,
        block_id: usize,
        buf: &mut [u8],
        is_write: bool,
    ) -> Result<(), BlockError> {
        if block_id >= self.total_blocks() {
            return Err(BlockError::InvalidBlock);
        }
        if buf.len() != self.block_size {
            return Err(BlockError::InvalidBlock);
        }

        let mut inner = self.inner.lock();
        let VirtioBlkInner {
            ref mut queue,
            ref mut transport,
        } = *inner;

        let mut req = VirtioBlkReq {
            type_: if is_write {
                VIRTIO_BLK_T_OUT
            } else {
                VIRTIO_BLK_T_IN
            },
            reserved: 0,
            sector: block_id as u64 * (self.block_size as u64 / 512),
        };
        let mut status: u8 = 0xFF;

        // Allocate a 3-descriptor chain: header → data → status.
        let head = queue.alloc_desc_chain(3).ok_or(BlockError::DeviceError)?;

        // Descriptor 0: request header (device-readable).
        let req_pa = mm::virt_to_phys(&mut req as *mut VirtioBlkReq as usize);
        let desc0 = head;
        queue.set_desc(
            desc0,
            req_pa as u64,
            core::mem::size_of::<VirtioBlkReq>() as u32,
            DESC_F_NEXT,
        );
        let next0 = unsafe { (*queue.desc.add(desc0 as usize)).next };

        // Descriptor 1: data buffer.
        let buf_pa = mm::virt_to_phys(buf.as_ptr() as usize);
        let desc1 = next0;
        let data_flags = if is_write { DESC_F_NEXT } else { DESC_F_NEXT | DESC_F_WRITE };
        queue.set_desc(desc1, buf_pa as u64, buf.len() as u32, data_flags);
        let next1 = unsafe { (*queue.desc.add(desc1 as usize)).next };

        // Descriptor 2: status byte (device-writable).
        let status_pa = mm::virt_to_phys(&mut status as *mut u8 as usize);
        queue.set_desc(next1, status_pa as u64, 1, DESC_F_WRITE);

        // Flush descriptors, request header, and (for writes) data buffer
        // to main memory so the device sees consistent data via DMA.
        for &idx in &[desc0, desc1, next1] {
            let desc_va = unsafe { queue.desc.add(idx as usize) } as usize;
            cache_clean_range(desc_va, core::mem::size_of::<VirtqDesc>());
        }
        cache_clean_range(&req as *const VirtioBlkReq as usize, core::mem::size_of::<VirtioBlkReq>());
        if is_write {
            cache_clean_range(buf.as_ptr() as usize, buf.len());
        }

        // Submit and wait.
        queue.submit_and_notify(head, &mut **transport);
        queue.wait_for(head);

        // Invalidate device-written buffers so the CPU reads fresh data.
        if !is_write {
            cache_invalidate_range(buf.as_ptr() as usize, buf.len());
        }
        cache_invalidate_range(&status as *const u8 as usize, 1);

        queue.free_desc_chain(head);

        if status != VIRTIO_BLK_S_OK {
            return Err(BlockError::IoError);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// BlockDevice trait implementation
// ---------------------------------------------------------------------------

impl BlockDevice for VirtioBlk {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) -> Result<(), BlockError> {
        self.do_block_io(block_id, buf, false)
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) -> Result<(), BlockError> {
        if self.read_only {
            return Err(BlockError::ReadOnly);
        }
        // SAFETY: The device reads from this buffer (DMA from device POV).
        // No actual mutation of the caller's data occurs.
        let buf_mut =
            unsafe { core::slice::from_raw_parts_mut(buf.as_ptr() as *mut u8, buf.len()) };
        self.do_block_io(block_id, buf_mut, true)
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn total_blocks(&self) -> usize {
        (self.capacity as usize) / (self.block_size / 512)
    }
}
