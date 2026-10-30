//! PRD-04 acceptance `native_snapshot.rs` for database sets (points 1, 2 and 3; lessons L-F5 and
//! L-F6): a set is copied whole with its `-wal` and `-shm` during the run once it changed and its
//! interval passed, a copy that moves under it is copied once more, a seal copy says whether the
//! harness still ran, and a seal cut by its guard cuts every file of the set.

mod common;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::recorder::{append, data_of, end_of, gap_rows, gaps, notes, opens};
use goalcup::connector::{Form, NativeSource, NativeSources};
use goalcup::native;
use goalcup::native::fsx::Dir;
use goalcup::native::snapshot::{Interval, Sets};
use goalcup::native::tail::{Stop, Tail};
use goalcup::pipeline::{Calls, Cursors, EndReason, GapReason, Recorder};
use serde_json::{Value, json};

const MS: Duration = Duration::from_millis(1);

/// One walk of every file in `dir`, handed to `sets` as the reader's pass does for row `t`, with
/// the run start `since`.
fn walk(sets: &mut Sets, dir: &Path, since: SystemTime) {
    let (root, tail) = (
        Dir::open_root(dir).unwrap(),
        Tail::new(since, Cursors::default()),
    );
    for name in root.names().unwrap() {
        let st = root.stat_at(&name).unwrap();
        sets.see("t", dir, Path::new(&name), st, tail.taken(&st));
    }
    sets.walked();
}

/// Each copy as its `set` and its streams (sid, meta), in order of first OPEN.
fn copies(c: &Calls) -> Vec<(Value, Vec<(u64, Value)>)> {
    let mut out: Vec<(Value, Vec<(u64, Value)>)> = Vec::new();
    for (sid, _, meta) in opens(c) {
        match out.iter_mut().find(|(set, _)| *set == meta["set"]) {
            Some((_, files)) => files.push((sid, meta)),
            None => out.push((meta["set"].clone(), vec![(sid, meta)])),
        }
    }
    out
}

/// Every stream of `copy` ended `reason` holding as many bytes as its meta `size`.
fn whole(c: &Calls, copy: &[(u64, Value)], reason: EndReason) {
    for (sid, meta) in copy {
        assert_eq!(end_of(c, *sid), Some(reason), "{meta}");
        assert_eq!(Value::from(data_of(c, *sid).len()), meta["size"], "{meta}");
    }
}

#[test]
fn a_changed_set_is_copied_whole_once_its_interval_passes_and_the_interval_grows_with_size() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for (name, b) in [("x.db", "main"), ("x.db-wal", "wal"), ("x.db-shm", "shm")] {
        fs::write(d.join(name), b).unwrap();
    }
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let iv = Interval {
        base: 300 * MS,
        per_2mib: 1000 * MS,
    };
    let mut sets = Sets::new(iv);
    walk(&mut sets, d, UNIX_EPOCH);
    sets.read(&rec, u64::MAX);
    assert!(opens(&calls).is_empty(), "copied before its interval");
    std::thread::sleep(350 * MS);
    sets.read(&rec, u64::MAX);
    let t0 = Instant::now();
    let first = copies(&calls);
    assert_eq!(
        (first.len(), first[0].1.len()),
        (1, 3),
        "one set, a stream per file"
    );
    for (sid, meta) in &first[0].1 {
        assert!(
            meta["form"] == "final" && meta["harness_exited"] == false,
            "{meta}"
        );
        let name = meta["path"].as_str().unwrap().strip_prefix("t:").unwrap();
        assert_eq!(data_of(&calls, *sid), fs::read(d.join(name)).unwrap());
    }
    whole(&calls, &first[0].1, EndReason::Complete);
    std::thread::sleep(350 * MS);
    walk(&mut sets, d, UNIX_EPOCH);
    sets.read(&rec, u64::MAX);
    assert_eq!(copies(&calls).len(), 1, "an unchanged set was copied again");
    // Changed and past 4 MiB: due 2 s after the last copy, not after the 300 ms base.
    append(&d.join("x.db-wal"), &vec![b'w'; 4 << 20]);
    walk(&mut sets, d, UNIX_EPOCH);
    sets.read(&rec, u64::MAX);
    assert_eq!(
        copies(&calls).len(),
        1,
        "a large set was copied before its longer interval"
    );
    std::thread::sleep((2100 * MS).saturating_sub(t0.elapsed()));
    sets.read(&rec, u64::MAX);
    let all = copies(&calls);
    assert_eq!(all.len(), 2);
    whole(&calls, &all[1].1, EndReason::Complete);
}

#[test]
fn a_set_written_during_its_copy_is_copied_once_more_then_kept_as_unstable() {
    let dir = tempfile::tempdir().unwrap();
    let (d, wal) = (dir.path(), dir.path().join("x.db-wal"));
    fs::write(d.join("x.db"), b"0123456789").unwrap();
    fs::write(&wal, b"0123456789").unwrap();
    let (rec, calls) = Recorder::for_tests(0);
    let mut sets = Sets::new(Interval::default());
    walk(&mut sets, d, UNIX_EPOCH);
    assert!(
        matches!(sets.read(&rec, u64::MAX), Stop::Room),
        "no room, nothing read"
    );
    append(&wal, b"more");
    calls.set_room(1 << 30);
    // The first copy reads the 20 bytes it saw; the slice ends as its retry begins.
    assert!(matches!(sets.read(&rec, 20), Stop::Slice));
    append(&wal, b"again");
    sets.read(&rec, u64::MAX);
    let c = copies(&calls);
    assert_eq!(c.len(), 2, "one copy and one retry, both kept");
    assert_ne!(c[0].0, c[1].0);
    for (sid, _) in &c[0].1 {
        assert_eq!(end_of(&calls, *sid), Some(EndReason::SourceChanged));
    }
    whole(&calls, &c[1].1, EndReason::Complete);
    let unstable: Vec<Value> = notes(&calls, "snapshot_unstable")
        .iter()
        .map(|n| n["path"].clone())
        .collect();
    assert_eq!(unstable, ["t:x.db-wal"]);
    assert!(gaps(&calls).is_empty());
}

#[test]
fn a_database_older_than_the_run_start_is_copied_with_its_new_wal() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::write(d.join("x.db"), b"old").unwrap();
    fs::write(d.join("y.db"), b"old, alone").unwrap();
    let m = fs::metadata(d.join("y.db")).unwrap();
    // File times come from the kernel's coarse clock, a tick behind `now` (as in native_walk.rs).
    let since = UNIX_EPOCH + Duration::from_secs(m.mtime().max(m.ctime()) as u64 + 1);
    while SystemTime::now() < since + 50 * MS {
        std::thread::sleep(20 * MS);
    }
    fs::write(d.join("x.db-wal"), b"new").unwrap();
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let mut sets = Sets::new(Interval::default());
    walk(&mut sets, d, since);
    sets.read(&rec, u64::MAX);
    let mut paths: Vec<Value> = opens(&calls)
        .into_iter()
        .map(|o| o.2["path"].clone())
        .collect();
    paths.sort_by_key(|p| p.to_string());
    assert_eq!(paths, ["t:x.db", "t:x.db-wal"]);
}

#[test]
fn a_seal_copy_says_whether_the_harness_still_ran_and_is_counted() {
    for exited in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("x.db"), b"main").unwrap();
        fs::write(dir.path().join("x.db-wal"), b"wal").unwrap();
        let rows = NativeSources(vec![NativeSource {
            name: "t".into(),
            root: dir.path().into(),
            pattern: "x.db*".into(),
            form: Form::Final,
            since: SystemTime::now() - Duration::from_secs(60),
            warn_missing: true,
        }]);
        let (rec, calls) = Recorder::for_tests(1 << 30);
        let done = native::start(rec, rows).finish(exited, None);
        let c = copies(&calls);
        assert_eq!((c.len(), c[0].1.len(), done.snapshots), (1, 2, 2));
        assert!(
            c[0].1
                .iter()
                .all(|(_, m)| m["harness_exited"] == exited && m["form"] == "final")
        );
        whole(&calls, &c[0].1, EndReason::Complete);
        let live = notes(&calls, "final_snapshot_live");
        assert_eq!(live.len(), usize::from(!exited));
        assert!(live.iter().all(|n| n["set"] == c[0].0));
    }
}

#[test]
fn a_seal_cut_part_way_cuts_every_file_of_the_set_and_keeps_the_periodic_copy_whole() {
    let dir = tempfile::tempdir().unwrap();
    let (d, wal) = (dir.path(), dir.path().join("x.db-wal"));
    fs::write(d.join("x.db"), vec![b'm'; 5000]).unwrap();
    fs::write(&wal, vec![b'w'; 300]).unwrap();
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let mut sets = Sets::new(Interval::default());
    walk(&mut sets, d, UNIX_EPOCH);
    sets.read(&rec, u64::MAX);
    append(&wal, b"more");
    walk(&mut sets, d, UNIX_EPOCH);
    sets.seal(true);
    // The seal copy reads x.db whole and 100 bytes of the wal; then its guard runs out.
    assert!(matches!(sets.read(&rec, 5100), Stop::Slice));
    let closed = sets.close(&rec);
    let c = copies(&calls);
    assert_eq!(c.len(), 2);
    whole(&calls, &c[0].1, EndReason::Complete);
    // Every file ends cut_at_seal, but only the wal has unread bytes: one GAP, its exact loss.
    let cut = |(sid, _): &(u64, Value)| end_of(&calls, *sid) == Some(EndReason::CutAtSeal);
    assert!(c[1].1.iter().all(cut));
    let wal_sid = c[1].1.iter().find(|(_, m)| m["path"] == "t:x.db-wal");
    let gap = (wal_sid.unwrap().0, GapReason::CutAtSeal, Some(204), None);
    assert_eq!(gap_rows(&calls), [gap]);
    assert_eq!((closed.cut_files, closed.cut_bytes, closed.streams), (1, 204, 2));
}

/// A set of x.db and x.db-wal, 10 bytes each, whose copy has begun, with `damage` done to the wal
/// before the copy reads it. With `retry`, an append first moves the copy and `damage` comes as
/// its retry begins.
fn damaged(retry: bool, damage: impl Fn(&Path)) -> Calls {
    let dir = tempfile::tempdir().unwrap();
    let (d, wal) = (dir.path(), dir.path().join("x.db-wal"));
    fs::write(d.join("x.db"), b"0123456789").unwrap();
    fs::write(&wal, b"0123456789").unwrap();
    let (rec, calls) = Recorder::for_tests(0);
    let mut sets = Sets::new(Interval::default());
    walk(&mut sets, d, UNIX_EPOCH);
    sets.read(&rec, u64::MAX);
    calls.set_room(1 << 30);
    if retry {
        append(&wal, b"more");
        // The first copy reads its 20 bytes; the slice ends as its retry begins.
        sets.read(&rec, 20);
    }
    damage(&wal);
    sets.read(&rec, u64::MAX);
    calls
}

#[test]
fn a_set_file_lost_during_a_copy_is_named_by_a_gap_and_the_copy_still_ends() {
    let unread = |n: u64| Some(json!({"unread_at_least": n}));
    let wal_of = |c: &[(u64, Value)]| c.iter().find(|(_, m)| m["path"] == "t:x.db-wal").map(|f| f.0);
    // Gone before the first copy reads it: exact loss, and the retry holds the main file only.
    let calls = damaged(false, |w| fs::remove_file(w).unwrap());
    let c = copies(&calls);
    assert_eq!((c.len(), c[1].1.len()), (2, 1));
    let gap = (wal_of(&c[0].1).unwrap(), GapReason::SourceChanged, Some(10), unread(10));
    assert_eq!(gap_rows(&calls), [gap]);
    whole(&calls, &c[1].1, EndReason::Complete);
    // Shrunk to 4 bytes: a lower bound, and the retry holds the 4 bytes that are there.
    let calls = damaged(false, |w| {
        fs::OpenOptions::new().write(true).open(w).unwrap().set_len(4).unwrap();
    });
    let c = copies(&calls);
    let gap = (wal_of(&c[0].1).unwrap(), GapReason::SourceChanged, None, unread(6));
    assert_eq!((c.len(), gap_rows(&calls)), (2, vec![gap]));
    whole(&calls, &c[1].1, EndReason::Complete);
    // Gone as the retry begins: the retry ends complete, names its loss and the path.
    let calls = damaged(true, |w| fs::remove_file(w).unwrap());
    let c = copies(&calls);
    let gap = (wal_of(&c[1].1).unwrap(), GapReason::SourceChanged, Some(14), unread(14));
    assert_eq!(gap_rows(&calls), [gap]);
    for (sid, _) in &c[1].1 {
        assert_eq!(end_of(&calls, *sid), Some(EndReason::Complete));
    }
    assert_eq!(notes(&calls, "snapshot_unstable")[0]["path"], "t:x.db-wal");
}
