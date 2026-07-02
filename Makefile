# Koros Makefile
# Usage:
#   make ARCH=riscv64 run
#   make ARCH=aarch64 run
#   make ARCH=loongarch64 run
#   make ARCH=x86_64 run

ARCH ?= riscv64
SMP  ?= 1

QEMU_EXEC := qemu-system-$(ARCH)

ifeq ($(ARCH), riscv64)
  TARGET := riscv64gc-unknown-none-elf
  QEMU_EXEC += -machine virt
  KERNEL_IMG := target/$(TARGET)/release/Koros.bin
  EXT2_IMG := os-images/ext2-test.img
else ifeq ($(ARCH), aarch64)
  TARGET := aarch64-unknown-none-softfloat
  QEMU_EXEC += -cpu cortex-a72 -machine virt
  KERNEL_IMG := target/$(TARGET)/release/Koros.bin
  EXT2_IMG := os-images/ext2-test.img
else ifeq ($(ARCH), loongarch64)
  TARGET := loongarch64-unknown-none
  QEMU_EXEC += -M virt -m 1G
  KERNEL_IMG := target/$(TARGET)/release/Koros
else ifeq ($(ARCH), x86_64)
  TARGET := x86_64-unknown-none
  RUSTFLAGS_EXTRA := -Clink-arg=-no-pie
  QEMU_EXEC += -machine q35 -cpu IvyBridge-v2
  # x86_64 multiboot needs the ELF, not the raw binary.
  KERNEL_IMG := target/$(TARGET)/release/Koros
  EXT2_IMG := os-images/ext2-test.img
else
  $(error ARCH must be one of: riscv64, x86_64, aarch64, loongarch64)
endif

KERNEL_ELF := target/$(TARGET)/release/Koros
OBJCOPY := rust-objcopy

QEMU_EXEC += -kernel $(KERNEL_IMG) -nographic -smp $(SMP)

ifdef EXT2_IMG
  ifeq ($(ARCH), x86_64)
    QEMU_EXEC += -drive file=$(EXT2_IMG),format=raw,if=none,id=ext2drv -device virtio-blk-pci,drive=ext2drv
  else
    QEMU_EXEC += -global virtio-mmio.force-legacy=false -drive file=$(EXT2_IMG),format=raw,if=none,id=ext2drv -device virtio-blk-device,drive=ext2drv
  endif
endif

export RUSTFLAGS := $(RUSTFLAGS_EXTRA)

.PHONY: build run clean verify

build:
	cargo build --release --target $(TARGET)
ifneq ($(KERNEL_IMG),$(KERNEL_ELF))
	$(OBJCOPY) $(KERNEL_ELF) --strip-all -O binary $(KERNEL_IMG)
endif

run: build $(EXT2_IMG)
	$(QEMU_EXEC)

os-images/ext2-test.img:
	@mkdir -p $(dir $@)
	dd if=/dev/zero of=$@ bs=1M count=4 2>/dev/null
	mkfs.ext2 -b 1024 -q $@

debug: build
	$(QEMU_EXEC) -s -S

clean:
	cargo clean
	rm -rf os-images

verify:
	./scripts/verify.sh
