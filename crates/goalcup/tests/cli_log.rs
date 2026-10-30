//! `goalcup.log` (seam S11): one line per call, never interleaved, mode 0600, no free text, and
//! never a reason to stop (points 3 and 4).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;

use goalcup::cli::log::{Fact, Host, line};

fn log_of(dir: &Path) -> String {
    fs::read_to_string(dir.join("goalcup.log")).expect("goalcup.log")
}

/// `<YYYY-MM-DDTHH:MM:SSZ> <event> k=v ...` with plain (unquoted) values.
fn parses(l: &str) -> bool {
    let mut words = l.split(' ');
    let time = words.next().unwrap_or("");
    let time_ok = time.len() == 20
        && time.ends_with('Z')
        && time.as_bytes()[10] == b'T'
        && time.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            10 => b == b'T',
            13 | 16 => b == b':',
            19 => b == b'Z',
            _ => b.is_ascii_digit(),
        });
    let event_ok = words.next().is_some_and(|e| !e.is_empty() && !e.contains('='));
    time_ok
        && event_ok
        && words.all(|kv| kv.split_once('=').is_some_and(|(k, v)| !k.is_empty() && !v.is_empty()))
}

#[test]
fn one_call_writes_one_line() {
    let dir = tempfile::tempdir().unwrap();
    line(dir.path(), "boot", &[("port", Fact::N(41_000)), ("mode", Fact::Word("null"))]);
    let text = log_of(dir.path());
    assert!(text.ends_with('\n'));
    assert_eq!(text.lines().count(), 1);
    assert!(parses(text.trim_end()), "{text}");
}

#[test]
fn concurrent_writers_never_interleave() {
    let dir = tempfile::tempdir().unwrap();
    let writers: Vec<_> = (0..2u64)
        .map(|t| {
            let d: PathBuf = dir.path().to_owned();
            thread::spawn(move || {
                for i in 0..500u64 {
                    line(&d, "tick", &[("writer", Fact::N(t)), ("i", Fact::N(i))]);
                }
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap();
    }
    let text = log_of(dir.path());
    assert!(text.ends_with('\n'));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1000);
    assert!(lines.iter().all(|l| parses(l)));
}

#[test]
fn file_is_private() {
    let dir = tempfile::tempdir().unwrap();
    line(dir.path(), "boot", &[]);
    let mode = fs::metadata(dir.path().join("goalcup.log")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn newline_in_a_path_stays_on_one_line() {
    let dir = tempfile::tempdir().unwrap();
    line(dir.path(), "skill", &[("path", Fact::Path(PathBuf::from("/a\nb\r\nc")))]);
    let text = log_of(dir.path());
    assert_eq!(text.lines().count(), 1);
    assert_eq!(text.matches('\n').count(), 1);
}

#[test]
fn a_long_line_is_cut_and_keeps_its_newline() {
    let dir = tempfile::tempdir().unwrap();
    line(dir.path(), "skill", &[("path", Fact::Path(PathBuf::from("x".repeat(10_000))))]);
    let text = log_of(dir.path());
    assert!(text.len() <= 4096);
    assert!(text.ends_with('\n'));
    assert_eq!(text.lines().count(), 1);
}

#[test]
fn host_check_refuses_tokens() {
    let token = "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8S9t0";
    assert_eq!(token.len(), 40);
    let jwt = concat!(
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.",
        "eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.",
        "SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
    );
    for bad in [token, jwt, ""] {
        assert!(Host::checked(bad).is_none(), "{bad}");
    }
    for good in ["api.openai.com", "localhost:8080", "[::1]:443", "127.0.0.1"] {
        assert_eq!(Host::checked(good).map(|h| h.as_str().to_owned()), Some(good.to_owned()));
    }
}

#[test]
fn missing_entry_dir_is_ignored_and_not_created() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("runs/r1/e1");
    line(&missing, "boot", &[("why", Fact::Io(std::io::ErrorKind::NotFound))]);
    assert!(!missing.exists());
    assert!(!dir.path().join("runs").exists());
}
