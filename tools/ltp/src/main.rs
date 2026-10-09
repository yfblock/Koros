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

// --- individual tests -------------------------------------------------------

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
    unsafe {
        write_str("--- pass ");
        write_str(dec(PASS));
        write_str(" fail ");
        write_str(dec(FAIL));
        write_str(" ---\n");
        exit(if FAIL > 0 { 1 } else { 0 })
    }
}
