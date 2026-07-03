# AGENTS.md

**Generated:** 2026-06-19 (updated 2026-07-02)
**Branch:** main

Koros is a `#![no_std]` Rust kernel targeting QEMU on four architectures via the [polyhal](https://github.com/Byte-OS/polyhal) HAL.

## Key Facts

- **Cargo workspace, two crates:**
  - `koros-core/` — library crate (`koros_core`) containing **all kernel subsystems**: `arch`, `boot`, `cmdline`, `drivers`, `fs`, `mm`, `trap`, plus the global allocator, the `println!`/`print!` macros, and the `#[panic_handler]`.
  - `koros/` — the binary crate. Contains **only `kernel_main`** (the entry) plus `build.rs` and the `linker.lds` template.
  - The boot code lives in `koros-core` and calls `kernel_main` through an `unsafe extern "C" { fn kernel_main() -> !; }` declaration resolved at link time against the `koros` binary's `#[no_mangle]` symbol.
- **Nightly Rust required** — pinned to `nightly-2026-06-19` in `rust-toolchain.toml`. Components: `rust-src`, `clippy`, `rustfmt`, `rust-analyzer`, `llvm-tools-preview`.
- **`build-std`** — `.cargo/config.toml` (workspace root) compiles `core`, `alloc`, `compiler_builtins` from source. No `std`.
- **`polyhal/` is an external symlink** — do not edit files there. It is a git submodule-style link to `../polyhal`.
- **No unit tests, no CI** — verification is running the kernel in QEMU (`koros_core::ext2_test` is the in-kernel functional self-check).

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
make build ARCH=<arch>   # runs `cargo build --release --target <t> -p koros` + objcopy
```

The binary is `koros`; artifacts are `target/<target>/release/koros` (ELF, used
directly by x86_64/loongarch64) and `koros.bin` (raw, used by riscv64/aarch64).

Clippy / check (no Makefile target, run directly; add `-p koros` to include the binary):
```
cargo clippy --target riscv64gc-unknown-none-elf --release
cargo clippy --target x86_64-unknown-none --release
cargo clippy --target aarch64-unknown-none-softfloat --release
cargo clippy --target loongarch64-unknown-none --release
```

## Linker Scripts

`koros/build.rs` generates `linker_<arch>.lds` at build time from the template `koros/linker.lds`, substituting `%ARCH%` and `%KERNEL_BASE%`. The generated file lands under `target/`. Do not edit the generated files; edit `koros/linker.lds` instead.

Kernel virtual base addresses (high-half kernel, all architectures):
| Arch | Base | Physical Load | KERNEL_OFFSET |
|------|------|---------------|---------------|
| riscv64 | `0xffffffc080200000` | `0x80200000` | `0xffffffc000000000` |
| x86_64 | `0xffff800000200000` | `0x200000` | `0xffff800000000000` |
| aarch64 | `0xffff000040080000` | `0x40080000` | `0xFFFF000000000000` |
| loongarch64 | `0x9000000080000000` | `0x80000000` | `0x9000000000000000` |

## Code Layout

```
koros/
  src/main.rs             — kernel_main ONLY (the binary entry)
  build.rs                — generates linker_<arch>.lds from linker.lds
  linker.lds              — linker script template
koros-core/               — library crate `koros_core` (all subsystems)
  src/lib.rs              — module decls, panic handler, ext2_test self-check,
                            `extern "C" { kernel_main }` declaration
  src/arch/mod.rs         — cfg_if dispatch to per-arch module
  src/arch/<arch>/boot.rs — rust_entry, calls mm::set_*_ptr (if needed), then kernel_main
  src/arch/<arch>/boot.S  — _start, BSS clear (physical), page tables, MMU, high-half jump
  src/arch/<arch>/mm.rs   — kernel_offset, phys<->virt, memory + cmdline detection
  src/arch/<arch>/page_table.rs — dynamic page mapping
  src/arch/<arch>/trap.rs / trap.S — trap init + handler / vector table
  src/boot/mod.rs         — shared boot stack (512 KiB BSS) only
  src/cmdline/mod.rs      — boot command-line storage + key=value/flag parsing
  src/drivers/uart/       — per-arch UART drivers, println!/print! macros
  src/drivers/block/      — BlockDevice trait + LRU write-back cache
  src/drivers/virtio.rs    — adapter over the `virtio-drivers` crate (MMIO + PCI) + virtio-mmio driver
  src/fs/                 — VFS (INode/SuperBlock, fd, mount, path) + ext2 + ramfs
  src/mm/                 — frame allocator, slab heap, page tables, FDT parse
  src/trap/mod.rs         — delegates init() to arch-specific trap::init()
```

Entry flow per arch: `_start` (asm, in `koros-core` `arch/<arch>/boot.S`) → **clear BSS at physical addresses** → set up page tables → enable MMU → jump to high-half VA → `rust_entry` (boot.rs) → `unsafe { crate::kernel_main() }` → (in `koros`) `trap::init()` → `mm::init()` (captures cmdline early) → `ext2_test()`.

## CODE MAP

| Symbol | Kind | Location | Role |
|--------|------|----------|------|
| `kernel_main` | fn | `koros/src/main.rs` | Kernel entry (binary crate) — init + self-check, spins |
| `kernel_main` (decl) | extern | `koros-core/src/lib.rs` | `extern "C"` declaration the boot code calls |
| `panic` | fn | `koros-core/src/lib.rs` | Panic handler — infinite loop |
| `ext2_test` | fn | `koros-core/src/lib.rs` | In-kernel ext2-on-virtio functional self-check |
| `rust_entry` | fn | per-arch `koros-core/src/arch/<arch>/boot.rs` | Early entry: set DTB/MBI ptr → `kernel_main` |
| `_start` | asm fn | per-arch `boot.S` / `boot.rs` | CPU entry point, page-table setup, mode switch |
| `clear_bss` | fn | `koros-core/src/boot/mod.rs` | (if present) Zeroes BSS via `_sbss`/`_ebss` |
| `_print` / `println!` / `print!` | fn/macro | `koros-core/src/drivers/uart/mod.rs` | UART output, exported as `koros_core::println!` |
| `cmdline::{raw,get,has_flag}` | fn | `koros-core/src/cmdline/mod.rs` | Boot command-line access |

## Arch-Specific Quirks

- **x86_64**: Uses Multiboot1 (magic `0x1BADB002`). Boot is a 32→64-bit transition in `multiboot.S`. Needs `-Clink-arg=-no-pie` (handled by Makefile). Uses `q35` machine and `IvyBridge-v2` CPU.
- **aarch64**: UART is PL011 at `0x09000000`; others are NS16550A. Uses `cortex-a72` CPU.
- **riscv64**: Simplest boot — just sets `sp` and calls Rust.
- **loongarch64**: QEMU gets `-m 1G` explicitly.

## Coding Rules

- `#![no_std]` only — use `core` and `alloc` (with provided allocator), never `std`.
- Minimize `unsafe` — wrap in safe abstractions, document why each block is needed.
- No magic numbers — use named constants for all hardware addresses, flags, bit masks.
- All kernel code lives in the `koros-core` crate (`koros-core/src/`); the `koros` binary crate holds only `kernel_main`.
- **`#[cfg(target_arch)]` must be confined to `koros-core/src/arch/` where possible.** Shared non-driver modules (`mm/`, `trap/`, `sched/`, etc.) must not contain `#[cfg(target_arch)]` except for a thin dispatch import (one `use` line per architecture) that re-exports a uniform interface from `arch/<arch>/`. **Drivers (`koros-core/src/drivers/`) are hardware-facing and may contain a modest amount of `#[cfg(target_arch)]`**, but should still prefer per-arch files under a subdirectory with thin dispatch (e.g. `drivers/uart/riscv64.rs`). Arch-specific constants, register definitions, and function bodies belong in per-arch files — never behind `#[cfg]` in shared code.
- **Arch-specific code belongs under `koros-core/src/arch/<arch>/`.** If a feature differs per architecture, implement it as a separate file in each arch directory, not as `#[cfg(target_arch)]` branches inside a shared module.
- Generic/shared modules (`mm/`, `trap/`, `sched/`, etc.) may use a thin `cfg_if` dispatch to pull in arch-specific submodules, but should avoid scattering `#[cfg]` conditionals inside function bodies. The goal is: open `arch/<arch>/` and see everything specific to that architecture; open a shared module and see architecture-neutral logic.
- **Every per-arch module must export a uniform interface.** All architectures that implement a module (e.g. `mm`, `trap`, `uart`) must export the same public functions and types with the same signatures, so shared code can call them through a thin cfg dispatch without architecture-specific branches.
- `println!` and `print!` are provided by `drivers::uart` (not `std`).
- For architecture-specific code bugs (boot, page tables, MMU, trap, context switch), refer to the `polyhal/` directory — it contains working implementations for all four architectures that can be used as reference.

## Code Types & Recommended Directory Structure

All paths are under the `koros-core/src/` crate root.

### Code Type Reference

| Code Type | Directory | arch-specific? | Status | Description |
|-----------|-----------|----------------|--------|-------------|
| Boot | `boot/` + `arch/<arch>/boot.rs` | Mixed | ✅ | Stack setup, BSS clear, MMU enable, CPU mode switch |
| HAL | `hal/` + `arch/<arch>/hal.rs` | Yes | — | Trait interfaces for CSR, registers, barriers, page tables |
| Trap/Interrupt | `trap/` + `arch/<arch>/trap.rs` | Mixed | ✅ (init) | Vector tables, trap dispatch, exception handlers |
| Memory Mgmt | `mm/` + `arch/<arch>/{mm,page_table}.rs` | Mixed | ✅ | Frame allocator, slab heap, page tables, FDT parse |
| Cmdline | `cmdline/` + `arch/<arch>/mm.rs` (source) | Mixed | ✅ | Boot command line (FDT bootargs / Multiboot cmdline) |
| Process/Thread | `process/` + `arch/<arch>/context.rs` | Mixed | — | PCB/TCB, PID alloc, process tree, context switch |
| Scheduler | `sched/` | No | — | FIFO, CFS, priority scheduling, idle thread |
| Sync Primitives | `sync/` | No | (uses `spin`) | Spinlock, mutex, semaphore, rwlock, per-CPU data |
| Time/Timer | `time/` + `drivers/timer/` | Mixed | — | Clock source, tick, high-res timers, sleep |
| Syscall | `syscall/` + `arch/<arch>/syscall.rs` | Mixed | — | Syscall entry (ecall/syscall/int 0x80), dispatch |
| Drivers | `drivers/<device>/` | Mostly no | ✅ block/uart/virtio | UART, block, virtio, net, GPU, PCIe, … |
| Filesystem | `fs/` | No | ✅ ext2 + ramfs | VFS (inode/file/mount/path), ext2, tmpfs |
| Networking | `net/` | No | — | Ethernet, IP, TCP/UDP, socket API |
| IPC | `ipc/` | No | — | Pipe, signal, shared memory, message queue |
| Userspace | `user/` | Mixed | — | ELF loader, copy_from/to_user, signal delivery |
| Platform/Bus | `platform/` | Mixed | (in mm/fdt) | FDT/DTB parse, ACPI, PCI/PCIe enumeration |
| Debug/Logging | `debug/` | Mixed | — | Log framework, backtrace, KASAN, perf counters |
| Utilities | `utils/` | No | — | Linked list, bitmap, B-tree, error types |

> **Key discipline for "Mixed" modules:** The shared module (e.g. `mm/mod.rs`) should use a thin dispatch to pull in arch-specific submodules, but avoid function-body `#[cfg]` branches. All arch-specific logic lives under `arch/<arch>/` — open that directory to see what belongs to a specific architecture.


