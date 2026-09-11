// Disposable synchronization observer. Only the fixture's known fcntl signatures
// are forwarded; an unexpected command terminates rather than reading wrong varargs.
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>

static int active;
static unsigned calls, refused, weak, at, burst;
static int failure;

void sync_probe_start(unsigned position, unsigned count, int error) {
    if (active || position > 4096 || count > 16 || (error != EINTR && error != EIO && error != ENOTSUP)) _exit(90);
    calls = refused = weak = 0;
    at = position;
    burst = count;
    failure = error;
    active = 1;
}
void sync_probe_stop(void) { active = 0; }
unsigned sync_probe_calls(void) { return calls; }
unsigned sync_probe_refused(void) { return refused; }
unsigned sync_probe_weak(void) { return weak; }

static int deny_sync(void) {
    if (!active) return 0;
    if (++calls > 4096) _exit(91);
    if (at != 0 && calls >= at && calls - at < burst) {
        ++refused;
        errno = failure;
        return 1;
    }
    return 0;
}

#include "native-interpose.h"
#if defined(__APPLE__)
static int probe_fcntl(int fd, int command, ...) {
    if (command == F_FULLFSYNC) {
        if (deny_sync()) return -1;
        return fcntl(fd, command);
    }
    if (command == F_BARRIERFSYNC) {
        if (active) ++weak;
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
            int length = snprintf(message, sizeof message, "unexpected fcntl command=%d active=%d\n", command, active);
            if (length > 0 && (size_t)length < sizeof message) {
                // Diagnostic only; exit status remains decisive if the sink fails.
                (void)write(STDERR_FILENO, message, (size_t)length);
            }
            _exit(92);
        }
    }
}
static int probe_fsync(int fd) {
    if (active) ++weak;
    return fsync(fd);
}
NATIVE_INTERPOSE(probe_fcntl, fcntl);
NATIVE_INTERPOSE(probe_fsync, fsync);
#else
NATIVE_BIND(fsync);
NATIVE_BIND(fdatasync);
static int probe_fsync(int fd) {
    if (deny_sync()) return -1;
    return NATIVE_REAL(fsync)(fd);
}
static int probe_fdatasync(int fd) {
    if (active) ++weak;
    return NATIVE_REAL(fdatasync)(fd);
}
NATIVE_INTERPOSE(probe_fsync, fsync);
NATIVE_INTERPOSE(probe_fdatasync, fdatasync);
#endif
