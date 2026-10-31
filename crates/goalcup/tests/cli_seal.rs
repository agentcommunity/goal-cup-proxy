//! `goalcup seal` (PRD-10 section 1): the signal goes to the watchdog and not the recorder, the
//! recorder alone gets SIGTERM when no watchdog lives, nothing is signalled inside the harness,
//! for a sealed part or when nobody can be asked. The watchdog and recorder are shell stand-ins.
//! Facts only; no wording.

mod cli_common;
mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use cli_common::{part, put, server_json, with};
use common::home::TempHome;
use goalcup::cli::proc_start_id;
use goalcup::pipeline::status::Mode;
use serde_json::json;

/// A stand-in process, killed when dropped.
struct Stand(Child);

impl Drop for Stand {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Stand {
    fn start_id(&self) -> u64 {
        proc_start_id(self.0.id()).expect("a start id")
    }
}

/// Runs `trap '<action>' <signals>` and waits; ready once its trap is set.
fn stand_in(dir: &Path, name: &str, signals: &str, action: &str) -> Stand {
    let ready = dir.join(format!("{name}.ready"));
    let script = format!(
        "trap '{action}' {signals}; touch '{}'; while :; do sleep 0.1; done",
        ready.display()
    );
    let child = Command::new("sh").arg("-c").arg(script).spawn().unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < until, "{name} did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    Stand(child)
}

/// One part of a run in a home: its `status.json` names `recorder` and the sealed form waits in
/// `sealed.json` for a stand-in to move into place.
struct World {
    home: TempHome,
    scratch: tempfile::TempDir,
}

impl World {
    fn new(mode: Mode) -> World {
        let w = World {
            home: TempHome::new(),
            scratch: tempfile::tempdir().unwrap(),
        };
        let dir = part(&w.home.goalcup_home());
        put(dir.parent().unwrap(), "server.json", &server_json());
        put(&dir, "status.json", &with(mode, json!({})));
        let sealed = with(
            Mode::Sealed,
            json!({"seal": {"reason": "seal_command", "workspace_head": null}}),
        );
        put(w.scratch.path(), "sealed.json", &sealed);
        w
    }

    fn dir(&self) -> PathBuf {
        part(&self.home.goalcup_home())
    }

    fn status(&self) -> PathBuf {
        self.dir().join("status.json")
    }

    /// The shell action that moves the sealed `status.json` into place.
    fn seals(&self) -> String {
        let (from, to) = (self.scratch.path().join("sealed.json"), self.status());
        let tmp = format!("{}.tmp", to.display());
        format!("cp {} {tmp}; mv {tmp} {}; exit 0", from.display(), to.display())
    }

    /// An action that only notes the signal in the `hit` file.
    fn notes(&self) -> String {
        format!("echo hit >> {}", self.scratch.path().join("hit").display())
    }

    fn hit(&self) -> bool {
        self.scratch.path().join("hit").exists()
    }

    /// Writes the recorder `r` into `status.json` as the part's recorder.
    fn recorder(&self, r: &Stand) {
        let st = with(
            Mode::Recording,
            json!({"pid": r.0.id(), "start_id": r.start_id()}),
        );
        put(&self.dir(), "status.json", &st);
    }

    fn watchdog(&self, w: &Stand) {
        let rec = self.dir().join("rec");
        std::fs::create_dir_all(&rec).unwrap();
        std::fs::write(rec.join("watchdog.pid"), format!("{} {}\n", w.0.id(), w.start_id()))
            .unwrap();
    }

    fn goalcup(&self, args: &[&str], harness: bool) -> Output {
        let mut cmd = Command::new(common::bin());
        self.home.apply(&mut cmd).args(args).env("GOALCUP_SERVER", "off");
        cmd.env_remove("GOALCUP_TIME_LEFT_FILE").stdin(Stdio::null());
        if harness {
            cmd.env("GOALCUP_TIME_LEFT_FILE", "/nowhere/upload.json");
        }
        cmd.output().unwrap()
    }

    fn sealed(&self) -> bool {
        std::fs::read_to_string(self.status())
            .is_ok_and(|s| s.contains("\"mode\":\"sealed\""))
    }
}

fn one_error_line(out: &Output) {
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert_eq!(err.lines().count(), 1, "stderr: {err}");
}

#[test]
fn signal_goes_to_the_watchdog_and_not_the_recorder() {
    let w = World::new(Mode::Recording);
    let rec = stand_in(w.scratch.path(), "rec", "USR1 TERM", &w.notes());
    w.recorder(&rec);
    let wd = stand_in(w.scratch.path(), "wd", "USR1", &w.seals());
    w.watchdog(&wd);
    let out = w.goalcup(&["seal", "--yes"], false);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(w.sealed() && !w.hit());
    assert!(!out.stdout.is_empty(), "the end screen is printed");
}

#[test]
fn without_a_watchdog_the_recorder_gets_sigterm() {
    let w = World::new(Mode::Recording);
    let rec = stand_in(w.scratch.path(), "rec", "TERM", &w.seals());
    w.recorder(&rec);
    let out = w.goalcup(&["seal", "--yes"], false);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(w.sealed());
}

#[test]
fn nothing_is_signalled_inside_the_harness_or_without_a_terminal() {
    let w = World::new(Mode::Recording);
    let rec = stand_in(w.scratch.path(), "rec", "USR1 TERM", &w.notes());
    w.recorder(&rec);
    let wd = stand_in(w.scratch.path(), "wd", "USR1 TERM", &w.notes());
    w.watchdog(&wd);
    one_error_line(&w.goalcup(&["seal", "--yes"], true));
    // No --yes and stdin is not a terminal.
    one_error_line(&w.goalcup(&["seal"], false));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!w.hit() && !w.sealed());
}

#[test]
fn a_sealed_or_absent_part_is_refused() {
    let w = World::new(Mode::Sealed);
    one_error_line(&w.goalcup(&["seal", "--yes"], false));
    one_error_line(&w.goalcup(&["seal", "--yes", "e0000000000000000"], false));
    let empty = World {
        home: TempHome::new(),
        scratch: tempfile::tempdir().unwrap(),
    };
    one_error_line(&empty.goalcup(&["seal", "--yes"], false));
}
