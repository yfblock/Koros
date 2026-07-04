//! Blocking synchronisation library.
//!
//! Built on the scheduler's [`WaitQueue`]/[`Semaphore`] (which block the
//! calling task rather than spinning).  Provides a guard-based [`Mutex`] and a
//! blocking [`Channel`].
//!
//! Note: these are *blocking* primitives — only usable from task context after
//! the scheduler is running, not from interrupt handlers or early boot.  For
//! short critical sections in that code, use `spin::Mutex` instead.

extern crate alloc;

use alloc::collections::VecDeque;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};

pub use crate::sched::{Semaphore, WaitQueue};

/// A mutual-exclusion lock that blocks (sleeps) the waiting task, guarding a
/// value of type `T` with an RAII guard.
pub struct Mutex<T> {
    sem: Semaphore,
    data: UnsafeCell<T>,
}

// SAFETY: access to `data` is serialised by the binary semaphore.
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self { sem: Semaphore::new(1), data: UnsafeCell::new(value) }
    }

    /// Acquire the lock, blocking until it is available.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.sem.wait();
        MutexGuard { mutex: self }
    }
}

/// RAII guard that releases the [`Mutex`] on drop.
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock, so exclusive access is guaranteed.
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock, so exclusive access is guaranteed.
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.sem.post();
    }
}

/// An unbounded multi-producer/multi-consumer channel.  `recv` blocks until an
/// item is available.
pub struct Channel<T> {
    queue: Mutex<VecDeque<T>>,
    items: Semaphore,
}

impl<T> Default for Channel<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Channel<T> {
    pub const fn new() -> Self {
        Self { queue: Mutex::new(VecDeque::new()), items: Semaphore::new(0) }
    }

    /// Send an item; wakes one blocked receiver.
    pub fn send(&self, item: T) {
        self.queue.lock().push_back(item);
        self.items.post();
    }

    /// Receive an item, blocking until one is available.
    pub fn recv(&self) -> T {
        self.items.wait();
        // An item is guaranteed to be present: `items` was posted once per
        // enqueued item, and this `wait` consumed exactly one token.
        self.queue.lock().pop_front().expect("channel item missing")
    }
}
