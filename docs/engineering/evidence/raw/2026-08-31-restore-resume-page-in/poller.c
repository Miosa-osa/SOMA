/* Samples one process's minor-fault counter often enough to see a 30 ms segment.
 *
 * `/proc/<pid>/stat` field 10 is the cumulative minor-fault count. Reading it is one short
 * read of one line, which is cheap enough to take every few hundred microseconds without
 * distorting the process being watched. */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

static long minflt(int fd, char *buf, size_t cap) {
    if (pread(fd, buf, cap - 1, 0) <= 0) return -1;
    buf[cap - 1] = 0;
    char *close_paren = strrchr(buf, ')');
    if (!close_paren) return -1;
    char *cursor = close_paren + 2;
    for (int field = 3; field < 10; field++) {
        cursor = strchr(cursor, ' ');
        if (!cursor) return -1;
        cursor++;
    }
    return atol(cursor);
}

int main(int argc, char **argv) {
    if (argc < 4) {
        fprintf(stderr, "usage: poller PID INTERVAL_US MAX_MS\n");
        return 2;
    }
    char path[64];
    snprintf(path, sizeof path, "/proc/%s/stat", argv[1]);
    long interval_us = atol(argv[2]), max_ms = atol(argv[3]);
    int fd = open(path, O_RDONLY);
    if (fd < 0) { perror("open"); return 1; }

    char buf[1024];
    long long start = now_ns(), deadline = start + max_ms * 1000000LL;
    long previous = -1;
    while (now_ns() < deadline) {
        long value = minflt(fd, buf, sizeof buf);
        if (value < 0) break;
        if (value != previous) {
            printf("%lld %ld\n", now_ns() - start, value);
            previous = value;
        }
        struct timespec sleep_for = {0, interval_us * 1000};
        nanosleep(&sleep_for, NULL);
    }
    printf("%lld %ld\n", now_ns() - start, previous);
    close(fd);
    return 0;
}
