//! The health states a status document reports. The daemon reports them; nothing chains them.

use serde::{Deserialize, Serialize};

/// The health states. The daemon reports them; no rule reads them.
///
/// Only the three states the daemon produces are named. A registry row written by an older
/// build may hold another word (`arming_pending`, `hooks_silent` and the rest of the V1 ladder,
/// removed at control API version 8); it reads as [`HealthState::Unknown`] rather than refusing
/// the row, because a row the daemon cannot read is a run it cannot finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Online,
    /// The entry's chain is closed. It is a state of the DOCUMENT: nothing chains it,
    /// because a health event chained after the seal would be a link past the last link. The
    /// status document carries it so that a sealed run stops reading as recording.
    Sealed,
    /// A durable seal intent could not be reconciled. Capture is closed until an operator
    /// repairs the entry; this is deliberately distinct from `Sealed`, because no terminal
    /// claim has been promoted.
    SealingRecoveryNeeded,
    /// A word this build does not produce, read from an older row. No code here sets it.
    #[serde(other)]
    Unknown,
}
