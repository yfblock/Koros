#!/usr/bin/env python3
import os
import select
import subprocess
import sys
import time

TARGETS = {
    "riscv64": {
        "build": ["make", "build", "ARCH=riscv64", "BIN=korix"],
        "qemu": ["qemu-system-riscv64", "-machine", "virt", "-kernel", "target/riscv64gc-unknown-none-elf/release/korix.bin", "-nographic", "-smp", "1", "-global", "virtio-mmio.force-legacy=false", "-drive", "file=tools/os-images/korix-riscv64.img,format=raw,if=none,id=ext2drv", "-device", "virtio-blk-device,drive=ext2drv"],
        "boot_delay": 8,
        "commands": [b"echo hello\n", b"ls /\n", b"cat /etc/hostname\n", b"id\n", b"exit\n"],
        "expected": ["hello", "bin", "dev", "etc", "proc", "root", "tmp", "korix", "uid=0"],
    },
    "aarch64": {
        "build": ["make", "build", "ARCH=aarch64", "BIN=korix"],
        "qemu": ["qemu-system-aarch64", "-cpu", "cortex-a72", "-machine", "virt", "-kernel", "target/aarch64-unknown-none-softfloat/release/korix.bin", "-nographic", "-smp", "1", "-global", "virtio-mmio.force-legacy=false", "-drive", "file=tools/os-images/korix-aarch64.img,format=raw,if=none,id=ext2drv", "-device", "virtio-blk-device,drive=ext2drv"],
        "boot_delay": 8,
        "commands": [b"echo hello\n", b"ls /\n", b"cat /etc/hostname\n", b"id\n", b"exit\n"],
        "expected": ["hello", "bin", "dev", "etc", "proc", "root", "tmp", "korix", "uid=0"],
    },
    "x86_64": {
        "build": ["make", "build", "ARCH=x86_64", "BIN=korix"],
        "qemu": ["qemu-system-x86_64", "-kernel", "target/x86_64-unknown-none/release/korix", "-nographic", "-smp", "1", "-m", "256M", "-no-reboot"],
        "boot_delay": 40,
        "commands": [],
        "expected": ["entering user mode"],
    },
}


def run_target(name):
    cfg = TARGETS[name]
    subprocess.run(cfg["build"], check=True)
    image = next((arg.split("=", 1)[1].split(",", 1)[0] for arg in cfg["qemu"] if arg.startswith("file=")), None)
    if image and not os.path.exists(image):
        subprocess.run(["scripts/mk-korix-rootfs.py", name, image], check=True)
    proc = subprocess.Popen(cfg["qemu"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=0)
    output = bytearray()
    start = time.time()
    sent = False
    try:
        while time.time() - start < cfg["boot_delay"] + 20:
            ready, _, _ = select.select([proc.stdout], [], [], 0.1)
            if ready:
                data = os.read(proc.stdout.fileno(), 4096)
                if not data:
                    break
                output.extend(data)
            if not sent and time.time() - start >= cfg["boot_delay"]:
                for command in cfg["commands"]:
                    proc.stdin.write(command)
                    proc.stdin.flush()
                    time.sleep(0.5)
                sent = True
            if sent and time.time() - start >= cfg["boot_delay"] + 8:
                break
    finally:
        proc.kill()
    text = output.decode("utf-8", errors="replace")
    if "KORIX PANIC" in text or "Triple fault" in text:
        raise RuntimeError(f"{name}: kernel failure\n{text[-3000:]}")
    missing = [needle for needle in cfg["expected"] if needle not in text]
    if missing:
        raise RuntimeError(f"{name}: missing output {missing}\n{text[-3000:]}")
    print(f"{name}: PASS")


def main():
    targets = sys.argv[1:] or list(TARGETS)
    for target in targets:
        if target not in TARGETS:
            raise SystemExit(f"unknown target: {target}")
        run_target(target)


if __name__ == "__main__":
    main()
