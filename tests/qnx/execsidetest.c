/* At exec, a close-on-exec connection at a descriptor index to a plain
 * MsgReceive() server gets _IO_CLOSE, and a close-on-exec side-channel
 * connection doesn't (it isn't a file; a server that never answers an
 * unknown type would hang the exec): procnto, measured (review of #240,
 * item PROCMGR-8). The server, a parent, logs what it receives; the client
 * is its child. Prints one "ok"/"FAIL" line per check and
 * "execsidetest: passed". */
#include <errno.h>
#include <process.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <unistd.h>

#define SELF "/proc/boot/execsidetest"
static int fails;

static void check(const char *what, int ok)
{
    printf("execsidetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

int main(int argc, char **argv)
{
    if (argc > 1 && strcmp(argv[1], "after") == 0) {
        return 0;
    }
    if (argc > 3 && strcmp(argv[1], "client") == 0) {
        int spid = atoi(argv[2]), chid = atoi(argv[3]);
        ConnectAttach(0, spid, chid, _NTO_SIDE_CHANNEL, _NTO_COF_CLOEXEC);
        ConnectAttach(0, spid, chid, 3, _NTO_COF_CLOEXEC);
        execl(SELF, "execsidetest", "after", NULL);
        return 1;
    }
    int chid = ChannelCreate(_NTO_CHF_DISCONNECT);
    char a[16], b[16];
    snprintf(a, sizeof a, "%d", getpid());
    snprintf(b, sizeof b, "%d", chid);
    pid_t c = spawnl(P_NOWAIT, SELF, "execsidetest", "client", a, b, NULL);
    int closes_fd = 0, closes_side = 0, other = 0, disconnect = 0;
    for (int n = 0; n < 20 && !disconnect; n++) {
        char m[64];
        struct _msg_info info;
        uint64_t to = 3000000000ULL;
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &to, NULL);
        int r = MsgReceive(chid, m, sizeof m, &info);
        if (r == -1) {
            break;
        }
        if (r == 0) {
            disconnect = ((struct _pulse *)m)->code == _PULSE_CODE_DISCONNECT;
            continue;
        }
        unsigned short ty;
        memcpy(&ty, m, 2);
        if (ty == 0x116 && (info.coid & _NTO_SIDE_CHANNEL)) {
            closes_side++;
        } else if (ty == 0x116) {
            closes_fd++;
        } else {
            other++;
        }
        MsgError(r, ENOSYS);
    }
    waitpid(c, NULL, 0);
    check("_IO_CLOSE for the close-on-exec descriptor", closes_fd == 1);
    check("no _IO_CLOSE for the close-on-exec side channel", closes_side == 0);
    check("no other message", other == 0);
    check("the disconnect pulse once the connections are gone", disconnect);
    printf("execsidetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}
