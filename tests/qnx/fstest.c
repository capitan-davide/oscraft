/* procnto's own files as procnto treats them: the image filesystem
 * (read-only, no links or renames, directories read with readdir() only,
 * lseek() within the file, statvfs()), /dev/null, /proc, /dev/shmem, and
 * fcntl()'s flags. The expectations are procnto's results (measured in
 * the reference image); OsCraft's mkdir() in /dev/shmem, a deliberate
 * difference, is left out. One open file description behind dup(),
 * fork() and a redirection (review item PATHMEM-1, measured): the
 * position and the status flags are shared. Prints one "ok"/"FAIL" line
 * per call and "fstest: passed". */
#include <stdio.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <dirent.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <sys/stat.h>
#include <sys/mman.h>
#include <sys/statvfs.h>
static int fails;

/* `expr` fails with errno `err`. */
#define E(expr, err)                                                                                   \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(expr);                                                                        \
        int ok_ = r_ == -1 && errno == (err);                                                          \
        printf("fstest: %s %s (%ld, errno %d)\n", ok_ ? "ok" : "FAIL", #expr, r_, errno);               \
        fails += !ok_;                                                                                 \
    } while (0)
/* `expr` gives `val`. */
#define V(expr, val)                                                                                   \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(expr);                                                                        \
        int ok_ = r_ == (long)(val);                                                                   \
        printf("fstest: %s %s (%ld, errno %d)\n", ok_ ? "ok" : "FAIL", #expr, r_, errno);               \
        fails += !ok_;                                                                                 \
    } while (0)

/* The contents of `path`, into `buf`. */
static int slurp(const char *path, char *buf, size_t len)
{
    int fd = open(path, O_RDONLY);
    int n = fd < 0 ? -1 : (int)read(fd, buf, len - 1);
    if (fd >= 0) {
        close(fd);
    }
    buf[n > 0 ? n : 0] = 0;
    return n;
}

/* dup(), fork() and the shell's redirection share one open file
 * description: its position and its status flags (procnto). */
static void shared_descriptions(void)
{
    char buf[32];
    int fd = open("/dev/shmem/fs_dup", O_RDWR | O_CREAT | O_TRUNC, 0644);
    int d = dup(fd);
    V(write(fd, "aaaa", 4), 4);
    V(write(d, "bbbb", 4), 4);
    V(lseek(fd, 0, SEEK_CUR), 8);
    V(lseek(d, 0, SEEK_CUR), 8);
    V(lseek(d, 2, SEEK_SET), 2);
    V(lseek(fd, 0, SEEK_CUR), 2);
    slurp("/dev/shmem/fs_dup", buf, sizeof buf);
    V(strcmp(buf, "aaaabbbb"), 0);
    V(fcntl(d, F_SETFL, fcntl(d, F_GETFL) | O_APPEND), 0);
    V(fcntl(fd, F_GETFL) & O_APPEND, O_APPEND);
    struct stat st;
    V(fstat(fd, &st), 0);
    V(st.st_nlink, 2);
    close(d);
    close(fd);
    fd = open("/dev/shmem/fs_fork", O_RDWR | O_CREAT | O_TRUNC, 0644);
    V(write(fd, "pp", 2), 2);
    pid_t pid = fork();
    if (pid == 0) {
        write(fd, "cc", 2);
        _exit(0);
    }
    waitpid(pid, NULL, 0);
    V(write(fd, "qq", 2), 2);
    V(lseek(fd, 0, SEEK_CUR), 6);
    slurp("/dev/shmem/fs_fork", buf, sizeof buf);
    V(strcmp(buf, "ppccqq"), 0);
    close(fd);
    V(system("(echo one; echo two) > /dev/shmem/fs_redir"), 0);
    slurp("/dev/shmem/fs_redir", buf, sizeof buf);
    V(strcmp(buf, "one\ntwo\n"), 0);
    unlink("/dev/shmem/fs_dup");
    unlink("/dev/shmem/fs_fork");
    unlink("/dev/shmem/fs_redir");
}

int main(void)
{
    char buf[256];
    struct stat st;
    struct statvfs vf;
    E(open("/proc/boot/nonexist", O_RDONLY), ENOENT);
    E(open("/proc/boot/ksh", O_WRONLY), ENOENT);
    E(open("/proc/boot", O_WRONLY), EROFS);
    E(open("/proc/boot/errtest", O_RDWR), EROFS);
    V(open("/proc/boot/errtest", O_RDONLY | O_CREAT | O_EXCL, 0644), 3);
    E(open("/proc/boot/newfile", O_RDWR | O_CREAT, 0644), ENOENT);
    E(open("/proc/boot/errtest/x", O_RDONLY), ENOENT);
    E(open("/proc/boot/errtest", O_RDONLY | O_DIRECTORY), ENOTDIR);
    E(open("", O_RDONLY), ENOENT);
    E(mkdir("/proc/boot/d", 0755), ENOENT);
    E(rmdir("/proc/boot"), EROFS);
    E(unlink("/proc/boot/errtest"), EROFS);
    E(unlink("/proc/boot/nonexist"), ENOENT);
    E(rename("/proc/boot/errtest", "/proc/boot/x"), ENOSYS);
    E(link("/proc/boot/errtest", "/proc/boot/x"), ENOSYS);
    E(symlink("/proc/boot/errtest", "/proc/boot/x"), ENOENT);
    E(chmod("/proc/boot/errtest", 0777), ENOENT);
    E(chown("/proc/boot/errtest", 1, 1), ENOENT);
    E(truncate("/proc/boot/errtest", 0), EROFS);
    E(access("/proc/boot/errtest", W_OK), EROFS);
    V(access("/proc/boot/errtest", X_OK), 0);
    E(access("/proc/boot/nonexist", F_OK), ENOENT);
    E(readlink("/proc/boot/errtest", buf, sizeof buf), EINVAL);
    V(stat("/proc/boot/errtest/", &st), 0);
    V(statvfs("/proc/boot", &vf), 0);
    V(vf.f_flag & ST_RDONLY, 1);
    int fd = open("/proc/boot/errtest", O_RDONLY);
    E(write(fd, "x", 1), ENOSYS);
    V(lseek(fd, -5, SEEK_SET), 0);
    E(lseek(fd, 0, 99), EINVAL);
    V(lseek(fd, 0, SEEK_END) > 0, 1);
    E(ftruncate(fd, 0), ENOSYS);
    E(fchmod(fd, 0777), ENOSYS);
    E(fsync(fd), EINVAL);
    E(fdatasync(fd), EINVAL);
    V(read(fd, buf, 0), 0);
    E(pread(fd, buf, 4, -1), EINVAL);
    E(pread(fd, buf, 4, 1L << 40), EINVAL);
    V(fcntl(fd, F_GETFL), 32768);
    V(fcntl(fd, F_SETFL, O_APPEND | O_NONBLOCK), 0);
    V(fcntl(fd, F_GETFL), 136);
    E(fcntl(fd, 999), EINVAL);
    E(dup2(fd, -1), EBADF);
    E(dup2(fd, 100000), EBADF);
    E(close(999), EBADF);
    E(close(-1), EBADF);
    int d = open("/proc/boot", O_RDONLY);
    E(read(d, buf, sizeof buf), EISDIR);
    E(write(d, "x", 1), ENOSYS);
    V(lseek(d, 0, SEEK_CUR), 0);
    int n = open("/dev/null", O_RDWR);
    V(lseek(n, 100, SEEK_SET), 0);
    V(lseek(n, 0, SEEK_CUR), 0);
    V(read(n, buf, 10), 0);
    E(ftruncate(n, 0), ENOSYS);
    V(fstat(n, &st), 0);
    V(S_ISCHR(st.st_mode), 1);
    V(st.st_size, 0);
    E(fchmod(n, 0600), ENOSYS);
    V(isatty(n), 0);
    E(open("/dev/null/x", O_RDONLY), ENOENT);
    E(open("/proc/1/nonexist", O_RDONLY), ENOENT);
    E(open("/proc/99999", O_RDONLY), ENOENT);
    V(open("/proc/1/as", O_RDWR), 7);
    E(rmdir("/dev/shmem/nonexist"), ENOENT);
    V(open("/dev/shmem/f1", O_RDWR | O_CREAT, 0600), 8);
    V(rmdir("/dev/shmem/f1"), 0);
    E(unlink("/dev/shmem/f1"), ENOENT);
    E(unlink("/dev/shmem/f1"), ENOENT);
    shared_descriptions();
    printf("fstest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}
