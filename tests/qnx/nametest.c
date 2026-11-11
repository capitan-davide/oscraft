/* name_attach() names in procnto's path space (/dev/name/local):
 * - a name exists once (EEXIST); empty and absolute names are EINVAL;
 * - /dev/name/local lists the newest names first;
 * - procnto answers for a name to anything but name_open(): stat() gives
 *   an S_IFNAM entry, unlink() is ENOSYS;
 * - name_open() attaches to the server without a message to it (procnto
 *   answers the connect): the server's first message is the client's;
 * - a detached name is gone (ENOENT).
 * Prints one "ok"/"FAIL" line per check and "nametest: passed". */
#include <dirent.h>
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <sys/dispatch.h>
#include <sys/neutrino.h>
#include <sys/stat.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("nametest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static name_attach_t *served;
static char first[16];

/* The server: the first message it receives. */
static void *server(void *arg)
{
    (void)arg;
    char buf[64];
    int rcvid = MsgReceive(served->chid, buf, sizeof buf, NULL);
    if (rcvid > 0) {
        memcpy(first, buf, sizeof first);
        MsgReply(rcvid, 42, NULL, 0);
    }
    return NULL;
}

int main(void)
{
    name_attach_t *a = name_attach(NULL, "nametest_a", 0);
    check("name_attach()", a != NULL);
    errno = 0;
    check("the same name again: EEXIST", name_attach(NULL, "nametest_a", 0) == NULL && errno == EEXIST);
    errno = 0;
    check("an empty name: EINVAL", name_attach(NULL, "", 0) == NULL && errno == EINVAL);
    errno = 0;
    check("an absolute name: EINVAL", name_attach(NULL, "/abs", 0) == NULL && errno == EINVAL);
    name_attach_t *b = name_attach(NULL, "nametest_b", 0);

    DIR *d = opendir("/dev/name/local");
    int ia = -1, ib = -1, i = 0;
    struct dirent *e;
    while (d && (e = readdir(d))) {
        if (!strcmp(e->d_name, "nametest_a")) {
            ia = i;
        }
        if (!strcmp(e->d_name, "nametest_b")) {
            ib = i;
        }
        i++;
    }
    if (d) {
        closedir(d);
    }
    check("/dev/name/local lists the newest name first", ib >= 0 && ia > ib);

    struct stat st;
    check("stat() of a name: S_IFNAM 0666",
          stat("/dev/name/local/nametest_a", &st) == 0 && (st.st_mode & S_IFMT) == S_IFNAM &&
              (st.st_mode & 0777) == 0666);
    errno = 0;
    check("unlink() of a name: ENOSYS", unlink("/dev/name/local/nametest_a") == -1 && errno == ENOSYS);
    errno = 0;
    check("name_open() of no such name: ENOENT", name_open("nametest_none", 0) == -1 && errno == ENOENT);

    served = a;
    pthread_t t;
    pthread_create(&t, NULL, server, NULL);
    int coid = name_open("nametest_a", 0);
    check("name_open()", coid >= 0);
    char msg[16] = "hello";
    check("a message to the name's server", MsgSend(coid, msg, sizeof msg, NULL, 0) == 42);
    pthread_join(t, NULL);
    check("the server's first message is the client's (no connect)", !strcmp(first, "hello"));
    name_close(coid);

    check("name_detach()", name_detach(a, 0) == 0);
    errno = 0;
    check("a detached name: ENOENT", name_open("nametest_a", 0) == -1 && errno == ENOENT);
    name_detach(b, 0);
    printf("nametest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}
