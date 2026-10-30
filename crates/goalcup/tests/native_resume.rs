//! PRD-04 acceptance `native_resume.rs` (points 1 and 5, section 7): cursors from an earlier boot
//! resume a file at its written offset, start a new generation when it changed, and keep an
//! unchanged snapshot from being copied again.

mod common;

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use common::recorder::data_of;
use goalcup::native::fsx::{Dir, Stat};
use goalcup::native::tail::Tail;
use goalcup::pipeline::cursors::{NativeCursor, SnapshotCursor};
use goalcup::pipeline::{Call, Cursors, Recorder};
use serde_json::Value;

/// An OPEN's meta and `from`, with the bytes of its stream.
type Opened = (Value, u64, Vec<u8>);

/// One boot: a Tail on `cursors`, one walk and one read of `name`.
fn boot(dir: &Path, name: &str, cursors: Cursors, snapshot: bool) -> Vec<Opened> {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let mut tail = Tail::new(UNIX_EPOCH, cursors);
    tail.settle(
        &rec,
        format!("row:{name}"),
        dir,
        Path::new(name),
        stat(dir, name),
        snapshot,
    );
    tail.read(&rec, 1 << 30, Duration::ZERO);
    let f = |c: Call| match c {
        Call::Open(sid, _, _, _, from, _, meta) => Some((meta, from, data_of(&calls, sid))),
        _ => None,
    };
    calls.all().into_iter().filter_map(f).collect()
}

fn stat(dir: &Path, name: &str) -> Stat {
    Dir::open_root(dir)
        .unwrap()
        .stat_at(OsStr::new(name))
        .unwrap()
}

fn cursor(id: &str, generation: u64, st: &Stat, off: u64) -> Cursors {
    let mut c = Cursors::default();
    c.native.insert(
        id.into(),
        NativeCursor {
            generation,
            fid: st.fid(),
            off,
        },
    );
    c
}

#[test]
fn the_same_file_resumes_its_generation_at_the_cursor() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a"), b"0123456789").unwrap();
    let st = stat(dir.path(), "a");
    let o = boot(dir.path(), "a", cursor("row:a", 3, &st, 4), false);
    assert_eq!(o.len(), 1);
    assert_eq!(
        (&o[0].0["gen"], &o[0].0["from"], o[0].1),
        (&Value::from(3), &Value::from(4), 4)
    );
    assert_eq!(o[0].2, b"456789");
}

#[test]
fn a_changed_fid_or_a_shorter_file_starts_the_next_generation_from_zero() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a"), b"0123456789").unwrap();
    let mut other = stat(dir.path(), "a");
    other.ino += 1;
    let shorter = cursor("row:a", 3, &stat(dir.path(), "a"), 11);
    for cursors in [cursor("row:a", 3, &other, 4), shorter] {
        let o = boot(dir.path(), "a", cursors, false);
        assert_eq!((&o[0].0["gen"], o[0].1), (&Value::from(4), 0));
        assert_eq!(o[0].2, b"0123456789");
    }
}

#[test]
fn a_snapshot_equal_to_its_cursor_is_not_copied_again() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("m.json"), b"{}").unwrap();
    let st = stat(dir.path(), "m.json");
    let mut c = Cursors::default();
    let snap = SnapshotCursor {
        size: st.size,
        mtime_ns: st.mtime_ns,
    };
    c.snapshots.insert("row:m.json".into(), snap);
    assert!(boot(dir.path(), "m.json", c.clone(), true).is_empty());
    fs::write(dir.path().join("m.json"), b"{\"v\":1}").unwrap();
    let o = boot(dir.path(), "m.json", c, true);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].2, b"{\"v\":1}");
}
