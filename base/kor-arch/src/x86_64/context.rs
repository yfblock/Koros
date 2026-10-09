//! x86_64 task context switch (callee-saved registers via the stack).

use core::arch::naked_asm;

/// Saved stack pointer and user FS base for one schedulable task.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TaskContext {
    rsp: usize,
    fs_base: usize,
}

impl TaskContext {
    /// Borrow the opaque kor buffer as this arch's typed layout.
    ///
    /// # Safety
    /// The buffer must be a valid `kor::TaskContext` (repr(C), align 16) and
    /// the arch layout must fit within (const-checked below).
    pub fn from_opaque(ctx: &mut kor::TaskContext) -> &mut Self {
        // SAFETY: same layout contract as the previous provider cast.
        unsafe { &mut *(ctx as *mut kor::TaskContext as *mut TaskContext) }
    }


    pub const fn zero() -> Self {
        Self { rsp: 0, fs_base: 0 }
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
    // rdi = prev, rsi = next. Save and restore IA32_FS_BASE because a task may
    // block in a user syscall while another process changes its TLS base.
    naked_asm!(
        "push rdi",
        "push rsi",
        "mov ecx, 0xC0000100",
        "rdmsr",
        "pop rsi",
        "pop rdi",
        "mov [rdi + 8], eax",
        "mov [rdi + 12], edx",
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, [rsi]",
        "mov rax, [rsi + 8]",
        "mov ecx, 0xC0000100",
        "mov rdx, rax",
        "shr rdx, 32",
        "wrmsr",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}
