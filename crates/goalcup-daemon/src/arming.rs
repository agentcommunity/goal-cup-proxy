//! The arming gate's vocabulary (D16, D7): three live states and one refusal.
//!
//! The V1 gate that moved an entry through these states (`ArmingGate`) went with the V1 code
//! path (D48). What stays is the vocabulary the status words and the health rollup still name,
//! so a status a pre-D48 daemon wrote reads the same way it did.
//!
//! # The live states are `control::ArmingState`
//!
//! The gate, the control API and `goalcup status` name them identically (as the V1 status
//! page, removed by D48, did), so there is one vocabulary rather than a translation table.
//! [`GateState::Refused`] carries a [`RefusalReason`] the wire's `ArmingState::Refused` does
//! not: `detail` is still the sentence for a human reader; the wire variant is what lets a
//! machine reader tell "will never arm" apart from "still pending".

use goalcup_protocol::control::ArmingState;

/// Why the gate refused. Never a wire state: each of these is a different sentence a
/// participant has to be able to act on, and "arming failed" is not one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// The signal arrived and nothing corroborated it before the window closed.
    ArmingTimedOut,
    /// The window closed without the declared signal arriving at all.
    StartSignalMissing,
}

impl std::fmt::Display for RefusalReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RefusalReason::ArmingTimedOut => "arming timed out",
            RefusalReason::StartSignalMissing => "the start signal never arrived",
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
    pub(crate) fn to_control(self) -> ArmingState {
        match self {
            GateState::Armed => ArmingState::Armed,
            GateState::StartSignalSeen => ArmingState::StartSignalSeen,
            GateState::Started => ArmingState::Started,
            GateState::Refused(_) => ArmingState::Refused,
        }
    }
}
