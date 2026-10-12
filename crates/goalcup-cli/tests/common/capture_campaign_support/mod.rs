//! Support the two T25 campaigns share: `capture_failure_campaign.rs` (storage and accounting)
//! and `capture_crash_campaign.rs` (recorder crashes). Each adds its own module on top of
//! `m2_vertical_support` (the real binaries, controlled endpoint, private roots, bounded waits,
//! owned children, chain reader) for what is genuinely its own, and takes the rest from here, so
//! neither campaign imports the other's support. Nothing here edits or shadows
//! `m2_vertical_support`, and nothing reaches the capture path except the real binaries, the
//! operating system and the filesystem.
//!
//! - [`RUN`] and [`OBSERVE`], the two bounds every row waits within;
//! - bounded waits on the signed status (never on a sleep alone) and counts of the workload's
//!   own successful exchanges, from its report outside the recorder;
//! - owned-process helpers that refuse to signal a process whose command line is not this
//!   test's own private root, so a stray daemon of another run can never be touched;
//! - [`Row`], which keeps each row's observations, binary identities, logs and byte snapshots
//!   under `target/<evidence_dir>/rows/<test>/run-NN/` even when an assertion fails first. Each
//!   run takes a directory that did not exist before and nothing earlier is overwritten.
//!
//! Each campaign uses a subset, which the one allow in `common/mod.rs` covers, rather than one
//! campaign's support being imported into the other.

use super::m2_vertical_support::{Vertical, wait_until};
use goalcup_protocol::capture::M2Status;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Room for a workload that ends on its own, or a finish after a release.
pub const RUN: Duration = Duration::from_secs(60);

/// Room for a status to change because of something the test did.
pub const OBSERVE: Duration = Duration::from_secs(30);

fn text(command: &mut Command) -> String {
    command
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn sha256(bytes: &[u8]) -> String {
    goalcup_daemon::workspace::sha256_hex(bytes)
}

// ------------------------------------------------------------------ status and delivery

/// How often a status wait asks the daemon. Each ask is one `goalcup status` process.
const POLL: Duration = Duration::from_millis(200);

/// The signed status's capture projection now.
pub fn capture(v: &Vertical, entry: &str) -> M2Status {
    v.status(entry).payload.capture.expect("a capture status")
}

/// Poll the status until `ready` accepts it, or fail naming `what` with the last status seen.
pub fn wait_capture(
    v: &Vertical,
    entry: &str,
    what: &str,
    within: Duration,
    ready: impl Fn(&M2Status) -> bool,
) -> M2Status {
    let until = Instant::now() + within;
    loop {
        let status = capture(v, entry);
        if ready(&status) {
            return status;
        }
        assert!(
            Instant::now() < until,
            "waited {within:?} for {what}; last status: {status:?}"
        );
        std::thread::sleep(POLL);
    }
}

/// What the workload reports it completed with `ok` true, per lane, from `events.jsonl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivered {
    /// HTTP echoes (a request relayed up and its response relayed back, byte for byte).
    pub http: usize,
    /// WebSocket messages echoed back whole.
    pub ws: usize,
    /// Exchanges of any lane the workload saw fail.
    pub failed: usize,
}

/// The exchanges the workload finished at or after `since` (seconds since the epoch; 0.0 for
/// all of them). An exchange belongs to the moment it started.
pub fn delivered(v: &Vertical, since: f64) -> Delivered {
    let mut found = Delivered {
        http: 0,
        ws: 0,
        failed: 0,
    };
    for event in v.events() {
        if event["start"].as_f64().is_some_and(|t| t < since) {
            continue;
        }
        if event["ok"] != true {
            found.failed += 1;
        } else if event["lane"] == "echo" {
            found.http += 1;
        } else if event["lane"] == "ws" {
            found.ws += 1;
        }
    }
    found
}

/// Wait until the workload has delivered at least `http` echoes and `ws` messages that started
/// at or after `since`.
pub fn wait_delivered(v: &Vertical, since: f64, http: usize, ws: usize, within: Duration) {
    let until = Instant::now() + within;
    loop {
        let now = delivered(v, since);
        if now.http >= http && now.ws >= ws {
            return;
        }
        assert!(
            Instant::now() < until,
            "waited {within:?} for {http} echoes and {ws} WS messages after {since}; saw {now:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ------------------------------------------------------------------ processes

/// `ps -o stat=` of `pid`: empty once the process is gone.
pub fn ps_stat(pid: i32) -> String {
    text(Command::new("/bin/ps").args(["-o", "stat=", "-p", &pid.to_string()]))
}

/// `ps -o lstart=` of `pid`: its start time, which tells a replacement process from the
/// original one even if the pid were reused.
pub fn ps_started(pid: i32) -> String {
    text(Command::new("/bin/ps").args(["-o", "lstart=", "-p", &pid.to_string()]))
}

pub fn ps_command(pid: i32) -> String {
    text(Command::new("/bin/ps").args(["-ww", "-o", "command=", "-p", &pid.to_string()]))
}

/// The one live process whose parent is `parent` and whose command line contains `contains`:
/// `goalcup start`'s parked capture child (a fork of the launcher, still running the
/// launcher's own image) is found this way. Polled, bounded.
pub fn child_of(parent: i32, contains: &str, what: &str) -> i32 {
    let mut found = None;
    wait_until(what, || {
        let table = text(Command::new("/bin/ps").args(["-axww", "-o", "pid=,ppid=,command="]));
        found = table.lines().find_map(|line| {
            let mut it = line.split_whitespace();
            let pid: i32 = it.next()?.parse().ok()?;
            let ppid: i32 = it.next()?.parse().ok()?;
            let command = it.collect::<Vec<_>>().join(" ");
            (ppid == parent && command.contains(contains) && running(pid)).then_some(pid)
        });
        found.is_some()
    });
    found.unwrap()
}

/// `kill -9` of `pid`, whose command line must contain `contains` and whose start time must
/// be `started`: a process this test identified, never a
/// bare pid.
pub fn kill9_owned_child(pid: i32, contains: &str, started: &str) {
    assert!(
        ps_started(pid) == started && ps_command(pid).contains(contains),
        "pid {pid} is not the process this test identified"
    );
    // SAFETY: two integers; the pid was just checked against the recorded start time.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
}

/// Whether `pid` is a live process (a zombie awaiting its parent is not).
pub fn running(pid: i32) -> bool {
    let stat = ps_stat(pid);
    !stat.is_empty() && !stat.starts_with('Z')
}

/// Wait, bounded, until `pid` has exited.
pub fn wait_exited(pid: i32, what: &str) {
    wait_until(what, || !running(pid));
}

/// `kill -9` of `pid`, which must be a daemon this test spawned: its command line has to name
/// this test's private root. Anything else is refused before a signal is sent, so the stray
/// daemons of earlier work under `/private/tmp` can never be reached from here.
pub fn kill9_owned_daemon(v: &Vertical, pid: i32) {
    let command = ps_command(pid);
    assert!(
        command.contains("goalcup-daemon") && command.contains(&v.root.display().to_string()),
        "pid {pid} is not this test's daemon: {command:?}"
    );
    // SAFETY: two integers; the pid was just checked to be this test's own daemon.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
    wait_exited(pid, "the killed daemon to exit");
}

/// The same for a SIGSTOP or SIGCONT.
pub fn signal_owned_daemon(v: &Vertical, pid: i32, signal: i32) {
    let command = ps_command(pid);
    assert!(
        command.contains("goalcup-daemon") && command.contains(&v.root.display().to_string()),
        "pid {pid} is not this test's daemon: {command:?}"
    );
    // SAFETY: as above.
    assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
}

// ------------------------------------------------------------------ evidence

/// One row's evidence: the observations the test made, who and what it ran against, the
/// named logs, and raw byte snapshots, written under
/// `target/<evidence_dir>/rows/<name>/run-NN/` when the row ends, whether by passing or by
/// failing an assertion. (`evidence_dir` is `phase1-task2-evidence` for the storage campaign
/// and `phase1-task3-evidence` for the crash campaign.) The directory is created exclusively
/// and numbered after the ones already there, so a rerun keeps every earlier run, failures
/// included. Declare it after the [`Vertical`] so it drops first, while the logs still exist.
pub struct Row {
    name: &'static str,
    began: String,
    logs: PathBuf,
    dir: PathBuf,
    notes: serde_json::Map<String, Value>,
    identity: Value,
}

fn sha256_file(path: &Path) -> String {
    sha256(&std::fs::read(path).unwrap())
}

fn filesystem_of(path: &Path) -> String {
    let df = text(Command::new("/bin/df").arg("-P").arg(path));
    let device = df
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or("")
        .to_string();
    text(&mut Command::new("/sbin/mount"))
        .lines()
        .find(|l| l.starts_with(&format!("{device} on ")))
        .unwrap_or("unknown")
        .to_string()
}

impl Row {
    pub fn new(name: &'static str, v: &Vertical, evidence_dir: &str) -> Row {
        let root = super::support::repo_root();
        let goalcup = PathBuf::from(env!("CARGO_BIN_EXE_goalcup"));
        let identity = json!({
            "git_head": text(Command::new("git").arg("-C").arg(&root).args(["rev-parse", "HEAD"])),
            "git_tree_clean": text(Command::new("git").arg("-C").arg(&root).args(["status", "--porcelain", "--", "crates", "connectors", "Cargo.toml", "Cargo.lock"])).is_empty(),
            "goalcup_sha256": sha256_file(&goalcup),
            "goalcup_daemon_sha256": sha256_file(&super::support::daemon_binary()),
            "goalcup_verify_sha256": sha256_file(&super::support::verify_binary()),
            "profile": goalcup.parent().and_then(|p| p.file_name()).map(|p| p.to_string_lossy().into_owned()),
            "os": text(Command::new("/usr/bin/sw_vers").arg("-productVersion")),
            "kernel": text(Command::new("/usr/bin/uname").arg("-a")),
            "arch": std::env::consts::ARCH,
            "filesystem": filesystem_of(&v.root),
            "trace_root": v.artifacts.display().to_string(),
        });
        // `GOALCUP_ROW_EVIDENCE_DIR` names another evidence root (a later task's own); unset,
        // the rows keep the location of the campaign that asked, `target/<evidence_dir>/rows`.
        let base = std::env::var_os("GOALCUP_ROW_EVIDENCE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("target").join(evidence_dir).join("rows"))
            .join(name);
        std::fs::create_dir_all(&base).expect("the evidence directory");
        // `create_dir` fails on an existing directory: this run owns the one it gets.
        let dir = (1u32..)
            .map(|n| base.join(format!("run-{n:02}")))
            .find(|dir| std::fs::create_dir(dir).is_ok())
            .expect("a free run directory");
        Row {
            name,
            began: text(Command::new("/bin/date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])),
            logs: v.logs.clone(),
            dir,
            notes: serde_json::Map::new(),
            identity,
        }
    }

    /// Keep one observation under `key` (a later one under the same key replaces it).
    pub fn note(&mut self, key: &str, value: impl Into<Value>) {
        let value = value.into();
        eprintln!("[{}] {key}: {value}", self.name);
        self.notes.insert(key.to_string(), value);
    }

    /// Keep raw bytes beside the row's notes, under a name this run has not used.
    pub fn keep_bytes(&self, name: &str, bytes: &[u8]) {
        let path = self.dir.join(name);
        assert!(!path.exists(), "{} was already retained", path.display());
        std::fs::write(&path, bytes).expect("retained bytes");
        eprintln!(
            "[{}] retained {name}: {} bytes, sha256 {}",
            self.name,
            bytes.len(),
            sha256(bytes)
        );
    }

    fn keep(&self, passed: bool) -> std::io::Result<()> {
        std::fs::create_dir_all(self.dir.join("logs"))?;
        let row = json!({
            "row": self.name,
            "began": self.began,
            "ended": text(Command::new("/bin/date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])),
            "test_completed_without_panic": passed,
            "identity": self.identity,
            "observations": self.notes,
        });
        std::fs::write(self.dir.join("row.json"), serde_json::to_vec_pretty(&row)?)?;
        // The named logs only: the numbered per-command files are thousands of status polls.
        for entry in std::fs::read_dir(&self.logs)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let numbered = name.as_bytes().first().is_some_and(u8::is_ascii_digit);
            if !numbered && entry.path().is_file() {
                std::fs::copy(entry.path(), self.dir.join("logs").join(&name))?;
            }
        }
        Ok(())
    }
}

impl Drop for Row {
    fn drop(&mut self) {
        if let Err(error) = self.keep(!std::thread::panicking()) {
            eprintln!("[{}] could not keep evidence: {error}", self.name);
        }
    }
}
