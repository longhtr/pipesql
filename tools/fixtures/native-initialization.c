/* Test-only effect substitution around the linked libSystem realpath, not a
 * replacement resolver. Scope is one owned child; no filesystem mutation here. */
#include <errno.h>
#include <stdatomic.h>
#include <stdint.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <time.h>

_Static_assert(sizeof(int) == 4, "Rust probe scalar ABI");
_Static_assert(sizeof(unsigned long) == 8, "Rust probe counter ABI");
static _Atomic int active, entered, released, pending, root_error;
static _Atomic unsigned long root_calls[2], fs_calls[2];
static _Thread_local int actor = -1;

void probe_actor(int value) { actor = value; }
void probe_start(int mode, int error) {
    atomic_store(&root_error, error);
    atomic_store(&active, mode);
}
void probe_release(void) { atomic_store(&released, 1); }
int probe_pending(void) { return atomic_load(&pending); }
void probe_stop(void) { atomic_store(&active, 0); }
unsigned long probe_roots(int value) { return atomic_load(&root_calls[value]); }
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

static int observed_stat(const char *path, struct stat *output) {
    int mode = atomic_load(&active);
    if (mode && actor >= 0 && actor < 2 && path[0] == '/' && path[1] == 0) {
        atomic_fetch_add(&root_calls[actor], 1);
        if (actor == 0 && (mode == 3 || mode == 4)) {
            atomic_store(&pending, 1);
            atomic_store(&entered, 1);
            int resumed = await_flag(&released);
            atomic_store(&pending, 0);
            if (!resumed) { errno = ETIMEDOUT; return -1; }
        }
        if (actor == 0 && (mode == 2 || mode == 4)) {
            errno = atomic_load(&root_error);
            return -1;
        }
    }
    /* dyld does not interpose the replacement image's own reference. */
    return stat(path, output);
}
static int observed_statfs(const char *path, struct statfs *output) {
    if (atomic_load(&active) && actor >= 0 && actor < 2) {
        atomic_fetch_add(&fs_calls[actor], 1);
    }
    return statfs(path, output);
}
#define INTERPOSE(replacement, original) \
    __attribute__((used, section("__DATA,__interpose"))) \
    static const struct { const void *new; const void *old; } pair_##original = \
        {(const void *)(uintptr_t)&replacement, (const void *)(uintptr_t)&original}
INTERPOSE(observed_stat, stat);
INTERPOSE(observed_statfs, statfs);
