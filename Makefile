# Koros Makefile
# Usage:
#   make ARCH=riscv64 run                       # boot koros
#   make ARCH=x86_64  BIN=korhv run             # boot the Type-1 hypervisor
#   make ARCH=aarch64 BIN=korhv run
#   make ARCH=riscv64 CMDLINE="root=/dev/vda loglevel=7" run
#
# BIN defaults to `koros`.  `korhv` selects hypervisor-friendly QEMU CPU/machine
# settings (AMD SVM via `-cpu max` on x86_64; EL2 via `virtualization=on` on
# aarch64) and skips the ext2 test image.

ARCH    ?= riscv64
BIN     ?= koros
SMP     ?= 1
CMDLINE ?=

QEMU_EXEC := qemu-system-$(ARCH)

ifeq ($(ARCH), riscv64)
  TARGET := riscv64gc-unknown-none-elf
  QEMU_EXEC += -machine virt
  KERNEL_IMG := target/$(TARGET)/release/$(BIN).bin
  ifeq ($(BIN), koros)
    EXT2_IMG := os-images/ext2-test.img
  endif
else ifeq ($(ARCH), aarch64)
  TARGET := aarch64-unknown-none-softfloat
  ifeq ($(BIN), korhv)
    QEMU_EXEC += -machine virt,virtualization=on -cpu cortex-a72
  else
    QEMU_EXEC += -cpu cortex-a72 -machine virt
  endif
  KERNEL_IMG := target/$(TARGET)/release/$(BIN).bin
  ifeq ($(BIN), koros)
    EXT2_IMG := os-images/ext2-test.img
  endif
else ifeq ($(ARCH), loongarch64)
  TARGET := loongarch64-unknown-none
  QEMU_EXEC += -M virt -m 1G
  KERNEL_IMG := target/$(TARGET)/release/$(BIN)
  ifeq ($(BIN), koros)
    EXT2_IMG := os-images/ext2-test.img
  endif
else ifeq ($(ARCH), x86_64)
  TARGET := x86_64-unknown-none
  RUSTFLAGS_EXTRA := -Clink-arg=-no-pie
  ifeq ($(BIN), korhv)
    # AMD SVM: QEMU TCG emulates SVM under `-cpu max` (Intel VMX is not emulated).
    QEMU_EXEC += -machine q35 -cpu max
  else
    QEMU_EXEC += -machine q35 -cpu IvyBridge-v2
  endif
  # x86_64 multiboot needs the ELF, not the raw binary.
  KERNEL_IMG := target/$(TARGET)/release/$(BIN)
  ifeq ($(BIN), koros)
    EXT2_IMG := os-images/ext2-test.img
  endif
else
  $(error ARCH must be one of: riscv64, x86_64, aarch64, loongarch64)
endif

KERNEL_ELF := target/$(TARGET)/release/$(BIN)
OBJCOPY := rust-objcopy

QEMU_EXEC += -kernel $(KERNEL_IMG) -nographic -smp $(SMP)

# Give each vCPU its own host thread so secondaries run in parallel with the
# boot CPU (the default single-threaded round-robin TCG starves them).
ifneq ($(SMP),1)
  QEMU_EXEC += -accel tcg,thread=multi
endif

ifdef EXT2_IMG
  # x86_64 (q35) and loongarch64 (virt) carry virtio on PCIe; riscv64/aarch64
  # 'virt' machines use the virtio-mmio bus.
  ifneq ($(filter $(ARCH),x86_64 loongarch64),)
    QEMU_EXEC += -drive file=$(EXT2_IMG),format=raw,if=none,id=ext2drv -device virtio-blk-pci,drive=ext2drv
  else
    QEMU_EXEC += -global virtio-mmio.force-legacy=false -drive file=$(EXT2_IMG),format=raw,if=none,id=ext2drv -device virtio-blk-device,drive=ext2drv
  endif
endif

# Kernel command line: FDT /chosen/bootargs (riscv64/aarch64/loongarch64) or
# the Multiboot cmdline (x86_64). Read at boot via koros_core::cmdline.
ifneq ($(CMDLINE),)
  QEMU_EXEC += -append "$(CMDLINE)"
endif

export RUSTFLAGS := $(RUSTFLAGS_EXTRA)

.PHONY: build run clean verify

build:
	cargo build --release --target $(TARGET) -p $(BIN)
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
