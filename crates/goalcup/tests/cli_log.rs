//! `goalcup.log` (seam S11): one line per call, never interleaved, mode 0600, no free text, and
//! never a reason to stop (points 3 and 4).

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;

use goalcup::cli::log::{Fact, Host, line};

fn log_of(dir: &Path) -> String {
    fs::read_to_string(dir.join("goalcup.log")).expect("goalcup.log")
}

#[test]
fn one_call_writes_one_line() {
    let dir = tempfile::tempdir().unwrap();
    line(
        dir.path(),
        "boot",
        &[("port", Fact::N(41_000)), ("mode", Fact::Word("null"))],
    );
    let text = log_of(dir.path());
    assert!(text.ends_with('\n'));
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains(" port=41000 "), "{text}");
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
    let mut seen = HashSet::new();
    for l in text.lines() {
        let words: Vec<&str> = l.split(' ').collect();
        assert_eq!(words.len(), 4, "{l}");
        assert_eq!(words[1], "tick", "{l}");
        let w = words[2].strip_prefix("writer=").expect(l);
        let i = words[3].strip_prefix("i=").expect(l);
        assert!(seen.insert((w.parse::<u64>().unwrap(), i.parse::<u64>().unwrap())), "{l}");
    }
    let all: HashSet<(u64, u64)> = (0..2).flat_map(|w| (0..500).map(move |i| (w, i))).collect();
    assert_eq!(seen, all);
}

#[test]
fn file_is_private() {
    let dir = tempfile::tempdir().unwrap();
    line(dir.path(), "boot", &[]);
    let mode = fs::metadata(dir.path().join("goalcup.log"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn newline_in_a_path_stays_on_one_line() {
    let dir = tempfile::tempdir().unwrap();
    line(
        dir.path(),
        "skill",
        &[("path", Fact::Path(PathBuf::from("/a\nb\r\nc")))],
    );
    let text = log_of(dir.path());
    assert_eq!(text.lines().count(), 1);
    assert_eq!(text.matches('\n').count(), 1);
}

#[test]
fn a_long_line_is_cut_and_keeps_its_newline() {
    let dir = tempfile::tempdir().unwrap();
    line(
        dir.path(),
        "skill",
        &[("path", Fact::Path(PathBuf::from("x".repeat(10_000))))],
    );
    let text = log_of(dir.path());
    assert!(text.len() <= 4096);
    assert!(text.ends_with('\n'));
    assert_eq!(text.lines().count(), 1);
}

#[test]
fn host_check_is_host_syntax() {
    for bad in ["", "no dots", "a..b", "-a.b", "a.b:99999", "a.b:", "[zz::1]", "a/b.c"] {
        assert!(Host::checked(bad).is_none(), "{bad}");
    }
    for good in ["api.openai.com", "localhost:8080", "[::1]:443", "127.0.0.1"] {
        assert_eq!(
            Host::checked(good).map(|h| h.as_str().to_owned()),
            Some(good.to_owned())
        );
    }
}

#[test]
fn missing_entry_dir_is_ignored_and_not_created() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("runs/r1/e1");
    line(
        &missing,
        "boot",
        &[("why", Fact::Io(std::io::ErrorKind::NotFound))],
    );
    assert!(!missing.exists());
    assert!(!dir.path().join("runs").exists());
}
