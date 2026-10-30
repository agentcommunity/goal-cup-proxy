//! PRD-04 acceptance `native_tail.rs` (points 1, 2 and 3): appends, generations, losses named by a
//! GAP, the run-start rule, snapshots and the seal's close, driven with real files.

mod common;

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::recorder::{data_of, ends, events, gaps, opens, pick};
use goalcup::native::fsx::{Dir, Kind, Stat};
use goalcup::native::tail::{Stop, Tail};
use goalcup::pipeline::{Call, Calls, Cursors, EndReason, GapReason, Recorder, StreamKind};
use serde_json::{Value, json};

struct T {
    dir: tempfile::TempDir,
    rec: Recorder,
    calls: Calls,
    tail: Tail,
}

impl T {
    fn new(since: SystemTime) -> T {
        let (rec, calls) = Recorder::for_tests(1 << 30);
        let dir = tempfile::tempdir().unwrap();
        T {
            dir,
            rec,
            calls,
            tail: Tail::new(since, Cursors::default()),
        }
    }

    fn p(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }

    /// One walk: settles the named files that exist, then sweeps.
    fn walk(&mut self, names: &[&str], snapshot: bool) {
        for name in names {
            let Ok(st) = stat(self.dir.path(), name) else {
                continue;
            };
            let id = format!("row:{name}");
            self.tail.settle(
                &self.rec,
                id,
                self.dir.path(),
                Path::new(name),
                st,
                snapshot,
            );
        }
        self.tail.sweep(&self.rec, &[]);
    }

    fn read(&mut self, settle: Duration) -> Stop {
        self.tail.read(&self.rec, 1 << 30, settle)
    }
}

fn stat(dir: &Path, name: &str) -> std::io::Result<Stat> {
    Dir::open_root(dir)?.stat_at(OsStr::new(name))
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

#[test]
fn live_eof_is_no_end_and_partial_lines_and_binary_bytes_are_kept() {
    let mut t = T::new(UNIX_EPOCH);
    let first: Vec<u8> = (0..=255u8).chain(*b"{\"partial").collect();
    append(&t.p("a.jsonl"), &first);
    t.walk(&["a.jsonl"], false);
    assert!(matches!(t.read(Duration::ZERO), Stop::Done));
    append(&t.p("a.jsonl"), b"\":1}\n");
    t.walk(&["a.jsonl"], false);
    assert!(matches!(t.read(Duration::ZERO), Stop::Done));
    let c = &t.calls;
    let o = opens(c);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].1, StreamKind::Native);
    assert_eq!(
        (&o[0].2["gen"], &o[0].2["from"]),
        (&Value::from(0), &Value::from(0))
    );
    assert_eq!(data_of(c, o[0].0), [first, b"\":1}\n".to_vec()].concat());
    assert!(ends(c).is_empty() && gaps(c).is_empty());
}

#[test]
fn truncate_and_rename_over_start_a_new_generation_from_zero() {
    let mut t = T::new(UNIX_EPOCH);
    append(&t.p("a"), b"hello\n");
    t.walk(&["a"], false);
    t.read(Duration::ZERO);
    File::create(t.p("a")).unwrap().write_all(b"hi\n").unwrap();
    t.walk(&["a"], false);
    t.read(Duration::ZERO);
    fs::write(t.p("tmp"), b"replaced\n").unwrap();
    fs::rename(t.p("tmp"), t.p("a")).unwrap();
    t.walk(&["a"], false);
    t.read(Duration::ZERO);
    let c = &t.calls;
    let o = opens(c);
    let gens: Vec<Value> = o.iter().map(|o| o.2["gen"].clone()).collect();
    assert_eq!(gens, [0, 1, 2]);
    assert_eq!(data_of(c, o[1].0), b"hi\n");
    assert_eq!(data_of(c, o[2].0), b"replaced\n");
    let e = ends(c);
    assert_eq!(
        e,
        [
            (o[0].0, EndReason::SourceChanged, 6, None),
            (o[1].0, EndReason::SourceChanged, 3, None)
        ]
    );
    assert!(gaps(c).is_empty(), "every byte seen was read");
}

#[test]
fn a_vanished_file_with_unread_bytes_gives_a_gap_then_an_end() {
    let mut t = T::new(UNIX_EPOCH);
    append(&t.p("a"), b"0123456789");
    t.walk(&["a"], false);
    fs::remove_file(t.p("a")).unwrap();
    t.walk(&["a"], false);
    let c = &t.calls;
    let sid = opens(c)[0].0;
    let tail = Some(json!({"unread_at_least": 10}));
    let lost = pick(c, |g| match g {
        Call::Gap(sid, r, .., lost, t) => Some((sid, r, lost, t)),
        _ => None,
    });
    assert_eq!(lost, [(sid, GapReason::SourceChanged, None, tail)]);
    assert_eq!(ends(c), [(sid, EndReason::SourceChanged, 0, None)]);
}

#[test]
fn a_file_under_a_folder_the_walk_could_not_list_is_not_gone() {
    let mut t = T::new(UNIX_EPOCH);
    append(&t.p("a"), b"0123456789");
    t.walk(&["a"], false);
    let unlisted = [t.dir.path().to_path_buf()];
    t.tail.sweep(&t.rec, &unlisted);
    assert!(ends(&t.calls).is_empty() && gaps(&t.calls).is_empty());
    t.read(Duration::ZERO);
    assert_eq!(data_of(&t.calls, opens(&t.calls)[0].0), b"0123456789");
}

#[test]
fn a_file_unreadable_to_the_seal_gives_one_open_one_gap_and_one_end() {
    unreadable_to_the_seal(false);
    unreadable_to_the_seal(true);
}

fn unreadable_to_the_seal(snapshot: bool) {
    // A 300-byte name fails every open with ENAMETOOLONG, whoever runs the test.
    let mut t = T::new(UNIX_EPOCH);
    let name = "n".repeat(300);
    let st = Stat {
        kind: Kind::File,
        dev: 1,
        ino: 1,
        size: 10,
        mtime_ns: 0,
        ctime_ns: 0,
        birth_ns: None,
    };
    for _ in 0..2 {
        let (root, rel) = (t.dir.path(), Path::new(&name));
        t.tail
            .settle(&t.rec, "row:long".into(), root, rel, st, snapshot);
        t.tail.sweep(&t.rec, &[]);
        t.read(Duration::ZERO);
    }
    let T {
        rec, calls, tail, ..
    } = t;
    let closed = tail.close(&rec, false);
    let streams = u64::from(!snapshot);
    assert_eq!((closed.streams, closed.cut_files), (streams, 0));
    let c = &calls;
    let notes = events(c);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["what"], "native_unreadable");
    assert_eq!(notes[0]["errno"], libc::ENAMETOOLONG);
    let (sid, kind, _) = opens(c)[0].clone();
    let snap = kind == StreamKind::Snapshot;
    assert_eq!((opens(c).len(), snap), (1, snapshot));
    let tail = Some(json!({"unread_at_least": 10}));
    let lost = pick(c, |g| match g {
        Call::Gap(sid, r, .., lost, t) => Some((sid, r, lost, t)),
        _ => None,
    });
    assert_eq!(lost, [(sid, GapReason::SourceChanged, Some(10), tail)]);
    assert_eq!(ends(c), [(sid, EndReason::SourceChanged, 10, None)]);
}

#[test]
fn a_shrink_still_past_the_offset_is_a_truncation_with_a_gap() {
    let mut t = T::new(UNIX_EPOCH);
    append(&t.p("a"), &[7; 100]);
    t.walk(&["a"], false);
    t.tail.read(&t.rec, 30, Duration::ZERO);
    let f = OpenOptions::new().write(true).open(t.p("a")).unwrap();
    f.set_len(50).unwrap();
    t.walk(&["a"], false);
    t.read(Duration::ZERO);
    let c = &t.calls;
    let o = opens(c);
    let gens: Vec<Value> = o.iter().map(|o| o.2["gen"].clone()).collect();
    assert_eq!(gens, [0, 1]);
    let tail = Some(json!({"unread_at_least": 70}));
    let lost = pick(c, |g| match g {
        Call::Gap(sid, r, .., lost, t) => Some((sid, r, lost, t)),
        _ => None,
    });
    assert_eq!(lost, [(o[0].0, GapReason::SourceChanged, None, tail)]);
    assert_eq!(ends(c), [(o[0].0, EndReason::SourceChanged, 30, None)]);
    assert_eq!(data_of(c, o[1].0), [7u8; 50]);
}

#[test]
fn a_rewrite_in_place_that_grows_past_the_offset_reads_as_an_append() {
    let mut t = T::new(UNIX_EPOCH);
    append(&t.p("a"), b"aaaa");
    t.walk(&["a"], false);
    t.read(Duration::ZERO);
    let mut f = OpenOptions::new().write(true).open(t.p("a")).unwrap();
    f.write_all(b"bbbbbbbb").unwrap();
    t.walk(&["a"], false);
    t.read(Duration::ZERO);
    let c = &t.calls;
    assert_eq!(opens(c).len(), 1);
    assert_eq!(
        data_of(c, opens(c)[0].0),
        b"aaaabbbb",
        "stated limit: read as an append"
    );
    assert!(ends(c).is_empty());
}

#[test]
fn run_start_rule_takes_by_max_of_mtime_and_ctime_in_whole_seconds() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("old"), b"x").unwrap();
    let old = stat(dir.path(), "old").unwrap();
    let secs = old.mtime_ns.max(old.ctime_ns) / 1_000_000_000 + 1;
    let since = UNIX_EPOCH + Duration::from_secs(secs as u64);
    while SystemTime::now() < since {
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut t = T::new(since);
    t.dir = dir;
    fs::write(t.p("backdated"), b"y").unwrap();
    let f = File::options().write(true).open(t.p("backdated")).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(1)).unwrap();
    t.walk(&["old", "backdated"], false);
    let o = opens(&t.calls);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].2["path"], "row:backdated");
}

#[test]
fn a_snapshot_is_copied_once_settled_and_again_after_a_change() {
    let mut t = T::new(UNIX_EPOCH);
    fs::write(t.p("m.json"), b"{\"v\":1}").unwrap();
    t.walk(&["m.json"], true);
    t.read(Duration::from_secs(3600));
    assert!(t.calls.all().is_empty(), "not settled yet");
    t.read(Duration::ZERO);
    t.read(Duration::ZERO);
    fs::write(t.p("m.json"), b"{\"v\":22}").unwrap();
    t.walk(&["m.json"], true);
    t.read(Duration::ZERO);
    let c = &t.calls;
    let o = opens(c);
    assert_eq!(o.len(), 2);
    assert!(
        o.iter()
            .all(|o| o.1 == StreamKind::Snapshot && o.2["form"] == "snapshot")
    );
    assert_eq!(data_of(c, o[0].0), b"{\"v\":1}");
    assert_eq!(data_of(c, o[1].0), b"{\"v\":22}");
    assert_eq!(
        ends(c),
        [
            (o[0].0, EndReason::Complete, 7, None),
            (o[1].0, EndReason::Complete, 8, None)
        ]
    );
}

#[test]
fn close_with_the_guard_out_cuts_with_the_exact_loss() {
    let mut t = T::new(UNIX_EPOCH);
    append(&t.p("done"), b"read");
    t.walk(&["done"], false);
    t.read(Duration::ZERO);
    append(&t.p("a"), &[1; 10]);
    append(&t.p("b"), &[2; 20]);
    t.walk(&["done", "a", "b"], false);
    let T {
        rec, calls, tail, ..
    } = t;
    let closed = tail.close(&rec, true);
    assert_eq!(
        (closed.streams, closed.cut_files, closed.cut_bytes),
        (3, 2, 30)
    );
    let c = &calls;
    let cut = pick(c, |g| match g {
        Call::Gap(_, r, .., lost, _) => Some((r, lost)),
        _ => None,
    });
    assert_eq!(
        cut,
        [
            (GapReason::CutAtSeal, Some(10)),
            (GapReason::CutAtSeal, Some(20))
        ]
    );
    let reasons: Vec<_> = ends(c).into_iter().map(|e| (e.1, e.2)).collect();
    let want = [
        (EndReason::CutAtSeal, 10),
        (EndReason::CutAtSeal, 20),
        (EndReason::Complete, 4),
    ];
    assert_eq!(reasons, want);
}

#[test]
fn close_with_the_guard_out_cuts_a_snapshot_not_copied_yet() {
    let mut t = T::new(UNIX_EPOCH);
    fs::write(t.p("m.json"), b"{\"v\":1}").unwrap();
    t.walk(&["m.json"], true);
    t.read(Duration::from_secs(3600));
    let T {
        rec, calls, tail, ..
    } = t;
    let closed = tail.close(&rec, true);
    assert_eq!(
        (closed.streams, closed.cut_files, closed.cut_bytes),
        (0, 1, 7)
    );
    let c = &calls;
    let o = opens(c);
    assert_eq!((o.len(), o[0].1), (1, StreamKind::Snapshot));
    let lost = pick(c, |g| match g {
        Call::Gap(sid, r, .., lost, t) => Some((sid, r, lost, t)),
        _ => None,
    });
    assert_eq!(lost, [(o[0].0, GapReason::CutAtSeal, Some(7), None)]);
    assert_eq!(ends(c), [(o[0].0, EndReason::CutAtSeal, 7, None)]);
}

#[test]
fn a_snapshot_gone_with_bytes_unread_gives_its_gap_and_end() {
    // Slice 10: the copy stops part way. Slice 0: the settled copy never starts.
    for (slice, read) in [(10, 10), (0, 0)] {
        let mut t = T::new(UNIX_EPOCH);
        fs::write(t.p("m.json"), [7; 100]).unwrap();
        t.walk(&["m.json"], true);
        t.tail.read(&t.rec, slice, Duration::ZERO);
        fs::remove_file(t.p("m.json")).unwrap();
        t.read(Duration::ZERO);
        t.walk(&["m.json"], true);
        t.read(Duration::ZERO);
        let T {
            rec, calls, tail, ..
        } = t;
        tail.close(&rec, false);
        let c = &calls;
        let o = opens(c);
        assert_eq!((o.len(), o[0].1), (1, StreamKind::Snapshot));
        let tail = Some(json!({"unread_at_least": 100 - read}));
        let gap = (o[0].0, GapReason::SourceChanged, Some(100 - read), tail);
        let lost = pick(c, |g| match g {
            Call::Gap(sid, r, .., lost, t) => Some((sid, r, lost, t)),
            _ => None,
        });
        assert_eq!(lost, [gap]);
        // END `seen` is the offset: the bytes read plus the exact GAP.
        assert_eq!(ends(c), [(o[0].0, EndReason::SourceChanged, 100, None)]);
    }
}

/// A copy of 100 bytes stops at 10, then the file is rewritten in place to `size` bytes and seen.
fn torn_at_10(size: usize) -> T {
    let mut t = T::new(UNIX_EPOCH);
    fs::write(t.p("m.json"), [7; 100]).unwrap();
    t.walk(&["m.json"], true);
    t.tail.read(&t.rec, 10, Duration::ZERO);
    fs::write(t.p("m.json"), vec![8; size]).unwrap();
    t.walk(&["m.json"], true);
    t
}

/// The torn copy ends with no GAP; the last state gets its own stream, its exact loss and END.
fn torn_then_last(c: &Calls, size: u64, gap: GapReason, end: EndReason) {
    let o = opens(c);
    assert_eq!(o.len(), 2);
    assert!(o.iter().all(|o| o.1 == StreamKind::Snapshot));
    assert_eq!(o[1].2["size"], size);
    assert_eq!(data_of(c, o[0].0), [7; 10]);
    let tail = (gap == GapReason::SourceChanged).then(|| json!({"unread_at_least": size}));
    let lost = pick(c, |g| match g {
        Call::Gap(sid, r, .., lost, t) => Some((sid, r, lost, t)),
        _ => None,
    });
    assert_eq!(lost, [(o[1].0, gap, Some(size), tail)]);
    let e = ends(c);
    assert_eq!(
        e,
        [
            (o[0].0, EndReason::SourceChanged, 10, None),
            (o[1].0, end, size, None)
        ]
    );
}

#[test]
fn a_snapshot_rewritten_during_a_copy_then_gone_names_the_last_state_lost() {
    // Rewritten larger or smaller; gone seen by a read first or by the walk alone.
    for (size, read_first) in [(200, false), (200, true), (50, false), (50, true)] {
        let mut t = torn_at_10(size);
        fs::remove_file(t.p("m.json")).unwrap();
        if read_first {
            t.read(Duration::ZERO);
        }
        t.walk(&["m.json"], true);
        t.read(Duration::ZERO);
        let T {
            rec, calls, tail, ..
        } = t;
        let closed = tail.close(&rec, true);
        assert_eq!((closed.cut_files, closed.cut_bytes), (0, 0));
        let (size, gap) = (size as u64, GapReason::SourceChanged);
        torn_then_last(&calls, size, gap, EndReason::SourceChanged);
    }
}

#[test]
fn a_snapshot_rewritten_during_a_copy_and_cut_at_the_seal_counts_the_last_state() {
    // Rewritten larger or smaller; still there at the seal or gone before it unseen.
    for (size, gone) in [(200, false), (200, true), (50, false), (50, true)] {
        let t = torn_at_10(size);
        if gone {
            fs::remove_file(t.p("m.json")).unwrap();
        }
        let T {
            rec, calls, tail, ..
        } = t;
        let closed = tail.close(&rec, true);
        let size = size as u64;
        assert_eq!((closed.cut_files, closed.cut_bytes), (1, size));
        torn_then_last(&calls, size, GapReason::CutAtSeal, EndReason::CutAtSeal);
    }
}
