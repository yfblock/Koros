#!/usr/bin/env python3
import os
import shutil
import subprocess
import sys
import tempfile

ARCHES = {
    "riscv64": "apps/korix/busybox-riscv64",
    "aarch64": "apps/korix/busybox-aarch64",
    "x86_64": "apps/korix/busybox-x86_64",
}
APPLETS = [
    "sh", "ls", "echo", "cat", "date", "true", "false", "pwd", "mkdir",
    "rmdir", "rm", "cp", "mv", "ln", "head", "tail", "wc", "sort", "uniq",
    "grep", "sed", "awk", "vi", "env", "id", "whoami", "hostname", "sleep",
    "test", "[",
]


def main():
    if len(sys.argv) not in (2, 3) or sys.argv[1] not in ARCHES:
        raise SystemExit("usage: mk-korix-rootfs.py <riscv64|aarch64|x86_64> [image]")
    arch = sys.argv[1]
    image = sys.argv[2] if len(sys.argv) == 3 else f"tools/os-images/korix-{arch}.img"
    busybox = ARCHES[arch]
    os.makedirs(os.path.dirname(image), exist_ok=True)
    with tempfile.TemporaryDirectory() as root:
        for directory in ("bin", "dev", "proc", "etc", "tmp", "root", "tests"):
            os.makedirs(os.path.join(root, directory), exist_ok=True)
        shutil.copy2(busybox, os.path.join(root, "bin", "busybox"))
        os.chmod(os.path.join(root, "bin", "busybox"), 0o755)
        ltp = "tools/ltp/target/riscv64gc-unknown-none-elf/release/korix-ltp"
        if arch == "riscv64" and os.path.exists(ltp):
            shutil.copy2(ltp, os.path.join(root, "tests", "korix-ltp"))
            os.chmod(os.path.join(root, "tests", "korix-ltp"), 0o755)
        for applet in APPLETS:
            os.symlink("busybox", os.path.join(root, "bin", applet))
        with open(os.path.join(root, "etc", "passwd"), "w", encoding="ascii") as file:
            file.write("root:x:0:0:root:/root:/bin/sh\n")
        with open(os.path.join(root, "etc", "group"), "w", encoding="ascii") as file:
            file.write("root:x:0:\n")
        with open(os.path.join(root, "etc", "hostname"), "w", encoding="ascii") as file:
            file.write("korix\n")
        subprocess.run(["dd", "if=/dev/zero", f"of={image}", "bs=1M", "count=16"], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["mkfs.ext2", "-F", "-b", "1024", "-d", root, image], check=True, stdout=subprocess.DEVNULL)
    print(image)


if __name__ == "__main__":
    main()
