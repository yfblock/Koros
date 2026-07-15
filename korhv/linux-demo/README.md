# korhv aarch64 Linux passthrough demo

This directory boots a real Linux kernel (6.12.6, ARM64) as a guest under the
korhv EL2 hypervisor, using full passthrough: the guest at EL1 drives the real
GIC, architected timer and PL011 UART. The hypervisor only does the initial
ERET into the guest (and would handle SMC/PSCI if QEMU trapped them here).

## Result

Linux boots all the way to PID 1 and runs /init:

    === korhv: Type-1 hypervisor (860 KiB free) ===
    el2: passthrough configured (guest EL1, real GIC/timer/UART)
    el2: entering Linux guest at 0x40200000 (dtb=0x49000000)
    [    0.000000] Linux version 6.12.6 ...
    [    0.516709] Run /init as init process
    === korhv Linux passthrough: userspace (PID 1) reached! ===
    Hello from /init running under the korhv EL2 hypervisor.
    [    0.537077] reboot: Power down

## Files

- `init.c` / `initramfs.list` -- a static PID 1 that prints a banner and
  powers off, plus the kernel initramfs list (with /dev/console).
- `virt.dts` / `virt.dtb` -- the guest device tree: QEMU's `virt` DTB with
  /memory moved to 0x40200000 (so the guest cannot clobber the hypervisor at
  0x40080000) and bootargs=console=ttyAMA0 earlycon=pl011,0x09000000.
- `run.sh` -- launches QEMU with the hypervisor, Image and DTB.

## How to (re)build

1. Build the hypervisor: `make build ARCH=aarch64 BIN=korhv` (produces
   `target/aarch64-unknown-none-softfloat/release/korhv.bin`).
2. Build a Linux ARM64 Image with the initramfs baked in:
   - `aarch64-linux-musl-gcc -static -O2 -s init.c -o init`
   - edit `initramfs.list` so the `file /init` line points at that `init`
   - configure+build the kernel:
     `make ARCH=arm64 CROSS_COMPILE=aarch64-linux-musl- defconfig`
     `./scripts/config --set-str CONFIG_INITRAMFS_SOURCE $PWD/initramfs.list`
     `make ARCH=arm64 CROSS_COMPILE=aarch64-linux-musl- olddefconfig`
     `make ARCH=arm64 CROSS_COMPILE=aarch64-linux-musl- -j$(nproc) Image`
   - copy `arch/arm64/boot/Image` here (or set $IMAGE).
3. (Re)build the DTB if you edited virt.dts:
   `dtc -I dts -O dtb virt.dts -o virt.dtb`
4. Run: `./run.sh` (override $KORHV / $IMAGE / $DTB if needed).


## Notes / limitations

- The current EL2 implementation uses owned stage-2 roots for two guests,
  virtualizes the GIC and UART, validates virtio descriptor GPAs before
  temporarily translating them for DMA, and routes one private block device to
  each guest. These boundaries require hardware qualification in addition to
  QEMU testing.
- QEMU TCG may handle platform details differently from physical EL2/SVM
  implementations and is not a substitute for hardware qualification.
- The Linux demo artifacts (`Image`, DTBs, and ext2 disks) remain optional.
