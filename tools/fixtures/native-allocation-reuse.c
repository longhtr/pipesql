/* Isolate allocator reuse from the engine and its Rust allocation observer.
 * The smaller request and larger usable extent come from the retained wide-join
 * diagnostic. Seeding with that larger extent is a controlled history, not a
 * reconstruction of every allocation in the original query. Cold mode skips
 * that seed; both modes report requested/usable bytes only after freeing owners.
 * tools/README.md owns the comparison command. This is an allocator diagnostic,
 * not an engine admission or whole-process memory bound. */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(__APPLE__)
#include <malloc/malloc.h>
#define allocation_extent(pointer) malloc_size(pointer)
#elif defined(__linux__)
#include <malloc.h>
#define allocation_extent(pointer) malloc_usable_size(pointer)
#else
#error "This diagnostic requires Darwin or GNU/Linux allocation extents"
#endif

int main(int argc, char **argv) {
    if (argc != 2 || (strcmp(argv[1], "cold") != 0 && strcmp(argv[1], "reuse") != 0)) {
        fputs("usage: native-allocation-reuse cold|reuse\n", stderr);
        return 2;
    }
    const bool seed_reuse = strcmp(argv[1], "reuse") == 0;
    const size_t requested = 3817440;
    const size_t seed_requested = seed_reuse ? 3899392 : 0;
    size_t seed_usable = 0;
    uintptr_t seed_address = 0;
    if (seed_reuse) {
        void *seed = malloc(seed_requested);
        if (seed == NULL) {
            fputs("seed allocation failed\n", stderr);
            return 1;
        }
        seed_usable = allocation_extent(seed);
        seed_address = (uintptr_t)seed;
        free(seed);
        if (seed_usable < seed_requested) {
            fputs("seed extent is smaller than its request\n", stderr);
            return 1;
        }
    }

    void *allocation = malloc(requested);
    if (allocation == NULL) {
        fputs("measured allocation failed\n", stderr);
        return 1;
    }
    const size_t usable = allocation_extent(allocation);
    // Compare saved integer addresses without accessing the freed seed owner.
    const bool reused = seed_reuse && seed_address == (uintptr_t)allocation;
    free(allocation);
    if (usable < requested) {
        fputs("measured extent is smaller than its request\n", stderr);
        return 1;
    }
    // Report after both owners are freed so stdio cannot alter the measured history.
    const int written = printf("{\"mode\":\"%s\",\"requested\":%zu,\"usable\":%zu,"
                               "\"seed_requested\":%zu,\"seed_usable\":%zu,\"reused\":%s}\n",
                               argv[1], requested, usable, seed_requested, seed_usable,
                               reused ? "true" : "false");
    return written < 0 || fflush(stdout) == EOF;
}
