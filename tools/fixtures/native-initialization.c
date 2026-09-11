/* Observe Darwin traversal entry or Linux libc realpath entry. The observer
 * delays/refuses one actor; it neither resolves paths nor changes the filesystem. */
#include <errno.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>
#if defined(__APPLE__)
#include <sys/mount.h>
#endif
#include <sys/stat.h>
#include <time.h>
#include "native-interpose.h"

_Static_assert(sizeof(int) == 4, "Rust probe scalar ABI");
_Static_assert(sizeof(unsigned long) == 8, "Rust probe counter ABI");
static _Atomic int active, entered, released, pending, resolution_error;
static _Atomic unsigned long resolution_calls[2], fs_calls[2];
static _Thread_local int actor = -1;

void probe_actor(int value) { actor = value; }
void probe_start(int mode, int error) {
    atomic_store(&resolution_error, error);
    atomic_store(&active, mode);
}
void probe_release(void) { atomic_store(&released, 1); }
int probe_pending(void) { return atomic_load(&pending); }
void probe_stop(void) { atomic_store(&active, 0); }
unsigned long probe_resolutions(int value) { return atomic_load(&resolution_calls[value]); }
unsigned long probe_mounts(int value) { return atomic_load(&fs_calls[value]); }

static int await_flag(_Atomic int *flag) {
    for (unsigned int i = 0; i < 10000; ++i) {
        if (atomic_load(flag)) return 1;
        struct timespec delay = {0, 1000000};
        if (nanosleep(&delay, 0) < 0 && errno != EINTR) return 0;
    }
    return 0;
}
int probe_wait(void) { return await_flag(&entered); }

static int enter_resolution(void) {
    int mode = atomic_load(&active);
    if (mode && actor >= 0 && actor < 2) {
        atomic_fetch_add(&resolution_calls[actor], 1);
        if (actor == 0 && (mode == 3 || mode == 4)) {
            atomic_store(&pending, 1);
            atomic_store(&entered, 1);
            int resumed = await_flag(&released);
            atomic_store(&pending, 0);
            if (!resumed) { errno = ETIMEDOUT; return -1; }
        }
        if (actor == 0 && (mode == 2 || mode == 4)) {
            errno = atomic_load(&resolution_error);
            return -1;
        }
    }
    return 0;
}

#if defined(__APPLE__)
static int observed_stat(const char *path, struct stat *output) {
    if (path[0] == '/' && path[1] == 0 && enter_resolution() < 0) return -1;
    return stat(path, output);
}
static int observed_statfs(const char *path, struct statfs *output) {
    if (atomic_load(&active) && actor >= 0 && actor < 2) {
        atomic_fetch_add(&fs_calls[actor], 1);
    }
    return statfs(path, output);
}
NATIVE_INTERPOSE(observed_stat, stat);
NATIVE_INTERPOSE(observed_statfs, statfs);
#elif defined(__linux__)
NATIVE_BIND(realpath);
static char *observed_realpath(const char *path, char *resolved) {
    if (enter_resolution() < 0) return NULL;
    return NATIVE_REAL(realpath)(path, resolved);
}
NATIVE_INTERPOSE(observed_realpath, realpath);
#endif
