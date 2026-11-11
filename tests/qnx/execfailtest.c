/* An exec or spawn of an image that can't be loaded (review item
 * PROCMGR-7), as procnto answers (measured): the call fails and the
 * caller goes on, with ELIBACC for an interpreter that can't be found
 * and for a segment whose file and memory offsets disagree within the
 * page, ENOMEM for a segment past the end of the address space or more
 * memory than there is, and ENOEXEC for a segment past the end of the
 * file or two segments on the same pages. The kernel used to
 * tear the process down after a failed exec and report exit status 0 to
 * the parent, and ENOEXEC for every spawn. The images are copies of
 * this program with one program header changed. Prints one "ok"/"FAIL"
 * line per check and "execfailtest: passed". */
#include <sys/elf.h>
#include <errno.h>
#include <fcntl.h>
#include <process.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static unsigned char *image, *orig;
static size_t size;
static Elf64_Phdr *ph;
static int firstload, lastload, interp;

static void check(const char *what, int ok)
{
    printf("execfailtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* write the patched image as `path`, spawn it and exec it from a forked
 * child; `err` is the errno both must fail with (0: it must run) */
static void try(const char *what, const char *path, int err)
{
    char line[160];
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0755);
    write(fd, image, size);
    close(fd);
    chmod(path, 0755);
    errno = 0;
    int st = spawnl(P_WAIT, path, path, "child", NULL);
    snprintf(line, sizeof line, "%s: spawnl(P_WAIT) -> %d errno %d", what, st, errno);
    check(line, err ? st == -1 && errno == err : st != -1 && WIFEXITED(st) && WEXITSTATUS(st) == 7);
    pid_t pid = fork();
    if (pid == 0) {
        execl(path, path, "child", NULL);
        _exit(100 + errno);
    }
    int ws = 0;
    waitpid(pid, &ws, 0);
    snprintf(line, sizeof line, "%s: fork+exec -> %s %d", what, WIFEXITED(ws) ? "exited" : "signal",
             WIFEXITED(ws) ? WEXITSTATUS(ws) : WTERMSIG(ws));
    check(line, WIFEXITED(ws) && WEXITSTATUS(ws) == (err ? 100 + err : 7));
    unlink(path);
    memcpy(image, orig, size);
}

int main(int argc, char **argv)
{
    if (argc > 1) {
        return 7; /* a copy that ran */
    }
    int fd = open("/proc/boot/execfailtest", O_RDONLY);
    struct stat st;
    check("open(/proc/boot/execfailtest)", fd >= 0 && fstat(fd, &st) == 0);
    size = st.st_size;
    image = malloc(size);
    orig = malloc(size);
    read(fd, image, size);
    close(fd);
    memcpy(orig, image, size);
    Elf64_Ehdr *eh = (Elf64_Ehdr *)image;
    ph = (Elf64_Phdr *)(image + eh->e_phoff);
    firstload = lastload = interp = -1;
    for (int i = 0; i < eh->e_phnum; i++) {
        if (ph[i].p_type == PT_INTERP) interp = i;
        if (ph[i].p_type == PT_LOAD) {
            if (firstload < 0) firstload = i;
            lastload = i;
        }
    }
    check("the image has an interpreter and PT_LOAD segments", interp >= 0 && lastload >= 0);
    try("intact copy", "/dev/shmem/execfail_ok", 0);
    char *s = (char *)(image + ph[interp].p_offset);
    s[strlen(s) - 1] = 'X';
    try("missing interpreter", "/dev/shmem/execfail_interp", ELIBACC);
    ph[lastload].p_memsz = 1ULL << 40;
    try("PT_LOAD of 1 TiB", "/dev/shmem/execfail_1t", ENOMEM);
    ph[lastload].p_memsz = 64ULL << 30;
    try("PT_LOAD of 64 GiB", "/dev/shmem/execfail_64g", ENOMEM);
    ph[lastload].p_vaddr = ph[lastload].p_paddr = (1ULL << 39) - 0x1000;
    try("PT_LOAD at the end of the address space", "/dev/shmem/execfail_end", ENOMEM);
    ph[lastload].p_offset = size + 0x10000;
    try("PT_LOAD past the end of the file", "/dev/shmem/execfail_past", ENOEXEC);
    ph[lastload].p_vaddr = ph[lastload].p_paddr = (ph[firstload].p_vaddr & ~0xfffULL) | (ph[lastload].p_offset & 0xfff);
    try("two PT_LOADs on the same pages", "/dev/shmem/execfail_overlap", ENOEXEC);
    ph[lastload].p_vaddr = ph[lastload].p_paddr = ph[firstload].p_vaddr;
    try("two PT_LOADs on the same pages, the second misaligned too", "/dev/shmem/execfail_overlap2", ELIBACC);
    free(image);
    free(orig);
    printf("execfailtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}
