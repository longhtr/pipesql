/* Measure synchronization cost inside a disposable Linux VM.
 * Raw mode compares flushes of unchanged data with 4-KiB direct writes followed
 * by flushes, then checks readback. Catalog mode runs the existing allocation
 * caller's capacity, healthy and refusal controls on ext4. PID 1 owns mounts
 * and shutdown; an unprivileged worker owns measurements. Failed workers cannot
 * emit PROBE_OK. The host selects the disk policy, while this workload stays
 * fixed. See tools/README.md; timings do not establish power-loss durability. */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static void require(int condition, const char *operation) {
    if (!condition) {
        printf("PROBE_ERROR operation=%s errno=%d\n", operation, errno);
        if (getpid() == 1) {
            reboot(RB_POWER_OFF);
        }
        exit(2);
    }
}

static uint64_t nanoseconds(void) {
    struct timespec time;
    require(clock_gettime(CLOCK_MONOTONIC, &time) == 0, "clock");
    return (uint64_t)time.tv_sec * UINT64_C(1000000000) + (uint64_t)time.tv_nsec;
}

static int compare_samples(const void *left, const void *right) {
    uint64_t a = *(const uint64_t *)left;
    uint64_t b = *(const uint64_t *)right;
    return (a > b) - (a < b);
}

static void require_child(pid_t child) {
    int status = 0;
    require(waitpid(child, &status, 0) == child, "wait-child");
    require(WIFEXITED(status) && WEXITSTATUS(status) == 0, "child-result");
}

static void raw_disk_probe(int disk) {
    void *written = NULL;
    void *observed = NULL;
    require(posix_memalign(&written, 4096, 4096) == 0, "write-buffer");
    require(posix_memalign(&observed, 4096, 4096) == 0, "read-buffer");

    /* O_DIRECT separates completed writes from flush requests. Repeating a
     * flush on unchanged data is the control for the dirty-write workload. */
    for (int dirty = 0; dirty <= 1; ++dirty) {
        uint64_t samples[128];
        uint64_t flush_time = 0;
        memset(written, 0, 4096);
        require(pwrite(disk, written, 4096, 0) == 4096, "initial-write");
        require(fsync(disk) == 0, "initial-flush");
        uint64_t start = nanoseconds();
        for (unsigned sample = 0; sample < 128; ++sample) {
            if (dirty) {
                memset(written, (int)sample + 1, 4096);
                require(pwrite(disk, written, 4096, 0) == 4096, "write");
            }
            uint64_t before = nanoseconds();
            require(fsync(disk) == 0, "flush");
            samples[sample] = nanoseconds() - before;
            flush_time += samples[sample];
        }
        uint64_t elapsed = nanoseconds() - start;
        require(pread(disk, observed, 4096, 0) == 4096, "read-back");
        require(memcmp(written, observed, 4096) == 0, "contents");
        qsort(samples, 128, sizeof *samples, compare_samples);
        printf("RAW workload=%s count=128 total_ns=%llu flush_ns=%llu "
               "median_ns=%llu p95_ns=%llu\n",
               dirty ? "write" : "clean", (unsigned long long)elapsed,
               (unsigned long long)flush_time, (unsigned long long)samples[64],
               (unsigned long long)samples[121]);
    }
    free(observed);
    free(written);
}

static void catalog_probe(void) {
    const char *modes[] = {
        "allocation-capacity", "catalog-control", "catalog-after-0"
    };
    const char *paths[] = {"/tmp/capacity", "/tmp/catalog", "/tmp/refusal"};
    for (unsigned cell = 0; cell < 3; ++cell) {
        uint64_t start = nanoseconds();
        pid_t child = fork();
        require(child >= 0, "fork-caller");
        if (child == 0) {
            execl("/driver", "/driver", paths[cell], modes[cell], (char *)NULL);
            _exit(126);
        }
        require_child(child);
        printf("CATALOG mode=%s elapsed_ns=%llu\n", modes[cell],
               (unsigned long long)(nanoseconds() - start));
    }
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    require(getpid() == 1, "run-as-guest-init");
    require(argc == 2, "arguments");
    int catalog = strcmp(argv[1], "catalog") == 0;
    require(catalog || strcmp(argv[1], "raw") == 0, "workload");

    /* Some kernels mount devtmpfs before init. The opened device must still
     * be a block device; a missing or incorrectly supplied disk fails closed. */
    require(mount("devtmpfs", "/dev", "devtmpfs", 0, NULL) == 0 || errno == EBUSY,
            "mount-dev");
    int disk = -1;
    if (catalog) {
        require(mount("proc", "/proc", "proc", 0, NULL) == 0, "mount-proc");
        require(mount("/dev/vdb", "/tmp", "ext4", 0, NULL) == 0, "mount-data");
        require(chmod("/tmp", 01777) == 0, "data-permissions");
    } else {
        disk = open("/dev/vdb", O_RDWR | O_DIRECT);
        require(disk >= 0, "open-data");
        struct stat status;
        require(fstat(disk, &status) == 0 && S_ISBLK(status.st_mode), "block-device");
    }

    /* Init owns mounts and shutdown. The measurement and every database caller
     * run with uid/gid 1000. Boot and unmount time are outside the measurements. */
    pid_t worker = fork();
    require(worker >= 0, "fork-worker");
    if (worker == 0) {
        require(setgid(1000) == 0 && setuid(1000) == 0, "measurement-identity");
        printf("IDENTITY uid=%u gid=%u\n", (unsigned)getuid(), (unsigned)getgid());
        if (catalog) {
            catalog_probe();
        } else {
            raw_disk_probe(disk);
            require(close(disk) == 0, "close-worker-disk");
        }
        return 0;
    }
    if (disk >= 0) {
        require(close(disk) == 0, "close-init-disk");
    }
    require_child(worker);
    if (catalog) {
        require(umount("/tmp") == 0, "unmount-data");
    }
    puts("PROBE_OK");
    require(reboot(RB_POWER_OFF) == 0, "power-off");
    return 2;
}
