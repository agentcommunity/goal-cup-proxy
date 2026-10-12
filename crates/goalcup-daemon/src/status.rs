//! Building, signing and mirroring the status document (D25).
//!
//! The document itself is `goalcup_protocol::control::{StatusPayload, SignedStatus}`;
//! **this module defines no second shape**, because a CLI that verified a signature over
//! a document the daemon never promised would disagree with it only at run time.
//!
//! # The cache is the evidence, the page is a view (D25)
//!
//! One signed document, signed with the entry's own run key, verifiable by any reader —
//! the CLI included — without asking the daemon anything.
//! `SignedStatus::verify_with` takes the key the **caller** pinned from
//! `WorkspaceInfo.run_public_key`; the `public_key` field on the document is a label, not
//! a credential, and nothing in this crate verifies against it.
//!
//! # Two of the control contract's obligations are files, not routes
//!
//! [`mirror`] writes the two cache copies a reader can open without a socket;
//! [`write_entry_status_line_rooted`] writes the one plain line Claude's `statusLine` command `cat`s. **Neither may carry a token, a key,
//! a seed or a path out of the participant's home**: the status route is unauthenticated by
//! design, and the status line is rendered into a terminal a screen recording may catch.

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
#[cfg(any(test, feature = "test-hooks"))]
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
#[cfg(any(test, feature = "test-hooks"))]
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

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

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StatusFaultCounts {
    pub mirror_attempts: u64,
    pub mirror_failures: u64,
    pub latest_attempts: u64,
    pub latest_failures: u64,
    pub status_line_attempts: u64,
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default)]
struct StatusFaultState {
    mirror_failures: BTreeSet<PathBuf>,
    latest_failures: BTreeSet<PathBuf>,
    mirror_counts: BTreeMap<PathBuf, StatusFaultCounts>,
    status_line_attempts: BTreeMap<PathBuf, u64>,
}

#[cfg(any(test, feature = "test-hooks"))]
fn status_fault_state() -> &'static Mutex<StatusFaultState> {
    static STATE: OnceLock<Mutex<StatusFaultState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(StatusFaultState::default()))
}

#[cfg(any(test, feature = "test-hooks"))]
fn take_mirror_failure(config_dir: &Path) -> bool {
    let mut state = status_fault_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let failed = state.mirror_failures.remove(config_dir);
    let counts = state
        .mirror_counts
        .entry(config_dir.to_path_buf())
        .or_default();
    counts.mirror_attempts += 1;
    if failed {
        counts.mirror_failures += 1;
    }
    failed
}

#[cfg(any(test, feature = "test-hooks"))]
fn take_latest_failure(config_dir: &Path) -> bool {
    let mut state = status_fault_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let failed = state.latest_failures.remove(config_dir);
    let counts = state
        .mirror_counts
        .entry(config_dir.to_path_buf())
        .or_default();
    counts.latest_attempts += 1;
    if failed {
        counts.latest_failures += 1;
    }
    failed
}

#[cfg(any(test, feature = "test-hooks"))]
fn count_status_line_attempt(path: &Path) {
    *status_fault_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status_line_attempts
        .entry(path.to_path_buf())
        .or_default() += 1;
}

/// The status cache is a **file**, at
/// `<config_dir>/cache/status-<entry_id>.json` and `<config_dir>/cache/status-latest.json`,
/// both 0644, so a reader asking the time needs no socket and cannot be pointed at a
/// network endpoint.
///
/// The write is atomic — temp file, then rename — because a reader that opens the cache
/// while it is being replaced must see the previous document whole rather than half of
/// the next one, and half a signed document does not verify.
///
/// `status-latest.json` is the pointer, and it is the **most recently started** run rather
/// than the last writer. `Daemon::refresh_statuses` walks the entries in id order, so an
/// unconditional write would make "latest" mean "the highest id", and a participant who
/// abandoned `entry-0007` and started `entry-0003`'s workspace afterwards would get the
/// older run every time they asked. [`ranks_above`] decides, on `started_at` first and on
/// the id as the creation-order tiebreak, so the answer does not depend on the order the
/// refresh happened to visit the map in.
pub fn mirror(signed: &SignedStatus, config_dir: &Path) -> Result<(), DaemonError> {
    mirror_entry(signed, config_dir)?;
    mirror_latest(signed, config_dir)
}

/// Replace only this entry's offline document. Kept separate from the shared latest pointer
/// so terminal review can attempt both sinks even when either filesystem operation fails.
pub fn mirror_entry(signed: &SignedStatus, config_dir: &Path) -> Result<(), DaemonError> {
    #[cfg(any(test, feature = "test-hooks"))]
    if take_mirror_failure(config_dir) {
        return Err(DaemonError::Refused(
            "test-hooks: status mirror publication failed".to_string(),
        ));
    }
    let entry_id = &signed.payload.entry_id;
    // The id becomes a file name. It is minted by the daemon, so this refuses rather than
    // sanitises: a caller that produced a traversing id has a bug worth seeing.
    if entry_id.is_empty()
        || entry_id.contains('/')
        || entry_id.contains('\\')
        || entry_id.contains("..")
    {
        return Err(DaemonError::Refused(format!(
            "entry id {entry_id:?} is not a file name, and the status cache is a file"
        )));
    }
    let bytes = serde_json::to_vec_pretty(signed)?;
    let cache = config_dir.join("cache");
    write_atomic(
        &cache.join(format!("status-{entry_id}.json")),
        &bytes,
        0o644,
    )
}

/// Replace the shared latest pointer if this status owns its rank. This is a separate
/// fallible view: a terminal review attempt must not skip it merely because its per-entry
/// document could not be replaced in the same pass.
pub fn mirror_latest(signed: &SignedStatus, config_dir: &Path) -> Result<(), DaemonError> {
    let _latest_guard = latest_view_lock();
    mirror_latest_locked(signed, config_dir)
}

/// The ranked write half of [`mirror_latest`], for a caller which already owns the one
/// process-local latest lock. Keeping this private prevents terminal replacement from
/// releasing its compare/delete interval only to recursively deadlock when it writes review.
fn mirror_latest_locked(signed: &SignedStatus, config_dir: &Path) -> Result<(), DaemonError> {
    let bytes = serde_json::to_vec_pretty(signed)?;
    let cache = config_dir.join("cache");
    let latest = cache.join("status-latest.json");
    match ranks_above(signed, &latest) {
        true => {
            #[cfg(any(test, feature = "test-hooks"))]
            if take_latest_failure(config_dir) {
                return Err(DaemonError::Refused(
                    "test-hooks: status latest publication failed".to_string(),
                ));
            }
            write_atomic(&latest, &bytes, 0o644)
        }
        false => Ok(()),
    }
}

fn validate_status_entry_id(entry_id: &str) -> Result<(), DaemonError> {
    let id_path = Path::new(entry_id);
    if !matches!(
        (id_path.components().count(), id_path.components().next()),
        (1, Some(std::path::Component::Normal(_)))
    ) {
        return Err(DaemonError::Refused(format!(
            "entry id {entry_id:?} is not one normal path component"
        )));
    }
    Ok(())
}

/// Write a participant status line through the runtime's retained entry-artifact
/// capability. The lexical path remains only a diagnostic/test-hook key; no write follows
/// it after entry acquisition.
pub fn write_entry_status_line_rooted(
    signed: &SignedStatus,
    artifact: &EntryArtifactRoot,
) -> Result<(), DaemonError> {
    #[cfg(any(test, feature = "test-hooks"))]
    count_status_line_attempt(&artifact.display_path().join("status-line.txt"));
    artifact
        .replace_status_line(status_line_bytes(signed).as_bytes())
        .map_err(|error| {
            DaemonError::Refused(format!(
                "status line cannot be replaced through retained entry artifact authority: {error}"
            ))
        })
}

/// One serialized ownership boundary for the process-wide latest pointer. Do not call a
/// function that takes this lock from inside another holder: terminal replacement retains
/// one guard across invalidation and its following ranked write through private `_locked`
/// helpers.
fn latest_view_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Remove the shared latest pointer only if it still names this exact entry. Used by abandon
/// as well as terminal invalidation, so all in-process readers/comparers/removers share the
/// same lock with ordinary ranked writers.
pub(crate) fn remove_latest_if_owned(entry_id: &str, config_dir: &Path) -> Result<(), DaemonError> {
    validate_status_entry_id(entry_id)?;
    let latest = config_dir.join("cache").join("status-latest.json");
    let _latest_guard = latest_view_lock();
    let owns_latest = std::fs::read_to_string(&latest)
        .ok()
        .and_then(|text| serde_json::from_str::<SignedStatus>(&text).ok())
        .is_some_and(|signed| signed.payload.entry_id == entry_id);
    if owns_latest {
        remove_view_if_present(&latest)?;
    }
    Ok(())
}

fn remove_view_if_present(path: &Path) -> Result<(), DaemonError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(DaemonError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Does `signed` belong in `status-latest.json`, given what is there now?
///
/// The rank is `(started_at, entry_id)`: the most recently started run wins, an unstarted
/// one ranks below every started one, and the id breaks the tie — daemon-minted ids
/// increase, so among runs that have not started (or started in the same second) the newest
/// entry is the highest id, which is the creation-order fallback.
///
/// A pointer that is absent, unreadable or not a `SignedStatus` is one this daemon may
/// claim, and so is one naming this same entry: a refresh must be able to replace its own
/// document, or the pointer would freeze at the first publication of the run.
fn ranks_above(signed: &SignedStatus, latest: &Path) -> bool {
    let Some(current) = std::fs::read_to_string(latest)
        .ok()
        .and_then(|text| serde_json::from_str::<SignedStatus>(&text).ok())
    else {
        return true;
    };
    if current.payload.entry_id == signed.payload.entry_id {
        return true;
    }
    let rank = |p: &StatusPayload| (p.started_at.clone(), p.entry_id.clone());
    rank(&signed.payload) >= rank(&current.payload)
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

/// Write `bytes` at `path`, through a temp file in the same directory, at `mode`.
///
/// The mechanics are [`crate::fsutil::write_atomic`]'s and are documented there. What this
/// wrapper decides is the two things a status write has an opinion about.
///
/// # The temp name is per-write, not per-target
///
/// `status-latest.json` is the same target for every entry in the process. A temp name
/// derived from the target — `status-latest.tmp` — is therefore one name two entries
/// mirroring at once both write and both rename, and the loser publishes the winner's
/// bytes under its own rename, or fails on a file the other thread already moved away.
/// [`crate::fsutil::unique_temp`] carries the process id and a per-process counter, so no
/// two writes anywhere can pick it.
///
/// # The parent is created, and the target is not guarded
///
/// `<config_dir>/cache/` is this daemon's own directory and the first mirror of a run is
/// what creates it. And a status document is a published, unauthenticated read: it carries
/// no token, no key and no path, so there is nothing here for a planted symlink to steal —
/// which is why the guard `control.json` sets is deliberately not set here.
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), DaemonError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| DaemonError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    crate::fsutil::write_atomic(
        path,
        &crate::fsutil::unique_temp(path),
        bytes,
        mode,
        crate::fsutil::TargetGuard::Any,
    )
}
