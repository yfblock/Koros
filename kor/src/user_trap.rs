//! Opaque, lifetime-bound access to a live architecture user trap frame.

use core::marker::PhantomData;
use core::ptr::NonNull;

/// Opaque handle to the current architecture's live user trap frame.
///
/// The raw frame representation remains owned by `kor-arch`; composition code
/// can pass this handle through hooks but cannot dereference it.
#[derive(Clone, Copy)]
pub struct UserTrapFrame<'frame> {
    pointer: NonNull<u8>,
    lifetime: PhantomData<&'frame mut u8>,
}

impl<'frame> UserTrapFrame<'frame> {
    /// Wrap a live architecture trap frame for the duration of one trap.
    ///
    /// # Safety
    /// `pointer` must be non-null, aligned for the selected architecture's trap
    /// frame, and uniquely borrowed for `'frame`.
    pub unsafe fn from_raw(pointer: *mut u8) -> Option<Self> {
        Some(Self {
            pointer: NonNull::new(pointer)?,
            lifetime: PhantomData,
        })
    }

    /// Return the opaque address for the architecture adapter.
    pub fn as_ptr(self) -> *mut u8 {
        self.pointer.as_ptr()
    }
}

// The handle never grants memory access and is only passed synchronously while
// the underlying frame is live.
unsafe impl Send for UserTrapFrame<'_> {}
unsafe impl Sync for UserTrapFrame<'_> {}
