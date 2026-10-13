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
//! `WorkspaceInfo.run_public_key`; the `public_key` field on the document is a label, not
//! a credential, and nothing in this crate verifies against it.
//!
//! Every document is a v5 local capture's: the upload, deadline and hook fields the wire
//! shape still carries are reported empty or zero, and `detail` is the capture's sentence.

use crate::DaemonError;
use crate::config::EventConfig;
use crate::registry::{Entry, EntryState};
use ed25519_dalek::SigningKey;
use goalcup_protocol::capture::{CaptureState, M2Status, RunState};
use goalcup_protocol::control::{
    ArmingState, CycleErrors, EntrySummary, EventStatus, EventStatusSource, SignedStatus,
    StatusPayload,
};
use goalcup_protocol::events::HealthState;

/// Everything the daemon knows about one entry at one moment. The daemon's own gathering
/// struct: it is never serialised and never reaches the wire.
pub struct StatusInputs<'a> {
    pub entry: &'a Entry,
    pub arming: ArmingState,
    pub event: EventStatus,
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

/// The payload, from the entry and the moment. No I/O, no clock.
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
        event_status_source: source_of(entry, &inputs.event),
        event: inputs.event.clone(),
        cycles: inputs.cycles.clone(),
        polls_capped: 0,
        cursor_restore_failures: 0,
        skipped_replayed: 0,
        buffer_gap_at: None,
        files_capped: 0,
        harness_unverified: false,
        finalization: None,
        capture: Some(inputs.capture.clone()),
        // The receiver's admission counts; the caller sets them when it holds them.
        producers_consumed: None,
        admissions_refused: None,
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
            "Local capture is recording to this machine only: nothing is uploaded, and no \
             server has signed a start or a deadline."
        }
        (_, CaptureState::Incomplete) => {
            "Local capture is recording with losses: some observations were not retained, and \
             each loss is chained as a gap."
        }
    }
}

/// The event timeline an entry falls back to when no round was configured.
///
/// **What each field means here, and why none of it is a claim about a round.** `t0`,
/// `start_window_closes_at` and `publication_closes_at` are the moment the entry's row was
/// created: the entry could not have started before it existed, and a local capture has no
/// signed deadline, so its own creation is the only window this daemon can measure without
/// being told one. `rules_version` is this build's version, which says which binary wrote the
/// document and nothing about which rulebook the run is judged under.
///
/// A reader is not asked to work any of that out: [`StatusPayload::event_status_source`] is
/// `fallback` on exactly the documents this built.
pub(crate) fn fallback_event_status(entry: &Entry) -> EventStatus {
    EventStatus {
        t0: entry.created_at.clone(),
        start_window_closes_at: entry.created_at.clone(),
        publication_closes_at: entry.created_at.clone(),
        rules_version: crate::DAEMON_VERSION.to_string(),
    }
}

/// The event status one entry should carry, given three configured fields from anywhere.
///
/// `publication_closes_at` is the fallback's in every case: no configured value replaces it.
pub(crate) fn event_status_from(entry: &Entry, configured: Option<&EventStatus>) -> EventStatus {
    let fallback = fallback_event_status(entry);
    match configured {
        None => fallback,
        Some(event) => EventStatus {
            t0: event.t0.clone(),
            start_window_closes_at: event.start_window_closes_at.clone(),
            publication_closes_at: fallback.publication_closes_at,
            rules_version: event.rules_version.clone(),
        },
    }
}

/// The same, from a `[event]` config section. The two callers that hold a `DaemonConfig`
/// use this; the terminal composers use [`event_status_from`] with the entry's own field.
pub(crate) fn event_status_for(entry: &Entry, configured: Option<&EventConfig>) -> EventStatus {
    let as_status = configured.map(|event| EventStatus {
        t0: event.t0.clone(),
        start_window_closes_at: event.start_window_closes_at.clone(),
        publication_closes_at: String::new(),
        rules_version: event.rules_version.clone(),
    });
    event_status_from(entry, as_status.as_ref())
}

/// Whether the three configurable fields of `event` are the entry's own timeline.
///
/// Derived rather than carried. `EventStatus` is the only channel between the construction
/// site that knows the answer and the document, and it is a `goalcup-protocol` struct built
/// by the literals in this module; a fifth field on it would be a field those literals do not
/// set. So the three fields `[event]` supplies are compared against what the fallback would
/// have put there, on `created_at` and `DAEMON_VERSION`, both of which are fixed for the life
/// of the entry — `publication_closes_at` is not compared, because `[event]` does not set it.
///
/// The one inaccuracy is benign and worth naming: a `[event]` section that declares exactly
/// the entry's creation instant and this build's version reads as `fallback`. The numbers in
/// the document are then identical either way, so nothing a reader does with it changes.
fn source_of(entry: &Entry, event: &EventStatus) -> EventStatusSource {
    let fallback = fallback_event_status(entry);
    let same = event.t0 == fallback.t0
        && event.start_window_closes_at == fallback.start_window_closes_at
        && event.rules_version == fallback.rules_version;
    match same {
        true => EventStatusSource::Fallback,
        false => EventStatusSource::Config,
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
