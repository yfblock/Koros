# Koros Makefile
# Usage:
#   make run                        # boot koros (default: BIN ?= koros)
#   make koros | korix | korhv      # convenience == BIN=<name> run
#   make ARCH=x86_64 korhv          # flags still combine with the shortcuts
#   make ARCH=riscv64 CMDLINE="mmutest shutdown" run
#
# BIN defaults to `koros`.  `korhv` selects hypervisor-friendly QEMU CPU/machine
# settings (AMD SVM via `-cpu max` on x86_64; EL2 via `virtualization=on` on
# aarch64) and skips the ext2 test image.

ARCH    ?= riscv64
BIN     ?= koros
SMP     ?= 1
CMDLINE ?=
ROOTFS ?= korix

QEMU_EXEC := qemu-system-$(ARCH)

ifeq ($(ARCH), riscv64)
  TARGET := riscv64gc-unknown-none-elf
  QEMU_EXEC += -machine virt
  KERNEL_IMG := target/$(TARGET)/release/$(BIN).bin
  ifeq ($(BIN), korix)
    EXT2_IMG := tools/os-images/$(ROOTFS)-riscv64.img
  else ifeq ($(BIN), koros)
    EXT2_IMG := tools/os-images/ext2-test.img
  endif
else ifeq ($(ARCH), aarch64)
  TARGET := aarch64-unknown-none-softfloat
  ifeq ($(BIN), korhv)
    QEMU_EXEC += -machine virt,virtualization=on -cpu cortex-a72
  else
    QEMU_EXEC += -cpu cortex-a72 -machine virt
  endif
  KERNEL_IMG := target/$(TARGET)/release/$(BIN).bin
  ifeq ($(BIN), korix)
    EXT2_IMG := tools/os-images/$(ROOTFS)-aarch64.img
  else ifeq ($(BIN), koros)
    EXT2_IMG := tools/os-images/ext2-test.img
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
  ifeq ($(BIN), korix)
    EXT2_IMG := tools/os-images/$(ROOTFS)-x86_64.img
  else ifeq ($(BIN), koros)
    EXT2_IMG := tools/os-images/ext2-test.img
  endif
else
  $(error ARCH must be one of: riscv64, x86_64, aarch64)
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
  # x86_64 (q35) carries virtio on PCIe; riscv64/aarch64 'virt' machines
  # use the virtio-mmio bus.
  ifeq ($(ARCH),x86_64)
    QEMU_EXEC += -drive file=$(EXT2_IMG),format=raw,if=none,id=ext2drv -device virtio-blk-pci,drive=ext2drv
    ifeq ($(BIN), korix)
      QEMU_EXEC += -netdev user,id=net0 -device virtio-net-pci,netdev=net0
    endif
  else
    QEMU_EXEC += -global virtio-mmio.force-legacy=false -drive file=$(EXT2_IMG),format=raw,if=none,id=ext2drv -device virtio-blk-device,drive=ext2drv
    ifeq ($(BIN), korix)
      QEMU_EXEC += -netdev user,id=net0 -device virtio-net-device,netdev=net0
      # Second blank disk exposed as /dev/vdb for LTP scratch-device tests.
      SCRATCH_IMG ?= tools/os-images/scratch.img
      $(shell test -f $(SCRATCH_IMG) || { mkdir -p tools/os-images; dd if=/dev/zero of=$(SCRATCH_IMG) bs=1M count=16 status=none; })
      QEMU_EXEC += -drive file=$(SCRATCH_IMG),format=raw,if=none,id=scratchdrv -device virtio-blk-device,drive=scratchdrv
    endif
  endif
endif

# Kernel command line: FDT /chosen/bootargs (riscv64/aarch64) or
# the Multiboot cmdline (x86_64). Read at boot via koros_core::cmdline.
ifneq ($(CMDLINE),)
  QEMU_EXEC += -append "$(CMDLINE)"
endif

export RUSTFLAGS := $(RUSTFLAGS_EXTRA)

.PHONY: build run clean verify koros korix korhv

build:
	cargo build --release --target $(TARGET) -p $(BIN)
ifneq ($(KERNEL_IMG),$(KERNEL_ELF))
	$(OBJCOPY) $(KERNEL_ELF) --strip-all -O binary $(KERNEL_IMG)
endif


run: build $(EXT2_IMG)
	@if [ "$(ARCH)" = "aarch64" ] && [ "$(BIN)" = "korhv" ]; then \
		exec apps/korhv/linux-demo/run2.sh; \
	else \
		exec $(QEMU_EXEC); \
	fi

# Convenience targets: `make koros` == `make BIN=koros run` (and `make run`
# defaults to koros because BIN ?= koros).
koros:
	$(MAKE) run BIN=koros

korix:
	$(MAKE) run BIN=korix

korhv:
	$(MAKE) run BIN=korhv


tools/os-images/ext2-test.img:
	@mkdir -p $(dir $@)
	dd if=/dev/zero of=$@ bs=1M count=4 2>/dev/null
	mkfs.ext2 -b 1024 -q $@

tools/os-images/korix-riscv64.img: apps/korix/busybox-riscv64 tools/scripts/mk-korix-rootfs.py
	tools/scripts/mk-korix-rootfs.py riscv64 $@

tools/os-images/korix-aarch64.img: apps/korix/busybox-aarch64 tools/scripts/mk-korix-rootfs.py
	tools/scripts/mk-korix-rootfs.py aarch64 $@

tools/os-images/korix-x86_64.img: apps/korix/busybox-x86_64 tools/scripts/mk-korix-rootfs.py
	tools/scripts/mk-korix-rootfs.py x86_64 $@

tools/os-images/alpine-riscv64.img: tools/scripts/mk-alpine-rootfs.py apps/korix/busybox-riscv64
	tools/scripts/mk-alpine-rootfs.py riscv64 $@

tools/os-images/alpine-aarch64.img: tools/scripts/mk-alpine-rootfs.py apps/korix/busybox-aarch64
	tools/scripts/mk-alpine-rootfs.py aarch64 $@

tools/os-images/alpine-x86_64.img: tools/scripts/mk-alpine-rootfs.py apps/korix/busybox-x86_64
	tools/scripts/mk-alpine-rootfs.py x86_64 $@

debug: build
	$(QEMU_EXEC) -s -S

clean:
	cargo clean
	rm -rf os-images

verify:
	./tools/scripts/verify.sh
