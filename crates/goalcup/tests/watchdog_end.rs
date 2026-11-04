//! PRD-02 section 8 steps 2 to 11 (point 3 and the never-stop rules; PRD-17 L-P1, L-P7, L-HF3):
//! `watchdog::run_with` with a hand-built plan whose harness is `sh`. The harness forks a child
//! and an orphaned grandchild and waits for a stop file. The `stub_` cases start a shell stub as
//! the recorder (it answers `ready` and `sealed`), so they test the watchdog alone; the `real_`
//! cases start the goalcup binary as the recorder, server off. The orphan checks are Linux only
//! (the subreaper).

mod common;
mod watchdog_share;

use common::recorder::records;
use goalcup::cli::{self, proc_start_id, watchdog_alive, watchdog_pid};
use goalcup::connector::{ConnectorRef, HomeLock, HostList, LaunchPlan, NativeSources, builtin};
use goalcup::watchdog::{
    self, CaPem, HarnessFacts, StartFacts, ToRecorder, ToWatchdog, read_line, send_line,
};
use goalcup_format::Record;
use serde_json::to_value;
use std::collections::BTreeMap;
use std::io::BufReader;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use watchdog_share::{WAIT, gone, until};

fn pid_in(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

struct Fixture {
    _tmp: tempfile::TempDir,
    /// Marker files.
    d: PathBuf,
    entry_dir: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("m");
        std::fs::create_dir_all(&d).unwrap();
        let home = tmp.path().join("g");
        let (run, entry) = ("r0123456789abcdef", "e0123456789abcdef");
        let run_dir = cli::run_dir(&home, run);
        let entry_dir = cli::entry_dir(&home, run, entry);
        std::fs::create_dir_all(&entry_dir).unwrap();
        let server = serde_json::json!({
            "v": 1, "server": null, "run": run, "mode": "practice", "deadline": null,
            "repo": null, "connector": {"id": "fake", "name": "Fake", "source": "builtin"},
            "hosts_file": null,
            "entries": [{"id": entry, "harness_version": null, "resumed": false}],
        });
        std::fs::write(cli::server_json(&run_dir), server.to_string()).unwrap();
        Fixture {
            _tmp: tmp,
            d,
            entry_dir,
        }
    }

    fn m(&self, name: &str) -> PathBuf {
        self.d.join(name)
    }

    /// `sh` as the harness: `before` and `after` touch their markers (`after` notes whether the
    /// seal came first); the harness notes whether `before` ran, forks a child and an orphan, and
    /// waits for `stop`.
    fn plan(&self) -> LaunchPlan {
        let d = self.d.display();
        let harness = format!(
            "[ -e {d}/before ] && : > {d}/saw_before
             sleep 1000 & echo $! > {d}/child.pid
             ( sleep 1000 & echo $! > {d}/orphan.pid )
             echo $$ > {d}/harness.pid
             while [ ! -e {d}/stop ]; do sleep 0.05; done"
        );
        let hosts = HostList::from_file(builtin::HOSTS.as_bytes(), "model-hosts.toml").unwrap();
        let sh = |s: String| vec!["-c".to_string(), s];
        LaunchPlan {
            connector: ConnectorRef {
                id: "fake".into(),
                name: "Fake".into(),
                sha256: "0".repeat(64),
                source: Default::default(),
            },
            harness_version: None,
            program: PathBuf::from("/bin/sh"),
            argv: sh(harness),
            env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            home: self.d.join("home"),
            lock: HomeLock::default(),
            before: vec![sh(format!(": > {d}/before"))],
            after: vec![sh(format!(
                "[ -e {d}/seal_line ] || : > {d}/after_first; : > {d}/after"
            ))],
            ca_env: Vec::new(),
            hosts,
            native: NativeSources::default(),
            mask_rules: Vec::new(),
            secret_values: Vec::new(),
            notes: Vec::new(),
            version_notes: Vec::new(),
            unread: Vec::new(),
            log: Vec::new(),
            refusal: None,
        }
    }

    fn facts(&self, plan: &LaunchPlan) -> StartFacts {
        StartFacts {
            run: "r0123456789abcdef".into(),
            entry: "e0123456789abcdef".into(),
            segment: 0,
            entry_dir: self.entry_dir.clone(),
            deadline: None,
            server_time: None,
            local_wall: 0,
            workspace: self.d.clone(),
            connector: to_value(&plan.connector).unwrap(),
            harness_version: None,
            keep: true,
            build: None,
            hosts: to_value(&plan.hosts).unwrap(),
            hosts_sha256: plan.hosts.sha256().to_string(),
            native: to_value(&plan.native).unwrap(),
            mask_rules: serde_json::json!([]),
            values: serde_json::json!([]),
            ca: CaPem {
                cert: String::new(),
                key: String::new(),
            },
            prev: None,
            harness: None,
            null_since: None,
            notes: Vec::new(),
        }
    }

    /// A recorder that reads the first line, says `ready`, notes the `seal` line and a signal,
    /// and says `sealed`.
    fn stub(&self) -> PathBuf {
        let d = self.d.display();
        let path = self.m("recorder.sh");
        let text = format!(
            "#!/bin/sh
             trap ': > {d}/recorder_signalled' TERM INT HUP USR1 QUIT
             read -r first <&4
             echo $$ > {d}/recorder.pid
             printf '{{\"line\":\"ready\"}}\\n' >&4
             while read -r l <&4; do
               case \"$l\" in
                 *'\"line\":\"seal\"'*) printf '%s\\n' \"$l\" > {d}/seal_line
                   printf '{{\"line\":\"sealed\"}}\\n' >&4; exit 0;;
               esac
             done"
        );
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// [`Fixture::plan`] where the harness, a child and an orphan each note SIGTERM in
    /// `<name>_term` and stay, so only SIGKILL ends them.
    fn stubborn(&self) -> LaunchPlan {
        let d = self.d.display();
        let stay = |n: &str| format!("trap ': > {d}/{n}_term' TERM; while :; do sleep 0.05; done");
        let mut plan = self.plan();
        plan.argv[1] = format!(
            "( {} ) & echo $! > {d}/child.pid
             ( ( {} ) & echo $! > {d}/orphan.pid )
             echo $$ > {d}/harness.pid
             {}",
            stay("child"),
            stay("orphan"),
            stay("harness")
        );
        plan
    }

    /// Runs the watchdog on this thread and `drive` beside it.
    fn run(&self, recorder: &Path, drive: impl FnOnce() + Send + 'static) {
        let plan = self.plan();
        let facts = self.facts(&plan);
        self.run_plan(plan, facts, recorder, drive);
    }

    fn run_plan(
        &self,
        plan: LaunchPlan,
        facts: StartFacts,
        recorder: &Path,
        drive: impl FnOnce() + Send + 'static,
    ) {
        let driver = std::thread::spawn(drive);
        let _ = watchdog::run_with(plan, facts, recorder);
        driver.join().unwrap();
    }

    fn status(&self) -> Option<serde_json::Value> {
        cli::read_json(&cli::status_json(&self.entry_dir))
    }

    fn sealed_status(&self) -> serde_json::Value {
        until("the sealed status.json", || {
            self.status()
                .filter(|s| s.get("seal").is_some_and(|v| !v.is_null()))
        })
    }
}

/// Sends `goalcup seal`'s signal to the pid in `rec/watchdog.pid`.
fn seal_command(entry_dir: &Path) {
    let (pid, _) = watchdog_pid(entry_dir).unwrap();
    // SAFETY: a signal to one pid.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGUSR1) };
}

/// Point 3 (PRD-02 section 8 steps 6 and 11): `before` ran before the harness started and
/// `after` ran after it exited and before the seal.
#[test]
fn watchdog_end_stub_helpers_run_in_order() {
    let f = Fixture::new();
    let (harness, stop) = (f.m("harness.pid"), f.m("stop"));
    f.run(&f.stub(), move || {
        until("the harness", || pid_in(&harness));
        std::fs::write(stop, "").unwrap();
    });
    assert!(f.m("saw_before").exists(), "before ran after the harness");
    assert!(f.m("after_first").exists(), "after ran after the seal");
    let seal = std::fs::read_to_string(f.m("seal_line")).unwrap();
    assert!(seal.contains("harness_exit"));
    for name in ["child.pid", "orphan.pid"] {
        // SAFETY: a signal to one pid this test started.
        unsafe { libc::kill(pid_in(&f.m(name)).unwrap() as libc::pid_t, libc::SIGKILL) };
    }
}

/// Point 3 and L-P7, L-HF3 (PRD-02 section 8 steps 2 and 10): while it runs, `rec/watchdog.pid`
/// names the watchdog as `<pid> <start_id>` and reads alive; on `goalcup seal` (SIGUSR1) the
/// harness, its child and its orphan are all ended, and the recorder and the watchdog are never
/// signalled.
#[test]
fn watchdog_end_stub_seal_command_ends_the_tree_only() {
    let f = Fixture::new();
    let (entry_dir, harness, orphan) = (f.entry_dir.clone(), f.m("harness.pid"), f.m("orphan.pid"));
    f.run(&f.stub(), move || {
        until("the orphan", || pid_in(&orphan));
        until("the harness", || pid_in(&harness));
        let me = std::process::id();
        assert_eq!(
            watchdog_pid(&entry_dir),
            Some((me, proc_start_id(me).unwrap()))
        );
        assert!(watchdog_alive(&entry_dir));
        seal_command(&entry_dir);
    });
    for name in ["harness.pid", "child.pid"] {
        let pid = pid_in(&f.m(name)).unwrap();
        until(name, || gone(pid).then_some(()));
    }
    orphan_ended(&f);
    assert!(!f.m("recorder_signalled").exists());
    let seal = std::fs::read_to_string(f.m("seal_line")).unwrap();
    assert!(seal.contains("seal_command"));
}

/// On Linux the orphan was ended with the tree (the subreaper took it in). Elsewhere nothing
/// finds it, so it is killed here and does not outlive the test.
fn orphan_ended(f: &Fixture) {
    let pid = pid_in(&f.m("orphan.pid")).unwrap();
    if cfg!(target_os = "linux") {
        until("orphan", || gone(pid).then_some(()));
    } else {
        // SAFETY: a signal to one pid this test started.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
}

/// The real recorder, server off.
fn real() -> PathBuf {
    // SAFETY: set before any thread of this test process starts; sound only because nextest
    // runs each test in its own process.
    unsafe { std::env::set_var("GOALCUP_SERVER", "off") };
    PathBuf::from(env!("CARGO_BIN_EXE_goalcup"))
}

/// Point 3 and point 6 (PRD-02 section 11 step 6): after `goalcup seal` the sealed
/// `status.json` names the reason and the harness's exit, and the CA key file is gone.
#[test]
fn watchdog_end_real_seal_command_seals() {
    let f = Fixture::new();
    let (entry_dir, orphan) = (f.entry_dir.clone(), f.m("orphan.pid"));
    f.run(&real(), move || {
        until("the orphan", || pid_in(&orphan));
        seal_command(&entry_dir);
    });
    let status = f.sealed_status();
    assert_eq!(status["seal"]["reason"], "seal_command");
    assert!(!status["harness"]["exit"].is_null());
    assert!(!f.entry_dir.join("rec").join(watchdog::KEY_FILE).exists());
    orphan_ended(&f);
}

/// Point 3, ledger R3 and L-P1 (PRD-02 section 8 step 8): a recorder killed mid-run is
/// followed at once by a second boot, and the sealed entry holds exactly one `recorder_down`
/// GAP, with start before end, in records that decode.
#[test]
fn watchdog_end_real_killed_recorder_restarts() {
    let f = Fixture::new();
    let (entry_dir, orphan) = (f.entry_dir.clone(), f.m("orphan.pid"));
    let status =
        move || -> Option<serde_json::Value> { cli::read_json(&cli::status_json(&entry_dir)) };
    let entry_dir = f.entry_dir.clone();
    f.run(&real(), move || {
        until("the orphan", || pid_in(&orphan));
        let first = until("boot 0", || {
            let s = status()?;
            (s["boot"] == 0 && s["pid"].as_u64()? > 0).then(|| s["pid"].as_u64())?
        });
        // A boot that dies before its first LINK leaves its number to the next, so wait for one.
        let counts = entry_dir.join("counts.jsonl");
        until("a link", || {
            std::fs::metadata(&counts).ok().filter(|m| m.len() > 0)
        });
        // SAFETY: a signal to one pid, the recorder's.
        unsafe { libc::kill(first as libc::pid_t, libc::SIGKILL) };
        until("boot 1", || {
            let s = status()?;
            (s["boot"].as_u64()? >= 1 && s["pid"].as_u64()? != first).then_some(())
        });
        seal_command(&entry_dir);
    });
    f.sealed_status();
    let down: Vec<(u64, u64)> = records(&f.entry_dir)
        .into_iter()
        .filter_map(|r| match r {
            Record::Gap {
                reason: goalcup_format::GapReason::RecorderDown,
                start_mono,
                end_mono,
                ..
            } => Some((start_mono, end_mono)),
            _ => None,
        })
        .collect();
    assert_eq!(down.len(), 1, "{down:?}");
    assert!(down[0].0 < down[0].1);
    orphan_ended(&f);
}

/// Point 3 (review focus 2): a recorder that can no longer be spawned never ends the watch while
/// the harness runs, and never wedges it after: the watchdog gives up 30 s after the harness
/// exit, and the entry is left for `close_dead_entry`.
#[test]
fn watchdog_end_real_unspawnable_recorder_ends_after_the_harness() {
    let f = Fixture::new();
    // Every boot after the first fails with ENOENT.
    let rec = f.m("rec.sh");
    let text = format!(
        "#!/bin/sh\nrm -f \"$0\"\nexec \"{}\" \"$@\"\n",
        real().display()
    );
    std::fs::write(&rec, text).unwrap();
    std::fs::set_permissions(&rec, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (entry_dir, stop) = (f.entry_dir.clone(), f.m("stop"));
    let (stopped_tx, stopped_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        f.run(&rec, move || {
            let pid = until("the recorder", || {
                let s: serde_json::Value = cli::read_json(&cli::status_json(&entry_dir))?;
                let counts = std::fs::metadata(entry_dir.join("counts.jsonl")).ok()?;
                s["pid"].as_u64().filter(|p| *p > 0 && counts.len() > 0)
            });
            // SAFETY: a signal to one pid, the recorder's.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            let log = entry_dir.join("goalcup.log");
            until("recorder_spawn_failed", || {
                let l = std::fs::read_to_string(&log).ok()?;
                l.contains("recorder_spawn_failed").then_some(())
            });
            std::thread::sleep(std::time::Duration::from_secs(2));
            stopped_tx.send(std::time::Instant::now()).unwrap();
            std::fs::write(stop, "").unwrap();
        });
        let _ = done_tx.send(f);
    });
    let f = done_rx.recv_timeout(WAIT).expect("the watchdog wedged");
    let stopped = stopped_rx.recv().unwrap();
    assert!(stopped.elapsed() >= std::time::Duration::from_secs(29));
    assert!(stopped.elapsed() < std::time::Duration::from_secs(60));
    let log = std::fs::read_to_string(f.entry_dir.join("goalcup.log")).unwrap();
    assert!(log.contains("recorder_spawn_gave_up"), "{log}");
    assert!(matches!(
        goalcup::pipeline::close_dead_entry(&f.entry_dir),
        Ok(goalcup::pipeline::Closed::Sealed(_))
    ));
    for name in ["child.pid", "orphan.pid"] {
        // SAFETY: a signal to one pid this test started.
        unsafe { libc::kill(pid_in(&f.m(name)).unwrap() as libc::pid_t, libc::SIGKILL) };
    }
}

/// Point 3 (PRD-02 section 8 step 10): at the deadline (boot time, from the start facts) the
/// harness, its child and its orphan get SIGTERM, then SIGKILL; the recorder is never signalled
/// (it seals as boot 0 with reason `deadline`, not `signal`), nor is the watchdog (this test).
#[test]
fn watchdog_end_real_deadline_ends_the_tree() {
    let f = Fixture::new();
    let plan = f.stubborn();
    let mut facts = f.facts(&plan);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    facts.deadline = Some(now.unwrap().as_millis() as u64 + 3_000);
    let orphan = f.m("orphan.pid");
    f.run_plan(plan, facts, &real(), move || {
        until("the orphan", || pid_in(&orphan));
    });
    for name in ["harness", "child"] {
        assert!(f.m(&format!("{name}_term")).exists(), "{name}: no SIGTERM");
        let pid = pid_in(&f.m(&format!("{name}.pid"))).unwrap();
        until(name, || gone(pid).then_some(()));
    }
    if cfg!(target_os = "linux") {
        assert!(f.m("orphan_term").exists(), "orphan: no SIGTERM");
    }
    orphan_ended(&f);
    let status = f.sealed_status();
    assert_eq!(status["seal"]["reason"], "deadline");
    assert_eq!(status["boot"], 0);
}

/// Point 3 (PRD-02 section 8 step 10): an orphan the subreaper took in that exits while the
/// harness runs is reaped then, not left a zombie until the seal; the harness's own exit status
/// is still the watchdog's to read.
#[cfg(target_os = "linux")]
#[test]
fn watchdog_end_real_adopted_orphans_are_reaped() {
    let f = Fixture::new();
    let d = f.d.display();
    let mut plan = f.plan();
    plan.argv[1] = format!(
        "( sleep 0.2 & echo $! > {d}/quick.pid ); echo $$ > {d}/harness.pid
         while [ ! -e {d}/stop ]; do sleep 0.05; done"
    );
    let facts = f.facts(&plan);
    let (quick, stop) = (f.m("quick.pid"), f.m("stop"));
    f.run_plan(plan, facts, &real(), move || {
        let reaped = std::panic::catch_unwind(|| {
            let pid = until("quick.pid", || pid_in(&quick));
            until("reaped", || {
                (!Path::new(&format!("/proc/{pid}")).exists()).then_some(())
            });
        });
        // Stopped either way, so a failed wait never leaves the watchdog running.
        std::fs::write(stop, "").unwrap();
        reaped.unwrap();
    });
    assert_eq!(
        f.sealed_status()["harness"]["exit"],
        serde_json::json!({"code": 0})
    );
}

/// The real recorder under a watchdog played by hand: `start` (deadline `deadline_in` ms ahead),
/// `ready`, then `launch` for a `sleep 600` harness. Returns the recorder, the control socket and
/// the harness; a stale `rec/run-ca.key` stands where the watchdog would write it.
fn by_hand(
    f: &Fixture,
    deadline_in: Option<u64>,
) -> (std::process::Child, UnixStream, std::process::Child) {
    let plan = f.plan();
    let mut facts = f.facts(&plan);
    let ca = goalcup::proxy::ca::RunCa::mint(&plan.hosts.ca_subtrees()).unwrap();
    facts.ca = ca.export();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    facts.deadline = deadline_in.map(|ms| now.unwrap().as_millis() as u64 + ms);
    let rec = f.entry_dir.join("rec");
    std::fs::create_dir_all(&rec).unwrap();
    std::fs::write(rec.join(watchdog::KEY_FILE), "x").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let (ours, theirs) = UnixStream::pair().unwrap();
    let recorder = watchdog::spawn::recorder(&real(), false, &listener, &theirs).unwrap();
    drop(theirs);
    assert!(send_line(&ours, &ToRecorder::Start(Box::new(facts))));
    let mut lines = BufReader::new(ours.try_clone().unwrap());
    let ready = common::within(WAIT, move || read_line::<ToWatchdog>(&mut lines));
    assert_eq!(ready, Some(Some(ToWatchdog::Ready)));
    let mut sleep = std::process::Command::new("sleep");
    let harness = sleep.arg("600").spawn().unwrap();
    let pid = harness.id();
    let launch = HarnessFacts {
        pid,
        program: "sleep".into(),
        started: "2026-11-01T00:00:00Z".into(),
        start_id: proc_start_id(pid).unwrap(),
    };
    assert!(send_line(&ours, &ToRecorder::Launch(launch)));
    (recorder, ours, harness)
}

/// Point 3 and the never-stop rules (PRD-02 sections 9.1 and 11): with the watchdog gone, the
/// recorder seals itself at its own deadline and never signals the harness.
#[test]
fn watchdog_end_real_recorder_alone_seals_at_its_deadline() {
    let f = Fixture::new();
    let (mut recorder, ours, mut harness) = by_hand(&f, Some(3_000));
    drop(ours);
    assert_eq!(f.sealed_status()["seal"]["reason"], "deadline");
    assert!(common::within(WAIT, move || recorder.wait().unwrap()).success());
    assert!(!gone(harness.id()), "the harness was ended");
    harness.kill().unwrap();
    harness.wait().unwrap();
}

/// Point 3 and point 4 (PRD-02 section 11): SIGTERM to a recorder whose watchdog is gone seals
/// with reason `signal`, removes the CA key and exits 0.
#[test]
fn watchdog_end_real_sigterm_seals_with_signal() {
    let f = Fixture::new();
    let (mut recorder, ours, mut harness) = by_hand(&f, None);
    drop(ours);
    // SAFETY: a signal to one pid, the recorder this test started.
    unsafe { libc::kill(recorder.id() as libc::pid_t, libc::SIGTERM) };
    let status = common::within(WAIT, move || recorder.wait().unwrap());
    assert_eq!(status.code(), Some(0));
    assert_eq!(f.sealed_status()["seal"]["reason"], "signal");
    assert!(!f.entry_dir.join("rec").join(watchdog::KEY_FILE).exists());
    harness.kill().unwrap();
    harness.wait().unwrap();
}
