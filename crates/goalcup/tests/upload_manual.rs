//! PRD-06 "Automated tests", `upload_manual.rs` (section 9, `goalcup upload`): it gives up on 401,
//! 404, 410 and `stuck` with that reason, and on a 401 past `upload_by` says the run is closed; it
//! refuses while a recorder holds `rec/lock` and never holds `rec/lock` after it returns; the final
//! heartbeat carries the counts of `counts.jsonl`; a dead entry (no seal anywhere, no recorder) is
//! closed with the `recovered` seal and sent whole, and is left alone while a recorder holds
//! `rec/lock` or its watchdog lives (U8); a sealed entry with bytes left ends sealed with the server holding
//! `cat rec-*.gcr.zst` and the parts pruned; an entry the position already calls sealed is not
//! sent again, and is pruned even with `upload.json` lost.

mod common;

use bytes::Bytes;
use common::recorder::{entry_dir, real, wait_settled};
use common::records::{BOOT_ID, ENTRY, at};
use common::worker::{CREDENTIAL, FakeWorker, Inject, RUN_ID, Route};
use goalcup::cli;
use goalcup::pipeline::{EndReason, OpenFlags, Status, StreamKind};
use goalcup::pipeline::status::{Mode, SealStatus};
use goalcup::upload::manual::{Outcome, run};
use goalcup::upload::state::{self, Final};
use goalcup::watchdog::SealReason;
use goalcup_format::read::part_files;
use goalcup_format::{PartOptions, PartWriter, Record, hex};
use serde_json::json;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

/// A sealed entry of several parts in `run`, with its `counts.jsonl`, `status.json`, `rec/lock`,
/// and the run's `server.json` naming `fake` with `deadline`. Returns the entry folder and the
/// bytes of `cat rec-*.gcr.zst`.
fn sealed(run: &Path, fake: &FakeWorker, deadline: &str) -> (PathBuf, Vec<u8>) {
    entry(run, fake, deadline, true)
}

/// As [`sealed`], or with `seal` false a dead entry: no seal, the writer dropped after its last
/// LINK, a `status.json` that is not sealed and an unheld `rec/lock`.
fn entry(run: &Path, fake: &FakeWorker, deadline: &str, seal: bool) -> (PathBuf, Vec<u8>) {
    let entry = run.join(ENTRY);
    let rec = entry.join("rec");
    fs::create_dir_all(&rec).unwrap();
    let opts = PartOptions {
        window_log: 20,
        rotate_at: 4096,
        rotate_after_ns: u64::MAX,
    };
    let mut w = PartWriter::open(&rec, opts, ENTRY, BOOT_ID, at(0)).unwrap();
    let mut last = 0;
    for n in 1..=6u64 {
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
        last = w.link(at(n)).unwrap().seq;
        // Not after the last LINK, so the last part holds index lines.
        if n < 6 {
            w.rotate_if_due(at(n)).unwrap();
        }
    }
    // A dead boot's line is at its last LINK, which its recovery frame takes.
    let seq = match seal {
        true => {
            w.seal(at(7)).unwrap();
            1
        }
        false => {
            drop(w);
            last
        }
    };
    // The boot's totals as `counts.jsonl` keeps them; the part files hold no COUNT event.
    let count =
        json!({"seq": seq, "count": {"ev": "count", "boot": 0, "http_post": 42}, "spans": 0});
    fs::write(entry.join("counts.jsonl"), format!("{count}\n")).unwrap();
    let status = match seal {
        true => Status {
            mode: Mode::Sealed,
            seal: Some(SealStatus {
                reason: SealReason::HarnessExit,
                workspace_head: None,
            }),
            ..Status::default()
        },
        false => Status::default(),
    };
    fs::write(
        cli::status_json(&entry),
        serde_json::to_vec(&status).unwrap(),
    )
    .unwrap();
    File::create(rec.join("lock")).unwrap();
    serve(run, fake, deadline);
    let all = part_files(&rec)
        .unwrap()
        .iter()
        .flat_map(|p| fs::read(p).unwrap())
        .collect();
    (entry, all)
}

/// The run's `server.json` naming `fake` with `deadline`, and its credential.
fn serve(run: &Path, fake: &FakeWorker, deadline: &str) {
    let server = json!({"v": 1, "server": fake.url(), "run": RUN_ID, "mode": "test",
        "deadline": deadline, "repo": null, "hosts_file": null, "entries": [],
        "connector": {"id": "c", "name": "c", "source": "builtin"}});
    fs::write(cli::server_json(run), server.to_string()).unwrap();
    fs::write(cli::credential(run), CREDENTIAL).unwrap();
}

const DEADLINE: &str = "2026-11-07T10:00:00Z";

fn upload(run: &Path, entry: &Path, keep: bool) -> Outcome {
    let (run, entry) = (run.to_path_buf(), entry.to_path_buf());
    common::within(common::HANG, move || run_on(&entry, &run, keep))
}

fn run_on(entry: &Path, run_dir: &Path, keep: bool) -> Outcome {
    run(entry, run_dir, (RUN_ID, ENTRY), keep)
}

#[test]
fn a_sealed_entry_goes_whole_is_pruned_and_is_not_sent_again() {
    let fake = FakeWorker::start();
    let dir = tempfile::tempdir().unwrap();
    let (entry, all) = sealed(dir.path(), &fake, DEADLINE);
    assert!(part_files(&entry.join("rec")).unwrap().len() >= 3);
    assert_eq!(upload(dir.path(), &entry, false), Outcome::Done);
    assert_eq!(
        fake.stream(ENTRY),
        all,
        "the server holds cat rec-*.gcr.zst"
    );
    assert_eq!(state::read(&entry).unwrap().final_, Final::Sealed);
    assert!(part_files(&entry.join("rec")).unwrap().is_empty(), "pruned");
    assert!(entry.join("counts.jsonl").exists());
    let beats = fake.heartbeats();
    let last = beats.last().unwrap();
    assert_eq!(last["final"], true);
    assert_eq!(last["counts"]["http_post"], 42, "counts from counts.jsonl");
    assert_eq!(last["model_requests"], 42);
    // `rec/lock` is free once it returns.
    File::open(entry.join("rec/lock"))
        .unwrap()
        .try_lock()
        .unwrap();

    let (chunks, heartbeats) = (fake.chunks().len(), beats.len());
    assert_eq!(upload(dir.path(), &entry, false), Outcome::AlreadyThere);
    assert_eq!(fake.chunks().len(), chunks, "no PUT");
    assert_eq!(fake.heartbeats().len(), heartbeats);
}

#[test]
fn it_gives_up_on_401_404_410_and_says_a_closed_run_after_upload_by() {
    for (status, deadline, want) in [
        (401, DEADLINE, Outcome::Refused(401)),
        (404, DEADLINE, Outcome::Refused(404)),
        (410, DEADLINE, Outcome::Refused(410)),
        (
            401,
            "2020-01-01T00:00:00Z",
            Outcome::Closed {
                upload_by: "2020-01-02T00:00:00Z".into(),
            },
        ),
    ] {
        let fake = FakeWorker::start();
        let dir = tempfile::tempdir().unwrap();
        let (entry, _) = sealed(dir.path(), &fake, deadline);
        let answer = Some((status, json!({"error": "refused"})));
        fake.inject(
            Route::Position,
            Inject {
                answer,
                ..Inject::default()
            },
        );
        assert_eq!(upload(dir.path(), &entry, false), want);
        assert!(fake.chunks().is_empty());
        assert!(
            part_files(&entry.join("rec")).unwrap().len() > 1,
            "nothing pruned"
        );
    }
}

#[test]
fn it_gives_up_when_the_copies_disagree() {
    let fake = FakeWorker::start();
    let dir = tempfile::tempdir().unwrap();
    let (entry, _) = sealed(dir.path(), &fake, DEADLINE);
    let far = json!({"offset": 1u64 << 40, "next_seq": 9, "head": "ab".repeat(32)});
    for _ in 0..2 {
        let answer = Some((200, far.clone()));
        fake.inject(
            Route::Position,
            Inject {
                answer,
                ..Inject::default()
            },
        );
    }
    assert_eq!(upload(dir.path(), &entry, false), Outcome::Stuck);
    assert!(fake.chunks().is_empty());
}

#[test]
fn it_refuses_while_a_recorder_holds_rec_lock() {
    let fake = FakeWorker::start();
    let dir = tempfile::tempdir().unwrap();
    let (entry, _) = sealed(dir.path(), &fake, DEADLINE);
    let held = File::open(entry.join("rec/lock")).unwrap();
    held.try_lock().unwrap();
    // A live recorder's upload thread holds `upload.lock` for its whole life.
    let uploading = state::lock_upload(&entry).unwrap();
    assert_eq!(upload(dir.path(), &entry, false), Outcome::StillRecording);
    drop(uploading);
    assert!(fake.chunks().is_empty() && fake.heartbeats().is_empty());
}

fn parts(entry: &Path) -> Vec<Vec<u8>> {
    let rec = part_files(&entry.join("rec")).unwrap();
    rec.iter().map(|p| fs::read(p).unwrap()).collect()
}

/// U8: a dead entry is closed with the `recovered` seal under `upload.lock`, its final heartbeat
/// goes first and says `gaps` with the counts of `counts.jsonl`, and it is sent whole.
#[test]
fn a_dead_entry_is_closed_recovered_and_sent_whole() {
    let fake = FakeWorker::start();
    let dir = tempfile::tempdir().unwrap();
    let (entry, _) = entry(dir.path(), &fake, DEADLINE, false);
    assert!(part_files(&entry.join("rec")).unwrap().len() >= 3);
    assert_eq!(upload(dir.path(), &entry, true), Outcome::Done);
    let status: Status =
        serde_json::from_slice(&fs::read(cli::status_json(&entry)).unwrap()).unwrap();
    assert_eq!(status.mode, Mode::Sealed);
    assert_eq!(status.seal.unwrap().reason, SealReason::Recovered);
    let beats = fake.heartbeats();
    assert_eq!(beats[0]["final"], true);
    assert_eq!(beats[0]["recording"], "gaps");
    assert_eq!(
        beats[0]["counts"]["http_post"], 42,
        "counts from counts.jsonl"
    );
    assert_eq!(beats[0]["model_requests"], 42);
    assert_eq!(state::read(&entry).unwrap().final_, Final::Sealed);
    assert_eq!(
        fake.stream(ENTRY),
        parts(&entry).concat(),
        "cat rec-*.gcr.zst"
    );
}

/// A dead entry while a recorder holds `rec/lock`: nothing is sent and no recovery frame written.
#[test]
fn a_dead_entry_a_recorder_holds_is_left_alone() {
    let fake = FakeWorker::start();
    let dir = tempfile::tempdir().unwrap();
    let (entry, _) = entry(dir.path(), &fake, DEADLINE, false);
    let before = parts(&entry);
    let held = File::open(entry.join("rec/lock")).unwrap();
    held.try_lock().unwrap();
    assert_eq!(upload(dir.path(), &entry, false), Outcome::StillRecording);
    assert!(fake.chunks().is_empty() && fake.heartbeats().is_empty());
    assert_eq!(parts(&entry), before, "no part file changed");
}

/// A dead entry whose watchdog lives: `rec/lock` is unheld between boots, and the next boot would
/// record into it. Nothing is closed or sent, and no part file changes.
#[test]
fn a_dead_entry_whose_watchdog_lives_is_left_alone() {
    let fake = FakeWorker::start();
    let dir = tempfile::tempdir().unwrap();
    let (entry, _) = entry(dir.path(), &fake, DEADLINE, false);
    let pid = std::process::id();
    let start_id = cli::proc_start_id(pid).unwrap();
    fs::write(entry.join("rec/watchdog.pid"), format!("{pid} {start_id}\n")).unwrap();
    assert!(cli::watchdog_alive(&entry));
    let (before, status) = (parts(&entry), fs::read(cli::status_json(&entry)).unwrap());
    assert_eq!(upload(dir.path(), &entry, false), Outcome::StillRecording);
    assert!(fake.chunks().is_empty() && fake.heartbeats().is_empty());
    assert_eq!(parts(&entry), before, "no part file changed");
    assert_eq!(fs::read(cli::status_json(&entry)).unwrap(), status);
}

/// U8 on what a killed recorder leaves: a real recorder's entry folder copied while its frame is
/// open past the last LINK, unsealed. It is closed `recovered` and sent whole, and the final
/// heartbeat carries every request it wrote.
#[test]
fn a_killed_recorders_entry_is_closed_and_sent_whole() {
    const CALLS: u64 = 3;
    let live = entry_dir();
    let rec = real(live.path());
    for _ in 0..CALLS {
        let flags = OpenFlags::POST | OpenFlags::DECRYPTED;
        let mut h = rec.open(StreamKind::HttpReq, 0, 0, 0, flags, json!({}));
        h.data(rec.now(), Bytes::from_static(b"{\"model\":\"m\"}"));
        h.end(rec.now(), EndReason::Complete, None);
    }
    wait_settled(&rec, |c| c.http_post >= CALLS);
    let fake = FakeWorker::start();
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join(ENTRY);
    copy(live.path(), &entry);
    serve(dir.path(), &fake, DEADLINE);
    assert_eq!(upload(dir.path(), &entry, true), Outcome::Done);
    let status: Status =
        serde_json::from_slice(&fs::read(cli::status_json(&entry)).unwrap()).unwrap();
    assert_eq!(status.seal.unwrap().reason, SealReason::Recovered);
    let beats = fake.heartbeats();
    assert_eq!(beats[0]["final"], true);
    assert_eq!(beats[0]["counts"]["http_post"], CALLS);
    assert_eq!(beats[0]["model_requests"], CALLS);
    assert_eq!(
        fake.stream(ENTRY),
        parts(&entry).concat(),
        "cat rec-*.gcr.zst"
    );
}

/// Copies the files of `from` into `to`, skipping temp files a live writer may rename away.
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        } else if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &to.join(&name));
        } else {
            fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

/// `upload.json` is a cache: with it lost, an entry the server holds sealed is confirmed from the
/// position matched to the seal LINK and pruned, and a second run finds nothing left.
#[test]
fn an_entry_already_on_the_server_is_pruned_without_upload_json() {
    let fake = FakeWorker::start();
    let dir = tempfile::tempdir().unwrap();
    let (entry, _) = sealed(dir.path(), &fake, DEADLINE);
    assert_eq!(upload(dir.path(), &entry, true), Outcome::Done);
    assert!(!part_files(&entry.join("rec")).unwrap().is_empty(), "kept");
    fs::remove_file(entry.join(state::FILE)).unwrap();
    let chunks = fake.chunks().len();
    assert_eq!(upload(dir.path(), &entry, false), Outcome::AlreadyThere);
    assert!(part_files(&entry.join("rec")).unwrap().is_empty(), "pruned");
    assert_eq!(fake.chunks().len(), chunks, "no PUT");
    let j = state::read(&entry).unwrap();
    assert_eq!(j.final_, Final::Sealed);
    assert!(j.pruned.parts > 0 && j.confirmed.is_some());
    assert_eq!(upload(dir.path(), &entry, false), Outcome::AlreadyThere);
    // Every part pruned and `upload.json` lost again: the server's sealed position decides.
    fs::remove_file(entry.join(state::FILE)).unwrap();
    assert_eq!(upload(dir.path(), &entry, false), Outcome::AlreadyThere);
    assert_eq!(fake.chunks().len(), chunks, "no PUT");
}
