//! Cooperative kernel-thread scheduler.
//!
//! Provides kernel tasks with a round-robin ready queue, voluntary
//! [`yield_now`], timer-based [`sleep_ms`], and [`exit`].  There is no
//! preemption yet: tasks run until they yield, sleep, or exit.  Scheduling
//! runs on the boot CPU; secondary CPUs still idle.
//!
//! Scheduler critical sections run with interrupts disabled so the timer
//! interrupt (which wakes sleepers via [`timer_tick`]) can never fire while a
//! queue lock is held.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use spin::{Mutex, Once};

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::context as arch_ctx;
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::context as arch_ctx;
#[cfg(target_arch = "aarch64")]
use crate::arch::aarch64::context as arch_ctx;
#[cfg(target_arch = "loongarch64")]
use crate::arch::loongarch64::context as arch_ctx;

use arch_ctx::TaskContext;

/// Per-task kernel stack size.
const STACK_SIZE: usize = 0x1_0000; // 64 KiB

// Task states.
const READY: u8 = 0;
const RUNNING: u8 = 1;
const SLEEPING: u8 = 2;
const EXITED: u8 = 3;

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// A kernel task (thread).
pub struct Task {
    pub id: usize,
    ctx: UnsafeCell<TaskContext>,
    // Kept alive for the task's lifetime; the idle task has an empty stack and
    // runs on the boot stack instead.
    _stack: Box<[u8]>,
    entry: fn(),
    state: core::sync::atomic::AtomicU8,
    wake_tick: AtomicU64,
}

// SAFETY: the `ctx` cell is only touched by the owning CPU during a switch
// while interrupts are disabled; tasks are otherwise accessed behind the
// scheduler locks.
unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl Task {
    fn new(entry: fn()) -> Arc<Self> {
        let stack = alloc::vec![0u8; STACK_SIZE].into_boxed_slice();
        let stack_top = (stack.as_ptr() as usize + STACK_SIZE) & !0xF;
        let mut ctx = TaskContext::zero();
        ctx.init(task_bootstrap as usize, stack_top);
        Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            ctx: UnsafeCell::new(ctx),
            _stack: stack,
            entry,
            state: core::sync::atomic::AtomicU8::new(READY),
            wake_tick: AtomicU64::new(0),
        })
    }

    fn idle() -> Arc<Self> {
        Arc::new(Self {
            id: 0,
            ctx: UnsafeCell::new(TaskContext::zero()),
            _stack: Box::new([]),
            entry: || {},
            state: core::sync::atomic::AtomicU8::new(RUNNING),
            wake_tick: AtomicU64::new(0),
        })
    }
}

static READY_QUEUE: Mutex<VecDeque<Arc<Task>>> = Mutex::new(VecDeque::new());
static SLEEPERS: Mutex<Vec<Arc<Task>>> = Mutex::new(Vec::new());
static CURRENT: Mutex<Option<Arc<Task>>> = Mutex::new(None);
static IDLE: Once<Arc<Task>> = Once::new();

/// Exited tasks awaiting cleanup (their stacks are still in use during the
/// switch away, so they're freed later from another task via [`reap`]).
static ZOMBIES: Mutex<Vec<Arc<Task>>> = Mutex::new(Vec::new());

/// Timer ticks a task may run before being preempted (~50 ms at 100 Hz).
const TIME_SLICE: u32 = 5;
/// Remaining ticks in the current task's slice.
static SLICE: AtomicU32 = AtomicU32::new(TIME_SLICE);

fn current() -> Arc<Task> {
    CURRENT.lock().clone().expect("scheduler not initialised")
}

/// Initialise the scheduler, turning the current (boot) execution into the
/// idle task.  Call once before [`spawn`]/[`idle_loop`].
pub fn init() {
    let idle = Task::idle();
    IDLE.call_once(|| idle.clone());
    *CURRENT.lock() = Some(idle);
}

/// Create a new ready kernel task running `entry`.
pub fn spawn(entry: fn()) -> usize {
    let task = Task::new(entry);
    let id = task.id;
    READY_QUEUE.lock().push_back(task);
    id
}

/// Pick the next task to run: the head of the ready queue, or the idle task.
fn pick_next() -> Arc<Task> {
    READY_QUEUE
        .lock()
        .pop_front()
        .unwrap_or_else(|| IDLE.get().expect("scheduler not initialised").clone())
}

/// Switch from `prev` to `next` (must be called with interrupts disabled and
/// no scheduler locks held).
fn switch_to(prev: &Arc<Task>, next: &Arc<Task>) {
    next.state.store(RUNNING, Ordering::Relaxed);
    SLICE.store(TIME_SLICE, Ordering::Relaxed); // fresh slice for the next task
    let prev_ctx = prev.ctx.get();
    let next_ctx = next.ctx.get() as *const TaskContext;
    // SAFETY: both tasks are kept alive (prev by the queue/sleepers/idle,
    // next by CURRENT); their context cells are valid for the switch.
    unsafe { arch_ctx::context_switch(prev_ctx, next_ctx) };
}

/// Voluntarily yield the CPU to the next ready task, if any.
pub fn yield_now() {
    let enabled = crate::irq::is_enabled();
    crate::irq::disable();

    let is_idle = IDLE.get().is_some_and(|idle| {
        CURRENT
            .lock()
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(c, idle))
    });

    let (prev, next) = {
        let next = pick_next();
        let mut cur = CURRENT.lock();
        let prev = cur.take().unwrap();
        if Arc::ptr_eq(&prev, &next) {
            *cur = Some(prev);
            drop(cur);
            if enabled {
                crate::irq::enable();
            }
            return;
        }
        // A running (non-idle) task goes back to the ready queue.
        if !is_idle {
            prev.state.store(READY, Ordering::Relaxed);
            READY_QUEUE.lock().push_back(prev.clone());
        }
        *cur = Some(next.clone());
        (prev, next)
    };

    switch_to(&prev, &next);
    if enabled {
        crate::irq::enable();
    }
}

/// Sleep the current task for at least `ms` milliseconds.
pub fn sleep_ms(ms: u64) {
    let wake = crate::time::ticks() + (ms * crate::time::TICK_HZ / 1000).max(1);
    crate::irq::disable();

    let (prev, next) = {
        let next = pick_next();
        let mut cur = CURRENT.lock();
        let prev = cur.take().unwrap();
        prev.state.store(SLEEPING, Ordering::Relaxed);
        prev.wake_tick.store(wake, Ordering::Relaxed);
        SLEEPERS.lock().push(prev.clone());
        *cur = Some(next.clone());
        (prev, next)
    };

    switch_to(&prev, &next);
    // Resumed after being woken; interrupts were enabled by whoever ran us.
    crate::irq::enable();
}

/// Terminate the current task and switch away permanently.
pub fn exit() -> ! {
    crate::irq::disable();
    let next = pick_next();
    let mut cur = CURRENT.lock();
    let prev = cur.take().unwrap();
    prev.state.store(EXITED, Ordering::Relaxed);
    *cur = Some(next.clone());
    drop(cur);

    next.state.store(RUNNING, Ordering::Relaxed);
    SLICE.store(TIME_SLICE, Ordering::Relaxed);
    let prev_ctx = prev.ctx.get();
    let next_ctx = next.ctx.get() as *const TaskContext;
    // Move `prev` into the zombie list (no lingering local ref → no leak);
    // its stack is still in use until the switch completes, so it is freed
    // later by [`reap`] from another task.
    ZOMBIES.lock().push(prev);
    // SAFETY: both tasks are alive (prev in ZOMBIES, next in CURRENT).
    unsafe { arch_ctx::context_switch(prev_ctx, next_ctx) };
    unreachable!("exited task resumed");
}

/// Free the stacks of tasks that have exited (called from a live task).
fn reap() {
    crate::irq::without(|| {
        let mut zombies = ZOMBIES.lock();
        if !zombies.is_empty() {
            zombies.clear();
        }
    });
}

/// Timer preemption hook: called from the arch trap handler right after
/// [`crate::time::tick`].  Yields when the running task's slice is used up.
pub fn preempt() {
    if IDLE.get().is_none() {
        return; // scheduler not running yet
    }
    if SLICE.load(Ordering::Relaxed) == 0 {
        SLICE.store(TIME_SLICE, Ordering::Relaxed);
        yield_now();
    }
}

/// Timer hook: move any sleepers whose deadline has passed back to ready.
/// Called from the timer interrupt (interrupts already disabled).
pub fn timer_tick() {
    if IDLE.get().is_none() {
        return; // scheduler not running yet
    }
    let now = crate::time::ticks();
    let mut sleepers = SLEEPERS.lock();
    let mut i = 0;
    while i < sleepers.len() {
        if sleepers[i].wake_tick.load(Ordering::Relaxed) <= now {
            let task = sleepers.swap_remove(i);
            task.state.store(READY, Ordering::Relaxed);
            READY_QUEUE.lock().push_back(task);
        } else {
            i += 1;
        }
    }
    drop(sleepers);

    // Count down the running task's time slice; expiry is acted on by
    // `preempt` (called from the trap handler after this returns).
    let rem = SLICE.load(Ordering::Relaxed);
    if rem > 0 {
        SLICE.store(rem - 1, Ordering::Relaxed);
    }
}

/// The idle loop: run ready tasks, otherwise wait for an interrupt.  Becomes
/// the idle task; never returns.
pub fn idle_loop() -> ! {
    loop {
        crate::irq::enable();
        reap(); // free any exited tasks' stacks
        yield_now();
        // Nothing ready right now — wait for the timer to wake a sleeper.
        crate::smp::wait_for_interrupt();
    }
}

/// First code every freshly spawned task runs: call its entry, then exit.
extern "C" fn task_bootstrap() -> ! {
    // The switch into us happened with interrupts disabled; enable them now.
    crate::irq::enable();
    let entry = current().entry;
    entry();
    exit();
}
