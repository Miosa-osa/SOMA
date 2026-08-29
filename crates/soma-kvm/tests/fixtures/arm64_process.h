#ifndef SOMA_ARM64_PROCESS_H
#define SOMA_ARM64_PROCESS_H

#include <stddef.h>
#include <stdint.h>

struct request {
    uint32_t timeout_ms;
    uint32_t limit;
    char *program;
    char **argv;
};

struct result {
    unsigned char *out;
    unsigned char *err;
    size_t out_len;
    size_t err_len;
    int terminal;
    int value;
};

int run_child(int control, const struct request *request, struct result *result);
int ensure_standard_descriptors(void);

#endif
