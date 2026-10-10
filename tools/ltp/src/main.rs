//! LTP-style syscall test binary for korix (riscv64).
//!
//! Builds as a freestanding static ELF that issues raw `ecall`s — no libc —
//! so every test exercises the kernel syscall layer exactly the way LTP does.
//! Run inside the korix rootfs: `/tests/korix-ltp`.

#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

// --- riscv64 (asm-generic) syscall numbers ---------------------------------
const SYS_EXIT_GROUP: usize = 94;
const SYS_READ: usize = 63;
const SYS_WRITE: usize = 64;
const SYS_OPENAT: usize = 56;
const SYS_CLOSE: usize = 57;
const SYS_PIPE2: usize = 59;
const SYS_PREAD64: usize = 67;
const SYS_POLL: usize = 7;
const SYS_SCHED_YIELD: usize = 124;
const SYS_GETPGID: usize = 155;
const SYS_SETPGID: usize = 154;
const SYS_WAITID: usize = 95;
const SYS_STATFS: usize = 43;
const SYS_TRUNCATE: usize = 45;
const SYS_FTRUNCATE: usize = 46;
const SYS_FSYNC: usize = 82;
const SYS_MADVISE: usize = 233;
const SYS_MINCORE: usize = 232;
const SYS_GETRUSAGE: usize = 165;
const SYS_SETITIMER: usize = 103;
const SYS_GETITIMER: usize = 102;
const SYS_UNLINKAT: usize = 35;
const SYS_RT_SIGACTION: usize = 134;
const SYS_RT_SIGPROCMASK: usize = 135;
const SYS_KILL: usize = 129;
const SYS_CLONE: usize = 220;
const SYS_EXECVE: usize = 221;
const SYS_WAIT4: usize = 260;
const SYS_GETPID: usize = 172;
const SYS_GETPPID: usize = 173;

const SIGUSR1: usize = 10;
const SIGPIPE: usize = 13;
const SIGCHLD: usize = 17;
const SIG_BLOCK: usize = 0;
const SIG_SETMASK: usize = 2;
const SIG_IGN: usize = 1;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    exit(99)
}

/// Raw syscall via ecall.
#[inline(always)]
fn syscall(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") a0 as isize => ret,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
            in("a4") a4,
            in("a5") a5,
            in("a7") nr,
        );
    }
    ret
}

fn sys1(nr: usize, a: usize) -> isize {
    syscall(nr, a, 0, 0, 0, 0, 0)
}
fn sys2(nr: usize, a: usize, b: usize) -> isize {
    syscall(nr, a, b, 0, 0, 0, 0)
}
fn sys3(nr: usize, a: usize, b: usize, c: usize) -> isize {
    syscall(nr, a, b, c, 0, 0, 0)
}
fn sys4(nr: usize, a: usize, b: usize, c: usize, d: usize) -> isize {
    syscall(nr, a, b, c, d, 0, 0)
}

fn write_str(s: &str) {
    let _ = sys3(SYS_WRITE, 1, s.as_ptr() as usize, s.len());
}

fn exit(code: usize) -> ! {
    loop {
        let _ = sys1(SYS_EXIT_GROUP, code);
    }
}

// --- test harness -----------------------------------------------------------

static mut PASS: usize = 0;
static mut FAIL: usize = 0;

fn report(name: &str, ok: bool, detail: &str) {
    if ok {
        unsafe { PASS += 1 };
        write_str("ok   ");
    } else {
        unsafe { FAIL += 1 };
        write_str("FAIL ");
    }
    write_str(name);
    if !detail.is_empty() {
        write_str(" (");
        write_str(detail);
        write_str(")");
    }
    write_str("\n");
}

fn dec(value: usize) -> &'static str {
    const DIGITS: &[u8; 10] = b"0123456789";
    // SAFETY: static scratch written before every read, single-threaded.
    static mut BUF: [u8; 20] = [0; 20];
    const CAP: usize = 20;
    if value == 0 {
        return "0";
    }
    unsafe {
        let buf = core::ptr::addr_of_mut!(BUF);
        let mut idx = CAP;
        let mut v = value;
        while v > 0 {
            idx -= 1;
            (*buf)[idx] = DIGITS[v % 10];
            v /= 10;
        }
        core::str::from_utf8_unchecked(&*buf).split_at(idx).1
    }
}

const AT_FDCWD: usize = 0xFFFF_FF9C;
const O_RDONLY: usize = 0;
const O_WRONLY: usize = 1;
const O_CREAT: usize = 0o100;
const O_TRUNC: usize = 0o1000;


// --- procfs tests ------------------------------------------------------------

fn read_file(path: &[u8]) -> isize {
    let fd = sys4(SYS_OPENAT, AT_FDCWD, path.as_ptr() as usize, O_RDONLY, 0);
    if fd < 0 {
        return fd;
    }
    let mut buf = [0u8; 256];
    let got = sys3(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len());
    sys1(SYS_CLOSE, fd as usize);
    if got < 0 {
        return got;
    }
    // Return the length; contents are checked by the caller via re-read.
    got
}

fn test_procfs() {
    let version = b"/proc/version\0";
    let mut buf = [0u8; 256];
    let fd = sys4(SYS_OPENAT, AT_FDCWD, version.as_ptr() as usize, O_RDONLY, 0);
    if fd < 0 {
        report("procfs_version", false, "open failed");
    } else {
        let got = sys3(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len());
        sys1(SYS_CLOSE, fd as usize);
        let ok = got > 0 && buf.starts_with(b"Linux version");
        report("procfs_version", ok, "starts with Linux version");
    }

    let self_stat = b"/proc/self/stat\0";
    let fd = sys4(SYS_OPENAT, AT_FDCWD, self_stat.as_ptr() as usize, O_RDONLY, 0);
    if fd < 0 {
        report("procfs_self", false, "open /proc/self/stat failed");
    } else {
        let got = sys3(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len());
        sys1(SYS_CLOSE, fd as usize);
        // First field is our own pid.
        let pid = sys1(SYS_GETPID, 0);
        let prefix = format_pid_prefix(buf.split(|b| *b == b' ').next().unwrap_or(&[]));
        let ok = got > 0 && prefix == pid;
        report("procfs_self", ok, "self stat pid matches");
    }

    let meminfo = b"/proc/meminfo\0";
    let fd = sys4(SYS_OPENAT, AT_FDCWD, meminfo.as_ptr() as usize, O_RDONLY, 0);
    if fd < 0 {
        report("procfs_meminfo", false, "open failed");
    } else {
        let got = sys3(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len());
        sys1(SYS_CLOSE, fd as usize);
        let ok = got > 0 && buf.starts_with(b"MemTotal:");
        report("procfs_meminfo", ok, "MemTotal present");
    }
}

/// Compare a decimal byte slice against a pid value.
fn format_pid_prefix(bytes: &[u8]) -> isize {
    let mut value: isize = 0;
    for b in bytes {
        if !b.is_ascii_digit() {
            break;
        }
        value = value * 10 + (b - b'0') as isize;
    }
    value
}

// --- individual tests -------------------------------------------------------

// --- signal tests -----------------------------------------------------------

static mut HANDLER_HITS: usize = 0;

extern "C" fn sigusr1_handler(_signal: usize) {
    // SAFETY: single-threaded; handler body is a plain increment.
    unsafe { HANDLER_HITS += 1 };
}

fn test_sigusr1_delivery() {
    // rt_sigaction ABI: {handler, flags, restorer, mask} — 32 bytes.
    let action: [usize; 4] = [sigusr1_handler as usize, 0, 0, 0];
    let mut old: [usize; 4] = [0; 4];
    let r = sys3(
        SYS_RT_SIGACTION,
        SIGUSR1,
        action.as_ptr() as usize,
        old.as_mut_ptr() as usize,
    );
    if r != 0 {
        report("sigaction", false, "install failed");
        return;
    }
    let fired = sys2(SYS_KILL, sys1(SYS_GETPID, 0) as usize, SIGUSR1);
    // Give the kernel a return-to-user point to deliver the pending signal.
    sys1(SYS_SCHED_YIELD, 0);
    sys1(SYS_SCHED_YIELD, 0);
    let hits = unsafe { HANDLER_HITS };
    report("sigusr1", r == 0 && fired == 0 && hits >= 1, "handler ran");
}

fn test_sigprocmask_roundtrip() {
    let block_sigusr1: u64 = 1 << (SIGUSR1 - 1);
    let mut old: u64 = 0;
    let set = sys3(
        SYS_RT_SIGPROCMASK,
        SIG_BLOCK,
        &block_sigusr1 as *const u64 as usize,
        &mut old as *mut u64 as usize,
    );
    let mut current: u64 = 0;
    let query = sys3(
        SYS_RT_SIGPROCMASK,
        SIG_SETMASK,
        &block_sigusr1 as *const u64 as usize,
        &mut current as *mut u64 as usize,
    );
    // Restore the empty mask.
    let empty: u64 = 0;
    sys3(SYS_RT_SIGPROCMASK, SIG_SETMASK, &empty as *const u64 as usize, 0);
    report(
        "sigprocmask",
        set == 0 && query == 0 && current & block_sigusr1 != 0,
        "block persists",
    );
}

fn test_sigpipe_epipe() {
    // Ignore SIGPIPE, then write to a pipe whose read end is fully closed:
    // the write must fail with EPIPE (-32), not kill the process.
    let ignore: [usize; 4] = [SIG_IGN, 0, 0, 0];
    sys3(SYS_RT_SIGACTION, SIGPIPE, ignore.as_ptr() as usize, 0);
    let mut fds = [0i32; 2];
    if sys3(SYS_PIPE2, fds.as_mut_ptr() as usize, 2, 0) != 0 {
        report("sigpipe_epipe", false, "pipe2 failed");
        return;
    }
    sys1(SYS_CLOSE, fds[0] as usize); // close the read end
    const MSG: &[u8] = b"doomed";
    let w = sys3(SYS_WRITE, fds[1] as usize, MSG.as_ptr() as usize, MSG.len());
    sys1(SYS_CLOSE, fds[1] as usize);
    report("sigpipe_epipe", w == -32, "EPIPE not death");
}

// --- fork/exec tests --------------------------------------------------------

fn test_fork_exit_status() {
    const SIGCHLD_FLAGS: usize = 17; // SIGCHLD
    let pid = syscall(SYS_CLONE, SIGCHLD_FLAGS, 0, 0, 0, 0, 0);
    if pid < 0 {
        report("fork_status", false, "clone failed");
        return;
    }
    if pid == 0 {
        exit(42);
    }
    let mut status: i32 = 0;
    sys3(SYS_WAIT4, pid as usize, &mut status as *mut i32 as usize, 0);
    report("fork_status", status == (42 << 8), "child exit 42");
}

fn test_exec_true() {
    const SIGCHLD_FLAGS: usize = 17;
    let pid = syscall(SYS_CLONE, SIGCHLD_FLAGS, 0, 0, 0, 0, 0);
    if pid < 0 {
        report("exec_true", false, "clone failed");
        return;
    }
    if pid == 0 {
        let path = b"/bin/true\0";
        let mut argv: [usize; 2] = [path.as_ptr() as usize, 0];
        let r = syscall(SYS_EXECVE, path.as_ptr() as usize, argv.as_mut_ptr() as usize, 0, 0, 0, 0);
        if r < 0 {
            exit(98); // exec failed
        }
        exit(0); // unreachable when exec succeeds
    }
    let mut status: i32 = 0;
    sys3(SYS_WAIT4, pid as usize, &mut status as *mut i32 as usize, 0);
    report("exec_true", status == 0, "exec /bin/true exit 0");
}

// --- error-path tests -------------------------------------------------------

/// Format an errno as decimal, e.g. "-2". Scratch buffer is per-call.
fn signed(value: isize) -> &'static str {
    const DIGITS: &[u8; 10] = b"0123456789";
    // SAFETY: static scratch rewritten before every read; single-threaded.
    static mut BUF: [u8; 12] = [0; 12];
    const CAP: usize = 12;
    let mut v = value.unsigned_abs();
    let mut idx = CAP;
    if v == 0 {
        idx -= 1;
        unsafe { (*core::ptr::addr_of_mut!(BUF))[idx] = b"0"[0]; }
    }
    while v > 0 {
        idx -= 1;
        unsafe { (*core::ptr::addr_of_mut!(BUF))[idx] = DIGITS[v % 10]; }
        v /= 10;
    }
    if value < 0 {
        idx -= 1;
        unsafe { (*core::ptr::addr_of_mut!(BUF))[idx] = b"-"[0]; }
    }
    unsafe { core::str::from_utf8_unchecked(&*core::ptr::addr_of!(BUF)).split_at(idx).1 }
}

fn test_error_paths() {
    let missing = b"/no/such/file\0";
    let open_errno = sys4(SYS_OPENAT, AT_FDCWD, missing.as_ptr() as usize, O_RDONLY, 0);
    let bad_close = sys1(SYS_CLOSE, 99);
    let bad_kill = sys2(SYS_KILL, 0xDEAD_BEEF, SIGUSR1);
    let bad_read = sys3(SYS_READ, 1234, 0x1000, 8);
    if open_errno == -2 && bad_close == -9 && bad_kill == -3 && bad_read == -9 {
        report("error_paths", true, "");
    } else {
        let detail = [
            ("open", open_errno),
            ("close", bad_close),
            ("kill", bad_kill),
            ("read", bad_read),
        ];
        for (name, value) in detail {
            write_str(name);
            write_str("=");
            write_str(signed(value));
            write_str(" ");
        }
        report("error_paths", false, "see values above");
    }
}

fn test_pipe2() {
    let mut fds = [0i32; 2];
    let r = sys3(SYS_PIPE2, fds.as_mut_ptr() as usize, 2, 0);
    if r != 0 {
        report("pipe2", false, "create failed");
        return;
    }
    let (rd, wr) = (fds[0] as usize, fds[1] as usize);
    const MSG: &[u8] = b"pipe-data";
    let w = sys3(SYS_WRITE, wr, MSG.as_ptr() as usize, MSG.len());
    let mut buf = [0u8; 32];
    let got = sys3(SYS_READ, rd, buf.as_mut_ptr() as usize, MSG.len());
    let payload_ok = got == MSG.len() as isize && &buf[..MSG.len()] == MSG;
    sys1(SYS_CLOSE, rd);
    sys1(SYS_CLOSE, wr);
    report("pipe2", r == 0 && w == MSG.len() as isize && payload_ok, "write/read echo");
}

fn test_pipe_eof() {
    let mut fds = [0i32; 2];
    if sys3(SYS_PIPE2, fds.as_mut_ptr() as usize, 2, 0) != 0 {
        report("pipe_eof", false, "pipe2 failed");
        return;
    }
    let (rd, wr) = (fds[0] as usize, fds[1] as usize);
    sys1(SYS_CLOSE, wr);
    let mut buf = [0u8; 8];
    let got = sys3(SYS_READ, rd, buf.as_mut_ptr() as usize, 8);
    sys1(SYS_CLOSE, rd);
    report("pipe_eof", got == 0, "read after writer close returns 0");
}

fn test_sched_yield() {
    let r = sys1(SYS_SCHED_YIELD, 0);
    report("sched_yield", r == 0, "returns 0");
}

fn test_getpgid() {
    let pgid = sys1(SYS_GETPGID, 0);
    report("getpgid", pgid >= 0, "no error");
    let set = sys2(SYS_SETPGID, 0, 0);
    report("setpgid", set == 0 || set == -1, "0 or EPERM");
}

fn test_statfs() {
    // struct statfs (asm-generic riscv64): f_type, f_bsize as long,
    // then f_blocks..f_ffree as quad.  Reading offsets 8..16 gives f_bsize.
    let mut buf = [0u8; 120];
    let path = b"/\0";
    let r = sys2(SYS_STATFS, path.as_ptr() as usize, buf.as_mut_ptr() as usize);
    let bsize = u64::from_le_bytes(buf[8..16].try_into().unwrap());
    report("statfs", r == 0 && bsize > 0, "root fs bsize nonzero");
}

fn test_ftruncate() {
    let path = b"/tmp/trunc-test\0";
    let fd = sys4(SYS_OPENAT, AT_FDCWD, path.as_ptr() as usize, O_WRONLY | O_CREAT | O_TRUNC, 0o644);
    if fd < 0 {
        report("ftruncate", false, "open for create failed");
        return;
    }
    const DATA: &[u8] = b"0123456789abcdef";
    sys3(SYS_WRITE, fd as usize, DATA.as_ptr() as usize, DATA.len());
    let t = sys2(SYS_FTRUNCATE, fd as usize, 4);
    let mut buf = [0u8; 32];
    let got = sys4(SYS_PREAD64, fd as usize, buf.as_mut_ptr() as usize, 32, 0);
    sys1(SYS_CLOSE, fd as usize);
    sys3(SYS_UNLINKAT, AT_FDCWD, path.as_ptr() as usize, 0);
    report("ftruncate", t == 0 && got == 4, "size shrinks to 4");
}

fn test_pread64() {
    let path = b"/etc/passwd\0";
    let fd = sys4(SYS_OPENAT, AT_FDCWD, path.as_ptr() as usize, O_RDONLY, 0);
    if fd < 0 {
        report("pread64", false, "open /etc/passwd failed");
        return;
    }
    let mut a = [0u8; 8];
    let mut b = [0u8; 8];
    let r1 = sys4(SYS_PREAD64, fd as usize, a.as_mut_ptr() as usize, 8, 0);
    let r2 = sys4(SYS_PREAD64, fd as usize, b.as_mut_ptr() as usize, 8, 0);
    sys1(SYS_CLOSE, fd as usize);
    report("pread64", r1 == 8 && r2 == 8 && a == b, "same offset twice");
}

fn test_getrusage() {
    let mut buf = [0u8; 144]; // struct rusage = 18 longs
    let r = sys2(SYS_GETRUSAGE, 0, buf.as_mut_ptr() as usize);
    report("getrusage", r == 0, "RUSAGE_SELF");
}

fn test_madvise_mincore() {
    let page = &test_madvise_mincore as *const _ as usize & !0xFFF;
    let m = sys3(SYS_MADVISE, page, 0x1000, 0);
    let mut vec = [0u8; 1];
    let i = sys3(SYS_MINCORE, page, 0x1000, vec.as_mut_ptr() as usize);
    report("madvise/mincore", m == 0 && i == 0, "no-op semantics");
}

fn test_poll() {
    let mut fds = [0i32; 2];
    if sys3(SYS_PIPE2, fds.as_mut_ptr() as usize, 2, 0) != 0 {
        report("poll", false, "pipe2 failed");
        return;
    }
    // struct pollfd { fd: i32, events: u16, revents: u16 } — 8 bytes packed.
    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: u16,
        revents: u16,
    }
    let mut pfd = [PollFd { fd: fds[0], events: 1, revents: 0 }]; // POLLIN on the read end
    let n = sys3(SYS_POLL, pfd.as_mut_ptr() as usize, 1, 0);
    let empty_ready = n == 0 && pfd[0].revents == 0;
    const MSG: &[u8] = b"x";
    sys3(SYS_WRITE, fds[1] as usize, MSG.as_ptr() as usize, 1);
    let n2 = sys3(SYS_POLL, pfd.as_mut_ptr() as usize, 1, 0);
    let ready = n2 == 1 && pfd[0].revents & 1 != 0;
    sys1(SYS_CLOSE, fds[0] as usize);
    sys1(SYS_CLOSE, fds[1] as usize);
    report("poll", empty_ready && ready, "empty then readable");
}

fn test_waitid_echild() {
    // waitid(P_ALL, 0, NULL, WEXITED) with no children must fail ECHILD (-10).
    let r = sys4(SYS_WAITID, 0, 0, 0, 4);
    report("waitid_echild", r == -10, "ECHILD from childless wait");
}

fn test_fsync() {
    let path = b"/etc/passwd\0";
    let fd = sys4(SYS_OPENAT, AT_FDCWD, path.as_ptr() as usize, O_RDONLY, 0);
    if fd < 0 {
        report("fsync", false, "open failed");
        return;
    }
    let r = sys1(SYS_FSYNC, fd as usize);
    sys1(SYS_CLOSE, fd as usize);
    report("fsync", r == 0, "sync open file");
}

fn test_setitimer() {
    let mut new_v = [0u8; 32];
    let mut old_v = [0u8; 32];
    let r = sys3(SYS_SETITIMER, 0, new_v.as_mut_ptr() as usize, old_v.as_mut_ptr() as usize);
    let g = sys2(SYS_GETITIMER, 0, old_v.as_mut_ptr() as usize);
    report("setitimer/getitimer", r == 0 && g == 0, "ITIMER_REAL");
}

fn test_truncate() {
    let path = b"/tmp/trunc-path\0";
    let fd = sys4(SYS_OPENAT, AT_FDCWD, path.as_ptr() as usize, O_WRONLY | O_CREAT | O_TRUNC, 0o644);
    if fd < 0 {
        report("truncate", false, "open failed");
        return;
    }
    const DATA: &[u8] = b"0123456789abcdef";
    sys3(SYS_WRITE, fd as usize, DATA.as_ptr() as usize, DATA.len());
    sys1(SYS_CLOSE, fd as usize);
    let t = sys2(SYS_TRUNCATE, path.as_ptr() as usize, 8);
    sys3(SYS_UNLINKAT, AT_FDCWD, path.as_ptr() as usize, 0);
    report("truncate", t == 0, "path truncate to 8");
}

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    write_str("=== korix LTP-style syscall tests ===\n");
    test_pipe2();
    test_pipe_eof();
    test_poll();
    test_sched_yield();
    test_getpgid();
    test_statfs();
    test_ftruncate();
    test_pread64();
    test_getrusage();
    test_madvise_mincore();
    test_waitid_echild();
    test_fsync();
    test_setitimer();
    test_truncate();
    test_sigusr1_delivery();
    test_sigprocmask_roundtrip();
    test_sigpipe_epipe();
    test_fork_exit_status();
    test_exec_true();
    test_error_paths();
    test_procfs();
    unsafe {
        write_str("--- pass ");
        write_str(dec(PASS));
        write_str(" fail ");
        write_str(dec(FAIL));
        write_str(" ---\n");
        exit(if FAIL > 0 { 1 } else { 0 })
    }
}
