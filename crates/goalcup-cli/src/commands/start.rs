//! `goalcup start` — a protocol-v5 capture run: the preflight, the daemon's capture, the launch
//! through the recording proxy, and the seal.
//!
//! No grant, no arming. The harness is launched through the recording proxy, which runs inside
//! this process, and the run ends with a seal (see `capture` below for the order). A headless
//! connector takes `--goal-file`, whose text is the harness's one goal. A chat-mode connector
//! (D57: `[capture] interactive = true`) takes none: the harness runs in its normal chat mode in
//! the participant's own terminal and the participant types the prompt.
//!
//! # Chat mode: the terminal is the harness's
//!
//! From the launch line to the harness's exit `goalcup start` writes nothing to the terminal:
//! every message goes to `<cfg>/runs/<run_id>/host.log` and the run's `state.json`
//! ([`crate::host_log`]). It catches what the terminal sends the foreground group it shares with
//! the harness (Ctrl-C, Ctrl-\, Ctrl-Z, a closed window) and lives on through each
//! ([`goalcup_platform::terminal::Shield`]), so the run is sealed and uploaded however the
//! harness ended. Once the harness has exited it speaks again, if the terminal is still there,
//! and it keeps absorbing a hangup until the run's wrap-up is done: a window closed after the
//! harness exited must not cut the seal or the upload short.
//!
//! # A run on the server
//!
//! With a server (`goalcup login`) the order is: the prep checks (`goalcup prep` ran: its repo,
//! work folder and GitHub token), the WSL checks on every folder the run writes to, **the participant's
//! approval of this run in the browser** ([`crate::account::device`]), and only then anything
//! local: the run directory, the clone of the participant's repository that is the run's
//! workspace, the run opened on the server with the credential the approval returned
//! (`POST /v1/runs`, with the machine's facts), and that credential kept in the run's own
//! directory (`<cfg>/runs/<run_id>/credential`). The credential is never in the harness's
//! environment or arguments; the harness's git asks `goalcup git-credential` for the GitHub
//! token. A Ctrl-C, a denial, an expired code or a server that is away during the approval ends
//! the start with one sentence, with nothing created and no run open.
//!
//! # A segment ends; the run does not
//!
//! A run on a server lasts until the server's deadline, and its agent may end long before: the
//! participant closes the window, the agent crashes. When the harness exits before the run's end,
//! this host seals that entry, sends its final heartbeat, lets go of the shared harness home,
//! says (if the terminal is still there) that the run stays open until its deadline and that
//! `goalcup continue` restarts it, keeps uploading the entry until it is caught up, and exits 0.
//! The run's disk reservation stays: the next segment takes the same file. The run is over, and
//! its reservation goes back after the last acknowledgement, only when the host stopped the
//! agent because the run's end had come (the deadline, or a newer run replacing it) or the
//! deadline has passed on this machine's clock; a full disk stops the agent and leaves the run
//! open. A run with no server has no segments: its end is its end.
//!
//! `goalcup continue` ([`super::continue_run`]) opens the next segment through the same phases
//! ([`Opening::Continue`]): no approval, no clone, the run's credential and workspace, the
//! harness started with its `resume_args` after its `args`, a new entry, and the run's record
//! and reservation extended, not made again.
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
use crate::run::reserve::{DiskPolicy, RunReserve, artifact_root, release_stale, reserve_path};
use crate::run::{EntryRef, RunRecord};
use crate::upload::policy::{WrapUp, WrapUpSpec};
use goalcup_protocol::capture::CaptureSink;
use goalcup_protocol::control::{
    CaptureFinishReport, CaptureGoalRequest, CaptureLaunchRequest, CaptureWorkspaceInfo,
    CaptureWorkspaceRequest, DaemonClient, FINISH_COMMIT_RESERVE, ProxyCounters, SignedStatus,
};
use goalcup_protocol::harness::{CAPTURE_HOMES_DIR, CaptureSpec, ConfigHome, RunHome};
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
    /// `--goal-file`: this file's text is a headless connector's one goal. A headless connector
    /// refuses `None`, and a chat-mode one (D57) refuses `Some`: its participant types the
    /// prompt in the harness.
    pub goal_file: Option<PathBuf>,
    /// Where a run with no goal file (a chat-mode connector, D57) makes its own folder: the
    /// directory `goalcup start` was run from, which `main.rs` reads. `None` reads this
    /// process's current directory. A run with a goal file makes it beside that file.
    pub run_folder: Option<PathBuf>,
    /// The sizes of the run's disk reservation and how often it is watched. The shipped values
    /// ([`DiskPolicy::default`]); only a test build can name others.
    pub disk: DiskPolicy,
    /// `--practice` / `--test`: a practice run, which the server keeps apart from the
    /// competition and which never replaces the account's competition run, or an organizer
    /// test run, which is a competition run in every way this build can see. Only meaningful
    /// for a run on a server.
    pub mode: crate::account::device::RunMode,
    /// Opens the approval address in the participant's browser, and says whether a program
    /// started: [`goalcup_platform::machine::open_url`] in every shipped build.
    pub open_url: OpenUrlFn,
    /// When the run's chunks go and how failures wait ([`crate::upload::uploader::Policy`]).
    pub upload_policy: crate::upload::uploader::Policy,
    /// How the machine is kept awake for the run's life: [`goalcup_platform::awake::hold`] in
    /// every shipped build. A lock that cannot be taken never refuses or stops a run; it is
    /// said by `goalcup status`.
    pub sleep_lock: SleepLockFn,
}

/// Opens an address in the browser ([`StartOptions::open_url`]).
pub type OpenUrlFn = fn(&str) -> bool;

/// Takes the sleep lock, given why ([`StartOptions::sleep_lock`]).
pub type SleepLockFn = fn(&str) -> Result<goalcup_platform::awake::SleepLock, String>;

/// What one `goalcup start` did.
#[derive(Debug)]
pub struct StartOutcome {
    pub entry_id: String,
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
    started(
        client,
        connectors,
        cfg,
        env,
        opts,
        None,
        Opening::Fresh,
        out,
    )
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
    started(
        client,
        connectors,
        cfg,
        env,
        opts,
        Some(roots),
        Opening::Fresh,
        out,
    )
}

/// How a capture run is opened.
pub(crate) enum Opening {
    /// A new run: the participant approves it in the browser, the repository is cloned, and the
    /// run is opened on the server (or, with no server, it is local).
    Fresh,
    /// A new segment of a run that is open on the server (`goalcup continue`): the run's saved
    /// credential, its workspace, its reserve, and a new entry (`POST /v1/runs/{id}/continue`).
    Continue(Box<Continuing>),
}

/// What `goalcup continue` has settled before the new segment is opened.
pub(crate) struct Continuing {
    /// This machine's record of the run as it stands: every segment so far.
    pub record: RunRecord,
    /// The run's own credential, saved when it was approved.
    pub credential: crate::account::credential::RunCredential,
    /// The run's latest entry: what the server is told this segment follows.
    pub previous_entry_id: String,
    /// The lock on the shared harness home, taken before anything was closed or changed (a
    /// connector with a home of its own per run has none).
    pub home_lock: Option<std::fs::File>,
    /// The earlier segments still being closed and sent on threads of their own.
    pub draining: Vec<crate::upload::uploader::Draining>,
}

/// `goalcup continue`: the run's next segment, launched the way `start` launches a run. The
/// caller has refused what is to be refused and closed the segment before ([`Continuing`]).
pub(crate) fn continue_segment(
    client: &(dyn DaemonClient + Sync),
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &StartOptions,
    continuing: Continuing,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    let _clear = crate::arming::clear_on_drop();
    started(
        client,
        connectors,
        cfg,
        env,
        opts,
        None,
        Opening::Continue(Box::new(continuing)),
        out,
    )
}

/// The whole of it, so [`start_with`] has one place to put the flag back from.
#[allow(clippy::too_many_arguments)]
fn started(
    client: &(dyn DaemonClient + Sync),
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &StartOptions,
    roots: Option<UpstreamRoots>,
    opening: Opening,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    // 0. The Ctrl-C handler, before anything exists, so that everything created below is
    //    inside the window a Ctrl-C unmakes. It is installed once per process and it does
    //    nothing but set a flag; the four places that read it are the four moments this
    //    command can act on one.
    crate::arming::watch_for_interrupt();

    let roots = roots.unwrap_or_else(UpstreamRoots::webpki);
    // Every line of the run goes through the one switch that keeps a chat-mode harness's
    // terminal clear (`crate::host_log`); a headless run's output is unchanged by it.
    let mut routed = crate::host_log::Routed::new(out);
    capture(
        client,
        connectors,
        cfg,
        env,
        opts,
        opts.goal_file.as_deref(),
        roots,
        opening,
        &mut routed,
    )
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

/// A test build's way to make a background thread panic while a chat-mode run is quiet
/// (`GOALCUP_TEST_PANIC_WHILE_QUIET` set): the panic must reach `host.log`, not the terminal.
#[cfg(feature = "test-support")]
fn panic_while_quiet_for_test() {
    if std::env::var_os("GOALCUP_TEST_PANIC_WHILE_QUIET").is_some() {
        let _ = std::thread::Builder::new()
            .name("goalcup-test-panic".into())
            .spawn(|| panic!("a test panic while the host is quiet"))
            .map(std::thread::JoinHandle::join);
    }
}

#[cfg(not(feature = "test-support"))]
#[inline(always)]
fn panic_while_quiet_for_test() {}

/// The soft open-file limit `goalcup start` asks for: a relayed connection holds two
/// descriptors, and the rest is room for the runtime, the sender and this process.
const OPEN_FILES_WANTED: u64 = 2 * MAX_CONNECTION_TASKS + 256;

/// What every phase of one capture run is given and none changes.
struct CaptureRun<'a> {
    client: &'a (dyn DaemonClient + Sync),
    /// The upload server this run opens with, or `None` for a local-only run.
    upload: Option<&'a Uploading>,
    cfg: &'a Path,
    env: &'a BTreeMap<String, String>,
    opts: &'a StartOptions,
    /// The folder the run's own folder is made in: the goal file's, or for a chat-mode run the
    /// one `goalcup start` was run from.
    beside: &'a Path,
    connector: &'a HarnessConnector,
    profile: &'a CaptureSpec,
    /// Set when an abandon was refused, so some of what the start made is still there: the
    /// cleanup sentence ([`CLEANED_UP`]) is not true then, and is not said.
    uncleaned: &'a std::cell::Cell<bool>,
    /// Set when the server said the run being continued is over (`410`): the earlier segments'
    /// uploads are then waited for like a stop's, not for a refusal's twenty seconds.
    over: &'a std::cell::Cell<bool>,
    /// The run this start continues, for `goalcup continue`; `None` for a new run.
    continuing: Option<&'a Continuing>,
}

/// Everything a capture run decides, and every refusal it makes, before anything is created.
struct CapturePrep {
    /// The goal file's bytes; `None` for a chat-mode run, whose participant types the prompt.
    goal: Option<Vec<u8>>,
    /// The provider key, if the profile names one. Taken (and so dropped) once the tees hold what
    /// they need: nothing after that reads the key from here.
    secret: Option<crate::launch::ChildSecret>,
    streams: StreamPlan,
    scope: LaunchScope,
    /// The scan of this run's artifacts for fragments of that key (D41), derived once, in
    /// memory, from the same bytes the child receives.
    matcher: Option<FragmentMatcher>,
    /// The harness file this run resolved and hashed; the release refuses it if it changed.
    executable: crate::launch::ResolvedExecutable,
    /// The harness home every run of this connector shares (D39); `None` when each run makes its
    /// own inside its run directory.
    shared_home: Option<SharedCaptureHome>,
    /// The run `HOME` every run of this connector shares (D59, `run_home = "shared"`): the
    /// folder `<cfg>/capture-homes/<id>.run-home`, under the same lock. `None` when each run
    /// gets a `HOME` of its own.
    shared_run_home: Option<SharedCaptureHome>,
    /// The lock that keeps a second run off that shared home: held from the claim, moved into
    /// the [`Forked`] run at the fork and let go at its seal (the upload that follows needs no
    /// home), and released by the OS if this process dies. A `goalcup continue` takes it before
    /// it closes anything and hands it over.
    shared_home_lock: Option<std::fs::File>,
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
    match try_lock_shared_home(config_home, connector)? {
        Ok(file) => Ok(file),
        Err(_busy) => Err(CliError::Refused(format!(
            "a {connector} run is already recording on this machine, and runs share one \
             {connector} home, so they go one at a time. Wait for it to finish, or stop it \
             and run `goalcup seal`. Nothing was created."
        ))),
    }
}

/// `<capture-homes>/<connector>.lock`, beside the shared home.
fn home_lock_path(config_home: &Path, connector: &str) -> PathBuf {
    config_home
        .parent()
        .expect("a capture-home parent")
        .join(format!("{connector}.lock"))
}

/// [`lock_shared_home`]'s attempt: the lock, or the lock file's path when another process holds
/// it (so that the holder can be named, [`home_holder`]).
#[cfg(unix)]
fn try_lock_shared_home(
    config_home: &Path,
    connector: &str,
) -> Result<Result<std::fs::File, PathBuf>, CliError> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = home_lock_path(config_home, connector);
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
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(Err(path)),
        Err(std::fs::TryLockError::Error(source)) => {
            return Err(CliError::Io {
                path: path.display().to_string(),
                source,
            });
        }
    }
    Ok(Ok(file))
}

/// Write the id of the run that holds the shared-home lock into the lock file, so that a
/// `goalcup continue` refused by it can name that run. Best effort: the lock is the holder's
/// whether or not the note is written.
#[cfg(unix)]
fn note_home_holder(lock: &std::fs::File, run_id: &str) {
    use std::os::unix::fs::FileExt;
    if lock.set_len(0).is_ok() {
        let _ = lock.write_all_at(run_id.as_bytes(), 0);
    }
}

/// The run id [`note_home_holder`] wrote into the lock file at `path`, if it is a run id.
fn home_holder(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(64)
        .read_to_string(&mut text)
        .ok()?;
    let text = text.trim();
    crate::upload::is_run_id(text).then(|| text.to_string())
}

/// `goalcup continue`: take the lock on the connector's shared harness home before anything of
/// the run is closed or changed. A connector with a home of its own per run has none. Another
/// run holding it is refused, naming that run (or, when it has not yet said who it is, the run
/// that was to be continued).
#[cfg(unix)]
pub(crate) fn lock_home_for_continue(
    cfg: &Path,
    connector: &HarnessConnector,
    run_id: &str,
) -> Result<Option<std::fs::File>, CliError> {
    let id = &connector.loaded.manifest.id;
    let Some((config_home, _)) = shared_leaf(cfg, &connector.loaded.manifest.capture, id) else {
        return Ok(None);
    };
    private_dir(config_home.parent().expect("a capture-home parent"), true)?;
    match try_lock_shared_home(&config_home, id)? {
        Ok(lock) => {
            // Said at once, so a start refused by this lock before the segment's own record is
            // written names this run.
            note_home_holder(&lock, run_id);
            Ok(Some(lock))
        }
        Err(path) => {
            let holder = home_holder(&path).unwrap_or_else(|| run_id.to_string());
            Err(CliError::Refused(still_recording(&holder, id)))
        }
    }
}

/// Which home of a connector is kept across runs, and so serialized under its one
/// `<capture-homes>/<id>.lock`: its config home (D39, `config_home = "shared"`), at
/// `<cfg>/capture-homes/<id>`, or its run `HOME` (D59, `run_home = "shared"`), at
/// `<cfg>/capture-homes/<id>.run-home`. The schema never admits both. The `bool` is whether it
/// is the run `HOME`. `None`: every home is made per run.
///
/// The run home's leaf has a `.` in it, which no connector id has, so it is never another
/// connector's home or a lock, and a connector that moves its shared home from the one to the
/// other does not inherit the old tree as its `HOME`.
fn shared_leaf(cfg: &Path, profile: &CaptureSpec, id: &str) -> Option<(PathBuf, bool)> {
    let homes = cfg.join(CAPTURE_HOMES_DIR);
    if profile.home.config_home == ConfigHome::Shared {
        Some((homes.join(id), false))
    } else if profile.home.run_home == RunHome::Shared {
        Some((homes.join(format!("{id}.run-home")), true))
    } else {
        None
    }
}

/// The folder of a connector's kept home, which `goalcup prep` opens the harness in so that a
/// sign-in lands where the runs will look for it: its config home (D39) or its run `HOME`
/// (D59). `None` for a connector whose homes are all made per run: nothing to prepare.
pub(crate) fn prep_home_leaf(cfg: &Path, connector: &HarnessConnector) -> Option<PathBuf> {
    shared_leaf(
        cfg,
        &connector.loaded.manifest.capture,
        &connector.loaded.manifest.id,
    )
    .map(|(leaf, _)| leaf)
}

/// The kept home `goalcup prep` opens the harness in, claimed: its folder, and the lock that
/// keeps a run from it (and it from a recording run) while the participant is signed in there.
pub(crate) struct PrepHome {
    /// `<cfg>/capture-homes/<id>` or `<cfg>/capture-homes/<id>.run-home`, checked.
    pub(crate) dir: PathBuf,
    /// `dir` is the run's `HOME` (D59, `run_home = "shared"`) and not its config home.
    pub(crate) is_run_home: bool,
    _lock: std::fs::File,
}

/// Claim the home `goalcup prep` opens the harness in ([`prep_home_leaf`]): the one every run of
/// the connector shares, made if there is none, under the `<id>.lock` a run takes. `None` for a
/// connector with no kept home. Judged as a run judges it.
pub(crate) fn claim_home_for_prep(
    cfg: &Path,
    connector: &HarnessConnector,
) -> Result<Option<PrepHome>, CliError> {
    let id = &connector.loaded.manifest.id;
    let Some((leaf, is_run_home)) = shared_leaf(cfg, &connector.loaded.manifest.capture, id) else {
        return Ok(None);
    };
    private_dir(leaf.parent().expect("a capture-home parent"), true)?;
    let lock = match try_lock_shared_home(&leaf, id)? {
        Ok(lock) => lock,
        Err(path) => {
            let holder = home_holder(&path);
            return Err(CliError::Refused(match holder {
                Some(run) => format!(
                    "run {run} is recording on this machine, and runs share one {id} home. \
                     Wait for it to end, then run `goalcup prep` again."
                ),
                None => format!(
                    "a {id} run is recording on this machine, or another `goalcup prep` has \
                     the {id} home open. Wait for it to end, then run `goalcup prep` again."
                ),
            }));
        }
    };
    let home = ensure_shared_home(&leaf)?;
    Ok(Some(PrepHome {
        dir: home.recheck()?,
        is_run_home,
        _lock: lock,
    }))
}

/// The shared harness home's folder (D39), made if there is none, and judged as every run
/// judges it. The caller holds the lock: `goalcup start` and `goalcup prep` differ only in
/// how they take it and what they say when they cannot.
fn ensure_shared_home(config_home: &Path) -> Result<SharedCaptureHome, CliError> {
    match std::fs::symlink_metadata(config_home) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match private_dir(config_home, false) {
                // A concurrent `goalcup start` made it between the look and the create;
                // `SharedCaptureHome::load` below judges that directory like any other.
                Err(CliError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                made => made?,
            }
        }
        Err(_) => {
            return Err(shared_home_refusal(
                config_home,
                "its exact leaf cannot be inspected",
            ));
        }
    }
    // SAFETY: reading the effective uid has no preconditions or memory effects.
    let owner = unsafe { libc::geteuid() };
    SharedCaptureHome::load(config_home, owner)
}

/// The refusal of a `goalcup continue` that a run still recording stands in the way of.
pub(crate) fn still_recording(holder: &str, connector: &str) -> String {
    format!(
        "run {holder} is still recording on this machine: its agent is running, and runs share \
         one {connector} home, so they go one at a time. Wait for it to end, or close its \
         terminal, and run `goalcup continue` again. Nothing was changed."
    )
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
    /// The recording proxy; `None` once a deadline finish has stopped it.
    proxied: Option<Proxied>,
    /// The run was given a provider key, so its directory is scanned and judged.
    scanning: bool,
    /// This machine's record of the entry, with no pid yet.
    template: EntryRecord,
    /// For a run opened on a server: given back if the harness is never launched.
    give_back: Option<crate::upload::GiveBack>,
    /// What the uploader needs, for a run opened on a server.
    upload: Option<UploadStart>,
    /// This machine's record of the run.
    record: RunRecord,
    /// The connector is a chat-mode one: `goalcup continue` can restart a run of it.
    chat_mode: bool,
    /// The run's one disk reservation, held from the entry's creation to the run's end, until
    /// the watch tick takes it over when the harness is released.
    reserve: Option<RunReserve>,
    /// The watch tick, which has the reservation from the harness's release to just before the
    /// reservation is released.
    watching: Option<crate::run::watch::Watching>,
    /// A chat-mode run's hold on the terminal, from just before the release to the end of the
    /// run's wrap-up.
    chat: Option<ChatTerminal>,
    /// The lock that keeps a second run off the shared harness home. It is let go at the seal:
    /// the upload that follows needs no harness home, and the next segment or run may start.
    home_lock: Option<std::fs::File>,
    /// The counters of the earlier segments of a continued run, being closed and sent beside
    /// this one: the disk watch covers their entries too.
    drained: Vec<crate::upload::uploader::DrainHandles>,
}

/// What a chat-mode run holds while the harness owns the terminal (D57): the terminal's signals
/// absorbed, the host quiet, and the terminal's modes as they were before the harness took it.
struct ChatTerminal {
    shield: goalcup_platform::terminal::Shield,
    quiet: crate::host_log::Quiet,
    modes: Option<goalcup_platform::terminal::Modes>,
}

impl ChatTerminal {
    /// Taken just before the release, once the launch line is out.
    fn take(run_dir: &Path) -> Result<ChatTerminal, CliError> {
        let shield = goalcup_platform::terminal::Shield::take().map_err(|source| CliError::Io {
            path: "the terminal's signals".to_string(),
            source,
        })?;
        Ok(ChatTerminal {
            shield,
            quiet: crate::host_log::quiet(run_dir),
            modes: goalcup_platform::terminal::Modes::save(libc::STDIN_FILENO),
        })
    }

    /// The harness has exited: Ctrl-C, Ctrl-\ and Ctrl-Z are this command's own again, and it
    /// speaks again if the terminal is still there. A hangup stays absorbed until the wrap-up is
    /// done ([`Drop`]), so a window closed now does not cut the seal or the upload short.
    /// The terminal's modes are put back first, while the terminal is there: a harness ended in
    /// raw mode would leave the participant's shell typing blind.
    fn harness_exited(&mut self) {
        self.shield.harness_exited();
        if self.quiet.harness_exited()
            && let Some(modes) = &self.modes
            && let Err(e) = modes.restore()
        {
            crate::host_log::say(&format!("the terminal's modes could not be put back ({e})"));
        }
    }
}

/// What the uploader of a run opened on a server starts from.
struct UploadStart {
    server: Option<crate::upload::http::ServerClient>,
    record: crate::upload::UploadRecord,
    /// The start receipt's key, which every acknowledgement must verify under.
    key: String,
    policy: crate::upload::uploader::Policy,
    /// The run's deadline by this machine's own clock, for when the server cannot say it.
    local_deadline: Option<crate::upload::uploader::LocalDeadline>,
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
        let keeps_reserve = self.keeps_reserve();
        // Whatever ended the launch, including a Ctrl-C, the seal below runs its full retries.
        crate::arming::close_window();
        if let Some(proxied) = self.proxied {
            finish_unlaunched(self.client, &self.info.entry_id, proxied, out);
        }
        note_unscanned(self.scanning, &self.run.dir, out);
        if let Some(reserve) = self.reserve {
            end_reserve(reserve, keeps_reserve);
        }
        if let Some(give_back) = self.give_back {
            give_back.give();
        }
        error
    }

    /// Whether the run's reserve file outlives a launch that failed: a segment of a server run
    /// with a chat-mode connector that is not over goes on to be continued, and its one
    /// reservation is the run's, not the segment's. A new run that is given back to the server,
    /// a headless run (it has no `continue`), and a run that is over take their reserve with
    /// them.
    fn keeps_reserve(&self) -> bool {
        self.give_back.is_none()
            && self.record.server_url.is_some()
            && self.chat_mode
            && !deadline_passed(self.record.deadline_at.as_deref())
    }

    /// The root ruling under D40: the entry was sealed while its harness was being launched, so
    /// this command ends the child it just released, says so, and answers the committed seal.
    fn sealed_launch(
        mut self,
        child: Child,
        handed: &ProcessHandle,
        tees: &Tees,
        out: &mut dyn std::io::Write,
    ) -> CliError {
        note_unscanned(self.scanning, &self.run.dir, out);
        let keeps_reserve = self.keeps_reserve();
        if let Some(reserve) = self.reserve.take() {
            end_reserve(reserve, keeps_reserve);
        }
        let Forked {
            client,
            info,
            proxied,
            give_back,
            mut chat,
            ..
        } = self;
        // The harness is ended at once and nothing was uploaded: the run goes back.
        if let Some(give_back) = give_back {
            give_back.give();
        }
        let end = crate::launch::end_released_child(child, handed);
        // Ended: the terminal is this command's again.
        if let Some(chat) = chat.as_mut() {
            chat.harness_exited();
        }
        let _ = say(out, &end.announcement(&info.entry_id));
        let deadline = std::time::Instant::now() + crate::finish_budget();
        if let Some(proxied) = proxied {
            let _ = finish(
                client,
                &info.entry_id,
                proxied,
                deadline,
                WrapUp::one_attempt(),
                termination_bound(0),
                None,
                None,
                out,
            );
        }
        tees.finish(deadline, out);
        // Worded from what was observed, never more: a native child that could not be checked is
        // not "nothing is running".
        CliError::Refused(end.refusal(&info.entry_id))
    }
}

/// The end of a reserve this command holds when a launch failed: released, or dropped with the
/// file kept (its lock goes with it) for the run's next segment.
fn end_reserve(reserve: RunReserve, keep: bool) {
    if keep {
        drop(reserve);
    } else {
        release_and_say(reserve);
    }
}

/// Give the run's reserve back to the disk, and say so when that fails: the file keeps its
/// space until a later start or `goalcup status` removes it.
fn release_and_say(reserve: RunReserve) {
    if let Err(error) = reserve.release() {
        crate::host_log::say(&format!(
            "the run's disk reservation could not be released ({error}); it keeps its space \
             until a later goalcup command removes it"
        ));
    }
}

/// What the gate holds for the phases after the fork.
struct Gate {
    plan: LaunchPlan,
    /// A chat-mode launch (D57): the harness takes the terminal at its release.
    chat: bool,
    /// What the harness reported for its `version_args`, or `unreported` (D53).
    harness_version: String,
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
    harness_version: String,
}

/// A harness that has exited.
struct Exited<'a> {
    forked: Forked<'a>,
    tees: Tees,
    status: Result<ExitStatus, CliError>,
    /// The finish a deadline made while the harness was still running, if one did.
    finished_early: Option<Result<CaptureFinishReport, CliError>>,
    /// The uploader, for a run opened on a server.
    uploading: Option<crate::upload::uploader::Running>,
    /// The wrap-up after the stop, which the seal and the upload's finish share.
    wrap_up: WrapUp,
    /// Linux: the refusal the command ends with, after the seal, when the sweep left a process
    /// running or could not read `/proc` after it had signalled one.
    left_running: Option<CliError>,
    /// The run's state file.
    keeper: crate::run::StateKeeper,
}

/// How the daemon says the recording is, read through a client made from `control.json` each
/// time, so a restarted daemon is followed. `None` when it cannot say, or when what answered is
/// not the daemon that opened the entry (see [`recording_of`]).
fn ask_recording(
    cfg: &Path,
    entry_id: &str,
    run_public_key: &str,
) -> Option<crate::run::Recording> {
    let client = crate::daemon::HttpDaemonClient::from_config_dir(cfg).ok()?;
    recording_of(&client, entry_id, run_public_key)
}

/// The recording as the entry's own signed status says it. The status route is unauthenticated,
/// so a document counts only when it is signed by the run key the entry's creation named and is
/// about this entry: the heartbeat tells the server this word, and a forged "ok" must not.
fn recording_of(
    client: &dyn DaemonClient,
    entry_id: &str,
    run_public_key: &str,
) -> Option<crate::run::Recording> {
    let status = retry_busy(|| client.status(entry_id)).ok()?;
    if status.verify_with(run_public_key).is_err() || status.payload.entry_id != entry_id {
        return None;
    }
    Some(crate::run::Recording::of_capture(
        status.payload.capture?.capture,
    ))
}

/// A v5 local capture run.
///
/// The order is the daemon's: the disk is checked, the entry is created (its chain is quota-only),
/// the run's one reservation is taken (a refusal abandons the entry), the proxy starts in this process and its sender hands the daemon a producer, the entry's own
/// status must show that producer admitted, the goal is recorded once, and only then is the
/// harness spawned; its process is registered before its launch is recorded. When it exits
/// the proxy stops, the seal carries the proxy's counters, and the sender drains.
///
/// Every failure before the spawn abandons the entry, which removes its chain and
/// still-empty workspace, releases the run's reservation, and removes this run's directory. From the spawn on nothing is
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
    goal_file: Option<&Path>,
    roots: UpstreamRoots,
    opening: Opening,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    let goal = goal_file.map(read_goal).transpose()?;
    let beside = match goal_file {
        Some(goal_file) => beside_goal(goal_file),
        None => match &opts.run_folder {
            Some(folder) => folder.clone(),
            None => std::env::current_dir().map_err(|source| CliError::Io {
                path: "the current directory".to_string(),
                source,
            })?,
        },
    };
    // Under WSL: WSL1 cannot run this, and WSL2 cannot keep a run on a Windows drive. Judged
    // before anything is created or removed, over every folder the run writes to.
    let work_folder = match &opening {
        Opening::Fresh => prepared_work_folder(cfg),
        Opening::Continue(continuing) => {
            continuing.record.workspace.parent().map(Path::to_path_buf)
        }
    };
    refuse_wsl(
        env,
        cfg,
        &beside,
        work_folder.as_deref(),
        "Nothing was created.",
    )?;
    // A reserve nothing will use again (a run that is over, or one `continue` cannot restart and
    // that is all sent) goes back to the disk.
    release_stale(cfg, &artifact_root(cfg), crate::run::now_unix());
    let c = crate::connector::load_harness(connectors, &opts.harness, opts.dev)?;
    let profile = &c.loaded.manifest.capture;
    // The earlier segments being closed: waited for, shortly, on any way out before the new
    // segment's own end (`DrainsOnDrop`).
    let over = std::cell::Cell::new(false);
    let mut draining = DrainsOnDrop {
        drains: Vec::new(),
        over: None,
    };
    let (continuing, held_home_lock) = match opening {
        Opening::Fresh => (None, None),
        Opening::Continue(mut continuing) => {
            let lock = continuing.home_lock.take();
            draining.drains = std::mem::take(&mut continuing.draining);
            (Some(continuing), lock)
        }
    };
    // A chat-mode connector takes no goal file and a headless one needs one: refused before
    // anything else is asked or made. A continued run has no goal file at all: only a chat-mode
    // harness can be continued.
    match &continuing {
        None => goal_file_fits(
            &c.loaded.manifest.id,
            profile.interactive,
            goal_file.is_some(),
        )?,
        Some(_) => continue_fits(&c.loaded.manifest.id, profile.interactive)?,
    }
    // With a server: the prep checks, before any other work and before the participant is asked
    // for anything. A continued run was prepared when it started.
    let serving = match &continuing {
        None => uploading(
            cfg,
            env,
            &beside,
            opts.mode,
            profile.credential.is_some(),
            out,
        )?,
        Some(_) => None,
    };
    let uncleaned = std::cell::Cell::new(false);
    draining.over = Some((&over, opts.upload_policy.wrap_up));
    let cx = CaptureRun {
        client,
        upload: None,
        cfg,
        env,
        opts,
        beside: &beside,
        connector: &c,
        profile,
        uncleaned: &uncleaned,
        over: &over,
        continuing: continuing.as_deref(),
    };
    let mut prep = prepare(&cx, goal, out)?;
    prep.shared_home_lock = held_home_lock;
    // Every refusal that needs no approval has been made: now the participant approves this
    // run in the browser. Nothing is created until they have. A continued run is approved
    // already: its credential is the one it was approved with.
    let upload = match (serving, &continuing) {
        (Some(serving), _) => Some(serving.approved(opts, &c.loaded.manifest.id, out)?),
        (None, Some(continuing)) => Some(Uploading::continuing(continuing)?),
        (None, None) => None,
    };
    if crate::arming::interrupted() {
        return Err(interrupted_refusal());
    }
    let cx = CaptureRun {
        upload: upload.as_ref(),
        ..cx
    };
    // The shared harness home is claimed only now: it creates a folder and holds a lock. Every
    // failure from here to the fork (the home, the run directory and the clone, the open on the
    // server, the entry, the launch) is one phase, and ends with one sentence about what was
    // removed.
    let started =
        claim_shared_home(&cx, &mut prep).and_then(|()| create_and_gate(&cx, roots, &prep, out));
    let (mut forked, gate) = match started {
        Ok(started) => started,
        Err(error) => {
            if upload.is_some() && !uncleaned.get() {
                let _ = say(out, CLEANED_UP);
            }
            return Err(error);
        }
    };
    // The lock on the shared harness home goes with the run from here, and is let go at its seal.
    forked.home_lock = prep.shared_home_lock.take();
    // The earlier segments of a continued run are closed and sent beside this one.
    forked.drained = draining
        .drains
        .iter()
        .map(|drain| drain.handles())
        .collect();
    // Right before the launch the machine is told to stay awake, and it stays so to the end of
    // the wrap-up. A lock that cannot be taken is no reason to refuse or stop a run: it is
    // kept as the reason in the run's state, and `goalcup status` says it.
    let (awake, sleep_lock) =
        match (opts.sleep_lock)(&format!("goalcup run {}", forked.record.run_id)) {
            Ok(lock) => (Some(lock), Ok(())),
            Err(why) => (None, Err(why)),
        };
    let released = register_and_release(forked, gate, &mut prep, out)?;
    let exited = run_harness(&cx, prep.scope, released, sleep_lock, out)?;
    let finished = finish_and_scan(
        exited,
        prep.matcher.as_ref(),
        std::mem::take(&mut draining.drains),
        out,
    );
    drop(awake);
    finished
}

/// How long a refusal waits for the earlier segments it left being closed.
const DRAIN_ON_REFUSAL: Duration = Duration::from_secs(20);

/// The earlier segments of a continued run, being closed and sent on threads of their own. A
/// start that ends before it has a segment of its own to wait through waits for them briefly
/// here, so a refusal does not cut the close of the segment before it short; what is still unsent
/// is `goalcup upload`'s.
struct DrainsOnDrop<'a> {
    drains: Vec<crate::upload::uploader::Draining>,
    /// Whether the server said the run is over, and the wrap-up an upload gets then. A run that
    /// is over has no next segment to finish the earlier ones' uploads beside, and its earlier
    /// segment is sent as a stop's is: up to the whole wrap-up, with its progress said.
    over: Option<(&'a std::cell::Cell<bool>, Duration)>,
}

impl Drop for DrainsOnDrop<'_> {
    fn drop(&mut self) {
        let wrap_up = match self.over {
            Some((over, wrap_up)) if over.get() && !self.drains.is_empty() => {
                crate::host_log::say(&format!(
                    "the run is over: waiting for the earlier segment's upload (up to {} \
                     minutes).",
                    wrap_up.as_secs().div_ceil(60)
                ));
                wrap_up
            }
            _ => DRAIN_ON_REFUSAL,
        };
        for drain in self.drains.drain(..) {
            let entry = drain.entry_id().to_string();
            let how_to_finish = drain.how_to_finish();
            let report = drain.wait(WrapUp::after(wrap_up));
            if report.caught_up {
                crate::host_log::say(&format!(
                    "uploaded: entry {entry}, an earlier segment of this run, acknowledged."
                ));
            } else {
                crate::host_log::say(&format!(
                    "upload of entry {entry}, an earlier segment, is incomplete ({}). {}",
                    report.why.as_deref().unwrap_or("the upload did not finish"),
                    how_to_finish
                ));
            }
        }
    }
}

/// `goalcup continue`'s refusals that need no server and change nothing, made before the
/// shared harness home is locked and before any earlier segment is sealed or sent: the connector
/// must be a chat-mode one, this must be the foreground of a terminal, and the harness, its pin
/// and its executable must be usable. [`prepare`] asks them again of the segment it launches.
pub(crate) fn continue_preflight(
    connector: &HarnessConnector,
    dev: bool,
    env: &BTreeMap<String, String>,
) -> Result<(), CliError> {
    let id = &connector.loaded.manifest.id;
    let profile = &connector.loaded.manifest.capture;
    continue_fits(id, profile.interactive)?;
    holds_the_terminal(id, "Nothing was changed.")?;
    let pin = crate::preflight::harness_pin_gate(connector, dev);
    if pin.level == Level::Fail {
        return Err(CliError::Refused(format!(
            "`goalcup continue` refuses this run:\n{}",
            pin.render().trim_end()
        )));
    }
    child_secret_for_harness(profile, env)?;
    crate::launch::capture_executable(connector, profile, env)?;
    Ok(())
}

/// A continued run needs a chat-mode harness: a headless one was given its one goal and has no
/// session to pick up.
fn continue_fits(id: &str, interactive: bool) -> Result<(), CliError> {
    if interactive {
        return Ok(());
    }
    Err(CliError::Refused(format!(
        "`{id}` runs headless: it was given one goal and has no session to pick up, so its run \
         cannot be continued. Nothing was changed."
    )))
}

/// [`goal_file_fits`] for the connector `opts` names, loaded from `connectors`: what `main.rs`
/// asks before it starts a daemon. `start_with` asks it again of the connector it loads.
pub fn goal_file_fits_connector(connectors: &Path, opts: &StartOptions) -> Result<(), CliError> {
    let c = crate::connector::load_harness(connectors, &opts.harness, opts.dev)?;
    goal_file_fits(
        &c.loaded.manifest.id,
        c.loaded.manifest.capture.interactive,
        opts.goal_file.is_some(),
    )
}

/// A chat-mode connector (D57) with a goal file, or a headless one without, refused in one
/// sentence that says what to type instead. Nothing is created before it.
fn goal_file_fits(id: &str, interactive: bool, has_goal_file: bool) -> Result<(), CliError> {
    match (interactive, has_goal_file) {
        (true, true) => Err(CliError::Refused(format!(
            "`{id}` runs in chat mode: the harness opens in this terminal and you type the \
             prompt there, so it takes no `--goal-file`. Run `goalcup start --harness {id}`. \
             Nothing was created."
        ))),
        (false, false) => Err(CliError::Refused(format!(
            "`{id}` runs headless and needs a goal file: `goalcup start --harness {id} \
             --goal-file <file>`. Nothing was created."
        ))),
        _ => Ok(()),
    }
}

/// A chat-mode harness reads and draws on this terminal: `goalcup start`'s input must be a
/// terminal it holds in the foreground (or the harness's first read stops it with `SIGTTIN` and
/// the run records a frozen harness), and its output must be a terminal too (a harness drawing
/// into a file is not a chat). `then` says what was left behind.
fn holds_the_terminal(id: &str, then: &str) -> Result<(), CliError> {
    use goalcup_platform::terminal::{is_foreground, is_terminal};
    if is_foreground(libc::STDIN_FILENO) && is_terminal(libc::STDOUT_FILENO) {
        return Ok(());
    }
    Err(CliError::Refused(format!(
        "`{id}` runs in chat mode in this terminal, so `goalcup start` must run in the \
         foreground of a terminal, with its input and its output that terminal: one of them is \
         not, or the command was moved to the background. {then}"
    )))
}

/// What a chat-mode run records as its goal. The v5 `Launch` record names the goal stream it
/// was launched with and the daemon records a launch only after a goal, so a run whose harness
/// was handed none records this fixed sentence instead: never sent to the harness, and never
/// anything the participant typed (that is in the harness's session files).
pub const CHAT_MODE_GOAL: &str = "chat mode (D57): the harness was given no goal; the \
     participant typed the prompt in the harness's own terminal, and it is recorded in the \
     harness's session files.";

/// The launch line of a chat-mode run: the last thing `goalcup start` prints before the harness
/// owns the terminal.
fn chat_launch_line(id: &str, log: &Path) -> String {
    format!(
        "Recording in chat mode: {id} starts now in this terminal. Type your prompt; after it, \
         type only \"keep going\". Until {id} exits goalcup prints nothing here: its messages go \
         to {}, and `goalcup status` shows the run.",
        log.display()
    )
}

/// Phase one: every refusal that comes before anything is created.
fn prepare(
    cx: &CaptureRun<'_>,
    goal: Option<Vec<u8>>,
    out: &mut dyn std::io::Write,
) -> Result<CapturePrep, CliError> {
    // The launch plan checks again at its own boundary, but that boundary follows every
    // capture allocation. Reject unusable goal bytes here so no shared home, run directory,
    // seeded home, daemon entry or proxy can precede this preflight decision.
    if let Some(goal) = &goal {
        crate::launch::checked_goal(goal)?;
    }
    // The disk is judged before the server is asked for a run or anything is created. A
    // continued run keeps its reserve file, and `RunReserve::take` judges what is missing of it.
    if cx.continuing.is_none() {
        crate::run::reserve::check_room(&artifact_root(cx.cfg).join("reserve"), &cx.opts.disk)?;
    }
    let CaptureRun {
        env,
        opts,
        beside,
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
    if profile.interactive {
        holds_the_terminal(&c.loaded.manifest.id, "Nothing was created.")?;
    }
    let executable = crate::launch::capture_executable(c, profile, env)?;
    // The provider hosts answer a TCP connect before anything is created (the proxy module's
    // preflight; it names a firewall holding this binary when they do not).
    if !offline_upstreams_for_test(env) {
        crate::proxy::preflight::upstreams_reachable(profile.hosts.iter().map(|h| h.host.as_str()))
            .map_err(|unreachable| CliError::Refused(unreachable.to_string()))?;
    }
    // A per-run home is made inside the run directory below. A run directory beside a goal file
    // inside a Git work tree is refused before anything is created: it would hold what the
    // harness wrote, which can include a fragment of the provider key (D41).
    let per_run = profile.home.config_home == ConfigHome::PerRun;
    if per_run {
        crate::per_run_home::refuse_inside_git_work_tree(beside)?;
        crate::per_run_home::refuse_synced_folder_under(beside, env.get("HOME").map(Path::new))?;
    }
    Ok(CapturePrep {
        goal,
        secret,
        streams,
        scope,
        matcher,
        executable,
        shared_home: None,
        shared_run_home: None,
        shared_home_lock: None,
    })
}

/// The harness home every run of a connector shares (D39), or its shared run `HOME` (D59), taken
/// for this run: its folder made (0700) if there is none, and the lock that keeps a second run
/// off it. A connector whose homes are made per run has none to take. This creates things on
/// disk, so a run on a server takes it only after the participant has approved the run.
fn claim_shared_home(cx: &CaptureRun<'_>, prep: &mut CapturePrep) -> Result<(), CliError> {
    let CaptureRun {
        cfg,
        connector: c,
        profile,
        ..
    } = *cx;
    let Some((config_home, is_run_home)) = shared_leaf(cfg, profile, &c.loaded.manifest.id) else {
        return Ok(());
    };
    private_dir(config_home.parent().expect("a capture-home parent"), true)?;
    // `goalcup continue` took the lock before it closed anything, and hands it over.
    let lock = match prep.shared_home_lock.take() {
        Some(held) => held,
        None => lock_shared_home(&config_home, &c.loaded.manifest.id)?,
    };
    let home = ensure_shared_home(&config_home)?;
    if is_run_home {
        prep.shared_run_home = Some(home);
    } else {
        prep.shared_home = Some(home);
    }
    prep.shared_home_lock = Some(lock);
    Ok(())
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
        upload,
        cfg,
        env,
        opts,
        beside,
        connector: c,
        profile,
        ..
    } = *cx;
    // Catch a replacement since prepare before writing even the temporary run directory.
    let shared_home = prep
        .shared_home
        .as_ref()
        .map(SharedCaptureHome::recheck)
        .transpose()?;
    let shared_run_home = prep
        .shared_run_home
        .as_ref()
        .map(SharedCaptureHome::recheck)
        .transpose()?;
    let run = RunDir::create(
        beside,
        profile.home.run_home,
        shared_run_home.as_deref(),
        env,
        match (cx.continuing, upload.map(|u| &u.how)) {
            // A continued run goes on in the clone it has.
            (Some(continuing), _) => WorkspaceAt::Existing(&continuing.record.workspace),
            (None, Some(UploadHow::Fresh(new))) => WorkspaceAt::Clone(&new.work_root),
            _ => WorkspaceAt::InRunDir,
        },
    )?;
    // The workspace of a run on a server is a fresh clone of the participant's repository.
    if let Some(upload) = upload
        && let Err(error) = upload.clone_workspace(&run.workspace)
    {
        let _ = run.remove();
        return Err(error);
    }
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

    // The harness's own account of its version, asked once with nothing but `PATH` and a
    // throwaway `HOME` beside the run's own, removed afterwards: a probe may write state (Codex
    // creates `~/.codex/tmp/arg0`), and the run's `HOME` stays as the profile made it. Reported,
    // not proven, and `unreported` when it does not say (D53).
    let harness_version = crate::harness_version::reported_version_in_scratch_home(
        &prep.executable,
        &profile.executable.version_args,
        &run.dir.join("version-probe-home"),
    );

    // The last alias/target identity check is directly before the create request. Preserve the
    // exact D39 leaf spelling in the request, local entry and child environment.
    if let Some(home) = prep.shared_home.as_ref().or(prep.shared_run_home.as_ref()) {
        pause_at("capture-shared-home-pre-create");
        if let Err(error) = home.recheck() {
            let _ = run.remove();
            return Err(error);
        }
    }

    let session_id = fresh_uuid();
    // The run's start: what its record keeps, and what a native source that reads from the run's
    // start (D57) is read from. Sent for every entry, the same number for each.
    let started_at_unix = match cx.continuing {
        Some(continuing) => continuing.record.started_at_unix,
        None => crate::run::now_unix(),
    };
    // With a server, the run is opened there before the daemon creates anything: the
    // server issues the entry id every record carries.
    let opened = match upload {
        Some(upload) => match upload.open(cx, &run, &session_id, &harness_version) {
            Ok(opened) => Some(opened),
            Err(error) => {
                let _ = run.remove();
                return Err(error);
            }
        },
        None => None,
    };
    // Give the run back to the server: this start failed before its launch.
    let give_back = |_cause: &CliError| {
        if let Some(give_back) = opened
            .as_ref()
            .and_then(|o| o.give_back.borrow_mut().take())
        {
            give_back.give();
        }
    };
    let request = CaptureWorkspaceRequest {
        workspace: run.workspace.display().to_string(),
        profile_id: c.loaded.manifest.id.clone(),
        connector_dir_sha256: c.loaded.connector_dir_sha256.clone(),
        reserve_bytes: opts.disk.chain_quota,
        run_home: run.home.display().to_string(),
        config_home: config_home.display().to_string(),
        entry_id: opened.as_ref().map(|o| o.run.entry_id.clone()),
        native_since: Some(started_at_unix),
    };
    let info = match retry_busy(|| client.create_capture_workspace(&request)) {
        Ok(info) => info,
        Err(e) => {
            let _ = run.remove();
            let error = e.into();
            give_back(&error);
            return Err(error);
        }
    };
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

    // This machine's record of the run: the server's run id for a run opened there, the entry's
    // own id for a local one. A continued run's is its record so far and the new segment.
    let new_entry = EntryRef {
        entry_id: info.entry_id.clone(),
        trace_dir: PathBuf::from(&info.trace_dir),
    };
    let record = match cx.continuing {
        Some(continuing) => {
            let mut record = continuing.record.clone();
            record.entries.push(new_entry);
            record
        }
        None => RunRecord {
            run_id: opened
                .as_ref()
                .map_or_else(|| info.entry_id.clone(), |o| o.run.run_id.clone()),
            server_url: upload.map(|u| u.server.base().to_string()),
            practice: opts.mode.is_practice(),
            test: opts.mode == crate::account::device::RunMode::Test,
            connector_id: c.loaded.manifest.id.clone(),
            workspace: run.workspace.clone(),
            started_at_unix,
            deadline_at: opened.as_ref().map(|o| o.run.deadline_at.clone()),
            entries: vec![new_entry],
        },
    };
    // The run's one reservation, taken here and released when the launch fails or the run ends.
    let mut reserve = None::<RunReserve>;
    // Up to the spawn, a failure unmakes everything above.
    let launched = write_entry(cfg, &template).and_then(|()| {
        // The record first: a reservation always has a record to say whose it is.
        crate::run::write(cfg, &record)?;
        #[cfg(unix)]
        if let Some(lock) = &prep.shared_home_lock {
            note_home_holder(lock, &record.run_id);
        }
        reserve = Some(RunReserve::take(
            &reserve_path(&artifact_root(cfg), &record.run_id),
            &opts.disk,
        )?);
        let bound = match (upload, opened.as_ref()) {
            (Some(upload), Some(opened)) => Some(upload.bind(cfg, opened, &info, out)?),
            _ => None,
        };
        say(out, &format!("entry {}", info.entry_id))?;
        say(out, &format!("run directory: {}", run.dir.display()))?;
        raise_open_file_limit(out)?;
        let proxied = Proxied::start(&info, profile, &run, roots)?;
        admitted(client, &info)?;
        // The goal-cup skill, in the harness's own skills folder (D57), replacing only its own.
        if let Some(skills_dir) = &profile.home.skills_dir {
            crate::skill::install(&config_home, skills_dir)?;
        }
        let mut vars = goalcup_protocol::template::TemplateVars::new();
        vars.set("workspace", run.workspace.display().to_string());
        vars.set("session_id", session_id.clone());
        let build_launch = if cx.continuing.is_some() {
            crate::launch::build_resumed_launch
        } else {
            crate::launch::build_capture_launch
        };
        let plan = build_launch(
            c,
            &vars,
            &run.workspace,
            &crate::launch::CaptureLaunch {
                goal: prep.goal.as_deref(),
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
        // A chat-mode run's harness is given no goal, and records the fixed sentence that says so.
        let goal_request = CaptureGoalRequest {
            bytes: prep
                .goal
                .clone()
                .unwrap_or_else(|| CHAT_MODE_GOAL.as_bytes().to_vec()),
        };
        let goal_stream = retry_busy(|| client.record_capture_goal(&info.entry_id, &goal_request))?;
        if crate::arming::interrupted() {
            return Err(interrupted_refusal());
        }
        // A `stdout_json` launch waits this long for its identity event (`thread.started` for
        // Codex) before recording the launch without one; the connector may set it.
        let scope_deadline = std::time::Instant::now() + profile.identity.stdout_wait();
        // The only way a capture child is spawned (D40): forked and parked before `exec`,
        // and released only once its registration is durable. Its pipes are the one
        // manifest-derived `StreamPlan` decision.
        let gated = spawn_gated(
            &plan,
            prep.streams.pipes(),
            GATE_READY_WITHIN,
            prep.secret.as_ref(),
        )?;
        Ok((proxied, plan, goal_stream, gated, scope_deadline, bound))
    });
    let (proxied, plan, goal_stream, gated, scope_deadline, bound) = match launched {
        Ok(launched) => launched,
        Err(error) => {
            if let Some(held) = reserve.take() {
                // A continued run's reserve is the run's, and outlives this segment.
                if cx.continuing.is_none() {
                    release_and_say(held);
                }
            }
            let unmade = unmake(client, cfg, &info.entry_id, &run, &error, out);
            if !unmade {
                cx.uncleaned.set(true);
            }
            match cx.continuing {
                // The run stays: this machine's record of it goes back to what it was. The
                // server's new entry (it cannot be given back) is found again by the next
                // `goalcup continue`, which follows it.
                Some(continuing) => {
                    let _ = std::fs::remove_file(crate::upload::record_path(cfg, &info.entry_id));
                    let _ = std::fs::remove_file(crate::upload::receipt_path(cfg, &info.entry_id));
                    // Whether or not the daemon agreed to the abandon: this entry never ran, and
                    // a record that still names it would leave the next `continue` refused for
                    // good ("has no upload record").
                    let _ = crate::run::write(cfg, &continuing.record);
                }
                None => {
                    // The run is given back first, and its credential outlives the give-back
                    // while the server may still hold the run open: the next start's
                    // `give_back_pending` asks with it.
                    give_back(&error);
                    if unmade {
                        remove_run_keeping_pending_credential(cfg, &record.run_id);
                    }
                }
            }
            return Err(error);
        }
    };
    let forked = Forked {
        client,
        cfg,
        info,
        run,
        proxied: Some(proxied),
        scanning: prep.matcher.is_some(),
        template,
        upload: bound.map(|(record, key)| UploadStart {
            server: upload.map(|u| u.server.clone()),
            // The server's time left, counted from when it was asked (WP11).
            local_deadline: opened.as_ref().and_then(|o| {
                crate::upload::uploader::LocalDeadline::at_start(
                    &record,
                    o.run.seconds_left,
                    o.opened_at,
                    cx.continuing.is_some(),
                )
            }),
            record,
            key,
            policy: opts.upload_policy,
        }),
        give_back: opened
            .as_ref()
            .and_then(|o| o.give_back.borrow_mut().take()),
        record,
        chat_mode: profile.interactive,
        reserve,
        watching: None,
        chat: None,
        home_lock: None,
        drained: Vec::new(),
    };
    let gate = Gate {
        chat: profile.interactive,
        plan,
        harness_version,
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
    mut forked: Forked<'a>,
    gate: Gate,
    prep: &mut CapturePrep,
    out: &mut dyn std::io::Write,
) -> Result<Released<'a>, CliError> {
    pause_at("capture-gate-forked");
    let Gate {
        plan,
        chat,
        harness_version,
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
    // The last look at the flag. A Ctrl-C during the spawn or the registration above reached
    // only this process and the parked child, which rides it out; released now, the harness
    // would run the whole goal the participant asked to stop.
    if crate::arming::interrupted() {
        gated.abort();
        return Err(forked.unlaunched(interrupted_before_launch(), out));
    }
    // A chat-mode harness takes the terminal at its release: the launch line is the last thing
    // this command prints there, and from here the terminal's signals are absorbed and every
    // message goes to `host.log` until the harness exits.
    if chat {
        // Moved to the background, or its terminal gone, while the run was approved and cloned:
        // the harness is never released onto a terminal it cannot read.
        if let Err(e) = holds_the_terminal(
            &forked.template.connector_id,
            "Nothing was launched, and the entry is sealed without a launch.",
        ) {
            gated.abort();
            return Err(forked.unlaunched(e, out));
        }
        let run_dir = crate::run::run_dir(forked.cfg, &forked.record.run_id);
        let line = chat_launch_line(
            &forked.template.connector_id,
            &run_dir.join(crate::host_log::HOST_LOG_NAME),
        );
        let _ = say(out, &line).and_then(|()| {
            out.flush().map_err(|source| CliError::Io {
                path: "stdout".to_string(),
                source,
            })
        });
        match ChatTerminal::take(&run_dir) {
            Ok(terminal) => {
                forked.chat = Some(terminal);
                panic_while_quiet_for_test();
            }
            Err(e) => {
                gated.abort();
                return Err(forked.unlaunched(e, out));
            }
        }
    }
    let mut spawned = match gated.release() {
        Ok(spawned) => spawned,
        // No harness ran, so no `Launch` is written, and the entry is sealed; the terminal is
        // this command's again, to say so.
        Err(e) => {
            forked.chat = None;
            return Err(forked.unlaunched(e, out));
        }
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
        harness_version,
    })
}

/// Phase four: record the launch and wait for the harness. A seal that raced in, or a launch
/// the daemon found sealed, ends the child (D40); any other failure to record leaves the trace
/// incomplete and never stops the run.
fn run_harness<'a>(
    cx: &CaptureRun<'_>,
    scope: LaunchScope,
    released: Released<'a>,
    sleep_lock: Result<(), String>,
    out: &mut dyn std::io::Write,
) -> Result<Exited<'a>, CliError> {
    let Released {
        mut forked,
        handed,
        mut child,
        spawned_at,
        tees,
        goal_stream,
        scope_deadline,
        harness_version,
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
        harness_version,
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
    // A chat-mode run said its launch line before the release, and is quiet now.
    if forked.chat.is_none() {
        let _ = say(
            out,
            "Recording. The harness runs headless; this command ends with it.",
        );
    }
    // Launched: the run on the server is this participant's now, and is not given back.
    if let Some(give_back) = forked.give_back.take() {
        give_back.launched();
    }
    // The first chunk goes now, beside the run; nothing the uploader does waits the harness.
    // The run's upload record, kept here so the workspace's commit can be noted the moment the
    // run stops.
    let noted = forked.upload.as_ref().map(|start| start.record.clone());
    // How long the wrap-up after the stop lasts: the upload policy's for a run on a server, and
    // the local default for one without.
    let wrap_spec = WrapUpSpec::for_run(forked.upload.as_ref().map(|start| &start.policy), cx.env);
    let uploads_dir = forked.cfg.join(crate::upload::UPLOADS_DIR_NAME);
    let note_repo_head = || {
        if let Some(record) = &noted {
            crate::upload::uploader::note_repo_head(&uploads_dir, record);
        }
    };
    // The run's state file, which the watch tick keeps and this function changes at each
    // transition of the agent; and what the threads of the run tell each other.
    let mut launched = crate::run::RunState::launched(&entry_id, sleep_lock);
    // What `goalcup status` may advise of `goalcup continue` for this run: only a run on a server
    // with a chat-mode connector can be restarted by it.
    launched.continuable = forked.record.server_url.is_some() && forked.chat_mode;
    let keeper = crate::run::StateKeeper::begin(
        &crate::run::run_dir(forked.cfg, &forked.record.run_id),
        launched,
    );
    let vitals = match forked.proxied.as_ref() {
        Some(proxied) => crate::run::Vitals::counting(proxied.proxy.model_requests()),
        None => crate::run::Vitals::default(),
    };

    // The run's one stop channel: the uploader's deadline or heartbeat stop and the watch's disk
    // stop both send on it, and the wait for the harness receives from it.
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<String>();
    let uploading = forked.upload.take().and_then(|start| {
        let server = start.server?;
        let spool = forked.cfg.join(crate::upload::UPLOADS_DIR_NAME);
        match crate::upload::uploader::begin(
            server,
            start.record,
            start.key,
            start.policy,
            spool,
            start.local_deadline,
            stop_tx.clone(),
            vitals.clone(),
        ) {
            Ok(running) => Some(running),
            Err(e) => {
                let _ = say(out, &format!("  (the upload could not start: {e})"));
                None
            }
        }
    });
    // At the run's deadline the uploader says stop: this command finishes the run there and
    // then, as it would once the harness exits (the proxy stopped, then the seal), except that
    // the daemon's seal ends the harness, given its grace. That ends the wait below. What the
    // finish said is printed once the wait is over.
    type Early = (Result<CaptureFinishReport, CliError>, Vec<u8>);
    let early = std::sync::Mutex::new(None::<Early>);
    // The wrap-up of a run stopped at its deadline starts there, not when the harness is gone.
    let stopped_wrap = std::sync::Mutex::new(None::<WrapUp>);
    let proxy_slot = std::sync::Mutex::new(forked.proxied.take());
    let (client, cfg) = (forked.client, forked.cfg);
    let grace = handed.grace_seconds;
    // The watch tick runs on a thread of its own from here to just before the reservation is
    // released (it keeps the reservation sized through the upload's wrap-up too).
    forked.watching = forked.reserve.take().map(|reserve| {
        crate::run::watch::watch_beside(crate::run::watch::Watch {
            reserve,
            run: forked.record.clone(),
            stop: stop_tx,
            // Every entry this process uploads: this segment's, and the earlier segments being
            // closed beside it, each with its own acknowledged offset.
            uploads: uploading
                .iter()
                .map(|running| running.last_ok())
                .chain(forked.drained.iter().map(|drain| drain.last_ok.clone()))
                .collect(),
            acked: uploading
                .iter()
                .map(|running| (running.entry_id().to_string(), running.acked_offset()))
                .chain(
                    forked
                        .drained
                        .iter()
                        .map(|drain| (drain.entry_id.clone(), drain.acked_offset.clone())),
                )
                .collect(),
            policy: cx.opts.disk,
            beat: Some(crate::run::watch::Beat {
                keeper: keeper.clone(),
                vitals: vitals.clone(),
                ask: {
                    let (cfg, entry_id) = (cfg.to_path_buf(), entry_id.clone());
                    let key = forked.info.run_public_key.clone();
                    Arc::new(move || ask_recording(&cfg, &entry_id, &key))
                },
            }),
        })
    });
    // A chat-mode harness that suspends itself is continued at once: suspension is not supported
    // during a run, and a stopped harness would hold the participant's terminal frozen.
    let mut continued = |signal: i32| {
        crate::host_log::say(&format!(
            "the harness stopped itself (signal {signal}); a run cannot be suspended, so it \
             was continued at once"
        ));
    };
    let on_stopped: Option<&mut dyn FnMut(i32)> = if forked.chat.is_some() {
        Some(&mut continued)
    } else {
        None
    };
    let status = crate::upload::wait_stopping(
        &mut child,
        stop_rx,
        |reason| {
            let Some(proxied) = proxy_slot.lock().unwrap_or_else(|e| e.into_inner()).take() else {
                return;
            };
            // The commit the workspace has now, before the harness's grace lets it change.
            note_repo_head();
            // The one bound of the wrap-up, for the seal below and the upload's finish later.
            let wrap_up = wrap_spec.begin();
            *stopped_wrap.lock().unwrap_or_else(|e| e.into_inner()) = Some(wrap_up);
            keeper.update(|state| state.stopped = Some(reason.to_string()));
            let label = if reason == crate::run::watch::DISK_STOP_REASON
                || reason == crate::upload::uploader::REPLACED_REASON
                || crate::upload::uploader::refused_run(reason)
            {
                "stopped"
            } else {
                "deadline"
            };
            let line = format!("{label}: {reason}. The agent is stopped and the run sealed now.");
            // At once, on stderr: the seal below may be asked again for minutes, and the stdout report
            // that follows it is printed only once it returns. A chat-mode run's goes to its log.
            crate::host_log::say(&line);
            let mut said = format!("{line}\n").into_bytes();

            let deadline = std::time::Instant::now() + crate::finish_budget();
            // A seal that did not take (the daemon busy or restarting) is asked again through the
            // whole wrap-up: the harness must not run on past its deadline, and only the daemon's
            // seal ends it.
            let finished = finish(
                client,
                &entry_id,
                proxied,
                deadline,
                wrap_up,
                termination_bound(grace),
                None,
                Some(cfg),
                &mut said,
            );
            *early.lock().unwrap_or_else(|e| e.into_inner()) = Some((finished, said));
        },
        on_stopped,
    )
    .map_err(|source| CliError::Io {
        path: handed.program.clone(),
        source,
    });
    forked.proxied = proxy_slot.into_inner().unwrap_or_else(|e| e.into_inner());
    // The harness has exited: the terminal is this command's again, if it is still there.
    if let Some(chat) = forked.chat.as_mut() {
        chat.harness_exited();
    }
    keeper.update(|state| {
        state.harness = crate::run::HarnessState::Exited {
            status: match &status {
                Ok(status) => describe_exit(status),
                Err(error) => error.to_string(),
            },
        };
    });
    // A run that ended on its own stopped just now.
    let wrap_up = stopped_wrap
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or_else(|| wrap_spec.begin());
    // A run that ended on its own: its commit as it ended.
    if forked.proxied.is_some() {
        note_repo_head();
    }
    let finished_early =
        early
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
            .map(|(finished, said)| {
                let _ = out.write_all(&said);
                finished
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
            crate::host_log::say_raw(&line);
        }
    }
    Ok(Exited {
        forked,
        tees,
        status,
        finished_early,
        uploading,
        wrap_up,
        left_running: swept.and_then(|swept| swept.refusal(&entry_id)),
        keeper,
    })
}

/// Phase five: stop the proxy and seal within the one finish budget, let the tees drain, and scan
/// the run for fragments of the provider key. The scan never changes how the harness or the seal
/// ended, and a run that is not clean is a failed row whose directory stays local.
fn finish_and_scan(
    exited: Exited<'_>,
    matcher: Option<&FragmentMatcher>,
    draining: Vec<crate::upload::uploader::Draining>,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    pause_at("capture-finish-started");
    let Exited {
        forked,
        tees,
        status,
        finished_early,
        uploading,
        wrap_up,
        left_running,
        keeper,
    } = exited;
    let Forked {
        client,
        cfg,
        info,
        run,
        proxied,
        watching,
        chat,
        record,
        home_lock,
        chat_mode,
        ..
    } = forked;
    // Only a chat-mode run says that it can be continued: only its harness can be.
    let continuable = chat_mode;
    // A chat-mode run's hold on the terminal (its hangup absorbed, its log open) lasts to the end
    // of this function: the seal and the upload's wrap-up below must outlive a closed window.
    let _chat = chat;
    let deadline = std::time::Instant::now() + crate::finish_budget();
    let finished = match (finished_early, proxied) {
        (Some(finished), _) => finished,
        (None, Some(proxied)) => finish(
            client,
            &info.entry_id,
            proxied,
            deadline,
            wrap_up,
            termination_bound(0),
            if tees.late_scope {
                tees.marked.as_ref()
            } else {
                None
            },
            Some(cfg),
            out,
        ),
        (None, None) => Err(CliError::Internal(
            "the run's proxy was stopped and nothing finished the run".to_string(),
        )),
    };
    // The run is over when it was stopped for its end (its deadline, or a newer run replacing
    // it; a full disk stops the agent and leaves the run open), when its deadline has passed on
    // this machine's clock, and always for a run with no server. Otherwise the agent's exit
    // ended this segment only: the run stays open on the server until its deadline and
    // `goalcup continue` restarts it.
    let snapshot = keeper.snapshot();
    let ended_by_stop = snapshot.stopped_for_its_end();
    let on_server = record.server_url.is_some();
    let over = !on_server || crate::run::is_over(&record, Some(&snapshot), crate::run::now_unix());
    // `goalcup continue` can restart the run: a run on a server with a chat-mode connector that
    // is not over. Only such a run keeps its reserve past this host.
    let run_goes_on = crate::run::can_continue(cfg, &record, continuable, over);
    // The entry is sealed and the run goes on: the reserve is handed over before anything else
    // is let go. This host's watch stops managing it and its lock is dropped, the file kept, so
    // that a `goalcup continue` takes the same file at once instead of finding it held for as
    // long as this host's wrap-up lasts (an hour at most). The upload that follows needs none
    // of it, and the next segment's watch prunes this entry's acknowledged parts.
    let mut watching = watching;
    if run_goes_on {
        drop(watching.take().and_then(crate::run::watch::Watching::end));
    }
    // The shared harness home is free for the next segment or run, and the upload that follows
    // needs none of it.
    drop(home_lock);
    tees.finish(deadline, out);
    // How the harness ended is the participant's to see: the exit code of a run that was recorded
    // is not this command's own (`main.rs`), so it is said here.
    if let Ok(status) = &status {
        let _ = say(out, &format!("harness exited: {}.", describe_exit(status)));
    }
    if on_server {
        let how = match &status {
            Ok(status) => describe_exit(status),
            Err(error) => error.to_string(),
        };
        // `goalcup continue` is advised only when it can restart the run; otherwise what is
        // true is said.
        match (run_goes_on, over, record.deadline_at.as_deref()) {
            (true, _, Some(deadline_at)) => {
                let _ = say(
                    out,
                    &format!(
                        "The agent's process ended ({how}). Your run stays open until \
                         {deadline_at}; run `goalcup continue` to restart it."
                    ),
                );
            }
            // Stopped for its end: the stop was said when it happened.
            (false, true, _) if ended_by_stop => {}
            (false, true, _) => {
                let _ = say(
                    out,
                    &format!(
                        "The agent's process ended ({how}). The run is over: its deadline has passed."
                    ),
                );
            }
            (false, false, Some(deadline_at)) => {
                let _ = say(
                    out,
                    &format!(
                        "The agent's process ended ({how}). This run cannot be continued: its \
                         connector runs headless and was given one goal. It stays open on the \
                         server until {deadline_at}, with nothing more recorded."
                    ),
                );
            }
            _ => {}
        }
    }
    // After the seal: the rest of the chain and the seal itself, within the finish budget.
    // A local run has nothing to upload, so there is nothing of it left to send.
    let mut sent_whole = !on_server;
    let mut server_ended = false;
    let mut upload_report = None;
    if let Some(running) = uploading {
        let report = running.finish(wrap_up);
        // Everything this entry has is on the server, and the seal; and the server may say the
        // run itself is sealed or incomplete.
        sent_whole = report.caught_up && report.sealed;
        server_ended = report.run_ended;
        let line = match &report {
            crate::upload::uploader::Report {
                caught_up: true,
                sealed: true,
                ..
            } => format!(
                "uploaded: {} bytes, acknowledged, and the seal.",
                report.offset
            ),
            _ => format!(
                "upload incomplete ({}): the run is complete on this machine. Run `goalcup \
                 upload {}` to finish sending it.",
                report.why.as_deref().unwrap_or("the seal was not sent"),
                info.entry_id
            ),
        };
        let _ = say(out, &line);
        upload_report = Some(report);
    }
    // The earlier segments of a continued run were being closed beside this one: what became of
    // each is said, and none is left unsent for a run that cannot be continued.
    for drain in draining {
        let entry = drain.entry_id().to_string();
        let how_to_finish = drain.how_to_finish();
        let report = drain.wait(wrap_up);
        sent_whole &= report.caught_up;
        let line = if report.caught_up {
            format!("uploaded: entry {entry}, an earlier segment of this run, acknowledged.")
        } else {
            format!(
                "upload of entry {entry}, an earlier segment, is incomplete ({}). {how_to_finish}",
                report.why.as_deref().unwrap_or("the upload did not finish")
            )
        };
        let _ = say(out, &line);
    }
    // The tick ends here, just before the reservation goes (or is left to the next start): it
    // hands the reservation back even if it died on the way.
    let reserve = watching.and_then(crate::run::watch::Watching::end);
    // The host's last word on the run's state, now that nothing else writes it: what became of
    // the upload is what `goalcup status` tells from here on.
    // Not written over a next segment's: its host may have started while this one sent.
    keeper.update_while_current(|state| {
        state.ended = true;
        if let Some(report) = &upload_report {
            state.upload.stopped = false;
            state.upload.problem = if report.caught_up && report.sealed {
                None
            } else {
                Some(
                    report
                        .why
                        .clone()
                        .unwrap_or_else(|| "the seal was not sent".to_string()),
                )
            };
        }
    });
    // The reserve is released when the run has ended (its deadline, a stop for its end, or the
    // server's word that it is sealed or incomplete), whatever became of its upload; or when the
    // run goes on but `goalcup continue` cannot restart it and all of it is on the server. A run
    // `continue` can restart keeps it. A reserve already handed over (the lock dropped, the
    // file kept) for a run the server now calls over is removed by the same rule.
    let release = over || server_ended || (!run_goes_on && sent_whole);
    match reserve {
        Some(reserve) if release => release_and_say(reserve),
        Some(_) => {}
        None if server_ended => {
            crate::run::reserve::release_if_done(
                cfg,
                &artifact_root(cfg),
                &record.run_id,
                true,
                crate::run::now_unix(),
            );
        }
        None => {}
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
        exit_status: Some(status?),
    })
}

/// Whether `deadline_at` (RFC 3339, `...Z`) has passed on this machine's clock.
fn deadline_passed(deadline_at: Option<&str>) -> bool {
    deadline_at
        .and_then(goalcup_protocol::control::parse_z)
        .is_some_and(|deadline| crate::run::now_unix() >= deadline)
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
    let deadline = std::time::Instant::now() + crate::finish_budget();
    let _ = finish(
        client,
        entry_id,
        proxied,
        deadline,
        WrapUp::one_attempt(),
        termination_bound(0),
        None,
        None,
        out,
    );
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
                    crate::finish_budget().as_secs()
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
///
/// A run on a server has its workspace elsewhere: a clone of the participant's repository in
/// their work folder, `<work_root>/goalcup-<16 hex digits>`, **a random name of its own**, never
/// this directory's (the goal file's folder may be the work folder, and the two must not meet).
/// It is named before the server has issued the run id (the server and the daemon are told the
/// path), so the id cannot be in it.
pub(crate) struct RunDir {
    pub(crate) dir: PathBuf,
    pub(crate) workspace: PathBuf,
    /// The workspace is outside `dir`, so [`RunDir::remove`] must remove it too.
    separate_workspace: bool,
    pub(crate) home: PathBuf,
    ca: PathBuf,
}

/// Where a run's workspace is.
pub(crate) enum WorkspaceAt<'a> {
    /// A folder the daemon makes inside the run directory (a run with no server).
    InRunDir,
    /// A fresh clone, under a random name of its own, in this work folder.
    Clone(&'a Path),
    /// A clone that is there already: a continued run's. It is not this directory's to remove.
    Existing(&'a Path),
}

impl RunDir {
    /// `shared_home` is the connector's shared run `HOME` (D59), claimed and checked by the
    /// caller: with `RunHome::Shared` it is the run's `HOME`, used as it is and never removed.
    pub(crate) fn create(
        beside: &Path,
        home: RunHome,
        shared_home: Option<&Path>,
        env: &BTreeMap<String, String>,
        workspace_at: WorkspaceAt<'_>,
    ) -> Result<RunDir, CliError> {
        if (home == RunHome::Shared) != shared_home.is_some() {
            return Err(CliError::Internal(
                "a shared run home was not claimed for the run that keeps one".into(),
            ));
        }
        let beside = std::fs::canonicalize(beside).map_err(|source| CliError::Io {
            path: beside.display().to_string(),
            source,
        })?;
        let name = format!("goalcup-{:016x}", rand::random::<u64>());
        let dir = beside.join(&name);
        private_dir(&dir, false)?;
        let (workspace, separate_workspace) = match workspace_at {
            WorkspaceAt::Clone(root) => (
                root.join(format!("goalcup-{:016x}", rand::random::<u64>())),
                true,
            ),
            WorkspaceAt::Existing(workspace) => (workspace.to_path_buf(), false),
            WorkspaceAt::InRunDir => (dir.join("workspace"), false),
        };
        let run = RunDir {
            workspace,
            separate_workspace,
            // Outside `dir`, so `remove` never touches it.
            home: shared_home.map_or_else(|| dir.join("home"), Path::to_path_buf),
            ca: dir.join("ca"),
            dir,
        };
        let made = match shared_home {
            // Kept across runs, made by the claim: nothing is made in it here.
            Some(_) => Ok(()),
            None => private_dir(&run.home, false),
        }
        .and_then(|()| private_dir(&run.ca, false))
        .and_then(|()| match home {
            RunHome::Empty | RunHome::Shared => Ok(()),
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

    /// Only ever before the spawn, when nothing but this command has written here. A clone
    /// outside the directory goes with it (and a failure to remove either is the error).
    ///
    /// A continued run's workspace ([`WorkspaceAt::Existing`]) is the run's clone, with the
    /// participant's agent's work in it, and is never this directory's to remove: only the
    /// segment's own folder goes.
    pub(crate) fn remove(&self) -> std::io::Result<()> {
        let clone = if self.separate_workspace {
            match std::fs::remove_dir_all(&self.workspace) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            Ok(())
        };
        std::fs::remove_dir_all(&self.dir).and(clone)
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
                crate::finish_budget(),
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
            shutdown_deadline: crate::finish_budget() - FINISH_COMMIT_RESERVE,
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
/// A seal that did not take (the daemon busy, restarting, or past its budget) is asked again with
/// growing waits until `wrap_up` has run out ([`crate::retry::retry_capture_seal_until`]): the
/// proxy is stopped once, and each later ask has a whole local-finish budget of its own. A
/// `WrapUp::one_attempt()` asks once.
///
/// The one deadline of a capture run's finish is `finish_budget()` (`LOCAL_CAPTURE_FINISH_BUDGET`), taken when the
/// harness has exited and immediately before the proxy's stop: nothing runs between the two. The
/// proxy's stop gets its drain share (the budget less `FINISH_COMMIT_RESERVE`), the daemon's seal
/// its own request's remainder, and the sender's drain and the scoped stdout what is left of the
/// same deadline (CONTRACTS.md, "Amendment 2026-10-02 (C6 local finish budget, H-10 4a)").
///
/// A Hermes child's session identity is read between the two: after the proxy has stopped and
/// before the seal is sent, by [`crate::capture_tee::marker_wait_end`]'s deadline, which leaves
/// the daemon's native read its share of the drain. The identity, if exactly one distinct one was
/// printed, rides on the seal request; any other answer sends none.
#[allow(clippy::too_many_arguments)]
fn finish(
    client: &(dyn DaemonClient + Sync),
    entry_id: &str,
    proxied: Proxied,
    deadline: std::time::Instant,
    wrap_up: WrapUp,
    termination: Duration,
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
    // After the harness exited, the launcher's own root is reaped and the daemon's termination
    // answers `AlreadyStopped` after one `ps`: `termination_bound(0)`. At a deadline the harness
    // is still running, and the bound gives it its grace.
    let sealed = crate::commands::seal::seal_locally_until(
        client,
        entry_id,
        Some(counters),
        native_scope,
        deadline,
        wrap_up,
        termination,
        reconnect,
    );
    // A committed seal stays committed: the report is this command's own output, and its
    // failing (a closed pipe, a quit pager) ends nothing. It is said on stderr, if that is
    // still there, so the trace's path is not lost with it.
    if let Ok(report) = &sealed
        && let Err(e) = crate::commands::seal::report_local_finish(entry_id, report, out)
    {
        crate::host_log::say(&format!(
            "entry {entry_id} is sealed, but its report could not be written to stdout ({e}). \
             The trace is {}.",
            report.trace_dir
        ));
    }
    // Cut at the deadline, the sender ends its connection with the rest unwritten, as its own
    // bound would. When the seal returned after the deadline, it is cut at once.
    runtime.block_on(async {
        let _ = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), sink.shutdown())
            .await;
    });
    sealed
}

/// The upload server this run opens with: `None` with no `goalcup login`, and for a harness
/// handed a provider key (`keyed`), whose run must pass the key scan at its end before
/// anything leaves the machine. With a server, `goalcup prep` must have run: its repository,
/// its work folder and the GitHub token are checked here, all before the participant is asked for
/// anything and before anything is created. A `--dev` run is uploaded like any other; the server refuses a
/// connector it does not accept.
fn uploading(
    cfg: &Path,
    env: &BTreeMap<String, String>,
    beside: &Path,
    mode: crate::account::device::RunMode,
    keyed: bool,
    out: &mut dyn std::io::Write,
) -> Result<Option<Serving>, CliError> {
    use crate::account::device::RunMode;
    // A practice run or an organizer test run is a run on the server: without one there is
    // nothing to practise on, and a test run that never reaches the server is no test.
    let (flag, kind) = match mode {
        RunMode::Practice => (Some("--practice"), "practice"),
        RunMode::Test => (Some("--test"), "test"),
        RunMode::Competition => (None, "competition"),
    };
    let Some(server) = crate::server::read_server_config(cfg)? else {
        return match flag {
            Some(flag) => Err(CliError::Refused(format!(
                "`{flag}` needs `goalcup login` and `goalcup prep` first. Nothing was created."
            ))),
            None => Ok(None),
        };
    };
    if keyed {
        if let Some(flag) = flag {
            return Err(CliError::Refused(format!(
                "`{flag}` cannot be used with this harness: a harness handed a provider key \
                 is never uploaded, so it cannot make a {kind} run. Nothing was created."
            )));
        }
        say(
            out,
            "A run of a harness given a provider key is not uploaded: it stays on this machine.",
        )?;
        return Ok(None);
    }
    // Each of these is the one sentence `run goalcup prep first`.
    let prepared = crate::prep_file::read_prep(cfg)?;
    let github = crate::account::github::GithubSetup::load(cfg)
        .map_err(|_| CliError::Refused(crate::prep_file::PREP_FIRST.to_string()))?;
    if same_repo(&prepared.repo_url) != same_repo(&github.repo_url) {
        return Err(CliError::Refused(crate::prep_file::PREP_FIRST.to_string()));
    }
    let work_root = PathBuf::from(&prepared.work_root);
    if !work_root.is_absolute() {
        return Err(CliError::Refused(crate::prep_file::PREP_FIRST.to_string()));
    }
    refuse_goal_folder_inside_work_root(&work_root, beside)?;
    #[cfg(feature = "test-support")]
    let clone_source = env.get(TEST_CLONE_SOURCE).cloned();
    #[cfg(not(feature = "test-support"))]
    let clone_source = {
        let _ = env;
        None
    };
    Ok(Some(Serving {
        url: server.url,
        cfg: std::fs::canonicalize(cfg).unwrap_or_else(|_| cfg.to_path_buf()),
        github,
        work_root,
        clone_source,
    }))
}

/// What a start that fails after the approval says once it has removed what it made (said once,
/// by `capture`, for every failure of that phase): the run directory beside the goal file, the
/// clone, and the run's credential directory. It does not
/// remove the work folder itself (the participant's folder, left in place once created) or the
/// shared harness-home folder of a connector under `<cfg>/capture-homes/` (one per connector,
/// persistent, and reused by every run of it).
const CLEANED_UP: &str = "Nothing was started, and what this start made was removed.";

/// The folder a continued run's own folder is made in: `requested` (the folder `goalcup
/// continue` was run from), or this process's current directory. It is refused inside the work
/// folder the run's clone is in, for the reason a new run's is
/// ([`refuse_goal_folder_inside_work_root`]).
pub(crate) fn run_folder_for_continue(
    requested: Option<&Path>,
    workspace: &Path,
) -> Result<PathBuf, CliError> {
    let beside = match requested {
        Some(folder) => folder.to_path_buf(),
        None => std::env::current_dir().map_err(|source| CliError::Io {
            path: "the current directory".to_string(),
            source,
        })?,
    };
    if let Some(work_root) = workspace.parent() {
        refuse_goal_folder_inside_work_root(work_root, &beside)?;
    }
    Ok(beside)
}

/// The work folder `goalcup prep` kept, when a server is set up (a run with none makes no clone).
fn prepared_work_folder(cfg: &Path) -> Option<PathBuf> {
    crate::server::read_server_config(cfg).ok().flatten()?;
    let kept = crate::prep_file::read_prep_lenient(cfg).work_root;
    (!kept.is_empty()).then(|| PathBuf::from(kept))
}

/// The WSL checks over every folder a run writes to: the work folder (when there is one), the
/// folder the run's own directory is made in, the artifact root and the configuration folder.
/// See [`crate::wsl_gate`]. `env` states the machine in a test build only.
pub(crate) fn refuse_wsl(
    env: &BTreeMap<String, String>,
    cfg: &Path,
    run_folder: &Path,
    work_folder: Option<&Path>,
    untouched: &str,
) -> Result<(), CliError> {
    #[cfg(feature = "test-support")]
    let host = crate::wsl_gate::Host::from_env(env);
    #[cfg(not(feature = "test-support"))]
    let host = {
        let _ = env;
        crate::wsl_gate::Host::here()
    };
    let artifacts = artifact_root(cfg);
    let mut folders = vec![
        ("run folder", run_folder),
        ("artifact root", artifacts.as_path()),
        ("config folder", cfg),
    ];
    if let Some(work_folder) = work_folder {
        folders.insert(0, ("work folder", work_folder));
    }
    host.refuse(&folders, untouched)
}

/// The run directory is made beside the goal file (for a chat-mode run, in the folder `goalcup
/// start` is run from) and holds the harness's `HOME`; the clone is made in the work folder,
/// under a name of its own. They cannot be the same folder, but a folder *below* the work folder
/// would put the run directory (and so the harness's home) inside some other clone, or inside an
/// older run's folder. So: `beside` may be the work folder itself, or anywhere outside it, and is
/// refused anywhere inside it.
fn refuse_goal_folder_inside_work_root(work_root: &Path, beside: &Path) -> Result<(), CliError> {
    let beside = std::fs::canonicalize(beside);
    let root = std::fs::canonicalize(work_root).unwrap_or_else(|_| work_root.to_path_buf());
    if let Ok(beside) = beside
        && beside != root
        && beside.starts_with(&root)
    {
        return Err(CliError::Refused(format!(
            "the run's folder would be made in {}, which is inside the work folder {}. Put the \
             goal file in (or, in chat mode, run `goalcup start` from) the work folder itself or \
             a folder outside it, so a run's folder is never inside a clone. Nothing was created.",
            beside.display(),
            work_root.display()
        )));
    }
    Ok(())
}

/// A repository address in the one spelling the two files are compared in.
fn same_repo(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    url.strip_suffix(".git").unwrap_or(url).to_ascii_lowercase()
}

/// A test build's door past the provider-host probe, so the harness contract kit can run the
/// shipped connectors (whose hosts are real providers) with no network and reach no provider.
/// A shipped build compiles none of this, and always probes.
#[cfg(feature = "test-support")]
pub const TEST_OFFLINE_UPSTREAMS: &str = "GOALCUP_TEST_OFFLINE_UPSTREAMS";

#[cfg(feature = "test-support")]
fn offline_upstreams_for_test(env: &BTreeMap<String, String>) -> bool {
    env.contains_key(TEST_OFFLINE_UPSTREAMS)
}

#[cfg(not(feature = "test-support"))]
fn offline_upstreams_for_test(_env: &BTreeMap<String, String>) -> bool {
    false
}

/// A test build's one door to the clone: a repository on this machine that stands in for the
/// participant's repository on GitHub (`account::github::clone_from`). A shipped build compiles
/// none of this.
#[cfg(feature = "test-support")]
pub const TEST_CLONE_SOURCE: &str = "GOALCUP_TEST_CLONE_SOURCE";

/// A run on a server that has passed every check that needs no approval.
struct Serving {
    url: String,
    /// The configuration directory, absolute: the clone's credential helper names it.
    cfg: PathBuf,
    github: crate::account::github::GithubSetup,
    work_root: PathBuf,
    /// A test's local repository to clone instead of GitHub; always `None` in a shipped build.
    clone_source: Option<String>,
}

impl Serving {
    /// Ask the participant to approve this run in the browser, and wait for them. Nothing has
    /// been created before this, and a denial, an expired code, a Ctrl-C or a server that
    /// cannot be reached leaves it so (the refusal says so).
    fn approved(
        self,
        opts: &StartOptions,
        connector_id: &str,
        out: &mut dyn std::io::Write,
    ) -> Result<Uploading, CliError> {
        let anonymous = crate::upload::http::ServerClient::anonymous(&self.url);
        let approved = crate::account::device::approve(
            &anonymous,
            crate::account::device::Purpose::Run { mode: opts.mode },
            connector_id,
            &opts.open_url,
            out,
        )?;
        let credential = match approved {
            crate::account::device::Approved::Run(credential) => credential,
            crate::account::device::Approved::Login { .. } => {
                return Err(CliError::Internal(
                    "the server approved a login when a run was asked for. Nothing was \
                     created; run the command again, and report it if it keeps happening."
                        .into(),
                ));
            }
        };
        Ok(Uploading {
            server: crate::upload::http::ServerClient::with_credential(&self.url, &credential),
            how: UploadHow::Fresh(Box::new(NewRun {
                credential,
                cfg: self.cfg,
                github: self.github,
                work_root: self.work_root,
                practice: opts.mode.is_practice(),
                clone_source: self.clone_source,
            })),
        })
    }
}

/// A run's upload server, for `goalcup start` and `goalcup continue`: the client that carries
/// this run's credential, and what is special to the way the run is opened.
struct Uploading {
    server: crate::upload::http::ServerClient,
    how: UploadHow,
}

enum UploadHow {
    /// A new run: approved, cloned, opened.
    Fresh(Box<NewRun>),
    /// A new segment of a run that is open.
    Continue {
        run_id: String,
        /// The entry the server is told this segment follows.
        previous_entry_id: String,
        /// The entries this machine has: a latest entry of the server's that is none of them is
        /// a segment a `goalcup continue` opened and did not launch.
        known_entries: Vec<String>,
    },
}

/// What a new run is cloned from and opened with.
struct NewRun {
    credential: crate::account::credential::RunCredential,
    cfg: PathBuf,
    github: crate::account::github::GithubSetup,
    work_root: PathBuf,
    /// A practice run never pushes: its clone is given no credential helper.
    practice: bool,
    #[cfg_attr(not(feature = "test-support"), allow(dead_code))]
    clone_source: Option<String>,
}

/// The run as the server opened it, what it was asked, and the give-back until the launch.
struct OpenedRun {
    run: crate::upload::api::OpenRun,
    request: crate::upload::api::OpenRunRequest,
    /// When the server answered, on a clock that runs through sleep: the run's own count of
    /// its length starts here.
    opened_at: goalcup_protocol::clock::ContinuousOrigin,
    give_back: std::cell::RefCell<Option<crate::upload::GiveBack>>,
    /// The segment's number, for a continued run.
    segment: Option<u64>,
}

impl Uploading {
    /// The upload side of a continued run: its saved credential, and the segment it follows.
    fn continuing(continuing: &Continuing) -> Result<Uploading, CliError> {
        let record = &continuing.record;
        let Some(url) = &record.server_url else {
            return Err(CliError::Internal(
                "a run with no upload server was asked to continue".to_string(),
            ));
        };
        Ok(Uploading {
            server: crate::upload::http::ServerClient::with_credential(url, &continuing.credential),
            how: UploadHow::Continue {
                run_id: record.run_id.clone(),
                previous_entry_id: continuing.previous_entry_id.clone(),
                known_entries: record.entries.iter().map(|e| e.entry_id.clone()).collect(),
            },
        })
    }

    /// Clone the participant's repository into `dest`, which is this run's workspace. The
    /// clone holds no token: its remote is the plain repository address, and its git asks
    /// `goalcup git-credential` for one when it pushes (`account::github`). A practice run's
    /// clone has no such helper and cannot push. A continued run has its clone already.
    fn clone_workspace(&self, dest: &Path) -> Result<(), CliError> {
        let UploadHow::Fresh(new) = &self.how else {
            return Ok(());
        };
        std::fs::create_dir_all(&new.work_root).map_err(|source| CliError::Io {
            path: new.work_root.display().to_string(),
            source,
        })?;
        let exe = std::env::current_exe().map_err(|source| CliError::Io {
            path: "the goalcup executable".to_string(),
            source,
        })?;
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        let cfg = &new.cfg;
        // The owner's rule: a practice run never pushes, so the competition repository stays
        // empty until the competition run. Its clone is anonymous and can only read.
        #[cfg(feature = "test-support")]
        if new.practice
            && let Some(source) = &new.clone_source
        {
            return crate::account::github::clone_readonly_from(
                Path::new("git"),
                source,
                &new.github,
                dest,
            );
        }
        if new.practice {
            return crate::account::github::clone_readonly_into(&new.github, dest);
        }
        #[cfg(feature = "test-support")]
        if let Some(source) = &new.clone_source {
            return crate::account::github::clone_from(
                Path::new("git"),
                source,
                &new.github,
                dest,
                &exe,
                cfg,
            );
        }
        crate::account::github::clone_into(&new.github, dest, &exe, cfg)
    }

    /// Open the run on the server, before the daemon creates anything; for a continued run, ask
    /// the server for its next segment.
    fn open(
        &self,
        cx: &CaptureRun<'_>,
        run: &RunDir,
        session_id: &str,
        harness_version: &str,
    ) -> Result<OpenedRun, CliError> {
        let c = cx.connector;
        let mut request = crate::upload::api::OpenRunRequest {
            connector_id: c.loaded.manifest.id.clone(),
            manifest_sha256: c.loaded.profile_sha256(),
            connector_dir_sha256: c.loaded.connector_dir_sha256.clone(),
            workspace: run.workspace.display().to_string(),
            session_id: session_id.to_string(),
            harness_version: harness_version.to_string(),
            tracer_version: crate::CLI_VERSION.to_string(),
            repo_url: String::new(),
            machine: None,
        };
        let new = match &self.how {
            UploadHow::Fresh(new) => new,
            UploadHow::Continue {
                run_id,
                previous_entry_id,
                known_entries,
            } => {
                return self.open_segment(
                    &request,
                    run_id,
                    previous_entry_id,
                    known_entries,
                    cx.over,
                );
            }
        };
        request.repo_url = new.github.repo_url.clone();
        request.machine = Some(goalcup_platform::machine::facts(&artifact_root(cx.cfg)));
        crate::upload::give_back_pending(self.server.base(), cx.cfg);
        // Taken before the first ask: a repeat after a lost reply answers the first
        // `official_start`, and the run's own clock must never start later than the server's.
        let opened_at = goalcup_protocol::clock::ContinuousOrigin::now();
        let run = crate::upload::open_run(&self.server, &request)?;
        // The run is bound to the credential now, and the credential is kept in the run's own
        // directory, where `goalcup upload` and `continue` find it from the entry. Both the
        // note of the open and the credential are written before anything else can fail.
        let kept = crate::upload::GiveBack::opened(&self.server, cx.cfg, &run).and_then(|give| {
            crate::account::credential::store(
                &crate::run::run_dir(cx.cfg, &run.run_id),
                &new.credential,
            )
            .map(|()| give)
        });
        match kept {
            Ok(give_back) => Ok(OpenedRun {
                run,
                request,
                opened_at,
                give_back: std::cell::RefCell::new(Some(give_back)),
                segment: None,
            }),
            Err(error) => {
                // The note of the open and the credential are deleted only once the server has
                // taken the run back; while it is away they are the only way to give it back.
                if !crate::upload::undo_open(&self.server, cx.cfg, &run.run_id) {
                    remove_run_keeping_pending_credential(cx.cfg, &run.run_id);
                    let kept = crate::upload::pending_path(cx.cfg).exists()
                        && crate::account::credential::credential_path(&crate::run::run_dir(
                            cx.cfg,
                            &run.run_id,
                        ))
                        .exists();
                    crate::host_log::say(&if kept {
                        format!(
                            "run {} could not be given back to the server just now; the next \
                             goalcup start gives it back",
                            run.run_id
                        )
                    } else {
                        format!(
                            "run {} could not be given back to the server, and this machine \
                             cannot ask again; the server ends it at its deadline or when your \
                             account starts another",
                            run.run_id
                        )
                    });
                }
                Err(error)
            }
        }
    }

    /// `POST /v1/runs/{id}/continue`, as the answer of an open. A run's latest entry on the
    /// server that this machine does not know is a segment an earlier `goalcup continue` opened
    /// and did not launch (the daemon or the terminal failed after the server had answered):
    /// this segment follows that one, which stays on the server with no chain, once.
    fn open_segment(
        &self,
        carried: &crate::upload::api::OpenRunRequest,
        run_id: &str,
        previous_entry_id: &str,
        known_entries: &[String],
        run_over: &std::cell::Cell<bool>,
    ) -> Result<OpenedRun, CliError> {
        use crate::upload::Continued;
        let over = || {
            run_over.set(true);
            CliError::Refused(format!(
                "run {run_id} is over: its deadline has passed, or a newer run on your account \
                 replaced it, so there is nothing left to continue. Nothing was started."
            ))
        };
        let opened_at = goalcup_protocol::clock::ContinuousOrigin::now();
        let mut previous = previous_entry_id.to_string();
        let mut followed = false;
        let segment = loop {
            let request = crate::upload::api::ContinueRequest {
                previous_entry_id: previous.clone(),
                connector_id: carried.connector_id.clone(),
                manifest_sha256: carried.manifest_sha256.clone(),
                connector_dir_sha256: carried.connector_dir_sha256.clone(),
                workspace: carried.workspace.clone(),
                session_id: carried.session_id.clone(),
                harness_version: carried.harness_version.clone(),
                tracer_version: carried.tracer_version.clone(),
            };
            match crate::upload::continue_run(&self.server, run_id, &request)? {
                Continued::Segment(segment) => break segment,
                Continued::Over => return Err(over()),
                Continued::Stale => {
                    let latest: crate::upload::api::RunPosition = self
                        .server
                        .json("GET", &format!("/v1/runs/{run_id}"), None::<&()>)
                        .map_err(|error| {
                            CliError::Refused(format!(
                                "the server did not say where run {run_id} is: {error}. \
                                 Nothing was started."
                            ))
                        })?;
                    if latest.stop {
                        return Err(over());
                    }
                    if followed || known_entries.contains(&latest.entry_id) {
                        return Err(CliError::Refused(format!(
                            "the server's latest entry of run {run_id} is {}, which is not the \
                             entry this machine ended last ({previous}). The run was continued \
                             from somewhere else, and a run stays on the machine that started \
                             it. Nothing was started.",
                            latest.entry_id
                        )));
                    }
                    followed = true;
                    previous = latest.entry_id;
                }
            }
        };
        Ok(OpenedRun {
            run: crate::upload::api::OpenRun {
                run_id: run_id.to_string(),
                entry_id: segment.entry_id,
                official_start: segment.official_start,
                deadline_at: segment.deadline_at,
                seconds_left: segment.seconds_left,
            },
            request: carried.clone(),
            opened_at,
            // A segment cannot be given back: the run is the participant's, and stays open.
            give_back: std::cell::RefCell::new(None),
            segment: Some(segment.segment),
        })
    }

    /// The daemon made the entry under the server's id: fetch the start receipt over its
    /// baseline, keep it, and say what the server stamped.
    fn bind(
        &self,
        cfg: &Path,
        opened: &OpenedRun,
        info: &goalcup_protocol::control::CaptureWorkspaceInfo,
        out: &mut dyn std::io::Write,
    ) -> Result<(crate::upload::UploadRecord, String), CliError> {
        if info.entry_id != opened.run.entry_id {
            return Err(CliError::Internal(format!(
                "the daemon created entry {} for the server's entry {}",
                info.entry_id, opened.run.entry_id
            )));
        }
        let receipt = crate::upload::fetch_receipt(
            &self.server,
            &opened.run,
            &opened.request,
            &info.chain_head,
        )?;
        let record = crate::upload::UploadRecord {
            run_id: opened.run.run_id.clone(),
            entry_id: opened.run.entry_id.clone(),
            server_url: self.server.base().to_string(),
            trace_dir: info.trace_dir.clone(),
            official_start: opened.run.official_start.clone(),
            deadline_at: opened.run.deadline_at.clone(),
            workspace: opened.request.workspace.clone(),
        };
        crate::upload::keep_run(cfg, &record, &receipt)?;
        match opened.segment {
            Some(segment) => say(
                out,
                &format!(
                    "run {} continued on {}: segment {segment}, deadline {}",
                    record.run_id, record.server_url, record.deadline_at
                ),
            )?,
            None => say(
                out,
                &format!(
                    "run {} opened on {}: official start {}, deadline {}",
                    record.run_id, record.server_url, record.official_start, record.deadline_at
                ),
            )?,
        }
        Ok((record, receipt.public_key))
    }
}

/// Remove this machine's directory for a run that never launched. When the note of the open
/// still names the run (the server was away when it was given back), the credential stays: it is
/// the only thing that can give that run back later.
fn remove_run_keeping_pending_credential(cfg: &Path, run_id: &str) {
    let pending = std::fs::read(crate::upload::pending_path(cfg))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<crate::upload::PendingOpen>(&bytes).ok())
        .is_some_and(|note| note.run_id == run_id);
    if !pending {
        crate::run::remove(cfg, run_id);
        return;
    }
    let dir = crate::run::run_dir(cfg, run_id);
    let keep = crate::account::credential::CREDENTIAL_FILE_NAME;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.file_name() != keep {
                let path = entry.path();
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                };
            }
        }
    }
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
) -> bool {
    let reason = format!("`goalcup start` failed before the launch: {cause}");
    let sentence = format!(
        "Entry {entry_id} is abandoned: nothing was launched, and its chain and this \
         machine's note of it are gone."
    );
    let abandoned = abandon_because(client, cfg, entry_id, &reason, &sentence, out);
    if abandoned && let Err(e) = run.remove() {
        let _ = say(
            out,
            &format!(
                "  its run directory {} could not be removed: {e}",
                run.dir.display()
            ),
        );
    }
    abandoned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::MockDaemonClient;

    /// The interrupt flag is process-global; the tests that move it take turns.
    static FLAG: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A continued segment's run directory is unmade on a failed launch, and the run's clone,
    /// which the participant's agent has worked in, is not.
    #[test]
    fn unmaking_a_continued_segment_leaves_the_runs_clone_with_its_files() {
        let tmp = tempfile::tempdir().unwrap();
        let clone = tmp.path().join("clone");
        std::fs::create_dir_all(clone.join(".git")).unwrap();
        std::fs::write(clone.join("hello.txt"), b"the agent's work").unwrap();
        let beside = tmp.path().join("beside");
        std::fs::create_dir_all(&beside).unwrap();
        let env = BTreeMap::new();
        let run = RunDir::create(
            &beside,
            RunHome::Empty,
            None,
            &env,
            WorkspaceAt::Existing(&clone),
        )
        .unwrap();
        assert_eq!(run.workspace, clone);
        run.remove().unwrap();
        assert!(!run.dir.exists(), "the segment's own folder goes");
        assert_eq!(
            std::fs::read(clone.join("hello.txt")).unwrap(),
            b"the agent's work"
        );
        assert!(clone.join(".git").is_dir());
        // A fresh run's own clone is its to remove.
        let fresh = RunDir::create(
            &beside,
            RunHome::Empty,
            None,
            &env,
            WorkspaceAt::Clone(tmp.path()),
        )
        .unwrap();
        std::fs::create_dir_all(&fresh.workspace).unwrap();
        fresh.remove().unwrap();
        assert!(!fresh.workspace.exists());
    }

    /// The reserve of a run that goes on is dropped, not released, when a launch fails.
    #[test]
    fn a_failed_launch_keeps_the_runs_reserve_file_only_when_the_run_goes_on() {
        let tmp = tempfile::tempdir().unwrap();
        let policy = DiskPolicy::from_test_spec("4194304,0,50").unwrap();
        let path = reserve_path(tmp.path(), "r1");
        end_reserve(RunReserve::take(&path, &policy).unwrap(), true);
        assert!(path.exists(), "kept for the run's next segment");
        assert!(!crate::run::reserve::held_by_another(&path), "and unlocked");
        end_reserve(RunReserve::take(&path, &policy).unwrap(), false);
        assert!(!path.exists(), "released");
    }

    /// The recording the heartbeat reports is the entry's own signed status: a document signed by
    /// another key, about another entry, or edited after it was signed says nothing.
    #[test]
    fn the_recording_is_read_from_the_verified_status_only() {
        use goalcup_protocol::capture::*;
        let tmp = tempfile::tempdir().unwrap();
        let daemon = MockDaemonClient::armed_ok(tmp.path(), "fake");
        daemon.set_capture_status(
            M2Status {
                run: RunState::Running,
                capture: CaptureState::Incomplete,
                delivery: DeliveryState::LocalOnly,
                analysis: AnalysisState::NotRun,
                anchor: AnchorState::LocalOnly,
                storage: CaptureStorage {
                    quota_bytes: 1,
                    reserved_bytes: 1,
                    used_bytes: 0,
                    durable: None,
                    accounting_available: true,
                },
                gap_count: 1,
                assistance_observed: false,
            },
            "gaps",
            1,
            0,
        );
        let info = daemon.info();
        let key = info.run_public_key.clone();
        let entry = info.entry_id.clone();
        assert_eq!(
            recording_of(&daemon, &entry, &key),
            Some(crate::run::Recording::Gaps),
            "signed by the entry's key, about the entry"
        );
        // Another run key: a process that bound the control port first, with a key of its own.
        let other = "00".repeat(32);
        assert_eq!(recording_of(&daemon, &entry, &other), None);
        // A good signature about another entry.
        assert_eq!(recording_of(&daemon, "entry-other", &key), None);
        // Edited after it was signed.
        daemon.tamper_status_payload();
        assert_eq!(recording_of(&daemon, &entry, &key), None);
    }

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
