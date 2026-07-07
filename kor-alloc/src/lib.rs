#![no_std]
//! Slab-based kernel heap type (`LockedSlabHeap`) — the `#[global_allocator]`
//! instance, bootstrap region and self-test live in the binary crate (`koros`),
//! which constructs a `LockedSlabHeap` static and inits it from a bootstrap
//! region before the frame allocator is populated.

extern crate alloc;

pub mod slab_heap;

pub use slab_heap::{LockedSlabHeap, SlabHeap, MIN_HEAP_BYTES};
