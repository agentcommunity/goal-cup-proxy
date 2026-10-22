//! `goalcup start` — a protocol-v5 capture run: the preflight, the daemon's capture, the launch
//! through the recording proxy, and the seal.
//!
//! No grant, no arming. The harness is launched through the recording proxy, which runs inside
//! this process, and the run ends with a seal (see `capture` below for the order). Every
//! connector is a chat-mode one (D57: `[capture] interactive = true`; headless mode, with its
//! `--goal-file`, was removed): the harness runs in its normal chat mode in the participant's own
//! terminal and the participant types the prompt.
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
//! agent because the run's end had come (the deadline, or a newer run replacing it) or its time
//! is up by the count of the server's last answer, which the run's `state.json` keeps for every
//! later command ([`crate::run::TimeLeft`]; never this machine's date). A run with no server
//! has no segments: its end is its end.
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

mod finish;
mod forked;
mod phases;
mod proxy;
mod refusals;
mod server;

#[cfg(feature = "test-support")]
pub use crate::home::validate_shared_capture_home_for_uid;
pub use phases::CHAT_MODE_GOAL;
#[cfg(feature = "test-support")]
pub use refusals::{TEST_GITHUB_API, TEST_OFFLINE_UPSTREAMS};
pub(crate) use refusals::{check_run_folders, continue_preflight, run_folder_for_continue};
#[cfg(feature = "test-support")]
pub use server::TEST_CLONE_SOURCE;

use crate::CliError;
use crate::commands::say;
use crate::connector::HarnessConnector;
use crate::proxy::tls::UpstreamRoots;
use crate::run::RunRecord;
use crate::run::reserve::{DiskPolicy, artifact_root, release_stale};
use crate::upload::policy::WrapUp;
use finish::finish_run;
use goalcup_protocol::control::DaemonClient;
use goalcup_protocol::harness::CaptureSpec;
use phases::{claim_shared_home, create_and_gate, prepare, register_and_release, run_harness};
use refusals::{CLEANED_UP, interrupted_refusal, prepared_work_folder};
use server::{Uploading, uploading};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

/// What one `goalcup start` was asked to do.
pub struct StartOptions {
    /// The connector id, for example `codex-cli`: a folder under the connectors directory.
    pub harness: String,
    /// `--dev`: admit a rulebook this build does not pin, and say so on stdout. The one
    /// door ([`crate::preflight::DEV_DOOR`]), typed by a human and never set by a shipped
    /// code path.
    pub dev: bool,
    /// Where the run makes its own folder (D57): the directory `goalcup start` was run from,
    /// which `main.rs` reads. `None` reads this process's current directory.
    pub run_folder: Option<PathBuf>,
    /// The sizes of the run's disk reservation and how often it is watched. The shipped values
    /// ([`DiskPolicy::default`]); only a test build can name others.
    pub disk: DiskPolicy,
    /// When the watch tick says the harness has written no session file while model requests are
    /// recorded ([`crate::run::NativeSilence::default`] in every shipped build).
    pub native_silence: crate::run::NativeSilence,
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

/// [`start_with`] with the recording proxy trusting `roots` upstream instead of the public web
/// roots, so a test can stand up a TLS upstream of its own. A shipped build has no such door.
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
    // terminal clear (`crate::host_log`).
    let mut routed = crate::host_log::Routed::new(out);
    capture(
        client,
        connectors,
        cfg,
        env,
        opts,
        roots,
        opening,
        &mut routed,
    )
}

// ------------------------------------------------------------------ v5 local capture

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

/// What every phase of one capture run is given and none changes.
struct CaptureRun<'a> {
    client: &'a (dyn DaemonClient + Sync),
    /// The upload server this run opens with, or `None` for a local-only run.
    upload: Option<&'a Uploading>,
    cfg: &'a Path,
    env: &'a BTreeMap<String, String>,
    opts: &'a StartOptions,
    /// The folder the run's own folder is made in: the one `goalcup start` was run from.
    beside: &'a Path,
    connector: &'a HarnessConnector,
    profile: &'a CaptureSpec,
    /// The model service the harness's own config names, read for this segment (D62).
    model_host: Option<&'a crate::model_host::ModelHost>,
    /// Set when an abandon was refused, so some of what the start made is still there: the
    /// cleanup sentence ([`CLEANED_UP`]) is not true then, and is not said.
    uncleaned: &'a std::cell::Cell<bool>,
    /// Set when the server said the run being continued is over (`410`): the earlier segments'
    /// uploads are then waited for like a stop's, not for a refusal's twenty seconds.
    over: &'a std::cell::Cell<bool>,
    /// The run this start continues, for `goalcup continue`; `None` for a new run.
    continuing: Option<&'a Continuing>,
}

/// A v5 local capture run.
///
/// The order is the daemon's: the disk is checked, the entry is created (its chain is quota-only),
/// the run's one reservation is taken (a refusal abandons the entry), the recording proxy starts in this process and its sender hands the daemon a producer, the entry's own
/// status must show that producer admitted, the goal is recorded once, and only then is the
/// harness spawned; its process is registered before its launch is recorded. When it exits
/// the recording proxy stops, the seal carries the recording proxy's counters, and the sender
/// drains.
///
/// Every failure before the spawn abandons the entry, which removes its chain and
/// still-empty workspace, releases the run's reservation, and removes this run's directory. From the spawn on nothing is
/// abandoned: a harness this command could not hand over is killed, and the entry is sealed,
/// so what it did stays on the record.
///
/// Five phases, whose boundaries are the window rules above: [`prepare`] (every refusal that
/// precedes anything created), [`create_and_gate`] (the abandon window, which ends at the fork),
/// [`register_and_release`], [`run_harness`] and [`finish_run`] (the entry is sealed from the
/// fork on).
#[allow(clippy::too_many_arguments)]
fn capture(
    client: &(dyn DaemonClient + Sync),
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &StartOptions,
    roots: UpstreamRoots,
    opening: Opening,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    let beside = match &opts.run_folder {
        Some(folder) => folder.clone(),
        None => std::env::current_dir().map_err(|source| CliError::Io {
            path: "the current directory".to_string(),
            source,
        })?,
    };
    // Under WSL: WSL1 cannot run this, and WSL2 cannot keep a run on a Windows drive. Judged
    // before anything is created or removed, over every folder the run writes to; a folder on a
    // filesystem a run cannot keep its records on is only warned about.
    let work_folder = match &opening {
        Opening::Fresh => prepared_work_folder(cfg),
        Opening::Continue(continuing) => {
            continuing.record.workspace.parent().map(Path::to_path_buf)
        }
    };
    check_run_folders(
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
    // The model service the harness's own config names (D62): read once for this segment,
    // so a continue after the participant switched service records the new one.
    let model_host = crate::model_host::for_run(cfg, &c)?;
    // With a server: the prep checks, before any other work and before the participant is asked
    // for anything. A continued run was prepared when it started.
    let serving = match &continuing {
        None => uploading(cfg, env, &beside, opts.mode)?,
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
        model_host: model_host.as_ref(),
        uncleaned: &uncleaned,
        over: &over,
        continuing: continuing.as_deref(),
    };
    let mut prep = prepare(&cx, out)?;
    prep.shared_home_lock = held_home_lock;
    // Every refusal that needs no approval has been made: now the participant approves this
    // run in the browser. Nothing is created until they have. A continued run is approved
    // already: its credential is the one it was approved with.
    let upload = match (serving, &continuing) {
        (Some(serving), _) => Some(serving.approved(opts, &c, out)?),
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
    let released = register_and_release(forked, gate, out)?;
    let exited = run_harness(&cx, released, sleep_lock, out)?;
    let finished = finish_run(exited, std::mem::take(&mut draining.drains), out);
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
