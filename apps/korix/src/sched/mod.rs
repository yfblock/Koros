//! Linux process state layered over kor-sched kernel tasks.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use spin::Mutex;

use crate::mm::AddressSpace;

pub const STATE_RUNNING: usize = 0;
pub const STATE_ZOMBIE: usize = 1;

pub struct ProcessState {
    pub pid: usize,
    pub parent: usize,
    address_space: Option<AddressSpace>,
    pub exit_code: i32,
    pub state: usize,
    child_events: Arc<kor_sched::Semaphore>,
    pub signal_events: Arc<kor_sched::Semaphore>,
    fork_context: Option<kor_arch::UserTrapContext>,
    pub signals: crate::syscall::SignalState,
    pub brk: usize,
    pub mmap_next: usize,
    pub executable: String,
    pub cwd: String,
    pub clear_child_tid: usize,
    pub files: BTreeMap<usize, crate::syscall::SharedFile>,
    pub stdio: BTreeMap<usize, usize>,
    pub sockets: BTreeMap<usize, usize>,
    pub pipes: BTreeMap<usize, (usize, bool)>,
    /// File descriptors created through AF_INET6; transport remains IPv4-only.
    pub ipv6_fds: BTreeMap<usize, ()>,
    pub robust_list: usize,
    pub robust_list_len: usize,
    pub name: [u8; 16],
}

impl ProcessState {
    fn new(pid: usize, parent: usize, address_space: AddressSpace) -> Self {
        Self {
            pid,
            parent,
            address_space: Some(address_space),
            exit_code: 0,
            state: STATE_RUNNING,
            child_events: Arc::new(kor_sched::Semaphore::new(0)),
            signal_events: Arc::new(kor_sched::Semaphore::new(0)),
            fork_context: None,
            signals: crate::syscall::SignalState::new(),
            brk: 0,
            mmap_next: crate::user::layout::MMAP_BASE,
            executable: "/bin/busybox".to_string(),
            cwd: "/".to_string(),
            clear_child_tid: 0,
            files: BTreeMap::new(),
            stdio: BTreeMap::new(),
            sockets: BTreeMap::new(),
            pipes: BTreeMap::new(),
            ipv6_fds: BTreeMap::new(),
            robust_list: 0,
            robust_list_len: 0,
            name: [0; 16],
        }
    }

    pub fn set_fork_context(&mut self, context: kor_arch::UserTrapContext) {
        self.fork_context = Some(context);
    }

    pub fn take_fork_context(&mut self) -> Option<kor_arch::UserTrapContext> {
        self.fork_context.take()
    }

    pub fn address_space(&self) -> Option<&AddressSpace> {
        self.address_space.as_ref()
    }

    pub fn address_space_mut(&mut self) -> Option<&mut AddressSpace> {
        self.address_space.as_mut()
    }

    pub fn activate_address_space(&self) {
        if let Some(address_space) = &self.address_space {
            address_space.activate();
        }
    }

    /// Install a replacement space as this task's active root, activating it
    /// if a preemptive switch raced the caller's own activate() call.
    pub fn replace_address_space(&mut self, address_space: AddressSpace) -> Option<AddressSpace> {
        kor_arch::irq_without(|| {
            if !address_space.is_active() {
                // SAFETY: the caller intends this space to be live now.
                unsafe { kor_mmu::activate_address_space(address_space.root()) };
            }
            kor_sched::current().set_address_space_root(address_space.root());
            self.address_space.replace(address_space)
        })
    }
}

static NEXT_PID: Mutex<usize> = Mutex::new(1);
static PROCESSES: Mutex<BTreeMap<usize, Arc<Mutex<ProcessState>>>> = Mutex::new(BTreeMap::new());
static TASK_PROCESSES: Mutex<BTreeMap<usize, usize>> = Mutex::new(BTreeMap::new());

/// Initialise the kernel task scheduler and adopt the boot address space for init.
pub fn init() {
    kor_sched::init();
    PROCESSES.lock().insert(
        1,
        Arc::new(Mutex::new(ProcessState::new(
            1,
            0,
            AddressSpace::adopt_active(),
        ))),
    );
}

pub fn alloc_process(parent: usize, address_space: AddressSpace) -> Arc<Mutex<ProcessState>> {
    let mut next = NEXT_PID.lock();
    *next += 1;
    let process = Arc::new(Mutex::new(ProcessState::new(*next, parent, address_space)));
    PROCESSES.lock().insert(*next, process.clone());
    process
}

/// Spawn a scheduler task for a process, publishing its association atomically
/// before the task can run on any CPU.
pub fn spawn_process(process: &Arc<Mutex<ProcessState>>, entry: fn()) -> usize {
    let (pid, root) = {
        let process = process.lock();
        let root = process
            .address_space()
            .expect("process has no address space")
            .root();
        (process.pid, root)
    };
    kor_sched::spawn_configured(entry, root, |task| {
        TASK_PROCESSES.lock().insert(task, pid);
    })
}

pub fn process(pid: usize) -> Option<Arc<Mutex<ProcessState>>> {
    PROCESSES.lock().get(&pid).cloned()
}

pub fn current_pid() -> Option<usize> {
    if !kor_sched::is_ready() {
        return None;
    }
    TASK_PROCESSES.lock().get(&kor_sched::current().id).copied()
}

pub fn current_process() -> Option<Arc<Mutex<ProcessState>>> {
    process(current_pid()?)
}

pub fn with_current_process<R>(f: impl FnOnce(&Mutex<ProcessState>) -> R) -> Option<R> {
    current_process().map(|process| f(&process))
}

pub fn current_process_arc() -> Option<Arc<Mutex<ProcessState>>> {
    current_process()
}

/// Find any child (running or zombie) of `parent` matching `requested`
/// (-1 = any, >0 = that pid). Used by wait4's ECHILD check.
pub fn matching_child(parent: usize, requested: isize) -> Option<Arc<Mutex<ProcessState>>> {
    PROCESSES
        .lock()
        .values()
        .find(|process| {
            let process = process.lock();
            process.parent == parent
                && (requested == -1 || requested > 0 && process.pid == requested as usize)
        })
        .cloned()
}

pub fn matching_zombie(parent: usize, requested: isize) -> Option<Arc<Mutex<ProcessState>>> {
    PROCESSES
        .lock()
        .values()
        .find(|process| {
            let process = process.lock();
            process.parent == parent
                && process.state == STATE_ZOMBIE
                && (requested == -1 || requested > 0 && process.pid == requested as usize)
        })
        .cloned()
}

pub fn child_wait_queue(pid: usize) -> Option<Arc<kor_sched::Semaphore>> {
    Some(process(pid)?.lock().child_events.clone())
}

pub fn reap(pid: usize) -> Option<Arc<Mutex<ProcessState>>> {
    PROCESSES.lock().remove(&pid)
}

/// Snapshot the process table for procfs: (pid, name, state, ppid).
pub fn process_snapshot() -> alloc::vec::Vec<(usize, [u8; 16], usize, usize)> {
    PROCESSES
        .lock()
        .values()
        .map(|process| {
            let process = process.lock();
            (process.pid, process.name, process.state, process.parent)
        })
        .collect()
}

/// Mark the current process dead, notify its parent, and terminate its task.
pub fn exit_current(code: i32) -> ! {
    let current = current_process().expect("exiting task is not a process");
    // A non-returning syscall leaves through the scheduler instead of the
    // architecture trap epilogue; do not retain a pointer into its old
    // CPU-local syscall stack after the task becomes a zombie.
    kor_sched::current().clear_user_context();
    let (pid, requested_parent, clear_child_tid, address_space_root) = {
        let mut process = current.lock();
        process.exit_code = code;
        process.state = STATE_ZOMBIE;
        process.files.clear();
        process.sockets.clear();
        let closed_pipes: alloc::vec::Vec<(usize, bool)> =
            process.pipes.values().copied().collect();
        process.pipes.clear();
        for (pipe, is_read) in closed_pipes {
            crate::syscall::pipe_release(pipe, is_read);
        }
        (
            process.pid,
            process.parent,
            process.clear_child_tid,
            process.address_space().map(|space| space.root()),
        )
    };
    if clear_child_tid != 0 {
        if let Some(root) = address_space_root {
            let old = kor_mmu::address_space_root();
            if old != root {
                unsafe { kor_mmu::activate_address_space(root) };
            }
            if let Some(paddr) = kor_mmu::active().translate(clear_child_tid) {
                unsafe {
                    *(kor_arch::phys_to_virt(paddr) as *mut u32) = 0;
                }
            }
        }
        crate::syscall::wake_futex(clear_child_tid, 1);
    }

    // The `current` Arc local was captured at function entry; this function
    // never returns, so without an explicit drop its destructor would never
    // run and the zombie's address space would leak.
    if crate::syscall::TRACE_SYSCALLS.load(core::sync::atomic::Ordering::Relaxed) {
        kor::println!(
            "[trace] exit pid={pid} refs-before-drop={}",
            alloc::sync::Arc::strong_count(&current)
        );
    }
    drop(current);

    if requested_parent != 0 && requested_parent != pid {
        let mut processes = PROCESSES.lock();
        if let Some(parent) = processes.get_mut(&requested_parent) {
            let mut parent = parent.lock();
            parent.child_events.post();
            parent.signals.enqueue(17);
            parent.signal_events.post();
        }
    }
    kor_sched::exit()
}
