// Independent native SDK check for the private directory boundary.
#define _GNU_SOURCE
#define _DARWIN_C_SOURCE
#include <stddef.h>
#include <stdint.h>
#include <fcntl.h>
#include <limits.h>
#include <unistd.h>
#include <stdio.h>
#include <pthread.h>
#include <errno.h>
#include <assert.h>
#if defined(__APPLE__)
#include <sys/attr.h>
#include <sys/stat.h>
#include <sys/vnode.h>
struct canonical_name_record {
    uint32_t length;
    attrreference_t name;
    dev_t device;
    fsobj_type_t type;
    fsobj_id_t object;
    char bytes[PATH_MAX];
};
_Static_assert(VLNK == 5, "canonical name symlink discriminator");
_Static_assert(offsetof(struct canonical_name_record, name) == 4, "canonical name reference");
_Static_assert(offsetof(struct canonical_name_record, device) == 12, "canonical device");
_Static_assert(offsetof(struct canonical_name_record, type) == 16, "canonical object type");
_Static_assert(offsetof(struct canonical_name_record, object) == 20, "legacy naming object");
_Static_assert(offsetof(struct canonical_name_record, bytes) == 28, "canonical name payload");
_Static_assert(sizeof(struct canonical_name_record) == 1052, "canonical record extent");
_Static_assert(sizeof(struct stat) == 144 && _Alignof(struct stat) == 8, "stat layout");
_Static_assert(offsetof(struct stat, st_dev) == 0 && offsetof(struct stat, st_mode) == 4, "stat device and mode");
_Static_assert(offsetof(struct stat, st_nlink) == 6 && offsetof(struct stat, st_ino) == 8, "stat links and inode");
_Static_assert(offsetof(struct stat, st_size) == 96, "stat byte length");
_Static_assert(offsetof(struct stat, st_mtimespec) == 48 && offsetof(struct stat, st_ctimespec) == 64, "stat timestamps");
_Static_assert(sizeof(struct timespec) == 16 && offsetof(struct timespec, tv_nsec) == 8, "timestamp units");
_Static_assert(PATH_MAX == 1024, "realpath result buffer premise");
_Static_assert(sizeof(struct attrlist) == 24 && _Alignof(struct attrlist) == 4, "attrlist layout");
_Static_assert(offsetof(struct attrlist, bitmapcount) == 0, "bitmap count");
_Static_assert(offsetof(struct attrlist, commonattr) == 4, "common attributes");
_Static_assert(offsetof(struct attrlist, forkattr) == 20, "fork attributes");
_Static_assert(sizeof(attribute_set_t) == 20, "returned attribute size");
_Static_assert(sizeof(attrreference_t) == 8, "name reference size");
_Static_assert(offsetof(attrreference_t, attr_length) == 4, "name length offset");
_Static_assert(ATTR_BIT_MAP_COUNT == 5, "bitmap count value");
_Static_assert((ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_ERROR | ATTR_CMN_NAME) == 0xa0000001, "requested attributes");
_Static_assert(FSOPT_PACK_INVAL_ATTRS == 8, "packing");
_Static_assert(O_RDONLY == 0 && O_NONBLOCK == 4 && O_DIRECTORY == 0x100000 && O_CLOEXEC == 0x1000000, "open flags");
#elif defined(__linux__)
#include <dirent.h>
#include <sys/syscall.h>
_Static_assert(offsetof(struct dirent64, d_ino) == 0, "inode");
_Static_assert(offsetof(struct dirent64, d_off) == 8, "offset");
_Static_assert(offsetof(struct dirent64, d_reclen) == 16, "record size");
_Static_assert(offsetof(struct dirent64, d_type) == 18, "kind");
_Static_assert(offsetof(struct dirent64, d_name) == 19, "name");
#else
#error unsupported filesystem ABI
#endif
int main(void) {
    pthread_mutexattr_t attributes;
    pthread_mutex_t mutex;
    assert(pthread_mutexattr_init(&attributes) == 0);
    assert(pthread_mutexattr_settype(&attributes, PTHREAD_MUTEX_NORMAL) == 0);
    assert(pthread_mutex_init(&mutex, &attributes) == 0);
    assert(pthread_mutexattr_destroy(&attributes) == 0);
    assert(pthread_mutex_lock(&mutex) == 0);
    assert(pthread_mutex_trylock(&mutex) == EBUSY);
    assert(pthread_mutex_unlock(&mutex) == 0);
    assert(pthread_mutex_destroy(&mutex) == 0);
    printf("native mutex storage: %zu bytes, alignment %zu\n",
           sizeof(mutex), _Alignof(pthread_mutex_t));
#if defined(__APPLE__)
    int (*call)(int, void *, void *, size_t, uint64_t) = getattrlistbulk;
    int (*canonical_call)(const char *, void *, void *, size_t, unsigned int) = getattrlist;
    (void)canonical_call;
#else
    long (*call)(long, ...) = syscall;
    (void)SYS_getdents64;
#endif
    (void)call;
    puts("native directory ABI matches checked layout");
}
