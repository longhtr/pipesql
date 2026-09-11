#ifndef PIPESQL_NATIVE_INTERPOSE_H
#define PIPESQL_NATIVE_INTERPOSE_H

// Disposable, single-threaded observers only. Resolve forwarding targets before
// main, while fault injection is disarmed. Missing symbols terminate the caller;
// they cannot turn an unobserved operation into a successful campaign cell.
#if defined(__APPLE__)
#define NATIVE_BIND(name)
#define NATIVE_REAL(name) name
#define NATIVE_INTERPOSE(replacement, original) \
    __attribute__((used)) static struct { const void *new_fn; const void *old_fn; } \
    pair_##original __attribute__((section("__DATA,__interpose"))) = \
    { (const void *)(uintptr_t)&replacement, (const void *)(uintptr_t)&original }
#elif defined(__linux__)
#include <dlfcn.h>
#include <string.h>
#include <unistd.h>
#define NATIVE_BIND(name) \
    static __typeof__(&name) real_##name; \
    __attribute__((constructor)) static void resolve_##name(void) { \
        dlerror(); \
        void *symbol = dlsym(RTLD_NEXT, #name); \
        if (dlerror() != NULL || symbol == NULL) _exit(98); \
        _Static_assert(sizeof(symbol) == sizeof(real_##name), "native function pointer ABI"); \
        memcpy(&real_##name, &symbol, sizeof(symbol)); \
    }
#define NATIVE_REAL(name) real_##name
#define NATIVE_INTERPOSE(replacement, original) \
    extern __typeof__(original) original __attribute__((alias(#replacement)))
#else
#error Native observers require macOS or Linux
#endif
#endif
