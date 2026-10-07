//! `goalcup start` — the preflight, the daemon's workspace, the launch, and the abandon
//! that keeps a failed start from leaving anything behind.
//!
//! # The order is the feature
//!
//! Every step before `create_workspace` is a refusal the participant can act on **before
//! anything exists**: no grant, a harness D23's table does not call `full`, a rulebook this
//! build does not pin, a banned variable in the environment, a port already held. Starting
//! under a rulebook the verifier will later call altered wastes 168 hours, so the question
//! is asked here, where the answer is still worth something.
//!
//! After that the order is fixed for a different reason. `create_workspace` is where the
//! **daemon** creates the empty workspace, places the artifact root outside it and writes
//! the folder-scoped config, which is why this file writes nothing into the workspace at
//! all. Then, immediately, the entry cache, which is the one file this command writes
//! anywhere and is written **before** `arm` so that a start which dies between the two still
//! leaves a record `abandon` and the participant can name. Then `arm`, and only then
//! `register_process`: D22 says the daemon may terminate this process from the moment it
//! knows about it, so it learns about it after the folder is armed and not before.
//!
//! # The abandon window
//!
//! A failure before the prompt is submitted leaves no entry and the participant may retry.
//! That promise is only true if something removes it, so **every failure between
//! `create_workspace` and a successful `register_process` calls `client.abandon`** — and
//! takes the local record with it, and kills the child if one was already spawned: a harness
//! left running against an entry the daemon has just dropped is measured by nothing and
//! sealed by nothing.
//!
//! **`--print-launch` abandons on the way out of a run that succeeded.** It is the one
//! caller that unmakes its entry with no failure to unmake it over: the flag is documented
//! "run nothing". The entry is genuinely needed to render the line — which names the
//! workspace, the artifact root and the session the daemon minted — so it is created,
//! printed from, and then abandoned through the same cleanup every failure uses.
//!
//! The window closes at `register_process`, deliberately. Past that the daemon owns the
//! process and the run is stamped; `abandon` is refused once `started_at` exists, and a CLI
//! that asked anyway would be asking the daemon to delete a live entry because its own
//! `wait()` returned an error.
//!
//! **The interrupt flag is read inside that window and nowhere after it.** A launched TUI
//! shares this terminal, so the Ctrl-C a participant types at their harness lands on this
//! process's handler too; a read past `register_process` would turn an armed, recorded,
//! ordinary run into `exit 1` and ask the daemon to abandon a stamped entry on the way out.
//! From `register_process` on, the flag changes this command's last line and nothing else.
//!
//! **And the local record is removed only where the daemon confirmed the abandon.** It is
//! this machine's note of an entry the tracker may still be holding — the pinned run key
//! `status`, `resume` and `seal` all read out of it — so an abandon the daemon refused
//! leaves it standing and says so.
//!
//! # `--goal-file` is a different run, and it has its own order
//!
//! With `--goal-file` this command records a protocol-v5 local capture instead (see
//! `capture` below): no grant, no arming, no server. The harness is launched headless through
//! the recording proxy, which runs inside this process, and the run ends with a local seal.
//! Without the flag everything in this header holds unchanged.
//!
//! # `--dev` is the one door, here as everywhere
//!
//! [`crate::preflight::DEV_DOOR`] names `goalcup start --dev` as one of the two places a
//! human types it. It admits two things at this command and no others — a rulebook this
//! build does not pin, and a machine that has never run `goalcup auth` — it never removes a
//! check, and a run that came through it says both on stdout. The entry record carries `dev`
//! into the seal's printed verify line and the verdict.
//!
//! Nothing is bypassed at the far end by that. `WorkspaceOpts.grant_token` still travels —
//! empty, where there is nothing to send — and it is the server, not this CLI, that decides
//! what an empty one is worth. A judged run is not started with `--dev`, and a judged run
//! without a grant fails at the server.

use crate::CliError;
use crate::arming::GateOutcome;
#[cfg(feature = "v1-compat")]
use crate::connector::Connector;
use crate::connector::HarnessConnector;
use crate::entries::{EntryRecord, remove_entry, write_entry};
use crate::key_scan::FragmentMatcher;
use crate::launch::{
    CaptureStderr, LaunchPlan, LaunchScope, ScopeMarker, StderrUse, StdoutUse, StreamPlan,
    child_secret_for_harness, gated_handle, spawn_gated, tee_marked_capture_with_rule,
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
use crate::retry::{retry_busy, retry_busy_entry, retry_busy_process_classified};
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

// What only the V1 half of this file reads (D42): compiled with `v1-compat` alone.
#[cfg(feature = "v1-compat")]
use crate::arming::GateReport;
#[cfg(feature = "v1-compat")]
use crate::commands::auth::load_grant;
#[cfg(feature = "v1-compat")]
use crate::entries::{ProcessCleanupOrigin, UnresolvedNativeCleanup};
#[cfg(feature = "v1-compat")]
use crate::launch::{LaunchKind, handle, plan, shell_line, spawn};
#[cfg(feature = "v1-compat")]
use crate::preflight::{DEV_DOOR, connector_checks_with_dev};
#[cfg(feature = "v1-compat")]
use goalcup_protocol::control::{ControlError, WorkspaceInfo, WorkspaceOpts};
#[cfg(feature = "v1-compat")]
use goalcup_protocol::manifest::{KillStrategy, LaunchMode};
#[cfg(feature = "v1-compat")]
use goalcup_protocol::process::{
    ProcessInstanceStatus, is_alive, native_pid_is_bound_to_handle, process_handle_instance_status,
    process_instance_status, snapshot,
};

/// The deadline a run is judged against, in hours: RULES.md's 168, one week.
///
/// **A constant and not a flag.** No code under `goalcup-daemon/src/` reads
/// `WorkspaceOpts.deadline_hours`, and the deadline a run is stamped with is the daemon's own
/// `receipt::ENTRY_WINDOW_SECONDS` — 604 800, this same week. A flag whose number is silently
/// replaced is worse than no flag, and the 168 hours is a rule rather than a preference, so
/// this constant travels in its place.
#[cfg(feature = "v1-compat")]
pub const DEFAULT_DEADLINE_HOURS: u32 = 168;

/// What one `goalcup start` was asked to do.
pub struct StartOptions {
    /// The connector id — `claude-code-cli` or `codex-cli`.
    pub harness: String,
    /// Attach mode: arm the folder and let the participant open their own session in it.
    /// Nothing is spawned.
    pub attach: bool,
    /// Print the launch line and stop, without spawning or registering anything.
    pub print_launch: bool,
    /// Hand the status URL to the participant's browser. `--no-open` clears it, which is
    /// the headless machine's path: the URL is printed either way.
    pub open_browser: bool,
    /// `--arming-timeout`, step three of the precedence chain stated once on
    /// `manifest::Arming`: connector default, daemon config, this. `None` leaves the first
    /// two in force.
    pub arming_timeout: Option<u64>,
    /// `--dev`: admit a rulebook this build does not pin, and say so on stdout. The one
    /// door ([`crate::preflight::DEV_DOOR`]), typed by a human and never set by a shipped
    /// code path.
    pub dev: bool,
    /// `--goal-file`: a local protocol-v5 capture run, launched headless with this file's
    /// text as its one goal. `None` is the legacy run.
    pub goal_file: Option<PathBuf>,
    /// `--capture-reserve-bytes`: the chain bytes the daemon reserves before a v5 launch.
    pub capture_reserve_bytes: u64,
    /// One explicitly supplied Claude M2 Bash rule; absent on ordinary runs.
    pub capture_claude_allowed_tool: Option<String>,
}

/// What one `goalcup start` did.
#[derive(Debug)]
pub struct StartOutcome {
    pub entry_id: String,
    pub gate: GateOutcome,
    /// How the harness exited, where this command launched one and waited for it. `None`
    /// for attach mode and for `--print-launch`, which spawn nothing.
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

    if opts.capture_claude_allowed_tool.is_some() && opts.goal_file.is_none() {
        return Err(CliError::Refused(
            "a Claude capture tool allowance requires --goal-file".into(),
        ));
    }

    if let Some(goal_file) = &opts.goal_file {
        let roots = roots.unwrap_or_else(UpstreamRoots::webpki);
        return capture(client, connectors, cfg, env, opts, goal_file, roots, out);
    }

    #[cfg(feature = "v1-compat")]
    return started_v1(client, connectors, cfg, env, opts, out);
    // D42: the plain `goalcup start` (no `--goal-file`) is the V1 transcript path, compiled
    // only with the `v1-compat` feature. A default build refuses it, and says why. Clap marks
    // `--goal-file` required, so this is a backstop a caller of `start_with` can still reach.
    #[cfg(not(feature = "v1-compat"))]
    {
        let _ = (client, connectors, cfg, env, out);
        Err(CliError::Refused(
            "`goalcup start` without `--goal-file` is the V1 transcript path, which this build \
             does not include (it compiles only with the `v1-compat` cargo feature). Name the \
             goal: `goalcup start --harness <id> --goal-file <file>`."
                .to_string(),
        ))
    }
}

/// The V1 half of `started`: the grant, the preflight, the daemon's workspace and everything
/// after it. `v1-compat` only.
#[cfg(feature = "v1-compat")]
fn started_v1(
    client: &(dyn DaemonClient + Sync),
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &StartOptions,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    // 1. The grant. Nothing is asked of the daemon by a machine that has not authenticated:
    //    the daemon exchanges this token with the server, and a start without one fails at
    //    the far end after an entry exists. `--dev` is the one door through it — see the
    //    module header — and the banner below says so on the run that came through.
    let grant = load_grant(cfg)?;
    if grant.is_none() && !opts.dev {
        return Err(CliError::Refused(format!(
            "this machine has no /goal Cup grant: run `goalcup auth` first, and start the \
             run again. A development run against a local pair takes `{DEV_DOOR}` instead, \
             and says so in its result."
        )));
    }

    // 2. The connector, and 3. every question about it that can be asked before a run:
    //    D23's tier table, the release-hash allowlist over the connector directory,
    //    D12's tested version range, the manifest's banned variables and required paths, and
    //    the port its launch reserves.
    let c = crate::connector::load(connectors, &opts.harness)?;
    if opts.dev {
        say(
            out,
            &format!(
                "{DEV_DOOR} was typed: this run may use a rulebook this build does not pin, \
                 and its result is a development run's rather than a judged entry's."
            ),
        )?;
        // The second admission, named separately and only where it was actually used: a
        // developer who has a grant and typed `--dev` for the rulebook must not be told
        // their run went out without one.
        if grant.is_none() {
            say(
                out,
                &format!(
                    "{DEV_DOOR} also admits a machine that has never run `goalcup auth`: \
                     this run carries no grant, so the server it uploads to is vouching for \
                     nobody."
                ),
            )?;
        }
    }
    let report = connector_checks_with_dev(&c, env, cfg, opts.dev);
    for check in report.checks.iter().filter(|c| c.level == Level::Warn) {
        // A warning informs and never refuses, so it goes to the participant's screen
        // rather than into an error nobody sees on a run that worked.
        say(out, check.render().trim_end())?;
    }
    if report.failed() {
        return Err(CliError::Refused(format!(
            "`goalcup start` refuses this run:\n{}",
            report
                .checks
                .iter()
                .filter(|c| c.level == Level::Fail)
                .map(|c| c.render())
                .collect::<String>()
                .trim_end()
        )));
    }

    // 4. The workspace. The daemon creates it, places the artifact root outside it and
    //    writes the folder-scoped config; this CLI writes nothing into the workspace, now or
    //    later.
    let ws_opts = WorkspaceOpts {
        connector_id: c.manifest.id.clone(),
        manifest_sha256: c.manifest_sha256.clone(),
        // No `--workspace-parent` flag exists: whether a participant may choose where the
        // workspace lands is a rules question.
        workspace_parent: None,
        attach: opts.attach,
        // [`DEFAULT_DEADLINE_HOURS`], always: the window is a rule and not an option, and
        // the daemon stamps its own `ENTRY_WINDOW_SECONDS` regardless. Still sent, because
        // the field is on the wire and a CLI that left it out would be describing a
        // different run from the one the daemon creates.
        deadline_hours: DEFAULT_DEADLINE_HOURS,
        // Empty where `--dev` admitted a machine with no grant. Still sent, because the
        // field is on the wire and what an empty token is worth is the server's judgement
        // and not this CLI's.
        grant_token: grant
            .as_ref()
            .map(|g| g.grant_token.clone())
            .unwrap_or_default(),
        arming_timeout_seconds: opts.arming_timeout,
        // What the preflight above measured, seconds ago. The daemon runs no version command of
        // its own, so without it the signed status document would say `harness: unknown` about
        // a harness `goalcup doctor` had just printed a number for. `None` where nothing could
        // be measured, and never an empty string: an empty string is a measurement of nothing
        // dressed as one.
        harness_version: report.harness_version.clone(),
    };
    let info = retry_busy(|| client.create_workspace(&opts.harness, &ws_opts))?;

    // From here on the entry exists, and a refusal inside the window has to unmake it.
    match after_the_workspace(client, &c, cfg, env, opts, &info, out) {
        Ok(outcome) => Ok(outcome),
        Err(failed) if failed.abandon => {
            abandon(client, cfg, &info.entry_id, &failed.error, out);
            Err(failed.error)
        }
        Err(failed) => Err(failed.error),
    }
}

/// A failure, and whether the entry it happened in is still this command's to unmake.
///
/// `abandon` is `false` from the moment `register_process` succeeds: past that the daemon
/// owns the process, the run is stamped, and `abandon` is refused on a stamped entry anyway
/// — a CLI that asked would be asking the daemon to delete a live run because its own
/// `wait()` returned an error.
#[cfg(feature = "v1-compat")]
struct StartFailure {
    error: CliError,
    abandon: bool,
}

/// Everything raised with `?` below happens inside the window; the failures that do not say
/// so by constructing this struct themselves.
#[cfg(feature = "v1-compat")]
impl From<CliError> for StartFailure {
    fn from(error: CliError) -> StartFailure {
        StartFailure {
            error,
            abandon: true,
        }
    }
}

/// The daemon's own refusals reach `?` unwrapped, and land on the same rung
/// `CliError::Control` gives them.
#[cfg(feature = "v1-compat")]
impl From<goalcup_protocol::control::ControlError> for StartFailure {
    fn from(e: goalcup_protocol::control::ControlError) -> StartFailure {
        StartFailure::from(CliError::Control(e))
    }
}

/// Everything after `create_workspace`, so the one caller above can wrap all of it in the
/// abandon.
#[cfg(feature = "v1-compat")]
fn after_the_workspace(
    client: &(dyn DaemonClient + Sync),
    c: &Connector,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &StartOptions,
    info: &WorkspaceInfo,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, StartFailure> {
    // 4a. The port the daemon reserved for this run, before anything is launched onto it. The
    //     launch renders `--port {server_port}` from this number and the daemon does not
    //     re-reserve it, so anything that took it between the reservation and now is a
    //     harness that binds nothing. `None` for every connector that reserves no port, which
    //     is every shipped one, so no line is printed. `goalcup resume` asks the same
    //     question of the port its relaunch will bind.
    if let Some(check) = crate::preflight::server_port_gate(info.server_port) {
        match check.level {
            Level::Fail => {
                return Err(StartFailure::from(CliError::Refused(
                    check.render().trim_end().to_string(),
                )));
            }
            _ => say(out, check.render().trim_end()).map_err(StartFailure::from)?,
        }
    }

    // 4b. The status URL, before it is written into anything. `goalcup ui` and `goalcup
    //     resume` both open the copy of this string that the next line caches, so this is
    //     where an unusable one enters the record — and refusing it here, inside the abandon
    //     window, leaves nothing behind rather than leaving a record whose page two later
    //     commands will refuse. A daemon whose page is not on a loopback port is not this
    //     machine's daemon.
    crate::browser::check_page_url(&info.status_url, crate::browser::PageKind::Daemon)?;

    // 5. The entry cache, before the arm: a start that dies between the two still leaves a
    //    record `abandon` and the participant can name, and it is where every later process
    //    on this machine reads the pinned run key from.
    write_entry(cfg, &record_of(c, info, None, opts.dev))?;

    // 6. Arm the folder.
    retry_busy_entry(|| client.arm(&info.entry_id))?;

    // From the arm on, a line that could not be written is not a reason to unmake the run.
    // The folder is armed, the daemon is watching it, and the only thing that failed is this
    // process's own stdout — a terminal that closed, a pipe whose reader quit. Abandoning
    // there would delete a live workspace because the participant pressed `q` in a pager, so
    // the failure is reported with the entry left standing, exactly as it is after the
    // hand-off in [`launch`].
    let stands = |error: CliError| StartFailure {
        error,
        abandon: false,
    };

    // 7. The URL is printed whether or not a browser is opened, so a headless machine
    //    still gets it. A browser that could not be opened is not a failure — the URL is
    //    already on the screen.
    say(out, &format!("entry {}", info.entry_id)).map_err(stands)?;
    say(out, &format!("workspace: {}", info.workspace)).map_err(stands)?;
    // The second spelling of the workspace (D16), printed only where it is a second spelling. The
    // daemon answers the canonical path everywhere and carries the path it was asked for
    // beside it; under a symlinked parent — `/var` is a link to `/private/var`, which is where
    // a macOS temporary directory lands — those are two names for one directory, and a
    // participant who typed one and reads the other is owed the sentence that says why.
    if info.workspace_requested != info.workspace {
        say(
            out,
            &format!(
                "  (asked for {}, which resolves to the line above)",
                info.workspace_requested
            ),
        )
        .map_err(stands)?;
    }
    say(out, &format!("status: {}", info.status_url)).map_err(stands)?;
    if opts.open_browser
        // The same string passed [`crate::browser::check_page_url`] above, so the only
        // thing this can report is a browser that would not open — and past the arm that is
        // never a reason to unmake a run.
        && let Some(note) = crate::browser::open_page(
            &info.status_url,
            crate::browser::PageKind::Daemon,
            &crate::browser::system_opener,
        )
        .map_err(stands)?
    {
        say(out, &note).map_err(stands)?;
    }

    // 8a. Attach mode builds no plan at all: the folder is armed and the participant opens
    //     their own session in it, with the connector's human step on screen where there is
    //     one.
    if opts.attach {
        say(
            out,
            &format!(
                "Open your harness in {} — the folder is armed and the tracker is watching \
                 it.",
                info.workspace
            ),
        )
        .map_err(stands)?;
        // The gate: the terminal is free, so it prints what it saw here. The key this run was
        // handed and the entry cache pinned a few lines above: the gate checks the status
        // document against it before that document decides an abandon.
        let report =
            crate::arming::wait_foreground(client, c, &info.entry_id, &info.run_public_key, out);
        return Ok(StartOutcome {
            entry_id: info.entry_id.clone(),
            gate: verdict(report, &info.entry_id, Window::Open)?,
            exit_status: None,
        });
    }

    // 8b. Launch mode. The environment the harness starts from is this process's own, minus
    //     D23's denylist — `scrubbed_env` is where that subtraction happens and `plan` is
    //     pure because it happens before the call rather than inside it. It is this
    //     process's environment and not the `env` map above deliberately: that map is what
    //     the preflight asks its questions of, and a caller may narrow it to ask a gate
    //     something specific, while the child inherits the participant's real shell.
    let vars = crate::vars::from_workspace(info);
    let launch_env = crate::launch::scrubbed_env(&crate::launch::process_env());
    let plan = plan(
        c,
        &vars,
        Path::new(&info.workspace),
        LaunchKind::Start,
        &launch_env,
    )?;
    if opts.print_launch {
        say(out, &shell_line(&plan)).map_err(stands)?;
        // **Not a placeholder, and not a gate that was skipped.** `--print-launch` spawns
        // nothing: no harness has been started in this workspace, so no transcript record is
        // coming. The honest answer is that there was nothing to wait for, which is what
        // this arm means. The interrupt is still read, because the entry does exist.
        interrupt_check()?;
        // …and then the entry goes: `--help` says this flag runs nothing, and an entry left
        // `armed  arming_pending` in `goalcup entries` would be an armed run nobody will ever
        // start, holding the daemon's attention until its deadline. The rendering genuinely
        // needs the entry: the launch line names the workspace, the artifact root and the
        // session the daemon minted, and a line rendered against invented values would be a
        // line that does not work. So the entry is created, printed from, and then abandoned.
        say(
            out,
            "`--print-launch` ran nothing, so the entry this line was rendered from is being \
             abandoned.",
        )
        .map_err(stands)?;
        abandon_because(
            client,
            cfg,
            &info.entry_id,
            "`goalcup start --print-launch` printed the launch line and ran nothing",
            &abandoned_sentence(&info.entry_id),
            out,
        );
        return Ok(StartOutcome {
            entry_id: info.entry_id.clone(),
            gate: GateOutcome::NotRequired,
            exit_status: None,
        });
    }
    // 8c. The per-run harness home is fresh, and a harness that keeps its first-run state
    //     there meets a brand-new user on every start. The connector says which keys to copy
    //     out of the participant's own configuration and this reads them — never the other
    //     way round. Before the spawn and after the print, because a `--print-launch` that
    //     seeded would write into a home it is about to abandon. `carried` is what the
    //     seeding took in, for the daemon to chain when this launch registers its process.
    //     `None` for a connector that declares no `[home_seed]` and for a seeding that
    //     failed: a record of a seeding that did not happen would be a claim.
    let mut carried = None;
    if let Some(spec) = &c.manifest.home_seed {
        let seeded = crate::per_run_home::home_from(env).and_then(|home| {
            crate::home_seed::seed(
                &c.manifest.id,
                spec,
                &home,
                Path::new(&info.harness_home),
                Path::new(&info.workspace),
            )
        });
        match seeded {
            Ok(seeded) => {
                carried = Some(seeded.record.clone());
                say(out, &seeded.line()).map_err(stands)?
            }
            // **A seed with `required` keys is a gate; one without is a refresh.**
            // Claude Code's names two keys without which the launch meets a login
            // screen no unattended run can answer, so a machine that cannot supply them is
            // refused before the entry is spent. Codex's copies a credential and requires
            // nothing: a home that was not seeded meets a sign-in screen a person can still
            // answer, and `CODEX_HOME=<entry home> codex login` is the other way round it.
            // Refusing there would take a run away over a step the participant can take.
            Err(e) if spec.required.is_empty() => say(
                out,
                &format!(
                    "this run's harness home could not be seeded from this machine's \
                     configuration: {e}\n  The run goes ahead. If the harness asks you to \
                     sign in, sign in inside it, or sign in against this run's own home \
                     first — README, \"The per-run home\", names the command."
                ),
            )
            .map_err(stands)?,
            Err(e) => return Err(StartFailure::from(e)),
        }
    }
    launch(client, c, cfg, info, &plan, opts, carried, out)
}

/// Spawn the harness, hand the daemon the process, and wait for it.
///
/// The kill on the failure path is the one place this crate ends a process it started, and
/// it is not the rule `tests/process_handoff.rs` guards: terminating a **run** is
/// `goalcup_daemon::terminate`, at the deadline, with a grace period. This is a child
/// spawned seconds ago that the daemon was never successfully told about — nothing will
/// measure it, nothing will seal what it writes, and the entry it belongs to is about to be
/// abandoned.
// The allow is for the wide signature: `carried` is a parameter rather than a value this
// function derives because the seeding happens above it, once, before the spawn — there is
// nothing here to recompute it from. The same allow the daemon's own wide functions carry
// (`runtime/entry.rs`, `control/handlers.rs`).
#[allow(clippy::too_many_arguments)]
#[cfg(feature = "v1-compat")]
fn launch(
    client: &(dyn DaemonClient + Sync),
    c: &Connector,
    cfg: &Path,
    info: &WorkspaceInfo,
    plan: &LaunchPlan,
    // The whole options struct rather than values derived from it: `opts.dev` is carried into
    // the record this rewrites (see `crate::entries::EntryRecord::dev`), and two derived
    // parameters where the struct itself would do is how a third one arrives next.
    opts: &StartOptions,
    // What `[home_seed]` carried into this run's harness home, or `None` when nothing did. It
    // is a parameter and not a third derived value read off `opts` because the seeding
    // happens above this function and cannot be recomputed inside it.
    carried: Option<goalcup_protocol::events::HomeSeedRecord>,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, StartFailure> {
    // A group-less native-pid launch may fork before its pinned native name becomes
    // discoverable. Persist the conservative latch before spawning, so even a crash or the
    // next cache write failing cannot erase the fact that cleanup may be incomplete.
    let unresolved_guard = unresolved_native_guard(plan, ProcessCleanupOrigin::Initial);
    if let Some(guard) = unresolved_guard.clone() {
        write_entry(
            cfg,
            &EntryRecord {
                pending_unresolved_native_cleanup: Some(guard),
                ..record_of(c, info, None, opts.dev)
            },
        )?;
    }
    let mut spawned = match spawn(plan) {
        Ok(spawned) => spawned,
        Err(error) => {
            // The syscall returned no Child, so the conservative pre-spawn latch is now
            // known false. Clear it before the caller attempts abandonment; a daemon whose
            // abandon response is unavailable must not leave this machine wedged behind a
            // process marker for a process that never existed.
            if unresolved_guard.is_some() {
                write_entry(cfg, &record_of(c, info, None, opts.dev))?;
            }
            return Err(error.into());
        }
    };
    // The instant the process started, read here and not taken from `info.created_at`.
    //
    // The daemon's terminator compares `launched_at` against the age the process table
    // reports for the pid, and refuses to signal a pid that is younger than its handle says
    // — that is how a recycled number is told from the launch's own process. The workspace's
    // `created_at` is stamped before the arm, before the entry cache and before the plan, and
    // a participant whose daemon took a few seconds over any of those would hand the daemon a
    // handle it can never act on: the harness would survive its own deadline.
    let launched_at = crate::launch::launched_at_now();
    // D22: argv and pgid, immediately, so the daemon can identify and terminate this tree.
    // The program travels on the `Spawned` rather than being looked up again here — a shim
    // rewritten between these two lines would otherwise be recorded as what ran.
    let mut handed = match handle(plan, &spawned, &launched_at) {
        Ok(handed) => handed,
        // No handle, so no group: the pid is the whole of what is known about the child.
        Err(e) => {
            let error = kill(spawned.child, None, e);
            return Err(StartFailure {
                error,
                // The pre-spawn latch is the only durable record that a group-less native
                // may have escaped before exact process identity could be captured.
                abandon: unresolved_guard.is_none(),
            });
        }
    };
    // The seeding rides on the handle, because `register_process` is the one call a launch
    // makes exactly once. `handle` leaves the field `None`; this is where it
    // is filled in, by the half that did the seeding.
    handed.home_seed = carried;
    let registration = ProcessRegistration {
        generation: info.process_generation,
        handle: handed.clone(),
    };
    let pre_post_cleanup = unresolved_guard.as_ref().map(|_| EntryRecord {
        pending_process_cleanup: Some(registration.clone()),
        pending_process_cleanup_origin: Some(ProcessCleanupOrigin::Initial),
        pending_unresolved_native_cleanup: unresolved_native_cleanup(
            &registration.handle,
            ProcessCleanupOrigin::Initial,
        ),
        ..record_of(c, info, None, opts.dev)
    });
    if let Some(cleaning) = &pre_post_cleanup
        && let Err(error) = write_entry(cfg, cleaning)
    {
        return Err(StartFailure {
            error: kill_registered(spawned.child, &handed, error),
            // The pre-spawn latch is still durable. It must survive rather than be erased
            // by abandonment when exact native containment could not be recorded.
            abandon: false,
        });
    }
    // The last moment a Ctrl-C is this command's to act on, and the only one where there is
    // a child to take with it. Past `register_process` the daemon owns the process and will
    // terminate it at the deadline; before it, nothing will ever measure this child and the
    // entry it belongs to is about to be abandoned.
    if crate::arming::interrupted() {
        let unresolved = pre_post_cleanup
            .as_ref()
            .is_some_and(|record| record.pending_unresolved_native_cleanup.is_some());
        return Err(StartFailure {
            error: kill_registered(spawned.child, &handed, interrupted_refusal()),
            abandon: !unresolved,
        });
    }
    // Persist before the first byte of `/process`: a CLI crash or a local cache-write
    // failure after an ambiguous response must not lose the only exact body that can replay
    // the daemon's idempotent handoff.
    let pending = EntryRecord {
        pending_process_registration: Some(registration.clone()),
        pending_process_registration_origin: Some(ProcessCleanupOrigin::Initial),
        pending_unresolved_native_cleanup: pre_post_cleanup
            .as_ref()
            .and_then(|record| record.pending_unresolved_native_cleanup.clone()),
        ..record_of(c, info, Some(handed.pid), opts.dev)
    };
    if let Err(error) = write_entry(cfg, &pending) {
        let unresolved = pre_post_cleanup
            .as_ref()
            .is_some_and(|record| record.pending_unresolved_native_cleanup.is_some());
        return Err(StartFailure {
            error: kill_registered(spawned.child, &handed, error),
            abandon: !unresolved,
        });
    }
    if let Err(failure) = retry_busy_process_classified(|| {
        DaemonClient::register_process_with_admission(client, &info.entry_id, &registration)
    }) {
        // Any response the process route could not interpret may have followed a durable
        // admission of this exact immutable body. Keep both child and replay body in that
        // case. A later Busy or Conflict cannot prove the earlier attempt did not admit it.
        if failure.may_have_been_admitted {
            return Err(StartFailure {
                error: if failure.interrupted {
                    CliError::Refused(format!(
                        "`goalcup start` was interrupted while the process handoff may already have been admitted: {}. The exact handoff remains pending; run `goalcup resume {}` to recover it.",
                        failure.error, info.entry_id
                    ))
                } else {
                    CliError::Control(failure.error)
                },
                abandon: false,
            });
        }
        // A verified refusal retires replay authority before any process side effect. If
        // this invocation dies after the write, its successor sees cleanup, never reposts
        // the rejected body, and never signals the cached numeric pid.
        let abandon_after_cleanup = matches!(failure.error, ControlError::Malformed(_));
        let refusal = if failure.interrupted {
            CliError::Refused(format!(
                "`goalcup start` was interrupted during a pre-admission process wait: {}. The refused harness has been stopped; run start again.",
                failure.error
            ))
        } else {
            CliError::Control(failure.error)
        };
        let cleaning = EntryRecord {
            pending_process_registration: None,
            pending_process_registration_origin: None,
            pending_process_cleanup: Some(registration.clone()),
            pending_process_cleanup_origin: Some(ProcessCleanupOrigin::Initial),
            pending_unresolved_native_cleanup: unresolved_native_cleanup(
                &registration.handle,
                ProcessCleanupOrigin::Initial,
            ),
            last_pid: None,
            ..pending
        };
        if let Err(cache_error) = write_entry(cfg, &cleaning) {
            return Err(StartFailure {
                // The pending row remains the fail-closed recovery latch, but this
                // invocation still owns a real Child and must not leak it merely because
                // the stronger cleanup-only transition could not be persisted.
                error: kill_registered(spawned.child, &registration.handle, cache_error),
                abandon: false,
            });
        }
        let error = kill_registered(spawned.child, &registration.handle, refusal);
        let cleanup_complete = cleaning.pending_unresolved_native_cleanup.is_none()
            && process_handle_instance_status(&registration.handle)
                == ProcessInstanceStatus::GoneOrReused;
        if cleanup_complete {
            let cleared = EntryRecord {
                pending_process_cleanup: None,
                pending_process_cleanup_origin: None,
                pending_unresolved_native_cleanup: None,
                ..cleaning
            };
            if let Err(cache_error) = write_entry(cfg, &cleared) {
                return Err(StartFailure {
                    error: cache_error,
                    abandon: false,
                });
            }
        }
        return Err(StartFailure {
            error,
            abandon: cleanup_complete && abandon_after_cleanup,
        });
    }

    // The abandon window is closed here and nowhere else: the daemon owns this process now,
    // so every failure below is reported with the entry left standing, because it is a real
    // run that is being recorded while this command reports.
    let closed = |error: CliError| StartFailure {
        error,
        abandon: false,
    };
    // The record already holds everything but the pid, from before the arm; this is the one
    // field a launch adds. A rewrite that fails is worth saying and is not worth ending a
    // run over — `goalcup resume` writes the same field again after its own relaunch.
    if let Err(e) = write_entry(cfg, &record_of(c, info, Some(handed.pid), opts.dev)) {
        say(
            out,
            &format!("  (this machine's own note of the harness's pid could not be updated: {e})"),
        )
        .map_err(closed)?;
    }
    say(out, "Recording. Leave this running.").map_err(closed)?;

    // The gate, in the mode the TUI owns the terminal in: read once, beside the harness, and
    // not printed until the harness has let go of the screen.
    let report = crate::arming::watch_beside(client, c, &info.entry_id, &info.run_public_key);
    let status = spawned.child.wait().map_err(|source| {
        closed(CliError::Io {
            path: handed.program.clone(),
            source,
        })
    })?;
    for line in &report.lines {
        say(out, line).map_err(closed)?;
    }
    // A Ctrl-C past `register_process` is the harness's and not this command's: a TUI shares
    // this terminal, the run is stamped and was recorded while it ran, and the only thing
    // left to do about the flag is to say why nothing was unmade. It changes this line and
    // nothing else — not the number this command leaves under, not the entry, not the record.
    if crate::arming::interrupted() {
        say(
            out,
            "A Ctrl-C reached this terminal after the harness was handed over: the entry \
             stands, the tracker went on recording it, and nothing was abandoned.",
        )
        .map_err(closed)?;
    }
    Ok(StartOutcome {
        entry_id: info.entry_id.clone(),
        gate: verdict(report, &info.entry_id, Window::Closed)?,
        exit_status: Some(status),
    })
}

/// Whether the abandon window is still open when the gate is judged.
///
/// [`Window::Closed`] is everything past `register_process`: the daemon owns the process and
/// the run is stamped. It reaches only a flag this command no longer acts on, because a gate
/// the daemon settled unmakes the entry either way: a run the tracker will never arm is a run
/// there is no entry for.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg(feature = "v1-compat")]
enum Window {
    Open,
    Closed,
}

/// What a [`GateReport`] means for the run: an outcome to report, or a refusal to leave under.
///
/// **A settled gate leaves under rung 4 and never under 2**: a
/// participant whose gate refused must not read the same number as a participant whose entry
/// failed verification, and the sentence carries the daemon's own words rather than a
/// paraphrase of them.
#[cfg(feature = "v1-compat")]
fn verdict(
    report: GateReport,
    entry_id: &str,
    window: Window,
) -> Result<GateOutcome, StartFailure> {
    match report.outcome {
        GateOutcome::Refused(detail) => Err(StartFailure::from(CliError::Gate(format!(
            "the arming gate refused entry {entry_id} and will not arm: {detail}"
        )))),
        // Reachable inside the window only — `arming::Watch::Beside` never reads the flag —
        // and it must not unmake a run if that ever stops being true.
        GateOutcome::Interrupted => Err(StartFailure {
            error: interrupted_refusal(),
            abandon: window == Window::Open,
        }),
        outcome => Ok(outcome),
    }
}

/// `Ok(())` unless this process has been asked to stop, in which case the entry is unmade.
///
/// [`StartFailure::from`] sets `abandon`, so this is the whole of what a Ctrl-C does: the
/// entry goes, the local record goes with it where the daemon confirmed the abandon, and the
/// child — where one has been spawned — is killed by the caller that owns it.
///
/// **Inside the abandon window only.** Its one caller is `--print-launch`, which spawns
/// nothing and registers nothing; the gate reads the flag for the attach case
/// ([`GateOutcome::Interrupted`]), and past `register_process` nothing reads it to act on.
#[cfg(feature = "v1-compat")]
fn interrupt_check() -> Result<(), StartFailure> {
    match crate::arming::interrupted() {
        true => Err(StartFailure::from(interrupted_refusal())),
        false => Ok(()),
    }
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

/// Stop a child the daemon was never told about, and report the failure that made it an
/// orphan. See [`launch`] for why this is not the terminator this crate must not grow.
///
/// **The group first, where the launch owns one.** A `Child::kill` signals one pid, and the
/// harnesses this spawns are wrappers: `npm` execs a native binary, a server forks a worker.
/// Killing the leader alone leaves those running against an entry the next line abandons.
/// `pgid` is `Some` exactly when the child was put at the head of a group of its own, so
/// the group is the honest target whenever there is one.
///
/// SIGKILL and no grace period, deliberately: a grace period is the terminator's, at the
/// deadline, and this is a process that has existed for a few milliseconds, has been sent no
/// prompt, and has nothing to flush.
///
/// **`pub(crate)`, and [`crate::commands::resume`] is the second caller.** A resume that
/// spawned the harness and could not hand it over is in the same position this is written
/// for — a child nothing will measure and nothing will seal — and there is one copy of the
/// group walk because `tests/process_handoff.rs` exempts this file alone.
#[cfg(feature = "v1-compat")]
pub(crate) fn kill(mut child: Child, pgid: Option<u32>, e: CliError) -> CliError {
    kill_the_group(pgid);
    let _ = child.kill();
    let _ = child.wait();
    e
}

/// Stop a definitively unadmitted child while this invocation still owns its `Child`.
/// For a TUI there is no private process group, so a launch-time native hint is signalled
/// first only after its creation identity and current descendant binding are revalidated.
/// Cached cleanup never calls this function.
#[cfg(feature = "v1-compat")]
pub(crate) fn kill_registered(child: Child, handle: &ProcessHandle, error: CliError) -> CliError {
    if handle.pgid.is_none() {
        kill_retained_native(handle);
    }
    kill(child, handle.pgid, error)
}

#[cfg(feature = "v1-compat")]
pub(crate) fn unresolved_native_cleanup(
    handle: &ProcessHandle,
    origin: ProcessCleanupOrigin,
) -> Option<UnresolvedNativeCleanup> {
    let native_binary = handle.native_binary.as_ref()?;
    if handle.pgid.is_some() || handle.native_pid.is_some() {
        return None;
    }
    let root_is_native = snapshot().ok().is_some_and(|table| {
        table
            .iter()
            .any(|row| row.pid == handle.pid && row.comm == *native_binary)
    });
    (!root_is_native).then(|| UnresolvedNativeCleanup {
        origin,
        native_binary: native_binary.clone(),
    })
}

#[cfg(feature = "v1-compat")]
pub(crate) fn unresolved_native_guard(
    plan: &LaunchPlan,
    origin: ProcessCleanupOrigin,
) -> Option<UnresolvedNativeCleanup> {
    if plan.mode != LaunchMode::Tui || plan.kill.strategy != KillStrategy::NativePid {
        return None;
    }
    Some(UnresolvedNativeCleanup {
        origin,
        native_binary: plan.kill.native_binary.clone()?,
    })
}

#[cfg(all(unix, feature = "v1-compat"))]
fn kill_retained_native(handle: &ProcessHandle) {
    let Some(native_pid) = handle.native_pid else {
        return;
    };
    if process_instance_status(native_pid, handle.native_process_start.as_deref())
        != ProcessInstanceStatus::Same
    {
        return;
    }
    let Ok(table) = snapshot() else {
        return;
    };
    if !native_pid_is_bound_to_handle(handle, &table) {
        return;
    }
    let Ok(raw) = i32::try_from(native_pid) else {
        return;
    };
    // SAFETY: the numeric target was sampled as the exact creation identity and a current
    // descendant of the retained Child immediately above. This path is never used from a
    // cached marker after restart.
    let _ = unsafe { libc::kill(raw, libc::SIGKILL) };
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while is_alive(native_pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(all(not(unix), feature = "v1-compat"))]
fn kill_retained_native(_handle: &ProcessHandle) {}

/// SIGKILL to the whole process group, where the launch was given one.
#[cfg(all(unix, feature = "v1-compat"))]
fn kill_the_group(pgid: Option<u32>) {
    let Some(pgid) = pgid.and_then(|g| i32::try_from(g).ok()) else {
        return;
    };
    // SAFETY: `killpg` takes two integers, dereferences no memory this process owns and
    // writes none. A group that is already gone answers -1/ESRCH, which is the ordinary
    // outcome for a child that exited by itself and is why the result is dropped.
    let _ = unsafe { libc::killpg(pgid, libc::SIGKILL) };
}

/// A platform with no process groups has nothing to signal; the pid is killed either way.
#[cfg(all(not(unix), feature = "v1-compat"))]
fn kill_the_group(_pgid: Option<u32>) {}

/// Unmake the entry, and the local record with it where the daemon agreed.
///
/// **The record goes only after a confirmed abandon**. It is the
/// one file this command wrote, and every later process on this machine reads the pinned run
/// key out of it; removing it over an abandon the daemon refused would leave the participant
/// holding a live entry that no command here can name any more. So a refused abandon keeps
/// it, and says on the screen that it did.
///
/// **The cause is not touched.** It is what the participant is told about, on stderr, with
/// the rung it already carries: a `conflict` from the arm is a refusal they can act on and a
/// daemon that answered something unreadable is not, and a cleanup wrapping both in one new
/// error would have flattened that difference — the exit ladder lives in `lib.rs` and is
/// decided by the failure, not by what this function did about it.
///
/// What the cleanup itself did goes to `out`, in the same place the rest of this command's
/// account of itself goes. A cleanup that failed says so line by line, because "the arm was
/// refused" and "and the workspace is still on your disk" are two different things for the
/// participant to do next.
#[cfg(feature = "v1-compat")]
fn abandon(
    client: &(dyn DaemonClient + Sync),
    cfg: &Path,
    entry_id: &str,
    cause: &CliError,
    out: &mut dyn std::io::Write,
) {
    let reason = format!("`goalcup start` failed before the run began: {cause}");
    abandon_because(
        client,
        cfg,
        entry_id,
        &reason,
        &abandoned_sentence(entry_id),
        out,
    );
}

/// What a confirmed abandon of a legacy entry leaves, in the participant's words.
///
/// **The workspace is not one of them.** What goes is the entry: the daemon drops its registry
/// row and removes its artifact directory and its signed status cache, and this command
/// removes the local note of it. The workspace stays exactly where it is, on purpose —
/// `control::handlers::abandon_entry` says why in terms: it is the participant's directory,
/// and a tracker that deleted a directory somebody had been working in would be the worst
/// possible reading of "abandon".
#[cfg(feature = "v1-compat")]
fn abandoned_sentence(entry_id: &str) -> String {
    format!(
        "Entry {entry_id} is abandoned: its artifact directory is gone, and so is this \
         machine's note of it — so this can be run again. Its workspace is left where it is; \
         the tracker made that directory but does not delete a directory you may have worked \
         in."
    )
}

/// The cleanup every abandon takes: ask the daemon, and remove the local record only where it
/// agreed. `sentence` is what the participant is told when both went through. Answers
/// whether the daemon abandoned the entry.
///
/// `--print-launch` succeeds and still leaves nothing behind, so the reason the
/// daemon is told is a sentence rather than a [`CliError`]. Everything below is the same
/// cleanup either way, and it is one function so that a `--print-launch` cannot quietly stop
/// removing the local record while a failure keeps doing it.
fn abandon_because(
    client: &(dyn DaemonClient + Sync),
    cfg: &Path,
    entry_id: &str,
    reason: &str,
    sentence: &str,
    out: &mut dyn std::io::Write,
) -> bool {
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

/// The record this start pins, with `last_pid` set once there is a process to name.
///
/// `dev` is written from `StartOptions::dev` because it is the last moment anything knows
/// it: the flag belongs to a command that is about to exit, and `goalcup seal` — a fresh
/// process, possibly days later — has to print a `goalcup-verify` line that actually works
/// on this trace. See [`crate::entries::EntryRecord::dev`].
#[cfg(feature = "v1-compat")]
fn record_of(c: &Connector, info: &WorkspaceInfo, last_pid: Option<u32>, dev: bool) -> EntryRecord {
    EntryRecord {
        entry_id: info.entry_id.clone(),
        process_generation: info.process_generation,
        connector_id: c.manifest.id.clone(),
        run_public_key: info.run_public_key.clone(),
        workspace: info.workspace.clone(),
        artifact_root: info.artifact_root.clone(),
        harness_home: info.harness_home.clone(),
        session_id: info.session_id.clone(),
        last_pid,
        workspace_relative: info.workspace_relative.clone(),
        status_url: info.status_url.clone(),
        created_at: info.created_at.clone(),
        dev,
        ..Default::default()
    }
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
        kind: metadata.mode() & libc::S_IFMT as u32,
    }
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

#[cfg(not(unix))]
fn observe_shared_capture_home(
    alias: &Path,
    _owner: u32,
) -> Result<SharedHomeObservation, CliError> {
    Err(shared_home_refusal(
        alias,
        "secure shared-home identity checks are unavailable on this platform",
    ))
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
    /// Hermes' two tees, which also scan and read the session marker.
    marked: Option<CaptureStderr>,
    /// Only a stderr-line identity is sent with the finish. A credential may require the same
    /// two tees while the identity itself was preassigned.
    late_scope: bool,
}

impl Tees {
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
    if let Some(rule) = opts.capture_claude_allowed_tool.as_deref() {
        let why = if opts.dev {
            Some("a development run (--dev) cannot carry one")
        } else {
            crate::launch::allowance_refusal(c, &profile.args, rule)
        };
        if let Some(why) = why {
            return Err(CliError::Refused(format!(
                "the Claude capture tool allowance cannot be used: {why}. Nothing was created."
            )));
        }
    }
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
    let executable = capture_executable(c, profile, env)?;
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
    let shared_home = if per_run {
        None
    } else {
        let config_home = cfg.join(CAPTURE_HOMES_DIR).join(&c.loaded.manifest.id);
        private_dir(config_home.parent().expect("a capture-home parent"), true)?;
        match std::fs::symlink_metadata(&config_home) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                private_dir(&config_home, false)?;
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
        workspace_relative: Vec::new(),
        status_url: String::new(),
        // No daemon stamp comes back for a capture entry, so this machine's clock orders it.
        created_at: crate::launch::launched_at_now(),
        dev: opts.dev,
        ..Default::default()
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
                allowed_tool_rule: opts.capture_claude_allowed_tool.as_deref(),
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
    if let Err(failure) = retry_busy_process_classified(|| {
        DaemonClient::register_process_with_admission(
            forked.client,
            &forked.info.entry_id,
            &registration,
        )
    }) {
        // Definite or ambiguous, the child is never released: it ends at its gate without
        // having run a byte of the harness, and the seal reads a registration that names a
        // process that never exec'd.
        gated.abort();
        return Err(forked.unlaunched(CliError::Control(failure.error), out));
    }
    pause_at("capture-gate-registered");
    if let Err(e) = crate::launch::recheck_pinned_executable(&prep.executable) {
        gated.abort();
        return Err(forked.unlaunched(e, out));
    }
    let mut spawned = match gated.release() {
        Ok(spawned) => spawned,
        // No harness ran, so no `Launch` is written, and the entry is sealed.
        Err(e) => return Err(forked.unlaunched(e, out)),
    };
    // Observed on this process's monotonic clock, the one the proxy's first producer stamps
    // its frames with under its own ID.
    let mut spawned_at = crate::proxy::metadata::capture_time();
    spawned_at.clock_id = forked.info.producer_id.clone();
    let stdout = if let StdoutUse::IdentityJson(rule) = prep.streams.stdout.clone() {
        match tee_scoped_capture_with_rule(&mut spawned, rule) {
            Ok(stdout) => Some(stdout),
            // The tee ended the child it could not forward; it is registered, so the seal
            // answers for it.
            Err(e) => return Err(forked.unlaunched(e, out)),
        }
    } else {
        None
    };
    // Hermes: both streams forwarded verbatim and scanned for key fragments, stderr read for
    // the session marker (the late scope of the finish).
    let marker_rule = match prep.streams.stderr.clone() {
        StderrUse::IdentityLine(rule) => Some(rule),
        StderrUse::Inherit | StderrUse::Forward => None,
    };
    let marked = if prep.streams.scan_for_key {
        match tee_marked_capture_with_rule(&mut spawned, prep.secret.as_ref(), marker_rule) {
            Ok(tee) => Some(tee),
            Err(e) => return Err(forked.unlaunched(e, out)),
        }
    } else if let Some(rule) = marker_rule {
        match tee_stderr_capture_with_rule(&mut spawned, rule) {
            Ok(tee) => Some(tee),
            Err(e) => return Err(forked.unlaunched(e, out)),
        }
    } else {
        None
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
        LaunchScope::FromStdout => tees.stdout.as_ref().and_then(|receiver| {
            crate::launch::wait_native_scope(&mut child, &receiver.scope, scope_deadline)
        }),
        // Hermes' scope is read from its stderr after the proxy stops, and sent with the seal.
        LaunchScope::AtFinish | LaunchScope::None => None,
    };
    let launch = CaptureLaunchRequest {
        goal_stream,
        spawned_at,
        harness_version: cx.profile.executable.version.clone(),
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
    Ok(Exited {
        forked,
        tees,
        status,
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

/// The connector's program as the `PATH` in `env` finds it, followed to the file an
/// updater's link names, and accepted only if its bytes are the profile's pin. That file is
/// the one the launch runs.
fn capture_executable(
    c: &HarnessConnector,
    profile: &CaptureSpec,
    env: &BTreeMap<String, String>,
) -> Result<crate::launch::PinnedExecutable, CliError> {
    let program = &profile.executable.program;
    let Some(found) = crate::preflight::resolve_on_path(program, env) else {
        return Err(CliError::Refused(format!(
            "`{program}` is not on PATH, so `{}`'s capture executable cannot be found. Nothing \
             was created.",
            c.loaded.manifest.id
        )));
    };
    let file = std::fs::canonicalize(&found).map_err(|source| CliError::Io {
        path: found.display().to_string(),
        source,
    })?;
    crate::launch::pin_harness_executable(profile, &file)
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

#[cfg(not(unix))]
fn symlink(_target: &Path, link: &Path) -> Result<(), CliError> {
    Err(CliError::Refused(format!(
        "{} cannot be made on this platform",
        link.display()
    )))
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

#[cfg(not(unix))]
fn raise_open_file_limit(_out: &mut dyn std::io::Write) -> Result<(), CliError> {
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
