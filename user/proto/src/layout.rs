//! The layout table: for each type and number of the crate, the C
//! expression that QNX's headers give it and the crate's value. The
//! host test below prints it; `tools/check-proto.py` compiles the C side
//! against the headers and compares. The error numbers it checks from
//! [`errno`](crate::qnx_errno)'s source.

use core::mem::{offset_of, size_of};

use crate::{connect, dcmd, event, info, io, ioctl, mem, proc, rsrc, stat, sys};

/// A struct's size, and its fields' offsets.
macro_rules! layout {
    ($t:ty, $c:literal $(, $f:ident = $cf:literal)* $(,)?) => {
        &[
            (concat!("sizeof(", $c, ")"), size_of::<$t>() as u64),
            $((concat!("offsetof(", $c, ", ", $cf, ")"), offset_of!($t, $f) as u64),)*
        ]
    };
}

/// Fields' offsets only, for a type that is only the start of C's.
macro_rules! fields {
    ($t:ty, $c:literal $(, $f:ident = $cf:literal)* $(,)?) => {
        &[$((concat!("offsetof(", $c, ", ", $cf, ")"), offset_of!($t, $f) as u64),)*]
    };
}

/// Numbers, by their C names.
macro_rules! values {
    ($($v:expr => $c:literal),* $(,)?) => {
        &[$(($c, $v as u64),)*]
    };
}

/// The table, in parts.
pub const TABLE: &[&[(&str, u64)]] = &[
    layout!(
        event::Sigevent,
        "struct sigevent",
        signo = "sigev_signo",
        value = "sigev_value",
        code = "sigev_code",
        priority = "sigev_priority",
    ),
    layout!(
        event::Pulse,
        "struct _pulse",
        code = "code",
        value = "value",
        scoid = "scoid"
    ),
    layout!(
        info::MsgInfo,
        "struct _msg_info",
        scoid = "scoid",
        coid = "coid",
        priority = "priority",
        msglen = "msglen",
        srcmsglen = "srcmsglen",
        dstmsglen = "dstmsglen",
        type_id = "type_id",
        reserved = "reserved",
    ),
    layout!(
        info::MsgInfo32,
        "struct _msg_info32",
        priority = "priority",
        type_id = "type_id"
    ),
    layout!(info::MsgInfoHdr, "struct _msg_info_hdr", coid = "coid"),
    layout!(
        info::ClientInfo,
        "struct _client_info",
        flags = "flags",
        ruid = "cred.ruid",
        ngroups = "cred.ngroups"
    ),
    layout!(
        info::Fdinfo,
        "struct _fdinfo",
        offset = "offset",
        sflag = "sflag",
        rlocks = "rlocks",
        filehandle = "filehandle",
        inodegen = "inodegen",
    ),
    layout!(
        io::Notify,
        "struct _io_notify",
        action = "action",
        flags = "flags",
        event = "event",
        mgr = "mgr",
        flags_extra_mask = "flags_extra_mask",
        nfds = "nfds",
        fd_first = "fd_first",
        nfds_ready = "nfds_ready",
        timo = "timo",
    ),
    layout!(
        io::Notify,
        "struct _io_notify_reply",
        action = "flags",
        flags = "flags2",
        timo = "timo"
    ),
    layout!(
        io::ReadWrite,
        "struct _io_read64",
        nbytes = "nbytes",
        xtype = "xtype",
        nbytes_hi = "nbytes_hi"
    ),
    layout!(
        io::ReadWrite,
        "struct _io_write",
        nbytes = "nbytes",
        xtype = "xtype"
    ),
    layout!(io::Stat, "struct _io_stat", format = "format"),
    layout!(
        io::Devctl,
        "struct _io_devctl",
        dcmd = "dcmd",
        nbytes = "nbytes"
    ),
    layout!(
        io::DevctlReply,
        "struct _io_devctl_reply",
        ret_val = "ret_val",
        nbytes = "nbytes"
    ),
    layout!(io::Pathconf, "struct _io_pathconf", name = "name"),
    layout!(
        io::Acl,
        "struct _io_acl",
        subtype = "subtype",
        zero = "zero"
    ),
    layout!(io::AclReply, "struct _io_acl_reply"),
    layout!(
        io::AclHeader,
        "struct _acl_header",
        size = "size",
        r#type = "type",
        nents = "nents"
    ),
    layout!(
        io::AclEntry,
        "struct _acl_posix_ent",
        tag_permset = "tag_permset",
        id = "id"
    ),
    layout!(
        io::Lseek,
        "struct _io_lseek",
        whence = "whence",
        flags = "flags",
        offset = "offset"
    ),
    layout!(io::Chmod, "struct _io_chmod", mode = "mode"),
    layout!(io::Chown, "struct _io_chown", gid = "gid", uid = "uid"),
    layout!(
        io::Utime,
        "struct _io_utime",
        cur_flag = "cur_flag",
        actime = "times._actime",
        modtime = "times._modtime"
    ),
    layout!(
        io::Utime64,
        "struct _io_utime64",
        atime_s = "atime_s",
        atime_ns = "atime_ns",
        mtime_s = "mtime_s",
        mtime_ns = "mtime_ns"
    ),
    layout!(
        io::Openfd,
        "struct _io_openfd",
        ioflag = "ioflag",
        sflag = "sflag",
        xtype = "xtype",
        info = "info",
        handle = "handle"
    ),
    layout!(
        io::FdinfoReq,
        "struct _io_fdinfo",
        flags = "flags",
        path_len = "path_len"
    ),
    layout!(io::FdinfoReply, "struct _io_fdinfo_reply", info = "info"),
    layout!(
        io::Lock,
        "struct _io_lock",
        subtype = "subtype",
        nbytes = "nbytes"
    ),
    layout!(io::LockReply, "struct _io_lock_reply"),
    layout!(
        io::Flock,
        "flock_t",
        l_whence = "l_whence",
        l_start = "l_start",
        l_len = "l_len",
        l_pid = "l_pid",
        l_sysid = "l_sysid"
    ),
    layout!(
        io::Space,
        "struct _io_space",
        subtype = "subtype",
        whence = "whence",
        start = "start",
        len = "len"
    ),
    layout!(
        io::Mmap,
        "struct _io_mmap",
        prot = "prot",
        offset = "offset",
        info = "info",
        required_prot = "required_prot",
        requested_len = "requested_len"
    ),
    layout!(
        io::MmapReply,
        "struct _io_mmap_reply",
        allowed_prot = "allowed_prot",
        offset = "offset",
        coid = "coid",
        fd = "fd"
    ),
    layout!(io::Dup, "struct _io_dup", info = "info", handle = "handle"),
    layout!(
        io::Msg,
        "struct _io_msg",
        mgrid = "mgrid",
        subtype = "subtype"
    ),
    layout!(io::Sync, "struct _io_sync", flag = "flag"),
    layout!(
        connect::LinkReply,
        "struct _io_connect_link_reply",
        file_type = "file_type",
        chroot_len = "chroot_len",
        umask = "umask",
        nentries = "nentries",
        path_len = "path_len"
    ),
    layout!(
        connect::Entry,
        "struct _io_connect_entry",
        file_type = "file_type",
        prefix_len = "prefix_len",
        zero = "zero"
    ),
    layout!(
        connect::FtypeReply,
        "struct _io_connect_ftype_reply",
        file_type = "file_type"
    ),
    values!(size_of::<connect::Connect>() => "offsetof(struct _io_connect, path)"),
    fields!(
        connect::Connect,
        "struct _io_connect",
        file_type = "file_type",
        reply_max = "reply_max",
        handle = "handle",
        ioflag = "ioflag",
        mode = "mode",
        sflag = "sflag",
        access = "access",
        dirfd = "dirfd",
        path_len = "path_len",
        eflag = "eflag",
        extra_type = "extra_type",
        extra_len = "extra_len",
    ),
    layout!(
        proc::GetSetId,
        "struct _proc_getsetid",
        pid = "pid",
        eid = "eid",
        rid = "rid",
        ngroups = "ngroups"
    ),
    fields!(
        proc::GetSetIdReply,
        "struct _proc_getsetid_reply",
        pgrp = "pgrp",
        orig_ppid = "orig_ppid"
    ),
    values!(proc::GETSETID_REPLY_SIZE => "sizeof(struct _proc_getsetid_reply)"),
    layout!(
        proc::GetLimit,
        "struct _proc_resource_getlimit",
        pid = "pid",
        count = "count",
        resource = "resource"
    ),
    layout!(
        proc::SetLimit,
        "struct _proc_resource_setlimit",
        resource = "entry[0].resource",
        limit = "entry[0].limit"
    ),
    layout!(proc::Rlimit, "struct rlimit", max = "rlim_max"),
    fields!(
        proc::SetAbilities,
        "struct _proc_resource_setabilities",
        pid = "pid",
        count = "count"
    ),
    values!(size_of::<proc::SetAbilities>() => "offsetof(struct _proc_resource_setabilities, entry)"),
    layout!(
        proc::LookupAbility,
        "struct _proc_resource_lookupability",
        pid = "pid",
        flags = "flags",
        namelen = "namelen"
    ),
    layout!(
        proc::Event,
        "struct _proc_event",
        flags = "flags",
        event = "event"
    ),
    layout!(proc::EventDel, "struct _proc_event_del", id = "id"),
    layout!(
        rsrc::RsrcCmd,
        "struct _rsrc_cmd",
        pid = "pid",
        nbytes = "nbytes",
        count = "count",
        index = "index"
    ),
    layout!(
        rsrc::MinorRequest,
        "msg_rsrc_minor_request_t",
        minor = "minor",
        flags = "flags"
    ),
    values!(
        rsrc::RSRC_CMD => "RSRCDBMGR_RSRC_CMD",
        rsrc::REQ_CREATE => "RSRCDBMGR_REQ_CREATE",
        rsrc::REQ_DESTROY => "RSRCDBMGR_REQ_DESTROY",
        rsrc::REQ_ATTACH => "RSRCDBMGR_REQ_ATTACH",
        rsrc::REQ_DETACH => "RSRCDBMGR_REQ_DETACH",
        rsrc::REQ_QUERY => "RSRCDBMGR_REQ_QUERY",
        rsrc::REQ_RESERVED => "RSRCDBMGR_RESERVED",
        rsrc::REQ_QUERY_NAME => "RSRCDBMGR_REQ_QUERY_NAME",
        rsrc::SYSPAGE_HWINFO => "offsetof(struct syspage_entry, hwinfo)",
        rsrc::SYSPAGE_STRINGS => "offsetof(struct syspage_entry, strings)",
        rsrc::SYSPAGE_ASINFO => "offsetof(struct syspage_entry, asinfo)",
        rsrc::ASINFO_SIZE => "sizeof(struct asinfo_entry)",
        rsrc::ASINFO_NAME => "offsetof(struct asinfo_entry, name)",
    ),
    layout!(
        dcmd::MsgStatvfs,
        "struct __msg_statvfs",
        blocks = "f_blocks",
        favail = "f_favail",
        fsid = "f_fsid",
        basetype = "f_basetype",
        flag = "f_flag",
        namemax = "f_namemax",
        filler = "f_filler"
    ),
    layout!(
        dcmd::Statvfs,
        "struct statvfs",
        frsize = "f_frsize",
        blocks = "f_blocks",
        favail = "f_favail",
        fsid = "f_fsid",
        basetype = "f_basetype",
        flag = "f_flag",
        namemax = "f_namemax",
        filler = "f_filler"
    ),
    layout!(
        proc::AbilityName,
        "struct _proc_resource_ability_name",
        able = "able"
    ),
    layout!(
        proc::Session,
        "struct _proc_session",
        xid = "xid",
        sid = "sid",
        id = "id",
        event = "event"
    ),
    layout!(proc::SetPgid, "struct _proc_setpgid", pgid = "pgid"),
    layout!(
        proc::Umask,
        "struct _proc_umask",
        umask = "umask",
        pid = "pid"
    ),
    layout!(
        proc::UmaskReply,
        "struct _proc_umask_reply",
        umask = "umask"
    ),
    layout!(
        proc::Wait,
        "struct _proc_wait",
        idtype = "idtype",
        options = "options",
        id = "id"
    ),
    layout!(
        proc::Fork,
        "struct _proc_fork",
        flags = "flags",
        frame = "frame"
    ),
    layout!(
        proc::Daemon,
        "struct _proc_daemon",
        status = "status",
        flags = "flags"
    ),
    layout!(proc::GetPid64, "struct _proc_getpid64", pid = "pid"),
    layout!(
        proc::SpawnHeader,
        "struct _proc_posix_spawn",
        pathlen = "pathlen",
        searchlen = "searchlen",
        filact_bytes = "filact_bytes",
        attr_bytes = "attr_bytes"
    ),
    layout!(
        mem::Map,
        "struct _mem_map",
        addr = "addr",
        prot = "prot",
        fd = "fd",
        align = "align",
        offset = "offset",
        shmem_handle = "shmem_handle",
        elf_pvaddr = "elf_pvaddr"
    ),
    layout!(
        mem::MapReply,
        "struct _mem_map_reply",
        real_addr = "real_addr",
        addr = "addr"
    ),
    layout!(
        mem::Ctrl,
        "struct _mem_ctrl",
        flags = "flags",
        addr = "addr",
        len = "len"
    ),
    layout!(mem::Info, "struct _mem_info", fd = "fd", flags = "flags"),
    layout!(
        mem::Offset,
        "struct _mem_offset",
        addr = "addr",
        len = "len"
    ),
    layout!(
        mem::OffsetReply,
        "struct _mem_offset_reply",
        offset = "offset",
        fd = "fd"
    ),
    layout!(mem::Peer, "struct _mem_peer", pid = "pid"),
    layout!(
        mem::ShmemHandle,
        "struct _mem_shmem_handle",
        options = "options",
        handle = "handle",
        pid = "pid",
        perm = "perm",
        sub_offset = "sub_offset",
        sub_length = "sub_length"
    ),
    layout!(mem::ShmemRevoke, "struct _mem_shmem_revoke", pid = "pid"),
    layout!(
        sys::Conf,
        "struct _sys_conf",
        cmd = "cmd",
        name = "name",
        value = "value"
    ),
    layout!(
        sys::ConfReply,
        "struct _sys_conf_reply",
        r#match = "match",
        value = "value"
    ),
    layout!(
        ioctl::Sgttyb,
        "struct sgttyb",
        sg_kill = "sg_kill",
        sg_flags = "sg_flags"
    ),
    layout!(
        ioctl::Termio,
        "struct termio",
        c_lflag = "c_lflag",
        c_line = "c_line",
        c_cc = "c_cc"
    ),
    values!(
        size_of::<ioctl::Tchars>() => "sizeof(struct tchars)",
        size_of::<ioctl::Ltchars>() => "sizeof(struct ltchars)",
    ),
    layout!(
        dcmd::QnxTermios,
        "struct termios",
        c_lflag = "c_lflag",
        c_cc = "c_cc",
        c_ispeed = "c_ispeed",
        c_ospeed = "c_ospeed"
    ),
    layout!(
        dcmd::Winsize,
        "struct winsize",
        ws_col = "ws_col",
        ws_ypixel = "ws_ypixel"
    ),
    layout!(
        dcmd::GetPtrEmbed,
        "struct __ioctl_getptrembed",
        niov = "niov"
    ),
    values!(
        io::IO_CONNECT => "_IO_CONNECT",
        io::IO_READ => "_IO_READ",
        io::IO_STAT => "_IO_STAT",
        io::IO_NOTIFY => "_IO_NOTIFY",
        io::IO_DEVCTL => "_IO_DEVCTL",
        io::IO_PATHCONF => "_IO_PATHCONF",
        io::IO_LSEEK => "_IO_LSEEK",
        io::IO_CHOWN => "_IO_CHOWN",
        io::IO_UTIME => "_IO_UTIME",
        io::IO_OPENFD => "_IO_OPENFD",
        io::IO_FDINFO => "_IO_FDINFO",
        io::IO_LOCK => "_IO_LOCK",
        io::IO_SPACE => "_IO_SPACE",
        io::IO_SHUTDOWN => "_IO_SHUTDOWN",
        io::IO_MMAP => "_IO_MMAP",
        io::IO_MSG => "_IO_MSG",
        io::IO_DUP => "_IO_DUP",
        io::IO_CLOSE => "_IO_CLOSE",
        io::IO_SYNC => "_IO_SYNC",
        io::IO_POWER => "_IO_POWER",
        io::IO_ACL => "_IO_ACL",
        io::IO_ACL_GET => "_IO_ACL_GET",
        io::IO_ACL_SET => "_IO_ACL_SET",
        io::IO_ACL_CHK => "_IO_ACL_CHK",
        io::IO_READ64 => "_IO_READ64",
        io::IO_WRITE64 => "_IO_WRITE64",
        io::IO_UTIME64 => "_IO_UTIME64",
        io::IO_MAX => "_IO_MAX",
        io::IO_COMBINE_FLAG => "_IO_COMBINE_FLAG",
        io::IO_XTYPE_READCOND => "_IO_XTYPE_READCOND",
        io::IO_XTYPE_OFFSET => "_IO_XTYPE_OFFSET",
        io::IO_FLAG_WR => "_IO_FLAG_WR",
        io::NOTIFY_ACTION_POLLARM => "_NOTIFY_ACTION_POLLARM",
        io::NOTIFY_ACTION_EDGEARM => "_NOTIFY_ACTION_EDGEARM",
        io::NOTIFY_COND_EXTEN => "_NOTIFY_COND_EXTEN",
        io::IO_OPENFD_PIPE => "_IO_OPENFD_PIPE",
        io::F_SETLK => "F_SETLK",
        io::F_SETLKW => "F_SETLKW",
        io::F_GETLK => "F_GETLK",
        io::F_RDLCK => "F_RDLCK",
        io::F_UNLCK => "F_UNLCK",
        io::F_EXLCK => "F_EXLCK",
        io::F_FLOCK => "F_FLOCK",
        connect::CONNECT_LINK => "_IO_CONNECT_LINK",
        connect::CONNECT_MOUNT => "_IO_CONNECT_MOUNT",
        connect::EXTRA_SEM => "_IO_CONNECT_EXTRA_SEM",
        connect::EXTRA_RESMGR_LINK => "_IO_CONNECT_EXTRA_RESMGR_LINK",
        connect::EXTRA_TYMEM => "_IO_CONNECT_EXTRA_TYMEM",
        connect::EXTRA_LINK => "_IO_CONNECT_EXTRA_LINK",
        connect::EFLAG_PATH_NEEDED => "_IO_CONNECT_EFLAG_PATH_NEEDED",
        connect::RET_FTYPE => "_IO_CONNECT_RET_FTYPE",
        connect::RET_CHROOT => "_IO_CONNECT_RET_CHROOT",
        connect::RET_MSG => "_IO_CONNECT_RET_MSG",
        connect::RET_FLAG => "_IO_CONNECT_RET_FLAG",
        connect::FTYPE_MATCHED => "_FTYPE_MATCHED",
        connect::FTYPE_ALL => "_FTYPE_ALL",
        connect::FTYPE_SHMEM => "_FTYPE_SHMEM",
        connect::FTYPE_MQUEUE => "_FTYPE_MQUEUE",
        connect::FTYPE_SEM => "_FTYPE_SEM",
        connect::FTYPE_MOUNT => "_FTYPE_MOUNT",
        connect::FTYPE_NAME => "_FTYPE_NAME",
        connect::FTYPE_TYMEM => "_FTYPE_TYMEM",
        proc::PROC_SPAWN => "_PROC_SPAWN",
        proc::PROC_WAIT => "_PROC_WAIT",
        proc::PROC_FORK => "_PROC_FORK",
        proc::PROC_GETSETID => "_PROC_GETSETID",
        proc::PROC_SETPGID => "_PROC_SETPGID",
        proc::PROC_UMASK => "_PROC_UMASK",
        proc::PROC_SESSION => "_PROC_SESSION",
        proc::PROC_DAEMON => "_PROC_DAEMON",
        proc::PROC_EVENT => "_PROC_EVENT",
        proc::PROC_RESOURCE => "_PROC_RESOURCE",
        proc::PROC_POSIX_SPAWN => "_PROC_POSIX_SPAWN",
        proc::PROC_POSIX_SPAWNP => "_PROC_POSIX_SPAWNP",
        proc::PROC_GETPID64 => "_PROC_GETPID64",
        proc::ID_SETREGID => "_PROC_ID_SETREGID",
        proc::ID_SETGROUPS => "_PROC_ID_SETGROUPS",
        proc::ID_GETID_NO_CRED => "_PROC_ID_GETID_NO_CRED",
        proc::RESOURCE_SETLIMIT => "_PROC_RESOURCE_SETLIMIT",
        proc::RESOURCE_SETABILITIES => "_PROC_RESOURCE_SETABILITIES",
        proc::EVENT_NOTIFY => "_PROC_EVENT_NOTIFY",
        proc::EVENT_TRIGGER => "_PROC_EVENT_TRIGGER",
        proc::EVENT_NOTIFY_ADD => "_PROC_EVENT_NOTIFY_ADD",
        proc::EVENT_NOTIFY_DEL => "_PROC_EVENT_NOTIFY_DEL",
        proc::EVENT_TRIGGER_UPDATEABLE => "_PROC_EVENT_TRIGGER_UPDATEABLE",
        dcmd::FSYS_STATVFS => "DCMD_FSYS_STATVFS",
        proc::RESOURCE_LOOKUPABILITY => "_PROC_RESOURCE_LOOKUPABILITY",
        proc::RESOURCE_CREATEABILITY => "_PROC_RESOURCE_CREATEABILITY",
        proc::RESOURCE_ABILITY_NAME => "_PROC_RESOURCE_ABILITY_NAME",
        proc::UMASK_GET => "_PROC_UMASK_GET",
        proc::SPAWN_START => "_PROC_SPAWN_START",
        mem::MEM_MAP => "_MEM_MAP",
        mem::MEM_INFO => "_MEM_INFO",
        mem::MEM_OFFSET => "_MEM_OFFSET",
        mem::MEM_PEER => "_MEM_PEER",
        mem::MEM_SHMEM_HANDLE => "_MEM_SHMEM_HANDLE",
        mem::MEM_SHMEM_REVOKE => "_MEM_SHMEM_REVOKE",
        mem::CTRL_ADVISE => "_MEM_CTRL_ADVISE",
        mem::OFFSET_FD => "_MEM_OFFSET_FD",
        mem::HANDLE_DELETE => "_MEM_SHMEM_DELETE_HANDLE",
        mem::HANDLE_OPT_SUBRANGE => "SHM_CREATE_HANDLE_OPT_SUBRANGE",
        sys::SYS_CONF => "_SYS_CONF",
        connect::PATH_CHDIR => "_PATH_CHDIR",
        connect::PATH_CHROOT => "_PATH_CHROOT",
        connect::PATH_OPENDIR => "_PATH_OPENDIR",
        stat::FORM_PREFERRED => "_STAT_FORM_PREFERRED",
        stat::FORM_T32_2008 => "_STAT_FORM_T32_2008",
        stat::S_IFNAM => "S_IFNAM",
        stat::S_IFLNK => "S_IFLNK",
        stat::S_IFSOCK => "S_IFSOCK",
        sys::SUB_SET => "_SYS_SUB_SET",
        sys::CONF_STR => "_CONF_STR",
        sys::CONF_NUM => "_CONF_NUM",
        event::SIGEV_SIGNAL_THREAD => "SIGEV_SIGNAL_THREAD",
        event::SIGEV_PULSE => "SIGEV_PULSE",
        event::SIGEV_SEM => "SIGEV_SEM",
        event::SIGEV_FLAG_UPDATEABLE => "SIGEV_FLAG_UPDATEABLE",
        event::SIGEV_FLAG_HANDLE => "SIGEV_FLAG_HANDLE",
        event::SIGEV_FLAG_CODE_UPDATEABLE => "SIGEV_FLAG_CODE_UPDATEABLE",
        event::SI_NOTIFY as i64 => "SI_NOTIFY",
        event::PULSE_CODE_DISCONNECT as i64 => "_PULSE_CODE_DISCONNECT",
        info::MI_UNBLOCK_REQ => "_NTO_MI_UNBLOCK_REQ",
        crate::SYSMGR_COID => "SYSMGR_COID",
        dcmd::ALL_GETFLAGS => "DCMD_ALL_GETFLAGS",
        dcmd::ALL_SETFLAGS => "DCMD_ALL_SETFLAGS",
        dcmd::ALL_GETOWN => "DCMD_ALL_GETOWN",
        dcmd::ALL_SETOWN => "DCMD_ALL_SETOWN",
        dcmd::ALL_GETMOUNTFLAGS => "DCMD_ALL_GETMOUNTFLAGS",
        dcmd::ALL_FADVISE => "DCMD_ALL_FADVISE",
        dcmd::CHR_ISATTY => "DCMD_CHR_ISATTY",
        dcmd::CHR_GETSID => "DCMD_CHR_TCGETSID",
        dcmd::CHR_TTYINFO => "DCMD_CHR_TTYINFO",
        dcmd::CHR_SERCTL => "DCMD_CHR_SERCTL",
        dcmd::CHR_TCINJECTC => "DCMD_CHR_TCINJECTC",
        dcmd::CHR_TCINJECTR => "DCMD_CHR_TCINJECTR",
        dcmd::CHR_GRANTPT => "DCMD_CHR_GRANTPT",
        dcmd::TIOCGETA => "TIOCGETA",
        dcmd::TIOCSETAF => "TIOCSETAF",
        dcmd::TIOCFLUSH => "TIOCFLUSH",
        dcmd::TIOCDRAIN => "TIOCDRAIN",
        dcmd::TIOCGPGRP => "TIOCGPGRP",
        dcmd::TIOCSPGRP => "TIOCSPGRP",
        dcmd::TIOCGWINSZ => "TIOCGWINSZ",
        dcmd::TIOCSWINSZ => "TIOCSWINSZ",
        dcmd::TIOCMGET => "TIOCMGET",
        dcmd::TCXONC => "TCXONC",
        dcmd::FIONREAD => "FIONREAD",
        dcmd::MISC_GETPTREMBED => "DCMD_MISC_GETPTREMBED",
    ),
];

#[cfg(test)]
mod tests {
    extern crate std;

    /// Prints the table for `tools/check-proto.py`, one `expression =
    /// value` per line between markers.
    #[test]
    fn layout() {
        std::println!("BEGIN LAYOUT");
        for part in super::TABLE {
            for (c, v) in *part {
                std::println!("{c} = {v}");
            }
        }
        std::println!("END LAYOUT");
    }
}
