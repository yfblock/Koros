//! On-demand storage throughput benchmark (triggered by the `bench` command
//! line flag).  Measures ext2-over-virtio and raw virtio block throughput.
//!
//! Only built for architectures with a convenient tick source (riscv64
//! `rdtime`, x86_64 TSC calibrated against the PIT).

extern crate alloc;

use crate::drivers::block::BlockDevice;
use crate::println;

/// Bytes transferred per benchmark phase.
const BENCH_BYTES: usize = 8 * 1024 * 1024;
/// ext2 write/read chunk size.
const CHUNK: usize = 64 * 1024;

/// Run the throughput benchmarks against the first registered block device.
pub fn run() {
    let device = match crate::drivers::block::first() {
        Some(dev) => dev,
        None => {
            println!("bench SKIPPED: no block device registered");
            return;
        }
    };
    ext2_bench(&device);
    virtio_bench(&device);
}

// ---------------------------------------------------------------------------
// Tick source
// ---------------------------------------------------------------------------

/// Read a monotonic tick counter.
#[cfg(target_arch = "riscv64")]
#[inline]
fn now_ticks() -> u64 {
    let t: u64;
    // SAFETY: `rdtime` is an unprivileged read of the timer CSR.
    unsafe {
        core::arch::asm!("rdtime {t}", t = out(reg) t);
    }
    t
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn now_ticks() -> u64 {
    // SAFETY: `rdtsc` reads the time-stamp counter.
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Frequency of [`now_ticks`] in Hz.
#[cfg(target_arch = "riscv64")]
fn timer_hz() -> u64 {
    10_000_000 // QEMU `riscv64 -machine virt` timebase
}

/// Calibrate the x86_64 TSC against the 1.193182 MHz PIT (channel 2).
#[cfg(target_arch = "x86_64")]
fn timer_hz() -> u64 {
    use x86_64::instructions::port::Port;
    const PIT_HZ: u64 = 1_193_182;
    let count: u16 = 11_932; // ≈ 10 ms

    // SAFETY: ring-0 port I/O to the legacy PIT and control port.
    unsafe {
        let mut p61 = Port::<u8>::new(0x61);
        let orig = p61.read();
        p61.write((orig & 0xFC) | 0x01);
        Port::<u8>::new(0x43).write(0b1011_0000);
        Port::<u8>::new(0x42).write((count & 0xFF) as u8);
        Port::<u8>::new(0x42).write((count >> 8) as u8);

        let start = now_ticks();
        while p61.read() & 0x20 == 0 {}
        let end = now_ticks();
        p61.write(orig);

        (end - start).saturating_mul(PIT_HZ) / count as u64
    }
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// Sequential ext2 file write/read throughput.
///
/// Write phase streams a large file and `sync()`s it; read phase reopens the
/// filesystem (cold cache) so reads hit the device.
fn ext2_bench(device: &alloc::sync::Arc<dyn BlockDevice>) {
    use crate::fs::ext2::Ext2Fs;
    use crate::fs::{FileType, SuperBlock as SuperBlockTrait};

    let timebase_hz = timer_hz();

    let fs = match Ext2Fs::open(device.clone()) {
        Ok(f) => f,
        Err(e) => {
            println!("ext2 bench SKIPPED: open: {:?}", e);
            return;
        }
    };
    let info = fs.info();
    if info.free_blocks.saturating_mul(info.block_size) < BENCH_BYTES + BENCH_BYTES / 8 {
        println!(
            "ext2 bench SKIPPED: image too small ({} KiB free)",
            info.free_blocks * info.block_size / 1024
        );
        return;
    }

    let block_size = fs.block_size();
    let root = fs.root_inode();
    let file = match root.create("bench.bin", FileType::Regular, 0o644) {
        Ok(f) => f,
        Err(e) => {
            println!("ext2 bench SKIPPED: create: {:?}", e);
            return;
        }
    };
    let buf = alloc::vec![0xCDu8; CHUNK];
    let t0 = now_ticks();
    let mut off = 0;
    while off < BENCH_BYTES {
        if let Err(e) = file.write_at(off, &buf) {
            println!("ext2 bench SKIPPED: write @{}: {:?}", off, e);
            return;
        }
        off += CHUNK;
    }
    fs.sync();
    let write_ticks = now_ticks() - t0;

    let fs2 = match Ext2Fs::open(device.clone()) {
        Ok(f) => f,
        Err(e) => {
            println!("ext2 bench SKIPPED: reopen: {:?}", e);
            return;
        }
    };
    let file2 = match fs2.root_inode().lookup("bench.bin") {
        Ok(f) => f,
        Err(e) => {
            println!("ext2 bench SKIPPED: lookup: {:?}", e);
            return;
        }
    };
    let mut rb = alloc::vec![0u8; CHUNK];
    let t2 = now_ticks();
    let mut off = 0;
    while off < BENCH_BYTES {
        match file2.read_at(off, &mut rb) {
            Ok(0) => break,
            Ok(n) => off += n,
            Err(e) => {
                println!("ext2 bench SKIPPED: read @{}: {:?}", off, e);
                return;
            }
        }
    }
    let read_ticks = now_ticks() - t2;

    report("ext2 bench", BENCH_BYTES, block_size, write_ticks, read_ticks, timebase_hz);
}

/// Sequential raw block-device throughput (no filesystem, no block cache).
/// Overwrites blocks on the device, so it runs after the ext2 phase.
fn virtio_bench(device: &alloc::sync::Arc<dyn BlockDevice>) {
    let timebase_hz = timer_hz();
    let bs = device.block_size();
    let nblocks = BENCH_BYTES / bs;
    let start = 2048;
    if start + nblocks > device.total_blocks() {
        println!("virtio bench SKIPPED: device too small");
        return;
    }

    let wbuf = alloc::vec![0xE5u8; bs];
    let t0 = now_ticks();
    for i in 0..nblocks {
        if device.write_block(start + i, &wbuf).is_err() {
            println!("virtio bench SKIPPED: write_block @{}", start + i);
            return;
        }
    }
    let write_ticks = now_ticks() - t0;

    let mut rbuf = alloc::vec![0u8; bs];
    let t2 = now_ticks();
    for i in 0..nblocks {
        if device.read_block(start + i, &mut rbuf).is_err() {
            println!("virtio bench SKIPPED: read_block @{}", start + i);
            return;
        }
    }
    let read_ticks = now_ticks() - t2;

    report("virtio raw bench", BENCH_BYTES, bs, write_ticks, read_ticks, timebase_hz);
}

fn report(name: &str, bytes: usize, block_size: usize, write_ticks: u64, read_ticks: u64, hz: u64) {
    let bytes = bytes as u64;
    let w_ms = write_ticks * 1000 / hz;
    let r_ms = read_ticks * 1000 / hz;
    let w_kbps = if write_ticks > 0 { bytes * hz / write_ticks / 1024 } else { 0 };
    let r_kbps = if read_ticks > 0 { bytes * hz / read_ticks / 1024 } else { 0 };
    println!("{}: {} KiB, block_size={}", name, bytes / 1024, block_size);
    println!("  write: {} ms  ({} KiB/s)", w_ms, w_kbps);
    println!("  read : {} ms  ({} KiB/s)", r_ms, r_kbps);
}
