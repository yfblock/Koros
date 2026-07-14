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
use alloc::collections::BTreeMap;
use spin::Mutex;
use kor::{FsError, INode, FileType};

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

fn rx_has_data() -> bool {
    rx_fill();
    !RX_BUF.lock().is_empty()
}
    rx_fill();
    let mut buf = RX_BUF.lock();
    if buf.is_empty() { None }
    else { Some(buf.remove(0)) }
}

fn rx_has_data() -> bool {
    rx_fill();
    !RX_BUF.lock().is_empty()
}

// Fork context: saved registers for parent process
static FORK_REGS: Mutex<Option<[usize; 32]>> = Mutex::new(None);
static FORK_SEPC: Mutex<usize> = Mutex::new(0);
static FORK_SSTATUS: Mutex<usize> = Mutex::new(0);
static FORK_TPIDR: Mutex<usize> = Mutex::new(0); // aarch64 TPIDR_EL0
static CHILD_EXIT_CODE: Mutex<i32> = Mutex::new(-1);
static FORK_ACTIVE: Mutex<bool> = Mutex::new(false);

// Saved state for parent restoration after execve
static SAVED_DATA_CONTENT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static SAVED_BRK: Mutex<usize> = Mutex::new(0);
static SAVED_MMAP_NEXT: Mutex<usize> = Mutex::new(0);
pub static mut CURRENT_TF: *mut u8 = core::ptr::null_mut();

// Exec info: stored at ELF load time, used by sys_execve
pub static ENTRY_POINT: Mutex<usize> = Mutex::new(0);
pub static PHDR_ADDR: Mutex<usize> = Mutex::new(0);
pub static PHNUM: Mutex<usize> = Mutex::new(0);

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
        SYS_GETPID => 1,
        SYS_GETPPID => 0,
        155 => 1, // getpgid → return pgid=1
        SYS_GETUID | 175 => 0, // GETUID | GETEUID
        SYS_GETGID | 177 => 0, // GETGID | GETEGID
        SYS_SYNC => 0,
        SYS_NANOSLEEP => 0,
        SYS_UNAME => sys_uname(args[0]),
        SYS_BRK => sys_brk(args[0]),
        SYS_MMAP => sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5] as i64),
        SYS_MUNMAP => 0,
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
        SYS_RT_SIGACTION => 0,
        SYS_RT_SIGPROCMASK => 0,
        SYS_SET_TID_ADDRESS => 1,
        SYS_SET_ROBUST_LIST => 0,
        SYS_FUTEX => 0,
        SYS_IOCTL => sys_ioctl(args[0], args[1], args[2]),
        SYS_FCNTL => 0,
        SYS_DUP => sys_dup(args[0]),
        SYS_DUP2 => sys_dup2(args[0], args[1]),
        SYS_CLONE => sys_clone(),
        SYS_EXECVE => sys_execve(args[0], args[1]),
        SYS_WAIT4 => {
            // Only return child PID once, then ECHILD
            let fork_was_active = *FORK_ACTIVE.lock();
            if !fork_was_active && *CHILD_EXIT_CODE.lock() != -1 {
                let code = *CHILD_EXIT_CODE.lock();
                *CHILD_EXIT_CODE.lock() = -1; // mark as reaped
                if args[1] != 0 {
                    let kstat = user_to_kernel(args[1]);
                    if kstat != 0 {
                        unsafe { *(kstat as *mut i32) = (code & 0xff) << 8; }
                    }
                }
                1 // return child PID
            } else {
                -10 // ECHILD
            }
        },
        SYS_KILL | SYS_TGKILL => 0,
        SYS_CHDIR => 0,
        SYS_READLINKAT => -2, // ENOENT
        SYS_MOUNT => 0,
        SYS_UMOUNT2 => 0,
        113 => 0, // clock_gettime
        157 => 1, // setsid → return new sid (1)
        96 => 0,  // set_tid_address (duplicate)
        99 => 0,  // set_robust_list (duplicate)
        98 => 0,  // futex (duplicate)
        169 => 0, // gettimeofday
        201 => 0, // time
        73 => sys_ppoll(args[0], args[1]), // ppoll

        167 => 0, // prctl
        154 => 0, // sched_getaffinity (return 0 = success, no CPUs)
        158 => 0, // getgroups (return 0 groups)
        156 => 1, // getsid → return sid 1
        144 => 0, // sched_setscheduler
        145 => 0, // sched_getscheduler
        146 => 0, // sched_getparam
        143 => 0, // sched_setparam
        148 => 0, // sched_setaffinity
        149 => 0, // sched_getaffinity
        155 => 1, // getpgid
        175 => 0, // geteuid
        177 => 0, // getegid
        178 => 1, // gettid
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
        _ => -38, // ENOSYS
        }
}

// --- Process table (minimal) ---
struct FileEntry {
    inode: Arc<dyn INode>,
    offset: u64,
}
static FD_TABLE: Mutex<alloc::collections::BTreeMap<usize, FileEntry>> = Mutex::new(alloc::collections::BTreeMap::new());

fn alloc_fd(inode: Arc<dyn INode>) -> usize {
    let mut table = FD_TABLE.lock();
    let mut fd = 3;
    while table.contains_key(&fd) { fd += 1; }
    table.insert(fd, FileEntry { inode, offset: 0 });
    fd
}

static NEXT_PID: Mutex<usize> = Mutex::new(1);

fn alloc_pid() -> usize {
    let mut p = NEXT_PID.lock();
    *p += 1;
    *p
}

fn current_pid() -> usize {
    1
}

// --- Syscall implementations ---

fn sys_write(fd: usize, buf: usize, count: usize) -> isize {
    if fd == 1 || fd == 2 {
        let kbuf = user_to_kernel(buf);
        if kbuf == 0 { return -14; }
        let data = unsafe { core::slice::from_raw_parts(kbuf as *const u8, count) };
        for &b in data { kor::console::putc(b); }
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
        let mut table = FD_TABLE.lock();
        let entry = match table.get_mut(&fd) {
            Some(e) => e,
            None => return -9,
        };
        let kbuf = user_to_kernel(buf);
        if kbuf == 0 { return -14; }
        let slice = unsafe { core::slice::from_raw_parts_mut(kbuf as *mut u8, count) };
        match entry.inode.read_at(entry.offset as usize, slice) {
            Ok(n) => { entry.offset += n as u64; n as isize }
            Err(_) => -5,
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
    let fork_active = *FORK_ACTIVE.lock();
    if fork_active {
        *CHILD_EXIT_CODE.lock() = code as i32;
        let regs = FORK_REGS.lock().take();
        let saved_pc = *FORK_SEPC.lock();
        let saved_status = *FORK_SSTATUS.lock();
        *FORK_ACTIVE.lock() = false;
        // Restore data segment content via memcpy
        let saved = SAVED_DATA_CONTENT.lock();
        if !saved.is_empty() {
            let arch = kor::arch::current();
            let ps = *crate::user::elf::DATA_SEG_PAGE_START.lock();
            let pe = *crate::user::elf::DATA_SEG_PAGE_END.lock();
            let mut offset = 0usize;
            for vaddr in (ps..pe).step_by(4096) {
                if offset + 4096 <= saved.len() {
                    if let Some(paddr) = arch.translate(vaddr) {
                        let kv = arch.phys_to_virt(paddr);
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                saved.as_ptr().add(offset), kv as *mut u8, 4096,
                            );
                        }
                    }
                    offset += 4096;
                }
            }
            #[cfg(target_arch = "riscv64")]
            unsafe { core::arch::asm!("sfence.vma"); }
            #[cfg(target_arch = "aarch64")]
            unsafe { core::arch::asm!("tlbi vmalle1is", "dsb sy", "isb"); }
        }
        drop(saved);
        SAVED_DATA_CONTENT.lock().clear();
        // Restore brk and mmap_next for parent
        *BRK_CURRENT.lock() = *SAVED_BRK.lock();
        *MMAP_NEXT.lock() = *SAVED_MMAP_NEXT.lock();
        if let Some(regs) = regs {
            unsafe {
                if !CURRENT_TF.is_null() {
                    #[cfg(target_arch = "riscv64")]
                    let nregs = 32;
                    #[cfg(target_arch = "aarch64")]
                    let nregs = 31;
                    #[cfg(target_arch = "x86_64")]
                    let nregs = 15;
                    let tf = CURRENT_TF as *mut usize;
                    for i in 0..nregs {
                        *tf.add(i) = regs[i];
                    }
                    // On aarch64, restore SP_EL0 (at offset 31*8) from saved[31]
                    #[cfg(target_arch = "aarch64")]
                    {
                        let tf_sp = (CURRENT_TF as *mut u8).add(31 * 8) as *mut usize;
                        *tf_sp = regs[31];
                        // Restore TPIDR_EL0 (at offset 34*8)
                        let tf_tpidr = (CURRENT_TF as *mut u8).add(34 * 8) as *mut usize;
                        *tf_tpidr = *FORK_TPIDR.lock();
                    }
                    // On x86_64, restore rsp (offset 20*8) from saved[15]
                    #[cfg(target_arch = "x86_64")]
                    {
                        let tf_rsp = (CURRENT_TF as *mut u8).add(20 * 8) as *mut usize;
                        *tf_rsp = regs[15];
                    }
                    // PC at offset 32*8 (sepc on riscv64, elr on aarch64)
                    #[cfg(not(target_arch = "x86_64"))]
                    {
                        let tf_pc = (CURRENT_TF as *mut u8).add(32 * 8) as *mut usize;
                        *tf_pc = saved_pc;
                        // Status at offset 33*8 (sstatus on riscv64, spsr on aarch64)
                        let tf_status = (CURRENT_TF as *mut u8).add(33 * 8) as *mut usize;
                        *tf_status = saved_status;
                    }
                    // On x86_64: rip at offset 17*8, rflags at offset 19*8
                    #[cfg(target_arch = "x86_64")]
                    {
                        let tf_rip = (CURRENT_TF as *mut u8).add(17 * 8) as *mut usize;
                        *tf_rip = saved_pc;
                        let tf_rflags = (CURRENT_TF as *mut u8).add(19 * 8) as *mut usize;
                        *tf_rflags = saved_status;
                    }
                }
            }
            return 1;
        }
    }
    kor::println!("[korix] init exited (code={})", code);
    loop { kor::arch::current().wait_for_interrupt(); }
}

fn sys_clone() -> isize {
    let fork_active = *FORK_ACTIVE.lock();
    if fork_active { return -38; }
    unsafe {
        if CURRENT_TF.is_null() { return -38; }
        // Save parent registers
        #[cfg(target_arch = "riscv64")]
        let nregs = 32;
        #[cfg(target_arch = "aarch64")]
        let nregs = 31;
        #[cfg(target_arch = "x86_64")]
        let nregs = 15;
        let tf = CURRENT_TF as *mut usize;
        let mut saved = [0usize; 32];
        for i in 0..nregs { saved[i] = *tf.add(i); }
        // On aarch64, also save SP_EL0 (at offset 31*8) in saved[31]
        #[cfg(target_arch = "aarch64")]
        {
            saved[31] = *((CURRENT_TF as *const u8).add(31 * 8) as *const usize);
        }
        // On x86_64, also save rsp (offset 20*8) in saved[15]
        #[cfg(target_arch = "x86_64")]
        {
            saved[15] = *((CURRENT_TF as *const u8).add(20 * 8) as *const usize);
        }
        *FORK_REGS.lock() = Some(saved);
        // Save PC and status (offsets differ per arch)
        #[cfg(not(target_arch = "x86_64"))]
        let real_pc = *((CURRENT_TF as *const u8).add(32 * 8) as *const usize);
        #[cfg(not(target_arch = "x86_64"))]
        let real_status = *((CURRENT_TF as *const u8).add(33 * 8) as *const usize);
        #[cfg(target_arch = "x86_64")]
        let real_pc = *((CURRENT_TF as *const u8).add(17 * 8) as *const usize); // rip
        #[cfg(target_arch = "x86_64")]
        let real_status = *((CURRENT_TF as *const u8).add(19 * 8) as *const usize); // rflags
        *FORK_SEPC.lock() = real_pc;
        *FORK_SSTATUS.lock() = real_status;
        // On aarch64, also save TPIDR_EL0 (at offset 34*8)
        #[cfg(target_arch = "aarch64")]
        { *FORK_TPIDR.lock() = *((CURRENT_TF as *const u8).add(34 * 8) as *const usize); }
        // Get parent SP
        #[cfg(target_arch = "riscv64")]
        let parent_sp = *tf.add(2); // x2 = sp
        #[cfg(target_arch = "aarch64")]
        let parent_sp = *((CURRENT_TF as *const u8).add(31 * 8) as *const usize); // SP_EL0
        #[cfg(target_arch = "x86_64")]
        let parent_sp = *((CURRENT_TF as *const u8).add(20 * 8) as *const usize); // rsp

        // Allocate child stack
        let arch = kor::arch::current();
        const CHILD_STACK_SIZE: usize = 64 * 1024;
        #[cfg(target_arch = "riscv64")]
        let (user_stack_top, child_stack_top) = (0x10_0000_0000usize, 0x20_0100_0000usize);
        #[cfg(target_arch = "aarch64")]
        let (user_stack_top, child_stack_top) = (0x80_0000_0000usize, 0x80_0100_0000usize);
        #[cfg(target_arch = "x86_64")]
        let (user_stack_top, child_stack_top) = (0x8_0000_0000usize, 0xC_0000_0000usize);
        let child_stack_base = child_stack_top - CHILD_STACK_SIZE;
        let map_flags = kor::MappingFlags::U | kor::MappingFlags::R | kor::MappingFlags::W;
        for vaddr in (child_stack_base..child_stack_top).step_by(4096) {
            if arch.translate(vaddr).is_none() {
                if let Some(paddr) = kor_frame::alloc_page() {
                    let _ = arch.map(vaddr, paddr, map_flags, kor::MapSize::Page4K);
                    let kv = arch.phys_to_virt(paddr);
                    core::ptr::write_bytes(kv as *mut u8, 0, 4096);
                }
            }
        }
        // Copy parent's stack to child's stack
        let child_sp = child_stack_top - (user_stack_top - parent_sp);
        let copy_size = (user_stack_top - parent_sp).min(4096 * 8);
        let mut copied = 0usize;
        while copied < copy_size {
            let src_va = parent_sp + copied;
            let dst_va = child_sp + copied;
            let src_pa = match arch.translate(src_va) { Some(p) => p, None => break };
            let dst_pa = match arch.translate(dst_va) { Some(p) => p, None => break };
            let src_kva = arch.phys_to_virt(src_pa);
            let dst_kva = arch.phys_to_virt(dst_pa);
            let src_page_off = src_va & 4095;
            let dst_page_off = dst_va & 4095;
            let src_chunk = (4096 - src_page_off).min(copy_size - copied);
            let dst_chunk = (4096 - dst_page_off).min(copy_size - copied);
            let chunk = src_chunk.min(dst_chunk);
            unsafe {
                core::ptr::copy_nonoverlapping(
                    src_kva as *const u8, dst_kva as *mut u8, chunk,
                );
            }
            copied += chunk;
        }
        // Set child's SP
        #[cfg(target_arch = "riscv64")]
        { *tf.add(2) = child_sp; }
        #[cfg(target_arch = "aarch64")]
        {
            let tf_sp = (CURRENT_TF as *mut u8).add(31 * 8) as *mut usize;
            *tf_sp = child_sp;
        }
        #[cfg(target_arch = "x86_64")]
        {
            let tf_rsp = (CURRENT_TF as *mut u8).add(20 * 8) as *mut usize;
            *tf_rsp = child_sp;
        }
    }
    *FORK_ACTIVE.lock() = true;
    0
}

fn sys_execve(filename: usize, argv: usize) -> isize {
    let entry = *ENTRY_POINT.lock();
    if entry == 0 { return -2; }

    // Read argv from user space
    let mut args_vec: Vec<String> = Vec::new();
    if argv != 0 {
        for i in 0..16 {
            let ptr_addr = user_to_kernel(argv + i * 8);
            if ptr_addr == 0 { break; }
            let ptr = unsafe { *(ptr_addr as *const usize) };
            if ptr == 0 { break; }
            if let Some(s) = user_to_str(ptr, 256) {
                args_vec.push(s);
            } else { break; }
        }
    }
    if args_vec.is_empty() {
        if let Some(s) = user_to_str(filename, 256) {
            args_vec.push(s);
        } else {
            args_vec.push(String::from("/bin/sh"));
        }
    }

    let arch = kor::arch::current();

    // Save and reload the data segment so the child gets fresh musl state
    let ds_ps = *crate::user::elf::DATA_SEG_PAGE_START.lock();
    let ds_pe = *crate::user::elf::DATA_SEG_PAGE_END.lock();
    if ds_ps != 0 && ds_pe != 0 {
        // Save current data segment content
        let mut saved_buf: Vec<u8> = Vec::new();
        for vaddr in (ds_ps..ds_pe).step_by(4096) {
            if let Some(paddr) = arch.translate(vaddr) {
                let kv = arch.phys_to_virt(paddr);
                unsafe { saved_buf.extend_from_slice(core::slice::from_raw_parts(kv as *const u8, 4096)); }
            }
        }
        *SAVED_DATA_CONTENT.lock() = saved_buf;
        // Save and reset brk/mmap so child gets fresh heap
        *SAVED_BRK.lock() = *BRK_CURRENT.lock();
        *SAVED_MMAP_NEXT.lock() = *MMAP_NEXT.lock();
        *BRK_CURRENT.lock() = 0;
        #[cfg(target_arch = "riscv64")]
        { *BRK_CURRENT.lock() = 0x2_0001_0000; *MMAP_NEXT.lock() = 0x30_0000_0000; }
        #[cfg(target_arch = "aarch64")]
        { *BRK_CURRENT.lock() = 0x80_0400_0000; *MMAP_NEXT.lock() = 0x80_0300_0000; }
        #[cfg(target_arch = "x86_64")]
        { *BRK_CURRENT.lock() = 0x10_0000_1000; *MMAP_NEXT.lock() = 0x14_0000_0000; }
        // Reload data segment in place (overwrite existing pages, no remap)
        let elf = crate::user::elf::ELF_BINARY.lock();
        if let Some(elf_data) = elf.as_ref() {
            let ds_vaddr = *crate::user::elf::DATA_SEG_VADDR.lock();
            let ds_file_off = *crate::user::elf::DATA_SEG_FILE_OFFSET.lock();
            let ds_filesz = *crate::user::elf::DATA_SEG_FILESZ.lock();
            let ds_memsz = *crate::user::elf::DATA_SEG_MEMSZ.lock();
            for vaddr in (ds_ps..ds_pe).step_by(4096) {
                if let Some(paddr) = arch.translate(vaddr) {
                    let kv = arch.phys_to_virt(paddr);
                    // Zero the page first
                    unsafe { core::ptr::write_bytes(kv as *mut u8, 0, 4096); }
                    // Copy file data at the correct offset within the page
                    let page_seg_off = ds_vaddr.saturating_sub(vaddr);
                    let file_seg_off = vaddr.saturating_sub(ds_vaddr);
                    if file_seg_off < ds_filesz {
                        let copy_start = ds_file_off + file_seg_off;
                        let copy_len = core::cmp::min(4096 - page_seg_off, ds_filesz - file_seg_off);
                        if copy_start + copy_len <= elf_data.len() {
                            unsafe {
                                core::ptr::copy_nonoverlapping(
                                    elf_data[copy_start..copy_start + copy_len].as_ptr(),
                                    (kv + page_seg_off) as *mut u8, copy_len,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Allocate a new stack for the execved program
    #[cfg(target_arch = "riscv64")]
    let exec_stack_top = 0x30_0000_0000usize;
    #[cfg(target_arch = "aarch64")]
    let exec_stack_top = 0x80_0200_0000usize;
    #[cfg(target_arch = "x86_64")]
    let exec_stack_top = 0x18_0000_0000usize;
    let exec_stack_size = 64 * 1024;
    let exec_stack_base = exec_stack_top - exec_stack_size;
    let map_flags = kor::MappingFlags::U | kor::MappingFlags::R | kor::MappingFlags::W;
    for vaddr in (exec_stack_base..exec_stack_top).step_by(4096) {
        if arch.translate(vaddr).is_none() {
            if let Some(paddr) = kor_frame::alloc_page() {
                let _ = arch.map(vaddr, paddr, map_flags, kor::MapSize::Page4K);
                let kv = arch.phys_to_virt(paddr);
                unsafe { core::ptr::write_bytes(kv as *mut u8, 0, 4096); }
            }
        }
    }

    let phdr_addr = *PHDR_ADDR.lock();
    let phnum = *PHNUM.lock();

    // Build the stack: strings first, then argc/argv/envp/auxv array
    let mut sp = exec_stack_top;
    sp -= 16; sp &= !0xF;
    let random_off = sp;
    let rand_bytes: [u8; 16] = [0x42; 16];
    let pa = arch.translate(random_off).expect("exec: stack");
    unsafe { core::ptr::copy_nonoverlapping(&rand_bytes[0], arch.phys_to_virt(pa) as *mut u8, 16); }

    let env0 = b"PATH=/bin\0";
    sp -= env0.len(); sp &= !0xF;
    let env0_off = sp;
    let pa = arch.translate(env0_off).expect("exec: stack");
    unsafe { core::ptr::copy_nonoverlapping(env0.as_ptr(), arch.phys_to_virt(pa) as *mut u8, env0.len()); }

    let mut arg_offsets: Vec<usize> = Vec::new();
    for arg in args_vec.iter().rev() {
        let mut s = arg.as_bytes().to_vec();
        s.push(0);
        sp -= s.len(); sp &= !0xF;
        arg_offsets.push(sp);
        let pa = arch.translate(sp).expect("exec: stack");
        unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), arch.phys_to_virt(pa) as *mut u8, s.len()); }
    }
    arg_offsets.reverse();

    let argc = arg_offsets.len();
    let array_size = (1 + argc + 1 + 2 + 14) * 8;
    sp -= array_size; sp &= !0xF;
    let array_base = sp;

    let write_u = |off: usize, val: usize| {
        let pa = arch.translate(off).expect("exec: stack");
        unsafe { *(arch.phys_to_virt(pa) as *mut usize) = val; }
    };
    let mut i = 0;
    write_u(array_base + i * 8, argc); i += 1;
    for &off in &arg_offsets { write_u(array_base + i * 8, off); i += 1; }
    write_u(array_base + i * 8, 0); i += 1;
    write_u(array_base + i * 8, env0_off); i += 1;
    write_u(array_base + i * 8, 0); i += 1;
    const AT_NULL: usize = 0; const AT_PHDR: usize = 3; const AT_PHNUM: usize = 5;
    const AT_PAGESZ: usize = 6; const AT_BASE: usize = 7; const AT_ENTRY: usize = 9;
    const AT_RANDOM: usize = 25;
    write_u(array_base + i * 8, AT_PHDR);   i += 1; write_u(array_base + i * 8, phdr_addr);  i += 1;
    write_u(array_base + i * 8, AT_PHNUM);  i += 1; write_u(array_base + i * 8, phnum);      i += 1;
    write_u(array_base + i * 8, AT_PAGESZ); i += 1; write_u(array_base + i * 8, 4096);       i += 1;
    write_u(array_base + i * 8, AT_BASE);   i += 1; write_u(array_base + i * 8, 0);          i += 1;
    write_u(array_base + i * 8, AT_ENTRY);  i += 1; write_u(array_base + i * 8, entry);      i += 1;
    write_u(array_base + i * 8, AT_RANDOM); i += 1; write_u(array_base + i * 8, random_off); i += 1;
    write_u(array_base + i * 8, AT_NULL);   i += 1; write_u(array_base + i * 8, 0);          i += 1;

    // Modify TrapFrame: zero regs, set SP and PC
    unsafe {
        if !CURRENT_TF.is_null() {
            #[cfg(target_arch = "riscv64")]
            {
                let tf = CURRENT_TF as *mut usize;
                for j in 0..32 { *tf.add(j) = 0; }
                *tf.add(2) = array_base;
                let tf_sstatus = (CURRENT_TF as *mut u8).add(32 * 8) as *mut usize;
                *tf_sstatus = (1 << 5) | (1 << 13); // SPIE + FS=initial
                let tf_sepc = (CURRENT_TF as *mut u8).add(33 * 8) as *mut usize;
                *tf_sepc = entry;
            }
            #[cfg(target_arch = "aarch64")]
            {
                let tf = CURRENT_TF as *mut usize;
                for j in 0..31 { *tf.add(j) = 0; }
                let tf_sp = (CURRENT_TF as *mut u8).add(31 * 8) as *mut usize;
                *tf_sp = array_base;
                let tf_elr = (CURRENT_TF as *mut u8).add(32 * 8) as *mut usize;
                *tf_elr = entry;
                let tf_spsr = (CURRENT_TF as *mut u8).add(33 * 8) as *mut usize;
                *tf_spsr = 0; // EL0t
                let tf_tpidr = (CURRENT_TF as *mut u8).add(34 * 8) as *mut usize;
                *tf_tpidr = 0; // Clear TPIDR_EL0 so child reinitializes TLS
            }
            #[cfg(target_arch = "x86_64")]
            {
                let tf = CURRENT_TF as *mut usize;
                for j in 0..15 { *tf.add(j) = 0; } // zero 15 GPRs
                let tf_rsp = (CURRENT_TF as *mut u8).add(20 * 8) as *mut usize;
                *tf_rsp = array_base;
                let tf_rip = (CURRENT_TF as *mut u8).add(17 * 8) as *mut usize;
                *tf_rip = entry;
                let tf_rflags = (CURRENT_TF as *mut u8).add(19 * 8) as *mut usize;
                *tf_rflags = 0x202; // IF flag set
                let tf_cs = (CURRENT_TF as *mut u8).add(18 * 8) as *mut usize;
                *tf_cs = 0x1B; // user code segment
                let tf_ss = (CURRENT_TF as *mut u8).add(21 * 8) as *mut usize;
                *tf_ss = 0x23; // user data segment
            }
        }
    }

    #[cfg(target_arch = "riscv64")]
    unsafe { core::arch::asm!("sfence.vma"); }
    #[cfg(target_arch = "aarch64")]
    unsafe { core::arch::asm!("tlbi vmalle1is", "dsb sy", "isb"); }

    0
}

fn sys_getpid() -> isize {
    current_pid() as isize
}

static BRK_CURRENT: Mutex<usize> = Mutex::new(0);
#[cfg(target_arch = "riscv64")]
static MMAP_NEXT: Mutex<usize> = Mutex::new(0x20_0000_0000);
#[cfg(target_arch = "aarch64")]
static MMAP_NEXT: Mutex<usize> = Mutex::new(0x80_0000_0000);
#[cfg(target_arch = "x86_64")]
static MMAP_NEXT: Mutex<usize> = Mutex::new(0x8_0000_0000);

fn sys_brk(addr: usize) -> isize {
    let mut brk = BRK_CURRENT.lock();
    if *brk == 0 {
        *brk = if cfg!(target_arch = "aarch64") { 0x80_0100_0000 }
            else if cfg!(target_arch = "x86_64") { 0x4_0000_1000 }
            else { 0x1_0001_0000 };
    }
    if addr == 0 { return *brk as isize; }
    let old = *brk;
    if addr > old {
        let arch = kor::arch::current();
        let ps = (old + 4095) & !4095;
        let pe = (addr + 4095) & !4095;
        let flags = kor::MappingFlags::U | kor::MappingFlags::R | kor::MappingFlags::W;
        for vaddr in (ps..pe).step_by(4096) {
            let need = match arch.translate(vaddr) { Some(p) => p < RAM_START, None => true };
            if need {
                if let Some(paddr) = kor_frame::alloc_page() {
                    let _ = arch.map(vaddr, paddr, flags, kor::MapSize::Page4K);
                    let kv = arch.phys_to_virt(paddr);
                    unsafe { core::ptr::write_bytes(kv as *mut u8, 0, 4096); }
                } else {
                    // Out of memory — don't advance brk past unmapped pages
                    return *brk as isize;
                }
            }
        }
        *brk = addr;
    }
    *brk as isize
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

fn sys_openat(_dirfd: usize, path: usize, _flags: usize, _mode: usize) -> isize {
    let path_str = match user_to_str(path, 256) {
        Some(p) => p,
        None => return -14,
    };
    // Special files
    if path_str == "/dev/null" { return 3; }
    if path_str.as_str() == "/dev/tty" || path_str.as_str() == "/dev/console" {
        #[cfg(target_arch = "x86_64")]
        { return 0; }
        #[cfg(not(target_arch = "x86_64"))]
        { return 3; }
    }
    match crate::fs::resolve(&path_str) {
        Ok(inode) => {
            let fd = alloc_fd(inode);
            fd as isize
        }
        Err(_) => -2,
    }
}

fn sys_close(fd: usize) -> isize {
    let mut table = FD_TABLE.lock();
    if fd > 2 { table.remove(&fd); }
    0
}

fn sys_dup(fd: usize) -> isize {
    // stdin/stdout/stderr are always available
    if fd <= 2 { return 3; }
    -9 // EBADF
}

fn sys_dup2(fd: usize, newfd: usize) -> isize {
    if fd <= 2 { return newfd as isize; }
    -9
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
    let new = match whence {
        0 => offset,
        1 => entry.offset as i64 + offset,
        _ => -1,
    };
    if new < 0 { return -22; }
    entry.offset = new as u64;
    new as isize
}

fn sys_mmap(_addr: usize, mut len: usize, _prot: usize, flags: usize, _fd: usize, _offset: i64) -> isize {
    if len == 0 { len = 4096; }
    let page_start = if _addr != 0 && (flags & 0x10) != 0 {
        _addr & !4095
    } else {
        let mut next = MMAP_NEXT.lock();
        let start = *next;
        *next += (len + 4095) & !4095;
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

fn sys_mkdirat(_dirfd: usize, _path: usize, _mode: usize) -> isize {
    -38
}

fn sys_unlinkat(_dirfd: usize, _path: usize) -> isize {
    -38
}

/// ppoll — block until stdin has data, then return POLLIN.
/// struct pollfd { int fd; short events; short revents; } = 8 bytes
fn sys_ppoll(fds: usize, nfds: usize) -> isize {;
    if nfds == 0 { return 0; }
    let kfds = user_to_kernel(fds);
    if kfds == 0 { return -14; }
    // Check each pollfd for stdin (fd=0) with POLLIN
    const POLLIN: u16 = 0x001;
    let mut ready = 0;
    for i in 0..nfds {
        let pfd = kfds + i * 8;
        let fd = unsafe { *(pfd as *const i32) };
        let events = unsafe { *((pfd + 4) as *const u16) };
        let revents_ptr = (pfd + 6) as *mut u16;
        unsafe { *revents_ptr = 0; }
        if fd == 0 && (events & POLLIN != 0) {
            // Block until data is available on stdin
            while !rx_has_data() {
                core::hint::spin_loop();
            }
            unsafe { *revents_ptr = POLLIN; }
            ready += 1;
        }
    }
    ready as isize
}

#[unsafe(no_mangle)]
pub extern "Rust" fn korix_set_current_tf(tf: *mut u8) {
    unsafe { crate::syscall::CURRENT_TF = tf; }
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
