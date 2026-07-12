/* Interactive shell for korhv guests.
 * Output via /dev/kmsg (earlycon polling mode).
 * Input via magic MMIO at 0x0a010000 (mmap /dev/mem) — traps to hypervisor.
 * Commands: ls, cat, echo, sync, poweroff, help */
#include <time.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/mount.h>
#include <sys/mman.h>
#include <fcntl.h>

static int kmsg;
static volatile unsigned int *getchar_mmio;

static void puts_(const char *s) {
    int n = 0; while (s[n]) n++;
    syscall(SYS_write, kmsg, s, n);
}

static int strlen_(const char *s) { int n = 0; while (s[n]) n++; return n; }
static int strcmp_(const char *a, const char *b) {
    while (*a && *a == *b) { a++; b++; } return (unsigned char)*a - (unsigned char)*b;
}

static int mmio_getchar(void) {
    if (!getchar_mmio) return -1;
    unsigned int val = *getchar_mmio;
    if (val == 0xffffffff) return -1;
    return (int)val;
}

static char buf[512];

static void list_dir(const char *path) {
    int fd = syscall(SYS_openat, -100, path, 0x10000, 0);
    if (fd < 0) { puts_("ls: cannot open "); puts_(path); puts_("\n"); return; }
    char ent[256]; long ret;
    while ((ret = syscall(SYS_getdents64, fd, ent, sizeof(ent))) > 0) {
        int pos = 0;
        while (pos < ret) {
            unsigned short reclen = *(unsigned short*)(ent + pos + 16);
            char *name = ent + pos + 19;
            puts_(name); puts_("  ");
            pos += reclen;
        }
    }
    puts_("\n"); syscall(SYS_close, fd);
}

static void cat_file(const char *path) {
    int fd = syscall(SYS_openat, -100, path, 0, 0);
    if (fd < 0) { puts_("cat: "); puts_(path); puts_(": not found\n"); return; }
    int n;
    while ((n = syscall(SYS_read, fd, buf, sizeof(buf))) > 0) syscall(SYS_write, kmsg, buf, n);
    syscall(SYS_close, fd);
}

static void read_line(char *line, int maxlen) {
    int pos = 0;
    while (pos < maxlen - 1) {
        int c = mmio_getchar();
        if (c < 0 || c > 255) {
            struct timespec ts = {0, 20000000};
            syscall(SYS_nanosleep, &ts, 0);
            continue;
        }
        if (c == '\r' || c == '\n') { puts_("\n"); line[pos] = 0; return; }
        if (c == 0x7f || c == 0x08) { if (pos > 0) pos--; continue; }
        char echo[2] = {c, 0}; puts_(echo);
        line[pos++] = c;
    }
    line[pos] = 0;
}

static void skip_spaces(char **p) { while (**p == ' ') (*p)++; }
static char *next_token(char **p) {
    skip_spaces(p);
    if (!**p) return 0;
    char *start = *p;
    while (**p && **p != ' ') (*p)++;
    if (**p) { **p = 0; (*p)++; }
    return start;
}

int main(void) {
    syscall(SYS_mount, "devtmpfs", "/dev", "devtmpfs", 0, 0);
    syscall(SYS_mount, "proc", "/proc", "proc", 0, 0);
    syscall(SYS_mkdirat, -100, "/mnt", 0755);

    kmsg = syscall(SYS_openat, -100, "/dev/kmsg", 02, 0);
    if (kmsg < 0) kmsg = 1;

    /* Map magic getchar MMIO at 0x0a010000 */
    int memfd = syscall(SYS_openat, -100, "/dev/mem", 02, 0); /* O_RDWR */
    if (memfd >= 0) {
        long m = syscall(SYS_mmap, 0, 4096, 3 /*PROT_READ|PROT_WRITE*/, 1 /*MAP_SHARED*/, memfd, 0x0a010000);
        if (m > 0) {
            getchar_mmio = (volatile unsigned int *)m;
            puts_("getchar MMIO mapped\n");
        } else {
            puts_("mmap failed\n");
        }
        syscall(SYS_close, memfd);
    } else {
        puts_("/dev/mem open failed\n");
    }

    long ret = syscall(SYS_mount, "/dev/vda", "/mnt", "ext2", 0, 0);

    puts_("\n=== korhv shell ===\n");
    if (ret == 0) puts_("vda mounted on /mnt\n");
    else puts_("vda mount failed\n");
    puts_("Commands: ls, cat, echo, sync, poweroff, help\n");

    for (;;) {
        puts_("\n$ ");
        read_line(buf, sizeof(buf));
        char *p = buf;
        char *cmd = next_token(&p);
        if (!cmd || !*cmd) continue;

        if (strcmp_(cmd, "help") == 0) {
            puts_("ls [path]  - list directory\n");
            puts_("cat <file> - show file\n");
            puts_("echo <txt> - print text\n");
            puts_("sync       - sync filesystem\n");
            puts_("poweroff   - power off\n");
        } else if (strcmp_(cmd, "ls") == 0) {
            char *path = next_token(&p);
            list_dir(path ? path : "/");
        } else if (strcmp_(cmd, "cat") == 0) {
            char *path = next_token(&p);
            if (path) cat_file(path); else puts_("usage: cat <file>\n");
        } else if (strcmp_(cmd, "echo") == 0) {
            puts_(p); puts_("\n");
        } else if (strcmp_(cmd, "sync") == 0) {
            syscall(SYS_sync); puts_("synced\n");
        } else if (strcmp_(cmd, "poweroff") == 0) {
            syscall(SYS_sync); puts_("powering off...\n");
            syscall(SYS_reboot, 0xfee1dead, 672274793, 0x4321fedc, 0);
        } else if (strcmp_(cmd, "mount") == 0) {
            ret = syscall(SYS_mount, "/dev/vda", "/mnt", "ext2", 0, 0);
            puts_(ret == 0 ? "mounted\n" : "mount failed\n");
        } else {
            puts_("unknown: "); puts_(cmd); puts_("\n");
        }
    }
}
