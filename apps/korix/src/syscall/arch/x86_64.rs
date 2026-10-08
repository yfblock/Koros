//! x86_64 adapter: translates the x86_64 syscall ABI (rax = nr,
//! rdi/rsi/rdx/r10/r8/r9 = args) into the canonical `asm-generic` form used
//! by [`crate::syscall`].
//!
//! Two kinds of mapping exist:
//! - pure renumbering (most calls), via the table below;
//! - argument reshuffling: calls with no direct generic equivalent are
//!   redirected to their `*at` relatives with `AT_FDCWD` spliced into the
//!   argument vector;
//! - native-only calls (e.g. `arch_prctl`) map to the internal pseudo-number
//!   block (500+) and run in [`native_syscall`].

use crate::syscall::user_mem::write as write_user;

/// Internal pseudo-number for the x86-only `arch_prctl(2)` (see
/// `crate::syscall::SYS_ARCH_PRCTL`).
use crate::syscall::SYS_ARCH_PRCTL;

/// `AT_FDCWD` sentinel for the `*at` redirects below.
const AT_FDCWD: usize = -100isize as usize;
/// `AT_SYMLINK_NOFOLLOW` for the lstat redirect.
const AT_SYMLINK_NOFOLLOW: usize = 0x100;
/// `AT_REMOVEDIR` for the rmdir redirect.
const AT_REMOVEDIR: usize = 0x200;

// Native x86_64 syscall numbers (a different table from asm-generic).
mod nr {
    pub const READ: usize = 0;
    pub const WRITE: usize = 1;
    pub const OPEN: usize = 2;
    pub const CLOSE: usize = 3;
    pub const STAT: usize = 4;
    pub const FSTAT: usize = 5;
    pub const LSTAT: usize = 6;
    pub const POLL: usize = 7;
    pub const LSEEK: usize = 8;
    pub const MMAP: usize = 9;
    pub const MPROTECT: usize = 10;
    pub const MUNMAP: usize = 11;
    pub const BRK: usize = 12;
    pub const RT_SIGACTION: usize = 13;
    pub const RT_SIGPROCMASK: usize = 14;
    pub const RT_SIGRETURN: usize = 15;
    pub const IOCTL: usize = 16;
    pub const WRITEV: usize = 20;
    pub const ACCESS: usize = 21;
    pub const SELECT: usize = 23;
    pub const DUP: usize = 32;
    pub const DUP2: usize = 33;
    pub const NANOSLEEP: usize = 35;
    pub const GETPID: usize = 39;
    pub const SOCKET: usize = 41;
    pub const CONNECT: usize = 42;
    pub const SENDTO: usize = 44;
    pub const RECVFROM: usize = 45;
    pub const SENDMSG: usize = 46;
    pub const RECVMSG: usize = 47;
    pub const SHUTDOWN: usize = 48;
    pub const BIND: usize = 49;
    pub const GETSOCKNAME: usize = 51;
    pub const GETPEERNAME: usize = 52;
    pub const SETSOCKOPT: usize = 54;
    pub const GETSOCKOPT: usize = 55;
    pub const CLONE: usize = 56;
    pub const FORK: usize = 57;
    pub const EXECVE: usize = 59;
    pub const EXIT: usize = 60;
    pub const WAIT4: usize = 61;
    pub const KILL: usize = 62;
    pub const UNAME: usize = 63;
    pub const FCNTL: usize = 72;
    pub const FLOCK: usize = 73;
    pub const GETCWD: usize = 79;
    pub const CHDIR: usize = 80;
    pub const RENAME: usize = 82;
    pub const MKDIR: usize = 83;
    pub const RMDIR: usize = 84;
    pub const LINK: usize = 86;
    pub const UNLINK: usize = 87;
    pub const SYMLINK: usize = 88;
    pub const FCHMOD: usize = 91;
    pub const FCHOWN: usize = 93;
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
    pub const RT_SIGSUSPEND: usize = 130;
    pub const STATFS: usize = 137;
    pub const FSTATFS: usize = 138;
    pub const GETPRIORITY: usize = 140;
    pub const ARCH_PRCTL: usize = 158;
    pub const SCHED_SETSCHEDULER: usize = 144;
    pub const SCHED_GETSCHEDULER: usize = 145;
    pub const SCHED_SETPARAM: usize = 142;
    pub const SCHED_GETPARAM: usize = 159;
    pub const SCHED_SETAFFINITY: usize = 203;
    pub const SCHED_GETAFFINITY: usize = 204;
    pub const GETXATTR: usize = 191;
    pub const GETTIMEOFDAY: usize = 96;
    pub const REBOOT: usize = 169;
    pub const PRCTL: usize = 157;
    pub const SYNC: usize = 162;
    pub const MOUNT: usize = 165;
    pub const UMOUNT2: usize = 166;
    pub const GETTID: usize = 186;
    pub const FUTEX: usize = 202;
    pub const GETDENTS64: usize = 217;
    pub const SET_TID_ADDRESS: usize = 218;
    pub const EXIT_GROUP: usize = 231;
    pub const TGKILL: usize = 234;
    pub const CLOCK_GETTIME: usize = 228;
    pub const OPENAT: usize = 257;
    pub const MKDIRAT: usize = 258;
    pub const NEWFSTATAT: usize = 262;
    pub const UNLINKAT: usize = 263;
    pub const RENAMEAT: usize = 264;
    pub const LINKAT: usize = 265;
    pub const SYMLINKAT: usize = 266;
    pub const READLINKAT: usize = 267;
    pub const FCHMODAT: usize = 268;
    pub const FACCESSAT: usize = 269;
    pub const PPOLL: usize = 271;
    pub const SET_ROBUST_LIST: usize = 273;
    pub const FCHOWNAT: usize = 260;
    pub const UTIMENSAT: usize = 280;
    pub const PRLIMIT64: usize = 302;
    pub const GETRANDOM: usize = 318;
    pub const RSEQ: usize = 334;
    pub const RENAMEAT2: usize = 316;
}

/// Translate a native x86_64 syscall `(nr, args)` into canonical form.
/// Returns `None` for calls the kernel does not know (→ -ENOSYS).
pub fn translate(nr: usize, args: &mut [usize; 6]) -> Option<usize> {
    use nr::*;
    match nr {
        // Calls without a generic relative: reshape the argument vector and
        // redirect to the `*at` form with AT_FDCWD.
        OPEN => {
            args[3] = args[2];
            args[2] = args[1];
            args[1] = args[0];
            args[0] = AT_FDCWD;
            Some(super::super::SYS_OPENAT)
        }
        STAT => {
            args[3] = 0;
            args[2] = args[1];
            args[1] = args[0];
            args[0] = AT_FDCWD;
            Some(super::super::SYS_NEWFSTATAT)
        }
        LSTAT => {
            args[3] = AT_SYMLINK_NOFOLLOW;
            args[2] = args[1];
            args[1] = args[0];
            args[0] = AT_FDCWD;
            Some(super::super::SYS_NEWFSTATAT)
        }
        RENAME => {
            args[3] = args[1];
            args[2] = args[0];
            args[1] = AT_FDCWD;
            args[0] = AT_FDCWD;
            Some(super::super::SYS_RENAMEAT)
        }
        MKDIR => {
            args[2] = args[1];
            args[1] = args[0];
            args[0] = AT_FDCWD;
            Some(super::super::SYS_MKDIRAT)
        }
        RMDIR => {
            args[2] = AT_REMOVEDIR;
            args[1] = args[0];
            args[0] = AT_FDCWD;
            Some(super::super::SYS_UNLINKAT)
        }
        LINK => {
            args[4] = 0;
            args[3] = args[1];
            args[2] = args[0];
            args[1] = AT_FDCWD;
            args[0] = AT_FDCWD;
            Some(super::super::SYS_LINKAT)
        }
        UNLINK => {
            args[2] = 0;
            args[1] = args[0];
            args[0] = AT_FDCWD;
            Some(super::super::SYS_UNLINKAT)
        }
        SYMLINK => {
            args[2] = args[1];
            args[1] = AT_FDCWD;
            Some(super::super::SYS_SYMLINKAT)
        }
        // Native-only call: internal pseudo-number.
        ARCH_PRCTL => Some(SYS_ARCH_PRCTL),

        // Pure renumbering.
        READ => Some(super::super::SYS_READ),
        WRITE => Some(super::super::SYS_WRITE),
        CLOSE => Some(super::super::SYS_CLOSE),
        FSTAT => Some(super::super::SYS_FSTAT),
        LSEEK => Some(super::super::SYS_LSEEK),
        MMAP => Some(super::super::SYS_MMAP),
        MPROTECT => Some(super::super::SYS_MPROTECT),
        MUNMAP => Some(super::super::SYS_MUNMAP),
        BRK => Some(super::super::SYS_BRK),
        RT_SIGACTION => Some(super::super::SYS_RT_SIGACTION),
        BIND => Some(200),
        SOCKET => Some(super::super::SYS_SOCKET),
        CONNECT => Some(super::super::SYS_CONNECT),
        SENDTO => Some(super::super::SYS_SENDTO),
        RECVFROM => Some(super::super::SYS_RECVFROM),
        SHUTDOWN => Some(super::super::SYS_SHUTDOWN),
        SENDMSG => Some(super::super::SYS_SENDMSG),
        RECVMSG => Some(super::super::SYS_RECVMSG),
        GETSOCKNAME => Some(super::super::SYS_GETSOCKNAME),
        GETPEERNAME => Some(super::super::SYS_GETPEERNAME),
        SETSOCKOPT => Some(super::super::SYS_SETSOCKOPT),
        GETSOCKOPT => Some(super::super::SYS_GETSOCKOPT),
        RT_SIGPROCMASK => Some(super::super::SYS_RT_SIGPROCMASK),
        FLOCK => Some(294),
        // Not implemented yet — the black-hole slot answers 0.
        FCHMOD | FCHOWN | FCHMODAT | FCHOWNAT | UTIMENSAT => Some(200),
        FACCESSAT => Some(295),
        ACCESS => Some(296),
        GETXATTR => Some(297),
        STATFS | FSTATFS => Some(298),
        GETPRIORITY => Some(0),
        RT_SIGSUSPEND => Some(super::super::SYS_RT_SIGSUSPEND),
        RT_SIGRETURN => Some(super::super::SYS_RT_SIGRETURN),
        IOCTL => Some(super::super::SYS_IOCTL),
        WRITEV => Some(super::super::SYS_WRITEV),
        DUP => Some(super::super::SYS_DUP),
        DUP2 => Some(super::super::SYS_DUP2),
        NANOSLEEP => Some(super::super::SYS_NANOSLEEP),
        POLL => Some(73), // poll → ppoll handler
        SELECT => Some(299),
        GETPID => Some(super::super::SYS_GETPID),
        CLONE => Some(super::super::SYS_CLONE),
        FORK => Some(super::super::SYS_CLONE), // fork → clone handler
        EXECVE => Some(super::super::SYS_EXECVE),
        EXIT => Some(super::super::SYS_EXIT),
        WAIT4 => Some(super::super::SYS_WAIT4),
        KILL => Some(super::super::SYS_KILL),
        UNAME => Some(super::super::SYS_UNAME),
        FCNTL => Some(super::super::SYS_FCNTL),
        GETDENTS64 => Some(super::super::SYS_GETDENTS64),
        GETCWD => Some(super::super::SYS_GETCWD),
        CHDIR => Some(super::super::SYS_CHDIR),
        GETUID => Some(super::super::SYS_GETUID),
        GETGID => Some(super::super::SYS_GETGID),
        GETEUID => Some(175),
        GETEGID => Some(177),
        GETPPID => Some(super::super::SYS_GETPPID),
        GETPGRP => Some(155),
        SETSID => Some(157),
        GETGROUPS => Some(158),
        GETPGID => Some(155),
        GETSID => Some(156),
        SCHED_SETSCHEDULER => Some(144),
        SCHED_GETSCHEDULER => Some(145),
        SCHED_GETPARAM => Some(146),
        SCHED_SETPARAM => Some(143),
        SCHED_SETAFFINITY => Some(148),
        SCHED_GETAFFINITY => Some(149),
        PRCTL => Some(167),
        SYNC => Some(super::super::SYS_SYNC),
        MOUNT => Some(super::super::SYS_MOUNT),
        UMOUNT2 => Some(super::super::SYS_UMOUNT2),
        GETTID => Some(178),
        FUTEX => Some(super::super::SYS_FUTEX),
        SET_TID_ADDRESS => Some(super::super::SYS_SET_TID_ADDRESS),
        EXIT_GROUP => Some(super::super::SYS_EXIT_GROUP),
        TGKILL => Some(super::super::SYS_TGKILL),
        CLOCK_GETTIME => Some(113),
        GETTIMEOFDAY => Some(169),
        REBOOT => Some(super::super::SYS_REBOOT),
        OPENAT => Some(super::super::SYS_OPENAT),
        MKDIRAT => Some(super::super::SYS_MKDIRAT),
        RENAMEAT2 => Some(super::super::SYS_RENAMEAT2),
        NEWFSTATAT => Some(super::super::SYS_NEWFSTATAT),
        UNLINKAT => Some(super::super::SYS_UNLINKAT),
        RENAMEAT => Some(super::super::SYS_RENAMEAT),
        LINKAT => Some(super::super::SYS_LINKAT),
        READLINKAT => Some(super::super::SYS_READLINKAT),
        PPOLL => Some(73),
        SYMLINKAT => Some(super::super::SYS_SYMLINKAT),
        SET_ROBUST_LIST => Some(super::super::SYS_SET_ROBUST_LIST),
        PRLIMIT64 => Some(261),
        GETRANDOM => Some(super::super::SYS_GETRANDOM),
        RSEQ => Some(293),
        201 => Some(201), // time (identity)
        _ => None,
    }
}

/// Run an x86_64-only syscall routed through the internal pseudo-numbers.
pub fn native_syscall(nr: usize, args: &[usize]) -> isize {
    match nr {
        SYS_ARCH_PRCTL => sys_arch_prctl(args[0], args[1]),
        _ => -38, // -ENOSYS
    }
}

/// `arch_prctl(2)`: FS/GS base management, used by musl for TLS setup.
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
