//! The two small persisted records of the V1 workspace-submission capture that the registry row and
//! the daemon configuration carry. They are plain data, kept so a registry written by a v0.5.0
//! V1 build still opens and the daemon configuration still parses; the writer that filled them
//! was removed with the V1 path (D48).

use serde::{Deserialize, Serialize};

pub(crate) const MAX_CAPTURE_NAMESPACE_NODES: u64 = 1_000_000;
pub(crate) const MAX_CAPTURE_INVENTORY_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_CAPTURE_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureResourceLimitsV1 {
    pub version: u8,
    pub namespace_nodes: u64,
    pub inventory_bytes: u64,
    pub manifest_bytes: u64,
}

impl CaptureResourceLimitsV1 {
    /// Validate an explicitly configured writer budget without exposing or duplicating the
    /// writer's hard maxima. Callers still own the configured values; this never defaults them.
    pub(crate) fn validate_for_config(&self) -> Result<(), &'static str> {
        if self.within_hard_bounds() {
            Ok(())
        } else {
            Err(
                "capture resource limits require version 1 and positive values within writer hard maxima",
            )
        }
    }

    fn within_hard_bounds(&self) -> bool {
        self.version == 1
            && self.namespace_nodes > 0
            && self.namespace_nodes <= MAX_CAPTURE_NAMESPACE_NODES
            && self.inventory_bytes > 0
            && self.inventory_bytes <= MAX_CAPTURE_INVENTORY_BYTES
            && self.manifest_bytes > 0
            && self.manifest_bytes <= MAX_CAPTURE_MANIFEST_BYTES
    }
}

/// Once-only source-root binding written before cutoff. The journal contains no reopenable
/// source pathname: a lost source handle is recovery-ambiguous, never an invitation to trust
/// a matching dev/inode at a later pathname.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRootJournalV1 {
    pub identity: String,
}
