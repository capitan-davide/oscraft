//! `syslog.conf`, as FreeBSD's `cfline()` and `parse_selector()` read it,
//! and which rules a message matches, as `logmsg()` decides.

/// `LOG_NFACILITIES`, and the internal `mark` one.
pub const FACILITIES: usize = 25;
const NOPRI: u8 = 0x10;
const ALLPRI: u8 = 0xff;
const PRI_LT: u8 = 1;
const PRI_EQ: u8 = 2;
const PRI_GT: u8 = 4;

const FACILITY_NAMES: [(&str, usize); 24] = [
    ("kern", 0), ("user", 1), ("mail", 2), ("daemon", 3), ("auth", 4), ("syslog", 5), ("lpr", 6),
    ("news", 7), ("uucp", 8), ("cron", 9), ("authpriv", 10), ("ftp", 11), ("ntp", 12),
    ("security", 13), ("console", 14), ("local0", 16), ("local1", 17), ("local2", 18),
    ("local3", 19), ("local4", 20), ("local5", 21), ("local6", 22), ("local7", 23), ("mark", 24),
];
const LEVEL_NAMES: [(&str, u8); 11] = [
    ("emerg", 0), ("panic", 0), ("alert", 1), ("crit", 2), ("err", 3), ("error", 3),
    ("warning", 4), ("warn", 4), ("notice", 5), ("info", 6), ("debug", 7),
];

/// The `mark` facility.
pub const MARK: usize = 24;

/// Where a rule's messages go.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Action {
    /// A file; `-/path` doesn't sync each line.
    File { path: String, sync: bool },
    Console,
    /// `@host` or `@host:port`.
    Forward { host: String, port: u16 },
    /// `|command`.
    Pipe(String),
    /// `*`: everybody logged in; there is no list of them yet.
    Everyone,
    /// User names, not served yet.
    Unsupported(String),
}

/// A `:property, op, "value"` filter, as `prop_filter_compile()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropFilter {
    property: Property,
    compare: Compare,
    invert: bool,
    insensitive: bool,
    value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Property {
    Message,
    Hostname,
    Program,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compare {
    Contains,
    Equal,
    StartsWith,
    /// `regex`, or `ereregex` when extended: libc's, through `Matcher`.
    Regex { extended: bool },
}

/// `regcomp()` and `regexec()`, which this library leaves to its caller:
/// (pattern, extended, ignore case, subject).
pub type Matcher<'a> = &'a dyn Fn(&str, bool, bool, &str) -> bool;

impl PropFilter {
    fn parse(text: &str) -> Result<Self, String> {
        let bad = || format!("bad filter: {text}");
        let (property, rest) = text.split_once(',').ok_or_else(bad)?;
        let property = match property.trim().to_ascii_lowercase().as_str() {
            "msg" => Property::Message,
            "hostname" | "source" => Property::Hostname,
            "programname" => Property::Program,
            _ => return Err(bad()),
        };
        let (op, value) = rest.split_once(',').ok_or_else(bad)?;
        let mut op = op.trim().to_ascii_lowercase();
        let invert = op.starts_with('!');
        if invert {
            op.remove(0);
        }
        let insensitive = op.starts_with("icase_");
        if insensitive {
            op.drain(..6);
        }
        let compare = match op.as_str() {
            "contains" => Compare::Contains,
            "isequal" => Compare::Equal,
            "startswith" => Compare::StartsWith,
            "regex" => Compare::Regex { extended: false },
            "ereregex" => Compare::Regex { extended: true },
            o => return Err(format!("filter not served yet: {o}")),
        };
        let value = value.trim();
        let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).ok_or_else(bad)?;
        Ok(Self { property, compare, invert, insensitive, value: value.replace("\\\"", "\"") })
    }

    fn accepts(&self, m: &Selected, regex: Matcher) -> bool {
        let subject = match self.property {
            Property::Message => m.text,
            Property::Hostname => m.host,
            Property::Program => m.program,
        };
        if let Compare::Regex { extended } = self.compare {
            return regex(&self.value, extended, self.insensitive, subject) != self.invert;
        }
        let folded;
        let (subject, value) = if self.insensitive {
            folded = (subject.to_ascii_lowercase(), self.value.to_ascii_lowercase());
            (folded.0.as_str(), folded.1.as_str())
        } else {
            (subject, self.value.as_str())
        };
        let hit = match self.compare {
            Compare::Contains => subject.contains(value),
            Compare::Equal => subject == value,
            Compare::StartsWith => subject.starts_with(value),
            Compare::Regex { .. } => false,
        };
        hit != self.invert
    }
}

/// What rules look at in a message.
#[derive(Clone, Copy)]
pub struct Selected<'a> {
    pub facility: usize,
    pub level: u8,
    pub program: &'a str,
    pub host: &'a str,
    pub text: &'a str,
}

/// `skip_message()`: whether `spec`, `+a,b` or `-a,b`, leaves `name` out.
fn skipped(name: &str, spec: &str, case: bool) -> bool {
    if spec.is_empty() {
        return false;
    }
    let (exclude, list) = match spec.as_bytes()[0] {
        b'-' => (true, &spec[1..]),
        b'+' => (false, &spec[1..]),
        _ => (false, spec),
    };
    let same = |item: &&str| if case { *item == name } else { item.eq_ignore_ascii_case(name) };
    let listed = list.split(',').any(|i| same(&i));
    if listed { exclude } else { !exclude }
}

/// One line of the file.
#[derive(Debug, Clone)]
pub struct Rule {
    level: [u8; FACILITIES],
    compare: [u8; FACILITIES],
    program: String,
    host: String,
    filter: Option<PropFilter>,
    pub action: Action,
}

impl Rule {
    /// `logmsg()`'s checks: priority, host, program and property.
    pub fn accepts(&self, m: &Selected, regex: Matcher) -> bool {
        self.matches(m.facility, m.level)
            && !skipped(m.host, &self.host, false)
            && !skipped(m.program, &self.program, true)
            && self.filter.as_ref().is_none_or(|f| f.accepts(m, regex))
    }

    /// Whether a message of `facility` and `level` goes to this rule.
    pub fn matches(&self, facility: usize, level: u8) -> bool {
        let Some(&want) = self.level.get(facility) else { return false };
        let compare = self.compare[facility];
        want != NOPRI
            && ((compare & PRI_EQ != 0 && want == level)
                || (compare & PRI_LT != 0 && want < level)
                || (compare & PRI_GT != 0 && want > level))
    }
}

fn parse_action(text: &str) -> Action {
    if let Some(path) = text.strip_prefix('-')
        && path.starts_with('/')
    {
        return Action::File { path: path.into(), sync: false };
    }
    if let Some(target) = text.strip_prefix('@') {
        // `[v6]:port`, `host:port`, or `host`.
        let (host, port) = match target.strip_prefix('[').and_then(|t| t.split_once(']')) {
            Some((host, rest)) => (host, rest.strip_prefix(':')),
            None => match target.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (target, None),
            },
        };
        let port = port.and_then(|p| p.parse().ok()).unwrap_or(514);
        return Action::Forward { host: host.into(), port };
    }
    if let Some(command) = text.strip_prefix('|') {
        return Action::Pipe(command.trim_start().into());
    }
    match text {
        "/dev/console" => Action::Console,
        "*" => Action::Everyone,
        t if t.starts_with('/') => Action::File { path: t.into(), sync: true },
        t => Action::Unsupported(t.into()),
    }
}

/// `facility[,facility].[<=>!]level` joined by `;`.
fn parse_selectors(text: &str, rule: &mut Rule) -> Result<(), String> {
    for selector in text.split(';').filter(|s| !s.is_empty()) {
        let (facilities, priority) = selector.split_once('.').ok_or_else(|| format!("no level: {selector}"))?;
        let mut compare = 0;
        let mut invert = false;
        let mut name = priority;
        while let Some(c) = name.chars().next() {
            match c {
                '!' => invert = !invert,
                '<' => compare |= PRI_LT,
                '=' => compare |= PRI_EQ,
                '>' => compare |= PRI_GT,
                _ => break,
            }
            name = &name[1..];
        }
        if compare == 0 {
            compare = PRI_EQ | PRI_GT;
        }
        if invert {
            compare ^= PRI_LT | PRI_EQ | PRI_GT;
        }
        let level = match name {
            "*" => ALLPRI,
            "none" => NOPRI,
            n => LEVEL_NAMES.iter().find(|l| l.0 == n).ok_or_else(|| format!("unknown level: {n}"))?.1,
        };
        for facility in facilities.split(',') {
            if facility == "*" {
                // `*` isn't `mark`, which is asked for by name.
                for f in 0..FACILITIES - 1 {
                    (rule.level[f], rule.compare[f]) = (level, compare);
                }
            } else {
                let f = FACILITY_NAMES.iter().find(|n| n.0 == facility);
                let f = f.ok_or_else(|| format!("unknown facility: {facility}"))?.1;
                (rule.level[f], rule.compare[f]) = (level, compare);
            }
        }
    }
    Ok(())
}

/// A line without its trailing comment: `#`, unless written `\\#`.
fn without_comment(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'#') => {
                chars.next();
                out.push('#');
            }
            '#' if !out.is_empty() => break,
            c => out.push(c),
        }
    }
    out.trim_end().into()
}

struct Blocks {
    program: String,
    host: String,
    filter: String,
}

/// `+host`, `-host`: from here on.
fn host_block(line: &str, this_host: &str) -> String {
    let (sign, rest) = line.split_at(1);
    let rest = rest.trim_start();
    if rest.is_empty() || rest.starts_with('*') {
        return String::new();
    }
    let rest = if rest.starts_with('@') { this_host } else { rest };
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || ".-,:%".contains(c))).unwrap_or(rest.len());
    format!("{sign}{}", &rest[..end])
}

/// The word after a block's mark: nothing, or `*`, ends the block.
fn block_word(rest: &str) -> String {
    let rest = rest.trim_start();
    if rest.is_empty() || rest.starts_with('*') {
        return String::new();
    }
    rest.split_whitespace().next().unwrap_or("").into()
}

/// Reads `text`, as `parseconfigfile()`. `include` names a directory, whose
/// `*.conf` files `read_dir` gives in order, and they can't include more.
fn parse_into(
    text: &str,
    this_host: &str,
    read_dir: Option<&dyn Fn(&str) -> Vec<String>>,
    rules: &mut Vec<Rule>,
    errors: &mut Vec<String>,
) {
    let mut blocks = Blocks { program: String::new(), host: String::new(), filter: String::new() };
    for line in text.lines() {
        let mut p = line.trim_start();
        if p.is_empty() {
            continue;
        }
        if let Some(rest) = p.strip_prefix("include")
            && rest.starts_with([' ', '\t'])
            && let Some(read_dir) = read_dir
        {
            for text in read_dir(rest.trim()) {
                parse_into(&text, this_host, None, rules, errors);
            }
            continue;
        }
        if let Some(rest) = p.strip_prefix('#') {
            if !rest.starts_with(['!', '+', '-', ':']) {
                continue;
            }
            p = rest;
        }
        match p.as_bytes()[0] {
            b'+' | b'-' => {
                blocks.host = host_block(p, this_host);
                continue;
            }
            b'!' => {
                blocks.program = block_word(&p[1..]);
                continue;
            }
            b':' => {
                let rest = p[1..].trim();
                blocks.filter = if rest.starts_with('*') { String::new() } else { rest.into() };
                continue;
            }
            _ => {}
        }
        let line = without_comment(line);
        let Some(split) = line.find([' ', '\t']) else {
            errors.push(format!("no action: {line}"));
            continue;
        };
        let (selectors, action) = line.split_at(split);
        let filter = if blocks.filter.is_empty() {
            None
        } else {
            match PropFilter::parse(&blocks.filter) {
                Ok(f) => Some(f),
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            }
        };
        let mut rule = Rule {
            level: [NOPRI; FACILITIES],
            compare: [0; FACILITIES],
            program: blocks.program.clone(),
            host: blocks.host.clone(),
            filter,
            action: parse_action(action.trim()),
        };
        match parse_selectors(selectors, &mut rule) {
            Ok(()) => rules.push(rule),
            Err(e) => errors.push(e),
        }
    }
}

/// The rules of a `syslog.conf`, and what couldn't be read.
pub fn parse_full(
    text: &str,
    this_host: &str,
    read_dir: &dyn Fn(&str) -> Vec<String>,
) -> (Vec<Rule>, Vec<String>) {
    let (mut rules, mut errors) = (Vec::new(), Vec::new());
    parse_into(text, this_host, Some(read_dir), &mut rules, &mut errors);
    (rules, errors)
}

/// `parse_full()` of a file without `include`s.
pub fn parse(text: &str) -> (Vec<Rule>, Vec<String>) {
    parse_full(text, "oscraft", &|_| Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> Rule {
        let (r, e) = parse(line);
        assert!(e.is_empty(), "{e:?}");
        r.into_iter().next().unwrap()
    }

    #[test]
    fn a_level_takes_what_is_as_severe_or_more() {
        let r = one("*.err /var/log/e");
        assert!(r.matches(3, 3) && r.matches(3, 0));
        assert!(!r.matches(3, 4));
    }

    #[test]
    fn the_comparisons() {
        assert!(one("*.=info /a").matches(1, 6));
        assert!(!one("*.=info /a").matches(1, 5));
        assert!(one("*.<info /a").matches(1, 7));
        assert!(!one("*.<info /a").matches(1, 6));
        assert!(one("*.!=info /a").matches(1, 5));
        assert!(!one("*.!=info /a").matches(1, 6));
    }

    #[test]
    fn selectors_add_up_and_none_takes_away() {
        let r = one("*.notice;mail.none;kern.debug\t/a");
        assert!(r.matches(1, 5));
        assert!(!r.matches(2, 0));
        assert!(r.matches(0, 7));
        assert!(!r.matches(1, 6));
    }

    #[test]
    fn facilities_in_a_list() {
        let r = one("mail,cron.info /a");
        assert!(r.matches(2, 6) && r.matches(9, 6));
        assert!(!r.matches(1, 6));
    }

    #[test]
    fn star_is_not_mark_and_level_star_is_all() {
        let r = one("*.* /a");
        assert!(r.matches(1, 7));
        assert!(!r.matches(24, 6));
        assert!(one("mark.* /a").matches(24, 6));
    }

    #[test]
    fn actions() {
        assert_eq!(one("*.* /var/log/m").action, Action::File { path: "/var/log/m".into(), sync: true });
        assert_eq!(one("*.* -/var/log/m").action, Action::File { path: "/var/log/m".into(), sync: false });
        assert_eq!(one("*.* /dev/console").action, Action::Console);
        assert_eq!(one("*.* root").action, Action::Unsupported("root".into()));
    }

    fn no_regex(_: &str, _: bool, _: bool, _: &str) -> bool {
        false
    }

    #[test]
    fn regex_goes_to_the_matcher() {
        let (r, e) = parse(":msg, ereregex, \"^a+$\"\n*.* /a\n:msg, !icase_regex, \"b\"\n*.* /b\n");
        assert!(e.is_empty(), "{e:?}");
        let says = |want: &'static str, extended: bool, icase: bool| {
            move |p: &str, x: bool, i: bool, _: &str| {
                assert_eq!((p, x, i), (want, extended, icase));
                true
            }
        };
        assert!(r[0].accepts(&sel("p", "h", "aa"), &says("^a+$", true, false)));
        assert!(!r[1].accepts(&sel("p", "h", "aa"), &says("b", false, true)), "inverted");
    }

    fn sel<'a>(program: &'a str, host: &'a str, text: &'a str) -> Selected<'a> {
        Selected { facility: 1, level: 5, program, host, text }
    }

    #[test]
    fn a_program_block() {
        let (r, _) = parse("!sshd\n*.* /a\n!*\n*.* /b\n");
        assert!(r[0].accepts(&sel("sshd", "h", ""), &no_regex) && !r[0].accepts(&sel("cron", "h", ""), &no_regex));
        assert!(r[1].accepts(&sel("cron", "h", ""), &no_regex));
    }

    #[test]
    fn excluding_and_lists() {
        let (r, _) = parse("!-devd,ntpd\n*.* /a\n");
        assert!(!r[0].accepts(&sel("devd", "h", ""), &no_regex) && r[0].accepts(&sel("cron", "h", ""), &no_regex));
        let (r, _) = parse("!+a,b\n*.* /a\n");
        assert!(r[0].accepts(&sel("b", "h", ""), &no_regex) && !r[0].accepts(&sel("ab", "h", ""), &no_regex));
    }

    #[test]
    fn a_host_block_ignores_case() {
        let (r, _) = parse("+GW1\n*.* /a\n-@\n*.* /b\n");
        assert!(r[0].accepts(&sel("p", "gw1", ""), &no_regex) && !r[0].accepts(&sel("p", "gw2", ""), &no_regex));
        assert!(!r[1].accepts(&sel("p", "oscraft", ""), &no_regex));
    }

    #[test]
    fn hash_bang_is_a_block_too() {
        let (r, _) = parse("#!sshd\n*.* /a\n");
        assert!(!r[0].accepts(&sel("cron", "h", ""), &no_regex));
    }

    #[test]
    fn property_filters() {
        let (r, e) = parse(":msg, contains, \"disk\"\n*.* /a\n:msg, !startswith, \"x\"\n*.* /b\n:*\n*.* /c\n");
        assert!(e.is_empty(), "{e:?}");
        assert!(r[0].accepts(&sel("p", "h", "a disk error"), &no_regex) && !r[0].accepts(&sel("p", "h", "fine"), &no_regex));
        assert!(!r[1].accepts(&sel("p", "h", "xy"), &no_regex) && r[1].accepts(&sel("p", "h", "yx"), &no_regex));
        assert!(r[2].accepts(&sel("p", "h", "anything"), &no_regex));
    }

    #[test]
    fn insensitive_filters_and_programs() {
        let (r, _) = parse(":programname, icase_isequal, \"SSHD\"\n*.* /a\n");
        assert!(r[0].accepts(&sel("sshd", "h", ""), &no_regex));
        let (r, e) = parse(":msg, !icase_contains, \"DISK\"\n*.* /a\n");
        assert!(e.is_empty(), "{e:?}");
        assert!(!r[0].accepts(&sel("p", "h", "a disk"), &no_regex));
    }

    #[test]
    fn trailing_comments() {
        let (r, _) = parse("*.* /a # why\n*.* /b\\#c\n");
        assert_eq!(r[0].action, Action::File { path: "/a".into(), sync: true });
        assert_eq!(r[1].action, Action::File { path: "/b#c".into(), sync: true });
    }

    #[test]
    fn include_reads_a_directory_once() {
        let dir = |d: &str| {
            assert_eq!(d, "/etc/syslog.d");
            vec!["*.* /in\ninclude /nowhere\n".to_string()]
        };
        let (r, e) = parse_full("include /etc/syslog.d\n*.* /out\n", "h", &dir);
        assert_eq!(r.len(), 2);
        assert_eq!(e.len(), 1, "an include in an included file isn't read: {e:?}");
    }

    #[test]
    fn forwarding_and_programs() {
        assert_eq!(one("*.* @log").action, Action::Forward { host: "log".into(), port: 514 });
        assert_eq!(one("*.* @log:5514").action, Action::Forward { host: "log".into(), port: 5514 });
        assert_eq!(one("*.* @[::1]:5514").action, Action::Forward { host: "::1".into(), port: 5514 });
        assert_eq!(one("*.* | /bin/cat").action, Action::Pipe("/bin/cat".into()));
        assert_eq!(one("*.emerg *").action, Action::Everyone);
    }

    #[test]
    fn comments_and_errors() {
        let (r, e) = parse("# a comment\n\n*.bad /a\nnonsense\n*.info /b\n");
        assert_eq!((r.len(), e.len()), (1, 2));
    }
}
