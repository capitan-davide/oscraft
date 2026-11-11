/* Server connection IDs, as procnto: one per client process and channel,
 * shared by all its connections there; numbered per server from 1, the
 * lowest free one; _PULSE_CODE_DISCONNECT when the client's last
 * connection goes, the ID reused once the server detaches it (at once on
 * a channel without _NTO_CHF_DISCONNECT); a forked child gets its own.
 * Prints one "ok"/"FAIL" line per check and "scoidtest: passed". */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("scoidtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* Shared with the server: its channels, and what it saw. */
struct shared {
    int chid_a, chid_b;
    int pulses, pulse_scoid;
};
static struct shared *sh;
static pid_t srv;

/* The scoid the server saw the message on (its reply's status). */
static int scoid_of(int coid)
{
    int tag = 1;
    int r = MsgSend(coid, &tag, sizeof tag, NULL, 0);
    return r == -1 ? -errno : r;
}

/* A server: channel A with _NTO_CHF_DISCONNECT, channel B without; it
 * replies with the message's scoid, counts disconnect pulses and detaches
 * their scoids. */
static void server(void)
{
    sh->chid_a = ChannelCreate(_NTO_CHF_DISCONNECT);
    sh->chid_b = ChannelCreate(0);
    for (;;) {
        struct _msg_info info;
        union {
            int tag;
            struct _pulse pulse;
        } m;
        struct sigevent to;
        SIGEV_UNBLOCK_INIT(&to);
        for (int c = 0; c < 2; c++) {
            uint64_t ns = 10000000;
            TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, &to, &ns, NULL);
            rcvid_t r = MsgReceive(c ? sh->chid_b : sh->chid_a, &m, sizeof m, &info);
            if (r == 0 && m.pulse.code == _PULSE_CODE_DISCONNECT) {
                sh->pulse_scoid = m.pulse.scoid;
                sh->pulses++;
                ConnectDetach(m.pulse.scoid);
            } else if (r > 0) {
                MsgReply(r, info.scoid, NULL, 0);
            }
        }
    }
}

int main(void)
{
    sh = mmap(0, sizeof *sh, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANON, -1, 0);
    srv = fork();
    if (srv == 0) {
        server();
    }
    usleep(100000);
    int a1 = ConnectAttach(0, srv, sh->chid_a, _NTO_SIDE_CHANNEL, 0);
    int a2 = ConnectAttach(0, srv, sh->chid_a, _NTO_SIDE_CHANNEL, 0);
    int b1 = ConnectAttach(0, srv, sh->chid_b, _NTO_SIDE_CHANNEL, 0);
    int s1 = scoid_of(a1), s2 = scoid_of(a2), s3 = scoid_of(b1);
    printf("scoidtest:   a1 %#x a2 %#x b1 %#x\n", s1, s2, s3);
    check("a process's connections to a channel share their scoid", s1 == 0x10001 && s2 == 0x10001);
    check("another channel of the same server, another scoid", s3 == 0x10002);
    struct _server_info si;
    ConnectServerInfo(0, a1, &si);
    check("ConnectServerInfo() gives it", si.scoid == 0x10001);
    ConnectDetach(a1);
    usleep(100000);
    int after_one = sh->pulses;
    ConnectDetach(a2);
    usleep(100000);
    check("the disconnect pulse after the last connection only", after_one == 0 && sh->pulses == 1 &&
                                                                   sh->pulse_scoid == 0x10001);
    int a3 = ConnectAttach(0, srv, sh->chid_a, _NTO_SIDE_CHANNEL, 0);
    check("the scoid the server detached is used again", scoid_of(a3) == 0x10001);
    ConnectDetach(b1);
    int b2 = ConnectAttach(0, srv, sh->chid_b, _NTO_SIDE_CHANNEL, 0);
    check("without _NTO_CHF_DISCONNECT, at once", scoid_of(b2) == 0x10002);
    pid_t c = fork();
    if (c == 0) {
        int ca = ConnectAttach(0, srv, sh->chid_a, _NTO_SIDE_CHANNEL, 0);
        _exit(scoid_of(ca) == 0x10003 ? 0 : 1);
    }
    int st;
    waitpid(c, &st, 0);
    usleep(100000);
    check("a forked child gets its own; its end, a pulse", WIFEXITED(st) && WEXITSTATUS(st) == 0 && sh->pulses == 2 &&
                                                             sh->pulse_scoid == 0x10003);
    kill(srv, SIGKILL);
    waitpid(srv, NULL, 0);
    printf("scoidtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}
