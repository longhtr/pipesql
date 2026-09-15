// Observe synchronization latency in disposable verification callers. Every call
// reaches the native implementation; this observer neither refuses nor weakens it.
// Time covers the native call, not the atomic bookkeeping or final diagnostic.
// Concurrent calls contribute summed time, which may exceed process wall time.
// Only known fcntl signatures are forwarded; unexpected commands fail closed.
// tools/README.md owns the command and stock-versus-observed result comparison.

#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>

#include "native-interpose.h"

static _Atomic uint64_t total_ns, total_calls;
static _Atomic uint64_t other_calls;

static uint64_t now_ns(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t) != 0) _exit(97);
    return (uint64_t)t.tv_sec * UINT64_C(1000000000) + (uint64_t)t.tv_nsec;
}

static void record(uint64_t start, int saved_errno) {
    atomic_fetch_add(&total_ns, now_ns() - start);
    atomic_fetch_add(&total_calls, 1);
    errno = saved_errno;
}

__attribute__((destructor)) static void report(void) {
    fprintf(stderr, "sync_timing calls=%llu nanoseconds=%llu other_calls=%llu\n",
        (unsigned long long)atomic_load(&total_calls),
        (unsigned long long)atomic_load(&total_ns),
        (unsigned long long)atomic_load(&other_calls));
}

#if defined(__APPLE__)
static int probe_fcntl(int fd, int command, ...) {
    if (command == F_FULLFSYNC) {
        uint64_t start = now_ns();
        int result = fcntl(fd, command);
        int saved_errno = errno;
        record(start, saved_errno);
        return result;
    }
    if (command == F_BARRIERFSYNC) {
        atomic_fetch_add(&other_calls, 1);
        return fcntl(fd, command);
    }
    switch (command) {
        case F_GETPATH: {
            va_list args;
            va_start(args, command);
            char *value = va_arg(args, char *);
            va_end(args);
            return fcntl(fd, command, value);
        }
        case F_GETFD: case F_GETFL: case F_GETOWN:
            return fcntl(fd, command);
        case F_SETFD: case F_SETFL: case F_SETOWN:
        case F_DUPFD: case F_DUPFD_CLOEXEC: {
            va_list args;
            va_start(args, command);
            int value = va_arg(args, int);
            va_end(args);
            return fcntl(fd, command, value);
        }
        default: {
            char message[64];
            int length = snprintf(message, sizeof message, "unexpected fcntl command=%d\n", command);
            if (length > 0 && (size_t)length < sizeof message) {
                // Diagnostic only; exit status remains decisive if the sink fails.
                (void)write(STDERR_FILENO, message, (size_t)length);
            }
            _exit(92);
        }
    }
}

static int probe_fsync(int fd) {
    atomic_fetch_add(&other_calls, 1);
    return fsync(fd);
}

NATIVE_INTERPOSE(probe_fcntl, fcntl);
NATIVE_INTERPOSE(probe_fsync, fsync);
#else
NATIVE_BIND(fsync);
NATIVE_BIND(fdatasync);

static int probe_fsync(int fd) {
    uint64_t start = now_ns();
    int result = NATIVE_REAL(fsync)(fd);
    int saved_errno = errno;
    record(start, saved_errno);
    return result;
}

static int probe_fdatasync(int fd) {
    atomic_fetch_add(&other_calls, 1);
    return NATIVE_REAL(fdatasync)(fd);
}
NATIVE_INTERPOSE(probe_fsync, fsync);
NATIVE_INTERPOSE(probe_fdatasync, fdatasync);
#endif
