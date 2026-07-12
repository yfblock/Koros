#!/bin/sh
# Run the korhv aarch64 2-Linux-guest demo with interactive shell.
# Serial on stdio. Both guests show shell prompt via /dev/kmsg (earlycon).
# Input via magic MMIO at 0x0a010000 (mmap /dev/mem) — traps to hypervisor.
# Ctrl-A toggles between G0 and G1 input.
#
# NOTE: Shell input requires QEMU to deliver stdin to PL011 RX FIFO.
# This works when running interactively in a real terminal (tty),
# but may not work through pipes/sockets due to QEMU TCG single-thread.
set -e
ROOT=$(cd "$(dirname "$0")" && pwd)
KORHV=${KORHV:-$ROOT/../../target/aarch64-unknown-none-softfloat/release/korhv.bin}
IMAGE=${IMAGE:-$ROOT/Image}
DTB=${DTB:-$ROOT/virt.dtb}
DISK=${DISK:-$ROOT/disk.img}

if [ ! -f "$DISK" ]; then
  echo "Creating $DISK (32MiB ext2)..." >&2
  dd if=/dev/zero of="$DISK" bs=1M count=32 status=none
  mkfs.ext2 -q "$DISK"
fi

exec qemu-system-aarch64 \
  -machine virt,virtualization=on -cpu cortex-a72 -m 256M -smp 1 \
  -kernel "$KORHV" \
  -device loader,file="$IMAGE",addr=0x40200000 \
  -device loader,file="$IMAGE",addr=0x44200000 \
  -device loader,file="$DTB",addr=0x49000000 \
  -drive file="$DISK",if=none,id=hd0,format=raw,file.locking=off \
  -drive file="$DISK",if=none,id=hd1,format=raw,file.locking=off \
  -device virtio-blk-device,drive=hd0,bus=virtio-mmio-bus.0 \
  -device virtio-blk-device,drive=hd1,bus=virtio-mmio-bus.1 \
  -serial stdio -monitor none -display none
