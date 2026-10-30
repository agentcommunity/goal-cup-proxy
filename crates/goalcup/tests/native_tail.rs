//! PRD-04 acceptance `native_tail.rs` (points 1, 2 and 3): appends, generations, losses named by a
//! GAP, the run-start rule, snapshots and the seal's close, driven with real files.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goalcup::native::fsx::{Dir, Stat};
use goalcup::native::tail::{Stop, Tail};
use goalcup::pipeline::{Call, Calls, Cursors, EndReason, GapReason, Recorder, StreamKind};
use serde_json::Value;

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
        T { dir, rec, calls, tail: Tail::new(since, Cursors::default()) }
    }

    fn p(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }

    /// One walk: settles the named files that exist, then sweeps.
    fn walk(&mut self, names: &[&str], snapshot: bool) {
        for name in names {
            let Ok(st) = stat(self.dir.path(), name) else { continue };
            let id = format!("row:{name}");
            self.tail.settle(&self.rec, id, self.dir.path(), Path::new(name), st, snapshot);
        }
        self.tail.sweep(&self.rec);
    }

    fn read(&mut self, settle: Duration) -> Stop {
        self.tail.read(&self.rec, 1 << 30, settle)
    }
}

fn stat(dir: &Path, name: &str) -> std::io::Result<Stat> {
    Dir::open_root(dir)?.stat_at(OsStr::new(name))
}

fn append(p: &Path, b: &[u8]) {
    OpenOptions::new().create(true).append(true).open(p).unwrap().write_all(b).unwrap();
}

fn opens(c: &[Call]) -> Vec<(u64, StreamKind, Value)> {
    let f = |c: &Call| match c {
        Call::Open(sid, kind, .., meta) => Some((*sid, *kind, meta.clone())),
        _ => None,
    };
    c.iter().filter_map(f).collect()
}

/// A stream's bytes; its DATA offsets must be contiguous from `from`.
fn data(c: &[Call], sid: u64) -> Vec<u8> {
    let from = c.iter().find_map(|c| match c {
        Call::Open(s, .., from, _, _) if *s == sid => Some(*from),
        _ => None,
    });
    let (mut next, mut out) = (from.unwrap(), Vec::new());
    for call in c {
        if let Call::Data(s, _, off, b) = call
            && *s == sid
        {
            assert_eq!(*off, next);
            next += b.len() as u64;
            out.extend_from_slice(b);
        }
    }
    out
}

fn ends(c: &[Call]) -> Vec<(u64, EndReason, u64)> {
    let f = |c: &Call| match c {
        Call::End(sid, _, r, seen, _) => Some((*sid, *r, *seen)),
        _ => None,
    };
    c.iter().filter_map(f).collect()
}

type Gap = (u64, GapReason, Option<u64>, Option<Value>);

fn gaps(c: &[Call]) -> Vec<Gap> {
    let f = |c: &Call| match c {
        Call::Gap(sid, r, _, _, _, lost, tail) => Some((*sid, *r, *lost, tail.clone())),
        _ => None,
    };
    c.iter().filter_map(f).collect()
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
    let c = t.calls.all();
    let o = opens(&c);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].1, StreamKind::Native);
    assert_eq!((&o[0].2["gen"], &o[0].2["from"]), (&Value::from(0), &Value::from(0)));
    assert_eq!(data(&c, o[0].0), [first, b"\":1}\n".to_vec()].concat());
    assert!(ends(&c).is_empty() && gaps(&c).is_empty());
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
    let c = t.calls.all();
    let o = opens(&c);
    let gens: Vec<Value> = o.iter().map(|o| o.2["gen"].clone()).collect();
    assert_eq!(gens, [0, 1, 2]);
    assert_eq!(data(&c, o[1].0), b"hi\n");
    assert_eq!(data(&c, o[2].0), b"replaced\n");
    let e = ends(&c);
    assert_eq!(e, [(o[0].0, EndReason::SourceChanged, 6), (o[1].0, EndReason::SourceChanged, 3)]);
    assert!(gaps(&c).is_empty(), "every byte seen was read");
}

#[test]
fn a_vanished_file_with_unread_bytes_gives_a_gap_then_an_end() {
    let mut t = T::new(UNIX_EPOCH);
    append(&t.p("a"), b"0123456789");
    t.walk(&["a"], false);
    fs::remove_file(t.p("a")).unwrap();
    t.walk(&["a"], false);
    let c = t.calls.all();
    let sid = opens(&c)[0].0;
    let tail = Some(serde_json::json!({"unread_at_least": 10}));
    assert_eq!(gaps(&c), [(sid, GapReason::SourceChanged, None, tail)]);
    assert_eq!(ends(&c), [(sid, EndReason::SourceChanged, 0)]);
}

#[test]
fn a_file_unreadable_to_the_seal_gives_one_open_one_gap_and_one_end() {
    let mut t = T::new(UNIX_EPOCH);
    append(&t.p("a"), b"0123456789");
    fs::set_permissions(t.p("a"), fs::Permissions::from_mode(0o000)).unwrap();
    t.walk(&["a"], false);
    t.read(Duration::ZERO);
    t.read(Duration::ZERO);
    let T { rec, calls, tail, .. } = t;
    let closed = tail.close(&rec, false);
    assert_eq!((closed.streams, closed.cut_files), (1, 0));
    let c = calls.all();
    let notes = c.iter().filter(|c| matches!(c, Call::Event(v) if v["what"] == "native_unreadable"));
    assert_eq!(notes.count(), 1);
    assert_eq!(opens(&c).len(), 1);
    let g = gaps(&c);
    assert_eq!(g.len(), 1);
    assert_eq!((g[0].1, g[0].2), (GapReason::SourceChanged, Some(10)));
    assert_eq!(ends(&c)[..], [(opens(&c)[0].0, EndReason::SourceChanged, 10)]);
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
    let c = t.calls.all();
    assert_eq!(opens(&c).len(), 1);
    assert_eq!(data(&c, opens(&c)[0].0), b"aaaabbbb", "stated limit: read as an append");
    assert!(ends(&c).is_empty());
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
    let o = opens(&t.calls.all());
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
    let c = t.calls.all();
    let o = opens(&c);
    assert_eq!(o.len(), 2);
    assert!(o.iter().all(|o| o.1 == StreamKind::Snapshot && o.2["form"] == "snapshot"));
    assert_eq!(data(&c, o[0].0), b"{\"v\":1}");
    assert_eq!(data(&c, o[1].0), b"{\"v\":22}");
    assert_eq!(ends(&c), [(o[0].0, EndReason::Complete, 7), (o[1].0, EndReason::Complete, 8)]);
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
    let T { rec, calls, tail, .. } = t;
    let closed = tail.close(&rec, true);
    assert_eq!((closed.streams, closed.cut_files, closed.cut_bytes), (3, 2, 30));
    let c = calls.all();
    let lost: Vec<_> = gaps(&c).into_iter().map(|g| (g.1, g.2)).collect();
    assert_eq!(lost, [(GapReason::CutAtSeal, Some(10)), (GapReason::CutAtSeal, Some(20))]);
    let reasons: Vec<_> = ends(&c).into_iter().map(|e| (e.1, e.2)).collect();
    let want = [(EndReason::CutAtSeal, 10), (EndReason::CutAtSeal, 20), (EndReason::Complete, 4)];
    assert_eq!(reasons, want);
}
