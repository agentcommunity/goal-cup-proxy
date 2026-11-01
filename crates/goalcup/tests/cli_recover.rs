//! `goalcup recover` (PRD-10 section 3.2): each check gives one refusal and starts nothing, a port
//! held by another program and a missing run certificate key refuse with exit 1, and the harness
//! is never signalled. Fake `status.json` and `server.json` files and shell stand-ins for the
//! harness and the other goalcup processes. Facts only; the full recover is acceptance 11 on the pad.
#![cfg(target_os = "linux")]

mod cli_common;
mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use cli_common::{Stand, part, put, server_json, with};
use common::home::TempHome;
use goalcup::cli::proc_start_id;
use goalcup::connector::{HostList, builtin};
use goalcup::pipeline::status::Mode;
use goalcup::proxy::ca::RunCa;
use serde_json::{Value, json};

/// A shell that notes any signal in `marker` and otherwise sleeps; ready once its trap is set.
fn stand_in(dir: &Path, marker: &Path) -> Stand {
    let ready = dir.join("ready");
    let script = format!(
        "trap ': > {}' TERM INT HUP USR1 USR2 QUIT; touch '{}'; while :; do sleep 0.1; done",
        marker.display(),
        ready.display()
    );
    let child = Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < until, "stand-in did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    Stand(child)
}

struct World {
    home: TempHome,
    scratch: tempfile::TempDir,
    harness: Stand,
}

impl World {
    /// A part whose recorder and watchdog are gone and whose harness stand-in runs.
    fn new() -> World {
        let scratch = tempfile::tempdir().unwrap();
        let marker = scratch.path().join("signalled");
        let harness = stand_in(scratch.path(), &marker);
        let w = World {
            home: TempHome::new(),
            scratch,
            harness,
        };
        let mut server = server_json();
        server["connector"] = json!({"id": "codex-cli", "name": "Codex", "source": "builtin"});
        server["entries"][0]["workspace"] = json!(w.scratch.path());
        server["entries"][0]["segment"] = json!(0);
        server["entries"][0]["hosts"] =
            serde_json::to_value(HostList::from_file(builtin::HOSTS.as_bytes(), "m").unwrap())
                .unwrap();
        put(w.dir().parent().unwrap(), "server.json", &server);
        w.status(Mode::Recording, json!({}));
        w
    }

    fn dir(&self) -> PathBuf {
        part(&self.home.goalcup_home())
    }

    fn status(&self, mode: Mode, p: Value) {
        let pid = self.harness.0.id();
        let start_id = proc_start_id(pid).unwrap();
        let harness = json!({
            "pid": pid, "started": "2026-11-07T10:00:00Z", "start_id": start_id, "exit": null,
        });
        let st = with(mode, json!({"harness": harness}));
        put(&self.dir(), "status.json", &cli_common::patch(&st, p));
    }

    fn goalcup(&self, inside: bool) -> Output {
        let mut cmd = Command::new(common::bin());
        self.home
            .apply(&mut cmd)
            .arg("recover")
            .env("GOALCUP_SERVER", "off");
        cmd.env_remove("GOALCUP_TIME_LEFT_FILE")
            .stdin(Stdio::null());
        if inside {
            cmd.env("GOALCUP_TIME_LEFT_FILE", "/nowhere/upload.json");
        }
        cmd.output().unwrap()
    }

    /// Nothing was started and the harness was never signalled.
    fn untouched(&self) {
        assert!(!self.dir().join("rec/watchdog.pid").exists());
        assert!(!self.scratch.path().join("signalled").exists());
    }
}

fn refused(out: &Output, code: i32) {
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(code), "{err}");
}

#[test]
fn inside_the_harness_nothing_starts() {
    let w = World::new();
    refused(&w.goalcup(true), 1);
    w.untouched();
}

#[test]
fn sealed_run_over_and_live_watchdog_refuse() {
    let w = World::new();
    w.status(Mode::Sealed, json!({}));
    refused(&w.goalcup(false), 1);
    w.untouched();

    let w = World::new();
    let mut server = server_json();
    server["deadline"] = json!("2020-01-01T00:00:00Z");
    server["connector"] = json!({"id": "codex-cli", "name": "Codex", "source": "builtin"});
    put(w.dir().parent().unwrap(), "server.json", &server);
    refused(&w.goalcup(false), 1);
    w.untouched();

    let w = World::new();
    let wd_dir = tempfile::tempdir().unwrap();
    let wd = stand_in(wd_dir.path(), &wd_dir.path().join("wd-hit"));
    let rec = w.dir().join("rec");
    std::fs::create_dir_all(&rec).unwrap();
    let id = proc_start_id(wd.0.id()).unwrap();
    std::fs::write(rec.join("watchdog.pid"), format!("{} {id}\n", wd.0.id())).unwrap();
    refused(&w.goalcup(false), 1);
    assert!(!w.scratch.path().join("signalled").exists());
    assert!(!wd_dir.path().join("wd-hit").exists());
}

#[test]
fn a_live_recorder_needs_nothing() {
    let w = World::new();
    let rec = Command::new("sleep").arg("30").spawn().unwrap();
    let rec = Stand(rec);
    let id = proc_start_id(rec.0.id()).unwrap();
    w.status(Mode::Recording, json!({"pid": rec.0.id(), "start_id": id}));
    let out = w.goalcup(false);
    assert_eq!(out.status.code(), Some(0));
    w.untouched();
}

#[test]
fn a_gone_harness_refuses() {
    let w = World::new();
    w.status(
        Mode::Recording,
        json!({"harness": {"pid": 0, "start_id": 1}}),
    );
    refused(&w.goalcup(false), 1);
    w.untouched();
}

#[test]
fn missing_key_and_taken_port_refuse_and_start_nothing() {
    let w = World::new();
    refused(&w.goalcup(false), 1);
    w.untouched();

    // A key and certificate that load, and a port another program listens on.
    let hosts = HostList::from_file(builtin::HOSTS.as_bytes(), "model-hosts.toml").unwrap();
    let ca = RunCa::mint(&hosts.ca_subtrees()).unwrap();
    ca.write_files(&w.dir()).unwrap();
    let key = w.dir().join("rec").join("run-ca.key");
    std::fs::create_dir_all(key.parent().unwrap()).unwrap();
    std::fs::write(&key, ca.export().key).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (_other, addr) = common::ports::held();
    w.status(Mode::Recording, json!({"port": addr.port()}));
    refused(&w.goalcup(false), 1);
    w.untouched();
}
