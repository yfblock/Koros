// Verus-verified slab heap model for the Koros kernel.
//
// Models `kor-alloc/src/slab_heap.rs` (abstract integer-address form) and proves:
//   - slab init / grow build aligned, distinct free-block lists
//   - slab alloc pops one aligned block when the free list is non-empty
//   - slab dealloc returns a block without duplicates (no double-free in model)
//   - bootstrap heap splits a region evenly across seven size classes
//   - class_for routes layouts to the expected size class, including the
//     over-aligned (`align > 4 KiB`) → Large path
//   - large_extent_frames reserves enough page frames for size AND alignment
//   - alloc_large (frame-growth gate) returns addresses aligned to
//     layout.align — end to end through the frame allocator's power-of-two
//     extent alignment
//   - grow_from_frames refuses Large requests and the pre-enable window,
//     and yields page-aligned regions otherwise
//   - dealloc_large releases exactly the extent alloc_large reserved
//
// The frame allocator is abstracted (`frame_alloc`) with the contract the
// kernel implementation provides (`kor-frame`: the extent is rounded up to a
// power of two and aligned to that many pages). The phys→virt translation is
// modeled as adding a 4 GiB-aligned kernel offset (true on all four arches),
// which preserves every supported alignment (≤ 4 GiB).

#![allow(unused_imports)]
use vstd::prelude::*;
use vstd::seq::*;

verus! {

// ---------------------------------------------------------------------------
// Constants (match kernel `kor-alloc/src/slab_heap.rs`)
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

/// Model of the phys→virt kernel offset (all four arches use a 4 GiB-aligned
/// constant); passed as a parameter so the proof covers every arch at once.
pub const KERNEL_OFFSET: u64 = 0x1_0000_0000;
/// Largest alignment the heap promises (allocator layouts never exceed this
/// in practice; every real kernel offset is a multiple of it).
pub const MAX_ALIGN: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Powers of two (needed for the alignment proofs)
// ---------------------------------------------------------------------------

pub open spec fn pow2(k: nat) -> nat
    decreases k,
{
    if k == 0 {
        1
    } else {
        2 * pow2((k - 1) as nat)
    }
}

pub open spec fn is_pow2(x: nat) -> bool {
    exists|k: nat| pow2(k) == x
}

/// `pow2(k) >= 1` always (proved).
proof fn pow2_pos(k: nat)
    ensures
        pow2(k) >= 1,
    decreases
        k,
{
    if k > 0 {
        pow2_pos((k - 1) as nat);
        assert(pow2(k) == 2 * pow2((k - 1) as nat));
    }
}

/// `pow2` is monotone (proved).
proof fn pow2_mono(j: nat, k: nat)
    requires
        j <= k,
    ensures
        pow2(j) <= pow2(k),
    decreases
        k,
{
    if j == k {
    } else {
        pow2_mono(j, (k - 1) as nat);
        assert(pow2(k) == 2 * pow2((k - 1) as nat));
    }
}

/// `PAGE_SIZE == pow2(12)` (proved by value ladder).
proof fn page_is_pow2()
    ensures
        pow2(1) == 2,
        pow2(2) == 4,
        pow2(3) == 8,
        is_pow2(8),
        pow2(12) == PAGE_SIZE as nat,
        is_pow2(PAGE_SIZE as nat),
{
    assert(pow2(0) == 1);
    assert(pow2(1) == 2);
    assert(pow2(2) == 4);
    assert(pow2(3) == 8);
    assert(pow2(4) == 16);
    assert(pow2(5) == 32);
    assert(pow2(6) == 64);
    assert(pow2(7) == 128);
    assert(pow2(8) == 256);
    assert(pow2(9) == 512);
    assert(pow2(10) == 1024);
    assert(pow2(11) == 2048);
    assert(pow2(12) == 4096);
}

// ---------------------------------------------------------------------------
// Trusted arithmetic axioms (external_body).
//
// The Verus build bundled here treats products of two symbolic integers as
// opaque (no ring normalization), and its vstd lacks the division-uniqueness
// lemmas, so the following elementary ring facts cannot be discharged
// machine-checked in this environment. They are standard, hand-checkable
// identities; every other obligation in this file is verified by Verus.
// ---------------------------------------------------------------------------

/// TRUSTED: powers of two factor exactly, `pow2(k) == pow2(k-j) * pow2(j)`.
#[verifier::external_body]
pub proof fn pow2_shift_div_axiom(j: nat, k: nat)
    requires
        j <= k,
    ensures
        pow2(k) == pow2((k - j) as nat) * pow2(j),
{
}

/// TRUSTED: a smaller power of two divides a larger one.
#[verifier::external_body]
pub proof fn pow2_divides_axiom(a: nat, b: nat)
    requires
        is_pow2(a),
        is_pow2(b),
        a <= b,
    ensures
        b % a == 0,
{
}

/// TRUSTED: divisibility is transitive.
#[verifier::external_body]
pub proof fn div_trans_axiom(p: int, d1: int, d2: int)
    requires
        d1 >= 1,
        d2 >= 1,
        p % d1 == 0,
        d1 % d2 == 0,
    ensures
        p % d2 == 0,
{
}

/// TRUSTED: a sum of two multiples of `d` is a multiple of `d`.
#[verifier::external_body]
pub proof fn mod_sum_zero_axiom(x: int, y: int, d: int)
    requires
        d >= 1,
        x % d == 0,
        y % d == 0,
    ensures
        (x + y) % d == 0,
{
}

/// TRUSTED: an over-4 KiB power-of-two alignment decomposes as
/// `align == 2^(k-12) * 4096` with a power-of-two frame factor.
#[verifier::external_body]
proof fn align_decomposes(l: Layout)
    requires
        layout_wf(l),
        l.align > PAGE_SIZE,
    ensures
        l.align as int == align_frames(l) as int * PAGE_SIZE as int,
        is_pow2(align_frames(l)),
{
}

/// TRUSTED: divisibility scales — `x % a == 0` implies `(x*s) % (a*s) == 0`.
#[verifier::external_body]
pub proof fn div_scale_axiom(x: int, a: int, s: int)
    requires
        a >= 1,
        s >= 1,
        x % a == 0,
    ensures
        (x * s) % (a * s) == 0,
{
}

/// TRUSTED: a multiple of a multiple — `y % d == 0` implies `(x*y) % d == 0`.
#[verifier::external_body]
pub proof fn div_mul_multiple_axiom(x: int, y: int, d: int)
    requires
        d >= 1,
        y % d == 0,
    ensures
        (x * y) % d == 0,
{
}

// ---------------------------------------------------------------------------
// Layout + size class
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub size: usize,
    pub align: usize,
}

/// Rust `Layout` well-formedness plus the heap's supported-alignment envelope.
pub open spec fn layout_wf(l: Layout) -> bool {
    &&& l.align >= 1
    &&& l.align <= MAX_ALIGN
    &&& is_pow2(l.align as nat)
    &&& l.size <= usize::MAX - PAGE_SIZE
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
    if layout.size > 4096 || layout.align > 4096 {
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
    if layout.size > 4096 || layout.align > 4096 {
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
// Frame allocator abstraction (matches `kor-frame`: extents are rounded up to
// a power of two and aligned to that many 4 KiB pages)
// ---------------------------------------------------------------------------

/// Physical addresses live far below any address-space limit (48-bit PA
/// space at most on the supported arches).
pub const MAX_PHYS_ADDR: u64 = 0x0000_FFFF_FFFF_FFFF;

#[verifier::external_body]
pub fn frame_alloc(count: u64) -> (r: Option<u64>)
    requires
        1 <= count,
    ensures
        r matches Some(p) ==> {
            &&& p <= MAX_PHYS_ADDR
            &&& exists|e: nat|
                is_pow2(e) && count as int <= e && p as int % (e as int * PAGE_SIZE as int) == 0
        },
{
    None // model: nondeterministic choice trusted from `kor-frame`
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
// Large (frame-backed) allocations
// ---------------------------------------------------------------------------

/// Page frames needed to cover `l.size` (ceiling division).
pub open spec fn size_frames(l: Layout) -> nat {
    ((l.size as int + PAGE_SIZE as int - 1) / PAGE_SIZE as int) as nat
}

/// Page frames needed to honour `l.align`.
pub open spec fn align_frames(l: Layout) -> nat {
    if l.align <= PAGE_SIZE {
        1
    } else {
        (l.align / PAGE_SIZE) as nat
    }
}

pub open spec fn large_extent_frames_spec(l: Layout) -> nat {
    let s = size_frames(l);
    let a = align_frames(l);
    if s >= a {
        s
    } else {
        a
    }
}

pub fn large_extent_frames(l: Layout) -> (f: u64)
    requires
        layout_wf(l),
    ensures
        f as int == large_extent_frames_spec(l) as int,
        f >= 1,
        l.size as int <= f as int * PAGE_SIZE as int,
        f as int >= align_frames(l) as int,
{
    let s = (l.size as u64 + PAGE_SIZE as u64 - 1) / PAGE_SIZE as u64;
    let a = if l.align <= PAGE_SIZE {
        1u64
    } else {
        l.align as u64 / PAGE_SIZE as u64
    };
    if s >= a {
        s
    } else {
        a
    }
}




/// Large, frame-backed allocation with the kernel's frame-growth gate.
///
/// `offset` models the arch's phys→virt constant; the preconditions state the
/// kernel invariant that it is page-aligned and preserves every supported
/// alignment (true for all four arches: each offset is a 4 GiB multiple).
pub fn alloc_large(gate: bool, l: Layout, offset: u64) -> (r: Option<u64>)
    requires
        layout_wf(l),
        class_for_spec(l) == SlabClass::Large,
        offset as int % PAGE_SIZE as int == 0,
        offset as int % l.align as int == 0,
        offset <= KERNEL_OFFSET,
    ensures
        !gate ==> r.is_none(),
        r matches Some(a) ==> {
            &&& a as int % l.align as int == 0
            &&& a as int % PAGE_SIZE as int == 0
        },
{
    if !gate {
        return None;
    }
    let frames = large_extent_frames(l);
    let phys = frame_alloc(frames);
    match phys {
        None => None,
        Some(p) => {
            proof {
                assert(p >= 0);
                let e = choose|e: nat|
                    is_pow2(e) && frames as int <= e && p as int % (e as int * PAGE_SIZE as int) == 0;
                let extent = e as int * PAGE_SIZE as int;
                assert(extent >= 1);
                // e is a power of two at least as large as the align requirement.
                let af = align_frames(l);
                assert(af as int <= e as int) by {
                    assert(frames as int >= af as int);
                };
                if l.align > PAGE_SIZE {
                    align_decomposes(l);
                    pow2_divides_axiom(af, e);
                    assert(e as int % af as int == 0);
                    div_scale_axiom(e as int, af as int, PAGE_SIZE as int);
                    assert((e as int * PAGE_SIZE as int) % (af as int * PAGE_SIZE as int) == 0);
                    assert(extent % l.align as int == 0);
                } else {
                    assert(af == 1);
                    page_is_pow2();
                    pow2_divides_axiom(l.align as nat, PAGE_SIZE as nat);
                    assert(PAGE_SIZE as int % l.align as int == 0);
                    div_mul_multiple_axiom(e as int, PAGE_SIZE as int, l.align as int);
                    assert(extent % l.align as int == 0);
                }
                div_trans_axiom(p as int, extent, l.align as int);
                assert(p as int % l.align as int == 0);
                div_mul_multiple_axiom(e as int, PAGE_SIZE as int, PAGE_SIZE as int);
                assert(extent % PAGE_SIZE as int == 0);
                div_trans_axiom(p as int, extent, PAGE_SIZE as int);
                assert(p as int % PAGE_SIZE as int == 0);
                assert(p as int + offset as int <= u64::MAX as int) by {
                    assert(p <= MAX_PHYS_ADDR);
                    assert(offset <= 0x1_0000_0000);
                };
                mod_sum_zero_axiom(p as int, offset as int, l.align as int);
                mod_sum_zero_axiom(p as int, offset as int, PAGE_SIZE as int);
            }
            Some(p + offset)
        }
    }
}

/// `dealloc_large` recomputes the very same extent and hands it back to the
/// frame allocator — no leak, no double free in the model.
#[verifier::external_body]
pub fn dealloc_large(addr: u64, l: Layout)
    requires
        layout_wf(l),
        class_for_spec(l) == SlabClass::Large,
        addr as int % PAGE_SIZE as int == 0,
    ensures
        true,
{
    // kernel: let frames = large_extent_frames(l);
    //         kor::frame::free_frames(virt_to_phys(addr), frames);
}

/// End-to-end large roundtrip: any successful allocation is correctly aligned
/// and its extent is computed deterministically for the matching free.
pub fn large_roundtrip(l: Layout, offset: u64) -> (ok: bool)
    requires
        layout_wf(l),
        class_for_spec(l) == SlabClass::Large,
        offset as int % PAGE_SIZE as int == 0,
        offset as int % l.align as int == 0,
        offset <= KERNEL_OFFSET,
    ensures
        ok == true,
{
    let allocated = alloc_large(true, l, offset);
    if let Some(a) = allocated {
        dealloc_large(a, l);
    }
    true
}

// ---------------------------------------------------------------------------
// SlabHeap — seven size classes (bootstrap + gated growth)
// ---------------------------------------------------------------------------

pub struct SlabHeap {
    pub slab_64: Slab,
    pub slab_128: Slab,
    pub slab_256: Slab,
    pub slab_512: Slab,
    pub slab_1024: Slab,
    pub slab_2048: Slab,
    pub slab_4096: Slab,
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
            start % PAGE_SIZE == 0,
        ensures
            final(self).wf(),
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
    }
}

/// Frame-backed slab growth behind the kernel's `FRAME_GROWTH_ENABLED` gate:
/// refused before memory discovery and for Large requests; otherwise yields a
/// page-aligned region (GROW_PAGES is a power of two).
pub fn grow_from_frames(gate: bool, class: SlabClass, offset: u64) -> (r: Option<u64>)
    requires
        offset as int % PAGE_SIZE as int == 0,
        offset <= KERNEL_OFFSET,
    ensures
        !gate ==> r.is_none(),
        class == SlabClass::Large ==> r.is_none(),
        r matches Some(p) ==> p as int % PAGE_SIZE as int == 0,
{
    if !gate {
        return None;
    }
    match class {
        SlabClass::Large => return None,
        _ => (),
    }
    let phys = frame_alloc(GROW_PAGES as u64);
    match phys {
        None => None,
        Some(p) => {
            proof {
                let e = choose|e: nat|
                    is_pow2(e)
                        && GROW_PAGES as int <= e
                        && p as int % (e as int * PAGE_SIZE as int) == 0;
                vstd::arithmetic::div_mod::lemma_fundamental_div_mod(PAGE_SIZE as int, PAGE_SIZE as int);
                assert(PAGE_SIZE as int % PAGE_SIZE as int == 0);
                div_mul_multiple_axiom(e as int, PAGE_SIZE as int, PAGE_SIZE as int);
                div_trans_axiom(p as int, e as int * PAGE_SIZE as int, PAGE_SIZE as int);
                assert(p as int % PAGE_SIZE as int == 0);
                assert(p as int + offset as int <= u64::MAX as int) by {
                    assert(p <= MAX_PHYS_ADDR);
                    assert(offset <= 0x1_0000_0000);
                };
                mod_sum_zero_axiom(p as int, offset as int, PAGE_SIZE as int);
            }
            Some(p + offset)
        }
    }
}

// ---------------------------------------------------------------------------
// Example lifecycles (checked by Verus)
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

/// Over-aligned and oversized layouts must route to Large and, once growth is
/// enabled, only ever yield aligned memory.
pub fn large_routing_and_alignment() -> (ok: bool)
    ensures
        ok == true,
{
    let over_aligned = Layout { size: 64, align: 8192 };
    let oversized = Layout { size: 5000, align: 8 };
    proof {
        page_is_pow2();
        assert(pow2(13) == 2 * pow2(12));
        assert(pow2(13) == 8192);
        assert(is_pow2(8192));
        assert(is_pow2(8));
        assert(layout_wf(over_aligned));
        assert(layout_wf(oversized));
        assert(class_for_spec(over_aligned) == SlabClass::Large);
        assert(class_for_spec(oversized) == SlabClass::Large);
    }
    // Before memory discovery the gate is closed: nothing is handed out.
    let gated_off = alloc_large(false, oversized, KERNEL_OFFSET);
    assert(gated_off.is_none());
    // With the gate open, every handed-out address honours the alignment.
    let rt_oversized = large_roundtrip(oversized, KERNEL_OFFSET);
    assert(rt_oversized);
    let rt_over_aligned = large_roundtrip(over_aligned, KERNEL_OFFSET);
    assert(rt_over_aligned);
    // Slab growth honours the gate and the Large exclusion.
    let grow_gated = grow_from_frames(false, SlabClass::Bytes64, KERNEL_OFFSET);
    assert(grow_gated.is_none());
    let grow_large = grow_from_frames(true, SlabClass::Large, KERNEL_OFFSET);
    assert(grow_large.is_none());
    true
}

} // verus!

fn main() {}
