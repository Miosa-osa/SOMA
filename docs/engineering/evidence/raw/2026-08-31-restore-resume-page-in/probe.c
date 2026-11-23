#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

/* Touch `pages` pages of `base`, either reading or writing one byte each.
   `stride_pages` walks the mapping so a scattered access pattern can be asked for. */
volatile unsigned long g_sink;
static unsigned long touch(char *base, size_t len, size_t pages, size_t stride_pages, int write) {
    size_t page = 4096, total = len / page, seen = 0;
    unsigned long sink = 0;
    for (size_t i = 0; seen < pages && i < total * 4; i++) {
        size_t idx = (i * stride_pages) % total;
        if (write) base[idx * page] = (char)(i & 0xff);
        else sink += (unsigned char)base[idx * page];
        seen++;
    }
    return sink;
}

int main(int argc, char **argv) {
    if (argc < 6) {
        fprintf(stderr, "usage: probe FILE MODE MIB STRIDE_PAGES read|write\n");
        return 2;
    }
    const char *path = argv[1], *mode = argv[2];
    size_t mib = (size_t)atoll(argv[3]);
    size_t stride = (size_t)atoll(argv[4]);
    int write = strcmp(argv[5], "write") == 0;

    int fd = open(path, O_RDONLY);
    if (fd < 0) { perror("open"); return 1; }
    struct stat st;
    if (fstat(fd, &st) < 0) { perror("fstat"); return 1; }
    size_t len = (size_t)st.st_size;

    int flags = MAP_PRIVATE | MAP_NORESERVE;
    if (strcmp(mode, "populate") == 0) flags |= MAP_POPULATE;

    long long t0 = now_ns();
    char *base = mmap(NULL, len, PROT_READ | PROT_WRITE, flags, fd, 0);
    if (base == MAP_FAILED) { perror("mmap"); return 1; }
    if (strcmp(mode, "hugepage") == 0 && madvise(base, len, MADV_HUGEPAGE) != 0) perror("madvise");
    if (strcmp(mode, "willneed") == 0 && madvise(base, len, MADV_WILLNEED) != 0) perror("madvise");
    long long t1 = now_ns();

    struct rusage before, after;
    getrusage(RUSAGE_SELF, &before);
    long long t2 = now_ns();
    g_sink = touch(base, len, mib * 256, stride, write);
    long long t3 = now_ns();
    getrusage(RUSAGE_SELF, &after);

    size_t pages = mib * 256;
    long faults = after.ru_minflt - before.ru_minflt;
    printf("{\"mode\":\"%s\",\"access\":\"%s\",\"stride_pages\":%zu,\"mapped_mib\":%zu,"
           "\"touched_mib\":%zu,\"map_ns\":%lld,\"touch_ns\":%lld,\"minor_faults\":%ld,"
           "\"ns_per_page\":%.2f,\"ns_per_fault\":%.2f}\n",
           mode, write ? "write" : "read", stride, len >> 20, mib, t1 - t0, t3 - t2,
           faults, (double)(t3 - t2) / (double)pages,
           faults ? (double)(t3 - t2) / (double)faults : 0.0);
    munmap(base, len);
    close(fd);
    return 0;
}
