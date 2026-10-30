//! `goalcup.log`, the one plain text log per entry (PRD-10 section 12, seam S11).
//!
//! The watchdog, the recorder and the uploader append to `runs/<run>/<entry>/goalcup.log`
//! through [`line`] and nothing else. The file is never uploaded and is not part of the record;
//! the end screen and refusals that need support detail name its path.
//!
//! No free text can enter: the event, every key and every [`Fact::Word`] are strings fixed in
//! the binary, a host is a checked [`Host`], a path is quoted with escapes, and an `io::Error` is
//! logged by its kind name only. So no header value, body byte, token or credential reaches the
//! file (point 4).
//!
//! Each line is opened, written with one `O_APPEND` write of at most 4 KiB, and closed, so
//! concurrent writers never interleave inside a line and only one descriptor is held at a time
//! (PRD-02 section 10). A failure is ignored: the log is never a reason to stop (point 3).

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{SystemTime, UNIX_EPOCH};

/// The longest line written, newline included. A longer line is cut and keeps its newline.
const MAX_LINE: usize = 4096;

/// One value on a log line.
#[derive(Debug)]
pub enum Fact {
    /// A count or size, written in decimal.
    N(u64),
    /// A time, written RFC 3339 UTC to the second.
    Time(SystemTime),
    /// A checked host name.
    Host(Host),
    /// A path, written quoted with escapes, so a newline in it never breaks the line.
    Path(PathBuf),
    /// A word fixed in the binary.
    Word(&'static str),
    /// A child's exit, written `code:N` or `signal:N`.
    Exit(ExitStatus),
    /// An I/O failure, written as its kind name only.
    Io(io::ErrorKind),
}

/// A host name or address checked for host syntax, with an optional `:port`.
///
/// Accepts ASCII letters, digits, `-` and `.` in labels of 1 to 63 characters, at most 253 in
/// all, with at least one dot (or `localhost`), or a bracketed IPv6 literal. A token-like string
/// (no dot, a label over 63, any other character) is refused, so a secret cannot pass as a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host(String);

impl Host {
    /// The host if `s` has host syntax, else `None`.
    pub fn checked(s: &str) -> Option<Host> {
        let (name, port) = if let Some(v6) = s.strip_prefix('[') {
            let (addr, rest) = v6.split_once(']')?;
            let v6_ok = (2..=45).contains(&addr.len())
                && addr.contains(':')
                && addr.bytes().all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
            if !v6_ok {
                return None;
            }
            let port = match rest {
                "" => None,
                r => Some(r.strip_prefix(':')?),
            };
            (None, port)
        } else {
            match s.rsplit_once(':') {
                Some((n, p)) => (Some(n), Some(p)),
                None => (Some(s), None),
            }
        };
        if let Some(p) = port {
            let digits = (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit());
            if !digits || p.parse::<u16>().is_err() {
                return None;
            }
        }
        if let Some(n) = name {
            let label_ok = |l: &str| {
                (1..=63).contains(&l.len())
                    && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !l.starts_with('-')
                    && !l.ends_with('-')
            };
            let n = n.strip_suffix('.').unwrap_or(n);
            let dotted = n.contains('.') || n.eq_ignore_ascii_case("localhost");
            if n.len() > 253 || !dotted || !n.split('.').all(label_ok) {
                return None;
            }
        }
        Some(Host(s.to_owned()))
    }

    /// The host as checked.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Appends one line `<time> <event> key=value ...` to `entry_dir/goalcup.log` (mode 0600).
///
/// Never panics, never returns an error and never creates a directory: a missing folder or a
/// failed write loses the line and nothing else.
///
/// ```
/// use goalcup::cli::log::{Fact, line};
/// line(std::path::Path::new("/nonexistent"), "boot", &[("port", Fact::N(8080))]);
/// ```
///
/// Free text cannot be passed as the event:
///
/// ```compile_fail,E0597
/// use goalcup::cli::log::line;
/// let event = String::from("a request header");
/// line(std::path::Path::new("/nonexistent"), &event, &[]);
/// ```
///
/// nor as a word:
///
/// ```compile_fail,E0308
/// use goalcup::cli::log::{Fact, line};
/// let word = String::from("a request header");
/// line(std::path::Path::new("/nonexistent"), "boot", &[("why", Fact::Word(word))]);
/// ```
pub fn line(entry_dir: &Path, event: &'static str, facts: &[(&'static str, Fact)]) {
    let mut buf = String::with_capacity(128);
    push_time(&mut buf, SystemTime::now());
    buf.push(' ');
    buf.push_str(event);
    for (key, fact) in facts {
        buf.push(' ');
        buf.push_str(key);
        buf.push('=');
        push_fact(&mut buf, fact);
    }
    if buf.len() > MAX_LINE - 1 {
        let mut cut = MAX_LINE - 1;
        while !buf.is_char_boundary(cut) {
            cut -= 1;
        }
        buf.truncate(cut);
    }
    buf.push('\n');
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(super::goalcup_log(entry_dir));
    if let Ok(mut file) = file {
        // One write call: an append of at most 4 KiB is never interleaved with another.
        let _ = file.write(buf.as_bytes());
    }
}

fn push_fact(buf: &mut String, fact: &Fact) {
    match fact {
        Fact::N(n) => {
            let _ = write!(buf, "{n}");
        }
        Fact::Time(t) => push_time(buf, *t),
        Fact::Host(h) => buf.push_str(h.as_str()),
        Fact::Path(p) => {
            let _ = write!(buf, "{p:?}");
        }
        Fact::Word(w) => buf.push_str(w),
        Fact::Exit(status) => {
            let _ = match (status.code(), status.signal()) {
                (Some(code), _) => write!(buf, "code:{code}"),
                (None, Some(sig)) => write!(buf, "signal:{sig}"),
                (None, None) => write!(buf, "unknown"),
            };
        }
        Fact::Io(kind) => {
            let _ = write!(buf, "{kind:?}");
        }
    }
}

/// RFC 3339 UTC to the second, `YYYY-MM-DDTHH:MM:SSZ`. A time before 1970 is written as 1970.
fn push_time(buf: &mut String, t: SystemTime) {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days);
    let _ = write!(
        buf,
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    );
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian (H. Hinnant's algorithm).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_756), (2026, 10, 30));
    }
}
