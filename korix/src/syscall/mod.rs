//! Linux syscall compatibility layer.
//!
//! Implements a subset of the Linux syscall ABI so that statically-linked
//! Linux user programs can run under korix.
//!
//! Syscall numbers follow the generic Linux syscall table (used by
//! riscv64, aarch64). x86_64 uses a different table — handled by
//! `arch::x86_64::syscall_translate`.
//!
//! # Calling convention
//! - riscv64: ecall, a7=nr, a0-a5=args, a0=return
//! - aarch64: svc #0, x8=nr, x0-x5=args, x0=return
//! - x86_64:  syscall, rax=nr, rdi,rsi,rdx,r10,r8,r9=args, rax=return

use alloc::string::String;
use alloc::vec::Vec;
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

/// Dispatch a syscall.
/// `num` is the syscall number, `args` is up to 6 arguments.
/// Returns the syscall result (positive = success, negative = -errno).
pub fn dispatch(num: usize, args: &[usize]) -> isize {
    match num {
        SYS_WRITE => sys_write(args[0], args[1], args[2]),
        SYS_READ => sys_read(args[0], args[1], args[2]),
        SYS_WRITEV => sys_writev(args[0], args[1], args[2]),
        SYS_EXIT => sys_exit(args[0]),
        SYS_EXIT_GROUP => sys_exit(args[0]),
        SYS_GETPID => sys_getpid(),
        SYS_GETUID => 0,
        SYS_GETGID => 0,
        SYS_SYNC => { 0 } // TODO: sync
        SYS_NANOSLEEP => { 0 } // TODO: nanosleep
        SYS_UNAME => sys_uname(args[0]),
        SYS_BRK => sys_brk(args[0]),
        SYS_GETRANDOM => sys_getrandom(args[0], args[1]),
        SYS_GETCWD => sys_getcwd(args[0], args[1]),
        SYS_GETDENTS64 => sys_getdents64(args[0], args[1], args[2]),
        SYS_OPENAT => sys_openat(args[0], args[1], args[2], args[3]),
        SYS_CLOSE => sys_close(args[0]),
        SYS_FSTAT => sys_fstat(args[0], args[1]),
        SYS_LSEEK => sys_lseek(args[0], args[1] as i64, args[2]),
        SYS_MMAP => sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5] as i64),
        SYS_MUNMAP => 0, // TODO
        SYS_MKDIRAT => sys_mkdirat(args[0], args[1], args[2]),
        SYS_UNLINKAT => sys_unlinkat(args[0], args[1]),
        SYS_RT_SIGACTION => 0, // No-op
        SYS_RT_SIGPROCMASK => 0,
        SYS_SET_TID_ADDRESS => 0,
        SYS_SET_ROBUST_LIST => 0,
        SYS_FUTEX => 0, // TODO
        _ => {
            kor::println!("[korix] unknown syscall: {}", num);
            -38 // ENOSYS
        }
    }
}

// --- Process table (minimal) ---
static NEXT_PID: Mutex<usize> = Mutex::new(1);

fn alloc_pid() -> usize {
    let mut p = NEXT_PID.lock();
    *p += 1;
    *p
}

fn current_pid() -> usize {
    // TODO: per-CPU current task
    1
}

// --- Syscall implementations ---

fn sys_write(fd: usize, buf: usize, count: usize) -> isize {
    if fd == 1 || fd == 2 {
        // stdout/stderr → console
        let data = unsafe { core::slice::from_raw_parts(buf as *const u8, count) };
        for &b in data { kor::console::putc(b); }
        count as isize
    } else {
        // TODO: file write
        -9 // EBADF
    }
}

fn sys_read(fd: usize, _buf: usize, _count: usize) -> isize {
    if fd == 0 {
        // stdin → TODO: read from console input
        -38 // ENOSYS
    } else {
        // TODO: file read
        -9 // EBADF
    }
}

fn sys_writev(fd: usize, iov: usize, iovcnt: usize) -> isize {
    if fd == 1 || fd == 2 {
        let mut total = 0usize;
        for i in 0..iovcnt {
            let base = unsafe { *((iov + i * 16) as *const usize) };
            let len = unsafe { *((iov + i * 16 + 8) as *const usize) };
            if len > 0 {
                let data = unsafe { core::slice::from_raw_parts(base as *const u8, len) };
                for &b in data { kor::console::putc(b); }
                total += len;
            }
        }
        total as isize
    } else {
        -9
    }
}

fn sys_exit(code: usize) -> isize {
    kor::println!("[korix] process exit({})", code);
    // TODO: kill current task, schedule next
    loop {
        core::hint::spin_loop();
    }
}

fn sys_getpid() -> isize {
    current_pid() as isize
}

fn sys_brk(addr: usize) -> isize {
    // TODO: implement heap for user process
    // For now, return current brk (no change)
    addr as isize
}

fn sys_uname(buf: usize) -> isize {
    let names = [
        "korix\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "localhost\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "6.12.0-korix\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "#1 SMP korix\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "aarch64\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "(none)\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    ];
    for (i, name) in names.iter().enumerate() {
        let dst = buf + i * 65;
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
    let data = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, count) };
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
        core::ptr::copy_nonoverlapping(cwd.as_ptr(), buf as *mut u8, cwd.len());
    }
    buf as isize
}

fn sys_getdents64(_fd: usize, _buf: usize, _count: usize) -> isize {
    // TODO: implement directory listing
    0
}

fn sys_openat(_dirfd: usize, _path: usize, _flags: usize, _mode: usize) -> isize {
    // TODO: implement file open via VFS
    -2 // ENOENT
}

fn sys_close(_fd: usize) -> isize {
    0
}

fn sys_fstat(_fd: usize, _stat: usize) -> isize {
    -38 // ENOSYS
}

fn sys_lseek(_fd: usize, _offset: i64, _whence: usize) -> isize {
    -38
}

fn sys_mmap(_addr: usize, _len: usize, _prot: usize, _flags: usize, _fd: usize, _offset: i64) -> isize {
    // TODO: implement user-space mmap
    -38
}

fn sys_mkdirat(_dirfd: usize, _path: usize, _mode: usize) -> isize {
    -38
}

fn sys_unlinkat(_dirfd: usize, _path: usize) -> isize {
    -38
}
