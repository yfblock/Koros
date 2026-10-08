#!/usr/bin/env python3
import hashlib
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

VERSION = "3.23.5"
BRANCH = "v3.23"
ARCHES = {
    "riscv64": "51019c39af89029a8bc19255738d370bc3b222830dbb78129b02721ac6b5933d",
    "aarch64": "d9a77cb31f715c56afa4f0a5aa42c04cfde813b70ad74a64725902b09c29a6cc",
    "x86_64": "fae0d78ad39563573ddececfdd55ae1040ed428442e95ea5401cf66d9079b327",
}
BUSYBOXES = {
    "riscv64": "korix/busybox-riscv64",
    "aarch64": "korix/busybox-aarch64",
    "x86_64": "korix/busybox-x86_64",
}
IMAGE_MIB = 64
BLOCK_SIZE = 1024


def download(url, destination):
    with urllib.request.urlopen(url) as response, open(destination, "wb") as output:
        shutil.copyfileobj(response, output)


def verify(path, expected):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    if digest.hexdigest() != expected:
        raise SystemExit(f"checksum mismatch for {path}")


def safe_extract(archive, destination):
    root = os.path.realpath(destination)
    for member in archive.getmembers():
        target = os.path.realpath(os.path.join(destination, member.name))
        if target != root and not target.startswith(root + os.sep):
            raise SystemExit(f"unsafe archive member: {member.name}")
    archive.extractall(destination, filter="fully_trusted")


def prepare_root(root, arch):
    for directory in ("dev", "proc", "sys", "run", "tmp", "root"):
        os.makedirs(os.path.join(root, directory), exist_ok=True)
    os.chmod(os.path.join(root, "tmp"), 0o1777)
    hostname = os.path.join(root, "etc", "hostname")
    with open(hostname, "w", encoding="ascii") as output:
        output.write("korix\n")
    profile = os.path.join(root, "etc", "profile")
    with open(profile, "a", encoding="ascii") as output:
        output.write("\nexport PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n")
    os.makedirs(os.path.join(root, "etc", "profile.d"), exist_ok=True)
    with open(os.path.join(root, "etc", "resolv.conf"), "w", encoding="ascii") as output:
        output.write("nameserver 10.0.2.3\n")
    repositories = os.path.join(root, "etc", "apk", "repositories")
    with open(repositories, "w", encoding="ascii") as output:
        output.write(f"http://dl-cdn.alpinelinux.org/alpine/{BRANCH}/main\n")
        output.write(f"http://dl-cdn.alpinelinux.org/alpine/{BRANCH}/community\n")
    busybox = os.path.join(root, "bin", "busybox")
    shutil.copy2(BUSYBOXES[arch], busybox)
    os.chmod(busybox, 0o755)


def main():
    if len(sys.argv) not in (2, 3) or sys.argv[1] not in ARCHES:
        raise SystemExit("usage: mk-alpine-rootfs.py <riscv64|aarch64|x86_64> [image]")
    arch = sys.argv[1]
    image = sys.argv[2] if len(sys.argv) == 3 else f"tools/os-images/alpine-{arch}.img"
    filename = f"alpine-minirootfs-{VERSION}-{arch}.tar.gz"
    url = f"https://dl-cdn.alpinelinux.org/alpine/{BRANCH}/releases/{arch}/{filename}"
    cache = os.path.join("tools", "os-images", "cache", filename)
    os.makedirs(os.path.dirname(cache), exist_ok=True)
    os.makedirs(os.path.dirname(image), exist_ok=True)
    if not os.path.exists(cache):
        print(f"downloading {url}")
        download(url, cache)
    verify(cache, ARCHES[arch])
    with tempfile.TemporaryDirectory() as root:
        with tarfile.open(cache, "r:gz") as archive:
            safe_extract(archive, root)
        prepare_root(root, arch)
        subprocess.run(
            ["dd", "if=/dev/zero", f"of={image}", "bs=1M", f"count={IMAGE_MIB}"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        subprocess.run(
            ["mkfs.ext2", "-F", "-b", str(BLOCK_SIZE), "-d", root, image],
            check=True,
            stdout=subprocess.DEVNULL,
        )
    print(image)


if __name__ == "__main__":
    main()
