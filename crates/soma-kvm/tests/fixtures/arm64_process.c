#define _GNU_SOURCE
#include "arm64_process.h"

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define CHUNK 4096

static int write_exact(int fd, const void *data, size_t length) {
    const unsigned char *bytes = data;
    while (length) {
        ssize_t count = write(fd, bytes, length);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) return -1;
        bytes += count;
        length -= (size_t)count;
    }
    return 0;
}

struct child_status { int kind; int error; };

static int read_child_status(int fd, struct child_status *status) {
    size_t length = 0;
    while (length < sizeof(*status)) {
        ssize_t count = read(fd, (unsigned char *)status + length, sizeof(*status) - length);
        if (count < 0 && errno == EINTR) continue;
        if (count < 0) return -1;
        if (count == 0) return length == 0 ? 0 : -1;
        length += (size_t)count;
    }
    unsigned char extra;
    ssize_t count;
    do count = read(fd, &extra, 1); while (count < 0 && errno == EINTR);
    return count == 0 ? 1 : -1;
}

static _Noreturn void child_failure(int fd, int kind, int error) {
    if (error <= 0) error = EIO;
    struct child_status status = {kind, error};
    (void)write_exact(fd, &status, sizeof(status));
    _exit(126);
}

static int set_nonblocking(int fd) {
    int flags = fcntl(fd, F_GETFL);
    return flags < 0 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0 ? -1 : 0;
}

int ensure_standard_descriptors(void) {
    for (int target = 0; target <= 2; target++) {
        if (fcntl(target, F_GETFD) >= 0) continue;
        if (errno != EBADF) return -1;
        int access = target == 0 ? O_RDONLY : O_WRONLY;
        int source = open("/dev/null", access | O_CLOEXEC);
        if (source < 0) return -1;
        if (source != target) {
            if (dup3(source, target, O_CLOEXEC) < 0) {
                int saved = errno;
                close(source);
                errno = saved;
                return -1;
            }
            close(source);
        }
    }
    return 0;
}

static uint64_t monotonic_ms(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return UINT64_MAX;
    return (uint64_t)now.tv_sec * 1000 + (uint64_t)now.tv_nsec / 1000000;
}

static int reap_all(void) {
    for (;;) {
        pid_t child = waitpid(-1, NULL, 0);
        if (child > 0 || (child < 0 && errno == EINTR)) continue;
        return child < 0 && errno == ECHILD ? 0 : -1;
    }
}

static void drain_close(int fd) {
    unsigned char bytes[CHUNK];
    while (fd >= 0 && read(fd, bytes, sizeof(bytes)) > 0) {}
    if (fd >= 0) close(fd);
}

static int stop_group(pid_t leader, int leader_reaped, int out_fd, int err_fd) {
    int failure = 0;
    if (kill(-leader, SIGKILL) < 0 && errno != ESRCH) failure = -1;
    if (!leader_reaped) {
        if (kill(leader, SIGKILL) < 0 && errno != ESRCH) failure = -1;
        pid_t waited;
        do waited = waitpid(leader, NULL, 0); while (waited < 0 && errno == EINTR);
        if (waited != leader && !(waited < 0 && errno == ECHILD)) failure = -1;
    }
    if (reap_all() != 0) failure = -1;
    drain_close(out_fd);
    drain_close(err_fd);
    return failure;
}

static int append_output(struct result *result, int stream, const unsigned char *data,
                         size_t length, uint32_t limit) {
    size_t used = result->out_len + result->err_len;
    size_t retained = length < limit - used ? length : limit - used;
    unsigned char *target = stream == 0 ? result->out : result->err;
    size_t *target_len = stream == 0 ? &result->out_len : &result->err_len;
    for (size_t index = 0; index < retained; index++) target[*target_len + index] = data[index];
    *target_len += retained;
    return retained != length;
}

static int establish_group(pid_t child) {
    if (setpgid(child, child) == 0) return 0;
    return getpgid(child) == child ? 0 : -1;
}

static _Noreturn void child_exec(int control, int out[2], int err[2], int status[2],
                                 const struct request *request) {
    close(out[0]); close(err[0]); close(status[0]); close(control);
    if (setpgid(0, 0) < 0) child_failure(status[1], 1, errno);
    int null_fd = open("/dev/null", O_RDONLY | O_CLOEXEC);
    if (null_fd < 0 || dup2(null_fd, 0) < 0 || dup2(out[1], 1) < 0 || dup2(err[1], 2) < 0)
        child_failure(status[1], 1, errno);
    if (null_fd > 2) close(null_fd);
    if (out[1] > 2) close(out[1]);
    if (err[1] > 2) close(err[1]);
    char *environment[] = {NULL};
    execve(request->program, request->argv, environment);
    child_failure(status[1], 2, errno);
}

static int wait_for_output(pid_t child, const struct request *request, struct result *result,
                           int fds[2], int *status, int *leader_reaped) {
    uint64_t started = monotonic_ms();
    if (started == UINT64_MAX) return -1;
    for (;;) {
        if (!*leader_reaped) {
            pid_t waited = waitpid(child, status, WNOHANG);
            if (waited < 0 && errno != EINTR) return -1;
            if (waited == child) {
                *leader_reaped = 1;
                if (kill(-child, SIGKILL) < 0 && errno != ESRCH) return -1;
                if (reap_all() != 0) return -1;
            }
        }
        if (*leader_reaped && fds[0] < 0 && fds[1] < 0) return 0;
        uint64_t now = monotonic_ms();
        if (now == UINT64_MAX || now - started >= request->timeout_ms) {
            result->terminal = 2;
            return 1;
        }
        struct pollfd polls[2] = {{fds[0], POLLIN | POLLHUP, 0}, {fds[1], POLLIN | POLLHUP, 0}};
        int wait_ms = (int)(request->timeout_ms - (now - started));
        if (wait_ms > 10) wait_ms = 10;
        int polled = poll(polls, 2, wait_ms);
        if (polled < 0 && errno != EINTR) return -1;
        for (int stream = 0; stream < 2; stream++) {
            if (polls[stream].revents & (POLLERR | POLLNVAL)) return -1;
            if (fds[stream] < 0 || !(polls[stream].revents & (POLLIN | POLLHUP))) continue;
            for (;;) {
                unsigned char bytes[CHUNK];
                ssize_t count = read(fds[stream], bytes, sizeof(bytes));
                if (count > 0 && append_output(result, stream, bytes, count, request->limit)) {
                    result->terminal = 3;
                    return 1;
                }
                if (count == 0) { close(fds[stream]); fds[stream] = -1; break; }
                if (count < 0 && errno != EAGAIN && errno != EINTR) return -1;
                if (count < 0) break;
            }
        }
    }
}

int run_child(int control, const struct request *request, struct result *result) {
    if (control <= 2 || ensure_standard_descriptors() != 0) return -1;
    int out[2], err[2], exec_status[2];
    if (pipe2(out, O_CLOEXEC) || pipe2(err, O_CLOEXEC) || pipe2(exec_status, O_CLOEXEC)) return -1;
    result->out = malloc(request->limit); result->err = malloc(request->limit);
    if (!result->out || !result->err) return -1;
    pid_t child = fork();
    if (child < 0) return -1;
    if (child == 0) child_exec(control, out, err, exec_status, request);
    close(out[1]); close(err[1]); close(exec_status[1]);
    if (establish_group(child) || set_nonblocking(out[0]) || set_nonblocking(err[0])) {
        (void)stop_group(child, 0, out[0], err[0]); close(exec_status[0]); return -1;
    }
    int fds[2] = {out[0], err[0]}, status = 0, leader_reaped = 0;
    int stopped = wait_for_output(child, request, result, fds, &status, &leader_reaped);
    if (stopped != 0 && stop_group(child, leader_reaped, fds[0], fds[1]) != 0) stopped = -1;
    if (stopped == 0 && !leader_reaped) {
        pid_t waited;
        do waited = waitpid(child, &status, 0); while (waited < 0 && errno == EINTR);
        if (waited != child) stopped = -1;
    }
    struct child_status child_status = {0};
    int exec_result = read_child_status(exec_status[0], &child_status);
    close(exec_status[0]);
    if (stopped < 0 || exec_result < 0) return -1;
    if (stopped > 0) return 0;
    if (exec_result == 1 && child_status.kind == 2) {
        result->terminal = 4; result->value = child_status.error;
    } else if (exec_result == 1 && child_status.kind == 1) {
        result->terminal = 5; result->value = child_status.error;
    } else if (exec_result == 1) return -1;
    else if (WIFEXITED(status)) { result->terminal = 0; result->value = WEXITSTATUS(status); }
    else if (WIFSIGNALED(status)) { result->terminal = 1; result->value = WTERMSIG(status); }
    else return -1;
    return 0;
}
