/* Observe native pathname resolution for check-native-initialization.py.
 * Per-thread actor IDs select one call to delay or refuse. Atomic handshakes let
 * the Rust caller prove that another actor progresses during the delay. Darwin
 * observes root stat; Linux observes root/component lstat and symlink readlink.
 * Calls otherwise reach the real implementation. The observer neither resolves
 * paths nor changes files; the caller checks names, errors and later reuse. */
#include <errno.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>
#if defined(__APPLE__)
#include <sys/mount.h>
#endif
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
#include "native-interpose.h"

_Static_assert(sizeof(int) == 4, "Rust probe scalar ABI");
_Static_assert(sizeof(unsigned long) == 8, "Rust probe counter ABI");
static _Atomic int active, entered, released, pending, resolution_error;
static _Atomic unsigned long resolution_calls[2], fs_calls[2];
static _Thread_local int actor = -1, selected;
enum { ROOT_ENTRY, COMPONENT_ENTRY, SYMLINK_ENTRY };
static _Atomic int fault_site;
static _Atomic unsigned long lstat_calls[2], readlink_calls[2], realpath_calls[2];
unsigned long probe_lstats(int value) { return atomic_load(&lstat_calls[value]); }
unsigned long probe_readlinks(int value) { return atomic_load(&readlink_calls[value]); }
unsigned long probe_realpaths(int value) { return atomic_load(&realpath_calls[value]); }

void probe_actor(int value) { actor = value; selected = 0; }
void probe_start(int mode, int error, int site) {
    atomic_store(&fault_site, site);
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

static int observing(void) {
    return atomic_load(&active) && actor >= 0 && actor < 2;
}

static int observe_step(int site) {
    if (!observing()) return 0;
    if (site == ROOT_ENTRY) atomic_fetch_add(&resolution_calls[actor], 1);
    if (site != atomic_load(&fault_site) || selected) return 0;
    selected = 1;
    return enter_resolution();
}

#if defined(__APPLE__)
static int observed_stat(const char *path, struct stat *output) {
    if (path[0] == '/' && path[1] == 0 && observe_step(ROOT_ENTRY) < 0) return -1;
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
NATIVE_BIND(lstat);
NATIVE_BIND(readlink);
NATIVE_BIND(realpath);
static int observed_lstat(const char *path, struct stat *output) {
    if (observing()) atomic_fetch_add(&lstat_calls[actor], 1);
    int site = path[0] == '/' && path[1] == 0 ? ROOT_ENTRY : COMPONENT_ENTRY;
    if (observe_step(site) < 0) return -1;
    return NATIVE_REAL(lstat)(path, output);
}
static ssize_t observed_readlink(const char *path, char *output, size_t capacity) {
    if (observing()) atomic_fetch_add(&readlink_calls[actor], 1);
    if (observe_step(SYMLINK_ENTRY) < 0) return -1;
    return NATIVE_REAL(readlink)(path, output, capacity);
}
static char *observed_realpath(const char *path, char *resolved) {
    if (observing()) atomic_fetch_add(&realpath_calls[actor], 1);
    return NATIVE_REAL(realpath)(path, resolved);
}
NATIVE_INTERPOSE(observed_lstat, lstat);
NATIVE_INTERPOSE(observed_readlink, readlink);
NATIVE_INTERPOSE(observed_realpath, realpath);
#endif
