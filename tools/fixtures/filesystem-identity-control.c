// Challenge the identity observer with a stable file and deliberate replacement.
// Both files contain the same bytes, so pathname lstat and opened-descriptor statx
// must distinguish identity rather than content. This standalone GNU/Linux caller
// runs no engine code. tools/README.md owns compilation and observer comparison.

#define _GNU_SOURCE
#include <assert.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static void create_file(const char *name) {
    int fd = open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    assert(fd >= 0);
    assert(write(fd, "same bytes", 10) == 10);
    assert(close(fd) == 0);
}

int main(int argc, char **argv) {
    assert(argc == 3);
    assert(strcmp(argv[2], "stable") == 0 || strcmp(argv[2], "replace") == 0);
    assert(mkdir(argv[1], 0700) == 0);
    assert(chdir(argv[1]) == 0);
    create_file("ROOT.A");
    create_file("next");
    char path[4097];
    assert(getcwd(path, sizeof path) != NULL);
    size_t length = strlen(path);
    assert(length + sizeof "/ROOT.A" <= sizeof path);
    memcpy(path + length, "/ROOT.A", sizeof "/ROOT.A");
    struct stat before;
    assert(lstat(path, &before) == 0);
    int replace = strcmp(argv[2], "replace") == 0;
    if (replace) assert(rename("next", "ROOT.A") == 0);
    int fd = open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    assert(fd >= 0);
    struct statx after;
    assert(statx(fd, "", AT_EMPTY_PATH, STATX_ALL, &after) == 0);
    assert(after.stx_mask & STATX_INO);
    assert((before.st_ino != after.stx_ino) == replace);
    assert(close(fd) == 0);
    puts(replace ? "replacement control passed" : "stable control passed");
    return 0;
}
