//! PRD-04 acceptance `native_finish.rs` (points 1, 3 and 6): the running reader reads what a
//! harness writes, the seal reads every file whole with no byte cap (a snapshot rewritten under a
//! copy in progress too), a seal whose time guard runs out names each unread file's exact loss (a
//! database set's too), and a row that finds nothing is named once.

mod common;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use common::recorder::{data_of, ends, events, gaps, opens};
use goalcup::connector::{Form, NativeSource, NativeSources};
use goalcup::native::{self, Missing};
use goalcup::pipeline::{Call, Calls, EndReason, GapReason, Mono, Recorder};
use serde_json::Value;

fn sources(rows: &[(&str, &Path, &str, bool)]) -> NativeSources {
    let since = SystemTime::now() - Duration::from_secs(60);
    let row = |&(name, root, pattern, warn_missing): &(&str, &Path, &str, bool)| NativeSource {
        name: name.into(),
        root: root.into(),
        pattern: pattern.into(),
        form: Form::Append,
        since,
        warn_missing,
    };
    NativeSources(rows.iter().map(row).collect())
}

fn append(p: &Path, b: &[u8]) {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .unwrap()
        .write_all(b)
        .unwrap();
}

fn notes(calls: &Calls, what: &str) -> Vec<Value> {
    events(calls)
        .into_iter()
        .filter(|v| v["what"] == what)
        .collect()
}

/// The stream ID opened for path ID `id`.
fn sid(c: &Calls, id: &str) -> Option<u64> {
    let mut o = opens(c).into_iter();
    o.find_map(|(s, _, m)| (m["path"] == id).then_some(s))
}

fn end_of(c: &Calls, sid: u64) -> Option<EndReason> {
    let mut e = ends(c).into_iter();
    e.find_map(|(s, r, ..)| (s == sid).then_some(r))
}

#[test]
fn the_running_reader_reads_and_finish_ends_every_stream_complete() {
    let dir = tempfile::tempdir().unwrap();
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let reader = native::start(rec, sources(&[("t", dir.path(), "**/*.jsonl", true)]));
    fs::create_dir(dir.path().join("sub")).unwrap();
    let p = dir.path().join("sub/a.jsonl");
    append(&p, b"{\"one\":1}\n");
    let deadline = Instant::now() + Duration::from_secs(10);
    while sid(&calls, "t:sub/a.jsonl").is_none_or(|s| data_of(&calls, s).is_empty()) {
        assert!(Instant::now() < deadline, "the running reader read nothing");
        std::thread::sleep(Duration::from_millis(20));
    }
    append(&p, b"{\"two\":2}\n{\"thr");
    let done = reader.finish(true, None);
    let s = sid(&calls, "t:sub/a.jsonl").unwrap();
    assert_eq!(data_of(&calls, s), fs::read(&p).unwrap());
    assert_eq!(end_of(&calls, s), Some(EndReason::Complete));
    assert_eq!((done.streams, done.cut_files, done.cut_bytes), (1, 0, 0));
    assert!(gaps(&calls).is_empty());
}

#[test]
fn finish_with_no_guard_reads_past_one_slice_and_a_file_written_just_before() {
    let dir = tempfile::tempdir().unwrap();
    let (rec, calls) = Recorder::for_tests(0);
    let big: Vec<u8> = (0..(64u32 << 20) + 4099).map(|i| (i % 251) as u8).collect();
    fs::write(dir.path().join("big.jsonl"), &big).unwrap();
    let reader = native::start(rec, sources(&[("t", dir.path(), "*.jsonl", true)]));
    std::thread::sleep(Duration::from_millis(50));
    append(&dir.path().join("late.jsonl"), b"late\n");
    calls.set_room(1 << 30);
    let done = reader.finish(false, None);
    for (id, want) in [("t:big.jsonl", big), ("t:late.jsonl", b"late\n".to_vec())] {
        let s = sid(&calls, id).unwrap();
        assert!(data_of(&calls, s) == want, "{id} not read whole");
        assert_eq!(end_of(&calls, s), Some(EndReason::Complete));
    }
    assert_eq!((done.streams, done.cut_files, done.cut_bytes), (2, 0, 0));
}

#[test]
fn finish_with_no_guard_reads_whole_a_snapshot_rewritten_under_a_copy_in_progress() {
    let dir = tempfile::tempdir().unwrap();
    let (rec, calls) = Recorder::for_tests(1);
    let mut rows = sources(&[("t", dir.path(), "*.db", true)]);
    rows.0[0].form = Form::Snapshot;
    let db = dir.path().join("x.db");
    fs::write(&db, vec![b'a'; 4 << 20]).unwrap();
    let reader = native::start(rec, rows);
    let deadline = Instant::now() + Duration::from_secs(10);
    while sid(&calls, "t:x.db").is_none_or(|s| data_of(&calls, s).is_empty()) {
        assert!(Instant::now() < deadline, "the copy never began");
        std::thread::sleep(Duration::from_millis(5));
    }
    calls.set_room(0);
    // The same file, rewritten whole while its copy waits for room, then seen by a run pass.
    let stable: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    fs::write(&db, &stable).unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    let c = calls.clone();
    let room = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        c.set_room(1 << 30);
    });
    let done = common::within(common::HANG, move || reader.finish(true, None));
    room.join().unwrap();
    let o = opens(&calls);
    assert_eq!(o.len(), 2, "a torn copy, then one of the state seen");
    assert_eq!(end_of(&calls, o[0].0), Some(EndReason::SourceChanged));
    assert_eq!(data_of(&calls, o[1].0), stable);
    assert_eq!(end_of(&calls, o[1].0), Some(EndReason::Complete));
    assert!(gaps(&calls).is_empty());
    assert_eq!((done.cut_files, done.cut_bytes), (0, 0));
}

#[test]
fn a_guard_that_runs_out_names_each_unread_file_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let (rec, calls) = Recorder::for_tests(0);
    let sizes = [
        ("a.jsonl", 10u64),
        ("b.jsonl", 70_000),
        ("c.jsonl", 1),
        ("d.db", 9),
    ];
    for (name, n) in sizes {
        fs::write(dir.path().join(name), vec![b'x'; n as usize]).unwrap();
    }
    let mut rows = sources(&[
        ("t", dir.path(), "*.jsonl", true),
        ("t", dir.path(), "*.db", true),
    ]);
    rows.0[1].form = Form::Final;
    let reader = native::start(rec, rows);
    let done = common::within(common::HANG, move || {
        reader.finish(false, Some(Duration::ZERO))
    });
    let mut lost_sum = 0;
    for (name, n) in sizes {
        let s = sid(&calls, &format!("t:{name}")).unwrap();
        let cut = |c: &Call| matches!(c, Call::Gap(g, GapReason::CutAtSeal, _, _, _, Some(l), _) if *g == s && *l == n);
        assert_eq!(gaps(&calls).iter().filter(|c| cut(c)).count(), 1, "{name}");
        assert_eq!(end_of(&calls, s), Some(EndReason::CutAtSeal));
        lost_sum += n;
    }
    assert_eq!(gaps(&calls).len(), sizes.len());
    assert_eq!((done.cut_files, done.cut_bytes), (4, lost_sum));
}

#[test]
fn a_row_that_finds_nothing_is_named_once_five_minutes_after_the_first_request() {
    let root = Path::new("/nonexistent");
    let rows = [
        ("a", root, "*", true),
        ("b", root, "*", true),
        ("c", root, "*", false),
    ];
    let (rec, calls) = Recorder::for_tests(0);
    let missing = || notes(&calls, "native_missing");
    let after = Duration::from_secs(300);
    let mut m = Missing::new(&sources(&rows));
    m.took("b");
    m.check(&rec, 0, after, Mono(1));
    m.check(&rec, 0, after, Mono(1 << 50));
    assert!(missing().is_empty(), "no model request, no note");
    let t0 = 1 << 51;
    m.check(&rec, 2, after, Mono(t0));
    m.check(&rec, 5, after, Mono(t0 + 299_999_999_999));
    assert!(missing().is_empty(), "named before the delay");
    m.check(&rec, 5, after, Mono(t0 + 300_000_000_000));
    m.check(&rec, 9, after, Mono(t0 + 900_000_000_000));
    let named: Vec<_> = missing().iter().map(|n| n["row"].clone()).collect();
    assert_eq!(named, [Value::from("a")]);
}
