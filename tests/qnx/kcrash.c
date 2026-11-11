/* Crash the OsCraft kernel on purpose (its private _OSCRAFT_DEBUG_CRASH
 * message to procnto, root only), to test kernel dumps (QNX's kdumper). */
#include <stdint.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <sys/sysmgr.h>

int main(void)
{
    uint16_t msg = 0xf001;
    int rc = MsgSend(SYSMGR_COID, &msg, sizeof msg, NULL, 0);
    printf("kcrash: the kernel is still running (%d)\n", rc);
    return 1;
}
