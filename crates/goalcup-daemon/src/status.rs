//! Building and signing the status document (D25).
//!
//! The document itself is `goalcup_protocol::control::{StatusPayload, SignedStatus}`;
//! **this module defines no second shape**, because a CLI that verified a signature over
//! a document the daemon never promised would disagree with it only at run time.
//!
//! # One signed document, served by the daemon that holds the registry (D25)
//!
//! Signed with the entry's own run key, so any reader, the CLI included, can check it.
//! There is no copy on disk: `goalcup status` starts the run-scoped daemon when none answers
//! and asks it, so the registry is the one source of truth, sealed runs included.
//! `SignedStatus::verify_with` takes the key the **caller** pinned from
//! `CaptureWorkspaceInfo.run_public_key`; the `public_key` field on the document is a label, not
//! a credential, and nothing in this crate verifies against it.
//!
//! Every document is a v5 local capture's: the upload, deadline and hook fields the wire
//! shape still carries are reported empty or zero, and `detail` is the capture's sentence.

use crate::DaemonError;
use crate::registry::{Entry, EntryState};
use ed25519_dalek::SigningKey;
use goalcup_protocol::capture::{CaptureState, M2Status, RunState};
use goalcup_protocol::control::{
    ArmingState, CycleErrors, EntrySummary, SignedStatus, StatusPayload,
};
use goalcup_protocol::events::HealthState;

/// Everything the daemon knows about one entry at one moment. The daemon's own gathering
/// struct: it is never serialised and never reaches the wire.
pub struct StatusInputs<'a> {
    pub entry: &'a Entry,
    pub arming: ArmingState,
    /// What the daemon's own cadences lost. Gathered here rather than computed from
    /// the entry, because the failures belong to the loop around the entry and not to the
    /// run: the runtime is handed the daemon's counters at registration.
    pub cycles: CycleErrors,
    /// The capture's state, which also supplies `detail`.
    pub capture: M2Status,
}

/// Whether the row's own durable state says capture is closed without a sealed terminal it
/// can vouch for: an interrupted `Sealing` row, or a terminal drain that needs review.
fn recovery_needed(entry: &Entry) -> bool {
    // `Sealing` is deliberately neither open nor sealed: an interrupted intent has closed
    // capture, but until recovery validates and promotes its one terminal link it cannot
    // honestly claim the run is sealed.
    (entry.state == EntryState::Sealing && entry.sealed_at.is_none())
        || entry.terminal_drain_needs_review()
}

/// The payload, from the entry and the capture snapshot. No I/O, no clock.
///
/// A sealed entry reports `sealed`: the seal is the chain's last link, so the entry's own row
/// is the authority. A local capture has no signed deadline, so `seconds_remaining` is `None`
/// before and after its finish. `detail` is the capture's sentence, so no surface that reads
/// only that sentence can show a legacy health sentence (uploads, a countdown).
pub fn build_payload(inputs: &StatusInputs) -> StatusPayload {
    let entry = inputs.entry;
    let health = match (&entry.sealed_at, recovery_needed(entry)) {
        (_, true) => HealthState::SealingRecoveryNeeded,
        (Some(_), false) => HealthState::Sealed,
        (None, false) => HealthState::Online,
    };
    StatusPayload {
        entry_id: entry.entry_id.clone(),
        connector_id: entry.connector_id.clone(),
        workspace: entry.workspace.to_string_lossy().to_string(),
        session_id: None,
        // `None` where nothing measured it, and never `Some("")`. The registry row's field is a
        // `String`; the document's is an `Option`, and an empty string in the `Some` arm is a
        // claim that a version was read whose value happens to be nothing.
        harness_version: Some(entry.harness_version.clone()).filter(|v| !v.is_empty()),
        arming: inputs.arming,
        health,
        started_at: None,
        deadline_at: None,
        sealed_at: entry.sealed_at.clone(),
        seconds_remaining: None,
        last_sync_at: None,
        events_uploaded: 0,
        // A quarantine means the ordinary count view cannot prove completion, so it is never
        // published as a completed all-clear.
        events_pending: u64::from(entry.terminal_drain_needs_review()),
        detail: capture_words(&inputs.capture).to_string(),
        cycles: inputs.cycles.clone(),
        polls_capped: 0,
        cursor_restore_failures: 0,
        skipped_replayed: 0,
        buffer_gap_at: None,
        files_capped: 0,
        harness_unverified: false,
        capture: Some(inputs.capture.clone()),
        // The receiver's admission counts; the caller sets them when it holds them.
        producers_consumed: None,
        admissions_refused: None,
        native_files_admitted: None,
    }
}

/// One sentence per capture state, in the words a participant acts on. A failure is named
/// first: a finish whose seal was not written is never called finished.
fn capture_words(snapshot: &M2Status) -> &'static str {
    match (snapshot.run, snapshot.capture) {
        (_, CaptureState::Failed) => {
            "Local capture failed: this entry's chain accepts no more records. The harness is \
             not stopped by it."
        }
        (RunState::Sealed, _) => "Local capture is finished: the chain is closed on this machine.",
        (_, CaptureState::Starting) => {
            "Local capture is starting: the chain is reserved on this machine and the harness \
             has not been launched."
        }
        (_, CaptureState::Recording) => {
            "Local capture is recording: the chain is written on this machine."
        }
        (_, CaptureState::Incomplete) => {
            "Local capture is recording with losses: some observations were not retained, and \
             each loss is chained as a gap."
        }
    }
}

/// Sign with this entry's per-run key. There is no machine-wide status document, because
/// a per-run key cannot honestly sign another run's facts.
pub fn sign_payload(
    payload: StatusPayload,
    cached_at: String,
    key: &SigningKey,
) -> Result<SignedStatus, DaemonError> {
    Ok(SignedStatus::sign(payload, cached_at, key)?)
}

/// The row `goalcup entries` prints and `GET /v0/entries` returns.
pub fn summary(entry: &Entry, arming: ArmingState, health: HealthState) -> EntrySummary {
    EntrySummary {
        entry_id: entry.entry_id.clone(),
        connector_id: entry.connector_id.clone(),
        workspace: entry.workspace.to_string_lossy().to_string(),
        arming,
        health: if entry.terminal_drain_needs_review() {
            HealthState::SealingRecoveryNeeded
        } else {
            health
        },
        started_at: None,
        deadline_at: None,
        sealed_at: entry.sealed_at.clone(),
    }
}
