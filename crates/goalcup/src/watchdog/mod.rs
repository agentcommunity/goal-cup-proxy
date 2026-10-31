//! The watchdog (PRD-02 section 8, seam S8): the `goalcup start` or `continue` process. It holds
//! the listen socket (fd 3 of the recorder) and one control socket (fd 4), restarts a dead
//! recorder at once on the same port, and ends the harness at the deadline and on seal.
//!
//! Here: the control lines (S8) and the start facts (S17), [`run`] with the restart loop, null
//! mode ([`Loop`], ledger R3a), the deadline and the server word, the terminal [`shield`], and
//! what the watchdog binds, [`spawn`]s and ends. `recover` comes in the next slice.

pub mod shield;
pub mod spawn;

use crate::cli::log::{Fact, line};
use crate::cli::{ServerJson, proc_start_id, read_json, server_json, status_json};
use crate::connector::{ConnectorRef, HostList, LaunchPlan, MaskRule, NativeSources};
use crate::mask::SecretValue;
use crate::pipeline::clock::{self, Mono};
use crate::pipeline::fds::{self, Target};
use crate::pipeline::status::Outlived;
use crate::proxy::ca::{BUNDLE_FILE, RUN_CA_FILE, RunCa};
use crate::upload::ServerWord;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use shield::Shield;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitCode, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The longest control line, newline included. Control lines carry no recorded byte and are
/// never logged.
pub const MAX_LINE: usize = 64 * 1024;

/// How a process ended: `{"code":n}` or `{"signal":n}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

/// How the previous boot ended, on the first line: `{"code":n}`, `{"signal":n}`,
/// `{"cause":"watchdog_lost"}` from `goalcup recover`, or after null mode
/// `{"cause":"null_mode","exits":n,"last":{"code":n}}` (`last` left out when no exit status was
/// read). Null on boot 0. The next boot's
/// `recorder_down` GAP tail is written from it (PRD-02 section 8 step 9, section 9.2 step 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Prev {
    Exit(Exit),
    Cause(Cause),
}

/// A previous boot that ended with no exit status of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum Cause {
    /// `goalcup recover` replaced a recorder whose watchdog was gone (section 8.1).
    WatchdogLost,
    /// Null mode, which always follows fast exits: how many, and how the last recorder ended
    /// when its exit status was read.
    NullMode {
        exits: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last: Option<Exit>,
    },
}

/// The seal reasons (PRD-01 section 11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealReason {
    HarnessExit,
    Deadline,
    ServerStop,
    SealCommand,
    Signal,
    /// A dead entry closed by `continue`, `seal` or `upload` after both processes died.
    Recovered,
}

/// The run CA, certificate and key, PEM. Sent on the first line only; its `Debug` never shows
/// the key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaPem {
    pub cert: String,
    pub key: String,
}

impl fmt::Debug for CaPem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CaPem { cert, key: <hidden> }")
    }
}

/// The harness as the watchdog launched it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessFacts {
    pub pid: u32,
    pub program: String,
    /// Wall time of the launch, RFC 3339 UTC.
    pub started: String,
    /// The OS start time of `pid` (Linux `/proc/<pid>/stat` field 22, macOS start time in us),
    /// so a reused pid is never taken for the harness.
    pub start_id: u64,
}

/// Seam S17: the start facts, the body of the first control line. Every type serializes with
/// serde. No `Debug`: it holds the CA key and the exact mask values.
///
/// `connector`, `hosts`, `native`, `mask_rules` and `values` carry the S3 and S2 types
/// (`ConnectorRef`, `HostList`, `NativeSources`, `Vec<MaskRule>`, `Vec<SecretValue>`) as their
/// serde JSON, so the line is the same bytes as with typed fields; the recorder parses them with
/// [`StartFacts::typed`]. Typing the fields themselves changes PRD-10's `start.rs` too, so it is
/// one change for the integrator.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct StartFacts {
    /// `r` and 16 hex.
    pub run: String,
    /// `e` and 16 hex.
    pub entry: String,
    /// The entry's segment number from the server's run approval: 0 for `start`, then 1, 2 for
    /// each `continue`.
    pub segment: u32,
    /// The entry folder, `$GOALCUP_HOME/runs/<run>/<entry>/`.
    pub entry_dir: PathBuf,
    /// Unix ms. Null with `GOALCUP_SERVER=off`.
    pub deadline: Option<u64>,
    /// The server's time and the local wall time it was read at, unix ms.
    pub server_time: Option<u64>,
    pub local_wall: u64,
    pub workspace: PathBuf,
    /// `LaunchPlan.connector` (S3 `connector::ConnectorRef`: id, name, sha256, source), for the
    /// `start` keys `connector`, `connector_source` and `connector_sha256`.
    pub connector: serde_json::Value,
    /// `LaunchPlan.harness_version`; null when it could not be read.
    pub harness_version: Option<String>,
    /// `GOALCUP_KEEP_PARTS=1`.
    pub keep: bool,
    /// The 40-hex commit the binary was built from, or null. The version is the binary's own
    /// `CARGO_PKG_VERSION`.
    pub build: Option<String>,
    pub hosts: serde_json::Value,
    /// SHA-256 of the host list file as read (the embedded list or a `--hosts` file), 64 hex.
    pub hosts_sha256: String,
    pub native: serde_json::Value,
    pub mask_rules: serde_json::Value,
    /// The exact mask values. Never logged.
    pub values: serde_json::Value,
    pub ca: CaPem,
    pub prev: Option<Prev>,
    pub harness: Option<HarnessFacts>,
    /// For a null recorder: wall time null mode began, unix ms.
    pub null_since: Option<u64>,
    /// `LaunchPlan.notes`, written as events on boot 0 only.
    pub notes: Vec<serde_json::Value>,
}

/// The typed S3 and S2 fields of [`StartFacts`]. No `Debug`: it holds the exact mask values.
pub struct Typed {
    pub connector: ConnectorRef,
    pub hosts: HostList,
    pub native: NativeSources,
    pub mask_rules: Vec<MaskRule>,
    pub values: Vec<SecretValue>,
}

impl StartFacts {
    /// Parses the JSON fields, taking the exact mask values out of `self`. None when one does not
    /// parse: the recorder treats that as no start line.
    pub fn typed(&mut self) -> Option<Typed> {
        use serde_json::from_value as from;
        let take = |v: &mut serde_json::Value| std::mem::take(v);
        Some(Typed {
            connector: from(take(&mut self.connector)).ok()?,
            hosts: from(take(&mut self.hosts)).ok()?,
            native: from(take(&mut self.native)).ok()?,
            mask_rules: from(take(&mut self.mask_rules)).ok()?,
            values: from(take(&mut self.values)).ok()?,
        })
    }

    /// `$GOALCUP_HOME` (for `upload::UploadCtx.home`, the queue of older entries): the entry
    /// folder is `$GOALCUP_HOME/runs/<run>/<entry>/` (PRD-01 section 1).
    pub fn home(&self) -> PathBuf {
        self.entry_dir
            .ancestors()
            .nth(3)
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }
}

/// The run CA key file in `rec/` (0600, created new, removed at the seal), so `goalcup recover`
/// can load the same CA (ledger R3b). Never in argv, env, a log line, a record or an upload.
pub const KEY_FILE: &str = "run-ca.key";

/// `rec/watchdog.pid`: `<pid> <start_id>`, read by `cli::watchdog_pid`.
pub const PID_FILE: &str = "watchdog.pid";

/// The first boot gave no `ready`, or the watchdog could not bind, write its files or mint the
/// CA: the harness was not started, and PRD-10 says so (its "could not do its job" code).
pub const NOT_STARTED: u8 = 3;

/// A recorder that cannot build its masker from the built-in shapes exits 78 before `ready`;
/// the watchdog passes it through and PRD-10 shows PRD-05's fixed text.
pub const MASK_REFUSED: u8 = 78;

/// The first boot's `ready` wait, on boot time.
const READY_NS: u64 = 10_000_000_000;

/// How long the last lines of an exited recorder are waited for.
const DRAIN_NS: u64 = 1_000_000_000;

/// One second of boot time, in ns.
const SEC: u64 = 1_000_000_000;

/// Ledger R3a: a recorder up less than this exited fast (SIGKILL included); one up this long
/// resets the count and the waits.
const FAST_NS: u64 = 60 * SEC;

/// Ledger R3a: fast exits further apart than this never add up.
const WINDOW_NS: u64 = 600 * SEC;

/// Ledger R3a: the waits in null mode before each normal try, in seconds; the last repeats for
/// ever.
const NULL_WAITS: [u64; 4] = [30, 60, 120, 300];

/// A boot that ends before `ready`, or this soon after its start, died at boot.
const QUICK_NS: u64 = SEC;

/// The space before the next boot after a null boot, or a boot on a sealed entry, that died at
/// boot: 1 s, doubling with each such death in a row, at most the first null wait. Short, so
/// the listener is soon served again; growing, so a boot that always dies never spins.
const SPACE_MAX_NS: u64 = NULL_WAITS[0] * SEC;

/// A null recorder that has not exited this long after the `restart` line is killed.
const RESTART_KILL_NS: u64 = 10 * SEC;

/// The exit code of a recorder whose write failed (`pipeline::writer`): for a null boot, its seal
/// failed and the entry is left for `close_dead_entry`.
const EXIT_WRITE_ERROR: i32 = 74;

/// Set by the SIGUSR1 handler: `goalcup seal`.
static SEAL_ASKED: AtomicBool = AtomicBool::new(false);

extern "C" fn ask_seal(_: libc::c_int) {
    SEAL_ASKED.store(true, Ordering::SeqCst);
}

/// The watchdog for one entry: binds, starts the recorder and the harness, restarts a dead
/// recorder at once, and seals when the harness exits or `goalcup seal` asks. Returns the
/// harness's exit code, [`NOT_STARTED`], or [`MASK_REFUSED`].
pub fn run(plan: LaunchPlan, facts: StartFacts) -> ExitCode {
    match std::env::current_exe() {
        Ok(exe) => run_with(plan, facts, &exe),
        Err(_) => ExitCode::from(NOT_STARTED),
    }
}

/// [`run`] with the recorder started from `recorder_exe`. Every wait runs on boot time
/// ([`Mono`]) compared by subtraction. The first control line is never logged.
pub fn run_with(mut plan: LaunchPlan, mut facts: StartFacts, recorder_exe: &Path) -> ExitCode {
    let dir = facts.entry_dir.clone();
    let rec = dir.join("rec");
    not_dumpable();
    fds::raise(Target::Harness);
    let failed = |why: &'static str| {
        line(&dir, "watchdog_failed", &[("why", Fact::Word(why))]);
        ExitCode::from(NOT_STARTED)
    };
    let Ok(listener) = spawn::bind(prev_port(&facts)) else {
        return failed("bind");
    };
    let port = listener.local_addr().map_or(0, |a| a.port());
    if make_rec(&rec).is_err() {
        return failed("rec");
    }
    // Before the pid file, so a `goalcup seal` that reads it never meets the default action.
    catch_seal();
    if write_pid(&rec).is_err() {
        line(&dir, "watchdog_pid_failed", &[]);
    }
    let Ok(ca) = RunCa::mint(&plan.hosts.ca_subtrees()) else {
        return failed("ca");
    };
    if ca.write_files(&dir).is_err() {
        return failed("ca_files");
    }
    facts.ca = ca.export();
    if write_key(&rec, &facts.ca.key).is_err() {
        remove_key(&rec);
        return failed("key");
    }
    plan.finish_env(port, &dir.join(RUN_CA_FILE), &dir.join(BUNDLE_FILE));

    // The recorder's pid, set under its lock in the same step as the spawn, so the tree walk
    // never takes a just-started recorder for a late child of the watchdog.
    let recorder_pid = Arc::new(Mutex::new(0));
    let mut boot = match Boot::start(recorder_exe, &listener, &facts, &recorder_pid, false) {
        Ok(boot) => boot,
        Err(_) => {
            remove_key(&rec);
            return failed("recorder_spawn");
        }
    };
    let mut boots = 1u64;
    if let Err(code) = boot.wait_ready() {
        remove_key(&rec);
        if code == Some(i32::from(MASK_REFUSED)) {
            return ExitCode::from(MASK_REFUSED);
        }
        return failed("no_ready");
    }

    spawn::helpers(
        &plan.program,
        &plan.before,
        &plan.env,
        spawn::BUDGET_NS,
        &dir,
        "before",
    );
    if spawn::subreaper().is_err() {
        line(&dir, "subreaper_failed", &[]);
    }
    // A harness ended in raw mode would leave the participant's shell typing blind.
    let modes = shield::Modes::save(libc::STDIN_FILENO);
    let harness = spawn::harness(&plan.program, &plan.argv, &plan.env, None);
    let mut shield = None;
    let mut harness_pid = None;
    let mut exit: Option<Option<ExitStatus>> = None;
    match harness {
        Ok(child) => {
            let pid = child.id();
            shield = Shield::take().ok();
            let zero = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: `setrlimit` only reads the struct. After the spawn, so the harness keeps its
            // own core limit.
            unsafe { libc::setrlimit(libc::RLIMIT_CORE, &zero) };
            let launched = HarnessFacts {
                pid,
                program: plan.program.display().to_string(),
                started: goalcup_format::dump::wall(clock::wall_ns()),
                start_id: proc_start_id(pid).unwrap_or(0),
            };
            boot.send(&ToRecorder::Launch(launched.clone()));
            facts.harness = Some(launched);
            harness_pid = Some(pid);
        }
        Err(e) => {
            line(&dir, "harness_failed", &[("io", Fact::Io(e.kind()))]);
            exit = Some(None);
        }
    }
    let harness_start = facts.harness.as_ref().map_or(0, |h| h.start_id);
    // Converted once to boot time; only a server word moves it (PRD-17 S7).
    let wall_ms = clock::wall_ns() / 1_000_000;
    let mut deadline = (facts.deadline).map(|d| due(Mono::now(), d.saturating_sub(wall_ms)));
    let spare = {
        let recorder_pid = Arc::clone(&recorder_pid);
        move |pid: u32| pid == *lock(&recorder_pid)
    };

    let mut ending: Option<JoinHandle<()>> = None;
    let mut after: Option<JoinHandle<()>> = None;
    let mut exit_line: Option<ToRecorder> = None;
    let mut seal: Option<ToRecorder> = None;
    let mut asked = None;
    let mut sealed = false;
    // The null recorder's seal failed (section 11 "In null mode"): the entry is left for
    // `close_dead_entry`, unless the harness still runs and the next boot takes the seal.
    let mut left = false;
    let mut restarts = Restarts::default();
    // Whether the next boot runs `--null`, decided when the last one ended.
    let mut next_null = false;
    // After a server `stop` the seal may come first: the harness is waited for all the same.
    while !(sealed || left) || harness_pid.is_some() {
        let next = boot.next();
        if next == Some(ToWatchdog::Ready) {
            boot.ready = true;
        }
        match next {
            // A restarted boot: what the dead one was told and did not answer, again. Once sealed
            // a boot only relays, and a seal sent again would end it.
            Some(ToWatchdog::Ready) if !sealed => {
                for l in [&exit_line, &seal].into_iter().flatten() {
                    boot.send(l);
                }
            }
            Some(ToWatchdog::Ready) | None => {}
            Some(ToWatchdog::Sealed) => sealed = true,
            Some(ToWatchdog::Server(word)) => {
                if let Some(s) = word.seconds_left {
                    deadline = Some(due(word.asked_at, s.saturating_mul(1_000)));
                }
                if word.stop && word.reason.as_deref() == Some("deadline") {
                    deadline = Some(Mono(0));
                } else if word.stop && seal.is_none() {
                    // Any other stop: seal at once and leave the harness alone (ledger section 7).
                    let l = ToRecorder::Seal {
                        reason: SealReason::ServerStop,
                        outlived: None,
                    };
                    boot.send(&l);
                    seal = Some(l);
                }
            }
        }
        let end = if SEAL_ASKED.swap(false, Ordering::SeqCst) {
            Some(SealReason::SealCommand)
        } else {
            deadline
                .filter(|d| Mono::now() >= *d)
                .map(|_| SealReason::Deadline)
        };
        if let Some(reason) = end
            && ending.is_none()
            && let (Some(pid), Some(h)) = (harness_pid, &facts.harness)
        {
            asked = Some(reason);
            let (target, spare) = ((pid, h.start_id), spare.clone());
            ending = Some(thread::spawn(move || spawn::end_tree(target, &spare)));
        }
        if let Some(status) = harness_pid.and_then(spawn::look) {
            harness_pid = None;
            exit = Some(status);
            if let Some(modes) = &modes {
                let _ = modes.restore();
            }
        }
        if exit.is_some() && exit_line.is_none() {
            let l = ToRecorder::Exit {
                exit: exit.flatten().map(exit_of),
            };
            boot.send(&l);
            exit_line = Some(l);
            let (program, entries, env, dir) = (
                plan.program.clone(),
                plan.after.clone(),
                plan.env.clone(),
                dir.clone(),
            );
            after = Some(thread::spawn(move || {
                spawn::helpers(&program, &entries, &env, spawn::BUDGET_NS, &dir, "after");
            }));
        }
        let helpers_done = after.as_ref().is_some_and(JoinHandle::is_finished)
            && ending.as_ref().is_none_or(JoinHandle::is_finished);
        if seal.is_none() && helpers_done {
            let l = ToRecorder::Seal {
                reason: asked.unwrap_or(SealReason::HarnessExit),
                outlived: spawn::outlived(harness_start, &spare),
            };
            boot.send(&l);
            seal = Some(l);
        }
        let waited = (!boot.ended).then(|| boot.child.try_wait());
        if let Some(Ok(Some(_)) | Err(_)) = waited {
            boot.ended = true;
            let status = waited.and_then(Result::ok).flatten().map(exit_of);
            if !sealed && boot.drain() {
                sealed = true;
            } else if !sealed
                && boot.null
                && seal.is_some()
                && (restarts.asked_at.is_none() || status == Some(Exit::Code(EXIT_WRITE_ERROR)))
            {
                // Section 11 "In null mode": the null seal failed, and the entry is left for
                // `close_dead_entry`. A null boot that exits on the `restart` line is no failure:
                // the normal try gets the seal again after `ready`.
                line(&dir, "null_seal_left", &[]);
                left = true;
            }
            let up_ms = Mono::now().0.saturating_sub(boot.started.0) / 1_000_000;
            restarts.space(&boot, sealed, left);
            // A boot on a sealed entry relays and records nothing: no null mode there, whose
            // `status.json` would hide the seal.
            next_null = !sealed && restarts.next(&boot, status, &mut facts, &dir, seal.is_some());
            if !(sealed || left) || harness_pid.is_some() {
                let facts = [("boot", Fact::N(boots)), ("up_ms", Fact::N(up_ms))];
                line(&dir, "recorder_restart", &facts);
            }
        }
        // Ledger R3: the next recorder at once, same listener, same CA, never capped; a crash
        // loop starts a null one (R3a). After the seal one runs too while the harness lives, so
        // its connections are always served (section 8 step 10): a boot on a sealed entry relays
        // and records nothing. Only a boot that died at boot spaces the next (`Restarts::space`).
        let held = (restarts.hold).is_some_and(|h| Mono::now() < h);
        if boot.ended && !held && (!(sealed || left) || harness_pid.is_some()) {
            // A null boot due when the wait is already over: the normal try takes its place.
            if next_null && restarts.wait_over() {
                restarts.end_null(false);
                (facts.null_since, next_null) = (None, false);
            }
            // A new boot takes the seal after `ready`.
            (restarts.hold, left) = (None, false);
            let mut failed = false;
            boot = loop {
                match Boot::start(recorder_exe, &listener, &facts, &recorder_pid, next_null) {
                    Ok(next) => break next,
                    Err(e) => {
                        if !failed {
                            line(&dir, "recorder_spawn_failed", &[("io", Fact::Io(e.kind()))]);
                        }
                        failed = true;
                        thread::sleep(spawn::TICK);
                    }
                }
            };
            boots += 1;
        }
        restarts.tick(&mut boot, seal.is_some());
    }
    for h in [ending, after].into_iter().flatten() {
        let _ = h.join();
    }
    if let Some(shield) = shield.as_mut()
        && !shield.harness_exited()
    {
        line(&dir, "shield_undrained", &[]);
    }
    drop(plan);
    match exit.flatten() {
        Some(status) => ExitCode::from(code_of(status)),
        None => ExitCode::from(NOT_STARTED),
    }
}

/// A lock that a panic elsewhere never poisons for good: the data are plain values.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One recorder and its end of a fresh control pair. A pair per boot: a dead recorder's unread
/// lines never reach the next one, and its exit closes the pair.
struct Boot {
    child: Child,
    to: UnixStream,
    lines: mpsc::Receiver<Option<ToWatchdog>>,
    /// Started with `--null`.
    null: bool,
    started: Mono,
    /// It sent `ready`.
    ready: bool,
    /// Its exit was seen and handled.
    ended: bool,
}

impl Boot {
    /// Spawns the recorder, sets `pid` to it under its lock, and writes the first line.
    fn start(
        exe: &Path,
        listener: &TcpListener,
        facts: &StartFacts,
        pid: &Mutex<u32>,
        null: bool,
    ) -> io::Result<Boot> {
        let (to, theirs) = UnixStream::pair()?;
        let reader = to.try_clone()?;
        to.set_write_timeout(Some(Duration::from_secs(10)))?;
        let child = {
            let mut pid = lock(pid);
            let child = spawn::recorder(exe, null, listener, &theirs)?;
            *pid = child.id();
            child
        };
        drop(theirs);
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || read_lines(reader, &tx));
        let mut boot = Boot {
            child,
            to,
            lines,
            null,
            started: Mono::now(),
            ready: false,
            ended: false,
        };
        boot.send(&ToRecorder::Start(Box::new(facts.clone())));
        Ok(boot)
    }

    /// One control line; a failed write is the restart loop's to notice.
    fn send(&mut self, l: &ToRecorder) {
        send_line(&self.to, l);
    }

    /// The next line from the recorder within one tick, if any.
    fn next(&self) -> Option<ToWatchdog> {
        match self.lines.recv_timeout(spawn::TICK) {
            Ok(l) => l,
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                thread::sleep(spawn::TICK);
                None
            }
        }
    }

    /// `ready` within 10 s of boot time, else the recorder's exit code (None when it gave none
    /// and was ended).
    fn wait_ready(&mut self) -> Result<(), Option<i32>> {
        let start = Mono::now();
        loop {
            if self.next() == Some(ToWatchdog::Ready) {
                return Ok(());
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(status.code());
            }
            if Mono::now().0.saturating_sub(start.0) >= READY_NS {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Err(None);
            }
        }
    }

    /// After the recorder exited: whether one of its last lines was `sealed`.
    fn drain(&self) -> bool {
        let start = Mono::now();
        while Mono::now().0.saturating_sub(start.0) < DRAIN_NS {
            match self.lines.recv_timeout(spawn::TICK) {
                Ok(Some(ToWatchdog::Sealed)) => return true,
                Ok(None) | Err(RecvTimeoutError::Disconnected) => return false,
                _ => {}
            }
        }
        false
    }
}

/// Null mode in the restart loop (PRD-02 section 8 step 9, ledger R3a).
#[derive(Default)]
struct Restarts {
    looped: Loop,
    /// While a null boot runs: when its wait ends.
    until: Option<Mono>,
    /// When the `restart` line went to the null boot.
    asked_at: Option<Mono>,
    /// The next boot starts no earlier than this (see [`SPACE_MAX_NS`]).
    hold: Option<Mono>,
    /// Boots in a row that died at boot.
    quick: u32,
}

impl Restarts {
    /// After `boot` exited unsealed with `status`: sets `facts` for the next boot and says whether
    /// it starts with `--null`. A null boot and the normal try after it carry the `null_mode`
    /// cause in `prev`, so the try's `recorder_down` gap names the whole span.
    /// A null boot that ended alone before its wait is over is replaced by another null boot
    /// with the same wait; after the `restart` line, the wait, or with a seal pending (the null
    /// wait never delays a seal) the normal try follows.
    fn next(
        &mut self,
        boot: &Boot,
        status: Option<Exit>,
        facts: &mut StartFacts,
        dir: &Path,
        sealing: bool,
    ) -> bool {
        if boot.null {
            let waiting = self.until.is_some_and(|u| Mono::now() < u);
            if self.asked_at.is_none() && waiting && !sealing {
                // The replacement is spaced, but never past the wait's end.
                self.hold = self.hold.min(self.until);
                return true;
            }
            self.end_null(sealing);
            facts.null_since = None;
            return false;
        }
        let Some(wait) = self.looped.exited(boot.started, Mono::now()) else {
            (facts.prev, facts.null_since) = (status.map(Prev::Exit), None);
            return false;
        };
        let exits = self.looped.count;
        facts.prev = Some(Prev::Cause(Cause::NullMode {
            exits,
            last: status,
        }));
        facts.null_since = Some(clock::wall_ns() / 1_000_000);
        self.until = Some(Mono(Mono::now().0.saturating_add(wait)));
        line(dir, "null_mode", &[("wait_s", Fact::N(wait / SEC))]);
        true
    }

    /// A null wait runs and is over.
    fn wait_over(&self) -> bool {
        self.until.is_some_and(|u| Mono::now() >= u)
    }

    /// A null boot ended and the normal try comes next. Its space never pushes that try past the
    /// R3a wait (ledger R3a, locked): the hold is cut to the wait's end, which has passed (the
    /// `restart` line goes only once it has). A fast death of the try sends it straight back to
    /// null mode with the next wait (`Loop::tries`), so no spin opens. Only with a seal pending,
    /// which no null wait delays, does the space stand: a null seal that failed is spaced.
    fn end_null(&mut self, sealing: bool) {
        if !sealing {
            self.hold = self.hold.min(self.until);
        }
        (self.until, self.asked_at) = (None, None);
    }

    /// After `boot` exited: a null boot, or any boot on a sealed entry, that died at boot holds
    /// the next boot back 1 s, then 2 s, 4 s and so on up to [`SPACE_MAX_NS`]. Normal boots on an
    /// unsealed entry follow R3a alone (its first two fast exits restart at once); they neither
    /// wait here nor end the run of deaths. A boot that served longer ends it. A null boot whose
    /// seal failed (`left`) counts as died at boot however long it was up, so a crash loop that
    /// also fails the null seal does not spin. `ready` is seen only when its line is read in a
    /// tick, so a boot that sends it and exits in the same tick counts as died at boot too: at
    /// most one extra 1 s space.
    fn space(&mut self, boot: &Boot, sealed: bool, left: bool) {
        let up = Mono::now().0.saturating_sub(boot.started.0);
        if boot.ready && up >= QUICK_NS && !left {
            self.quick = 0;
            return;
        }
        if !(boot.null || sealed) {
            return;
        }
        let space = (SEC << self.quick.min(8)).min(SPACE_MAX_NS);
        self.quick = self.quick.saturating_add(1);
        self.hold = Some(Mono(Mono::now().0.saturating_add(space)));
    }

    /// Each tick: once a null boot's wait is over and no seal was sent, the `restart` line (never
    /// SIGTERM); SIGKILL if it has not exited 10 s later (a null recorder holds no records).
    fn tick(&mut self, boot: &mut Boot, sealing: bool) {
        let Some(until) = self.until.filter(|_| boot.null && !boot.ended && !sealing) else {
            return;
        };
        let now = Mono::now();
        match self.asked_at {
            None if now >= until => {
                boot.send(&ToRecorder::Restart);
                self.asked_at = Some(now);
            }
            Some(at) if now.0.saturating_sub(at.0) >= RESTART_KILL_NS => {
                let _ = boot.child.kill();
            }
            _ => {}
        }
    }
}

/// The restart loop's memory (ledger R3a): the end times of normal recorders' fast exits on boot
/// time, none older than 10 minutes, how many since the last reset, and the null waits spent
/// since then.
#[derive(Default)]
struct Loop {
    exits: Vec<Mono>,
    count: u64,
    tries: usize,
}

impl Loop {
    /// Ledger R3a (locked): a normal recorder ran from `start` to `end`; the wait in ns in null
    /// mode before the next normal boot, or None for one at once. A run under 60 s is a fast
    /// exit, SIGKILL included; three within 10 minutes switch to null mode, and from there every
    /// fast exit goes straight back. The waits are 30 s, 60 s, 120 s, then 300 s for ever. One up
    /// 60 s resets the count and the waits.
    fn exited(&mut self, start: Mono, end: Mono) -> Option<u64> {
        if end.0.saturating_sub(start.0) >= FAST_NS {
            *self = Loop::default();
            return None;
        }
        self.exits.retain(|e| end.0.saturating_sub(e.0) < WINDOW_NS);
        self.exits.push(end);
        self.count += 1;
        let wait = (self.tries > 0 || self.exits.len() >= 3)
            .then(|| NULL_WAITS[self.tries.min(NULL_WAITS.len() - 1)] * SEC);
        self.tries += usize::from(wait.is_some());
        wait
    }
}

/// The deadline on boot time: `from` plus `ms_left`, saturating. At start `from` is now and
/// `ms_left` the deadline less the wall time; on a server word, `asked_at` and `seconds_left`.
fn due(from: Mono, ms_left: u64) -> Mono {
    Mono(from.0.saturating_add(ms_left.saturating_mul(1_000_000)))
}

/// Reads the recorder's lines until the pair closes, then sends `None`. A line that does not
/// parse (a variant this build does not know) is skipped.
fn read_lines(stream: UnixStream, tx: &mpsc::Sender<Option<ToWatchdog>>) {
    let mut reader = BufReader::new(stream);
    while let Some(l) = read_line(&mut reader) {
        if let Some(l) = l
            && tx.send(Some(l)).is_err()
        {
            return;
        }
    }
    let _ = tx.send(None);
}

/// One control line, both ways: JSON and a newline. Never logged; false when the write failed.
pub fn send_line(mut to: &UnixStream, l: &impl Serialize) -> bool {
    let Ok(mut text) = serde_json::to_vec(l) else {
        return false;
    };
    text.push(b'\n');
    to.write_all(&text).is_ok()
}

/// The next control line, both ways: None at EOF or on a read error; `Some(None)` for a line
/// that is too long (the rest of it is skipped unread) or does not parse, never echoed.
pub fn read_line<T: DeserializeOwned>(r: &mut impl BufRead) -> Option<Option<T>> {
    let mut buf = Vec::new();
    let n = r
        .by_ref()
        .take(MAX_LINE as u64)
        .read_until(b'\n', &mut buf)
        .ok()?;
    match buf.last() {
        _ if n == 0 => None,
        Some(b'\n') => Some(serde_json::from_slice(&buf).ok()),
        _ if n == MAX_LINE => r.skip_until(b'\n').ok().map(|_| None),
        _ => None,
    }
}

/// The previous entry's port on `continue`, so a helper left from it reaches a live proxy;
/// else 0.
fn prev_port(facts: &StartFacts) -> u16 {
    let Some(run_dir) = facts.entry_dir.parent().filter(|_| facts.segment > 0) else {
        return 0;
    };
    let server: Option<ServerJson> = read_json(&server_json(run_dir));
    let ids: Vec<String> =
        server.map_or_else(Vec::new, |s| s.entries.into_iter().map(|e| e.id).collect());
    let prev = ids.iter().take_while(|id| **id != facts.entry).last();
    prev.and_then(|id| read_json::<serde_json::Value>(&status_json(&run_dir.join(id))))
        .and_then(|s| s.get("port")?.as_u64())
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(0)
}

/// `rec/`, mode 0700.
fn make_rec(rec: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(rec)?;
    fs::set_permissions(rec, fs::Permissions::from_mode(0o700))
}

/// `rec/watchdog.pid` as `<pid> <start_id>`, written whole by a rename.
fn write_pid(rec: &Path) -> io::Result<()> {
    let me = std::process::id();
    let start = proc_start_id(me).ok_or(io::ErrorKind::NotFound)?;
    let tmp = rec.join("watchdog.pid.tmp");
    fs::write(&tmp, format!("{me} {start}\n"))?;
    fs::rename(tmp, rec.join(PID_FILE))
}

/// `rec/run-ca.key`: created new, 0600, synced, and `rec/` synced after it.
fn write_key(rec: &Path, key: &str) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(rec.join(KEY_FILE))?;
    file.write_all(key.as_bytes())?;
    file.sync_all()?;
    fs::File::open(rec)?.sync_all()
}

fn remove_key(rec: &Path) {
    let _ = fs::remove_file(rec.join(KEY_FILE));
    let _ = fs::File::open(rec).and_then(|d| d.sync_all());
}

/// `PR_SET_DUMPABLE` 0 on Linux: the watchdog holds the CA key and the exact mask values.
fn not_dumpable() {
    #[cfg(target_os = "linux")]
    // SAFETY: integers only.
    unsafe {
        let z: libc::c_ulong = 0;
        libc::prctl(libc::PR_SET_DUMPABLE, z, z, z, z);
    }
}

/// SIGUSR1 sets [`SEAL_ASKED`]. Caught, never ignored, so the harness gets the default back at
/// `exec`.
fn catch_seal() {
    // SAFETY: a zeroed local struct, filled before `sigaction` reads it; the handler stores into
    // a static atomic and nothing else.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = ask_seal as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_flags = libc::SA_RESTART;
        libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut());
    }
}

fn exit_of(status: ExitStatus) -> Exit {
    match (status.code(), status.signal()) {
        (Some(code), _) => Exit::Code(code),
        (None, Some(signal)) => Exit::Signal(signal),
        (None, None) => Exit::Code(-1),
    }
}

/// A shell's reading of how the harness ended: its code, or 128 plus the signal.
fn code_of(status: ExitStatus) -> u8 {
    match exit_of(status) {
        Exit::Code(code) => u8::try_from(code).unwrap_or(1),
        Exit::Signal(signal) => u8::try_from(128 + signal).unwrap_or(1),
    }
}

/// A line from the watchdog to the recorder, one JSON object per line.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "line", rename_all = "snake_case")]
pub enum ToRecorder {
    /// The first line.
    Start(Box<StartFacts>),
    /// The harness was spawned.
    Launch(HarnessFacts),
    /// The harness exited; null when it was not ours to wait on.
    Exit { exit: Option<Exit> },
    /// Seal now. `outlived`: the Linux children that started after the harness and are still
    /// alive, listed by the watchdog (the subreaper) for the `outlived` note and the sealed
    /// `status.json`; null elsewhere or when none. Absent on a resent seal, which still parses.
    Seal {
        reason: SealReason,
        outlived: Option<Outlived>,
    },
    /// Null recorder only: exit 0 without sealing.
    Restart,
}

/// A line from the recorder to the watchdog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "line", rename_all = "snake_case")]
pub enum ToWatchdog {
    /// Proxy accepting, part file open.
    Ready,
    /// The seal is durable.
    Sealed,
    /// The last heartbeat answer (PRD-06's `ServerWord`, seam S10), forwarded by the recorder:
    /// `seconds_left` resets the deadline, `stop` ends the run.
    Server(ServerWord),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::{self, Env, builtin};
    use crate::mask::ValueSource;
    use serde_json::to_value;
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    /// The first line carries the whole start facts. With the largest built-in connector's
    /// plan, a minted CA and 32 shaped fake values of 120 bytes it stays under the line limit,
    /// so the recorder can read it (point 1: a start line that does not fit records nothing).
    #[test]
    fn the_start_line_fits() {
        let dir = tempfile::tempdir().unwrap();
        let ca = RunCa::mint(&["example.org".to_string()]).unwrap().export();
        let mut longest = 0;
        for (id, _) in builtin::CONNECTORS {
            let c = connector::load(id, None).unwrap();
            let env = Env {
                vars: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                workspace: dir.path().join("w"),
                resume: true,
                run_start: SystemTime::now(),
                goalcup_home: dir.path().join("g"),
                server_host: "ingest.example.org".into(),
            };
            let plan = connector::plan(&c, &env, &dir.path().join("homes").join(id));
            let facts = StartFacts {
                run: format!("r{}", "a".repeat(16)),
                entry: format!("e{}", "b".repeat(16)),
                segment: 2,
                entry_dir: dir.path().join("runs/r/e"),
                deadline: Some(u64::MAX),
                server_time: Some(u64::MAX),
                local_wall: u64::MAX,
                workspace: env.workspace.clone(),
                connector: to_value(&plan.connector).unwrap(),
                harness_version: Some("1.2.3 (fake)".into()),
                keep: true,
                build: Some("f".repeat(40)),
                hosts: to_value(&plan.hosts).unwrap(),
                hosts_sha256: "0".repeat(64),
                native: to_value(&plan.native).unwrap(),
                mask_rules: to_value(&plan.mask_rules).unwrap(),
                values: to_value(
                    (0..32)
                        .map(|i| SecretValue::new(ValueSource::Env, vec![b'x' + (i % 2); 120]))
                        .collect::<Vec<_>>(),
                )
                .unwrap(),
                ca: ca.clone(),
                prev: Some(Prev::Cause(Cause::NullMode {
                    exits: 3,
                    last: Some(Exit::Signal(9)),
                })),
                harness: Some(HarnessFacts {
                    pid: u32::MAX,
                    program: "/usr/local/bin/harness".into(),
                    started: "2026-11-07T00:00:00Z".into(),
                    start_id: u64::MAX,
                }),
                null_since: Some(u64::MAX),
                notes: plan.notes.clone(),
            };
            let line = serde_json::to_vec(&ToRecorder::Start(Box::new(facts))).unwrap();
            longest = longest.max(line.len() + 1);
        }
        assert!(longest <= MAX_LINE, "{longest}");
    }

    /// The `prev` shapes the `recorder_down` GAP tail is written from (PRD-02 section 8).
    #[test]
    fn prev_json_shapes() {
        let cases = [
            (Prev::Exit(Exit::Code(75)), r#"{"code":75}"#),
            (Prev::Exit(Exit::Signal(9)), r#"{"signal":9}"#),
            (
                Prev::Cause(Cause::WatchdogLost),
                r#"{"cause":"watchdog_lost"}"#,
            ),
            (
                Prev::Cause(Cause::NullMode {
                    exits: 3,
                    last: Some(Exit::Code(70)),
                }),
                r#"{"cause":"null_mode","exits":3,"last":{"code":70}}"#,
            ),
            (
                Prev::Cause(Cause::NullMode {
                    exits: 3,
                    last: None,
                }),
                r#"{"cause":"null_mode","exits":3}"#,
            ),
        ];
        for (prev, json) in cases {
            assert_eq!(serde_json::to_string(&prev).unwrap(), json);
            assert_eq!(serde_json::from_str::<Prev>(json).unwrap(), prev);
        }
        // A `null_mode` cause always counts its exits.
        assert!(serde_json::from_str::<Prev>(r#"{"cause":"null_mode"}"#).is_err());
    }

    /// Ledger R3a: a null boot's space never puts the normal try after the wait's end; with a
    /// seal pending (a failed null seal) the space stands.
    #[test]
    fn a_null_space_never_passes_the_wait() {
        let r = |hold: u64| Restarts {
            until: Some(at(30)),
            hold: Some(at(hold)),
            ..Restarts::default()
        };
        let mut late = r(61);
        late.end_null(false);
        assert_eq!((late.hold, late.until), (Some(at(30)), None));
        let mut early = r(20);
        early.end_null(false);
        assert_eq!(early.hold, Some(at(20)));
        let mut sealing = r(61);
        sealing.end_null(true);
        assert_eq!(sealing.hold, Some(at(61)));
    }

    fn at(s: u64) -> Mono {
        Mono(s * SEC)
    }

    /// Runs each (start, end) in seconds through one [`Loop`]: the waits in seconds.
    fn waits(runs: &[(u64, u64)]) -> Vec<Option<u64>> {
        let mut l = Loop::default();
        let wait = |(s, e): &(u64, u64)| l.exited(at(*s), at(*e)).map(|w| w / SEC);
        runs.iter().map(wait).collect()
    }

    /// Ledger R3a, the contract: the first two fast exits restart at once, the third within
    /// 10 minutes switches to null mode, and the waits are 30, 60, 120, then 300 s, never more.
    #[test]
    fn next_wait_r3a_waits() {
        let mut runs = vec![(1_000, 1_010), (1_010, 1_011), (1_011, 1_070)];
        let mut t = 1_100;
        for _ in 0..6 {
            runs.push((t, t + 1));
            t += 400;
        }
        let w = [None, None, Some(30), Some(60), Some(120), Some(300)];
        assert_eq!(waits(&runs), [&w[..], &[Some(300); 3]].concat());
    }

    /// A recorder up 60 s resets the count and the waits; exits at hour 5 and hour 140 never
    /// add up; three fast exits spread over more than 10 minutes do not switch.
    #[test]
    fn next_wait_resets() {
        let fast3 = [(0, 1), (1, 2), (2, 3)];
        let reset = [&fast3[..], &[(3, 63), (63, 64), (64, 65), (65, 66)]].concat();
        assert_eq!(
            waits(&reset),
            [None, None, Some(30), None, None, None, Some(30)]
        );
        let (h5, h140) = (5 * 3_600, 140 * 3_600);
        let far = [(h5, h5 + 1), (h140, h140 + 1), (h140 + 1, h140 + 2)];
        assert_eq!(waits(&far), [None; 3]);
        let spread = [(0, 1), (300, 301), (602, 603)];
        assert_eq!(waits(&spread), [None; 3]);
    }
}
