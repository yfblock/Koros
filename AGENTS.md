# AGENTS.md

**Generated:** 2026-06-19 (rewritten 2026-07-07)
**Branch:** main

Koros is a `#![no_std]` Rust kernel targeting QEMU on four architectures (riscv64, x86_64, aarch64, loongarch64). It is structured as a **seven-crate Cargo workspace** of small, composable libraries plus one binary composition crate. Architecture abstraction is **runtime trait-object dispatch** (`kor::ArchProvider`), not `#[cfg(target_arch)]` branches in shared code.

## Key Facts

- **Cargo workspace, seven crates** (`Cargo.toml`, `resolver = "2"`):
  - `kor/` — library crate `kor`. The **bottom generic-abstraction layer**: traits + registries, no arch code, no subsystem implementations. Defines `ArchProvider`, `InterruptController`, `Console`, `TrapCallbacks`, `BlockDevice`, the VFS traits, plus boot-time helpers (FDT parse, region collection, cmdline, config, SMP/time bookkeeping).
  - `kor-frame/` — library crate `kor_frame`. Physical frame allocator (buddy system via `buddy_system_allocator`). Global free-function API + an instance type.
  - `kor-alloc/` — library crate `kor_alloc`. Slab-based kernel heap (`LockedSlabHeap`, used as `#[global_allocator]`).
  - `kor-fs/` — library crate `kor_fs`. VFS layer + **ext2** (read/write, symlink, hard link, truncate, rename, indirect blocks, xattr, mknod) + **ramfs** + block cache + mount table + path resolver + fd abstraction. Re-exports VFS traits from `kor`.
  - `kor-sched/` — library crate `kor_sched`. **Preemptive multi-core kernel-thread scheduler** + blocking sync primitives (`WaitQueue`, `Semaphore`, `Mutex<T>`, `Channel<T>`).
  - `kor-arch/` — library crate `kor_arch`. **All architecture-specific code**: per-arch `boot`/`trap`/`mm`/`page_table`/`smp`/`time`/`irq`/`context`/`ic`/`console`/`provider` modules under `src/<arch>/`, plus the shared UART drivers. Provides concrete `ArchProvider` impls and `provider()`/`interrupt_controller()`/`console()` selectors.
  - `koros/` — the **binary crate** (`[[bin]] name = "koros"`). The composition root: `kernel_main` init sequence, the `#[global_allocator]`, `#[panic_handler]`, the virtio driver adapter, device/IRQ/block registries, the ext2 self-check, the storage benchmark, and the scheduler demo tasks. Holds `build.rs` + `linker.lds`.
- **Boot → entry resolution:** the boot assembly lives in `kor-arch/src/<arch>/boot*.S` and calls `rust_entry` → `kernel_main` through an `unsafe extern "C" { fn kernel_main() -> !; }` declaration in `kor-arch/src/lib.rs`, resolved at link time against the `koros` binary's `#[no_mangle]` symbol. Secondary CPUs enter via `secondary_entry` (also declared extern in `kor-arch`, defined in `koros`).
- **Nightly Rust required** — pinned to `nightly-2026-06-19` in `rust-toolchain.toml`. Components: `rust-src`, `clippy`, `rustfmt`, `rust-analyzer`, `llvm-tools-preview`. Targets pre-installed: `riscv64gc-unknown-none-elf`, `x86_64-unknown-none`, `aarch64-unknown-none-softfloat`, `loongarch64-unknown-none`.
- **`build-std`** — `.cargo/config.toml` (workspace root) compiles `core`, `alloc`, `compiler_builtins` from source with `compiler-builtins-mem`. No `std`. (No rustflags here; x86_64's `-Clink-arg=-no-pie` is set by the Makefile via `RUSTFLAGS`.)
- **`polyhal/` is an external symlink** (`polyhal ⇒ ../polyhal`) — do not edit files there. It is reference material only; the kernel does not link against it. For architecture-specific code bugs (boot, page tables, MMU, trap, context switch), consult `polyhal/` for working implementations on all four architectures.
- **No unit tests, no CI** — verification is running the kernel in QEMU (`koros/src/ext2_test.rs` is the in-kernel functional self-check) and, optionally, Verus formal verification of the allocator modules via `./scripts/verify.sh` (operates on `verified/*.rs`).

## Build & Run

The only entry point is the Makefile. Always uses `--release`.

```
make ARCH=riscv64 run       # default, simplest
make ARCH=x86_64 run        # requires -Clink-arg=-no-pie (set automatically)
make ARCH=aarch64 run
make ARCH=loongarch64 run
make ARCH=riscv64 SMP=4 run # multi-core
make ARCH=riscv64 debug     # QEMU with -s -S for GDB attach
make ARCH=riscv64 CMDLINE="bench loglevel=7" run   # set kernel cmdline
```

`ARCH` defaults to `riscv64` if omitted. Valid values: `riscv64`, `x86_64`, `aarch64`, `loongarch64`.

To build without running:
```
make build ARCH=<arch>   # runs `cargo build --release --target <t> -p koros` + objcopy
```

The binary is `koros`; artifacts are `target/<target>/release/koros` (ELF, used directly by x86_64/loongarch64) and `koros.bin` (raw, used by riscv64/aarch64, produced by `rust-objcopy --strip-all -O binary`).

The ext2 self-check needs a backing disk. The Makefile auto-creates `os-images/ext2-test.img` (4 MiB, `mkfs.ext2 -b 1024`) on first run and attaches it as a virtio-blk device — virtio-mmio on riscv64/aarch64 `virt`, virtio-blk-pci on x86_64/loongarch64. **Caveat:** the test image persists between runs and the self-check creates `test_dir` without cleaning up, so a second run reports `ext2 test FAILED: AlreadyExists`. Delete `os-images/ext2-test.img` (or pass `EXT2_IMG=<fresh path>`) to get a clean `ext2 test passed`.

Clippy / check (no Makefile target, run directly; add `-p koros` to include the binary):
```
cargo clippy --target riscv64gc-unknown-none-elf --release
cargo clippy --target x86_64-unknown-none --release
cargo clippy --target aarch64-unknown-none-softfloat --release
cargo clippy --target loongarch64-unknown-none --release
```

## Linker Scripts

`koros/build.rs` generates `linker_<arch>.lds` at build time from the template `koros/linker.lds`, substituting `%ARCH%` (e.g. `riscv`, `i386:x86-64`) and `%KERNEL_BASE%`. The generated file lands in the target directory (5 levels up from `OUT_DIR`); `build.rs` emits `cargo:rustc-link-arg=-T<path>` so the linker picks it up. Do not edit the generated files; edit `koros/linker.lds` instead. Notable linker symbols: `_skernel`/`_end` (kernel image bounds, used by `kor::kernel_phys_range()` to clip the frame allocator), `stext`/`etext`, `_sdata`/`_edata`, `_sbss`/`_ebss`, `_load_end`, and `.bss.bstack` (the 512 KiB boot stack, `bstack_top`).

Kernel virtual base addresses (high-half kernel; loongarch64 is the exception — direct-map, offset 0):

| Arch | Link/load VA (`KERNEL_BASE`) | Physical Load | `kernel_offset()` (direct-map base) |
|------|------------------------------|---------------|-------------------------------------|
| riscv64 | `0xffffffc080200000` | `0x80200000` | `0xffffffc000000000` |
| x86_64 | `0xffff800000200000` | `0x200000` | `0xffff800000000000` |
| aarch64 | `0xffff000040080000` | `0x40080000` | `0xffff000000000000` |
| loongarch64 | `0x9000000080000000` | `0x9000000080000000` | `0` (no paging yet — VA == PA via DMW) |

## Architecture Abstraction Model

This is the central design decision and differs from the old `koros-core` layout. **Shared crates contain no `#[cfg(target_arch)]`** (with one narrow exception: `kor-arch`'s own `lib.rs` selector). Instead:

- `kor` defines the `ArchProvider` trait (and `InterruptController`, `Console`, `TrapCallbacks`). These are registered once at boot into `spin::Once`-backed globals: `kor::arch::install` / `kor::arch::current()`, `kor::install_controller` / `kor::controller()`, `kor::install_console`, `kor::install_callbacks`.
- `kor-arch` provides one concrete `impl ArchProvider` per architecture (`Riscv64Provider`, `X86_64Provider`, `Aarch64Provider`, `Loongarch64Provider`), each a zero-sized struct behind a `pub static PROVIDER`. `kor-arch/src/lib.rs` uses a single `cfg_if!` to pick the active arch module and expose `provider()`, `interrupt_controller()`, `console()`, `TEST_VA_4K`, `TEST_VA_2M`.
- `koros::kernel_main` installs them: `kor::install_console(kor_arch::console())`, `kor::install(kor_arch::provider())`, `kor::install_controller(kor_arch::interrupt_controller())`.
- All downstream code calls `kor::arch::current()` (a `&'static dyn ArchProvider`) — never cfg-selects a module. This keeps `kor`, `kor-frame`, `kor-alloc`, `kor-fs`, `kor-sched` fully arch-neutral.

`kor::TaskContext` is an opaque `#[repr(C, align(16))]` buffer of 16 `usize` (sized for the largest arch — riscv64 uses 14 callee-saved slots, aarch64 13, loongarch64 12, x86_64 1). Each arch's `context.rs` casts it to its internal layout (size-checked with a `const _: () = assert!(...)`).

## Crate Layout

```
kor/
  src/lib.rs              — module decls, re-exports, kernel_phys_range()
  src/arch.rs             — ArchProvider trait, TaskContext, PROVIDER registry
  src/addr.rs             — PhysAddr/VirtAddr newtypes
  src/interrupt.rs        — InterruptController trait + registry
  src/trap_callbacks.rs   — TrapCallbacks trait + registry; on_timer/dispatch_external
  src/console.rs          — Console trait + registry; print!/println! macros
  src/irq.rs              — local IRQ enable/disable/without wrappers
  src/time.rs             — TICK_HZ, tick counter
  src/smp.rs              — MAX_CPUS, online tracking, boot_secondaries
  src/cmdline.rs          — boot cmdline storage + key=value/flag parsing
  src/config.rs           — PlatformConfig (console, firmware_phys_start, dtb, PciEcam)
  src/mapping.rs          — MappingFlags, MapSize, MapError
  src/block.rs            — BlockDevice trait + BlockError
  src/vfs.rs              — FileType, Metadata, FsInfo, FsError, SuperBlock, INode traits
  src/regions.rs          — RegionCollector + clip_region (boot memory-region splitter)
  src/fdt.rs              — FDT parse: memory regions, cpu_count, bootargs
  src/driver.rs           — DeviceDriver trait, DtDevice, probe_fdt
  src/boot_stack.rs       — global_asm! 512 KiB boot stack (bstack/bstack_top)
kor-frame/src/lib.rs      — FrameAllocator (buddy) + global free fns
kor-alloc/src/
  lib.rs                  — re-exports LockedSlabHeap/SlabHeap
  slab_heap.rs            — 7 size-class slab heap + frame-backed large allocs
kor-fs/src/
  lib.rs                  — re-exports VFS/block types from kor
  block_cache.rs          — LRU write-back BlockCache
  fd.rs                   — FileDescriptor, OpenFlags, SeekFrom
  mount.rs                — MountTable + mount/unmount/resolve/sync_all
  path.rs                 — Path, resolve_path (symlink-following walker)
  ext2/{mod,bitmap,block_group,dir,inode,super_block,xattr}.rs
  ramfs/mod.rs            — in-memory reference filesystem
kor-sched/src/
  lib.rs                  — Task/PerCpu, scheduler core, WaitQueue/Semaphore/Mutex2
  sync.rs                 — guard-based Mutex<T>, unbounded Channel<T>
kor-arch/src/
  lib.rs                  — cfg_if arch selector; provider()/interrupt_controller()/console()
  uart/{mod,ns16550a,pl011}.rs
  <arch>/{mod,boot,trap,mm,page_table,smp,time,irq,context,ic,console,provider}.rs
    + riscv64: boot.S, trap.S, plic.rs
    + x86_64:  multiboot.S, trap.S, ap_boot.S  (no boot.S, no plic/gic)
    + aarch64: boot.S, trap.S, gic.rs
    + loongarch64: trap.S only (boot is inline naked_asm! in boot.rs; no paging, no plic/gic)
koros/
  src/main.rs             — kernel_main (composition root) + demo tasks
  src/ext2_test.rs        — in-kernel ext2 functional self-check
  src/bench.rs            — storage benchmark (8 MiB ext2 + raw virtio)
  src/heap.rs             — bootstrap slab heap init + self_test
  src/panic.rs            — #[panic_handler]
  src/registries.rs       — BLOCKS / IRQS / MOUNTS singletons
  src/virtio.rs           — virtio-drivers adapter (KorosHal, VdBlk, MMIO + PCI + PCI-ECAM)
  build.rs                — generates linker_<arch>.lds from linker.lds
  linker.lds              — linker script template
```

## Entry Flow

Per arch (assembly in `kor-arch/src/<arch>/`): `_start` → **clear BSS at physical addresses** (x86_64 excepted — Multiboot zeroes BSS via `bss_end_addr`) → build boot page tables → enable MMU → jump to high-half VA (loongarch64 skips MMU) → `rust_entry` (boot.rs) → `unsafe { kernel_main() }` (in `koros`).

`kernel_main` init sequence (`koros/src/main.rs`):
1. Console — `kor::install_console(kor_arch::console())`.
2. Platform config — `config::init(platform_config())` (per-arch console type, `firmware_phys_start`, dtb, optional `PciEcam`).
3. Arch provider — `kor::install(kor_arch::provider())`.
4. Interrupt controller — `install_controller` + `ic.init()` (GICv2 on aarch64, PLIC on riscv64, polling stubs on x86_64/loongarch64).
5. Trap callbacks — `install_callbacks(&TRAP_CB)`; `on_timer` does `increment_tick` + `arch.handle_tick` + `kor_sched::timer_tick` + `kor_sched::preempt`; `on_external(irq)` dispatches `registries::IRQS`.
6. Bootstrap heap (`heap::init_bootstrap` on a 0xE000-byte static region).
7. Trap vectors — `arch.trap_init()`.
8. Memory init — capture cmdline, `RegionCollector`, `detect_memory_regions`, clip `kernel_phys_range()` + firmware hole, feed `kor_frame::add_region`, `heap::self_test()`, `arch.page_table_init()`, `page_table_self_test()` (4K + 2M map/translate round-trip).
9. Timer + IRQ on this CPU; print `Hello, world!` and cmdline.
10. `probe_devices()` (FDT virtio-mmio + optional PCI-ECAM on non-x86; PCI port-I/O on x86_64).
11. `kor::smp::boot_secondaries()`.
12. (riscv64/x86_64) if `cmdline::has_flag("bench")` → `bench::run()` then spin.
13. `ext2_test::ext2_test()`.
14. `kor_sched::init()`; spawn demo tasks (A/B/C/D, producer/consumer via `Channel`, blkio); `kor_sched::idle_loop()`.

Secondary CPUs: `secondary_entry(cpu_id)` → `trap_init` → `register_online` → per-CPU `ic.init()` → `timer_init` + `irq_enable` → `wait_for_interrupt` until `kor_sched::is_ready()` → `init_this_cpu()` → `idle_loop()`.

## CODE MAP

| Symbol | Kind | Location | Role |
|--------|------|----------|------|
| `kernel_main` | fn | `koros/src/main.rs` | Kernel entry (binary crate) — full init + self-check + scheduler demo |
| `kernel_main` (decl) | extern | `kor-arch/src/lib.rs` | `extern "C"` declaration the boot code calls |
| `secondary_entry` | fn | `koros/src/main.rs` | Per-CPU secondary entry (declared extern in `kor-arch`) |
| `rust_entry` | fn | per-arch `kor-arch/src/<arch>/boot.rs` | Early entry: set DTB/MBI ptr → `kernel_main` |
| `_start` / `_secondary_start` | asm fn | per-arch `kor-arch/src/<arch>/boot*.S` | CPU entry, page-table setup, MMU, high-half jump; secondary stub |
| `panic` | fn | `koros/src/panic.rs` | Panic handler — prints + infinite loop |
| `ext2_test` | fn | `koros/src/ext2_test.rs` | In-kernel ext2-on-virtio functional self-check |
| `ArchProvider` | trait | `kor/src/arch.rs` | Uniform arch abstraction (MM, traps, IRQ, timer, SMP, context switch) |
| `provider` / `interrupt_controller` / `console` | fn | `kor-arch/src/lib.rs` | Per-arch singletons handed to `kor` registries |
| `print!` / `println!` | macro | `kor/src/console.rs` | UART output via the `Console` registry (exported as `kor::println!`) |
| `LockedSlabHeap` | struct | `kor-alloc/src/slab_heap.rs` | `#[global_allocator]` (static in `koros/src/main.rs`) |
| `FrameAllocator` / `alloc_page` / `alloc_huge_2m` | struct/fn | `kor-frame/src/lib.rs` | Buddy physical frame allocator |
| `Task` / `spawn` / `yield_now` / `sleep_ms` / `idle_loop` | struct/fn | `kor-sched/src/lib.rs` | Preemptive multi-core scheduler |
| `WaitQueue` / `Semaphore` / `Mutex<T>` / `Channel<T>` | struct | `kor-sched/src/{lib,sync}.rs` | Blocking sync primitives |
| `Ext2Fs` / `Ext2INode` | struct | `kor-fs/src/ext2/` | ext2 filesystem + full `INode` impl |
| `VdBlk` / `KorosHal` | struct | `koros/src/virtio.rs` | virtio-blk adapter (IRQ-driven + polling) over `virtio-drivers` |
| `BLOCKS` / `IRQS` / `MOUNTS` | static | `koros/src/registries.rs` | Composition-owned device/IRQ/mount registries |

## Scheduler (`kor-sched`)

- **Model:** preemptive, multi-core, blocking kernel-thread scheduler. Tasks are `Arc<Task>` with a saved `TaskContext`, a 64 KiB private stack, an entry `fn()`, an atomic `state` (READY/RUNNING/SLEEPING/EXITED/BLOCKED), and a `wake_tick`.
- **Multi-core:** per-CPU state in `static CPUS: [PerCpu; MAX_CPUS]` (own `current`/`idle`/`prev`/`slice`); the ready/sleeper/zombie queues are global `Mutex`-protected statics, so tasks migrate freely between CPUs.
- **Preemption:** `timer_tick()` (called from the arch timer trap) decrements this CPU's `slice`; `preempt()` calls `yield_now()` when `slice == 0` (`TIME_SLICE = 5` ticks ≈ 50 ms at `TICK_HZ = 100`). Scheduler critical sections run with interrupts disabled.
- **Context switch:** `schedule(prev_action, wait_ptr)` picks the next task, records a deferred transition for the outgoing task, switches via `arch.context_switch`, then `finish_switch()` applies the transition (TO_READY/TO_SLEEP/TO_ZOMBIE/TO_WAIT) **after** the switch completes — so a task is never made runnable until its context is fully saved. For blocking, the `WaitQueue` raw lock is held across the switch and released by `finish_switch`, closing lost-wakeup and migration races.
- **Blocking sync:** `WaitQueue`, `Semaphore`, `Mutex2`, `sync::Mutex<T>` (guard-based), `sync::Channel<T>` (unbounded MPMC) all *block* (schedule away), not spin. Use them only from task context after the scheduler is running; use `spin::Mutex` from interrupt handlers or early boot. (No `RwLock` exists.)

## Arch-Specific Quirks

- **riscv64**: Sv39 paging; OpenSBI/SBI ecalls (HSM for secondaries, TIME for timer); PLIC at `0x0c00_0000`; NS16550A MMIO at `0x1000_0000`. Simplest boot. `_secondary_start` entered via SBI `hart_start`.
- **x86_64**: Multiboot1 (magic `0x1BADB002`); 4-level paging with 2 MiB huge pages; LAPIC timer (calibrated against PIT); APs via INIT-SIPI-SIPI trampoline copied to `0x8000`; NS16550A port I/O at `0x3F8`. `q35` machine, `IvyBridge-v2` CPU. IC is a polling stub (no real interrupt controller). Needs `-Clink-arg=-no-pie` (Makefile sets `RUSTFLAGS`).
- **aarch64**: EL1, 4-level paging (1 GiB blocks at boot); EL1 physical generic timer via GICv2 PPI; PSCI `CPU_ON` (HVC) for secondaries; GICv2 distributor `0x0800_0000` / CPU interface `0x0801_0000`; PL011 MMIO at `0x0900_0000`. `cortex-a72` CPU.
- **loongarch64**: **No paging yet** (direct-map, `kernel_offset() == 0`, VA == PA via DMW); boot entry is inline `naked_asm!` (no `boot.S`); constant timer; IPI+IOCSR-mailbox secondary bring-up; NS16550A MMIO at `0x1FE0_01E0`; PCI ECAM at `0x2000_0000`. IC and dynamic page-table are stubs (`dynamic_maps_supported() == false`, `TEST_VA_* == 0`). QEMU gets `-m 1G` explicitly.

## Coding Rules

- `#![no_std]` only — use `core` and `alloc` (with provided allocator), never `std`.
- Minimize `unsafe` — wrap in safe abstractions, document why each block is needed.
- No magic numbers — use named constants for all hardware addresses, flags, bit masks.
- **Architecture abstraction is runtime trait-object dispatch, not `#[cfg]`.** Shared crates (`kor`, `kor-frame`, `kor-alloc`, `kor-fs`, `kor-sched`) must contain **no `#[cfg(target_arch)]`**. Call `kor::arch::current()` (a `&'static dyn ArchProvider`) instead. The only permitted cfg is `kor-arch/src/lib.rs`'s single `cfg_if!` arch selector.
- **All arch-specific code lives under `kor-arch/src/<arch>/`.** If a feature differs per architecture, implement it as a separate file in each arch directory (each exporting the same `ArchProvider` methods), not as `#[cfg]` branches in shared code. Open `kor-arch/src/<arch>/` to see everything specific to that architecture; open a shared crate and see architecture-neutral logic.
- **Every per-arch module must satisfy the uniform `ArchProvider` interface.** All four architectures implement the same `ArchProvider` trait with the same method signatures, so shared code calls through `kor::arch::current()` without architecture-specific branches.
- `println!` and `print!` are provided by `kor::console` (via the `Console` registry), not `std`.
- For architecture-specific code bugs (boot, page tables, MMU, trap, context switch), refer to the `polyhal/` directory — it contains working implementations for all four architectures that can be used as reference.
- Registries (`kor::arch`, `kor::interrupt`, `kor::console`, `kor::trap_callbacks`, `kor::config`, `kor::cmdline`) are installed once at boot via `spin::Once`. Composition-owned registries (`BLOCKS`, `IRQS`, `MOUNTS`) live in `koros/src/registries.rs`.

## Verification

- **Functional:** `make ARCH=<arch> run` boots the kernel, runs the ext2 self-check, then the scheduler demo. All four architectures boot to a completed scheduler demo on a fresh ext2 image.
- **Formal:** `./scripts/verify.sh` runs Verus on `verified/{frame_allocator,slab_heap}.rs` (standalone, Verus-annotated copies of the allocator logic). Set `VERUS` env var or install Verus to `/tmp/verus-release/`. Pass a module name to verify one file: `./scripts/verify.sh frame_allocator`.
