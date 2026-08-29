#define _GNU_SOURCE
#include "arm64_process.h"

#include <fcntl.h>
#include <unistd.h>

int main(void) {
    close(0);
    close(1);
    close(2);
    if (ensure_standard_descriptors() != 0) return 1;
    for (int descriptor = 0; descriptor <= 2; descriptor++) {
        int flags = fcntl(descriptor, F_GETFD);
        if (flags < 0 || !(flags & FD_CLOEXEC)) return 2;
    }
    int pair[2];
    if (pipe2(pair, O_CLOEXEC) != 0) return 3;
    if (pair[0] <= 2 || pair[1] <= 2) return 4;
    return 0;
}
