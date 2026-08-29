#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <signal.h>
#include <string.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

static int write_all(int fd, const void *data, size_t length) {
    const char *bytes = data;
    while (length != 0) {
        ssize_t written = write(fd, bytes, length);
        if (written < 0 && errno == EINTR) continue;
        if (written <= 0) return -1;
        bytes += written;
        length -= (size_t)written;
    }
    return 0;
}

static int write_repeated(int fd, size_t length, char value) {
    char block[256];
    memset(block, value, sizeof(block));
    while (length) {
        size_t count = length < sizeof(block) ? length : sizeof(block);
        if (write_all(fd, block, count) != 0) return -1;
        length -= count;
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 2) return 2;
    if (strcmp(argv[1], "argv") == 0) {
        for (int index = 0; index < argc; index++) {
            if (write_all(STDOUT_FILENO, argv[index], strlen(argv[index]) + 1) != 0) return 3;
        }
        return write_all(STDERR_FILENO, "probe-stderr\n", 13) == 0 ? 0 : 3;
    }
    if (strcmp(argv[1], "exit") == 0) return 7;
    if (strcmp(argv[1], "signal") == 0) {
        raise(SIGTERM);
        return 4;
    }
    if (strcmp(argv[1], "sleep") == 0) {
        for (;;) pause();
    }
    if (strcmp(argv[1], "binary") == 0) {
        unsigned char bytes[256];
        for (int index = 0; index < 256; index++) bytes[index] = (unsigned char)index;
        return write_all(STDOUT_FILENO, bytes, sizeof(bytes)) == 0 ? 0 : 3;
    }
    if (strcmp(argv[1], "exact") == 0)
        return write_repeated(STDOUT_FILENO, 1024, 'x') == 0 ? 0 : 3;
    if (strcmp(argv[1], "maximum") == 0)
        return write_repeated(STDOUT_FILENO, 64 * 1024, 'm') == 0 ? 0 : 3;
    if (strcmp(argv[1], "one-over") == 0)
        return write_repeated(STDOUT_FILENO, 1025, 'x') == 0 ? 0 : 3;
    if (strcmp(argv[1], "combined") == 0) {
        if (write_repeated(STDOUT_FILENO, 512, 'o')) return 3;
        return write_repeated(STDERR_FILENO, 513, 'e') == 0 ? 0 : 3;
    }
    if (strcmp(argv[1], "delayed") == 0) {
        struct timespec delay = {.tv_sec = 0, .tv_nsec = 50 * 1000 * 1000};
        if (write_all(STDOUT_FILENO, "a", 1) != 0) return 3;
        nanosleep(&delay, NULL);
        return write_all(STDOUT_FILENO, "b", 1) == 0 ? 0 : 3;
    }
    if (strcmp(argv[1], "descendant") == 0) {
        pid_t child = fork();
        if (child < 0) return 5;
        if (child == 0) for (;;) pause();
        return 0;
    }
    if (strcmp(argv[1], "closed") == 0) {
        close(STDOUT_FILENO);
        close(STDERR_FILENO);
        for (;;) pause();
    }
    return 2;
}
