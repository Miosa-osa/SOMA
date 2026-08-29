/*
 * SOMA x86_64 acceptance-test PID 1.
 *
 * Reads the `soma.nonce=<hex>` argument from /proc/cmdline, writes the challenge-bound line
 * `SOMA-BOOT-<hex>` to the kernel console (ttyS0 through the 8250 driver), drains it, and asks
 * the kernel to restart. With `reboot=k` the kernel pulses the keyboard-controller reset line,
 * which the SOMA machine reports as an orderly reset. Freestanding, static, no libc.
 */

typedef unsigned long usize;
typedef long isize;

#define SYS_read 0
#define SYS_write 1
#define SYS_open 2
#define SYS_close 3
#define SYS_ioctl 16
#define SYS_nanosleep 35
#define SYS_pause 34
#define SYS_mount 165
#define SYS_reboot 169

#define O_RDONLY 0
#define O_WRONLY 1
#define TCSBRK 0x5409
#define LINUX_REBOOT_MAGIC1 0xfee1deadUL
#define LINUX_REBOOT_MAGIC2 672274793UL
#define LINUX_REBOOT_CMD_RESTART 0x01234567UL

#define NONCE_MAX 64
#define CMDLINE_MAX 4096

static isize sys6(usize n, usize a, usize b, usize c, usize d, usize e, usize f) {
    isize ret;
    register usize r10 __asm__("r10") = d;
    register usize r8 __asm__("r8") = e;
    register usize r9 __asm__("r9") = f;
    __asm__ volatile("syscall"
                     : "=a"(ret)
                     : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory");
    return ret;
}

static isize sys3(usize n, usize a, usize b, usize c) { return sys6(n, a, b, c, 0, 0, 0); }

static usize str_len(const char *s) {
    usize n = 0;
    while (s[n] != 0) {
        n++;
    }
    return n;
}

static isize write_all(int fd, const char *buf, usize len) {
    usize done = 0;
    while (done < len) {
        isize n = sys3(SYS_write, (usize)fd, (usize)(buf + done), len - done);
        if (n <= 0) {
            return n;
        }
        done += (usize)n;
    }
    return (isize)done;
}

/* Copies the value after `soma.nonce=` into `out`; returns its length or 0. */
static usize find_nonce(const char *cmdline, usize len, char *out) {
    static const char key[] = "soma.nonce=";
    usize key_len = sizeof(key) - 1;
    for (usize i = 0; i + key_len <= len; i++) {
        usize k = 0;
        while (k < key_len && cmdline[i + k] == key[k]) {
            k++;
        }
        if (k != key_len) {
            continue;
        }
        usize n = 0;
        usize p = i + key_len;
        while (p < len && n < NONCE_MAX) {
            char c = cmdline[p];
            if (c == ' ' || c == '\n' || c == '\r' || c == '\t' || c == 0) {
                break;
            }
            out[n++] = c;
            p++;
        }
        return n;
    }
    return 0;
}

static int console_fd(void) {
    static const char *paths[] = {"/dev/console", "/dev/ttyS0"};
    for (usize i = 0; i < 2; i++) {
        isize fd = sys3(SYS_open, (usize)paths[i], O_WRONLY, 0);
        if (fd >= 0) {
            return (int)fd;
        }
    }
    return 1;
}

void soma_init(void) {
    static char cmdline[CMDLINE_MAX];
    static char line[16 + NONCE_MAX + 2];
    static const char prefix[] = "SOMA-BOOT-";
    static const char missing[] = "SOMA-BOOT-MISSING-NONCE\n";
    static const char proc[] = "proc";
    static const char proc_dir[] = "/proc";
    static const char proc_cmdline[] = "/proc/cmdline";
    struct {
        long sec;
        long nsec;
    } delay = {0, 20000000L};

    sys6(SYS_mount, (usize)proc, (usize)proc_dir, (usize)proc, 0, 0, 0);
    usize cmdline_len = 0;
    isize fd = sys3(SYS_open, (usize)proc_cmdline, O_RDONLY, 0);
    if (fd >= 0) {
        isize n = sys3(SYS_read, (usize)fd, (usize)cmdline, CMDLINE_MAX);
        if (n > 0) {
            cmdline_len = (usize)n;
        }
        sys3(SYS_close, (usize)fd, 0, 0);
    }

    int out = console_fd();
    char nonce[NONCE_MAX];
    usize nonce_len = find_nonce(cmdline, cmdline_len, nonce);
    if (nonce_len == 0) {
        write_all(out, missing, sizeof(missing) - 1);
    } else {
        usize pos = 0;
        for (usize i = 0; i < str_len(prefix); i++) {
            line[pos++] = prefix[i];
        }
        for (usize i = 0; i < nonce_len; i++) {
            line[pos++] = nonce[i];
        }
        line[pos++] = '\n';
        write_all(out, line, pos);
    }
    sys3(SYS_ioctl, (usize)out, TCSBRK, 1);
    sys3(SYS_nanosleep, (usize)&delay, 0, 0);
    sys6(SYS_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2, LINUX_REBOOT_CMD_RESTART, 0, 0, 0);
    for (;;) {
        sys3(SYS_pause, 0, 0, 0);
    }
}

__asm__(".section .text\n"
        ".globl _start\n"
        "_start:\n"
        "    xor %rbp, %rbp\n"
        "    and $-16, %rsp\n"
        "    call soma_init\n"
        "1:  pause\n"
        "    jmp 1b\n");
