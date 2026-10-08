#!/bin/sh
# Run the korhv aarch64 Linux passthrough demo.
#
# Prerequisites (see README.md for how to build them):
#   - KORHV kernel Image (ARM64) with a built-in initramfs, at $IMAGE
#   - a guest DTB (virt.dtb) next to this script
#   - the korhv.bin hypervisor at ../../target/.../korhv.bin
#
# Memory layout (QEMU -m 256M, RAM 0x40000000..0x50000000):
#   0x40080000  korhv.bin        (-kernel)
#   0x40200000  Linux Image      (-device loader)  <- guest entry, /memory start
#   0x49000000  guest DTB        (-device loader)  <- passed to Linux in x0
set -e
ROOT=$(cd "$(dirname "$0")" && pwd)
KORHV=${KORHV:-$ROOT/../../target/aarch64-unknown-none-softfloat/release/korhv.bin}
IMAGE=${IMAGE:-$ROOT/Image}
DTB=${DTB:-$ROOT/virt.dtb}

exec qemu-system-aarch64 \
  -machine virt,virtualization=on -cpu cortex-a72 -m 256M -smp 1 \
  -kernel "$KORHV" \
  -device loader,file="$IMAGE",addr=0x40200000 \
  -device loader,file="$DTB",addr=0x49000000 \
  -nographic -no-reboot
