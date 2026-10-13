//! The health states a status document reports. The daemon reports them; nothing chains them.

use serde::{Deserialize, Serialize};

/// The health states. The daemon reports them; no rule reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Online,
    UploadLagging,
    ConnectionLost,
    AgentExited,
    TrackerInterrupted,
    CaptureStalled,
    HooksSilent,
    ConfigRestored,
    DeadlineReached,
    ArmingPending,
    ArmingLost,
    VersionChanged,
    ConfigMismatch,
    /// A tailed store was truncated or its inode changed: the file on disk is no longer
    /// the file the cursor was reading.
    StoreReplaced,
    /// Free space on the artifact-root volume fell below the daemon's reserve, so the
    /// buffer degrades before `append` starts failing.
    DiskPressure,
    /// The entry's chain is closed. Additive, and it is a state of the
    /// DOCUMENT rather than a rung of the V1 health ladder (removed by D48): nothing chains it, because a
    /// health event chained after the seal would be a link past the last link. The status
    /// document carries it so that a sealed run stops reading `online — Recording…` with a
    /// countdown running.
    Sealed,
    /// A durable seal intent could not be reconciled. Capture is closed until an operator
    /// repairs the entry; this is deliberately distinct from `Sealed`, because no terminal
    /// claim has been promoted.
    SealingRecoveryNeeded,
}
