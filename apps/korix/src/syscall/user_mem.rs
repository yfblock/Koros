//! Checked access to the active process's user address space.
//!
//! Raw-pointer operations are confined here. Callers work with copied values
//! and byte slices, never direct aliases into user mappings.

use alloc::string::String;
use alloc::vec::Vec;

const PAGE_SIZE: usize = 4096;

fn current_user_phys(address: usize, write: bool) -> Option<usize> {
    crate::sched::with_current_process(|process| {
        let process = process.lock();
        let space = process.address_space()?;
        space
            .validate_range(address, 1, write)
            .then(|| space.translate(address))?
    })?
}

/// Translate one validated user byte for legacy single-byte users.
pub(super) fn kernel_address(address: usize) -> usize {
    current_user_phys(address, false)
        .map(kor_arch::phys_to_virt)
        .unwrap_or(0)
}

pub(super) fn check_range(address: usize, length: usize, write: bool) -> Option<()> {
    if address == 0 {
        return None;
    }
    let end = address.checked_add(length)?;
    let process = crate::sched::current_process()?;
    let process = process.lock();
    process
        .address_space()?
        .validate_range(address, end - address, write)
        .then_some(())
}

pub(super) fn copy_from(address: usize, destination: &mut [u8]) -> Option<()> {
    check_range(address, destination.len(), false)?;
    let mut copied = 0;
    while copied < destination.len() {
        let user = address.checked_add(copied)?;
        let physical = current_user_phys(user, false)?;
        let chunk = (PAGE_SIZE - (user & (PAGE_SIZE - 1))).min(destination.len() - copied);
        // SAFETY: `current_user_phys` validates the mapped source byte and the
        // range check validates every covered page. `destination` owns `chunk`
        // writable bytes and does not overlap physical user memory.
        unsafe {
            core::ptr::copy_nonoverlapping(
                kor_arch::phys_to_virt(physical) as *const u8,
                destination.as_mut_ptr().add(copied),
                chunk,
            );
        }
        copied += chunk;
    }
    Some(())
}

pub(super) fn copy_to(address: usize, source: &[u8]) -> Option<()> {
    check_range(address, source.len(), true)?;
    let mut copied = 0;
    while copied < source.len() {
        let user = address.checked_add(copied)?;
        let physical = current_user_phys(user, true)?;
        let chunk = (PAGE_SIZE - (user & (PAGE_SIZE - 1))).min(source.len() - copied);
        // SAFETY: the mapped destination and source slice were validated for
        // `chunk` bytes and cannot overlap across the kernel/user boundary.
        unsafe {
            core::ptr::copy_nonoverlapping(
                source.as_ptr().add(copied),
                kor_arch::phys_to_virt(physical) as *mut u8,
                chunk,
            );
        }
        copied += chunk;
    }
    Some(())
}

pub(super) fn read<T: Copy>(address: usize) -> Option<T> {
    let mut value = core::mem::MaybeUninit::<T>::uninit();
    // SAFETY: `value` is writable uninitialized storage for exactly `size_of<T>`
    // bytes. `copy_from` initializes all bytes before `assume_init`.
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), core::mem::size_of::<T>())
    };
    copy_from(address, bytes)?;
    // SAFETY: `copy_from` initialized every byte and `T: Copy` is the syscall
    // ABI contract for these plain value representations.
    Some(unsafe { value.assume_init() })
}

pub(super) fn write<T: Copy>(address: usize, value: &T) -> Option<()> {
    // SAFETY: an immutable reference exposes `size_of<T>` initialized bytes.
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), core::mem::size_of::<T>())
    };
    copy_to(address, bytes)
}

pub(super) fn read_string(address: usize, max: usize) -> Option<String> {
    let mut bytes = Vec::new();
    for offset in 0..max {
        let byte: u8 = read(address.checked_add(offset)?)?;
        if byte == 0 {
            return String::from_utf8(bytes).ok();
        }
        bytes.push(byte);
    }
    None
}
