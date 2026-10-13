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
//! test can pin `WorkspaceInfo.run_public_key` and mean it, and it can
//! answer one `busy` on a named method — the transient the daemon raises on every mutating
//! route while an entry is mid-pump.
//!
//! # What it does not model: headers
//!
//! It is in-process. It has no `Authorization` header to inspect and claims none, so the
//! bearer/public split is asserted on the wire, against `HttpDaemonClient` and a real
//! socket, in `tests/it/daemon_client.rs`. A mock that pretended to hold a bearer would be a
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
    ControlError, DaemonClient, DaemonHealth, EntrySummary, EventStatus, LocalSealOutcome,
    SealResponse, SignedStatus, StatusPayload, WorkspaceInfo,
};
use goalcup_protocol::events::HealthState;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use goalcup_protocol::provenance::{Rfc3339Utc, TerminationOutcomeV1};
use goalcup_protocol::seal::{SealManifest, SealedFile};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

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
    pub status: SignedStatus,
    pub entries: Vec<EntrySummary>,
    pub seal: SealManifest,
    pub sealed: bool,
    pub processes: Vec<ProcessHandle>,
    pub abandoned: Vec<(String, String)>,
    pub calls: Vec<String>,
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
/// protocol's own `SignedStatus::sign` produces the same alphabet, which is what makes the
/// key the mock puts on `WorkspaceInfo.run_public_key` the one its documents verify with.
fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
            // Empty, as `goalcup start` writes it for a capture entry: the V1 page is gone.
            status_url: String::new(),
            status_cache_path: artifact_root.join("status.json").display().to_string(),
            status_line_path: artifact_root.join("status-line.txt").display().to_string(),
            session_id: "session-mock-0001".to_string(),
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
            seal: SealManifest {
                entry_id: entry_id.clone(),
                sealed_at: NOW.to_string(),
                workspace: info.workspace.clone(),
                files: Vec::new(),
                excluded: Vec::new(),
            },
            sealed: false,
            processes: Vec::new(),
            abandoned: Vec::new(),
            calls: Vec::new(),
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
                    // The mock chains nothing; a fixed head and no trace directory.
                    chain_head: "ab".repeat(32),
                    trace_dir: String::new(),
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

    /// The trait methods called on this mock, in order.
    pub fn calls(&self) -> Vec<String> {
        self.read(|state| state.calls.clone()).unwrap_or_default()
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
        created_at: said,
    }
}

// ------------------------------------------------------------------ the hooks

impl MockDaemonClient {
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

    /// Answer one `busy` on this method and then succeed — the transient the daemon raises
    /// on every mutating route while an entry is mid-pump.
    pub fn busy_once(&self, method: &str) {
        self.fail_next(
            method,
            ControlError::Busy("this entry is held by a call already running".to_string()),
        );
    }

    /// The pid of the handle most recently `register_process`ed, so a test can assert the
    /// relaunch was handed over and the entry cache followed it.
    pub fn last_registered_pid(&self) -> Option<u32> {
        self.read(|state| state.processes.last().map(|h| h.pid))
            .flatten()
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
        ControlError::PreflightFailed(s) => ControlError::PreflightFailed(s.clone()),
        ControlError::Busy(s) => ControlError::Busy(s.clone()),
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

impl DaemonClient for MockDaemonClient {
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

    /// A second seal is a `conflict` — already sealed is a state that will not change, and
    /// a conflict is the one `409` a client must never retry.
    ///
    /// **The sealed entry leaves this mock's list**, as a V1 daemon's live map evicted it,
    /// while `status` keeps answering for it. The capture daemon lists sealed rows too; the
    /// mock keeps the eviction so `goalcup entries` is shown pointing a reader at `goalcup
    /// status <id>` either way.
    fn seal(&self, entry_id: &str) -> Result<SealResponse, ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "seal")?;
        if state.sealed {
            return Err(ControlError::Conflict(format!(
                "{} is already sealed",
                state.info.entry_id
            )));
        }
        state.sealed = true;
        state.entries.retain(|e| e.entry_id != entry_id);
        Ok(SealResponse::Legacy(state.seal.clone()))
    }

    fn register_process(
        &self,
        _entry_id: &str,
        registration: &ProcessRegistration,
    ) -> Result<(), ControlError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(poisoned());
        };
        record(&mut state, "register_process")?;
        state.processes.push(registration.handle.clone());
        Ok(())
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
