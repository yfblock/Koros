#!/bin/sh
# Run the Korhv aarch64 two-Linux-guest demo.
# Korhv starts at its own shell. Use `enter 0` or `enter 1` to attach a guest,
# then press Ctrl-O to return to the Korhv shell.
#
# NOTE: Shell input requires QEMU to deliver stdin to PL011 RX FIFO.
# This works when running interactively in a real terminal (tty),
# but may not work through pipes/sockets due to QEMU TCG single-thread.
set -e
ROOT=$(cd "$(dirname "$0")" && pwd)
KORHV=${KORHV:-$ROOT/../../../target/aarch64-unknown-none-softfloat/release/korhv.bin}
IMAGE=${IMAGE:-$ROOT/Image}
DTB=${DTB:-$ROOT/virt.dtb}
DTB0=${DTB0:-$ROOT/virt0.dtb}
DTB1=${DTB1:-$ROOT/virt1.dtb}
DISK0=${DISK0:-$ROOT/disk0.img}
DISK1=${DISK1:-$ROOT/disk1.img}

if [ ! -f "$DISK0" ]; then
  echo "Creating $DISK0 (32MiB ext2)..." >&2
  dd if=/dev/zero of="$DISK0" bs=1M count=32 status=none
  mkfs.ext2 -q "$DISK0"
fi
if [ ! -f "$DISK1" ]; then
  cp "$DISK0" "$DISK1"
fi
cp "$DTB" "$DTB0"
cp "$DTB" "$DTB1"

exec qemu-system-aarch64 \
  -machine virt,virtualization=on -cpu cortex-a72 -m 512M -smp 1 \
  -kernel "$KORHV" \
  -device loader,file="$IMAGE",addr=0x40200000 \
  -device loader,file="$IMAGE",addr=0x44200000 \
  -device loader,file="$DTB0",addr=0x48200000 \
  -device loader,file="$DTB1",addr=0x48400000 \
  -drive file="$DISK0",if=none,id=hd0,format=raw \
  -drive file="$DISK1",if=none,id=hd1,format=raw \
  -device virtio-blk-device,drive=hd0,bus=virtio-mmio-bus.0 \
  -device virtio-blk-device,drive=hd1,bus=virtio-mmio-bus.1 \
  -serial stdio -monitor none -display none
