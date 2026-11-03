//! PRD-06 section 1, "One queue of entries" (point 5): the uploader of a new live entry also sends
//! older sealed entries whole and sealed; an older entry the server refuses never stops the live
//! entry's chunks; an older entry past `upload_by` is closed and nothing of it is sent; an older
//! entry whose `upload.lock` another uploader holds is not sent while held, and `goalcup upload`
//! answers at once for an older entry the queue holds refused; a live entry with no server still
//! sends an older entry of a run that has one; older entries on a server that never answers never
//! hold the live entry's retry.

mod common;
mod upload_common;

use common::records::ENTRY;
use common::worker::{CREDENTIAL, FakeWorker, Inject, RUN_ID, Route};
use goalcup::cli;
use goalcup::upload::state::{self, Final, State};
use goalcup::upload::{UploadCtx, UploadHandle, manual, spawn};
use goalcup_format::read::part_files;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The older entry, in the live entry's run or in an earlier one.
const OLD: &str = "e00000000000000b1";
const DEADLINE: &str = "2026-11-07T10:00:00Z";

/// A run folder under `home` naming `url`, with `deadline` and the run credential.
fn run(home: &Path, id: &str, url: &str, deadline: &str) -> PathBuf {
    let dir = home.join("runs").join(id);
    fs::create_dir_all(&dir).unwrap();
    let server = json!({"v": 1, "server": url, "run": id, "mode": "test",
        "deadline": deadline, "repo": null, "hosts_file": null, "entries": [],
        "connector": {"id": "c", "name": "c", "source": "builtin"}});
    fs::write(cli::server_json(&dir), server.to_string()).unwrap();
    fs::write(cli::credential(&dir), CREDENTIAL).unwrap();
    dir
}

/// Entry `id` in `run_dir`; see [`upload_common::entry`].
fn entry(run_dir: &Path, id: &str, sealed: bool) -> (PathBuf, Vec<u8>) {
    let dir = run_dir.join(id);
    let bytes = upload_common::entry(&dir, id, sealed);
    (dir, bytes)
}

/// The uploader of the live entry `ENTRY` in `run_dir`, in null mode.
fn live(home: &Path, run_dir: &Path) -> UploadHandle {
    spawn(UploadCtx {
        home: home.into(),
        run_dir: run_dir.into(),
        entry_dir: run_dir.join(ENTRY),
        run: RUN_ID.into(),
        entry: ENTRY.into(),
        recorder: None,
        keep: false,
    })
}

/// Waits up to `common::HANG` for `f`.
fn until(f: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while !f() && start.elapsed() < common::HANG {
        std::thread::sleep(Duration::from_millis(50));
    }
    f()
}

/// Whether the fake saw a call for `OLD`.
fn touched(fake: &FakeWorker) -> bool {
    let beat = fake.heartbeats().iter().any(|b| b["entry_id"] == OLD);
    beat || !fake.stream(OLD).is_empty()
}

#[test]
fn an_older_sealed_entry_goes_whole_and_sealed() {
    let fake = FakeWorker::start();
    let home = tempfile::tempdir().unwrap();
    let run_dir = run(home.path(), RUN_ID, &fake.url(), DEADLINE);
    let (old, all) = entry(&run_dir, OLD, true);
    let (_, mine) = entry(&run_dir, ENTRY, false);
    let _h = live(home.path(), &run_dir);
    let done = || state::read(&old).is_some_and(|j| j.state == State::Done);
    assert!(until(done), "the older entry is done");
    assert_eq!(fake.stream(OLD), all, "the server holds cat rec-*.gcr.zst");
    assert_eq!(state::read(&old).unwrap().final_, Final::Sealed);
    assert!(
        until(|| fake.stream(ENTRY) == mine),
        "the live entry goes too"
    );
}

#[test]
fn a_refused_older_entry_never_stops_the_live_entry() {
    let (fake, refusing) = (FakeWorker::start(), FakeWorker::start());
    for route in [Route::Position, Route::Heartbeat, Route::Chunk] {
        for _ in 0..10 {
            let answer = Some((401, json!({"error": "unauthorized"})));
            let inject = Inject {
                answer,
                ..Inject::default()
            };
            refusing.inject(route, inject);
        }
    }
    let home = tempfile::tempdir().unwrap();
    let earlier = run(home.path(), "r00000000000000b2", &refusing.url(), DEADLINE);
    let (old, _) = entry(&earlier, OLD, true);
    let run_dir = run(home.path(), RUN_ID, &fake.url(), DEADLINE);
    let (_, mine) = entry(&run_dir, ENTRY, false);
    let _h = live(home.path(), &run_dir);
    assert!(
        until(|| fake.stream(ENTRY) == mine),
        "the live entry is stored"
    );
    let refused = || {
        let e = state::read(&old).and_then(|j| j.last_error);
        e.is_some_and(|e| e.status == Some(401))
    };
    assert!(until(refused));
    assert!(refusing.stream(OLD).is_empty());
}

#[test]
fn goalcup_upload_answers_at_once_for_an_older_entry_the_queue_holds_refused() {
    let (fake, refusing) = (FakeWorker::start(), FakeWorker::start());
    for route in [Route::Position, Route::Heartbeat, Route::Chunk] {
        for _ in 0..1000 {
            let answer = Some((404, json!({"error": "not_found"})));
            let inject = Inject {
                answer,
                ..Inject::default()
            };
            refusing.inject(route, inject);
        }
    }
    let home = tempfile::tempdir().unwrap();
    let earlier = run(home.path(), "r00000000000000b2", &refusing.url(), DEADLINE);
    let (old, _) = entry(&earlier, OLD, true);
    let run_dir = run(home.path(), RUN_ID, &fake.url(), DEADLINE);
    entry(&run_dir, ENTRY, false);
    let _h = live(home.path(), &run_dir);
    let refused = || {
        let e = state::read(&old).and_then(|j| j.last_error);
        e.is_some_and(|e| e.status == Some(404))
    };
    assert!(until(refused), "the queue took the older entry");
    // The recorder still runs: `goalcup upload` answers anyway.
    let ids = ("r00000000000000b2", OLD);
    let out = common::within(common::HANG, move || {
        manual::run(&old, &earlier, ids, false)
    });
    assert_eq!(out, manual::Outcome::Refused(404));
}

#[test]
fn an_older_entry_past_upload_by_is_closed_and_not_sent() {
    let fake = FakeWorker::start();
    let home = tempfile::tempdir().unwrap();
    let earlier = run(
        home.path(),
        "r00000000000000b2",
        &fake.url(),
        "2020-01-01T00:00:00Z",
    );
    let (old, _) = entry(&earlier, OLD, true);
    let run_dir = run(home.path(), RUN_ID, &fake.url(), DEADLINE);
    let (_, mine) = entry(&run_dir, ENTRY, false);
    let _h = live(home.path(), &run_dir);
    let closed = || state::read(&old).is_some_and(|j| j.state == State::Closed);
    assert!(until(closed), "upload.json says closed");
    assert!(until(|| fake.stream(ENTRY) == mine));
    assert!(!touched(&fake), "nothing of it is sent");
    assert!(
        part_files(&old.join("rec")).unwrap().len() > 1,
        "nothing pruned"
    );
}

#[test]
fn an_older_entry_another_uploader_holds_is_not_sent() {
    let fake = FakeWorker::start();
    let home = tempfile::tempdir().unwrap();
    let run_dir = run(home.path(), RUN_ID, &fake.url(), DEADLINE);
    let (old, _) = entry(&run_dir, OLD, true);
    let (_, mine) = entry(&run_dir, ENTRY, false);
    let held = state::lock_upload(&old).unwrap();
    let _h = live(home.path(), &run_dir);
    assert!(until(|| fake.stream(ENTRY) == mine));
    // A few more ticks of the queue.
    std::thread::sleep(Duration::from_secs(3));
    assert!(!touched(&fake), "not sent while held");
    drop(held);
}

#[test]
fn a_local_live_entry_still_sends_an_older_one() {
    let fake = FakeWorker::start();
    let home = tempfile::tempdir().unwrap();
    let earlier = run(home.path(), "r00000000000000b2", &fake.url(), DEADLINE);
    let (old, all) = entry(&earlier, OLD, true);
    let run_dir = run(home.path(), RUN_ID, &fake.url(), DEADLINE);
    // The live run is local: `server.json` names no server.
    let local = json!({"v": 1, "server": null, "run": RUN_ID, "mode": "test",
        "deadline": DEADLINE, "repo": null, "hosts_file": null, "entries": [],
        "connector": {"id": "c", "name": "c", "source": "builtin"}});
    fs::write(cli::server_json(&run_dir), local.to_string()).unwrap();
    entry(&run_dir, ENTRY, false);
    let _h = live(home.path(), &run_dir);
    let done = || state::read(&old).is_some_and(|j| j.state == State::Done);
    assert!(until(done), "the older entry is done");
    assert_eq!(fake.stream(OLD), all);
    assert!(fake.stream(ENTRY).is_empty(), "nothing of the local entry");
}

#[test]
fn older_entries_on_a_silent_server_never_hold_the_live_entry() {
    // Accepts every connection and never answers.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", silent.local_addr().unwrap());
    std::thread::spawn(move || {
        let held: Vec<_> = silent.incoming().collect();
        drop(held);
    });
    let fake = FakeWorker::start();
    let answer = Some((503, json!({"error": "unavailable"})));
    let inject = Inject {
        answer,
        ..Inject::default()
    };
    fake.inject(Route::Chunk, inject);
    let home = tempfile::tempdir().unwrap();
    let earlier = run(home.path(), "r00000000000000b2", &url, DEADLINE);
    for id in [
        "e00000000000000c1",
        "e00000000000000c2",
        "e00000000000000c3",
    ] {
        entry(&earlier, id, true);
    }
    let run_dir = run(home.path(), RUN_ID, &fake.url(), DEADLINE);
    let (_, mine) = entry(&run_dir, ENTRY, false);
    let start = Instant::now();
    let _h = live(home.path(), &run_dir);
    assert!(
        until(|| fake.stream(ENTRY) == mine),
        "the live entry is stored"
    );
    // One older call alone may wait 30 s or more; the live retry is due within a few seconds.
    assert!(
        start.elapsed() < Duration::from_secs(25),
        "{:?}",
        start.elapsed()
    );
}
