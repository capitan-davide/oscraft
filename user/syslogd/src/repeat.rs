//! Repeated messages, per output, as `logmsg()` and `markit()`: a line
//! like the one before is counted, not written, and the count comes out
//! as "last message repeated N times" when another line comes, or after
//! 30 s, then 2 min, then 10 min (`repeatinterval`).

const INTERVALS: [u64; 3] = [30, 120, 600];

/// `fprintlog_successive()`'s text.
fn repeated(count: u32) -> String {
    format!("syslogd: last message repeated {count} times")
}

/// What to write for an event, in order.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Writes {
    /// A count first, then the line itself.
    pub count: Option<String>,
    pub line: bool,
}

#[derive(Default)]
pub struct Repeat {
    previous: String,
    count: u32,
    /// When this output was last written.
    pub written: u64,
    interval: usize,
}

impl Repeat {
    fn due(&self) -> u64 {
        self.written + INTERVALS[self.interval]
    }

    fn back_off(&mut self) {
        self.interval = (self.interval + 1).min(INTERVALS.len() - 1);
    }

    fn flush(&mut self, now: u64) -> Option<String> {
        if self.count == 0 {
            return None;
        }
        let text = repeated(self.count);
        (self.count, self.written) = (0, now);
        Some(text)
    }

    /// A message `text` at `now`.
    pub fn message(&mut self, text: &str, now: u64) -> Writes {
        if text == self.previous && !text.is_empty() {
            self.count += 1;
            if now > self.due() {
                let count = self.flush(now);
                self.back_off();
                return Writes { count, line: false };
            }
            return Writes::default();
        }
        let count = self.flush(now);
        self.interval = 0;
        self.previous = text.into();
        self.written = now;
        Writes { count, line: true }
    }

    /// The timer at `now`, as `markit()`: a count held too long.
    pub fn tick(&mut self, now: u64) -> Option<String> {
        if self.count == 0 || now < self.due() {
            return None;
        }
        let text = self.flush(now);
        self.back_off();
        text
    }

    /// Before the file closes (`die()`, a reload): what is held.
    pub fn finish(&mut self, now: u64) -> Option<String> {
        self.flush(now)
    }

    /// `-- MARK --` isn't for a file written lately.
    pub fn takes_mark(&self, now: u64, interval: u64) -> bool {
        now.saturating_sub(self.written) >= interval / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> Writes {
        Writes { count: None, line: true }
    }

    #[test]
    fn a_repeat_is_counted_and_comes_out_before_the_next_line() {
        let mut r = Repeat::default();
        assert_eq!(r.message("a", 100), line());
        assert_eq!(r.message("a", 101), Writes::default());
        assert_eq!(r.message("a", 102), Writes::default());
        let w = r.message("b", 103);
        assert_eq!(w.count.as_deref(), Some("syslogd: last message repeated 2 times"));
        assert!(w.line);
    }

    #[test]
    fn different_lines_all_go_through() {
        let mut r = Repeat::default();
        assert_eq!(r.message("a", 1), line());
        assert_eq!(r.message("b", 2), line());
        assert_eq!(r.message("a", 3), line());
    }

    #[test]
    fn the_timer_flushes_at_30_then_120_then_600() {
        let mut r = Repeat::default();
        r.message("a", 0);
        r.message("a", 1);
        assert_eq!(r.tick(29), None);
        assert!(r.tick(30).is_some());
        r.message("a", 31);
        assert_eq!(r.tick(149), None);
        assert!(r.tick(150).is_some());
        r.message("a", 151);
        assert_eq!(r.tick(749), None);
        assert!(r.tick(750).is_some());
        r.message("a", 751);
        assert!(r.tick(1350).is_some(), "stays at 10 minutes");
    }

    #[test]
    fn a_long_burst_reports_as_it_goes() {
        let mut r = Repeat::default();
        r.message("a", 0);
        r.message("a", 10);
        assert!(r.message("a", 31).count.is_some());
    }

    #[test]
    fn nothing_held_nothing_to_say() {
        let mut r = Repeat::default();
        r.message("a", 0);
        assert_eq!(r.tick(1000), None);
        assert_eq!(r.finish(1000), None);
    }

    #[test]
    fn marks_skip_a_file_written_lately() {
        let mut r = Repeat::default();
        r.message("a", 1000);
        assert!(!r.takes_mark(1500, 1200));
        assert!(r.takes_mark(1600, 1200));
    }
}
