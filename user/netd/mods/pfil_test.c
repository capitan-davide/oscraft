/*
 * pfil_test: a packet filter as a netd module, through FreeBSD's pfil(9):
 * it drops IPv4 ICMP echo requests coming in, and counts them in
 * net.pfil_test.dropped. Loaded, it hooks itself to the "inet" head;
 * unloaded, it takes the hook away, as FreeBSD's filters do.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/mbuf.h>
#include <sys/module.h>
#include <sys/socket.h>
#include <sys/sysctl.h>

#include <net/if.h>
#include <net/pfil.h>
#include <netinet/in.h>
#include <netinet/in_systm.h>
#include <netinet/ip.h>
#include <netinet/ip_icmp.h>
#include <netinet/ip_var.h>

static u_long dropped;
static pfil_hook_t hook;

static SYSCTL_NODE(_net, OID_AUTO, pfil_test, CTLFLAG_RW | CTLFLAG_MPSAFE, 0,
    "pfil_test, a netd module");
SYSCTL_ULONG(_net_pfil_test, OID_AUTO, dropped, CTLFLAG_RD, &dropped, 0,
    "ICMP echo requests dropped");

static pfil_return_t
check(struct mbuf **mp, struct ifnet *ifp, int flags, void *ruleset, struct inpcb *inp)
{
	struct ip *ip;
	struct icmp *icmp;
	int hlen;

	(void)ifp;
	(void)flags;
	(void)ruleset;
	(void)inp;
	if ((*mp)->m_len < (int)sizeof(struct ip) &&
	    (*mp = m_pullup(*mp, sizeof(struct ip))) == NULL)
		return (PFIL_CONSUMED);
	ip = mtod(*mp, struct ip *);
	if (ip->ip_p != IPPROTO_ICMP)
		return (PFIL_PASS);
	hlen = ip->ip_hl << 2;
	if ((*mp)->m_len < hlen + ICMP_MINLEN &&
	    (*mp = m_pullup(*mp, hlen + ICMP_MINLEN)) == NULL)
		return (PFIL_CONSUMED);
	icmp = (struct icmp *)(mtod(*mp, char *) + hlen);
	if (icmp->icmp_type != ICMP_ECHO)
		return (PFIL_PASS);
	atomic_add_long(&dropped, 1);
	m_freem(*mp);
	*mp = NULL;
	return (PFIL_DROPPED);
}

static int
pfil_test_modevent(module_t mod, int what, void *arg)
{
	struct pfil_hook_args ha;
	struct pfil_link_args la;
	int error;

	(void)mod;
	(void)arg;
	switch (what) {
	case MOD_LOAD:
		memset(&ha, 0, sizeof(ha));
		ha.pa_version = PFIL_VERSION;
		ha.pa_flags = PFIL_IN;
		ha.pa_type = PFIL_TYPE_IP4;
		ha.pa_mbuf_chk = check;
		ha.pa_modname = "pfil_test";
		ha.pa_rulname = "default";
		hook = pfil_add_hook(&ha);
		memset(&la, 0, sizeof(la));
		la.pa_version = PFIL_VERSION;
		la.pa_flags = PFIL_IN | PFIL_HOOKPTR;
		la.pa_headname = PFIL_INET_NAME;
		la.pa_hook = hook;
		error = pfil_link(&la);
		if (error != 0)
			pfil_remove_hook(hook);
		return (error);
	case MOD_UNLOAD:
		pfil_remove_hook(hook);
		return (0);
	default:
		return (EOPNOTSUPP);
	}
}

static moduledata_t pfil_test_mod = {
	"pfil_test",
	pfil_test_modevent,
	NULL
};
DECLARE_MODULE(pfil_test, pfil_test_mod, SI_SUB_PROTO_PFIL, SI_ORDER_ANY);
MODULE_VERSION(pfil_test, 1);
