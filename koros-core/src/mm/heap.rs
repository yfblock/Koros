//! Kernel heap backing the global allocator.
//!
//! A small static **bootstrap** region satisfies the frame buddy's internal
//! `BTreeSet` during `mm::init()`.  After physical frames are registered, the
//! heap grows on demand by mapping new pages from the frame buddy allocator.

use crate::mm::slab_heap::{LockedSlabHeap, MIN_HEAP_BYTES};

/// Bootstrap heap — only used while the frame allocator is first populated.
const BOOTSTRAP_HEAP_SIZE: usize = 0xE000;

#[global_allocator]
static HEAP: LockedSlabHeap = LockedSlabHeap::empty();

#[repr(align(4096))]
struct BootstrapSpace([u8; BOOTSTRAP_HEAP_SIZE]);

static mut BOOTSTRAP_SPACE: BootstrapSpace = BootstrapSpace([0; BOOTSTRAP_HEAP_SIZE]);

const _: () = assert!(BOOTSTRAP_HEAP_SIZE >= MIN_HEAP_BYTES);
const _: () = assert!(BOOTSTRAP_HEAP_SIZE % MIN_HEAP_BYTES == 0);

/// Initialise the bootstrap slab heap (before the frame allocator).
pub fn init_bootstrap() {
    unsafe {
        let start = core::ptr::addr_of!(BOOTSTRAP_SPACE) as usize;
        HEAP.init(start, BOOTSTRAP_HEAP_SIZE);
    }
}

/// Exercise slab size classes and one frame-backed large allocation.
pub fn self_test() {
    use alloc::collections::BTreeSet;
    use alloc::vec::Vec;

    let mut v: Vec<u64> = Vec::with_capacity(64);
    v.push(0xDEAD_BEEF);
    assert_eq!(v[0], 0xDEAD_BEEF);

    let mut set: BTreeSet<usize> = BTreeSet::new();
    set.insert(42);
    assert!(set.contains(&42));

    // Force at least one slab grow from the frame buddy.
    let mut blocks: Vec<Vec<u8>> = Vec::new();
    for i in 0..512 {
        blocks.push(alloc::vec![i as u8; 64]);
    }
    drop(blocks);

    // Large allocation (> 4 KiB) comes straight from frames.
    let large: Vec<u8> = alloc::vec![0xAB; 5000];
    assert_eq!(large.len(), 5000);

    crate::println!("mm: slab heap OK (frame-backed)");
}
