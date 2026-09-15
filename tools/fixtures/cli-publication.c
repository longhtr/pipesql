// Refuse one actual rename before it runs in a single-threaded CLI caller.
// All other calls forward unchanged; the observer does not edit the namespace.
// cli-publication.rs arms the selected position and preserves the CLI exit status.
// check-cli-allocation.py interprets tokens and persisted outcomes afterward.
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>
#include "native-interpose.h"

NATIVE_BIND(rename);

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
    return NATIVE_REAL(rename)(source, destination);
}
NATIVE_INTERPOSE(observed_rename, rename);
