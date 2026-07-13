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

// Physical RAM start — used to distinguish user pages from identity-mapped kernel pages.
#[cfg(target_arch = "riscv64")]
pub const RAM_START: usize = 0x8000_0000;
#[cfg(target_arch = "aarch64")]
pub const RAM_START: usize = 0x4000_0000;
#[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64")))]
pub const RAM_START: usize = 0x8000_0000;
/// Dispatch a syscall.
/// `num` is the syscall number, `args` is up to 6 arguments.
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
static CHILD_EXIT_CODE: Mutex<i32> = Mutex::new(0);
static FORK_ACTIVE: Mutex<bool> = Mutex::new(false);

pub static mut CURRENT_TF: *mut u8 = core::ptr::null_mut();
// Fork context: saved registers for parent process

pub fn dispatch(num: usize, args: &[usize]) -> isize {
    dispatch_inner(num, args)
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
        SYS_EXECVE => 0, // standalone mode: no-op (applet runs directly)
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
    // TODO: per-CPU current task
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
            if let Some(b) = rx_get() { slice[read] = b; read += 1; }
        }
        while read < count {
            match rx_get() { Some(b) => { slice[read] = b; read += 1; } None => break, }
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
        if let Some(regs) = regs {
            unsafe {
                if !CURRENT_TF.is_null() {
                    #[cfg(target_arch = "riscv64")]
                    let nregs = 32;
                    #[cfg(target_arch = "aarch64")]
                    let nregs = 31;
                    let tf = CURRENT_TF as *mut usize;
                    for i in 0..nregs {
                        *tf.add(i) = regs[i];
                    }
                    // On aarch64, restore SP_EL0 (at offset 31*8) from saved[31]
                    #[cfg(target_arch = "aarch64")]
                    {
                        let tf_sp = (CURRENT_TF as *mut u8).add(31 * 8) as *mut usize;
                        *tf_sp = regs[31];
                    }
                    // PC at offset 32*8 (sepc on riscv64, elr on aarch64)
                    let tf_pc = (CURRENT_TF as *mut u8).add(32 * 8) as *mut usize;
                    *tf_pc = saved_pc;
                    // Status at offset 33*8 (sstatus on riscv64, spsr on aarch64)
                    let tf_status = (CURRENT_TF as *mut u8).add(33 * 8) as *mut usize;
                    *tf_status = saved_status;
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
        let tf = CURRENT_TF as *mut usize;
        let mut saved = [0usize; 32];
        for i in 0..nregs { saved[i] = *tf.add(i); }
        // On aarch64, also save SP_EL0 (at offset 31*8) in saved[31]
        #[cfg(target_arch = "aarch64")]
        {
            saved[31] = *((CURRENT_TF as *const u8).add(31 * 8) as *const usize);
        }
        *FORK_REGS.lock() = Some(saved);
        // Save PC (offset 32*8) and status (offset 33*8)
        let real_pc = *((CURRENT_TF as *const u8).add(32 * 8) as *const usize);
        let real_status = *((CURRENT_TF as *const u8).add(33 * 8) as *const usize);
        *FORK_SEPC.lock() = real_pc;
        *FORK_SSTATUS.lock() = real_status;

        // Get parent SP
        #[cfg(target_arch = "riscv64")]
        let parent_sp = *tf.add(2); // x2 = sp
        #[cfg(target_arch = "aarch64")]
        let parent_sp = *((CURRENT_TF as *const u8).add(31 * 8) as *const usize); // SP_EL0

        // Allocate child stack
        let arch = kor::arch::current();
        const CHILD_STACK_SIZE: usize = 64 * 1024;
        #[cfg(target_arch = "riscv64")]
        let (user_stack_top, child_stack_top) = (0x10_0000_0000usize, 0x20_0100_0000usize);
        #[cfg(target_arch = "aarch64")]
        let (user_stack_top, child_stack_top) = (0x80_0000_0000usize, 0x80_0100_0000usize);
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
    }
    *FORK_ACTIVE.lock() = true;
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

fn sys_brk(addr: usize) -> isize {
    let mut brk = BRK_CURRENT.lock();
    if *brk == 0 {
        *brk = if cfg!(target_arch = "aarch64") { 0x80_0100_0000 } else { 0x1_0001_0000 };
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
    if path_str == "/dev/null" || path_str.as_str() == "/dev/tty" || path_str.as_str() == "/dev/console" {
        return 3; // fake fd
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
