//! `diskd`: a disk's server, QNX's shape for FreeBSD's parts. One process
//! holds the driver, the cache (`bufcache`) and the file system
//! (`msdosfs`), and serves a mount point as a resource manager. An open
//! file is a path, looked up again on each operation.

#[cfg(target_os = "freebsd")]
mod server;

#[cfg(target_os = "freebsd")]
pub use server::run;

/// Off OsCraft there is nothing to serve; the library's parts are tested
/// on the host.
#[cfg(not(target_os = "freebsd"))]
pub fn run(_device: Box<dyn bufcache::BlockDevice + Send>, _name: &str, _mount: Option<&str>) {}
