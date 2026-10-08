//! riscv64 adapter: the ABI already uses the `asm-generic` syscall numbers
//! (a7 = nr, a0-a5 = args), so translation is the identity.

/// Translate a native riscv64 syscall into canonical form: identity.
pub fn translate(nr: usize, _args: &mut [usize; 6]) -> Option<usize> {
    Some(nr)
}

/// riscv64 has no syscalls outside the generic set.
pub fn native_syscall(_nr: usize, _args: &[usize]) -> isize {
    -38 // -ENOSYS
}
