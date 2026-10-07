//! The M2 status projection: what the daemon reports about a capture run, apart from the
//! records of its chain.
//!
//! These types carry [`crate::control::StatusPayload::capture`] and nothing is chained or
//! signed from them, so they are not part of the v5 record freeze (`docs/CONTRACTS.md`, "The
//! v5 format freeze") and may grow with the status API under `CONTROL_API_VERSION`. They
//! lived in [`crate::capture`] until contract S1 (F6); that module re-exports them behind a
//! `#[doc(hidden)]` shim until S5.

use serde::{Deserialize, Serialize};

/// The synced chain frontier: `seq`/`hash` of the last durable link and the chain bytes
/// through it. An appended head is not a durable frontier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableFrontier {
    pub seq: u64,
    pub hash: String,
    pub chain_bytes: u64,
}

/// Storage accounting. `quota_bytes` is the admission ceiling; `reserved_bytes` is measured
/// physical allocation, zero for an explicitly unreserved (quota-only) experiment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureStorage {
    pub quota_bytes: u64,
    pub reserved_bytes: u64,
    pub used_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durable: Option<DurableFrontier>,
    pub accounting_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Starting,
    Running,
    Exited,
    Sealed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Starting,
    Recording,
    Incomplete,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    LocalOnly,
    Pending,
    Synced,
    Stuck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisState {
    NotRun,
    Available,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorState {
    LocalOnly,
    Signed,
}

/// The M2 status projection carried by `StatusPayload::capture`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct M2Status {
    pub run: RunState,
    pub capture: CaptureState,
    pub delivery: DeliveryState,
    pub analysis: AnalysisState,
    pub anchor: AnchorState,
    pub storage: CaptureStorage,
    pub gap_count: u64,
    pub assistance_observed: bool,
}
