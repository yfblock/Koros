//! Kernel heap: bootstrap region + self-test for the global allocator.
//!
//! `#[global_allocator] static HEAP` lives at the crate root (`main.rs`) -- the
//! attribute must be on a crate-root static.  This module inits it from a static
//! bootstrap region (before the frame allocator is populated) and runs the slab
//! self-test.

use kor_alloc::MIN_HEAP_BYTES;

use crate::HEAP;

const BOOTSTRAP_HEAP_SIZE: usize = 0xE000;

#[repr(align(4096))]
struct BootstrapSpace([u8; BOOTSTRAP_HEAP_SIZE]);

static mut BOOTSTRAP_SPACE: BootstrapSpace = BootstrapSpace([0; BOOTSTRAP_HEAP_SIZE]);

const _: () = assert!(BOOTSTRAP_HEAP_SIZE >= MIN_HEAP_BYTES);
const _: () = assert!(BOOTSTRAP_HEAP_SIZE % MIN_HEAP_BYTES == 0);

pub fn init_bootstrap() {
    unsafe {
        let start = core::ptr::addr_of!(BOOTSTRAP_SPACE) as usize;
        HEAP.init(start, BOOTSTRAP_HEAP_SIZE);
    }
}

pub fn self_test() {
    use alloc::collections::BTreeSet;
    use alloc::vec::Vec;

    let mut v: Vec<u64> = Vec::with_capacity(64);
    v.push(0xDEAD_BEEF);
    assert_eq!(v[0], 0xDEAD_BEEF);

    let mut set: BTreeSet<usize> = BTreeSet::new();
    set.insert(42);
    assert!(set.contains(&42));

    let mut blocks: Vec<Vec<u8>> = Vec::new();
    for i in 0..512 {
        blocks.push(alloc::vec![i as u8; 64]);
    }
    drop(blocks);

    let large: Vec<u8> = alloc::vec![0xAB; 5000];
    assert_eq!(large.len(), 5000);

    kor::println!("mm: slab heap OK (frame-backed)");
}
