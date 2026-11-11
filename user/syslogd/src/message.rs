//! A message as `parsemsg()` reads it: `<priority>` and the RFC 3164 time
//! `syslog(3)` writes, and the line `fprintlog()` writes to a file.

/// `LOG_USER | LOG_NOTICE`, for a message without a priority.
const DEFAULT_PRIORITY: u32 = (1 << 3) | 5;
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

#[derive(Debug, PartialEq, Eq)]
pub struct Message {
    pub facility: usize,
    pub level: u8,
    /// `Mmm dd hh:mm:ss`.
    pub time: String,
    /// `tag[pid]: text`.
    pub text: String,
    /// The priority, the tag and the text alone, for rules.
    pub priority: u32,
    pub program: String,
    pub body: String,
}

/// `Mmm dd hh:mm:ss` of `secs` since the Epoch, in UTC.
pub fn format_time(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rest = secs % 86400;
    // Civil from days, after Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let _ = era;
    format!("{} {day:2} {:02}:{:02}:{:02}", MONTHS[month as usize - 1], rest / 3600, rest / 60 % 60, rest % 60)
}

/// Whether `s` starts with `Mmm dd hh:mm:ss `.
fn has_time(s: &[u8]) -> bool {
    s.len() >= 16
        && MONTHS.iter().any(|m| s[..3] == *m.as_bytes())
        && s[3] == b' '
        && s[6] == b' '
        && s[9] == b':'
        && s[12] == b':'
        && s[15] == b' '
}

/// `parsemsg_remove_unsafe_characters()`: a control character as `^X`.
fn safe_text(raw: &[u8]) -> String {
    let mut out = String::with_capacity(raw.len());
    for &c in raw {
        match c {
            b'\t' => out.push('\t'),
            0..=0x1f | 0x7f => {
                out.push('^');
                out.push((c ^ 0x40) as char);
            }
            _ => out.push(c as char),
        }
    }
    out
}

/// `Mmm dd hh:mm:ss` of an RFC 3339 time.
fn short_time(t: &str) -> Option<String> {
    let (date, clock) = t.split_once('T')?;
    let mut d = date.split('-');
    let (_, month, day) = (d.next()?, d.next()?.parse::<usize>().ok()?, d.next()?.parse::<u32>().ok()?);
    let clock = clock.get(..8)?;
    Some(format!("{} {day:2} {clock}", MONTHS.get(month.checked_sub(1)?)?))
}

/// `parsemsg_rfc5424()`: `1 time host tag pid msgid sd text`, what
/// FreeBSD's `syslog(3)` sends. The line stays the classic one.
fn rfc5424(priority: u32, rest: &[u8], now: u64) -> Option<Message> {
    let rest = rest.strip_prefix(b"1 ")?;
    let mut fields = rest.splitn(6, |&c| c == b' ');
    let word = |f: &[u8]| std::str::from_utf8(f).ok().map(str::to_owned);
    let time = word(fields.next()?)?;
    let _host = fields.next()?;
    let tag = word(fields.next()?)?;
    let pid = word(fields.next()?)?;
    let _msgid = fields.next()?;
    let tail = fields.next()?;
    // Structured data: `-`, or `[...]` groups, then the text.
    let text = if let Some(t) = tail.strip_prefix(b"- ") {
        t
    } else if tail == b"-" {
        &[]
    } else {
        let end = tail.windows(2).position(|w| w == b"] ")?;
        &tail[end + 2..]
    };
    let time = short_time(&time).unwrap_or_else(|| format_time(now));
    let head = match (tag.as_str(), pid.as_str()) {
        ("-", _) => String::new(),
        (t, "-") => format!("{t}: "),
        (t, p) => format!("{t}[{p}]: "),
    };
    let body = safe_text(text);
    Some(Message {
        facility: (priority >> 3) as usize,
        level: (priority & 7) as u8,
        time,
        text: head + &body,
        priority,
        program: if tag == "-" { String::new() } else { tag },
        body,
    })
}

/// Reads a datagram received at `now`, seconds since the Epoch.
pub fn parse(raw: &[u8], now: u64) -> Message {
    let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
    let mut priority = DEFAULT_PRIORITY;
    let mut rest = raw;
    if let Some(after) = raw.strip_prefix(b"<")
        && let Some(end) = after.iter().position(|&c| c == b'>')
        && end > 0
        && let Some(n) = std::str::from_utf8(&after[..end]).ok().and_then(|n| n.parse::<u32>().ok())
        && n < 192
    {
        priority = n;
        rest = &after[end + 1..];
    }
    // Only the kernel logs as `kern`: anyone else is `user`, as FreeBSD.
    if priority >> 3 == 0 {
        priority |= 1 << 3;
    }
    if let Some(m) = rfc5424(priority, rest, now) {
        return m;
    }
    let (time, text) = if has_time(rest) {
        (safe_text(&rest[..15]), &rest[16..])
    } else {
        (format_time(now), rest)
    };
    let text = safe_text(text);
    // `tag[pid]: rest`, RFC 3164's convention.
    let end = text.find([':', '[', ' ']).unwrap_or(0);
    let tagged = text[end..].starts_with([':', '[']);
    let program = if tagged { text[..end].to_string() } else { String::new() };
    let body = text.split_once(": ").filter(|_| tagged).map_or(text.as_str(), |b| b.1).to_string();
    Message { facility: (priority >> 3) as usize, level: (priority & 7) as u8, time, priority, program, body, text }
}

/// A line from `/dev/klog`, as `printsys()`: `kern`, and `notice` unless
/// it says otherwise. Only this input may be the kernel.
pub fn parse_kernel(line: &[u8], now: u64) -> Message {
    let mut priority = 5;
    let mut rest = line;
    if let Some(after) = line.strip_prefix(b"<")
        && let Some(end) = after.iter().position(|&c| c == b'>')
        && let Some(n) = std::str::from_utf8(&after[..end]).ok().and_then(|n| n.parse::<u32>().ok())
        && n < 192
    {
        priority = n;
        rest = &after[end + 1..];
    }
    let body = safe_text(rest);
    Message {
        facility: (priority >> 3) as usize,
        level: (priority & 7) as u8,
        time: format_time(now),
        text: format!("kernel: {body}"),
        priority,
        program: "kernel".into(),
        body,
    }
}

/// The datagram `@host` gets, RFC 3164's.
pub fn forward_line(m: &Message, host: &str) -> String {
    format!("<{}>{} {host} {}", m.priority, m.time, m.text)
}

/// The line a file gets.
pub fn file_line(m: &Message, host: &str) -> String {
    format!("{} {host} {}\n", m.time, m.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_syslog_3_sends() {
        let m = parse(b"<30>Nov  6 09:15:01 cron[12]: job done", 0);
        assert_eq!((m.facility, m.level), (3, 6));
        assert_eq!(m.time, "Nov  6 09:15:01");
        assert_eq!(m.text, "cron[12]: job done");
        assert_eq!(file_line(&m, "oscraft"), "Nov  6 09:15:01 oscraft cron[12]: job done\n");
    }

    #[test]
    fn no_priority_is_user_notice_and_no_time_is_now() {
        let m = parse(b"hello", 1_762_420_501);
        assert_eq!((m.facility, m.level), (1, 5));
        assert_eq!(m.time, "Nov  6 09:15:01");
    }

    #[test]
    fn nobody_is_the_kernel() {
        assert_eq!(parse(b"<3>x", 0).facility, 1);
    }

    #[test]
    fn a_bad_priority_is_text() {
        assert_eq!(parse(b"<999>x", 0).text, "<999>x");
        assert_eq!(parse(b"<>x", 0).text, "<>x");
    }

    #[test]
    fn the_program_and_the_body() {
        let m = parse(b"<13>1 2026-11-06T09:15:01Z h sshd 7 - - login ok", 0);
        assert_eq!((m.program.as_str(), m.body.as_str()), ("sshd", "login ok"));
        let m = parse(b"<13>Nov  6 09:15:01 cron[12]: job", 0);
        assert_eq!((m.program.as_str(), m.body.as_str()), ("cron", "job"));
        assert_eq!(parse(b"plain words", 0).program, "");
    }

    #[test]
    fn a_forwarded_line() {
        let m = parse(b"<13>1 2026-11-06T09:15:01Z h tag 7 - - hi", 0);
        assert_eq!(forward_line(&m, "oscraft"), "<13>Nov  6 09:15:01 oscraft tag[7]: hi");
    }

    #[test]
    fn the_kernels_lines() {
        let m = parse_kernel(b"bootscript: done", 0);
        assert_eq!((m.facility, m.level), (0, 5));
        assert_eq!(m.text, "kernel: bootscript: done");
        assert_eq!(parse_kernel(b"<3>disk error", 0).level, 3);
    }

    #[test]
    fn control_characters_show() {
        assert_eq!(parse(b"<13>a\x1bb\tc\n", 0).text, "a^[b\tc");
    }

    #[test]
    fn what_freebsds_libc_sends() {
        let m = parse(b"<13>1 2026-11-06T09:15:01.572154+00:00 oscraft sltest 36878 - - notice one", 0);
        assert_eq!((m.facility, m.level), (1, 5));
        assert_eq!(file_line(&m, "oscraft"), "Nov  6 09:15:01 oscraft sltest[36878]: notice one\n");
    }

    #[test]
    fn rfc5424_without_a_pid_or_with_structured_data() {
        let m = parse(b"<13>1 2026-11-06T09:15:01Z h tag - - - text", 0);
        assert_eq!(m.text, "tag: text");
        let m = parse(b"<13>1 2026-11-06T09:15:01Z h tag 7 - [a b=\"c\"] text", 0);
        assert_eq!(m.text, "tag[7]: text");
    }

    #[test]
    fn the_calendar() {
        assert_eq!(format_time(0), "Jan  1 00:00:00");
        assert_eq!(format_time(951_782_400), "Feb 29 00:00:00");
    }
}
