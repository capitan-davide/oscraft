/* The I/O messages procnto's own resource managers answer, as procnto
 * (the image filesystem, /dev/null, /dev/zero, /dev/text, /proc):
 * messages shorter than their structure or of no defined type are EBADMSG;
 * stat() of the image's entries (trusted, device 0x802, a directory's
 * size its entries rounded up to ten) and of the devices (device 1, their
 * st_rdev); writes on read-only devices EBADF; /dev/text's open flags;
 * _IO_READ64, _IO_FDINFO, openfd() and statvfs() where they are refused;
 * /proc's stat() (inodes from the pid, devices, sizes) and its refusals;
 * /dev/shmem's stat() (devices, no blocks, no clock) and its directory;
 * the regions' flags, devices and offsets in DCMD_PROC_MAPINFO; vmstat,
 * pmap and mappings; /proc/mount; /proc/vm and /proc/ker.
 * Prints one "ok"/"FAIL" line per check and "iomsgtest: passed". */
#include <devctl.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/iomgr.h>
#include <sys/iomsg.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/procfs.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <time.h>
#include <unistd.h>

#define S_TRUST_EXT 0x20000

static int fails;

static void check(const char *what, int ok)
{
    printf("iomsgtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* A message of `len` bytes, zeros but its type; the status or -errno. */
static long raw(int fd, unsigned type, int len)
{
    uint8_t m[128];
    char r[256];
    memset(m, 0, sizeof m);
    m[0] = type;
    m[1] = type >> 8;
    long s = MsgSend(fd, m, len, r, sizeof r);
    return s == -1 ? -errno : s;
}

/* _IO_READ64 or _IO_WRITE64 of 4 bytes. */
static long rw64(int fd, unsigned type)
{
    struct {
        uint16_t type, combine_len;
        uint32_t nbytes, xtype, nbytes_hi;
        char data[4];
    } m = { type, 16, 4, 0, 0, "abc" };
    char r[16];
    long s = MsgSend(fd, &m, type == _IO_WRITE64 ? sizeof m : 16, r, sizeof r);
    return s == -1 ? -errno : s;
}

static int err(long r, int e)
{
    return r == -1 && errno == e;
}

int main(void)
{
    struct stat st;
    struct statvfs sv;
    struct _fdinfo info;
    char path[256], buf[16];

    int null = open("/dev/null", O_RDONLY);
    int boot = open("/proc/boot", O_RDONLY);
    check("a message shorter than its structure: EBADMSG",
          raw(null, _IO_STAT, 7) == -EBADMSG && raw(null, _IO_STAT, 8) == 3 && raw(boot, _IO_READ, 15) == -EBADMSG &&
              raw(null, _IO_CLOSE, 2) == -EBADMSG);
    check("types past _IO_UTIME64: EBADMSG; known ones not handled: ENOSYS",
          raw(null, 0x150, 64) == -EBADMSG && raw(null, 0x1ff, 64) == -EBADMSG && raw(null, _IO_MSG, 8) == -ENOSYS &&
              raw(boot, _IO_NOTIFY, 88) == -ENOSYS && raw(boot, _IO_NOTIFY, 87) == -EBADMSG);

    /* The image filesystem. */
    int n = 0;
    DIR *d = opendir("/proc/boot");
    while (d && readdir(d)) {
        n++;
    }
    closedir(d);
    fstat(boot, &st);
    printf("iomsgtest:   /proc/boot: mode %o size %lld (%d entries) nlink %d dev %#x\n", (unsigned)st.st_mode,
           (long long)st.st_size, n, (int)st.st_nlink, (unsigned)st.st_dev);
    check("an image directory: trusted, two links, its entries rounded up to ten",
          st.st_mode == (S_TRUST_EXT | S_IFDIR | 0755) && st.st_nlink == 2 && st.st_size == (n + 9) / 10 * 10 &&
              st.st_dev == 0x802 && st.st_blocks == 1);
    check("pread() of an image directory: EISDIR, EINVAL past 32 bits",
          err(pread(boot, buf, 1, 0), EISDIR) && err(pread(boot, buf, 1, 0xffffffffLL), EISDIR) &&
              err(pread(boot, buf, 1, 0x100000000LL), EINVAL));
    stat("/proc/boot/iomsgtest", &st);
    check("an image file: trusted", st.st_mode == (S_TRUST_EXT | S_IFREG | 0755) && st.st_dev == 0x802 && st.st_nlink == 1);
    lstat("/usr/tmp", &st);
    check("an image link: trusted, its target's length, no blocks",
          st.st_mode == (S_TRUST_EXT | S_IFLNK | 0777) && st.st_size == 4 && st.st_blocks == 0);
    int iofd = open("/proc/boot/iomsgtest", O_RDONLY);
    check("_IO_READ64 and _IO_WRITE64 of image files: ENOSYS",
          rw64(iofd, _IO_READ64) == -ENOSYS && rw64(iofd, _IO_WRITE64) == -ENOSYS && rw64(boot, _IO_READ64) == -ENOSYS);
    int r = iofdinfo(boot, 0, &info, path, sizeof path);
    check("iofdinfo() of an image directory: \"/\", one reader",
          r == 2 && strcmp(path, "/") == 0 && info.count == 1 && info.rcount == 1 && info.wcount == 0);
    check("openfd(): ENOSYS", err(openfd(iofd, O_RDONLY), ENOSYS) && err(openfd(null, O_RDONLY), ENOSYS));

    /* The devices. */
    fstat(null, &st);
    check("/dev/null: device 1, st_rdev 0x400, no block size",
          st.st_dev == 1 && st.st_rdev == 0x400 && st.st_blksize == 0 && st.st_mode == (S_IFCHR | 0666));
    int zero = open("/dev/zero", O_RDONLY);
    fstat(zero, &st);
    check("/dev/zero: st_rdev 0x406", st.st_dev == 1 && st.st_rdev == 0x406 && st.st_blksize == 0);
    check("writes on read-only devices: EBADF", err(write(null, "x", 1), EBADF) && err(write(zero, "x", 1), EBADF));
    check("lseek() of /dev/zero: ENOSYS; of /dev/null: 0",
          err(lseek(zero, 0, SEEK_END), ENOSYS) && lseek(null, 5, SEEK_SET) == 0);
    check("fstatvfs() of the devices: ENOSYS", err(fstatvfs(null, &sv), ENOSYS) && err(fstatvfs(zero, &sv), ENOSYS));
    int nullw = open("/dev/null", O_RDWR);
    check("_IO_READ64 and _IO_WRITE64 of /dev/null and /dev/zero",
          rw64(nullw, _IO_READ64) == 0 && rw64(nullw, _IO_WRITE64) == 4 && rw64(zero, _IO_READ64) == 4);

    /* /dev/text. */
    int text = open("/dev/text", O_RDONLY | O_NONBLOCK);
    fstat(text, &st);
    check("/dev/text: owner only, size 1, st_rdev 0x401",
          st.st_mode == (S_IFCHR | 0600) && st.st_size == 1 && st.st_rdev == 0x401 && st.st_dev == 1);
    check("/dev/text: its open flags", fcntl(text, F_GETFL) == (O_NONBLOCK | O_LARGEFILE));
    check("/dev/text: a non-blocking read with no input EAGAIN, a write EBADF",
          err(read(text, buf, sizeof buf), EAGAIN) && err(write(text, "x", 1), EBADF));
    check("/dev/text: pread(), lseek() and _IO_READ64 ENOSYS, fsync() EINVAL",
          err(pread(text, buf, 1, 0), ENOSYS) && err(lseek(text, 0, SEEK_SET), ENOSYS) &&
              rw64(text, _IO_READ64) == -ENOSYS && err(fsync(text), EINVAL));
    check("/dev/text: dup() keeps the flags", fcntl(dup(text), F_GETFL) == (O_NONBLOCK | O_LARGEFILE));

    /* /proc. */
    int proc = open("/proc", O_RDONLY);
    check("/proc: no iofdinfo(), fstatvfs() or _IO_READ64 (ENOSYS)",
          err(iofdinfo(proc, 0, &info, path, sizeof path), ENOSYS) && err(fstatvfs(proc, &sv), ENOSYS) &&
              rw64(proc, _IO_READ64) == -ENOSYS);
    fstat(proc, &st);
    check("/proc: device 1, two links, 512-byte blocks",
          st.st_dev == 1 && st.st_nlink == 2 && st.st_blksize == 512 && st.st_blocks == (st.st_size + 511) / 512);
    check("/proc: write EIO, pread() EISDIR, chmod() EIO, its end the end of user space",
          err(write(proc, "x", 1), EIO) && err(pread(proc, buf, 1, 0), EISDIR) && err(fchmod(proc, 0555), EIO) &&
              lseek(proc, 0, SEEK_END) == 1LL << 39);
    unsigned long long me = (unsigned long long)getpid() << 32;
    int self = open("/proc/self", O_RDONLY);
    fstat(self, &st);
    check("/proc/self: its inode from the pid, size 1, four links, device 0x800",
          st.st_ino == me && st.st_size == 1 && st.st_nlink == 4 && st.st_dev == 0x800 && st.st_blksize == 0);
    snprintf(path, sizeof path, "/proc/%d", getppid());
    stat(path, &st);
    check("another process's directory: two links", st.st_nlink == 2 && st.st_ino == (unsigned long long)getppid() << 32);
    check("/proc/self: write EBADF, lseek() only to 0", err(write(self, "x", 1), EBADF) &&
                                                             err(lseek(self, 5, SEEK_SET), EINVAL) && lseek(self, 0, SEEK_SET) == 0);
    unsigned long long as_ino = 0, ctl_ino = 0;
    d = opendir("/proc/self");
    struct dirent *e;
    while (d && (e = readdir(d))) {
        as_ino = strcmp(e->d_name, "as") == 0 ? e->d_ino : as_ino;
        ctl_ino = strcmp(e->d_name, "ctl") == 0 ? e->d_ino : ctl_ino;
    }
    check("its files' inodes: as 1, ctl 7", as_ino == (me | 1) && ctl_ino == (me | 7));
    int as = open("/proc/self/as", O_RDONLY);
    check("as: an unmapped address ESRVRFAULT, write EBADF, chmod() taken, its end the end of user space",
          err(pread(as, buf, 4, 0), ESRVRFAULT) && err(write(as, "x", 1), EBADF) && fchmod(as, 0600) == 0 &&
              lseek(as, 0, SEEK_END) == 1LL << 39);
    fstat(as, &st);
    off_t before = st.st_size;
    void *m = mmap(NULL, 1 << 20, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    void *x = mmap(NULL, 1 << 16, PROT_READ | PROT_EXEC, MAP_PRIVATE | MAP_ANON, -1, 0);
    fstat(as, &st);
    off_t with = st.st_size;
    munmap(m, 1 << 20);
    munmap(x, 1 << 16);
    fstat(as, &st);
    check("as: its size counts readable mappings (anonymous code too)",
          with == before + (1 << 20) + (1 << 16) && st.st_size == before);
    int ctl = open("/proc/self/ctl", O_RDONLY);
    check("ctl: read ENOSYS, pread() EINVAL", err(read(ctl, buf, 4), ENOSYS) && err(pread(ctl, buf, 4, 0), EINVAL));
    int exe = open("/proc/self/exefile", O_RDONLY);
    check("exefile: pread() EINVAL, lseek() past 0 EINVAL, chmod() EIO",
          read(exe, path, sizeof path) > 0 && err(pread(exe, buf, 1, 0), EINVAL) && err(lseek(exe, 0, SEEK_CUR), EINVAL) &&
              err(fchmod(exe, 0600), EIO));
    int conf = open("/proc/config", O_RDONLY);
    fstat(conf, &st);
    check("/proc/config: inode 0xc on device 0x409, its end 0",
          st.st_ino == 0xc && st.st_dev == 0x409 && lseek(conf, 0, SEEK_END) == 0);

    /* The regions as DCMD_PROC_MAPINFO describes them. */
    int file2 = open("/proc/boot/iomsgtest", O_RDONLY);
    void *anon = mmap(0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_LAZY, -1, 0);
    void *fixed = mmap((char *)anon + 4096, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0);
    void *code = mmap(0, 8192, PROT_READ | PROT_EXEC, MAP_PRIVATE, file2, 0);
    void *sanon = mmap(0, 8192, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANON, -1, 0);
    int stack;
    procfs_mapinfo mi[64];
    int nmi = 0;
    devctl(as, DCMD_PROC_MAPINFO, mi, sizeof mi, &nmi);
    unsigned f_anon = 0, f_fixed = 0, f_code = 0, f_sanon = 0, f_stack = 0, f_guard = 0, f_main = 0, pg = 0;
    unsigned long long code_off = 0, code_dev = 0, anon_dev = 0, anon_ino = 0;
    for (int i = 0; i < nmi && i < 64; i++) {
        uintptr_t lo = mi[i].vaddr, hi = lo + mi[i].size;
#define AT(p) (lo <= (uintptr_t)(p) && (uintptr_t)(p) < hi)
        pg |= mi[i].flags & PG_MASK;
        if (AT(anon)) { f_anon = mi[i].flags; anon_dev = mi[i].dev; anon_ino = mi[i].ino; }
        if (AT(fixed)) f_fixed = mi[i].flags;
        if (AT(code)) { f_code = mi[i].flags; code_off = mi[i].offset; code_dev = mi[i].dev; }
        if (AT(sanon)) f_sanon = mi[i].flags;
        if (AT(&stack)) {
            f_stack = mi[i].flags;
            /* The guard page right below. */
            for (int j = 0; j < nmi && j < 64; j++) {
                f_guard = mi[j].vaddr + mi[j].size == lo && mi[j].size == 4096 ? mi[j].flags : f_guard;
            }
        }
        if (AT(main)) f_main = mi[i].flags;
    }
    printf("iomsgtest:   anon %#x fixed %#x code %#x shared anon %#x stack %#x guard %#x main %#x\n", f_anon, f_fixed, f_code,
           f_sanon, f_stack, f_guard, f_main);
    check("anonymous memory: MAP_SYSRAM | MAP_ANON, /dev/zero on device 1, MAP_LAZY dropped",
          f_anon == (MAP_SYSRAM | MAP_ANON | PROT_READ | PROT_WRITE | MAP_PRIVATE) && anon_dev == 1 && anon_ino == 3);
    check("MAP_FIXED kept", f_fixed == (MAP_SYSRAM | MAP_ANON | PROT_READ | PROT_WRITE | MAP_PRIVATE | MAP_FIXED));
    check("an image file's pages: MAP_PHYS, at their physical address",
          f_code == (MAP_PHYS | PROT_READ | PROT_EXEC | MAP_PRIVATE) && code_dev == 0x802 && code_off != 0);
    check("shared anonymous memory: MAP_PHYS too",
          f_sanon == (MAP_SYSRAM | MAP_PHYS | MAP_ANON | PROT_READ | PROT_WRITE | MAP_SHARED));
    check("the main thread's stack and its guard", f_stack == 0x81081302 && f_guard == 0x80081002);
    check("the program's code: shared, MAP_FIXED | MAP_ELF", f_main == 0x10531);
    check("no PG_* bits", pg == 0);

    /* vmstat, pmap and mappings. */
    static char pt[65536];
    int vm = open("/proc/self/vmstat", O_RDONLY);
    int vn = read(vm, pt, sizeof pt - 1);
    pt[vn > 0 ? vn : 0] = 0;
    close(vm);
    unsigned long size = 0, shared = 0, priv = 0, heap0 = 0;
    char *v = strstr(pt, "as_stats.map_size=");
    if (v) size = strtoul(v + 18, NULL, 16);
    if ((v = strstr(pt, "as_stats.map_shared="))) shared = strtoul(v + 20, NULL, 16);
    if ((v = strstr(pt, "as_stats.map_private="))) priv = strtoul(v + 21, NULL, 16);
    if ((v = strstr(pt, "as_stats.rlimit_data="))) heap0 = strtoul(v + 21, NULL, 16);
    check("vmstat: procnto's lines, in pages",
          strncmp(pt, "as_stats.vm_region=", 19) == 0 && strstr(pt, "\nas_stats.vm_map=") &&
              strstr(pt, "\nas_stats.map_phys=") && strstr(pt, "\nas_stats.rss=") && strstr(pt, "\nas_stats.anon_rsv=") &&
              size == shared + priv && strstr(pt, "kB)\n"));
    void *more = mmap(0, 1 << 20, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    vm = open("/proc/self/vmstat", O_RDONLY);
    vn = read(vm, pt, sizeof pt - 1);
    pt[vn > 0 ? vn : 0] = 0;
    close(vm);
    unsigned long heap1 = (v = strstr(pt, "as_stats.rlimit_data=")) ? strtoul(v + 21, NULL, 16) : 0;
    check("vmstat: the heap counts private anonymous memory", heap1 == heap0 + 256);
    munmap(more, 1 << 20);
    int pm = open("/proc/self/pmap", O_RDONLY);
    int first = read(pm, pt, sizeof pt - 1);
    int rest = read(pm, pt + first, sizeof pt - 1 - first);
    pt[first + (rest > 0 ? rest : 0)] = 0;
    close(pm);
    check("pmap: the header comes alone, then the regions",
          first == 85 && strncmp(pt, "vaddr,size,flags,prot,maxprot,dev,ino,offset,rsv,guardsize,refcnt,mapcnt,path,asinfo\n", 85) == 0 &&
              rest > 0);
    check("pmap: the program's code, the main stack with its guard",
          strstr(pt, ",0x00010031,0x05,0x0d,0x00000802,") && strstr(pt, "/proc/boot/iomsgtest,{ram}") &&
              strstr(pt, ",0x0000000000080000,0x00081002,0x03,0x0f,0x00000001,0x0000000000000003,0x0000000000000000,0x0000000000080000,0x00001000,") &&
              strstr(pt, "{stack},{sysram}"));
    char *three = mmap(0, 3 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    char want[64];
    snprintf(want, sizeof want, "0x%lx,0x00080002,", (unsigned long)(uintptr_t)three);
    int mp = open("/proc/self/mappings", O_RDONLY);
    int pages = 0, got, header = read(mp, pt, sizeof pt);
    while ((got = read(mp, pt, sizeof pt - 1)) > 0) {
        pt[got] = 0;
        if (strncmp(pt, want, strlen(want)) == 0) {
            for (char *l = pt; (l = strchr(l, '\n')); l++) {
                pages++;
            }
        }
    }
    close(mp);
    check("mappings: the header alone, then a line per page, a region per read", header == 137 && pages == 3);
    munmap(three, 3 * 4096);

    /* /proc/mount: the path space by server. */
    char leaf[128] = "", image_leaf[300] = "";
    int names = 0, has_dev = 0, has_proc = 0;
    d = opendir("/proc/mount");
    while (d && (e = readdir(d))) {
        names++;
        has_dev |= strcmp(e->d_name, "dev") == 0;
        has_proc |= strcmp(e->d_name, "proc") == 0;
        snprintf(path, sizeof path, "/proc/mount/%s/proc/boot", e->d_name);
        if (strchr(e->d_name, ',') && stat(path, &st) == 0 && st.st_dev == 0x802) {
            snprintf(image_leaf, sizeof image_leaf, "%s", e->d_name);
        }
    }
    if (d) closedir(d);
    check("/proc/mount: the root's servers and the names below it", has_dev && has_proc && image_leaf[0]);
    d = opendir("/proc/mount/dev/null");
    names = 0;
    while (d && (e = readdir(d))) {
        names++;
        snprintf(leaf, sizeof leaf, "%s", e->d_name);
    }
    if (d) closedir(d);
    snprintf(path, sizeof path, "/proc/mount/dev/null/%s", leaf);
    int vianull = open(path, O_WRONLY);
    stat(path, &st);
    check("/proc/mount/dev/null: its one server, reached by its name", names == 1 && S_ISCHR(st.st_mode) &&
                                                                          write(vianull, "abc", 3) == 3);
    close(vianull);
    stat("/proc/mount/dev", &st);
    check("/proc/mount's directories: size 1, device 1", S_ISDIR(st.st_mode) && (st.st_mode & 0777) == 0555 &&
                                                           st.st_size == 1 && st.st_dev == 1);
    snprintf(path, sizeof path, "/proc/mount/dev/null/%s/x", leaf);
    int mnt = open("/proc/mount", O_RDONLY);
    check("/proc/mount: no such name ENOENT, a path below a device ENOTDIR, no iofdinfo()",
          err(stat("/proc/mount/nonexist", &st), ENOENT) && err(stat("/proc/mount/dev/nonexist", &st), ENOENT) &&
              err(stat(path, &st), ENOTDIR) && err(iofdinfo(mnt, 0, &info, path, sizeof path), ENOSYS));
    close(mnt);

    /* procnto's statistics: /proc/vm, /proc/ker. */
    int vmd = 0, kerd = 0;
    d = opendir("/proc");
    while (d && (e = readdir(d))) {
        vmd |= strcmp(e->d_name, "vm") == 0;
        kerd |= strcmp(e->d_name, "ker") == 0;
    }
    if (d) closedir(d);
    int vs = open("/proc/vm/stats", O_RDONLY), ks = open("/proc/ker/stats", O_RDONLY), ki = open("/proc/ker/intr", O_RDONLY);
    char sb[1024] = "", kb[1024] = "", ib[64] = "";
    read(vs, sb, sizeof sb - 1);
    read(ks, kb, sizeof kb - 1);
    read(ki, ib, sizeof ib - 1);
    stat("/proc/vm/stats", &st);
    check("/proc/vm/stats, /proc/ker/stats and intr: procnto's lines",
          vmd && kerd && strncmp(sb, "vm_aspace=", 10) == 0 && strstr(sb, "\npage_count=0x") &&
              strstr(sb, "\npages_reserved=0x") && strncmp(kb, "Processes:", 10) == 0 && strcmp(ib, "intrctlr:N/A\n") == 0 &&
              st.st_mode == (S_IFREG | 0444) && st.st_size == 1 && st.st_ino == 0xa);
    check("the statistics: no lseek(), write() or chmod() (ENOSYS), not for writing (EACCES)",
          err(lseek(vs, 0, SEEK_SET), ENOSYS) && err(write(ks, "x", 1), ENOSYS) && err(fchmod(ki, 0444), ENOSYS) &&
              err(open("/proc/vm/stats", O_RDWR), EACCES));
    close(vs);
    close(ks);
    close(ki);

    /* /dev/shmem. */
    int shm = shm_open("/iomsgtest", O_RDWR | O_CREAT, 0600);
    ftruncate(shm, 8192);
    fstat(shm, &st);
    check("a shared memory object: device 1, no blocks, no time",
          st.st_dev == 1 && st.st_rdev == 2 && st.st_blocks == 0 && st.st_mtime == 0);
    struct timespec ts[2] = { { 1000, 0 }, { 2000, 0 } };
    futimens(shm, ts);
    fstat(shm, &st);
    long given = st.st_mtime;
    write(shm, "x", 1);
    fstat(shm, &st);
    check("a time given is kept, a write's is 0", given == 2000 && st.st_mtime == 0);
    int file = open("/dev/shmem/iomsgtest.f", O_RDWR | O_CREAT, 0600);
    fstat(file, &st);
    check("a file of /dev/shmem: no st_rdev", S_ISREG(st.st_mode) && st.st_rdev == 0 && st.st_dev == 1);
    check("no links in /dev/shmem (ENOSYS)",
          err(symlink("/x", "/dev/shmem/iomsgtest.l"), ENOSYS) && err(link("/dev/shmem/iomsgtest.f", "/dev/shmem/iomsgtest.h"), ENOSYS));
    int sd = open("/dev/shmem", O_RDONLY);
    fstat(sd, &st);
    check("/dev/shmem: device 0x407, no time", st.st_dev == 0x407 && st.st_mtime == 0 && st.st_nlink == 2);
    check("/dev/shmem: writes take nothing, pread() reads nothing, utime() taken, chmod() and ftruncate() ENOSYS",
          write(sd, "xy", 2) == 0 && pread(sd, buf, 4, 0) == 0 && futimens(sd, NULL) == 0 && err(fchmod(sd, 0777), ENOSYS) &&
              err(ftruncate(sd, 0), ENOSYS));
    r = iofdinfo(sd, 0, &info, path, sizeof path);
    check("/dev/shmem: iofdinfo() names \"memory\", its access mode only",
          r == 7 && strcmp(path, "memory") == 0 && info.ioflag == 1);
    unlink("/dev/shmem/iomsgtest.f");
    shm_unlink("/iomsgtest");

    printf("iomsgtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}
