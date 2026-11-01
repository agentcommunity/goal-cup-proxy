//! PRD-06 point 5 in live mode: with a real recorder (`Recorder::real_for_tests`) the upload
//! thread's rounds are bounded by `Recorder::durable()`. The server ends up holding the part files
//! up to the durable point and nothing past it, `upload.json` confirms that point, and the beat
//! says `ok` from the live recorder's health. `upload_null.rs` and the `recorder: None` cases
//! cover the on-disk bound only.

mod common;

use bytes::Bytes;
use common::recorder::{entry_dir, real, wait_settled};
use common::worker::{CREDENTIAL, FakeWorker, RUN_ID};
use goalcup::cli;
use goalcup::pipeline::{EndReason, OpenFlags, StreamKind};
use goalcup::upload::{UploadCtx, spawn, state};
use goalcup_format::read::{part_files, read_index};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// The entry id `Recorder::real_for_tests` writes into its part headers.
const ENTRY: &str = "e00000000000000a1";

/// Model requests fed through the recorder.
const CALLS: u64 = 3;

#[test]
fn live_rounds_follow_the_durable_point() {
    let fake = FakeWorker::start();
    let run = tempfile::tempdir().unwrap();
    let server = json!({"v": 1, "server": fake.url(), "run": RUN_ID, "mode": "test",
        "deadline": "2026-11-07T10:00:00Z", "repo": null, "hosts_file": null, "entries": [],
        "connector": {"id": "c", "name": "c", "source": "builtin"}});
    fs::write(cli::server_json(run.path()), server.to_string()).unwrap();
    fs::write(cli::credential(run.path()), CREDENTIAL).unwrap();
    let entry = entry_dir();
    let rec = real(entry.path());

    // A few model requests, durable before the uploader starts: its first round is due at once,
    // the next one only after the round period, so the bound it reads is this point or later.
    let flags = OpenFlags::POST | OpenFlags::DECRYPTED;
    for _ in 0..CALLS {
        let mut h = rec.open(StreamKind::HttpReq, 0, 0, 0, flags, json!({}));
        h.data(
            rec.now(),
            Bytes::from_static(b"{\"model\":\"m\",\"input\":\"hi\"}"),
        );
        h.end(rec.now(), EndReason::Complete, None);
    }
    wait_settled(&rec, |c| c.http_post >= CALLS);
    let settled = rec.durable().borrow().point.clone().unwrap().line.raw_end;

    let _h = spawn(UploadCtx {
        home: run.path().into(),
        run_dir: run.path().into(),
        entry_dir: entry.path().into(),
        run: RUN_ID.into(),
        entry: ENTRY.into(),
        recorder: Some(rec.clone()),
        keep: true,
    });

    // Bounded by HANG, never a speed check: `upload.json` confirms the durable point it measured.
    let dir: PathBuf = entry.path().into();
    let durable = common::within(common::HANG, move || {
        loop {
            if let Some(j) = state::read(&dir)
                && j.durable_raw > 0
                && j.confirmed.as_ref().map(|c| c.raw) == Some(j.durable_raw)
            {
                return j.durable_raw;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    assert!(
        durable >= settled,
        "the calls are inside the confirmed point"
    );

    // Each part file up to its last index line at or below the durable point: what the server
    // holds, and nothing past it.
    let want: Vec<u8> = part_files(&entry.path().join("rec"))
        .unwrap()
        .iter()
        .flat_map(|p| {
            let idx = p.with_extension("").with_extension("idx");
            let lines = read_index(&idx).unwrap();
            let end = lines
                .iter()
                .rfind(|l| l.raw_end <= durable)
                .map_or(0, |l| l.c_end);
            fs::read(p).unwrap()[..end as usize].to_vec()
        })
        .collect();
    assert!(!want.is_empty());
    assert_eq!(
        fake.stream(ENTRY),
        want,
        "the parts up to the durable point"
    );
    assert!(
        fake.chunks().iter().all(|c| c.offset < durable),
        "no chunk starts at or past the durable point"
    );

    // A beat from the live recorder's health says `ok`, with no more requests than were fed.
    let beats = fake.heartbeats();
    let ok = beats.iter().find(|b| b["recording"] == "ok");
    let ok = ok.unwrap_or_else(|| panic!("a beat says ok: {beats:?}"));
    let posts = ok["counts"]["http_post"].as_u64().unwrap();
    assert!(posts <= CALLS, "counts.http_post {posts} of {CALLS} fed");
    assert!(ok["model_requests"].as_u64().unwrap() <= CALLS);
}
