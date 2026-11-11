/* Pseudo-terminals as QNX programs see them, on the native manager
 * (user/pty): open rules, grantpt(), ptsname(), the worker's defaults, and
 * each end's close. The expected lines in tests/target.tests are the
 * output on QNX's devc-pty, measured with tools/qnx/probe.py. */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <termios.h>
#include <unistd.h>
#include <sys/stat.h>
#include <poll.h>
int main(void){ char b[64]; int n;
 errno=0; int w0=open("/dev/ttyp0",O_RDWR|O_NONBLOCK); printf("worker first nonblock %d errno %d\n",w0>=0,errno); if(w0>=0)close(w0);
 int m=open("/dev/ptyp0",O_RDWR); printf("controller %d\n",m>=0);
 errno=0; int m2=open("/dev/ptyp0",O_RDWR); printf("second %d errno %d\n",m2>=0,errno);
 printf("grantpt %d unlockpt %d ptsname %s\n",grantpt(m),unlockpt(m),ptsname(m)?ptsname(m):"null");
 int s=open("/dev/ttyp0",O_RDWR); printf("worker %d isatty %d\n",s>=0,isatty(s));
 struct stat st; stat("/dev/ttyp0",&st); printf("mode %o\n",st.st_mode&0777);
 struct termios t; tcgetattr(s,&t); printf("flags %x %x %x %x cc %d %d %d %d %d\n",t.c_iflag,t.c_oflag,t.c_cflag,t.c_lflag,t.c_cc[VINTR],t.c_cc[VERASE],t.c_cc[VKILL],t.c_cc[VEOF],t.c_cc[VMIN]);
 write(m,"hi\n",3); n=read(s,b,64); printf("worker read %d\n",n); n=read(m,b,64); printf("echo %d\n",n);
 write(s,"o\n",2); n=read(m,b,64); printf("out %d\n",n);
 errno=0; close(s); n=read(m,b,1); printf("after worker close: read %d errno %d\n",n,errno);
 errno=0; n=write(m,"x",1); printf("write %d errno %d\n",n,errno);
 s=open("/dev/ttyp0",O_RDWR); close(m); errno=0; n=read(s,b,1); printf("after controller close: read %d errno %d\n",n,errno);
 errno=0; n=write(s,"x",1); printf("write %d errno %d\n",n,errno); close(s); return 0;}
