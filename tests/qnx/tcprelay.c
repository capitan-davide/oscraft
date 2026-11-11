/* tcprelay: join a serial line to a TCP port on the target, so that a host
 * tool can reach a TCP service through QEMU's serial port (the raspi4b
 * machine has no network device). Used to test qconn: the SDP's gdb
 * connects to QEMU's UART0 socket, and `target qnx` reaches qconn on
 * 127.0.0.1:8000 through this relay.
 *
 * Usage: tcprelay <serial device> <ip address> <port>
 * The connection is made when the first bytes arrive on the line and
 * dropped when the TCP peer closes; the next bytes make a new one. */
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <termios.h>
#include <unistd.h>

static int tcp_connect(const char *ip, int port)
{
    struct sockaddr_in a = { .sin_family = AF_INET, .sin_port = htons(port) };
    inet_pton(AF_INET, ip, &a.sin_addr);
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0 || connect(s, (struct sockaddr *)&a, sizeof a) < 0) {
        perror("tcprelay: connect");
        if (s >= 0) {
            close(s);
        }
        return -1;
    }
    return s;
}

/* Copy what `from` has to `to`; false at end of file or on an error. */
static int pass(int from, int to)
{
    char buf[4096];
    ssize_t n = read(from, buf, sizeof buf);
    if (n <= 0) {
        return 0;
    }
    for (ssize_t done = 0; done < n;) {
        ssize_t w = write(to, buf + done, n - done);
        if (w <= 0) {
            return 0;
        }
        done += w;
    }
    return 1;
}

int main(int argc, char **argv)
{
    if (argc != 4) {
        fprintf(stderr, "usage: tcprelay <serial device> <ip address> <port>\n");
        return 1;
    }
    int line = open(argv[1], O_RDWR);
    if (line < 0) {
        perror("tcprelay: open");
        return 1;
    }
    struct termios t;
    if (tcgetattr(line, &t) == 0) {
        cfmakeraw(&t);
        tcsetattr(line, TCSANOW, &t);
    }
    int sock = -1;
    for (;;) {
        fd_set r;
        FD_ZERO(&r);
        FD_SET(line, &r);
        if (sock >= 0) {
            FD_SET(sock, &r);
        }
        if (select((sock > line ? sock : line) + 1, &r, NULL, NULL, NULL) < 0) {
            if (errno == EINTR) {
                continue;
            }
            perror("tcprelay: select");
            return 1;
        }
        if (FD_ISSET(line, &r)) {
            if (sock < 0 && (sock = tcp_connect(argv[2], atoi(argv[3]))) < 0) {
                char drop[4096];
                (void)read(line, drop, sizeof drop);
                continue;
            }
            if (!pass(line, sock)) {
                perror("tcprelay: line");
                return 1;
            }
        }
        if (sock >= 0 && FD_ISSET(sock, &r) && !pass(sock, line)) {
            close(sock);
            sock = -1;
        }
    }
}
