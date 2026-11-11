//! Abilities (`procmgr_ability()`, `<sys/procmgr.h>`): what a process may
//! do, separately when its effective user ID is 0 (the root domain) and
//! when it isn't (the non-root domain).
//!
//! As procnto (checked with abiltest in the reference image):
//! - By default the root domain has every ability and the non-root
//!   domain those `<sys/procmgr.h>` marks `ABLE_ALLOW_NONROOT` (spawn,
//!   fork, mapping code, process groups, ...).
//! - An entry of `procmgr_ability()` is an ability ID with operations
//!   (`PROCMGR_AOP_DENY`, `_ALLOW`, `_SUBRANGE`, `_LOCK`, `_INHERIT_YES`,
//!   `_INHERIT_NO`) and domains (`PROCMGR_ADN_ROOT`, `_NONROOT`); a
//!   subrange's bounds (two `uint64_t`) follow it. `PROCMGR_AID_EOL` ends
//!   the list, and with operations applies them to every ability.
//! - Allowing an ability that the non-root domain doesn't have by default
//!   (a privileged one), or giving it a subrange, takes
//!   `PROCMGR_AID_ABLE_PRIV`; another process's abilities,
//!   `PROCMGR_AID_XPROCESS_ABLE` (EPERM). A locked ability is left as it
//!   is, without an error.
//! - A subrange restricts an allowed ability to the values in it (e.g. the
//!   user IDs of `SETUID`); subranges add up and are never removed.
//! - `fork()` copies the abilities; `spawn()` and `exec()` keep only those
//!   changed with `PROCMGR_AOP_INHERIT_YES` and start the others afresh.
//! - Custom abilities (`procmgr_ability_create()`, see `proc::procmgr`)
//!   are by default the domains' their creation named; an ID with
//!   `PROCMGR_AID_UNCREATED` (libc's temporary one) is EINVAL.
//! - A server checks its client's abilities with `ConnectClientInfoAble()`
//!   (`allowed_range`).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::errno::{EINVAL, EPERM};
use super::process::{self, Process};

pub const SPAWN_SETUID: u32 = 0;
pub const SPAWN_SETGID: u32 = 1;
pub const SETUID: u32 = 2;
pub const SETGID: u32 = 3;
pub const PATHSPACE: u32 = 5;
pub const RSRCDBMGR: u32 = 10;
pub const SESSION: u32 = 11;
pub const EVENT: u32 = 13;
pub const REBOOT: u32 = 6;
pub const CONFSET: u32 = 9;
pub const RLIMIT: u32 = 14;
pub const MEM_PEER: u32 = 19;
pub const MEM_LOCK: u32 = 20;
pub const SPAWN: u32 = 21;
pub const FORK: u32 = 22;
pub const SIGNAL: u32 = 36;
pub const PGRP: u32 = 38;
pub const MAP_FIXED: u32 = 39;
pub const CHILD_NEWAPP: u32 = 43;
pub const CONNECTION: u32 = 34;
pub const XPROCESS_QUERY: u32 = 50;
pub const PUBLIC_CHANNEL: u32 = 44;
pub const DEFAULT_TIMER_TOLERANCE: u32 = 48;
pub const CHROOT: u32 = 51;
pub const PROT_WRITE_AND_EXEC: u32 = 53;
pub const SRANDOM: u32 = 54;
pub const SIGEV_THREAD: u32 = 69;
pub const HIGH_RESOLUTION_TIMER: u32 = 66;
pub const MEM_PHYS: u32 = 16;
pub const MEM_SPECIAL: u32 = 17;
pub const CLOCKSET: u32 = 27;
pub const INTERRUPT: u32 = 29;
pub const IO: u32 = 31;
pub const TRACE: u32 = 32;
pub const PRIORITY: u32 = 33;
pub const SCHEDULE: u32 = 35;
pub const RUNSTATE: u32 = 8;
pub const CPUMODE: u32 = 7;
pub const UMASK: u32 = 12;
pub const MSG_QUEUE: u32 = 26;
pub const ABLE_CREATE: u32 = 46;
pub const SERVER_MONITOR: u32 = 68;
const ABLE_PRIV: u32 = 61;
const XPROCESS_ABLE: u32 = 62;
/// `PROCMGR_AID_MAX`.
const MAX: u32 = 75;
/// IDs of the abilities `<sys/procmgr.h>` doesn't define (gaps in the list).
const UNDEFINED: [u32; 5] = [25, 30, 49, 55, 63];
/// The abilities the non-root domain has by default (`ABLE_ALLOW_NONROOT`):
/// SPAWN, FORK, PROT_EXEC, PGRP, MAP_FIXED, RCONSTRAINT, PUBLIC_CHANNEL,
/// RUNSTATE_BURST, PROT_WRITE_AND_EXEC, UNTRUSTED_EXEC, SIGEV_THREAD.
const NONROOT_DEFAULT: [u32; 11] = [21, 22, 23, 38, 39, 42, 44, 47, 53, 67, 69];

const EOL: u32 = 0xffff;
const AOP_DENY: u32 = 0x0001_0000;
const AOP_ALLOW: u32 = 0x0002_0000;
const AOP_SUBRANGE: u32 = 0x0004_0000;
const AOP_LOCK: u32 = 0x0008_0000;
const AID_UNCREATED: u32 = 0x0010_0000;
const AOP_INHERIT_YES: u32 = 0x0040_0000;
const AOP_INHERIT_NO: u32 = 0x0080_0000;
const AOP_MASK: u32 = AOP_DENY | AOP_ALLOW | AOP_SUBRANGE | AOP_LOCK | AOP_INHERIT_YES | AOP_INHERIT_NO;
const ADN_ROOT: u32 = 0x1000_0000;
const ADN_NONROOT: u32 = 0x2000_0000;

/// One ability in one domain, once changed from its default.
#[derive(Debug, Clone)]
struct Domain {
    allowed: bool,
    locked: bool,
    inherit: bool,
    /// Allowed values, (low, high) inclusive; none: all.
    ranges: Vec<(u64, u64)>,
}

/// A process's abilities: those changed from the defaults, by ID, for the
/// root and the non-root domain.
#[derive(Debug, Clone)]
pub struct Abilities {
    changed: BTreeMap<u32, [Domain; 2]>,
}

/// The default of ability `aid` in domain `dom` (0 root, 1 non-root).
fn default_allowed(aid: u32, dom: usize) -> bool {
    if aid > MAX {
        return crate::proc::procmgr::custom_default(aid, dom).unwrap_or(false);
    }
    // No process has an ID `<sys/procmgr.h>` leaves "Available", root
    // included (procnto, measured with `ConnectClientInfoAble()`; review
    // IPC-18).
    if UNDEFINED.contains(&aid) {
        return false;
    }
    match dom {
        0 => true,
        _ => NONROOT_DEFAULT.contains(&aid),
    }
}

/// Whether `aid` is a privileged ability (not the non-root domain's by
/// default): allowing it takes `ABLE_PRIV`.
fn privileged(aid: u32) -> bool {
    !default_allowed(aid, 1)
}

impl Abilities {
    pub const fn new() -> Self {
        Self {
            changed: BTreeMap::new(),
        }
    }

    fn entry(&mut self, aid: u32) -> &mut [Domain; 2] {
        self.changed.entry(aid).or_insert_with(|| {
            [0, 1].map(|dom| Domain {
                allowed: default_allowed(aid, dom),
                locked: false,
                inherit: false,
                ranges: Vec::new(),
            })
        })
    }

    fn allows(&self, aid: u32, dom: usize, value: Option<u64>) -> bool {
        self.allows_range(aid, dom, value.map(|v| (v, v)))
    }

    // ANCHOR: allows_range
    /// Does domain `dom` have ability `aid`, for all the values `range`
    /// holds (none: any value)? With subranges, one must hold them all.
    fn allows_range(&self, aid: u32, dom: usize, range: Option<(u64, u64)>) -> bool {
        match self.changed.get(&aid) {
            None => default_allowed(aid, dom),
            Some(d) => {
                let d = &d[dom];
                d.allowed
                    && (d.ranges.is_empty()
                        || range.is_none_or(|(v, w)| d.ranges.iter().any(|&(lo, hi)| lo <= v && w <= hi)))
            }
        }
    }
    // ANCHOR_END: allows_range

    /// What a spawned or exec()ed process keeps: the domains changed with
    /// `PROCMGR_AOP_INHERIT_YES`.
    pub fn inherited(&self) -> Self {
        let mut new = Self::new();
        for (&aid, doms) in &self.changed {
            for (dom, d) in doms.iter().enumerate() {
                if d.inherit {
                    new.entry(aid)[dom] = d.clone();
                }
            }
        }
        new
    }
}

/// The domain process `p` is in now.
fn domain(p: &Process) -> usize {
    if p.cred.lock().euid == 0 { 0 } else { 1 }
}

/// May process `p`, with effective user ID `euid`, use ability `aid`
/// (with `value`)? For callers that hold `p`'s credentials already.
pub fn allowed_as(p: &Process, euid: u32, aid: u32, value: Option<u64>) -> bool {
    p.abilities.lock().allows(aid, if euid == 0 { 0 } else { 1 }, value)
}

/// May process `p` use ability `aid` (with `value`, for an ability with
/// subranges)?
pub fn allowed(p: &Process, aid: u32, value: Option<u64>) -> bool {
    let dom = domain(p);
    p.abilities.lock().allows(aid, dom, value)
}

/// May process `p` use ability `aid` for the values `lo` to `hi`, as a
/// server's `ConnectClientInfoAble()` asks (procnto's `nto_ability_check`)?
/// Unknown IDs are denied.
pub fn allowed_range(p: &Process, aid: u32, lo: u64, hi: u64) -> bool {
    let dom = domain(p);
    p.abilities.lock().allows_range(aid, dom, Some((lo, hi)))
}

/// The calling process's ability `aid` (with `value`): EPERM without it.
pub fn check(aid: u32, value: Option<u64>) -> Result<(), u32> {
    if allowed(process::current(), aid, value) {
        Ok(())
    } else {
        Err(EPERM)
    }
}

/// The highest priority a thread of the normal policies may take: below
/// procnto's clock interrupt thread (254 by default, `procnto -C`; 255
/// is the interprocessor-interrupt threads'), as procnto's
/// `sched_policy_max_priority()`.
pub fn priority_max() -> u8 {
    crate::clock::ist_priority() - 1
}

/// The highest priority of `SCHED_OFFLINING` and
/// `SCHED_HIGH_PRIORITY_IST` threads: at or above the clock's.
pub const PRIORITY_MAX_HIGH: u8 = 254;
/// The highest priority without `PROCMGR_AID_PRIORITY`.
pub const PRIORITY_MAX_NONPRIV: u8 = 63;

/// A priority the calling process gives one of its threads: 1 to
/// `priority_max()` (EINVAL), above `PRIORITY_MAX_NONPRIV` only with
/// `PROCMGR_AID_PRIORITY` (EPERM), as procnto.
pub fn user_priority(prio: i64) -> Result<u8, u32> {
    priority_for(process::current(), prio)
}

/// The highest priority process `p`'s threads may take:
/// `priority_max()` with `PROCMGR_AID_PRIORITY`, else
/// `PRIORITY_MAX_NONPRIV` (`sched_get_priority_max()`'s answer).
pub fn priority_cap(p: &Process) -> u8 {
    if allowed(p, PRIORITY, None) { priority_max() } else { PRIORITY_MAX_NONPRIV }
}

/// `user_priority` for process `p`'s threads.
pub fn priority_for(p: &Process, prio: i64) -> Result<u8, u32> {
    priority_up_to(p, prio, priority_max())
}

/// `priority_for` with `max` the highest priority of the policy.
pub fn priority_up_to(p: &Process, prio: i64, max: u8) -> Result<u8, u32> {
    let prio = u8::try_from(prio)
        .ok()
        .filter(|p| (1..=max).contains(p))
        .ok_or(EINVAL)?;
    if prio > PRIORITY_MAX_NONPRIV && !allowed(p, PRIORITY, None) {
        return Err(EPERM);
    }
    Ok(prio)
}

/// Apply `procmgr_ability()`'s list `words` (`struct
/// _proc_resource_setabilities.entry`) to `target`, on behalf of `caller`.
pub fn set(caller: &Process, target: &Process, words: &[u32]) -> Result<(), u32> {
    if !core::ptr::eq(caller, target) && !allowed(caller, XPROCESS_ABLE, None) {
        return Err(EPERM);
    }
    let may_grant = allowed(caller, ABLE_PRIV, None);
    let mut i = 0;
    while let Some(&e) = words.get(i) {
        i += 1;
        let range = if e & AOP_SUBRANGE != 0 {
            let w = words.get(i..i + 4).ok_or(EINVAL)?;
            i += 4;
            let lo = w[0] as u64 | (w[1] as u64) << 32;
            let hi = w[2] as u64 | (w[3] as u64) << 32;
            Some((lo, hi))
        } else {
            None
        };
        let (aid, ops, doms) = (e & 0xffff, e & AOP_MASK, e & (ADN_ROOT | ADN_NONROOT));
        if e & AID_UNCREATED != 0 {
            return Err(EINVAL);
        }
        if aid == EOL && ops == 0 {
            return Ok(());
        }
        if ops == 0 || doms == 0 {
            return Err(EINVAL);
        }
        let aids: Vec<u32> = if aid == EOL {
            (0..=MAX)
                .filter(|a| !UNDEFINED.contains(a))
                .chain(super::super::proc::procmgr::custom_abilities())
                .collect()
        } else {
            Vec::from([aid])
        };
        for aid in aids {
            if ops & (AOP_ALLOW | AOP_SUBRANGE) != 0 && privileged(aid) && !may_grant {
                return Err(EPERM);
            }
            let mut a = target.abilities.lock();
            let entry = a.entry(aid);
            for (dom, flag) in [(0, ADN_ROOT), (1, ADN_NONROOT)] {
                let d = &mut entry[dom];
                if doms & flag == 0 || d.locked {
                    continue;
                }
                if ops & AOP_DENY != 0 {
                    d.allowed = false;
                }
                if ops & AOP_ALLOW != 0 {
                    d.allowed = true;
                }
                if let Some(r) = range {
                    d.ranges.push(r);
                }
                if ops & AOP_INHERIT_YES != 0 {
                    d.inherit = true;
                }
                if ops & AOP_INHERIT_NO != 0 {
                    d.inherit = false;
                }
                if ops & AOP_LOCK != 0 {
                    d.locked = true;
                }
            }
        }
        if aid == EOL {
            return Ok(());
        }
    }
    Ok(())
}

/// Whether `caller` may read or change the scheduling of `target`'s
/// threads (`SchedGet()`, `SchedSet()`): its own process, or a target
/// whose real, effective and saved user IDs all equal the caller's
/// effective one, or with `PROCMGR_AID_SCHEDULE`; else EPERM. procnto
/// does it in `sched_thread_lookup`, shared by `ker_sched_get` and
/// `ker_sched_set`, through `nto_cred_is_allowed(.., PROCMGR_AID_SCHEDULE)`,
/// which compares only the caller's effective user ID with the target's
/// three and otherwise goes to the ability check; it runs before the
/// policy or the parameters are looked at, so a caller without the
/// right gets EPERM even for a bad policy (measured: EPERM from uid 100
/// on root's thread, for both calls; the docs of sched_setparam() and
/// sched_getparam() say "real or effective", which is looser than the
/// code). The two credential locks are taken one after the other, never
/// nested.
pub fn may_schedule(caller: &Process, target: &Process) -> Result<(), u32> {
    if core::ptr::eq(caller, target) {
        return Ok(());
    }
    let euid = caller.cred.lock().euid;
    let same_user = {
        let c = target.cred.lock();
        c.ruid == euid && c.euid == euid && c.suid == euid
    };
    if same_user || allowed(caller, SCHEDULE, None) {
        Ok(())
    } else {
        Err(EPERM)
    }
}

/// Whether `caller` may look into process `target` (its timers, CPU time,
/// runmask): itself, a process of the same effective user, or with
/// `PROCMGR_AID_XPROCESS_QUERY` (procnto's `nto_ability_check_uid`).
pub fn may_query(caller: &Process, target: &Process) -> bool {
    if core::ptr::eq(caller, target) {
        return true;
    }
    // The two credential locks one after the other, never nested (two
    // processes querying each other from two CPUs would otherwise each
    // hold one and spin on the other: review item IPC-5).
    let caller_euid = caller.cred.lock().euid;
    let target_euid = target.cred.lock().euid;
    caller_euid == target_euid || allowed(caller, XPROCESS_QUERY, None)
}
