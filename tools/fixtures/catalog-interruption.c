// Single-threaded stock caller. Observe real mutations and terminate
// without engine teardown at one recorded before/after boundary. No write loss.
#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "native-interpose.h"
NATIVE_BIND(write);
NATIVE_BIND(pwrite);
NATIVE_BIND(rename);
NATIVE_BIND(unlink);

static unsigned active, target, sequence;
static int trace_fd;
void interruption_start(int fd, unsigned cut) {
    if (active || fd < 0 || cut > 4096) _exit(90);
    trace_fd = fd; target = cut; sequence = 0; active = 1;
}
void interruption_stop(void) { active = 0; }
static const char *role(const char *path) {
    const char *name = strrchr(path, '/');
    name = name ? name + 1 : path;
    if (!strcmp(name, "ROOT.A")) return "A";
    if (!strcmp(name, "ROOT.B")) return "B";
    return "other";
}
static void event(const char *kind, const char *phase, const char *name) {
    if (!active) return;
    if (++sequence > 4096) _exit(91);
    char record[96];
    int n = snprintf(record, sizeof record, "%u %s %s %s\n", sequence, kind, phase, name);
    if (n <= 0 || (size_t)n >= sizeof record) _exit(92);
    // The observer's trace is outside the database and cannot count itself.
    active = 0;
    ssize_t wrote = NATIVE_REAL(write)(trace_fd, record, (size_t)n);
    active = 1;
    if (wrote != n) _exit(93);
    if (sequence == target) _exit(86);
}
static int regular(int fd) {
    if (!active) return 0;
    struct stat metadata;
    if (fstat(fd, &metadata) != 0) _exit(95);
    return S_ISREG(metadata.st_mode);
}
static ssize_t probe_write(int fd, const void *bytes, size_t count) {
    int observe = regular(fd);
    if (observe) event("write", "before", "other");
    ssize_t result = NATIVE_REAL(write)(fd, bytes, count);
    if (observe && result >= 0) event("write", "after", "other");
    return result;
}
static ssize_t probe_pwrite(int fd, const void *bytes, size_t count, off_t offset) {
    int observe = regular(fd);
    if (observe) event("pwrite", "before", "other");
    ssize_t result = NATIVE_REAL(pwrite)(fd, bytes, count, offset);
    if (observe && result >= 0) event("pwrite", "after", "other");
    return result;
}
static int probe_rename(const char *source, const char *destination) {
    event("rename", "before", role(destination));
    int result = NATIVE_REAL(rename)(source, destination);
    if (result == 0) event("rename", "after", role(destination));
    return result;
}
static int probe_unlink(const char *path) {
    event("unlink", "before", "other");
    int result = NATIVE_REAL(unlink)(path);
    if (result == 0) event("unlink", "after", "other");
    return result;
}
#if defined(__APPLE__)
static int probe_fcntl(int fd, int command, ...) {
    if (command == F_FULLFSYNC) {
        event("sync", "before", "other");
        int result = fcntl(fd, command);
        if (result == 0) event("sync", "after", "other");
        return result;
    }
    switch (command) {
        case F_GETPATH: {
            va_list args; va_start(args, command);
            char *value = va_arg(args, char *); va_end(args);
            return fcntl(fd, command, value);
        }
        case F_GETFD: case F_GETFL: case F_GETOWN:
            return fcntl(fd, command);
        case F_SETFD: case F_SETFL: case F_SETOWN:
        case F_DUPFD: case F_DUPFD_CLOEXEC: {
            va_list args; va_start(args, command);
            int value = va_arg(args, int); va_end(args);
            return fcntl(fd, command, value);
        }
        default: _exit(94); // Never guess an unknown variadic ABI.
    }
}
#else
NATIVE_BIND(fsync);
static int probe_fsync(int fd) {
    event("sync", "before", "other");
    int result = NATIVE_REAL(fsync)(fd);
    if (result == 0) event("sync", "after", "other");
    return result;
}
#endif
NATIVE_INTERPOSE(probe_write, write);
NATIVE_INTERPOSE(probe_pwrite, pwrite);
NATIVE_INTERPOSE(probe_rename, rename);
NATIVE_INTERPOSE(probe_unlink, unlink);
#if defined(__APPLE__)
NATIVE_INTERPOSE(probe_fcntl, fcntl);
#else
NATIVE_INTERPOSE(probe_fsync, fsync);
_Static_assert(sizeof(off64_t) == sizeof(off_t), "large-file offset ABI");
NATIVE_INTERPOSE(probe_pwrite, pwrite64);
#endif
