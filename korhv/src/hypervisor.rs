//! Architecture-neutral hypervisor lifecycle.
//!
//! Architecture modules implement [`Hypervisor`] and expose one static runtime.
//! The composition root only performs common boot setup, then calls this API.

/// Hardware-virtualization runtime selected by the architecture layer.
pub trait Hypervisor {
    /// Enable the CPU virtualization extension and initialize host-side state.
    fn init(&self);

    /// Create and run the architecture's configured guests until shutdown.
    fn run(&self);
}
