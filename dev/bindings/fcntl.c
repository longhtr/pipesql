// Forward Darwin's variadic fcntl ABI without guessing an argument's type.
// Rust records synchronization events; this bridge performs no allocation.

#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <unistd.h>

extern void pipesql_sync_event(unsigned after);
extern int pipesql_sync_refuse(void);
extern void pipesql_sync_weak(void);

static int observed_fcntl(int fd, int command, ...) {
    if (command == F_FULLFSYNC) {
        if (pipesql_sync_refuse()) return -1;
        int before = errno;
        pipesql_sync_event(0);
        errno = before;
        int result = fcntl(fd, command);
        int after = errno;
        if (result == 0) pipesql_sync_event(1);
        errno = after;
        return result;
    }
    if (command == F_BARRIERFSYNC) {
        pipesql_sync_weak();
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
        default: _exit(94);
    }
}

__attribute__((used, section("__DATA,__interpose")))
static const struct { const void *replacement; const void *original; } pair = {
    (const void *)(uintptr_t)&observed_fcntl,
    (const void *)(uintptr_t)&fcntl
};
