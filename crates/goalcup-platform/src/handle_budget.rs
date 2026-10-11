//! Shared native-handle accounting for bounded Git source capture.

use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Maximum simultaneously live source and scratch handles in one capture.
pub const MAX_GIT_LIVE_HANDLES_V1: usize = 32;

/// A shared fixed-capacity handle budget.
#[derive(Clone, Debug)]
pub struct NativeHandleBudgetV1 {
    live: Arc<AtomicUsize>,
}

impl NativeHandleBudgetV1 {
    /// Create an empty 32-handle budget.
    pub fn new() -> Self {
        Self {
            live: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Reserve one handle before attempting its native acquisition.
    pub fn reserve_scratch(&self) -> Result<NativeHandlePermitV1, HandleBudgetError> {
        self.live
            .try_update(Ordering::AcqRel, Ordering::Acquire, |live| {
                (live < MAX_GIT_LIVE_HANDLES_V1).then_some(live + 1)
            })
            .map_err(|_| HandleBudgetError::Exhausted)?;
        Ok(NativeHandlePermitV1 {
            live: Arc::clone(&self.live),
        })
    }
}

impl Default for NativeHandleBudgetV1 {
    fn default() -> Self {
        Self::new()
    }
}

/// One non-clone reservation released exactly once on drop.
pub struct NativeHandlePermitV1 {
    live: Arc<AtomicUsize>,
}

impl fmt::Debug for NativeHandlePermitV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeHandlePermitV1")
            .finish_non_exhaustive()
    }
}

impl Drop for NativeHandlePermitV1 {
    fn drop(&mut self) {
        let previous = self.live.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
    }
}

/// Failure to reserve within the fixed handle budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandleBudgetError {
    Exhausted,
}

impl fmt::Display for HandleBudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native handle budget exhausted")
    }
}

impl std::error::Error for HandleBudgetError {}
