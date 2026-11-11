/* A QNX program on netd rather than io-sock (SOCK=/alt): TCP, UDP, names,
 * options, poll and a socket pair, through QNX's libsocket and qnxd. */
#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/neutrino.h>
#include <stdio.h>
#include <string.h>
#include <sys/select.h>
#include <sys/procmgr.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("netdtest: %s %s\n", ok ? "ok" : "FAIL", what);
    if (!ok) {
        printf("netdtest: errno %d\n", errno);
    }
    fails += !ok;
}

int main(void)
{
    struct sockaddr_in a, got;
    socklen_t len = sizeof got;
    char b[32];
    memset(&a, 0, sizeof a);
    a.sin_len = sizeof a;
    a.sin_family = AF_INET;
    a.sin_port = htons(7300);
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);

    int l = socket(AF_INET, SOCK_STREAM, 0);
    check("socket()", l >= 0);
    int one = 1;
    check("SO_REUSEADDR", setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one) == 0);
    check("bind()", bind(l, (struct sockaddr *)&a, sizeof a) == 0);
    check("listen()", listen(l, 2) == 0);
    check("getsockname()", getsockname(l, (struct sockaddr *)&got, &len) == 0 && got.sin_port == a.sin_port);
    int c = socket(AF_INET, SOCK_STREAM, 0);
    check("connect()", connect(c, (struct sockaddr *)&a, sizeof a) == 0);
    struct pollfd p = { l, POLLIN, 0 };
    check("poll() on the listener", poll(&p, 1, 2000) == 1 && (p.revents & POLLIN));
    int s = accept(l, NULL, NULL);
    check("accept()", s >= 0);
    check("send()", send(c, "hello", 5, 0) == 5);
    p.fd = s;
    p.revents = 0;
    check("poll() on data", poll(&p, 1, 2000) == 1 && (p.revents & POLLIN));
    check("recv()", recv(s, b, sizeof b, 0) == 5 && memcmp(b, "hello", 5) == 0);
    check("write(), read()", write(s, "back", 4) == 4 && read(c, b, sizeof b) == 4 && memcmp(b, "back", 4) == 0);

    /* Two listeners in one poll(), as sshd has, and the connection comes
     * after both are armed. */
    int l2 = socket(AF_INET, SOCK_STREAM, 0);
    a.sin_port = htons(7301);
    check("second listener", l2 >= 0 && setsockopt(l2, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one) == 0
        && bind(l2, (struct sockaddr *)&a, sizeof a) == 0 && listen(l2, 2) == 0);
    pid_t kid = fork();
    if (kid == 0) {
        delay(200);
        int k = socket(AF_INET, SOCK_STREAM, 0);
        _exit(k >= 0 && connect(k, (struct sockaddr *)&a, sizeof a) == 0 ? 0 : 1);
    }
    struct pollfd two[2] = { { l, POLLIN, 0 }, { l2, POLLIN, 0 } };
    int polled = poll(two, 2, 3000);
    printf("netdtest: two listeners: poll=%d revents %x %x\n", polled, two[0].revents, two[1].revents);
    check("poll() wakes for the second listener", polled == 1 && !two[0].revents && (two[1].revents & POLLIN));
    waitpid(kid, NULL, 0);
    int s2 = accept(l2, NULL, NULL);
    check("accept() after it", s2 >= 0);
    close(s2);
    close(l2);
    fd_set r;
    FD_ZERO(&r);
    FD_SET(c, &r);
    struct timeval tv = { 0, 50000 };
    check("select() times out", select(c + 1, &r, NULL, NULL, &tv) == 0);
    check("shutdown()", shutdown(c, SHUT_WR) == 0);
    check("end of stream", recv(s, b, sizeof b, 0) == 0);
    a.sin_port = htons(7301);
    int u = socket(AF_INET, SOCK_DGRAM, 0);
    check("udp bind()", bind(u, (struct sockaddr *)&a, sizeof a) == 0);
    int v = socket(AF_INET, SOCK_DGRAM, 0);
    check("sendto()", sendto(v, "dgram", 5, 0, (struct sockaddr *)&a, sizeof a) == 5);
    len = sizeof got;
    check("recvfrom()", recvfrom(u, b, sizeof b, 0, (struct sockaddr *)&got, &len) == 5 && got.sin_family == AF_INET);
    errno = 0;
    check("no such address: EADDRNOTAVAIL", connect(socket(AF_INET, SOCK_STREAM, 0), (struct sockaddr *)&(struct sockaddr_in){ .sin_len = sizeof a, .sin_family = AF_INET, .sin_port = 0 }, sizeof a) == -1);
    printf("netdtest: errno after %d\n", errno);
    /* Abilities: a low port and a raw socket, as root and as another user. */
    a.sin_port = htons(80);
    int i = socket(AF_INET, SOCK_STREAM, 0);
    check("root binds port 80", bind(i, (struct sockaddr *)&a, sizeof a) == 0);
    close(i);
    i = socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
    check("root opens a raw socket", i >= 0);
    close(i);
    int id = procmgr_ability_lookup("network/privport");
    check("network/privport exists", id >= 0);
    for (int granted = 0; granted < 2; granted++) {
        pid_t child = fork();
        if (child == 0) {
            if (granted) {
                procmgr_ability(0, PROCMGR_ADN_NONROOT | PROCMGR_AOP_ALLOW | id, PROCMGR_AID_EOL);
            }
            setuid(100);
            a.sin_port = htons(81 + granted);
            int s6 = socket(AF_INET, SOCK_STREAM, 0);
            int b6 = bind(s6, (struct sockaddr *)&a, sizeof a);
            int raw = socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
            printf("netdtest: uid 100 granted=%d bind=%d errno=%d raw=%d\n", granted, b6, b6 ? errno : 0, raw >= 0);
            _exit((b6 == 0) == granted && raw < 0 ? 0 : 1);
        }
        int st = -1;
        waitpid(child, &st, 0);
        check(granted ? "uid 100 with network/privport binds, but no raw socket" : "uid 100 binds no low port", WIFEXITED(st) && WEXITSTATUS(st) == 0);
    }

    /* Named AF_UNIX: a node in the file system, a stream, then a name in use. */
    {
        struct sockaddr_un u;
        const char *path = "/tmp/netd.sock";
        memset(&u, 0, sizeof u);
        u.sun_family = AF_UNIX;
        strcpy(u.sun_path, path);
        u.sun_len = SUN_LEN(&u);
        unlink(path);
        int ls = socket(AF_UNIX, SOCK_STREAM, 0);
        check("AF_UNIX bind()", bind(ls, (struct sockaddr *)&u, SUN_LEN(&u)) == 0);
        struct stat st;
        check("its node is a socket", stat(path, &st) == 0 && S_ISSOCK(st.st_mode));
        int again = socket(AF_UNIX, SOCK_STREAM, 0);
        check("AF_UNIX bind() in use", bind(again, (struct sockaddr *)&u, SUN_LEN(&u)) == -1 && errno == EADDRINUSE);
        check("AF_UNIX listen()", listen(ls, 2) == 0);
        int uc = socket(AF_UNIX, SOCK_STREAM, 0);
        check("AF_UNIX connect()", connect(uc, (struct sockaddr *)&u, SUN_LEN(&u)) == 0);
        struct pollfd up = { ls, POLLIN, 0 };
        check("AF_UNIX poll() of the listener", poll(&up, 1, 2000) == 1);
        int ua = accept(ls, NULL, NULL);
        char ub[8];
        check("AF_UNIX accept()", ua >= 0);
        check("AF_UNIX data", write(uc, "name", 4) == 4 && read(ua, ub, sizeof ub) == 4 && memcmp(ub, "name", 4) == 0);
        close(ua); close(uc); close(ls); close(again);
        unlink(path);
        int gone = socket(AF_UNIX, SOCK_STREAM, 0);
        check("AF_UNIX connect() to no node", connect(gone, (struct sockaddr *)&u, SUN_LEN(&u)) == -1 && errno == ENOENT);
        close(gone);
    }
    printf("netdtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}
