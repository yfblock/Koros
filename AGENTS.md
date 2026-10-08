# AGENTS.md

Koros is a `#![no_std]` Rust kernel targeting QEMU on four architectures (riscv64, x86_64, aarch64, loongarch64). The workspace is laid out in **four tiers**: `kor/` (foundation crate), `base/` (core kernel services), `filesystems/` (the storage stack), `apps/` (composition roots). It is a **twelve-crate Cargo workspace** of small, composable libraries plus two binary composition crates (`koros` kernel, `korhv` hypervisor). Architecture abstraction is **runtime trait-object dispatch** (`kor::ArchProvider`), not `#[cfg(target_arch)]` branches in shared code — see [Architecture Abstraction Model](#architecture-abstraction-model).

## Key Facts

- **Twelve crates** (`Cargo.toml`, `resolver = "2"`):
  - `kor/` (`kor`) — bottom abstraction layer: traits + registries, no arch code, no subsystem impls — **one exception: `kor::frame`**, the allocation-free physical frame allocator (every boot feeds it via `regions`; it has no dependencies of its own). Also defines `ArchProvider`, `InterruptController`, `Console`, `TrapCallbacks`, `BlockDevice`, the VFS traits, plus boot-time helpers (FDT, regions, cmdline, config, SMP/time).
  - `base/kor-alloc/` (`kor_alloc`) — slab kernel heap (`LockedSlabHeap`, the `#[global_allocator]`).
  - `filesystems/kor-fs/` (`kor_fs`) — VFS infrastructure: block cache, mount table, path resolver, fd, and the **filesystem-driver registry**. No concrete filesystems.
  - `filesystems/kor-ext2/` (`kor_ext2`) — ext2 (read/write, symlink, hard link, truncate, rename, indirect blocks, xattr, mknod) + `Ext2Driver` singleton.
  - `filesystems/kor-ramfs/` (`kor_ramfs`) — in-memory reference fs + `RamFsDriver` singleton.
  - `base/kor-sched/` (`kor_sched`) — preemptive multi-core kernel-thread scheduler + blocking sync primitives.
  - `base/kor-arch/` (`kor_arch`) — architecture-specific code **except page tables**; concrete `ArchProvider` impls + `provider()`/`interrupt_controller()`/`console()` selectors.
  - `base/kor-virtio/` (`kor_virtio`) — virtio adapter for apps/koros/korix (block + net, MMIO / PCI / PCI-ECAM).
  - `apps/koros/` (`[[bin]] koros`) — kernel composition root: `kernel_main` init, `#[global_allocator]`, `#[panic_handler]`, virtio adapter, registries, ext2 self-check, bench, scheduler demo.
  - `apps/korix/` (`[[bin]] korix`) — Linux-compatible composition root: thin syscall layer (`syscall/arch/` ABI adapters + portable dispatch), processes/signals, user address spaces, ext2 rootfs, smoltcp networking.
  - `apps/korhv/` (`[[bin]] korhv`) — Type-1 hypervisor, parallel composition root: reuses `kor`/`kor-alloc` and enables hardware virtualization instead of fs/sched/timer. x86_64 = AMD SVM (VMRUN/#VMEXIT, NPT stage-2, VMMCALL); aarch64 = EL2 (ERET to EL1 guest, HVC). Run with `make ARCH=x86_64 BIN=korhv run` or `make ARCH=aarch64 BIN=korhv run`.
- **Boot → entry:** boot assembly in `base/kor-arch/src/<arch>/boot*.S` calls `rust_entry` → `kernel_main` via an `unsafe extern "C" { fn kernel_main() -> !; }` in `base/kor-arch/src/lib.rs`, linked against the `koros` `#[no_mangle]` symbol. Secondary CPUs enter via `secondary_entry` (declared extern in `kor-arch`, defined in `koros`).
- **Nightly Rust** — pinned `nightly-2026-06-19` (`rust-toolchain.toml`). `build-std` in `.cargo/config.toml` compiles `core`/`alloc`/`compiler_builtins` from source with `compiler-builtins-mem`. No `std`. (x86_64's `-Clink-arg=-no-pie` is set by the Makefile via `RUSTFLAGS`, not here.)
- **`base/kor-mmu/` (`kor-mmu`)** — **owns the page tables end to end** (CortenMM, SOSP '25): per-table-page descriptors (lock + per-PTE metadata), a transactional cursor (`AddrSpace::lock(range) → RCursor{query,map,mark,unmap}`), per-arch back-ends under `src/arch/` that supply PTE flags/encoders as **inherent associated items of `AddrSpace`** (compile-time dispatch, no trait objects), plus active-root switching, fork cloning, boot identity splits. `ArchProvider` no longer has page-table methods — composition crates call the `kor_mmu::map/translate/unmap/…` facade directly.
- **No unit tests, no CI** — verification is running in QEMU (`apps/koros/src/ext2_test.rs` is the in-kernel self-check) plus optional Verus formal verification via `./tools/scripts/verify.sh`.

## Build & Run

The only entry point is the Makefile. Always `--release`.

```
make ARCH=<riscv64|x86_64|aarch64|loongarch64> run   # riscv64 default; x86_64 sets -no-pie
make ARCH=riscv64 SMP=4 run   # multi-core
make ARCH=riscv64 debug       # QEMU with -s -S for GDB attach
make ARCH=riscv64 CMDLINE="bench loglevel=7" run   # kernel cmdline
make build ARCH=<arch>        # cargo build --release --target <t> -p koros + objcopy
```

`ARCH` defaults to `riscv64`. Artifacts: `target/<target>/release/koros` (ELF, x86_64/loongarch64) and `koros.bin` (raw, riscv64/aarch64, via `rust-objcopy --strip-all -O binary`).

The ext2 self-check needs a backing disk. The Makefile auto-creates `tools/os-images/ext2-test.img` (4 MiB, `mkfs.ext2 -b 1024`) and attaches it as virtio-blk — virtio-mmio on riscv64/aarch64 `virt`, virtio-blk-pci on x86_64/loongarch64. **Caveat:** the image persists and the self-check creates `test_dir` without cleanup, so a second run reports `ext2 test FAILED: AlreadyExists`. Delete `tools/os-images/ext2-test.img` (or pass `EXT2_IMG=<fresh path>`) for a clean pass.

Clippy (no Makefile target; add `-p koros` to include the binary) — target is one of `riscv64gc-unknown-none-elf`, `x86_64-unknown-none`, `aarch64-unknown-none-softfloat`, `loongarch64-unknown-none`:
```
cargo clippy --target <target> --release
```

## Linker Scripts

`apps/koros/build.rs` generates `linker_<arch>.lds` from `tools/lds/koros.lds`, substituting `%ARCH%` and `%KERNEL_BASE%`. The generated script lands inside `OUT_DIR` (always under `target/`); `build.rs` emits `cargo:rustc-link-arg=-T<path>`. Edit `tools/lds/koros.lds`, not the generated file. Notable symbols: `_skernel`/`_end` (image bounds, clip the frame allocator via `kor::kernel_phys_range()`), `stext`/`etext`, `_sdata`/`_edata`, `_sbss`/`_ebss`, `_load_end`, `.bss.bstack` (512 KiB boot stack, `bstack_top`).

Kernel virtual base addresses (high-half kernel; loongarch64 is direct-map, offset 0):

| Arch | Link/load VA (`KERNEL_BASE`) | Physical Load | `kernel_offset()` |
|------|------------------------------|---------------|-------------------------------------|
| riscv64 | `0xffffffc080200000` | `0x80200000` | `0xffffffc000000000` |
| x86_64 | `0xffff800000200000` | `0x200000` | `0xffff800000000000` |
| aarch64 | `0xffff000040080000` | `0x40080000` | `0xffff000000000000` |
| loongarch64 | `0x9000000080000000` | `0x9000000080000000` | `0` (no paging — VA == PA via DMW) |

## Architecture Abstraction Model

This is the central design decision. **Shared crates contain no `#[cfg(target_arch)]`** (two narrow exceptions: the `lib.rs`/`arch.rs` selectors of `kor-arch` and `kor-mmu`). Instead:

- `kor` defines `ArchProvider` (and `InterruptController`, `Console`, `TrapCallbacks`), registered once at boot into `spin::Once`-backed globals: `kor::arch::install`/`current()`, `install_controller`/`controller()`, `install_console`, `install_callbacks`.
- `kor-arch` provides one concrete `impl ArchProvider` per architecture (`Riscv64Provider`, `X86_64Provider`, `Aarch64Provider`, `Loongarch64Provider`), each a zero-sized struct behind a `pub static PROVIDER`. `base/kor-arch/src/lib.rs` uses a single `cfg_if!` to pick the active module and expose `provider()`/`interrupt_controller()`/`console()`/`TEST_VA_4K`/`TEST_VA_2M`.
- `koros::kernel_main` installs them.
- All downstream code calls `kor::arch::current()` (`&'static dyn ArchProvider`) — never cfg-selects a module. This keeps `kor`, `kor-alloc`, `kor-fs`, `kor-sched` fully arch-neutral.
- If a feature differs per architecture, implement it as a separate file in each arch directory (each exporting the same `ArchProvider` methods), not as `#[cfg]` branches in shared code.

`kor::TaskContext` is an opaque `#[repr(C, align(16))]` buffer of 16 `usize` (sized for the largest arch — riscv64 14 callee-saved slots, aarch64 13, loongarch64 12, x86_64 1). Each arch's `context.rs` casts it to its internal layout (size-checked with `const _: () = assert!(...)`).

## Crate Layout

```
kor/
  src/lib.rs              — module decls, re-exports, kernel_phys_range()
  src/arch.rs             — ArchProvider trait, TaskContext, PROVIDER registry
  src/addr.rs             — PhysAddr/VirtAddr newtypes
  src/interrupt.rs        — InterruptController trait + registry
  src/trap_callbacks.rs   — TrapCallbacks trait + registry; on_timer/dispatch_external
  src/console.rs          — Console trait + registry; print!/println! macros
  src/irq.rs / time.rs / smp.rs / cmdline.rs
  src/config.rs           — PlatformConfig (console, firmware_phys_start, dtb, PciEcam)
  src/mapping.rs          — MappingFlags, MapSize, MapError
  src/block.rs            — BlockDevice trait + BlockError
  src/vfs.rs              — FileType, Metadata, FsInfo, FsError, SuperBlock, INode traits
  src/regions.rs          — RegionCollector + clip_region (boot memory-region splitter)
  src/fdt.rs              — FDT parse: memory regions, cpu_count, bootargs
  src/driver.rs           — DeviceDriver trait, DtDevice, probe_fdt
  src/boot_stack.rs       — global_asm! 512 KiB boot stack (bstack/bstack_top)
kor/src/frame.rs           — FrameAllocator (bitmap + 2^n-aligned extents) + global free fns
base/kor-alloc/src/
  lib.rs                  — re-exports LockedSlabHeap/SlabHeap
  slab_heap.rs            — 7 size-class slab heap + frame-backed large allocs
filesystems/kor-fs/src/
  lib.rs                  — re-exports VFS/block types from kor; module decls
  block_cache.rs          — LRU write-back BlockCache
  fd.rs                   — FileDescriptor, OpenFlags, SeekFrom
  mount.rs                — MountTable + mount/unmount/resolve/sync_all
  path.rs                 — Path, resolve_path (symlink-following walker)
  registry.rs             — FileSystemDriver registry: register/find/mount_named
filesystems/kor-ext2/src/
  lib.rs                  — Ext2Fs + Ext2Driver (FileSystemDriver singleton)
  {bitmap,block_group,dir,inode,super_block,xattr}.rs
filesystems/kor-ramfs/src/
  lib.rs                  — RamFs + RamFsDriver (FileSystemDriver singleton)
base/kor-sched/src/
  lib.rs                  — Task/PerCpu, scheduler core, WaitQueue/Semaphore/Mutex2
  sync.rs                 — guard-based Mutex<T>, unbounded Channel<T>
base/kor-arch/src/
  lib.rs                  — cfg_if arch selector; provider()/interrupt_controller()/console()
  uart/{mod,ns16550a,pl011}.rs
  <arch>/{mod,boot,trap,mm,smp,time,irq,context,ic,console,provider}.rs   (page_table/mmu_format moved to base/kor-mmu/src/arch/)
    + riscv64: boot.S, trap.S, plic.rs
    + x86_64:  multiboot.S, trap.S, ap_boot.S  (no boot.S, no plic/gic)
    + aarch64: boot.S, trap.S, gic.rs
    + loongarch64: trap.S only (boot is inline naked_asm! in boot.rs; no paging, no plic/gic)
apps/koros/
  src/main.rs             — kernel_main (composition root) + demo tasks
  src/ext2_test.rs        — in-kernel ext2 functional self-check
  src/bench.rs            — storage benchmark (8 MiB ext2 + raw virtio)
  src/heap.rs             — bootstrap slab heap init + self_test
  src/panic.rs            — #[panic_handler]
  src/registries.rs       — BLOCKS / IRQS / MOUNTS singletons
  src/virtio.rs           — virtio-drivers adapter (KorosHal, VdBlk, MMIO + PCI + PCI-ECAM)
  build.rs                — generates linker_<arch>.lds from tools/lds/koros.lds


apps/korhv/                       — Type-1 hypervisor binary crate (parallel to koros)
  Cargo.toml                — depends on kor / kor-alloc (+ x86_64 crate on x86_64)
  build.rs                — same template as koros (tools/lds/korhv.lds); aarch64 links at 0x40080000 (identity, non-VHE EL2)
  src/main.rs               — hyp_main: console + provider + heap + frame alloc, then arch::hyp::init/run
  src/panic.rs / heap.rs    — copies of the koros panic/heap bootstrap
  src/arch/mod.rs           — cfg-selects x86_64 or aarch64 (other arches compile_error)
  src/arch/x86_64/hyp.rs    — SVM hyp: VMCB + NPT stage-2, VMRUN/#VMEXIT loop, VMMCALL hypercall, embedded guest
  src/arch/aarch64/hyp.rs   — EL2 hyp: vcpu_enter (ERET to EL1) + guest-exit handler, HVC hypercall, embedded guest.
                              Stage-2/VTTBR prepared but currently disabled (see HCR_EL2_VAL note in-file).
```

## Entry Flow

Per arch (assembly in `base/kor-arch/src/<arch>/`): `_start` → **clear BSS at physical addresses** (x86_64 excepted — Multiboot zeroes BSS via `bss_end_addr`) → build boot page tables → enable MMU → jump to high-half VA (loongarch64 skips MMU) → `rust_entry` (boot.rs) → `unsafe { kernel_main() }` (in `koros`).

`kernel_main` init sequence (`apps/koros/src/main.rs`):
1. Console — `kor::install_console(kor_arch::console())`.
2. Platform config — `config::init(platform_config())` (per-arch console type, `firmware_phys_start`, dtb, optional `PciEcam`).
3. Arch provider — `kor::install(kor_arch::provider())`.
4. Interrupt controller — `install_controller` + `ic.init()` (GICv2 on aarch64, PLIC on riscv64, polling stubs on x86_64/loongarch64).
5. Trap callbacks — `install_callbacks(&TRAP_CB)`; `on_timer` does `increment_tick` + `arch.handle_tick` + `kor_sched::timer_tick` + `kor_sched::preempt`; `on_external(irq)` dispatches `registries::IRQS`.
6. Bootstrap heap (`heap::init_bootstrap` on a 0xE000-byte static region), then register filesystem drivers (`kor_fs::register_filesystem(&kor_ext2::EXT2_DRIVER)` + `&kor_ramfs::RAMFS_DRIVER`) — the registry `Vec` allocates, so this must follow heap init.
7. Trap vectors — `arch.trap_init()`.
8. Memory init — capture cmdline, `RegionCollector`, `detect_memory_regions`, clip `kernel_phys_range()` + firmware hole, feed `kor::frame::add_region`, `heap::self_test()`, `kor_mmu::page_table_init()`, `page_table_self_test()` (4K + 2M map/translate round-trip).
9. Timer + IRQ on this CPU; print `Hello, world!` and cmdline.
10. `probe_devices()` (FDT virtio-mmio + optional PCI-ECAM on non-x86; PCI port-I/O on x86_64).
11. `kor::smp::boot_secondaries()`.
12. (riscv64/x86_64) if `cmdline::has_flag("bench")` → `bench::run()` then spin.
13. `ext2_test::ext2_test()`.
14. `kor_sched::init()`; spawn demo tasks (A/B/C/D, producer/consumer via `Channel`, blkio); `kor_sched::idle_loop()`.

Secondary CPUs: `secondary_entry(cpu_id)` → `trap_init` → `register_online` → per-CPU `ic.init()` → `timer_init` + `irq_enable` → `wait_for_interrupt` until `kor_sched::is_ready()` → `init_this_cpu()` → `idle_loop()`.

## Scheduler (`kor-sched`)

- **Model:** preemptive, multi-core, blocking kernel-thread scheduler. Tasks are `Arc<Task>` with a saved `TaskContext`, a 64 KiB private stack, an entry `fn()`, an atomic `state` (READY/RUNNING/SLEEPING/EXITED/BLOCKED), and a `wake_tick`.
- **Multi-core:** per-CPU state in `static CPUS: [PerCpu; MAX_CPUS]` (own `current`/`idle`/`prev`/`slice`); ready/sleeper/zombie queues are global `Mutex`-protected statics, so tasks migrate freely.
- **Preemption:** `timer_tick()` (from the arch timer trap) decrements this CPU's `slice`; `preempt()` calls `yield_now()` when `slice == 0` (`TIME_SLICE = 5` ticks ≈ 50 ms at `TICK_HZ = 100`). Critical sections run with interrupts disabled.
- **Context switch:** `schedule(prev_action, wait_ptr)` picks the next task, records a deferred transition for the outgoing task, switches via `arch.context_switch`, then `finish_switch()` applies the transition (TO_READY/TO_SLEEP/TO_ZOMBIE/TO_WAIT) **after** the switch completes — so a task is never made runnable until its context is fully saved. For blocking, the `WaitQueue` raw lock is held across the switch and released by `finish_switch`, closing lost-wakeup and migration races.
- **Blocking sync:** `WaitQueue`, `Semaphore`, `Mutex2`, `sync::Mutex<T>` (guard-based), `sync::Channel<T>` (unbounded MPMC) all *block* (schedule away), not spin. Use them only from task context after the scheduler is running; use `spin::Mutex` from interrupt handlers or early boot. (No `RwLock` exists.)

## Arch-Specific Quirks

- **riscv64**: Sv39 paging; OpenSBI/SBI ecalls (HSM for secondaries, TIME for timer); PLIC at `0x0c00_0000`; NS16550A MMIO at `0x1000_0000`. Simplest boot. `_secondary_start` entered via SBI `hart_start`.
- **x86_64**: Multiboot1 (magic `0x1BADB002`); 4-level paging with 2 MiB huge pages; LAPIC timer (calibrated against PIT); APs via INIT-SIPI-SIPI trampoline copied to `0x8000`; NS16550A port I/O at `0x3F8`. `q35` machine, `IvyBridge-v2` CPU. IC is a polling stub. Needs `-Clink-arg=-no-pie` (Makefile sets `RUSTFLAGS`).
- **aarch64**: EL1, 4-level paging (1 GiB blocks at boot); EL1 physical generic timer via GICv2 PPI; PSCI `CPU_ON` (HVC) for secondaries; GICv2 distributor `0x0800_0000` / CPU interface `0x0801_0000`; PL011 MMIO at `0x0900_0000`. `cortex-a72` CPU.
- **loongarch64**: **No paging yet** (direct-map, `kernel_offset() == 0`, VA == PA via DMW); boot entry is inline `naked_asm!` (no `boot.S`); constant timer; IPI+IOCSR-mailbox secondary bring-up; NS16550A MMIO at `0x1FE0_01E0`; PCI ECAM at `0x2000_0000`. IC and dynamic page-table are stubs (`dynamic_maps_supported() == false`, `TEST_VA_* == 0`). QEMU gets `-m 1G` explicitly.

## Coding Rules

- `#![no_std]` only — `core` and `alloc` (with provided allocator), never `std`. `println!`/`print!` come from `kor::console` (via the `Console` registry), not `std`.
- Minimize `unsafe` — wrap in safe abstractions, document why each block is needed. Keep `unsafe` blocks minimal in scope (smallest enclosing block, not whole-function), each paired with a `// SAFETY:` comment.
- **No magic numbers.** Never inline raw numeric literals for hardware addresses, register offsets, bit masks, flag bits, struct sizes, timeouts, or protocol constants. Define a named `const` (with a doc comment and unit) at the top of the module or in a dedicated `constants`/`regs` section, then refer to it by name. The only literals that may appear inline are `0`, `1`, self-evident indices, and bit-shift amounts paired with a named constant (e.g. `1 << UART_TX_SHIFT`). Prefer `core::mem::size_of`/`offset_of!` over hand-counted byte offsets, and bitfield helpers/newtypes over raw `& mask` arithmetic.
- **Good code style.** Match the surrounding crate's conventions: `snake_case` for items/locals, `PascalCase`/`UpperCamelCase` for types, `UPPER_SNAKE` for constants. Keep functions short and single-purpose; extract helpers rather than nesting deep `if`/`match` ladders. Document every `pub` item with a `///` summary and note invariants, preconditions, and why any `unsafe` is sound. Use meaningful names (`recv`/`dispatch`/`enqueue`, not `f`/`g`/`do_it`); avoid single-letter names outside tight loops or math. Prefer `Result`/`Option` and `?` over `unwrap()`/`expect()` in library code (panics are for genuinely unreachable invariants). Run `cargo fmt` before committing.

## Verification

- **Functional:** `make ARCH=<arch> run` boots the kernel, runs the ext2 self-check, then the scheduler demo. All four architectures boot to a completed scheduler demo on a fresh ext2 image.
- **Formal:** `./tools/scripts/verify.sh` runs Verus on the per-crate verification modules (`base/kor-alloc/verify/slab_heap.rs`, `kor/verify/frame_allocator.rs` — standalone, Verus-annotated copies of the allocator logic; keep new verification code under `<crate>/verify/`). Set `VERUS` env var or install Verus to `/tmp/verus-release/`. Pass a module name to verify one file: `./tools/scripts/verify.sh frame_allocator`.
