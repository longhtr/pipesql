// Single-threaded disposable observer around regular-file native byte I/O.
#include <errno.h>
#include <stdint.h>
#include <sys/stat.h>
#include <unistd.h>

_Static_assert(sizeof(off_t) == 8, "probe requires signed 64-bit file offsets");
#include "native-interpose.h"
NATIVE_BIND(read);
NATIVE_BIND(write);
NATIVE_BIND(pread);
NATIVE_BIND(pwrite);

static unsigned selected, position, burst, calls, refused, partial;
static int active, failure, crossed;

void io_probe_start(unsigned kind, unsigned at, unsigned count, int error) {
    if (active || kind > 3 || at > 4096 || count > 16 ||
        (error != 0 && error != EINTR && error != EIO &&
         error != -EINTR && error != -EIO))
        _exit(90);
    selected = kind;
    position = at;
    burst = count;
    crossed = error < 0;
    failure = crossed ? -error : error;
    calls = refused = partial = 0;
    active = 1;
}

void io_probe_stop(void) {
    active = 0;
}

unsigned io_probe_calls(void) {
    return calls;
}

unsigned io_probe_refused(void) {
    return refused;
}

unsigned io_probe_partial(void) {
    return partial;
}

static int deny(unsigned kind, int fd) {
    if (!active || kind != selected) return 0;
    struct stat metadata;
    if (fstat(fd, &metadata) != 0) _exit(91);
    if (!S_ISREG(metadata.st_mode)) return 0; // Do not inject into diagnostic pipes.
    if (++calls > 4096) _exit(92);
    if (position != 0 && calls >= position) {
        unsigned distance = calls - position;
        if (crossed && distance == 0) return 2;
        if ((crossed && distance <= burst) || (!crossed && distance < burst)) {
            if (failure == 0) return 2; // Shorten a real call; do not fabricate bytes.
            ++refused;
            errno = failure;
            return 1;
        }
    }
    return 0;
}

static size_t request(int action, size_t count) {
    return action == 2 && count > 1 ? 1 : count;
}

static ssize_t observe(int action, ssize_t result, size_t count) {
    if (action == 2 && result > 0 && (size_t)result < count) ++partial;
    return result;
}

static ssize_t probe_read(int fd, void *bytes, size_t count) {
    int action = deny(0, fd);
    if (action == 1) return -1;
    return observe(action, NATIVE_REAL(read)(fd, bytes, request(action, count)), count);
}

static ssize_t probe_write(int fd, const void *bytes, size_t count) {
    int action = deny(1, fd);
    if (action == 1) return -1;
    return observe(action, NATIVE_REAL(write)(fd, bytes, request(action, count)), count);
}

static ssize_t probe_pread(int fd, void *bytes, size_t count, off_t offset) {
    int action = deny(2, fd);
    if (action == 1) return -1;
    return observe(action, NATIVE_REAL(pread)(fd, bytes, request(action, count), offset), count);
}

static ssize_t probe_pwrite(int fd, const void *bytes, size_t count, off_t offset) {
    int action = deny(3, fd);
    if (action == 1) return -1;
    return observe(action, NATIVE_REAL(pwrite)(fd, bytes, request(action, count), offset), count);
}

NATIVE_INTERPOSE(probe_read, read);
NATIVE_INTERPOSE(probe_write, write);
NATIVE_INTERPOSE(probe_pread, pread);
NATIVE_INTERPOSE(probe_pwrite, pwrite);

#if defined(__linux__)
// Rust/glibc can bind either spelling on 64-bit Linux. Both reach the same
// signed 64-bit offset ABI and must be visible to the census.
_Static_assert(sizeof(off64_t) == sizeof(off_t), "large-file offset ABI");
NATIVE_INTERPOSE(probe_pread, pread64);
NATIVE_INTERPOSE(probe_pwrite, pwrite64);
#endif
