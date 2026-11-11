/*
 * FreeBSD's sockets for the server in front of them: netd's calls, one per
 * operation, with FreeBSD's return convention (0 or an errno). A socket here
 * is a struct socket: there are no file descriptors below netd's own table.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/ioccom.h>
#include <sys/kernel.h>
#include <sys/lock.h>
#include <sys/malloc.h>
#include <sys/mbuf.h>
#include <sys/mutex.h>
#include <sys/proc.h>
#include <sys/poll.h>
#include <sys/protosw.h>
#include <sys/selinfo.h>
#include <sys/sleepqueue.h>
#include <sys/socket.h>
#include <sys/socketvar.h>
#include <sys/sockio.h>
#include <sys/sysctl.h>
#include <sys/uio.h>
#include <sys/ucred.h>
#include <net/if.h>
#include <net/if_var.h>
#include <net/vnet.h>
#include <netinet/in.h>
#include <netinet/in_var.h>

#include "bsdkern.h"

/* A thread of the server gets a struct thread at its first call. */
static struct thread *
me(void)
{
	bsdkern_attach();
	return (curthread);
}

int
bsdnet_socket(int domain, int type, int proto, struct socket **so)
{
	struct thread *td = me();
	int e;

	CURVNET_SET(vnet0);
	e = socreate(domain, so, type, proto, td->td_ucred, td);
	CURVNET_RESTORE();
	return (e);
}

int
bsdnet_close(struct socket *so)
{
	(void)me();
	return (soclose(so));
}

int
bsdnet_bind(struct socket *so, struct sockaddr *a)
{
	return (sobind(so, a, me()));
}

int
bsdnet_connect(struct socket *so, struct sockaddr *a)
{
	struct thread *td = me();
	int e;

	e = soconnect(so, a, td);
	if (e != 0)
		return (e);
	if ((so->so_state & SS_NBIO) != 0 && (so->so_state & SS_ISCONNECTING) != 0)
		return (EINPROGRESS);
	SOCK_LOCK(so);
	while ((so->so_state & SS_ISCONNECTING) != 0 && so->so_error == 0)
		msleep(&so->so_timeo, &so->so_lock, PSOCK | PCATCH, "connec", 0);
	e = so->so_error;
	so->so_error = 0;
	SOCK_UNLOCK(so);
	return (e);
}

int
bsdnet_listen(struct socket *so, int backlog)
{
	return (solisten(so, backlog, me()));
}

int
bsdnet_accept(struct socket *head, struct socket **out, struct sockaddr *a)
{
	struct socket *so;
	int e;

	(void)me();
	SOLISTEN_LOCK(head);
	e = solisten_dequeue(head, &so, 0);
	if (e != 0)
		return (e);
	if (a != NULL)
		a->sa_len = sizeof(struct sockaddr_storage);
	e = soaccept(so, a);
	*out = so;
	return (e);
}

int
bsdnet_send(struct socket *so, const void *buf, size_t *len,
    struct sockaddr *to, int flags)
{
	struct thread *td = me();
	struct iovec iov = { .iov_base = __DECONST(void *, buf), .iov_len = *len };
	struct uio uio = {
		.uio_iov = &iov, .uio_iovcnt = 1, .uio_resid = *len,
		.uio_segflg = UIO_SYSSPACE, .uio_rw = UIO_WRITE, .uio_td = td,
	};
	int e;

	e = sosend(so, to, &uio, NULL, NULL, flags, td);
	*len -= uio.uio_resid;
	return (e);
}

int
bsdnet_recv(struct socket *so, void *buf, size_t *len, struct sockaddr *from,
    int *flags)
{
	struct thread *td = me();
	struct iovec iov = { .iov_base = buf, .iov_len = *len };
	struct uio uio = {
		.uio_iov = &iov, .uio_iovcnt = 1, .uio_resid = *len,
		.uio_segflg = UIO_SYSSPACE, .uio_rw = UIO_READ, .uio_td = td,
	};
	struct sockaddr *sa = NULL;
	int e;

	e = soreceive(so, from != NULL ? &sa : NULL, &uio, NULL, NULL, flags);
	*len -= uio.uio_resid;
	if (sa != NULL) {
		memcpy(from, sa, MIN(sa->sa_len, sizeof(struct sockaddr_storage)));
		free(sa, M_SONAME);
	} else if (from != NULL)
		from->sa_len = 0;
	return (e);
}

int
bsdnet_sockaddr(struct socket *so, struct sockaddr *a, int peer)
{
	(void)me();
	a->sa_len = sizeof(struct sockaddr_storage);
	return (peer ? sopeeraddr(so, a) : sosockaddr(so, a));
}

int
bsdnet_shutdown(struct socket *so, int how)
{
	(void)me();
	return (soshutdown(so, how));
}

int
bsdnet_setopt(struct socket *so, int level, int name, void *val, size_t len)
{
	struct sockopt o = {
		.sopt_dir = SOPT_SET, .sopt_level = level, .sopt_name = name,
		.sopt_val = val, .sopt_valsize = len, .sopt_td = me(),
	};

	return (sosetopt(so, &o));
}

int
bsdnet_getopt(struct socket *so, int level, int name, void *val, size_t *len)
{
	struct sockopt o = {
		.sopt_dir = SOPT_GET, .sopt_level = level, .sopt_name = name,
		.sopt_val = val, .sopt_valsize = *len, .sopt_td = me(),
	};
	int e;

	e = sogetopt(so, &o);
	*len = o.sopt_valsize;
	return (e);
}

void
bsdnet_nbio(struct socket *so, int on)
{
	SOCK_LOCK(so);
	if (on)
		so->so_state |= SS_NBIO;
	else
		so->so_state &= ~SS_NBIO;
	SOCK_UNLOCK(so);
}

/*
 * poll(). selwakeuppri() is called only for a selinfo with someone on it, and
 * FreeBSD's selrecord() is not here: a selinfo is marked as watched for good.
 */
void *
bsdnet_watch(struct socket *so, int write)
{
	struct selinfo *si = write ? &so->so_wrsel : &so->so_rdsel;

	si->si_tdlist.tqh_first = (void *)1;
	return (si);
}

int
bsdnet_poll(struct socket *so, int events)
{
	struct thread *td = me();

	return (so->so_proto->pr_sopoll(so, events, td));
}

int
bsdnet_ioctl(struct socket *so, u_long cmd, void *data)
{
	struct thread *td = me();
	int e;

	if (IOCGROUP(cmd) != 'i')
		return (ENOTTY);
	CURVNET_SET(so->so_vnet);
	e = ifioctl(so, cmd, data, td);
	CURVNET_RESTORE();
	return (e);
}

int
bsdnet_nread(struct socket *so)
{
	return (sbavail(&so->so_rcv));
}

int
bsdnet_sysctl(int *name, u_int namelen, void *old, size_t *oldlen, void *new,
    size_t newlen)
{
	size_t got = 0;
	int e;

	e = kernel_sysctl(me(), name, namelen, old, oldlen, new, newlen, &got, 0);
	*oldlen = got;
	return (e);
}

/*
 * This thread acts for a client: what FreeBSD stores, a socket's owner, has
 * its IDs. A socket holds its credential, so a new one, not this one changed.
 */
void
bsdnet_client(uid_t uid, gid_t gid)
{
	struct thread *td = me();
	struct ucred *old = td->td_ucred, *cr;

	if (old->cr_uid == uid && old->cr_gid == gid)
		return;
	cr = crget();
	crcopy(cr, old);
	cr->cr_uid = cr->cr_ruid = cr->cr_svuid = uid;
	cr->cr_gid = cr->cr_rgid = cr->cr_svgid = gid;
	td->td_ucred = cr;
	crfree(old);
}

void *
bsdnet_self(void)
{
	return (me());
}

/*
 * A client wants out (a signal, a timeout): the thread serving it, if it
 * sleeps interruptibly, stops with EINTR, as a signal does in a kernel. 1 if
 * it was, 0 if it isn't asleep there yet.
 */
int
bsdnet_interrupt(struct thread *td)
{
	thread_lock(td);
	if (TD_ON_SLEEPQ(td) && (td->td_flags & TDF_SINTR) != 0) {
		sleepq_abort(td, EINTR);
		return (1);
	}
	thread_unlock(td);
	return (0);
}

/* Loopback up, 127.0.0.1/8: what rc.d/netif does, here at start. */
int
bsdnet_loopback_up(void)
{
	struct thread *td = me();
	struct ifaliasreq ifra;
	struct ifreq ifr;
	struct sockaddr_in *sin;
	struct socket *so;
	int e;

	CURVNET_SET(vnet0);
	e = socreate(AF_INET, &so, SOCK_DGRAM, 0, td->td_ucred, td);
	if (e == 0) {
		bzero(&ifr, sizeof(ifr));
		strlcpy(ifr.ifr_name, "lo0", sizeof(ifr.ifr_name));
		e = ifioctl(so, SIOCIFCREATE, (caddr_t)&ifr, td);
		if (e == EEXIST)
			e = 0;
		bzero(&ifra, sizeof(ifra));
		strlcpy(ifra.ifra_name, "lo0", sizeof(ifra.ifra_name));
		sin = (struct sockaddr_in *)&ifra.ifra_addr;
		sin->sin_len = sizeof(*sin);
		sin->sin_family = AF_INET;
		sin->sin_addr.s_addr = htonl(INADDR_LOOPBACK);
		sin = (struct sockaddr_in *)&ifra.ifra_mask;
		sin->sin_len = sizeof(*sin);
		sin->sin_family = AF_INET;
		sin->sin_addr.s_addr = htonl(0xff000000);
		if (e == 0)
			e = ifioctl(so, SIOCAIFADDR, (caddr_t)&ifra, td);
		soclose(so);
	}
	CURVNET_RESTORE();
	return (e);
}
