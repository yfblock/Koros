//! Opaque `TaskContext` — the arch-neutral task-context buffer shared by
//! kor-arch's context switch and the scheduler.

/// Opaque callee-saved register storage, large enough for every supported arch.
///
/// Arch code casts this buffer to its internal layout with
/// `&mut *(ctx as *mut _ as *mut ArchTaskContext)`.  32 `usize`s (256 bytes)
/// accommodate integer and callee-saved floating-point task state on all
/// supported architectures.
#[repr(C, align(16))]
pub struct TaskContext {
    pub _storage: [usize; 32],
}
