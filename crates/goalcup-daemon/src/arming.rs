//! The arming gate (D16, D7): three live states, one refusal, and no rule interpretation.
//!
//! # The start signal is the first transcript record
//!
//! No hook-derived signal can reach this gate. [`ArmingGate::observe_leg1`] is the only
//! door: on every connector the first leg-1 record is both the signal and its corroboration.
//! Leg 1 is the harness writing its own transcript to its own store, which nothing but the
//! running harness produces.
//!
//! [`ArmingGate::may_stamp_started`] wants **two** facts inside one arming timeout — a
//! signal and a leg-1 record at or after it.
//!
//! # What "corroboration" is, and what it is not
//!
//! It is a **comparison**, not a judgement (D3). This module asks whether a leg-1 record
//! arrived on some store after the signal did. It does not read the record and does not
//! decide whether the record is a prompt. Whether the pair is a human message is
//! `goalcup-verifier`'s V1/V2, on the chained evidence.
//!
//! # The four live states are `control::ArmingState`
//!
//! The gate, the control API, the status page and `goalcup status` name them identically,
//! so there is one vocabulary rather than a translation table. [`GateState::Refused`]
//! carries a [`RefusalReason`] the wire's `ArmingState::Refused` does not — `detail` is
//! still the sentence for a human reader; the wire variant is what lets a machine reader
//! tell "will never arm" apart from "still pending". The entry the gate
//! belongs to moves to [`crate::registry::EntryState::Refused`] on the same transition.
//!
//! # The started transition is stamped with `started_at`
//!
//! The goal-length gate keys on `EntryState::Armed | StartSignalSeen` — the states before
//! `started_at` exists — so an entry carrying a `started_at` under a pre-goal state would
//! have the goal-length rule applied to every later prompt of the run. The registry side of
//! that pairing is [`crate::receipt::record_start`], which writes both in one transaction
//! and asserts it; this side is [`ArmingGate::mark_started`], which is the only way into
//! [`GateState::Started`] and refuses unless both facts are in.

use crate::DaemonError;
use goalcup_protocol::control::ArmingState;
use goalcup_protocol::manifest::ConnectorManifest;

/// Why the gate refused. Never a wire state: each of these is a different sentence a
/// participant has to be able to act on, and "arming failed" is not one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// The signal arrived and nothing corroborated it before the window closed.
    ArmingTimedOut,
    /// The window closed without the declared signal arriving at all.
    StartSignalMissing,
    /// The harness home already holds a session working in a tracked workspace.
    /// Raised by [`crate::foreign::pre_arm_scan`], before an entry is armed at all.
    HarnessHomeOccupied,
}

impl std::fmt::Display for RefusalReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RefusalReason::ArmingTimedOut => "arming timed out",
            RefusalReason::StartSignalMissing => "the start signal never arrived",
            RefusalReason::HarnessHomeOccupied => "the harness home is occupied",
        })
    }
}

/// Where the gate is. The three live values are [`ArmingState`]'s, spelled the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    Armed,
    StartSignalSeen,
    Started,
    Refused(RefusalReason),
}

impl GateState {
    /// The wire state. Total: every `GateState` has a wire value, including a refusal, so a
    /// new variant fails to compile here rather than silently reading as `Armed`.
    pub fn to_control(&self) -> ArmingState {
        match self {
            GateState::Armed => ArmingState::Armed,
            GateState::StartSignalSeen => ArmingState::StartSignalSeen,
            GateState::Started => ArmingState::Started,
            GateState::Refused(_) => ArmingState::Refused,
        }
    }
}

/// One entry's arming gate.
///
/// It holds the moment the entry was armed, the timeout that won, and the instants it has
/// observed. Nothing else — no record, no post, no text.
pub struct ArmingGate {
    timeout_seconds: u64,
    armed_at: f64,
    state: GateState,
    /// When the signal was seen, and which leg carried it.
    start_signal_at: Option<f64>,
    signal_source: Option<&'static str>,
    /// The latest leg-1 observation, not the first: corroboration is "leg 1 has grown
    /// since the signal", so a record that arrived before the signal is superseded by one
    /// that arrived after it rather than standing in for it.
    leg1_at: Option<f64>,
    /// The store that last grew, for the sentence.
    leg1_store: Option<String>,
    /// **The provenance of the corroboration.** The chained sequence number of the leg-1
    /// record that corroborated, and the session it belongs to. Held because a gate that could
    /// only say "a record arrived" produces a `started_at` that nothing in the chain points back
    /// at, and because [`crate::receipt::record_start`] takes the receipt's `session_id` from
    /// here rather than from a caller's argument: the session the run is recorded under is the
    /// one the daemon watched a record arrive in, not one a caller named.
    leg1_seq: Option<u64>,
    session_id: Option<String>,
}

impl ArmingGate {
    /// The gate on the connector's own timeout.
    pub fn new(manifest: &ConnectorManifest, armed_at: f64) -> ArmingGate {
        ArmingGate::with_timeout(manifest, armed_at, None)
    }

    /// The gate on whichever timeout won. Precedence is normative and stated once, on
    /// `manifest::Arming::timeout_seconds`: the connector's default, overridden by the
    /// daemon's `DaemonConfig.arming_timeout_seconds`, overridden by the CLI's
    /// `--arming-timeout` for one start. This constructor takes the number that survived
    /// that and does not re-derive it.
    pub fn with_timeout(
        manifest: &ConnectorManifest,
        armed_at: f64,
        override_seconds: Option<u64>,
    ) -> ArmingGate {
        ArmingGate {
            timeout_seconds: override_seconds.unwrap_or(manifest.arming.timeout_seconds),
            armed_at,
            state: GateState::Armed,
            start_signal_at: None,
            signal_source: None,
            leg1_at: None,
            leg1_store: None,
            leg1_seq: None,
            session_id: None,
        }
    }

    /// Put a rebuilt gate back where the entry's own registry row says the run already was.
    ///
    /// A gate is built fresh from the manifest and the moment the entry was armed, which on a
    /// run this daemon started is exactly right and on a run a previous daemon started is a lie
    /// the arming timeout then acts on: `armed_at` is hours ago, the state says `Armed`, and the
    /// first [`ArmingGate::tick`] after a restart would refuse a run that has been recording all
    /// along. The row is the durable half of this gate — `start_signal_at` and `session_id` are
    /// copied onto it by [`crate::receipt::record_start`] — so a rebuilt gate is told what it
    /// already knew.
    ///
    /// **It is not a second way to start a run.** `started` comes from the row's `started_at`,
    /// which only `record_start` writes and only after [`ArmingGate::mark_started`] allowed it;
    /// nothing here can stamp one.
    ///
    /// **`signal_source` and `leg1_at` are recovered, not invented.** No row records either.
    /// [`ArmingGate::see_signal`] returns early once `start_signal_at` is set, so the source
    /// would never fill in later; and [`ArmingGate::may_stamp_started`] wants a `leg1_at`, so a
    /// run whose corroboration is already in the chain would come back uncorroborated. Neither
    /// is guessed here: [`ArmingGate::observe_leg1`] is the only door into `see_signal`, and it
    /// stamps `leg1_at` with the very instant it passes on. So the source is `"leg1"` and the
    /// signal's instant is the corroborating record's.
    ///
    /// A row an older daemon stamped from its hook leg reads the same way. Such a run that had
    /// started is `Started` either way; one that had seen the hook signal but not yet started is
    /// treated as corroborated at the signal's instant.
    pub fn resume(
        &mut self,
        start_signal_at: Option<f64>,
        session_id: Option<String>,
        started: bool,
    ) {
        self.start_signal_at = start_signal_at;
        self.session_id = session_id;
        // `started` counts as well as a stamped instant: a run that started saw the signal by
        // definition, and a row can carry the one fact and not the other.
        if started || start_signal_at.is_some() {
            self.signal_source = Some("leg1");
            self.leg1_at = start_signal_at;
        }
        self.state = match (started, start_signal_at) {
            (true, _) => GateState::Started,
            (false, Some(_)) => GateState::StartSignalSeen,
            (false, None) => self.state,
        };
    }

    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }

    pub fn state(&self) -> GateState {
        self.state
    }

    /// When the signal was observed, on whichever leg carried it. This is the daemon's
    /// local observation time and is never what the registry's `started_at` is set from —
    /// that is the server's clock, off the receipt.
    pub fn start_signal_at(&self) -> Option<f64> {
        self.start_signal_at
    }

    /// `"leg1"`, or `None` while no signal has been seen.
    pub fn signal_source(&self) -> Option<&'static str> {
        self.signal_source
    }

    /// The session the latest corroborating leg-1 record belongs to, and the chained
    /// sequence number it was linked at. `None` until a leg-1 record has arrived.
    ///
    /// This is what the start receipt's `session_id` claim is built from
    /// ([`crate::receipt::record_start`]). A caller cannot supply one instead: the run is
    /// recorded under the session the daemon watched write.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn leg1_seq(&self) -> Option<u64> {
        self.leg1_seq
    }

    /// The store the latest corroborating record arrived on.
    pub fn leg1_store(&self) -> Option<&str> {
        self.leg1_store.as_deref()
    }

    /// A leg-1 record arrived on `store`.
    ///
    /// `chained_seq` is where that record sits in this entry's chain and `session_id` is
    /// the session it belongs to — the provenance of the corroboration, kept because a
    /// `started_at` no chain link points back at is a claim about a record nobody can find
    /// again, and because the receipt's `session_id` is taken from here rather than from a
    /// caller. Both name the latest record, as the instant does.
    ///
    /// This is the corroboration, and it is also the signal: the first record the harness
    /// writes is what the gate has. The path then satisfies itself, which is not a
    /// weakening: leg 1 is the half nothing but the running harness produces.
    pub fn observe_leg1(
        &mut self,
        store: &str,
        chained_seq: u64,
        session_id: &str,
        at: f64,
    ) -> GateState {
        if self.settled() {
            return self.state;
        }
        self.leg1_at = Some(at);
        self.leg1_store = Some(store.to_string());
        self.leg1_seq = Some(chained_seq);
        self.session_id = Some(session_id.to_string());
        if self.start_signal_at.is_none() {
            self.see_signal("leg1", at);
        }
        self.state
    }

    /// Close the window if it has run out. Idempotent, and a no-op once the gate is
    /// settled: a refusal is a sentence the participant was already given.
    ///
    /// The two refusals are different sentences. Nothing arrived at all is
    /// [`RefusalReason::StartSignalMissing`] — the harness never wrote to its own store. A
    /// signal that arrived and was never corroborated is [`RefusalReason::ArmingTimedOut`].
    pub fn tick(&mut self, now: f64) -> GateState {
        if self.settled() || now - self.armed_at <= self.timeout_seconds as f64 {
            return self.state;
        }
        self.state = GateState::Refused(match self.start_signal_at {
            Some(_) => RefusalReason::ArmingTimedOut,
            None => RefusalReason::StartSignalMissing,
        });
        self.state
    }

    /// Both facts, not one: the signal was seen and leg 1 has grown since.
    ///
    /// "Since" is `>=` rather than `>` on purpose. The first leg-1 record is the signal, so the
    /// two instants are the same number, and that case is corroborated by construction.
    pub fn may_stamp_started(&self) -> bool {
        if self.state != GateState::StartSignalSeen {
            return false;
        }
        match (self.start_signal_at, self.leg1_at) {
            (Some(signal), Some(leg1)) => leg1 >= signal,
            _ => false,
        }
    }

    /// Move to [`GateState::Started`]. The only way in, and it refuses unless both facts
    /// are in — so a caller cannot stamp a `started_at` the gate would not have allowed.
    ///
    /// Paired with [`crate::receipt::record_start`], which writes `started_at`,
    /// `deadline_at` and `EntryState::Started` in one registry transaction. The
    /// length gate reads the entry's state, so the two must move together.
    pub fn mark_started(&mut self) -> Result<(), DaemonError> {
        if !self.may_stamp_started() {
            return Err(DaemonError::Refused(format!(
                "the run was not started: {}",
                self.on_screen()
            )));
        }
        self.state = GateState::Started;
        debug_assert_eq!(
            self.state.to_control(),
            ArmingState::Started,
            "the started gate state is the wire's `started`, and `started_at` is stamped \
             with it"
        );
        Ok(())
    }

    /// What the status line and the terminal say (D25). The signal is named in words,
    /// because "arming pending" tells a participant nothing about what to do next.
    pub fn on_screen(&self) -> String {
        let signal = "the harness's first transcript record";
        match self.state {
            GateState::Armed => format!(
                "armed, waiting for {signal} from this workspace; nothing is recorded as the \
                 run until it arrives"
            ),
            GateState::StartSignalSeen if self.may_stamp_started() => {
                format!("{signal} arrived: the run can start")
            }
            GateState::StartSignalSeen => "the start signal arrived, and the harness has \
                 written nothing to its own store since; the run starts when it does"
                .to_string(),
            GateState::Started => format!("started; {signal} arrived"),
            GateState::Refused(RefusalReason::ArmingTimedOut) => format!(
                "the start signal arrived and no record followed it within {} seconds, so \
                 the run was not started",
                self.timeout_seconds
            ),
            GateState::Refused(RefusalReason::StartSignalMissing) => format!(
                "{signal} never arrived within {} seconds, so the run was not started",
                self.timeout_seconds
            ),
            GateState::Refused(RefusalReason::HarnessHomeOccupied) => format!(
                "the harness home already holds a session working in a tracked workspace, \
                 so {signal} was never waited for"
            ),
        }
    }

    /// Started or refused: nothing observed afterwards changes either.
    fn settled(&self) -> bool {
        matches!(self.state, GateState::Started | GateState::Refused(_))
    }

    /// Record the signal once. A second one does not re-date the first: the start is the
    /// moment the run became observable, not the latest signal.
    fn see_signal(&mut self, source: &'static str, at: f64) {
        if self.start_signal_at.is_some() {
            return;
        }
        self.start_signal_at = Some(at);
        self.signal_source = Some(source);
        self.state = GateState::StartSignalSeen;
    }
}
