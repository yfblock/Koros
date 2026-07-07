#![no_std]
//! Preemptive multi-core kernel-thread scheduler.
//!
//! Each CPU keeps its own current task, idle task, and time slice; the ready,
//! sleeper, and zombie lists are shared.  Tasks migrate freely between CPUs.
//!
//! Correctness on SMP hinges on never making a task runnable (on the ready
//! queue) until its context is fully saved — otherwise another CPU could start
//! running a half-switched task.  This is handled with a deferred transition:
//! [`schedule`] records the outgoing task in a per-CPU slot and performs the
//! switch; [`finish_switch`] runs on the *incoming* task (after the switch
//! completed, so the outgoing context is saved) and only then applies the
//! transition (enqueue / sleep / reap).
//!
//! Scheduler critical sections run with interrupts disabled so the timer
//! interrupt (which decrements the slice and wakes sleepers) can never fire
//! while this CPU holds a scheduler lock.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use spin::Mutex;

use kor::smp::MAX_CPUS;

use kor::TaskContext;

/// Per-task kernel stack size.
const STACK_SIZE: usize = 0x1_0000; // 64 KiB

// Task states.
const READY: u8 = 0;
const RUNNING: u8 = 1;
const SLEEPING: u8 = 2;
const EXITED: u8 = 3;
const BLOCKED: u8 = 4;

// Deferred transition applied by `finish_switch` to the outgoing task.
const TO_READY: u8 = 0;
const TO_SLEEP: u8 = 1;
const TO_ZOMBIE: u8 = 2;
/// Block on a wait queue (its pointer is carried alongside the action); the
/// queue's raw lock is held across the switch and released by `finish_switch`
/// once the task is enqueued — closing both the migration and lost-wakeup
/// races (see [`WaitQueue`]).
const TO_WAIT: u8 = 3;

/// Timer ticks a task may run before being preempted (~50 ms at 100 Hz).
const TIME_SLICE: u32 = 5;

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// A kernel task (thread).  `id == 0` marks a per-CPU idle task.
pub struct Task {
    pub id: usize,
    ctx: UnsafeCell<TaskContext>,
    _stack: Box<[u8]>,
    entry: fn(),
    state: AtomicU8,
    wake_tick: AtomicU64,
}

// SAFETY: a task's `ctx` is only touched by the CPU currently running it (or
// switching it) with interrupts disabled; the deferred-transition protocol
// ensures no task is enqueued until its context is saved.
unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl Task {
    fn new(entry: fn()) -> Arc<Self> {
        let stack = alloc::vec![0u8; STACK_SIZE].into_boxed_slice();
        let stack_top = (stack.as_ptr() as usize + STACK_SIZE) & !0xF;
        let mut ctx = kor::arch::current().task_context_zero();
        kor::arch::current().task_context_init(&mut ctx, task_bootstrap as *const () as usize, stack_top);
        Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            ctx: UnsafeCell::new(ctx),
            _stack: stack,
            entry,
            state: AtomicU8::new(READY),
            wake_tick: AtomicU64::new(0),
        })
    }

    /// A per-CPU idle task; it runs on the CPU's boot/secondary stack, so its
    /// own stack buffer is empty and its context is filled on first switch.
    fn idle() -> Arc<Self> {
        Arc::new(Self {
            id: 0,
            ctx: UnsafeCell::new(kor::arch::current().task_context_zero()),
            _stack: Box::new([]),
            entry: || {},
            state: AtomicU8::new(RUNNING),
            wake_tick: AtomicU64::new(0),
        })
    }

    fn is_idle(&self) -> bool {
        self.id == 0
    }
}

/// Per-CPU scheduler state.
struct PerCpu {
    current: Mutex<Option<Arc<Task>>>,
    idle: Mutex<Option<Arc<Task>>>,
    /// Outgoing task awaiting its deferred transition (task, action, wait-queue
    /// pointer for `TO_WAIT`).
    prev: Mutex<Option<(Arc<Task>, u8, usize)>>,
    slice: AtomicU32,
}

impl PerCpu {
    const fn new() -> Self {
        Self {
            current: Mutex::new(None),
            idle: Mutex::new(None),
            prev: Mutex::new(None),
            slice: AtomicU32::new(TIME_SLICE),
        }
    }
}

static CPUS: [PerCpu; MAX_CPUS] = [const { PerCpu::new() }; MAX_CPUS];

fn this_cpu() -> &'static PerCpu {
    &CPUS[kor::arch::current().cpu_id()]
}

// Shared queues.
static READY_QUEUE: Mutex<VecDeque<Arc<Task>>> = Mutex::new(VecDeque::new());
static SLEEPERS: Mutex<Vec<Arc<Task>>> = Mutex::new(Vec::new());
static ZOMBIES: Mutex<Vec<Arc<Task>>> = Mutex::new(Vec::new());

/// Set once the boot CPU has initialised the scheduler; secondary CPUs wait
/// for this before joining.
static SCHED_READY: AtomicBool = AtomicBool::new(false);

/// The task currently running on this CPU.
pub fn current() -> Arc<Task> {
    this_cpu().current.lock().clone().expect("scheduler not initialised on this CPU")
}

/// Whether the scheduler has been initialised (by the boot CPU).
pub fn is_ready() -> bool {
    SCHED_READY.load(Ordering::Acquire)
}

/// Initialise the scheduler on the boot CPU (turns the current execution into
/// this CPU's idle task).  Call once before [`spawn`].
pub fn init() {
    init_this_cpu();
    SCHED_READY.store(true, Ordering::Release);
}

/// Set up the calling CPU's idle/current task.  Boot CPU calls it via
/// [`init`]; secondary CPUs call it directly once [`is_ready`].
pub fn init_this_cpu() {
    let idle = Task::idle();
    let cpu = this_cpu();
    *cpu.idle.lock() = Some(idle.clone());
    *cpu.current.lock() = Some(idle);
}

/// Create a new ready kernel task running `entry`.
pub fn spawn(entry: fn()) -> usize {
    let task = Task::new(entry);
    let id = task.id;
    READY_QUEUE.lock().push_back(task);
    id
}

/// The head of the shared ready queue, or this CPU's idle task.
fn pick_next(cpu: &PerCpu) -> Arc<Task> {
    READY_QUEUE
        .lock()
        .pop_front()
        .unwrap_or_else(|| cpu.idle.lock().clone().expect("no idle task"))
}

/// Apply the deferred transition for the task this CPU just switched away
/// from.  Runs on the incoming task, after the switch completed.
fn finish_switch() {
    let taken = this_cpu().prev.lock().take();
    if let Some((prev, action, wait_ptr)) = taken {
        match action {
            TO_READY => {
                // The idle task is a per-CPU fallback, never queued.
                if !prev.is_idle() {
                    prev.state.store(READY, Ordering::Relaxed);
                    READY_QUEUE.lock().push_back(prev);
                }
            }
            TO_SLEEP => {
                prev.state.store(SLEEPING, Ordering::Relaxed);
                SLEEPERS.lock().push(prev);
            }
            TO_ZOMBIE => {
                prev.state.store(EXITED, Ordering::Relaxed);
                ZOMBIES.lock().push(prev);
            }
            TO_WAIT => {
                // SAFETY: `wait_ptr` points to the WaitQueue the blocking task
                // is parking on; its raw lock is held (taken in `block_on`) and
                // released here after the task is enqueued.
                let wq = unsafe { &*(wait_ptr as *const WaitQueue) };
                prev.state.store(BLOCKED, Ordering::Relaxed);
                // SAFETY: the queue's raw lock is held across the switch.
                unsafe { (*wq.list.get()).push_back(prev) };
                wq.raw_unlock();
            }
            _ => {}
        }
    }
}

/// Core switch: pick the next task, record the outgoing one for the deferred
/// transition, and switch.  Must be called with interrupts disabled.
fn schedule(prev_action: u8, wait_ptr: usize) {
    let cpu = this_cpu();
    let next = pick_next(cpu);

    let (prev_ctx, next_ctx) = {
        let mut cur = cpu.current.lock();
        let prev = cur.as_ref().unwrap().clone();
        // Idle yielding with nothing else ready: keep running it.
        if Arc::ptr_eq(&prev, &next) && prev_action == TO_READY {
            return;
        }
        *cpu.prev.lock() = Some((prev.clone(), prev_action, wait_ptr));
        next.state.store(RUNNING, Ordering::Relaxed);
        cpu.slice.store(TIME_SLICE, Ordering::Relaxed);
        *cur = Some(next.clone());
        (prev.ctx.get(), next.ctx.get() as *const TaskContext)
    };

    // SAFETY: prev is kept alive via `cpu.prev`, next via `cpu.current`.
    unsafe { kor::arch::current().context_switch(prev_ctx, next_ctx) };
    finish_switch();
}

/// Voluntarily yield to the next ready task, if any.
pub fn yield_now() {
    let enabled = kor::irq::is_enabled();
    kor::irq::disable();
    schedule(TO_READY, 0);
    if enabled {
        kor::irq::enable();
    }
}

/// Sleep the current task for at least `ms` milliseconds.
pub fn sleep_ms(ms: u64) {
    let wake = kor::time::ticks() + (ms * kor::time::TICK_HZ / 1000).max(1);
    kor::irq::disable();
    current().wake_tick.store(wake, Ordering::Relaxed);
    schedule(TO_SLEEP, 0);
    kor::irq::enable();
}

/// Terminate the current task and switch away permanently.
pub fn exit() -> ! {
    kor::irq::disable();
    schedule(TO_ZOMBIE, 0);
    unreachable!("exited task resumed");
}

/// Move a blocked/parked task back to the ready queue.
fn make_ready(task: Arc<Task>) {
    task.state.store(READY, Ordering::Relaxed);
    READY_QUEUE.lock().push_back(task);
}

/// Free the stacks of exited tasks (called from a live task).
fn reap() {
    kor::irq::without(|| {
        let mut zombies = ZOMBIES.lock();
        if !zombies.is_empty() {
            zombies.clear();
        }
    });
}

/// Timer hook (called from the arch trap handler via [`kor::time`]):
/// wake due sleepers and count down this CPU's time slice.
pub fn timer_tick() {
    if !is_ready() {
        return;
    }
    let now = kor::time::ticks();
    {
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
    }
    let rem = this_cpu().slice.load(Ordering::Relaxed);
    if rem > 0 {
        this_cpu().slice.store(rem - 1, Ordering::Relaxed);
    }
}

/// Preemption hook: called from the arch trap handler after [`timer_tick`].
/// Yields when this CPU's slice is used up.
pub fn preempt() {
    if !is_ready() {
        return;
    }
    if this_cpu().slice.load(Ordering::Relaxed) == 0 {
        this_cpu().slice.store(TIME_SLICE, Ordering::Relaxed);
        yield_now();
    }
}

/// Run ready tasks; otherwise wait for an interrupt.  Becomes this CPU's idle
/// task; never returns.
pub fn idle_loop() -> ! {
    loop {
        kor::irq::enable();
        reap();
        yield_now();
        kor::arch::current().wait_for_interrupt();
    }
}

/// First code every freshly spawned task runs.
extern "C" fn task_bootstrap() -> ! {
    finish_switch();
    kor::irq::enable();
    let entry = current().entry;
    entry();
    exit();
}

// ---------------------------------------------------------------------------
// Blocking synchronisation primitives
// ---------------------------------------------------------------------------

/// A queue of tasks blocked waiting for an event.
///
/// Uses a raw spin lock that is *held across the context switch* when a task
/// blocks and released by [`finish_switch`] after the task is enqueued.  A
/// waker acquiring the same lock therefore cannot miss a task that has decided
/// to block (no lost wakeup), and the enqueue-after-save ordering means a woken
/// task is never run on another CPU before its context is saved.
pub struct WaitQueue {
    lock: AtomicBool,
    list: UnsafeCell<VecDeque<Arc<Task>>>,
}

// SAFETY: all access to `list` is serialised by the raw `lock`.
unsafe impl Sync for WaitQueue {}
unsafe impl Send for WaitQueue {}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            lock: AtomicBool::new(false),
            list: UnsafeCell::new(VecDeque::new()),
        }
    }

    fn raw_lock(&self) {
        while self.lock.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
    }

    fn raw_unlock(&self) {
        self.lock.store(false, Ordering::Release);
    }

    /// Block the current task on this queue.  The queue's raw lock **must be
    /// held** by the caller; it is released (after the task is enqueued) by
    /// `finish_switch`.  Interrupts must be disabled.
    fn block_locked(&self) {
        schedule(TO_WAIT, self as *const _ as usize);
    }

    /// Wake one blocked task, if any.
    pub fn wake_one(&self) {
        kor::irq::without(|| {
            self.raw_lock();
            let task = unsafe { (*self.list.get()).pop_front() };
            self.raw_unlock();
            if let Some(task) = task {
                make_ready(task);
            }
        });
    }

    /// Wake all blocked tasks.
    pub fn wake_all(&self) {
        kor::irq::without(|| {
            self.raw_lock();
            let mut drained = VecDeque::new();
            core::mem::swap(unsafe { &mut *self.list.get() }, &mut drained);
            self.raw_unlock();
            for task in drained {
                make_ready(task);
            }
        });
    }
}

/// A counting semaphore with blocking `wait`.
pub struct Semaphore {
    wq: WaitQueue,
    count: UnsafeCell<isize>,
}

// SAFETY: `count` is only accessed under `wq`'s raw lock.
unsafe impl Sync for Semaphore {}
unsafe impl Send for Semaphore {}

impl Semaphore {
    pub const fn new(initial: isize) -> Self {
        Self { wq: WaitQueue::new(), count: UnsafeCell::new(initial) }
    }

    /// Decrement the count, blocking while it is zero.
    pub fn wait(&self) {
        let enabled = kor::irq::is_enabled();
        kor::irq::disable();
        loop {
            self.wq.raw_lock();
            let count = unsafe { &mut *self.count.get() };
            if *count > 0 {
                *count -= 1;
                self.wq.raw_unlock();
                break;
            }
            // Block: `finish_switch` enqueues us and releases the queue lock,
            // so a concurrent `post` cannot slip between the check and the
            // block.
            self.wq.block_locked();
            // Resumed after a wake; re-check the count.
        }
        if enabled {
            kor::irq::enable();
        }
    }

    /// Increment the count, waking one waiter.
    pub fn post(&self) {
        kor::irq::without(|| {
            self.wq.raw_lock();
            let count = unsafe { &mut *self.count.get() };
            *count += 1;
            let task = unsafe { (*self.wq.list.get()).pop_front() };
            self.wq.raw_unlock();
            if let Some(task) = task {
                make_ready(task);
            }
        });
    }
}

/// A blocking mutual-exclusion lock built on a binary semaphore.
pub struct Mutex2 {
    sem: Semaphore,
}

impl Default for Mutex2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Mutex2 {
    pub const fn new() -> Self {
        Self { sem: Semaphore::new(1) }
    }

    pub fn lock(&self) {
        self.sem.wait();
    }

    pub fn unlock(&self) {
        self.sem.post();
    }
}


pub mod sync;
