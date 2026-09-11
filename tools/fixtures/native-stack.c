// Independent pthread premise check. No engine or Rust bindings are used.
#define _GNU_SOURCE
#define _DARWIN_C_SOURCE
#include <assert.h>
#include <errno.h>
#include <limits.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>

struct observation {
    size_t bytes;
    size_t guard;
};

static void *observe(void *argument) {
    struct observation *result = argument;
    void *base;
#if defined(__APPLE__)
    result->bytes = pthread_get_stacksize_np(pthread_self());
    base = (char *)pthread_get_stackaddr_np(pthread_self()) - result->bytes;
#elif defined(__linux__)
    pthread_attr_t attributes;
    assert(pthread_getattr_np(pthread_self(), &attributes) == 0);
    assert(pthread_attr_getstack(&attributes, &base, &result->bytes) == 0);
    assert(pthread_attr_getguardsize(&attributes, &result->guard) == 0);
    assert(pthread_attr_destroy(&attributes) == 0);
#else
#error unsupported stack observation
#endif
    uintptr_t local = (uintptr_t)&result;
    assert(result->bytes > 0);
    assert(local >= (uintptr_t)base && local - (uintptr_t)base < result->bytes);
    return NULL;
}

int main(void) {
    pthread_attr_t attributes;
    assert(pthread_attr_init(&attributes) == 0);
    printf("native page=%ld minimum=%zu\n", sysconf(_SC_PAGESIZE), (size_t)PTHREAD_STACK_MIN);
    size_t small_request = 48 * 1024;
    size_t ceiling = 64 * 1024;
#if defined(__linux__) && defined(__aarch64__) && defined(__GLIBC__)
    assert(PTHREAD_STACK_MIN == 128 * 1024);
    assert(pthread_attr_setstacksize(&attributes, 48 * 1024) == EINVAL);
    assert(pthread_attr_setstacksize(&attributes, 64 * 1024) == EINVAL);
    small_request = 128 * 1024;
    ceiling = 144 * 1024;
#endif
    size_t requests[] = {small_request, 2 * 1024 * 1024};
    for (size_t i = 0; i < 2; ++i) {
        assert(pthread_attr_setstacksize(&attributes, requests[i]) == 0);
        struct observation result = {0, 0};
        pthread_t thread;
        assert(pthread_create(&thread, &attributes, observe, &result) == 0);
        assert(pthread_join(thread, NULL) == 0);
        int accepted = result.bytes <= ceiling;
        assert(accepted == (i == 0));
        printf("native requested=%zu reported=%zu accepted=%d", requests[i], result.bytes, accepted);
#if defined(__linux__)
        printf(" guard=%zu", result.guard);
#endif
        puts("");
    }
    assert(pthread_attr_destroy(&attributes) == 0);
    puts("native stack premise and oversized-thread control passed");
}
