//! aarch64 adapter: the ABI already uses the `asm-generic` syscall numbers
//! (x8 = nr, x0-x5 = args), so translation is the identity.

/// Translate a native aarch64 syscall into canonical form: identity.
pub fn translate(nr: usize, _args: &mut [usize; 6]) -> Option<usize> {
    Some(nr)
}

/// aarch64 has no syscalls outside the generic set.
pub fn native_syscall(_nr: usize, _args: &[usize]) -> isize {
    -38 // -ENOSYS
}
