//! Linux syscall compatibility layer.
//!
//! Implements a subset of the Linux syscall ABI so that statically-linked
//! Linux user programs can run under korix.
//!
//! Syscall numbers follow the generic Linux syscall table (used by
//! riscv64, aarch64). x86_64 uses a different table — handled by
//! arch::x86_64::syscall_translate (translate_syscall below).
//!
//! # Calling convention
//! - riscv64: ecall, a7=nr, a0-a5=args, a0=return
//! - aarch64: svc #0, x8=nr, x0-x5=args, x0=return
//! - x86_64:  syscall, rax=nr, rdi,rsi,rdx,r10,r8,r9=args, rax=return

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use kor::INode;
use spin::Mutex;

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
pub const SYS_GETRANDOM: usize = 278;
pub const SYS_WRITEV: usize = 66;
pub const SYS_READV: usize = 65;
pub const SYS_MPROTECT: usize = 226;

// x86_64 syscall numbers — a different table from the generic (riscv64/aarch64)
// one used by dispatch_inner. translate_syscall() maps these to generic numbers.
#[cfg(target_arch = "x86_64")]
mod x86_nr {
    pub const READ: usize = 0;
    pub const WRITE: usize = 1;
    pub const CLOSE: usize = 3;
    pub const OPEN: usize = 2;
    pub const STAT: usize = 4;
    pub const LSTAT: usize = 6;
    pub const FSTAT: usize = 5;
    pub const LSEEK: usize = 8;
    pub const MMAP: usize = 9;
    pub const MPROTECT: usize = 10;
    pub const MUNMAP: usize = 11;
    pub const BRK: usize = 12;
    pub const RT_SIGACTION: usize = 13;
    pub const RT_SIGPROCMASK: usize = 14;
    pub const RT_SIGSUSPEND: usize = 130;
    pub const RT_SIGRETURN: usize = 15;
    pub const IOCTL: usize = 16;
    pub const WRITEV: usize = 20;
    pub const DUP: usize = 32;
    pub const DUP2: usize = 33;
    pub const NANOSLEEP: usize = 35;
    pub const POLL: usize = 7;
    pub const GETPID: usize = 39;
    pub const CLONE: usize = 56;
    pub const FORK: usize = 57;
    pub const EXECVE: usize = 59;
    pub const EXIT: usize = 60;
    pub const WAIT4: usize = 61;
    pub const KILL: usize = 62;
    pub const UNAME: usize = 63;
    pub const RENAME: usize = 82;
    pub const MKDIR: usize = 83;
    pub const RMDIR: usize = 84;
    pub const LINK: usize = 86;
    pub const UNLINK: usize = 87;
    pub const SYMLINK: usize = 88;
    pub const FCNTL: usize = 72;
    pub const GETDENTS64: usize = 217;
    pub const GETCWD: usize = 79;
    pub const CHDIR: usize = 80;
    pub const GETUID: usize = 102;
    pub const GETGID: usize = 104;
    pub const GETEUID: usize = 107;
    pub const GETEGID: usize = 108;
    pub const GETPPID: usize = 110;
    pub const GETPGRP: usize = 111;
    pub const SETSID: usize = 112;
    pub const GETGROUPS: usize = 115;
    pub const GETPGID: usize = 121;
    pub const GETSID: usize = 124;
    pub const SCHED_SETSCHEDULER: usize = 144;
    pub const SCHED_GETSCHEDULER: usize = 145;
    pub const SCHED_GETPARAM: usize = 159;
    pub const SCHED_SETPARAM: usize = 142;
    pub const SCHED_SETAFFINITY: usize = 203;
    pub const SCHED_GETAFFINITY: usize = 204;
    pub const PRCTL: usize = 157;
    pub const SYNC: usize = 162;
    pub const MOUNT: usize = 165;
    pub const UMOUNT2: usize = 166;
    pub const GETTID: usize = 186;
    pub const FUTEX: usize = 202;
    pub const SET_TID_ADDRESS: usize = 218;
    pub const EXIT_GROUP: usize = 231;
    pub const TGKILL: usize = 234;
    pub const CLOCK_GETTIME: usize = 228;
    pub const GETTIMEOFDAY: usize = 96;
    pub const OPENAT: usize = 257;
    pub const MKDIRAT: usize = 258;
    pub const RENAMEAT2: usize = 316;
    pub const NEWFSTATAT: usize = 262;
    pub const UNLINKAT: usize = 263;
    pub const RENAMEAT: usize = 264;
    pub const SYMLINKAT: usize = 266;
    pub const LINKAT: usize = 265;
    pub const READLINKAT: usize = 267;
    pub const PPOLL: usize = 271;
    pub const SET_ROBUST_LIST: usize = 273;
    pub const PRLIMIT64: usize = 302;
    pub const GETRANDOM: usize = 318;
    pub const RSEQ: usize = 334;
}

// Physical RAM start — used to distinguish user pages from identity-mapped kernel pages.
#[cfg(target_arch = "riscv64")]
pub const RAM_START: usize = 0x8000_0000;
#[cfg(target_arch = "aarch64")]
pub const RAM_START: usize = 0x4000_0000;
#[cfg(target_arch = "x86_64")]
pub const RAM_START: usize = 0x1_0000_0000;
#[cfg(not(any(
    target_arch = "riscv64",
    target_arch = "aarch64",
    target_arch = "x86_64"
)))]
pub const RAM_START: usize = 0x8000_0000;
const AT_FDCWD: isize = -100;
/// Dispatch a syscall.
/// num is the syscall number, args is up to 6 arguments.
/// Returns the syscall result (positive = success, negative = -errno).
fn current_user_phys(address: usize, write: bool) -> Option<usize> {
    crate::sched::with_current_process(|process| {
        let process = process.lock();
        let space = process.address_space()?;
        space
            .validate_range(address, 1, write)
            .then(|| space.translate(address))?
    })?
}

/// Translate one validated user byte. Bulk callers must use pagewise copy
/// helpers instead of extending this pointer across a page boundary.
fn user_to_kernel(address: usize) -> usize {
    current_user_phys(address, false)
        .map(kor::arch::phys_to_virt)
        .unwrap_or(0)
}
fn checked_user_range(address: usize, length: usize, write: bool) -> Option<()> {
    if address == 0 {
        return None;
    }
    crate::sched::with_current_process(|process| {
        process
            .lock()
            .address_space()
            .is_some_and(|space| space.validate_range(address, length, write))
    })?
    .then_some(())
}

fn copy_from_user(address: usize, destination: &mut [u8]) -> Option<()> {
    checked_user_range(address, destination.len(), false)?;
    let mut copied = 0;
    while copied < destination.len() {
        let user = address.checked_add(copied)?;
        let physical = current_user_phys(user, false)?;
        let chunk = (0x1000 - (user & 0xfff)).min(destination.len() - copied);
        unsafe {
            core::ptr::copy_nonoverlapping(
                kor::arch::current().phys_to_virt(physical) as *const u8,
                destination.as_mut_ptr().add(copied),
                chunk,
            );
        }
        copied += chunk;
    }
    Some(())
}

fn copy_to_user(address: usize, source: &[u8]) -> Option<()> {
    checked_user_range(address, source.len(), true)?;
    let mut copied = 0;
    while copied < source.len() {
        let user = address.checked_add(copied)?;
        let physical = current_user_phys(user, true)?;
        let chunk = (0x1000 - (user & 0xfff)).min(source.len() - copied);
        unsafe {
            core::ptr::copy_nonoverlapping(
                source.as_ptr().add(copied),
                kor::arch::current().phys_to_virt(physical) as *mut u8,
                chunk,
            );
        }
        copied += chunk;
    }
    Some(())
}

fn read_user<T: Copy>(address: usize) -> Option<T> {
    let mut value = core::mem::MaybeUninit::<T>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), core::mem::size_of::<T>())
    };
    copy_from_user(address, bytes)?;
    Some(unsafe { value.assume_init() })
}

fn write_user<T: Copy>(address: usize, value: &T) -> Option<()> {
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), core::mem::size_of::<T>())
    };
    copy_to_user(address, bytes)
}

fn user_to_str(address: usize, max: usize) -> Option<String> {
    let mut bytes = Vec::new();
    for offset in 0..max {
        let byte: u8 = read_user(address.checked_add(offset)?)?;
        if byte == 0 {
            return String::from_utf8(bytes).ok();
        }
        bytes.push(byte);
    }
    None
}

// Kernel-side UART RX buffer (compensates for small hardware FIFO)
pub static RX_BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());

pub fn rx_fill() {
    let mut buf = RX_BUF.lock();
    #[cfg(target_arch = "riscv64")]
    while let Some(b) = kor_arch::uart::ns16550a::getchar_mmio(0x1000_0000) {
        buf.push(b);
    }
    #[cfg(target_arch = "aarch64")]
    while let Some(b) = kor_arch::uart::pl011::getchar(0x0900_0000) {
        buf.push(b);
    }
    #[cfg(target_arch = "x86_64")]
    while let Some(b) = kor_arch::uart::ns16550a::getchar_port(0x3F8) {
        buf.push(b);
    }
}

fn rx_get() -> Option<u8> {
    rx_fill();
    let mut buf = RX_BUF.lock();
    if buf.is_empty() {
        None
    } else {
        Some(buf.remove(0))
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
            restorer: 0,
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
    let arch = kor::arch::current();
    if arch.translate(SIGNAL_RESTORER_ADDRESS).is_none() {
        let physical = kor_frame::alloc_page()?;
        let flags = kor::MappingFlags::U | kor::MappingFlags::R | kor::MappingFlags::X;
        if arch
            .map(
                SIGNAL_RESTORER_ADDRESS,
                physical,
                flags,
                kor::MapSize::Page4K,
            )
            .is_err()
        {
            kor_frame::free_frames(physical, 1);
            return None;
        }
        let kernel = arch.phys_to_virt(physical);
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
    }
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

pub fn dispatch(num: usize, args: &[usize]) -> isize {
    #[cfg(target_arch = "x86_64")]
    let raw_num = num;
    #[cfg(target_arch = "x86_64")]
    if raw_num == 158 {
        return sys_arch_prctl(args[0], args[1]);
    }
    #[cfg(target_arch = "x86_64")]
    match raw_num {
        x86_nr::OPEN => return sys_openat(AT_FDCWD as usize, args[0], args[1], args[2]),
        x86_nr::STAT => return sys_fstatat(AT_FDCWD as usize, args[0], args[1], 0),
        x86_nr::LSTAT => return sys_fstatat(AT_FDCWD as usize, args[0], args[1], 0x100),
        x86_nr::RENAME => {
            return sys_renameat(AT_FDCWD as usize, args[0], AT_FDCWD as usize, args[1]);
        }
        x86_nr::MKDIR => return sys_mkdirat(AT_FDCWD as usize, args[0], args[1]),
        x86_nr::RMDIR => return sys_unlinkat(AT_FDCWD as usize, args[0], 0x200),
        x86_nr::LINK => {
            return sys_linkat(AT_FDCWD as usize, args[0], AT_FDCWD as usize, args[1], 0);
        }
        x86_nr::UNLINK => return sys_unlinkat(AT_FDCWD as usize, args[0], 0),
        x86_nr::SYMLINK => return sys_symlinkat(args[0], AT_FDCWD as usize, args[1]),
        _ => {}
    }
    #[cfg(target_arch = "x86_64")]
    let num = translate_syscall(num);
    dispatch_inner(num, args)
}

/// Map an x86_64 syscall number to the generic number used by dispatch_inner.
/// x86_64 numbers (read=0, write=1, exit=60, ...) do not align with the
/// generic table; without translation they would hit the wrong handlers or
/// collide dangerously (e.g. x86_64 uname=63 == generic read=63).
#[cfg(target_arch = "x86_64")]
fn translate_syscall(nr: usize) -> usize {
    use x86_nr::*;
    match nr {
        READ => SYS_READ,
        WRITE => SYS_WRITE,
        CLOSE => SYS_CLOSE,
        FSTAT => SYS_FSTAT,
        LSEEK => SYS_LSEEK,
        MMAP => SYS_MMAP,
        MPROTECT => SYS_MPROTECT,
        MUNMAP => SYS_MUNMAP,
        BRK => SYS_BRK,
        RT_SIGACTION => SYS_RT_SIGACTION,
        RT_SIGPROCMASK => SYS_RT_SIGPROCMASK,
        RT_SIGSUSPEND => SYS_RT_SIGSUSPEND,
        RT_SIGRETURN => SYS_RT_SIGRETURN,
        IOCTL => SYS_IOCTL,
        WRITEV => SYS_WRITEV,
        DUP => SYS_DUP,
        DUP2 => SYS_DUP2,
        NANOSLEEP => SYS_NANOSLEEP,
        POLL => 73, // poll → ppoll handler
        GETPID => SYS_GETPID,
        CLONE => SYS_CLONE,
        FORK => SYS_CLONE, // fork → clone handler
        EXECVE => SYS_EXECVE,
        EXIT => SYS_EXIT,
        WAIT4 => SYS_WAIT4,
        KILL => SYS_KILL,
        UNAME => SYS_UNAME,
        FCNTL => SYS_FCNTL,
        GETDENTS64 => SYS_GETDENTS64,
        RENAME => SYS_RENAMEAT,
        MKDIR => SYS_MKDIRAT,
        RMDIR => SYS_UNLINKAT,
        LINK => SYS_LINKAT,
        UNLINK => SYS_UNLINKAT,
        SYMLINK => SYS_SYMLINKAT,
        GETCWD => SYS_GETCWD,
        CHDIR => SYS_CHDIR,
        GETUID => SYS_GETUID,
        GETGID => SYS_GETGID,
        GETEUID => 175,
        GETEGID => 177,
        GETPPID => SYS_GETPPID,
        GETPGRP => 155,
        SETSID => 157,
        GETGROUPS => 158,
        GETPGID => 155,
        GETSID => 156,
        SCHED_SETSCHEDULER => 144,
        SCHED_GETSCHEDULER => 145,
        SCHED_GETPARAM => 146,
        SCHED_SETPARAM => 143,
        SCHED_SETAFFINITY => 148,
        SCHED_GETAFFINITY => 149,
        PRCTL => 167,
        SYNC => SYS_SYNC,
        MOUNT => SYS_MOUNT,
        UMOUNT2 => SYS_UMOUNT2,
        GETTID => 178,
        FUTEX => SYS_FUTEX,
        SET_TID_ADDRESS => SYS_SET_TID_ADDRESS,
        EXIT_GROUP => SYS_EXIT_GROUP,
        TGKILL => SYS_TGKILL,
        CLOCK_GETTIME => 113,
        GETTIMEOFDAY => 169,
        RENAMEAT2 => SYS_RENAMEAT2,
        OPENAT => SYS_OPENAT,
        MKDIRAT => SYS_MKDIRAT,
        NEWFSTATAT => SYS_NEWFSTATAT,
        UNLINKAT => SYS_UNLINKAT,
        RENAMEAT => SYS_RENAMEAT,
        LINKAT => SYS_LINKAT,
        READLINKAT => SYS_READLINKAT,
        PPOLL => 73,
        SYMLINKAT => SYS_SYMLINKAT,
        SET_ROBUST_LIST => SYS_SET_ROBUST_LIST,
        PRLIMIT64 => 261,
        GETRANDOM => SYS_GETRANDOM,
        RSEQ => 293,
        201 => 201, // time (identity)
        // Unknown x86_64 syscalls → ENOSYS via dispatch_inner's catch-all.
        _ => 0xFFFF,
    }
}

#[cfg(target_arch = "x86_64")]
fn sys_arch_prctl(code: usize, addr: usize) -> isize {
    const ARCH_SET_GS: usize = 0x1001;
    const ARCH_SET_FS: usize = 0x1002;
    const ARCH_GET_FS: usize = 0x1003;
    const ARCH_GET_GS: usize = 0x1004;
    const IA32_FS_BASE: u32 = 0xC000_0100;
    const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;

    unsafe fn read_msr(msr: u32) -> u64 {
        let low: u32;
        let high: u32;
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            lateout("eax") low,
            lateout("edx") high,
            options(nostack),
        );
        ((high as u64) << 32) | low as u64
    }

    unsafe fn write_msr(msr: u32, value: u64) {
        core::arch::asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nostack),
        );
    }

    match code {
        ARCH_SET_FS => unsafe {
            write_msr(IA32_FS_BASE, addr as u64);
            0
        },
        ARCH_SET_GS => unsafe {
            // Native SYSCALL has swapped back to kernel GS, so the user's GS
            // base is parked in IA32_KERNEL_GS_BASE until return.
            write_msr(IA32_KERNEL_GS_BASE, addr as u64);
            0
        },
        ARCH_GET_FS | ARCH_GET_GS => {
            let msr = if code == ARCH_GET_FS {
                IA32_FS_BASE
            } else {
                IA32_KERNEL_GS_BASE
            };
            if write_user(addr, &unsafe { read_msr(msr) }).is_none() {
                -14
            } else {
                0
            }
        }
        _ => -22,
    }
}

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
        SYS_GETUID | 175 => 0, // GETUID | GETEUID
        SYS_GETGID | 177 => 0, // GETGID | GETEGID
        SYS_SYNC => {
            if kor_fs::mount::sync_all(&crate::MOUNTS).is_ok() {
                0
            } else {
                -5
            }
        }
        SYS_NANOSLEEP => sys_nanosleep(args[0]),
        SYS_UNAME => sys_uname(args[0]),
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
        SYS_FUTEX => sys_futex(args[0], args[1], args[2]),
        SYS_IOCTL => sys_ioctl(args[0], args[1], args[2]),
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
        113 => sys_clock_gettime(args[1]),
        157 => {
            let pid = current_pid();
            *SESSION_ID.lock() = pid;
            *PROCESS_GROUP.lock() = pid;
            pid as isize
        }
        169 => sys_gettimeofday(args[0]),
        201 => sys_time(args[0]),
        73 => sys_ppoll(args[0], args[1]), // ppoll

        167 => sys_prctl(args),
        154 | 149 => -38, // sched_getaffinity
        158 => 0,         // getgroups: no supplementary groups
        156 => *SESSION_ID.lock() as isize,
        144 | 143 | 148 => -38, // scheduler mutation is unsupported
        145 => 0,               // sched_getscheduler: SCHED_OTHER
        146 => -38,             // sched_getparam requires an output parameter
        178 => current_pid() as isize,
        3 => -38, // io_cancel
        0 => -38, // io_setup
        2 => -2,  // legacy open
        293 => 0, // rseq: single-threaded userspace needs no restartable sequence area
        261 => {
            // prlimit64 → return default limits
            if args[3] != 0 {
                let rlim = [u64::MAX, u64::MAX];
                if copy_to_user(args[3], unsafe {
                    core::slice::from_raw_parts(
                        rlim.as_ptr().cast::<u8>(),
                        core::mem::size_of_val(&rlim),
                    )
                })
                .is_none()
                {
                    return -14;
                }
            }
            0
        }
        _ => -38,
    }
}

pub(crate) struct FileEntry {
    inode: Arc<dyn INode>,
    offset: u64,
    path: String,
}
pub(crate) type SharedFile = Arc<Mutex<FileEntry>>;

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
    let mut fd = 3;
    while process.files.contains_key(&fd) {
        fd += 1;
    }
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

// --- Syscall implementations ---

fn sys_write(fd: usize, buf: usize, count: usize) -> isize {
    let mut data = alloc::vec![0u8; count];
    if copy_from_user(buf, &mut data).is_none() {
        return -14;
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
    if checked_user_range(buf, count, true).is_none() {
        return -14;
    }
    let mut data = alloc::vec![0u8; count];
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
    if fd != 1 && fd != 2 {
        return -9;
    }
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
        let mut data = alloc::vec![0u8; length];
        if copy_from_user(base, &mut data).is_none() {
            return -14;
        }
        for byte in data {
            kor::console::putc(byte);
        }
        let Some(next) = total.checked_add(length) else {
            return -22;
        };
        total = next;
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
    }
    #[cfg(target_arch = "aarch64")]
    {
        context.regs[0] = 0;
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
        child.mmap_next = child_mmap_next;
        child.files = child_files;
        child.stdio = child_stdio;
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
                    *(kor::arch::current().phys_to_virt(paddr) as *mut u32) = pid as u32;
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
    let mut replacement = match crate::mm::AddressSpace::empty() {
        Ok(address_space) => address_space,
        Err(_) => return -12,
    };
    let program = match crate::user::elf::load_elf(&mut replacement, &image, &args) {
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
        let flags = kor::MappingFlags::U | kor::MappingFlags::R | kor::MappingFlags::W;
        let mut mapped = page_start;
        while mapped < page_end {
            let result = process
                .address_space_mut()
                .ok_or(kor::MapError::Unsupported)
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
    let path_str = match user_absolute_path(dirfd, path) {
        Ok(path) => path,
        Err(error) => return error,
    };
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
    let inode = match crate::fs::resolve(&path_str) {
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
    if fd <= 2 {
        return 0;
    }
    let Some(process) = crate::sched::current_process() else {
        return -9;
    };
    let mut process = process.lock();
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
    if file.is_none() && stdio.is_none() {
        return -9;
    }
    let mut newfd = 3;
    while process.files.contains_key(&newfd) || process.stdio.contains_key(&newfd) {
        newfd += 1;
    }
    if let Some(entry) = file {
        process.files.insert(newfd, entry);
    } else if let Some(target) = stdio {
        process.stdio.insert(newfd, target);
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
    if file.is_none() && stdio.is_none() {
        return -9;
    }
    if fd == newfd {
        return newfd as isize;
    }
    process.files.remove(&newfd);
    process.stdio.remove(&newfd);
    if let Some(entry) = file {
        process.files.insert(newfd, entry);
    } else if let Some(target) = stdio {
        process.stdio.insert(newfd, target);
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
    if file.is_none() && stdio.is_none() {
        return -9;
    }
    match command {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            let mut newfd = argument.max(3);
            while process.files.contains_key(&newfd) || process.stdio.contains_key(&newfd) {
                newfd = match newfd.checked_add(1) {
                    Some(value) => value,
                    None => return -24,
                };
            }
            if let Some(entry) = file {
                process.files.insert(newfd, entry);
            } else if let Some(target) = stdio {
                process.stdio.insert(newfd, target);
            }
            newfd as isize
        }
        F_GETFD | F_GETFL => 0,
        F_SETFD | F_SETFL => 0,
        _ => -22,
    }
}

/// ioctl handler — supports TCGETS so the shell thinks stdin is a terminal.
fn sys_ioctl(fd: usize, request: usize, argp: usize) -> isize {
    // TCGETS = 0x5401: get terminal attributes
    if request == 0x5401 && fd <= 2 {
        // struct termios: c_iflag, c_oflag, c_cflag, c_lflag, c_line, c_cc[19]
        // Total size = 4*4 + 1 + 19 = 36 bytes
        // Set ECHO | ICANON | ISIG so the shell echoes and does line editing
        const ECHO: u32 = 0o10;
        const ICANON: u32 = 0o2;
        const ISIG: u32 = 0o1;
        const ECHOE: u32 = 0o20;
        const ECHOK: u32 = 0o40;
        const IEXTEN: u32 = 0o100000;
        const ICRNL: u32 = 0o400;
        const OPOST: u32 = 0o1;
        const ONLCR: u32 = 0o4;

        let mut termios = [0u8; 36];
        termios[0..4].copy_from_slice(&ICRNL.to_ne_bytes());
        termios[4..8].copy_from_slice(&(OPOST | ONLCR).to_ne_bytes());
        termios[8..12].copy_from_slice(&0o141u32.to_ne_bytes());
        termios[12..16]
            .copy_from_slice(&(ECHO | ICANON | ISIG | ECHOE | ECHOK | IEXTEN).to_ne_bytes());
        termios[17..24].copy_from_slice(&[3, 28, 127, 21, 4, 0, 1]);
        return if copy_to_user(argp, &termios).is_some() {
            0
        } else {
            -14
        };
    }
    // TCSETS = 0x5402, TCSETSW = 0x5403, TCSETSF = 0x5404: set terminal attrs
    if request >= 0x5402 && request <= 0x5404 {
        return if checked_user_range(argp, 36, false).is_some() {
            0
        } else {
            -14
        };
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

fn linux_stat(meta: kor::Metadata) -> LinuxStat {
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
            st_ino: 0,
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
            st_dev: 0,
            st_ino: 0,
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
    let st = linux_stat(entry.inode.getattr().unwrap());
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

fn sys_mmap(
    address: usize,
    mut length: usize,
    prot: usize,
    flags: usize,
    _fd: usize,
    _offset: i64,
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
        let start = process.mmap_next;
        let Some(next) = start.checked_add((length + 4095) & !4095) else {
            return -12;
        };
        process.mmap_next = next;
        start
    };
    let Some(rounded) = length.checked_add(4095).map(|value| value & !4095) else {
        return -22;
    };
    let mut map_flags = kor::MappingFlags::U;
    if prot & 1 != 0 {
        map_flags = map_flags | kor::MappingFlags::R;
    }
    if prot & 2 != 0 {
        map_flags = map_flags | kor::MappingFlags::W;
    }
    if prot & 4 != 0 {
        map_flags = map_flags | kor::MappingFlags::X;
    }
    // Linux anonymous mappings without explicit PROT bits are inaccessible;
    // BusyBox requests the required permissions explicitly.
    let Some(process) = crate::sched::current_process() else {
        return -12;
    };
    let mut process = process.lock();
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
                return if error == kor::MapError::OutOfMemory {
                    -12
                } else {
                    -22
                };
            }
            mapped += 4096;
        }
        Ok(())
    };
    match result {
        Ok(()) => page_start as isize,
        Err(kor::MapError::OutOfMemory) => -12,
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
    let mut flags = kor::MappingFlags::U;
    if prot & 1 != 0 {
        flags = flags | kor::MappingFlags::R;
    }
    if prot & 2 != 0 {
        flags = flags | kor::MappingFlags::W;
    }
    if prot & 4 != 0 {
        flags = flags | kor::MappingFlags::X;
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
fn sys_futex(address: usize, operation: usize, value: usize) -> isize {
    const FUTEX_WAIT: usize = 0;
    const FUTEX_WAKE: usize = 1;
    let operation = operation & 0x7f;
    match operation {
        FUTEX_WAIT => {
            let ptr = user_to_kernel(address);
            if ptr == 0 {
                return -14;
            }
            if !kor_sched::is_ready() {
                return -11;
            }
            let blocked = futex_queue(address).wait_if(|| unsafe {
                (*(ptr as *const core::sync::atomic::AtomicU32))
                    .load(core::sync::atomic::Ordering::SeqCst)
                    == value as u32
            });
            if blocked { 0 } else { -11 }
        }
        FUTEX_WAKE => {
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
    loop {
        if let Some(child) = crate::sched::matching_zombie(parent, requested) {
            let (child_pid, code) = {
                let child = child.lock();
                (child.pid, child.exit_code)
            };
            if status != 0 && write_user(status, &((code & 0xff) << 8)).is_none() {
                return -14;
            }
            let reaped = crate::sched::reap(child_pid).expect("zombie disappeared while reaping");
            drop(reaped);
            return child_pid as isize;
        }
        if options & WNOHANG != 0 {
            return 0;
        }
        events.wait();
    }
}

fn realtime_seconds() -> u64 {
    let arch = kor::arch::current();
    let hz = arch.timer_hz().max(1);
    arch.now_ticks() / hz
}

fn sys_clock_gettime(timespec: usize) -> isize {
    let arch = kor::arch::current();
    let hz = arch.timer_hz().max(1);
    let ticks = arch.now_ticks();
    let value = [
        (ticks / hz) as i64,
        ((ticks % hz) * 1_000_000_000 / hz) as i64,
    ];
    if write_user(timespec, &value).is_none() {
        -14
    } else {
        0
    }
}

fn sys_gettimeofday(timeval: usize) -> isize {
    if timeval == 0 {
        return 0;
    }
    let arch = kor::arch::current();
    let hz = arch.timer_hz().max(1);
    let ticks = arch.now_ticks();
    let value = [(ticks / hz) as i64, ((ticks % hz) * 1_000_000 / hz) as i64];
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
            kor_sched::sleep_ms(millis);
        } else {
            let start = kor::arch::current().now_ticks();
            let wait = millis * kor::arch::current().timer_hz() / 1000;
            while kor::arch::current().now_ticks().wrapping_sub(start) < wait {
                core::hint::spin_loop();
            }
        }
    }
    0
}

/// ppoll — block until stdin has data, then return POLLIN.
/// struct pollfd { int fd; short events; short revents; } = 8 bytes
fn sys_ppoll(fds: usize, nfds: usize) -> isize {
    if nfds == 0 {
        return 0;
    }
    let Some(byte_len) = nfds.checked_mul(8) else {
        return -22;
    };
    if checked_user_range(fds, byte_len, true).is_none() {
        return -14;
    }
    const POLLIN: u16 = 0x001;
    loop {
        let mut ready = 0;
        for i in 0..nfds {
            let Some(pfd) = fds.checked_add(i * 8) else {
                return -14;
            };
            let Some(fd) = read_user::<i32>(pfd) else {
                return -14;
            };
            let Some(events) = read_user::<u16>(pfd + 4) else {
                return -14;
            };
            let mut revents = 0u16;
            if fd == 0 && events & POLLIN != 0 && rx_has_data() {
                revents = POLLIN;

                ready += 1;
            }
            if write_user(pfd + 6, &revents).is_none() {
                return -14;
            }
        }
        if ready != 0 {
            return ready;
        }
        if kor_sched::is_ready() {
            kor_sched::sleep_ms(10);
        } else {
            core::hint::spin_loop();
        }
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
        let guard = task.user_context_scope(tf);
        core::mem::forget(guard);
    }
}

pub unsafe fn user_trap_enter(frame: *mut u8) {
    korix_set_current_tf(frame);
}

pub unsafe fn user_trap_leave(frame: *mut u8) {
    let task = kor_sched::current();
    if task.with_user_context(|active| core::ptr::eq(active, frame)) == Some(true) {
        task.clear_user_context();
    }
}
pub unsafe fn user_trap_status(frame: *mut u8) -> Option<i32> {
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
    let st = linux_stat(metadata);
    if write_user(stat, &st).is_none() {
        -14
    } else {
        0
    }
}
