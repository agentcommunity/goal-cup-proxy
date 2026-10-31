//! PRD-06 section 1, "One queue of entries" (point 5): the uploader of a new live entry also sends
//! older sealed entries whole and sealed; an older entry the server refuses never stops the live
//! entry's chunks; an older entry past `upload_by` is closed and nothing of it is sent; an older
//! entry whose `upload.lock` another uploader holds is not sent while held.

mod common;

use common::records::{BOOT_ID, ENTRY, at};
use common::worker::{CREDENTIAL, FakeWorker, Inject, RUN_ID, Route};
use goalcup::cli;
use goalcup::pipeline::Status;
use goalcup::pipeline::status::{Mode, SealStatus};
use goalcup::upload::state::{self, Final, State};
use goalcup::upload::{UploadCtx, UploadHandle, spawn};
use goalcup::watchdog::SealReason;
use goalcup_format::read::{part_files, read_index};
use goalcup_format::{PartOptions, PartWriter, Record, hex};
use serde_json::json;
use std::fs::{self, File};
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

/// Entry `id` of several parts in `run_dir`: sealed with its seal in `status.json`, or left open at
/// a synced LINK, as a live entry in null mode. Returns the folder and the bytes of each part up
/// to its last index line, joined.
fn entry(run_dir: &Path, id: &str, sealed: bool) -> (PathBuf, Vec<u8>) {
    let dir = run_dir.join(id);
    let rec = dir.join("rec");
    fs::create_dir_all(&rec).unwrap();
    let opts = PartOptions {
        window_log: 20,
        rotate_at: 4096,
        rotate_after_ns: u64::MAX,
    };
    let mut w = PartWriter::open(&rec, opts, id, BOOT_ID, at(0)).unwrap();
    for n in 1..=5u64 {
        // Bytes that do not compress, so the parts rotate.
        let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let noise: Vec<u8> = (0..400)
            .flat_map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x.to_le_bytes()
            })
            .collect();
        let json = format!(r#"{{"ev":"x","n":"{}"}}"#, hex(&noise)).into_bytes();
        w.write(&Record::Event {
            t: at(n).mono_ns,
            json,
        })
        .unwrap();
        w.link(at(n)).unwrap();
        if n < 5 {
            w.rotate_if_due(at(n)).unwrap();
        }
    }
    if sealed {
        w.seal(at(6)).unwrap();
        let status = Status {
            mode: Mode::Sealed,
            seal: Some(SealStatus {
                reason: SealReason::HarnessExit,
                workspace_head: None,
            }),
            ..Status::default()
        };
        fs::write(cli::status_json(&dir), serde_json::to_vec(&status).unwrap()).unwrap();
    } else {
        w.sync().unwrap();
        std::mem::forget(w);
    }
    File::create(rec.join("lock")).unwrap();
    let bytes = part_files(&rec)
        .unwrap()
        .iter()
        .flat_map(|p| {
            let idx = p.with_extension("").with_extension("idx");
            let end = read_index(&idx).unwrap().last().unwrap().c_end as usize;
            fs::read(p).unwrap()[..end].to_vec()
        })
        .collect();
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
