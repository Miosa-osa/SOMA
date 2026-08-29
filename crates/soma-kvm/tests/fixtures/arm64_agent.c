#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <termios.h>
#include <unistd.h>

#include "arm64_process.h"

#define HEADER 64
#define MAX_PAYLOAD (64 * 1024)
#define CHUNK 4096

struct identity { uint64_t request_id; unsigned char challenge[32]; };

static uint16_t get16(const unsigned char *p) { return (uint16_t)(p[0] << 8 | p[1]); }
static uint32_t get32(const unsigned char *p) {
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3];
}
static uint64_t get64(const unsigned char *p) {
    return (uint64_t)get32(p) << 32 | get32(p + 4);
}
static void put32(unsigned char *p, uint32_t v) {
    p[0] = v >> 24; p[1] = v >> 16; p[2] = v >> 8; p[3] = v;
}
static void put64(unsigned char *p, uint64_t v) { put32(p, v >> 32); put32(p + 4, v); }

static uint32_t crc_part(uint32_t crc, const unsigned char *data, size_t length) {
    for (size_t i = 0; i < length; i++) {
        crc ^= data[i];
        for (int bit = 0; bit < 8; bit++) crc = (crc >> 1) ^ (0x82f63b78U & -(crc & 1U));
    }
    return crc;
}
static uint32_t checksum(const unsigned char *head, const unsigned char *data, size_t length) {
    return ~crc_part(crc_part(~0U, head, 60), data, length);
}
static int write_all(int fd, const void *data, size_t length) {
    const unsigned char *bytes = data;
    while (length != 0) {
        ssize_t count = write(fd, bytes, length);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) return -1;
        bytes += count; length -= (size_t)count;
    }
    return 0;
}
static int read_all(int fd, void *data, size_t length) {
    unsigned char *bytes = data;
    while (length != 0) {
        ssize_t count = read(fd, bytes, length);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) return -1;
        bytes += count; length -= (size_t)count;
    }
    return 0;
}

static int fail_stage(const char *tag, size_t length, int code) {
    (void)write_all(2, "SOMA_AGENT_FAIL:", 16);
    (void)write_all(2, tag, length);
    (void)write_all(2, "\n", 1);
    return code;
}

#define FAIL_STAGE(tag, code) fail_stage(tag, sizeof(tag) - 1, code)

static int send_frame(int fd, int kind, const struct identity *id, uint32_t sequence,
                      const void *payload, uint32_t length) {
    unsigned char head[HEADER] = {0};
    memcpy(head, "SMAC", 4); head[4] = 1; head[5] = HEADER; head[6] = kind;
    if (id) { put64(head + 12, id->request_id); memcpy(head + 28, id->challenge, 32); }
    put32(head + 20, sequence); put32(head + 24, length);
    put32(head + 60, checksum(head, payload, length));
    return write_all(fd, head, sizeof(head)) || write_all(fd, payload, length) ? -1 : 0;
}

static unsigned char *receive_request(int fd, struct identity *id, uint32_t *length) {
    unsigned char head[HEADER];
    if (read_all(fd, head, sizeof(head)) != 0) return NULL;
    if (memcmp(head, "SMAC", 4) || head[4] != 1 || head[5] != HEADER || head[6] != 2) return NULL;
    for (int i = 7; i < 12; i++) if (head[i] != 0) return NULL;
    if (get32(head + 20) != 0 || (*length = get32(head + 24)) > MAX_PAYLOAD) return NULL;
    id->request_id = get64(head + 12); memcpy(id->challenge, head + 28, 32);
    unsigned char zero[32] = {0};
    if (id->request_id == 0 || memcmp(id->challenge, zero, 32) == 0) return NULL;
    unsigned char *payload = malloc(*length ? *length : 1);
    if (!payload || read_all(fd, payload, *length) != 0) { free(payload); return NULL; }
    if (checksum(head, payload, *length) != get32(head + 60)) { free(payload); return NULL; }
    return payload;
}

static char *take_string(const unsigned char *data, size_t length, size_t *offset) {
    if (*offset + 2 > length) return NULL;
    size_t size = get16(data + *offset); *offset += 2;
    if (size > 4096 || *offset + size > length || memchr(data + *offset, 0, size)) return NULL;
    char *value = malloc(size + 1);
    if (!value) return NULL;
    memcpy(value, data + *offset, size); value[size] = 0; *offset += size;
    return value;
}

static int parse_request(unsigned char *data, size_t length, struct request *request) {
    if (length < 12) return -1;
    request->timeout_ms = get32(data); request->limit = get32(data + 4);
    if (!request->timeout_ms || request->timeout_ms > 30000 || !request->limit || request->limit > 65536) return -1;
    size_t offset = 8;
    request->program = take_string(data, length, &offset);
    if (!request->program || request->program[0] != '/' || offset + 2 > length) return -1;
    uint16_t count = get16(data + offset); offset += 2;
    if (count > 64) return -1;
    request->argv = calloc((size_t)count + 2, sizeof(char *));
    if (!request->argv) return -1;
    request->argv[0] = request->program;
    for (uint16_t i = 0; i < count; i++) if (!(request->argv[i + 1] = take_string(data, length, &offset))) return -1;
    return offset == length ? 0 : -1;
}

static int send_result(int fd, const struct identity *id, const struct result *result) {
    uint32_t sequence = 0;
    for (size_t offset = 0; offset < result->out_len; offset += CHUNK) {
        uint32_t size = result->out_len - offset < CHUNK ? result->out_len - offset : CHUNK;
        if (send_frame(fd, 3, id, sequence++, result->out + offset, size)) return -1;
    }
    for (size_t offset = 0; offset < result->err_len; offset += CHUNK) {
        uint32_t size = result->err_len - offset < CHUNK ? result->err_len - offset : CHUNK;
        if (send_frame(fd, 4, id, sequence++, result->err + offset, size)) return -1;
    }
    unsigned char terminal[16] = {0}; terminal[0] = result->terminal;
    put32(terminal + 4, result->value); put32(terminal + 8, result->out_len); put32(terminal + 12, result->err_len);
    return send_frame(fd, 5, id, sequence, terminal, sizeof(terminal));
}

int main(void) {
    mkdir("/dev", 0755);
    if (mount("devtmpfs", "/dev", "devtmpfs", 0, NULL) && errno != EBUSY)
        return FAIL_STAGE("mount-devtmpfs", 100);
    if (ensure_standard_descriptors() != 0) return FAIL_STAGE("standard-fds", 99);
    (void)write_all(1, "SOMA_ARM64_OK", 13);
    int control = open("/dev/ttyS1", O_RDWR | O_NOCTTY | O_CLOEXEC);
    if (control < 0) return FAIL_STAGE("open-control", 101);
    struct termios tty;
    if (tcgetattr(control, &tty)) return FAIL_STAGE("read-termios", 102);
    cfmakeraw(&tty);
    tty.c_cflag = (tty.c_cflag & ~(CSIZE | CSTOPB | CRTSCTS)) | CS8 | CLOCAL | CREAD;
    if (tcsetattr(control, TCSANOW, &tty)) return FAIL_STAGE("write-termios", 103);
    if (unlink("/dev/ttyS1")) return FAIL_STAGE("unlink-control", 104);
    if (send_frame(control, 1, NULL, 0, NULL, 0)) return FAIL_STAGE("send-hello", 105);
    struct identity id; uint32_t length; unsigned char *payload = receive_request(control, &id, &length);
    struct request request = {0};
    if (!payload || parse_request(payload, length, &request)) return 106;
    struct result result = {0};
    if (run_child(control, &request, &result)) {
        result.terminal = 5; result.value = errno > 0 ? errno : EIO;
    }
    if (send_result(control, &id, &result)) return 107;
    for (;;) pause();
}
