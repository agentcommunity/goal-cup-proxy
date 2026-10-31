//! PRD-06 "Automated tests", `upload_manual.rs` (section 9, `goalcup upload`): it gives up on 401,
//! 404, 410 and `stuck` with that reason, and on a 401 past `upload_by` says the run is closed; it
//! refuses while a recorder holds `rec/lock` and never holds `rec/lock` after it returns; the final
//! heartbeat carries the counts of `counts.jsonl`; a sealed entry with bytes left ends sealed with
//! the server holding `cat rec-*.gcr.zst` and the parts pruned; an entry the position already
//! calls sealed is not sent again.

mod common;

use common::records::{BOOT_ID, ENTRY, at};
use common::worker::{CREDENTIAL, FakeWorker, Inject, RUN_ID, Route};
use goalcup::cli;
use goalcup::pipeline::Status;
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
    let entry = run.join(ENTRY);
    let rec = entry.join("rec");
    fs::create_dir_all(&rec).unwrap();
    let opts = PartOptions {
        window_log: 20,
        rotate_at: 4096,
        rotate_after_ns: u64::MAX,
    };
    let mut w = PartWriter::open(&rec, opts, ENTRY, BOOT_ID, at(0)).unwrap();
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
        w.link(at(n)).unwrap();
        // Not after the last LINK, so the last part holds index lines.
        if n < 6 {
            w.rotate_if_due(at(n)).unwrap();
        }
    }
    w.seal(at(7)).unwrap();
    // The boot's totals as `counts.jsonl` keeps them; the part files hold no COUNT event.
    let count = json!({"seq": 1, "count": {"ev": "count", "boot": 0, "http_post": 42}, "spans": 0});
    fs::write(entry.join("counts.jsonl"), format!("{count}\n")).unwrap();
    let seal = SealStatus {
        reason: SealReason::HarnessExit,
        workspace_head: None,
    };
    let status = Status {
        mode: Mode::Sealed,
        seal: Some(seal),
        ..Status::default()
    };
    fs::write(
        cli::status_json(&entry),
        serde_json::to_vec(&status).unwrap(),
    )
    .unwrap();
    File::create(rec.join("lock")).unwrap();
    let server = json!({"v": 1, "server": fake.url(), "run": RUN_ID, "mode": "test",
        "deadline": deadline, "repo": null, "hosts_file": null, "entries": [],
        "connector": {"id": "c", "name": "c", "source": "builtin"}});
    fs::write(cli::server_json(run), server.to_string()).unwrap();
    fs::write(cli::credential(run), CREDENTIAL).unwrap();
    let all = part_files(&rec)
        .unwrap()
        .iter()
        .flat_map(|p| fs::read(p).unwrap())
        .collect();
    (entry, all)
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
    assert_eq!(upload(dir.path(), &entry, false), Outcome::StillRecording);
    assert!(fake.chunks().is_empty() && fake.heartbeats().is_empty());
}
