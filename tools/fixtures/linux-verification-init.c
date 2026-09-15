/* Own mounts, privilege reduction and shutdown as PID 1 in the verification VM.
 * The gate runs as uid/gid 1000 from the read-only source image, with /tmp on
 * the private ext4 data disk. Report its real exit or signal status, reap exited
 * orphans and refuse a live descendant before unmounting. Only then report
 * VERIFICATION_STOPPED. check-linux-vm.py requires both records; VM shutdown
 * alone cannot establish successful verification. */
#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mount.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static void require(int condition, const char *operation) {
    if (!condition) {
        printf("VERIFICATION_ERROR operation=%s errno=%d\n", operation, errno);
        if (getpid() == 1) {
            reboot(RB_POWER_OFF);
        }
        exit(2);
    }
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    require(getpid() == 1 && argc == 2, "guest-init");
    require(mount("devtmpfs", "/dev", "devtmpfs", 0, NULL) == 0 || errno == EBUSY,
            "mount-dev");
    require(mount("proc", "/proc", "proc", 0, NULL) == 0, "mount-proc");
    /* devtmpfs supplies device nodes; a normal userspace init supplies this
     * descriptor alias, which the independent leak checks also use. */
    require(symlink("/proc/self/fd", "/dev/fd") == 0, "descriptor-alias");
    require(mount("sysfs", "/sys", "sysfs", 0, NULL) == 0, "mount-sys");
    require(mount("/dev/vdb", "/tmp", "ext4", 0, "barrier=1,data=ordered") == 0, "mount-data");
    require(chmod("/tmp", 01777) == 0, "data-permissions");
    require(mkdir("/tmp/home", 0700) == 0, "home");
    require(chown("/tmp/home", 1000, 1000) == 0, "home-owner");

    pid_t child = fork();
    require(child >= 0, "fork-gate");
    if (child == 0) {
        require(setgid(1000) == 0 && setuid(1000) == 0, "gate-identity");
        require(chdir("/source") == 0, "source-directory");
        require(setenv("HOME", "/tmp/home", 1) == 0, "home-environment");
        execl("/bin/sh", "/bin/sh", "/verification.sh", argv[1], (char *)NULL);
        perror("execute verification script");
        _exit(126);
    }
    int status = 0;
    require(waitpid(child, &status, 0) == child, "wait-gate");
    int result = WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
    printf("VERIFICATION_RESULT exit=%d\n", result);

    /* A live descendant is a failed cleanup, even if the immediate gate passed.
     * Reap exited orphans before checking that no child remains. */
    pid_t orphan;
    do {
        orphan = waitpid(-1, &status, WNOHANG);
    } while (orphan > 0);
    require(orphan == -1 && errno == ECHILD, "remaining-child");
    require(umount("/tmp") == 0, "unmount-data");
    puts("VERIFICATION_STOPPED");
    require(reboot(RB_POWER_OFF) == 0, "power-off");
    return 2;
}
