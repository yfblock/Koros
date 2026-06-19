# AGENTS.md

**Generated:** 2026-06-19
**Branch:** master

Koros is a `#![no_std]` Rust kernel targeting QEMU on four architectures via the [polyhal](https://github.com/Byte-OS/polyhal) HAL.

## Key Facts

- **Nightly Rust required** — pinned to `nightly-2026-06-19` in `rust-toolchain.toml`. Components: `rust-src`, `clippy`, `rustfmt`, `rust-analyzer`, `llvm-tools-preview`.
- **`build-std`** — `.cargo/config.toml` compiles `core`, `alloc`, `compiler_builtins` from source. No `std`.
- **`polyhal/` is an external symlink** — do not edit files there. It is a git submodule-style link to `../polyhal`.
- **No tests, no CI** — verification is running the kernel in QEMU.

## Build & Run

The only entry point is the Makefile. Always uses `--release`.

```
make ARCH=riscv64 run       # default, simplest
make ARCH=x86_64 run        # requires -Clink-arg=-no-pie (set automatically)
make ARCH=aarch64 run
make ARCH=loongarch64 run
make ARCH=riscv64 SMP=4 run # multi-core
make ARCH=riscv64 debug     # QEMU with -s -S for GDB attach
```

`ARCH` defaults to `riscv64` if omitted. Valid values: `riscv64`, `x86_64`, `aarch64`, `loongarch64`.

To build without running:
```
make build ARCH=<arch>
```

Clippy / check (no Makefile target, run directly):
```
cargo clippy --target riscv64gc-unknown-none-elf --release
cargo clippy --target x86_64-unknown-none --release
cargo clippy --target aarch64-unknown-none-softfloat --release
cargo clippy --target loongarch64-unknown-none --release
```

## Linker Scripts

`build.rs` generates `linker_<arch>.lds` at build time from the template `linker.lds`, substituting `%ARCH%` and `%KERNEL_BASE%`. The generated file lands in `target/<target>/release/`. Do not edit the generated files; edit `linker.lds` instead.

Kernel virtual base addresses (high-half kernel, all architectures):
| Arch | Base | Physical Load | KERNEL_OFFSET |
|------|------|---------------|---------------|
| riscv64 | `0xffffffc080200000` | `0x80200000` | `0xffffffc000000000` |
| x86_64 | `0xffff800000200000` | `0x200000` | `0xffff800000000000` |
| aarch64 | `0xffff000040080000` | `0x40080000` | `0xFFFF000000000000` |
| loongarch64 | `0x9000000080000000` | `0x80000000` | `0x9000000000000000` |

## Code Layout

```
src/
  main.rs              — kernel_main entry, panic handler
  arch/mod.rs           — cfg_if dispatch to per-arch module
  arch/<arch>/boot.rs   — rust_entry, calls mm::set_*_ptr (if needed), then kernel_main
  arch/<arch>/boot.S    — _start, BSS clear (physical), page tables, MMU, high-half jump
  arch/<arch>/trap.rs   — trap vector init + handle_trap
  arch/<arch>/trap.S    — assembly trampoline / vector table
  boot/mod.rs           — shared boot stack (512 KiB BSS) only
  drivers/uart/         — per-arch UART drivers (mod.rs + <device>.rs), println! macro
  mm/                   — physical memory management
  trap/mod.rs           — delegates init() to arch-specific trap::init()
```

Entry flow per arch: `_start` (asm, in `arch/<arch>/boot.S`) → **clear BSS at physical addresses** → set up page tables → enable MMU → jump to high-half VA → `rust_entry` (boot.rs) → `mm::init()` → `trap::init()` → `kernel_main()`.

## CODE MAP

| Symbol | Kind | Location | Role |
|--------|------|----------|------|
| `kernel_main` | fn | `src/main.rs:12` | Kernel entry — prints hello, spins |
| `panic` | fn | `src/main.rs:20` | Panic handler — infinite loop |
| `clear_bss` | fn | `src/boot/mod.rs:19` | Zeroes BSS via linker symbols `_sbss`/`_ebss` |
| `rust_entry` | fn | per-arch `boot.rs` | Arch-independent entry: calls `mm::set_*_ptr` (if needed) → `kernel_main` |
| `_start` | asm fn | per-arch `boot.S` / `boot.rs` | CPU entry point, page-table setup, mode switch |
| `putchar` | fn | `src/drivers/uart.rs:31` | Blocking UART byte write (NS16550A / PL011) |
| `puts` | fn | `src/drivers/uart.rs:52` | Writes string + `\r` before `\n` |
| `_print` | fn | `src/drivers/uart.rs:70` | fmt::Write adapter for UART |
| `println!` / `print!` | macro | `src/drivers/uart.rs:76` | Public print macros (no-std) |

## Arch-Specific Quirks

- **x86_64**: Uses Multiboot1 (magic `0x1BADB002`). Boot is a 32→64-bit transition in `multiboot.S`. Needs `-Clink-arg=-no-pie` (handled by Makefile). Uses `q35` machine and `IvyBridge-v2` CPU.
- **aarch64**: UART is PL011 at `0x09000000`; others are NS16550A. Uses `cortex-a72` CPU.
- **riscv64**: Simplest boot — just sets `sp` and calls Rust.
- **loongarch64**: QEMU gets `-m 1G` explicitly.

## Coding Rules

- `#![no_std]` only — use `core` and `alloc` (with provided allocator), never `std`.
- Minimize `unsafe` — wrap in safe abstractions, document why each block is needed.
- No magic numbers — use named constants for all hardware addresses, flags, bit masks.
- **`#[cfg(target_arch)]` must be confined to `src/arch/` where possible.** Shared non-driver modules (`mm/`, `trap/`, `sched/`, etc.) must not contain `#[cfg(target_arch)]` except for a thin dispatch import (one `use` line per architecture) that re-exports a uniform interface from `src/arch/<arch>/`. **Drivers (`src/drivers/`) are hardware-facing and may contain a modest amount of `#[cfg(target_arch)]`**, but should still prefer per-arch files under a subdirectory with thin dispatch (e.g. `drivers/uart/riscv64.rs`). Arch-specific constants, register definitions, and function bodies belong in per-arch files — never behind `#[cfg]` in shared code.
- **Arch-specific code belongs under `src/arch/<arch>/`.** If a feature differs per architecture, implement it as a separate file in each arch directory, not as `#[cfg(target_arch)]` branches inside a shared module.
- Generic/shared modules (`mm/`, `trap/`, `sched/`, etc.) may use a thin `cfg_if` dispatch to pull in arch-specific submodules, but should avoid scattering `#[cfg]` conditionals inside function bodies. The goal is: open `src/arch/<arch>/` and see everything specific to that architecture; open a shared module and see architecture-neutral logic.
- **Every per-arch module must export a uniform interface.** All architectures that implement a module (e.g. `mm`, `trap`, `uart`) must export the same public functions and types with the same signatures, so shared code can call them through a thin cfg dispatch without architecture-specific branches.
- `println!` and `print!` are provided by `drivers::uart` (not `std`).
- For architecture-specific code bugs (boot, page tables, MMU, trap, context switch), refer to the `polyhal/` directory — it contains working implementations for all four architectures that can be used as reference.

## Code Types & Recommended Directory Structure

### Code Type Reference

| Code Type | Directory | arch-specific? | Description |
|-----------|-----------|----------------|-------------|
| Boot | `src/boot/` + `src/arch/<arch>/boot.rs` | Mixed | Stack setup, BSS clear, MMU enable, CPU mode switch |
| HAL | `src/hal/` + `src/arch/<arch>/hal.rs` | Yes | Trait interfaces for CSR, registers, barriers, page tables |
| Trap/Interrupt | `src/trap/` + `src/arch/<arch>/trap.rs` | Mixed | Vector tables, trap dispatch, exception handlers, interrupt controller drivers |
| Memory Mgmt | `src/mm/` + `src/arch/<arch>/page_table.rs` | Mixed | Physical allocator, virtual memory, kernel heap, VmArea |
| Process/Thread | `src/process/` + `src/arch/<arch>/context.rs` | Mixed | PCB/TCB, PID alloc, process tree, context switch |
| Scheduler | `src/sched/` | No | FIFO, CFS, priority scheduling, load balancing, idle thread |
| Sync Primitives | `src/sync/` | No | Spinlock, mutex, semaphore, rwlock, RCU, per-CPU data |
| Time/Timer | `src/time/` + `src/drivers/timer/` | Mixed | Clock source, tick, high-res timers, sleep |
| Syscall | `src/syscall/` + `src/arch/<arch>/syscall.rs` | Mixed | Syscall entry (ecall/syscall/int 0x80), arg parsing, dispatch table |
| Drivers | `src/drivers/<device>/` | Mostly no | UART, block, net, GPU, input, PCIe, GPIO, I2C, SPI |
| Filesystem | `src/fs/` | No | VFS (inode/dentry/file), ext4, FAT, tmpfs, procfs, devfs |
| Networking | `src/net/` | No | Ethernet, IP, TCP/UDP, socket API, skbuff |
| IPC | `src/ipc/` | No | Pipe, signal, shared memory, message queue, eventfd |
| Userspace | `src/user/` | Mixed | ELF loader, copy_from/to_user, signal delivery |
| Platform/Bus | `src/platform/` | Mixed | FDT/DTB parse, ACPI, PCI/PCIe enumeration |
| Debug/Logging | `src/debug/` | Mixed | Log framework, backtrace, KASAN, perf counters |
| Utilities | `src/utils/` | No | Linked list, bitmap, B-tree, error types |

> **Key discipline for "Mixed" modules:** The shared module (e.g. `src/mm/mod.rs`) should use a thin dispatch to pull in arch-specific submodules, but avoid function-body `#[cfg]` branches. All arch-specific logic lives under `src/arch/<arch>/` — open that directory to see what belongs to a specific architecture.


