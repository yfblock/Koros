//! Scheduler for korix.
//!
//! Reuses kor-sched for kernel threads, adds user process scheduling.
//! User processes are scheduled as kernel threads that enter user mode
//! when run and trap back to kernel mode on syscall/interrupt.

// TODO: integrate with kor-sched
