//! x86_64 task context switch (callee-saved registers via the stack).

use core::arch::naked_asm;

/// The whole saved state is the stack pointer; callee-saved registers and the
/// return address live on that task's stack.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TaskContext {
    rsp: usize,
}

impl TaskContext {
    pub const fn zero() -> Self {
        Self { rsp: 0 }
    }

    /// Build an initial stack frame matching what [`context_switch`] restores:
    /// six zeroed callee-saved slots followed by `entry` as the return address.
    /// Leaves `rsp % 16 == 8` at `entry` (System V ABI on function entry).
    pub fn init(&mut self, entry: usize, stack_top: usize) {
        let base = (stack_top & !0xF) - 64;
        // SAFETY: `base` lies within the task's freshly allocated stack.
        unsafe {
            let frame = base as *mut usize;
            for i in 0..6 {
                frame.add(i).write(0); // r15,r14,r13,r12,rbx,rbp
            }
            frame.add(6).write(entry); // return address popped by `ret`
        }
        self.rsp = base;
    }
}

/// Save callee-saved registers on the current stack, swap stacks via
/// `prev`/`next`, and return into `next`.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn context_switch(prev: *mut TaskContext, next: *const TaskContext) {
    // rdi = prev, rsi = next (Rust inline asm uses Intel syntax by default)
    naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, [rsi]",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}
