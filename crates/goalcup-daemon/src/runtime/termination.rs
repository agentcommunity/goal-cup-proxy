//! How a harness's process tree is ended and what the measurement is allowed to claim, plus the
//! monotone finalization clock every durable timestamp reads.
//!
//! This is the one termination path a local capture finish uses (`capture::finish`). It lives
//! beside that module rather than in it because the V1 workspace finalizer shared it until
//! D48 removed that finalizer.
//! `Confirmed` means exactly what [`crate::terminate`] and `docs/TERMINATION.md` say it means.

use super::Daemon;
use crate::DaemonError;
use goalcup_protocol::process::ProcessHandle;
#[cfg(unix)]
use goalcup_protocol::process::{ProcessError, liveness};
use goalcup_protocol::provenance::{
    Rfc3339Utc, TerminationFailureV1, TerminationMethodV1, TerminationOutcomeV1,
};

/// End `handle`'s tree, walking at most `walk_cap` processes, and say what the measurement
/// established: `Confirmed` exactly as [`terminate_for_finalization`] defines it,
/// `AlreadyStopped` only after asking, and `Unresolved` for everything else.
#[cfg(unix)]
pub(super) fn terminate_handle(
    daemon: &Daemon,
    handle: &ProcessHandle,
    walk_cap: usize,
    requested_at: &Rfc3339Utc,
) -> Result<(TerminationOutcomeV1, Option<String>), DaemonError> {
    let unresolved = |reason: TerminationFailureV1, note: String| {
        unresolved_termination(daemon, requested_at, reason, note)
    };
    // Asked before any signal, because the two facts are different records. A harness the
    // participant quit hours ago was not ended by this seal, and a seal that reported a kill
    // it did not perform would be a lie with a timestamp on it. `liveness` and not
    // `is_alive`: a `/bin/ps` that could not be spawned is "could not ask", and reading that
    // as `AlreadyStopped` would be that same lie for a harness that is in fact running.
    match liveness(handle.pid) {
        Ok(true) => {}
        Ok(false) => {
            return Ok((
                TerminationOutcomeV1::AlreadyStopped {
                    checked_at: daemon
                        .finalization_timestamp_at_or_after(requested_at, "termination check")?,
                },
                Some(format!(
                    "strategy={:?} root={} already_stopped=true signalled=false",
                    handle.strategy, handle.pid
                )),
            ));
        }
        Err(error) => {
            return unresolved(
                TerminationFailureV1::ProcessAuthorityUnavailable,
                format!(
                    "strategy={:?} root={} could not ask whether the harness is running: {error}",
                    handle.strategy, handle.pid
                ),
            );
        }
    }

    let report = match crate::terminate::terminate_within(handle, walk_cap) {
        Ok(report) => report,
        // The guard's own refusal: a recycled number, or a live pid running an argv the
        // launch never registered. Nothing was signalled.
        Err(error @ ProcessError::NoTarget(_)) => {
            return unresolved(
                TerminationFailureV1::ProcessAuthorityUnavailable,
                format!(
                    "strategy={:?} root={} refused={error}",
                    handle.strategy, handle.pid
                ),
            );
        }
        // A kill this daemon attempted and could not finish, including a connector that
        // declares no verified termination path at all.
        Err(error) => {
            return unresolved(
                TerminationFailureV1::RequestFailed,
                format!(
                    "strategy={:?} root={} failed={error}",
                    handle.strategy, handle.pid
                ),
            );
        }
    };
    let measured = termination_summary(&report);

    // A capped walk kept its root and its authority; it simply ran out of budget, and the
    // honest record of that is "something may still be running", not "authority unavailable".
    if report.walk_cap_exceeded || !report.still_alive.is_empty() {
        return unresolved(TerminationFailureV1::ProcessStillRunning, measured);
    }
    // `still_alive` can be empty beside a lost root: containment itself became unverified
    // and no remaining process can be named safely. An empty `targeted` is the same kind of
    // emptiness — nothing was signalled, so nothing was observed gone.
    if !report.observed_gone || report.targeted.is_empty() {
        return unresolved(TerminationFailureV1::ProcessAuthorityUnavailable, measured);
    }
    // The second sweep. The first one ends the instant a snapshot finds nothing, and that
    // snapshot is taken while the tree is still unwinding.
    match crate::terminate::none_alive_after(
        &report.targeted,
        std::time::Duration::from_millis(crate::terminate::RECONFIRM_DELAY_MS),
    ) {
        Ok(true) => {}
        Ok(false) => {
            return unresolved(
                TerminationFailureV1::ProcessStillRunning,
                format!("{measured} reconfirmed=false"),
            );
        }
        // The second sweep could not spawn `/bin/ps`. "Could not ask" is not "still running"
        // and it is certainly not the `Confirmed` below: record the uncertainty instead.
        Err(error) => {
            return unresolved(
                TerminationFailureV1::ProcessAuthorityUnavailable,
                format!("{measured} reconfirmed=unavailable: {error}"),
            );
        }
    }
    Ok((
        TerminationOutcomeV1::Confirmed {
            requested_at: requested_at.clone(),
            confirmed_at: daemon
                .finalization_timestamp_at_or_after(requested_at, "termination confirmation")?,
            method: TerminationMethodV1::ProcessSignal,
        },
        Some(format!("{measured} reconfirmed=true")),
    ))
}

/// [`terminate_for_finalization`]'s reason on a platform other than unix.
#[cfg(not(unix))]
pub(super) fn terminate_handle(
    daemon: &Daemon,
    _handle: &ProcessHandle,
    _walk_cap: usize,
    requested_at: &Rfc3339Utc,
) -> Result<(TerminationOutcomeV1, Option<String>), DaemonError> {
    unresolved_termination(
        daemon,
        requested_at,
        TerminationFailureV1::UnsupportedPlatform,
        "no verified termination path on this platform in V1".to_string(),
    )
}

/// `Unresolved` for `reason`, stopped waiting now and never before `requested_at`, with
/// `note` for the durable record beside it.
pub(super) fn unresolved_termination(
    daemon: &Daemon,
    requested_at: &Rfc3339Utc,
    reason: TerminationFailureV1,
    note: String,
) -> Result<(TerminationOutcomeV1, Option<String>), DaemonError> {
    Ok((
        TerminationOutcomeV1::Unresolved {
            requested_at: requested_at.clone(),
            stopped_waiting_at: daemon
                .finalization_timestamp_at_or_after(requested_at, "termination stop-wait")?,
            reason,
        },
        Some(note),
    ))
}

/// Every number the kill measured, in one line, for the durable record beside the verdict.
///
/// `all_gone` and `harness_unverified` are in it deliberately: the outcome the verifier
/// judges is built from `observed_gone`, and a reader comparing a `Confirmed` record against
/// what this module can prove on this platform has to be able to see both.
#[cfg(unix)]
fn termination_summary(report: &crate::terminate::TerminationReport) -> String {
    format!(
        "strategy={:?} targeted={:?} survived_term={:?} killed={:?} still_alive={:?} \
         observed_gone={} walk_cap_exceeded={} all_gone={} harness_unverified={} \
         guard_degraded={} waited_ms={}",
        report.strategy,
        report.targeted,
        report.survived_term,
        report.killed,
        report.still_alive,
        report.observed_gone,
        report.walk_cap_exceeded,
        report.all_gone,
        report.harness_unverified,
        report.guard_degraded,
        report.waited_ms
    )
}

impl Daemon {
    pub(super) fn finalization_now_unix(&self) -> f64 {
        #[cfg(any(test, feature = "test-hooks"))]
        let candidate = self
            .inner
            .finalization_clock_steps
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_front()
            .or_else(|| {
                *self
                    .inner
                    .finalization_clock
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
            })
            .unwrap_or_else(|| {
                self.inner.finalization_epoch_unix + self.inner.started.elapsed().as_secs_f64()
            });
        #[cfg(not(any(test, feature = "test-hooks")))]
        let candidate =
            self.inner.finalization_epoch_unix + self.inner.started.elapsed().as_secs_f64();

        let mut high_water = self
            .inner
            .finalization_high_water
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *high_water = high_water.max(candidate);
        *high_water
    }

    pub(super) fn finalization_timestamp_at_or_after(
        &self,
        minimum: &Rfc3339Utc,
        fact: &str,
    ) -> Result<Rfc3339Utc, DaemonError> {
        let current = self.finalization_timestamp(fact)?;
        if goalcup_protocol::control::parse_z(current.as_str())
            < goalcup_protocol::control::parse_z(minimum.as_str())
        {
            Ok(minimum.clone())
        } else {
            Ok(current)
        }
    }

    /// Now on the finalization clock, as the timestamp of `fact`.
    pub(super) fn finalization_timestamp(&self, fact: &str) -> Result<Rfc3339Utc, DaemonError> {
        Rfc3339Utc::try_from(goalcup_protocol::control::format_z(
            self.finalization_now_unix().max(0.0) as u64,
        ))
        .map_err(|error| DaemonError::Refused(format!("cannot timestamp {fact}: {error}")))
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_finalization_clock_for_test(&self, now: f64) {
        self.inner
            .finalization_clock_steps
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        *self
            .inner
            .finalization_clock
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(now);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_finalization_clock_steps_for_test(&self, steps: impl IntoIterator<Item = f64>) {
        *self
            .inner
            .finalization_clock_steps
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = steps.into_iter().collect();
    }
}
