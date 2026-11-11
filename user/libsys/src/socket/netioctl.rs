//! `ioctl()`'s network requests ('i' of the interfaces, 'B' of `bpf`,
//! 'D' of `pf`): each a device command with its structure in the
//! message. A request whose structure points at more memory has that
//! memory carried after it: the message then starts with a table of
//! `Embedded` below, under the command `DCMD_EMBEDDED`, and `io-sock`
//! puts the pointers right. The table is `io-sock`'s contract, as QNX's
//! libsocket speaks it.

use core::ffi::{c_int, c_void};
use core::ptr::null;

use proto::errno::ENOTSUP;
use proto::io::{Devctl, IO_DEVCTL};

use super::request;
use crate::kercall::Iov;

/// The command of a message with a table.
const DCMD_EMBEDDED: i32 = 0xc008_6900_u32 as i32;
/// The most memory one command points at.
const MAX_EMBEDDED: usize = 2;

const SIZE_SHIFT: u32 = 16;
const SIZE_MASK: u32 = 0x1fff;
const IOC_IN: u32 = 0x8000_0000;
const IOC_OUT: u32 = 0x4000_0000;

/// Commands of a structure with a pointer.
const SIOCGIFCONF: u32 = 0xc010_6924;
const SIOCGIFMAC: u32 = 0xc020_6926;
const SIOCSIFMAC: u32 = 0x8020_6927;
const SIOCSIFNAME: u32 = 0x8020_6928;
const SIOCSIFDESCR: u32 = 0x8020_6929;
const SIOCGIFDESCR: u32 = 0xc020_692a;
const SIOCGIFDATA: u32 = 0x8020_692c;
const SIOCGIFMEDIA: u32 = 0xc030_6938;
const SIOCSIFGENERIC: u32 = 0x8020_6939;
const SIOCGIFGENERIC: u32 = 0xc020_693a;
const GREGKEY: u32 = 0xc020_696b;
const GRESKEY: u32 = 0x8020_696c;
const GREGOPTS: u32 = 0xc020_696d;
const GRESOPTS: u32 = 0x8020_696e;
const SIOCIFGCLONERS: u32 = 0xc010_6978;
const SIOCGDRVSPEC: u32 = 0xc028_697b;
const SIOCSDRVSPEC: u32 = 0x8028_697b;
const SIOCIFCREATE2: u32 = 0xc020_697c;
const SIOCGIFGROUP: u32 = 0xc028_6988;
const SIOCGIFGMEMB: u32 = 0xc028_698a;
const SIOCGIFXMEDIA: u32 = 0xc030_698b;
const SIOCSLAGG: u32 = 0x8050_6990;
const SIOCGLAGG: u32 = 0xc050_698f;
const GIFGOPTS: u32 = 0xc020_6996;
const GIFSOPTS: u32 = 0x8020_6997;
const NIOCCTRL: u32 = 0xc058_6997;
const SIOCSIFCAPNV: u32 = 0x8020_699b;
const SIOCGIFCAPNV: u32 = 0xc020_699c;
const IPSECGREQID: u32 = 0xc020_69a0;
const IPSECSREQID: u32 = 0x8020_69a1;
const SIOCS80211: u32 = 0x8020_69ea;
const SIOCG80211: u32 = 0xc020_69eb;
const SIOCG80211STATS: u32 = 0xc020_69ec;
const SIOCSVH: u32 = 0xc020_69f5;
const SIOCGVH: u32 = 0xc020_69f6;
const SIOCSETPFSYNC: u32 = 0x8020_69f7;
const SIOCGETPFSYNC: u32 = 0xc020_69f8;
const SIOCSETPFSYNCNV: u32 = 0x8020_69f9;
const SIOCGETPFSYNCNV: u32 = 0xc020_69fa;
const IOETHERSWITCHGETPORT: u32 = 0xc050_6904;
const BIOCSETF: u32 = 0x8010_4267;
const BIOCGDLTLIST: u32 = 0xc010_4279;
const BIOCSETWF: u32 = 0x8010_427b;
const BIOCSETFNR: u32 = 0x8010_4282;
const DIOCADDRULENV: u32 = 0xc018_4404;
const DIOCGETRULENV: u32 = 0xc018_4407;
const DIOCCLRSTATESNV: u32 = 0xc018_4412;
const DIOCGETSTATENV: u32 = 0xc018_4413;
const DIOCGETSTATUSNV: u32 = 0xc018_4415;
const DIOCGETSTATES: u32 = 0xc010_4419;
const DIOCKILLSTATESNV: u32 = 0xc018_4429;
const DIOCGETQSTATSV0: u32 = 0xc018_4432;
const DIOCGETQSTATSV1: u32 = 0xc020_4432;
const DIOCXBEGIN: u32 = 0xc010_4451;
const DIOCXCOMMIT: u32 = 0xc010_4452;
const DIOCXROLLBACK: u32 = 0xc010_4453;
const DIOCGETSRCNODES: u32 = 0xc010_4454;
const DIOCIGETIFACES: u32 = 0xc028_4457;
const DIOCSETIFFLAG: u32 = 0xc028_4459;
const DIOCCLRIFFLAG: u32 = 0xc028_445a;
const DIOCKEEPCOUNTERS_FREEBSD13: u32 = 0xc018_445c;
const DIOCGETSTATESV2: u32 = 0xc018_445d;
const DIOCGETSYNCOOKIES: u32 = 0xc018_445e;
const DIOCSETSYNCOOKIES: u32 = 0xc018_445f;
const DIOCKEEPCOUNTERS: u32 = 0xc018_4460;
const DIOCADDETHRULE: u32 = 0xc018_4461;
const DIOCGETETHRULE: u32 = 0xc018_4462;
const DIOCGETETHRULES: u32 = 0xc018_4463;
const DIOCGETETHRULESETS: u32 = 0xc018_4464;
const DIOCGETETHRULESET: u32 = 0xc018_4465;
/// `DIOCRCLRTABLES` to `DIOCRSETTFLAGS`, of `struct pfioc_table`.
const DIOCR_FIRST: u32 = 0xc450_443c;
const DIOCR_LAST: u32 = 0xc450_444a;
const DIOCRINADEFINE: u32 = 0xc450_444d;

/// How many bytes each request of `netmap`'s `NIOCCTRL` points at, by
/// its type less one (`NETMAP_REQ_*`).
const NETMAP_BODY: [usize; 16] = [0x38, 0x20, 0x40, 8, 8, 8, 8, 0x10, 0, 0x10, 0x10, 0x40, 8, 0, 0, 0x10];
/// In `struct nmreq_header`: the type, the options and the body.
const NMREQ_TYPE: usize = 2;
const NMREQ_OPTIONS: usize = 0x48;
const NMREQ_BODY: usize = 0x50;
/// A `struct nmreq_option`, which alone can follow; in it, the next one
/// and the bytes it has after it.
const NMREQ_OPTION: usize = 0x18;
const NMOPT_NEXT: usize = 0;
const NMOPT_SIZE: usize = 2 * 8;

/// `struct ifbifconf`, which the bridge's commands of `SIOCGDRVSPEC`
/// point at: the length, the pointer. Those commands are `BRDGGIFS`,
/// `BRDGRTS` and `BRDGGIFSSTP`.
const BRDGGIFS: u64 = 6;
const BRDGRTS: u64 = 7;
const BRDGGIFSSTP: u64 = 0x1b;
const IFBIC_BUF: usize = 8;

/// `struct ifdrv`: its command, length and pointer.
const IFD_CMD: usize = 0x10;
const IFD_LEN: usize = 0x18;
const IFD_DATA: usize = 0x20;
const IFDRV_SIZE: usize = 0x28;

/// A row of the table: `len` bytes for the pointer at `offset`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Embedded {
    len: usize,
    offset: usize,
}

/// The message's table.
#[repr(C)]
#[derive(Default)]
struct Table {
    cmd: i32,
    count: i32,
    rows: [Embedded; MAX_EMBEDDED],
}

/// A command's argument, for reading its fields: the structure the
/// command takes, which `io-sock` defines, and the offsets below are
/// fields of it.
#[derive(Clone, Copy)]
struct Arg(*const u8);

impl Arg {
    /// The field at `offset`.
    fn field<T: Copy>(self, offset: usize) -> T {
        // SAFETY: the type's guarantee.
        unsafe { self.0.wrapping_add(offset).cast::<T>().read_unaligned() }
    }

    /// The field at `offset`, an int, as a length.
    fn int(self, offset: usize) -> usize {
        self.field::<i32>(offset) as isize as usize
    }

    /// The field at `offset`, an unsigned int, as a length.
    fn uint(self, offset: usize) -> usize {
        self.field::<u32>(offset) as usize
    }
}

/// What a command's structure points at.
struct Pointers {
    rows: [Embedded; MAX_EMBEDDED],
    bases: [*const c_void; MAX_EMBEDDED],
    count: usize,
}

impl Pointers {
    /// Adds the pointer at `offset` of `arg`, to `len` bytes, known as
    /// `offset`.
    fn add(&mut self, arg: Arg, offset: usize, len: usize) {
        self.add_base(arg.field(offset), offset, len);
    }

    fn add_base(&mut self, base: *const c_void, offset: usize, len: usize) {
        self.rows[self.count] = Embedded { len, offset };
        self.bases[self.count] = base;
        self.count += 1;
    }
}

/// Whether `name` starts with `prefix` (a loop: libsys links no memcmp).
fn has_prefix(name: &[u8], prefix: &[u8]) -> bool {
    name.len() >= prefix.len() && name.iter().zip(prefix).all(|(a, b)| a == b)
}

/// Whether the name of a clone is a VLAN's: `vlan0`, `em0.5`.
fn vlan_named(name: &[u8]) -> bool {
    has_prefix(name, b"vlan")
        || name
            .iter()
            .position(|&c| c == b'.')
            .is_some_and(|dot| dot < 15 && name.get(dot + 1).is_some_and(u8::is_ascii_digit))
}

/// Fills in what `cmd`'s structure points at: `Err` with an errno.
fn pointers(cmd: u32, arg: Arg, out: &mut Pointers) -> Result<(), c_int> {
    /// The pointer of `struct ifreq`.
    const IFR_DATA: usize = 0x10;
    match cmd {
        DIOCGETQSTATSV0 | DIOCGETQSTATSV1 => out.add(arg, 8, arg.int(0x10)),
        BIOCGDLTLIST => out.add(arg, 8, arg.uint(0) << 2),
        SIOCGIFDATA => out.add(arg, IFR_DATA, 0x98),
        SIOCSIFMAC | SIOCGIFMAC => out.add(arg, 8, arg.field::<usize>(0) + 0x10),
        BIOCSETF | BIOCSETWF | BIOCSETFNR => out.add(arg, 8, arg.uint(0) << 3),
        SIOCSIFNAME => out.add(arg, IFR_DATA, 0x10),
        SIOCSIFDESCR | SIOCGIFDESCR => out.add(arg, 0x18, arg.field(0x10)),
        SIOCSIFCAPNV | SIOCGIFCAPNV => out.add(arg, 0x18, arg.uint(0x10)),
        SIOCS80211 | SIOCG80211 => out.add(arg, 0x18, usize::from(arg.field::<u16>(0x14))),
        SIOCSIFGENERIC | SIOCGIFGENERIC => out.add(arg, IFR_DATA, 0x14),
        GRESKEY | GREGKEY | GRESOPTS | GREGOPTS | GIFSOPTS | GIFGOPTS | IPSECSREQID | IPSECGREQID => {
            out.add(arg, IFR_DATA, 4);
        }
        SIOCSETPFSYNC | SIOCGETPFSYNC => out.add(arg, IFR_DATA, 0x1c),
        SIOCSETPFSYNCNV | SIOCGETPFSYNCNV => out.add(arg, IFR_DATA, 0x18),
        SIOCSVH | SIOCGVH => {
            // The count at the start of what it points at.
            let carp = Arg(arg.field(IFR_DATA));
            out.add(arg, IFR_DATA, carp.int(0) * 0x28);
        }
        SIOCG80211STATS => out.add(arg, IFR_DATA, 0x26c),
        SIOCSDRVSPEC | SIOCGDRVSPEC => {
            out.add(arg, IFD_DATA, arg.field(IFD_LEN));
            if matches!(arg.field::<u64>(IFD_CMD), BRDGGIFS | BRDGRTS | BRDGGIFSSTP) {
                let bif = Arg(arg.field(IFD_DATA));
                let base: *const c_void = bif.field(IFBIC_BUF);
                out.add_base(base, IFDRV_SIZE + IFBIC_BUF, bif.uint(0));
            }
        }
        SIOCGIFGROUP | SIOCGIFGMEMB => out.add(arg, IFD_LEN, arg.uint(0x10)),
        SIOCIFCREATE2 => {
            // The structure starts with the name, a string of 16 bytes.
            let mut n = 0;
            while n < 16 && arg.field::<u8>(n) != 0 {
                n += 1;
            }
            // SAFETY: `n` bytes of the name were read above.
            let name = unsafe { core::slice::from_raw_parts(arg.0, n) };
            let data: *const c_void = arg.field(IFR_DATA);
            if has_prefix(name, b"vxlan") {
                out.add(arg, IFR_DATA, 0x68);
            } else if has_prefix(name, b"lagg") {
                out.add(arg, IFR_DATA, 0x10);
            } else if has_prefix(name, b"wlan") {
                out.add(arg, IFR_DATA, 0x20);
            } else if !data.is_null() && vlan_named(name) {
                out.add(arg, IFR_DATA, 0x14);
            }
        }
        SIOCGIFMEDIA | SIOCGIFXMEDIA => out.add(arg, 0x28, arg.int(0x20) << 2),
        SIOCSLAGG | SIOCGLAGG => out.add(arg, 0x20, arg.field(0x18)),
        SIOCGIFCONF | DIOCGETSTATES | DIOCGETSRCNODES => out.add(arg, 8, arg.int(0)),
        SIOCIFGCLONERS => out.add(arg, 8, arg.int(4) << 4),
        DIOCXBEGIN | DIOCXCOMMIT | DIOCXROLLBACK => {
            out.add(arg, 8, arg.int(0) * arg.int(4));
        }
        DIOCADDRULENV | DIOCGETRULENV | DIOCCLRSTATESNV | DIOCGETSTATENV | DIOCGETSTATUSNV | DIOCKILLSTATESNV
        | DIOCKEEPCOUNTERS_FREEBSD13 | DIOCGETSYNCOOKIES | DIOCSETSYNCOOKIES | DIOCKEEPCOUNTERS | DIOCADDETHRULE
        | DIOCGETETHRULE | DIOCGETETHRULES | DIOCGETETHRULESETS | DIOCGETETHRULESET => {
            out.add(arg, 0, arg.field(0x10));
        }
        DIOCGETSTATESV2 => out.add(arg, 0x10, arg.int(0)),
        DIOCIGETIFACES | DIOCSETIFFLAG | DIOCCLRIFFLAG => {
            out.add(arg, 0x10, arg.int(0x18) * arg.int(0x1c));
        }
        IOETHERSWITCHGETPORT => out.add(arg, 0x48, arg.int(0x40) << 2),
        DIOCR_FIRST..=DIOCR_LAST | DIOCRINADEFINE => {
            out.add(arg, 0x428, arg.int(0x430) * arg.int(0x434));
        }
        NIOCCTRL => {
            let kind = usize::from(arg.field::<u16>(NMREQ_TYPE));
            let body = NETMAP_BODY.get(kind.wrapping_sub(1)).copied().unwrap_or(0);
            if body != 0 {
                out.add(arg, NMREQ_BODY, body);
            }
            let options: *const c_void = arg.field(NMREQ_OPTIONS);
            if !options.is_null() {
                let option = Arg(options.cast());
                if option.field::<u64>(NMOPT_NEXT) != 0 || option.field::<u64>(NMOPT_SIZE) != 0 {
                    return Err(ENOTSUP);
                }
                out.add_base(options, NMREQ_OPTIONS, NMREQ_OPTION);
            }
        }
        _ => {}
    }
    Ok(())
}

/// A network request on `fd`: `data` points at its structure. The
/// server's answer, or QNX's error.
///
/// # Safety
///
/// `data` is the request's structure, and what it points at is valid.
pub unsafe fn ioctl(fd: c_int, cmd: u32, data: *mut c_void) -> Result<c_int, c_int> {
    let size = ((cmd >> SIZE_SHIFT) & SIZE_MASK) as usize;
    let mut found = Pointers {
        rows: [Embedded::default(); MAX_EMBEDDED],
        bases: [null(); MAX_EMBEDDED],
        count: 0,
    };
    pointers(cmd, Arg(data.cast()), &mut found)?;
    let mut msg = Devctl {
        r#type: IO_DEVCTL,
        combine_len: size_of::<Devctl>() as u16,
        dcmd: cmd as i32,
        nbytes: size as u32,
        zero: 0,
    };
    if found.count == 0 {
        let header = Iov::raw((&raw mut msg).cast(), size_of::<Devctl>());
        let sent = if cmd & IOC_IN != 0 { size } else { 0 };
        let back = if cmd & IOC_OUT != 0 { size } else { 0 };
        let data = data.cast_const();
        let s = [header, Iov::raw(data, sent)];
        let r = [header, Iov::raw(data, back)];
        // SAFETY: the structure, of the size the command says.
        unsafe { request(fd, &s, &r) }?;
    } else {
        let count = found.count;
        let mut table = Table {
            cmd: cmd as i32,
            count: count as i32,
            rows: found.rows,
        };
        msg.dcmd = DCMD_EMBEDDED;
        msg.nbytes = 8;
        let mut parts = [Iov::raw(null(), 0); 3 + MAX_EMBEDDED];
        parts[0] = Iov::raw((&raw mut msg).cast(), size_of::<Devctl>());
        parts[1] = Iov::raw((&raw mut table).cast_const().cast(), 8 + count * size_of::<Embedded>());
        parts[2] = Iov::raw(data.cast_const(), size);
        for i in 0..count {
            parts[3 + i] = Iov::raw(found.bases[i], found.rows[i].len);
        }
        let n = 3 + count;
        // SAFETY: the structure and what it points at, as above.
        unsafe { request(fd, &parts[..n], &parts[..n]) }?;
    }
    Ok(msg.dcmd)
}
