//! PRD-02 section 8 steps 2 to 11 (point 3 and the never-stop rules; PRD-17 L-P1, L-P7, L-HF3):
//! `watchdog::run_with` with a hand-built plan whose harness is `sh`. The harness forks a child
//! and an orphaned grandchild and waits for a stop file. The `stub_` cases start a shell stub as
//! the recorder (it answers `ready` and `sealed`), so they test the watchdog alone; the `real_`
//! cases start the goalcup binary as the recorder, server off. Linux only (the subreaper).
#![cfg(target_os = "linux")]

use goalcup::cli::{self, proc_start_id, watchdog_alive, watchdog_pid};
use goalcup::connector::{ConnectorRef, HomeLock, HostList, LaunchPlan, NativeSources, builtin};
use goalcup::watchdog::{self, CaPem, StartFacts};
use goalcup_format::Record;
use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(60);

/// Waits until `f` gives `Some`, at most [`WAIT`].
fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The pid is gone: no `/proc` entry, or a zombie.
fn gone(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |s| {
        s[s.rfind(')').unwrap() + 1..].trim_start().starts_with('Z')
    })
}

fn pid_in(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

struct Fixture {
    _tmp: tempfile::TempDir,
    /// Marker files.
    d: PathBuf,
    home: PathBuf,
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
            home,
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
            home: self.home.clone(),
            connector: plan.connector.clone(),
            harness_version: None,
            keep: true,
            build: None,
            hosts: plan.hosts.clone(),
            hosts_sha256: plan.hosts.sha256().to_string(),
            native: plan.native.clone(),
            mask_rules: Vec::new(),
            values: Vec::new(),
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

    /// Runs the watchdog on this thread and `drive` beside it.
    fn run(&self, recorder: &Path, drive: impl FnOnce() + Send + 'static) {
        let plan = self.plan();
        let facts = self.facts(&plan);
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
    for name in ["harness.pid", "child.pid", "orphan.pid"] {
        let pid = pid_in(&f.m(name)).unwrap();
        until(name, || gone(pid).then_some(()));
    }
    assert!(!f.m("recorder_signalled").exists());
    let seal = std::fs::read_to_string(f.m("seal_line")).unwrap();
    assert!(seal.contains("seal_command"));
}

/// Every record of the entry up to each part file's last index line.
fn records(entry_dir: &Path) -> Vec<Record> {
    let mut out = Vec::new();
    for part in goalcup_format::read::part_files(&entry_dir.join("rec")).unwrap() {
        let name = part.file_name().unwrap().to_str().unwrap();
        let idx = part.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")));
        let Some(last) = goalcup_format::read::read_index(&idx).unwrap().pop() else {
            continue;
        };
        let file = std::fs::File::open(&part).unwrap().take(last.c_end);
        out.extend(
            goalcup_format::read::decode(file)
                .unwrap()
                .map(|i| i.unwrap().rec),
        );
    }
    out
}

/// The real recorder, server off.
fn real() -> PathBuf {
    // SAFETY: set before any thread of this test process starts.
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
}
