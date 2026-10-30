//! `goalcup.log`, the one plain text log per entry (PRD-10 section 12, seam S11).
//!
//! The watchdog, the recorder and the uploader append to `runs/<run>/<entry>/goalcup.log`
//! through [`line`] and nothing else. The file is never uploaded and is not part of the record;
//! the end screen and refusals that need support detail name its path.
//!
//! The event, every key and every [`Fact::Word`] are strings fixed in the binary, and an
//! `io::Error` is logged by its kind name only. Two facts carry runtime text, and the caller
//! answers for them: a [`Fact::Host`] is a host the proxy or connector already parsed from a
//! connection authority or the host list, and a [`Fact::Path`] is a path goalcup built itself (the
//! home, run or entry paths, or a file flag the participant gave), never a value taken from
//! traffic or the environment. With that, no header value, body byte, token or credential reaches
//! the file (point 4).
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

use goalcup_format::dump::wall;

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
    /// A path goalcup built (home, run or entry paths, or a file flag the participant gave),
    /// never a value from traffic or the environment. Written quoted with escapes, so a newline in
    /// it never breaks the line.
    Path(PathBuf),
    /// A word fixed in the binary.
    Word(&'static str),
    /// A child's exit, written `code:N` or `signal:N`.
    Exit(ExitStatus),
    /// An I/O failure, written as its kind name only (PRD-10 section 12's prose; a seventh
    /// variant beside the six of the S11 shape).
    Io(io::ErrorKind),
}

/// A host as the log writes it: a name or address checked for host syntax, with an optional
/// `:port`.
///
/// Accepts ASCII letters, digits, `-` and `.` in labels of 1 to 63 characters, at most 253 in
/// all, with at least one dot (or `localhost`), or a bracketed IPv6 literal. It accepts IP
/// addresses, `localhost` and ports because the log names the connection it saw, local servers
/// included; the connector's host list check has its own, stricter rules. The check is about
/// syntax only and does not tell a secret from a host: pass only hosts the proxy or connector
/// already parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host(String);

impl Host {
    /// The host if `s` has host syntax, else `None`.
    pub fn checked(s: &str) -> Option<Host> {
        let (name, port) = if let Some(v6) = s.strip_prefix('[') {
            let (addr, rest) = v6.split_once(']')?;
            let v6_ok = (2..=45).contains(&addr.len())
                && addr.contains(':')
                && addr
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
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
    buf.push_str(&wall_secs(SystemTime::now()));
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
        Fact::Time(t) => buf.push_str(&wall_secs(*t)),
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
fn wall_secs(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    wall(secs.saturating_mul(1_000_000_000))
}
