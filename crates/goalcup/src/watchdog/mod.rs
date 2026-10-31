//! The watchdog (PRD-02 section 8, seam S8): the `goalcup start` or `continue` process. It holds
//! the listen socket (fd 3 of the recorder) and one control socket (fd 4), restarts a dead
//! recorder at once on the same port, and ends the harness at the deadline and on seal.
//!
//! Here: the control lines (S8) and the start facts (S17), [`run`] with the restart loop, the
//! terminal [`shield`], and what the watchdog binds, [`spawn`]s and ends. Null mode, the deadline,
//! the server word and `recover` come in the next slice.

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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
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
/// `{"cause":"null_mode","exits":n,"last":{"code":n}}`. Null on boot 0. The next boot's
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
    /// Null mode, which always follows fast exits: how many, and how the last recorder ended.
    NullMode { exits: u64, last: Exit },
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
    /// `$GOALCUP_HOME`, for `upload::UploadCtx.home` (the queue of older entries). Not the
    /// harness's isolated home.
    pub home: PathBuf,
    /// `LaunchPlan.connector` (S3 `connector::ConnectorRef`: id, name, sha256, source), for the
    /// `start` keys `connector`, `connector_source` and `connector_sha256`.
    pub connector: ConnectorRef,
    /// `LaunchPlan.harness_version`; null when it could not be read.
    pub harness_version: Option<String>,
    /// `GOALCUP_KEEP_PARTS=1`.
    pub keep: bool,
    /// The 40-hex commit the binary was built from, or null. The version is the binary's own
    /// `CARGO_PKG_VERSION`.
    pub build: Option<String>,
    pub hosts: HostList,
    /// SHA-256 of the host list file as read (the embedded list or a `--hosts` file), 64 hex.
    pub hosts_sha256: String,
    pub native: NativeSources,
    pub mask_rules: Vec<MaskRule>,
    /// The exact mask values. Never logged.
    pub values: Vec<SecretValue>,
    pub ca: CaPem,
    pub prev: Option<Prev>,
    pub harness: Option<HarnessFacts>,
    /// For a null recorder: wall time null mode began, unix ms.
    pub null_since: Option<u64>,
    /// `LaunchPlan.notes`, written as events on boot 0 only.
    pub notes: Vec<serde_json::Value>,
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

    let mut lp = Loop::default();
    let recorder_pid = Arc::new(AtomicU32::new(0));
    let mut boot = match Boot::start(recorder_exe, &listener, &facts) {
        Ok(boot) => boot,
        Err(_) => {
            remove_key(&rec);
            return failed("recorder_spawn");
        }
    };
    lp.started(Mono::now());
    recorder_pid.store(boot.child.id(), Ordering::SeqCst);
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
    let spare = {
        let recorder_pid = Arc::clone(&recorder_pid);
        move |pid: u32| pid == recorder_pid.load(Ordering::SeqCst)
    };

    let mut ending: Option<JoinHandle<()>> = None;
    let mut after: Option<JoinHandle<()>> = None;
    let mut exit_line: Option<ToRecorder> = None;
    let mut seal: Option<ToRecorder> = None;
    let mut asked = false;
    let mut sealed = false;
    while !sealed {
        match boot.next() {
            Some(ToWatchdog::Ready) => {
                // A restarted boot: what the dead one was told and did not answer, again.
                for l in [&exit_line, &seal].into_iter().flatten() {
                    boot.send(l);
                }
            }
            Some(ToWatchdog::Sealed) => sealed = true,
            None => {}
        }
        if SEAL_ASKED.swap(false, Ordering::SeqCst)
            && ending.is_none()
            && let (Some(pid), Some(h)) = (harness_pid, &facts.harness)
        {
            asked = true;
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
            let reason = if asked {
                SealReason::SealCommand
            } else {
                SealReason::HarnessExit
            };
            let l = ToRecorder::Seal {
                reason,
                outlived: spawn::outlived(harness_start, &spare),
            };
            boot.send(&l);
            seal = Some(l);
        }
        if !sealed && matches!(boot.child.try_wait(), Ok(Some(_)) | Err(_)) {
            if boot.drain() {
                sealed = true;
                continue;
            }
            // Ledger R3: the next recorder at once, same listener, same CA, never capped.
            let prev = boot.child.try_wait().ok().flatten().map(exit_of);
            let up_ms = lp.exited(Mono::now());
            facts.prev = prev.map(Prev::Exit);
            line(
                &dir,
                "recorder_restart",
                &[
                    ("boot", Fact::N(lp.boots.len() as u64)),
                    ("up_ms", Fact::N(up_ms)),
                ],
            );
            boot = loop {
                match Boot::start(recorder_exe, &listener, &facts) {
                    Ok(next) => break next,
                    Err(e) => {
                        line(&dir, "recorder_spawn_failed", &[("io", Fact::Io(e.kind()))]);
                        thread::sleep(spawn::TICK);
                    }
                }
            };
            lp.started(Mono::now());
            recorder_pid.store(boot.child.id(), Ordering::SeqCst);
        }
    }
    for h in [ending, after].into_iter().flatten() {
        let _ = h.join();
    }
    if let Some(shield) = shield.as_mut() {
        shield.harness_exited();
    }
    drop(plan);
    match exit.flatten() {
        Some(status) => ExitCode::from(code_of(status)),
        None => ExitCode::from(NOT_STARTED),
    }
}

/// Each recorder's start and exit on boot time (ledger R3). Slice B's null mode reads it to
/// space out boots that crash at once.
#[derive(Default)]
struct Loop {
    boots: Vec<(Mono, Option<Mono>)>,
}

impl Loop {
    fn started(&mut self, at: Mono) {
        self.boots.push((at, None));
    }

    /// Notes the last recorder's exit; how long it was up, in ms.
    fn exited(&mut self, at: Mono) -> u64 {
        self.boots.last_mut().map_or(0, |(start, end)| {
            *end = Some(at);
            at.0.saturating_sub(start.0) / 1_000_000
        })
    }
}

/// One recorder and its end of a fresh control pair. A pair per boot: a dead recorder's unread
/// lines never reach the next one, and its exit closes the pair.
struct Boot {
    child: Child,
    to: UnixStream,
    lines: mpsc::Receiver<Option<ToWatchdog>>,
}

impl Boot {
    /// Spawns the recorder and writes the first line.
    fn start(exe: &Path, listener: &TcpListener, facts: &StartFacts) -> io::Result<Boot> {
        let (to, theirs) = UnixStream::pair()?;
        let reader = to.try_clone()?;
        to.set_write_timeout(Some(Duration::from_secs(10)))?;
        let child = spawn::recorder(exe, false, listener, &theirs)?;
        drop(theirs);
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || read_lines(reader, &tx));
        let mut boot = Boot { child, to, lines };
        boot.send(&ToRecorder::Start(Box::new(facts.clone())));
        Ok(boot)
    }

    /// One control line. Never logged; a failed write is the restart loop's to notice.
    fn send(&mut self, l: &ToRecorder) {
        if let Ok(mut text) = serde_json::to_vec(l) {
            text.push(b'\n');
            let _ = self.to.write_all(&text);
        }
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

/// Reads the recorder's lines until the pair closes, then sends `None`. A line that does not
/// parse (a variant this build does not know) is skipped.
fn read_lines(stream: UnixStream, tx: &mpsc::Sender<Option<ToWatchdog>>) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match (&mut reader)
            .take(MAX_LINE as u64)
            .read_until(b'\n', &mut buf)
        {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if let Ok(l) = serde_json::from_slice(&buf)
                    && tx.send(Some(l)).is_err()
                {
                    return;
                }
            }
        }
    }
    let _ = tx.send(None);
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
    // The `server` line (the last heartbeat answer) carries PRD-06's `ServerWord` (seam S10) as
    // `Server(upload::ServerWord)` once that stub is on integration.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::{self, Env, builtin};
    use crate::mask::ValueSource;
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
                home: env.goalcup_home.clone(),
                connector: plan.connector.clone(),
                harness_version: Some("1.2.3 (fake)".into()),
                keep: true,
                build: Some("f".repeat(40)),
                hosts: plan.hosts.clone(),
                hosts_sha256: "0".repeat(64),
                native: plan.native.clone(),
                mask_rules: plan.mask_rules.clone(),
                values: (0..32)
                    .map(|i| SecretValue::new(ValueSource::Env, vec![b'x' + (i % 2); 120]))
                    .collect(),
                ca: ca.clone(),
                prev: Some(Prev::Cause(Cause::NullMode {
                    exits: 3,
                    last: Exit::Signal(9),
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
                    last: Exit::Code(70),
                }),
                r#"{"cause":"null_mode","exits":3,"last":{"code":70}}"#,
            ),
        ];
        for (prev, json) in cases {
            assert_eq!(serde_json::to_string(&prev).unwrap(), json);
            assert_eq!(serde_json::from_str::<Prev>(json).unwrap(), prev);
        }
        for bad in [
            r#"{"cause":"null_mode"}"#,
            r#"{"cause":"watchdog_lots"}"#,
            r#"{"cause":"null_mode","exits":3}"#,
        ] {
            assert!(serde_json::from_str::<Prev>(bad).is_err(), "{bad}");
        }
    }
}
