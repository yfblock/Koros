//! Linux syscall compatibility layer.
//!
//! Syscall numbers follow the generic Linux syscall table (used by
//! riscv64, aarch64). ABI-specific numbering, argument layouts, and
//! native-only calls are confined to the `arch/` adapters — see
//! `arch/mod.rs` for the porting checklist.
//!
//! # Calling convention
//! - riscv64: ecall, a7=nr, a0-a5=args, a0=return
//! - aarch64: svc #0, x8=nr, x0-x5=args, x0=return
//! - x86_64:  syscall, rax=nr, rdi,rsi,rdx,r10,r8,r9=args, rax=return

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use kor::INode;
use spin::Mutex;
mod arch;
mod user_mem;

use user_mem::{
    check_range as checked_user_range, copy_from as copy_from_user, copy_to as copy_to_user,
    kernel_address as user_to_kernel, read as read_user, read_string as user_to_str,
    write as write_user,
};

// Linux syscall numbers (generic table, shared by riscv64/aarch64)
pub const SYS_READ: usize = 63;
pub const SYS_WRITE: usize = 64;
pub const SYS_OPENAT: usize = 56;
pub const SYS_CLOSE: usize = 57;
pub const SYS_MMAP: usize = 222;
pub const SYS_MUNMAP: usize = 215;
pub const SYS_BRK: usize = 214;
pub const SYS_EXIT: usize = 93;
pub const SYS_EXIT_GROUP: usize = 94;
pub const SYS_CLONE: usize = 220;
pub const SYS_EXECVE: usize = 221;
pub const SYS_WAIT4: usize = 260;
pub const SYS_GETPID: usize = 172;
pub const SYS_GETPPID: usize = 173;
pub const SYS_GETUID: usize = 174;
pub const SYS_GETGID: usize = 176;
pub const SYS_MOUNT: usize = 40;
pub const SYS_UMOUNT2: usize = 39;
pub const SYS_SYNC: usize = 81;
pub const SYS_NANOSLEEP: usize = 101;
pub const SYS_GETCWD: usize = 17;
pub const SYS_CHDIR: usize = 49;
pub const SYS_DUP: usize = 23;
pub const SYS_DUP2: usize = 33;
pub const SYS_FCNTL: usize = 25;
pub const SYS_DUP3: usize = 24;
pub const SYS_IOCTL: usize = 29;
pub const SYS_FSTAT: usize = 80;
pub const SYS_NEWFSTATAT: usize = 79;
pub const SYS_GETDENTS64: usize = 61;
pub const SYS_LSEEK: usize = 62;
pub const SYS_MKDIRAT: usize = 34;
pub const SYS_UNLINKAT: usize = 35;
pub const SYS_RENAMEAT: usize = 38;
pub const SYS_LINKAT: usize = 37;
pub const SYS_READLINKAT: usize = 78;
pub const SYS_SYMLINKAT: usize = 36;
pub const SYS_RENAMEAT2: usize = 276;
pub const SYS_RT_SIGACTION: usize = 134;
pub const SYS_RT_SIGPROCMASK: usize = 135;
pub const SYS_RT_SIGSUSPEND: usize = 133;
pub const SYS_RT_SIGRETURN: usize = 139;
pub const SYS_KILL: usize = 129;
pub const SYS_TGKILL: usize = 131;
pub const SYS_SET_TID_ADDRESS: usize = 96;
pub const SYS_SET_ROBUST_LIST: usize = 99;
pub const SYS_FUTEX: usize = 98;
pub const SYS_UNAME: usize = 160;
pub const SYS_SYSINFO: usize = 179;
pub const SYS_CLOCK_GETRES: usize = 114;
pub const SYS_CLOCK_NANOSLEEP: usize = 115;

pub const SYS_GETRANDOM: usize = 278;
pub const SYS_WRITEV: usize = 66;
pub const SYS_READV: usize = 65;
pub const SYS_MPROTECT: usize = 226;
pub const SYS_SOCKET: usize = 198;
pub const SYS_CONNECT: usize = 203;
pub const SYS_SENDTO: usize = 206;
pub const SYS_RECVFROM: usize = 207;
pub const SYS_GETSOCKNAME: usize = 204;
pub const SYS_GETPEERNAME: usize = 205;
pub const SYS_SETSOCKOPT: usize = 208;
pub const SYS_GETSOCKOPT: usize = 209;
pub const SYS_SENDMSG: usize = 211;
pub const SYS_RECVMSG: usize = 212;
pub const SYS_SHUTDOWN: usize = 210;
pub const SYS_REBOOT: usize = 142;
const SYS_FCHOWNAT: usize = 54;
const SYS_PSELECT6: usize = 72;
const SYS_PPOLL: usize = 73;
const SYS_UTIMENSAT: usize = 88;
const SYS_CLOCK_GETTIME: usize = 113;
const SYS_SETSID: usize = 157;
const SYS_GETTIMEOFDAY: usize = 169;
const SYS_TIME: usize = 201;
const SYS_MREMAP: usize = 216;

/// `AT_FDCWD` sentinel for `*at` syscalls (relative to the process cwd).
const AT_FDCWD: isize = -100;

// Kernel-side UART RX buffer (compensates for small hardware FIFO)
pub static RX_BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());

pub fn rx_fill() {
    // LoongArch64 (unsupported for korix) has no UART source compiled in;
    // every other target drains its console into the shared buffer.
    {
        let mut buf = RX_BUF.lock();
        while let Some(b) = kor::console::getchar() {
            buf.push(b);
        }
    }
}

/// Terminal attributes currently in effect for fd 0-2 (set via TCSETS*;
/// starts as the cooked defaults advertised by TCGETS).  The kernel honours
/// the input flags that user programs actually depend on (ICRNL).
static TERMIOS: Mutex<[u8; TERMIOS_LEN]> = Mutex::new([0; TERMIOS_LEN]);

fn termios() -> [u8; TERMIOS_LEN] {
    let mut t = TERMIOS.lock();
    if *t == [0u8; TERMIOS_LEN] {
        *t = cooked_termios();
    }
    *t
}

/// `struct termios` byte length: 4 flag words + c_line + c_cc[19].
const TERMIOS_LEN: usize = 36;

/// Cooked defaults: ICRNL | OPOST | ONLCR, ECHO | ICANON | ISIG family.
fn cooked_termios() -> [u8; TERMIOS_LEN] {
    const IFLAG: u32 = 0o400; // ICRNL
    const OFLAG: u32 = 0o1 | 0o4; // OPOST | ONLCR
    const CFLAG: u32 = 0o141;
    const LFLAG: u32 = 0o1 | 0o2 | 0o10 | 0o20 | 0o40 | 0o100000; // ISIG|ICANON|ECHO|ECHOE|ECHOK|IEXTEN
    let mut t = [0u8; TERMIOS_LEN];
    t[0..4].copy_from_slice(&IFLAG.to_ne_bytes());
    t[4..8].copy_from_slice(&OFLAG.to_ne_bytes());
    t[8..12].copy_from_slice(&CFLAG.to_ne_bytes());
    t[12..16].copy_from_slice(&LFLAG.to_ne_bytes());
    // c_cc: VINTR=3, VQUIT=28, VERASE=127, VKILL=21, VEOF=4, VTIME=0, VMIN=1
    t[17..24].copy_from_slice(&[3, 28, 127, 21, 4, 0, 1]);
    t
}

/// c_iflag bit: map CR (0x0d) to NL (0x0a) on input.
const ICRNL_FLAG: u32 = 0o400;

fn termios_icrnl() -> bool {
    let t = termios();
    u32::from_ne_bytes([t[0], t[1], t[2], t[3]]) & ICRNL_FLAG != 0
}

fn rx_get() -> Option<u8> {
    rx_fill();
    let mut buf = RX_BUF.lock();
    if buf.is_empty() {
        None
    } else {
        let byte = buf.remove(0);
        if byte == b'\r' && termios_icrnl() {
            Some(b'\n')
        } else {
            Some(byte)
        }
    }
}

fn rx_has_data() -> bool {
    rx_fill();
    !RX_BUF.lock().is_empty()
}

type FutexKey = (usize, usize);
static FUTEX_QUEUES: Mutex<
    alloc::collections::BTreeMap<FutexKey, alloc::sync::Arc<kor_sched::WaitQueue>>,
> = Mutex::new(alloc::collections::BTreeMap::new());

fn futex_queue(address: usize) -> alloc::sync::Arc<kor_sched::WaitQueue> {
    let key = (crate::sched::current_pid().unwrap_or(0), address);
    let mut queues = FUTEX_QUEUES.lock();
    queues
        .entry(key)
        .or_insert_with(|| alloc::sync::Arc::new(kor_sched::WaitQueue::new()))
        .clone()
}

pub(crate) fn wake_futex(address: usize, count: usize) -> usize {
    let queue = futex_queue(address);
    let mut woken = 0;
    while woken < count && queue.wake_one() {
        woken += 1;
    }
    woken
}
const SIGNAL_COUNT: usize = 64;
const SIGKILL: usize = 9;
const SIGSTOP: usize = 19;
const UNBLOCKABLE_MASK: u64 = (1 << (SIGKILL - 1)) | (1 << (SIGSTOP - 1));
const SIG_DFL: usize = 0;
const SIG_IGN: usize = 1;
const SA_RESTORER: usize = 0x0400_0000;
const SA_NODEFER: usize = 0x4000_0000;
const SA_RESETHAND: usize = 0x8000_0000;

#[derive(Clone, Copy)]
struct SignalAction {
    handler: usize,
    flags: usize,
    restorer: usize,
    mask: u64,
}

const EMPTY_SIGNAL_ACTION: SignalAction = SignalAction {
    handler: SIG_DFL,
    flags: 0,
    restorer: 0,
    mask: 0,
};

/// Linux signal dispositions, blocked mask, and standard-signal pending set.

#[cfg(target_arch = "riscv64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct UserSignalAction {
    handler: usize,
    flags: usize,
    restorer: usize,
    mask: u64,
}

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
type UserSignalAction = SignalAction;

fn kernel_signal_action(action: UserSignalAction) -> SignalAction {
    #[cfg(target_arch = "riscv64")]
    {
        SignalAction {
            handler: action.handler,
            flags: action.flags,
            restorer: action.restorer,
            mask: action.mask,
        }
    }
    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    {
        action
    }
}

fn user_signal_action(action: SignalAction) -> UserSignalAction {
    #[cfg(target_arch = "riscv64")]
    {
        UserSignalAction {
            handler: action.handler,
            flags: action.flags,
            restorer: action.restorer,
            mask: action.mask,
        }
    }
    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    {
        action
    }
}
pub struct SignalState {
    actions: [SignalAction; SIGNAL_COUNT + 1],
    blocked: u64,
    pending: u64,
}

impl SignalState {
    /// Construct empty signal state with all dispositions set to default.
    pub const fn new() -> Self {
        Self {
            actions: [EMPTY_SIGNAL_ACTION; SIGNAL_COUNT + 1],
            blocked: 0,
            pending: 0,
        }
    }

    /// Clone dispositions and mask for fork while clearing process-local pending signals.
    pub fn fork_clone(&self) -> Self {
        Self {
            actions: self.actions,
            blocked: self.blocked,
            pending: 0,
        }
    }
    pub fn enqueue(&mut self, signal: usize) {
        self.pending |= 1 << (signal - 1);
    }

    /// Reset caught dispositions for exec while preserving ignored dispositions and mask.
    pub fn reset_for_exec(&mut self) {
        for action in &mut self.actions[1..] {
            if action.handler != SIG_IGN {
                *action = EMPTY_SIGNAL_ACTION;
            }
        }
        self.pending = 0;
    }
}

impl Default for SignalState {
    fn default() -> Self {
        Self::new()
    }
}

const SIGINFO_SIZE: usize = 128;
const EFAULT: isize = -14;
const EINVAL: isize = -22;
const ESRCH: isize = -3;
const SOCK_AF_INET: usize = 2;
const SOCK_AF_INET6: usize = 10;
const SOCKADDR_IN_SIZE: usize = 16;
const SOCKADDR_IN6_SIZE: usize = 28;
const SOCKET_TYPE_MASK: usize = 0xff;
const IPV4_MAPPED_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff];

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxStack {
    sp: usize,
    flags: i32,
    _pad: u32,
    size: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxSigInfo {
    signo: i32,
    errno: i32,
    code: i32,
    _pad0: i32,
    sender_pid: i32,
    sender_uid: u32,
    _pad: [u8; 104],
}

const _: () = assert!(core::mem::size_of::<LinuxSigInfo>() == SIGINFO_SIZE);

#[cfg(target_arch = "riscv64")]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct LinuxMachineContext {
    regs: [usize; 32],
    fp_and_extensions: [u8; 528],
}

#[cfg(target_arch = "aarch64")]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct LinuxMachineContext {
    fault_address: usize,
    regs: [usize; 31],
    sp: usize,
    pc: usize,
    pstate: usize,
    _reserved_align: [u8; 8],
    reserved: [u8; 4096],
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxMachineContext {
    regs: [usize; 18],
    cs: u16,
    gs: u16,
    fs: u16,
    ss: u16,
    err: usize,
    trapno: usize,
    oldmask: usize,
    cr2: usize,
    fpstate: usize,
    reserved: [usize; 8],
}

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxUContext {
    flags: usize,
    link: usize,
    stack: LinuxStack,
    sigmask: u64,
    unused_mask: [u8; 120],
    mcontext: LinuxMachineContext,
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxUContext {
    flags: usize,
    link: usize,
    stack: LinuxStack,
    mcontext: LinuxMachineContext,
    sigmask: u64,
}

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct LinuxRtSignalFrame {
    info: LinuxSigInfo,
    context: LinuxUContext,
}

#[cfg(target_arch = "x86_64")]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct LinuxRtSignalFrame {
    restorer: usize,
    context: LinuxUContext,
    info: LinuxSigInfo,
}

static PROCESS_GROUP: Mutex<usize> = Mutex::new(1);

#[cfg(target_arch = "riscv64")]
fn machine_context(context: &kor_arch::UserTrapContext) -> LinuxMachineContext {
    let mut regs = context.regs;
    regs[0] = context.pc;
    LinuxMachineContext {
        regs,
        fp_and_extensions: [0; 528],
    }
}

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
const SIGNAL_RESTORER_ADDRESS: usize = crate::user::layout::SIGNAL_RESTORER;

#[cfg(target_arch = "riscv64")]
const SIGNAL_RESTORER_CODE: [u32; 3] = [0x08b0_0893, 0x0000_0073, 0x0000_006f];
#[cfg(target_arch = "aarch64")]
const SIGNAL_RESTORER_CODE: [u32; 3] = [0xd280_1168, 0xd400_0001, 0x1400_0000];

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
fn ensure_signal_restorer() -> Option<usize> {
    let page = kor_mmu::active()
        .lock(SIGNAL_RESTORER_ADDRESS..SIGNAL_RESTORER_ADDRESS + 0x1000)
        .query(SIGNAL_RESTORER_ADDRESS);
    if let kor_mmu::PageStatus::Mapped { flags, .. } = page {
        if flags.contains(kor_arch::MappingFlags::U | kor_arch::MappingFlags::X) {
            return Some(SIGNAL_RESTORER_ADDRESS);
        }
        // A non-user placeholder (e.g. an exec-time span) covers the page;
        // replace it with a user-executable restorer mapping.
        let _ = kor_mmu::active().unmap(SIGNAL_RESTORER_ADDRESS, kor_arch::MapSize::Page4K);
    }
    let physical = kor::frame::alloc_frame()?;
    let flags = kor_arch::MappingFlags::U | kor_arch::MappingFlags::R | kor_arch::MappingFlags::X;
    if kor_mmu::active()
        .map(
            SIGNAL_RESTORER_ADDRESS,
            physical,
            flags,
            kor_arch::MapSize::Page4K,
        )
        .is_err()
    {
        kor::frame::free_frames(physical, 1);
        return None;
    }
    let kernel = kor_arch::phys_to_virt(physical);
    unsafe {
        core::ptr::write_bytes(kernel as *mut u8, 0, 0x1000);
        core::ptr::copy_nonoverlapping(
            SIGNAL_RESTORER_CODE.as_ptr(),
            kernel as *mut u32,
            SIGNAL_RESTORER_CODE.len(),
        );
    }
    #[cfg(target_arch = "riscv64")]
    unsafe {
        core::arch::asm!("fence.i")
    };
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("ic ivau, {}", "dsb ish", "isb", in(reg) SIGNAL_RESTORER_ADDRESS)
    };
    Some(SIGNAL_RESTORER_ADDRESS)
}

fn signal_restorer(action: SignalAction) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    {
        (action.flags & SA_RESTORER != 0 && action.restorer != 0).then_some(action.restorer)
    }
    #[cfg(target_arch = "aarch64")]
    {
        if action.flags & SA_RESTORER != 0 {
            (action.restorer != 0).then_some(action.restorer)
        } else {
            ensure_signal_restorer()
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        let _ = action;
        ensure_signal_restorer()
    }
}

#[cfg(target_arch = "riscv64")]
fn trap_context(machine: &LinuxMachineContext) -> Option<kor_arch::UserTrapContext> {
    let mut regs = machine.regs;
    let pc = regs[0];
    regs[0] = 0;
    Some(kor_arch::UserTrapContext {
        regs,
        pc,
        status: 0,
        sp: regs[2],
        tls: regs[4],
        fp_callee: [0; 12],
        fcsr: 0,
    })
}

#[cfg(target_arch = "aarch64")]
fn machine_context(context: &kor_arch::UserTrapContext) -> LinuxMachineContext {
    let mut regs = [0usize; 31];
    regs.copy_from_slice(&context.regs[..31]);
    LinuxMachineContext {
        fault_address: 0,
        regs,
        sp: context.sp,
        pc: context.pc,
        pstate: context.status,
        _reserved_align: [0; 8],
        reserved: [0; 4096],
    }
}

#[cfg(target_arch = "aarch64")]
fn trap_context(machine: &LinuxMachineContext) -> Option<kor_arch::UserTrapContext> {
    let mut regs = [0usize; 32];
    regs[..31].copy_from_slice(&machine.regs);
    Some(kor_arch::UserTrapContext {
        regs,
        pc: machine.pc,
        status: machine.pstate,
        sp: machine.sp,
        tls: 0,
        fp_callee: [0; 12],
        fcsr: 0,
    })
}

#[cfg(target_arch = "x86_64")]
fn machine_context(context: &kor_arch::UserTrapContext) -> LinuxMachineContext {
    let mut regs = [0usize; 18];
    regs[..15].copy_from_slice(&context.regs[..15]);
    regs[15] = context.sp;
    regs[16] = context.pc;
    regs[17] = context.status;
    LinuxMachineContext {
        regs,
        cs: 0x1b,
        gs: 0,
        fs: 0,
        ss: 0x23,
        err: 0,
        trapno: 0,
        oldmask: 0,
        cr2: 0,
        fpstate: 0,
        reserved: [0; 8],
    }
}

#[cfg(target_arch = "x86_64")]
fn trap_context(machine: &LinuxMachineContext) -> Option<kor_arch::UserTrapContext> {
    if machine.cs != 0x1b || machine.ss != 0x23 {
        return None;
    }
    let mut regs = [0usize; 32];
    regs[..15].copy_from_slice(&machine.regs[..15]);
    Some(kor_arch::UserTrapContext {
        regs,
        pc: machine.regs[16],
        status: machine.regs[17],
        sp: machine.regs[15],
        tls: 0,
        fp_callee: [0; 12],
        fcsr: 0,
    })
}

fn make_ucontext(context: &kor_arch::UserTrapContext, mask: u64) -> LinuxUContext {
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    {
        LinuxUContext {
            flags: 0,
            link: 0,
            stack: LinuxStack {
                sp: 0,
                flags: 0,
                _pad: 0,
                size: 0,
            },
            sigmask: mask,
            unused_mask: [0; 120],
            mcontext: machine_context(context),
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        LinuxUContext {
            flags: 6,
            link: 0,
            stack: LinuxStack {
                sp: 0,
                flags: 0,
                _pad: 0,
                size: 0,
            },
            mcontext: machine_context(context),
            sigmask: mask,
        }
    }
}

fn signal_frame_address(context: &kor_arch::UserTrapContext) -> Option<usize> {
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    let address = context.sp;
    #[cfg(target_arch = "x86_64")]
    let address = context.sp.checked_sub(core::mem::size_of::<usize>())?;
    Some(address)
}

fn restored_syscall_result(context: &kor_arch::UserTrapContext) -> isize {
    #[cfg(target_arch = "riscv64")]
    {
        context.regs[10] as isize
    }
    #[cfg(target_arch = "aarch64")]
    {
        context.regs[0] as isize
    }
    #[cfg(target_arch = "x86_64")]
    {
        context.regs[13] as isize
    }
}
static SESSION_ID: Mutex<usize> = Mutex::new(1);

/// Internal pseudo-number block (500+, outside the generic ABI range):
/// adapters emit these for ABI-specific calls with no generic equivalent;
/// they never arrive from user space directly.
pub const SYS_ARCH_PRCTL: usize = 500;

// LTP-critical syscall numbers (riscv64 asm-generic).
// riscv64 has no legacy `pipe` (59) — it is `pipe2`.
pub const SYS_PIPE2: usize = 59;
pub const SYS_PREAD64: usize = 67;
pub const SYS_PWRITE64: usize = 68;
pub const SYS_POLL: usize = 7;
pub const SYS_SENDFILE: usize = 71;
pub const SYS_FCHDIR: usize = 50;
pub const SYS_FSYNC: usize = 82;
pub const SYS_FTRUNCATE: usize = 46;
pub const SYS_TRUNCATE: usize = 45;
pub const SYS_FCHMOD: usize = 52;
pub const SYS_FCHMODAT: usize = 53;
pub const SYS_FCHOWN: usize = 55;
// riscv64 has no `rmdir` (84) — use unlinkat with AT_REMOVEDIR.
pub const SYS_GETPGID: usize = 155;
pub const SYS_SETPGID: usize = 154;
pub const SYS_WAITID: usize = 95;
pub const SYS_SCHED_YIELD: usize = 124;
pub const SYS_RT_SIGTIMEDWAIT: usize = 137;
pub const SYS_STATFS: usize = 43;
pub const SYS_FSTATFS: usize = 44;
pub const SYS_MINCORE: usize = 232;
pub const SYS_MADVISE: usize = 233;
pub const SYS_SETITIMER: usize = 103;
pub const SYS_GETITIMER: usize = 102;
const _SYS_ALARM_UNUSED: () = ();
pub const SYS_GETRUSAGE: usize = 165;
pub const SYS_SOCKETPAIR: usize = 199;
pub const SYS_READLINK: usize = 78; // same as READLINKAT on riscv64

/// Dispatch a syscall.
/// nr is the syscall number, args is up to 6 arguments.
/// Returns the syscall result (positive = success, negative = -errno).
///
/// This entry is deliberately thin and arch-neutral: the per-ABI adapter
/// (`arch::current`) first translates `(nr, args)` into canonical
/// asm-generic form; `dispatch_inner` and every handler below are portable.
pub fn dispatch(nr: usize, args: &[usize]) -> isize {
    LAST_SYSCALL.store(nr, core::sync::atomic::Ordering::Relaxed);
    let mut canonical = [0usize; 6];
    for (slot, value) in canonical.iter_mut().zip(args.iter()) {
        *slot = *value;
    }
    let result = match arch::current::translate(nr, &mut canonical) {
        Some(canonical_nr) => dispatch_inner(canonical_nr, &canonical),
        None => -38, // -ENOSYS
    };
    if TRACE_SYSCALLS.load(core::sync::atomic::Ordering::Relaxed) {
        kor::println!("[trace] nr={nr} -> {result}");
    }
    result
}

/// Toggle full syscall tracing from the kernel cmdline (`ltptrace`).
pub static TRACE_SYSCALLS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Last dispatched syscall number — diagnostic aid for user-fault triage.
pub static LAST_SYSCALL: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

fn dispatch_inner(num: usize, args: &[usize]) -> isize {
    match num {
        SYS_WRITE => sys_write(args[0], args[1], args[2]),
        SYS_READ => sys_read(args[0], args[1], args[2]),
        SYS_WRITEV => {
            let r = sys_writev(args[0], args[1], args[2]);
            r
        }
        SYS_EXIT | SYS_EXIT_GROUP => sys_exit_fork(args[0]),
        SYS_GETPID => current_pid() as isize,
        SYS_GETPPID => crate::sched::current_process()
            .map(|process| process.lock().parent as isize)
            .unwrap_or(0),
        155 => *PROCESS_GROUP.lock() as isize,
        SYS_SETPGID => {
            // Join-or-create own process group: single global group in korix.
            let pid = current_pid();
            *PROCESS_GROUP.lock() = pid;
            0
        }
        SYS_GETUID | 175 => 0, // GETUID | GETEUID
        SYS_GETGID | 177 => 0, // GETGID | GETEGID
        SYS_PIPE2 => sys_pipe2(args[0], args[1]),
        SYS_PREAD64 => sys_pread64(args[0], args[1], args[2], args[3]),
        SYS_PWRITE64 => sys_pwrite64(args[0], args[1], args[2], args[3]),
        SYS_POLL => sys_poll(args[0], args[1], args[2]),
        SYS_SENDFILE => sys_sendfile(args[0], args[1], args[2], args[3]),
        SYS_FCHDIR => sys_fchdir(args[0]),
        SYS_FSYNC => sys_fsync(args[0]),
        SYS_FTRUNCATE => sys_ftruncate(args[0], args[1] as i64),
        SYS_TRUNCATE => sys_truncate(args[0], args[1] as i64),
        SYS_FCHMOD => sys_fchmod(args[0], args[1]),
        // Classic fchmodat(53) has no flags argument (only fchmodat2 does);
        // musl leaves a3 holding garbage, so do not validate it.
        SYS_FCHMODAT => sys_fchmodat(args[0], args[1], args[2], 0),
        SYS_FCHOWN => 0, // stub: chown to self
        SYS_WAITID => sys_waitid(args[0], args[1], args[2], args[3] | args[4]),
        SYS_SCHED_YIELD => {
            kor_sched::yield_now();
            0
        }
        SYS_RT_SIGTIMEDWAIT => sys_sigtimedwait(args[0], args[1], args[2]),
        SYS_STATFS => sys_statfs(args[0], args[1]),
        SYS_FSTATFS => sys_fstatfs(args[0], args[1]),
        SYS_MADVISE => 0, // no-op
        SYS_MINCORE => 0, // no-op: all pages resident
        SYS_SETITIMER => sys_setitimer(args[0], args[1], args[2]),
        SYS_GETITIMER => sys_getitimer(args[0], args[1]),
        SYS_GETRUSAGE => sys_getrusage(args[0], args[1]),
        SYS_SYNC => {
            if kor_fs::mount::sync_all(&crate::MOUNTS).is_ok() {
                0
            } else {
                -5
            }
        }
        SYS_NANOSLEEP => sys_nanosleep(args[0]),
        SYS_UNAME => sys_uname(args[0]),
        SYS_SYSINFO => sys_sysinfo(args[0]),
        SYS_BRK => sys_brk(args[0]),
        SYS_MMAP => sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5] as i64),
        SYS_MUNMAP => sys_munmap(args[0], args[1]),
        SYS_MPROTECT => sys_mprotect(args[0], args[1], args[2]),
        SYS_GETRANDOM => sys_getrandom(args[0], args[1]),
        SYS_GETCWD => sys_getcwd(args[0], args[1]),
        SYS_GETDENTS64 => sys_getdents64(args[0], args[1], args[2]),
        SYS_RENAMEAT2 => {
            if args[4] == 0 {
                sys_renameat(args[0], args[1], args[2], args[3])
            } else {
                -22
            }
        }
        SYS_OPENAT => sys_openat(args[0], args[1], args[2], args[3]),
        SYS_CLOSE => sys_close(args[0]),
        SYS_FSTAT => sys_fstat(args[0], args[1]),
        SYS_NEWFSTATAT => sys_fstatat(args[0], args[1], args[2], args[3]),
        SYS_LSEEK => sys_lseek(args[0], args[1] as i64, args[2]),
        SYS_MKDIRAT => sys_mkdirat(args[0], args[1], args[2]),
        SYS_UNLINKAT => sys_unlinkat(args[0], args[1], args[2]),
        SYS_RENAMEAT => sys_renameat(args[0], args[1], args[2], args[3]),
        SYS_LINKAT => sys_linkat(args[0], args[1], args[2], args[3], args[4]),
        SYS_SYMLINKAT => sys_symlinkat(args[0], args[1], args[2]),
        SYS_RT_SIGACTION => sys_rt_sigaction(args[0], args[1], args[2]),
        SYS_RT_SIGSUSPEND => sys_rt_sigsuspend(args[0]),
        SYS_RT_SIGPROCMASK => sys_rt_sigprocmask(args[0], args[1], args[2]),
        SYS_RT_SIGRETURN => sys_rt_sigreturn(),
        SYS_SET_TID_ADDRESS => 1,
        SYS_SET_ROBUST_LIST => sys_set_robust_list(args[0], args[1]),
        SYS_FUTEX => sys_futex(args[0], args[1], args[2], args[3]),
        200 => 0,
        SYS_IOCTL => sys_ioctl(args[0], args[1], args[2]),
        SYS_SOCKET => sys_socket(args[0], args[1], args[2]),
        SYS_CONNECT => sys_connect(args[0], args[1], args[2]),
        SYS_SENDTO => sys_sendto(args[0], args[1], args[2], args[4], args[5]),
        SYS_SENDMSG => sys_sendmsg(args[0], args[1]),
        SYS_RECVMSG => sys_recvmsg(args[0], args[1]),
        SYS_RECVFROM => sys_recvfrom(args[0], args[1], args[2], args[4], args[5]),
        SYS_GETSOCKNAME | SYS_GETPEERNAME => sys_socket_name(args[0], args[1], args[2]),
        SYS_SETSOCKOPT => 0,
        SYS_GETSOCKOPT => sys_getsockopt(args[3], args[4]),
        SYS_SHUTDOWN => 0,
        SYS_REBOOT => sys_reboot(args[0], args[1], args[2]),
        SYS_FCNTL => sys_fcntl(args[0], args[1], args[2]),
        SYS_DUP => sys_dup(args[0]),
        SYS_DUP2 => sys_dup2(args[0], args[1]),
        SYS_DUP3 => sys_dup2(args[0], args[1]),
        SYS_CLONE => sys_clone(args),
        SYS_EXECVE => sys_execve(args[0], args[1]),
        SYS_WAIT4 => sys_wait4(args[0], args[1], args[2]),
        SYS_KILL => sys_kill(args[0], args[1]),
        SYS_TGKILL => sys_kill(args[1], args[2]),
        SYS_CHDIR => sys_chdir(args[0]),
        SYS_READLINKAT => sys_readlinkat(args[0], args[1], args[2], args[3]),
        SYS_MOUNT | SYS_UMOUNT2 => -38,
        SYS_MREMAP => sys_mremap(args[0], args[1], args[2], args[3]),
        SYS_FCHOWNAT | SYS_UTIMENSAT => 0,
        SYS_CLOCK_GETTIME => sys_clock_gettime(args[0], args[1]),
        SYS_CLOCK_GETRES => sys_clock_getres(args[0], args[1]),
        SYS_CLOCK_NANOSLEEP => sys_clock_nanosleep(args[0], args[1], args[2], args[3]),
        SYS_SETSID => {
            let pid = current_pid();
            *SESSION_ID.lock() = pid;
            *PROCESS_GROUP.lock() = pid;
            pid as isize
        }
        SYS_GETTIMEOFDAY => sys_gettimeofday(args[0]),
        SYS_TIME => sys_time(args[0]),
        SYS_PPOLL => sys_ppoll(args[0], args[1], args[2]),
        SYS_PSELECT6 => sys_select(args[0], args[1], args[2], args[3], args[4]),

        167 => sys_prctl(args),
        149 => -38, // sched_getaffinity
        158 => 0,   // getgroups: no supplementary groups
        156 => *SESSION_ID.lock() as isize,
        144 | 143 | 148 => -38, // scheduler mutation is unsupported
        145 => 0,               // sched_getscheduler: SCHED_OTHER
        146 => -38,             // sched_getparam requires an output parameter
        32 => 0,                // flock: package database access is serialized by this kernel
        178 => current_pid() as isize,
        3 => -38, // io_cancel
        0 => -38, // io_setup
        2 => -2,  // legacy open
        293 => 0, // rseq: single-threaded userspace needs no restartable sequence area
        294 => 0, // flock: single-kernel package database access is serialized
        295 => {
            let path = match user_absolute_path(args[0], args[1]) {
                Ok(path) => path,
                Err(error) => return error,
            };
            match crate::fs::resolve(&path) {
                Ok(_) => 0,
                Err(error) => fs_errno(error),
            }
        }
        296 => {
            let path = match user_absolute_path(AT_FDCWD as usize, args[0]) {
                Ok(path) => path,
                Err(error) => return error,
            };
            match crate::fs::resolve(&path) {
                Ok(_) => 0,
                Err(error) => fs_errno(error),
            }
        }
        297 => {
            let path = match user_absolute_path(AT_FDCWD as usize, args[0]) {
                Ok(path) => path,
                Err(error) => return error,
            };
            let inode = match crate::fs::resolve(&path) {
                Ok(inode) => inode,
                Err(error) => return fs_errno(error),
            };
            let name = match user_to_str(args[1], 256) {
                Some(name) => name,
                None => return -14,
            };
            let value = match inode.getxattr(&name) {
                Ok(value) => value,
                Err(kor::FsError::NotFound) => return -61,
                Err(error) => return fs_errno(error),
            };
            if args[3] == 0 {
                value.len() as isize
            } else if args[3] < value.len() {
                -34
            } else if copy_to_user(args[2], &value).is_some() {
                value.len() as isize
            } else {
                -14
            }
        }
        298 => 0,
        299 => sys_select(args[0], args[1], args[2], args[3], 0),
        261 => {
            // prlimit64 → return default limits
            if args[3] != 0 {
                let rlim = [u64::MAX, u64::MAX];
                if write_user(args[3], &rlim).is_none() {
                    return -14;
                }
            }
            0
        }
        // ABI-specific calls routed through the internal pseudo-numbers.
        SYS_ARCH_PRCTL => arch::current::native_syscall(num, args),
        _ => -38,
    }
}

fn read_sockaddr(address: usize, length: usize) -> Result<([u8; 4], u16), isize> {
    const AF_INET: u16 = SOCK_AF_INET as u16;
    const AF_INET6: u16 = SOCK_AF_INET6 as u16;

    let family = read_user::<u16>(address).ok_or(-14isize)?;
    let port = u16::from_be_bytes([
        read_user::<u8>(address + 2).ok_or(-14isize)?,
        read_user::<u8>(address + 3).ok_or(-14isize)?,
    ]);
    match family {
        AF_INET => {
            if length < SOCKADDR_IN_SIZE {
                return Err(-22);
            }
            let raw = read_user::<[u8; SOCKADDR_IN_SIZE]>(address).ok_or(-14isize)?;
            Ok(([raw[4], raw[5], raw[6], raw[7]], port))
        }
        AF_INET6 => {
            if length < SOCKADDR_IN6_SIZE {
                return Err(-22);
            }
            let raw = read_user::<[u8; SOCKADDR_IN6_SIZE]>(address).ok_or(-14isize)?;
            if raw[8..20] != IPV4_MAPPED_PREFIX {
                return Err(-101);
            }
            Ok(([raw[20], raw[21], raw[22], raw[23]], port))
        }
        _ => Err(-97),
    }
}

fn write_sockaddr(
    address: usize,
    length: usize,
    ip: [u8; 4],
    port: u16,
    ipv6: bool,
) -> Result<usize, isize> {
    const AF_INET: u16 = SOCK_AF_INET as u16;
    const AF_INET6: u16 = SOCK_AF_INET6 as u16;

    if address == 0 || length == 0 {
        return Ok(0);
    }
    if ipv6 {
        if length < SOCKADDR_IN6_SIZE {
            return Err(-22);
        }
        let mut raw = [0u8; SOCKADDR_IN6_SIZE];
        raw[..2].copy_from_slice(&AF_INET6.to_ne_bytes());
        raw[2..4].copy_from_slice(&port.to_be_bytes());
        raw[8..20].copy_from_slice(&IPV4_MAPPED_PREFIX);
        raw[20..24].copy_from_slice(&ip);
        copy_to_user(address, &raw).ok_or(-14isize)?;
        Ok(SOCKADDR_IN6_SIZE)
    } else {
        if length < SOCKADDR_IN_SIZE {
            return Err(-22);
        }
        let mut raw = [0u8; SOCKADDR_IN_SIZE];
        raw[..2].copy_from_slice(&AF_INET.to_ne_bytes());
        raw[2..4].copy_from_slice(&port.to_be_bytes());
        raw[4..8].copy_from_slice(&ip);
        copy_to_user(address, &raw).ok_or(-14isize)?;
        Ok(SOCKADDR_IN_SIZE)
    }
}

fn sys_socket(domain: usize, socket_type: usize, _protocol: usize) -> isize {
    if !matches!(domain, SOCK_AF_INET | SOCK_AF_INET6) {
        return -97;
    }
    let socket = match crate::net::socket((socket_type & SOCKET_TYPE_MASK) as i32) {
        Ok(socket) => socket,
        Err(error) => return error,
    };
    let Some(process) = crate::sched::current_process() else {
        let _ = crate::net::close(socket);
        return -24;
    };
    let mut process = process.lock();
    let fd = next_fd(&process);
    process.sockets.insert(fd, socket);
    if domain == SOCK_AF_INET6 {
        process.ipv6_fds.insert(fd, ());
    }
    fd as isize
}

fn socket_id(fd: usize) -> Result<usize, isize> {
    crate::sched::current_process()
        .and_then(|process| process.lock().sockets.get(&fd).copied())
        .ok_or(-9)
}

fn socket_is_ipv6(fd: usize) -> bool {
    crate::sched::current_process().is_some_and(|process| process.lock().ipv6_fds.contains_key(&fd))
}

fn sys_connect(fd: usize, address: usize, length: usize) -> isize {
    let (ip, port) = match read_sockaddr(address, length) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let socket = match socket_id(fd) {
        Ok(socket) => socket,
        Err(error) => return error,
    };
    match crate::net::connect(socket, ip, port) {
        Ok(()) => 0,
        Err(error) => error,
    }
}

fn sys_sendto(
    fd: usize,
    buffer: usize,
    length: usize,
    address: usize,
    address_length: usize,
) -> isize {
    let socket = match socket_id(fd) {
        Ok(socket) => socket,
        Err(error) => return error,
    };
    let mut data = alloc::vec![0u8; length];
    if copy_from_user(buffer, &mut data).is_none() {
        return -14;
    }
    let destination = if address == 0 {
        None
    } else {
        match read_sockaddr(address, address_length) {
            Ok(value) => Some(value),
            Err(error) => return error,
        }
    };
    match crate::net::send(socket, &data, destination) {
        Ok(count) => count as isize,
        Err(error) => error,
    }
}

fn sys_recvfrom(
    fd: usize,
    buffer: usize,
    length: usize,
    address: usize,
    address_length: usize,
) -> isize {
    let socket = match socket_id(fd) {
        Ok(socket) => socket,
        Err(error) => return error,
    };
    let mut data = alloc::vec![0u8; length];
    let (count, ip, port) = match crate::net::recv(socket, &mut data) {
        Ok(value) => value,
        Err(error) => return error,
    };
    if copy_to_user(buffer, &data[..count]).is_none() {
        return -14;
    }
    let supplied = if address_length == 0 {
        0
    } else {
        read_user::<u32>(address_length).unwrap_or(0) as usize
    };
    let address_size = match write_sockaddr(address, supplied, ip, port, socket_is_ipv6(fd)) {
        Ok(size) => size as u32,
        Err(_) => return -14,
    };
    if address_length != 0 && write_user(address_length, &address_size).is_none() {
        return -14;
    }
    count as isize
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxMsgHdr {
    name: usize,
    name_length: u32,
    _pad: u32,
    iov: usize,
    iov_length: usize,
    control: usize,
    control_length: usize,
    flags: i32,
    _pad2: u32,
}

fn sys_sendmsg(fd: usize, message: usize) -> isize {
    let socket = match socket_id(fd) {
        Ok(socket) => socket,
        Err(error) => return error,
    };
    let header = match read_user::<LinuxMsgHdr>(message) {
        Some(header) => header,
        None => return -14,
    };
    let destination = if header.name == 0 {
        None
    } else {
        match read_sockaddr(header.name, header.name_length as usize) {
            Ok(value) => Some(value),
            Err(error) => return error,
        }
    };
    let mut data = Vec::new();
    for index in 0..header.iov_length {
        let base = match read_user::<usize>(header.iov + index * 16) {
            Some(value) => value,
            None => return -14,
        };
        let length = match read_user::<usize>(header.iov + index * 16 + 8) {
            Some(value) => value,
            None => return -14,
        };
        let old = data.len();
        data.resize(old + length, 0);
        if copy_from_user(base, &mut data[old..]).is_none() {
            return -14;
        }
    }
    match crate::net::send(socket, &data, destination) {
        Ok(count) => count as isize,
        Err(error) => error,
    }
}

fn sys_recvmsg(fd: usize, message: usize) -> isize {
    let socket = match socket_id(fd) {
        Ok(socket) => socket,
        Err(error) => return error,
    };
    let mut header = match read_user::<LinuxMsgHdr>(message) {
        Some(header) => header,
        None => return -14,
    };
    let mut capacity = 0usize;
    for index in 0..header.iov_length {
        capacity =
            capacity.saturating_add(read_user::<usize>(header.iov + index * 16 + 8).unwrap_or(0));
    }
    let mut data = alloc::vec![0u8; capacity];
    let (count, ip, port) = match crate::net::recv(socket, &mut data) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let mut copied = 0;
    for index in 0..header.iov_length {
        let base = match read_user::<usize>(header.iov + index * 16) {
            Some(value) => value,
            None => return -14,
        };
        let length = read_user::<usize>(header.iov + index * 16 + 8)
            .unwrap_or(0)
            .min(count - copied);
        if copy_to_user(base, &data[copied..copied + length]).is_none() {
            return -14;
        }
        copied += length;
        if copied == count {
            break;
        }
    }
    if header.name != 0 {
        header.name_length = match write_sockaddr(
            header.name,
            header.name_length as usize,
            ip,
            port,
            socket_is_ipv6(fd),
        ) {
            Ok(size) => size as u32,
            Err(_) => return -14,
        };
    }
    header.flags = 0;
    if write_user(message, &header).is_none() {
        return -14;
    }
    count as isize
}
fn sys_socket_name(fd: usize, address: usize, address_length: usize) -> isize {
    if socket_id(fd).is_err() {
        return -9;
    }
    let supplied = if address_length == 0 {
        0
    } else {
        read_user::<u32>(address_length).unwrap_or(0) as usize
    };
    let size = match write_sockaddr(address, supplied, [10, 0, 2, 15], 0, socket_is_ipv6(fd)) {
        Ok(size) => size as u32,
        Err(_) => return -14,
    };
    if address_length != 0 && write_user(address_length, &size).is_none() {
        return -14;
    }
    0
}

fn sys_getsockopt(value: usize, length: usize) -> isize {
    if value != 0 && write_user(value, &0i32).is_none() {
        return -14;
    }
    if length != 0 && write_user(length, &4u32).is_none() {
        return -14;
    }
    0
}
pub(crate) struct FileEntry {
    inode: Arc<dyn INode>,
    offset: u64,
    path: String,
}
pub(crate) type SharedFile = Arc<Mutex<FileEntry>>;

fn next_fd(process: &crate::sched::ProcessState) -> usize {
    let mut fd = 3;
    while process.files.contains_key(&fd)
        || process.stdio.contains_key(&fd)
        || process.sockets.contains_key(&fd)
        || process.pipes.contains_key(&fd)
    {
        fd += 1;
    }
    fd
}

fn current_file(fd: usize) -> Option<SharedFile> {
    crate::sched::with_current_process(|process| process.lock().files.get(&fd).cloned())?
}

fn stdio_target(fd: usize) -> Option<usize> {
    if fd <= 2 {
        return Some(fd);
    }
    crate::sched::current_process().and_then(|process| process.lock().stdio.get(&fd).copied())
}

fn alloc_fd(inode: Arc<dyn INode>, path: String) -> Option<usize> {
    let process = crate::sched::current_process()?;
    let mut process = process.lock();
    let fd = next_fd(&process);
    process.files.insert(
        fd,
        Arc::new(Mutex::new(FileEntry {
            inode,
            offset: 0,
            path,
        })),
    );
    Some(fd)
}

fn current_pid() -> usize {
    crate::sched::current_pid().unwrap_or(1)
}

// --- Pipe implementation ----------------------------------------------------

/// Kernel-side pipe: a shared circular buffer plus per-end holder counts.
/// An end is only "closed" once its LAST fd holder (across forks and dups)
/// releases it — that is what EOF and EPIPE are derived from.
struct PipeBuffer {
    data: alloc::vec::Vec<u8>,
    read_pos: usize,
    write_pos: usize,
    readers: usize,
    writers: usize,
}

impl PipeBuffer {
    fn new() -> Self {
        Self {
            data: alloc::vec![0; 65536],
            read_pos: 0,
            write_pos: 0,
            readers: 1,
            writers: 1,
        }
    }

    fn closed_read(&self) -> bool {
        self.readers == 0
    }

    fn closed_write(&self) -> bool {
        self.writers == 0
    }
}

/// Register one more holder of a pipe end (fork/dup inherits).
fn pipe_add_holder(pipe: usize, is_read: bool) {
    if let Some(buffer) = PIPE_REGISTRY.lock().get(&pipe) {
        let mut pipe = buffer.lock();
        if is_read {
            pipe.readers += 1;
        } else {
            pipe.writers += 1;
        }
    }
}

/// Release one holder of a pipe end; the end closes when the last holder
/// drops it.
pub fn pipe_release(pipe: usize, is_read: bool) {
    if let Some(buffer) = PIPE_REGISTRY.lock().get(&pipe) {
        let mut pipe = buffer.lock();
        if is_read {
            pipe.readers = pipe.readers.saturating_sub(1);
        } else {
            pipe.writers = pipe.writers.saturating_sub(1);
        }
    }
}

static PIPE_REGISTRY: Mutex<BTreeMap<usize, Arc<Mutex<PipeBuffer>>>> = Mutex::new(BTreeMap::new());
static NEXT_PIPE_ID: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(1);

fn sys_pipe(pipefd: usize) -> isize {
    let (read_fd, write_fd) = match create_pipe() {
        Some(fds) => fds,
        None => return -24, // EMFILE
    };
    let result = [read_fd as i32, write_fd as i32];
    if write_user(pipefd, &result).is_none() {
        return -14;
    }
    0
}

fn sys_pipe2(pipefd: usize, _flags: usize) -> isize {
    // Flags (O_NONBLOCK, O_CLOEXEC) not yet supported.
    sys_pipe(pipefd)
}

fn create_pipe() -> Option<(usize, usize)> {
    let pipe_id = NEXT_PIPE_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let buffer = Arc::new(Mutex::new(PipeBuffer::new()));
    PIPE_REGISTRY.lock().insert(pipe_id, buffer.clone());

    let process = crate::sched::current_process()?;
    let mut process = process.lock();
    let read_fd = next_fd(&process);
    process.pipes.insert(read_fd, (pipe_id, true));
    let write_fd = next_fd(&process);
    process.pipes.insert(write_fd, (pipe_id, false));
    Some((read_fd, write_fd))
}

fn pipe_id(fd: usize) -> Option<(usize, bool)> {
    crate::sched::current_process().and_then(|process| process.lock().pipes.get(&fd).copied())
}

fn pipe_read(fd: usize, buf: usize, count: usize) -> isize {
    let Some((pipe_id, true)) = pipe_id(fd) else {
        return -9;
    };
    let Some(buffer) = PIPE_REGISTRY.lock().get(&pipe_id).cloned() else {
        return -9;
    };
    let mut data = alloc::vec![0u8; count];
    let mut read = 0;
    // Block until at least one byte or writer closes.
    loop {
        let mut pipe = buffer.lock();
        if pipe.read_pos < pipe.write_pos {
            let available = pipe.write_pos - pipe.read_pos;
            let to_read = available.min(count);
            // Copy with wraparound: read_pos/write_pos are monotonic counters
            // into the ring's modulo domain.
            let mut remaining = to_read;
            let mut src = pipe.read_pos % pipe.data.len();
            let mut dst = 0;
            while remaining > 0 {
                let chunk = remaining.min(pipe.data.len() - src);
                data[dst..dst + chunk].copy_from_slice(&pipe.data[src..src + chunk]);
                dst += chunk;
                src = (src + chunk) % pipe.data.len();
                remaining -= chunk;
            }
            pipe.read_pos += to_read;
            read = to_read;
            break;
        }
        if pipe.closed_write() {
            break;
        }
        drop(pipe);
        kor_sched::yield_now();
    }
    if read == 0 && count > 0 {
        return 0; // EOF
    }
    if copy_to_user(buf, &data[..read]).is_none() {
        return -14;
    }
    read as isize
}

fn pipe_write(fd: usize, buf: usize, count: usize) -> isize {
    let Some((pipe_id, false)) = pipe_id(fd) else {
        return -9;
    };
    let Some(buffer) = PIPE_REGISTRY.lock().get(&pipe_id).cloned() else {
        return -9;
    };
    let mut data = alloc::vec![0u8; count];
    if copy_from_user(buf, &mut data).is_none() {
        return -14;
    }

    let mut pipe = buffer.lock();
    if pipe.closed_read() {
        return -32; // EPIPE
    }
    let mut written = 0;
    while written < count {
        let space = pipe.data.len() - (pipe.write_pos - pipe.read_pos);
        if space == 0 {
            drop(pipe);
            kor_sched::yield_now();
            pipe = buffer.lock();
            continue;
        }
        let to_write = space.min(count - written);
        let write_start = pipe.write_pos % pipe.data.len();
        let end = (write_start + to_write).min(pipe.data.len());
        let chunk = end - write_start;
        pipe.data[write_start..end].copy_from_slice(&data[written..written + chunk]);
        pipe.write_pos += chunk;
        written += chunk;
    }
    written as isize
}

fn pipe_close(fd: usize) {
    let Some((pipe_id, is_read)) = pipe_id(fd) else {
        return;
    };
    pipe_release(pipe_id, is_read);
}

/// True when a pipe read-end has buffered data or its writer has closed
/// (EOF also reports readable, matching poll(2) semantics).
fn pipe_readable(fd: usize) -> bool {
    let Some((pipe, true)) = pipe_id(fd) else {
        return false;
    };
    match PIPE_REGISTRY.lock().get(&pipe) {
        Some(buffer) => {
            let pipe = buffer.lock();
            pipe.read_pos < pipe.write_pos || pipe.closed_write()
        }
        None => false,
    }
}

// --- LTP-critical syscall implementations -----------------------------------

fn sys_pread64(fd: usize, buf: usize, count: usize, offset: usize) -> isize {
    if checked_user_range(buf, count, true).is_none() {
        return -14;
    }
    let entry = match current_file(fd) {
        Some(entry) => entry,
        None => return -9,
    };
    let entry = entry.lock();
    let mut data = alloc::vec![0u8; count];
    match entry.inode.read_at(offset, &mut data) {
        Ok(read) => {
            drop(entry);
            if copy_to_user(buf, &data[..read]).is_none() {
                return -14;
            }
            read as isize
        }
        Err(error) => fs_errno(error),
    }
}

fn sys_pwrite64(fd: usize, buf: usize, count: usize, offset: usize) -> isize {
    let mut data = alloc::vec![0u8; count];
    if copy_from_user(buf, &mut data).is_none() {
        return -14;
    }
    let entry = match current_file(fd) {
        Some(entry) => entry,
        None => return -9,
    };
    let entry = entry.lock();
    match entry.inode.write_at(offset, &data) {
        Ok(written) => written as isize,
        Err(error) => fs_errno(error),
    }
}

/// struct pollfd { int fd; short events; short revents; } — 8 bytes
fn sys_poll(fds: usize, nfds: usize, _timeout: usize) -> isize {
    if nfds == 0 {
        return 0;
    }
    let byte_len = nfds.checked_mul(8).unwrap_or(usize::MAX);
    if checked_user_range(fds, byte_len, true).is_none() {
        return -14;
    }
    const POLLIN: u16 = 0x001;
    let mut ready = 0;
    for i in 0..nfds {
        let pfd = fds + i * 8;
        let fd = match read_user::<i32>(pfd) {
            Some(fd) => fd,
            None => return -14,
        };
        let events = match read_user::<u16>(pfd + 4) {
            Some(e) => e,
            None => return -14,
        };
        let mut revents: u16 = 0;
        if events & POLLIN != 0 {
            let readable = (fd == 0 && rx_has_data())
                || pipe_readable(fd as usize)
                || socket_id(fd as usize).is_ok_and(crate::net::has_data);
            if readable {
                revents = POLLIN;
            }
        }
        if revents != 0 {
            ready += 1;
        }
        if write_user(pfd + 6, &revents).is_none() {
            return -14;
        }
    }
    ready as isize
}

fn sys_sendfile(out_fd: usize, in_fd: usize, offset: usize, count: usize) -> isize {
    let _ = offset;
    let mut buf = alloc::vec![0u8; count.min(65536)];
    let read = sys_read(in_fd, buf.as_mut_ptr() as usize, buf.len());
    if read < 0 {
        return read;
    }
    sys_write(out_fd, buf.as_ptr() as usize, read as usize)
}

fn sys_fchdir(fd: usize) -> isize {
    let Some(entry) = current_file(fd) else {
        return -9;
    };
    let entry = entry.lock();
    let path = entry.path.clone();
    crate::sched::with_current_process(|process| process.lock().cwd = path);
    0
}

fn sys_fsync(fd: usize) -> isize {
    let _ = fd;
    match kor_fs::mount::sync_all(&crate::MOUNTS) {
        Ok(()) => 0,
        Err(_) => -5,
    }
}

fn sys_ftruncate(fd: usize, length: i64) -> isize {
    let Some(entry) = current_file(fd) else {
        return -9;
    };
    let inode = entry.lock().inode.clone();
    drop(entry);
    match inode.truncate(length as usize) {
        Ok(()) => 0,
        Err(_) => -5,
    }
}

fn sys_truncate(path_addr: usize, length: i64) -> isize {
    let full = match user_absolute_path(0xFF, path_addr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match kor_fs::mount::resolve(&crate::MOUNTS, &full) {
        Ok(inode) => match inode.truncate(length as usize) {
            Ok(()) => 0,
            Err(_) => -5,
        },
        Err(_) => -2,
    }
}

/// Change permission bits on an inode via getattr → setattr.
fn chmod_inode(inode: &Arc<dyn INode>, mode: u32) -> isize {
    let mut metadata = match inode.getattr() {
        Ok(metadata) => metadata,
        Err(error) => return fs_errno(error),
    };
    metadata.mode = (metadata.mode & !0o7777) | (mode & 0o7777);
    match inode.setattr(&metadata) {
        Ok(()) => 0,
        Err(error) => fs_errno(error),
    }
}

fn sys_fchmod(fd: usize, mode: usize) -> isize {
    let Some(entry) = current_file(fd) else {
        return -9;
    };
    let inode = entry.lock().inode.clone();
    chmod_inode(&inode, mode as u32)
}

fn sys_fchmodat(dirfd: usize, path: usize, mode: usize, flags: usize) -> isize {
    const AT_SYMLINK_NOFOLLOW: usize = 0x100;
    const AT_EMPTY_PATH: usize = 0x1000;
    if flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
        return -22;
    }
    if flags & AT_SYMLINK_NOFOLLOW != 0 {
        // Linux rejects this combination (no O_PATH-based chmod yet).
        return -95; // EOPNOTSUPP
    }
    if flags & AT_EMPTY_PATH != 0 && path == 0 {
        return sys_fchmod(dirfd, mode);
    }
    let full = match user_absolute_path(dirfd, path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    match kor_fs::mount::resolve(&crate::MOUNTS, &full) {
        Ok(inode) => chmod_inode(&inode, mode as u32),
        Err(error) => fs_errno(error),
    }
}

fn sys_waitid(_idtype: usize, _pid: usize, _infop: usize, _options: usize) -> isize {
    sys_wait4(usize::MAX, 0, 0)
}

fn sys_sigtimedwait(set: usize, info: usize, timeout: usize) -> isize {
    let Some(process) = crate::sched::current_process() else {
        return ESRCH;
    };
    let Some(interest) = read_user::<u64>(set) else {
        return EFAULT;
    };
    // A NULL timespec means wait forever (Linux treats it as such).
    let deadline = if timeout == 0 {
        None
    } else {
        let Some([seconds, nanos]) = read_user::<[i64; 2]>(timeout) else {
            return EFAULT;
        };
        if seconds < 0 || !(0..1_000_000_000).contains(&nanos) {
            return EINVAL;
        }
        Some(
            kor_arch::now()
                + core::time::Duration::from_secs(seconds as u64)
                + core::time::Duration::from_nanos(nanos as u32 as u64),
        )
    };
    loop {
        let deliverable = {
            let mut process = process.lock();
            let signals = &mut process.signals;
            let ready = signals.pending & !signals.blocked & interest;
            match ready.trailing_zeros() {
                index if index < 64 => {
                    let signal = index + 1;
                    signals.pending &= !(1 << index);
                    signal
                }
                _ => 0,
            }
        };
        if deliverable != 0 {
            if info != 0 {
                let payload = LinuxSigInfo {
                    signo: deliverable as i32,
                    errno: 0,
                    code: 0,
                    _pad0: 0,
                    sender_pid: 0,
                    sender_uid: 0,
                    _pad: [0; 104],
                };
                if write_user(info, &payload).is_none() {
                    return EFAULT;
                }
            }
            return deliverable as isize;
        }
        match deadline {
            Some(limit) if kor_arch::now() >= limit => return -11, // EAGAIN
            Some(_) | None => {
                kor_sched::yield_now();
            }
        }
    }
}

/// struct statfs { long type, bsize; long long blocks, bfree, bavail, files, ffree; ... }
fn sys_statfs(path_addr: usize, buf: usize) -> isize {
    let _path = match user_absolute_path(0xFF, path_addr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    // Minimal ext2 stats
    let statfs: [u64; 8] = [0xEF53, 1024, 4096, 3800, 100, 1024, 1000, 0];
    if write_user(buf, &statfs).is_none() {
        return -14;
    }
    0
}

fn sys_fstatfs(fd: usize, buf: usize) -> isize {
    let _ = fd;
    let statfs: [u64; 8] = [0xEF53, 1024, 4096, 3800, 100, 1024, 1000, 0];
    if write_user(buf, &statfs).is_none() {
        return -14;
    }
    0
}

fn sys_setitimer(_which: usize, _new_val: usize, _old_val: usize) -> isize {
    0 // no-op: no interval timers yet
}

fn sys_getitimer(_which: usize, _val: usize) -> isize {
    0
}

fn sys_getrusage(_who: usize, usage: usize) -> isize {
    // struct rusage: 18 * 8 bytes (timevals + counters)
    let rusage = [0u64; 18];
    if write_user(usage, &rusage).is_none() {
        return -14;
    }
    0
}

// --- Syscall implementations ---

fn sys_write(fd: usize, buf: usize, count: usize) -> isize {
    // Linux checks the fd first: EBADF wins over EFAULT.
    if pipe_id(fd).is_none()
        && socket_id(fd).is_err()
        && stdio_target(fd).is_none()
        && current_file(fd).is_none()
    {
        return -9;
    }
    if pipe_id(fd).is_some() {
        return pipe_write(fd, buf, count);
    }
    let mut data = alloc::vec![0u8; count];
    if copy_from_user(buf, &mut data).is_none() {
        return -14;
    }
    if let Ok(socket) = socket_id(fd) {
        return match crate::net::send(socket, &data, None) {
            Ok(written) => written as isize,
            Err(error) => error,
        };
    }
    let entry = current_file(fd);
    if let Some(entry) = entry {
        let mut entry = entry.lock();
        match entry.inode.write_at(entry.offset as usize, &data) {
            Ok(written) => {
                entry.offset += written as u64;
                written as isize
            }
            Err(error) => fs_errno(error),
        }
    } else if matches!(stdio_target(fd), Some(1 | 2)) {
        for byte in data {
            kor::console::putc(byte);
        }
        count as isize
    } else {
        -9
    }
}

fn sys_read(fd: usize, buf: usize, count: usize) -> isize {
    // Linux checks the fd first: EBADF wins over EFAULT.
    if pipe_id(fd).is_none()
        && socket_id(fd).is_err()
        && stdio_target(fd).is_none()
        && current_file(fd).is_none()
    {
        return -9;
    }
    if checked_user_range(buf, count, true).is_none() {
        return -14;
    }
    if pipe_id(fd).is_some() {
        return pipe_read(fd, buf, count);
    }
    let mut data = alloc::vec![0u8; count];
    if let Ok(socket) = socket_id(fd) {
        return match crate::net::recv(socket, &mut data) {
            Ok((read, _, _)) => {
                if copy_to_user(buf, &data[..read]).is_some() {
                    read as isize
                } else {
                    -14
                }
            }
            Err(error) => error,
        };
    }
    let read = if stdio_target(fd) == Some(0) {
        let mut read = 0;
        while read == 0 {
            if let Some(byte) = rx_get() {
                data[read] = byte;
                read += 1;
            }
        }
        while read < count {
            match rx_get() {
                Some(byte) => {
                    data[read] = byte;
                    read += 1;
                }
                None => break,
            }
        }
        read
    } else {
        let entry = match current_file(fd) {
            Some(entry) => entry,
            None => return -9,
        };
        let mut entry = entry.lock();
        match entry.inode.read_at(entry.offset as usize, &mut data) {
            Ok(read) => {
                entry.offset += read as u64;
                read
            }
            Err(error) => return fs_errno(error),
        }
    };
    if copy_to_user(buf, &data[..read]).is_none() {
        return -14;
    }
    read as isize
}
fn sys_set_robust_list(head: usize, length: usize) -> isize {
    if length != 3 * core::mem::size_of::<usize>() {
        return -22;
    }
    if checked_user_range(head, length, false).is_none() {
        return -14;
    }
    let Some(process) = crate::sched::current_process() else {
        return -3;
    };
    let mut process = process.lock();
    process.robust_list = head;
    process.robust_list_len = length;
    0
}

fn sys_prctl(args: &[usize]) -> isize {
    const PR_SET_NAME: usize = 15;
    const PR_GET_NAME: usize = 16;
    let Some(process) = crate::sched::current_process() else {
        return -3;
    };
    match args[0] {
        PR_SET_NAME => {
            let mut name = [0u8; 16];
            if copy_from_user(args[1], &mut name).is_none() {
                return -14;
            }
            process.lock().name = name;
            0
        }
        PR_GET_NAME => {
            let name = process.lock().name;
            if copy_to_user(args[1], &name).is_none() {
                -14
            } else {
                0
            }
        }
        _ => -22,
    }
}

fn sys_writev(fd: usize, iov: usize, iovcnt: usize) -> isize {
    if iovcnt > 1024 {
        return -22;
    }
    let mut total = 0usize;
    for index in 0..iovcnt {
        let Some(base) = read_user::<usize>(iov + index * 16) else {
            return -14;
        };
        let Some(length) = read_user::<usize>(iov + index * 16 + 8) else {
            return -14;
        };
        let result = sys_write(fd, base, length);
        if result < 0 {
            return if total == 0 { result } else { total as isize };
        }
        total = match total.checked_add(result as usize) {
            Some(value) => value,
            None => return -22,
        };
    }
    total as isize
}

fn sys_exit_fork(code: usize) -> isize {
    crate::sched::exit_current(code as i32)
}
fn sys_clone(args: &[usize]) -> isize {
    let parent = match crate::sched::current_process_arc() {
        Some(process) => process,
        None => return -38,
    };
    let (
        parent_pid,
        child_signals,
        child_brk,
        child_mmap_next,
        child_executable,
        child_cwd,
        child_files,
        child_stdio,
        child_sockets,
        child_ipv6_fds,
        child_pipes,
    ) = {
        let parent = parent.lock();
        (
            parent.pid,
            parent.signals.fork_clone(),
            parent.brk,
            parent.mmap_next,
            parent.executable.clone(),
            parent.cwd.clone(),
            parent.files.clone(),
            parent.stdio.clone(),
            parent.sockets.clone(),
            parent.ipv6_fds.clone(),
            parent.pipes.clone(),
        )
    };
    let child_space = match parent
        .lock()
        .address_space()
        .and_then(|space| space.fork().ok())
    {
        Some(address_space) => address_space,
        None => return -12,
    };
    let mut context = match kor_sched::current()
        .with_user_context(|tf| unsafe { kor_arch::capture_user_context(tf) })
        .flatten()
    {
        Some(context) => context,
        None => return -38,
    };
    #[cfg(target_arch = "riscv64")]
    {
        context.regs[10] = 0;
        context.pc = context.pc.wrapping_add(4);
        // A nonzero `stack` argument (posix_spawn, pthread_create) is where
        // the child must run: __clone's trampoline loads fn/arg from [sp].
        if args[1] != 0 {
            context.sp = args[1];
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        context.regs[0] = 0;
        if args[1] != 0 {
            context.sp = args[1];
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        // UserTrapContext follows Linux x86_64 sigcontext order; RAX is slot 13.
        context.regs[13] = 0;
        if args[1] != 0 {
            context.sp = args[1];
        }
    }
    let child = crate::sched::alloc_process(parent_pid, child_space);
    {
        let mut child = child.lock();
        child.cwd = child_cwd;
        child.signals = child_signals;
        child.brk = child_brk;
        child.executable = child_executable;
        child.name = parent.lock().name;
        child.mmap_next = child_mmap_next;
        child.files = child_files;
        child.stdio = child_stdio;
        child.sockets = child_sockets;
        child.ipv6_fds = child_ipv6_fds;
        child.pipes = child_pipes.clone();
        for (pipe, is_read) in child_pipes.values() {
            pipe_add_holder(*pipe, *is_read);
        }
        child.set_fork_context(context);
    }
    let pid = child.lock().pid;
    const CLONE_CHILD_SETTID: usize = 0x0100_0000;
    const CLONE_CHILD_CLEARTID: usize = 0x0020_0000;
    if args[4] != 0 {
        if args[0] & CLONE_CHILD_CLEARTID != 0 {
            child.lock().clear_child_tid = args[4];
        }
        if args[0] & CLONE_CHILD_SETTID != 0 {
            if let Some(paddr) = child
                .lock()
                .address_space()
                .and_then(|space| space.translate(args[4]))
            {
                unsafe {
                    *(kor_arch::phys_to_virt(paddr) as *mut u32) = pid as u32;
                }
            }
        }
    }
    crate::sched::spawn_process(&child, fork_child_entry);
    pid as isize
}

fn fork_child_entry() {
    let process = crate::sched::current_process().expect("fork child task has no process");
    #[cfg(target_arch = "x86_64")]
    kor_arch::x86_64::trap::set_syscall_stack(kor_sched::current().user_stack_top());
    let context = {
        let mut process = process.lock();
        process.activate_address_space();
        process
            .take_fork_context()
            .expect("fork child has no user context")
    };
    // `resume_user_context` never returns: entering user mode abandons this
    // kernel stack, so the process Arc must be released here or the task's
    // zombie address space would leak for good.
    drop(process);
    // The architecture trampoline masks IRQs immediately before restoring the
    // detached user register set, closing the kernel-to-user transition window.
    unsafe { kor_arch::resume_user_context(&context) }
}

fn sys_execve(filename: usize, argv: usize) -> isize {
    let path = match user_to_str(filename, 4096) {
        Some(path) => path,
        None => return -14,
    };
    let image_path = if path == "/proc/self/exe" {
        match crate::sched::current_process() {
            Some(process) => process.lock().executable.clone(),
            None => return -2,
        }
    } else {
        path.clone()
    };
    let mut args = Vec::new();
    if argv != 0 {
        for index in 0..256 {
            let pointer_address = match argv.checked_add(index * core::mem::size_of::<usize>()) {
                Some(address) => address,
                None => return -14,
            };
            let Some(pointer) = read_user::<usize>(pointer_address) else {
                return -14;
            };
            if pointer == 0 {
                break;
            }
            let argument = match user_to_str(pointer, 4096) {
                Some(argument) => argument,
                None => return -14,
            };
            args.push(argument);
        }
    }
    if args.is_empty() {
        args.push(path.clone());
    }
    let image = match crate::fs::read_file(&image_path) {
        Ok(image) => image,
        Err(error) => return fs_errno(error),
    };
    let interpreter_path = match crate::user::elf::interpreter_path(&image) {
        Ok(path) => path,
        Err(_) => return -8,
    };
    let interpreter = match interpreter_path.as_deref() {
        Some(path) => match crate::fs::read_file(path) {
            Ok(image) => Some(image),
            Err(error) => return fs_errno(error),
        },
        None => None,
    };
    let mut replacement = match crate::mm::AddressSpace::empty() {
        Ok(address_space) => address_space,
        Err(_) => return -12,
    };
    let program = match crate::user::elf::load_program(
        &mut replacement,
        &image,
        interpreter.as_deref(),
        &args,
    ) {
        Ok(program) => program,
        Err(_) => return -8,
    };

    let updated = kor_sched::current()
        .with_user_context(|tf| unsafe {
            let mut context = match kor_arch::capture_user_context(tf) {
                Some(context) => context,
                None => return false,
            };
            context.regs = [0; 32];
            context.pc = program.entry;
            context.sp = program.stack_top;
            context.tls = 0;
            kor_arch::restore_user_context(tf, &context)
        })
        .unwrap_or(false);
    if !updated {
        return -38;
    }

    let process = match crate::sched::current_process() {
        Some(process) => process,
        None => return -38,
    };
    replacement.activate();
    let old = {
        let mut process = process.lock();
        let old = process.replace_address_space(replacement);
        process.executable = image_path;
        process.name = {
            // Linux comm = basename of the executable, truncated to 15 bytes.
            let mut name = [0u8; 16];
            let basename = path.rsplit('/').next().unwrap_or(&path);
            let bytes = basename.as_bytes();
            let length = bytes.len().min(15);
            name[..length].copy_from_slice(&bytes[..length]);
            name
        };
        process.signals.reset_for_exec();
        process.brk = 0;
        process.mmap_next = crate::user::layout::MMAP_BASE;
        old
    };
    drop(old);
    0
}

fn sys_brk(addr: usize) -> isize {
    let Some(process) = crate::sched::current_process() else {
        return -12;
    };
    let mut process = process.lock();
    if process.brk == 0 {
        process.brk = crate::user::layout::BRK_BASE;
    }
    if addr == 0 {
        return process.brk as isize;
    }
    let old = process.brk;
    if addr > old {
        let page_start = (old + 4095) & !4095;
        let Some(page_end) = addr.checked_add(4095).map(|value| value & !4095) else {
            return process.brk as isize;
        };
        let flags =
            kor_arch::MappingFlags::U | kor_arch::MappingFlags::R | kor_arch::MappingFlags::W;
        let mut mapped = page_start;
        while mapped < page_end {
            let result = process
                .address_space_mut()
                .ok_or(kor_arch::MapError::Unsupported)
                .and_then(|space| {
                    if space.is_mapped(mapped) {
                        Ok(0)
                    } else {
                        space.map_zeroed(mapped, flags)
                    }
                });
            if result.is_err() {
                return process.brk as isize;
            }
            mapped += 4096;
        }
        process.brk = addr;
    }
    process.brk as isize
}

fn sys_uname(buf: usize) -> isize {
    let machine = if cfg!(target_arch = "riscv64") {
        "riscv64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    let fields = [
        "korix",
        "localhost",
        "6.12.0-korix",
        "#1 SMP korix",
        machine,
        "(none)",
    ];
    let mut uts = [0u8; 6 * 65];

    for (index, field) in fields.iter().enumerate() {
        let bytes = field.as_bytes();
        let length = bytes.len().min(64);
        uts[index * 65..index * 65 + length].copy_from_slice(&bytes[..length]);
    }
    if copy_to_user(buf, &uts).is_none() {
        -14
    } else {
        0
    }
}

/// sysinfo(2) — struct sysinfo (asm-generic, 112 bytes) with memory and
/// process counts filled from the frame allocator and scheduler.
fn sys_sysinfo(address: usize) -> isize {
    const SYSINFO_LEN: usize = 112;
    const FIELD_TOTALRAM: usize = 32;
    const FIELD_FREERAM: usize = 40;
    const FIELD_PROCS: usize = 80;
    const FIELD_MEM_UNIT: usize = 104;
    const MEM_UNIT: u32 = 1024;

    let mut info = [0u8; SYSINFO_LEN];
    let uptime = kor_arch::now().as_secs() as u64;
    info[0..8].copy_from_slice(&uptime.to_ne_bytes());
    let (total_kib, free_kib) = crate::procfs::memory_report();
    info[FIELD_TOTALRAM..FIELD_TOTALRAM + 8].copy_from_slice(&(total_kib as u64).to_ne_bytes());
    info[FIELD_FREERAM..FIELD_FREERAM + 8].copy_from_slice(&(free_kib as u64).to_ne_bytes());
    let procs = crate::sched::process_snapshot().len() as u16;
    info[FIELD_PROCS..FIELD_PROCS + 2].copy_from_slice(&procs.to_ne_bytes());
    info[FIELD_MEM_UNIT..FIELD_MEM_UNIT + 4].copy_from_slice(&MEM_UNIT.to_ne_bytes());
    if copy_to_user(address, &info).is_none() {
        return -14;
    }
    0
}

fn sys_getrandom(buf: usize, count: usize) -> isize {
    let mut data = alloc::vec![0u8; count];
    for (index, byte) in data.iter_mut().enumerate() {
        *byte = ((index as u64)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407)
            >> 32) as u8;
    }
    if copy_to_user(buf, &data).is_none() {
        -14
    } else {
        count as isize
    }
}

fn is_at_fdcwd(dirfd: usize) -> bool {
    dirfd as isize == AT_FDCWD || dirfd as u32 == AT_FDCWD as i32 as u32
}

fn normalize_absolute(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            name => components.push(name),
        }
    }
    let mut normalized = String::from("/");
    normalized.push_str(&components.join("/"));
    Some(normalized)
}

fn absolute_path(dirfd: usize, path: &str) -> Result<String, isize> {
    if path.starts_with('/') {
        return normalize_absolute(path).ok_or(-22isize);
    }
    let base = if is_at_fdcwd(dirfd) {
        crate::sched::current_process()
            .map(|process| process.lock().cwd.clone())
            .ok_or(-9isize)?
    } else {
        let entry = current_file(dirfd).ok_or(-9isize)?;
        let entry = entry.lock();
        if entry.inode.getattr().map_err(fs_errno)?.file_type != kor::FileType::Directory {
            return Err(-20isize);
        }
        entry.path.clone()
    };
    normalize_absolute(&alloc::format!("{base}/{path}")).ok_or(-22isize)
}

fn user_absolute_path(dirfd: usize, path: usize) -> Result<String, isize> {
    let path = user_to_str(path, 4096).ok_or(-14isize)?;
    if path.is_empty() {
        return Err(-2isize);
    }
    absolute_path(dirfd, &path)
}

fn split_absolute_parent(path: &str) -> Result<(&str, &str), isize> {
    if path == "/" {
        return Err(-16isize);
    }
    let (parent, name) = path.rsplit_once('/').ok_or(-22isize)?;
    if name.is_empty() || name == "." || name == ".." {
        return Err(-22isize);
    }
    Ok((if parent.is_empty() { "/" } else { parent }, name))
}

fn resolve_user_path(dirfd: usize, path: usize) -> Result<(String, Arc<dyn INode>), isize> {
    let path = user_absolute_path(dirfd, path)?;
    let inode = crate::fs::resolve(&path).map_err(fs_errno)?;
    Ok((path, inode))
}

fn sys_getcwd(buf: usize, size: usize) -> isize {
    let Some(process) = crate::sched::current_process() else {
        return -9;
    };
    let cwd = process.lock().cwd.clone();
    let required = cwd.len() + 1;
    if required > size {
        return -34;
    }
    if copy_to_user(buf, cwd.as_bytes()).is_none() || copy_to_user(buf + cwd.len(), &[0]).is_none()
    {
        -14
    } else {
        required as isize
    }
}

fn sys_chdir(path: usize) -> isize {
    let absolute = match user_absolute_path(AT_FDCWD as usize, path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let inode = match crate::fs::resolve(&absolute) {
        Ok(inode) => inode,
        Err(error) => return fs_errno(error),
    };
    if inode.getattr().map_err(fs_errno).map(|meta| meta.file_type) != Ok(kor::FileType::Directory)
    {
        return -20;
    }
    let Some(process) = crate::sched::current_process() else {
        return -9;
    };
    process.lock().cwd = absolute;
    0
}

fn sys_getdents64(fd: usize, buf: usize, count: usize) -> isize {
    let entry = match current_file(fd) {
        Some(entry) => entry,
        None => return -9,
    };
    let mut entry = entry.lock();
    let entries = match entry.inode.list() {
        Ok(entries) => entries,
        Err(_) => return -5,
    };
    let start_index = (entry.offset as usize).min(entries.len());
    // Linux requires the buffer to fit at least one record.
    if count < 19 && start_index < entries.len() {
        return -22;
    }
    let mut output = alloc::vec![0u8; count];
    let mut offset = 0usize;
    let mut index = start_index;
    while index < entries.len() {
        let (name, inode_number) = &entries[index];
        let record_length = ((19 + name.len() + 1 + 7) & !7) as u16;
        if offset + record_length as usize > count {
            break;
        }
        output[offset..offset + 8].copy_from_slice(&(*inode_number as u64).to_ne_bytes());
        output[offset + 8..offset + 16].copy_from_slice(&((index + 1) as i64).to_ne_bytes());
        output[offset + 16..offset + 18].copy_from_slice(&record_length.to_ne_bytes());
        output[offset + 18] = if name == "." || name == ".." {
            4
        } else {
            entry
                .inode
                .lookup(name)
                .ok()
                .and_then(|inode| inode.getattr().ok())
                .map(|metadata| match metadata.file_type {
                    kor::FileType::Directory => 4,
                    kor::FileType::Symlink => 10,
                    _ => 8,
                })
                .unwrap_or(8)
        };
        output[offset + 19..offset + 19 + name.len()].copy_from_slice(name.as_bytes());
        offset += record_length as usize;
        index += 1;
    }
    if copy_to_user(buf, &output[..offset]).is_none() {
        return -14;
    }
    entry.offset = index as u64;
    if offset == 0 && start_index >= entries.len() {
        0
    } else {
        offset as isize
    }
}

fn fs_errno(error: kor::FsError) -> isize {
    match error {
        kor::FsError::NotFound => -2,
        kor::FsError::AlreadyExists => -17,
        kor::FsError::NotDirectory => -20,
        kor::FsError::NotEmpty => -39,
        kor::FsError::TooManyLinks => -40,
        kor::FsError::CrossDevice => -18,
        kor::FsError::PermissionDenied => -13,
        kor::FsError::IsDirectory => -21,
        kor::FsError::InvalidInput => -22,
        kor::FsError::ReadOnly => -30,
        kor::FsError::NoSpace => -28,
        kor::FsError::Unsupported => -38,
        _ => -5,
    }
}

fn sys_openat(dirfd: usize, path: usize, flags: usize, mode: usize) -> isize {
    const O_CREAT: usize = 0o100;
    const O_EXCL: usize = 0o200;
    const O_TRUNC: usize = 0o1000;
    const O_DIRECTORY: usize = 0o200000;
    const O_TMPFILE: usize = 0o20000000;
    let temporary = flags & O_TMPFILE != 0;
    let flags = flags & !O_TMPFILE;
    let mut path_str = match user_absolute_path(dirfd, path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    if temporary {
        const TEMPORARY_NAME: &str = ".korix-tmp";
        if path_str != "/" {
            path_str.push('/');
        }
        path_str.push_str(TEMPORARY_NAME);
    }
    if path_str == "/dev/null" {
        return 3;
    }
    if path_str == "/dev/tty" || path_str == "/dev/console" {
        #[cfg(target_arch = "x86_64")]
        {
            return 0;
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            return 3;
        }
    }
    let inode = if temporary {
        let (parent_path, name) = match split_absolute_parent(&path_str) {
            Ok(parts) => parts,
            Err(error) => return error,
        };
        let parent = match crate::fs::resolve(parent_path) {
            Ok(node) => node,
            Err(error) => return fs_errno(error),
        };
        match parent.create(name, kor::FileType::Regular, (mode as u32) & 0o777) {
            Ok(node) => node,
            Err(kor::FsError::AlreadyExists) => match parent.lookup(name) {
                Ok(node) => {
                    if let Err(error) = node.truncate(0) {
                        return fs_errno(error);
                    }
                    node
                }
                Err(error) => return fs_errno(error),
            },
            Err(error) => return fs_errno(error),
        }
    } else {
        match crate::fs::resolve(&path_str) {
            Ok(_) if flags & O_CREAT != 0 && flags & O_EXCL != 0 => return -17,
            Ok(inode) => inode,
            Err(kor::FsError::NotFound) if flags & O_CREAT != 0 => {
                let (parent_path, name) = match split_absolute_parent(&path_str) {
                    Ok(parts) => parts,
                    Err(error) => return error,
                };
                let parent = match crate::fs::resolve(parent_path) {
                    Ok(node) => node,
                    Err(error) => return fs_errno(error),
                };
                match parent.create(name, kor::FileType::Regular, (mode as u32) & 0o777) {
                    Ok(node) => node,
                    Err(error) => return fs_errno(error),
                }
            }
            Err(error) => return fs_errno(error),
        }
    };
    let metadata = match inode.getattr() {
        Ok(metadata) => metadata,
        Err(error) => return fs_errno(error),
    };
    if flags & O_DIRECTORY != 0 && metadata.file_type != kor::FileType::Directory {
        return -20;
    }
    if flags & O_TRUNC != 0 && metadata.file_type == kor::FileType::Regular && metadata.size != 0 {
        if let Err(error) = inode.truncate(0) {
            return fs_errno(error);
        }
    }
    alloc_fd(inode, path_str)
        .map(|fd| fd as isize)
        .unwrap_or(-24)
}

fn sys_close(fd: usize) -> isize {
    if pipe_id(fd).is_some() {
        pipe_close(fd);
        crate::sched::with_current_process(|process| process.lock().pipes.remove(&fd));
        return 0;
    }
    if fd <= 2 {
        return 0;
    }
    let Some(process) = crate::sched::current_process() else {
        return -9;
    };
    let mut process = process.lock();
    process.ipv6_fds.remove(&fd);
    if let Some(socket) = process.sockets.remove(&fd) {
        return if crate::net::close(socket) { 0 } else { -9 };
    }
    if process.files.remove(&fd).is_some() || process.stdio.remove(&fd).is_some() {
        0
    } else {
        -9
    }
}

fn sys_dup(fd: usize) -> isize {
    let Some(process) = crate::sched::current_process() else {
        return -9;
    };
    let mut process = process.lock();
    let file = process.files.get(&fd).cloned();
    let stdio = if fd <= 2 {
        Some(fd)
    } else {
        process.stdio.get(&fd).copied()
    };
    let socket = process.sockets.get(&fd).copied();
    let pipe = process.pipes.get(&fd).copied();
    if file.is_none() && stdio.is_none() && socket.is_none() && pipe.is_none() {
        return -9;
    }
    let newfd = next_fd(&process);
    if let Some(entry) = file {
        process.files.insert(newfd, entry);
    } else if let Some(target) = stdio {
        process.stdio.insert(newfd, target);
    } else if let Some(socket) = socket {
        process.sockets.insert(newfd, socket);
        if process.ipv6_fds.contains_key(&fd) {
            process.ipv6_fds.insert(newfd, ());
        }
    } else if let Some(pipe) = pipe {
        process.pipes.insert(newfd, pipe);
        pipe_add_holder(pipe.0, pipe.1);
    }
    newfd as isize
}

fn sys_dup2(fd: usize, newfd: usize) -> isize {
    let Some(process) = crate::sched::current_process() else {
        return -9;
    };
    let mut process = process.lock();
    let file = process.files.get(&fd).cloned();
    let stdio = if fd <= 2 {
        Some(fd)
    } else {
        process.stdio.get(&fd).copied()
    };
    let socket = process.sockets.get(&fd).copied();
    let pipe = process.pipes.get(&fd).copied();
    if file.is_none() && stdio.is_none() && socket.is_none() && pipe.is_none() {
        return -9;
    }
    if fd == newfd {
        return newfd as isize;
    }
    process.files.remove(&newfd);
    process.stdio.remove(&newfd);
    process.sockets.remove(&newfd);
    process.ipv6_fds.remove(&newfd);
    process.pipes.remove(&newfd);
    if let Some(entry) = file {
        process.files.insert(newfd, entry);
    } else if let Some(target) = stdio {
        process.stdio.insert(newfd, target);
    } else if let Some(socket) = socket {
        process.sockets.insert(newfd, socket);
        if process.ipv6_fds.contains_key(&fd) {
            process.ipv6_fds.insert(newfd, ());
        }
    } else if let Some(pipe) = pipe {
        process.pipes.insert(newfd, pipe);
        pipe_add_holder(pipe.0, pipe.1);
    }
    newfd as isize
}

fn sys_fcntl(fd: usize, command: usize, argument: usize) -> isize {
    const F_DUPFD: usize = 0;
    const F_GETFD: usize = 1;
    const F_SETFD: usize = 2;
    const F_GETFL: usize = 3;
    const F_SETFL: usize = 4;
    const F_DUPFD_CLOEXEC: usize = 1030;
    let Some(process) = crate::sched::current_process() else {
        return -9;
    };
    let mut process = process.lock();
    let file = process.files.get(&fd).cloned();
    let stdio = if fd <= 2 {
        Some(fd)
    } else {
        process.stdio.get(&fd).copied()
    };
    let socket = process.sockets.get(&fd).copied();
    let pipe = process.pipes.get(&fd).copied();
    if file.is_none() && stdio.is_none() && socket.is_none() && pipe.is_none() {
        return -9;
    }
    match command {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            let mut newfd = argument.max(3);
            while process.files.contains_key(&newfd)
                || process.stdio.contains_key(&newfd)
                || process.sockets.contains_key(&newfd)
                || process.pipes.contains_key(&newfd)
            {
                newfd = match newfd.checked_add(1) {
                    Some(value) => value,
                    None => return -24,
                };
            }
            if let Some(entry) = file {
                process.files.insert(newfd, entry);
            } else if let Some(target) = stdio {
                process.stdio.insert(newfd, target);
            } else if let Some(socket) = socket {
                process.sockets.insert(newfd, socket);
                if process.ipv6_fds.contains_key(&fd) {
                    process.ipv6_fds.insert(newfd, ());
                }
            } else if let Some(pipe) = pipe {
                process.pipes.insert(newfd, pipe);
                pipe_add_holder(pipe.0, pipe.1);
            }
            newfd as isize
        }
        F_GETFD | F_GETFL => 0,
        F_SETFD | F_SETFL => 0,
        _ => -22,
    }
}

/// ioctl handler — terminal attributes are stored for real: TCGETS
/// returns what TCSETS last installed (cooked defaults before that), and the
/// input path honours ICRNL, so raw-mode programs (vi) and line programs
/// (sh) both behave.
fn sys_ioctl(fd: usize, request: usize, argp: usize) -> isize {
    // TCGETS = 0x5401: get terminal attributes
    if request == 0x5401 && fd <= 2 {
        let termios = termios();
        return if copy_to_user(argp, &termios).is_some() {
            0
        } else {
            -14
        };
    }
    // TCSETS = 0x5402, TCSETSW = 0x5403, TCSETSF = 0x5404: set terminal attrs
    if request >= 0x5402 && request <= 0x5404 && fd <= 2 {
        let mut source = [0u8; TERMIOS_LEN];
        if user_mem::copy_from(argp, &mut source).is_none() {
            return -14;
        }
        *TERMIOS.lock() = source;
        return 0;
    }
    // TIOCGWINSZ = 0x5413: get window size
    if request == 0x5413 {
        let winsize = [24u16, 80, 0, 0];
        return if write_user(argp, &winsize).is_some() {
            0
        } else {
            -14
        };
    }
    // TIOCGPGRP = 0x540F: get foreground process group
    if request == 0x540F {
        return if write_user(argp, &1u32).is_some() {
            0
        } else {
            -14
        };
    }
    // TIOCSPGRP = 0x5410: set foreground process group
    if request == 0x5410 {
        return 0;
    }
    // TIOCGSID = 0x5429: get session id
    if request == 0x5429 {
        return if write_user(argp, &1u32).is_some() {
            0
        } else {
            -14
        };
    }
    -25 // ENOTTY
}

#[cfg(not(target_arch = "x86_64"))]
#[repr(C)]
#[derive(Copy, Clone)]
struct LinuxStat {
    st_dev: u64,
    st_ino: u64,
    st_mode: u32,
    st_nlink: u32,
    st_uid: u32,
    st_gid: u32,
    st_rdev: u64,
    _pad: u64,
    st_size: i64,
    st_blksize: u32,
    _pad2: u32,
    st_blocks: u64,
    st_atime: i64,
    st_mtime: i64,
    st_ctime: i64,
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Copy, Clone)]
struct LinuxStat {
    st_dev: u64,
    st_ino: u64,
    st_nlink: u64,
    st_mode: u32,
    st_uid: u32,
    st_gid: u32,
    _pad0: u32,
    st_rdev: u64,
    st_size: i64,
    st_blksize: i64,
    st_blocks: i64,
    st_atime: i64,
    st_atime_nsec: i64,
    st_mtime: i64,
    st_mtime_nsec: i64,
    st_ctime: i64,
    st_ctime_nsec: i64,
    _unused: [i64; 3],
}

fn linux_stat(meta: kor::Metadata, inode: u64) -> LinuxStat {
    let mode = match meta.file_type {
        kor::FileType::Regular => 0o100664,
        kor::FileType::Directory => 0o040755,
        kor::FileType::Symlink => 0o120777,
        _ => 0o0600,
    };
    #[cfg(not(target_arch = "x86_64"))]
    {
        LinuxStat {
            st_dev: 0,
            st_ino: inode,
            st_mode: mode,
            st_nlink: 1,
            st_uid: 0,
            st_gid: 0,
            st_rdev: 0,
            _pad: 0,
            st_size: meta.size as i64,
            st_blksize: 512,
            _pad2: 0,
            st_blocks: ((meta.size as u64) + 511) / 512,
            st_atime: 0,
            st_mtime: 0,
            st_ctime: 0,
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        LinuxStat {
            st_dev: 1,
            st_ino: inode,
            st_nlink: 1,
            st_mode: mode,
            st_uid: 0,
            st_gid: 0,
            _pad0: 0,
            st_rdev: 0,
            st_size: meta.size as i64,
            st_blksize: 512,
            st_blocks: ((meta.size as u64) + 511) as i64 / 512,
            st_atime: 0,
            st_atime_nsec: 0,
            st_mtime: 0,
            st_mtime_nsec: 0,
            st_ctime: 0,
            st_ctime_nsec: 0,
            _unused: [0; 3],
        }
    }
}

fn sys_fstat(fd: usize, stat: usize) -> isize {
    let entry = match current_file(fd) {
        Some(entry) => entry,
        None => return -9,
    };
    let entry = entry.lock();
    let inode = entry.inode.ino() as u64;
    let st = linux_stat(entry.inode.getattr().unwrap(), inode);
    if write_user(stat, &st).is_none() {
        -14
    } else {
        0
    }
}

fn sys_lseek(fd: usize, offset: i64, whence: usize) -> isize {
    let entry = match current_file(fd) {
        Some(entry) => entry,
        None => return -9,
    };
    let mut entry = entry.lock();
    let new = match whence {
        0 => offset,
        1 => entry.offset as i64 + offset,
        _ => -1,
    };
    if new < 0 {
        return -22;
    }
    entry.offset = new as u64;
    new as isize
}
fn sys_mremap(old_address: usize, old_size: usize, new_size: usize, flags: usize) -> isize {
    const PAGE_SIZE: usize = 4096;
    const MREMAP_MAYMOVE: usize = 1;
    if old_address & (PAGE_SIZE - 1) != 0
        || old_size == 0
        || new_size == 0
        || flags & !MREMAP_MAYMOVE != 0
    {
        return -22;
    }
    let Some(old_rounded) = old_size
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
    else {
        return -22;
    };
    let Some(new_rounded) = new_size
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
    else {
        return -22;
    };
    if new_rounded <= old_rounded {
        return old_address as isize;
    }
    let Some(process) = crate::sched::current_process() else {
        return -12;
    };
    let mut process = process.lock();
    let Some(address_space) = process.address_space_mut() else {
        return -12;
    };
    let growth_start = match old_address.checked_add(old_rounded) {
        Some(address) => address,
        None => return -22,
    };
    let growth = new_rounded - old_rounded;
    if !address_space.range_is_free(growth_start, growth) {
        return -12;
    }
    let mapping_flags =
        kor_arch::MappingFlags::U | kor_arch::MappingFlags::R | kor_arch::MappingFlags::W;
    let mut mapped = 0;
    while mapped < growth {
        if let Err(error) = address_space.map_zeroed(growth_start + mapped, mapping_flags) {
            let _ = address_space.unmap_range(growth_start, mapped);
            return if error == kor_arch::MapError::OutOfMemory {
                -12
            } else {
                -22
            };
        }
        mapped += PAGE_SIZE;
    }
    old_address as isize
}

/// Shared-memory registry: frames backing a MAP_SHARED file mapping,
/// keyed by the inode's Arc identity so forks and re-maps alias the
/// same physical pages.
static SHM_REGISTRY: Mutex<BTreeMap<usize, Arc<Mutex<alloc::vec::Vec<usize>>>>> =
    Mutex::new(BTreeMap::new());

/// Map `length` bytes of shared, registry-owned frames at `page_start`.
fn sys_mmap_shared(
    page_start: usize,
    length: usize,
    flags: kor_arch::MappingFlags,
    fd: usize,
    offset: i64,
) -> isize {
    let Some(process) = crate::sched::current_process() else {
        return -12;
    };
    let process = process.lock();
    if offset < 0 || offset as usize % 4096 != 0 {
        return -22;
    }
    let Some(entry) = process.files.get(&fd).cloned() else {
        return -9;
    };
    let inode = entry.lock().inode.clone();
    drop(process);

    let key = Arc::as_ptr(&inode) as *const dyn INode as *const () as usize;
    let frames = {
        let mut registry = SHM_REGISTRY.lock();
        registry
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(alloc::vec::Vec::new())))
            .clone()
    };
    {
        let mut frames = frames.lock();
        while frames.len() * 4096 < length {
            match kor::frame::alloc_frame() {
                Some(phys) => {
                    // SAFETY: freshly allocated frame; zero it once at
                    // registry-creation time.
                    unsafe {
                        core::ptr::write_bytes(kor_arch::phys_to_virt(phys) as *mut u8, 0, 4096);
                    }
                    frames.push(phys);
                }
                None => return -12,
            }
        }
    }

    let Some(process) = crate::sched::current_process() else {
        return -12;
    };
    let mut process = process.lock();
    let Some(address_space) = process.address_space_mut() else {
        return -12;
    };
    let frames = frames.lock();
    let mut mapped = 0;
    while mapped < length {
        let phys = frames[mapped / 4096];
        if address_space
            .map_shared_frame(page_start + mapped, phys, flags)
            .is_err()
        {
            let _ = address_space.unmap_range(page_start, mapped);
            return -22;
        }
        mapped += 4096;
    }
    page_start as isize
}

fn sys_mmap(
    address: usize,
    mut length: usize,
    prot: usize,
    flags: usize,
    fd: usize,
    offset: i64,
) -> isize {
    if length == 0 {
        length = 4096;
    }
    let fixed = flags & 0x10 != 0;
    let page_start = if fixed {
        if address & 4095 != 0 {
            return -22;
        }
        address
    } else {
        let Some(process) = crate::sched::current_process() else {
            return -12;
        };
        let mut process = process.lock();
        let Some(rounded) = length.checked_add(4095).map(|value| value & !4095) else {
            return -12;
        };
        let Some(space) = process.address_space() else {
            return -12;
        };
        let mut start = process.mmap_next;
        while !space.range_is_free(start, rounded) {
            let Some(next) = start.checked_add(4096) else {
                return -12;
            };
            start = next;
        }
        let Some(next) = start.checked_add(rounded) else {
            return -12;
        };
        process.mmap_next = next;
        start
    };
    let Some(rounded) = length.checked_add(4095).map(|value| value & !4095) else {
        return -22;
    };
    let mut map_flags = kor_arch::MappingFlags::U;
    if prot & 1 != 0 {
        map_flags = map_flags | kor_arch::MappingFlags::R;
    }
    if prot & 2 != 0 {
        map_flags = map_flags | kor_arch::MappingFlags::W;
    }
    if prot & 4 != 0 {
        map_flags = map_flags | kor_arch::MappingFlags::X;
    }
    // MAP_SHARED on a file: alias registry-owned frames so forks and other
    // mappings of the same inode observe the same memory (LTP ipc).
    const MAP_SHARED_FLAG: usize = 0x01;
    const MAP_ANONYMOUS_FLAG: usize = 0x20;
    if flags & MAP_SHARED_FLAG != 0 && flags & MAP_ANONYMOUS_FLAG == 0 {
        return sys_mmap_shared(page_start, rounded, map_flags, fd, offset);
    }
    // Linux anonymous mappings without explicit PROT bits are inaccessible;
    // BusyBox requests the required permissions explicitly.
    let Some(process) = crate::sched::current_process() else {
        return -12;
    };
    let mut process = process.lock();
    let backing_file = if flags & 0x20 == 0 {
        process.files.get(&fd).cloned()
    } else {
        None
    };
    let Some(address_space) = process.address_space_mut() else {
        return -12;
    };
    let result = if fixed {
        address_space.replace_zeroed(page_start, rounded, map_flags)
    } else {
        let mut mapped = 0;
        while mapped < rounded {
            if let Err(error) = address_space.map_zeroed(page_start + mapped, map_flags) {
                let _ = address_space.unmap_range(page_start, mapped);
                return if error == kor_arch::MapError::OutOfMemory {
                    -12
                } else {
                    -22
                };
            }
            mapped += 4096;
        }
        Ok(())
    };
    if result.is_ok() && flags & 0x20 == 0 {
        if offset < 0 {
            let _ = address_space.unmap_range(page_start, rounded);
            return -22;
        }
        let Some(file) = backing_file else {
            let _ = address_space.unmap_range(page_start, rounded);
            return -9;
        };
        let mut data = alloc::vec![0u8; length];
        let read = match file.lock().inode.read_at(offset as usize, &mut data) {
            Ok(read) => read,
            Err(error) => {
                let _ = address_space.unmap_range(page_start, rounded);
                return fs_errno(error);
            }
        };
        if address_space.populate(page_start, &data[..read]).is_err() {
            let _ = address_space.unmap_range(page_start, rounded);
            return -14;
        }
    }
    match result {
        Ok(()) => page_start as isize,
        Err(kor_arch::MapError::OutOfMemory) => -12,
        Err(_) => -22,
    }
}

fn sys_mprotect(address: usize, length: usize, prot: usize) -> isize {
    if address & 4095 != 0 || length == 0 || prot & !7 != 0 {
        return -22;
    }
    let Some(rounded) = length.checked_add(4095).map(|value| value & !4095) else {
        return -22;
    };
    let mut flags = kor_arch::MappingFlags::U;
    if prot & 1 != 0 {
        flags = flags | kor_arch::MappingFlags::R;
    }
    if prot & 2 != 0 {
        flags = flags | kor_arch::MappingFlags::W;
    }
    if prot & 4 != 0 {
        flags = flags | kor_arch::MappingFlags::X;
    }
    let Some(process) = crate::sched::current_process() else {
        return -12;
    };
    let mut process = process.lock();
    let Some(space) = process.address_space_mut() else {
        return -12;
    };
    match space.protect_range(address, rounded, flags) {
        Ok(()) => 0,
        Err(_) => -12,
    }
}

fn sys_munmap(addr: usize, len: usize) -> isize {
    if addr & 4095 != 0 || len == 0 {
        return -22;
    }
    let Some(rounded) = len.checked_add(4095).map(|value| value & !4095) else {
        return -22;
    };
    let Some(process) = crate::sched::current_process() else {
        return -22;
    };
    let mut process = process.lock();
    let Some(address_space) = process.address_space_mut() else {
        return -22;
    };
    match address_space.unmap_range(addr, rounded) {
        Ok(()) => 0,
        Err(_) => -22,
    }
}

fn sys_mkdirat(dirfd: usize, path: usize, mode: usize) -> isize {
    let path = match user_absolute_path(dirfd, path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let (parent_path, name) = match split_absolute_parent(&path) {
        Ok(parts) => parts,
        Err(error) => return error,
    };
    let parent = match crate::fs::resolve(parent_path) {
        Ok(node) => node,
        Err(error) => return fs_errno(error),
    };
    match parent.mkdir(name, (mode as u32) & 0o777) {
        Ok(_) => 0,
        Err(error) => fs_errno(error),
    }
}

fn sys_unlinkat(dirfd: usize, path: usize, flags: usize) -> isize {
    const AT_REMOVEDIR: usize = 0x200;
    if flags & !(AT_REMOVEDIR | 0x4) != 0 {
        return -22;
    }
    let path = match user_absolute_path(dirfd, path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let (parent_path, name) = match split_absolute_parent(&path) {
        Ok(parts) => parts,
        Err(error) => return error,
    };
    let parent = match crate::fs::resolve(parent_path) {
        Ok(node) => node,
        Err(error) => return fs_errno(error),
    };
    if flags & AT_REMOVEDIR == 0 {
        let target = match parent.lookup(name) {
            Ok(target) => target,
            Err(error) => return fs_errno(error),
        };
        let file_type = target
            .getattr()
            .map_err(fs_errno)
            .map(|meta| meta.file_type);
        if file_type == Ok(kor::FileType::Symlink) {
            return parent.unlink(name).map(|()| 0).unwrap_or_else(fs_errno);
        }
    }
    let result = if flags & AT_REMOVEDIR != 0 {
        parent.rmdir(name)
    } else {
        parent.unlink(name)
    };
    result.map(|()| 0).unwrap_or_else(fs_errno)
}

fn sys_renameat(old_dirfd: usize, old_path: usize, new_dirfd: usize, new_path: usize) -> isize {
    let old_path = match user_absolute_path(old_dirfd, old_path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let new_path = match user_absolute_path(new_dirfd, new_path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let (old_parent_path, old_name) = match split_absolute_parent(&old_path) {
        Ok(parts) => parts,
        Err(error) => return error,
    };
    let (new_parent_path, new_name) = match split_absolute_parent(&new_path) {
        Ok(parts) => parts,
        Err(error) => return error,
    };
    let old_parent = match crate::fs::resolve(old_parent_path) {
        Ok(node) => node,
        Err(error) => return fs_errno(error),
    };
    let new_parent = match crate::fs::resolve(new_parent_path) {
        Ok(node) => node,
        Err(error) => return fs_errno(error),
    };
    old_parent
        .rename(old_name, &new_parent, new_name)
        .map(|()| 0)
        .unwrap_or_else(fs_errno)
}

fn sys_linkat(
    old_dirfd: usize,
    old_path: usize,
    new_dirfd: usize,
    new_path: usize,
    flags: usize,
) -> isize {
    const AT_SYMLINK_FOLLOW: usize = 0x400;
    if flags & !AT_SYMLINK_FOLLOW != 0 {
        return -22;
    }
    let (_, target) = match resolve_user_path(old_dirfd, old_path) {
        Ok(target) => target,
        Err(error) => return error,
    };
    let new_path = match user_absolute_path(new_dirfd, new_path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let (parent_path, name) = match split_absolute_parent(&new_path) {
        Ok(parts) => parts,
        Err(error) => return error,
    };
    let parent = match crate::fs::resolve(parent_path) {
        Ok(node) => node,
        Err(error) => return fs_errno(error),
    };
    parent
        .link(name, &target)
        .map(|()| 0)
        .unwrap_or_else(fs_errno)
}

fn sys_symlinkat(target: usize, new_dirfd: usize, link_path: usize) -> isize {
    let target = match user_to_str(target, 4096) {
        Some(target) if !target.is_empty() => target,
        Some(_) => return -2,
        None => return -14,
    };
    let link_path = match user_absolute_path(new_dirfd, link_path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let (parent_path, name) = match split_absolute_parent(&link_path) {
        Ok(parts) => parts,
        Err(error) => return error,
    };
    let parent = match crate::fs::resolve(parent_path) {
        Ok(node) => node,
        Err(error) => return fs_errno(error),
    };
    parent
        .symlink(name, &target)
        .map(|_| 0)
        .unwrap_or_else(fs_errno)
}

fn sys_readlinkat(dirfd: usize, path: usize, buf: usize, size: usize) -> isize {
    if size == 0 {
        return -22;
    }
    let path = match user_absolute_path(dirfd, path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let (parent_path, name) = match split_absolute_parent(&path) {
        Ok(parts) => parts,
        Err(error) => return error,
    };
    let parent = match crate::fs::resolve(parent_path) {
        Ok(node) => node,
        Err(error) => return fs_errno(error),
    };
    let inode = match parent.lookup(name) {
        Ok(inode) => inode,
        Err(error) => return fs_errno(error),
    };
    let target = match inode.readlink() {
        Ok(target) => target,
        Err(error) => return fs_errno(error),
    };
    let length = target.len().min(size);
    if copy_to_user(buf, &target.as_bytes()[..length]).is_none() {
        -14
    } else {
        length as isize
    }
}

fn sys_rt_sigaction(signal: usize, action: usize, old_action: usize) -> isize {
    if signal == 0 || signal > SIGNAL_COUNT || signal == SIGKILL || signal == SIGSTOP {
        return EINVAL;
    }
    let new_action = if action != 0 {
        let Some(user_action) = read_user::<UserSignalAction>(action) else {
            return EFAULT;
        };
        let action = kernel_signal_action(user_action);
        if action.handler > SIG_IGN && checked_user_range(action.handler, 1, false).is_none() {
            return EFAULT;
        }
        if action.flags & SA_RESTORER != 0
            && checked_user_range(action.restorer, 1, false).is_none()
        {
            return EFAULT;
        }
        Some(SignalAction {
            mask: action.mask & !UNBLOCKABLE_MASK,
            ..action
        })
    } else {
        None
    };
    let Some(process) = crate::sched::current_process() else {
        return ESRCH;
    };
    let old = {
        let mut process = process.lock();
        let old = user_signal_action(process.signals.actions[signal]);
        if let Some(action) = new_action {
            process.signals.actions[signal] = action;
        }
        old
    };
    if old_action != 0 && write_user(old_action, &old).is_none() {
        return EFAULT;
    }
    0
}

fn sys_rt_sigprocmask(how: usize, set: usize, old_set: usize) -> isize {
    let requested = if set != 0 {
        let Some(mask) = read_user::<u64>(set) else {
            return EFAULT;
        };
        Some(mask & !UNBLOCKABLE_MASK)
    } else {
        None
    };
    crate::sched::with_current_process(|process| {
        let old = {
            let mut process = process.lock();
            let old = process.signals.blocked;
            if let Some(requested) = requested {
                match how {
                    0 => process.signals.blocked |= requested,
                    1 => process.signals.blocked &= !requested,
                    2 => process.signals.blocked = requested,
                    _ => return EINVAL,
                }
            }
            old
        };
        if old_set != 0 && write_user(old_set, &old).is_none() {
            return EFAULT;
        }
        0
    })
    .unwrap_or(ESRCH)
}

fn sys_rt_sigsuspend(mask: usize) -> isize {
    let Some(process) = crate::sched::current_process() else {
        return ESRCH;
    };
    let Some(requested) = read_user::<u64>(mask) else {
        return EFAULT;
    };
    let (events, old_mask) = {
        let mut process = process.lock();
        let old = process.signals.blocked;
        process.signals.blocked = requested & !UNBLOCKABLE_MASK;
        if process.signals.pending & !process.signals.blocked != 0 {
            process.signals.blocked = old;
            return -4;
        }
        (process.signal_events.clone(), old)
    };
    events.wait();
    process.lock().signals.blocked = old_mask;
    -4
}

fn sys_rt_sigreturn() -> isize {
    let Some(process) = crate::sched::current_process() else {
        return ESRCH;
    };
    let Some(result) = kor_sched::current().with_user_context(|frame| unsafe {
        let active = kor_arch::capture_user_context(frame)?;
        let frame_address = signal_frame_address(&active)?;
        let signal_frame = read_user::<LinuxRtSignalFrame>(frame_address)?;
        let mut restored = trap_context(&signal_frame.context.mcontext)?;
        restored.tls = active.tls;
        if restored.pc >= crate::user::layout::ADDRESS_LIMIT
            || restored.sp >= crate::user::layout::ADDRESS_LIMIT
        {
            return None;
        }
        if !kor_arch::restore_user_context(frame, &restored) {
            return None;
        }
        process.lock().signals.blocked = signal_frame.context.sigmask & !UNBLOCKABLE_MASK;
        Some(restored_syscall_result(&restored))
    }) else {
        return EFAULT;
    };
    match result {
        Some(value) => value,
        None => crate::sched::exit_current(128 + 11),
    }
}

fn sys_kill(pid: usize, signal: usize) -> isize {
    if signal > SIGNAL_COUNT {
        return EINVAL;
    }
    let target_pid = if pid == 0 {
        crate::sched::current_pid().unwrap_or(0)
    } else {
        pid
    };
    let Some(target) = crate::sched::process(target_pid) else {
        return ESRCH;
    };
    if signal != 0 {
        let mut target = target.lock();
        target.signals.pending |= 1 << (signal - 1);
        target.signal_events.post();
    }
    0
}

/// Deliver one pending, unblocked signal into the active user trap frame.
/// Called by architecture trap handlers immediately before their user return.
#[unsafe(no_mangle)]
pub extern "Rust" fn korix_deliver_pending_signal(frame: *mut u8) -> Option<i32> {
    let Some(process) = crate::sched::current_process() else {
        return None;
    };
    if kor_sched::current().with_user_context(|active| core::ptr::eq(active, frame)) != Some(true) {
        return None;
    }
    loop {
        let pending = {
            let mut process = process.lock();
            let deliverable = process.signals.pending & !process.signals.blocked;
            if deliverable == 0 {
                return None;
            }
            let signal = deliverable.trailing_zeros() as usize + 1;
            let bit = 1u64 << (signal - 1);
            process.signals.pending &= !bit;
            (
                signal,
                bit,
                process.signals.actions[signal],
                process.signals.blocked,
            )
        };
        let (signal, bit, action, blocked) = pending;
        if action.handler == SIG_IGN {
            continue;
        }
        if action.handler == SIG_DFL && signal == 17 {
            continue;
        }
        if action.handler == SIG_DFL {
            return Some(128 + signal as i32);
        }

        let Some(context) = (unsafe { kor_arch::capture_user_context(frame) }) else {
            return None;
        };
        let frame_size = core::mem::size_of::<LinuxRtSignalFrame>();
        let Some(frame_address) = context.sp.checked_sub(frame_size).map(|sp| sp & !15) else {
            return Some(128 + 11);
        };
        if checked_user_range(frame_address, frame_size, true).is_none() {
            return Some(128 + 11);
        }
        let info = LinuxSigInfo {
            signo: signal as i32,
            errno: 0,
            code: 0,
            _pad0: 0,
            sender_pid: crate::sched::current_pid().unwrap_or(0) as i32,
            sender_uid: 0,
            _pad: [0; 104],
        };
        let user_context = make_ucontext(&context, blocked);
        #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
        let signal_frame = LinuxRtSignalFrame {
            info,
            context: user_context,
        };
        #[cfg(target_arch = "x86_64")]
        let signal_frame = LinuxRtSignalFrame {
            restorer: action.restorer,
            context: user_context,
            info,
        };
        if write_user(frame_address, &signal_frame).is_none() {
            return Some(128 + 11);
        }
        let Some(restorer) = signal_restorer(action) else {
            return Some(128 + 11);
        };

        {
            let mut process = process.lock();
            if action.flags & SA_RESETHAND != 0 {
                process.signals.actions[signal] = EMPTY_SIGNAL_ACTION;
            }
            process.signals.blocked |= action.mask;
            if action.flags & SA_NODEFER == 0 {
                process.signals.blocked |= bit;
            }
            process.signals.blocked &= !UNBLOCKABLE_MASK;
        }
        let info_address = frame_address + core::mem::offset_of!(LinuxRtSignalFrame, info);
        let context_address = frame_address + core::mem::offset_of!(LinuxRtSignalFrame, context);
        if !unsafe {
            kor_arch::setup_user_signal(
                frame,
                action.handler,
                restorer,
                frame_address,
                signal,
                info_address,
                context_address,
            )
        } {
            return Some(128 + 11);
        }
        return None;
    }
}
fn sys_futex(address: usize, operation: usize, value: usize, timeout: usize) -> isize {
    const FUTEX_WAIT: usize = 0;
    const FUTEX_WAKE: usize = 1;
    const FUTEX_WAIT_BITSET: usize = 9;
    const FUTEX_WAKE_BITSET: usize = 10;
    let operation = operation & 0x7f;
    match operation {
        FUTEX_WAIT | FUTEX_WAIT_BITSET => {
            let ptr = user_to_kernel(address);
            if ptr == 0 {
                return -14;
            }
            if !kor_sched::is_ready() {
                return -11;
            }
            // A NULL timeout means wait forever: use the blocking queue.
            if timeout == 0 {
                let blocked = futex_queue(address).wait_if(|| unsafe {
                    (*(ptr as *const core::sync::atomic::AtomicU32))
                        .load(core::sync::atomic::Ordering::SeqCst)
                        == value as u32
                });
                return if blocked { 0 } else { -11 };
            }
            // Timed wait: poll the word with a 1 ms sleep cadence —
            // EAGAIN if the value moved, ETIMEDOUT at the deadline.
            let Some([seconds, nanos]) = read_user::<[i64; 2]>(timeout) else {
                return -14;
            };
            if seconds < 0 || !(0..1_000_000_000).contains(&nanos) {
                return -22;
            }
            let deadline = kor_arch::now()
                + core::time::Duration::from_secs(seconds as u64)
                + core::time::Duration::from_nanos(nanos as u32 as u64);
            loop {
                let current = unsafe {
                    (*(ptr as *const core::sync::atomic::AtomicU32))
                        .load(core::sync::atomic::Ordering::SeqCst)
                };
                if current != value as u32 {
                    return -11;
                }
                if kor_arch::now() >= deadline {
                    return -110;
                }
                kor_sched::sleep(core::time::Duration::from_millis(1));
            }
        }
        FUTEX_WAKE | FUTEX_WAKE_BITSET => {
            let queue = futex_queue(address);
            let mut woken = 0;
            while woken < value && queue.wake_one() {
                woken += 1;
            }
            woken as isize
        }
        _ => -38,
    }
}

fn sys_wait4(pid: usize, status: usize, options: usize) -> isize {
    const WNOHANG: usize = 1;
    const WUNTRACED: usize = 2;
    const WCONTINUED: usize = 8;
    const SUPPORTED_OPTIONS: usize = WNOHANG | WUNTRACED | WCONTINUED;
    if options & !SUPPORTED_OPTIONS != 0 {
        return -22;
    }
    let requested = pid as isize;
    if requested == 0 || requested < -1 {
        return -10;
    }
    let parent = match crate::sched::current_pid() {
        Some(pid) => pid,
        None => return -10,
    };
    let events = match crate::sched::child_wait_queue(parent) {
        Some(events) => events,
        None => return -10,
    };
    if crate::sched::matching_child(parent, requested).is_none() {
        return -10; // ECHILD: no children exist (zombie or running).
    }
    loop {
        if let Some(child) = crate::sched::matching_zombie(parent, requested) {
            let (child_pid, code) = {
                let child = child.lock();
                (child.pid, child.exit_code)
            };
            if status != 0 && write_user(status, &((code & 0xff) << 8)).is_none() {
                return -14;
            }
            drop(crate::sched::reap(child_pid).expect("zombie disappeared while reaping"));
            return child_pid as isize;
        }
        if options & WNOHANG != 0 {
            return 0;
        }
        events.wait();
    }
}

fn realtime_seconds() -> u64 {
    kor_arch::now().as_secs()
}

fn sys_clock_gettime(clockid: usize, timespec: usize) -> isize {
    const CLOCK_REALTIME: usize = 0;
    const CLOCK_MONOTONIC: usize = 1;
    const CLOCK_PROCESS_CPUTIME_ID: usize = 2;
    const CLOCK_THREAD_CPUTIME_ID: usize = 3;
    const CLOCK_MONOTONIC_RAW: usize = 4;
    const CLOCK_REALTIME_RAW: usize = 5;
    const CLOCK_BOOTTIME: usize = 7;
    // All supported ids share the tick-derived time base.
    match clockid {
        CLOCK_REALTIME
        | CLOCK_MONOTONIC
        | CLOCK_PROCESS_CPUTIME_ID
        | CLOCK_THREAD_CPUTIME_ID
        | CLOCK_MONOTONIC_RAW
        | CLOCK_REALTIME_RAW
        | CLOCK_BOOTTIME => {}
        _ => return -22, // EINVAL
    }
    let now = kor_arch::now();
    let value = [now.as_secs() as i64, now.subsec_nanos() as i64];
    if write_user(timespec, &value).is_none() {
        -14
    } else {
        0
    }
}

/// clock_getres(2): every supported clock ticks at nanosecond resolution.
fn sys_clock_getres(clockid: usize, res: usize) -> isize {
    const CLOCK_MONOTONIC: usize = 1;
    if clockid > CLOCK_MONOTONIC && clockid != 4 && clockid != 7 {
        return -22;
    }
    if res != 0 {
        let value = [1i64, 0i64]; // 1 ns
        if write_user(res, &value).is_none() {
            return -14;
        }
    }
    0
}

/// clock_nanosleep(2): absolute or relative sleep on the monotonic base.
fn sys_clock_nanosleep(_clockid: usize, flags: usize, request: usize, _remain: usize) -> isize {
    const TIMER_ABSTIME: usize = 1;
    if flags & !TIMER_ABSTIME != 0 {
        return -22;
    }
    let Some(value) = read_user::<[i64; 2]>(request) else {
        return -14;
    };
    let [seconds, nanos] = value;
    if seconds < 0 || !(0..1_000_000_000).contains(&nanos) {
        return -22;
    }
    if flags & TIMER_ABSTIME != 0 {
        // Absolute deadline on the shared time base.
        let deadline = core::time::Duration::new(seconds as u64, nanos as u32);
        let mut now = kor_arch::now();
        while now < deadline {
            let remaining = deadline - now;
            kor_sched::sleep(remaining.min(core::time::Duration::from_millis(20)));
            now = kor_arch::now();
        }
        return 0;
    }
    let millis = seconds as u64 * 1000 + (nanos as u64).div_ceil(1_000_000);
    if millis != 0 && kor_sched::is_ready() {
        kor_sched::sleep(core::time::Duration::from_millis(millis));
    }
    0
}

fn sys_gettimeofday(timeval: usize) -> isize {
    if timeval == 0 {
        return 0;
    }
    let now = kor_arch::now();
    let value = [now.as_secs() as i64, (now.subsec_nanos() / 1000) as i64];
    if write_user(timeval, &value).is_none() {
        -14
    } else {
        0
    }
}

fn sys_time(result: usize) -> isize {
    let seconds = realtime_seconds() as i64;
    if result != 0 && write_user(result, &seconds).is_none() {
        return -14;
    }
    seconds as isize
}

/// reboot(2): validate the Linux magic numbers, then power the machine off.
/// Restart and halt both map to poweroff — there is no reset path back to
/// firmware yet.
fn sys_reboot(magic: usize, _magic2: usize, cmd: usize) -> isize {
    // C `int` arguments arrive sign-extended (e.g. magic1 = 0xffff_ffff_fee1_dead),
    // so compare the low 32 bits like the Linux kernel does.
    const REBOOT_MAGIC1: u32 = 0xFEE1_DEAD;
    const RB_HALT_SYSTEM: u32 = 0xCDEF_0123;
    const RB_POWER_OFF: u32 = 0x4321_FEDC;
    const RB_AUTOBOOT: u32 = 0x0123_4567;

    if magic as u32 != REBOOT_MAGIC1 {
        return -22; // -EINVAL
    }
    match cmd as u32 {
        RB_HALT_SYSTEM | RB_POWER_OFF | RB_AUTOBOOT => {
            kor::println!("[kernel] reboot: powering off");
            kor_arch::shutdown();
        }
        _ => -22, // -EINVAL
    }
}

fn sys_nanosleep(request: usize) -> isize {
    let Some(value) = read_user::<[i64; 2]>(request) else {
        return -14;
    };
    let [seconds, nanos] = value;
    if seconds < 0 || !(0..1_000_000_000).contains(&nanos) {
        return -22;
    }
    let millis = seconds as u64 * 1000 + (nanos as u64).div_ceil(1_000_000);
    if millis != 0 {
        if kor_sched::is_ready() {
            kor_sched::sleep(core::time::Duration::from_millis(millis));
        } else {
            let deadline = kor_arch::now() + core::time::Duration::from_millis(millis);
            while kor_arch::now() < deadline {
                core::hint::spin_loop();
            }
        }
    }
    0
}

/// ppoll — block until stdin or a socket has data, then return POLLIN.
/// struct pollfd { int fd; short events; short revents; } = 8 bytes
/// Convert a user `struct timespec *` (16 bytes: tv_sec, tv_nsec) into an
/// absolute deadline; `None` for null/infinite.
fn poll_deadline(timespec_ptr: usize) -> Option<core::time::Duration> {
    if timespec_ptr == 0 {
        return None;
    }
    let seconds = user_mem::read::<usize>(timespec_ptr)? as u64;
    let nanos = user_mem::read::<usize>(timespec_ptr + core::mem::size_of::<usize>())? as u64;
    let timeout = core::time::Duration::new(seconds, nanos.min(1_000_000_000) as u32);
    Some(kor_arch::now() + timeout)
}

fn sys_ppoll(fds: usize, nfds: usize, tmo_p: usize) -> isize {
    if nfds == 0 {
        return 0;
    }
    // tmo_p: `struct timespec { tv_sec, tv_nsec }` in kernel timebase
    // units below; null = infinite.  Honoring the timeout is what lets
    // programs time out an escape-sequence probe (busybox vi's ESC key).
    let deadline = poll_deadline(tmo_p);
    let Some(byte_len) = nfds.checked_mul(8) else {
        return -22;
    };
    if checked_user_range(fds, byte_len, true).is_none() {
        return -14;
    }
    const POLLIN: u16 = 0x001;
    loop {
        let mut ready = 0;
        for index in 0..nfds {
            let Some(pfd) = fds.checked_add(index * 8) else {
                return -14;
            };
            let Some(fd) = read_user::<i32>(pfd) else {
                return -14;
            };
            let Some(events) = read_user::<u16>(pfd + 4) else {
                return -14;
            };
            let mut revents = 0u16;
            if events & POLLIN != 0 {
                let readable = (fd == 0 && rx_has_data())
                    || (fd >= 0 && socket_id(fd as usize).is_ok_and(crate::net::has_data));
                if readable {
                    revents = POLLIN;
                }
            }
            if revents != 0 {
                ready += 1;
            }
            if write_user(pfd + 6, &revents).is_none() {
                return -14;
            }
        }
        if ready != 0 {
            return ready;
        }
        if let Some(deadline) = deadline
            && kor_arch::now() >= deadline
        {
            return 0;
        }
        crate::net::wait();
    }
}

fn sys_select(
    nfds: usize,
    readfds: usize,
    writefds: usize,
    exceptfds: usize,
    timeout: usize,
) -> isize {
    const BITS_PER_WORD: usize = usize::BITS as usize;
    let word_count = nfds.div_ceil(BITS_PER_WORD);
    let mut requested_read = Vec::with_capacity(word_count);
    let mut requested_write = Vec::with_capacity(word_count);
    let mut requested_except = Vec::with_capacity(word_count);
    for word in 0..word_count {
        let offset = word * core::mem::size_of::<usize>();
        requested_read.push(if readfds == 0 {
            0
        } else {
            read_user::<usize>(readfds + offset).unwrap_or(0)
        });
        requested_write.push(if writefds == 0 {
            0
        } else {
            read_user::<usize>(writefds + offset).unwrap_or(0)
        });
        requested_except.push(if exceptfds == 0 {
            0
        } else {
            read_user::<usize>(exceptfds + offset).unwrap_or(0)
        });
    }
    loop {
        let mut ready = 0;
        let mut ready_read = alloc::vec![0usize; word_count];
        let mut ready_write = alloc::vec![0usize; word_count];
        for fd in 0..nfds {
            let word = fd / BITS_PER_WORD;
            let bit = 1usize << (fd % BITS_PER_WORD);
            if requested_read[word] & bit != 0 {
                let readable =
                    (fd == 0 && rx_has_data()) || socket_id(fd).is_ok_and(crate::net::has_data);
                if readable {
                    ready_read[word] |= bit;
                    ready += 1;
                }
            }
            if requested_write[word] & bit != 0
                && (socket_id(fd).is_ok()
                    || current_file(fd).is_some()
                    || matches!(stdio_target(fd), Some(1 | 2)))
            {
                ready_write[word] |= bit;
                ready += 1;
            }
        }
        if ready != 0 {
            for word in 0..word_count {
                let offset = word * core::mem::size_of::<usize>();
                if readfds != 0 && write_user(readfds + offset, &ready_read[word]).is_none() {
                    return -14;
                }
                if writefds != 0 && write_user(writefds + offset, &ready_write[word]).is_none() {
                    return -14;
                }
                if exceptfds != 0 && write_user(exceptfds + offset, &0usize).is_none() {
                    return -14;
                }
            }
            return ready;
        }
        if let Some(deadline) = poll_deadline(timeout)
            && kor_arch::now() >= deadline
        {
            return 0;
        }
        crate::net::wait();
    }
}
#[unsafe(no_mangle)]
pub extern "Rust" fn korix_set_current_tf(tf: *mut u8) {
    if !kor_sched::is_ready() {
        return;
    }
    let task = kor_sched::current();
    if tf.is_null() {
        task.clear_user_context();
    } else {
        task.set_user_context(tf);
    }
}

pub fn user_trap_enter(frame: *mut u8) {
    korix_set_current_tf(frame);
}

pub fn user_trap_leave(frame: *mut u8) {
    let frame = frame;
    let task = kor_sched::current();
    if task.with_user_context(|active| core::ptr::eq(active, frame)) == Some(true) {
        task.clear_user_context();
    }
}

pub fn user_trap_status(frame: *mut u8) -> Option<i32> {
    korix_deliver_pending_signal(frame)
}

fn sys_fstatat(dirfd: usize, path: usize, stat: usize, flags: usize) -> isize {
    const AT_EMPTY_PATH: usize = 0x1000;
    const AT_SYMLINK_NOFOLLOW: usize = 0x100;
    if flags & !(AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW) != 0 {
        return -22;
    }
    let user_path = match user_to_str(path, 4096) {
        Some(path) => path,
        None => return -14,
    };
    let inode = if user_path.is_empty() && flags & AT_EMPTY_PATH != 0 {
        match current_file(dirfd) {
            Some(entry) => entry.lock().inode.clone(),
            None => return -9,
        }
    } else if flags & AT_SYMLINK_NOFOLLOW != 0 {
        let absolute = match absolute_path(dirfd, &user_path) {
            Ok(path) => path,
            Err(error) => return error,
        };
        let (parent_path, name) = match split_absolute_parent(&absolute) {
            Ok(parts) => parts,
            Err(error) => return error,
        };
        let parent = match crate::fs::resolve(parent_path) {
            Ok(parent) => parent,
            Err(error) => return fs_errno(error),
        };
        match parent.lookup(name) {
            Ok(inode) => inode,
            Err(error) => return fs_errno(error),
        }
    } else {
        match absolute_path(dirfd, &user_path)
            .and_then(|path| crate::fs::resolve(&path).map_err(fs_errno))
        {
            Ok(inode) => inode,
            Err(error) => return error,
        }
    };
    let metadata = match inode.getattr() {
        Ok(metadata) => metadata,
        Err(error) => return fs_errno(error),
    };
    let st = linux_stat(metadata, inode.ino() as u64);
    if write_user(stat, &st).is_none() {
        -14
    } else {
        0
    }
}
