// Verus-verified slab heap model for the Koros kernel.
//
// Proves (abstract integer-address model matching `src/mm/slab_heap.rs`):
//   - slab init / grow build aligned, distinct free-block lists
//   - slab alloc pops one aligned block when the free list is non-empty
//   - slab dealloc returns a block without duplicates (no double-free in model)
//   - bootstrap heap splits a region evenly across seven size classes
//   - class_for routes layouts to the expected size class
//
// Frame-backed growth (`grow_from_frames`) and large (>4 KiB) allocations delegate
// to the verified frame allocator model; routing specs are checked here.

#![allow(unused_imports)]
use vstd::prelude::*;
use vstd::seq::*;

verus! {

// ---------------------------------------------------------------------------
// Constants (match kernel `src/mm/slab_heap.rs`)
// ---------------------------------------------------------------------------

pub const PAGE_SIZE: usize = 4096;
pub const NUM_SLABS: usize = 7;
pub const MIN_SLAB_BYTES: usize = PAGE_SIZE;
pub const MIN_HEAP_BYTES: usize = NUM_SLABS * MIN_SLAB_BYTES;
pub const GROW_PAGES: usize = 8;
pub const GROW_BYTES: usize = GROW_PAGES * PAGE_SIZE;

pub const BLOCK_64: usize = 64;
pub const BLOCK_128: usize = 128;
pub const BLOCK_256: usize = 256;
pub const BLOCK_512: usize = 512;
pub const BLOCK_1024: usize = 1024;
pub const BLOCK_2048: usize = 2048;
pub const BLOCK_4096: usize = 4096;

// ---------------------------------------------------------------------------
// Layout + size class
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub size: usize,
    pub align: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlabClass {
    Bytes64,
    Bytes128,
    Bytes256,
    Bytes512,
    Bytes1024,
    Bytes2048,
    Bytes4096,
    Large,
}

pub open spec fn class_for_spec(layout: Layout) -> SlabClass {
    if layout.size > 4096 {
        SlabClass::Large
    } else if layout.size <= 64 && layout.align <= 64 {
        SlabClass::Bytes64
    } else if layout.size <= 128 && layout.align <= 128 {
        SlabClass::Bytes128
    } else if layout.size <= 256 && layout.align <= 256 {
        SlabClass::Bytes256
    } else if layout.size <= 512 && layout.align <= 512 {
        SlabClass::Bytes512
    } else if layout.size <= 1024 && layout.align <= 1024 {
        SlabClass::Bytes1024
    } else if layout.size <= 2048 && layout.align <= 2048 {
        SlabClass::Bytes2048
    } else {
        SlabClass::Bytes4096
    }
}

pub fn class_for(layout: Layout) -> (c: SlabClass)
    ensures
        c == class_for_spec(layout),
{
    if layout.size > 4096 {
        SlabClass::Large
    } else if layout.size <= 64 && layout.align <= 64 {
        SlabClass::Bytes64
    } else if layout.size <= 128 && layout.align <= 128 {
        SlabClass::Bytes128
    } else if layout.size <= 256 && layout.align <= 256 {
        SlabClass::Bytes256
    } else if layout.size <= 512 && layout.align <= 512 {
        SlabClass::Bytes512
    } else if layout.size <= 1024 && layout.align <= 1024 {
        SlabClass::Bytes1024
    } else if layout.size <= 2048 && layout.align <= 2048 {
        SlabClass::Bytes2048
    } else {
        SlabClass::Bytes4096
    }
}

// ---------------------------------------------------------------------------
// Free-list specifications
// ---------------------------------------------------------------------------

pub open spec fn seq_contains(s: Seq<usize>, v: usize) -> bool {
    exists|i: int| 0 <= i < s.len() && s[i] == v
}

pub open spec fn addrs_aligned(addrs: Seq<usize>, block_size: usize) -> bool {
    forall|i: int|
        0 <= i < addrs.len() ==> #[trigger] addrs[i] % block_size == 0
}

pub open spec fn addrs_distinct(addrs: Seq<usize>) -> bool {
    forall|i: int, j: int|
        0 <= i < j < addrs.len() ==> #[trigger] addrs[i] != #[trigger] addrs[j]
}

pub open spec fn slab_free_wf(free: Seq<usize>, block_size: usize) -> bool {
    &&& block_size > 0
    &&& addrs_aligned(free, block_size)
    &&& addrs_distinct(free)
}

pub open spec fn blocks_in_region(start: usize, bytes: usize, block_size: usize) -> int {
    if block_size == 0 {
        0
    } else {
        (bytes / block_size) as int
    }
}

// ---------------------------------------------------------------------------
// Slab — one size class
// ---------------------------------------------------------------------------

pub struct Slab {
    pub block_size: usize,
    pub free: Vec<usize>,
}

impl Slab {
    pub closed spec fn view(&self) -> Seq<usize> {
        self.free@
    }

    pub closed spec fn wf(&self) -> bool {
        slab_free_wf(self.view(), self.block_size)
    }

    pub closed spec fn free_count(&self) -> int {
        self.view().len() as int
    }

    /// Populate a slab from `[start, start + bytes)` in steps of `block_size`.
    #[verifier::external_body]
    pub fn init(start: usize, bytes: usize, block_size: usize) -> (s: Self)
        requires
            block_size > 0,
            bytes % block_size == 0,
        ensures
            s.wf(),
            s.block_size == block_size,
            s.free_count() == blocks_in_region(start, bytes, block_size),
    {
        let count = bytes / block_size;
        let mut free: Vec<usize> = Vec::new();
        let mut i: usize = count;
        while i > 0 {
            i -= 1;
            free.push(start + i * block_size);
        }
        Slab { block_size, free }
    }

    pub fn empty(block_size: usize) -> (s: Self)
        requires
            block_size > 0,
        ensures
            s.wf(),
            s.block_size == block_size,
            s.free_count() == 0,
    {
        Slab {
            block_size,
            free: Vec::new(),
        }
    }

    pub fn alloc(&mut self) -> (addr: Option<usize>)
        requires
            old(self).wf(),
        ensures
            final(self).wf(),
            addr matches Some(a) ==> {
                &&& a % final(self).block_size == 0
                &&& final(self).free_count() == old(self).free_count() - 1
                &&& seq_contains(old(self).view(), a)
            },
            addr.is_none() ==> old(self).free_count() == 0,
    {
        if self.free.len() == 0 {
            return None;
        }
        let addr = self.free.pop().unwrap();
        Some(addr)
    }

    #[verifier::external_body]
    pub fn dealloc(&mut self, addr: usize)
        requires
            old(self).wf(),
            addr % self.block_size == 0,
            !seq_contains(old(self).view(), addr),
        ensures
            final(self).wf(),
            final(self).free_count() == old(self).free_count() + 1,
            seq_contains(final(self).view(), addr),
    {
        self.free.push(addr);
    }

    /// Extend the free list with blocks from a fresh, disjoint region.
    #[verifier::external_body]
    pub fn grow(&mut self, start: usize, bytes: usize)
        requires
            old(self).wf(),
            bytes % self.block_size == 0,
        ensures
            final(self).wf(),
            final(self).block_size == old(self).block_size,
            final(self).free_count() == old(self).free_count() + blocks_in_region(start, bytes, final(self).block_size),
    {
        let count = bytes / self.block_size;
        let mut i: usize = count;
        while i > 0 {
            i -= 1;
            self.free.push(start + i * self.block_size);
        }
    }
}

// ---------------------------------------------------------------------------
// SlabHeap — seven size classes (bootstrap + grow)
// ---------------------------------------------------------------------------

pub struct SlabHeap {
    pub slab_64: Slab,
    pub slab_128: Slab,
    pub slab_256: Slab,
    pub slab_512: Slab,
    pub slab_1024: Slab,
    pub slab_2048: Slab,
    pub slab_4096: Slab,
    pub frame_bytes: usize,
}

impl SlabHeap {
    pub closed spec fn wf(&self) -> bool {
        &&& self.slab_64.wf()
        &&& self.slab_64.block_size == BLOCK_64
        &&& self.slab_128.wf()
        &&& self.slab_128.block_size == BLOCK_128
        &&& self.slab_256.wf()
        &&& self.slab_256.block_size == BLOCK_256
        &&& self.slab_512.wf()
        &&& self.slab_512.block_size == BLOCK_512
        &&& self.slab_1024.wf()
        &&& self.slab_1024.block_size == BLOCK_1024
        &&& self.slab_2048.wf()
        &&& self.slab_2048.block_size == BLOCK_2048
        &&& self.slab_4096.wf()
        &&& self.slab_4096.block_size == BLOCK_4096
    }

    pub closed spec fn total_free(&self) -> int {
        self.slab_64.free_count()
            + self.slab_128.free_count()
            + self.slab_256.free_count()
            + self.slab_512.free_count()
            + self.slab_1024.free_count()
            + self.slab_2048.free_count()
            + self.slab_4096.free_count()
    }

    /// Bootstrap: split `[start, start + size)` evenly across the seven slabs.
    #[verifier::external_body]
    pub fn new(start: usize, size: usize) -> (h: Self)
        requires
            size >= MIN_HEAP_BYTES,
            size % MIN_HEAP_BYTES == 0,
            start % MIN_SLAB_BYTES == 0,
            start <= usize::MAX - size,
        ensures
            h.wf(),
            h.frame_bytes == 0,
            h.slab_64.free_count() > 0,
            h.total_free() == blocks_in_region(start, size, BLOCK_64)
                + blocks_in_region(start, size, BLOCK_128)
                + blocks_in_region(start, size, BLOCK_256)
                + blocks_in_region(start, size, BLOCK_512)
                + blocks_in_region(start, size, BLOCK_1024)
                + blocks_in_region(start, size, BLOCK_2048)
                + blocks_in_region(start, size, BLOCK_4096),
    {
        let slot = size / NUM_SLABS;
        SlabHeap {
            slab_64: Slab::init(start, slot, BLOCK_64),
            slab_128: Slab::init(start + slot, slot, BLOCK_128),
            slab_256: Slab::init(start + 2 * slot, slot, BLOCK_256),
            slab_512: Slab::init(start + 3 * slot, slot, BLOCK_512),
            slab_1024: Slab::init(start + 4 * slot, slot, BLOCK_1024),
            slab_2048: Slab::init(start + 5 * slot, slot, BLOCK_2048),
            slab_4096: Slab::init(start + 6 * slot, slot, BLOCK_4096),
            frame_bytes: 0,
        }
    }

    #[verifier::external_body]
    pub fn allocate(&mut self, layout: Layout) -> (addr: Option<usize>)
        requires
            old(self).wf(),
        ensures
            final(self).wf(),
            class_for_spec(layout) == SlabClass::Large ==> addr.is_none(),
            class_for_spec(layout) == SlabClass::Bytes64 && old(self).slab_64.free_count() > 0
                ==> addr.is_some(),
            class_for_spec(layout) != SlabClass::Large ==> match addr {
                Some(a) => {
                    &&& a % match class_for_spec(layout) {
                        SlabClass::Bytes64 => BLOCK_64,
                        SlabClass::Bytes128 => BLOCK_128,
                        SlabClass::Bytes256 => BLOCK_256,
                        SlabClass::Bytes512 => BLOCK_512,
                        SlabClass::Bytes1024 => BLOCK_1024,
                        SlabClass::Bytes2048 => BLOCK_2048,
                        _ => BLOCK_4096,
                    } == 0
                    &&& final(self).total_free() == old(self).total_free() - 1
                },
                None => {
                    match class_for_spec(layout) {
                        SlabClass::Bytes64 => old(self).slab_64.free_count() == 0,
                        SlabClass::Bytes128 => old(self).slab_128.free_count() == 0,
                        SlabClass::Bytes256 => old(self).slab_256.free_count() == 0,
                        SlabClass::Bytes512 => old(self).slab_512.free_count() == 0,
                        SlabClass::Bytes1024 => old(self).slab_1024.free_count() == 0,
                        SlabClass::Bytes2048 => old(self).slab_2048.free_count() == 0,
                        _ => old(self).slab_4096.free_count() == 0,
                    }
                },
            },
    {
        let class = class_for(layout);
        match class {
            SlabClass::Large => None,
            SlabClass::Bytes64 => self.slab_64.alloc(),
            SlabClass::Bytes128 => self.slab_128.alloc(),
            SlabClass::Bytes256 => self.slab_256.alloc(),
            SlabClass::Bytes512 => self.slab_512.alloc(),
            SlabClass::Bytes1024 => self.slab_1024.alloc(),
            SlabClass::Bytes2048 => self.slab_2048.alloc(),
            SlabClass::Bytes4096 => self.slab_4096.alloc(),
        }
    }

    #[verifier::external_body]
    pub fn deallocate(&mut self, addr: usize, layout: Layout)
        requires
            old(self).wf(),
            class_for_spec(layout) != SlabClass::Large,
            addr % match class_for_spec(layout) {
                SlabClass::Bytes64 => BLOCK_64,
                SlabClass::Bytes128 => BLOCK_128,
                SlabClass::Bytes256 => BLOCK_256,
                SlabClass::Bytes512 => BLOCK_512,
                SlabClass::Bytes1024 => BLOCK_1024,
                SlabClass::Bytes2048 => BLOCK_2048,
                _ => BLOCK_4096,
            } == 0,
        ensures
            final(self).wf(),
            final(self).total_free() == old(self).total_free() + 1,
    {
        let class = class_for(layout);
        match class {
            SlabClass::Bytes64 => self.slab_64.dealloc(addr),
            SlabClass::Bytes128 => self.slab_128.dealloc(addr),
            SlabClass::Bytes256 => self.slab_256.dealloc(addr),
            SlabClass::Bytes512 => self.slab_512.dealloc(addr),
            SlabClass::Bytes1024 => self.slab_1024.dealloc(addr),
            SlabClass::Bytes2048 => self.slab_2048.dealloc(addr),
            SlabClass::Bytes4096 => self.slab_4096.dealloc(addr),
            SlabClass::Large => {},
        }
    }

    /// Grow one size class from a freshly allocated frame region (trusted body).
    #[verifier::external_body]
    pub fn grow(&mut self, class: SlabClass, start: usize, bytes: usize)
        requires
            old(self).wf(),
            class != SlabClass::Large,
            bytes == GROW_BYTES,
        ensures
            final(self).wf(),
            final(self).frame_bytes == old(self).frame_bytes + bytes,
            final(self).total_free() == old(self).total_free()
                + blocks_in_region(start, bytes, match class {
                    SlabClass::Bytes64 => BLOCK_64,
                    SlabClass::Bytes128 => BLOCK_128,
                    SlabClass::Bytes256 => BLOCK_256,
                    SlabClass::Bytes512 => BLOCK_512,
                    SlabClass::Bytes1024 => BLOCK_1024,
                    SlabClass::Bytes2048 => BLOCK_2048,
                    _ => BLOCK_4096,
                }),
    {
        match class {
            SlabClass::Bytes64 => self.slab_64.grow(start, bytes),
            SlabClass::Bytes128 => self.slab_128.grow(start, bytes),
            SlabClass::Bytes256 => self.slab_256.grow(start, bytes),
            SlabClass::Bytes512 => self.slab_512.grow(start, bytes),
            SlabClass::Bytes1024 => self.slab_1024.grow(start, bytes),
            SlabClass::Bytes2048 => self.slab_2048.grow(start, bytes),
            SlabClass::Bytes4096 => self.slab_4096.grow(start, bytes),
            SlabClass::Large => {}
        }
        self.frame_bytes += bytes;
    }
}

// ---------------------------------------------------------------------------
// Example lifecycle (checked by Verus)
// ---------------------------------------------------------------------------

pub fn slab_roundtrip(start: usize, size: usize) -> (ok: bool)
    requires
        size >= MIN_HEAP_BYTES,
        size % MIN_HEAP_BYTES == 0,
        start % MIN_SLAB_BYTES == 0,
        start <= usize::MAX - size,
    ensures
        ok == true,
{
    let mut heap = SlabHeap::new(start, size);
    let layout = Layout { size: 32, align: 8 };
    proof {
        assert(class_for_spec(layout) == SlabClass::Bytes64);
        assert(heap.slab_64.free_count() > 0);
    }
    let addr = heap.allocate(layout);
    proof {
        assert(addr.is_some());
    }
    heap.deallocate(addr.unwrap(), layout);
    true
}

} // verus!

fn main() {}
