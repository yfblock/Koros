//! Kernel heap: bootstrap region + self-test.
//!
//! Copied from koros — uses a 4K-aligned static region for the slab heap
//! before the frame allocator is populated.

use kor_alloc::MIN_HEAP_BYTES;

use crate::HEAP;

const BOOTSTRAP_HEAP_SIZE: usize = 0xE000;

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
    use alloc::vec::Vec;

    let mut v: Vec<u64> = Vec::with_capacity(64);
    v.push(0xDEAD_BEEF);
    assert_eq!(v[0], 0xDEAD_BEEF);

    let large: Vec<u8> = alloc::vec![0xAB; 5000];
    assert_eq!(large.len(), 5000);

    kor::println!("mm: slab heap OK (frame-backed)");
}
