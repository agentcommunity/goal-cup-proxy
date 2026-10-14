//! Support for `m2_cli_lifecycle.rs`: a real in-process daemon over a development capture
//! connector whose harness is `fixtures/m2-fake-harness.sh`, a client that can lose or refuse
//! one call, a loopback relay in front of the daemon's ingest endpoint, and a TLS upstream
//! the proxy can be told to trust.
//!
//! Every wait is bounded; every child the fake harness becomes is ended by its own owner.
//!
//! Two test binaries pull this file in (`m2_cli_lifecycle` and `m2_hermes_lifecycle`) and neither
//! uses all of it.

#![allow(dead_code)]

use super::m2_common::{TestPki, read_at_least, read_head};
use goalcup_cli::CliError;
use goalcup_cli::commands::start::{StartOptions, StartOutcome};
use goalcup_cli::daemon::HttpDaemonClient;
use goalcup_cli::proxy::tls::UpstreamRoots;
use goalcup_protocol::capture::CaptureRecord;
use goalcup_protocol::control::{
    self, CaptureFinishReport, CaptureGoalRequest, CaptureLaunchRequest, CaptureSealError,
    CaptureSealRequest, CaptureWorkspaceInfo, CaptureWorkspaceRequest, ControlError, DaemonClient,
};
use goalcup_protocol::process::ProcessRegistration;
use goalcup_verifier::capture_analysis::reader::CaptureIntegrityReport;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// The development connector every test launches.
pub const FAKE_ID: &str = "m2-fake";

/// The Hermes-shaped development connector: the shipped `hermes-cli` manifest and seed files
/// under this id, with its program replaced by `fixtures/m2-fake-hermes.sh`.
pub const HERMES_FAKE_ID: &str = "m2-fake-hermes";

/// The exact path the fake connector records on `localhost`, the host the proxy intercepts.
pub const EVIDENCE_PATH: &str = "/v1/responses";

/// One test's world: a daemon, its configuration directory (also the CLI's), a development
/// connector, and the directory the fake harness reports into.
pub struct Rig {
    pub root: PathBuf,
    pub cfg: PathBuf,
    /// The schema-1 connector root consumed by both the daemon and CLI.
    pub connectors: PathBuf,
    /// Alias of [`Rig::connectors`] retained for the capture-CLI helper call sites.
    pub capture_connectors: PathBuf,
    /// Where the fake harness writes its pid, argv, environment and markers.
    pub out: PathBuf,
    pub runtime: tokio::runtime::Runtime,
    pub daemon: Option<goalcup_daemon::runtime::Daemon>,
    pub config: goalcup_daemon::config::DaemonConfig,
    /// A listener the daemon is told is its upload server. Nothing may connect to it.
    pub upload_trap: std::net::TcpListener,
    /// Added to the environment `goalcup start` is given (the Hermes provider key, in a test).
    pub extra_env: Mutex<std::collections::BTreeMap<String, String>>,
    _tmp: tempfile::TempDir,
}

impl Rig {
    pub fn new() -> Rig {
        Rig::with_capture(true)
    }

    /// A capture rig whose connector is the Hermes-shaped `m2-fake-hermes` (see
    /// [`HERMES_FAKE_ID`]): a per-run home, a goal flag, a child-only credential and a harness
    /// that prints its session identity on stderr.
    pub fn hermes() -> Rig {
        Rig::build(true, Some(2), None, true, true)
    }

    /// [`Rig::hermes`] under the old family-oriented helper signature. Schema 1 has no family:
    /// the harness manifest proves that every credential-bearing connector is scanned.
    pub fn hermes_in_family(family: &str) -> Rig {
        let _ = family;
        Rig::build(true, Some(2), None, true, true)
    }

    /// A rig whose connector has no `[capture]` section when `capture` is false.
    pub fn with_capture(capture: bool) -> Rig {
        Rig::build(capture, None, None, false, true)
    }

    /// A capture rig whose connector gives a harness `grace_seconds` between SIGTERM and
    /// SIGKILL (the shipped connector's 10 s is too long for a test that ends a stubborn one).
    pub fn with_grace(grace_seconds: u64) -> Rig {
        Rig::build(true, Some(grace_seconds), None, false, true)
    }

    /// A capture rig whose connector says `family = "codex"`, so the launcher tees the child's
    /// stdout and waits for its scope identity, as it does for the shipped Codex connector.
    pub fn codex_family() -> Rig {
        Rig::build(true, Some(2), Some("codex"), false, true)
    }

    /// The capture rig with **no daemon running**, for the binary to start one: its roots are
    /// the ones the default `<cfg>/daemon.toml` names (`<cfg>-artifacts`, `<cfg>-runs`), which
    /// `goalcup start` writes on first use, so [`Rig::trace`] finds what that daemon wrote.
    pub fn run_scoped() -> Rig {
        Rig::build(true, Some(2), None, false, false)
    }

    fn build(
        capture: bool,
        grace: Option<u64>,
        family: Option<&str>,
        hermes: bool,
        start_daemon: bool,
    ) -> Rig {
        // These tests cut a finish that cannot complete; the shipped budget is three minutes
        // (D56), a test's is ten seconds.
        goalcup_cli::set_finish_budget(std::time::Duration::from_secs(10));
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let out = root.join("harness-out");
        std::fs::create_dir(&out).unwrap();
        let exe = root.join("bin").join("m2-fake-harness");
        std::fs::create_dir(root.join("bin")).unwrap();
        let fixture = if hermes {
            "m2-fake-hermes.sh"
        } else {
            "m2-fake-harness.sh"
        };
        std::fs::copy(super::support::fixture(fixture), &exe).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let connectors = root.join("connectors");
        let capture_connectors = connectors.clone();
        // `with_capture(false)` still represents a real, readable connector root. The CLI's
        // refusal is that no schema-1 harness declares the requested id, not an I/O failure
        // because the test forgot to make the root it passed in.
        std::fs::create_dir_all(&capture_connectors).unwrap();
        if hermes {
            write_hermes_connector(&connectors, &capture_connectors, &exe, &out);
        } else {
            write_connector(
                &connectors,
                &capture_connectors,
                &exe,
                &out,
                capture,
                grace,
                family,
            );
        }
        let upload_trap = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        upload_trap.set_nonblocking(true).unwrap();
        let (artifact_root, workspace_parent) = if start_daemon {
            (root.join("artifacts"), root.join("workspaces"))
        } else {
            (root.join("cfg-artifacts"), root.join("cfg-runs"))
        };
        let config = goalcup_daemon::config::DaemonConfig {
            artifact_root,
            connectors_dir: connectors.clone(),
            config_dir: root.join("cfg"),
            workspace_parent,
            control_addr: "127.0.0.1:0".parse().unwrap(),
            upload_url: format!("http://{}", upload_trap.local_addr().unwrap()),
            server_public_key: super::support::TEST_SERVER_PUBLIC_KEY_HEX.to_string(),
            receipt_binding: goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
            poll_interval_ms: 250,
            // Short, so a run of a second meets several upload ticks that must not dial.
            upload_interval_ms: 100,
            upload_chunk_max: 256,
            status_interval_ms: 1_000,
            stall_after_seconds: 900.0,
            silence_window_seconds: 60.0,
            ack_lag_seconds: 300.0,
            arming_timeout_seconds: None,
            heartbeat_seconds: 60,
            clock_skew_seconds: 120.0,
            disk_reserve_bytes: 0,
            event: None,
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let daemon = start_daemon.then(|| {
            runtime
                .block_on(goalcup_daemon::runtime::Daemon::start(config.clone()))
                .unwrap()
        });
        Rig {
            cfg: config.config_dir.clone(),
            root,
            connectors,
            capture_connectors,
            out,
            runtime,
            daemon,
            config,
            upload_trap,
            extra_env: Mutex::default(),
            _tmp: tmp,
        }
    }

    pub fn client(&self) -> HttpDaemonClient {
        HttpDaemonClient::from_config_dir(&self.cfg).unwrap()
    }

    /// A goal file in its own directory, which is where the run directory is created.
    pub fn goal(&self, text: &str) -> PathBuf {
        let dir = self.root.join("runs");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("goal.txt");
        std::fs::write(&path, text).unwrap();
        path
    }

    /// `start_with` options for a v5 run of the fake connector, which no build pins, so
    /// through the one door (`--dev`).
    pub fn options(&self, goal: &Path, reserve: u64) -> StartOptions {
        let mut opts = super::support::start_options(FAKE_ID);
        opts.goal_file = Some(goal.to_path_buf());
        opts.disk = super::support::disk_of(reserve);
        opts.dev = true;
        opts
    }

    /// [`Rig::options`] for the Hermes-shaped connector.
    pub fn hermes_options(&self, goal: &Path, reserve: u64) -> StartOptions {
        let mut opts = self.options(goal, reserve);
        opts.harness = HERMES_FAKE_ID.to_string();
        opts
    }

    /// Give `goalcup start` one more environment variable.
    pub fn with_env(&self, name: &str, value: &str) {
        self.extra_env
            .lock()
            .unwrap()
            .insert(name.to_string(), value.to_string());
    }

    /// The directories `goalcup start` made beside the goal file.
    pub fn run_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(self.root.join("runs"))
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        dirs.retain(|p| p.is_dir());
        dirs.sort();
        dirs
    }

    /// Write `name` into the fake harness's directory: `stop` ends it, `steps.sh` runs first.
    pub fn tell(&self, name: &str, text: &str) {
        std::fs::write(self.out.join(name), text).unwrap();
    }

    pub fn harness_file(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.out.join(name)).ok()
    }

    pub fn harness_started(&self) -> bool {
        self.out.join("started").exists()
    }

    pub fn harness_pid(&self) -> Option<u32> {
        self.harness_file("pid")?.trim().parse().ok()
    }

    /// Wait for the fake harness to write `name`.
    pub fn wait_for(&self, name: &str) {
        super::support::wait_for_file(&self.out.join(name));
    }

    /// How many connections the upload trap accepted.
    pub fn upload_dials(&self) -> usize {
        let mut n = 0;
        while self.upload_trap.accept().is_ok() {
            n += 1;
        }
        n
    }

    /// The trace the daemon exports for `entry_id`.
    pub fn trace(&self, entry_id: &str) -> PathBuf {
        self.config
            .artifact_root
            .join("artifacts")
            .join(entry_id)
            .join("trace")
    }

    /// The v5 reader's report and every record it validated.
    pub fn read_trace(&self, entry_id: &str) -> (CaptureIntegrityReport, Vec<CaptureRecord>) {
        let mut records = Vec::new();
        let report = goalcup_verifier::capture_analysis::reader::scan_capture_trace(
            &self.trace(entry_id),
            &goalcup_verifier::VerifyOptions::default(),
            &mut |record| {
                records.push(record.clone());
                Ok(())
            },
        )
        .unwrap();
        (report, records)
    }

    /// This process's `PATH` with the fake harness's directory in front.
    pub fn path(&self) -> String {
        format!(
            "{}:{}",
            self.root.join("bin").display(),
            super::support::clean_env()["PATH"]
        )
    }

    /// [`Rig::start`] writing to `out`, which a test can make fail. Hands back the result alone;
    /// the writer is the test's own record of what it was given.
    pub fn start_writing<'scope, 'env, W: std::io::Write + Send + 'scope>(
        &'env self,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        client: &'env (dyn DaemonClient + Sync),
        opts: StartOptions,
        mut out: W,
    ) -> std::thread::ScopedJoinHandle<'scope, Result<StartOutcome, CliError>> {
        let mut env = super::support::clean_env();
        env.insert("PATH".to_string(), self.path());
        env.extend(self.extra_env.lock().unwrap().clone());
        scope.spawn(move || {
            goalcup_cli::commands::start::start_with(
                client,
                &self.capture_connectors,
                &self.cfg,
                &env,
                &opts,
                &mut out,
            )
        })
    }

    /// Run `goalcup start` on its own thread, as the binary would, and hand back its result
    /// and everything it printed.
    pub fn start<'scope, 'env>(
        &'env self,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        client: &'env (dyn DaemonClient + Sync),
        opts: StartOptions,
        roots: Option<UpstreamRoots>,
    ) -> std::thread::ScopedJoinHandle<'scope, (Result<StartOutcome, CliError>, String)> {
        let mut env = super::support::clean_env();
        env.insert("PATH".to_string(), self.path());
        env.extend(self.extra_env.lock().unwrap().clone());
        scope.spawn(move || {
            let mut out = Vec::new();
            let result = match roots {
                Some(roots) => goalcup_cli::commands::start::start_with_upstream_roots(
                    client,
                    &self.capture_connectors,
                    &self.cfg,
                    &env,
                    &opts,
                    roots,
                    &mut out,
                ),
                None => goalcup_cli::commands::start::start_with(
                    client,
                    &self.capture_connectors,
                    &self.cfg,
                    &env,
                    &opts,
                    &mut out,
                ),
            };
            (result, String::from_utf8_lossy(&out).into_owned())
        })
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // A harness a failed test left behind waits on its one `sleep`; ending that ends it.
        if let Some(sleeper) = self.harness_file("sleeper")
            && let Ok(pid) = sleeper.trim().parse::<u32>()
            && goalcup_protocol::process::snapshot().is_ok_and(|table| {
                table
                    .iter()
                    .any(|r| r.pid == pid && r.comm.ends_with("sleep"))
            })
        {
            // SAFETY: two integers; the pid was just seen running `sleep`.
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
        if let Some(daemon) = self.daemon.take() {
            self.runtime.block_on(daemon.stop());
        }
    }
}

/// `connectors/m2-fake/`: a schema-1 development connector whose program is found on the `PATH`
/// [`Rig::start`] hands over. A new id is a directory this build pins nothing for, which the
/// daemon admits as a development connector.
fn write_connector(
    connectors: &Path,
    capture_connectors: &Path,
    exe: &Path,
    out: &Path,
    capture: bool,
    grace: Option<u64>,
    family: Option<&str>,
) {
    let out_text = out.display().to_string();
    assert!(
        !out_text.contains('\''),
        "the test path must be shell-quotable"
    );
    let fake = std::fs::read_to_string(exe).unwrap();
    let (shebang, body) = fake.split_once('\n').expect("the fake has a shebang");
    std::fs::write(
        exe,
        format!("{shebang}\nM2_FAKE_OUT='{out_text}'\nexport M2_FAKE_OUT\n{body}"),
    )
    .unwrap();
    assert_eq!(connectors, capture_connectors, "the schema root is shared");

    if capture {
        let capture_dir = capture_connectors.join(FAKE_ID);
        std::fs::create_dir_all(&capture_dir).unwrap();
        let identity = if family == Some("codex") {
            r#"source = "stdout_json"
binding = "at_launch"
match_field = "type"
match_value = "thread.started"
id_field = "thread_id""#
        } else {
            "source = \"none\""
        };
        let native_pattern = if family == Some("codex") {
            "native/*-{id}.jsonl"
        } else {
            "native/*.jsonl"
        };
        let harness = format!(
            r#"schema_version = 1
id = "{FAKE_ID}"

[capture]
auth_hosts = ["auth.openai.com"]
continuation = "none: the fake harness takes no input after its goal"
args = ["run", "-C", "{{workspace}}", "--"]

[capture.executable]
program = "m2-fake-harness"

[capture.home]
run_home = "empty"
config_home = "shared"
env = "CODEX_HOME"
real_dirs = ["{{user_home}}/.codex"]

[capture.ca]
env = ["CODEX_CA_CERTIFICATE"]
mode = "extend"

[capture.identity]
{identity}

[[capture.native]]
name = "fake"
root = "{{run_home}}"
pattern = "{native_pattern}"
form = "append_only"

[capture.termination]
strategy = "process_group"
grace_seconds = {grace}

[[capture.hosts]]
host = "localhost"
evidence_paths = ["{EVIDENCE_PATH}"]
protocol_hint = "openai-responses"
"#,
            grace = grace.unwrap_or(10),
        );
        std::fs::write(capture_dir.join("harness.toml"), harness).unwrap();
    }
}

/// The shipped `hermes-cli` connector rendered into the shared schema-1 test root, with its id
/// and program replaced. The fake's report directory is injected into its private
/// script, not the manifest. A new id is unpinned and admitted only through `--dev`.
fn write_hermes_connector(connectors: &Path, capture_connectors: &Path, exe: &Path, out: &Path) {
    assert_eq!(connectors, capture_connectors, "the schema root is shared");
    let shipped = super::support::connectors_dir().join("hermes-cli");
    let out = out.display().to_string();
    assert!(!out.contains('\''), "the test path must be shell-quotable");
    let fake = std::fs::read_to_string(exe).unwrap();
    let (shebang, body) = fake.split_once('\n').expect("the fake has a shebang");
    std::fs::write(
        exe,
        format!("{shebang}\nM2_FAKE_OUT='{out}'\nexport M2_FAKE_OUT\n{body}"),
    )
    .unwrap();
    let capture_dir = capture_connectors.join(HERMES_FAKE_ID);
    std::fs::create_dir_all(capture_dir.join("files")).unwrap();
    for name in ["config.yaml", "auth.json", ".update_check"] {
        std::fs::copy(
            shipped.join("files").join(name),
            capture_dir.join("files").join(name),
        )
        .unwrap();
    }
    let mut harness = std::fs::read_to_string(shipped.join("harness.toml")).unwrap();
    for (from, to) in [
        ("id = \"hermes-cli\"", format!("id = \"{HERMES_FAKE_ID}\"")),
        (
            "program = \"hermes\"",
            "program = \"m2-fake-harness\"".to_string(),
        ),
        // `goalcup start` probes every provider host that is not loopback before it creates
        // anything; a test never reaches the network, and the fake never calls its provider.
        (
            "host = \"api.deepseek.com\"",
            "host = \"api.deepseek.localhost\"".to_string(),
        ),
    ] {
        assert!(
            harness.contains(from),
            "the shipped harness has no `{from}`"
        );
        harness = harness.replacen(from, &to, 1);
    }
    std::fs::write(capture_dir.join("harness.toml"), harness).unwrap();
}

// ---------------------------------------------------------------------------------------
// A client that can lose or refuse one call, and records the order of everything.
// ---------------------------------------------------------------------------------------

/// How the next `register_process` goes wrong.
#[derive(Clone, Copy)]
pub enum RegistrationFault {
    /// Refused before the daemon saw it: a definite refusal.
    Refused,
    /// Admitted by the daemon, then the answer is unreadable.
    AdmittedThenMalformed,
    /// Admitted by the daemon, and then the entry is sealed before the answer is returned:
    /// a seal that raced in between the registration and the release (D40). The child is
    /// still parked, so the daemon's seal cannot have signalled it.
    AdmittedThenSealed,
}

/// How the next `record_capture_launch` goes wrong.
#[derive(Clone, Copy)]
pub enum LaunchFault {
    /// Refused as a conflict with the entry still open: a capture-side fault (an invalid
    /// allowance, a wrong goal stream), never a reason to end the harness.
    Conflict,
}

/// What a [`Hooked`] client saw, and the faults armed on it.
#[derive(Default)]
pub struct Book {
    /// Every call, in order, with whether the fake harness had started when it was made.
    pub calls: Vec<(String, bool)>,
    /// The create answer as the CLI received it (the ingest secret included).
    pub created: Option<CaptureWorkspaceInfo>,
    pub creates: Vec<CaptureWorkspaceRequest>,
    /// Each status answer's per-entry admission counts.
    pub admission: Vec<(Option<u64>, Option<u64>)>,
    /// When the daemon's answer to the goal reached the CLI.
    pub goal_answered: Option<std::time::SystemTime>,
    /// What the daemon said to a second goal after it accepted the first.
    pub second_goal: Option<ControlError>,
    /// The pid of every process offered for registration.
    pub registered: Vec<u32>,
    pub seals: Vec<CaptureSealRequest>,
    /// Every capture seal is answered with this refusal, and the daemon is not asked.
    pub seal_fault: Option<fn() -> CaptureSealError>,
    /// The read timeout of every capture seal call, in order.
    pub seal_timeouts: Vec<Duration>,
    /// Answer the create call with this ingest endpoint instead of the daemon's.
    pub ingest_addr: Option<SocketAddr>,
    /// Let the daemon accept the goal, then lose its answer.
    pub lose_goal_answer: bool,
    pub registration: Option<RegistrationFault>,
    /// Hold every `register_process` this long before it acts, so a harness started before its
    /// registration has time to show it (the fake writes its pid file within milliseconds).
    pub registration_delay: Option<Duration>,
    pub launch_fault: Option<LaunchFault>,
    /// The first status read after a registration waits for this file in the harness's
    /// directory before it is answered: the launcher's look at the entry right after the release
    /// then happens after the harness has run far enough to ignore SIGTERM (the exec and the
    /// launcher's look are a few milliseconds apart, a script's first line is not).
    pub status_waits_for: Option<&'static str>,
    /// The first status read after a registration (after that wait) is lost.
    pub status_fault_once: bool,
    /// The report of the seal a fault above made while the harness was parked or just released.
    pub seal_report: Option<CaptureFinishReport>,
    /// Every status answer's `sealed_at`, in order.
    pub sealed_seen: Vec<bool>,
    /// Put a file where the proxy's CA certificate goes, before the proxy starts.
    pub occupy_ca_file: bool,
    /// Called as each capture seal begins, before the daemon is asked: what a test wants to
    /// know "at the seal".
    pub on_seal: Option<Box<dyn Fn() + Send>>,
    relay: Option<Relay>,
}

/// `inner`, with every call written in a [`Book`] and the faults the book arms.
pub struct Hooked<'a> {
    inner: &'a (dyn DaemonClient + Sync),
    out: PathBuf,
    started: PathBuf,
    relay_on: Option<&'a tokio::runtime::Runtime>,
    book: Mutex<Book>,
}

impl<'a> Hooked<'a> {
    pub fn new(inner: &'a (dyn DaemonClient + Sync), rig: &Rig) -> Hooked<'a> {
        Hooked {
            inner,
            out: rig.out.clone(),
            started: rig.out.join("started"),
            relay_on: None,
            book: Mutex::default(),
        }
    }

    /// Put a [`Relay`] on `runtime` between the sender and the daemon's ingest endpoint.
    pub fn relaying(mut self, runtime: &'a tokio::runtime::Runtime) -> Hooked<'a> {
        self.relay_on = Some(runtime);
        self
    }

    pub fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap()
    }

    pub fn relay_cut(&self) {
        self.book().relay.as_ref().expect("no relay").cut();
    }

    pub fn relay_pause(&self) {
        self.book().relay.as_ref().expect("no relay").pause();
    }

    pub fn names(&self) -> Vec<String> {
        self.book()
            .calls
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn count(&self, name: &str) -> usize {
        self.names().iter().filter(|n| *n == name).count()
    }

    pub fn created(&self) -> CaptureWorkspaceInfo {
        self.book().created.clone().expect("no create call")
    }

    /// A standalone seal of `entry_id` straight through the daemon, as `goalcup seal` would
    /// send it: no proxy counters, the whole budget (a test's, [`goalcup_cli::finish_budget`]).
    /// It is not recorded as a call.
    fn seal_now(&self, entry_id: &str) {
        let request = CaptureSealRequest {
            proxy: None,
            remaining_finish_ms: u64::try_from(goalcup_cli::finish_budget().as_millis()).ok(),
            native_scope: None,
        };
        let report = self
            .inner
            .seal_capture(entry_id, &request, Duration::from_secs(60))
            .unwrap_or_else(|e| panic!("the test's own seal was refused: {e:?}"));
        self.book().seal_report = Some(report);
    }

    fn note(&self, name: &str) {
        let started = self.started.exists();
        self.book().calls.push((name.to_string(), started));
    }
}

impl DaemonClient for Hooked<'_> {
    fn status(&self, entry_id: &str) -> Result<control::SignedStatus, ControlError> {
        self.note("status");
        if !self.book().registered.is_empty() {
            let wait = self.book().status_waits_for.take();
            if let Some(name) = wait {
                super::support::wait_for_file(&self.out.join(name));
            }
            if std::mem::take(&mut self.book().status_fault_once) {
                return Err(ControlError::Unreachable(
                    "the test lost this status read".to_string(),
                ));
            }
        }
        let status = self.inner.status(entry_id)?;
        self.book().admission.push((
            status.payload.producers_consumed,
            status.payload.admissions_refused,
        ));
        self.book()
            .sealed_seen
            .push(status.payload.sealed_at.is_some());
        Ok(status)
    }

    fn entries(&self) -> Result<Vec<control::EntrySummary>, ControlError> {
        self.inner.entries()
    }

    fn seal(&self, entry_id: &str) -> Result<control::SealResponse, ControlError> {
        self.note("seal");
        self.inner.seal(entry_id)
    }

    fn register_process(
        &self,
        entry_id: &str,
        request: &ProcessRegistration,
    ) -> Result<(), ControlError> {
        self.note("register_process");
        self.book().registered.push(request.handle.pid);
        let delay = self.book().registration_delay;
        if let Some(delay) = delay {
            std::thread::sleep(delay);
        }
        let fault = self.book().registration.take();
        match fault {
            None => self.inner.register_process(entry_id, request),
            Some(RegistrationFault::Refused) => Err(ControlError::Conflict(
                "the test refuses this process registration".to_string(),
            )),
            Some(RegistrationFault::AdmittedThenMalformed) => {
                self.inner.register_process(entry_id, request)?;
                Err(ControlError::Malformed(
                    "the test lost the registration's answer".to_string(),
                ))
            }
            Some(RegistrationFault::AdmittedThenSealed) => {
                self.inner.register_process(entry_id, request)?;
                self.seal_now(entry_id);
                Ok(())
            }
        }
    }

    fn health(&self) -> Result<control::DaemonHealth, ControlError> {
        self.inner.health()
    }

    fn current_entry(&self) -> Result<Option<String>, ControlError> {
        self.inner.current_entry()
    }

    fn abandon(&self, entry_id: &str, reason: &str) -> Result<(), ControlError> {
        self.note("abandon");
        self.inner.abandon(entry_id, reason)
    }

    fn create_capture_workspace(
        &self,
        request: &CaptureWorkspaceRequest,
    ) -> Result<CaptureWorkspaceInfo, ControlError> {
        self.note("create_capture_workspace");
        self.book().creates.push(request.clone());
        if self.book().occupy_ca_file {
            let ca = Path::new(&request.workspace).with_file_name("ca");
            std::fs::write(ca.join(goalcup_cli::proxy::tls::CA_CERT_FILE_NAME), b"").unwrap();
        }
        let mut info = self.inner.create_capture_workspace(request)?;
        let mut book = self.book();
        if let Some(addr) = book.ingest_addr {
            info.ingest_addr = addr;
        }
        if let Some(runtime) = self.relay_on {
            let relay = Relay::start(runtime, info.ingest_addr);
            info.ingest_addr = relay.addr;
            book.relay = Some(relay);
        }
        book.created = Some(info.clone());
        Ok(info)
    }

    fn record_capture_goal(
        &self,
        entry_id: &str,
        request: &CaptureGoalRequest,
    ) -> Result<goalcup_protocol::capture::StreamKey, ControlError> {
        self.note("record_capture_goal");
        let key = self.inner.record_capture_goal(entry_id, request)?;
        if std::mem::take(&mut self.book().lose_goal_answer) {
            let second = self.inner.record_capture_goal(entry_id, request).err();
            self.book().second_goal = second;
            return Err(ControlError::Unreachable(
                "the test lost the goal's answer".to_string(),
            ));
        }
        self.book().goal_answered = Some(std::time::SystemTime::now());
        Ok(key)
    }

    fn record_capture_launch(
        &self,
        entry_id: &str,
        request: &CaptureLaunchRequest,
    ) -> Result<(), ControlError> {
        self.note("record_capture_launch");
        let fault = self.book().launch_fault.take();
        match fault {
            None => self.inner.record_capture_launch(entry_id, request),
            Some(LaunchFault::Conflict) => Err(ControlError::Conflict(
                "the test refuses this launch record".to_string(),
            )),
        }
    }

    fn seal_capture(
        &self,
        entry_id: &str,
        request: &CaptureSealRequest,
        read_timeout: Duration,
    ) -> Result<CaptureFinishReport, CaptureSealError> {
        self.note("seal_capture");
        // Taken out and put back, so the hook runs outside the book's lock.
        let on_seal = self.book().on_seal.take();
        if let Some(on_seal) = on_seal {
            on_seal();
            self.book().on_seal = Some(on_seal);
        }
        let fault = {
            let mut book = self.book();
            book.seals.push(request.clone());
            book.seal_timeouts.push(read_timeout);
            book.seal_fault
        };
        if let Some(fault) = fault {
            return Err(fault());
        }
        self.inner.seal_capture(entry_id, request, read_timeout)
    }
}

// ---------------------------------------------------------------------------------------
// A relay in front of the ingest endpoint.
// ---------------------------------------------------------------------------------------

/// Forwards each accepted connection to `target`. `cut` closes every live connection;
/// `pause` stops reading from the sender, so what it writes waits in the socket, and holds
/// back the receiver's close from it.
pub struct Relay {
    pub addr: SocketAddr,
    paused: Arc<AtomicBool>,
    live: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    accept: tokio::task::JoinHandle<()>,
}

impl Relay {
    pub fn start(runtime: &tokio::runtime::Runtime, target: SocketAddr) -> Relay {
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let paused = Arc::new(AtomicBool::new(false));
        let live: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();
        let (p, l) = (Arc::clone(&paused), Arc::clone(&live));
        let accept = runtime.spawn(async move {
            while let Ok((from, _)) = listener.accept().await {
                let Ok(to) = tokio::net::TcpStream::connect(target).await else {
                    continue;
                };
                let paused = Arc::clone(&p);
                l.lock()
                    .unwrap()
                    .push(tokio::spawn(forward(from, to, paused)));
            }
        });
        Relay {
            addr,
            paused,
            live,
            accept,
        }
    }

    /// Close every connection relayed so far, both ends.
    pub fn cut(&self) {
        for task in self.live.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.accept.abort();
        self.cut();
    }
}

async fn forward(from: tokio::net::TcpStream, to: tokio::net::TcpStream, paused: Arc<AtomicBool>) {
    use tokio::io::AsyncReadExt;
    let (mut from_read, mut from_write) = from.into_split();
    let (mut to_read, mut to_write) = to.into_split();
    let held = Arc::clone(&paused);
    let mut back = Back(tokio::spawn(async move {
        let _ = tokio::io::copy(&mut to_read, &mut from_write).await;
        // A paused relay passes the receiver's close on to the sender no more than its bytes.
        while held.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }));
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        if paused.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        }
        match from_read.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to_write.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = to_write.shutdown().await;
    let _ = (&mut back.0).await;
}

/// The receiver-to-sender half of one relayed connection, ended with it when a cut aborts it.
struct Back(tokio::task::JoinHandle<()>);

impl Drop for Back {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// ---------------------------------------------------------------------------------------
// A TLS upstream for `localhost`.
// ---------------------------------------------------------------------------------------

/// Answers every request on `localhost:<port>` with `200 ok` and closes. The proxy trusts it
/// only through [`Upstream::roots`].
pub struct Upstream {
    pub port: u16,
    root: rustls::pki_types::CertificateDer<'static>,
    root_pem: String,
    accept: tokio::task::JoinHandle<()>,
}

impl Upstream {
    pub fn start(runtime: &tokio::runtime::Runtime) -> Upstream {
        let pki = TestPki::new("m2 cli upstream");
        let (chain, key) = pki.leaf(&["localhost"]);
        let config = rustls::ServerConfig::builder_with_provider(super::m2_common::provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let accept = runtime.spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let (head, mut body) = read_head(&mut tls).await;
                    let text = String::from_utf8_lossy(&head).to_lowercase();
                    let length: usize = text
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    read_at_least(&mut tls, &mut body, length).await;
                    let _ = tls
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                              Content-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        Upstream {
            port,
            root: pki.root(),
            root_pem: pki.root_pem(),
            accept,
        }
    }

    pub fn roots(&self) -> UpstreamRoots {
        UpstreamRoots::from_certificates([self.root.clone()]).unwrap()
    }

    /// This upstream's root certificate as a PEM file.
    pub fn root_pem(&self) -> String {
        self.root_pem.clone()
    }

    /// Shell lines that post `bytes` bytes of `a` through the proxy, in one request.
    pub fn curl_bytes(&self, bytes: usize) -> String {
        format!(
            "head -c {bytes} /dev/zero | tr '\\0' a > \"$M2_FAKE_OUT/body\"\n{}",
            self.post("--data-binary \"@$M2_FAKE_OUT/body\"")
        )
    }

    /// One shell line that posts `body` through the proxy the harness was given.
    pub fn curl(&self, body: &str) -> String {
        self.post(&format!("--data '{body}'"))
    }

    fn post(&self, data: &str) -> String {
        format!(
            "curl -sS --http1.1 --max-time 10 --noproxy '' --proxy \"$HTTPS_PROXY\" \
             --cacert \"$CODEX_CA_CERTIFICATE\" -H 'content-type: application/json' \
             {data} -o /dev/null https://localhost:{port}{EVIDENCE_PATH} \
             2>>\"$M2_FAKE_OUT/curl.err\"\n",
            port = self.port
        )
    }

    /// One shell line that reaches the upstream through a CONNECT to `localhost` whose TLS
    /// names another host: the proxy relays it unread (a fail-open tunnel) and counts it.
    pub fn curl_misnamed(&self) -> String {
        format!(
            "curl -sS --http1.1 --max-time 10 --noproxy '' --proxy \"$HTTPS_PROXY\" \
             --connect-to other.invalid:443:localhost:{port} -o /dev/null \
             https://other.invalid{EVIDENCE_PATH} 2>/dev/null\n",
            port = self.port
        )
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.accept.abort();
    }
}
