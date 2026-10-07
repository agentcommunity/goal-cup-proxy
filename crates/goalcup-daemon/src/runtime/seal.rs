use super::termination::{terminate_handle, unresolved_termination};
use super::{Daemon, EntryRuntime, held_elsewhere, path_free};
use crate::buffer::Buffer;
#[cfg(any(test, feature = "test-hooks"))]
use crate::buffer::OutOfWindowLimitsV1;
use crate::config::DaemonConfig;
use crate::registry::{
    ChainPositionV1, ChainPredecessorV1, Entry, EntryState, FinalizationPhaseV1, Registry,
    SealOutcome,
};
use crate::status;
use crate::upload::{PumpOutcome, UploadClient, Uploader};
use crate::{DaemonError, now_unix};
use goalcup_platform::tracecap::{ArtifactRoot, EntryArtifactRoot, TraceDir};
use goalcup_protocol::control::FinishReportV1;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{CHAIN_FILE, TraceExport, TraceRoot};
use goalcup_protocol::process::{ProcessError, ProcessHandle, liveness};
use goalcup_protocol::provenance::{
    CutoffReachedV1, FinalizationV1, Rfc3339Utc, Sha256Digest, TerminationFailureV1,
    TerminationOutcomeV1,
};
use goalcup_protocol::seal::SealManifest;
use goalcup_protocol::submission::{FinishReasonV1, SubmissionFailureV1, SubmissionOutcomeV1};
use std::collections::BTreeMap;
#[cfg(any(test, feature = "test-hooks"))]
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
#[cfg(any(test, feature = "test-hooks"))]
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::{Mutex, OnceLock};

fn predecessor_from_head(head: Option<(u64, String)>) -> Result<ChainPredecessorV1, DaemonError> {
    match head {
        Some((seq, hash)) => Ok(ChainPredecessorV1::Position(ChainPositionV1 {
            seq,
            hash: Sha256Digest::try_from(hash).map_err(|error| {
                DaemonError::Refused(format!("chain head is not canonical: {error}"))
            })?,
        })),
        None => Ok(ChainPredecessorV1::Genesis),
    }
}

fn expected_successor(predecessor: &ChainPredecessorV1) -> Result<(u64, &str), DaemonError> {
    match predecessor {
        ChainPredecessorV1::Genesis => Ok((0, goalcup_protocol::chain::GENESIS)),
        ChainPredecessorV1::Position(position) => Ok((
            position.seq.checked_add(1).ok_or_else(|| {
                DaemonError::Refused("workspace finalization exhausted chain sequence".to_string())
            })?,
            position.hash.as_str(),
        )),
    }
}

fn termination_terminal_at(outcome: &TerminationOutcomeV1) -> &Rfc3339Utc {
    match outcome {
        TerminationOutcomeV1::AlreadyStopped { checked_at } => checked_at,
        TerminationOutcomeV1::Confirmed { confirmed_at, .. } => confirmed_at,
        TerminationOutcomeV1::Unresolved {
            stopped_waiting_at, ..
        } => stopped_waiting_at,
    }
}

trait WorkspaceFinalizationOwner {
    fn entry(&self) -> &Entry;
    fn drain_admitted(
        &mut self,
        watermarks: &crate::registry::AdmissionWatermarksV1,
        expired: &mut dyn FnMut() -> bool,
    ) -> goalcup_protocol::provenance::PreCutoffDrainOutcomeV1;
    fn writer_head(&mut self) -> Result<Option<(u64, String)>, DaemonError>;
    fn read_chain_line(&mut self, seq: u64) -> Result<Envelope, DaemonError>;
    fn write_cutoff(
        &mut self,
        recorded_at: &str,
        cutoff: CutoffReachedV1,
    ) -> Result<goalcup_protocol::chain::Link, DaemonError>;
    fn write_finalization(
        &mut self,
        recorded_at: &str,
        finalization: FinalizationV1,
    ) -> Result<goalcup_protocol::chain::Link, DaemonError>;
    fn retain_boundary(&mut self, envelope: &Envelope) -> Result<(), DaemonError>;
    fn trace(&self) -> Result<goalcup_platform::tracecap::TraceDir, DaemonError>;
    #[cfg(any(test, feature = "test-hooks"))]
    fn pause_before_capture_for_test(&mut self) {}
    fn advance<F>(
        &mut self,
        expected: FinalizationPhaseV1,
        next: FinalizationPhaseV1,
        update: F,
    ) -> Result<(), DaemonError>
    where
        F: FnOnce(&mut crate::registry::WorkspaceSealingIntentV1) -> Result<(), DaemonError>;
    fn artifact(&self) -> &EntryArtifactRoot;
}

impl WorkspaceFinalizationOwner for EntryRuntime {
    fn entry(&self) -> &Entry {
        self.entry()
    }

    fn drain_admitted(
        &mut self,
        watermarks: &crate::registry::AdmissionWatermarksV1,
        expired: &mut dyn FnMut() -> bool,
    ) -> goalcup_protocol::provenance::PreCutoffDrainOutcomeV1 {
        self.drain_finalization_admitted(watermarks, expired)
    }

    fn writer_head(&mut self) -> Result<Option<(u64, String)>, DaemonError> {
        self.workspace_writer_head()
    }

    fn read_chain_line(&mut self, seq: u64) -> Result<Envelope, DaemonError> {
        self.read_workspace_chain_line(seq)
    }

    fn write_cutoff(
        &mut self,
        recorded_at: &str,
        cutoff: CutoffReachedV1,
    ) -> Result<goalcup_protocol::chain::Link, DaemonError> {
        self.write_workspace_cutoff(recorded_at, cutoff)
    }

    fn write_finalization(
        &mut self,
        recorded_at: &str,
        finalization: FinalizationV1,
    ) -> Result<goalcup_protocol::chain::Link, DaemonError> {
        self.write_workspace_finalization(recorded_at, finalization)
    }

    fn retain_boundary(&mut self, envelope: &Envelope) -> Result<(), DaemonError> {
        self.buffer_workspace_boundary(envelope)
    }

    fn trace(&self) -> Result<goalcup_platform::tracecap::TraceDir, DaemonError> {
        self.workspace_trace_root()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn pause_before_capture_for_test(&mut self) {
        self.pause_before_workspace_capture_for_test();
    }

    fn advance<F>(
        &mut self,
        expected: FinalizationPhaseV1,
        next: FinalizationPhaseV1,
        update: F,
    ) -> Result<(), DaemonError>
    where
        F: FnOnce(&mut crate::registry::WorkspaceSealingIntentV1) -> Result<(), DaemonError>,
    {
        self.advance_workspace_finalization(expected, next, update)
    }

    fn artifact(&self) -> &EntryArtifactRoot {
        self.artifact()
    }
}

fn reconcile_or_write_cutoff<O: WorkspaceFinalizationOwner>(
    runtime: &mut O,
    intent: &crate::registry::WorkspaceSealingIntentV1,
) -> Result<goalcup_protocol::chain::Link, DaemonError> {
    let predecessor = intent.expected_cutoff_predecessor.as_ref().ok_or_else(|| {
        DaemonError::Refused("workspace cutoff append has no durable predecessor".to_string())
    })?;
    let (expected_seq, expected_prev) = expected_successor(predecessor)?;
    let cutoff = CutoffReachedV1 {
        receipt_sha256: intent.receipt_sha256.clone(),
        deadline_at: Rfc3339Utc::try_from(
            runtime
                .entry()
                .receipt()
                .ok_or_else(|| {
                    DaemonError::Refused("workspace cutoff lost its verified receipt".to_string())
                })?
                .claims
                .deadline_at
                .clone(),
        )
        .map_err(|error| DaemonError::Refused(format!("invalid signed deadline: {error}")))?,
        pre_cutoff_drain: intent.drain.clone().ok_or_else(|| {
            DaemonError::Refused("workspace cutoff lost its durable drain outcome".to_string())
        })?,
    };
    let head = runtime.writer_head()?;
    let at_predecessor = match (head.as_ref(), predecessor) {
        (None, ChainPredecessorV1::Genesis) => true,
        (Some((seq, hash)), ChainPredecessorV1::Position(position)) => {
            *seq == position.seq && hash == position.hash.as_str()
        }
        _ => false,
    };
    if at_predecessor {
        runtime.write_cutoff(intent.cutoff_at.as_str(), cutoff)?;
        let envelope = runtime.read_chain_line(expected_seq)?;
        runtime.retain_boundary(&envelope)?;
        return Ok(envelope.link);
    }
    if head.as_ref().is_some_and(|(seq, _)| *seq == expected_seq) {
        let envelope = runtime.read_chain_line(expected_seq)?;
        if envelope.entry_id == intent.entry_id
            && envelope.source == "daemon"
            && envelope.recorded_at == intent.cutoff_at.as_str()
            && envelope.link.prev == expected_prev
            && envelope.redaction.is_none()
            && envelope.body == EventBody::CutoffReachedV1(cutoff)
        {
            runtime.retain_boundary(&envelope)?;
            return Ok(envelope.link);
        }
    }
    Err(DaemonError::Refused(format!(
        "entry {} cutoff append does not reconcile with its durable predecessor",
        intent.entry_id
    )))
}

fn reconcile_or_write_finalization<O: WorkspaceFinalizationOwner>(
    runtime: &mut O,
    intent: &crate::registry::WorkspaceSealingIntentV1,
) -> Result<goalcup_protocol::chain::Link, DaemonError> {
    let predecessor = intent
        .expected_finalization_predecessor
        .as_ref()
        .ok_or_else(|| {
            DaemonError::Refused(
                "workspace finalization append has no durable predecessor".to_string(),
            )
        })?;
    let (expected_seq, expected_prev) = expected_successor(predecessor)?;
    let finalization = FinalizationV1::WorkspaceSubmission {
        receipt_sha256: intent.receipt_sha256.clone(),
        termination: intent.termination.clone().ok_or_else(|| {
            DaemonError::Refused("workspace finalization lost termination outcome".to_string())
        })?,
        submission: intent.submission.clone().ok_or_else(|| {
            DaemonError::Refused("workspace finalization lost submission outcome".to_string())
        })?,
    };
    let recorded_at = intent.finalization_at.as_ref().ok_or_else(|| {
        DaemonError::Refused("workspace finalization lost its timestamp".to_string())
    })?;
    let head = runtime.writer_head()?;
    if head.as_ref().is_some_and(|(seq, hash)| match predecessor {
        ChainPredecessorV1::Genesis => false,
        ChainPredecessorV1::Position(position) => {
            *seq == position.seq && hash == position.hash.as_str()
        }
    }) {
        runtime.write_finalization(recorded_at.as_str(), finalization)?;
        let envelope = runtime.read_chain_line(expected_seq)?;
        runtime.retain_boundary(&envelope)?;
        return Ok(envelope.link);
    }
    if head.as_ref().is_some_and(|(seq, _)| *seq == expected_seq) {
        let envelope = runtime.read_chain_line(expected_seq)?;
        if envelope.entry_id == intent.entry_id
            && envelope.source == "daemon"
            && envelope.recorded_at == recorded_at.as_str()
            && envelope.link.prev == expected_prev
            && envelope.redaction.is_none()
            && envelope.body == EventBody::FinalizationV1(finalization)
        {
            runtime.retain_boundary(&envelope)?;
            return Ok(envelope.link);
        }
    }
    Err(DaemonError::Refused(format!(
        "entry {} finalization append does not reconcile with its durable predecessor",
        intent.entry_id
    )))
}

fn workspace_finish_report(entry: &Entry) -> Result<FinishReportV1, DaemonError> {
    let intent = entry.workspace_sealing_intent().ok_or_else(|| {
        DaemonError::Refused(format!(
            "entry {} has no workspace finalization result",
            entry.entry_id
        ))
    })?;
    if intent.phase != FinalizationPhaseV1::Terminal || !intent.status_mirror_complete {
        return Err(DaemonError::Refused(format!(
            "entry {} workspace finalization is not terminal",
            entry.entry_id
        )));
    }
    Ok(FinishReportV1 {
        entry_id: entry.entry_id.clone(),
        trace_dir: entry.trace_dir.display().to_string(),
        termination: intent.termination.clone().ok_or_else(|| {
            DaemonError::Refused("terminal workspace intent lost termination outcome".to_string())
        })?,
        submission: intent.submission.clone().ok_or_else(|| {
            DaemonError::Refused("terminal workspace intent lost submission outcome".to_string())
        })?,
    })
}

/// End the harness this finalization intent latched authority over, and say what the
/// measurement actually established.
///
/// # The authority is the one the cutoff latched, not the one the row holds now
///
/// `intent.accepted_handoff` was written in the same transaction that closed admission, and
/// it carries the handoff generation and canonical digest beside the handle. A relaunch that
/// registered a new process after the cutoff is a different generation, and signalling it
/// from this finalization would end a session this record does not describe. Reading the
/// live row instead would have made that a silent behaviour of restart ordering.
///
/// # What `Confirmed` means here, exactly
///
/// Five conditions, all of them facts this daemon measured without privilege:
///
/// 1. the latched root was **alive** when finalization reached this phase (otherwise
///    `AlreadyStopped`, which is not a clean result);
/// 2. the whole owned set was enumerated **within the signed profile's `process_walk`**
///    budget (otherwise the set below speaks for a fraction of a tree);
/// 3. [`crate::terminate::terminate_within`] ran from first signal to last sweep under a
///    root it re-authenticated at every poll, adopting each descendant that appeared while
///    the tree was dying;
/// 4. the final authenticated snapshot found **none of `targeted` alive**; and
/// 5. a **second** sweep, taken [`crate::terminate::RECONFIRM_DELAY_MS`] later, still found
///    none of them alive.
///
/// # What it does not mean
///
/// It is not a claim that every `kill(2)` was atomic with the snapshot that authorised it —
/// macOS has no pidfd-class handle for a tree member, so `TerminationReport::all_gone`, the
/// field that conjoins instance-bound delivery with the observation, is never true there.
/// That is why the predicate below reads `observed_gone` and not `all_gone`, and why the
/// measurement string goes into the durable record beside the verdict: `all_gone` and
/// `harness_unverified` are in it, so a reader is never told less than the daemon knew.
///
/// It is also not a claim about a process that left both the group and the lineage before
/// the first snapshot existed. Nothing unprivileged can make that claim, and the module doc
/// on `terminate.rs` says so in the same words.
#[cfg(unix)]
fn terminate_for_finalization(
    daemon: &Daemon,
    intent: &crate::registry::WorkspaceSealingIntentV1,
    requested_at: &Rfc3339Utc,
) -> Result<(TerminationOutcomeV1, Option<String>), DaemonError> {
    let unresolved = |reason: TerminationFailureV1, note: String| {
        unresolved_termination(daemon, requested_at, reason, note)
    };
    let Some(accepted) = intent.accepted_handoff.as_ref() else {
        return unresolved(
            TerminationFailureV1::ProcessAuthorityUnavailable,
            format!(
                "entry {} reached termination with no latched process authority: this \
                 finalization signalled nothing and cannot say whether the harness is running",
                intent.entry_id
            ),
        );
    };
    if accepted.registration.generation != intent.generation {
        return unresolved(
            TerminationFailureV1::ProcessAuthorityUnavailable,
            format!(
                "entry {} latched process authority for generation {} while finalizing \
                 generation {}",
                intent.entry_id, accepted.registration.generation, intent.generation
            ),
        );
    }
    // The signed profile's budget, clamped by the module's own ceiling. `process_walk` is a
    // number the participant's start receipt was signed over; a seal that walked further
    // than the rules it is judged by would be reporting on a set those rules never admitted.
    let walk_cap = usize::try_from(intent.limits.process_walk).unwrap_or(usize::MAX);
    terminate_handle(
        daemon,
        &accepted.registration.handle,
        walk_cap,
        requested_at,
    )
}

/// A platform other than unix has no verified termination path. Saying so is the honest
/// report; claiming the tree is gone is not.
#[cfg(not(unix))]
fn terminate_for_finalization(
    daemon: &Daemon,
    intent: &crate::registry::WorkspaceSealingIntentV1,
    requested_at: &Rfc3339Utc,
) -> Result<(TerminationOutcomeV1, Option<String>), DaemonError> {
    unresolved_termination(
        daemon,
        requested_at,
        TerminationFailureV1::UnsupportedPlatform,
        format!(
            "entry {} ran on a platform with no verified termination path in V1",
            intent.entry_id
        ),
    )
}

fn advance_workspace_runtime<O: WorkspaceFinalizationOwner>(
    daemon: &Daemon,
    runtime: &mut O,
    source: Option<goalcup_platform::sourcecap::SourceDir>,
) -> Result<(), DaemonError> {
    loop {
        let intent = runtime
            .entry()
            .workspace_sealing_intent()
            .cloned()
            .ok_or_else(|| {
                DaemonError::Refused("workspace finalization lost its durable intent".to_string())
            })?;
        #[cfg(any(test, feature = "test-hooks"))]
        {
            let mut fault = daemon
                .inner
                .workspace_finalization_fault
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if fault.as_ref() == Some(&intent.phase) {
                *fault = None;
                return Err(DaemonError::Refused(format!(
                    "injected workspace finalization interruption after {:?}",
                    intent.phase
                )));
            }
        }
        match intent.phase {
            FinalizationPhaseV1::AdmissionClosed => {
                let started = daemon.finalization_now_unix();
                let deadline = started + intent.limits.pre_cutoff_drain as f64 / 1_000.0;
                let mut expired = || daemon.finalization_now_unix() >= deadline;
                let drain = runtime.drain_admitted(&intent.admission_watermarks, &mut expired);
                runtime.advance(
                    FinalizationPhaseV1::AdmissionClosed,
                    FinalizationPhaseV1::DrainResolved,
                    |staged| {
                        staged.drain = Some(drain.clone());
                        Ok(())
                    },
                )?;
            }
            FinalizationPhaseV1::DrainResolved => {
                let predecessor = predecessor_from_head(runtime.writer_head()?)?;
                runtime.advance(
                    FinalizationPhaseV1::DrainResolved,
                    FinalizationPhaseV1::CutoffAppendPending,
                    |staged| {
                        staged.expected_cutoff_predecessor = Some(predecessor.clone());
                        Ok(())
                    },
                )?;
            }
            FinalizationPhaseV1::CutoffAppendPending => {
                let cutoff = reconcile_or_write_cutoff(runtime, &intent)?;
                #[cfg(any(test, feature = "test-hooks"))]
                if daemon
                    .inner
                    .fail_after_workspace_cutoff_append
                    .swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err(DaemonError::Refused(
                        "injected workspace finalization interruption after cutoff append"
                            .to_string(),
                    ));
                }
                runtime.advance(
                    FinalizationPhaseV1::CutoffAppendPending,
                    FinalizationPhaseV1::CutoffBound,
                    |staged| {
                        staged.cutoff = Some(cutoff.clone());
                        Ok(())
                    },
                )?;
            }
            FinalizationPhaseV1::CutoffBound => {
                runtime.advance(
                    FinalizationPhaseV1::CutoffBound,
                    FinalizationPhaseV1::HandoffReconciled,
                    |_| Ok(()),
                )?;
            }
            FinalizationPhaseV1::HandoffReconciled => {
                // The daemon's half of the seal: it kills, because at the deadline
                // the participant's terminal is long gone. `requested_at` is taken before the
                // first signal so the record cannot date a kill earlier than the cutoff that
                // authorised it.
                let requested_at = daemon
                    .finalization_timestamp_at_or_after(&intent.cutoff_at, "termination request")?;
                let (termination, termination_report) =
                    terminate_for_finalization(daemon, &intent, &requested_at)?;
                runtime.advance(
                    FinalizationPhaseV1::HandoffReconciled,
                    FinalizationPhaseV1::Terminated,
                    move |staged| {
                        staged.termination = Some(termination);
                        staged.termination_report = termination_report;
                        Ok(())
                    },
                )?;
            }
            FinalizationPhaseV1::Terminated => {
                #[cfg(any(test, feature = "test-hooks"))]
                runtime.pause_before_capture_for_test();
                let trace = runtime.trace()?;
                let request = crate::submission::SubmissionRequestV1 {
                    generation: intent.generation,
                    entry_id: intent.entry_id.clone(),
                    receipt_sha256: intent.receipt_sha256.clone(),
                    cutoff: intent.cutoff.clone().ok_or_else(|| {
                        DaemonError::Refused("workspace capture lost its cutoff link".to_string())
                    })?,
                    reason: intent.reason,
                    excluded: intent.excluded.clone(),
                    limit_profile: intent.limit_profile.clone(),
                    limits: intent.limits.clone(),
                    capture_limits: intent.capture_limits.clone(),
                };
                let submission = match intent.source_root.journal() {
                    None => SubmissionOutcomeV1::Incomplete {
                        reason: SubmissionFailureV1::RecoveryAmbiguous,
                    },
                    Some(journal) => {
                        let captured = match source.as_ref() {
                            Some(source) => crate::submission::capture_submission(
                                source, &trace, journal, &request,
                            ),
                            None => {
                                crate::submission::recover_submission(&trace, journal, &request)
                            }
                        };
                        match captured {
                            Ok(captured) => {
                                match crate::submission::readback_submission(&trace, &captured) {
                                    Ok(_) => SubmissionOutcomeV1::Captured(captured),
                                    Err(error) => SubmissionOutcomeV1::Incomplete {
                                        reason: error
                                            .submission_failure()
                                            .unwrap_or(SubmissionFailureV1::ReadbackFailed),
                                    },
                                }
                            }
                            Err(error) => SubmissionOutcomeV1::Incomplete {
                                reason: error.submission_failure().unwrap_or(if source.is_some() {
                                    SubmissionFailureV1::StorageFailed
                                } else {
                                    SubmissionFailureV1::RecoveryAmbiguous
                                }),
                            },
                        }
                    }
                };
                #[cfg(any(test, feature = "test-hooks"))]
                if daemon
                    .inner
                    .fail_after_workspace_capture
                    .swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err(DaemonError::Refused(
                        "injected workspace finalization interruption after durable capture"
                            .to_string(),
                    ));
                }
                runtime.advance(
                    FinalizationPhaseV1::Terminated,
                    FinalizationPhaseV1::SubmissionResolved,
                    |staged| {
                        staged.submission = Some(submission.clone());
                        Ok(())
                    },
                )?;
            }
            FinalizationPhaseV1::SubmissionResolved => {
                let termination = intent.termination.as_ref().ok_or_else(|| {
                    DaemonError::Refused(
                        "workspace finalization lost its termination outcome".to_string(),
                    )
                })?;
                let finalization_at = daemon.finalization_timestamp_at_or_after(
                    termination_terminal_at(termination),
                    "workspace finalization",
                )?;
                let cutoff = intent.cutoff.as_ref().ok_or_else(|| {
                    DaemonError::Refused("workspace finalization lost its cutoff".to_string())
                })?;
                let predecessor = ChainPredecessorV1::Position(ChainPositionV1 {
                    seq: cutoff.seq,
                    hash: Sha256Digest::try_from(cutoff.hash.clone()).map_err(|error| {
                        DaemonError::Refused(format!("invalid cutoff hash: {error}"))
                    })?,
                });
                runtime.advance(
                    FinalizationPhaseV1::SubmissionResolved,
                    FinalizationPhaseV1::FinalizationAppendPending,
                    |staged| {
                        staged.finalization_at = Some(finalization_at.clone());
                        staged.expected_finalization_predecessor = Some(predecessor.clone());
                        Ok(())
                    },
                )?;
            }
            FinalizationPhaseV1::FinalizationAppendPending => {
                let finalization = reconcile_or_write_finalization(runtime, &intent)?;
                #[cfg(any(test, feature = "test-hooks"))]
                if daemon
                    .inner
                    .fail_after_workspace_finalization_append
                    .swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err(DaemonError::Refused(
                        "injected workspace finalization interruption after finalization append"
                            .to_string(),
                    ));
                }
                runtime.advance(
                    FinalizationPhaseV1::FinalizationAppendPending,
                    FinalizationPhaseV1::FinalizationBound,
                    |staged| {
                        staged.finalization = Some(finalization.clone());
                        Ok(())
                    },
                )?;
            }
            FinalizationPhaseV1::FinalizationBound => {
                let status_at = daemon.finalization_timestamp_at_or_after(
                    intent.finalization_at.as_ref().ok_or_else(|| {
                        DaemonError::Refused(
                            "workspace finalization lost its terminal timestamp".to_string(),
                        )
                    })?,
                    "terminal status",
                )?;
                runtime.advance(
                    FinalizationPhaseV1::FinalizationBound,
                    FinalizationPhaseV1::TerminalStatusPending,
                    |staged| {
                        staged.terminal_status_at = Some(status_at.clone());
                        Ok(())
                    },
                )?;
            }
            FinalizationPhaseV1::TerminalStatusPending | FinalizationPhaseV1::Terminal => {
                return Ok(());
            }
        }
    }
}

/// Publish the signed mirror only after every post-cutoff source has either been retained
/// durably or raised a durable nonclean marker. Callers hold the admission latch across
/// the final OOW sweep and the terminal CAS.
fn complete_workspace_terminal<O: WorkspaceFinalizationOwner>(
    daemon: &Daemon,
    runtime: &mut O,
) -> Result<FinishReportV1, DaemonError> {
    let intent = runtime
        .entry()
        .workspace_sealing_intent()
        .cloned()
        .ok_or_else(|| {
            DaemonError::Refused("workspace finalization lost its durable intent".to_string())
        })?;
    if intent.phase == FinalizationPhaseV1::Terminal {
        return workspace_finish_report(runtime.entry());
    }
    if intent.phase != FinalizationPhaseV1::TerminalStatusPending {
        return Err(DaemonError::Refused(format!(
            "entry {} cannot publish terminal status from {:?}",
            intent.entry_id, intent.phase
        )));
    }
    let status_at = intent.terminal_status_at.as_ref().ok_or_else(|| {
        DaemonError::Refused("workspace terminal status lost its durable timestamp".to_string())
    })?;
    let signed = status::publish_capture_closed(
        runtime.entry(),
        &daemon.inner.config.config_dir,
        runtime.artifact(),
        goalcup_protocol::control::parse_z(status_at.as_str()).ok_or_else(|| {
            DaemonError::Refused("workspace terminal status timestamp is invalid".to_string())
        })? as f64,
    )?;
    daemon.cache()?.insert(intent.entry_id.clone(), signed);
    #[cfg(any(test, feature = "test-hooks"))]
    if daemon
        .inner
        .fail_after_workspace_status_publish
        .swap(false, std::sync::atomic::Ordering::SeqCst)
    {
        return Err(DaemonError::Refused(
            "injected workspace finalization interruption after status publication".to_string(),
        ));
    }
    runtime.advance(
        FinalizationPhaseV1::TerminalStatusPending,
        FinalizationPhaseV1::Terminal,
        |staged| {
            staged.status_mirror_complete = true;
            Ok(())
        },
    )?;
    workspace_finish_report(runtime.entry())
}

/// The upload-only state a sealed row needs after a daemon restart.
///
/// A normal sealed runtime stays in the live map until its terminal envelope is
/// acknowledged.  A restart intentionally does not rebuild terminal runtimes: their
/// tailers and workspace routing are no longer valid.  This smaller owner retains just the
/// buffer, uploader and read-only terminal trace needed to finish that one outstanding
/// delivery, including the uploader's retry backoff.
pub(super) struct SealedDrain {
    entry: Entry,
    artifact: EntryArtifactRoot,
    buffer: Buffer,
    writer: TraceExport,
    uploader: Uploader,
    /// The final validator found terminal-invalid evidence but could not commit its monotonic
    /// quarantine row. The daemon must distinguish this transient unsafe state from an
    /// ordinary uploader error and withhold stale sealed status until it resolves.
    terminal_validation_persistence_failed: bool,
    /// A test-only post-uploader mutation seam. It is consumed after the real upload and
    /// durable registry ACK transaction but before [`Daemon::pump_recovered_sealed_buffers`]
    /// performs its final `is_drained` validation, covering the ordering race directly.
    #[cfg(any(test, feature = "test-hooks"))]
    post_pump_tail_for_test: Option<Envelope>,
    /// Test-only post-uploader final-validation faults. The phase bit stays false during
    /// `pump`'s pre-upload validation, so these seams cannot accidentally test an earlier
    /// refusal instead of the status/eviction decision after a successful upload.
    #[cfg(any(test, feature = "test-hooks"))]
    post_pump_final_validation_for_test: bool,
    #[cfg(any(test, feature = "test-hooks"))]
    fail_next_quarantine_persistence_for_test: bool,
    #[cfg(any(test, feature = "test-hooks"))]
    fail_next_durable_refresh_for_test: bool,
}

impl SealedDrain {
    #[cfg(any(test, feature = "test-hooks"))]
    fn out_of_window_limits(&self) -> Result<OutOfWindowLimitsV1, DaemonError> {
        let intent = self.entry.workspace_sealing_intent().ok_or_else(|| {
            DaemonError::Refused(format!(
                "sealed entry {} has no workspace finalization limits for a late post",
                self.entry.entry_id
            ))
        })?;
        Ok(super::entry::workspace_out_of_window_limits(intent))
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn out_of_window_stats_for_test(&self) -> Result<(u64, u64), DaemonError> {
        let stats = self
            .buffer
            .verify_out_of_window(self.out_of_window_limits()?)?;
        Ok((stats.records, stats.overflowed))
    }

    pub(super) fn rebuild(
        entry: Entry,
        config: &DaemonConfig,
        registry: &Registry,
        artifact_root: &ArtifactRoot,
    ) -> Result<Self, DaemonError> {
        validate_recovery_storage(&entry, config)?;
        let artifact = artifact_root.open_entry(&entry.entry_id).map_err(|error| {
                DaemonError::Refused(format!(
                    "sealed entry {} artifact root cannot be acquired through the daemon's retained authority: {error}",
                    entry.entry_id
                ))
            })?;
        let root = TraceRoot::from_acquired(
            artifact.open_trace().map_err(|error| {
                DaemonError::Refused(format!(
                    "sealed trace cannot be opened through retained entry artifact authority: {error}"
                ))
            })?,
            entry.trace_dir.clone(),
        );
        let root_identity = root.identity()?;
        let legacy_root_identity = entry.has_legacy_sealed_trace_root_identity();
        if !legacy_root_identity {
            entry.matches_sealed_trace_root(&root_identity)?;
        }
        let mut prepared = TraceExport::prepare_resume_from_root(root)?;
        let (terminal_seq, terminal_head) = prepared.head().ok_or_else(|| {
            DaemonError::Refused(format!(
                "sealed entry {} has no terminal chain head for its drain",
                entry.entry_id
            ))
        })?;
        if entry.chain_seq != Some(terminal_seq) || entry.chain_head != terminal_head {
            return Err(DaemonError::Refused(format!(
                "sealed entry {} registry head does not match its validated terminal trace",
                entry.entry_id
            )));
        }
        let mut last: Option<Envelope> = None;
        for line in BufReader::new(prepared.independent_chain_reader()?).lines() {
            let line = line.map_err(|source| DaemonError::Io {
                path: entry.trace_dir.join("chain.jsonl").display().to_string(),
                source,
            })?;
            last = Some(serde_json::from_str(&line)?);
        }
        let Some(terminal) = last.as_ref() else {
            return Err(DaemonError::Refused(format!(
                "sealed entry {} has no terminal envelope its buffer may acknowledge",
                entry.entry_id
            )));
        };
        match entry.sealing_intent() {
            Some(crate::registry::SealingIntent::Workspace(intent)) => {
                intent.matches_terminal_envelope(terminal)?;
            }
            Some(crate::registry::SealingIntent::Legacy(intent)) => {
                intent.matches_terminal(terminal)?;
            }
            None => {
                if !matches!(terminal.body, EventBody::Seal { .. }) {
                    return Err(DaemonError::Refused(format!(
                        "sealed entry {} has no durable intent for its terminal envelope",
                        entry.entry_id
                    )));
                }
            }
        }
        // Historical sealed rows predate trace-root identities.  Do not make their drain
        // depend on a current connector or a guessed path: only after the exact terminal
        // trace has validated against the registry head may this one-way migration bind the
        // root capability that was actually reopened.
        let entry = if legacy_root_identity {
            registry.record_legacy_sealed_trace_root_identity(
                &entry.entry_id,
                &root_identity,
                terminal,
            )?
        } else {
            entry
        };
        // A sealed drain appends acknowledgements only. It deliberately resumes the exact
        // validated export rather than constructing a `ChainWriter`: no current or
        // trace-pinned connector manifest, redactor, glob or regex participates in this
        // post-terminal path.
        let writer = TraceExport::resume_prepared(prepared)?;
        let buffer = Buffer::open_from_entry_artifact(&artifact, &entry.keys.buffer_key_hex)?;
        if let Some(intent) = entry.workspace_sealing_intent() {
            let stats = buffer
                .verify_out_of_window(super::entry::workspace_out_of_window_limits(intent))?;
            let validation = buffer.verify_out_of_window_batch_state()?;
            if stats.overflowed > 0 || validation.flags != 0 {
                return Err(DaemonError::Refused(format!(
                    "sealed entry {} has incomplete out-of-window evidence",
                    entry.entry_id
                )));
            }
        }
        let uploader = Uploader::new(
            entry.entry_id.clone(),
            UploadClient::new(config.upload_url.clone()),
            config.upload_chunk_max,
            entry.keys.clone(),
            config.server_public_key.clone(),
            config.clock_skew_seconds,
        );
        Ok(Self {
            entry,
            artifact,
            buffer,
            writer,
            uploader,
            terminal_validation_persistence_failed: false,
            #[cfg(any(test, feature = "test-hooks"))]
            post_pump_tail_for_test: None,
            #[cfg(any(test, feature = "test-hooks"))]
            post_pump_final_validation_for_test: false,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_next_quarantine_persistence_for_test: false,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_next_durable_refresh_for_test: false,
        })
    }

    pub(super) async fn pump(&mut self, registry: &Registry) -> Result<PumpOutcome, DaemonError> {
        #[cfg(any(test, feature = "test-hooks"))]
        {
            // A prior cadence may have completed an idle/backing-off pump. Its test phase
            // cannot make this cadence's pre-upload refresh look post-uploader.
            self.post_pump_final_validation_for_test = false;
        }
        self.refresh_durable_entry_for_pump(registry)?;
        self.ensure_buffer_is_capped_at_terminal(registry)?;
        let terminal = self.entry.chain_seq.ok_or_else(|| {
            DaemonError::Refused(format!(
                "sealed entry {} has no terminal sequence for its drain",
                self.entry.entry_id
            ))
        })?;
        let outcome = self
            .uploader
            .pump_through(&self.buffer, &mut self.writer, Some(terminal))
            .await?;
        if outcome.uploaded > 0 || outcome.acked_seq != self.entry.last_ack_seq {
            let acked = outcome.acked_seq;
            let acknowledged = acknowledgement_delta(self.entry.last_ack_seq, acked)?;
            let at = now_unix();
            self.entry = registry.update(&self.entry.entry_id, |entry| {
                // This drain only ever owns a terminal row.  It may move the verified
                // acknowledgement watermark, but cannot reopen or otherwise alter the
                // authority the completed seal left behind.
                entry.last_ack_seq = acked;
                entry.last_ack_at = Some(at);
                // A crash can commit Buffer::ack before this registry update.  On that
                // recovery pump there are no newly sent envelopes, but the durable
                // acknowledgement still advances the count that the signed status reports.
                entry.events_uploaded = entry.events_uploaded.saturating_add(acknowledged);
            })?;
        }
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(envelope) = self.post_pump_tail_for_test.take() {
            self.buffer.append(&envelope)?;
        }
        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.post_pump_final_validation_for_test = true;
        }
        Ok(outcome)
    }

    /// Re-read the terminal authority before *every* recovered pump. A retained drain is a
    /// cache for trusted handles and uploader backoff, not for a registry row: accepting a
    /// later impossible durable acknowledgement from that cache would overwrite the row
    /// this cadence was supposed to refuse.
    fn refresh_durable_entry_for_pump(&mut self, registry: &Registry) -> Result<(), DaemonError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.post_pump_final_validation_for_test
            && std::mem::take(&mut self.fail_next_durable_refresh_for_test)
        {
            return Err(DaemonError::Refused(
                "test-hooks: retained post-pump durable refresh failed".to_string(),
            ));
        }
        let entry = registry
            .get(&self.entry.entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {}", self.entry.entry_id)))?;
        if entry.state != EntryState::Sealed || entry.sealed_at.is_none() {
            return Err(DaemonError::Refused(format!(
                "sealed drain entry {} is no longer a sealed terminal row",
                self.entry.entry_id
            )));
        }
        let terminal = match entry.chain_seq {
            Some(terminal) => terminal,
            None => {
                let error = DaemonError::Refused(format!(
                    "sealed entry {} has no terminal sequence for its drain",
                    entry.entry_id
                ));
                self.quarantine_validation_failure(registry)?;
                return Err(error);
            }
        };
        if entry.last_ack_seq.is_some_and(|acked| acked > terminal) {
            let error = DaemonError::Refused(format!(
                "sealed entry {} acknowledges beyond terminal seq {terminal}",
                entry.entry_id
            ));
            self.quarantine_validation_failure(registry)?;
            return Err(error);
        }
        self.entry = entry;
        Ok(())
    }

    /// Persist a buffer/trace validation failure that cannot be chained after the seal, then
    /// retain the returned row so a caller can republish the honest capture-closed status.
    fn quarantine_validation_failure(&mut self, registry: &Registry) -> Result<(), DaemonError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.post_pump_final_validation_for_test
            && std::mem::take(&mut self.fail_next_quarantine_persistence_for_test)
        {
            self.terminal_validation_persistence_failed = true;
            return Err(DaemonError::Refused(
                "test-hooks: retained terminal quarantine persistence failed".to_string(),
            ));
        }
        match registry.record_terminal_drain_validation_failure(&self.entry.entry_id) {
            Ok(entry) => {
                self.entry = entry;
                self.terminal_validation_persistence_failed = false;
            }
            Err(error) => {
                self.terminal_validation_persistence_failed = true;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Refuse before any upload if the retained buffer can name a link after the terminal
    /// head. Nothing legitimately appends there after a restart; sending a
    /// contiguous prefix and silently evicting after the terminal acknowledgement would
    /// otherwise let an unvalidated tail reach the server on the next cadence.
    fn ensure_buffer_is_capped_at_terminal(
        &mut self,
        registry: &Registry,
    ) -> Result<(), DaemonError> {
        let terminal = self.entry.chain_seq.ok_or_else(|| {
            DaemonError::Refused(format!(
                "sealed entry {} has no terminal sequence for its drain",
                self.entry.entry_id
            ))
        })?;
        let validation = (|| {
            if self
                .entry
                .last_ack_seq
                .is_some_and(|acked| acked > terminal)
            {
                return Err(DaemonError::Refused(format!(
                    "sealed entry {} acknowledges beyond terminal seq {terminal}",
                    self.entry.entry_id
                )));
            }
            let floor = self.buffer.floor()?;
            if floor.is_some_and(|acked| acked > terminal) {
                return Err(DaemonError::Refused(format!(
                    "sealed entry {} buffer acknowledgement exceeds terminal seq {terminal}",
                    self.entry.entry_id
                )));
            }
            if self.entry.last_ack_seq.is_some_and(|registry_ack| {
                floor.is_none_or(|buffer_floor| buffer_floor < registry_ack)
            }) {
                return Err(DaemonError::Refused(format!(
                    "sealed entry {} registry acknowledgement is ahead of durable buffer floor",
                    self.entry.entry_id
                )));
            }
            self.buffer
                .ensure_no_records_after(terminal)
                .map_err(|error| {
                    DaemonError::Refused(format!(
                        "sealed entry {} cannot drain beyond its terminal: {error}",
                        self.entry.entry_id
                    ))
                })
        })();
        if validation.is_err() {
            self.quarantine_validation_failure(registry)?;
        }
        validation
    }

    pub(super) fn is_drained(&mut self, registry: &Registry) -> Result<bool, DaemonError> {
        // A failed marker write earlier in this same cadence is already enough to withhold
        // terminal truth. Do not let the post-pump confirmation retry it immediately and
        // overwrite that transient failure with a later durable review before the daemon
        // has installed its fail-closed latch. The next upload cadence explicitly begins a
        // fresh validation attempt.
        if self.terminal_validation_persistence_failed {
            return Err(DaemonError::Refused(format!(
                "sealed entry {} terminal quarantine persistence remains unclassified",
                self.entry.entry_id
            )));
        }
        self.refresh_durable_entry_for_pump(registry)?;
        self.ensure_buffer_is_capped_at_terminal(registry)?;
        // A clean retry can heal only the process-local withholding state; the daemon
        // releases that latch after a fresh clean completion reaches every public view.
        self.terminal_validation_persistence_failed = false;
        Ok(self.entry.chain_seq.is_some() && self.entry.last_ack_seq == self.entry.chain_seq)
    }

    pub(super) fn has_persisted_validation_failure(&self) -> bool {
        self.entry.terminal_drain_validation_failed
    }

    pub(super) fn terminal_validation_persistence_failed(&self) -> bool {
        self.terminal_validation_persistence_failed
    }

    /// The exact entry artifact authority acquired while this recovered drain was rebuilt.
    /// Terminal review/invalidation keeps using it while this owner remains retained.
    pub(super) fn artifact(&self) -> &EntryArtifactRoot {
        &self.artifact
    }

    /// Begin one daemon upload cadence.  A later cadence may retry a transient marker
    /// failure; within a cadence, [`Self::is_drained`] preserves it through the final
    /// validation decision so no ordinary terminal publication can overtake the latch.
    pub(super) fn begin_terminal_validation_cadence(&mut self) {
        self.terminal_validation_persistence_failed = false;
    }

    /// Publish an ordinary recovered terminal movement. Terminal-review rows deliberately do
    /// not use this path: the daemon installs their composed signed truth before attempting
    /// the fallible filesystem sinks.
    pub(super) fn publish_status(
        &self,
        config_dir: &std::path::Path,
    ) -> Result<goalcup_protocol::control::SignedStatus, DaemonError> {
        status::publish_capture_closed(&self.entry, config_dir, &self.artifact, now_unix())
    }

    /// Test-only: corrupt the buffer the real retained drain already owns. Opening a second
    /// redb owner would test a lock race rather than the terminal validation path.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn append_buffer_for_test(&self, envelope: &Envelope) -> Result<(), DaemonError> {
        self.buffer.append(envelope)
    }

    /// Test-only: stage a recovered buffer floor after daemon start, without changing the
    /// registry mirror. This is the inverse of the ordinary ACK-before-registry crash
    /// window and exercises the irreconcilable registry-ahead validation.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn acknowledge_buffer_for_test(&self, up_to_seq: u64) -> Result<(), DaemonError> {
        self.buffer.ack(up_to_seq).map(|_| ())
    }

    /// Test-only: put a record behind the real uploader's successful return and ahead of
    /// the daemon's final drain check. It proves status publication is ordered after that
    /// validation rather than merely testing a corruption visible before pumping starts.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn stage_post_pump_tail_for_test(&mut self, envelope: Envelope) {
        self.post_pump_tail_for_test = Some(envelope);
    }

    /// Test-only: fail the post-uploader durable refresh inside `is_drained` once.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn fail_next_post_pump_refresh_for_test(&mut self) {
        self.fail_next_durable_refresh_for_test = true;
    }

    /// Test-only: fail the post-uploader attempt to persist terminal quarantine once.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn fail_next_terminal_quarantine_persistence_for_test(&mut self) {
        self.fail_next_quarantine_persistence_for_test = true;
    }

    /// Test-only: discard retained uploader backoff so a deterministic post-uploader seam
    /// proves the successful upload/ACK path rather than an idle retry cadence.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn resume_upload_for_test(&mut self) {
        self.uploader.resume(self.entry.last_ack_seq);
    }
}

/// The number of chain events whose verified acknowledgement is newly reflected in the
/// registry.  `Buffer::ack` is durable before the registry transaction, so recovery must
/// derive this from watermarks rather than from this process's `uploaded` counter.
pub(super) fn acknowledgement_delta(
    previous: Option<u64>,
    current: Option<u64>,
) -> Result<u64, DaemonError> {
    match (previous, current) {
        (None, None) => Ok(0),
        (Some(_), None) => Err(DaemonError::Refused(
            "upload attempted to clear a verified acknowledgement watermark".to_string(),
        )),
        (Some(previous), Some(current)) if current < previous => Err(DaemonError::Refused(
            "upload attempted to move a verified acknowledgement backward".to_string(),
        )),
        (Some(previous), Some(current)) => Ok(current - previous),
        (None, Some(current)) => current.checked_add(1).ok_or_else(|| {
            DaemonError::Refused("upload acknowledgement range overflowed".to_string())
        }),
    }
}

/// What one seal did, beyond the manifest it published.
///
/// [`Daemon::seal_entry`] answers the control API with the [`SealManifest`] — that is the
/// document the participant and the verifier read — and keeps this beside it, because three
/// facts about a seal are not in the manifest and have nowhere else to go:
///
/// - **`lock_paths`**: the connector's `lock_paths[]`, resolved to
///   real paths. A resume that cannot take the harness's thread-writer lock silently becomes
///   a new thread, so the CLI checks them before relaunching, and the seal is where the run
///   last knew what they were.
/// - **`guard_degraded`**: the registered handle carried no argv, so only the age half of
///   the kill guard ran — or there was no handle to guard. A seal that signalled on a weaker
///   guard than usual, or on none at all, says so.
/// - **`termination`** / **`termination_error`**: whether the tree is actually gone. A
///   `kill.strategy = wrapper_unsafe` connector refuses termination by declaration, and a
///   handle whose pid was recycled is refused too — neither may look like a clean kill, and
///   neither may undo a seal that is already durable on disk.
#[derive(Debug, Clone)]
pub struct SealReport {
    pub entry_id: String,
    pub manifest: SealManifest,
    /// The connector's `lock_paths[]`, resolved through this entry's own path variables.
    /// Empty when the templates would not resolve — see `lock_paths_error`.
    pub lock_paths: Vec<String>,
    /// Why `lock_paths` is empty, when it is.
    ///
    /// **Resolving them may not abort a seal.** They are read off the manifest and the
    /// entry's path variables, they are advice for a later relaunch, and a connector whose
    /// template names a variable this daemon does not supply is a packaging mistake — none
    /// of which is a reason to leave an entry sealed with its harness still running. So the
    /// failure is recorded here and the seal carries on.
    pub lock_paths_error: Option<String>,
    /// True when the kill guard could not ask the identity question: the registered handle
    /// carried no argv, so only the age half ran — or there was no handle at all and no
    /// guard ran.
    pub guard_degraded: bool,
    /// The kill guard refused the registered pid — a recycled number, or a live process
    /// running an argv the launch never registered — so this seal cannot say what the
    /// harness is doing.
    ///
    /// A loud fact and not a silent one. Recorded in `termination_error` alone, such a
    /// refusal would be a sentence on a report nothing compares, beside a seal
    /// that completed, a manifest that was taken and a chain that was closed while the
    /// agent carried on editing with nothing recording it. This is the field a status
    /// document and a reviewer read instead.
    ///
    /// Never true beside a `termination` claiming `all_gone`: a tree nobody could identify
    /// is a tree nobody may claim is gone.
    pub harness_unverified: bool,
    #[cfg(unix)]
    pub termination: Option<crate::terminate::TerminationReport>,
    /// What the kill did, in one line, on every platform. The durable half of
    /// `termination`, which is unix-only.
    pub termination_summary: Option<String>,
    /// Why the harness was not terminated, when it was not. `None` on a clean kill and on
    /// an entry that never registered a process.
    pub termination_error: Option<String>,
}

impl SealReport {
    /// The row the registry keeps, so a daemon that restarts after this one died can read
    /// what the seal already did and finish what it did not.
    pub fn outcome(&self) -> SealOutcome {
        SealOutcome {
            lock_paths: self.lock_paths.clone(),
            lock_paths_error: self.lock_paths_error.clone(),
            guard_degraded: self.guard_degraded,
            harness_unverified: self.harness_unverified,
            termination: self.termination_summary.clone(),
            termination_error: self.termination_error.clone(),
        }
    }
}

/// What ending one harness cost, in the one shape both endings report.
///
/// The seal is one caller; the reap a starting daemon runs over a `Sealed` entry whose
/// harness outlived the process that sealed it is the other. Neither may claim a tree is
/// gone that is not, so a refusal — `wrapper_unsafe`, a recycled pid, a platform with no
/// verified path — comes back as `error` and never as a quiet success.
struct Ending {
    guard_degraded: bool,
    /// The guard refused the pid: nothing in the tree was signalled by number, and this
    /// seal cannot vouch for the harness. See [`SealReport::harness_unverified`].
    harness_unverified: bool,
    #[cfg(unix)]
    report: Option<crate::terminate::TerminationReport>,
    summary: Option<String>,
    error: Option<String>,
}

/// Terminate one registered harness per its `kill.strategy` (D22) and say what that cost.
fn end_harness(handle: &ProcessHandle) -> Ending {
    #[cfg(unix)]
    match crate::terminate::terminate(handle) {
        Ok(report) => Ending {
            guard_degraded: report.guard_degraded,
            harness_unverified: report.harness_unverified,
            summary: Some(format!(
                "strategy={:?} targeted={:?} survived_term={:?} killed={:?} \
                 still_alive={:?} all_gone={} harness_unverified={} waited_ms={}",
                report.strategy,
                report.targeted,
                report.survived_term,
                report.killed,
                report.still_alive,
                report.all_gone,
                report.harness_unverified,
                report.waited_ms
            )),
            report: Some(report),
            error: None,
        },
        // `NoTarget` is the guard's own refusal — a recycled pid, or one running an argv
        // the launch never registered — and it is the one error that means nothing was
        // signalled while the seal completes anyway. Every other failure here is a
        // kill this daemon attempted and could not finish, which `termination_error`
        // already says.
        Err(e) => Ending {
            guard_degraded: false,
            harness_unverified: matches!(e, ProcessError::NoTarget(_)),
            report: None,
            summary: None,
            error: Some(e.to_string()),
        },
    }
    // A platform other than unix has no verified termination path. Saying so is the honest
    // report; claiming the tree is gone is not.
    #[cfg(not(unix))]
    Ending {
        guard_degraded: false,
        harness_unverified: true,
        summary: None,
        error: Some(format!(
            "this platform has no verified termination path in M1: pid {} was not \
             signalled",
            handle.pid
        )),
    }
}

/// The recovery paths (`abandon` and a cancelled prepared resume) are allowed to discard a
/// durable handoff only after this returns.  Unlike an ordinary seal, those paths have no
/// safe "record the uncertainty and continue" outcome: removing the row after a guarded
/// refusal would orphan the child.  A process already absent is safely dead; a live process
/// must have a verified all-gone termination report.
pub(super) fn end_harness_verified(handle: &ProcessHandle) -> Result<(), DaemonError> {
    // `liveness` and not `is_alive`, so a `/bin/ps` that could not be spawned fails closed
    // as "could not ask" rather than being folded to "absent": both refuse to delete the
    // authority, but only one of them is honest about why.
    match liveness(handle.pid) {
        Ok(true) => {}
        Ok(false) => {
            // `terminate` reports an absent root as the ordinary *seal* case, where it is fine
            // to record that no target remains.  Abandon/cancel are stricter: a TUI wrapper can
            // exit after orphaning its native child, and with no live root there is no ancestry
            // from which this daemon can prove that child gone.  No persisted handle currently
            // carries a complete independently-verifiable listener/lock witness, so fail closed
            // rather than delete the only authority that can still name the incident.
            return Err(DaemonError::Refused(format!(
                "registered process pid {} is absent, so its descendant tree cannot be verified gone before authority removal",
                handle.pid
            )));
        }
        Err(error) => {
            return Err(DaemonError::Refused(format!(
                "could not ask whether registered process pid {} is alive ({error}), so its descendant tree cannot be verified gone before authority removal",
                handle.pid
            )));
        }
    }
    let ending = end_harness(handle);
    #[cfg(unix)]
    let verified = ending
        .report
        .as_ref()
        .is_some_and(|report| report.all_gone && !report.harness_unverified);
    #[cfg(not(unix))]
    let verified = false;
    if verified {
        Ok(())
    } else {
        Err(DaemonError::Refused(format!(
            "registered process pid {} could not be verified gone before authority removal{}",
            handle.pid,
            ending
                .error
                .as_deref()
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )))
    }
}

/// A crash between `write_seal` and `terminate` leaves the harness alive with nothing to
/// reap it: the seal is durable on disk, the entry is `Sealed` in the registry, and the
/// process that was going to end the tree is gone. This is the daemon that comes next
/// finishing the job.
///
/// Run at startup, over the registry and before any entry is opened, because the fact it
/// acts on outlives every handle in memory. An absent root pid is not proof a TUI wrapper's
/// native or orphan descendant is gone, so that case becomes a durable reap-pending
/// unverified outcome rather than a silent skip.
pub(super) fn reap_sealed_harnesses(
    registry: &Registry,
    config: &DaemonConfig,
) -> Result<(), DaemonError> {
    #[cfg(not(any(test, feature = "test-hooks")))]
    let _ = config;
    #[cfg(any(test, feature = "test-hooks"))]
    if take_reap_failure_for_test(&config.artifact_root) {
        return Err(DaemonError::Refused(
            "test-hooks: startup sealed-harness reaper failed".to_string(),
        ));
    }
    for entry in registry.all()? {
        if entry.state != EntryState::Sealed {
            continue;
        }
        // **A V4 row is out of this reaper's scope entirely.** The workspace finalization
        // intent is the ending authority for such an entry, and there is no outcome it can
        // hold that this legacy pass may act on:
        //
        // * `Unresolved` or `AlreadyStopped` — reaping would silently manufacture a
        //   termination after the signed trace had already said none was confirmed;
        // * `Confirmed` — the finalization already ended the tree and recorded the sweep it
        //   read that from, so a second pass finds an absent root and can only write a
        //   contradictory "reap remains pending / unverified" note about a process that is
        //   gone because this daemon killed it. That note would reach the signed status document
        //   through `SealReport::harness_unverified`, and a clean run would describe itself as
        //   unverified the first time the daemon restarted after it;
        // * no outcome yet — `recover_sealing_entries_for_config` resumes that intent and
        //   performs the termination itself, and two passes signalling one tree is a race.
        if entry.workspace_sealing_intent().is_some() {
            continue;
        }
        // An attached/not-required row can honestly have no termination target. A
        // `Registered` row cannot: that is a damaged authority record, and silently
        // skipping it would present a sealed row as reaped while a child may survive.
        if entry.last_process.is_none() && !entry.is_registered_without_termination_handle() {
            continue;
        }
        if entry.is_registered_without_termination_handle() {
            note_unverified_reap(
                registry,
                &entry,
                "the sealed row is registered without a persisted termination handle; preserving it for manual recovery",
            )?;
            continue;
        }
        let handle = match entry.verified_registered_termination_handle() {
            Ok(Some(handle)) => handle,
            Ok(None) => {
                note_unverified_reap(
                    registry,
                    &entry,
                    "the sealed row names a process but has no verified registered handoff authority; preserving it for manual recovery",
                )?;
                continue;
            }
            Err(error) => {
                note_unverified_reap(
                    registry,
                    &entry,
                    &format!(
                        "the sealed row's process authority cannot be verified; preserving it for manual recovery: {error}"
                    ),
                )?;
                continue;
            }
        };
        match liveness(handle.pid) {
            Ok(true) => {}
            Ok(false) => {
                note_unverified_reap(
                    registry,
                    &entry,
                    "the sealed process root is already absent; its native or orphan descendants cannot be proven gone, so reap remains pending",
                )?;
                continue;
            }
            // A `/bin/ps` that could not be spawned is not evidence the root is gone. Keep the
            // reap pending rather than folding "could not ask" to "already absent".
            Err(error) => {
                note_unverified_reap(
                    registry,
                    &entry,
                    &format!(
                        "could not ask whether the sealed process root is alive ({error}); reap remains pending"
                    ),
                )?;
                continue;
            }
        }
        let ending = end_harness(&handle);
        // The seal's own findings are kept: `lock_paths` were resolved by the process that
        // sealed, this one has no runtime to resolve them from, and overwriting them with
        // an empty list would turn a recorded answer into a missing one.
        let previous = entry.seal_outcome.clone();
        let outcome = SealOutcome {
            lock_paths: previous
                .as_ref()
                .map(|o| o.lock_paths.clone())
                .unwrap_or_default(),
            lock_paths_error: previous.as_ref().and_then(|o| o.lock_paths_error.clone()),
            guard_degraded: ending.guard_degraded,
            // The reap's answer and not the seal's: this is the process that actually
            // signalled the survivor, so what its guard made of the pid is the current fact
            // about a tree the earlier seal left running.
            harness_unverified: ending.harness_unverified,
            termination: ending.summary,
            termination_error: ending.error,
        };
        registry.update(&entry.entry_id, |e| e.seal_outcome = Some(outcome.clone()))?;
    }
    Ok(())
}

/// Test-only: fail exactly the next reaper for this artifact root.  The root key makes
/// parallel test daemons independent; the flag is consumed before the reaper touches a row.
#[cfg(any(test, feature = "test-hooks"))]
pub(super) fn fail_next_reap_for_test(artifact_root: &Path) {
    reap_failure_roots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(artifact_root.to_path_buf());
}

#[cfg(any(test, feature = "test-hooks"))]
fn take_reap_failure_for_test(artifact_root: &Path) -> bool {
    reap_failure_roots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(artifact_root)
}

#[cfg(any(test, feature = "test-hooks"))]
fn reap_failure_roots() -> &'static Mutex<BTreeSet<PathBuf>> {
    static ROOTS: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();
    ROOTS.get_or_init(|| Mutex::new(BTreeSet::new()))
}

#[cfg(any(test, feature = "test-hooks"))]
fn note_workspace_recovery_writer_prepare_for_test(artifact_root: &Path) {
    let mut calls = workspace_recovery_writer_prepare_calls_for_test()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    *calls.entry(artifact_root.to_path_buf()).or_default() += 1;
}

#[cfg(any(test, feature = "test-hooks"))]
fn workspace_recovery_writer_prepare_call_count_for_test(artifact_root: &Path) -> u64 {
    workspace_recovery_writer_prepare_calls_for_test()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(artifact_root)
        .copied()
        .unwrap_or_default()
}

#[cfg(any(test, feature = "test-hooks"))]
fn workspace_recovery_writer_prepare_calls_for_test() -> &'static Mutex<BTreeMap<PathBuf, u64>> {
    static CALLS: OnceLock<Mutex<BTreeMap<PathBuf, u64>>> = OnceLock::new();
    CALLS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn note_unverified_reap(
    registry: &Registry,
    entry: &Entry,
    reason: &str,
) -> Result<(), DaemonError> {
    let previous = entry.seal_outcome.clone();
    let outcome = SealOutcome {
        lock_paths: previous
            .as_ref()
            .map(|outcome| outcome.lock_paths.clone())
            .unwrap_or_default(),
        lock_paths_error: previous
            .as_ref()
            .and_then(|outcome| outcome.lock_paths_error.clone()),
        guard_degraded: true,
        harness_unverified: true,
        termination: None,
        termination_error: Some(reason.to_string()),
    };
    registry.update(&entry.entry_id, |row| {
        row.seal_outcome = Some(outcome.clone())
    })?;
    Ok(())
}

/// The outcome of inspecting one durable sealing intent.  `Restored` means no terminal
/// chain link exists and the old open lifecycle is safe again; `Sealed` means every store
/// names the one staged terminal link.  Any error deliberately leaves `Sealing` in place
/// and is capture-closed.
#[derive(Debug)]
pub(super) enum SealingRecovery {
    Restored,
    Sealed,
}

struct WorkspaceRecoveryOwner<'a> {
    entry: Entry,
    registry: &'a Registry,
    artifact: EntryArtifactRoot,
    buffer: Buffer,
    writer: TraceExport,
}

fn workspace_recovery_genesis_hash(
    entry: &Entry,
    trace_dir: &TraceDir,
) -> Result<String, DaemonError> {
    let chain = trace_dir
        .read_regular(CHAIN_FILE)
        .map_err(|error| {
            DaemonError::Refused(format!(
                "entry {} workspace recovery chain cannot be opened read-only: {error}",
                entry.entry_id
            ))
        })?
        .ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {} workspace recovery chain is absent",
                entry.entry_id
            ))
        })?;
    let mut reader = BufReader::new(chain);
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|source| DaemonError::Io {
            path: entry.trace_dir.join(CHAIN_FILE).display().to_string(),
            source,
        })?
        == 0
    {
        return Err(DaemonError::Refused(format!(
            "entry {} workspace recovery chain has no genesis link",
            entry.entry_id
        )));
    }
    let envelope: Envelope = serde_json::from_str(line.trim_end_matches(['\r', '\n']))?;
    if envelope.link.seq != 0 || envelope.link.prev != goalcup_protocol::chain::GENESIS {
        return Err(DaemonError::Refused(format!(
            "entry {} workspace recovery chain does not begin at genesis",
            entry.entry_id
        )));
    }
    let expected = envelope
        .expected_hash(goalcup_protocol::chain::GENESIS)
        .map_err(|error| {
            DaemonError::Refused(format!(
                "entry {} workspace recovery genesis link cannot be verified: {error}",
                entry.entry_id
            ))
        })?;
    if envelope.link.hash != expected {
        return Err(DaemonError::Refused(format!(
            "entry {} workspace recovery genesis hash is invalid",
            entry.entry_id
        )));
    }
    Ok(envelope.link.hash)
}

impl<'a> WorkspaceRecoveryOwner<'a> {
    fn open(
        registry: &'a Registry,
        entry: Entry,
        config: &DaemonConfig,
        artifact_root: &ArtifactRoot,
    ) -> Result<Self, DaemonError> {
        validate_recovery_storage(&entry, config)?;
        let intent = entry.workspace_sealing_intent().ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {} has no workspace finalization intent",
                entry.entry_id
            ))
        })?;
        intent.validate_recovery_authority(&entry)?;
        entry
            .sealing_intent()
            .ok_or_else(|| {
                DaemonError::Refused(format!(
                    "entry {} lost its workspace finalization authority",
                    entry.entry_id
                ))
            })?
            .validate(&entry.entry_id)?;
        let artifact = artifact_root.open_entry(&entry.entry_id).map_err(|error| {
            DaemonError::Refused(format!(
                "workspace recovery for entry {} cannot reacquire its artifact root: {error}",
                entry.entry_id
            ))
        })?;
        let trace_dir = artifact.open_trace().map_err(|error| {
            DaemonError::Refused(format!(
                "workspace recovery for entry {} cannot reacquire its trace root: {error}",
                entry.entry_id
            ))
        })?;
        let trace_root_identity = trace_dir.identity().map_err(|error| {
            DaemonError::Refused(format!(
                "entry {} workspace recovery trace root identity cannot be read: {error}",
                entry.entry_id
            ))
        })?;
        if trace_root_identity != intent.trace_root_identity {
            return Err(DaemonError::Refused(format!(
                "entry {} workspace recovery trace root no longer matches its durable intent",
                entry.entry_id
            )));
        }
        let authoritative_genesis_hash = workspace_recovery_genesis_hash(&entry, &trace_dir)?;
        if let Err(error) = crate::receipt::validate_authoritative_receipt(
            &entry,
            &authoritative_genesis_hash,
            &config.server_public_key,
        ) {
            registry.record_terminal_drain_validation_failure(&entry.entry_id)?;
            return Err(error);
        }
        let root = TraceRoot::from_acquired(trace_dir, entry.trace_dir.clone());
        #[cfg(any(test, feature = "test-hooks"))]
        note_workspace_recovery_writer_prepare_for_test(&config.artifact_root);
        let mut prepared = TraceExport::prepare_resume_from_root(root)?;
        let mut registry_head_found =
            entry.chain_seq.is_none() && entry.chain_head == goalcup_protocol::chain::GENESIS;
        let mut validated_genesis_hash = None;
        for line in BufReader::new(prepared.independent_chain_reader()?).lines() {
            let envelope: Envelope =
                serde_json::from_str(&line.map_err(|source| DaemonError::Io {
                    path: entry.trace_dir.join("chain.jsonl").display().to_string(),
                    source,
                })?)?;
            if envelope.link.seq == 0 {
                validated_genesis_hash = Some(envelope.link.hash.clone());
            }
            if entry.chain_seq == Some(envelope.link.seq) {
                if entry.chain_head != envelope.link.hash {
                    return Err(DaemonError::Refused(format!(
                        "entry {} workspace recovery registry hash does not match its validated chain",
                        entry.entry_id
                    )));
                }
                registry_head_found = true;
            }
        }
        if !registry_head_found {
            return Err(DaemonError::Refused(format!(
                "entry {} workspace recovery registry head is absent from its validated chain",
                entry.entry_id
            )));
        }
        let validated_genesis_hash = validated_genesis_hash.ok_or_else(|| {
            DaemonError::Refused(format!(
                "entry {} workspace recovery chain has no genesis link",
                entry.entry_id
            ))
        })?;
        if validated_genesis_hash != authoritative_genesis_hash {
            return Err(DaemonError::Refused(format!(
                "entry {} workspace recovery genesis changed while writer authority was acquired",
                entry.entry_id
            )));
        }
        let writer = TraceExport::resume_prepared(prepared)?;
        let buffer = Buffer::open_from_entry_artifact(&artifact, &entry.keys.buffer_key_hex)?;
        Ok(Self {
            entry,
            registry,
            artifact,
            buffer,
            writer,
        })
    }

    fn verify_out_of_window(&self) -> Result<(), DaemonError> {
        let intent = self.entry.workspace_sealing_intent().ok_or_else(|| {
            DaemonError::Refused(format!(
                "sealing entry {} lost its workspace finalization limits",
                self.entry.entry_id
            ))
        })?;
        let stats = self
            .buffer
            .verify_out_of_window(super::entry::workspace_out_of_window_limits(intent))?;
        let validation = self.buffer.verify_out_of_window_batch_state()?;
        if stats.overflowed > 0 || validation.flags != 0 {
            return Err(DaemonError::Refused(format!(
                "sealing entry {} has incomplete out-of-window evidence",
                self.entry.entry_id
            )));
        }
        Ok(())
    }

    fn buffer(&self) -> &Buffer {
        &self.buffer
    }
}

impl WorkspaceFinalizationOwner for WorkspaceRecoveryOwner<'_> {
    fn entry(&self) -> &Entry {
        &self.entry
    }

    fn drain_admitted(
        &mut self,
        _watermarks: &crate::registry::AdmissionWatermarksV1,
        _expired: &mut dyn FnMut() -> bool,
    ) -> goalcup_protocol::provenance::PreCutoffDrainOutcomeV1 {
        goalcup_protocol::provenance::PreCutoffDrainOutcomeV1::Degraded {
            reason: goalcup_protocol::provenance::PreCutoffDrainFailureV1::RestartStateAmbiguous,
        }
    }

    fn writer_head(&mut self) -> Result<Option<(u64, String)>, DaemonError> {
        Ok(self.writer.head())
    }

    fn read_chain_line(&mut self, seq: u64) -> Result<Envelope, DaemonError> {
        for line in BufReader::new(self.writer.independent_chain_reader()?).lines() {
            let envelope: Envelope = serde_json::from_str(&line.map_err(|source| {
                DaemonError::Io {
                    path: self
                        .entry
                        .trace_dir
                        .join("chain.jsonl")
                        .display()
                        .to_string(),
                    source,
                }
            })?)?;
            if envelope.link.seq == seq {
                return Ok(envelope);
            }
        }
        Err(DaemonError::Missing(format!(
            "workspace recovery chain has no sequence {seq}"
        )))
    }

    fn write_cutoff(
        &mut self,
        recorded_at: &str,
        cutoff: CutoffReachedV1,
    ) -> Result<goalcup_protocol::chain::Link, DaemonError> {
        Ok(self
            .writer
            .write_cutoff(&self.entry.entry_id, recorded_at, cutoff)?)
    }

    fn write_finalization(
        &mut self,
        recorded_at: &str,
        finalization: FinalizationV1,
    ) -> Result<goalcup_protocol::chain::Link, DaemonError> {
        Ok(self
            .writer
            .write_finalization(&self.entry.entry_id, recorded_at, finalization)?)
    }

    fn retain_boundary(&mut self, envelope: &Envelope) -> Result<(), DaemonError> {
        self.buffer.append(envelope)
    }

    fn trace(&self) -> Result<goalcup_platform::tracecap::TraceDir, DaemonError> {
        self.artifact
            .open_trace()
            .map_err(|_| DaemonError::SubmissionCapture {
                reason: SubmissionFailureV1::StorageFailed,
            })
    }

    fn advance<F>(
        &mut self,
        expected: FinalizationPhaseV1,
        next: FinalizationPhaseV1,
        update: F,
    ) -> Result<(), DaemonError>
    where
        F: FnOnce(&mut crate::registry::WorkspaceSealingIntentV1) -> Result<(), DaemonError>,
    {
        self.entry = self.registry.advance_workspace_finalization(
            &self.entry.entry_id,
            expected,
            next,
            update,
        )?;
        Ok(())
    }

    fn artifact(&self) -> &EntryArtifactRoot {
        &self.artifact
    }
}

/// Reconcile one exact seal intent.  The registry plan is the authority for *which* seal
/// can finish: a validated but different terminal (including a future finalization) is
/// never used to reopen or promote this entry.
#[cfg(test)]
pub(super) fn reconcile_sealing_entry(
    registry: &Registry,
    entry: &Entry,
    config: Option<&DaemonConfig>,
) -> Result<SealingRecovery, DaemonError> {
    reconcile_sealing_entry_on_restart(registry, entry, config, None)
}

fn reconcile_sealing_entry_on_restart(
    registry: &Registry,
    entry: &Entry,
    config: Option<&DaemonConfig>,
    artifact_root: Option<&ArtifactRoot>,
) -> Result<SealingRecovery, DaemonError> {
    if entry.state != EntryState::Sealing || entry.sealed_at.is_some() {
        return Err(DaemonError::Refused(format!(
            "entry {} has no unresolved sealing intent",
            entry.entry_id
        )));
    }
    let intent = entry.sealing_intent().cloned().ok_or_else(|| {
        DaemonError::Refused(format!(
            "entry {} is sealing without a staged exact manifest",
            entry.entry_id
        ))
    })?;
    intent.validate(&entry.entry_id)?;

    let artifact = match (config, artifact_root) {
        (Some(_), Some(artifact_root)) => Some(artifact_root.open_entry(&entry.entry_id).map_err(
            |error| {
                DaemonError::Refused(format!(
                    "sealing entry {} artifact root cannot be acquired through the daemon's retained authority: {error}",
                    entry.entry_id
                ))
            },
        )?),
        (Some(config), None) => Some(
            EntryArtifactRoot::open(&config.artifact_root, &entry.entry_id).map_err(|error| {
                DaemonError::Refused(format!(
                    "sealing entry {} artifact root cannot be acquired without following a swapped component: {error}",
                    entry.entry_id
                ))
            })?,
        ),
        (None, _) => None,
    };
    let root = match &artifact {
        Some(artifact) => TraceRoot::from_acquired(
            artifact.open_trace().map_err(|error| {
                DaemonError::Refused(format!(
                    "sealing trace cannot be opened through retained entry artifact authority: {error}"
                ))
            })?,
            entry.trace_dir.clone(),
        ),
        None => TraceRoot::open(&entry.trace_dir)?,
    };
    let mut prepared = TraceExport::prepare_resume_from_root(root)?;
    intent.matches_trace_root(&prepared.root_identity()?)?;
    let mut terminal: Option<Envelope> = None;
    let reader = prepared.independent_chain_reader()?;
    for line in BufReader::new(reader).lines() {
        let env: Envelope = serde_json::from_str(&line.map_err(|source| DaemonError::Io {
            path: entry.trace_dir.join("chain.jsonl").display().to_string(),
            source,
        })?)?;
        if matches!(
            env.body,
            EventBody::Seal { .. } | EventBody::FinalizationV1(_)
        ) && terminal.replace(env).is_some()
        {
            return Err(DaemonError::Refused(format!(
                "entry {} has more than one terminal link while its seal intent is unresolved",
                entry.entry_id
            )));
        }
    }
    let Some(terminal) = terminal else {
        // This is startup-only. It owns no old writer, so after the exact root identity
        // and predecessor head have both validated it may restore the pre-seal lifecycle
        // for a new runtime to acquire. A valid ordinary tail, truncation, or replacement
        // at the same root cannot be reopened merely because it has no terminal seal.
        // Same-process failures never call this path.
        intent.matches_pre_seal_head(prepared.head())?;
        return registry
            .restore_unfinished_sealing(&entry.entry_id)
            .map(|_| SealingRecovery::Restored);
    };
    // Reject any other terminal kind before touching the buffer or fixed documents.
    intent.matches_terminal(&terminal)?;

    // Finish the three material stores in replay-safe order. Buffer::append replaces an
    // identical sequence, and the fixed-document replacements are atomic; repeating after
    // an interruption therefore converges on exactly this envelope and manifest.
    open_recovery_buffer(entry, config, artifact.as_ref())?.append(&terminal)?;
    let export = TraceExport::resume_prepared(prepared)?;
    let legacy = intent.legacy()?;
    export.repair_seal(&legacy.manifest, &terminal)?;
    export.write_descriptor(&legacy.descriptor)?;
    let staged = registry.record_sealing_terminal_head(&entry.entry_id, &terminal)?;
    let promoted = registry.promote_sealing_terminal(&entry.entry_id, &terminal)?;
    debug_assert_eq!(staged.chain_seq, promoted.chain_seq);
    Ok(SealingRecovery::Sealed)
}

/// Production sealing recovery receives the configured root and therefore acquires its
/// buffer through the platform's rooted capability.  The unit-only no-config path
/// is confined to synthetic same-process registry fixtures; the real restart path never
/// trusts `Entry::artifact_dir` as an openable capability.
fn open_recovery_buffer(
    entry: &Entry,
    config: Option<&DaemonConfig>,
    artifact: Option<&EntryArtifactRoot>,
) -> Result<Buffer, DaemonError> {
    match (config, artifact) {
        (Some(_), Some(artifact)) => {
            Buffer::open_from_entry_artifact(artifact, &entry.keys.buffer_key_hex)
        }
        (Some(_), None) => Err(DaemonError::Refused(
            "production sealing recovery lost its retained entry artifact authority".to_string(),
        )),
        (None, _) => Buffer::open(
            &Buffer::default_path(&entry.artifact_dir),
            &entry.keys.buffer_key_hex,
        ),
    }
}

/// Reconcile every interrupted row without letting one corrupt trace prevent unrelated
/// entries from restarting. The caller records the returned per-entry failures in its cycle
/// counters; the rows themselves remain `Sealing`, quarantined and capture-closed.
#[cfg(test)]
pub(super) fn recover_sealing_entries(
    registry: &Registry,
) -> Result<Vec<(String, DaemonError)>, DaemonError> {
    let mut failures = Vec::new();
    for entry in registry.all()? {
        if entry.state != EntryState::Sealing {
            continue;
        }
        if let Err(error) = reconcile_sealing_entry(registry, &entry, None) {
            failures.push((entry.entry_id, error));
        }
    }
    Ok(failures)
}

/// The production startup variant rejects rows whose persisted paths are not exactly the
/// paths this daemon derives from its configured artifact root and entry id *before* any
/// trace or buffer handle is opened. Registry JSON is durable state, not a capability to
/// redirect recovery through a symlink or an outside artifact tree.
pub(super) fn recover_sealing_entries_for_config(
    daemon: &Daemon,
    registry: &Registry,
    config: &DaemonConfig,
    artifact_root: &ArtifactRoot,
) -> Result<Vec<(String, DaemonError)>, DaemonError> {
    let mut failures = Vec::new();
    for entry in registry.all()? {
        let workspace_unfinished = entry.workspace_sealing_intent().is_some_and(|intent| {
            intent.phase != FinalizationPhaseV1::Terminal
                && matches!(entry.state, EntryState::Sealing | EntryState::Sealed)
        });
        if entry.state != EntryState::Sealing && !workspace_unfinished {
            continue;
        }
        let result = validate_recovery_storage(&entry, config).and_then(|_| {
            if entry.workspace_sealing_intent().is_some() {
                let mut owner =
                    WorkspaceRecoveryOwner::open(registry, entry.clone(), config, artifact_root)?;
                if let Err(error) = owner.verify_out_of_window() {
                    registry.record_terminal_drain_validation_failure(&entry.entry_id)?;
                    return Err(error);
                }
                advance_workspace_runtime(daemon, &mut owner, None)?;
                let milliseconds = owner
                    .entry()
                    .workspace_sealing_intent()
                    .ok_or_else(|| {
                        DaemonError::Refused(format!(
                            "entry {} lost its finalization limits during recovery",
                            entry.entry_id
                        ))
                    })?
                    .limits
                    .pre_cutoff_drain;
                let deadline = daemon.finalization_now_unix() + milliseconds as f64 / 1_000.0;
                let limits = super::entry::workspace_out_of_window_limits(
                    owner.entry().workspace_sealing_intent().ok_or_else(|| {
                        DaemonError::Refused(format!(
                            "entry {} lost its finalization limits during recovery",
                            entry.entry_id
                        ))
                    })?,
                );
                let retained = daemon
                    .recovery_tailer_cutover(owner.entry(), owner.artifact(), owner.buffer())
                    .and_then(|mut tailers| {
                        tailers.retain(owner.buffer(), limits, || {
                            daemon.finalization_now_unix() >= deadline
                        })
                    });
                if let Err(error) = retained {
                    registry.record_terminal_drain_validation_failure(&entry.entry_id)?;
                    return Err(error);
                }
                complete_workspace_terminal(daemon, &mut owner).map(|_| SealingRecovery::Sealed)
            } else {
                reconcile_sealing_entry_on_restart(
                    registry,
                    &entry,
                    Some(config),
                    Some(artifact_root),
                )
            }
        });
        if let Err(error) = result {
            failures.push((entry.entry_id, error));
        }
    }
    Ok(failures)
}

/// Validate every terminal buffer during startup, including rows whose registry watermark
/// already equals the terminal head.  The recovered upload cadence keeps the same check;
/// this earlier pass prevents a daemon from publishing a stale all-clear cache and serving
/// a corrupt sealed row until its first upload tick.
pub(super) fn validate_sealed_drains_for_config(
    registry: &Registry,
    config: &DaemonConfig,
    artifact_root: &ArtifactRoot,
) -> Result<Vec<(String, DaemonError, bool)>, DaemonError> {
    let mut failures = Vec::new();
    for entry in registry.all()? {
        if entry.state != EntryState::Sealed {
            continue;
        }
        let result = validate_recovery_storage(&entry, config)
            .and_then(|_| SealedDrain::rebuild(entry.clone(), config, registry, artifact_root))
            .and_then(|mut drain| drain.is_drained(registry).map(|_| ()));
        if let Err(error) = result {
            match registry.record_terminal_drain_validation_failure(&entry.entry_id) {
                Ok(_) => failures.push((entry.entry_id, error, false)),
                Err(persistence_error) => {
                    // Bootstrap has no Daemon latch yet, but it does have the exact
                    // configured roots. Revoke only safe cache views before it later starts
                    // with the transient latch; never leave raw MCP readers on old all-clear
                    // bytes merely because marker persistence itself was unavailable.
                    let invalidation = status::invalidate_terminal_views_with_root(
                        &entry,
                        &config.config_dir,
                        artifact_root,
                    );
                    let reported = match invalidation {
                        Ok(()) => persistence_error,
                        Err(invalidation_error) => DaemonError::Refused(format!(
                            "sealed entry {} terminal quarantine persistence failed and its \
                             cache invalidation also failed: {}; {}",
                            entry.entry_id, persistence_error, invalidation_error
                        )),
                    };
                    failures.push((entry.entry_id, reported, true));
                }
            }
        }
    }
    Ok(failures)
}

fn validate_recovery_storage(entry: &Entry, config: &DaemonConfig) -> Result<(), DaemonError> {
    let id_path = std::path::Path::new(&entry.entry_id);
    if id_path.components().count() != 1
        || !matches!(
            id_path.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err(DaemonError::Refused(format!(
            "entry {} has an unsafe identifier for artifact recovery",
            entry.entry_id
        )));
    }
    let artifact_dir = config.artifact_root.join("artifacts").join(&entry.entry_id);
    let trace_dir = artifact_dir.join("trace");
    if entry.artifact_dir != artifact_dir || entry.trace_dir != trace_dir {
        return Err(DaemonError::Refused(format!(
            "entry {} storage paths do not match this daemon's configured artifact root",
            entry.entry_id
        )));
    }
    if entry.workspace_sealing_intent().is_some()
        && entry.receipt_binding() != &config.receipt_binding
    {
        return Err(DaemonError::Refused(format!(
            "entry {} workspace finalization authority does not match this daemon's receipt policy",
            entry.entry_id
        )));
    }
    // These registry paths are exact-match diagnostics only. Production callers acquire
    // root → artifacts → entry → trace through `EntryArtifactRoot`; do not turn this
    // comparison into a second pathname probe whose result another thread could swap.
    Ok(())
}

impl Daemon {
    /// Interrupt the next workspace finalizer immediately after `phase` is durable.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_workspace_finalization_after_for_test(&self, phase: FinalizationPhaseV1) {
        *self
            .inner
            .workspace_finalization_fault
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(phase);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn workspace_recovery_writer_prepare_calls_for_test(&self) -> u64 {
        workspace_recovery_writer_prepare_call_count_for_test(&self.inner.config.artifact_root)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_before_workspace_intent_cas_for_test(
        &self,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        self.with_entry(entry_id, |runtime| {
            runtime.fail_before_workspace_intent_cas_for_test()
        })
    }

    /// Interrupt after the immutable writer resolves, before its outcome reaches the row.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_after_workspace_capture_for_test(&self) {
        self.inner
            .fail_after_workspace_capture
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_after_workspace_cutoff_append_for_test(&self) {
        self.inner
            .fail_after_workspace_cutoff_append
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_after_workspace_finalization_append_for_test(&self) {
        self.inner
            .fail_after_workspace_finalization_append
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_after_workspace_status_publish_for_test(&self) {
        self.inner
            .fail_after_workspace_status_publish
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Pause a V4 finalizer at the filesystem capture boundary. The runtime is already
    /// durably `Sealing` and borrowed out of its slot when the pause is reached.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn pause_workspace_capture_for_test(
        &self,
        entry_id: &str,
    ) -> Result<super::LiveSealWritePause, DaemonError> {
        self.with_entry(entry_id, |runtime| {
            runtime.arm_workspace_capture_pause_for_test()
        })?
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn pause_workspace_intent_cas_for_test(
        &self,
        entry_id: &str,
    ) -> Result<super::LiveSealWritePause, DaemonError> {
        self.with_entry(entry_id, |runtime| {
            runtime.arm_workspace_intent_cas_pause_for_test()
        })?
    }

    /// Finish one V4 workspace run through the single durable manual/deadline state
    /// machine. Repeated callers join the persisted intent and return the same outcomes;
    /// legacy entries continue through [`Self::seal_entry`].
    pub fn finish_entry(
        &self,
        entry_id: &str,
        reason: FinishReasonV1,
    ) -> Result<FinishReportV1, DaemonError> {
        self.finish_entry_with_source(entry_id, reason, true)
    }

    pub(super) fn finish_entry_with_source(
        &self,
        entry_id: &str,
        reason: FinishReasonV1,
        use_live_source: bool,
    ) -> Result<FinishReportV1, DaemonError> {
        let durable = self
            .inner
            .registry
            .get(entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        if durable
            .workspace_sealing_intent()
            .is_some_and(|intent| intent.phase == FinalizationPhaseV1::Terminal)
        {
            return workspace_finish_report(&durable);
        }
        if !matches!(
            durable.receipt_binding(),
            crate::config::ReceiptBindingPolicy::WorkspaceSubmission { .. }
        ) {
            return Err(DaemonError::Refused(format!(
                "entry {entry_id} is not configured for workspace-submission finalization"
            )));
        }

        let admission = self.admission_guard();
        let mut held = self.borrow_entry(entry_id)?;
        let source = held.get()?.begin_workspace_finalization(
            reason,
            || self.finalization_now_unix(),
            use_live_source,
        )?;
        drop(admission);
        let advanced = advance_workspace_runtime(self, held.get()?, source);
        let post_cutoff_millis = held
            .get()?
            .entry()
            .workspace_sealing_intent()
            .map(|intent| intent.limits.pre_cutoff_drain)
            .unwrap_or(0);
        let post_cutoff_deadline =
            self.finalization_now_unix() + post_cutoff_millis as f64 / 1_000.0;
        let mut post_cutoff_expired = || self.finalization_now_unix() >= post_cutoff_deadline;
        let tailers_retained = held
            .get()?
            .retain_post_cutoff_tailers(&mut post_cutoff_expired);

        // The terminal CAS and the return-to-slot run under the same latch as admission.
        let admission = self.admission_guard();
        let persistence_failed = held.get()?.terminal_validation_persistence_failed();
        let latched = if persistence_failed {
            self.latch_terminal_status_before_owner_release(entry_id)
        } else {
            Ok(())
        };
        let completed = if advanced.is_ok() && tailers_retained.is_ok() && latched.is_ok() {
            Some(complete_workspace_terminal(self, held.get()?))
        } else {
            None
        };
        held.return_to_slot();
        drop(admission);
        let invalidated = if persistence_failed && latched.is_ok() {
            self.finish_withholding_terminal_status(entry_id)
        } else {
            Ok(())
        };
        advanced?;
        tailers_retained?;
        latched?;
        invalidated?;
        completed.expect("successful prerequisites create terminal completion")
    }

    /// Finalize one overdue rebuilt runtime before the listener or the ordinary
    /// rehydration map exists. The runtime never becomes routable: on any failure the
    /// caller drops it and withholds the stale pre-restart status for this process.
    pub(super) fn finish_overdue_workspace_runtime(
        &self,
        runtime: &mut EntryRuntime,
    ) -> Result<FinishReportV1, DaemonError> {
        let admission = self.admission_guard();
        let source = runtime.begin_workspace_finalization(
            FinishReasonV1::Deadline,
            || self.finalization_now_unix(),
            false,
        )?;
        drop(admission);
        advance_workspace_runtime(self, runtime, source)?;
        let milliseconds = runtime
            .entry()
            .workspace_sealing_intent()
            .map(|intent| intent.limits.pre_cutoff_drain)
            .unwrap_or(0);
        let deadline = self.finalization_now_unix() + milliseconds as f64 / 1_000.0;
        let mut expired = || self.finalization_now_unix() >= deadline;
        runtime.retain_post_cutoff_tailers(&mut expired)?;
        let admission = self.admission_guard();
        let report = complete_workspace_terminal(self, runtime)?;
        drop(admission);
        Ok(report)
    }

    /// The seal, in the one order both callers share.
    ///
    /// The workspace manifest, then the second `foreign::scan_once`, then `write_seal` —
    /// which chains `EventBody::Seal` and then writes `seal.json`, in that order, inside the
    /// protocol crate — and only then `terminate`, per the connector's `kill.strategy`.
    /// The first three are [`EntryRuntime::seal`]; this method adds the kill and the
    /// bookkeeping around it.
    ///
    /// **The manifest is taken before the kill**, because the reverse races the harness's
    /// own final writes: a harness that flushes on SIGTERM writes files this walk would then
    /// have missed. And **the kill runs after the seal is durable**, so a termination that
    /// fails — `wrapper_unsafe`, a recycled pid, a tree that outlived its grace — is
    /// recorded in the [`SealReport`] rather than undoing a seal that is already on disk and
    /// already in the chain.
    ///
    /// After it returns the entry's chain is closed: every further `event`, `leg1` or `leg2`
    /// is `ExportError::Sealed`, which is why the entry is moved to `EntryState::Sealed`
    /// before it returns and why [`Daemon::tick`] then skips it. A second call is refused —
    /// a seal is the entry's last link, and there is no second last link.
    pub fn seal_entry(&self, entry_id: &str) -> Result<SealManifest, DaemonError> {
        // The refusal and the seal under one hold of the slot lock. Asking `is_sealed`
        // first and taking the lock afterwards is a check the winner of a race passes twice:
        // two callers both read "not sealed", both take the lock in turn, and the loser
        // reaches `EntryRuntime::seal` and meets `ExportError::Sealed` from the export
        // instead of the friendly refusal this method exists to give.
        // The slot is taken once, here, and the handle to it is kept for the whole method.
        // `Daemon::slot` looks the entry up in the live map, and a concurrent
        // pump's `evict_if_drained` takes a drained sealed entry out of that map — which it
        // may do while `end_harness` below is sleeping out a ten-second grace. Looking the
        // slot up again at the bottom would get `Missing`, which would be swallowed, so the
        // last document a participant could read would predate the seal it
        // was published to announce. The `Arc` outlives the map entry; the runtime inside it
        // is still there and still this entry's.
        let slot = self.slot(entry_id)?;
        // Keep all pre-terminal failures inside a closure result. A bare block would let
        // `?` return from `seal_entry` before the quarantine publisher below runs, leaving
        // the old signed live status beside a durable capture-closed `Sealing` row.
        let sealing = (|| -> Result<_, DaemonError> {
            let mut held = slot.runtime.lock().unwrap_or_else(|e| e.into_inner());
            let runtime = held.as_mut().ok_or_else(|| held_elsewhere(entry_id))?;
            if runtime.entry().state == EntryState::Refused {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} was terminally refused and cannot be sealed: its \
                     preserved trace is not a started run"
                )));
            }
            if runtime.entry().state == EntryState::Sealed || runtime.entry().sealed_at.is_some() {
                return Err(DaemonError::Refused(format!(
                    "entry {entry_id} is already sealed: `EventBody::Seal` is the chain's \
                    last link and a second seal would have nothing to link onto"
                )));
            }
            // A daemon crash can leave a durable `Prepared` process handoff after its chain
            // marker/buffer write and before `Registered`.  Reconcile it while this same
            // slot is held, before the seal reads `last_process`; ambiguity refuses the
            // seal rather than allowing an accepted child to escape its termination target.
            runtime.reconcile_prepared_process_handoff()?;
            // **Resolved before the seal, and never allowed to abort one.** It reads the
            // manifest and this entry's path variables and touches nothing else, so there
            // is no reason for it to run after `seal()` — and every reason not to: a
            // template this daemon cannot render would leave the entry sealed, the
            // harness alive, and no report anywhere saying either.
            let (lock_paths, lock_paths_error) = match runtime.lock_paths() {
                Ok(paths) => (paths, None),
                Err(e) => (Vec::new(), Some(path_free(&e))),
            };
            // Did this entry ever owe a process handle? Read here, under the same hold, so
            // the `None` arm below can tell "nothing was ever launched" from "the handle
            // was lost". A launch-mode connector that has started is the case where
            // a missing handle is a loss.
            let expected_a_handle = runtime.owes_a_process_handle();
            let manifest = runtime.seal()?;
            Ok::<_, DaemonError>((
                manifest,
                runtime.entry().last_process.clone(),
                expected_a_handle,
                lock_paths,
                lock_paths_error,
            ))
        })();
        let (manifest, handle, expected_a_handle, lock_paths, lock_paths_error) = match sealing {
            Ok(sealed) => sealed,
            Err(error) => {
                // A live writer can fail after the durable intent closed capture but
                // before it appends a terminal link. Publish that quarantine now; a
                // timer may never get another chance if the caller immediately stops.
                let published = {
                    let mut held = slot.runtime.lock().unwrap_or_else(|e| e.into_inner());
                    match held.as_mut() {
                        Some(runtime) if runtime.entry().state == EntryState::Sealing => runtime
                            .publish_status(
                                &self.inner.config.config_dir,
                                &self.inner.config.artifact_root,
                                now_unix(),
                            )
                            .ok(),
                        _ => None,
                    }
                };
                if let Some(signed) = published {
                    self.cache()?.insert(entry_id.to_string(), signed);
                    self.refresh_current()?;
                }
                return Err(error);
            }
        };

        // From here nothing may return early: the seal is durable on disk and in the chain,
        // and the harness is still running. The kill and the report are what is left.
        let ending = match &handle {
            Some(handle) => end_harness(handle),
            // **A started launch-mode entry with no handle is a handle that was lost**, not an
            // entry with nothing to verify. No shipped v1 connector is
            // `attach_only` — both are `mode = "tui"` — and three ordinary routes
            // reach this state without leaving a mark: a resume that cleared the slot whose
            // relaunch then failed to register, a `set_process` whose registry write did not
            // land before a restart, and a daemon that died between `chain_home_seed` and
            // `set_process`. This seal signalled nothing, so it cannot say the harness is gone —
            // and the silent arm below would report exactly what a verified clean kill reports.
            None if expected_a_handle => Ending {
                guard_degraded: true,
                harness_unverified: true,
                #[cfg(unix)]
                report: None,
                summary: None,
                error: Some(
                    "no process handle was registered for this launch-mode entry, so this \
                     seal signalled nothing and cannot say whether the harness is still \
                     running"
                        .to_string(),
                ),
            },
            // Attach mode, or an entry that never started: there was never a process to
            // verify and nothing outlived the seal. Silence here is the honest answer.
            None => Ending {
                guard_degraded: false,
                harness_unverified: false,
                #[cfg(unix)]
                report: None,
                summary: None,
                error: None,
            },
        };
        let report = SealReport {
            entry_id: entry_id.to_string(),
            manifest: manifest.clone(),
            lock_paths,
            lock_paths_error,
            guard_degraded: ending.guard_degraded,
            harness_unverified: ending.harness_unverified,
            #[cfg(unix)]
            termination: ending.report,
            termination_summary: ending.summary,
            termination_error: ending.error,
        };
        let outcome = report.outcome();
        self.seals()?.insert(entry_id.to_string(), report);
        // …and on the entry, so a daemon that restarts after this process dies can read
        // what this seal did instead of reaping a harness it has no record of.
        //
        self.inner
            .registry
            .update(entry_id, |e| e.seal_outcome = Some(outcome.clone()))?;
        // …and into the runtime's copy of that row, which is what the publication below
        // builds its document from. Written only to the registry, the runtime would go on
        // answering `status` from the row it had read before the kill — so
        // `harness_unverified` would be true on disk and false on the document a participant
        // reads about the run that has just finished.
        //
        // Ignored when the runtime is not in its slot: another call may hold it, and the row
        // above is the durable answer either way. Nothing after the seal may turn into a
        // failure of it.
        //
        // Test-hooks only: `skip_next_note_seal_outcome` stands in for a concurrent eviction,
        // deterministically, by skipping this call outright rather than racing a pump for it.
        // `EntryRuntime::status_at`'s fallback to the row above is what a status built
        // afterwards then depends on.
        #[cfg(any(test, feature = "test-hooks"))]
        let skip_note = self
            .inner
            .skip_next_note_seal_outcome
            .swap(false, std::sync::atomic::Ordering::SeqCst);
        #[cfg(not(any(test, feature = "test-hooks")))]
        let skip_note = false;

        // The surfaces last: the status document now says the run is sealed, and the
        // current-entry pointer no longer names it.
        //
        // This entry's own publication, not the whole cadence. `refresh_statuses`
        // still publishes a sealed entry, but it is a cadence on its own timer, and the
        // document that says the run is sealed must not wait for one.
        //
        // **Through the slot this call has held all along** and not through `with_entry`: a
        // pump that evicted the entry during the kill takes it out of the live map, and a
        // lookup here would answer `Missing` and be swallowed, which is how a
        // stale document would survive. The note and the publication share the one slot — they
        // take its lock one after the other, not under one hold — so on this thread's
        // ordering no status is built from a runtime that has not been told how the seal
        // ended, and no lookup can answer `Missing` in between.
        if !skip_note {
            let mut held = slot.runtime.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(runtime) = held.as_mut() {
                runtime.note_seal_outcome(outcome.clone());
            }
        }
        let _ = self.publish_one(entry_id, &slot);
        Ok(manifest)
    }

    /// [`Daemon::seal_entry`] on a blocking worker, counted so [`Daemon::stop`] can wait
    /// for it.
    ///
    /// **The blocking worker is not optional.** `seal_entry` walks the workspace, writes
    /// the chain and then calls `terminate`, which sleeps out the handle's `grace_seconds`
    /// — up to ten on the shipped connectors — whenever the harness survives its SIGTERM.
    /// Run inline it is an `async` handler holding a runtime worker for seconds, and on a
    /// single-threaded runtime the status page and every other request stop for the length
    /// of a kill.
    ///
    /// **And it is not abortable.** `spawn_blocking` runs a synchronous closure: a `stop`
    /// arriving mid-seal cannot interrupt it, and a `stop` that returned over it would release
    /// nothing — the worker holds an `Arc<Daemon>`, so the redb registry and buffer locks would
    /// outlive the stop that was supposed to release them, and a restart-in-place would then
    /// fail to open a database it had been told was free. The permit this holds is what `stop`
    /// waits to take back, and it is dropped when the closure ends, including when it
    /// unwinds.
    ///
    /// The signature is the shape a handler wants: the outer `Result` is the worker's own
    /// (a panic, a runtime shutting down), the inner one is the seal's.
    pub async fn seal_entry_blocking(
        &self,
        entry_id: &str,
    ) -> Result<Result<SealManifest, DaemonError>, tokio::task::JoinError> {
        let permit = self.inner.blocking.clone().acquire_owned().await;
        let daemon = Daemon {
            inner: self.inner.clone(),
        };
        let entry_id = entry_id.to_string();
        tokio::task::spawn_blocking(move || {
            let sealed = daemon.seal_entry(&entry_id);
            drop(permit);
            sealed
        })
        .await
    }

    pub async fn finish_entry_blocking(
        &self,
        entry_id: &str,
        reason: FinishReasonV1,
    ) -> Result<Result<FinishReportV1, DaemonError>, tokio::task::JoinError> {
        let permit = self.inner.blocking.clone().acquire_owned().await;
        let daemon = Daemon {
            inner: self.inner.clone(),
        };
        let entry_id = entry_id.to_string();
        tokio::task::spawn_blocking(move || {
            let finished = daemon.finish_entry(&entry_id, reason);
            drop(permit);
            finished
        })
        .await
    }

    /// What the seal learned beyond the manifest: the connector's `lock_paths[]`, whether
    /// the kill guard was degraded, and whether the tree is actually gone.
    pub fn seal_report(&self, entry_id: &str) -> Result<Option<SealReport>, DaemonError> {
        Ok(self.seals()?.get(entry_id).cloned())
    }

    /// The paths a resume check has to find free: the durable row when there is one,
    /// and the connector's own templates when there is not.
    ///
    /// Not [`Daemon::seal_report`], which reads this process's memory. A daemon that
    /// restarted after the seal holds nothing in that map, so every path the check would
    /// have asked about is vacuously free — and a restart is exactly when a participant
    /// resumes. [`Daemon::seal_entry`] writes the resolved paths into the registry row
    /// (`SealOutcome::lock_paths`), which is the copy that survives this process.
    ///
    /// An entry with no `SealOutcome` has not sealed, and its lock is whatever the
    /// connector's `lock_paths[]` render to now: a harness killed without a seal leaves its
    /// lock on disk with no row anywhere saying so, and a relaunch over it is the new thread
    /// the resume check exists to refuse.
    pub fn resume_lock_paths(&self, entry_id: &str) -> Result<Vec<String>, DaemonError> {
        if let Some(outcome) = self
            .inner
            .registry
            .get(entry_id)?
            .and_then(|e| e.seal_outcome)
        {
            return Ok(outcome.lock_paths);
        }
        self.with_entry(entry_id, |r| r.lock_paths())?
    }

    /// Whether this entry's chain is closed — asked of the open runtime when there is one,
    /// and of the registry row when there is not.
    ///
    /// The fallback is not a convenience. [`Daemon::rehydrate`] deliberately leaves sealed
    /// rows out of the entry map, so on a restarted daemon a sealed entry has no runtime to
    /// ask, and without the row this would answer `DaemonError::Missing` — the resume route
    /// would then tell a participant "no such entry" about a run whose seal is on disk in
    /// front of them, and the resume refusal would be a `404` instead of the `409` it is. An id
    /// no row names is still `Missing`: that one really is no such entry.
    pub fn is_sealed(&self, entry_id: &str) -> Result<bool, DaemonError> {
        let sealed = |state: EntryState, sealed_at: &Option<String>| {
            state.capture_closed() || sealed_at.is_some()
        };
        // The guard is dropped by the end of this statement, on purpose: `with_entry` takes
        // the same lock, `std::sync::Mutex` is not reentrant, and a `match` on the call
        // itself would hold the map across both arms and deadlock the first time anything
        // asked whether an open entry was sealed.
        let open = self.entry_map()?.contains_key(entry_id);
        match open {
            true => self.with_entry(entry_id, |r| sealed(r.entry().state, &r.entry().sealed_at)),
            false => match self.inner.registry.get(entry_id)? {
                Some(entry) => Ok(sealed(entry.state, &entry.sealed_at)),
                None => Err(DaemonError::Missing(format!(
                    "no entry {entry_id} in this daemon"
                ))),
            },
        }
    }

    pub(super) fn seals(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, SealReport>>, DaemonError> {
        Ok(self.inner.seals.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[cfg(test)]
mod sealing_recovery_tests {
    use super::*;
    use crate::config::ReceiptBindingPolicy;
    use crate::registry::{Entry, ExpectedProcessHandoff, RunKeys, SealingIntent};
    use goalcup_protocol::export::{
        ACKS_FILE, BASELINE_FILE, CHAIN_FILE, MANIFEST_FILE, SEAL_FILE, TraceDescriptorDoc,
        TraceExport,
    };

    fn entry(root: &std::path::Path, id: &str) -> Entry {
        Entry::new(
            id,
            "claude-code-cli",
            root.join("workspace"),
            root.join("artifact").join(id),
            &"ab".repeat(32),
            "2.1.259",
            ReceiptBindingPolicy::Legacy,
            ExpectedProcessHandoff::NotRequired,
            RunKeys::from_seeds([1; 32], [2; 32], [3; 16]),
        )
    }

    fn manifest(entry: &Entry) -> SealManifest {
        SealManifest {
            entry_id: entry.entry_id.clone(),
            sealed_at: "2026-09-11T00:00:00Z".to_string(),
            workspace: entry.workspace.display().to_string(),
            files: Vec::new(),
            excluded: Vec::new(),
        }
    }

    fn intent(
        entry: &Entry,
        manifest: &SealManifest,
        pre_seal_head: Option<(u64, String)>,
    ) -> SealingIntent {
        SealingIntent::new(
            manifest.clone(),
            TraceDescriptorDoc {
                connector: entry.connector_id.clone(),
                entry_id: entry.entry_id.clone(),
                workspace: entry.workspace.display().to_string(),
                tracked_sessions: Vec::new(),
                harness_version: entry.harness_version.clone(),
                connector_manifest: MANIFEST_FILE.to_string(),
                chain: Some(CHAIN_FILE.to_string()),
                receipt: None,
                baseline: Some(BASELINE_FILE.to_string()),
                acks: Some(ACKS_FILE.to_string()),
                seal: Some(SEAL_FILE.to_string()),
                server_public_key: None,
                expected_reread: false,
            },
            pre_seal_head,
            TraceRoot::open(&entry.trace_dir)
                .and_then(|root| root.identity())
                .unwrap_or_else(|_| "unopened-test-root".to_string()),
        )
        .unwrap()
    }

    fn append_ordinary(writer: &mut TraceExport, entry: &Entry, at: &str) {
        let mut heartbeat = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: entry.entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: at.to_string(),
            link: goalcup_protocol::chain::Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Heartbeat {
                ts: at.to_string(),
                stores_seen: Vec::new(),
                receiver_ok: true,
            },
            redaction: None,
        };
        writer.append_chain(&mut heartbeat).unwrap();
    }

    #[test]
    fn terminal_sealing_intent_repairs_the_exact_buffer_envelope_before_promotion() {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(&root.path().join("registry.redb")).unwrap();
        let entry = entry(root.path(), "seal-recovery");
        registry.put(&entry).unwrap();
        let manifest = manifest(&entry);
        let mut writer = TraceExport::create(&entry.trace_dir).unwrap();
        writer.write_baseline(&manifest).unwrap();
        let pre_seal_head = writer.head();
        drop(writer);

        registry
            .begin_sealing(&entry.entry_id, intent(&entry, &manifest, pre_seal_head))
            .unwrap();
        let mut resumed = TraceExport::resume(&entry.trace_dir).unwrap();
        resumed.write_seal(&manifest).unwrap();
        drop(resumed);
        // Simulate the crash window after the chained terminal append but before the fixed
        // manifest/descriptor repair. Recovery must reconstruct them from the staged exact
        // intent, never from a loose digest or current connector state.
        std::fs::remove_file(entry.trace_dir.join(SEAL_FILE)).unwrap();

        recover_sealing_entries(&registry).unwrap();
        let recovered = registry.get(&entry.entry_id).unwrap().unwrap();
        assert_eq!(recovered.state, EntryState::Sealed);
        assert_eq!(
            recovered.sealed_at.as_deref(),
            Some(manifest.sealed_at.as_str())
        );
        assert!(recovered.sealing_from_state.is_none());
        let repaired_manifest: SealManifest =
            serde_json::from_slice(&std::fs::read(entry.trace_dir.join(SEAL_FILE)).unwrap())
                .unwrap();
        assert_eq!(repaired_manifest, manifest);
        let repaired_descriptor: TraceDescriptorDoc =
            toml::from_str(&std::fs::read_to_string(entry.trace_dir.join("trace.toml")).unwrap())
                .unwrap();
        assert_eq!(repaired_descriptor.seal.as_deref(), Some(SEAL_FILE));

        let buffer = Buffer::open(
            &Buffer::default_path(&entry.artifact_dir),
            &entry.keys.buffer_key_hex,
        )
        .unwrap();
        let repaired = buffer.read_from(Some(0), 8).unwrap();
        assert_eq!(repaired.envelopes.len(), 1);
        assert!(matches!(repaired.envelopes[0].body, EventBody::Seal { .. }));

        // Repeat after the promotion: an idempotent restart does not rewrite a terminal
        // row or create a second buffered terminal envelope.
        recover_sealing_entries(&registry).unwrap();
        assert_eq!(buffer.read_from(Some(0), 8).unwrap().envelopes.len(), 1);
    }

    #[test]
    fn mismatched_terminal_never_promotes_or_reopens_a_staged_intent() {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(&root.path().join("registry.redb")).unwrap();
        let entry = entry(root.path(), "wrong-terminal");
        let staged = manifest(&entry);
        registry.put(&entry).unwrap();
        let mut writer = TraceExport::create(&entry.trace_dir).unwrap();
        writer.write_baseline(&staged).unwrap();
        let pre_seal_head = writer.head();
        drop(writer);
        registry
            .begin_sealing(&entry.entry_id, intent(&entry, &staged, pre_seal_head))
            .unwrap();
        let mut writer = TraceExport::resume(&entry.trace_dir).unwrap();
        let mut different = staged.clone();
        different.sealed_at = "2026-09-11T00:00:01Z".to_string();
        writer.write_seal(&different).unwrap();

        let failures = recover_sealing_entries(&registry).unwrap();
        assert_eq!(failures.len(), 1);
        let row = registry.get(&entry.entry_id).unwrap().unwrap();
        assert_eq!(row.state, EntryState::Sealing);
        assert!(row.sealed_at.is_none());
        assert!(row.sealing_intent().is_some());
    }

    #[test]
    fn no_terminal_ordinary_tail_at_the_staged_root_stays_quarantined() {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(&root.path().join("registry.redb")).unwrap();
        let entry = entry(root.path(), "ordinary-tail");
        let staged = manifest(&entry);
        registry.put(&entry).unwrap();
        let mut writer = TraceExport::create(&entry.trace_dir).unwrap();
        writer.write_baseline(&staged).unwrap();
        let pre_seal_head = writer.head();
        drop(writer);
        registry
            .begin_sealing(&entry.entry_id, intent(&entry, &staged, pre_seal_head))
            .unwrap();

        let mut replacement = TraceExport::resume(&entry.trace_dir).unwrap();
        append_ordinary(&mut replacement, &entry, "2026-09-11T00:00:01Z");
        drop(replacement);

        let failures = recover_sealing_entries(&registry).unwrap();
        assert_eq!(failures.len(), 1);
        let row = registry.get(&entry.entry_id).unwrap().unwrap();
        assert_eq!(row.state, EntryState::Sealing);
        assert!(row.sealing_intent().is_some());
    }

    #[test]
    fn no_terminal_truncated_history_at_the_staged_root_stays_quarantined() {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(&root.path().join("registry.redb")).unwrap();
        let entry = entry(root.path(), "truncated-history");
        let staged = manifest(&entry);
        registry.put(&entry).unwrap();
        let mut writer = TraceExport::create(&entry.trace_dir).unwrap();
        writer.write_baseline(&staged).unwrap();
        let baseline = root.path().join("baseline-chain.jsonl");
        std::fs::copy(entry.trace_dir.join(CHAIN_FILE), &baseline).unwrap();
        append_ordinary(&mut writer, &entry, "2026-09-11T00:00:01Z");
        let pre_seal_head = writer.head();
        drop(writer);
        registry
            .begin_sealing(&entry.entry_id, intent(&entry, &staged, pre_seal_head))
            .unwrap();

        std::fs::copy(baseline, entry.trace_dir.join(CHAIN_FILE)).unwrap();
        let failures = recover_sealing_entries(&registry).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(
            registry.get(&entry.entry_id).unwrap().unwrap().state,
            EntryState::Sealing
        );
    }

    #[test]
    fn no_terminal_valid_replacement_history_at_the_staged_root_stays_quarantined() {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(&root.path().join("registry.redb")).unwrap();
        let entry = entry(root.path(), "replacement-no-terminal");
        let staged = manifest(&entry);
        registry.put(&entry).unwrap();
        let mut writer = TraceExport::create(&entry.trace_dir).unwrap();
        writer.write_baseline(&staged).unwrap();
        append_ordinary(&mut writer, &entry, "2026-09-11T00:00:01Z");
        let pre_seal_head = writer.head();
        drop(writer);
        registry
            .begin_sealing(&entry.entry_id, intent(&entry, &staged, pre_seal_head))
            .unwrap();

        let alternate = root.path().join("valid-replacement");
        let mut replacement = TraceExport::create(&alternate).unwrap();
        replacement.write_baseline(&staged).unwrap();
        append_ordinary(&mut replacement, &entry, "2026-09-11T00:00:02Z");
        drop(replacement);
        std::fs::copy(alternate.join(CHAIN_FILE), entry.trace_dir.join(CHAIN_FILE)).unwrap();

        let failures = recover_sealing_entries(&registry).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(
            registry.get(&entry.entry_id).unwrap().unwrap().state,
            EntryState::Sealing
        );
    }

    #[test]
    fn replacement_history_with_the_same_seal_body_never_overwrites_the_staged_head() {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(&root.path().join("registry.redb")).unwrap();
        let entry = entry(root.path(), "swapped-history");
        let staged = manifest(&entry);
        registry.put(&entry).unwrap();

        // Stage a terminal link extending this exact one-link history.
        let mut original = TraceExport::create(&entry.trace_dir).unwrap();
        original.write_baseline(&staged).unwrap();
        let pre_seal_head = original.head();
        drop(original);
        registry
            .begin_sealing(&entry.entry_id, intent(&entry, &staged, pre_seal_head))
            .unwrap();

        // Replace the chain pathname with a separately valid history for the same entry
        // and the exact same terminal body. It has one extra ordinary link, so its seal
        // is self-consistent but does not extend the predecessor the live writer staged.
        let alternate = root.path().join("alternate-trace");
        let mut replacement = TraceExport::create(&alternate).unwrap();
        replacement.write_baseline(&staged).unwrap();
        let mut heartbeat = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: entry.entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-11T00:00:00Z".to_string(),
            link: goalcup_protocol::chain::Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Heartbeat {
                ts: "2026-09-11T00:00:00Z".to_string(),
                stores_seen: Vec::new(),
                receiver_ok: true,
            },
            redaction: None,
        };
        replacement.append_chain(&mut heartbeat).unwrap();
        replacement.write_seal(&staged).unwrap();
        drop(replacement);
        std::fs::copy(alternate.join(CHAIN_FILE), entry.trace_dir.join(CHAIN_FILE)).unwrap();

        let failures = recover_sealing_entries(&registry).unwrap();
        assert_eq!(failures.len(), 1);
        let row = registry.get(&entry.entry_id).unwrap().unwrap();
        assert_eq!(row.state, EntryState::Sealing);
        assert!(row.sealed_at.is_none());
        // Recovery has not adopted the replacement chain's head into the registry.
        assert_eq!(row.chain_seq, None);
        assert_eq!(row.chain_head, goalcup_protocol::chain::GENESIS);
    }

    #[test]
    fn replacement_trace_root_with_an_exact_terminal_is_not_promoted() {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(&root.path().join("registry.redb")).unwrap();
        let entry = entry(root.path(), "swapped-root");
        let staged = manifest(&entry);
        registry.put(&entry).unwrap();

        let mut original = TraceExport::create(&entry.trace_dir).unwrap();
        original.write_baseline(&staged).unwrap();
        let pre_seal_head = original.head();
        drop(original);
        registry
            .begin_sealing(&entry.entry_id, intent(&entry, &staged, pre_seal_head))
            .unwrap();

        // An attacker can make another directory carry byte-for-byte valid baseline and
        // terminal links. Link identity alone then passes, so recovery also binds the
        // directory capability identity staged before the original terminal append.
        let alternate = root.path().join("alternate-root");
        let mut replacement = TraceExport::create(&alternate).unwrap();
        replacement.write_baseline(&staged).unwrap();
        replacement.write_seal(&staged).unwrap();
        drop(replacement);
        let original_path = root.path().join("original-trace");
        std::fs::rename(&entry.trace_dir, &original_path).unwrap();
        std::fs::rename(&alternate, &entry.trace_dir).unwrap();

        let failures = recover_sealing_entries(&registry).unwrap();
        assert_eq!(failures.len(), 1);
        let row = registry.get(&entry.entry_id).unwrap().unwrap();
        assert_eq!(row.state, EntryState::Sealing);
        assert!(row.sealed_at.is_none());
        assert_eq!(row.chain_seq, None);
    }

    #[test]
    fn one_corrupt_sealing_row_does_not_block_another_rows_recovery() {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(&root.path().join("registry.redb")).unwrap();
        let bad = entry(root.path(), "bad-sealing");
        let mut good = entry(root.path(), "good-sealing");
        good.workspace = root.path().join("workspace-good");
        registry.put(&bad).unwrap();
        registry.put(&good).unwrap();
        let bad_manifest = manifest(&bad);
        let good_manifest = manifest(&good);
        registry
            .begin_sealing(&bad.entry_id, intent(&bad, &bad_manifest, None))
            .unwrap();
        let mut writer = TraceExport::create(&good.trace_dir).unwrap();
        writer.write_baseline(&good_manifest).unwrap();
        let pre_seal_head = writer.head();
        drop(writer);
        registry
            .begin_sealing(&good.entry_id, intent(&good, &good_manifest, pre_seal_head))
            .unwrap();
        let mut writer = TraceExport::resume(&good.trace_dir).unwrap();
        writer.write_seal(&good_manifest).unwrap();

        let failures = recover_sealing_entries(&registry).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, bad.entry_id);
        assert_eq!(
            registry.get(&bad.entry_id).unwrap().unwrap().state,
            EntryState::Sealing
        );
        assert_eq!(
            registry.get(&good.entry_id).unwrap().unwrap().state,
            EntryState::Sealed
        );
    }
}
