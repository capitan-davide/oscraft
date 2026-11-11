/* Channels: a wide channel (_NTO_CCTL_WIDE, io-sock's qnx.chan_width)
 * is also named by the channel IDs after its own, its lanes, on which it
 * can be received from. A receive on a channel another thread destroys
 * fails with ESRCH, whether it was blocked already or arrives as the
 * channel goes (review item IPC-7: the kernel used to let a receive
 * that found the channel between its lookup and its destruction join a
 * queue nothing drains). Prints one "ok"/"FAIL" line per case. */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <unistd.h>

static int fails;
static volatile int race_chid;
static volatile int race_errs, race_other;

/* receive on race_chid until the channel goes */
static void *receiver(void *a)
{
    (void)a;
    for (;;) {
        char buf[64];
        errno = 0;
        int r = MsgReceive(race_chid, buf, sizeof buf, NULL);
        if (r == -1) {
            if (errno == ESRCH) {
                race_errs++;
            } else {
                race_other = errno;
            }
            return NULL;
        }
    }
}

static void check(const char *what, int ok)
{
    printf("chantest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

int main(void)
{
    int width = 4;
    int chid = ChannelCreate(0);
    check("create", chid > 0);
    check("widen", ChannelCtl(chid, _NTO_CCTL_WIDE, &width) == 0);
    int next = ChannelCreate(0);
    check("lanes skipped by the next channel", next == chid + width);

    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    check("connect", coid != -1);
    check("pulse", MsgSendPulse(coid, -1, 5, 42) == 0);
    struct _pulse p = {0};
    int r = MsgReceivePulse(chid + 2, &p, sizeof p, NULL);
    check("pulse received on a lane", r == 0 && p.code == 5 && p.value.sival_int == 42);
    check("connect to a lane", ConnectAttach(0, 0, chid + 3, _NTO_SIDE_CHANNEL, 0) != -1);

    /* The IDs after `next` are taken by another channel. */
    int other = ChannelCreate(0);
    int two = 2;
    check("widen over a taken ID",
          other == next + 1 && ChannelCtl(next, _NTO_CCTL_WIDE, &two) == -1 && errno == EBUSY);

    check("a lane isn't destroyed (ESRCH, as procnto)", ChannelDestroy(chid + 1) == -1 && errno == ESRCH);
    check("destroy", ChannelDestroy(chid) == 0);
    check("all lanes freed", ChannelCreate(0) == chid && ChannelCreate(0) == chid + 1);

    /* a receiver blocked on a channel that is destroyed: ESRCH */
    pthread_t t;
    race_chid = ChannelCreate(0);
    pthread_create(&t, NULL, receiver, NULL);
    usleep(20000);
    check("destroy under a blocked receiver", ChannelDestroy(race_chid) == 0);
    pthread_join(t, NULL);
    check("the blocked receiver got ESRCH", race_errs == 1 && race_other == 0);
    /* receives arriving as the channel goes, many times over */
    race_errs = race_other = 0;
    int rounds = 300;
    for (int i = 0; i < rounds; i++) {
        race_chid = ChannelCreate(0);
        pthread_create(&t, NULL, receiver, NULL);
        for (volatile int spin = 0; spin < (i % 50) * 20; spin++) {
        }
        ChannelDestroy(race_chid);
        pthread_join(t, NULL);
    }
    check("every receive racing a destroy got ESRCH", race_errs == rounds && race_other == 0);
    printf("chantest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}
