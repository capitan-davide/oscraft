/*
 * A QNX client's priority reaches a native server through qnxd: priochain
 * answers each message with the priority it runs at, boosted above its
 * own and lowered below it as the client's priority. OsCraft's design;
 * there is no qnxd on QNX.
 */
#include <fcntl.h>
#include <sched.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <unistd.h>

static int fails;

static void at(int coid, int prio)
{
    struct sched_param p = { 0 };
    p.sched_priority = prio;
    SchedSet(0, 0, SCHED_FIFO, &p);
    unsigned msg = 0x7f00, got = 0;
    int r = MsgSend(coid, &msg, sizeof msg, &got, sizeof got);
    int ok = r == 0 && (int)got == prio;
    printf("priochaintest: %s client at %d, server at %d\n", ok ? "ok" : "FAIL", prio, (int)got);
    fails += !ok;
}

int main(void)
{
    int pid = 0, chid = -1;
    FILE *f = NULL;
    for (int i = 0; i < 200 && !f; i++) {
        f = fopen("/dev/shmem/priochain", "r");
        if (!f) {
            usleep(10000);
        }
    }
    if (!f || fscanf(f, "%d %d", &pid, &chid) != 2) {
        printf("priochaintest: FAIL no server\n");
        return 1;
    }
    fclose(f);
    unlink("/dev/shmem/priochain");
    int coid = ConnectAttach(0, pid, chid, _NTO_SIDE_CHANNEL, 0);
    at(coid, 40);
    at(coid, 15);
    at(coid, 60);
    at(coid, 5);
    unsigned msg = 0, got;
    MsgSend(coid, &msg, sizeof msg, &got, sizeof got);
    printf("priochaintest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}
