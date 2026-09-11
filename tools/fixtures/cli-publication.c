// Single-threaded CLI caller observer: refuse one actual rename before it runs.
// No namespace is changed by the observer; all other calls forward unchanged.
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>

static unsigned at, calls;

void cli_publication_start(unsigned position) {
    if (position > 4) _exit(90);
    at = position;
    calls = 0;
}
unsigned cli_publication_calls(void) { return calls; }

static int observed_rename(const char *source, const char *destination) {
    if (++calls > 4) _exit(91);
    if (calls == at) {
        errno = EIO;
        return -1;
    }
    return rename(source, destination);
}
__attribute__((used)) static struct { const void *replacement; const void *original; }
rename_pair __attribute__((section("__DATA,__interpose"))) = {
    (const void *)(uintptr_t)&observed_rename, (const void *)(uintptr_t)&rename
};
