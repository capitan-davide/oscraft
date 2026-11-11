//! Boot script compilation (`[+script]` files) into `union script_cmd`
//! records, which procnto runs after it has started.
//!
//! Syntax (QNX 8.0 `mkifs` reference, "Script files"): one command per line,
//! optionally preceded by `[modifiers]` and `NAME=value` assignments. On a
//! line of their own, modifiers and assignments apply to all later commands.
//! Internal commands: `display_msg`, `waitfor`, `reopen`, `procmgr_symlink`.

use anyhow::{Context, Result, anyhow, bail, ensure};

use crate::buildfile::{split_words, strip_comment, unquote};
use crate::format::*;

#[derive(Debug, Clone, Default)]
struct Modifiers {
    priority: Option<(u8, u8)>,
    cpu: Option<u8>,
    session: bool,
    external: bool,
    critical: bool,
    debug: bool,
    argv0: Option<String>,
}

impl Modifiers {
    fn apply(&mut self, token: &str) -> Result<()> {
        let flag = |t: &str| t.starts_with('+');
        match token.trim_start_matches(['+', '-']) {
            "session" => self.session = flag(token),
            "external" => self.external = flag(token),
            "critical" => self.critical = flag(token),
            "debug" => self.debug = flag(token),
            _ => match token.split_once('=') {
                Some(("pri", v)) => {
                    let (num, policy) =
                        match v.char_indices().find(|(_, c)| c.is_ascii_alphabetic()) {
                            Some((i, _)) => (&v[..i], &v[i..]),
                            None => (v, ""),
                        };
                    let policy = match policy {
                        "" => SCRIPT_POLICY_NOCHANGE,
                        "f" => SCRIPT_POLICY_FIFO,
                        "r" => SCRIPT_POLICY_RR,
                        "o" => SCRIPT_POLICY_OTHER,
                        p => bail!("unknown scheduling policy '{p}' in '{token}'"),
                    };
                    let prio: u8 = num
                        .parse()
                        .map_err(|_| anyhow!("bad priority in '{token}'"))?;
                    self.priority = Some((prio, policy));
                }
                Some(("cpu", v)) => {
                    self.cpu = Some(v.parse().map_err(|_| anyhow!("bad cpu in '{token}'"))?)
                }
                Some(("argv0", v)) => self.argv0 = Some(unquote(v)),
                _ => bail!("unknown script modifier '{token}'"),
            },
        }
        Ok(())
    }
}

/// Set or replace `NAME=value` in an environment list.
fn set_env(env: &mut Vec<String>, assignment: String) {
    let name = assignment.split('=').next().unwrap_or("").to_string();
    match env
        .iter_mut()
        .find(|e| e.split('=').next() == Some(name.as_str()))
    {
        Some(e) => *e = assignment,
        None => env.push(assignment),
    }
}

fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Wait time in seconds (one decimal allowed) to the stored count of
/// 100 ms checks. The reference image stores 50 for the default 5 s.
fn checks(secs: Option<&str>) -> Result<u16> {
    let secs: f64 = match secs {
        Some(s) => unquote(s)
            .parse()
            .map_err(|_| anyhow!("bad wait time '{s}'"))?,
        None => 5.0,
    };
    let n = (secs * 1000.0 / SCRIPT_CHECKS_MS as f64).round() as i64;
    Ok(n.clamp(0, u16::MAX as i64) as u16)
}

/// A script record: its size is 16 bits, so a longer one is an error,
/// not a wrapped size (review TOOLS-6).
fn record(ty: u8, body: &[u8]) -> Result<Vec<u8>> {
    let mut r = vec![0, 0, ty, 0];
    r.extend_from_slice(body);
    pad_to(&mut r, 4);
    let size = r.len();
    ensure!(size <= u16::MAX as usize, "script record of {size} bytes: at most {}", u16::MAX);
    r[0] = size as u8;
    r[1] = (size >> 8) as u8;
    Ok(r)
}

/// Compile a script. `boot_env` is the environment of the last bootstrap
/// executable (procnto), which every external command inherits.
pub fn compile(text: &str, boot_env: &[String]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut global_mods = Modifiers::default();
    let mut global_env: Vec<String> = boot_env.to_vec();
    for (n, raw) in text.lines().enumerate() {
        let ctx = || format!("script line {}", n + 1);
        let words = split_words(strip_comment(raw)).with_context(ctx)?;
        let mut words = words.as_slice();
        let mut mods: Option<Modifiers> = None;
        while let Some(w) = words.first().filter(|w| w.starts_with('[')) {
            let m = mods.get_or_insert_with(|| global_mods.clone());
            for t in split_words(&w[1..w.len() - 1]).with_context(ctx)? {
                m.apply(&t).with_context(ctx)?;
            }
            words = &words[1..];
        }
        let mut env_local: Vec<String> = Vec::new();
        while let Some(w) = words.first().filter(|w| is_assignment(w)) {
            env_local.push(unquote(w));
            words = &words[1..];
        }
        let Some((cmd, args)) = words.split_first() else {
            if let Some(m) = mods {
                global_mods = m;
            }
            for e in env_local {
                set_env(&mut global_env, e);
            }
            continue;
        };
        let mods = mods.unwrap_or_else(|| global_mods.clone());
        let args: Vec<String> = args.iter().map(|a| unquote(a)).collect();
        let cmd = unquote(cmd);
        let internal = !mods.external;
        let rec = match cmd.as_str() {
            "display_msg" if internal => {
                let mut msg = args.join(" ").into_bytes();
                msg.extend_from_slice(b"\n\0");
                record(SCRIPT_TYPE_DISPLAY_MSG, &msg).with_context(ctx)?
            }
            "waitfor" | "reopen" if internal => {
                let ty = if cmd == "waitfor" {
                    SCRIPT_TYPE_WAITFOR
                } else {
                    SCRIPT_TYPE_REOPEN
                };
                let path = match args.first() {
                    Some(p) => p.clone(),
                    None if ty == SCRIPT_TYPE_REOPEN => "/dev/console".into(),
                    None => bail!("{}: waitfor needs a path", ctx()),
                };
                let c = checks(args.get(1).map(String::as_str)).with_context(ctx)?;
                let mut body = c.to_le_bytes().to_vec();
                push_cstr(&mut body, &path);
                record(ty, &body).with_context(ctx)?
            }
            "procmgr_symlink" if internal => {
                let [src, dst] = args.as_slice() else {
                    bail!(
                        "{}: procmgr_symlink needs a source and a destination",
                        ctx()
                    );
                };
                let mut body = Vec::new();
                push_cstr(&mut body, src);
                push_cstr(&mut body, dst);
                record(SCRIPT_TYPE_PROCMGR_SYMLINK, &body).with_context(ctx)?
            }
            _ => external(&cmd, args, &mods, &global_env, env_local).with_context(ctx)?,
        };
        out.extend_from_slice(&rec);
    }
    out.extend_from_slice(&[0; 4]);
    Ok(out)
}

fn external(
    cmd: &str,
    mut args: Vec<String>,
    mods: &Modifiers,
    global_env: &[String],
    env_local: Vec<String>,
) -> Result<Vec<u8>> {
    let mut flags = 0;
    if args.last().is_some_and(|a| a == "&") {
        args.pop();
        flags |= SCRIPT_FLAGS_BACKGROUND;
    }
    if mods.session {
        flags |= SCRIPT_FLAGS_SESSION;
    }
    if mods.critical {
        flags |= SCRIPT_FLAGS_CRITICAL;
    }
    if mods.debug {
        flags |= SCRIPT_FLAGS_KDEBUG;
    }
    let (priority, policy) = match mods.priority {
        Some((p, pol)) => {
            flags |= SCRIPT_FLAGS_SCHED_SET;
            (p, pol)
        }
        None => (0, SCRIPT_POLICY_NOCHANGE),
    };
    let cpu = match mods.cpu {
        Some(c) => {
            flags |= SCRIPT_FLAGS_CPU_SET;
            c
        }
        None => 0,
    };
    let mut env = global_env.to_vec();
    for e in env_local {
        set_env(&mut env, e);
    }
    let argv0 = mods.argv0.clone().unwrap_or_else(|| cmd.to_string());
    let argv: Vec<String> = std::iter::once(argv0).chain(args).collect();
    if argv.len() > u8::MAX as usize || env.len() > u8::MAX as usize {
        bail!("too many arguments or environment variables for '{cmd}'");
    }
    let mut body = vec![
        cpu,
        flags,
        0,
        0,
        policy,
        priority,
        argv.len() as u8,
        env.len() as u8,
    ];
    push_cstr(&mut body, cmd);
    for s in argv.iter().chain(&env) {
        push_cstr(&mut body, s);
    }
    record(SCRIPT_TYPE_EXTERNAL, &body).with_context(|| format!("command '{cmd}' too long for a script record"))
}

/// Decode a compiled script into readable lines (for `dumpifs`).
pub fn decode(mut b: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    while b.len() >= 4 {
        let size = get_u16(b, 0) as usize;
        if size == 0 || size > b.len() {
            break;
        }
        let rec = &b[..size];
        let strings = |from: usize| -> Vec<String> {
            rec[from..]
                .split(|&c| c == 0)
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect()
        };
        let line = match rec[2] {
            SCRIPT_TYPE_EXTERNAL => {
                let (cpu, flags, policy, prio, argc, envc) = (
                    rec[4],
                    rec[5],
                    rec[8],
                    rec[9],
                    rec[10] as usize,
                    rec[11] as usize,
                );
                let s = strings(12);
                let mut mods = Vec::new();
                if flags & SCRIPT_FLAGS_SCHED_SET != 0 {
                    let p = ["", "f", "r", "o"]
                        .get(policy as usize)
                        .copied()
                        .unwrap_or("?");
                    mods.push(format!("pri={prio}{p}"));
                }
                if flags & SCRIPT_FLAGS_CPU_SET != 0 {
                    mods.push(format!("cpu={cpu}"));
                }
                if flags & SCRIPT_FLAGS_SESSION != 0 {
                    mods.push("+session".into());
                }
                if flags & SCRIPT_FLAGS_CRITICAL != 0 {
                    mods.push("+critical".into());
                }
                let env = s
                    .get(1 + argc..1 + argc + envc)
                    .unwrap_or_default()
                    .join(" ");
                let argv = s.get(1..1 + argc).unwrap_or_default().join(" ");
                let bg = if flags & SCRIPT_FLAGS_BACKGROUND != 0 {
                    " &"
                } else {
                    ""
                };
                let mods = if mods.is_empty() {
                    String::new()
                } else {
                    format!("[{}] ", mods.join(" "))
                };
                format!("{mods}{} (argv: {argv}){bg}  env: {env}", s[0])
            }
            SCRIPT_TYPE_DISPLAY_MSG => format!("display_msg {:?}", strings(4)[0]),
            t @ (SCRIPT_TYPE_WAITFOR | SCRIPT_TYPE_REOPEN) => {
                let checks = get_u16(rec, 4) as u32;
                let name = if t == SCRIPT_TYPE_WAITFOR {
                    "waitfor"
                } else {
                    "reopen"
                };
                format!(
                    "{name} {} {:.1}",
                    strings(6)[0],
                    checks as f64 * SCRIPT_CHECKS_MS as f64 / 1000.0
                )
            }
            SCRIPT_TYPE_PROCMGR_SYMLINK => {
                let s = strings(4);
                format!(
                    "procmgr_symlink {} {}",
                    s[0],
                    s.get(1).map_or("", String::as_str)
                )
            }
            t => format!("<record type {t}, {size} bytes>"),
        };
        lines.push(line);
        b = &b[size.next_multiple_of(4).min(b.len())..];
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOT_ENV: [&str; 2] = [
        "PATH=/proc/boot:/sbin:/bin:/usr/bin:/usr/sbin:/usr/libexec",
        "LD_LIBRARY_PATH=/proc/boot:/lib:/usr/lib:/lib/dll:/lib/dll/pci",
    ];

    fn env() -> Vec<String> {
        BOOT_ENV.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn record_size_limit() {
        assert!(record(SCRIPT_TYPE_DISPLAY_MSG, &[b'x'; 100]).is_ok());
        assert!(record(SCRIPT_TYPE_DISPLAY_MSG, &[b'x'; 70_000]).is_err());
    }

    #[test]
    fn external_command_matches_reference_layout() {
        // From the QNX 8.0 RPi4 BSP image: "slogger2" after SYSNAME/TERM/ENV.
        let s = compile(
            "SYSNAME=nto\nTERM=qansi\nENV=/etc/profile\nslogger2\n",
            &env(),
        )
        .unwrap();
        let mut expect = vec![0u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 5];
        for p in [
            "slogger2",
            "slogger2",
            BOOT_ENV[0],
            BOOT_ENV[1],
            "SYSNAME=nto",
            "TERM=qansi",
            "ENV=/etc/profile",
        ] {
            push_cstr(&mut expect, p);
        }
        pad_to(&mut expect, 4);
        expect[0] = expect.len() as u8;
        assert_eq!(expect.len(), 192, "record size seen in the reference image");
        assert_eq!(&s[..192], &expect[..]);
        assert_eq!(&s[192..], &[0, 0, 0, 0]);
    }

    #[test]
    fn internal_commands() {
        let s = compile(
            "display_msg \"Starting wdtkick ...\"\nwaitfor /dev/pipe\nreopen /dev/ser1 2.5\n",
            &[],
        )
        .unwrap();
        assert_eq!(
            &s[..4],
            &[28, 0, SCRIPT_TYPE_DISPLAY_MSG, 0],
            "size seen in the reference image"
        );
        assert_eq!(&s[4..26], b"Starting wdtkick ...\n\0");
        let w = &s[28..];
        assert_eq!(w[2], SCRIPT_TYPE_WAITFOR);
        assert_eq!(
            get_u16(w, 4),
            50,
            "5 s default, as stored in the reference image"
        );
        assert_eq!(
            decode(&s),
            vec![
                "display_msg \"Starting wdtkick ...\\n\"",
                "waitfor /dev/pipe 5.0",
                "reopen /dev/ser1 2.5"
            ]
        );
    }

    #[test]
    fn modifiers_and_background() {
        let s = compile("[+session] ksh &\n[pri=10f cpu=2] PATH=/bin top\n", &[]).unwrap();
        assert_eq!(s[5], SCRIPT_FLAGS_SESSION | SCRIPT_FLAGS_BACKGROUND);
        let second = &s[s[0] as usize..];
        assert_eq!(second[4], 2);
        assert_eq!(second[5], SCRIPT_FLAGS_SCHED_SET | SCRIPT_FLAGS_CPU_SET);
        assert_eq!((second[8], second[9]), (SCRIPT_POLICY_FIFO, 10));
        assert_eq!(second[11], 1, "local PATH only");
    }
}
