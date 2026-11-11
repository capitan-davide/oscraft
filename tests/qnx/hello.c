/*
 * Smallest QNX program: static, libc's printf. Built with the SDP's
 * ntoaarch64-gcc; used to find out what QNX's libc needs from the kernel
 * and procnto.
 */
#include <stdio.h>

int main(int argc, char **argv)
{
    printf("hello from QNX libc, argc=%d argv[0]=%s\n", argc, argv[0]);
    return 0;
}
