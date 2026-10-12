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
//! # The status line is a file, not a route
//!
//! [`write_entry_status_line_rooted`] writes the one plain line Claude's `statusLine` command
//! `cat`s. **It may not carry a token, a key, a seed or a path out of the participant's
//! home**: the status route is unauthenticated by design, and the status line is rendered into
//! a terminal a screen recording may catch.

use crate::DaemonError;
use crate::arming::{GateState, RefusalReason};
use crate::config::EventConfig;
use crate::registry::{Entry, EntryState};
use ed25519_dalek::SigningKey;
use goalcup_platform::tracecap::EntryArtifactRoot;
use goalcup_protocol::capture::{CaptureState, M2Status, RunState};
use goalcup_protocol::control::{
    CycleErrors, EntrySummary, EventStatus, EventStatusSource, SignedStatus, StatusPayload,
    WorkspaceFinalizationStatusV1,
};
use goalcup_protocol::events::HealthState;
use goalcup_protocol::provenance::TerminationOutcomeV1;

/// Everything the daemon knows about one entry at one moment. The daemon's own gathering
/// struct: it is never serialised and never reaches the wire.
pub struct StatusInputs<'a> {
    pub entry: &'a Entry,
    pub arming: GateState,
    pub health: HealthState,
    pub events_uploaded: u64,
    pub events_pending: u64,
    pub last_sync_at: Option<String>,
    pub now: f64,
    pub event: EventStatus,
    /// What the daemon's own four cadences lost. Gathered here rather than computed from
    /// the entry, because the failures belong to the loop around the entry and not to the
    /// run: the runtime is handed the daemon's counters at registration.
    pub cycles: CycleErrors,
    /// The three counters the tick keeps and nothing else states.
    pub polls_capped: u64,
    pub cursor_restore_failures: u64,
    pub skipped_replayed: u64,
    /// The first hole in the write-ahead buffer the last pump could not close.
    pub buffer_gap_at: Option<u64>,
    /// Polls that stopped at the seen-file bound with uncaptured files still in front of
    /// them. Its notice chains nothing, so this is the only place
    /// the fact reaches a reader.
    pub files_capped: u64,
    /// The seal could not vouch for the harness tree. Taken from the entry's
    /// `SealOutcome`, so it is `false` until there is a seal to read it off.
    pub harness_unverified: bool,
}

fn finalization_needs_harness_review(finalization: Option<&WorkspaceFinalizationStatusV1>) -> bool {
    match finalization {
        None | Some(WorkspaceFinalizationStatusV1::Finishing {}) => false,
        Some(WorkspaceFinalizationStatusV1::Incomplete { termination, .. }) => termination
            .as_ref()
            .is_some_and(|outcome| !matches!(outcome, TerminationOutcomeV1::Confirmed { .. })),
        Some(WorkspaceFinalizationStatusV1::Terminal { termination, .. }) => {
            !matches!(termination, TerminationOutcomeV1::Confirmed { .. })
        }
    }
}

/// The payload, from the entry and the moment. No I/O, no clock: `now` is an argument, so a
/// test can ask the same inputs about two moments.
///
/// # A refusal is `ArmingState::Refused`
///
/// Given a refused gate, the payload reports `ArmingState::Refused`, so a machine reader that
/// keys on `arming` alone can tell a run that is waiting to be armed from one that will never
/// arm; `health` reports `ArmingPending` and `detail` says which refusal it was, in words a
/// participant can act on. No current producer builds `GateState::Refused`: the V1 arming
/// poll that did was removed by D48, and the capture runtime never refuses a gate. The
/// mapping stays because the status type still carries the variant. `GateState::to_control`
/// is total, so there is no fallback here to
/// hide a future `GateState` variant behind — an unmapped variant is a compile error.
pub fn build_payload(inputs: &StatusInputs) -> StatusPayload {
    let arming = inputs.arming.to_control();
    let finalization = inputs.entry.workspace_finalization_status();
    let finalization_needs_review = finalization_needs_harness_review(finalization.as_ref());
    // A sealed entry reports `sealed`, and its countdown stops.
    //
    // The seal is the chain's last link, so the entry's own row is the authority and this
    // is read off it rather than off `inputs.health` — which is the last state the tick
    // evaluated, and the tick stops at the seal.
    let sealed_at = inputs.entry.sealed_at.clone();
    // `Sealing` is deliberately neither open nor sealed: an interrupted intent has closed
    // capture, but until recovery validates and promotes its one terminal link it cannot
    // honestly claim the run is sealed.  Its status is the durable quarantine rather than
    // the stale live health/counted-down deadline the last tick happened to publish.
    let sealing_recovery_needed = inputs.entry.state == EntryState::Sealing && sealed_at.is_none();
    let terminal_drain_needs_review = inputs.entry.terminal_drain_needs_review();
    let capture_closed_recovery_needed =
        sealing_recovery_needed || terminal_drain_needs_review || finalization_needs_review;
    let health = match (&sealed_at, capture_closed_recovery_needed) {
        (_, true) => HealthState::SealingRecoveryNeeded,
        (Some(_), false) => HealthState::Sealed,
        (None, false) => inputs.health,
    };
    let seconds_remaining = match (&sealed_at, capture_closed_recovery_needed) {
        // Not `None`: a reader that has to tell "no deadline" from "the deadline no longer
        // matters" gets two different answers, and every surface already renders `Some(0)`
        // as time being up.
        (Some(_), _) | (None, true) => Some(0),
        (None, false) => inputs
            .entry
            .deadline_unix
            .map(|d| (d - inputs.now).max(0.0) as u64),
    };
    StatusPayload {
        entry_id: inputs.entry.entry_id.clone(),
        connector_id: inputs.entry.connector_id.clone(),
        workspace: inputs.entry.workspace.to_string_lossy().to_string(),
        session_id: inputs.entry.session_id.clone(),
        // `None` where nothing measured it, and never `Some("")`. The registry row's field is a
        // `String` because the version-drift event on it is a pair of strings; the document's is
        // an `Option`, and an empty string in the `Some` arm is a claim that a version was read
        // whose value happens to be nothing.
        harness_version: Some(inputs.entry.harness_version.clone()).filter(|v| !v.is_empty()),
        arming,
        health,
        started_at: inputs.entry.started_at.clone(),
        deadline_at: inputs.entry.deadline_at.clone(),
        sealed_at: sealed_at.clone(),
        seconds_remaining,
        last_sync_at: inputs.last_sync_at.clone(),
        events_uploaded: inputs.events_uploaded,
        // A quarantine means the ordinary byte/count view cannot prove completion. The
        // buffer can be behind a registry terminal mirror (or the registry can be beyond
        // terminal), both of which otherwise render zero here. Keep a real pending count
        // when there is one, but never publish elevated review as a completed all-clear.
        events_pending: if terminal_drain_needs_review {
            inputs.events_pending.max(1)
        } else {
            inputs.events_pending
        },
        // The signed field stays; no connector has a human arming step (the first transcript
        // record arms every entry), so it is always `None`.
        arming_recipe: None,
        detail: if terminal_drain_needs_review {
            "Capture is closed: terminal upload validation failed. This entry requires elevated \
             review and must not be treated as fully acknowledged."
                .to_string()
        } else if finalization_needs_review {
            "Capture is closed: termination could not be confirmed. This entry requires elevated \
             review and must not be treated as fully acknowledged."
                .to_string()
        } else {
            plain_words(health, inputs.arming, inputs.entry.started_at.is_some())
        },
        event_status_source: source_of(inputs.entry, &inputs.event),
        event: inputs.event.clone(),
        cycles: inputs.cycles.clone(),
        // Hook-post counters. The payload fields are signed wire and stay; with no hook
        // receiver in the daemon nothing is ever dropped, so both are zero.
        posts_dropped: 0,
        posts_dropped_after_seal: 0,
        // D24's reported-pin counter: signed wire that stays, with no hook leg to feed it.
        pins_dropped: 0,
        polls_capped: inputs.polls_capped,
        cursor_restore_failures: inputs.cursor_restore_failures,
        skipped_replayed: inputs.skipped_replayed,
        buffer_gap_at: inputs.buffer_gap_at,
        files_capped: inputs.files_capped,
        harness_unverified: inputs.harness_unverified || finalization_needs_review,
        finalization,
        // The capture projection; this builder leaves it unset.
        capture: None,
        producers_consumed: None,
        admissions_refused: None,
    }
}

/// Project a v5 entry's capture state onto a payload before it is signed. `Some` also
/// replaces `detail`, so no surface that reads only that sentence can show a legacy health
/// sentence (uploads, a countdown) for a local capture. Pure: no I/O and no clock.
pub(crate) fn apply_capture_snapshot(payload: &mut StatusPayload, capture: Option<M2Status>) {
    if let Some(snapshot) = &capture {
        payload.detail = capture_words(snapshot).to_string();
    }
    payload.capture = capture;
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
/// **What each field means here, and why none of it is a claim about a round.** `t0` and
/// `start_window_closes_at` are the moment the entry's row was created: the entry could not
/// have started before it existed, and it started as soon as it was allowed to, so its own
/// creation is the only window this daemon can measure without being told one.
/// `publication_closes_at` is the entry's `deadline_at` — the number the start receipt
/// fixed, which is a real bound rather than a stand-in — and `created_at` again only before
/// a receipt has fixed one. `rules_version` is this build's version, which says which binary
/// wrote the document and nothing about which rulebook the run is judged under.
///
/// A reader is not asked to work any of that out: [`StatusPayload::event_status_source`] is
/// `fallback` on exactly the documents this built.
///
/// **This is the one definition.** The V1 per-entry runtime held a copy of it, and D48
/// removed that runtime with its tests, so every caller left is in this module.
pub(crate) fn fallback_event_status(entry: &Entry) -> EventStatus {
    EventStatus {
        t0: entry.created_at.clone(),
        start_window_closes_at: entry.created_at.clone(),
        publication_closes_at: entry
            .deadline_at
            .clone()
            .unwrap_or_else(|| entry.created_at.clone()),
        rules_version: crate::DAEMON_VERSION.to_string(),
    }
}

/// The event status one entry should carry, given three configured fields from anywhere.
///
/// `publication_closes_at` is taken from the entry in every case: it is the deadline the
/// receipt fixed, and that is immutable. A caller that restated a stored
/// `publication_closes_at` verbatim would regress it to `created_at`, because `deadline_at`
/// is `None` until a start receipt sets it, which is later than the round is resolved.
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
/// of the entry — `publication_closes_at` is deliberately not compared, because the deadline
/// moves when the receipt fixes it and the runtime's copy is taken at construction.
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

/// Write a participant status line through the runtime's retained entry-artifact
/// capability. The lexical path remains only a diagnostic; no write follows it after entry
/// acquisition.
pub fn write_entry_status_line_rooted(
    signed: &SignedStatus,
    artifact: &EntryArtifactRoot,
) -> Result<(), DaemonError> {
    artifact
        .replace_status_line(status_line_bytes(signed).as_bytes())
        .map_err(|error| {
            DaemonError::Refused(format!(
                "status line cannot be replaced through retained entry artifact authority: {error}"
            ))
        })
}

/// The status line is a file. One line of plain text, no token, no grant,
/// no key and no path — the harness `cat`s it into a surface the participant sees and the
/// agent does not (D25).
///
/// The workspace path is deliberately **not** in it: the line is rendered in a terminal
/// the participant may be recording, and it would say nothing the participant does not
/// already know.
fn status_line_bytes(signed: &SignedStatus) -> String {
    let p = &signed.payload;
    let remaining = match p.seconds_remaining {
        Some(0) => "time is up".to_string(),
        // The last 59 seconds are not "0h 0m left": that reads as a rounding artefact
        // beside a run that genuinely has forty minutes, and this is the minute a
        // participant most needs told apart from the others.
        Some(s) if s < 60 => "<1m left".to_string(),
        Some(s) => format!("{}h {}m left", s / 3_600, (s % 3_600) / 60),
        None => "not started".to_string(),
    };
    // One line: the detail is one sentence and carries no newline, and the format string
    // ends with the only `\n` in the file.
    format!(
        "/goal Cup — {} — {remaining}\n",
        p.detail.replace(['\n', '\r'], " ")
    )
}

/// The row `goalcup entries` prints and `GET /v0/entries` returns.
///
/// A refused gate would report `ArmingState::Refused` here, for the same reason as
/// [`build_payload`] (which also says why no current producer refuses one).
pub fn summary(entry: &Entry, arming: GateState, health: HealthState) -> EntrySummary {
    let finalization_needs_review =
        finalization_needs_harness_review(entry.workspace_finalization_status().as_ref());
    EntrySummary {
        entry_id: entry.entry_id.clone(),
        connector_id: entry.connector_id.clone(),
        workspace: entry.workspace.to_string_lossy().to_string(),
        arming: arming.to_control(),
        health: if entry.terminal_drain_needs_review() || finalization_needs_review {
            HealthState::SealingRecoveryNeeded
        } else {
            health
        },
        started_at: entry.started_at.clone(),
        deadline_at: entry.deadline_at.clone(),
        sealed_at: entry.sealed_at.clone(),
    }
}

/// The health state in plain words. No enum names, no jargon, and every line says what — if
/// anything — the participant should do about it.
///
/// `started` is `Entry::started_at.is_some()`, the entry's own row and the same question
/// the V1 drain eviction asked before D48 removed it. Reading `arming != Started` instead
/// would disagree on exactly one entry — one that started, sealed, and whose gate was
/// rebuilt `pending` by a daemon restart — and that entry would read "sealed, never
/// started — nothing to upload" over a run with a chain full of records. `started_at` is
/// the durable fact: the start receipt stamped it, and no restart forgets it. The gate is
/// runtime state, rebuilt from what the tick has seen since.
pub fn plain_words(health: HealthState, arming: GateState, started: bool) -> String {
    match health {
        HealthState::Online => "Recording. Everything is being captured and synced.".to_string(),
        HealthState::UploadLagging => {
            "Recording. The upload is behind, which is normal on a slow link; nothing is lost."
                .to_string()
        }
        HealthState::ConnectionLost => {
            "Recording locally. The tracker cannot reach the server; everything is buffered and \
             will sync when it comes back."
                .to_string()
        }
        HealthState::AgentExited => {
            "Your agent is no longer running. Run `goalcup seal` to finish this entry.".to_string()
        }
        HealthState::TrackerInterrupted => {
            "The tracker restarted. It has picked up where it left off.".to_string()
        }
        HealthState::CaptureStalled => {
            "Your agent is running but has written nothing for a while. If it looks stuck, run \
             `goalcup doctor`."
                .to_string()
        }
        HealthState::HooksSilent => {
            "The tracker has stopped receiving events from your harness while its transcript \
             keeps growing. Run `goalcup doctor`."
                .to_string()
        }
        HealthState::ConfigRestored => {
            "A tracker file in the workspace changed and was put back. The change is recorded."
                .to_string()
        }
        HealthState::DeadlineReached => "Time is up. This entry is being sealed.".to_string(),
        // The second sentence is the one difference that matters to a participant reading
        // it: a run that never started has nothing on the server, and saying so is what
        // tells "finished" from "finished with nothing in it".
        //
        // Read off `started_at` and not off the gate: see [`plain_words`].
        HealthState::Sealed => {
            if started {
                "This entry is sealed. Nothing more will be recorded, and what was recorded \
                 is on disk beside the chain."
                    .to_string()
            } else {
                "sealed, never started — nothing to upload".to_string()
            }
        }
        HealthState::SealingRecoveryNeeded => {
            "Capture is closed while the tracker verifies an interrupted seal. This entry is \
             not yet claimed as sealed; inspect or restart the tracker before continuing."
                .to_string()
        }
        HealthState::ArmingPending => match arming {
            GateState::Refused(RefusalReason::ArmingTimedOut) => {
                "The tracker saw your harness start but no transcript followed, so nothing was \
                 recorded. Run `goalcup start` again and open a NEW session in the folder it \
                 creates."
                    .to_string()
            }
            GateState::Refused(RefusalReason::StartSignalMissing) => {
                "The tracker never saw your harness start in this folder, so nothing was \
                 recorded. Run `goalcup start` again and open a NEW session in the folder it \
                 creates."
                    .to_string()
            }
            _ => "Not recording yet. Open your harness in the folder the tracker created; the \
                  clock starts when you type your goal."
                .to_string(),
        },
        HealthState::ArmingLost => {
            "A hook file was put back, and on this harness that cancels its trust. Re-arm before \
             continuing, or the rest of the run will not be captured."
                .to_string()
        }
        HealthState::VersionChanged => {
            "Your harness updated itself mid-run. That is recorded and is not a violation."
                .to_string()
        }
        HealthState::ConfigMismatch => {
            "Your harness is running with a different hook configuration than the tracker wrote. \
             Capture may be incomplete. Run `goalcup doctor`."
                .to_string()
        }
        // The participant may well have done nothing — a harness that rotates its own
        // transcript looks identical from here — so the words say what was seen and what
        // follows, not what was done.
        HealthState::StoreReplaced => {
            "One of the files your harness writes was replaced or shortened. The tracker recorded \
             that and is still capturing; a reviewer will see it in your trace."
                .to_string()
        }
        // Said while there is still room to act on it.
        HealthState::DiskPressure => {
            "This disk is nearly full. The tracker is still capturing, but it will stop when the \
             disk runs out. Free some space."
                .to_string()
        }
    }
}
