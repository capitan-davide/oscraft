/* A FAT file system served by diskd-md, on an image real tools made:
   FreeBSD's semantics, through libc, the messages and msdosfs.
   Prints one "ok"/"FAIL" line per check and "fs-test: passed". */
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int fails;

static void
check(const char *what, int ok)
{
	printf("fs-test: %s %s\n", ok ? "ok" : "FAIL", what);
	fflush(stdout);
	fails += !ok;
}

static int
listed(const char *dir, const char *name)
{
	DIR *d = opendir(dir);
	struct dirent *e;
	int found = 0;
	while (d != NULL && (e = readdir(d)) != NULL)
		found |= strcmp(e->d_name, name) == 0;
	if (d != NULL)
		closedir(d);
	return found;
}

int
main(void)
{
	char b[4096];
	struct stat st;

	int in = open("/proc/boot/fat.img", O_RDONLY);
	int out = open("/tmp/fat.img", O_WRONLY | O_CREAT | O_TRUNC, 0644);
	ssize_t n;
	static char chunk[65536];
	while ((n = read(in, chunk, sizeof chunk)) > 0)
		write(out, chunk, n);
	close(in);
	close(out);
	pid_t server = fork();
	if (server == 0) {
		execl("/proc/boot/diskd-md", "diskd-md", "-f", "/tmp/fat.img", "-m", "/md0", (char *)NULL);
		_exit(127);
	}
	for (int i = 0; i < 200 && stat("/md0/HELLO.TXT", &st) != 0; i++)
		usleep(10000);

	int f = open("/md0/HELLO.TXT", O_RDONLY);
	n = f < 0 ? -1 : read(f, b, sizeof b);
	check("reads what mcopy wrote", n == 12 && memcmp(b, "hello, disk\n", 12) == 0);
	check("end of file", read(f, b, 1) == 0);
	check("fstat", fstat(f, &st) == 0 && st.st_size == 12 && S_ISREG(st.st_mode));
	check("lseek", lseek(f, 7, SEEK_SET) == 7 && read(f, b, 4) == 4 && memcmp(b, "disk", 4) == 0);
	close(f);
	check("case doesn't matter", stat("/md0/hello.txt", &st) == 0);
	check("a long name mcopy wrote", stat("/md0/a long name.text", &st) == 0);
	check("readdir", listed("/md0", "HELLO.TXT") && listed("/md0", "a long name.text"));
	check("missing: ENOENT", open("/md0/none", O_RDONLY) < 0 && errno == ENOENT);

	f = open("/md0/new file.txt", O_WRONLY | O_CREAT | O_EXCL, 0644);
	check("creates", f >= 0);
	check("writes", write(f, "first ", 6) == 6 && write(f, "second", 6) == 6);
	close(f);
	check("O_EXCL: EEXIST", open("/md0/new file.txt", O_WRONLY | O_CREAT | O_EXCL, 0644) < 0 && errno == EEXIST);
	f = open("/md0/new file.txt", O_WRONLY | O_APPEND);
	write(f, "!", 1);
	close(f);
	f = open("/md0/new file.txt", O_RDONLY);
	n = read(f, b, sizeof b);
	close(f);
	check("reads it back, appended", n == 13 && memcmp(b, "first second!", 13) == 0);

	check("mkdir", mkdir("/md0/dir", 0755) == 0);
	check("mkdir again: EEXIST", mkdir("/md0/dir", 0755) != 0 && errno == EEXIST);
	check("rename into it", rename("/md0/new file.txt", "/md0/dir/moved") == 0);
	check("listed there", listed("/md0/dir", "moved"));
	check("gone from here", stat("/md0/new file.txt", &st) != 0 && errno == ENOENT);
	check("rmdir with something in it: ENOTEMPTY", rmdir("/md0/dir") != 0 && errno == ENOTEMPTY);
	check("truncate", truncate("/md0/dir/moved", 5) == 0 && stat("/md0/dir/moved", &st) == 0 && st.st_size == 5);
	f = open("/md0/dir/moved", O_WRONLY | O_TRUNC);
	close(f);
	check("O_TRUNC", stat("/md0/dir/moved", &st) == 0 && st.st_size == 0);
	check("unlink", unlink("/md0/dir/moved") == 0);
	check("rmdir", rmdir("/md0/dir") == 0);

	static char big[200000];
	memset(big, 'z', sizeof big);
	f = open("/md0/big", O_WRONLY | O_CREAT, 0644);
	check("a large write", write(f, big, sizeof big) == (ssize_t)sizeof big);
	close(f);
	f = open("/md0/big", O_RDONLY);
	size_t total = 0;
	while ((n = read(f, chunk, sizeof chunk)) > 0)
		total += n;
	close(f);
	check("a large read", total == sizeof big);
	/* A link's length has no NUL in it, which realpath() relies on. */
	char link[64];
	check("readlink", readlink("/usr/bin/od", link, sizeof link) == 16 && memcmp(link, "/usr/bin/hexdump", 16) == 0);
	check("realpath through a link", realpath("/usr/bin/od", link) != NULL && strcmp(link, "/usr/bin/hexdump") == 0);

	/* msdosfs_setattr()'s rules. */
	f = open("/md0/attrs", O_WRONLY | O_CREAT, 0644);
	close(f);
	check("chmod: read-only", chmod("/md0/attrs", 0444) == 0 && stat("/md0/attrs", &st) == 0 && (st.st_mode & 0222) == 0);
	f = open("/md0/attrs", O_WRONLY);
	check("read-only isn't enforced, as msdosfs_access()", f >= 0 && write(f, "x", 1) == 1);
	close(f);
	check("chmod: writable", chmod("/md0/attrs", 0644) == 0 && stat("/md0/attrs", &st) == 0 && (st.st_mode & 0200) != 0);
	check("chmod of the root: EINVAL", chmod("/md0", 0755) != 0 && errno == EINVAL);
	check("chown to the mount's owner", chown("/md0/attrs", 0, 0) == 0 && chown("/md0/attrs", -1, -1) == 0);
	check("chown as root to another: EINVAL", chown("/md0/attrs", 1000, 0) != 0 && errno == EINVAL);
	if (fork() == 0) {
		setuid(1000);
		errno = 0;
		_exit(chown("/md0/attrs", 1000, 0) != 0 && errno == EPERM ? 0 : 1);
	}
	int st2 = -1;
	wait(&st2);
	check("chown as a user: EPERM", WIFEXITED(st2) && WEXITSTATUS(st2) == 0);
	struct timeval tv[2] = { { 1000000000, 0 }, { 1000000000, 0 } };
	check("utimes", utimes("/md0/attrs", tv) == 0 && stat("/md0/attrs", &st) == 0 && st.st_mtime == 1000000000);

	/* pread(), pwrite(): the file's position stays. */
	f = open("/md0/HELLO.TXT", O_RDWR);
	check("pread", pread(f, b, 4, 7) == 4 && memcmp(b, "disk", 4) == 0);
	check("the position stays", lseek(f, 0, SEEK_CUR) == 0);
	check("pwrite", pwrite(f, "DISK", 4, 7) == 4 && pread(f, b, 4, 7) == 4 && memcmp(b, "DISK", 4) == 0);

	/* mmap() of a file: a copy, fully backed, as QNX 8 maps. */
	char *p = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, f, 0);
	check("mmap private", p != MAP_FAILED && memcmp(p, "hello, DISK\n", 12) == 0);
	check("past the end, zeros", p != MAP_FAILED && p[12] == 0);
	if (p != MAP_FAILED)
		munmap(p, 4096);
	p = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE, f, 0);
	if (p != MAP_FAILED) {
		p[0] = 'J';
		munmap(p, 4096);
	}
	check("a private change stays private", pread(f, b, 1, 0) == 1 && b[0] == 'h');
	p = mmap(NULL, 4096, PROT_READ, MAP_SHARED, f, 0);
	check("mmap shared, read-only", p != MAP_FAILED && p[0] == 'h');
	errno = 0;
	check("mmap shared and writable: ENODEV",
	    mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, f, 0) == MAP_FAILED && errno == ENODEV);
	close(f);

	/* A program on the disk runs, with its libraries from /lib. */
	in = open("/bin/echo", O_RDONLY);
	out = open("/md0/echo", O_WRONLY | O_CREAT, 0755);
	while ((n = read(in, chunk, sizeof chunk)) > 0)
		write(out, chunk, n);
	close(in);
	close(out);
	pid_t run = fork();
	if (run == 0) {
		execl("/md0/echo", "echo", "from the disk", (char *)NULL);
		printf("fs-test: exec: errno %d\n", errno);
		_exit(127);
	}
	int status = -1;
	waitpid(run, &status, 0);
	check("a program on the disk runs", WIFEXITED(status) && WEXITSTATUS(status) == 0);

	printf("fs-test: %s\n", fails ? "FAILED" : "passed");
	return fails != 0;
}
