//! The `DaemonClient` every command test is driven against.
//!
//! Behind `feature = "test-support"`, which no shipped build turns on: `cargo build
//! --offline` with no features compiles neither this file nor the signature library it
//! needs. The reason it is in `src/` at all rather than in `tests/` is the usual one — this
//! crate's integration tests are separate crates and cannot see a `#[cfg(test)]` item in
//! this one — and it is the same shape the daemon's `test-hooks` feature has.
//!
//! # It is not a stub
//!
//! It records the order calls arrived in, it signs its `SignedStatus` with a fixed seed so a
//! test can pin `WorkspaceInfo.run_public_key` and mean it, it can withhold or delay the
//! start signal (which is how the D16 refusal is tested without a harness), and it can
//! answer one `busy` on a named method — the transient the daemon raises on every mutating
//! route while an entry is mid-pump.
//!
//! # What it does not model: headers
//!
//! It is in-process. It has no `Authorization` header to inspect and claims none, so the
//! bearer/public split is asserted on the wire, against `HttpDaemonClient` and a real
//! socket, in `tests/daemon_client.rs`. A mock that pretended to hold a bearer would be a
//! second, weaker copy of a rule that already has one home.
//!
//! # The surface is closed
//!
//! [`MockDaemonClient`]'s non-trait surface is the constructors and hooks below. A method
//! invented at a call site is a method the next test spells differently, so anything a test
//! needs that is not here is added here, in this file.

use ed25519_dalek::SigningKey;
use goalcup_protocol::capture::{DEFAULT_CAPTURE_RESERVE_BYTES, M2Status, StreamKey};
use goalcup_protocol::control::{
    self, ArmingState, CaptureFinishReport, CaptureGoalRequest, CaptureLaunchRequest,
    CaptureSealError, CaptureSealRequest, CaptureWorkspaceInfo, CaptureWorkspaceRequest,
    ControlError, DaemonClient, DaemonHealth, EntrySummary, EventStatus, FinishReportV1,
    LocalSealOutcome, ResumeInfo, ResumePreviewInfo, SealResponse, SignedStatus, StatusPayload,
    WorkspaceInfo, WorkspaceOpts,
};
use goalcup_protocol::events::HealthState;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use goalcup_protocol::provenance::{Rfc3339Utc, TerminationOutcomeV1};
use goalcup_protocol::seal::{SealManifest, SealedFile};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// The seed the mock's run key is built from. Fixed, because a test that pinned a random
/// key would be pinning nothing.
const SEED: [u8; 32] = [9u8; 32];

/// One clock. A mock that formatted `now` would be a different mock tomorrow.
const NOW: &str = "2026-09-05T00:00:00Z";

/// The goal stream the mock answers for an accepted goal.
const MOCK_GOAL_PRODUCER: &str = "6d6f636b676f616c70726f6475636572";

/// What the mock answers, and every body it was offered, for the v5 capture calls. A body is
/// kept even where the mock then refused the call.
#[derive(Debug, Clone)]
pub struct MockCapture {
    /// The create answer. Its run key is the mock's own.
    pub info: CaptureWorkspaceInfo,
    /// The local finish the v5 seal answers.
    pub finish: Option<CaptureFinishReport>,
    pub creates: Vec<CaptureWorkspaceRequest>,
    pub goals: Vec<CaptureGoalRequest>,
    pub launches: Vec<CaptureLaunchRequest>,
    pub seals: Vec<CaptureSealRequest>,
}

/// What the mock will hand back, and what it has been asked so far.
///
/// Every field is public because the hooks below are the only writers a test uses and this
/// is what they write to.
pub struct MockState {
    pub info: WorkspaceInfo,
    pub armed: bool,
    /// The start signal, or `None` for a gate that will never clear.
    pub signal: Option<control::StartSignal>,
    /// How long `wait_start_signal` waits before handing the signal over.
    pub signal_after: Option<Duration>,
    pub status: SignedStatus,
    pub entries: Vec<EntrySummary>,
    pub resume: ResumeInfo,
    /// Durable handoff state returned by the read-only resume preview.  Production keeps it
    /// in the registry; this test fake keeps it beside the matching answer so command tests
    /// can model a CLI that died after `/resume` opened its permit.
    pub resume_handoff_phase: String,
    /// A one-shot barrier for `resume`, consumed before waiting.  This lets a concurrent
    /// command test distinguish local serialization from two callers merely waiting on the
    /// same fake daemon response.
    pub resume_wait_once_for: Option<PathBuf>,
    /// Makes the next resume simulate the server accepting its CAS and opening the next
    /// permit, followed by a response body the client cannot decode.  This is the ambiguous
    /// after-send outcome a successor must recover through its durable marker.
    pub resume_accept_then_malformed: bool,
    /// Makes the answer `resume` (not `resume_preview`) gives, once, name a different
    /// workspace than every reading before it. `resume` and `resume_preview` both read
    /// `resume.workspace` off this one struct, so nothing on this fake can otherwise put the
    /// post-CAS answer at odds with the preview that ran moments before — the two always
    /// agree. A real daemon cannot disagree here either — `entry.workspace` is immutable
    /// once the entry is created — so this is a synthetic vehicle, not a model of any
    /// daemon race: it exists only to exercise the one lock-roots check that reads the CAS
    /// answer instead of the preview.
    pub resume_workspace_disagree_once: Option<String>,
    /// Expected generations passed to successful cancellation calls.  Kept separately from
    /// `calls` so recovery tests can prove the successor cancelled exactly N+1.
    pub cancelled_process_handoff_generations: Vec<u64>,
    pub seal: SealManifest,
    /// A durable finish answer. `None` retains the legacy manifest behavior used by
    /// historical command tests.
    pub workspace_finish: Option<FinishReportV1>,
    pub sealed: bool,
    /// Every typed compare-and-swap body the CLI offered to `/resume`, including a request
    /// the mock subsequently refused.  `calls` answers order; this preserves the authority
    /// generation the caller actually named.
    pub resume_requests: Vec<control::ResumeRequest>,
    pub processes: Vec<ProcessHandle>,
    /// Every typed `/process` body offered by the CLI, including a body the mock refused.
    /// `processes` remains the accepted-handle ledger; this one proves a retry did not
    /// silently change its generation or termination identity.
    pub process_registrations: Vec<ProcessRegistration>,
    /// Every typed cancellation body.  A cancelled permit has an authority generation and,
    /// once a child was prepared, its canonical request digest; tests need both rather than
    /// only the route name in `calls`.
    pub process_handoff_cancellations: Vec<control::ProcessHandoffCancelRequest>,
    pub abandoned: Vec<(String, String)>,
    pub calls: Vec<String>,
    pub last_opts: Option<WorkspaceOpts>,
    /// One refusal per method name, taken by the next call to it and then gone.
    pub failures: BTreeMap<String, ControlError>,
    /// One refusal per method name that is not taken: every call to that method answers it.
    /// Read after [`MockState::failures`], so a `fail_next` armed on a method that also has
    /// a standing refusal is still the one the next call gets.
    pub standing: BTreeMap<String, ControlError>,
    pub capture: MockCapture,
}

/// The in-process `DaemonClient`.
pub struct MockDaemonClient {
    state: Mutex<MockState>,
}

// ------------------------------------------------------------------ the fixed key

/// Lowercase hex, written out rather than pulling `hex` into this crate for one call. The
/// protocol's own `SignedStatus::sign` produces the same alphabet, which is what makes
/// [`MockDaemonClient::run_public_key`] comparable to `WorkspaceInfo.run_public_key`.
fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The repository's `connectors/`, as this crate's own tests see it.
///
/// `config::connectors_dir()` resolves relative to the running executable, and a test
/// binary runs from `target/debug/deps/` where neither of its candidates exists. This file
/// is compiled only under `test-support`, so the manifest directory is the honest answer
/// and it is never baked into anything a participant installs.
fn shipped_connectors() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop();
    dir.pop();
    dir.join("connectors")
}

/// The `arming.signal` connector `id` declares, read from the shipped manifest rather than
/// copied into a table here — a signal spelled twice is a signal that drifts.
///
/// A connector that cannot be read yields the reason in place of a signal, so the assertion
/// that compares this against the manifest fails naming the cause. Nothing in `src/` panics,
/// including here.
fn arming_signal_of(id: &str) -> String {
    match crate::connector::load(&shipped_connectors(), id) {
        Ok(connector) => connector.manifest.arming.signal,
        Err(e) => format!("the shipped connector {id} could not be read: {e}"),
    }
}

// ------------------------------------------------------------------ construction

impl MockDaemonClient {
    /// A mock holding one armed entry for `connector_id`, with its workspace, artifact root
    /// and harness home created under `tmp`.
    pub fn armed_ok(tmp: &Path, connector_id: &str) -> MockDaemonClient {
        let workspace = tmp.join("workspace");
        let artifact_root = tmp.join("artifacts");
        let harness_home = tmp.join("home");
        for dir in [&workspace, &artifact_root, &harness_home] {
            // Ignored rather than unwrapped: this file may not panic, and a directory that
            // could not be created fails the test that opens it, naming the path itself.
            let _ = std::fs::create_dir_all(dir);
        }
        let entry_id = "entry-mock-0001".to_string();
        let key = MockDaemonClient::signing_key();
        let run_public_key = hex_of(&key.verifying_key().to_bytes());
        let info = WorkspaceInfo {
            entry_id: entry_id.clone(),
            process_generation: 1,
            workspace: workspace.display().to_string(),
            // The mock hands back the same string for both spellings: it names a directory
            // it created itself, under a `tmp` the caller chose, so there is no requested
            // form for a canonicalisation to change. A test that needs the two to differ
            // builds that difference itself.
            workspace_requested: workspace.display().to_string(),
            artifact_root: artifact_root.display().to_string(),
            harness_home: harness_home.display().to_string(),
            run_public_key,
            // The page, with the entry id in the query, which is what
            // `goalcup_daemon::control::handlers` builds: `/v0/entries/current` is
            // authenticated and a browser has no token, so the daemon tells the page which
            // entry it is looking at. `goalcup ui` opens this string byte for byte, so it must
            // not be the signed status route, which no daemon puts on this field.
            status_url: format!(
                "http://127.0.0.1:53310{}?entry={entry_id}",
                control::ROUTE_UI
            ),
            status_cache_path: artifact_root.join("status.json").display().to_string(),
            status_line_path: artifact_root.join("status-line.txt").display().to_string(),
            session_id: "session-mock-0001".to_string(),
            server_port: None,
            installed_files: Vec::new(),
            workspace_relative: Vec::new(),
            created_at: NOW.to_string(),
        };
        let payload = StatusPayload {
            entry_id: entry_id.clone(),
            connector_id: connector_id.to_string(),
            workspace: info.workspace.clone(),
            session_id: Some(info.session_id.clone()),
            harness_version: Some("0.0.0-mock".to_string()),
            arming: ArmingState::Armed,
            health: HealthState::Online,
            started_at: Some(NOW.to_string()),
            deadline_at: None,
            sealed_at: None,
            seconds_remaining: None,
            last_sync_at: Some(NOW.to_string()),
            events_uploaded: 0,
            events_pending: 0,
            arming_recipe: None,
            // The daemon's own plain words for `Online` (`goalcup_daemon::status::
            // plain_words`), so a command test asserts the CLI prints the daemon's
            // vocabulary rather than a second one of its own.
            detail: "Recording. Everything is being captured and synced.".to_string(),
            // The entry's own timeline standing in for a configured round, which is
            // exactly what `EventStatusSource::Fallback` below declares it to be.
            event: EventStatus {
                t0: NOW.to_string(),
                start_window_closes_at: NOW.to_string(),
                publication_closes_at: NOW.to_string(),
                rules_version: "mock".to_string(),
            },
            event_status_source: control::EventStatusSource::Fallback,
            cycles: control::CycleErrors::default(),
            posts_dropped: 0,
            posts_dropped_after_seal: 0,
            pins_dropped: 0,
            polls_capped: 0,
            cursor_restore_failures: 0,
            skipped_replayed: 0,
            buffer_gap_at: None,
            files_capped: 0,
            harness_unverified: false,
            finalization: None,
            capture: None,
            producers_consumed: None,
            admissions_refused: None,
        };
        let state = MockState {
            armed: true,
            signal: Some(control::StartSignal {
                entry_id: entry_id.clone(),
                signal: arming_signal_of(connector_id),
                session_id: Some(info.session_id.clone()),
                observed_at: NOW.to_string(),
                source: "leg1".to_string(),
            }),
            signal_after: None,
            status: signed(payload, &key),
            entries: vec![EntrySummary {
                entry_id: entry_id.clone(),
                connector_id: connector_id.to_string(),
                workspace: info.workspace.clone(),
                arming: ArmingState::Armed,
                health: HealthState::Online,
                started_at: Some(NOW.to_string()),
                deadline_at: None,
                sealed_at: None,
            }],
            resume: ResumeInfo {
                entry_id: entry_id.clone(),
                process_generation: 2,
                session_id: bound_session_of(&info.session_id),
                caught_up_events: 0,
                workspace: info.workspace.clone(),
                artifact_root: info.artifact_root.clone(),
                harness_home: info.harness_home.clone(),
                status_line_path: info.status_line_path.clone(),
                server_port: None,
                last_process: None,
            },
            resume_handoff_phase: "registered".to_string(),
            resume_wait_once_for: None,
            resume_accept_then_malformed: false,
            resume_workspace_disagree_once: None,
            cancelled_process_handoff_generations: Vec::new(),
            seal: SealManifest {
                entry_id: entry_id.clone(),
                sealed_at: NOW.to_string(),
                workspace: info.workspace.clone(),
                files: Vec::new(),
                excluded: Vec::new(),
            },
            workspace_finish: None,
            sealed: false,
            resume_requests: Vec::new(),
            processes: Vec::new(),
            process_registrations: Vec::new(),
            process_handoff_cancellations: Vec::new(),
            abandoned: Vec::new(),
            calls: Vec::new(),
            last_opts: None,
            failures: BTreeMap::new(),
            standing: BTreeMap::new(),
            capture: MockCapture {
                info: CaptureWorkspaceInfo {
                    entry_id: entry_id.clone(),
                    process_generation: 1,
                    reserved_bytes: DEFAULT_CAPTURE_RESERVE_BYTES,
                    ingest_addr: std::net::SocketAddr::from(([127, 0, 0, 1], 53311)),
                    ingest_secret: "mock-ingest-secret-0123456789abcdef".to_string(),
                    producer_id: "6d6f636b70726f787970726f64756365".to_string(),
                    run_public_key: info.run_public_key.clone(),
                },
                finish: Rfc3339Utc::try_from(NOW)
                    .ok()
                    .map(|checked_at| CaptureFinishReport {
                        entry_id: entry_id.clone(),
                        trace_dir: artifact_root.join("trace").display().to_string(),
                        termination: TerminationOutcomeV1::AlreadyStopped { checked_at },
                        terminal_seq: Some(3),
                        seal: LocalSealOutcome::SealWritten,
                    }),
                creates: Vec::new(),
                goals: Vec::new(),
                launches: Vec::new(),
                seals: Vec::new(),
            },
            info,
        };
        MockDaemonClient {
            state: Mutex::new(state),
        }
    }

    /// The run key, from the fixed [`SEED`].
    pub fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&SEED)
    }

    /// The hex verifying key the mock signs with — the same string
    /// [`MockDaemonClient::info`] carries on `run_public_key`.
    pub fn run_public_key(&self) -> String {
        hex_of(&MockDaemonClient::signing_key().verifying_key().to_bytes())
    }
}

/// The payload signed with `key`, or an unsigned document whose signature will not check.
///
/// A signing failure cannot happen for a payload this file built — the serialisation is
/// `serde_json` over a struct of strings and numbers — and the empty signature is here so
/// that nothing in this file unwraps, not because the case is reachable.
fn signed(payload: StatusPayload, key: &SigningKey) -> SignedStatus {
    match SignedStatus::sign(payload.clone(), NOW.to_string(), key) {
        Ok(status) => status,
        Err(_) => SignedStatus {
            payload,
            public_key: hex_of(&key.verifying_key().to_bytes()),
            signature: String::new(),
            cached_at: NOW.to_string(),
        },
    }
}

// ------------------------------------------------------------------ reading it back

impl MockDaemonClient {
    /// Whatever the lock hands back, or `None` on a poisoned one. Every accessor below goes
    /// through it, because a `Mutex` this file may not `unwrap` still has to be readable.
    fn read<T>(&self, f: impl FnOnce(&MockState) -> T) -> Option<T> {
        self.state.lock().ok().map(|state| f(&state))
    }

    /// The same, for a hook that writes.
    fn write<T>(&self, f: impl FnOnce(&mut MockState) -> T) -> Option<T> {
        self.state.lock().ok().map(|mut state| f(&mut state))
    }

    /// The `WorkspaceInfo` this mock will hand back, readable before the call is made so a
    /// test can assert against the same struct the command under test received.
    pub fn info(&self) -> WorkspaceInfo {
        match self.read(|state| state.info.clone()) {
            Some(info) => info,
            None => unreachable_info(),
        }
    }

    /// What a resume of this entry answers.
    pub fn resume_info(&self) -> ResumeInfo {
        match self.read(|state| state.resume.clone()) {
            Some(resume) => resume,
            None => unreachable_resume(),
        }
    }

    /// Every complete `/resume` CAS body this mock was offered, in call order.
    ///
    /// The ordinary string call log can prove that a route was reached, but it cannot prove
    /// that the client used the daemon's freshly previewed generation rather than a stale
    /// cache value. Keep the typed bodies observable without teaching command tests a second
    /// fake control protocol.
    pub fn resume_requests(&self) -> Vec<control::ResumeRequest> {
        self.read(|state| state.resume_requests.clone())
            .unwrap_or_default()
    }

    /// Every complete `/process` body this mock was offered, including one it refused.
    ///
    /// `last_registered_handle` intentionally answers a different question: the latest
    /// *accepted* handle. A recovery test needs the exact request body before an ambiguous
    /// or refused response has been classified.
    pub fn process_registrations(&self) -> Vec<ProcessRegistration> {
        self.read(|state| state.process_registrations.clone())
            .unwrap_or_default()
    }

    /// Every complete `/process/cancel` body this mock was offered, in call order.
    pub fn process_handoff_cancellations(&self) -> Vec<control::ProcessHandoffCancelRequest> {
        self.read(|state| state.process_handoff_cancellations.clone())
            .unwrap_or_default()
    }

    /// What the read-only resume preview answers for this entry.
    pub fn resume_preview_info(&self) -> ResumePreviewInfo {
        match self
            .read(|state| preview_from_resume(&state.resume, state.resume_handoff_phase.clone()))
        {
            Some(preview) => preview,
            None => preview_from_resume(&unreachable_resume(), "registered".to_string()),
        }
    }

    /// Make the next `resume` wait until `path` exists, after recording the call and before
    /// returning its permit.  A command test can then hold one local CLI in the critical
    /// section while a second one tries the same entry.
    pub fn block_next_resume_until(&self, path: &Path) {
        let path = path.to_path_buf();
        self.write(|state| state.resume_wait_once_for = Some(path));
    }

    /// Make the next resume mutate the simulated daemon as though the request won, then
    /// return a malformed response to the caller.
    pub fn accept_next_resume_then_malformed(&self) {
        self.write(|state| state.resume_accept_then_malformed = true);
    }

    /// Make the next `resume` answer a different workspace than every `resume_preview`
    /// before it. Synthetic: no real daemon can do this, since `entry.workspace` is
    /// immutable after creation — see [`MockState::resume_workspace_disagree_once`].
    pub fn disagree_workspace_on_next_resume(&self, workspace: &str) {
        self.write(|state| {
            state.resume_workspace_disagree_once = Some(workspace.to_string());
        });
    }

    /// Set the durable handoff state a restart observes.  It models the registry rather than
    /// a client cache, which is why a test can distinguish its own crashed resume from an
    /// unrelated open permit.
    pub fn set_resume_handoff(&self, generation: u64, phase: &str) {
        let phase = phase.to_string();
        self.write(|state| {
            state.resume.process_generation = generation;
            state.resume_handoff_phase = phase;
        });
    }

    /// The trait methods called on this mock, in order.
    pub fn calls(&self) -> Vec<String> {
        self.read(|state| state.calls.clone()).unwrap_or_default()
    }

    /// The exact expected generation on each successful handoff cancellation.
    pub fn cancelled_process_handoff_generations(&self) -> Vec<u64> {
        self.read(|state| state.cancelled_process_handoff_generations.clone())
            .unwrap_or_default()
    }

    /// How many entries this mock reports.
    pub fn entry_count(&self) -> usize {
        self.read(|state| state.entries.len()).unwrap_or_default()
    }

    /// Every `(entry_id, reason)` the CLI abandoned, in order.
    pub fn abandoned(&self) -> Vec<(String, String)> {
        self.read(|state| state.abandoned.clone())
            .unwrap_or_default()
    }

    /// The options the last `create_workspace` actually sent.
    ///
    /// With nothing sent yet the answer is a sentinel whose `connector_id` says so, rather
    /// than a plausible options document a test could assert against by accident.
    pub fn last_workspace_opts(&self) -> WorkspaceOpts {
        match self.read(|state| state.last_opts.clone()) {
            Some(Some(opts)) => opts,
            _ => WorkspaceOpts {
                connector_id: "no create_workspace call was made on this mock".to_string(),
                manifest_sha256: String::new(),
                workspace_parent: None,
                attach: false,
                deadline_hours: 0,
                grant_token: String::new(),
                arming_timeout_seconds: None,
                harness_version: None,
            },
        }
    }

    /// `status.payload.detail` — the daemon's own plain words, so a command test asserts
    /// that the CLI prints them rather than a second vocabulary of its own.
    pub fn status_detail(&self) -> String {
        self.read(|state| state.status.payload.detail.clone())
            .unwrap_or_default()
    }

    /// The v5 capture answers and every capture body this mock was offered.
    pub fn capture(&self) -> Option<MockCapture> {
        self.read(|state| state.capture.clone())
    }

    /// Make the status a v5 entry's: its capture block, `detail` and admission counts,
    /// re-signed with the run key.
    pub fn set_capture_status(
        &self,
        capture: M2Status,
        detail: &str,
        producers_consumed: u64,
        admissions_refused: u64,
    ) {
        let detail = detail.to_string();
        self.write(|state| {
            let payload = &mut state.status.payload;
            payload.capture = Some(capture);
            payload.detail = detail;
            payload.producers_consumed = Some(producers_consumed);
            payload.admissions_refused = Some(admissions_refused);
            resign(state);
        });
    }
}

/// The id the arming gate bound, given the id `create_workspace` minted.
///
/// A real daemon answers a resume with "the session the latest corroborating leg-1 record
/// belongs to", and only `claude-code-cli` pins the minted id at launch (`--session-id`):
/// `codex-cli` pins none, so the harness mints its own and the daemon's bound id is a
/// different string. Deriving it rather than naming a constant keeps it recognisable in a
/// failure message and keeps it unequal by construction.
pub fn bound_session_of(minted: &str) -> String {
    format!("{minted}-bound")
}

/// The `WorkspaceInfo` a poisoned lock yields: every string says what happened, so an
/// assertion against it fails naming the cause instead of comparing empty strings.
fn unreachable_info() -> WorkspaceInfo {
    let said = "the mock's state was poisoned by a panic in another test thread".to_string();
    WorkspaceInfo {
        entry_id: said.clone(),
        process_generation: 0,
        workspace: said.clone(),
        workspace_requested: said.clone(),
        artifact_root: said.clone(),
        harness_home: said.clone(),
        run_public_key: said.clone(),
        status_url: said.clone(),
        status_cache_path: said.clone(),
        status_line_path: said.clone(),
        session_id: said.clone(),
        server_port: None,
        installed_files: Vec::new(),
        workspace_relative: Vec::new(),
        created_at: said,
    }
}

/// See [`unreachable_info`].
fn unreachable_resume() -> ResumeInfo {
    let said = "the mock's state was poisoned by a panic in another test thread".to_string();
    ResumeInfo {
        entry_id: said.clone(),
        process_generation: 0,
        session_id: said.clone(),
        caught_up_events: 0,
        workspace: said.clone(),
        artifact_root: said.clone(),
        harness_home: said.clone(),
        status_line_path: said,
        server_port: None,
        last_process: None,
    }
}

fn preview_from_resume(info: &ResumeInfo, process_handoff_phase: String) -> ResumePreviewInfo {
    ResumePreviewInfo {
        entry_id: info.entry_id.clone(),
        process_generation: info.process_generation,
        process_handoff_phase,
        session_id: info.session_id.clone(),
        workspace: info.workspace.clone(),
        artifact_root: info.artifact_root.clone(),
        harness_home: info.harness_home.clone(),
        status_line_path: info.status_line_path.clone(),
        server_port: info.server_port,
    }
}

// ------------------------------------------------------------------ the hooks

impl MockDaemonClient {
    /// Hand the start signal over only after `d`. A delay longer than the caller's timeout
    /// is an `ArmingTimeout`, exactly as the daemon's long poll would report it.
    ///
    /// **It sets the delay and nothing else.** In particular it does not re-arm a signal
    /// [`MockDaemonClient::withhold_signal`] took away: a hook named for one thing that
    /// quietly undoes another is a hook whose call order changes the answer, and a test
    /// asserting a gate stays shut would pass or fail on where in the file the delay was
    /// set. The two hooks compose in the honest direction instead — delay it, watch it
    /// arrive, withhold it, watch it time out — which is what `tests/daemon_client.rs`
    /// does.
    pub fn set_signal_delay(&self, d: Duration) {
        self.write(|state| state.signal_after = Some(d));
    }

    /// A gate that will never clear, and nothing below re-opens it.
    pub fn withhold_signal(&self) {
        self.write(|state| state.signal = None);
    }

    /// Put the status payload into `ArmingState::Refused` with this `detail` — the fourth
    /// arming state, which is a refusal and not a timeout.
    pub fn set_arming_refused(&self, detail: &str) {
        let detail = detail.to_string();
        self.write(|state| {
            state.status.payload.arming = ArmingState::Refused;
            state.status.payload.health = HealthState::ArmingPending;
            state.status.payload.detail = detail;
            resign(state);
        });
    }

    /// One of the four cadences the daemon's own loop runs, and the words its last failure
    /// left. `cycle` is `tick`, `upload`, `status` or `deadline`; anything else counts
    /// nothing and still records the message, which is what makes a mistyped cadence visible
    /// in the assertion rather than silent.
    pub fn set_cycle_error(&self, cycle: &str, message: &str) {
        let (cycle, message) = (cycle.to_string(), message.to_string());
        self.write(|state| {
            let cycles = &mut state.status.payload.cycles;
            match cycle.as_str() {
                "tick" => cycles.tick += 1,
                "upload" => cycles.upload += 1,
                "status" => cycles.status += 1,
                "deadline" => cycles.deadline += 1,
                _ => {}
            }
            cycles.last = Some(message);
            resign(state);
        });
    }

    /// What the daemon says the harness's version is, including the empty string.
    ///
    /// `armed_ok` answers `"0.0.0-mock"`; a harness that answered the probe with nothing is
    /// `Some("")`, which only this hook reaches, and `goalcup status` must not print a line
    /// with a dangling colon for it.
    pub fn set_harness_version(&self, version: Option<&str>) {
        let version = version.map(str::to_string);
        self.write(|state| {
            state.status.payload.harness_version = version.clone();
            resign(state);
        });
    }

    /// The session this entry is bound to, or `None` for a run whose harness never started.
    ///
    /// `goalcup seal` asks the daemon this before it seals, so a participant sealing a run
    /// that never began is told what the verdict will be while stopping is still a choice
    /// they have. Reachable from a test only through here, because every mock builds an
    /// entry that is already bound.
    pub fn set_session_id(&self, session_id: Option<&str>) {
        let session_id = session_id.map(str::to_string);
        self.write(|state| {
            state.status.payload.session_id = session_id.clone();
            resign(state);
        });
    }

    /// Answer with a document about a different entry, correctly signed.
    ///
    /// One run key signs every document of a run, so a payload for another entry verifies
    /// exactly as well as this one's — a routing bug, or an id the CLI resolved and the
    /// daemon did not. Re-signed here on purpose: the point of this hook is a document that
    /// passes the signature check and fails the id check, which is the half a signature
    /// cannot cover, and it is the only way a test reaches that comparison.
    pub fn set_status_entry_id(&self, entry_id: &str) {
        let entry_id = entry_id.to_string();
        self.write(|state| {
            state.status.payload.entry_id = entry_id.clone();
            resign(state);
        });
    }

    /// Hook posts that never reached this entry's chain.
    pub fn set_posts_dropped(&self, n: u64) {
        self.write(|state| {
            state.status.payload.posts_dropped = n;
            resign(state);
        });
    }

    /// Mark the signed ending as unable to prove complete harness termination.
    pub fn set_harness_unverified(&self, unverified: bool) {
        self.write(|state| {
            state.status.payload.harness_unverified = unverified;
            resign(state);
        });
    }

    /// Re-arm the seal: the next one succeeds and every one after it is a `conflict`, which
    /// is what this mock does from the moment it is built.
    pub fn seal_once_then_conflict(&self) {
        self.write(|state| state.sealed = false);
    }

    /// Return the daemon-owned finish result instead of the historical live-scan manifest.
    pub fn set_workspace_finish(&self, finish: FinishReportV1) {
        self.write(|state| state.workspace_finish = Some(finish));
    }

    /// Edit the signed payload's bytes without re-signing: the signature no longer checks,
    /// which is the only way to test the "did not verify" path.
    pub fn tamper_status_payload(&self) {
        self.write(|state| state.status.payload.detail.push_str(" (tampered)"));
    }

    /// Refuse the next call to `method` with `err`, once.
    pub fn fail_next(&self, method: &str, err: ControlError) {
        let method = method.to_string();
        self.write(|state| state.failures.insert(method, err));
    }

    /// Refuse **every** call to `method` with `err`, standing until this mock is dropped.
    ///
    /// [`MockDaemonClient::fail_next`] holds one refusal per method and takes it, so arming
    /// it twice arms it once — and a command whose calls go through
    /// one of this crate's route-scoped retry helpers makes its bounded number of them. "The
    /// daemon is still busy when the retry is spent" is a distinct fact from "it was busy
    /// once", the two land on different rungs, and this is the only way to say the first.
    pub fn fail_always(&self, method: &str, err: ControlError) {
        let method = method.to_string();
        self.write(|state| state.standing.insert(method, err));
    }

    /// The page URL the workspace answer carries, and the spelling of the workspace it was
    /// asked for.
    ///
    /// Two fields and one hook, because both are answers about the workspace that only
    /// `create_workspace` gives and neither has any other way in. `goalcup start` refuses a
    /// `status_url` that is not a loopback page before it writes one into the entry cache, and
    /// prints `workspace_requested` where it differs from the canonical spelling (D16). A test
    /// cannot reach either through the fixture, which builds one consistent answer.
    ///
    /// `None` leaves a field as it stands, so a test that is about one of the two says so.
    pub fn set_workspace_answer(&self, status_url: Option<&str>, requested: Option<&str>) {
        self.write(|state| {
            if let Some(url) = status_url {
                state.info.status_url = url.to_string();
            }
            if let Some(requested) = requested {
                state.info.workspace_requested = requested.to_string();
            }
        });
    }

    /// The port the daemon reserved for this entry, on **both** answers it gives about it.
    ///
    /// `armed_ok` builds a mock with no port, which is right for the shipped connectors, none
    /// of which reserves one, and wrong for a manifest whose launch and resume recipes both
    /// render `{server_port}`. Both `WorkspaceInfo` and `ResumeInfo` are set from the one
    /// call: the daemon remembers the port it reserved at creation and hands the same one
    /// back on a resume, so a mock whose two answers disagreed about it would be one no
    /// daemon could ever produce.
    pub fn set_server_port(&self, port: Option<u16>) {
        self.write(|state| {
            state.info.server_port = port;
            state.resume.server_port = port;
        });
    }

    /// The `hook_install[]` files the daemon rendered and wrote, and the
    /// workspace-relative list derived from them.
    ///
    /// `armed_ok` lists none, which leaves the entry record's `workspace_relative` empty —
    /// and that list is the half of the seal's exclusion set that names the tracker's own
    /// files. It is load-bearing for **disclosure**, not for exclusion. A run against a mock
    /// that answered nothing here would still not count `.claude/settings.json` as the
    /// agent's work, because `.claude` is in `claude-code-cli`'s own `workspace_exclusions`
    /// and the connector prunes that subtree on its own; what it would lose is the naming —
    /// the seal would apply two patterns instead of three and never say out loud which file
    /// the tracker wrote into the participant's workspace.
    ///
    /// `workspace_relative` is **derived here** rather than taken as a second argument, the
    /// way `goalcup_daemon::workspace::Installation::from_files` derives it: the
    /// `Workspace`-scoped relative paths, sorted and deduplicated. Two fields a caller could
    /// set apart from each other is a fixture no daemon could ever answer with, and a
    /// duplicate this dropped is one `from_files` drops too.
    pub fn set_workspace_files(&self, files: Vec<control::InstalledFile>) {
        let mut relative: Vec<String> = files
            .iter()
            .filter(|f| f.scope == goalcup_protocol::manifest::InstallScope::Workspace)
            .map(|f| f.relative_path.clone())
            .collect();
        relative.sort();
        relative.dedup();
        self.write(|state| {
            state.info.installed_files = files;
            state.info.workspace_relative = relative;
        });
    }

    /// How many events the daemon caught up before answering the resume.
    pub fn set_caught_up_events(&self, n: u64) {
        self.write(|state| state.resume.caught_up_events = n);
    }

    /// Answer one `busy` on this method and then succeed — the transient the daemon raises
    /// on every mutating route while an entry is mid-pump.
    pub fn busy_once(&self, method: &str) {
        self.fail_next(
            method,
            ControlError::Busy("this entry is held by a call already running".to_string()),
        );
    }

    /// Put a process on the resume answer, so a test can meet a live or a dead holder in
    /// front of the pre-relaunch lock check.
    pub fn set_last_process_pid(&self, pid: u32) {
        self.write(|state| {
            let handle = match state.resume.last_process.take() {
                Some(handle) => ProcessHandle { pid, ..handle },
                None => ProcessHandle {
                    pid,
                    pgid: Some(pid),
                    native_pid: None,
                    native_binary: None,
                    native_process_start: None,
                    process_start: None,
                    strategy: goalcup_protocol::manifest::KillStrategy::ProcessGroup,
                    grace_seconds: 5,
                    launched_at: NOW.to_string(),
                    program: "/usr/bin/true".to_string(),
                    argv: vec!["true".to_string()],
                    env_keys: Vec::new(),
                    home_seed: None,
                },
            };
            state.resume.last_process = Some(handle);
        });
    }

    /// The pid of the handle most recently `register_process`ed, so a test can assert the
    /// relaunch was handed over and the entry cache followed it.
    pub fn last_registered_pid(&self) -> Option<u32> {
        self.read(|state| state.processes.last().map(|h| h.pid))
            .flatten()
    }

    /// The whole of that handle.
    ///
    /// The pid alone cannot answer the question a resume raises: whether what was handed
    /// over describes the resume recipe. The daemon's
    /// `same_process` guard compares a live command line against `argv`, and a relaunch that
    /// registered the start argv would be handing the daemon a description of a process that
    /// no longer exists.
    pub fn last_registered_handle(&self) -> Option<ProcessHandle> {
        self.read(|state| state.processes.last().cloned()).flatten()
    }

    /// Put `rel` in the sealed manifest with a digest the workspace's own bytes cannot
    /// match, so the CLI's mirror self-check has something to disagree with.
    pub fn make_sealed_manifest_differ_on(&self, rel: &str) {
        let rel = rel.to_string();
        self.write(|state| {
            let differs = SealedFile {
                path: rel.clone(),
                bytes: 0,
                sha256: "00".repeat(32),
            };
            match state.seal.files.iter_mut().find(|f| f.path == rel) {
                Some(file) => *file = differs,
                None => state.seal.files.push(differs),
            }
        });
    }
}

/// Sign the payload again after a hook changed it, so every hook except
/// [`MockDaemonClient::tamper_status_payload`] leaves a document that still verifies.
fn resign(state: &mut MockState) {
    state.status = signed(
        state.status.payload.clone(),
        &MockDaemonClient::signing_key(),
    );
}

// ------------------------------------------------------------------ the trait

/// Record the call and take whatever refusal was armed for it.
///
/// The once-only refusal first and the standing one after it, so a method carrying both
/// answers the specific refusal on its next call and the standing one from then on.
fn record(state: &mut MockState, method: &str) -> Result<(), ControlError> {
    state.calls.push(method.to_string());
    if let Some(err) = state.failures.remove(method) {
        return Err(err);
    }
    match state.standing.get(method) {
        Some(err) => Err(clone_control_error(err)),
        None => Ok(()),
    }
}

/// `ControlError` is not `Clone`, and a standing refusal has to be answerable more than
/// once. Written out rather than deriving `Clone` in the protocol crate for a test fake:
/// every variant of the daemon's refusal vocabulary carries strings and nothing else.
fn clone_control_error(e: &ControlError) -> ControlError {
    match e {
        ControlError::Unreachable(s) => ControlError::Unreachable(s.clone()),
        ControlError::NotFound(s) => ControlError::NotFound(s.clone()),
        ControlError::Conflict(s) => ControlError::Conflict(s.clone()),
        ControlError::Unauthorized(s) => ControlError::Unauthorized(s.clone()),
        ControlError::UnsupportedConnector(s) => ControlError::UnsupportedConnector(s.clone()),
        ControlError::PreflightFailed(s) => ControlError::PreflightFailed(s.clone()),
        ControlError::Busy(s) => ControlError::Busy(s.clone()),
        ControlError::ArmingTimeout { detail } => ControlError::ArmingTimeout {
            detail: detail.clone(),
        },
        ControlError::Internal(s) => ControlError::Internal(s.clone()),
        ControlError::Malformed(s) => ControlError::Malformed(s.clone()),
        ControlError::Unsupported(s) => ControlError::Unsupported(s.clone()),
        ControlError::ProcessAdmissionUncertain(s) => {
            ControlError::ProcessAdmissionUncertain(s.clone())
        }
        ControlError::ProcessAdmissionBusy(s) => ControlError::ProcessAdmissionBusy(s.clone()),
        ControlError::FinishDeadline(s) => ControlError::FinishDeadline(s.clone()),
    }
}

/// A poisoned lock, as the one error a caller can do something about.
fn poisoned() -> ControlError {
    ControlError::Internal(
        "the mock's state was poisoned by a panic in another test thread".to_string(),
    )
}

/// Test-only bounded barrier for a concurrent command test.  A missing release marker must
/// become a test failure, not hang the suite forever.
fn wait_for_test_file(path: &Path) -> Result<(), ControlError> {
    for _ in 0..400 {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(ControlError::Internal(format!(
        "the test release marker {} never appeared",
        path.display()
    )))
}

impl DaemonClient for MockDaemonClient {
    fn create_workspace(
        &self,
        _harness: &str,
        opts: &WorkspaceOpts,
    ) -> Result<WorkspaceInfo, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "create_workspace")?;
        state.last_opts = Some(opts.clone());
        Ok(state.info.clone())
    }

    fn arm(&self, _entry_id: &str) -> Result<(), ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "arm")?;
        state.armed = true;
        Ok(())
    }

    fn wait_start_signal(
        &self,
        entry_id: &str,
        timeout: Duration,
    ) -> Result<control::StartSignal, ControlError> {
        let waited = {
            let Ok(mut state) = self.state.lock() else {
                return Err(poisoned());
            };
            record(&mut state, "wait_start_signal")?;
            // The signal is about the entry that was asked about, the way the daemon's
            // own long poll answers off the gate it was handed an id for.
            let Some(signal) = state.signal.clone().map(|s| control::StartSignal {
                entry_id: entry_id.to_string(),
                ..s
            }) else {
                return Err(ControlError::ArmingTimeout {
                    detail: "the tracker never saw your harness start in this folder, so \
                             nothing was recorded"
                        .to_string(),
                });
            };
            let after = state.signal_after.unwrap_or_default();
            if after > timeout {
                return Err(ControlError::ArmingTimeout {
                    detail: format!(
                        "the start signal did not arrive within {}ms",
                        timeout.as_millis()
                    ),
                });
            }
            (after, signal)
        };
        // Outside the lock: a mock that slept holding it would serialise a test's threads
        // against a delay the test asked one of them to wait out.
        std::thread::sleep(waited.0);
        Ok(waited.1)
    }

    fn status(&self, _entry_id: &str) -> Result<SignedStatus, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "status")?;
        Ok(state.status.clone())
    }

    fn entries(&self) -> Result<Vec<EntrySummary>, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "entries")?;
        Ok(state.entries.clone())
    }

    fn resume(
        &self,
        _entry_id: &str,
        request: &control::ResumeRequest,
    ) -> Result<ResumeInfo, ControlError> {
        let (info, wait_for, malformed_after_accept) = {
            let Ok(mut state) = self.state.lock() else {
                return Err(poisoned());
            };
            // Keep the request even where the mock refuses it: the caller has already
            // offered a concrete generation to the daemon, and recovery tests must
            // distinguish that from a route that was never reached.
            state.resume_requests.push(request.clone());
            record(&mut state, "resume")?;
            let malformed_after_accept = std::mem::take(&mut state.resume_accept_then_malformed);
            if malformed_after_accept {
                state.resume.process_generation += 1;
                state.resume_handoff_phase = "open_resume".to_string();
            }
            let mut answered = state.resume.clone();
            if let Some(workspace) = state.resume_workspace_disagree_once.take() {
                answered.workspace = workspace;
            }
            (
                answered,
                state.resume_wait_once_for.take(),
                malformed_after_accept,
            )
        };
        if let Some(path) = wait_for {
            wait_for_test_file(&path)?;
        }
        if malformed_after_accept {
            return Err(ControlError::Malformed(
                "resume was accepted but its response could not be decoded".to_string(),
            ));
        }
        Ok(info)
    }

    fn resume_preview(&self, _entry_id: &str) -> Result<ResumePreviewInfo, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "resume_preview")?;
        Ok(preview_from_resume(
            &state.resume,
            state.resume_handoff_phase.clone(),
        ))
    }

    /// A second seal is a `conflict` — already sealed is a state that will not change, and
    /// a conflict is the one `409` a client must never retry.
    ///
    /// **The sealed entry leaves the live list.** `GET /v0/entries` walks the
    /// daemon's in-memory entry map and a sealed entry is evicted from it once the ack
    /// watermark reaches the chain head, while `signed_status` keeps answering for that
    /// entry out of the frozen status cache. A mock whose `entries` still listed a sealed
    /// run could not show that disagreement at all, and it is the disagreement `goalcup
    /// entries` exists to explain: a participant must not read a finished run's absence
    /// from this list as the run being gone. The eviction is immediate here rather than at
    /// an ack watermark this mock has no chain to measure — the fact under test is that it
    /// happens, not when.
    fn seal(&self, entry_id: &str) -> Result<SealResponse, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "seal")?;
        if state.sealed && state.workspace_finish.is_none() {
            return Err(ControlError::Conflict(format!(
                "{} is already sealed",
                state.info.entry_id
            )));
        }
        state.sealed = true;
        state.entries.retain(|e| e.entry_id != entry_id);
        Ok(match &state.workspace_finish {
            Some(finish) => SealResponse::Workspace(finish.clone()),
            None => SealResponse::Legacy(state.seal.clone()),
        })
    }

    fn register_process(
        &self,
        _entry_id: &str,
        registration: &ProcessRegistration,
    ) -> Result<(), ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        // As above, this is the offered immutable body, not the accepted-handle ledger.
        // The latter remains `processes` and changes only after `record` permits the route.
        state.process_registrations.push(registration.clone());
        record(&mut state, "register_process")?;
        state.processes.push(registration.handle.clone());
        Ok(())
    }

    fn cancel_process_handoff(
        &self,
        _entry_id: &str,
        request: &control::ProcessHandoffCancelRequest,
    ) -> Result<control::ProcessHandoffCancelInfo, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        state.process_handoff_cancellations.push(request.clone());
        record(&mut state, "cancel_process_handoff")?;
        state
            .cancelled_process_handoff_generations
            .push(request.expected_generation);
        state.resume.process_generation = request.expected_generation;
        state.resume_handoff_phase = "registered".to_string();
        Ok(control::ProcessHandoffCancelInfo {
            process_generation: request.expected_generation,
        })
    }

    fn health(&self) -> Result<DaemonHealth, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "health")?;
        Ok(DaemonHealth {
            control_api_version: control::CONTROL_API_VERSION,
            daemon_version: "0.0.0-mock".to_string(),
            uptime_seconds: 1,
            entries_open: state.entries.len() as u32,
            upload_ok: true,
        })
    }

    fn current_entry(&self) -> Result<Option<String>, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "current_entry")?;
        Ok(Some(state.info.entry_id.clone()))
    }

    fn abandon(&self, entry_id: &str, reason: &str) -> Result<(), ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "abandon")?;
        state
            .abandoned
            .push((entry_id.to_string(), reason.to_string()));
        Ok(())
    }

    fn create_capture_workspace(
        &self,
        request: &CaptureWorkspaceRequest,
    ) -> Result<CaptureWorkspaceInfo, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        state.capture.creates.push(request.clone());
        record(&mut state, "create_capture_workspace")?;
        Ok(state.capture.info.clone())
    }

    /// Like the daemon, a goal is taken once: every later call is a `conflict`, including
    /// one after a refused first call.
    fn record_capture_goal(
        &self,
        _entry_id: &str,
        request: &CaptureGoalRequest,
    ) -> Result<StreamKey, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        state.capture.goals.push(request.clone());
        record(&mut state, "record_capture_goal")?;
        if state.capture.goals.len() > 1 {
            return Err(ControlError::Conflict(
                "this entry's goal was already submitted, and a goal is recorded once".to_string(),
            ));
        }
        Ok(StreamKey {
            producer: MOCK_GOAL_PRODUCER.to_string(),
            stream: 1,
        })
    }

    fn record_capture_launch(
        &self,
        _entry_id: &str,
        request: &CaptureLaunchRequest,
    ) -> Result<(), ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        state.capture.launches.push(request.clone());
        record(&mut state, "record_capture_launch")
    }

    fn seal_capture(
        &self,
        _entry_id: &str,
        request: &CaptureSealRequest,
        _read_timeout: std::time::Duration,
    ) -> Result<CaptureFinishReport, CaptureSealError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(CaptureSealError::from_unclassified(poisoned()));
        };
        state.capture.seals.push(request.clone());
        record(&mut state, "seal_capture").map_err(CaptureSealError::from_unclassified)?;
        state.capture.finish.clone().ok_or_else(|| {
            CaptureSealError::from_unclassified(ControlError::Internal(
                "the mock has no local finish".to_string(),
            ))
        })
    }
}
