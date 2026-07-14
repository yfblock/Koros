//! Linux process state layered over kor-sched kernel tasks.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::string::{String, ToString};
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
    pub clear_child_tid: usize,
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
            mmap_next: default_mmap_base(),
            executable: "/bin/busybox".to_string(),
            clear_child_tid: 0,
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

    pub fn activate_address_space(&self) {
        if let Some(address_space) = &self.address_space {
            address_space.activate();
        }
    }

    /// Install an already-active replacement and return the old owned space.
    pub fn replace_address_space(&mut self, address_space: AddressSpace) -> Option<AddressSpace> {
        assert!(address_space.is_active(), "replacement address space must be active");
        kor_sched::current().set_address_space_root(address_space.root());
        self.address_space.replace(address_space)
    }

}
fn default_mmap_base() -> usize {
    if cfg!(target_arch = "riscv64") { 0x20_0000_0000 }
    else if cfg!(target_arch = "aarch64") { 0x80_0000_0000 }
    else { 0x8_0000_0000 }
}

static NEXT_PID: Mutex<usize> = Mutex::new(1);
static PROCESSES: Mutex<BTreeMap<usize, Arc<Mutex<ProcessState>>>> = Mutex::new(BTreeMap::new());
static TASK_PROCESSES: Mutex<BTreeMap<usize, usize>> = Mutex::new(BTreeMap::new());

/// Initialise the kernel task scheduler and adopt the boot address space for init.
pub fn init() {
    kor_sched::init();
    PROCESSES.lock().insert(
        1,
        Arc::new(Mutex::new(ProcessState::new(1, 0, AddressSpace::adopt_active()))),
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
        let root = process.address_space().expect("process has no address space").root();
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


pub fn matching_child(parent: usize, requested: isize) -> Option<Arc<Mutex<ProcessState>>> {
    PROCESSES.lock().values().find(|process| {
        let process = process.lock();
        process.parent == parent && (requested == -1 || requested > 0 && process.pid == requested as usize)
    }).cloned()
}

pub fn matching_zombie(parent: usize, requested: isize) -> Option<Arc<Mutex<ProcessState>>> {
    PROCESSES.lock().values().find(|process| {
        let process = process.lock();
        process.parent == parent
            && process.state == STATE_ZOMBIE
            && (requested == -1 || requested > 0 && process.pid == requested as usize)
    }).cloned()
}

pub fn child_wait_queue(pid: usize) -> Option<Arc<kor_sched::Semaphore>> {
    Some(process(pid)?.lock().child_events.clone())
}

pub fn reap(pid: usize) -> Option<Arc<Mutex<ProcessState>>> {
    PROCESSES.lock().remove(&pid)
}

/// Mark the current process dead, notify its parent, and terminate its task.
pub fn exit_current(code: i32) -> ! {
    let task_id = kor_sched::current().id;
    let current = current_process().expect("exiting task is not a process");
    let (parent, clear_child_tid, address_space_root) = {
        let mut process = current.lock();
        process.exit_code = code;
        process.state = STATE_ZOMBIE;
        (process.parent, process.clear_child_tid, process.address_space().map(|space| space.root()))
    };
    if clear_child_tid != 0 {
        if let Some(root) = address_space_root {
            let old = kor::arch::current().address_space_root();
            if old != root { unsafe { kor::arch::current().activate_address_space(root) }; }
            if let Some(paddr) = kor::arch::current().translate(clear_child_tid) {
                unsafe { *(kor::arch::current().phys_to_virt(paddr) as *mut u32) = 0; }
            }
        }
        crate::syscall::wake_futex(clear_child_tid, 1);
    }
    TASK_PROCESSES.lock().remove(&task_id);
    if let Some(parent) = process(parent) {
        parent.lock().child_events.post();
        let mut parent = parent.lock();
        parent.signals.enqueue(17);
        parent.signal_events.post();
    }
    kor_sched::exit()
}
