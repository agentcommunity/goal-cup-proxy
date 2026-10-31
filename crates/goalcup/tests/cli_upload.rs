//! `goalcup upload` (PRD-10 section 1, PRD-06 section 9) through the binary, against PRD-14's fake
//! Worker and a sealed entry with bytes left: the tail is sent and the exit is 0; a server that
//! refuses leaves a non-zero exit with every record file still on this computer and nothing
//! sent; no run is the `no_run_yet` refusal; `GOALCUP_SERVER=off` sends nothing. Facts only.

mod common;

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Command, Output};

use common::home::TempHome;
use common::records::{BOOT_ID, ENTRY, at};
use common::worker::{CREDENTIAL, FakeWorker, Inject, RUN_ID, Route};
use goalcup::cli;
use goalcup::pipeline::Status;
use goalcup::pipeline::status::{Mode, SealStatus};
use goalcup::upload::state::{self, Final};
use goalcup::watchdog::SealReason;
use goalcup_format::read::part_files;
use goalcup_format::{PartOptions, PartWriter, Record, hex};
use serde_json::json;

/// A sealed entry of several record files under `home`; returns its folder and the bytes of
/// `cat rec-*.gcr.zst`.
fn sealed(home: &TempHome, fake: &FakeWorker) -> (PathBuf, Vec<u8>) {
    let run = home.goalcup_home().join("runs").join(RUN_ID);
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
        if n < 6 {
            w.rotate_if_due(at(n)).unwrap();
        }
    }
    w.seal(at(7)).unwrap();
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
    let server = json!({"v": 1, "server": fake.url(), "run": RUN_ID, "mode": "practice",
        "deadline": "2026-11-07T10:00:00Z", "repo": null, "hosts_file": null,
        "entries": [{"id": ENTRY, "harness_version": null, "resumed": false}],
        "connector": {"id": "c", "name": "c", "source": "builtin"}});
    fs::write(cli::server_json(&run), server.to_string()).unwrap();
    fs::write(cli::credential(&run), CREDENTIAL).unwrap();
    let all = part_files(&rec)
        .unwrap()
        .iter()
        .flat_map(|p| fs::read(p).unwrap())
        .collect();
    (entry, all)
}

fn upload(home: &TempHome, server: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_goalcup"));
    home.apply(&mut cmd).arg("upload");
    cmd.env_remove("GOALCUP_SERVER");
    if let Some(s) = server {
        cmd.env("GOALCUP_SERVER", s);
    }
    common::within(common::HANG, move || cmd.output().unwrap())
}

#[test]
fn a_sealed_part_with_bytes_left_is_sent_and_exits_0() {
    let (home, fake) = (TempHome::new(), FakeWorker::start());
    let (entry, all) = sealed(&home, &fake);
    let out = upload(&home, Some(&fake.url()));
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(fake.stream(ENTRY), all);
    assert_eq!(state::read(&entry).unwrap().final_, Final::Sealed);
}

#[test]
fn a_server_that_refuses_leaves_a_non_zero_exit_and_every_record_file() {
    let (home, fake) = (TempHome::new(), FakeWorker::start());
    let (entry, _) = sealed(&home, &fake);
    let before = part_files(&entry.join("rec")).unwrap().len();
    let answer = Some((410, json!({"error": "gone"})));
    fake.inject(
        Route::Position,
        Inject {
            answer,
            ..Inject::default()
        },
    );
    let out = upload(&home, Some(&fake.url()));
    assert_eq!(out.status.code(), Some(1));
    assert!(fake.chunks().is_empty());
    assert_eq!(part_files(&entry.join("rec")).unwrap().len(), before);
    assert_ne!(state::read(&entry).unwrap().final_, Final::Sealed);
}

#[test]
fn no_run_exits_1_and_creates_nothing() {
    let home = TempHome::new();
    let out = upload(&home, None);
    assert_eq!(out.status.code(), Some(1));
    assert!(!home.goalcup_home().join("runs").exists());
}

#[test]
fn server_off_sends_nothing() {
    let (home, fake) = (TempHome::new(), FakeWorker::start());
    let (entry, _) = sealed(&home, &fake);
    let before = part_files(&entry.join("rec")).unwrap().len();
    let out = upload(&home, Some("off"));
    assert_eq!(out.status.code(), Some(0));
    assert!(fake.chunks().is_empty() && fake.heartbeats().is_empty());
    assert_eq!(part_files(&entry.join("rec")).unwrap().len(), before);
}
