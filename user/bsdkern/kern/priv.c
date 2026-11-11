/*
 * priv_check() and priv_check_cred(), in place of kern_priv.c: see
 * src/privilege.rs. The answer is the client's abilities, whoever the
 * credential says. io-sock's priv_check_cred() has the same shape.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/priv.h>
#include <sys/proc.h>
#include <sys/ucred.h>

int	bsdkern_priv_denied(int);

int
priv_check_cred(struct ucred *cred, int priv)
{
	(void)cred;
	return (bsdkern_priv_denied(priv));
}

int
priv_check(struct thread *td, int priv)
{
	(void)td;
	return (bsdkern_priv_denied(priv));
}
