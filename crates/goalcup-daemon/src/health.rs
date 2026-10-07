//! The five things the daemon judges (D17), rendered as health state names.
//!
//! D17 fixes the list: (1) the expected transcript stalled while the harness process is
//! alive, (2) hook posts silent while leg 1 grows — no longer judged: the hook leg is
//! gone and `HealthState::HooksSilent` is never produced — (3) the arming signal absent at start,
//! (4) a tracker config file deviating from its expected hash, (5) the harness version
//! changed mid-run. Everything else in [`HealthInputs`] is a **fact the daemon measured**
//! — a process handle that no longer answers, a free-space number, an inode that changed,
//! an acknowledgement that did not verify — and every branch below is a comparison
//! against a constant a manifest or a config already carries. Nothing here reads a rule.
//!
//! # Why this is a report and not a state
//!
//! A ladder that returned one `HealthState` with `AgentExited` above `CaptureStalled` would
//! let killing the harness mask both. "The agent exited" and "capture stopped while the
//! agent was still running" are the two facts a reviewer most needs to tell apart, so the
//! ladder picks one to show the participant and [`HealthReport::also`] carries the rest.
//!
//! # Why `now` is an argument
//!
//! Every time-dependent comparison here reads `inputs.now`, and this module never calls
//! [`crate::now_unix`]. That is what lets a test ask the same inputs about two moments,
//! and what lets `EntryRuntime::poll_once_at` evaluate a whole tick at one clock rather
//! than at slightly different ones.

use crate::arming::GateState;
use goalcup_protocol::events::HealthState;

/// The thresholds the judgements are made against: the arming timeout the gate settled on,
/// and the stall, acknowledgement-lag and disk-reserve values from `DaemonConfig`.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthPolicy {
    pub arming_timeout_seconds: f64,
    pub stall_after_seconds: f64,
    pub ack_lag_seconds: f64,
    pub disk_reserve_bytes: u64,
}

/// Everything one evaluation is allowed to look at. There is no `&Entry` here and no
/// clock: what the caller did not measure, the ladder cannot judge.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthInputs {
    /// The moment this evaluation is about. Never read from the wall clock in this module.
    pub now: f64,
    pub arming: GateState,
    /// Computed from the registered `ProcessHandle`.
    pub harness_alive: bool,
    pub last_leg1_at: Option<f64>,
    pub last_ack_at: Option<f64>,
    pub pending_events: u64,
    /// The last pump reached the server.
    pub connected: bool,
    pub config_mismatch: bool,
    pub arming_lost: bool,
    /// D17's signal 5, sampled where the connector declares `version.self_updating`.
    pub version_changed: bool,
    pub deadline_at: Option<f64>,
    pub agent_exited: bool,
    /// A tailer reported a replacement or a truncation since the last evaluation.
    pub store_replaced: bool,
    /// Free space is below `HealthPolicy.disk_reserve_bytes`.
    pub disk_pressure: bool,
    /// The daemon found an existing chain for this entry on start and wrote an
    /// `EventBody::DaemonRestart` before anything else.
    pub tracker_restarted: bool,
    /// A tracker file deviated and was put back since the last evaluation.
    pub config_restored: bool,
    /// The server's `acked_at` and the local clock disagree beyond the threshold.
    pub clock_skew_seconds: Option<f64>,
    /// The uploader is waiting out a backoff rather than failing.
    pub backing_off: bool,
    /// The server answered with an acknowledgement the pinned key did not vouch
    /// for. Reported as a detail on `ConnectionLost`, because an unverifiable anchor is
    /// closer to "no anchor" than to "anchored".
    pub ack_unverified: bool,
}

/// What the ladder picked, and everything else that was true at the same moment.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthReport {
    pub state: HealthState,
    pub also: Vec<HealthState>,
    /// Human-readable specifics for the chained health event: the clock skew in seconds,
    /// that a store was replaced, how little space is left.
    pub details: Vec<String>,
}

impl HealthReport {
    /// Every state this report asserts, primary first. What the runtime walks before it
    /// chains, filtering through [`chains_as_event`].
    pub fn states(&self) -> Vec<HealthState> {
        let mut all = vec![self.state];
        all.extend(self.also.iter().copied());
        all
    }

    /// The details as one sentence, for `HealthEvent.detail`. Empty when there are none.
    pub fn detail(&self) -> String {
        self.details.join("; ")
    }
}

/// Chain only the states that have no dedicated `EventBody` of their own.
///
/// Six states duplicate a richer event — `ConfigMismatch`, `ConfigRestored` and
/// `ArmingLost` are the three faces of `EventBody::ConfigDeviation` (`restored` and
/// `arming_lost` are fields on it), `VersionChanged` is `EventBody::VersionChange`,
/// `StoreReplaced` is `EventBody::TailNotice`, and `TrackerInterrupted` is
/// `EventBody::DaemonRestart`. Chaining a `Health` event beside them would write the same
/// fact twice, and the richer event is the one a check reads: `ConfigDeviation` carries the
/// path and both hashes, `TailNotice` carries the offsets and the inodes, and V9 reads
/// those, not a state name.
///
/// **All health states still reach the signed status cache.** The cut is about the chain, which
/// is evidence, not about the participant surface, which is D25's whole point.
pub fn chains_as_event(state: HealthState) -> bool {
    !matches!(
        state,
        HealthState::ConfigMismatch
            | HealthState::ConfigRestored
            | HealthState::ArmingLost
            | HealthState::VersionChanged
            | HealthState::StoreReplaced
            | HealthState::TrackerInterrupted
            // `Sealed` is a state of the document and never a rung this ladder picks.
            // Chaining it would be a link past the seal, which is the last
            // link by construction; `status::build_payload` is the only place it is built.
            | HealthState::Sealed
            // Like `Sealed`, this is a durable status-only quarantine. Chaining it would
            // either append after a terminal link or mutate the very intent under review.
            | HealthState::SealingRecoveryNeeded
    )
}

/// The priority ladder. The order is the order a participant needs to hear things in: a
/// cause before its symptom, and a thing they can still act on before a thing that has
/// already happened.
///
/// `AgentExited` is deliberately **not** a rung. It is computed first, kept aside, and
/// reported alongside whatever the ladder picks — a harness that has exited does not
/// explain a capture that stalled while it was still running.
///
/// **The two latched inputs sit at the bottom**: `version_changed` and
/// `tracker_restarted` are set once and never cleared, so a rung above the live signals
/// freezes the surface for the rest of the run rather than reporting it. See the comment
/// where they are read.
pub fn evaluate(inputs: &HealthInputs, policy: &HealthPolicy) -> HealthReport {
    let exited = inputs.agent_exited || !inputs.harness_alive;
    let mut details = Vec::new();
    if let Some(skew) = inputs.clock_skew_seconds {
        // Recorded as a detail on whatever state applies, not as a state of its own.
        // The clocks disagreeing is a fact about the trace, not a capture failure.
        details.push(format!(
            "clock_skew {skew:.0}s against the server's acked_at"
        ));
    }
    if inputs.backing_off {
        details.push("upload backing off after a failed attempt".to_string());
    }
    if inputs.ack_unverified {
        details
            .push("the server's acknowledgement did not verify against the pinned key".to_string());
    }

    // `also` carries exactly one state and no other: `AgentExited`, appended here whenever
    // the ladder picked something else. Every other rung returns an empty `also` — the ladder
    // does not report what it never evaluated.
    let finish = |state: HealthState, mut also: Vec<HealthState>, details: Vec<String>| {
        if exited && state != HealthState::AgentExited {
            also.push(HealthState::AgentExited);
        }
        HealthReport {
            state,
            also,
            details,
        }
    };

    if let Some(deadline) = inputs.deadline_at
        && inputs.now >= deadline
    {
        return finish(HealthState::DeadlineReached, Vec::new(), details);
    }
    if inputs.arming_lost {
        return finish(HealthState::ArmingLost, Vec::new(), details);
    }
    // Signal 3: the clock has not started. A refused gate reports the same state, because
    // what the participant has to do about it is the same thing.
    if matches!(
        inputs.arming,
        GateState::Armed | GateState::StartSignalSeen | GateState::Refused(_)
    ) {
        return finish(HealthState::ArmingPending, Vec::new(), details);
    }
    if inputs.config_mismatch {
        return finish(HealthState::ConfigMismatch, Vec::new(), details);
    }
    // Above the capture states, because a full disk stops capture next and the
    // participant can still act on it while there is room.
    if inputs.disk_pressure {
        details.push(format!(
            "less than {} bytes free where the buffer lives",
            policy.disk_reserve_bytes
        ));
        return finish(HealthState::DiskPressure, Vec::new(), details);
    }
    // The on-disk signature of a rewritten transcript.
    if inputs.store_replaced {
        details.push("a tailed store was replaced or truncated".to_string());
        return finish(HealthState::StoreReplaced, Vec::new(), details);
    }
    // Signal 1: the expected transcript stalled. The "while the harness is alive"
    // qualifier is real — `exited` rides along rather than replacing this.
    let stalled = inputs
        .last_leg1_at
        .map(|at| inputs.now - at > policy.stall_after_seconds)
        .unwrap_or(false);
    if stalled {
        return finish(HealthState::CaptureStalled, Vec::new(), details);
    }
    // These sit below the capture states because they are recoveries — something went
    // wrong and was handled — rather than failures in progress.
    if inputs.config_restored {
        return finish(HealthState::ConfigRestored, Vec::new(), details);
    }
    // An acknowledgement the pinned key did not vouch for is not an anchor, so it reads
    // as no anchor: `details` already says which of the two it was.
    if !inputs.connected || inputs.ack_unverified {
        return finish(HealthState::ConnectionLost, Vec::new(), details);
    }
    let ack_age = inputs
        .last_ack_at
        .map(|at| inputs.now - at)
        .unwrap_or(f64::INFINITY);
    if inputs.pending_events > 0 && ack_age > policy.ack_lag_seconds {
        return finish(HealthState::UploadLagging, Vec::new(), details);
    }
    // Nothing above applied. If the harness has exited, that is the story.
    if exited {
        return HealthReport {
            state: HealthState::AgentExited,
            also: Vec::new(),
            details,
        };
    }
    // **The two latches, at the bottom.** `version_changed` and `tracker_restarted` are the
    // only inputs in this list that are set once and never cleared for the rest of the run:
    // `EntryRuntime` stamps `tracker_restarted` when it finds a chain at startup and
    // `version_changed` when it samples a new harness build, and neither has an event that
    // un-sets it.
    //
    // A rung above the live signals would therefore not report a condition, it would freeze the
    // surface: after one daemon restart the participant would see `TrackerInterrupted` forever,
    // and a server that then vanished, an upload that fell behind, a harness that exited and a
    // capture that stalled would each be reported as a restart that had already been handled.
    // Both are chained as their own richer events — `EventBody::DaemonRestart` and
    // `EventBody::VersionChange`, which is why [`chains_as_event`] cuts them from the chained
    // health — so nothing is lost by their ranking below a condition that is still true right
    // now. They are still the headline when nothing live is.
    if inputs.version_changed {
        return finish(HealthState::VersionChanged, Vec::new(), details);
    }
    if inputs.tracker_restarted {
        return finish(HealthState::TrackerInterrupted, Vec::new(), details);
    }
    finish(HealthState::Online, Vec::new(), details)
}
