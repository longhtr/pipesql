// Route selected OS calls through an observer while the DBMS runs unchanged.
// The observer can count or alter a call, then use NATIVE_REAL to invoke its
// original implementation. These macros supply that routing; each observer
// defines its own measurements, injected failures, and thread synchronization.
//
// macOS uses a loader table of replacement/original function pairs. Linux exports
// a replacement symbol and resolves the next implementation before main, while
// fault injection is disarmed. Resolving early keeps loader work outside the
// measured operation. A missing symbol exits with status 98 rather than allowing
// a campaign to pass without observing the call.

#ifndef PIPESQL_NATIVE_INTERPOSE_H
#define PIPESQL_NATIVE_INTERPOSE_H

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
