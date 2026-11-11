/*
 * Devices: FreeBSD's make_dev() and the rest, for a server holding drivers.
 * There is no devfs: a device is a struct cdev in a list here, with its
 * name and its driver's functions, and the server is told of each
 * (bsdkern_cdev_added(), bsdkern_cdev_removed(), its own, weak) to give it
 * a path of its own and call the functions for its clients. Per-open data
 * (devfs_set_cdevpriv()) is that of the open the thread serves, which the
 * server names (bsdkern_cdev_serving()). destroy_dev() waits for the
 * threads inside the device's functions, as devfs's does
 * (destroy_devl()), so a driver may go away under clients.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/conf.h>
#include <sys/kernel.h>
#include <sys/lock.h>
#include <sys/malloc.h>
#include <sys/mutex.h>
#include <sys/queue.h>
#include <machine/stdarg.h>

#include "bsdkern.h"

void	bsdkern_cdev_added(struct cdev *) __attribute__((weak));
void	bsdkern_cdev_removed(struct cdev *) __attribute__((weak));
struct cdev *bsdkern_cdev_find(const char *name);
void	*bsdkern_cdev_enter(struct cdev *dev);
void	bsdkern_cdev_leave(struct cdev *dev);
void	bsdkern_cdev_serving(void **priv, d_priv_dtor_t **dtr);

struct node {
	struct cdev	dev;
	TAILQ_ENTRY(node) link;
	int		inside;		/* threads in its functions */
	bool		gone;
};

static TAILQ_HEAD(, node) nodes = TAILQ_HEAD_INITIALIZER(nodes);
static struct mtx nodes_lock;
MTX_SYSINIT(cdev_nodes, &nodes_lock, "cdev nodes", MTX_DEF);

static struct cdev *
make(struct cdevsw *devsw, int unit, uid_t uid, gid_t gid, int mode,
    void *drv1, void *drv2, struct cdev *parent, const char *fmt, va_list ap)
{
	struct node *n = malloc(sizeof(*n), M_DEVBUF, M_WAITOK | M_ZERO);
	struct cdev *dev = &n->dev;

	vsnprintf(dev->si_name, sizeof(dev->si_name), fmt, ap);
	dev->si_devsw = devsw;
	dev->si_drv0 = unit;
	dev->si_uid = uid;
	dev->si_gid = gid;
	dev->si_mode = mode;
	dev->si_drv1 = drv1;
	dev->si_drv2 = drv2;
	dev->si_parent = parent;
	dev->si_refcount = 1;
	mtx_lock(&nodes_lock);
	TAILQ_INSERT_TAIL(&nodes, n, link);
	mtx_unlock(&nodes_lock);
	if (bsdkern_cdev_added != NULL)
		bsdkern_cdev_added(dev);
	return (dev);
}

struct cdev *
make_dev(struct cdevsw *devsw, int unit, uid_t uid, gid_t gid, int mode,
    const char *fmt, ...)
{
	struct cdev *dev;
	va_list ap;

	va_start(ap, fmt);
	dev = make(devsw, unit, uid, gid, mode, NULL, NULL, NULL, fmt, ap);
	va_end(ap);
	return (dev);
}

struct cdev *
make_dev_credf(int flags, struct cdevsw *devsw, int unit, struct ucred *cr,
    uid_t uid, gid_t gid, int mode, const char *fmt, ...)
{
	struct cdev *dev;
	va_list ap;

	(void)flags;
	(void)cr;
	va_start(ap, fmt);
	dev = make(devsw, unit, uid, gid, mode, NULL, NULL, NULL, fmt, ap);
	va_end(ap);
	return (dev);
}

void
make_dev_args_init_impl(struct make_dev_args *args, size_t sz)
{
	bzero(args, sz);
	args->mda_size = sz;
}

int
make_dev_s(struct make_dev_args *args, struct cdev **cdev, const char *fmt, ...)
{
	va_list ap;

	va_start(ap, fmt);
	*cdev = make(args->mda_devsw, args->mda_unit, args->mda_uid, args->mda_gid,
	    args->mda_mode, args->mda_si_drv1, args->mda_si_drv2, NULL, fmt, ap);
	va_end(ap);
	return (0);
}

/* Another name for the same device: a device of its own, its parent's driver. */
struct cdev *
make_dev_alias(struct cdev *pdev, const char *fmt, ...)
{
	struct cdev *dev;
	va_list ap;

	va_start(ap, fmt);
	dev = make(pdev->si_devsw, dev2unit(pdev), pdev->si_uid, pdev->si_gid,
	    pdev->si_mode, pdev->si_drv1, pdev->si_drv2, pdev, fmt, ap);
	va_end(ap);
	return (dev);
}

/* No longer found by name; its functions may still be running. */
void
delist_dev(struct cdev *dev)
{
	struct node *n = __containerof(dev, struct node, dev);
	bool listed;

	mtx_lock(&nodes_lock);
	listed = !n->gone;
	if (listed) {
		n->gone = true;
		TAILQ_REMOVE(&nodes, n, link);
	}
	mtx_unlock(&nodes_lock);
	if (listed && bsdkern_cdev_removed != NULL)
		bsdkern_cdev_removed(dev);
}

/* Delisted, and freed once no thread is inside its functions. */
void
destroy_dev(struct cdev *dev)
{
	struct node *n = __containerof(dev, struct node, dev);

	delist_dev(dev);
	mtx_lock(&nodes_lock);
	while (n->inside > 0)
		msleep(n, &nodes_lock, 0, "devdrn", hz / 10);
	mtx_unlock(&nodes_lock);
	free(n, M_DEVBUF);
}

struct cdev *
bsdkern_cdev_find(const char *name)
{
	struct node *n;

	mtx_lock(&nodes_lock);
	TAILQ_FOREACH(n, &nodes, link)
		if (strcmp(n->dev.si_name, name) == 0)
			break;
	mtx_unlock(&nodes_lock);
	return (n != NULL ? &n->dev : NULL);
}

/*
 * Around a call of a device's function for a client: its driver's switch,
 * or NULL once it is going (the client's call fails, ENXIO).
 */
void *
bsdkern_cdev_enter(struct cdev *dev)
{
	struct node *n = __containerof(dev, struct node, dev);
	void *devsw = NULL;

	mtx_lock(&nodes_lock);
	if (!n->gone) {
		n->inside++;
		devsw = dev->si_devsw;
	}
	mtx_unlock(&nodes_lock);
	return (devsw);
}

void
bsdkern_cdev_leave(struct cdev *dev)
{
	struct node *n = __containerof(dev, struct node, dev);

	mtx_lock(&nodes_lock);
	if (--n->inside == 0 && n->gone)
		wakeup(n);
	mtx_unlock(&nodes_lock);
}

/* The open this thread serves: where its per-open data goes. */
static __thread void **serving_priv;
static __thread d_priv_dtor_t **serving_dtr;

void
bsdkern_cdev_serving(void **priv, d_priv_dtor_t **dtr)
{
	serving_priv = priv;
	serving_dtr = dtr;
}

int
devfs_get_cdevpriv(void **datap)
{
	if (serving_priv == NULL || *serving_priv == NULL)
		return (EBADF);
	*datap = *serving_priv;
	return (0);
}

int
devfs_set_cdevpriv(void *priv, d_priv_dtor_t *dtr)
{
	if (serving_priv == NULL)
		return (ENOENT);
	if (*serving_priv != NULL)
		return (EBUSY);
	*serving_priv = priv;
	*serving_dtr = dtr;
	return (0);
}

const char *
devtoname(struct cdev *dev)
{
	return (dev->si_name);
}

/*
 * The calls of a server's file system on its devices (src/devfs.rs): an
 * open is a record of the device, the open's flags and its per-open data,
 * passed back with each call; the calling thread gets a struct thread of
 * its own (bsdkern_attach()) and the device's functions run as they would
 * under devfs.
 */
#include <sys/fcntl.h>
#include <sys/proc.h>
#include <sys/stat.h>

struct bsdkern_open {
	struct cdev	*dev;
	int		 fflags;
	void		*priv;
	d_priv_dtor_t	*dtr;
};

int	bsdkern_cdev_name(int, char *, size_t);
int	bsdkern_cdev_open(const char *, int, struct bsdkern_open **);
int	bsdkern_cdev_ioctl(struct bsdkern_open *, u_long, void *);
void	bsdkern_cdev_close(struct bsdkern_open *);

/* The name of the i-th device, from 0; ENOENT past the last. */
int
bsdkern_cdev_name(int i, char *buf, size_t len)
{
	struct node *n;

	mtx_lock(&nodes_lock);
	TAILQ_FOREACH(n, &nodes, link)
		if (!n->gone && i-- == 0)
			break;
	if (n != NULL)
		strlcpy(buf, n->dev.si_name, len);
	mtx_unlock(&nodes_lock);
	return (n != NULL ? 0 : ENOENT);
}

int
bsdkern_cdev_open(const char *name, int fflags, struct bsdkern_open **op)
{
	struct bsdkern_open *o;
	struct cdevsw *csw;
	struct cdev *dev;
	int error;

	bsdkern_attach();
	dev = bsdkern_cdev_find(name);
	if (dev == NULL)
		return (ENXIO);
	o = malloc(sizeof(*o), M_DEVBUF, M_WAITOK | M_ZERO);
	o->dev = dev;
	o->fflags = fflags;
	if ((csw = bsdkern_cdev_enter(dev)) == NULL) {
		free(o, M_DEVBUF);
		return (ENXIO);
	}
	bsdkern_cdev_serving(&o->priv, &o->dtr);
	error = csw->d_open != NULL ?
	    csw->d_open(dev, fflags, S_IFCHR, curthread) : 0;
	bsdkern_cdev_serving(NULL, NULL);
	bsdkern_cdev_leave(dev);
	if (error != 0) {
		if (o->dtr != NULL)
			o->dtr(o->priv);
		free(o, M_DEVBUF);
		return (error);
	}
	*op = o;
	return (0);
}

int
bsdkern_cdev_ioctl(struct bsdkern_open *o, u_long cmd, void *data)
{
	struct cdevsw *csw;
	int error;

	bsdkern_attach();
	if ((csw = bsdkern_cdev_enter(o->dev)) == NULL)
		return (ENXIO);
	bsdkern_cdev_serving(&o->priv, &o->dtr);
	error = csw->d_ioctl != NULL ?
	    csw->d_ioctl(o->dev, cmd, data, o->fflags, curthread) : ENOTTY;
	bsdkern_cdev_serving(NULL, NULL);
	bsdkern_cdev_leave(o->dev);
	return (error);
}

void
bsdkern_cdev_close(struct bsdkern_open *o)
{
	struct cdevsw *csw;

	bsdkern_attach();
	if ((csw = bsdkern_cdev_enter(o->dev)) != NULL) {
		bsdkern_cdev_serving(&o->priv, &o->dtr);
		if (csw->d_close != NULL)
			csw->d_close(o->dev, o->fflags, S_IFCHR, curthread);
		bsdkern_cdev_serving(NULL, NULL);
		bsdkern_cdev_leave(o->dev);
	}
	if (o->dtr != NULL)
		o->dtr(o->priv);
	free(o, M_DEVBUF);
}

/*
 * Requests whose data holds pointers into the client, which libsys packs
 * with their buffers: a server holding such drivers unpacks them (pcid:
 * PCIOCGETCONF). By default none: -1.
 */
int	bsdkern_cdev_ioctl_packed(struct bsdkern_open *, u_long, void *,
	    size_t, size_t *) __weak_symbol;

int
bsdkern_cdev_ioctl_packed(struct bsdkern_open *o, u_long cmd, void *buf,
    size_t len, size_t *out)
{
	(void)o; (void)cmd; (void)buf; (void)len; (void)out;
	return (-1);
}
