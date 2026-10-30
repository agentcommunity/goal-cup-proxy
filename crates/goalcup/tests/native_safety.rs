//! PRD-04 acceptance `native_safety.rs` (points 3, 4 and 7): no symlink is followed, a FIFO is
//! never read and never hangs the reader, a replaced file is refused, no descriptor is kept, the
//! root never reaches a path, and `src/native/` names no harness.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use goalcup::connector::{self, builtin::CONNECTORS};
use goalcup::native::fsx::{Dir, Kind, file_stat};
use goalcup::native::pattern::{parse, path_id};
use goalcup::native::walk::{Row, Walker};
use goalcup::pipeline::{Call, Calls, Recorder};
use serde_json::Value;

fn n(s: &str) -> &OsStr {
    OsStr::new(s)
}

#[test]
fn a_symlink_is_never_followed() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("real.jsonl"), b"secret-free\n").unwrap();
    fs::create_dir(t.path().join("real_dir")).unwrap();
    symlink(t.path().join("real.jsonl"), t.path().join("link.jsonl")).unwrap();
    symlink(t.path().join("real_dir"), t.path().join("link_dir")).unwrap();

    let root = Dir::open_root(t.path()).unwrap();
    let link = root.stat_at(n("link.jsonl")).unwrap();
    assert_eq!(link.kind, Kind::Symlink);
    assert!(root.open_file(n("link.jsonl"), link.fid()).is_err());
    let target = root.stat_at(n("real.jsonl")).unwrap();
    assert!(root.open_file(n("link.jsonl"), target.fid()).is_err());

    assert_eq!(root.stat_at(n("link_dir")).unwrap().kind, Kind::Symlink);
    assert!(root.open_child(n("link_dir")).is_err());
    assert!(root.open_child(n("real_dir")).is_ok());

    let linked_root = t.path().join("linked_root");
    symlink(t.path().join("real_dir"), &linked_root).unwrap();
    assert!(Dir::open_root(&linked_root).is_err());
}

#[test]
fn a_fifo_is_never_read_and_never_hangs() {
    let t = tempfile::tempdir().unwrap();
    let fifo = t.path().join("session.jsonl");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: `c` is a live NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);

    let root = Dir::open_root(t.path()).unwrap();
    let st = root.stat_at(n("session.jsonl")).unwrap();
    assert_eq!(st.kind, Kind::Other);

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = root.open_file(n("session.jsonl"), st.fid());
        let _ = tx.send(r.is_err());
    });
    let refused = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("opening a FIFO must not hang");
    assert!(refused);
}

#[test]
fn a_file_replaced_after_its_stat_is_refused() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("s.jsonl"), b"one\n").unwrap();
    let root = Dir::open_root(t.path()).unwrap();
    let seen = root.stat_at(n("s.jsonl")).unwrap();

    fs::write(t.path().join("s.jsonl.tmp"), b"two\n").unwrap();
    fs::rename(t.path().join("s.jsonl.tmp"), t.path().join("s.jsonl")).unwrap();

    let err = root.open_file(n("s.jsonl"), seen.fid()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    let now = root.stat_at(n("s.jsonl")).unwrap();
    assert_ne!(now.fid(), seen.fid());
    assert!(root.open_file(n("s.jsonl"), now.fid()).is_ok());
}

#[test]
fn a_second_listing_starts_from_the_top() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a.jsonl"), b"a").unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    let root = Dir::open_root(t.path()).unwrap();
    let mut names = root.names().unwrap();
    names.sort();
    assert_eq!(names, [OsString::from("a.jsonl"), OsString::from("sub")]);
    assert_eq!(root.names().unwrap().len(), 2);
}

/// Descriptors this process holds on paths under `dir` (other tests may run in parallel).
#[cfg(target_os = "linux")]
fn fds_under(dir: &std::path::Path) -> usize {
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|e| fs::read_link(e.ok()?.path()).ok())
        .filter(|p| p.starts_with(dir))
        .count()
}

#[cfg(target_os = "linux")]
#[test]
fn no_descriptor_is_kept() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().canonicalize().unwrap();
    fs::create_dir(dir.join("d")).unwrap();
    fs::write(dir.join("d").join("f.jsonl"), b"x").unwrap();
    let before = fds_under(&dir);
    for _ in 0..50 {
        let root = Dir::open_root(&dir).unwrap();
        let d = root.open_child(n("d")).unwrap();
        for name in d.names().unwrap() {
            let st = d.stat_at(&name).unwrap();
            let f = d.open_file(&name, st.fid()).unwrap();
            file_stat(&f).unwrap();
        }
        let _ = d.open_file(n("f.jsonl"), root.stat_at(n(".")).unwrap().fid());
        let _ = root.open_child(n("missing"));
    }
    assert_eq!(fds_under(&dir), before);
}

fn row(name: &str, root: &Path, pattern: &str) -> Row {
    Row {
        name: name.into(),
        root: root.into(),
        pattern: parse(pattern).unwrap(),
    }
}

fn notes(calls: &Calls, what: &str) -> Vec<Value> {
    let all = calls.all().into_iter();
    let events = all.filter_map(|c| {
        if let Call::Event(v) = c {
            Some(v)
        } else {
            None
        }
    });
    events.filter(|e| e["what"] == what).collect()
}

#[test]
fn a_walk_follows_no_symlink_and_names_a_linked_file_once() {
    let t = tempfile::tempdir().unwrap();
    let (root, outside) = (t.path().join("root"), t.path().join("outside"));
    fs::create_dir_all(outside.join("sub")).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(outside.join("a.jsonl"), b"x").unwrap();
    fs::write(outside.join("sub").join("b.jsonl"), b"x").unwrap();
    symlink(outside.join("a.jsonl"), root.join("link.jsonl")).unwrap();
    symlink(outside.join("sub"), root.join("linked")).unwrap();

    let rows = [row("r", &root, "**/*.jsonl")];
    let (rec, calls) = Recorder::for_tests(1 << 20);
    let mut walker = Walker::default();
    assert!(walker.pass(&rec, &rows).found.is_empty());
    assert!(walker.pass(&rec, &rows).found.is_empty());
    let linked = notes(&calls, "native_symlink");
    assert_eq!(linked.len(), 1, "one note per path: {linked:?}");
    assert_eq!(linked[0]["path"], path_id("r", Path::new("link.jsonl")));
}

#[test]
fn the_root_never_reaches_a_path() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("private-root-name");
    fs::create_dir_all(root.join("x/y")).unwrap();
    fs::write(root.join("x/y/s.jsonl"), b"x").unwrap();
    symlink(root.join("x/y/s.jsonl"), root.join("x/l.jsonl")).unwrap();
    let rows = [row("r", &root, "**/*.jsonl")];
    let (rec, calls) = Recorder::for_tests(1 << 20);
    let found = Walker::default().pass(&rec, &rows).found;
    let mut ids: Vec<String> = found.iter().map(|f| path_id("r", &f.rel)).collect();
    let linked = notes(&calls, "native_symlink");
    ids.extend(linked.iter().map(|n| n["path"].to_string()));
    let marker = t.path().to_string_lossy().into_owned();
    assert!(
        ids.iter()
            .all(|id| !id.contains("private-root-name") && !id.contains(&marker)),
        "{ids:?}"
    );
}

#[test]
fn hermes_rows_take_the_state_database_and_no_request_dump() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    fs::write(home.join("sessions/request_dump_x.json"), b"{}").unwrap();
    fs::write(home.join("state.db"), b"SQLite format 3\0").unwrap();
    let c = connector::load("hermes-cli", None).unwrap();
    let home_s = home.to_str().unwrap();
    let rows: Vec<Row> = c
        .native
        .iter()
        .map(|n| {
            row(
                &n.name,
                Path::new(&n.root.replace("{home}", home_s)),
                &n.pattern,
            )
        })
        .collect();
    let (rec, _) = Recorder::for_tests(1 << 20);
    let found = Walker::default().pass(&rec, &rows).found;
    let names: Vec<PathBuf> = found
        .iter()
        .map(|f| rows[f.row].root.join(&f.rel))
        .collect();
    assert!(names.contains(&home.join("state.db")), "{names:?}");
    assert!(
        !names
            .iter()
            .any(|p| p.to_string_lossy().contains("request_dump"))
    );
}

#[test]
fn no_native_source_file_names_a_built_in_harness() {
    // Whole words only, comments included, so `example` never matches `amp`.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/native");
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let text = fs::read_to_string(&path).unwrap().to_lowercase();
        let words: Vec<&str> = text.split(|c: char| !c.is_ascii_alphanumeric()).collect();
        for (id, _) in CONNECTORS {
            let name = id.split(['-', '_', ' ']).next().unwrap().to_lowercase();
            assert!(!words.contains(&name.as_str()), "{} names `{name}`", path.display());
        }
    }
}
