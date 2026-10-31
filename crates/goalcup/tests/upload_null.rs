//! PRD-06 "Automated tests", `upload_null.rs` (ledger R7, points 1 and 5): with no live writer the
//! uploader's threads send what is on disk up to the last whole index line within the part file
//! and nothing past it, beat `failed` at that line, and say its raw end in `upload.json`. No
//! `fdatasync` counting hook (ledger L6): the bytes the fake Worker holds are the check.

mod common;

use common::records::{BOOT_ID, ENTRY, at};
use common::worker::{CREDENTIAL, FakeWorker, RUN_ID};
use goalcup::cli;
use goalcup::pipeline::Status;
use goalcup::pipeline::status::Mode;
use goalcup::upload::{UploadCtx, spawn, state};
use goalcup_format::read::read_index;
use goalcup_format::{PartOptions, PartWriter, Record, hex, index_name, part_name};
use serde_json::json;
use std::time::{Duration, Instant};
use std::{fs, mem, thread};

#[test]
fn null_mode_sends_up_to_the_last_whole_line_and_beats_failed() {
    let fake = FakeWorker::start();
    let run = tempfile::tempdir().unwrap();
    let entry = run.path().join(ENTRY);
    let rec = entry.join("rec");
    fs::create_dir_all(&rec).unwrap();
    let opts = PartOptions {
        window_log: 20,
        rotate_at: u64::MAX,
        rotate_after_ns: u64::MAX,
    };
    let mut w = PartWriter::open(&rec, opts, ENTRY, BOOT_ID, at(0)).unwrap();
    for n in 1..=6u64 {
        let json = format!(r#"{{"n":"{}"}}"#, hex(&n.to_le_bytes()).repeat(400)).into_bytes();
        let t = at(n).mono_ns;
        w.write(&Record::Event { t, json }).unwrap();
        w.link(at(n)).unwrap();
    }
    w.sync().unwrap();
    // The writer is gone without its drop, as after a crash, and the last line is torn off.
    mem::forget(w);
    let lines = read_index(&rec.join(index_name(0))).unwrap();
    let (last, whole) = (lines[lines.len() - 1], lines[lines.len() - 2]);
    let part = rec.join(part_name(0));
    let file = fs::OpenOptions::new().write(true).open(&part).unwrap();
    file.set_len(last.c_end - 1).unwrap();
    let want = fs::read(&part).unwrap()[..whole.c_end as usize].to_vec();
    let status = Status {
        mode: Mode::Null,
        ..Status::default()
    };
    fs::write(
        cli::status_json(&entry),
        serde_json::to_vec(&status).unwrap(),
    )
    .unwrap();
    let server = json!({"v": 1, "server": fake.url(), "run": RUN_ID, "mode": "test",
        "deadline": "2026-11-07T10:00:00Z", "repo": null, "hosts_file": null, "entries": [],
        "connector": {"id": "c", "name": "c", "source": "builtin"}});
    fs::write(cli::server_json(run.path()), server.to_string()).unwrap();
    fs::write(cli::credential(run.path()), format!("{CREDENTIAL}\n")).unwrap();

    let _h = spawn(UploadCtx {
        home: run.path().into(),
        run_dir: run.path().into(),
        entry_dir: entry.clone(),
        run: RUN_ID.into(),
        entry: ENTRY.into(),
        recorder: None,
        keep: true,
    });
    let beat = || {
        fake.heartbeats()
            .into_iter()
            .find(|b| b["seq"] == whole.seq)
    };
    let raw = || state::read(&entry).map(|j| j.durable_raw);
    let start = Instant::now();
    while (fake.stream(ENTRY) != want || beat().is_none() || raw() != Some(whole.raw_end))
        && start.elapsed() < Duration::from_secs(15)
    {
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        fake.stream(ENTRY),
        want,
        "the bytes up to the last whole line, none past it"
    );
    assert_eq!(
        beat().map(|b| b["recording"].clone()),
        Some(json!("failed"))
    );
    assert_eq!(raw(), Some(whole.raw_end));
    // Section 9: the tail window is the deadline plus a day.
    let by = state::read(&entry).unwrap().upload_by;
    assert_eq!(by.as_deref(), Some("2026-11-08T10:00:00Z"));
}
