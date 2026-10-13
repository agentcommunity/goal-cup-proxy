//! `goalcup start --goal-file` — a protocol-v5 local capture run: the preflight, the daemon's
//! capture, the launch through the recording proxy, and the local seal.
//!
//! No grant, no arming, no server. The harness is launched headless through the recording
//! proxy, which runs inside this process, and the run ends with a local seal (see `capture`
//! below for the order). A plain `goalcup start` without `--goal-file` was the V1 transcript
//! path, which this tree no longer carries (D48); clap marks `--goal-file` required, and
//! `started` refuses the rest.
//!
//! # The abandon window
//!
//! A failure before the harness is launched leaves no entry and the participant may retry.
//! That promise is only true if something removes it, so a failure in that window calls
//! `client.abandon`, takes the local record with it, and kills the child if one was already
//! spawned. **The local record is removed only where the daemon confirmed the abandon**: an
//! abandon the daemon refused leaves it standing and says so.
//!
//! # `--dev` is the one door, here as everywhere
//!
//! [`crate::preflight::DEV_DOOR`] names `goalcup start --dev` as one of the two places a
//! human types it. It admits a connector directory this build does not pin; it never removes
//! a check, and a run that came through it says so on stdout and in the entry record.

use crate::CliError;
use crate::arming::GateOutcome;
use crate::connector::HarnessConnector;
use crate::entries::{EntryRecord, remove_entry, write_entry};
use crate::key_scan::FragmentMatcher;
use crate::launch::{
    CaptureStderr, LaunchPlan, LaunchScope, ScopeMarker, StderrUse, StdoutUse, StreamPlan,
    child_secret_for_harness, gated_handle, spawn_gated, tee_marked_capture_with_identity,
    tee_scoped_capture_with_rule, tee_stderr_capture_with_rule,
};
use crate::preflight::Level;
use crate::proxy::api::{CapturePath, CapturePolicy};
use crate::proxy::connect::{
    ApprovedHost, DEFAULT_ADMISSION_TIMEOUT, DEFAULT_CONNECT_ATTEMPT_TIMEOUT, ProxyCredential,
};
use crate::proxy::ingest::{IngestSecret, IngestSink, SinkConfig, connect_sink};
use crate::proxy::tls::UpstreamRoots;
use crate::proxy::{MAX_CONNECTION_TASKS, ProxyConfig, ProxyHandle};
use crate::retry::{retry_busy, retry_busy_entry, retry_busy_process};
use goalcup_protocol::capture::CaptureSink;
use goalcup_protocol::control::{
    CaptureFinishReport, CaptureGoalRequest, CaptureLaunchRequest, CaptureWorkspaceInfo,
    CaptureWorkspaceRequest, DaemonClient, FINISH_COMMIT_RESERVE, LOCAL_CAPTURE_FINISH_BUDGET,
    ProxyCounters, SignedStatus,
};
use goalcup_protocol::harness::{CaptureSpec, ConfigHome, RunHome};
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration, termination_bound};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::sync::Arc;
use std::time::Duration;

/// What one `goalcup start` was asked to do.
pub struct StartOptions {
    /// The connector id — `claude-code-cli` or `codex-cli`.
    pub harness: String,
    /// `--dev`: admit a rulebook this build does not pin, and say so on stdout. The one
    /// door ([`crate::preflight::DEV_DOOR`]), typed by a human and never set by a shipped
    /// code path.
    pub dev: bool,
    /// `--goal-file`: a local protocol-v5 capture run, launched headless with this file's
    /// text as its one goal. Required: `None` is refused.
    pub goal_file: Option<PathBuf>,
    /// `--capture-reserve-bytes`: the chain bytes the daemon reserves before a v5 launch.
    pub capture_reserve_bytes: u64,
}

/// What one `goalcup start` did.
#[derive(Debug)]
pub struct StartOutcome {
    pub entry_id: String,
    pub gate: GateOutcome,
    /// How the harness exited, where this command launched one and waited for it.
    pub exit_status: Option<ExitStatus>,
}

/// The testable half: everything except resolving a real client and a real config dir.
///
/// `connectors`, `cfg`, `env` and `out` are parameters and not lookups for the reason the
/// rest of this crate takes them that way — a command that read the participant's
/// environment could not be tested without mutating this process's, and a command that
/// printed with `println!` could not be asserted at all.
pub fn start_with(
    client: &(dyn DaemonClient + Sync),
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &StartOptions,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    // The flag is process-global — a signal handler has nowhere else to put what it saw — so
    // what this start was told does not reach the next one: a process that runs `start`
    // twice, and every test binary that raises a real `SIGINT` at itself.
    //
    // A guard and not a trailing call: `started` below runs a scoped watcher thread and
    // returns from a dozen `?`s, and a line at the bottom of this function is a promise about
    // every one of those paths that only reading to the end can check.
    let _clear = crate::arming::clear_on_drop();
    started(client, connectors, cfg, env, opts, None, out)
}

/// [`start_with`] with the proxy trusting `roots` upstream instead of the public web roots,
/// so a test can stand up a TLS upstream of its own. A shipped build has no such door.
#[cfg(feature = "test-support")]
pub fn start_with_upstream_roots(
    client: &(dyn DaemonClient + Sync),
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &StartOptions,
    roots: UpstreamRoots,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    let _clear = crate::arming::clear_on_drop();
    started(client, connectors, cfg, env, opts, Some(roots), out)
}

/// The whole of it, so [`start_with`] has one place to put the flag back from.
fn started(
    client: &(dyn DaemonClient + Sync),
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &StartOptions,
    roots: Option<UpstreamRoots>,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    // 0. The Ctrl-C handler, before anything exists, so that everything created below is
    //    inside the window a Ctrl-C unmakes. It is installed once per process and it does
    //    nothing but set a flag; the four places that read it are the four moments this
    //    command can act on one.
    crate::arming::watch_for_interrupt();

    if let Some(goal_file) = &opts.goal_file {
        let roots = roots.unwrap_or_else(UpstreamRoots::webpki);
        return capture(client, connectors, cfg, env, opts, goal_file, roots, out);
    }

    // D48: the plain `goalcup start` (no `--goal-file`) was the V1 transcript path, which this
    // tree no longer carries. Clap marks `--goal-file` required, so this is a backstop a caller
    // of `start_with` can still reach.
    let _ = (client, connectors, cfg, env, out);
    Err(CliError::Refused(
        "`goalcup start` needs a goal file: \
         `goalcup start --harness <id> --goal-file <file>`."
            .to_string(),
    ))
}

/// One line, because a participant who pressed Ctrl-C is not reading a page.
///
/// Rung 1 and not rung 4: nothing refused this run, the participant stopped it, and a script
/// branching on the number must not read a cancellation as an arming gate that closed.
fn interrupted_refusal() -> CliError {
    CliError::Refused(
        "`goalcup start` was interrupted before the run began, so the entry is being \
         abandoned and nothing is left behind."
            .to_string(),
    )
}

/// A Ctrl-C after the entry was registered and before the harness was released: the parked
/// child is ended without running, and the entry is sealed with no launch.
fn interrupted_before_launch() -> CliError {
    CliError::Refused(
        "`goalcup start` was interrupted before the harness started, so nothing ran; the \
         entry is being sealed without a launch."
            .to_string(),
    )
}

/// The cleanup every abandon takes: ask the daemon, and remove the local record only where it
/// agreed. `sentence` is what the participant is told when both went through. Answers
/// whether the daemon abandoned the entry.
fn abandon_because(
    client: &(dyn DaemonClient + Sync),
    cfg: &Path,
    entry_id: &str,
    reason: &str,
    sentence: &str,
    out: &mut dyn std::io::Write,
) -> bool {
    // A Ctrl-C may be what started this abandon; it has been acted on, and must not cut the
    // abandon's own busy retries short.
    crate::arming::close_window();
    let abandoned = retry_busy_entry(|| client.abandon(entry_id, reason));
    let confirmed = abandoned.is_ok();
    let removed = match &abandoned {
        Ok(()) => remove_entry(cfg, entry_id),
        // Not attempted rather than attempted and failed, which is why this is `Ok`: the
        // sentence below names the abandon that was refused, and a second complaint about a
        // file nobody tried to delete would be noise on top of it.
        Err(_) => Ok(()),
    };
    // A write that failed while reporting a failure is not worth a second failure on top of
    // the first — the same call `main.rs` makes when it cannot print the error itself.
    let _ = match (&abandoned, &removed) {
        (Ok(()), Ok(())) => say(out, sentence),
        _ => {
            let mut said = format!("Entry {entry_id} could not be cleaned up:");
            if let Err(e) = abandoned {
                said.push_str(&format!("\n  the daemon refused the abandon: {e}"));
                said.push_str(
                    "\n  this machine's note of the entry has been kept, because the tracker \
                     may still be holding it.",
                );
            }
            if let Err(e) = removed {
                said.push_str(&format!("\n  the local record is still there: {e}"));
            }
            said.push_str("\n  `goalcup entries` shows what is left.");
            say(out, &said)
        }
    };
    confirmed
}

/// One line to the participant, or the write error naming the stream.
fn say(out: &mut dyn std::io::Write, line: &str) -> Result<(), CliError> {
    writeln!(out, "{line}").map_err(|source| CliError::Io {
        path: "stdout".to_string(),
        source,
    })
}

// ------------------------------------------------------------------ v5 local capture

/// Bounds the proxy's start (the sender's admission of its first producer included), and
/// separately the read of the entry's own admission counts that follows it.
const CAPTURE_STARTUP: Duration = Duration::from_secs(5);

/// What the gate is given for its two steps: the parked child's pid to arrive and, once it is
/// released, its acknowledgement that it is running. Separate from [`CAPTURE_STARTUP`] because it
/// bounds a different thing, even though both are five seconds.
const GATE_READY_WITHIN: Duration = Duration::from_secs(5);

/// How long Codex's launch waits for its scope identity (the `thread.started` event on stdout)
/// before recording the launch without one.
const CODEX_SCOPE_WAIT: Duration = Duration::from_secs(120);

/// A named point in the capture launch where a test can hold this process (CONTRACTS.md, C6 local finish budget and capture spawn gate).
///
/// Compiled only under `test-support`: a shipped build has the no-op below and nothing reads
/// the environment. When `GOALCUP_TEST_PAUSE_DIR` holds `<point>.arm`, this writes
/// `<point>.reached` (this process's pid) and waits for `<point>.go`, polled every 20 ms with a
/// 60 s safety release. It pauses; it replaces nothing.
#[cfg(feature = "test-support")]
fn pause_at(point: &'static str) {
    let Some(dir) = std::env::var_os("GOALCUP_TEST_PAUSE_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    if !dir.join(format!("{point}.arm")).exists() {
        return;
    }
    let _ = std::fs::write(
        dir.join(format!("{point}.reached")),
        std::process::id().to_string(),
    );
    let until = std::time::Instant::now() + Duration::from_secs(60);
    while !dir.join(format!("{point}.go")).exists() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(not(feature = "test-support"))]
#[inline(always)]
fn pause_at(_point: &'static str) {}

/// The soft open-file limit `goalcup start` asks for: a relayed connection holds two
/// descriptors, and the rest is room for the runtime, the sender and this process.
const OPEN_FILES_WANTED: u64 = 2 * MAX_CONNECTION_TASKS + 256;

/// `<cfg>/capture-homes/<connector>`: the one isolated harness home every capture run of a
/// connector shares (D39). It is created here and never seeded.
const CAPTURE_HOMES_DIR: &str = "capture-homes";

/// What every phase of one capture run is given and none changes.
struct CaptureRun<'a> {
    client: &'a (dyn DaemonClient + Sync),
    cfg: &'a Path,
    env: &'a BTreeMap<String, String>,
    opts: &'a StartOptions,
    goal_file: &'a Path,
    connector: &'a HarnessConnector,
    profile: &'a CaptureSpec,
}

/// Everything a capture run decides, and every refusal it makes, before anything is created.
struct CapturePrep {
    goal: Vec<u8>,
    /// The provider key, if the profile names one. Taken (and so dropped) once the tees hold what
    /// they need: nothing after that reads the key from here.
    secret: Option<crate::launch::ChildSecret>,
    streams: StreamPlan,
    scope: LaunchScope,
    /// The scan of this run's artifacts for fragments of that key (D41), derived once, in
    /// memory, from the same bytes the child receives.
    matcher: Option<FragmentMatcher>,
    executable: crate::launch::PinnedExecutable,
    /// The harness home every run of this connector shares (D39); `None` when each run makes its
    /// own inside its run directory.
    shared_home: Option<SharedCaptureHome>,
    /// The lock that keeps a second run off that shared home, held until this run's finish
    /// returns (the struct lives to the end of the capture) and released by the OS if this
    /// process dies.
    _shared_home_lock: Option<std::fs::File>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    kind: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SharedHomeObservation {
    leaf: FileIdentity,
    target: FileIdentity,
    canonical_target: PathBuf,
}

/// D39's exact shared-home leaf spelling plus the identities that made it safe at prepare time.
#[derive(Clone)]
struct SharedCaptureHome {
    alias: PathBuf,
    owner: u32,
    observed: SharedHomeObservation,
}

impl SharedCaptureHome {
    fn load(alias: &Path, owner: u32) -> Result<Self, CliError> {
        Ok(Self {
            alias: alias.to_path_buf(),
            owner,
            observed: observe_shared_capture_home(alias, owner)?,
        })
    }

    fn recheck(&self) -> Result<PathBuf, CliError> {
        // SAFETY: reading the effective uid has no preconditions or memory effects.
        let effective_uid = unsafe { libc::geteuid() };
        if effective_uid != self.owner {
            return Err(shared_home_refusal(
                &self.alias,
                "the launcher's effective user changed after preflight",
            ));
        }
        let observed = observe_shared_capture_home(&self.alias, effective_uid)?;
        if observed != self.observed {
            return Err(shared_home_refusal(
                &self.alias,
                "its leaf, target, or canonical target changed after preflight",
            ));
        }
        Ok(self.alias.clone())
    }
}

fn file_identity(metadata: &std::fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        #[allow(
            clippy::unnecessary_cast,
            reason = "mode_t is u16 on macOS; the cast widens there even though it is a no-op on Linux glibc"
        )]
        kind: metadata.mode() & libc::S_IFMT as u32,
    }
}

/// D39: runs on one shared harness home are serialized. Two Codex runs on one `CODEX_HOME`
/// that both refresh the subscription token fail with `refresh_token_reused` and log the
/// participant out of that home. A second start is refused plainly, before anything is made.
///
/// An exclusive `flock` on `<capture-homes>/<connector>.lock`, beside the home rather than in
/// it, so the harness never sees it. The descriptor is close-on-exec (std's default), so the
/// harness never inherits the lock either.
#[cfg(unix)]
fn lock_shared_home(config_home: &Path, connector: &str) -> Result<std::fs::File, CliError> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let path = config_home
        .parent()
        .expect("a capture-home parent")
        .join(format!("{connector}.lock"));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|source| CliError::Io {
            path: path.display().to_string(),
            source,
        })?;
    // SAFETY: `file` owns a live descriptor; `flock` reads nothing else.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let source = std::io::Error::last_os_error();
        if source.kind() == std::io::ErrorKind::WouldBlock {
            return Err(CliError::Refused(format!(
                "a {connector} run is already recording on this machine, and runs share one \
                 {connector} home, so they go one at a time. Wait for it to finish, or stop it \
                 and run `goalcup seal`. Nothing was created."
            )));
        }
        return Err(CliError::Io {
            path: path.display().to_string(),
            source,
        });
    }
    Ok(file)
}

fn shared_home_refusal(alias: &Path, why: &str) -> CliError {
    CliError::Refused(format!(
        "the shared capture home `{}` is unsafe: {why}. Nothing was created.",
        alias.display()
    ))
}

#[cfg(unix)]
fn observe_shared_capture_home(
    alias: &Path,
    owner: u32,
) -> Result<SharedHomeObservation, CliError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let leaf = std::fs::symlink_metadata(alias).map_err(|_| {
        shared_home_refusal(
            alias,
            "its exact leaf cannot be inspected without following it",
        )
    })?;
    if !leaf.is_dir() && !leaf.file_type().is_symlink() {
        return Err(shared_home_refusal(
            alias,
            "its exact leaf is neither a directory nor a directory alias",
        ));
    }
    if leaf.uid() != owner {
        return Err(shared_home_refusal(
            alias,
            "its exact leaf is not owned by the effective user",
        ));
    }
    let leaf_identity = file_identity(&leaf);
    let canonical_target = std::fs::canonicalize(alias).map_err(|_| {
        shared_home_refusal(
            alias,
            "its exact leaf is dangling or has an unresolvable loop",
        )
    })?;
    if canonical_target.to_str().is_none() {
        return Err(shared_home_refusal(
            alias,
            "its canonical target is not a UTF-8 path",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let target = options.open(&canonical_target).map_err(|_| {
        shared_home_refusal(
            alias,
            "its canonical target cannot be opened as a no-follow directory",
        )
    })?;
    let target = target.metadata().map_err(|_| {
        shared_home_refusal(alias, "its opened canonical target cannot be inspected")
    })?;
    if !target.is_dir() {
        return Err(shared_home_refusal(
            alias,
            "its canonical target is not a directory",
        ));
    }
    if target.uid() != owner {
        return Err(shared_home_refusal(
            alias,
            "its canonical target is not owned by the effective user",
        ));
    }
    if target.mode() & 0o7777 != 0o700 {
        return Err(shared_home_refusal(
            alias,
            "its canonical target must have exact mode 0700 with no special, group, or world bits",
        ));
    }
    let target_identity = file_identity(&target);
    let leaf_after = std::fs::symlink_metadata(alias)
        .map_err(|_| shared_home_refusal(alias, "its exact leaf changed while it was inspected"))?;
    let canonical_after = std::fs::canonicalize(alias)
        .map_err(|_| shared_home_refusal(alias, "its canonical target changed while inspected"))?;
    if file_identity(&leaf_after) != leaf_identity || canonical_after != canonical_target {
        return Err(shared_home_refusal(
            alias,
            "its exact leaf or canonical spelling changed while it was inspected",
        ));
    }
    Ok(SharedHomeObservation {
        leaf: leaf_identity,
        target: target_identity,
        canonical_target,
    })
}

/// Test seam for the ownership decision without requiring a privileged `chown` fixture.
#[cfg(feature = "test-support")]
pub fn validate_shared_capture_home_for_uid(path: &Path, uid: u32) -> Result<(), CliError> {
    SharedCaptureHome::load(path, uid).map(|_| ())
}

/// A capture entry from the moment its child is forked: from here the entry is sealed, never
/// abandoned, and this command's own output failing ends nothing. A released harness is waited
/// for and the entry sealed either way. A parked child that is never released never ran, so there
/// is nothing to wait for.
///
/// Every way out of the later phases that does not run to the harness's own end goes through
/// [`Forked::unlaunched`] or [`Forked::sealed_launch`], so a run that ends early is sealed the
/// same way and, if it holds a provider key, is marked unscanned the same way.
struct Forked<'a> {
    client: &'a (dyn DaemonClient + Sync),
    cfg: &'a Path,
    info: CaptureWorkspaceInfo,
    run: RunDir,
    proxied: Proxied,
    /// The run was given a provider key, so its directory is scanned and judged.
    scanning: bool,
    /// This machine's record of the entry, with no pid yet.
    template: EntryRecord,
}

impl Forked<'_> {
    /// This machine's record of the entry, with the harness's pid once there is one.
    fn record(&self, last_pid: Option<u32>) -> EntryRecord {
        EntryRecord {
            last_pid,
            ..self.template.clone()
        }
    }

    /// End a run whose harness never ran or was never told about: the entry is sealed within
    /// the one budget, and the run is marked unscanned. Returns the error that ended it.
    fn unlaunched(self, error: CliError, out: &mut dyn std::io::Write) -> CliError {
        // Whatever ended the launch, including a Ctrl-C, the seal below runs its full retries.
        crate::arming::close_window();
        finish_unlaunched(self.client, &self.info.entry_id, self.proxied, out);
        note_unscanned(self.scanning, &self.run.dir, out);
        error
    }

    /// The root ruling under D40: the entry was sealed while its harness was being launched, so
    /// this command ends the child it just released, says so, and answers the committed seal.
    fn sealed_launch(
        self,
        child: Child,
        handed: &ProcessHandle,
        tees: &Tees,
        out: &mut dyn std::io::Write,
    ) -> CliError {
        note_unscanned(self.scanning, &self.run.dir, out);
        let Forked {
            client,
            info,
            proxied,
            ..
        } = self;
        let end = crate::launch::end_released_child(child, handed);
        let _ = say(out, &end.announcement(&info.entry_id));
        let deadline = std::time::Instant::now() + LOCAL_CAPTURE_FINISH_BUDGET;
        let _ = finish(client, &info.entry_id, proxied, deadline, None, None, out);
        tees.finish(deadline, out);
        // Worded from what was observed, never more: a native child that could not be checked is
        // not "nothing is running".
        CliError::Refused(end.refusal(&info.entry_id))
    }
}

/// What the gate holds for the phases after the fork.
struct Gate {
    plan: LaunchPlan,
    goal_stream: goalcup_protocol::capture::StreamKey,
    gated: crate::launch::GatedSpawn,
    scope_deadline: std::time::Instant,
}

/// The tees on a released child's streams.
struct Tees {
    /// Codex's stdout tee, which also reads the thread id.
    stdout: Option<crate::launch::CaptureStdout>,
    /// A keyed `stdout_json` child's thread id, read by the marked tee, which reads stdout
    /// once for both jobs and reports its end through `marked`.
    keyed_scope: Option<crate::launch::ScopeReceiver>,
    /// Hermes' two tees, which also scan and read the session marker.
    marked: Option<CaptureStderr>,
    /// Only a stderr-line identity is sent with the finish. A credential may require the same
    /// two tees while the identity itself was preassigned.
    late_scope: bool,
}

impl Tees {
    /// The receiver of the stdout JSON identity, from whichever tee reads it.
    fn scope(&self) -> Option<&crate::launch::ScopeReceiver> {
        self.stdout
            .as_ref()
            .map(|tee| &tee.scope)
            .or(self.keyed_scope.as_ref())
    }

    /// Let the tees finish while the entry seals, within what remains of the one finish deadline.
    fn finish(&self, deadline: std::time::Instant, out: &mut dyn std::io::Write) {
        report_tee_finish(self.stdout.as_ref().map(|tee| tee.finish(deadline)), out);
        report_tee_finish(self.marked.as_ref().map(|tee| tee.finish(deadline)), out);
    }
}

/// A harness registered, released and tee'd.
struct Released<'a> {
    forked: Forked<'a>,
    handed: ProcessHandle,
    child: Child,
    spawned_at: goalcup_protocol::capture::CaptureTime,
    tees: Tees,
    goal_stream: goalcup_protocol::capture::StreamKey,
    scope_deadline: std::time::Instant,
}

/// A harness that has exited.
struct Exited<'a> {
    forked: Forked<'a>,
    tees: Tees,
    status: Result<ExitStatus, CliError>,
    /// Linux: the refusal the command ends with, after the seal, when the sweep left a process
    /// running or could not read `/proc` after it had signalled one.
    left_running: Option<CliError>,
}

/// A v5 local capture run.
///
/// The order is the daemon's: the entry is created with its chain physically reserved, the
/// proxy starts in this process and its sender hands the daemon a producer, the entry's own
/// status must show that producer admitted, the goal is recorded once, and only then is the
/// harness spawned; its process is registered before its launch is recorded. When it exits
/// the proxy stops, the seal carries the proxy's counters, and the sender drains.
///
/// Every failure before the spawn abandons the entry, which removes its chain, reservation
/// and still-empty workspace, and removes this run's directory. From the spawn on nothing is
/// abandoned: a harness this command could not hand over is killed, and the entry is sealed,
/// so what it did stays on the record.
///
/// Five phases, whose boundaries are the window rules above: [`prepare`] (every refusal that
/// precedes anything created), [`create_and_gate`] (the abandon window, which ends at the fork),
/// [`register_and_release`], [`run_harness`] and [`finish_and_scan`] (the entry is sealed from the
/// fork on).
#[allow(clippy::too_many_arguments)]
fn capture(
    client: &(dyn DaemonClient + Sync),
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &StartOptions,
    goal_file: &Path,
    roots: UpstreamRoots,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    let goal = read_goal(goal_file)?;
    let c = crate::connector::load_harness(connectors, &opts.harness, opts.dev)?;
    let profile = &c.loaded.manifest.capture;
    let cx = CaptureRun {
        client,
        cfg,
        env,
        opts,
        goal_file,
        connector: &c,
        profile,
    };
    let mut prep = prepare(&cx, goal, out)?;
    let (forked, gate) = create_and_gate(&cx, roots, &prep, out)?;
    let released = register_and_release(forked, gate, &mut prep, out)?;
    let exited = run_harness(&cx, prep.scope, released, out)?;
    finish_and_scan(exited, prep.matcher.as_ref(), out)
}

/// Phase one: every refusal that comes before anything is created.
fn prepare(
    cx: &CaptureRun<'_>,
    goal: Vec<u8>,
    out: &mut dyn std::io::Write,
) -> Result<CapturePrep, CliError> {
    // The launch plan checks again at its own boundary, but that boundary follows every
    // capture allocation. Reject unusable goal bytes here so no shared home, run directory,
    // seeded home, daemon entry or proxy can precede this preflight decision.
    crate::launch::checked_goal(&goal)?;
    let CaptureRun {
        cfg,
        env,
        opts,
        goal_file,
        connector: c,
        profile,
        ..
    } = *cx;
    // The rulebook the proxy's host rules and the launch come from, through the one door.
    let pin = crate::preflight::harness_pin_gate(c, opts.dev);
    match pin.level {
        Level::Fail => {
            return Err(CliError::Refused(format!(
                "`goalcup start` refuses this run:\n{}",
                pin.render().trim_end()
            )));
        }
        Level::Warn => say(out, pin.render().trim_end())?,
        _ => {}
    }
    // The provider key, if the profile names one, is read from this process's own environment
    // before anything is created, and is handed to the child alone (D40). A missing or unusable
    // key is a refusal with no daemon request behind it.
    let secret = child_secret_for_harness(profile, env)?;
    // What is done with the child's output, and where its native scope comes from, is decided
    // here once: a child handed a provider key (Hermes, or any profile naming a credential) has
    // both streams tee'd, scanned and withheld when they hold a fragment.
    let streams = StreamPlan::for_spec(profile)?;
    let scope = LaunchScope::for_spec(profile)?;
    let matcher = match secret.as_ref() {
        Some(secret) => Some(FragmentMatcher::new(secret.expose())?),
        None => None,
    };
    let executable = crate::launch::capture_executable(c, profile, env)?;
    // The provider hosts answer a TCP connect before anything is created (the proxy module's
    // preflight; it names a firewall holding this binary when they do not).
    crate::proxy::preflight::upstreams_reachable(profile.hosts.iter().map(|h| h.host.as_str()))
        .map_err(|unreachable| CliError::Refused(unreachable.to_string()))?;
    // A per-run home is made inside the run directory below. A run directory beside a goal file
    // inside a Git work tree is refused before anything is created: it would hold what the
    // harness wrote, which can include a fragment of the provider key (D41).
    let per_run = profile.home.config_home == ConfigHome::PerRun;
    if per_run {
        crate::per_run_home::refuse_inside_git_work_tree(&beside_goal(goal_file))?;
        crate::per_run_home::refuse_synced_folder_under(
            &beside_goal(goal_file),
            env.get("HOME").map(Path::new),
        )?;
    }
    let mut shared_home_lock = None;
    let shared_home = if per_run {
        None
    } else {
        let config_home = cfg.join(CAPTURE_HOMES_DIR).join(&c.loaded.manifest.id);
        private_dir(config_home.parent().expect("a capture-home parent"), true)?;
        shared_home_lock = Some(lock_shared_home(&config_home, &c.loaded.manifest.id)?);
        match std::fs::symlink_metadata(&config_home) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match private_dir(&config_home, false) {
                    // A concurrent `goalcup start` made it between the look and the create;
                    // `SharedCaptureHome::load` below judges that directory like any other.
                    Err(CliError::Io { source, .. })
                        if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                    made => made?,
                }
            }
            Err(_) => {
                return Err(shared_home_refusal(
                    &config_home,
                    "its exact leaf cannot be inspected",
                ));
            }
        }
        // SAFETY: reading the effective uid has no preconditions or memory effects.
        let owner = unsafe { libc::geteuid() };
        Some(SharedCaptureHome::load(&config_home, owner)?)
    };
    Ok(CapturePrep {
        goal,
        secret,
        streams,
        scope,
        matcher,
        executable,
        shared_home,
        _shared_home_lock: shared_home_lock,
    })
}

/// Phase two, the abandon window: the run directory, the entry, the proxy, the recorded goal and
/// the parked child. Up to the fork a failure unmakes everything made here; the fork ends the
/// window.
fn create_and_gate<'a>(
    cx: &CaptureRun<'a>,
    roots: UpstreamRoots,
    prep: &CapturePrep,
    out: &mut dyn std::io::Write,
) -> Result<(Forked<'a>, Gate), CliError> {
    let CaptureRun {
        client,
        cfg,
        env,
        opts,
        goal_file,
        connector: c,
        profile,
    } = *cx;
    // Catch a replacement since prepare before writing even the temporary run directory.
    let shared_home = prep
        .shared_home
        .as_ref()
        .map(SharedCaptureHome::recheck)
        .transpose()?;
    let run = RunDir::create(goal_file, profile.home.run_home, env)?;
    let config_home = match shared_home {
        Some(home) => home,
        // Seeded from the connector's own files, which the manifest gate has pinned.
        None => {
            match crate::per_run_home::create_per_run_home(&run.dir, &profile.home, &c.loaded.seeds)
            {
                Ok(home) => home,
                Err(e) => {
                    let _ = run.remove();
                    return Err(e);
                }
            }
        }
    };

    // The last alias/target identity check is directly before the create request. Preserve the
    // exact D39 leaf spelling in the request, local entry and child environment.
    if let Some(home) = prep.shared_home.as_ref() {
        pause_at("capture-shared-home-pre-create");
        if let Err(error) = home.recheck() {
            let _ = run.remove();
            return Err(error);
        }
    }

    let request = CaptureWorkspaceRequest {
        workspace: run.workspace.display().to_string(),
        profile_id: c.loaded.manifest.id.clone(),
        connector_dir_sha256: c.loaded.connector_dir_sha256.clone(),
        reserve_bytes: opts.capture_reserve_bytes,
        run_home: run.home.display().to_string(),
        config_home: config_home.display().to_string(),
    };
    let info = match retry_busy(|| client.create_capture_workspace(&request)) {
        Ok(info) => info,
        Err(e) => {
            let _ = run.remove();
            return Err(e.into());
        }
    };
    let session_id = fresh_uuid();
    let template = EntryRecord {
        entry_id: info.entry_id.clone(),
        process_generation: info.process_generation,
        connector_id: c.loaded.manifest.id.clone(),
        run_public_key: info.run_public_key.clone(),
        workspace: run.workspace.display().to_string(),
        // A local capture has no artifact root or status page this machine is told about.
        artifact_root: String::new(),
        harness_home: config_home.display().to_string(),
        session_id: session_id.clone(),
        last_pid: None,
        status_url: String::new(),
        // No daemon stamp comes back for a capture entry, so this machine's clock orders it.
        created_at: crate::launch::launched_at_now(),
        dev: opts.dev,
    };

    // Up to the spawn, a failure unmakes everything above.
    let launched = write_entry(cfg, &template).and_then(|()| {
        say(out, &format!("entry {}", info.entry_id))?;
        say(out, &format!("run directory: {}", run.dir.display()))?;
        raise_open_file_limit(out)?;
        let proxied = Proxied::start(&info, profile, &run, roots)?;
        admitted(client, &info)?;
        let mut vars = goalcup_protocol::template::TemplateVars::new();
        vars.set("workspace", run.workspace.display().to_string());
        vars.set("session_id", session_id.clone());
        let plan = crate::launch::build_capture_launch(
            c,
            &vars,
            &run.workspace,
            &crate::launch::CaptureLaunch {
                goal: &prep.goal,
                executable: &prep.executable,
                run_home: &run.home,
                config_home: &config_home,
                proxy_url: &proxied.proxy_url,
                ca_certificate: proxied.proxy.ca_certificate(),
                parent_env: env,
            },
        )?;
        // Offered once. `busy` is the daemon's refusal before it reads a request and is
        // asked again; any other failure aborts the launch, because an answer that did not
        // arrive may have followed an accepted goal and a second would be a second
        // instruction.
        let goal_request = CaptureGoalRequest {
            bytes: prep.goal.clone(),
        };
        let goal_stream = retry_busy(|| client.record_capture_goal(&info.entry_id, &goal_request))?;
        if crate::arming::interrupted() {
            return Err(interrupted_refusal());
        }
        let scope_deadline = std::time::Instant::now() + CODEX_SCOPE_WAIT;
        // The only way a capture child is spawned (D40): forked and parked before `exec`,
        // and released only once its registration is durable. Its pipes are the one
        // manifest-derived `StreamPlan` decision.
        let gated = spawn_gated(
            &plan,
            prep.streams.pipes(),
            GATE_READY_WITHIN,
            prep.secret.as_ref(),
        )?;
        Ok((proxied, plan, goal_stream, gated, scope_deadline))
    });
    let (proxied, plan, goal_stream, gated, scope_deadline) = match launched {
        Ok(launched) => launched,
        Err(error) => {
            unmake(client, cfg, &info.entry_id, &run, &error, out);
            return Err(error);
        }
    };
    let forked = Forked {
        client,
        cfg,
        info,
        run,
        proxied,
        scanning: prep.matcher.is_some(),
        template,
    };
    let gate = Gate {
        plan,
        goal_stream,
        gated,
        scope_deadline,
    };
    Ok((forked, gate))
}

/// Phase three, from the fork: take the parked child's handle, register it durably, only then
/// release it, and start the tees on its streams. Every way out is [`Forked::unlaunched`]: the
/// child never ran, or it is registered and the seal answers for it.
fn register_and_release<'a>(
    forked: Forked<'a>,
    gate: Gate,
    prep: &mut CapturePrep,
    out: &mut dyn std::io::Write,
) -> Result<Released<'a>, CliError> {
    pause_at("capture-gate-forked");
    let Gate {
        plan,
        goal_stream,
        gated,
        scope_deadline,
    } = gate;
    let handed = match gated_handle(&plan, &gated, &crate::launch::launched_at_now()) {
        Ok(handed) => handed,
        Err(e) => {
            gated.abort();
            return Err(forked.unlaunched(e, out));
        }
    };
    let registration = ProcessRegistration {
        generation: forked.info.process_generation,
        handle: handed.clone(),
    };
    let (client, entry_id) = (forked.client, &forked.info.entry_id);
    if let Err(error) = retry_busy_process(|| client.register_process(entry_id, &registration)) {
        // Definite or ambiguous, the child is never released: it ends at its gate without
        // having run a byte of the harness, and the seal reads a registration that names a
        // process that never exec'd.
        gated.abort();
        return Err(forked.unlaunched(CliError::Control(error), out));
    }
    pause_at("capture-gate-registered");
    if let Err(e) = crate::launch::recheck_pinned_executable(&prep.executable) {
        gated.abort();
        return Err(forked.unlaunched(e, out));
    }
    // The last look at the flag. A Ctrl-C during the spawn, the registration or the re-hash
    // above reached only this process and the parked child, which rides it out; released now,
    // the harness would run the whole goal the participant asked to stop.
    if crate::arming::interrupted() {
        gated.abort();
        return Err(forked.unlaunched(interrupted_before_launch(), out));
    }
    let mut spawned = match gated.release() {
        Ok(spawned) => spawned,
        // No harness ran, so no `Launch` is written, and the entry is sealed.
        Err(e) => return Err(forked.unlaunched(e, out)),
    };
    // The harness owns every Ctrl-C from here (it shares the terminal), and the launch record
    // and the seal below must not give up on their first `busy` because of one.
    crate::arming::close_window();
    // Observed on this process's monotonic clock, the one the proxy's first producer stamps
    // its frames with under its own ID.
    let mut spawned_at = crate::proxy::metadata::capture_time();
    spawned_at.clock_id = forked.info.producer_id.clone();
    let stdout_rule = match prep.streams.stdout.clone() {
        StdoutUse::IdentityJson(rule) => Some(rule),
        StdoutUse::Inherit | StdoutUse::Forward => None,
    };
    // Hermes: both streams forwarded verbatim and scanned for key fragments, stderr read for
    // the session marker (the late scope of the finish).
    let marker_rule = match prep.streams.stderr.clone() {
        StderrUse::IdentityLine(rule) => Some(rule),
        StderrUse::Inherit | StderrUse::Forward => None,
    };
    let (stdout, keyed_scope, marked) = if prep.streams.scan_for_key {
        // A keyed child: both streams scanned, and a stdout JSON identity, if any, read by the
        // same stdout reader (one tee per stream).
        match tee_marked_capture_with_identity(
            &mut spawned,
            prep.secret.as_ref(),
            stdout_rule,
            marker_rule,
        ) {
            Ok((scope, tee)) => (None, scope, Some(tee)),
            // The tee ended the child it could not forward; it is registered, so the seal
            // answers for it.
            Err(e) => return Err(forked.unlaunched(e, out)),
        }
    } else {
        let stdout = match stdout_rule {
            Some(rule) => match tee_scoped_capture_with_rule(&mut spawned, rule) {
                Ok(stdout) => Some(stdout),
                Err(e) => return Err(forked.unlaunched(e, out)),
            },
            None => None,
        };
        let marked = match marker_rule {
            Some(rule) => match tee_stderr_capture_with_rule(&mut spawned, rule) {
                Ok(tee) => Some(tee),
                Err(e) => return Err(forked.unlaunched(e, out)),
            },
            None => None,
        };
        (stdout, None, marked)
    };
    // The tees hold what they need; nothing after this reads the key from here.
    drop(prep.secret.take());
    let child = spawned.child;
    if let Err(e) = write_entry(forked.cfg, &forked.record(Some(handed.pid))) {
        let _ = say(
            out,
            &format!("  (this machine's own note of the harness's pid could not be updated: {e})"),
        );
    }
    Ok(Released {
        forked,
        handed,
        child,
        spawned_at,
        tees: Tees {
            stdout,
            keyed_scope,
            marked,
            late_scope: prep.scope == LaunchScope::AtFinish,
        },
        goal_stream,
        scope_deadline,
    })
}

/// Phase four: record the launch and wait for the harness. A seal that raced in, or a launch
/// the daemon found sealed, ends the child (D40); any other failure to record leaves the trace
/// incomplete and never stops the run.
fn run_harness<'a>(
    cx: &CaptureRun<'_>,
    scope: LaunchScope,
    released: Released<'a>,
    out: &mut dyn std::io::Write,
) -> Result<Exited<'a>, CliError> {
    let Released {
        forked,
        handed,
        mut child,
        spawned_at,
        tees,
        goal_stream,
        scope_deadline,
    } = released;
    // Linux: what the harness leaves behind is this process's own child (a subreaper, `main.rs`),
    // and is swept between the root's exit and the seal. Armed first, while the root is certainly
    // unreaped: Codex's scope wait below reaps a root that exits before its scope line.
    let sweep = crate::subreaper::Sweep::for_root(handed.pid);
    let (client, entry_id) = (forked.client, forked.info.entry_id.clone());
    // A seal that raced in between the registration and the release cannot have ended a
    // harness that was still parked, so one that is sealed now is a harness running on a
    // sealed entry: this command ends the child it just released (D40).
    match entry_is_sealed(client, &forked.info) {
        Ok(true) => return Err(forked.sealed_launch(child, &handed, &tees, out)),
        Ok(false) => {}
        Err(e) => {
            let _ = say(
                out,
                &format!(
                    "  (the entry's status could not be read after the harness was released: {e})"
                ),
            );
        }
    }
    pause_at("capture-gate-released");
    let native_scope = match scope {
        LaunchScope::Preassigned => Some(forked.template.session_id.clone()),
        LaunchScope::FromStdout => tees.scope().and_then(|receiver| {
            crate::launch::wait_native_scope(&mut child, receiver, scope_deadline)
        }),
        // Hermes' scope is read from its stderr after the proxy stops, and sent with the seal.
        LaunchScope::AtFinish | LaunchScope::None => None,
    };
    let launch = CaptureLaunchRequest {
        goal_stream,
        spawned_at,
        harness_version: cx
            .profile
            .resolved_executable(goalcup_protocol::harness::PlatformKey::current())
            .version
            .to_string(),
        continuation: cx.profile.continuation.clone(),
        native_scope,
    };
    // A launch the daemon did not record leaves the trace incomplete; it never stops the run,
    // unless the entry is sealed: then the harness has no entry to run on (D40). A `Conflict`
    // on an open entry is a capture-side fault, and it never ends the child.
    if let Err(e) = retry_busy(|| client.record_capture_launch(&entry_id, &launch)) {
        if matches!(entry_is_sealed(client, &forked.info), Ok(true)) {
            return Err(forked.sealed_launch(child, &handed, &tees, out));
        }
        let _ = say(
            out,
            &format!(
                "  (the launch could not be recorded, so the trace will read incomplete: {e})"
            ),
        );
    }
    let _ = say(
        out,
        "Recording. The harness runs headless; this command ends with it.",
    );
    let status = child.wait().map_err(|source| CliError::Io {
        path: handed.program.clone(),
        source,
    });
    // What was swept is said on stdout; what may still be running goes to stderr now, and makes
    // the command exit 1 after the seal (`finish_and_scan`), so a script that reads only the exit
    // code still learns of it. The trace is unchanged either way.
    let swept = sweep.map(|sweep| sweep.run(Duration::from_secs(handed.grace_seconds)));
    if let Some(swept) = &swept {
        if let Some(line) = swept.swept_line() {
            let _ = say(out, &line);
        }
        for line in swept.warnings() {
            eprintln!("{line}");
        }
    }
    Ok(Exited {
        forked,
        tees,
        status,
        left_running: swept.and_then(|swept| swept.refusal(&entry_id)),
    })
}

/// Phase five: stop the proxy and seal within the one finish budget, let the tees drain, and scan
/// the run for fragments of the provider key. The scan never changes how the harness or the seal
/// ended, and a run that is not clean is a failed row whose directory stays local.
fn finish_and_scan(
    exited: Exited<'_>,
    matcher: Option<&FragmentMatcher>,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    pause_at("capture-finish-started");
    let Exited {
        forked,
        tees,
        status,
        left_running,
    } = exited;
    let Forked {
        client,
        cfg,
        info,
        run,
        proxied,
        ..
    } = forked;
    let deadline = std::time::Instant::now() + LOCAL_CAPTURE_FINISH_BUDGET;
    let finished = finish(
        client,
        &info.entry_id,
        proxied,
        deadline,
        if tees.late_scope {
            tees.marked.as_ref()
        } else {
            None
        },
        Some(cfg),
        out,
    );
    tees.finish(deadline, out);
    // How the harness ended is the participant's to see: the exit code of a run that was recorded
    // is not this command's own (`main.rs`), so it is said here.
    if let Ok(status) = &status {
        let _ = say(out, &format!("harness exited: {}.", describe_exit(status)));
    }
    // After the finish report, before this run's artifacts are called anything: the key scan.
    let trace_scanned = finished
        .as_ref()
        .ok()
        .map(|report| report.trace_dir.as_str());
    // What the verdict is about is fixed before the scan reads anything (H-18a), so a change
    // made while or after it ran reads as stale, never as covered.
    let binding = matcher
        .is_some()
        .then(|| crate::key_scan::Binding::capture(&run.dir, trace_scanned.map(Path::new)));
    let verdict =
        matcher.map(|matcher| key_verdict(matcher, tees.marked.as_ref(), &run.dir, trace_scanned));
    let failed = match (&verdict, &binding) {
        (Some(outcome), Some(binding)) => {
            match crate::key_scan::write_verdict_bound(&run.dir, outcome, binding) {
                Ok(()) if outcome.is_clean() => {
                    let _ = say(
                        out,
                        &format!(
                            "key scan: clean ({} binds it to this run).",
                            run.dir.join("key-scan.txt").display()
                        ),
                    );
                    None
                }
                Ok(()) => Some(key_scan_sentence(outcome, &run.dir)),
                // A verdict that was not written cannot be read as clean by whatever retains the
                // run, so the row is failed whatever the scan found.
                Err(e) => Some(format!(
                    "The key-scan verdict {} could not be written ({e}), so the row is failed. This \
                 run's directory stays local: do not commit, upload or share it.",
                    run.dir.join("key-scan.txt").display()
                )),
            }
        }
        _ => None,
    };
    match (finished, failed) {
        (Ok(_), None) => {}
        (Ok(_), Some(sentence)) => return Err(CliError::Refused(sentence)),
        (Err(error), failed) => {
            if let Some(sentence) = failed {
                let _ = say(out, &sentence);
            }
            return Err(error);
        }
    }
    if let Some(refusal) = left_running {
        return Err(refusal);
    }
    Ok(StartOutcome {
        entry_id: info.entry_id.clone(),
        gate: GateOutcome::NotRequired,
        exit_status: Some(status?),
    })
}

/// A harness's end in words: its exit code, or the signal that ended it.
fn describe_exit(status: &ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit code {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("ended by signal {signal}");
        }
    }
    status.to_string()
}

/// A run that ended before its finish (a parked harness never released, a sealed entry, a tee
/// that could not start) was not scanned: it leaves `key-scan.txt` saying `incomplete`, so no
/// retention step reads a missing file as an absence of trouble. Only a run given a provider key.
fn note_unscanned(scanning: bool, run_dir: &Path, out: &mut dyn std::io::Write) {
    if !scanning {
        return;
    }
    let outcome = crate::key_scan::ScanOutcome::Incomplete {
        reason: "the run ended before its finish, so it was not scanned",
    };
    if let Err(e) = crate::key_scan::write_verdict(run_dir, &outcome) {
        let _ = say(
            out,
            &format!("  (the key-scan verdict could not be written: {e})"),
        );
    }
}

/// The most bytes the key scan reads of one run: its directory and its trace together.
const KEY_SCAN_BOUND: u64 = 512 * 1024 * 1024;

/// The scan of one Hermes run, as one outcome: the run directory and the trace (text and decoded
/// streams) when the finish returned a report, the run directory alone and an incomplete verdict
/// when it did not, and a live hit on the harness's own output counted as a hit. Only `Clean`
/// is clean.
fn key_verdict(
    matcher: &FragmentMatcher,
    marked: Option<&CaptureStderr>,
    run_dir: &Path,
    trace_dir: Option<&str>,
) -> crate::key_scan::ScanOutcome {
    use crate::key_scan::ScanOutcome;
    let scanned = match trace_dir {
        Some(trace) => {
            crate::key_scan::scan_run(matcher, &[run_dir], Path::new(trace), KEY_SCAN_BOUND)
        }
        None => crate::key_scan::scan_tree(matcher, &[run_dir], KEY_SCAN_BOUND).merge(
            ScanOutcome::Incomplete {
                reason: "the finish did not complete, so the trace was not scanned",
            },
        ),
    };
    if marked.is_some_and(CaptureStderr::key_hit) {
        return scanned.merge(ScanOutcome::Hit {
            relative_paths: vec![PathBuf::from("(the harness's own output)")],
        });
    }
    scanned
}

/// What the participant is told when the scan is not clean: fixed words and the verdict file's
/// path, never a path inside the run or any fragment.
fn key_scan_sentence(outcome: &crate::key_scan::ScanOutcome, run_dir: &Path) -> String {
    let what = match outcome {
        crate::key_scan::ScanOutcome::Hit { .. } => {
            "A fragment of the provider key was found in this run's artifacts".to_string()
        }
        crate::key_scan::ScanOutcome::Incomplete { reason } => {
            format!("The key-fragment scan of this run could not read everything ({reason})")
        }
        crate::key_scan::ScanOutcome::Clean => String::new(),
    };
    format!(
        "{what}, so the row is failed ({} says where). This run's directory stays local: do not \
         commit, upload or share it.",
        run_dir.join("key-scan.txt").display()
    )
}

/// Seal an entry whose harness never ran (or was never told about), within the one budget.
fn finish_unlaunched(
    client: &(dyn DaemonClient + Sync),
    entry_id: &str,
    proxied: Proxied,
    out: &mut dyn std::io::Write,
) {
    let deadline = std::time::Instant::now() + LOCAL_CAPTURE_FINISH_BUDGET;
    let _ = finish(client, entry_id, proxied, deadline, None, None, out);
}

/// Whether the entry's own signed status shows it sealed. The read waits for the entry's run
/// lock, which a finish holds from its first stage to its commit, so a seal in flight is
/// answered after it commits.
fn entry_is_sealed(
    client: &(dyn DaemonClient + Sync),
    info: &CaptureWorkspaceInfo,
) -> Result<bool, CliError> {
    Ok(verified_status(client, info)?.payload.sealed_at.is_some())
}

/// The entry's own status, a busy tracker waited out, and only if it is signed by the key the
/// entry's creation named and is about this entry.
fn verified_status(
    client: &(dyn DaemonClient + Sync),
    info: &CaptureWorkspaceInfo,
) -> Result<SignedStatus, CliError> {
    let document = retry_busy(|| client.status(&info.entry_id))?;
    if document.verify_with(&info.run_public_key).is_err()
        || document.payload.entry_id != info.entry_id
    {
        return Err(CliError::Internal(format!(
            "the status of entry {} is not signed by the key its creation named",
            info.entry_id
        )));
    }
    Ok(document)
}

/// Say what became of the tees' last bytes: nothing when they were all forwarded, and what is
/// known when they were not. `result` is what a tee's finish returned within the one finish
/// deadline (`None` when the run has no such tee). Expiry is the finish budget as a whole, not
/// the drain share inside it, and buffered output delivery is then unconfirmed.
fn report_tee_finish(result: Option<std::io::Result<bool>>, out: &mut dyn std::io::Write) {
    match result {
        None | Some(Ok(true)) => {}
        Some(Ok(false)) => {
            let _ = say(
                out,
                &format!(
                    "Final harness output did not finish within the {}-second finish budget; its \
                     tail may be missing.",
                    LOCAL_CAPTURE_FINISH_BUDGET.as_secs()
                ),
            );
        }
        Some(Err(error)) => {
            let _ = say(
                out,
                &format!(
                    "Final harness output could not be drained: {error}; its tail may be missing."
                ),
            );
        }
    }
}

/// The goal file's bytes, read once, with the bound applied while reading. What the goal
/// may hold is the launch builder's and the daemon's to judge; no refusal here quotes it.
fn read_goal(path: &Path) -> Result<Vec<u8>, CliError> {
    use std::io::Read;
    let io = |source| CliError::Io {
        path: path.display().to_string(),
        source,
    };
    let limit = goalcup_protocol::capture::MAX_GOAL_BYTES;
    let mut goal = Vec::new();
    std::fs::File::open(path)
        .map_err(io)?
        .take(limit as u64 + 1)
        .read_to_end(&mut goal)
        .map_err(io)?;
    if goal.len() > limit {
        return Err(CliError::Refused(format!(
            "the goal in {} is longer than {limit} bytes. Nothing was created.",
            path.display()
        )));
    }
    Ok(goal)
}

/// The directory a run's own directory is made in: the goal file's.
fn beside_goal(goal_file: &Path) -> PathBuf {
    goal_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

/// This run's own directory, made beside the goal file: `workspace/` (the daemon makes it),
/// `home/` (the harness's `HOME`) and `ca/` (the proxy's CA certificate).
struct RunDir {
    dir: PathBuf,
    workspace: PathBuf,
    home: PathBuf,
    ca: PathBuf,
}

impl RunDir {
    fn create(
        goal_file: &Path,
        home: RunHome,
        env: &BTreeMap<String, String>,
    ) -> Result<RunDir, CliError> {
        let beside = beside_goal(goal_file);
        let beside = std::fs::canonicalize(&beside).map_err(|source| CliError::Io {
            path: beside.display().to_string(),
            source,
        })?;
        let dir = beside.join(format!("goalcup-{:016x}", rand::random::<u64>()));
        private_dir(&dir, false)?;
        let run = RunDir {
            workspace: dir.join("workspace"),
            home: dir.join("home"),
            ca: dir.join("ca"),
            dir,
        };
        let made = private_dir(&run.home, false)
            .and_then(|()| private_dir(&run.ca, false))
            .and_then(|()| match home {
                RunHome::Empty => Ok(()),
                // The home's one entry: the user's keychain, so a subscription login is found
                // and nothing else of theirs is (D38).
                RunHome::KeychainLink => {
                    let keychains = crate::per_run_home::home_from(env)?.join("Library/Keychains");
                    let library = run.home.join("Library");
                    private_dir(&library, false)?;
                    symlink(&keychains, &library.join("Keychains"))
                }
                _ => Err(CliError::Refused(
                    "the harness uses a run-home mode this goalcup does not support".into(),
                )),
            });
        if let Err(e) = made {
            let _ = run.remove();
            return Err(e);
        }
        Ok(run)
    }

    /// Only ever before the spawn, when nothing but this command has written here.
    fn remove(&self) -> std::io::Result<()> {
        std::fs::remove_dir_all(&self.dir)
    }
}

fn private_dir(dir: &Path, recursive: bool) -> Result<(), CliError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(|source| CliError::Io {
        path: dir.display().to_string(),
        source,
    })
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> Result<(), CliError> {
    std::os::unix::fs::symlink(target, link).map_err(|source| CliError::Io {
        path: link.display().to_string(),
        source,
    })
}

/// A fresh version-4 UUID, the form a pre-assigned harness session id takes.
fn fresh_uuid() -> String {
    let bits = (rand::random::<u128>() & !(0xf << 76) & !(0x3 << 62)) | (0x4 << 76) | (0x2 << 62);
    let hex = format!("{bits:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Raise the soft open-file limit toward the hard one, far enough for the proxy's connection
/// bound: launchd starts a process at 256, and at two descriptors a connection the proxy
/// would stop accepting near 120.
#[cfg(unix)]
fn raise_open_file_limit(out: &mut dyn std::io::Write) -> Result<(), CliError> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes the one struct it is handed and reads nothing else.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return Ok(());
    }
    let wanted = (OPEN_FILES_WANTED as libc::rlim_t).min(limit.rlim_max);
    if limit.rlim_cur >= wanted {
        return Ok(());
    }
    let raised = libc::rlimit {
        rlim_cur: wanted,
        rlim_max: limit.rlim_max,
    };
    // SAFETY: `setrlimit` reads the one struct it is handed.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } != 0 {
        say(
            out,
            &format!(
                "  (the open-file limit stays at {}, so the proxy may refuse connections past \
                 about half of it)",
                limit.rlim_cur
            ),
        )?;
    }
    Ok(())
}

/// The recording proxy and its IPC sender, on one runtime thread in this process. When this
/// process dies they die with it, and the daemon reads the open producer as an unknown tail.
///
/// The fields drop in order: the proxy's tasks, then the sender's, then the runtime.
struct Proxied {
    proxy: ProxyHandle,
    sink: Arc<IngestSink>,
    /// `http://goalcup:<credential>@127.0.0.1:<port>`, handed to the harness and nowhere else.
    proxy_url: String,
    runtime: tokio::runtime::Runtime,
}

impl Proxied {
    fn start(
        info: &CaptureWorkspaceInfo,
        profile: &CaptureSpec,
        run: &RunDir,
        roots: UpstreamRoots,
    ) -> Result<Proxied, CliError> {
        let refused = |what: &str, e: &dyn std::fmt::Display| {
            CliError::Refused(format!(
                "the recording proxy did not start ({what}: {e}), so the harness is not \
                 launched."
            ))
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| refused("runtime", &e))?;
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| refused("listener", &e))?;
        let address = listener.local_addr().map_err(|e| refused("listener", &e))?;
        let credential = ProxyCredential::generate();
        let proxy_url = credential.proxy_url(address);
        let sink = {
            let _inside = runtime.enter();
            connect_sink(SinkConfig::new(
                info.ingest_addr,
                info.entry_id.clone(),
                IngestSecret::new(info.ingest_secret.clone()),
                info.producer_id.clone(),
                LOCAL_CAPTURE_FINISH_BUDGET,
            ))
            .map_err(|e| refused("sender", &e))?
        };
        let config = ProxyConfig {
            listener,
            ca_dir: run.ca.clone(),
            approved: approved_hosts(profile),
            auth_hosts: profile.auth_hosts.clone(),
            credential,
            upstream_roots: roots,
            producer: info.producer_id.clone(),
            sink: Arc::clone(&sink) as Arc<dyn CaptureSink>,
            startup_deadline: CAPTURE_STARTUP,
            // The stop is the finish's first step and is measured from its own call, which is
            // the finish deadline's start, so its bound is the drain share: it ends by
            // `drain_end`, leaving the commit reserve to the daemon's seal.
            shutdown_deadline: LOCAL_CAPTURE_FINISH_BUDGET - FINISH_COMMIT_RESERVE,
            admission_timeout: DEFAULT_ADMISSION_TIMEOUT,
            connect_attempt_timeout: DEFAULT_CONNECT_ATTEMPT_TIMEOUT,
            max_connections: MAX_CONNECTION_TASKS as usize,
        };
        let proxy = runtime
            .block_on(crate::proxy::start(config))
            .map_err(|e| refused("proxy", &e))?;
        Ok(Proxied {
            proxy,
            sink,
            proxy_url,
            runtime,
        })
    }
}

/// The profile's hosts as the proxy's host rules: exact paths, then prefixes, each with the
/// host's protocol hint.
fn approved_hosts(profile: &CaptureSpec) -> Vec<ApprovedHost> {
    profile
        .hosts
        .iter()
        .map(|h| {
            let exact = h.evidence_paths.iter().map(|p| (p, false));
            let prefix = h.prefix_paths.iter().map(|p| (p, true));
            let paths = exact
                .chain(prefix)
                .map(|(path, prefix)| CapturePath {
                    path: path.clone(),
                    prefix,
                    protocol_hint: h.protocol_hint.clone(),
                })
                .collect();
            ApprovedHost {
                host: h.host.clone(),
                policy: CapturePolicy { paths },
            }
        })
        .collect()
}

/// Wait, within [`CAPTURE_STARTUP`], for the entry's own status to show the proxy's producer
/// admitted: one producer consumed and none refused. A started proxy is not that proof.
fn admitted(
    client: &(dyn DaemonClient + Sync),
    info: &CaptureWorkspaceInfo,
) -> Result<(), CliError> {
    let until = std::time::Instant::now() + CAPTURE_STARTUP;
    loop {
        let document = verified_status(client, info)?;
        match (
            document.payload.producers_consumed,
            document.payload.admissions_refused,
        ) {
            (Some(1), Some(0)) => return Ok(()),
            (_, Some(refused)) if refused > 0 => {
                return Err(CliError::Refused(format!(
                    "the tracker did not admit the proxy's capture producer ({refused} \
                     refused), so nothing would be recorded and the harness is not launched."
                )));
            }
            _ => {}
        }
        if std::time::Instant::now() >= until {
            return Err(CliError::Refused(format!(
                "the tracker did not show the proxy's capture producer admitted within {}s, so \
                 the harness is not launched.",
                CAPTURE_STARTUP.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The one `proxy admission:` line: `accepted` and `intercepted` always, every other counter only
/// when it is not zero, as `name=count` words in a fixed order. A run in which nothing went wrong
/// is two numbers; one in which something did names it.
fn proxy_admission_line(
    h: &crate::proxy::ProxyHealth,
    a: &crate::proxy::AdmissionFailureCounts,
) -> String {
    let always = [("accepted", h.accepted), ("intercepted", h.intercepted)];
    let when_nonzero = [
        ("fail_open", h.tunneled_fail_open),
        ("auth_refused", h.admission_refused_auth),
        ("failed", h.admission_failed),
        ("malformed", a.malformed),
        ("head_too_large", a.head_too_large),
        ("not_connect", a.not_connect),
        ("timeout", a.timeout),
        ("client_io", a.client_io),
        ("upstream_connect", a.upstream_connect),
        ("upstream_connect_dns", a.upstream_connect_dns),
        (
            "upstream_connect_no_addresses",
            a.upstream_connect_no_addresses,
        ),
        (
            "upstream_connect_tcp_last_refused",
            a.upstream_connect_tcp_last_refused,
        ),
        (
            "upstream_connect_tcp_last_timeout",
            a.upstream_connect_tcp_last_timeout,
        ),
        (
            "upstream_connect_tcp_last_permission_denied",
            a.upstream_connect_tcp_last_permission_denied,
        ),
        (
            "upstream_connect_tcp_last_other",
            a.upstream_connect_tcp_last_other,
        ),
        ("upstream_tls", a.upstream_tls),
        ("client_tls", a.client_tls),
        ("alpn_incompatible", a.alpn_incompatible),
        ("upstream_outages", h.upstream_outages),
    ];
    let mut line = String::from("proxy admission:");
    for (name, count) in always
        .into_iter()
        .chain(when_nonzero.into_iter().filter(|(_, count)| *count != 0))
    {
        line.push_str(&format!(" {name}={count}"));
    }
    line
}

/// Stop the proxy, seal with its counters, then let the sender drain, and say how the run
/// ended, all within `deadline` (the one local-finish budget, taken by the caller immediately
/// before this call); nothing here waits on the harness.
///
/// The one deadline of a capture run's finish is `LOCAL_CAPTURE_FINISH_BUDGET`, taken when the
/// harness has exited and immediately before the proxy's stop: nothing runs between the two. The
/// proxy's stop gets its drain share (the budget less `FINISH_COMMIT_RESERVE`), the daemon's seal
/// its own request's remainder, and the sender's drain and the scoped stdout what is left of the
/// same deadline (CONTRACTS.md, "Amendment 2026-10-02 (C6 local finish budget, H-10 4a)").
///
/// A Hermes child's session identity is read between the two: after the proxy has stopped and
/// before the seal is sent, by [`crate::capture_tee::marker_wait_end`]'s deadline, which leaves
/// the daemon's native read its share of the drain. The identity, if exactly one distinct one was
/// printed, rides on the seal request; any other answer sends none.
fn finish(
    client: &(dyn DaemonClient + Sync),
    entry_id: &str,
    proxied: Proxied,
    deadline: std::time::Instant,
    marked: Option<&CaptureStderr>,
    reconnect: Option<&Path>,
    out: &mut dyn std::io::Write,
) -> Result<CaptureFinishReport, CliError> {
    let Proxied {
        proxy,
        sink,
        runtime,
        ..
    } = proxied;
    let stopped = runtime.block_on(proxy.stop());
    pause_at("capture-finish-proxy-stopped");
    let h = &stopped.health;
    let a = &h.admission_failures;
    // Diagnostics are fixed counters only. A failed stdout write must not prevent sealing.
    let _ = say(out, &proxy_admission_line(h, a));
    for line in crate::proxy::outage::finish_lines(h.upstream_outages, &h.outage_windows) {
        let _ = say(out, &line);
    }
    let counters = ProxyCounters {
        tunneled_fail_open: h.tunneled_fail_open,
        opaque_fallbacks: h.opaque_fallbacks,
        capture_table_full: h.capture_table_full,
        cut_unsettled: h.cut_unsettled,
        unterminated: stopped.unterminated as u64,
        sink_refusals: h.sink_refusals,
    };
    // The marker wait is after the proxy's stop and ends before the drain share does.
    let native_scope = marked.and_then(|tee| {
        match tee.marker_until(crate::capture_tee::marker_wait_end(deadline)) {
            ScopeMarker::One(id) => Some(id),
            ScopeMarker::Absent | ScopeMarker::Conflicting => None,
        }
    });
    // The launcher's own root is reaped, so the daemon's termination answers `AlreadyStopped`
    // after one `ps`: the allowance is the bound for a root with no grace to give.
    let sealed = crate::commands::seal::seal_locally(
        client,
        entry_id,
        Some(counters),
        native_scope,
        deadline,
        termination_bound(0),
        reconnect,
    );
    // A committed seal stays committed: the report is this command's own output, and its
    // failing (a closed pipe, a quit pager) ends nothing. It is said on stderr, if that is
    // still there, so the trace's path is not lost with it.
    if let Ok(report) = &sealed
        && let Err(e) = crate::commands::seal::report_local_finish(entry_id, report, out)
    {
        use std::io::Write as _;
        let _ = writeln!(
            std::io::stderr(),
            "goalcup: entry {entry_id} is sealed, but its report could not be written to stdout \
             ({e}). The trace is {}.",
            report.trace_dir
        );
    }
    // Cut at the deadline, the sender ends its connection with the rest unwritten, as its own
    // bound would. When the seal returned after the deadline, it is cut at once.
    runtime.block_on(async {
        let _ = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), sink.shutdown())
            .await;
    });
    sealed
}

/// Abandon an entry nothing was launched into, and remove what this command made for it: the
/// daemon removes its chain, reservation and still-empty workspace, and the run directory
/// goes once the daemon has agreed.
fn unmake(
    client: &(dyn DaemonClient + Sync),
    cfg: &Path,
    entry_id: &str,
    run: &RunDir,
    cause: &CliError,
    out: &mut dyn std::io::Write,
) {
    let reason = format!("`goalcup start --goal-file` failed before the launch: {cause}");
    let sentence = format!(
        "Entry {entry_id} is abandoned: nothing was launched, and its chain and this \
         machine's note of it are gone."
    );
    if abandon_because(client, cfg, entry_id, &reason, &sentence, out)
        && let Err(e) = run.remove()
    {
        let _ = say(
            out,
            &format!(
                "  its run directory {} could not be removed: {e}",
                run.dir.display()
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::MockDaemonClient;

    /// The interrupt flag is process-global; the tests that move it take turns.
    static FLAG: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// #7: a Ctrl-C started this abandon; the daemon's ordinary `busy` must be ridden out, not
    /// surfaced at the first try, or the entry stays held with its reservation.
    #[test]
    fn an_abandon_after_ctrl_c_rides_out_a_busy() {
        let _flag = FLAG.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let daemon = MockDaemonClient::armed_ok(tmp.path(), "fake");
        daemon.busy_once("abandon");
        crate::arming::clear_interrupt();
        crate::arming::interrupt_for_test();

        let mut out = Vec::new();
        let abandoned = abandon_because(&daemon, tmp.path(), "entry-1", "ctrl-c", "gone", &mut out);

        crate::arming::clear_interrupt();
        assert!(abandoned, "{}", String::from_utf8_lossy(&out));
        assert_eq!(daemon.abandoned().len(), 1);
    }

    /// #7: once the window is closed (the harness released), a Ctrl-C meant for the harness
    /// is not latched, so the launch record and the seal keep their retries.
    #[test]
    fn a_ctrl_c_after_the_release_is_not_latched() {
        let _flag = FLAG.lock().unwrap_or_else(|e| e.into_inner());
        crate::arming::clear_interrupt();
        crate::arming::close_window();
        crate::arming::interrupt_for_test();
        let latched = crate::arming::interrupted();
        crate::arming::clear_interrupt();
        assert!(!latched);
        crate::arming::interrupt_for_test();
        let reopened = crate::arming::interrupted();
        crate::arming::clear_interrupt();
        assert!(reopened, "the next start's window is open again");
    }

    /// #11: two runs on one shared harness home. The second is refused plainly while the first
    /// holds the home, and admitted once it lets go.
    #[test]
    fn a_second_run_on_a_shared_home_is_refused_until_the_first_ends() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("codex-cli");
        let first = lock_shared_home(&home, "codex-cli").expect("the first run takes the home");

        let refused = lock_shared_home(&home, "codex-cli").expect_err("a second run is refused");
        let said = refused.to_string();
        assert!(matches!(refused, CliError::Refused(_)), "{said}");
        assert!(said.contains("already recording"), "{said}");

        drop(first);
        lock_shared_home(&home, "codex-cli").expect("free again once the first run ends");
    }
}
