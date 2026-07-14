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
use spin::Mutex;
use kor::INode;

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
    pub const FCNTL: usize = 72;
    pub const GETDENTS64: usize = 78;
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
    pub const NEWFSTATAT: usize = 262;
    pub const UNLINKAT: usize = 263;
    pub const RENAMEAT: usize = 264;
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
#[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64")))]
pub const RAM_START: usize = 0x8000_0000;
/// Dispatch a syscall.
/// num is the syscall number, args is up to 6 arguments.
/// Returns the syscall result (positive = success, negative = -errno).
fn user_to_kernel(uva: usize) -> usize {
    let arch = kor::arch::current();
    match arch.translate(uva) {
        Some(p) => arch.phys_to_virt(p),
        None => 0,
    }
}

fn checked_user_range(address: usize, length: usize) -> Option<()> {
    if address == 0 || address >= USER_ADDRESS_LIMIT { return None; }
    let end = address.checked_add(length)?;
    if end > USER_ADDRESS_LIMIT { return None; }
    let mut page = address & !0xfff;
    let last = end.saturating_sub(1) & !0xfff;
    loop {
        kor::arch::current().translate(page)?;
        if page == last { break; }
        page = page.checked_add(0x1000)?;
    }
    Some(())
}

fn read_user<T: Copy>(address: usize) -> Option<T> {
    checked_user_range(address, core::mem::size_of::<T>())?;
    let mut value = core::mem::MaybeUninit::<T>::uninit();
    let mut copied = 0usize;
    while copied < core::mem::size_of::<T>() {
        let user = address + copied;
        let chunk = (0x1000 - (user & 0xfff)).min(core::mem::size_of::<T>() - copied);
        let kernel = user_to_kernel(user);
        if kernel == 0 { return None; }
        unsafe { core::ptr::copy_nonoverlapping(kernel as *const u8, value.as_mut_ptr().cast::<u8>().add(copied), chunk) };
        copied += chunk;
    }
    Some(unsafe { value.assume_init() })
}

fn write_user<T: Copy>(address: usize, value: &T) -> Option<()> {
    checked_user_range(address, core::mem::size_of::<T>())?;
    let mut copied = 0usize;
    while copied < core::mem::size_of::<T>() {
        let user = address + copied;
        let chunk = (0x1000 - (user & 0xfff)).min(core::mem::size_of::<T>() - copied);
        let kernel = user_to_kernel(user);
        if kernel == 0 { return None; }
        unsafe { core::ptr::copy_nonoverlapping((value as *const T).cast::<u8>().add(copied), kernel as *mut u8, chunk) };
        copied += chunk;
    }
    Some(())
}

fn user_to_str(ptr: usize, max: usize) -> Option<String> {
    let kptr = user_to_kernel(ptr);
    if kptr == 0 { return None; }
    let bytes = unsafe { core::slice::from_raw_parts(kptr as *const u8, max) };
    let nul = bytes.iter().position(|&b| b == 0)?;
    Some(String::from_utf8_lossy(&bytes[..nul]).into_owned())
}

// Kernel-side UART RX buffer (compensates for small hardware FIFO)
pub static RX_BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());

pub fn rx_fill() {
    let mut buf = RX_BUF.lock();
    #[cfg(target_arch = "riscv64")]
    while let Some(b) = kor_arch::uart::ns16550a::getchar_mmio(0x1000_0000) { buf.push(b); }
    #[cfg(target_arch = "aarch64")]
    while let Some(b) = kor_arch::uart::pl011::getchar(0x0900_0000) { buf.push(b); }
    #[cfg(target_arch = "x86_64")]
    while let Some(b) = kor_arch::uart::ns16550a::getchar_port(0x3F8) { buf.push(b); }
}

fn rx_get() -> Option<u8> {

    rx_fill();
    let mut buf = RX_BUF.lock();
    if buf.is_empty() { None }
    else { Some(buf.remove(0)) }
}

fn rx_has_data() -> bool {
    rx_fill();
    !RX_BUF.lock().is_empty()
}

type FutexKey = (usize, usize);
static FUTEX_QUEUES: Mutex<alloc::collections::BTreeMap<FutexKey, alloc::sync::Arc<kor_sched::WaitQueue>>> = Mutex::new(alloc::collections::BTreeMap::new());

fn futex_queue(address: usize) -> alloc::sync::Arc<kor_sched::WaitQueue> {
    let key = (crate::sched::current_pid().unwrap_or(0), address);
    let mut queues = FUTEX_QUEUES.lock();
    queues.entry(key).or_insert_with(|| alloc::sync::Arc::new(kor_sched::WaitQueue::new())).clone()
}

pub(crate) fn wake_futex(address: usize, count: usize) -> usize {
    let queue = futex_queue(address);
    let mut woken = 0;
    while woken < count && queue.wake_one() { woken += 1; }
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
struct SignalAction { handler: usize, flags: usize, restorer: usize, mask: u64 }

const EMPTY_SIGNAL_ACTION: SignalAction = SignalAction { handler: SIG_DFL, flags: 0, restorer: 0, mask: 0 };

/// Linux signal dispositions, blocked mask, and standard-signal pending set.

#[cfg(target_arch = "riscv64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct UserSignalAction { handler: usize, flags: usize, mask: u64 }

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
type UserSignalAction = SignalAction;

fn kernel_signal_action(action: UserSignalAction) -> SignalAction {
    #[cfg(target_arch = "riscv64")]
    { SignalAction { handler: action.handler, flags: action.flags, restorer: 0, mask: action.mask } }
    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    { action }
}

fn user_signal_action(action: SignalAction) -> UserSignalAction {
    #[cfg(target_arch = "riscv64")]
    { UserSignalAction { handler: action.handler, flags: action.flags, mask: action.mask } }
    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    { action }
}
pub struct SignalState {
    actions: [SignalAction; SIGNAL_COUNT + 1],
    blocked: u64,
    pending: u64,
}

impl SignalState {
    /// Construct empty signal state with all dispositions set to default.
    pub const fn new() -> Self {
        Self { actions: [EMPTY_SIGNAL_ACTION; SIGNAL_COUNT + 1], blocked: 0, pending: 0 }
    }

    /// Clone dispositions and mask for fork while clearing process-local pending signals.
    pub fn fork_clone(&self) -> Self {
        Self { actions: self.actions, blocked: self.blocked, pending: 0 }
    }
    pub fn enqueue(&mut self, signal: usize) { self.pending |= 1 << (signal - 1); }

    /// Reset caught dispositions for exec while preserving ignored dispositions and mask.
    pub fn reset_for_exec(&mut self) {
        for action in &mut self.actions[1..] {
            if action.handler != SIG_IGN { *action = EMPTY_SIGNAL_ACTION; }
        }
        self.pending = 0;
    }
}

impl Default for SignalState {
    fn default() -> Self { Self::new() }
}

const SIGINFO_SIZE: usize = 128;
const EFAULT: isize = -14;
const EINVAL: isize = -22;
const ESRCH: isize = -3;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxStack { sp: usize, flags: i32, _pad: u32, size: usize }

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxSigInfo { signo: i32, errno: i32, code: i32, _pad0: i32, sender_pid: i32, sender_uid: u32, _pad: [u8; 104] }

const _: () = assert!(core::mem::size_of::<LinuxSigInfo>() == SIGINFO_SIZE);

#[cfg(target_arch = "riscv64")]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct LinuxMachineContext { regs: [usize; 32], fp_and_extensions: [u8; 528] }

#[cfg(target_arch = "aarch64")]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct LinuxMachineContext { fault_address: usize, regs: [usize; 31], sp: usize, pc: usize, pstate: usize, _reserved_align: [u8; 8], reserved: [u8; 4096] }

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxMachineContext {
    regs: [usize; 18],
    cs: u16, gs: u16, fs: u16, ss: u16,
    err: usize, trapno: usize, oldmask: usize, cr2: usize, fpstate: usize,
    reserved: [usize; 8],
}

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxUContext { flags: usize, link: usize, stack: LinuxStack, sigmask: u64, unused_mask: [u8; 120], mcontext: LinuxMachineContext }

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxUContext { flags: usize, link: usize, stack: LinuxStack, mcontext: LinuxMachineContext, sigmask: u64 }

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct LinuxRtSignalFrame { info: LinuxSigInfo, context: LinuxUContext }

#[cfg(target_arch = "x86_64")]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct LinuxRtSignalFrame { restorer: usize, context: LinuxUContext, info: LinuxSigInfo }

#[cfg(target_arch = "riscv64")]
const USER_ADDRESS_LIMIT: usize = 1 << 38;
#[cfg(target_arch = "aarch64")]
const USER_ADDRESS_LIMIT: usize = 1 << 48;
#[cfg(target_arch = "x86_64")]
const USER_ADDRESS_LIMIT: usize = 1 << 47;
static PROCESS_GROUP: Mutex<usize> = Mutex::new(1);

#[cfg(target_arch = "riscv64")]
fn machine_context(context: &kor_arch::UserTrapContext) -> LinuxMachineContext {
    let mut regs = context.regs;
    regs[0] = context.pc;
    LinuxMachineContext { regs, fp_and_extensions: [0; 528] }
}

#[cfg(target_arch = "riscv64")]
const SIGNAL_RESTORER_ADDRESS: usize = 0x0f_fffe_f000;
#[cfg(target_arch = "aarch64")]
const SIGNAL_RESTORER_ADDRESS: usize = 0x7f_fffe_f000;

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
        if arch.map(SIGNAL_RESTORER_ADDRESS, physical, flags, kor::MapSize::Page4K).is_err() {
            kor_frame::free_frames(physical, 1);
            return None;
        }
        let kernel = arch.phys_to_virt(physical);
        unsafe {
            core::ptr::write_bytes(kernel as *mut u8, 0, 0x1000);
            core::ptr::copy_nonoverlapping(SIGNAL_RESTORER_CODE.as_ptr(), kernel as *mut u32, SIGNAL_RESTORER_CODE.len());
        }
        #[cfg(target_arch = "riscv64")]
        unsafe { core::arch::asm!("fence.i") };
        #[cfg(target_arch = "aarch64")]
        unsafe { core::arch::asm!("ic ivau, {}", "dsb ish", "isb", in(reg) SIGNAL_RESTORER_ADDRESS) };
    }
    Some(SIGNAL_RESTORER_ADDRESS)
}

fn signal_restorer(action: SignalAction) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    { (action.flags & SA_RESTORER != 0 && action.restorer != 0).then_some(action.restorer) }
    #[cfg(target_arch = "aarch64")]
    { if action.flags & SA_RESTORER != 0 { (action.restorer != 0).then_some(action.restorer) } else { ensure_signal_restorer() } }
    #[cfg(target_arch = "riscv64")]
    { let _ = action; ensure_signal_restorer() }
}

#[cfg(target_arch = "riscv64")]
fn trap_context(machine: &LinuxMachineContext) -> Option<kor_arch::UserTrapContext> {
    let mut regs = machine.regs;
    let pc = regs[0];
    regs[0] = 0;
    Some(kor_arch::UserTrapContext { regs, pc, status: 0, sp: regs[2], tls: regs[4], fp_callee: [0; 12], fcsr: 0 })
}

#[cfg(target_arch = "aarch64")]
fn machine_context(context: &kor_arch::UserTrapContext) -> LinuxMachineContext {
    let mut regs = [0usize; 31];
    regs.copy_from_slice(&context.regs[..31]);
    LinuxMachineContext { fault_address: 0, regs, sp: context.sp, pc: context.pc, pstate: context.status, _reserved_align: [0; 8], reserved: [0; 4096] }
}

#[cfg(target_arch = "aarch64")]
fn trap_context(machine: &LinuxMachineContext) -> Option<kor_arch::UserTrapContext> {
    let mut regs = [0usize; 32];
    regs[..31].copy_from_slice(&machine.regs);
    Some(kor_arch::UserTrapContext { regs, pc: machine.pc, status: machine.pstate, sp: machine.sp, tls: 0, fp_callee: [0; 12], fcsr: 0 })
}

#[cfg(target_arch = "x86_64")]
fn machine_context(context: &kor_arch::UserTrapContext) -> LinuxMachineContext {
    let mut regs = [0usize; 18];
    regs[..15].copy_from_slice(&context.regs[..15]);
    regs[15] = context.sp;
    regs[16] = context.pc;
    regs[17] = context.status;
    LinuxMachineContext { regs, cs: 0x1b, gs: 0, fs: 0, ss: 0x23, err: 0, trapno: 0, oldmask: 0, cr2: 0, fpstate: 0, reserved: [0; 8] }
}

#[cfg(target_arch = "x86_64")]
fn trap_context(machine: &LinuxMachineContext) -> Option<kor_arch::UserTrapContext> {
    if machine.cs != 0x1b || machine.ss != 0x23 { return None; }
    let mut regs = [0usize; 32];
    regs[..15].copy_from_slice(&machine.regs[..15]);
    Some(kor_arch::UserTrapContext { regs, pc: machine.regs[16], status: machine.regs[17], sp: machine.regs[15], tls: 0, fp_callee: [0; 12], fcsr: 0 })
}

fn make_ucontext(context: &kor_arch::UserTrapContext, mask: u64) -> LinuxUContext {
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    { LinuxUContext { flags: 0, link: 0, stack: LinuxStack { sp: 0, flags: 0, _pad: 0, size: 0 }, sigmask: mask, unused_mask: [0; 120], mcontext: machine_context(context) } }
    #[cfg(target_arch = "x86_64")]
    { LinuxUContext { flags: 6, link: 0, stack: LinuxStack { sp: 0, flags: 0, _pad: 0, size: 0 }, mcontext: machine_context(context), sigmask: mask } }
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
    { context.regs[10] as isize }
    #[cfg(target_arch = "aarch64")]
    { context.regs[0] as isize }
    #[cfg(target_arch = "x86_64")]
    { context.regs[13] as isize }
}
static SESSION_ID: Mutex<usize> = Mutex::new(1);


pub fn dispatch(num: usize, args: &[usize]) -> isize {
    #[cfg(target_arch = "x86_64")]
    let raw_num = num;
    #[cfg(target_arch = "x86_64")]
    let num = translate_syscall(num);
    #[cfg(target_arch = "x86_64")]
    if raw_num == 158 {
        return sys_arch_prctl(args[0], args[1]);
    }
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
        READ               => SYS_READ,
        WRITE              => SYS_WRITE,
        CLOSE              => SYS_CLOSE,
        FSTAT              => SYS_FSTAT,
        LSEEK              => SYS_LSEEK,
        MMAP               => SYS_MMAP,
        MPROTECT           => SYS_MPROTECT,
        MUNMAP             => SYS_MUNMAP,
        BRK                => SYS_BRK,
        RT_SIGACTION       => SYS_RT_SIGACTION,
        RT_SIGPROCMASK     => SYS_RT_SIGPROCMASK,
        RT_SIGSUSPEND      => SYS_RT_SIGSUSPEND,
        RT_SIGRETURN       => SYS_RT_SIGRETURN,
        IOCTL              => SYS_IOCTL,
        WRITEV             => SYS_WRITEV,
        DUP                => SYS_DUP,
        DUP2               => SYS_DUP2,
        NANOSLEEP          => SYS_NANOSLEEP,
        POLL               => 73,            // poll → ppoll handler
        GETPID             => SYS_GETPID,
        CLONE              => SYS_CLONE,
        FORK               => SYS_CLONE,     // fork → clone handler
        EXECVE             => SYS_EXECVE,
        EXIT               => SYS_EXIT,
        WAIT4              => SYS_WAIT4,
        KILL               => SYS_KILL,
        UNAME              => SYS_UNAME,
        FCNTL              => SYS_FCNTL,
        GETDENTS64         => SYS_GETDENTS64,
        GETCWD             => SYS_GETCWD,
        CHDIR              => SYS_CHDIR,
        GETUID             => SYS_GETUID,
        GETGID             => SYS_GETGID,
        GETEUID            => 175,
        GETEGID            => 177,
        GETPPID            => SYS_GETPPID,
        GETPGRP            => 155,
        SETSID             => 157,
        GETGROUPS          => 158,
        GETPGID            => 155,
        GETSID             => 156,
        SCHED_SETSCHEDULER => 144,
        SCHED_GETSCHEDULER => 145,
        SCHED_GETPARAM     => 146,
        SCHED_SETPARAM     => 143,
        SCHED_SETAFFINITY  => 148,
        SCHED_GETAFFINITY  => 149,
        PRCTL              => 167,
        SYNC               => SYS_SYNC,
        MOUNT              => SYS_MOUNT,
        UMOUNT2            => SYS_UMOUNT2,
        GETTID             => 178,
        FUTEX              => SYS_FUTEX,
        SET_TID_ADDRESS    => SYS_SET_TID_ADDRESS,
        EXIT_GROUP         => SYS_EXIT_GROUP,
        TGKILL             => SYS_TGKILL,
        CLOCK_GETTIME      => 113,
        GETTIMEOFDAY       => 169,
        OPENAT             => SYS_OPENAT,
        MKDIRAT            => SYS_MKDIRAT,
        NEWFSTATAT         => SYS_NEWFSTATAT,
        UNLINKAT           => SYS_UNLINKAT,
        RENAMEAT           => SYS_RENAMEAT,
        LINKAT             => SYS_LINKAT,
        READLINKAT         => SYS_READLINKAT,
        PPOLL              => 73,
        SET_ROBUST_LIST    => SYS_SET_ROBUST_LIST,
        PRLIMIT64          => 261,
        GETRANDOM          => SYS_GETRANDOM,
        RSEQ               => 293,
        201                => 201,          // time (identity)
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
    const IA32_GS_BASE: u32 = 0xC000_0101;

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
        ARCH_SET_FS => unsafe { write_msr(IA32_FS_BASE, addr as u64); 0 },
        ARCH_SET_GS => unsafe { write_msr(IA32_GS_BASE, addr as u64); 0 },
        ARCH_GET_FS | ARCH_GET_GS => {
            let kptr = user_to_kernel(addr);
            if kptr == 0 { return -14; }
            let msr = if code == ARCH_GET_FS { IA32_FS_BASE } else { IA32_GS_BASE };
            unsafe { *(kptr as *mut u64) = read_msr(msr); }
            0
        }
        _ => -22,
    }
}

fn dispatch_inner(num: usize, args: &[usize]) -> isize {
    match num {
        SYS_WRITE => sys_write(args[0], args[1], args[2]),
        SYS_READ => sys_read(args[0], args[1], args[2]),
        SYS_WRITEV => { let r = sys_writev(args[0], args[1], args[2]); r },
        SYS_EXIT | SYS_EXIT_GROUP => sys_exit_fork(args[0]),
        SYS_GETPID => current_pid() as isize,
        SYS_GETPPID => crate::sched::current_process().map(|process| process.lock().parent as isize).unwrap_or(0),
        155 => *PROCESS_GROUP.lock() as isize,
        SYS_GETUID | 175 => 0, // GETUID | GETEUID
        SYS_GETGID | 177 => 0, // GETGID | GETEGID
        SYS_SYNC => { kor_fs::mount::sync_all(&crate::MOUNTS); 0 },
        SYS_NANOSLEEP => sys_nanosleep(args[0]),
        SYS_UNAME => sys_uname(args[0]),
        SYS_BRK => sys_brk(args[0]),
        SYS_MMAP => sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5] as i64),
        SYS_MUNMAP => sys_munmap(args[0], args[1]),
        SYS_MPROTECT => 0,
        SYS_GETRANDOM => sys_getrandom(args[0], args[1]),
        SYS_GETCWD => sys_getcwd(args[0], args[1]),
        SYS_GETDENTS64 => sys_getdents64(args[0], args[1], args[2]),
        SYS_OPENAT => sys_openat(args[0], args[1], args[2], args[3]),
        SYS_CLOSE => sys_close(args[0]),
        SYS_FSTAT => sys_fstat(args[0], args[1]),
        SYS_NEWFSTATAT => sys_fstatat(args[1], args[2]),
        SYS_LSEEK => sys_lseek(args[0], args[1] as i64, args[2]),
        SYS_MKDIRAT => sys_mkdirat(args[0], args[1], args[2]),
        SYS_UNLINKAT => sys_unlinkat(args[0], args[1]),
        SYS_RT_SIGACTION => sys_rt_sigaction(args[0], args[1], args[2]),
        SYS_RT_SIGSUSPEND => sys_rt_sigsuspend(args[0]),
        SYS_RT_SIGPROCMASK => sys_rt_sigprocmask(args[0], args[1], args[2]),
        SYS_RT_SIGRETURN => sys_rt_sigreturn(),
        SYS_SET_TID_ADDRESS => 1,
        SYS_SET_ROBUST_LIST => 0,
        SYS_FUTEX => sys_futex(args[0], args[1], args[2]),
        SYS_IOCTL => sys_ioctl(args[0], args[1], args[2]),
        SYS_FCNTL => 0,
        SYS_DUP => sys_dup(args[0]),
        SYS_DUP2 => sys_dup2(args[0], args[1]),
        SYS_DUP3 => sys_dup2(args[0], args[1]),
        SYS_CLONE => sys_clone(args),
        SYS_EXECVE => sys_execve(args[0], args[1]),
        SYS_WAIT4 => sys_wait4(args[0], args[1], args[2]),
        SYS_KILL => sys_kill(args[0], args[1]),
        SYS_TGKILL => sys_kill(args[1], args[2]),
        SYS_CHDIR => 0,
        SYS_READLINKAT => -2, // ENOENT
        SYS_MOUNT => 0,
        SYS_UMOUNT2 => 0,
        113 => sys_clock_gettime(args[1]),
        157 => { let pid = current_pid(); *SESSION_ID.lock() = pid; *PROCESS_GROUP.lock() = pid; pid as isize },
        169 => sys_gettimeofday(args[0]),
        201 => sys_time(args[0]),
        73 => sys_ppoll(args[0], args[1]), // ppoll

        167 => 0, // prctl
        154 => 0, // sched_getaffinity (return 0 = success, no CPUs)
        158 => 0, // getgroups (return 0 groups)
        156 => *SESSION_ID.lock() as isize,
        144 => 0, // sched_setscheduler
        145 => 0, // sched_getscheduler
        146 => 0, // sched_getparam
        143 => 0, // sched_setparam
        148 => 0, // sched_setaffinity
        149 => 0, // sched_getaffinity
        178 => current_pid() as isize, // gettid (single-threaded processes)
        3 => 0,   // io_cancel → no-op
        0 => -38, // io_setup → ENOSYS (no async I/O support)
        2 => -2,  // open (legacy)
        293 => 0, // rseq → success (no-op, musl falls back gracefully)
        261 => {   // prlimit64 → return default limits
            if args[3] != 0 {
                let kptr = user_to_kernel(args[3]);
                if kptr != 0 {
                    // struct rlimit { rlim_cur, rlim_max } — 16 bytes
                    let rlim = unsafe { core::slice::from_raw_parts_mut(kptr as *mut u64, 2) };
                    rlim[0] = u64::MAX; // rlim_cur = unlimited
                    rlim[1] = u64::MAX; // rlim_max = unlimited
                }
            }
            0
        },
        _ => -38,
        }
}

// --- Process table (minimal) ---
struct FileEntry {
    inode: Arc<dyn INode>,
    offset: u64,
}
type SharedFile = Arc<Mutex<FileEntry>>;
static FD_TABLE: Mutex<alloc::collections::BTreeMap<usize, SharedFile>> = Mutex::new(alloc::collections::BTreeMap::new());

fn alloc_fd(inode: Arc<dyn INode>) -> usize {
    let mut table = FD_TABLE.lock();
    let mut fd = 3;
    while table.contains_key(&fd) { fd += 1; }
    table.insert(fd, Arc::new(Mutex::new(FileEntry { inode, offset: 0 })));
    fd
}


fn current_pid() -> usize {
    crate::sched::current_pid().unwrap_or(1)
}

// --- Syscall implementations ---

fn sys_write(fd: usize, buf: usize, count: usize) -> isize {
    let kbuf = user_to_kernel(buf);
    if kbuf == 0 { return -14; }
    let data = unsafe { core::slice::from_raw_parts(kbuf as *const u8, count) };
    let entry = FD_TABLE.lock().get(&fd).cloned();
    if let Some(entry) = entry {
        let mut entry = entry.lock();
        match entry.inode.write_at(entry.offset as usize, data) {
            Ok(written) => { entry.offset += written as u64; written as isize }
            Err(error) => fs_errno(error),
        }
    } else if fd == 1 || fd == 2 {
        for &byte in data { kor::console::putc(byte); }
        count as isize
    } else {
        -9
    }
}

fn sys_read(fd: usize, buf: usize, count: usize) -> isize {
    if fd == 0 {
        let kbuf = user_to_kernel(buf);
        if kbuf == 0 { return -14; }
        let slice = unsafe { core::slice::from_raw_parts_mut(kbuf as *mut u8, count) };
        let mut read = 0;
        while read == 0 {
            #[cfg(target_arch = "x86_64")]
            {
                if let Some(b) = kor_arch::uart::ns16550a::getchar_port(0x3F8) {
                    slice[read] = b;
                    read += 1;
                }
            }
            #[cfg(not(target_arch = "x86_64"))]
            if let Some(b) = rx_get() { slice[read] = b; read += 1; }
        }
        while read < count {
            #[cfg(target_arch = "x86_64")]
            let next = kor_arch::uart::ns16550a::getchar_port(0x3F8);
            #[cfg(not(target_arch = "x86_64"))]
            let next = rx_get();
            match next { Some(b) => { slice[read] = b; read += 1; } None => break, }
        }
        read as isize
    } else {
        let entry = match FD_TABLE.lock().get(&fd).cloned() {
            Some(entry) => entry,
            None => return -9,
        };
        let mut entry = entry.lock();
        let kbuf = user_to_kernel(buf);
        if kbuf == 0 { return -14; }
        let slice = unsafe { core::slice::from_raw_parts_mut(kbuf as *mut u8, count) };
        match entry.inode.read_at(entry.offset as usize, slice) {
            Ok(n) => { entry.offset += n as u64; n as isize }
            Err(error) => fs_errno(error),
        }
    }
}
fn sys_writev(fd: usize, iov: usize, iovcnt: usize) -> isize {
    if fd == 1 || fd == 2 {
        let kiov = user_to_kernel(iov);
        if kiov == 0 { return -14; }
        let mut total = 0usize;
        for i in 0..iovcnt {
            let base = unsafe { *((kiov + i * 16) as *const usize) };
            let len = unsafe { *((kiov + i * 16 + 8) as *const usize) };
            if len > 0 {
                let kbase = user_to_kernel(base);
                if kbase == 0 { return -14; }
                let data = unsafe { core::slice::from_raw_parts(kbase as *const u8, len) };
                for &b in data { kor::console::putc(b); }
                total += len;
            }
        }
        total as isize
    } else { -9 }
}

fn sys_exit_fork(code: usize) -> isize {
    kor_fs::mount::sync_all(&crate::MOUNTS);
    crate::sched::exit_current(code as i32)
}

fn sys_clone(args: &[usize]) -> isize {
    let parent = match crate::sched::current_process() {
        Some(process) => process,
        None => return -38,
    };
    let (parent_pid, child_signals, child_brk, child_mmap_next, child_executable) = {
        let parent = parent.lock();
        (parent.pid, parent.signals.fork_clone(), parent.brk, parent.mmap_next, parent.executable.clone())
    };
    let child_space = match parent.lock().address_space().and_then(|space| space.fork().ok()) {
        Some(address_space) => address_space,
        None => return -12,
    };
    let mut context = match kor_sched::current().with_user_context(|tf| unsafe {
        kor_arch::capture_user_context(tf)
    }).flatten() {
        Some(context) => context,
        None => return -38,
    };
    #[cfg(target_arch = "riscv64")]
    {
        context.regs[10] = 0;
        context.pc = context.pc.wrapping_add(4);
        // fork preserves the exact parent SP/TLS virtual addresses. Thread-like
        // clone forms are not yet supported by this process-only implementation.
    }
    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    { context.regs[0] = 0; }
    let child = crate::sched::alloc_process(parent_pid, child_space);
    child.lock().signals = child_signals;
    child.lock().brk = child_brk;
    child.lock().executable = child_executable;
    child.lock().mmap_next = child_mmap_next;
    child.lock().set_fork_context(context);
    let pid = child.lock().pid;
    const CLONE_CHILD_SETTID: usize = 0x0100_0000;
    const CLONE_CHILD_CLEARTID: usize = 0x0020_0000;
    if args[4] != 0 {
        if args[0] & CLONE_CHILD_CLEARTID != 0 { child.lock().clear_child_tid = args[4]; }
        if args[0] & CLONE_CHILD_SETTID != 0 {
            if let Some(paddr) = child.lock().address_space().and_then(|space| space.translate(args[4])) {
                unsafe { *(kor::arch::current().phys_to_virt(paddr) as *mut u32) = pid as u32; }
            }
        }
    }
    crate::sched::spawn_process(&child, fork_child_entry);
    pid as isize
}

fn fork_child_entry() {
    let process = crate::sched::current_process().expect("fork child task has no process");
    let context = {
        let mut process = process.lock();
        process.activate_address_space();
        process.take_fork_context().expect("fork child has no user context")
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
            let pointer_address = user_to_kernel(argv + index * core::mem::size_of::<usize>());
            if pointer_address == 0 { return -14; }
            let pointer = unsafe { *(pointer_address as *const usize) };
            if pointer == 0 { break; }
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
    let replacement = match crate::mm::AddressSpace::empty() {
        Ok(address_space) => address_space,
        Err(_) => return -12,
    };
    let program = match crate::user::elf::load_elf(&replacement, &image, &args) {
        Ok(program) => program,
        Err(_) => return -8,
    };

    let updated = kor_sched::current().with_user_context(|tf| unsafe {
        let mut context = match kor_arch::capture_user_context(tf) {
            Some(context) => context,
            None => return false,
        };
        context.regs = [0; 32];
        context.pc = program.entry;
        context.sp = program.stack_top;
        context.tls = 0;
        kor_arch::restore_user_context(tf, &context)
    }).unwrap_or(false);
    if !updated { return -38; }

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
        process.mmap_next = default_mmap_base();
        old
    };
    drop(old);
    0
}



fn sys_brk(addr: usize) -> isize {
    let Some(process) = crate::sched::current_process() else { return -12 };
    let mut process = process.lock();
    if process.brk == 0 {
        process.brk = if cfg!(target_arch = "aarch64") { 0x80_0100_0000 }
            else if cfg!(target_arch = "x86_64") { 0x4_0000_1000 }
            else { 0x1_0001_0000 };
    }
    if addr == 0 { return process.brk as isize; }
    let old = process.brk;
    if addr > old {
        let arch = kor::arch::current();
        let ps = (old + 4095) & !4095;
        let pe = (addr + 4095) & !4095;
        let flags = kor::MappingFlags::U | kor::MappingFlags::R | kor::MappingFlags::W;
        for vaddr in (ps..pe).step_by(4096) {
            let need = match arch.translate(vaddr) { Some(p) => p < RAM_START, None => true };
            if need {
                if let Some(paddr) = kor_frame::alloc_page() {
                    if arch.map(vaddr, paddr, flags, kor::MapSize::Page4K).is_err() {
                        kor_frame::free_frames(paddr, 1);
                        return process.brk as isize;
                    }
                    let kv = arch.phys_to_virt(paddr);
                    unsafe { core::ptr::write_bytes(kv as *mut u8, 0, 4096); }
                } else { return process.brk as isize; }
            }
        }
        process.brk = addr;
    }
    process.brk as isize
}

fn sys_uname(buf: usize) -> isize {
    let machine = if cfg!(target_arch = "riscv64") { "riscv64" }
        else if cfg!(target_arch = "aarch64") { "aarch64" }
        else { "x86_64" };
    let names: [&str; 6] = [
        "korix\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "localhost\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "6.12.0-korix\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "#1 SMP korix\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        machine,
        "(none)\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    ];
    for (i, name) in names.iter().enumerate() {
        let dst = user_to_kernel(buf) + i * 65;
        unsafe {
            core::ptr::copy_nonoverlapping(
                name.as_ptr(),
                dst as *mut u8,
                65,
            );
        }
    }
    0
}

fn sys_getrandom(buf: usize, count: usize) -> isize {
    // Fill with pseudo-random data (deterministic for now)
    let data = unsafe { core::slice::from_raw_parts_mut(user_to_kernel(buf) as *mut u8, count) };
    for (i, b) in data.iter_mut().enumerate() {
        *b = ((i as u64).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 32) as u8;
    }
    count as isize
}

fn sys_getcwd(buf: usize, size: usize) -> isize {
    let cwd = b"/\0";
    if cwd.len() > size {
        return -34; // ERANGE
    }
    unsafe {
        core::ptr::copy_nonoverlapping(cwd.as_ptr(), user_to_kernel(buf) as *mut u8, cwd.len());
    }
    buf as isize
}

fn sys_getdents64(fd: usize, buf: usize, count: usize) -> isize {
    let mut table = FD_TABLE.lock();
    let entry = match table.get_mut(&fd) {
        Some(e) => e,
        None => return -9,
    };
    let mut entry = entry.lock();
    let entries = match entry.inode.list() {
        Ok(e) => e,
        Err(_) => return -5,
    };
    // Use the file offset as the index into the entries list.
    // Each getdents64 call returns entries starting from the offset.
    // When all entries have been returned, return 0 (EOF).
    let start_idx = (entry.offset as usize).min(entries.len());
    let kbuf = user_to_kernel(buf);
    if kbuf == 0 { return -14; }
    let mut offset = 0usize;
    let mut idx = start_idx;
    while idx < entries.len() {
        let (name, ino) = &entries[idx];
        let reclen = ((19 + name.len() + 1 + 7) & !7) as u16;
        if offset + reclen as usize > count { break; }
        let ptr = (kbuf + offset) as *mut u8;
        unsafe {
            *(ptr as *mut u64) = *ino as u64;
            *((ptr as *mut u8).add(8) as *mut i64) = (idx + 1) as i64;
            *((ptr as *mut u8).add(16) as *mut u16) = reclen;
            // Determine file type
            let ft = if name == "." || name == ".." { 4u8 }
                else { entry.inode.lookup(name).ok()
                    .and_then(|i| i.getattr().ok())
                    .map(|m| match m.file_type {
                        kor::FileType::Directory => 4u8,
                        kor::FileType::Symlink => 10u8,
                        _ => 8u8,
                    }).unwrap_or(8u8) };
            *((ptr as *mut u8).add(18)) = ft;
            let np = (ptr as *mut u8).add(19);
            core::ptr::copy_nonoverlapping(name.as_ptr(), np, name.len());
            *np.add(name.len()) = 0;
        }
        offset += reclen as usize;
        idx += 1;
    }
    entry.offset = idx as u64; // advance offset for next call
    if offset == 0 && start_idx >= entries.len() { return 0; } // EOF
    offset as isize
}

fn fs_errno(error: kor::FsError) -> isize {
    match error {
        kor::FsError::NotFound => -2,
        kor::FsError::AlreadyExists => -17,
        kor::FsError::NotDirectory => -20,
        kor::FsError::IsDirectory => -21,
        kor::FsError::InvalidInput => -22,
        kor::FsError::ReadOnly => -30,
        kor::FsError::NoSpace => -28,
        kor::FsError::Unsupported => -38,
        _ => -5,
    }
}

fn sys_openat(_dirfd: usize, path: usize, flags: usize, mode: usize) -> isize {
    const O_CREAT: usize = 0o100;
    const O_TRUNC: usize = 0o1000;
    let path_str = match user_to_str(path, 256) {
        Some(p) => p,
        None => return -14,
    };
    if path_str == "/dev/null" { return 3; }
    if path_str.as_str() == "/dev/tty" || path_str.as_str() == "/dev/console" {
        #[cfg(target_arch = "x86_64")]
        { return 0; }
        #[cfg(not(target_arch = "x86_64"))]
        { return 3; }
    }
    let inode = match crate::fs::resolve(&path_str) {
        Ok(inode) => inode,
        Err(_) if flags & O_CREAT != 0 => {
            let (parent_path, name) = match path_str.rsplit_once('/') {
                Some(("", name)) => ("/", name),
                Some((parent, name)) => (parent, name),
                None => ("/", path_str.as_str()),
            };
            let parent = match crate::fs::resolve(parent_path) { Ok(node) => node, Err(_) => return -2 };
            match parent.create(name, kor::FileType::Regular, (mode as u32) & 0o777) {
                Ok(node) => node,
                Err(error) => return fs_errno(error),
            }
        }
        Err(_) => return -2,
    };
    if flags & O_TRUNC != 0 {
        let size = inode.getattr().map(|meta| meta.size).unwrap_or(0);
        if size != 0 {
            if let Err(error) = inode.truncate(0) {
                if !matches!(error, kor::FsError::Unsupported) { return fs_errno(error); }
            }
        }
    }
    alloc_fd(inode) as isize
}

fn sys_close(fd: usize) -> isize {
    let mut table = FD_TABLE.lock();
    if fd > 2 { table.remove(&fd); }
    0
}

fn sys_dup(fd: usize) -> isize {
    let mut table = FD_TABLE.lock();
    let entry = if fd <= 2 { None } else { match table.get(&fd) { Some(entry) => Some(entry.clone()), None => return -9 } };
    let mut newfd = 3;
    while table.contains_key(&newfd) { newfd += 1; }
    if let Some(entry) = entry { table.insert(newfd, entry); }
    newfd as isize
}

fn sys_dup2(fd: usize, newfd: usize) -> isize {
    if fd == newfd { return newfd as isize; }
    let mut table = FD_TABLE.lock();
    if fd <= 2 { table.remove(&newfd); return newfd as isize; }
    let entry = match table.get(&fd) { Some(entry) => entry.clone(), None => return -9 };
    table.insert(newfd, entry);
    newfd as isize
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

        let kbuf = user_to_kernel(argp);
        if kbuf == 0 { return -14; }
        let ptr = kbuf as *mut u32;
        unsafe {
            *ptr.add(0) = ICRNL;                              // c_iflag
            *ptr.add(1) = OPOST | ONLCR;                      // c_oflag
            *ptr.add(2) = 0o141;                              // c_cflag (B38400 | CS8 | CREAD)
            *ptr.add(3) = ECHO | ICANON | ISIG | ECHOE | ECHOK | IEXTEN; // c_lflag
            // c_line = 0 (at offset 16)
            *((kbuf + 16) as *mut u8) = 0;
            // c_cc[19] — set standard control chars
            let cc = (kbuf + 17) as *mut u8;
            *cc.add(0) = 3;   // VINTR = ^C
            *cc.add(1) = 28;  // VQUIT = ^\
            *cc.add(2) = 127; // VERASE = DEL
            *cc.add(3) = 21;  // VKILL = ^U
            *cc.add(4) = 4;   // VEOF = ^D
            *cc.add(5) = 0;   // VTIME
            *cc.add(6) = 1;   // VMIN
            // rest = 0
            for i in 7..19 { *cc.add(i) = 0; }
        }
        return 0;
    }
    // TCSETS = 0x5402, TCSETSW = 0x5403, TCSETSF = 0x5404: set terminal attrs
    if request >= 0x5402 && request <= 0x5404 {
        return 0; // accept silently
    }
    // TIOCGWINSZ = 0x5413: get window size
    if request == 0x5413 {
        let kbuf = user_to_kernel(argp);
        if kbuf == 0 { return -14; }
        // struct winsize: ws_row, ws_col, ws_xpixel, ws_ypixel (all u16)
        let ptr = kbuf as *mut u16;
        unsafe {
            *ptr.add(0) = 24;  // rows
            *ptr.add(1) = 80;  // cols
            *ptr.add(2) = 0;   // xpixel
            *ptr.add(3) = 0;   // ypixel
        }
        return 0;
    }
    // TIOCGPGRP = 0x540F: get foreground process group
    if request == 0x540F {
        let kbuf = user_to_kernel(argp);
        if kbuf != 0 { unsafe { *(kbuf as *mut u32) = 1; } } // pgid = 1
        return 0;
    }
    // TIOCSPGRP = 0x5410: set foreground process group
    if request == 0x5410 { return 0; }
    // TIOCGSID = 0x5429: get session id
    if request == 0x5429 {
        let kbuf = user_to_kernel(argp);
        if kbuf != 0 { unsafe { *(kbuf as *mut u32) = 1; } }
        return 0;
    }
    0 // default: silently succeed for other ioctls
}

fn sys_fstat(fd: usize, stat: usize) -> isize {
    let table = FD_TABLE.lock();
    let entry = match table.get(&fd) {
        Some(e) => e,
        None => return -9,
    };
    let entry = entry.lock();
    #[repr(C)]
    struct LinuxStat {
        st_dev: u64, st_ino: u64, st_mode: u32, st_nlink: u32,
        st_uid: u32, st_gid: u32, st_rdev: u64, _pad: u64,
        st_size: i64, st_blksize: u32, _pad2: u32, st_blocks: u64,
        st_atime: i64, st_mtime: i64, st_ctime: i64,
    }
    let meta = entry.inode.getattr().unwrap();
    let mode = match meta.file_type {
        kor::FileType::Regular => 0o100664,
        kor::FileType::Directory => 0o040755,
        kor::FileType::Symlink => 0o120777,
        _ => 0o0600,
    };
    let st = LinuxStat {
        st_dev: 0, st_ino: 0, st_mode: mode, st_nlink: 1,
        st_uid: 0, st_gid: 0, st_rdev: 0, _pad: 0,
        st_size: meta.size as i64, st_blksize: 512, _pad2: 0,
        st_blocks: ((meta.size as u64) + 511) / 512,
        st_atime: 0, st_mtime: 0, st_ctime: 0,
    };
    let kstat = user_to_kernel(stat);
    if kstat == 0 { return -14; }
    unsafe { *(kstat as *mut LinuxStat) = st; }
    0
}

fn sys_lseek(fd: usize, offset: i64, whence: usize) -> isize {
    let mut table = FD_TABLE.lock();
    let entry = match table.get_mut(&fd) {
        Some(e) => e,
        None => return -9,
    };
    let mut entry = entry.lock();
    let new = match whence {
        0 => offset,
        1 => entry.offset as i64 + offset,
        _ => -1,
    };
    if new < 0 { return -22; }
    entry.offset = new as u64;
    new as isize
}

fn default_mmap_base() -> usize {
    if cfg!(target_arch = "riscv64") { 0x20_0000_0000 }
    else if cfg!(target_arch = "aarch64") { 0x80_0000_0000 }
    else { 0x8_0000_0000 }
}

fn sys_mmap(_addr: usize, mut len: usize, _prot: usize, flags: usize, _fd: usize, _offset: i64) -> isize {
    if len == 0 { len = 4096; }
    let page_start = if _addr != 0 && (flags & 0x10) != 0 {
        _addr & !4095
    } else {
        let Some(process) = crate::sched::current_process() else { return -12 };
        let mut process = process.lock();
        let start = process.mmap_next;
        process.mmap_next += (len + 4095) & !4095;
        start
    };
    let page_end = (page_start + len + 4095) & !4095;
    let map_flags = kor::MappingFlags::U | kor::MappingFlags::R | kor::MappingFlags::W;
    for vaddr in (page_start..page_end).step_by(4096) {
        let need = match kor::arch::current().translate(vaddr) { Some(p) => p < RAM_START, None => true };
        if need {
            let arch = kor::arch::current();
            if let Some(paddr) = kor_frame::alloc_page() {
                let _ = arch.map(vaddr, paddr, map_flags, kor::MapSize::Page4K);
                let kv = arch.phys_to_virt(paddr);
                unsafe { core::ptr::write_bytes(kv as *mut u8, 0, 4096); }
            } else { return -12; }
        }
    }
    page_start as isize
}

fn sys_munmap(addr: usize, len: usize) -> isize {
    if addr & 4095 != 0 { return -22; }
    let end = match addr.checked_add(len) { Some(end) => (end + 4095) & !4095, None => return -22 };
    for vaddr in (addr..end).step_by(4096) {
        if let Ok(paddr) = kor::arch::current().unmap(vaddr, kor::MapSize::Page4K) {
            kor_frame::free_frames(paddr, 1);
        }
    }
    0
}

fn split_parent(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some(("", name)) => ("/", name),
        Some((parent, name)) => (parent, name),
        None => ("/", path),
    }
}

fn sys_mkdirat(_dirfd: usize, path: usize, mode: usize) -> isize {
    let path = match user_to_str(path, 256) { Some(path) => path, None => return -14 };
    let (parent_path, name) = split_parent(&path);
    let parent = match crate::fs::resolve(parent_path) { Ok(node) => node, Err(error) => return fs_errno(error) };
    match parent.mkdir(name, (mode as u32) & 0o777) { Ok(_) => 0, Err(error) => fs_errno(error) }
}

fn sys_unlinkat(_dirfd: usize, path: usize) -> isize {
    let path = match user_to_str(path, 256) { Some(path) => path, None => return -14 };
    let (parent_path, name) = split_parent(&path);
    let parent = match crate::fs::resolve(parent_path) { Ok(node) => node, Err(error) => return fs_errno(error) };
    match parent.unlink(name) { Ok(()) => 0, Err(error) => fs_errno(error) }
}

fn sys_rt_sigaction(signal: usize, action: usize, old_action: usize) -> isize {
    if signal == 0 || signal > SIGNAL_COUNT || signal == SIGKILL || signal == SIGSTOP { return EINVAL; }
    let Some(process) = crate::sched::current_process() else { return ESRCH };
    let mut process = process.lock();
    if old_action != 0 && write_user(old_action, &user_signal_action(process.signals.actions[signal])).is_none() { return EFAULT; }
    if action != 0 {
        let Some(user_action) = read_user::<UserSignalAction>(action) else { return EFAULT };
        let new_action = kernel_signal_action(user_action);
        if new_action.handler > SIG_IGN && checked_user_range(new_action.handler, 1).is_none() { return EFAULT; }
        if new_action.flags & SA_RESTORER != 0 && checked_user_range(new_action.restorer, 1).is_none() { return EFAULT; }
        process.signals.actions[signal] = SignalAction { mask: new_action.mask & !UNBLOCKABLE_MASK, ..new_action };
    }
    0
}

fn sys_rt_sigprocmask(how: usize, set: usize, old_set: usize) -> isize {
    let Some(process) = crate::sched::current_process() else { return ESRCH };
    let mut process = process.lock();
    if old_set != 0 && write_user(old_set, &process.signals.blocked).is_none() { return EFAULT; }
    if set != 0 {
        let Some(requested) = read_user::<u64>(set) else { return EFAULT };
        let requested = requested & !UNBLOCKABLE_MASK;
        match how { 0 => process.signals.blocked |= requested, 1 => process.signals.blocked &= !requested, 2 => process.signals.blocked = requested, _ => return EINVAL }
    }
    0
}

fn sys_rt_sigsuspend(mask: usize) -> isize {
    let Some(process) = crate::sched::current_process() else { return ESRCH };
    let Some(requested) = read_user::<u64>(mask) else { return EFAULT };
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
    let Some(process) = crate::sched::current_process() else { return ESRCH };
    let Some(result) = kor_sched::current().with_user_context(|frame| unsafe {
        let active = kor_arch::capture_user_context(frame)?;
        let frame_address = signal_frame_address(&active)?;
        let signal_frame = read_user::<LinuxRtSignalFrame>(frame_address)?;
        let mut restored = trap_context(&signal_frame.context.mcontext)?;
        restored.tls = active.tls;
        if restored.pc >= USER_ADDRESS_LIMIT || restored.sp >= USER_ADDRESS_LIMIT { return None; }
        if !kor_arch::restore_user_context(frame, &restored) { return None; }
        process.lock().signals.blocked = signal_frame.context.sigmask & !UNBLOCKABLE_MASK;
        Some(restored_syscall_result(&restored))
    }) else { return EFAULT };
    match result { Some(value) => value, None => crate::sched::exit_current(128 + 11) }
}

fn sys_kill(pid: usize, signal: usize) -> isize {
    if signal > SIGNAL_COUNT { return EINVAL; }
    let target_pid = if pid == 0 { crate::sched::current_pid().unwrap_or(0) } else { pid };
    let Some(target) = crate::sched::process(target_pid) else { return ESRCH };
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
pub extern "Rust" fn korix_deliver_pending_signal(frame: *mut u8) {
    let Some(process) = crate::sched::current_process() else { return };
    let mut process = process.lock();
    loop {
        let deliverable = process.signals.pending & !process.signals.blocked;
        if deliverable == 0 { return; }
        let signal = deliverable.trailing_zeros() as usize + 1;
        let bit = 1u64 << (signal - 1);
        process.signals.pending &= !bit;
        let action = process.signals.actions[signal];
        if action.handler == SIG_IGN { continue; }
        if action.handler == SIG_DFL && signal == 17 { continue; }
        if action.handler == SIG_DFL {
            drop(process);
            crate::sched::exit_current(128 + signal as i32);
        }
        let Some(context) = (unsafe { kor_arch::capture_user_context(frame) }) else { return };
        let frame_size = core::mem::size_of::<LinuxRtSignalFrame>();
        let Some(frame_address) = context.sp.checked_sub(frame_size).map(|sp| sp & !15) else {
            drop(process); crate::sched::exit_current(128 + 11)
        };
        if checked_user_range(frame_address, frame_size).is_none() {
            drop(process); crate::sched::exit_current(128 + 11)
        }
        let info = LinuxSigInfo { signo: signal as i32, errno: 0, code: 0, _pad0: 0, sender_pid: crate::sched::current_pid().unwrap_or(0) as i32, sender_uid: 0, _pad: [0; 104] };
        let user_context = make_ucontext(&context, process.signals.blocked);
        #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
        let signal_frame = LinuxRtSignalFrame { info, context: user_context };
        #[cfg(target_arch = "x86_64")]
        let signal_frame = LinuxRtSignalFrame { restorer: action.restorer, context: user_context, info };
        if write_user(frame_address, &signal_frame).is_none() {
            drop(process); crate::sched::exit_current(128 + 11)
        }
        if action.flags & SA_RESETHAND != 0 { process.signals.actions[signal] = EMPTY_SIGNAL_ACTION; }
        process.signals.blocked |= action.mask;
        if action.flags & SA_NODEFER == 0 { process.signals.blocked |= bit; }
        process.signals.blocked &= !UNBLOCKABLE_MASK;
        let Some(restorer) = signal_restorer(action) else {
            drop(process); crate::sched::exit_current(128 + 11)
        };
        let info_address = frame_address + core::mem::offset_of!(LinuxRtSignalFrame, info);
        let context_address = frame_address + core::mem::offset_of!(LinuxRtSignalFrame, context);
        drop(process);
        if !unsafe { kor_arch::setup_user_signal(frame, action.handler, restorer, frame_address, signal, info_address, context_address) } {
            crate::sched::exit_current(128 + 11)
        }
        return;
    }
}

fn sys_futex(address: usize, operation: usize, value: usize) -> isize {
    const FUTEX_WAIT: usize = 0;
    const FUTEX_WAKE: usize = 1;
    let operation = operation & 0x7f;
    match operation {
        FUTEX_WAIT => {
            let ptr = user_to_kernel(address);
            if ptr == 0 { return -14; }
            if !kor_sched::is_ready() { return -11; }
            let blocked = futex_queue(address).wait_if(|| unsafe {
                (*(ptr as *const core::sync::atomic::AtomicU32)).load(core::sync::atomic::Ordering::SeqCst) == value as u32
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
    if options & !SUPPORTED_OPTIONS != 0 { return -22; }
    let requested = pid as isize;
    if requested == 0 || requested < -1 { return -10; }
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
            if status != 0 {
                let ptr = user_to_kernel(status);
                if ptr == 0 { return -14; }
                unsafe { *(ptr as *mut i32) = (code & 0xff) << 8; }
            }
            let reaped = crate::sched::reap(child_pid).expect("zombie disappeared while reaping");
            drop(reaped);
            return child_pid as isize;
        }
        if crate::sched::matching_child(parent, requested).is_none() { return -10; }
        if options & WNOHANG != 0 { return 0; }
        events.wait();
    }
}

fn realtime_seconds() -> u64 {
    let arch = kor::arch::current();
    let hz = arch.timer_hz().max(1);
    arch.now_ticks() / hz
}

fn sys_clock_gettime(timespec: usize) -> isize {
    let ptr = user_to_kernel(timespec); if ptr == 0 { return -14; }
    let arch = kor::arch::current(); let hz = arch.timer_hz().max(1); let ticks = arch.now_ticks();
    unsafe { *(ptr as *mut i64) = (ticks / hz) as i64; *((ptr + 8) as *mut i64) = ((ticks % hz) * 1_000_000_000 / hz) as i64; }
    0
}

fn sys_gettimeofday(timeval: usize) -> isize {
    if timeval == 0 { return 0; }
    let ptr = user_to_kernel(timeval); if ptr == 0 { return -14; }
    let arch = kor::arch::current(); let hz = arch.timer_hz().max(1); let ticks = arch.now_ticks();
    unsafe { *(ptr as *mut i64) = (ticks / hz) as i64; *((ptr + 8) as *mut i64) = ((ticks % hz) * 1_000_000 / hz) as i64; }
    0
}

fn sys_time(result: usize) -> isize {
    let seconds = realtime_seconds() as isize;
    if result != 0 { let ptr = user_to_kernel(result); if ptr == 0 { return -14; } unsafe { *(ptr as *mut i64) = seconds as i64; } }
    seconds
}

fn sys_nanosleep(request: usize) -> isize {
    let ptr = user_to_kernel(request);
    if ptr == 0 { return -14; }
    let seconds = unsafe { *(ptr as *const i64) };
    let nanos = unsafe { *((ptr + 8) as *const i64) };
    if seconds < 0 || !(0..1_000_000_000).contains(&nanos) { return -22; }
    let millis = seconds as u64 * 1000 + (nanos as u64).div_ceil(1_000_000);
    if millis != 0 {
        if !kor_sched::is_ready() { return -38; }
        kor_sched::sleep_ms(millis);
    }
    0
}

/// ppoll — block until stdin has data, then return POLLIN.
/// struct pollfd { int fd; short events; short revents; } = 8 bytes
fn sys_ppoll(fds: usize, nfds: usize) -> isize {
    if nfds == 0 { return 0; }
    let kfds = user_to_kernel(fds);
    if kfds == 0 { return -14; }
    const POLLIN: u16 = 0x001;
    loop {
        let mut ready = 0;
        for i in 0..nfds {
            let pfd = kfds + i * 8;
            let fd = unsafe { *(pfd as *const i32) };
            let events = unsafe { *((pfd + 4) as *const u16) };
            let revents = (pfd + 6) as *mut u16;
            unsafe { *revents = 0; }
            if fd == 0 && events & POLLIN != 0 && rx_has_data() {
                unsafe { *revents = POLLIN; }
                ready += 1;
            }
        }
        if ready != 0 { return ready; }
        if kor_sched::is_ready() { kor_sched::sleep_ms(10); }
        else { core::hint::spin_loop(); }
    }
}

#[unsafe(no_mangle)]
pub extern "Rust" fn korix_set_current_tf(tf: *mut u8) {
    if !kor_sched::is_ready() { return; }
    let task = kor_sched::current();
    if tf.is_null() {
        task.clear_user_context();
    } else {
        let guard = task.user_context_scope(tf);
        core::mem::forget(guard);
    }
}

fn sys_fstatat(path: usize, stat: usize) -> isize {
    let path_str = match user_to_str(path, 256) {
        Some(p) => p,
        None => return -14,
    };
    let inode = match crate::fs::resolve(&path_str) {
        Ok(i) => i,
        Err(_) => return -2,
    };
    #[repr(C)]
    struct LinuxStat {
        st_dev: u64, st_ino: u64, st_mode: u32, st_nlink: u32,
        st_uid: u32, st_gid: u32, st_rdev: u64, _pad: u64,
        st_size: i64, st_blksize: u32, _pad2: u32, st_blocks: u64,
        st_atime: i64, st_mtime: i64, st_ctime: i64,
    }
    let meta = inode.getattr().unwrap();
    let mode = match meta.file_type {
        kor::FileType::Regular => 0o100664,
        kor::FileType::Directory => 0o040755,
        kor::FileType::Symlink => 0o120777,
        _ => 0o0600,
    };
    let st = LinuxStat {
        st_dev: 0, st_ino: 0, st_mode: mode, st_nlink: 1,
        st_uid: 0, st_gid: 0, st_rdev: 0, _pad: 0,
        st_size: meta.size as i64, st_blksize: 512, _pad2: 0,
        st_blocks: ((meta.size as u64) + 511) / 512,
        st_atime: 0, st_mtime: 0, st_ctime: 0,
    };
    let kstat = user_to_kernel(stat);
    if kstat == 0 { return -14; }
    unsafe { *(kstat as *mut LinuxStat) = st; }
    0
}
