// Read-only GNU/Linux observation of the stock catalog caller's root identities.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

#include "native-interpose.h"

#if !defined(__linux__)
#error This observer requires GNU/Linux statx and /proc/self/fd
#endif

NATIVE_BIND(lstat);
NATIVE_BIND(fstat64);
NATIVE_BIND(statx);

enum { PATH_BYTES = 4096 };

struct root_observation {
    char path[PATH_BYTES + 1];
    dev_t device;
    ino_t inode;
    int present;
};

// Correlate only calls on the same thread; this is not a concurrency oracle.
static _Thread_local struct root_observation roots[2];

static int root_index(const char *path) {
    const char *name = strrchr(path, '/');
    name = name ? name + 1 : path;
    if (strcmp(name, "ROOT.A") == 0) return 0;
    if (strcmp(name, "ROOT.B") == 0) return 1;
    return -1;
}

static void report_identity(int root, const struct root_observation *before,
                            const struct stat64 *descriptor,
                            const struct statx *extended, const struct stat *after) {
    char line[512];
    int length = snprintf(
        line, sizeof line,
        "filesystem identity ROOT.%c before=%ju:%ju fstat=%ju:%ju statx=%ju:%ju after=%ju:%ju\n",
        'A' + root, (uintmax_t)before->device, (uintmax_t)before->inode,
        (uintmax_t)descriptor->st_dev, (uintmax_t)descriptor->st_ino,
        (uintmax_t)makedev(extended->stx_dev_major, extended->stx_dev_minor),
        (uintmax_t)extended->stx_ino, (uintmax_t)after->st_dev, (uintmax_t)after->st_ino);
    // A missing/truncated witness must not look like successful observation.
    if (length < 0 || length >= (int)sizeof line || write(2, line, length) != length) {
        _exit(98);
    }
}

int identity_lstat(const char *path, struct stat *out) {
    int result = NATIVE_REAL(lstat)(path, out);
    int saved = errno;
    int root = result == 0 ? root_index(path) : -1;
    if (root >= 0) {
        size_t length = strnlen(path, PATH_BYTES + 1);
        if (length > PATH_BYTES) _exit(98);
        memcpy(roots[root].path, path, length + 1);
        roots[root].device = out->st_dev;
        roots[root].inode = out->st_ino;
        roots[root].present = 1;
    }
    errno = saved;
    return result;
}
NATIVE_INTERPOSE(identity_lstat, lstat);

int identity_statx(int fd, const char *name, int flags, unsigned int mask,
                   struct statx *out) {
    // Compare legacy fstat with statx before inspecting the pathname again.
    struct stat64 descriptor;
    int inspected = -1;
    int incoming_errno = errno;
    if (name != NULL && name[0] == '\0' && (flags & AT_EMPTY_PATH)) {
        inspected = NATIVE_REAL(fstat64)(fd, &descriptor);
    }
    errno = incoming_errno;
    int result = NATIVE_REAL(statx)(fd, name, flags, mask, out);
    int saved = errno;
    if (result == 0 && inspected == 0) {
        char link[64], path[PATH_BYTES + 1];
        int length = snprintf(link, sizeof link, "/proc/self/fd/%d", fd);
        if (length < 0 || length >= (int)sizeof link) _exit(98);
        ssize_t bytes = readlink(link, path, sizeof path);
        if (bytes < 0 || bytes > PATH_BYTES) _exit(98);
        path[bytes] = '\0';
        int root = root_index(path);
        if (root >= 0 && roots[root].present && strcmp(roots[root].path, path) == 0) {
            const struct root_observation *before = &roots[root];
            if (!(out->stx_mask & STATX_INO)) _exit(98);
            dev_t device = makedev(out->stx_dev_major, out->stx_dev_minor);
            if (before->device != descriptor.st_dev || before->inode != descriptor.st_ino ||
                before->device != device || before->inode != out->stx_ino) {
                struct stat after;
                if (NATIVE_REAL(lstat)(path, &after) != 0) _exit(98);
                report_identity(root, before, &descriptor, out, &after);
            }
        }
    }
    errno = saved;
    return result;
}
NATIVE_INTERPOSE(identity_statx, statx);
