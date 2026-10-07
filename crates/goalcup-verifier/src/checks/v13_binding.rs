//! V13: the trace is bound to a start the participant did not sign.
//!
//! Every check before this one reads the run's *contents*. This one reads its **frame**:
//! who declared the run, when it was declared to start, when it was declared to end, and
//! which chain the server witnessed at that moment. The participant writes the entry id,
//! the connector, the workspace and the path to the manifest in `trace.toml`. Five threats
//! live in that frame:
//!
//!   * **The substituted frame.** Run the work in one place, hand over a descriptor
//!     naming another. Every check downstream reads the substituted frame as the truth
//!     about the run, because it is the only frame there is. `descriptor_mismatch` is the
//!     comparison against a document the participant did not write.
//!   * **Work outside the window.** A receipt names `started_at` and `deadline_at`; a seal
//!     names `sealed_at`. A participant could chain yesterday's work into today's entry, or
//!     keep working after the seal and chain it below one. `outside_window` is the second
//!     of those, and it is where the window has its teeth. The first is answered by
//!     `pre_start_records` and `pre_start_records_early` rather than by a Fail — see "The
//!     lower edge is not a deadline" below.
//!   * **The seal beside the chain.** `seal.json` is a file at a path the descriptor
//!     names. Unless the seal is itself a **link**, an acknowledgement over the head
//!     anchors whatever record happened to be last and covers no byte of the manifest — and
//!     a seal with records chained after it closes nothing. `seal_not_chained` is the three
//!     ways that can be false, said apart.
//!   * **The baseline beside the chain.** `baseline.json` is the same shape of file at
//!     the other end of the run, and it is written by the participant's own daemon. A
//!     participant who pre-populated the workspace could list every planted file's real
//!     path and digest in it and have `attribute_seal` exempt each one as "the workspace
//!     already held this". The baseline is bound by being the chain's first link — seq 0,
//!     which is the link a receipt's `chain_head` witnesses — and `baseline_missing` and
//!     `baseline_not_chained` are the ways that can fail.
//!   * **The rewritten rulebook.** `connector_manifest` is a participant-written path to
//!     the file that supplies the human rule, the exclusion catalogue and the tool classes.
//!     A hand-written manifest whose exclusions match the participant's own steering
//!     messages passes `validate()` and leaves V2 with nothing to fail.
//!     `manifest_not_pinned` compares the manifest's digest against
//!     `CONNECTOR_MANIFEST_SHA256S`. Its other half is the connector's `files/`, which no
//!     trace carries: the daemon hashes its own connector directory and the server signs
//!     the value into the receipt, and `connector_dir_unpinned` / `connector_dir_unbound`
//!     are what read it back.
//!
//! # The lower edge is not a deadline
//!
//! A record dated before `started_at` is **not** an offence, and V13 does not fail one.
//! Two facts about how a run begins decide that:
//!
//!   * **The baseline is chained at seq 0, before any arming record.** `open_trace` writes
//!     the baseline as the chain's first link, and the receipt witnesses that link as
//!     its `chain_head`. Whatever the workspace held at t0 is therefore already fixed, and
//!     fixed by a document the server saw, before a single arming record exists.
//!   * **The workspace is empty at start.** The empty workspace is enforced against the baseline the server
//!     witnessed, so a record chained before `started_at` has no pre-planted work to carry
//!     and no earlier state to smuggle: the state it could have described is the state the
//!     baseline already pinned.
//!
//! And a run cannot avoid producing such records. A connector's arming signal is the
//! harness's own first output — `launch+first_transcript_record` for `claude-code-cli` —
//! so the records that *cause* the start receipt to be requested are chained before the
//! request goes out. `started_at` is then the server's clock at whole-second granularity,
//! which puts the window's lower edge at or barely above records the run had to write to
//! reach it. Failing on that decides an honest run by which side of a second boundary one
//! HTTP round trip landed on.
//!
//! So the lower edge is reported rather than enforced. A record in
//! `[started_at - `ARMING_TOLERANCE_SECONDS`, started_at)` is counted as arming-phase and
//! said out loud as a Pass (`pre_start_records`); one earlier than that is a **Flag**
//! (`pre_start_records_early`) — far enough below the start that "the round trip took a
//! moment" no longer explains it, and near enough to the run that calling it a forgery
//! would be an accusation the verifier cannot support. The deadline keeps its Fail.
//!
//! **A legacy M0 trace is not punished for any of it.** The committed descriptors
//! declare `[[leg1]]` stores and no chain; they predate the daemon, the receipt and the
//! seal, and a `receipt_missing` on one of them would be the verifier blaming a capture for
//! not carrying a document that did not exist. Such a trace gets one Pass finding saying
//! exactly that, and nothing else in this check runs — except the manifest pin, which is
//! about a file every descriptor names and which the age of the capture does not excuse.
//!
//! **What this check does not decide.** The chain's link integrity is computed once, in
//! `chain::load_and_verify`; V13 *renders* what that pass found (`chain_broken`,
//! `protocol_version_unsupported`, `envelope_version_body_incompatible`, `truncated_tail`)
//! rather than recomputing it, so the loader and the verdict cannot disagree about the
//! supported, compatible prefix they are judging.

use super::{Check, CheckContext, CheckResult, Evidence, Finding, fail, flag, pass, pass_with};
use crate::chain::{ChainBreakReason, RedactionAuditIssueReason, VerifiedChain};
use goalcup_protocol::control::parse_z;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::manifest::InstallScope;
use goalcup_protocol::provenance::{FinalizationV1, PreCutoffDrainOutcomeV1, TerminationOutcomeV1};
use goalcup_protocol::provenance::{canonical_run_public_key, validate_questionnaire_version};
use goalcup_protocol::receipt::{ReceiptError, StartReceipt};
use goalcup_protocol::seal::SealManifest;
use goalcup_protocol::submission::{FinishReasonV1, SubmissionOutcomeV1};

pub struct V13Binding;

/// How far below `started_at` a record is read as the run arming itself.
///
/// Five minutes. The lower edge is not enforced at all (see the module header), so this
/// number decides only which of two reports a pre-start record earns, and it is chosen to
/// sit far above anything the arming sequence can take — the receipt is one loopback
/// round trip and one signature — and far below the hour a participant would need for
/// "yesterday's work" to be worth smuggling.
pub const ARMING_TOLERANCE_SECONDS: u64 = 300;

/// One leg-1 record or leg-2 post dated after the run's upper bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutOfWindow {
    pub source: String,
    pub seq: u64,
    pub at: String,
}

/// One leg-1 record or leg-2 post dated before `started_at`, and whether it is far enough
/// below the start that the arming sequence no longer explains it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreStart {
    pub source: String,
    pub seq: u64,
    pub at: String,
    /// The record's own `recorded_at`, in seconds — so the earliest of a list can be named
    /// without re-parsing it.
    pub at_secs: u64,
    /// Earlier than `started_at - ARMING_TOLERANCE_SECONDS`.
    pub early: bool,
}

/// One leg-1 record or leg-2 post whose `recorded_at` this build cannot read, so it can
/// be placed neither inside the window nor outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unplaceable {
    pub source: String,
    pub seq: u64,
}

/// The envelopes the window is measured over, named by where they came from.
///
/// Only leg-1 records and leg-2 posts are measured. The daemon's own stream is not: a
/// heartbeat, a restart notice or the seal link are the daemon saying when *it* was
/// running, and a heartbeat after the seal is how the daemon reports that it outlived the
/// entry rather than evidence the participant kept working.
///
/// A leg-1 record is named by the store its body declares; a leg-2 post by the component
/// that emitted it, which is the receiver. Neither is ever the record's content: this list
/// is published, and a finding that quoted a record would put the run's text in the
/// verdict.
fn measured_source(env: &Envelope) -> Option<String> {
    match &env.body {
        EventBody::Leg1Record { store, .. } => Some(store.clone()),
        EventBody::Leg2Post { .. } => Some(env.source.clone()),
        _ => None,
    }
}

/// Every record and post past the run's upper bound, in seq order.
///
/// **Only the upper bound.** A record dated below `started_at` is not returned here and is
/// not an offence — the module header says why, and `pre_start` is what reports it. This
/// function is the half of the window that still has teeth: a participant who kept working
/// after their own seal, or past the deadline they never sealed against, chained work the
/// run does not cover, and no property of how a run begins excuses that.
///
/// A record whose `recorded_at` does not parse is not *outside* the window: it is nowhere
/// relative to it, and counting an unreadable clock as an offence would turn a malformed
/// timestamp into a tamper finding. Such records are not silently dropped either — they
/// are what `unplaceable` returns and what `window_unplaceable` reports, so a reviewer
/// learns that some of the run could not be placed rather than reading an empty list as
/// "everything was inside".
pub fn out_of_window(chain: &VerifiedChain, sealed_at: u64) -> Vec<OutOfWindow> {
    let mut out: Vec<OutOfWindow> = Vec::new();
    for env in &chain.envelopes {
        let Some(source) = measured_source(env) else {
            continue;
        };
        let Some(at) = env.recorded_at_secs() else {
            continue;
        };
        if at <= sealed_at {
            continue;
        }
        out.push(OutOfWindow {
            source,
            seq: env.link.seq,
            at: env.recorded_at.clone(),
        });
    }
    // Sorted rather than assumed: file order is seq order on a chain that holds, and this
    // function also runs on one that does not. `sort_by_key` is stable, so two envelopes
    // sharing a seq keep their file order and the answer stays deterministic.
    out.sort_by_key(|w| w.seq);
    out
}

/// Every record and post dated below `started_at`, in seq order, each carrying whether it
/// is early enough that the arming sequence no longer explains it.
///
/// The same scan as `out_of_window` at the other edge, and deliberately a separate
/// function: the two edges are answered at two different severities, and merging them would
/// produce one Fail whose count mixed records a run had to write with records nothing in
/// the run explains.
///
/// An unreadable clock is skipped here for the reason it is skipped there: it is nowhere
/// relative to the start, and `unplaceable` is what reports it.
pub fn pre_start(chain: &VerifiedChain, started_at: u64) -> Vec<PreStart> {
    let floor = started_at.saturating_sub(ARMING_TOLERANCE_SECONDS);
    let mut out: Vec<PreStart> = Vec::new();
    for env in &chain.envelopes {
        let Some(source) = measured_source(env) else {
            continue;
        };
        let Some(at) = env.recorded_at_secs() else {
            continue;
        };
        if at >= started_at {
            continue;
        }
        out.push(PreStart {
            source,
            seq: env.link.seq,
            at: env.recorded_at.clone(),
            at_secs: at,
            early: at < floor,
        });
    }
    out.sort_by_key(|w| w.seq);
    out
}

/// Every record and post `out_of_window` could not place, in seq order.
///
/// The two halves of one scan, kept as two functions. They agree on what a measured
/// envelope is because both ask `measured_source`.
///
/// This needs no window: a clock that does not parse is unreadable against any bounds, so
/// it is reported even when the bounds themselves could not be read.
///
/// **Not `gaps::unplaceable_records`.** That one counts records a legacy leg-1 store
/// cannot place in its own time order, and it makes a hole `Dirty`. This one counts
/// chained envelopes that cannot be placed relative to the run's declared window. Two
/// measurements, two emitters, two codes — `window_unplaceable` says which window it means.
pub fn unplaceable(chain: &VerifiedChain) -> Vec<Unplaceable> {
    let mut out: Vec<Unplaceable> = chain
        .envelopes
        .iter()
        .filter(|env| env.recorded_at_secs().is_none())
        .filter_map(|env| {
            measured_source(env).map(|source| Unplaceable {
                source,
                seq: env.link.seq,
            })
        })
        .collect();
    out.sort_by_key(|w| w.seq);
    out
}

impl Check for V13Binding {
    fn id(&self) -> &'static str {
        "V13"
    }
    fn title(&self) -> &'static str {
        // The lower edge is named too, because it is the half a reader would otherwise
        // assume: a start the participant did not sign, and a close — not a start — that
        // the work has to sit above. See the module header.
        "the trace is bound to a server-signed start and closes at the run's deadline"
    }

    fn run(&self, cx: &CheckContext) -> CheckResult {
        let mut findings: Vec<Finding> = Vec::new();

        // Raised before the legacy escape hatch below: a descriptor that declares
        // `[[leg1]]` stores carries no receipt to bind, but it still points at a manifest,
        // and the rules a run was judged by are not excused by the capture predating the
        // daemon. The committed M0 descriptors point at the shipped manifests, so a
        // pinned manifest says nothing and their one `legacy_trace` finding stands.
        findings.extend(manifest_pin_finding(cx.manifest_pin, cx.manifest_sha256));

        // The other half of the same question, and raised in the same place for the same
        // reason: the manifest is the rulebook, and the hook bodies beside it are what
        // captured the run. A legacy capture answers neither, and says so as a Flag.
        findings.extend(connector_dir_pin_finding(cx));

        // Raised before the legacy escape hatch as well, and for a related reason: which
        // key checked a signature is a fact about the frame, not about the contents, and a
        // legacy capture that carried neither a receipt nor an acknowledgement is silent
        // here by construction rather than by exemption.
        key_provenance(cx, &mut findings);

        // Raised before the legacy escape hatch too, and for the reason the tail exists:
        // a hard kill is what leaves a half-written last line, and a hard kill is exactly
        // the run a legacy capture is most likely to be. A chain-derived store carries no
        // tail of its own — `structural` reports the chain's, once — so this loop is
        // silent on a chain trace rather than saying the same thing twice.
        truncated_stores(cx, &mut findings);

        let Some(chain) = cx.chain else {
            // A descriptor that declares `[[leg1]]` stores and no chain. Said once, as a
            // Pass, so a reader of the verdict knows the binding was not checked rather
            // than assuming it passed.
            findings.push(pass(
                "legacy_trace",
                "this descriptor declares [[leg1]] stores and no chain, so it predates the \
                 daemon's start receipt and carries none to check; the binding statements \
                 this check makes are about a document the capture never had"
                    .to_string(),
            ));
            return CheckResult::from_findings(self.id(), self.title(), findings);
        };

        structural(chain, &mut findings);
        let summary = binding(cx, chain, &mut findings);
        workspace_submission_bridge(chain, &mut findings);
        baseline_is_a_link(cx, chain, &mut findings);
        seal_is_a_link(cx, chain, &mut findings);
        submission_integrity(cx, chain, &mut findings);

        // The summary is pushed here and not inside `binding`, because `seal_not_chained`
        // is decided after the binding was measured and a verdict carrying both a Fail and
        // a Pass saying the same trace was bound is a document that contradicts itself.
        if let Some(message) = summary
            && !findings.iter().any(|f| refuses_the_binding(&f.code))
        {
            findings.push(pass("binding_checked", message));
        }
        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

/// V4 finalization names retained bytes, never a mutable live workspace or legacy seal.
fn submission_integrity(cx: &CheckContext, chain: &VerifiedChain, findings: &mut Vec<Finding>) {
    let finalizations = chain
        .envelopes
        .iter()
        .filter_map(|env| match &env.body {
            EventBody::FinalizationV1(finalization) => Some((env, finalization)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let workspace = finalizations
        .iter()
        .filter(|(_, finalization)| {
            matches!(finalization, FinalizationV1::WorkspaceSubmission { .. })
        })
        .collect::<Vec<_>>();
    let Some((finalization_env, finalization)) = workspace.first().copied() else {
        return;
    };
    let finalization_seq = finalization_env.link.seq;
    if finalizations.len() != 1
        || workspace.len() != 1
        || chain.envelopes.last().map(|env| env.link.seq) != Some(finalization_seq)
    {
        findings.push(fail(
            "submission_finalization_not_terminal",
            "a workspace submission must be the chain's one finalization and final link"
                .to_string(),
            vec![Evidence::at(
                "finalization",
                finalization_seq,
                None,
                "workspace submission is not the unique terminal link",
            )],
        ));
    }
    let FinalizationV1::WorkspaceSubmission {
        receipt_sha256,
        termination,
        submission,
    } = finalization
    else {
        return;
    };

    let SubmissionOutcomeV1::Captured(captured) = submission else {
        // The accepted bridge retains the permanent incomplete outcome failure.
        return;
    };
    let retained = match cx.trace.load_submission(captured) {
        Ok(manifest) => manifest,
        Err(error) => {
            findings.push(fail(
                "submission_invalid",
                format!(
                    "the V4 finalization names a submission the verifier cannot validate: {error}"
                ),
                vec![Evidence::at(
                    "submission",
                    finalization_seq,
                    Some(captured.manifest_sha256.as_str().to_string()),
                    "fixed private submission layout",
                )],
            ));
            return;
        }
    };
    let cutoffs = chain
        .envelopes
        .iter()
        .filter(|env| matches!(env.body, EventBody::CutoffReachedV1(_)))
        .collect::<Vec<_>>();
    let cutoff = cutoffs
        .iter()
        .find(|env| env.link.seq == retained.cutoff_seq)
        .copied();
    let Some(cutoff) = cutoff else {
        findings.push(fail(
            "submission_cutoff_unbound",
            "the submission names no cutoff link in the retained chain".to_string(),
            Vec::new(),
        ));
        return;
    };
    if cutoffs.len() != 1 || cutoff.link.seq >= finalization_seq {
        findings.push(fail(
            "submission_cutoff_unbound",
            "a workspace submission must name the chain's one cutoff before finalization"
                .to_string(),
            Vec::new(),
        ));
    }
    let EventBody::CutoffReachedV1(cutoff_body) = &cutoff.body else {
        findings.push(fail(
            "submission_cutoff_unbound",
            "the submission cutoff sequence names a non-cutoff chain link".to_string(),
            Vec::new(),
        ));
        return;
    };
    let actual_receipt_sha256 = cx
        .receipt
        .and_then(|receipt| receipt.canonical_digest().ok());
    if retained.entry_id != cx.trace.entry_id
        || retained.receipt_sha256 != *receipt_sha256
        || retained.receipt_sha256 != cutoff_body.receipt_sha256
        || actual_receipt_sha256.as_ref() != Some(&retained.receipt_sha256)
        || retained.cutoff_chain_hash.as_str() != cutoff.link.hash
    {
        findings.push(fail(
            "submission_binding_mismatch",
            "the retained submission disagrees with its trace entry, receipt or cutoff link"
                .to_string(),
            Vec::new(),
        ));
    }
    if cx.receipt.is_none()
        || cx
            .receipt
            .is_some_and(|receipt| cutoff_body.deadline_at.as_str() != receipt.claims.deadline_at)
    {
        findings.push(fail(
            "submission_deadline_mismatch",
            "the cutoff deadline does not equal the deadline in the signed start receipt"
                .to_string(),
            Vec::new(),
        ));
    }
    let cutoff_at = parse_z(&cutoff.recorded_at);
    let deadline_at = cx
        .receipt
        .and_then(|receipt| parse_z(&receipt.claims.deadline_at));
    let reason_matches = match retained.finish_reason {
        FinishReasonV1::Manual => cutoff_at.zip(deadline_at).is_some_and(|(c, d)| c < d),
        FinishReasonV1::Deadline => cutoff_at.zip(deadline_at).is_some_and(|(c, d)| c == d),
    };
    if !reason_matches {
        findings.push(fail(
            "submission_finish_reason_mismatch",
            "the submission finish reason disagrees with the cutoff's position at the signed deadline"
                .to_string(),
            Vec::new(),
        ));
    }
    if let TerminationOutcomeV1::Confirmed {
        requested_at,
        confirmed_at,
        ..
    } = termination
    {
        let chronology = [
            cx.receipt
                .and_then(|receipt| parse_z(&receipt.claims.started_at)),
            cutoff_at,
            parse_z(requested_at.as_str()),
            parse_z(confirmed_at.as_str()),
            parse_z(retained.copy_started_at.as_str()),
            parse_z(retained.copy_completed_at.as_str()),
            parse_z(retained.readback_verified_at.as_str()),
            parse_z(&finalization_env.recorded_at),
        ];
        let ordered = chronology.iter().all(Option::is_some)
            && chronology
                .windows(2)
                .all(|pair| pair[0].is_some_and(|left| pair[1].is_some_and(|right| left <= right)));
        if !ordered {
            findings.push(fail(
                "submission_chronology_invalid",
                "the signed start, cutoff, confirmed termination, capture, readback and finalization are not one parseable ordered lifecycle"
                    .to_string(),
                vec![Evidence::at(
                    "submission",
                    finalization_seq,
                    Some(captured.manifest_sha256.as_str().to_string()),
                    "required order: start <= cutoff <= termination request <= termination confirmation <= copy start <= copy complete <= readback <= finalization",
                )],
            ));
        }
    }
    let mut expected_exclusions = cx.manifest.workspace_exclusions.clone();
    expected_exclusions.extend(
        cx.manifest
            .hook_install
            .iter()
            .filter(|hook| hook.scope == InstallScope::Workspace)
            .map(|hook| hook.path.clone()),
    );
    expected_exclusions.sort();
    expected_exclusions.dedup();
    if retained.excluded != expected_exclusions {
        findings.push(fail(
            "submission_exclusions_mismatch",
            "the submission exclusions do not exactly equal connector exclusions plus workspace hook paths"
                .to_string(),
            Vec::new(),
        ));
    }
    let post_cutoff = chain.envelopes.iter().find(|env| {
        env.link.seq > retained.cutoff_seq
            && matches!(
                env.body,
                EventBody::Leg1Record { .. } | EventBody::Leg2Post { .. }
            )
    });
    if let Some(env) = post_cutoff {
        findings.push(fail(
            "submission_post_cutoff_evidence",
            "the chain admits leg evidence after the workspace cutoff".to_string(),
            vec![Evidence::at(
                &env.source,
                env.link.seq,
                Some(env.link.hash.clone()),
                "leg evidence appears after cutoff",
            )],
        ));
    }
    if !matches!(
        cutoff_body.pre_cutoff_drain,
        PreCutoffDrainOutcomeV1::Complete
    ) {
        findings.push(fail(
            "submission_drain_degraded",
            "the cutoff drain was degraded, so the retained artifact cannot be a clean V1 result"
                .to_string(),
            Vec::new(),
        ));
    }
}

/// The codes that say the binding did not stand.
///
/// The baseline's two codes are here for the same reason `seal_not_chained` is: a run
/// whose start the chain does not cover was not bound at seq 0, and a Pass three lines
/// down saying it was would be a document that contradicts itself.
///
/// `outside_window` is deliberately absent: it is a statement about *when the work
/// happened*, measured against a binding that did hold, and suppressing the summary for it
/// would leave a reviewer unable to tell "the window was measured and work fell outside
/// it" from "the window was never measured". `window_unplaceable` is absent for the same
/// reason — it reports what could not be placed, not a binding that failed. So are
/// `pre_start_records` and `pre_start_records_early`, which are the same kind of statement
/// at the other edge and which the module header explains are not offences at all.
///
/// `manifest_not_pinned` is absent too, and for the same kind of reason: it says the rules
/// the run was judged by are not this build's, which is a statement about the rulebook and
/// not about whether the server witnessed the start. Where the two do meet — a receipt
/// claiming a digest the manifest does not have — `descriptor_mismatch` is what withholds
/// the summary. `connector_dir_unpinned` and `connector_dir_unbound` are absent for exactly
/// that reason as well: the receipt they read is a receipt that verified, and what they
/// report is which connector bytes it names — a Fail about the templates, over a binding
/// that held.
fn refuses_the_binding(code: &str) -> bool {
    // Every `receipt_*` code, so `receipt_unverified` — a signature checked against
    // nothing — withholds the summary exactly as `receipt_invalid` does.
    code.starts_with("receipt_")
        || code.starts_with("submission_")
        || matches!(
            code,
            "descriptor_mismatch"
                | "chain_head_mismatch"
                | "seal_not_chained"
                | "baseline_missing"
                | "baseline_not_chained"
                | "window_unmeasurable"
                | "protocol_version_unsupported"
                | "envelope_version_body_incompatible"
                | "workspace_submission_termination_not_confirmed"
                | "workspace_submission_incomplete"
        )
}

/// Permanent V4 terminal gates that precede retained-artifact verification.
///
/// Captured submissions fall through to `submission_integrity`, which independently reads
/// the fixed private layout. Historical termination uncertainty and incomplete capture remain
/// terminal failures even when retained bytes are otherwise available for review.
fn workspace_submission_bridge(chain: &VerifiedChain, findings: &mut Vec<Finding>) {
    for env in &chain.envelopes {
        let EventBody::FinalizationV1(FinalizationV1::WorkspaceSubmission {
            termination,
            submission,
            ..
        }) = &env.body
        else {
            continue;
        };

        if !matches!(termination, TerminationOutcomeV1::Confirmed { .. }) {
            findings.push(fail(
                "workspace_submission_termination_not_confirmed",
                "the V4 workspace-submission finalization does not record confirmed process \
                 termination. Only a confirmed termination is a V1 clean result; historical \
                 already-stopped and unresolved outcomes retain the artifact for review or \
                 recovery but cannot qualify it"
                    .to_string(),
                vec![Evidence::at(
                    "finalization",
                    env.link.seq,
                    None,
                    "workspace-submission termination is not confirmed",
                )],
            ));
        }

        match submission {
            SubmissionOutcomeV1::Incomplete { .. } => findings.push(fail(
                "workspace_submission_incomplete",
                "the V4 workspace-submission finalization records an incomplete capture, \
                 so it cannot establish an immutable workspace artifact"
                    .to_string(),
                vec![Evidence::at(
                    "finalization",
                    env.link.seq,
                    None,
                    "workspace-submission capture is incomplete",
                )],
            )),
            // `submission_integrity` validates every captured retained object.
            SubmissionOutcomeV1::Captured(_) => {}
        }
    }
}

/// The rules the run was judged by, compared to the rules this build ships.
///
/// **A matching pin says nothing.** No Pass finding, no evidence row: the M0 traces point
/// at the shipped manifests and V13 holds exactly one finding on them, and a second Pass
/// here would change committed verdicts to say something already true. Silence is what a
/// pinned manifest looks like in the queue; the verdict's own `manifest_pin` field is where
/// it is said out loud.
///
/// Takes the resolved pinning and the digest of the bytes that were read, rather than a
/// `CheckContext`, so every reading it renders — including the one no shipped connector can
/// reach — is exercised directly by the unit tests below.
fn manifest_pin_finding(pin: crate::ManifestPinning, read: &str) -> Option<Finding> {
    let (code, subject) = match pin.state {
        crate::ManifestPin::Pinned => return None,
        crate::ManifestPin::Altered => (
            "manifest_not_pinned",
            format!(
                "the connector manifest this run was judged by is not the one this build \
                 pins: the pin is {}, and the file `trace.toml` named hashes to {read}",
                // `Altered` is resolved from a pin that exists, so the fallback is a value
                // no `ManifestPinning::resolve` produces — written rather than unwrapped
                // because a verifier must not panic over the wording of a finding.
                pin.pinned.unwrap_or("(none)")
            ),
        ),
        crate::ManifestPin::Unpinned => (
            "manifest_not_pinned",
            format!(
                "this build pins no manifest for this connector, so nothing says which \
                 rules judged the run; the file `trace.toml` named hashes to {read}"
            ),
        ),
    };
    // The reason, stated once for both readings: the manifest is the rulebook, and until
    // it is pinned it is a participant input like any other file the descriptor names.
    let why = "the manifest supplies the human rule, the exclusion catalogue and the tool \
               classes, so a participant free to write it is free to write the rule that \
               excludes their own steering messages — which V2 then has nothing to fail";
    let evidence = vec![Evidence {
        store: "manifest".to_string(),
        seq: None,
        sha256: Some(read.to_string()),
        note: "the manifest this run was judged by".to_string(),
    }];
    Some(if pin.allowed {
        flag(
            code,
            format!(
                "{subject}. {why}. Downgraded from a Fail because this run was made with \
                 `--dev`: it proves nothing about which rules judged the entry",
            ),
            evidence,
        )
    } else {
        fail(code, format!("{subject}. {why}"), evidence)
    })
}

/// The connector **directory** the run's tracker files were rendered out of, compared to
/// the directories this build ships.
///
/// [`manifest_pin_finding`] answers "which rulebook judged this run" from a file the trace
/// carries. This answers "which hook bodies captured it" — and the trace carries none of
/// them. `TraceExport::write_manifest` copies `manifest.toml` and no other byte of
/// `connectors/<id>/`, so the only thing that can bring the directory's digest into a
/// verdict is a document the participant did not write: the daemon hashes its own connector
/// directory at workspace creation and the server signs the value into the start receipt as
/// `StartReceiptClaims::connector_dir_sha256`.
///
/// **Nothing here is read out of a receipt that did not verify**. The whole
/// value of this check is that the digest comes from a document the participant could not
/// write; a receipt whose signature was never checked, or was checked and did not stand, is
/// exactly such a document written by anybody at all. Reading its claims would let a
/// participant with edited hook templates self-sign a receipt naming the pinned digest and
/// buy silence from this check. So the gate is the one [`key_provenance`] uses through
/// [`key_verified_something`], narrowed to the half that matters here: a receipt this build
/// verified against the resolved server key, or no finding of any kind. What such a trace
/// gets instead is `receipt_unverified` or `receipt_invalid` — a refusal of the whole
/// binding, which is a larger statement than anything this function could add — and a
/// capture with no receipt at all gets `receipt_missing` or `legacy_trace`.
///
/// Two readings, and a matching pin is silent for the reason a matching manifest pin is:
///
///   * **`connector_dir_unpinned`** — the verified receipt carries a digest, and it is not
///     this build's for that connector. Either the daemon that recorded the run shipped
///     edited hook templates, or it shipped a connector this build does not know. A
///     **Fail**: the daemon renders those bodies into the workspace, records their rendered
///     digests as the config it will guard, and then guards them, so a body nobody agreed
///     to becomes the legitimate one in writing and every later config check agrees with
///     it. Pass-level under `--dev`, exactly as `placeholder_server_key` is: developing a
///     connector means running one whose bytes are not yet pinned, and a Fail there would
///     make the development path unusable rather than more honest.
///   * **`connector_dir_unbound`** — the receipt verified and carries no digest at all. A
///     **Flag**, never a Fail: a receipt signed before this claim existed is an old
///     document rather than a dishonest one. What the Flag says is that the question was
///     not answered, which is the thing a reader would otherwise have to infer from
///     silence. It is not raised for a trace with no receipt: there is no document there to
///     be old, and `receipt_missing` — or `legacy_trace` on an M0 capture — is already the
///     statement that no start was carried.
///
/// Raised before the legacy escape hatch in [`V13Binding::run`], and for the same reason
/// the manifest pin is: it is a statement about the frame the run was captured in, and the
/// age of a capture does not make the templates that captured it known.
fn connector_dir_pin_finding(cx: &CheckContext) -> Option<Finding> {
    let receipt = verified_receipt(cx)?;
    // The receipt's own `connector_id` and not the descriptor's: the descriptor is written
    // by the participant, and the digest and the id it is looked up under have to come out
    // of the same signed document or the pair proves nothing. Where the two disagree,
    // `descriptor_mismatch` is what says so.
    let Some(digest) = receipt.claims.connector_dir_sha256.as_deref() else {
        return Some(flag(
            "connector_dir_unbound",
            "the start receipt this trace carries verifies, and makes no claim about the \
             connector DIRECTORY the run was captured with, so nothing here says which hook \
             templates the daemon rendered into the workspace and then guarded. The \
             manifest is pinned and the templates beside it are not: a receipt signed \
             before that claim existed leaves the question unanswered rather than answered \
             badly"
                .to_string(),
            vec![Evidence::note("receipt", "no `connector_dir_sha256` claim")],
        ));
    };
    // Read from the same resolved tables as the manifest pin, so the two readings share one
    // trust source. Production resolves `PinTables::CURRENT_BUILD`, whose directory half is
    // exactly `goalcup_protocol::pins::CONNECTOR_DIR_SHA256S`.
    if cx
        .manifest_pin
        .tables
        .directory_is_pinned(&receipt.claims.connector_id, digest)
    {
        return None;
    }
    let subject = format!(
        "the connector directory this run was captured with is not one this build ships: \
         the receipt claims {digest} for `{}`, and no directory pin of this build's has \
         that digest",
        receipt.claims.connector_id
    );
    let why = "the daemon renders the hook bodies under `files/` into the workspace, records \
               their rendered digests as the config it will guard, and then guards them — so \
               an edited template is not a file that slipped past a check, it is the file \
               every later check agrees with. The manifest beside it can be pinned and \
               unchanged while this is true";
    let evidence = vec![Evidence {
        store: "receipt".to_string(),
        seq: None,
        sha256: Some(digest.to_string()),
        note: "the connector directory the daemon hashed at workspace creation".to_string(),
    }];
    Some(if cx.dev_mode {
        pass_with(
            "connector_dir_unpinned",
            format!(
                "{subject}. {why}. Reported at Pass because this run was made with `--dev`, \
                 where a connector whose bytes are not yet pinned is the expected thing to \
                 be running"
            ),
            evidence,
        )
    } else {
        fail(
            "connector_dir_unpinned",
            format!("{subject}. {why}"),
            evidence,
        )
    })
}

/// Whether the resolved server key actually verified anything on this trace.
///
/// Both codes below rest on it, because a key that checked nothing is a fact about the
/// invocation and not about the trace: `--dev-key` on a capture with no receipt and no
/// acknowledgements resolves a key, uses it for nothing, and deserves no finding.
///
/// The acknowledgement half reads `acks_verified.verified`, which `gaps::verify_acks`
/// filled using this same key — "verified" there means signature-checked and reconciled —
/// so a non-empty set is the statement that the key stood behind an anchor. The receipt
/// half is [`verified_receipt`], which `connector_dir_pin_finding` reads as well: one
/// definition of "the server signed this and the signature stood", so the two cannot come
/// to different answers about the same document.
fn key_verified_something(cx: &CheckContext) -> bool {
    verified_receipt(cx).is_some() || !cx.acks_verified.verified.is_empty()
}

/// The trace's start receipt, and `None` unless this build actually checked its signature
/// against the resolved server key and the signature stood.
///
/// One definition, read by [`key_verified_something`] and by [`connector_dir_pin_finding`],
/// because the two ask the same question — "did anything the server signed survive a
/// check" — and a second spelling is how they come to disagree. `verify_with` is re-run
/// rather than `receipt_refusal` consulted, because that function runs below the
/// `receipt_missing` early return and both callers run above it.
fn verified_receipt<'a>(cx: &CheckContext<'a>) -> Option<&'a StartReceipt> {
    let (receipt, key) = (cx.receipt?, cx.server_key?);
    receipt.verify_with(key).is_ok().then_some(receipt)
}

/// The word the verdict itself uses for a key's provenance, read out of `KeySource`'s own
/// serialisation rather than matched by hand — a `match` here would be a second place that
/// decides how a provenance is spelled, and the finding's whole job is to say the word the
/// verdict's `key_source` field says.
fn key_source_word(source: crate::KeySource) -> String {
    match serde_json::to_value(source) {
        Ok(serde_json::Value::String(word)) => word,
        _ => "unknown".to_string(),
    }
}

/// Who the verifier borrowed its trust from.
///
/// Two statements, both about the key that actually verified something:
///
///   * **`server_key_unpinned`.** The key is not on this build's allowlist — it came from
///     the descriptor (`--dev-descriptor-key`) or from the command line (`--dev-key`).
///     Without this code the only trace of that is the verdict's `key_source` field, a word
///     in a header; a development run over an otherwise clean trace would reach `overall:
///     pass` with an empty review queue, and a reader of that document would have nothing
///     telling them the anchor was borrowed. **Never suppressed**, `--dev` included: a Flag
///     is the mildest thing a verdict can say and it still has to say it.
///
///   * **`placeholder_server_key`.** The key is `crate::PLACEHOLDER_SERVER_PUBLIC_KEY`,
///     whose seed is written in this repository, so anybody can mint a receipt or an
///     acknowledgement that reads as `KeySource::Pinned` — the strongest provenance the
///     verdict has. Suppressed by `--dev` and by nothing else, because signing with the
///     published seed is the expected thing to do while developing a connector. Replacing
///     the placeholder with the real organiser key is the release gate; until then this
///     Flag is what keeps a pinned reading from being read as an anchored one.
///
/// Neither message carries the key: a verdict is published, and a full hex key in a
/// sentence is a value a reader can mistake for a pin.
fn key_provenance(cx: &CheckContext, findings: &mut Vec<Finding>) {
    if !key_verified_something(cx) {
        return;
    }
    let word = key_source_word(cx.key_source);

    if cx.key_source != crate::KeySource::Pinned {
        findings.push(flag(
            "server_key_unpinned",
            format!(
                "the key that verified this trace's server signatures is not one this build \
                 pins: its provenance is `{word}`, which means the run borrowed its trust \
                 from the invocation that produced this verdict rather than from the \
                 verifier. A start and an anchor are only evidence about who witnessed them"
            ),
            vec![Evidence::note("server key", format!("key source: {word}"))],
        ));
    }

    let placeholder = cx
        .server_key
        .is_some_and(|k| k.eq_ignore_ascii_case(crate::PLACEHOLDER_SERVER_PUBLIC_KEY));
    if placeholder {
        // Under `--dev` this is reported at Pass, not suppressed — the same shape
        // `connector_dir_unpinned` uses one function away.
        //
        // Suppressed, a `--dev` verdict over a placeholder-signed trace with a pinned
        // manifest and a pinned connector directory would come out `overall: pass`,
        // `key_source: "pinned"`, `review_queue: []` — byte for byte the shape a genuinely
        // anchored strict run produces, with nothing in the document to tell them apart. A
        // notice on stderr does not travel with the file. `Verdict::dev` is the other half.
        let subject = "the key that verified this trace's server signatures is the M1 \
                       placeholder anchor, whose private seed is published in this \
                       repository so the fixtures can sign with it. Anybody can mint a \
                       receipt or an acknowledgement under it, so a `key_source` of \
                       `pinned` says only that the signature matched a key everyone holds. \
                       Replacing the placeholder with the real organiser key is the release \
                       gate before any entry is judged";
        let evidence = vec![Evidence::note(
            "server key",
            format!("key source: {word}, and the key is the M1 placeholder"),
        )];
        findings.push(if cx.dev_mode {
            pass_with(
                "placeholder_server_key",
                format!(
                    "{subject}. Reported at Pass because this run was made with `--dev`, \
                     where the placeholder is the expected anchor — the sentence stays in \
                     the document so a reader can tell this verdict from a strict one"
                ),
                evidence,
            )
        } else {
            flag("placeholder_server_key", subject.to_string(), evidence)
        });
    }
}

/// What `chain::load_and_verify` already found, rendered. Nothing is recomputed here.
fn structural(chain: &VerifiedChain, findings: &mut Vec<Finding>) {
    if let Some(b) = &chain.break_at {
        let reason = match &b.reason {
            ChainBreakReason::EntryIdMismatch => {
                format!("entry id changed from `{}` to `{}`", b.expected, b.observed)
            }
            ChainBreakReason::SeqOutOfOrder { expected } => format!(
                "it names seq {} where the chain is at {expected}, so a line went missing \
                 or was inserted",
                b.observed
            ),
            ChainBreakReason::PrevMismatch => format!(
                "its `prev` names {} and the link below it hashes to {}, so that line was \
                 rewritten",
                b.observed, b.expected
            ),
            ChainBreakReason::HashMismatch => format!(
                "it records hash {} and its own bytes hash to {}, so its body was edited \
                 after the link was stamped",
                b.observed, b.expected
            ),
            ChainBreakReason::SequenceExhausted => {
                "it reaches the largest representable sequence, so no following link can be contiguous"
                    .to_string()
            }
        };
        findings.push(fail(
            "chain_broken",
            format!(
                "the chain stops holding at seq {}: {reason}. Only the first break is \
                 reported — every link above a rewritten one is chained over a hash that \
                 moved, so the second is a consequence and not a second problem",
                b.seq
            ),
            vec![Evidence::at(
                "chain",
                b.seq,
                Some(b.observed.clone()),
                "the first link that does not hold",
            )],
        ));
    }

    if let Some((seq, version)) = chain.unsupported_version {
        findings.push(fail(
            "protocol_version_unsupported",
            format!(
                "the envelope at seq {seq} declares protocol version {version}, and this \
                 build understands versions {} through {}; a verifier that read it anyway would be \
                 guessing what its fields mean",
                goalcup_protocol::LEGACY_PROTOCOL_VERSION,
                goalcup_protocol::PROTOCOL_VERSION
            ),
            vec![Evidence::at(
                "chain",
                seq,
                None,
                format!("protocol version {version}"),
            )],
        ));
    }

    if let Some(issue) = &chain.incompatible_envelope {
        findings.push(fail(
            "envelope_version_body_incompatible",
            format!(
                "the envelope at seq {} names a version/body combination this build rejects: {}. \
                 The verifier retained only the compatible prefix below that boundary",
                issue.seq, issue.reason
            ),
            vec![Evidence::at(
                "chain",
                issue.seq,
                None,
                "the first envelope whose body is incompatible with its declared version",
            )],
        ));
    }

    if let Some(bytes) = chain.truncated_tail {
        findings.push(truncated_tail_finding("chain", "chain", bytes));
    }

    for issue in &chain.redaction_audit_issues {
        findings.push(fail(
            "redaction_audit_invalid",
            format!(
                "the redaction audit at seq {} is invalid: {}",
                issue.seq,
                redaction_issue_message(&issue.reason)
            ),
            vec![Evidence::at(
                &issue.store,
                issue.seq,
                None,
                "the envelope whose inline redaction evidence is invalid",
            )],
        ));
    }

    for subject in &chain.legacy_redaction_log_missing {
        findings.push(flag(
            "legacy_redaction_log_missing",
            format!(
                "an exact legacy salted redaction marker appears at seq {}, but no legacy \
                 Redaction record names that sequence. The marker may be natural text or \
                 an altered legacy record; missing audit evidence leaves that distinction \
                 unresolved",
                subject.seq
            ),
            vec![Evidence::at(
                &subject.store,
                subject.seq,
                None,
                "the record containing the exact legacy marker",
            )],
        ));
    }
}

fn redaction_issue_message(reason: &RedactionAuditIssueReason) -> String {
    match reason {
        RedactionAuditIssueReason::AuditOnLegacyVersion => {
            "protocol v1 has no inline redaction-audit field".to_string()
        }
        RedactionAuditIssueReason::AuditOnNonLeg => {
            "only a leg-1 record or leg-2 post may carry an inline audit".to_string()
        }
        RedactionAuditIssueReason::EmptySpans => "the audit contains no spans".to_string(),
        RedactionAuditIssueReason::EmptyKind { span } => {
            format!("span {span} has an empty kind")
        }
        RedactionAuditIssueReason::EmptyOrReversedSpan { span } => {
            format!("span {span} does not satisfy start < end")
        }
        RedactionAuditIssueReason::UnorderedOrOverlappingSpan { span } => {
            format!("span {span} is out of order or overlaps the span before it")
        }
        RedactionAuditIssueReason::LegacyLogInV2 => {
            "protocol v2 requires audit evidence on the altered leg envelope and does not \
             permit a separate legacy Redaction record"
                .to_string()
        }
    }
}

/// The half-written last line, said the same way wherever it was read.
///
/// One writer for both readings — `VerifiedChain::truncated_tail` and a legacy
/// `Leg1Store::truncated_tail` — because they are the same fact about the same kind of
/// interrupted write, and a reviewer who learns to read one has learned to read the other.
///
/// A `Flag` and never a `Fail`: an unfinished write is what a hard kill leaves behind, and
/// a hard kill is a thing the tracker exists to record rather than an accusation. A hard
/// error (exit 3, no verdict) would hand a participant a way to turn a Fail into a crash by
/// killing the harness mid-append.
fn truncated_tail_finding(subject: &str, store: &str, bytes: u64) -> Finding {
    flag(
        "truncated_tail",
        format!(
            "the {subject} ends in {bytes} byte(s) that did not parse and were not \
             newline-terminated: a write that did not finish. The records below it are \
             unaffected, so this is what the file ends with rather than a break"
        ),
        vec![Evidence::note(
            store,
            format!("{bytes} unterminated byte(s)"),
        )],
    )
}

/// Every legacy leg-1 store whose file ends in a write that did not finish.
///
/// In descriptor order, which is the order `verify_trace` loaded the stores in, so two
/// truncated stores are reported in the order the participant declared them rather than in
/// whatever order a map iterated. A store lifted out of a chain always reports `None`, so
/// this adds nothing on a chain trace.
fn truncated_stores(cx: &CheckContext, findings: &mut Vec<Finding>) {
    for store in cx.stores {
        if let Some(bytes) = store.truncated_tail {
            findings.push(truncated_tail_finding(
                &format!("leg-1 store `{}`", store.name),
                &store.name,
                bytes,
            ));
        }
    }
}

/// The receipt, and everything measured against its claims.
///
/// Returns the `binding_checked` summary its caller emits when nothing anywhere in this
/// check refused the binding, and `None` when this pass alone already knows there is
/// nothing to summarise. The decision itself is not made here: `seal_not_chained` is
/// raised after this function returns.
fn binding(
    cx: &CheckContext,
    chain: &VerifiedChain,
    findings: &mut Vec<Finding>,
) -> Option<String> {
    let Some(receipt) = cx.receipt else {
        findings.push(fail(
            "receipt_missing",
            "this trace declares a chain and no start receipt, so nothing outside the \
             participant's own machine says this run was ever declared: no entry, no \
             workspace, no deadline and no witnessed chain head. A chain that recomputes \
             cleanly is self-consistent, which is not the same as started"
                .to_string(),
            Vec::new(),
        ));
        return None;
    };

    let refusal = receipt_refusal(receipt, cx.server_key);
    let receipt_binding = if refusal.is_none() {
        receipt_binding(receipt)
    } else {
        ReceiptBinding::Unverified
    };
    if let Some(r) = refusal {
        let evidence = vec![Evidence::note("receipt", r.note)];
        findings.push(if r.fail {
            fail(r.code, r.message, evidence)
        } else {
            flag(r.code, r.message, evidence)
        });
    }

    match receipt_binding {
        ReceiptBinding::Invalid => findings.push(fail(
            "receipt_binding_invalid",
            "the verified start receipt carries a malformed or incomplete run-key and \
             questionnaire binding, so it does not identify one usable run key under one \
             questionnaire contract"
                .to_string(),
            vec![Evidence::note(
                "receipt",
                "the optional binding claims are not a valid pair",
            )],
        )),
        ReceiptBinding::LegacyUnbound
            if chain
                .envelopes
                .iter()
                .any(|env| matches!(env.body, EventBody::FinalizationV1(_))) =>
        {
            findings.push(fail(
                "receipt_binding_required_for_finalization",
                "this trace carries a provenance finalization, but its verified start \
                 receipt predates the run-key and questionnaire binding. The terminal \
                 document therefore cannot be tied to the questionnaire contract and run \
                 key the receipt service accepted"
                    .to_string(),
                vec![Evidence::note("receipt", "both binding claims are absent")],
            ));
        }
        ReceiptBinding::Unverified | ReceiptBinding::LegacyUnbound | ReceiptBinding::Bound => {}
    }

    // The claims are compared whether or not the signature verified. A receipt that fails
    // both is two statements — one about who signed it, one about what it says — and
    // collapsing them would let a reviewer read "invalid" and stop.
    let c = &receipt.claims;
    let mut differ: Vec<(&str, Option<String>)> = Vec::new();
    if cx.trace.entry_id != c.entry_id {
        differ.push(("entry_id", None));
    }
    if cx.trace.connector != c.connector_id {
        differ.push(("connector", None));
    }
    if cx.trace.workspace != c.workspace {
        differ.push(("workspace", None));
    }
    // The session the server signed has to be one of the sessions this trace claims to
    // track. Without it a participant could hand over a frame naming the session the
    // receipt witnessed and chain a different one's records beneath it — the half of the
    // substituted frame that `entry_id`, `connector` and `workspace` do not close.
    if !cx.trace.tracked_sessions.contains(&c.session_id) {
        differ.push(("session_id", None));
    }
    // Hex case is not meaningful, and the digest is the one `verify_trace` took over the
    // manifest bytes it actually read — never a re-hash of a path named here.
    if !cx.manifest_sha256.eq_ignore_ascii_case(&c.manifest_sha256) {
        differ.push(("connector manifest sha256", Some(c.manifest_sha256.clone())));
    }
    if !differ.is_empty() {
        let fields: Vec<&str> = differ.iter().map(|(f, _)| *f).collect();
        findings.push(fail(
            "descriptor_mismatch",
            format!(
                "the descriptor and the receipt disagree about {}: the receipt is the \
                 server's record of what this run was declared to be, and the descriptor is \
                 what the participant handed over. The fields are named and their values are \
                 not — a workspace is an absolute path, and a verdict is published",
                and_list(&fields)
            ),
            differ
                .iter()
                .map(|(field, digest)| Evidence {
                    store: "receipt".to_string(),
                    seq: None,
                    sha256: digest.clone(),
                    note: format!("`{field}` differs from the receipt's claim"),
                })
                .collect(),
        ));
    }

    if chain.genesis_link != c.chain_head {
        findings.push(fail(
            "chain_head_mismatch",
            format!(
                "the receipt witnessed chain head {} when the run was declared, and this \
                 chain's seq 0 hashes to {}; a chain whose start the server never saw is not \
                 the chain it started",
                c.chain_head, chain.genesis_link
            ),
            vec![Evidence::at(
                "chain",
                0,
                Some(chain.genesis_link.clone()),
                "the chain's own genesis link",
            )],
        ));
    }

    // The upper bound: the earlier of the seal and the declared deadline. A run that never
    // sealed still has a deadline, and treating its absence as "no upper bound" would make
    // the window unmeasurable in exactly the case a participant would arrange; a run that sealed
    // late has one too, and see `late_by` below for why that half matters just as much.
    //
    // Where any seal claims disagree, no one of them gets to widen the accepted interval.
    // The envelope and seal body are both carried by the authenticated chain; the declared
    // manifest can only make the bound earlier here, never rescue later work. Therefore a
    // disputed seal is dated by the earliest parseable claim among all three. An unreadable
    // claim makes the bound unmeasurable below rather than silently extending the run.
    let chained = chained_seal(chain);
    let declared_digest = cx
        .seal
        .and_then(|m| goalcup_protocol::export::manifest_sha256(m).ok());
    let digest_disagrees = match (&chained, &declared_digest) {
        (Some(s), Some(d)) => &s.manifest_sha256 != d,
        _ => false,
    };
    let claims_disagree = match (&chained, cx.seal) {
        (Some(s), Some(m)) => !manifest_claim_mismatches(cx.trace, m, &s.claims).is_empty(),
        _ => false,
    };
    let by_seal: Option<(&str, String)> = match cx.seal {
        None => None,
        Some(seal) if digest_disagrees || claims_disagree => chained
            .as_ref()
            .map(|linked| conservative_disputed_seal_cutoff(seal, linked)),
        Some(seal) => Some(("the seal", seal.sealed_at.clone())),
    };

    // The deadline is a ceiling on the seal, not the other way round.
    //
    // `due_entries` seals whenever the daemon notices `now >= deadline_unix`, so a daemon
    // that is not running at that second — a closed laptop, a SIGTERM, a reboot — misses
    // the instant and seals at its next start, dating the window by whenever that was.
    // Every file written in between lands in the sealed manifest and every record the
    // tailers catch up on is chained with a `recorded_at` past `deadline_at`. Taking the
    // bound from `sealed_at` alone would mean `outside_window` — the check whose message
    // says "work chained after the run closed is work the run does not cover" — could not
    // fire on any of it, and "168 hours, the clock never pauses and there is no build grace
    // period" (RULES, Timeline) would rest entirely on one process being alive at one
    // second.
    //
    // So the bound is the earlier of the two. `deadline_at` is a first-class checkable
    // fact: the daemon derives it from the server's `started_at` and the receipt is signed
    // over it. The seal still binds when it is the earlier one, which is the ordinary case
    // — a run sealed on time, or early by `goalcup seal`.
    let deadline_secs = parse_z(&c.deadline_at);
    let seal_secs = by_seal.as_ref().and_then(|(_, at)| parse_z(at));
    let late_by = match (seal_secs, deadline_secs) {
        (Some(sealed), Some(deadline)) if sealed > deadline => Some(sealed - deadline),
        _ => None,
    };
    if let Some(seconds) = late_by {
        // Said as its own sentence, so a reviewer reads why the records above the deadline
        // are outside the window rather than inferring it from a list of sequences. A Flag
        // and not a Fail: the innocent case (a sleeping laptop) and the arranged one look
        // identical from here, and what the arranged one produces — records past the
        // deadline — is the Fail below.
        findings.push(flag(
            "seal_after_deadline",
            format!(
                "the run sealed {seconds}s after its own deadline. The daemon seals when it \
                 next notices the deadline has passed, so a daemon that was not running at \
                 that moment dates the seal by its restart — which is why the window's upper \
                 edge below is `deadline_at` and not `sealed_at`. Work chained in between is \
                 `outside_window`",
            ),
            vec![Evidence::note(
                "receipt",
                format!("deadline_at {}", c.deadline_at),
            )],
        ));
    }
    let upper: Option<(&str, String)> = match (&by_seal, late_by) {
        (Some(_), None) => by_seal.clone(),
        _ => Some(("the declared deadline", c.deadline_at.clone())),
    };
    let upper_bound_name = if matches!(upper, Some(("the declared deadline", _))) {
        "deadline_at"
    } else {
        "sealed_at"
    };

    // Both edges have to be readable for the window to mean anything — and a bound that is
    // not readable is said so, rather than returning out of this function and taking
    // `outside_window` and the summary below with it. An unreadable bound is exactly what
    // somebody arranging that would write.
    let started = parse_z(&c.started_at);
    if started.is_none() {
        findings.push(unmeasurable_bound(
            "receipt",
            "started_at",
            "lower",
            Some(&c.started_at),
        ));
    }
    let upper_secs = upper.as_ref().and_then(|(_, at)| parse_z(at));
    if upper_secs.is_none() {
        findings.push(unmeasurable_bound(
            if upper_bound_name == "sealed_at" {
                "seal"
            } else {
                "receipt"
            },
            upper_bound_name,
            "upper",
            upper.as_ref().map(|(_, at)| at.as_str()),
        ));
    }

    // Records the scan could not place. Independent of the bounds — a clock that does not
    // parse is unreadable against any window — so this is reported even when the window
    // itself could not be measured.
    let unplaceable = unplaceable(chain);
    if let (Some(first), Some(last)) = (unplaceable.first(), unplaceable.last()) {
        findings.push(flag(
            "window_unplaceable",
            format!(
                "{} record(s) and post(s) carry a `recorded_at` this build cannot read, so \
                 they sit neither inside the run's window nor outside it. The first is seq \
                 {}, the last seq {}. This is a malformed clock rather than a tamper \
                 signature, and it is a Flag for that reason — but a record that could not \
                 be placed is not a record that was checked, and an empty `outside_window` \
                 does not cover it",
                unplaceable.len(),
                first.seq,
                last.seq,
            ),
            cite(&unplaceable, |w| {
                Evidence::at(&w.source, w.seq, None, "its `recorded_at` did not parse")
            }),
        ));
    }

    let (Some(started), Some(sealed), Some((upper_label, upper_at))) =
        (started, upper_secs, upper.as_ref())
    else {
        return None;
    };

    // The lower edge, reported and not enforced. See the module header: the baseline is
    // chained at seq 0 before any arming record and the workspace is empty at start, so a
    // record below `started_at` carries no work the run did not already pin — and the
    // arming records are chained before the receipt that dates the start is requested, so
    // an honest run produces them by construction.
    let pre = pre_start(chain, started);
    // Two disjoint buckets, counted apart: "the run armed itself" and "this is further
    // below the start than arming explains" are different statements, and one count
    // covering both would conflate them.
    let (early, arming): (Vec<&PreStart>, Vec<&PreStart>) = pre.iter().partition(|w| w.early);
    if !arming.is_empty() {
        findings.push(pass_with(
            "pre_start_records",
            format!(
                "{} record(s) and post(s) sit in the {ARMING_TOLERANCE_SECONDS}s arming window \
                 below `started_at` ({}). This is what a run arming itself looks like: the \
                 connector's arming signal is the harness's own first output, so the records \
                 that CAUSE the start receipt to be requested are chained before the request \
                 goes out. They carry no work — the baseline the receipt witnessed is seq 0, \
                 below all of them — so they are counted here rather than failed",
                arming.len(),
                c.started_at,
            ),
            cite(&arming, |w| {
                Evidence::at(
                    &w.source,
                    w.seq,
                    None,
                    format!("recorded {}, before the start", w.at),
                )
            }),
        ));
    }
    // The earliest by clock, which is the number a reviewer wants, and not the lowest seq —
    // a chain is ordered by when it was written, not by what its records claim.
    if let Some(earliest) = early.iter().min_by_key(|w| w.at_secs).map(|w| w.at.clone()) {
        findings.push(flag(
            "pre_start_records_early",
            format!(
                "{} record(s) and post(s) are dated more than {ARMING_TOLERANCE_SECONDS}s below \
                 the start the server signed ({}); the earliest reads {earliest}. A run arming \
                 itself writes records in the second or two before its receipt is issued, and \
                 this is further below the start than that explains. It is a Flag and not a \
                 Fail because the baseline the receipt witnessed is still seq 0: these records \
                 sit above a workspace the server saw, so what they can carry is a clock \
                 nobody set, not work nobody watched",
                early.len(),
                c.started_at,
            ),
            cite(&early, |w| {
                Evidence::at(
                    &w.source,
                    w.seq,
                    None,
                    format!("recorded {}, before the arming window", w.at),
                )
            }),
        ));
    }

    let outside = out_of_window(chain, sealed);
    if let (Some(first), Some(last)) = (outside.first(), outside.last()) {
        findings.push(fail(
            "outside_window",
            format!(
                "{} record(s) and post(s) carry a clock past {upper_label}, the run's own \
                 close ({upper_at}). The first is seq {}, the last seq {}. This is the edge \
                 the window enforces: work chained after the run closed is work the run does \
                 not cover, and no property of how a run BEGINS excuses it",
                outside.len(),
                first.seq,
                last.seq,
            ),
            cite(&outside, |w| {
                Evidence::at(
                    &w.source,
                    w.seq,
                    None,
                    format!("recorded {}, after the window", w.at),
                )
            }),
        ));
    }

    Some(format!(
        "the receipt binds this trace's entry, connector, workspace, session and connector \
         manifest to a start the participant did not sign, and dates the run \
         [{}, {upper_at}] against {upper_label}",
        c.started_at
    ))
}

/// Internal receipt-binding state. `Bound` is reachable only after the pinned receipt
/// signature verified; callers classify an unchecked receipt as `Unverified` without
/// interpreting participant-editable claims as service evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiptBinding {
    Unverified,
    LegacyUnbound,
    Bound,
    Invalid,
}

fn receipt_binding(receipt: &StartReceipt) -> ReceiptBinding {
    match (
        receipt.claims.run_public_key.as_deref(),
        receipt.claims.questionnaire_version.as_deref(),
    ) {
        (None, None) => ReceiptBinding::LegacyUnbound,
        (Some(key), Some(version))
            if canonical_run_public_key(key).is_ok()
                && validate_questionnaire_version(version).is_ok() =>
        {
            ReceiptBinding::Bound
        }
        _ => ReceiptBinding::Invalid,
    }
}

/// The first and the last of a list, and nothing between them.
///
/// A reviewer needs the extent of an offence and its count; a list of every seq would put
/// the whole run in the verdict and still say the same thing. One entry is its own first
/// and last, and is cited once. Empty in, empty out.
fn cite<T>(items: &[T], render: impl Fn(&T) -> Evidence) -> Vec<Evidence> {
    match items {
        [] => Vec::new(),
        [one] => vec![render(one)],
        [first, .., last] => vec![render(first), render(last)],
    }
}

/// A window edge this build cannot read, stated rather than stepped over.
///
/// The bound's own text is a participant-written string and a verdict is a published
/// document, so at most 32 characters of it reach the message: enough to recognise
/// "not-a-date" or a truncated timestamp, and not enough to carry an arbitrary payload.
fn unmeasurable_bound(store: &str, bound: &str, edge: &str, value: Option<&str>) -> Finding {
    let seen = match value {
        Some(v) => format!("it reads `{}`", first_32(v)),
        // Reachable only when the declared seal and the chained one disagree about the
        // digest and the chain carries no seal link to read the date off instead.
        None => "the chain carries no seal link to read one from".to_string(),
    };
    fail(
        "window_unmeasurable",
        format!(
            "`{bound}` is not a timestamp this build can read, so the run's window has no \
             {edge} edge and nothing can be placed inside or outside it: {seen}. The bound \
             is reported rather than dropped — an unreadable one is exactly what somebody \
             wanting the window check to vanish would write"
        ),
        vec![Evidence::note(store, format!("`{bound}` did not parse"))],
    )
}

/// What is wrong with the receipt: its code, its severity, its sentence and the evidence
/// note that goes with it.
struct Refusal {
    code: &'static str,
    /// `true` for `receipt_invalid` — a signature that was checked and did not stand.
    /// `false` for `receipt_unverified`, which is a Flag.
    fail: bool,
    message: String,
    /// Never a hardcoded "signature": in the unverified case no signature was examined,
    /// and evidence saying otherwise would describe a check that did not happen.
    note: &'static str,
}

/// Why the receipt was refused, or `None` when it verifies.
///
/// Each `ReceiptError` variant becomes a different sentence, because "signature: invalid"
/// is not a finding a reviewer can act on: a receipt the participant signed themselves and
/// a receipt whose claims were edited after signing are two different accusations.
///
/// **Two codes, not one**. `receipt_invalid` is a Fail and means the
/// signature was checked and did not stand — `KeyMismatch`, `SignatureInvalid`, or a
/// signature that is not a signature at all. Everything else is `receipt_unverified`, a
/// Flag mirroring V12's `anchor_unverified`: the document was carried and nothing checked
/// it, which is an absence rather than an accusation. Collapsing the two would have a
/// verdict say "invalid" about a receipt this build never examined.
///
/// **No key is carried whole.** A verdict is a published document, and a full hex key in a
/// message is a value a reader can mistake for a pin. Eight characters are enough to tell
/// two keys apart and are not a key.
fn receipt_refusal(receipt: &StartReceipt, server_key: Option<&str>) -> Option<Refusal> {
    let Some(key) = server_key else {
        return Some(Refusal {
            code: "receipt_unverified",
            fail: false,
            message: "no recognised server key applies to this trace, so the receipt's \
                      signature was carried and not checked; a start nobody can verify is \
                      not a binding, and it is not a forgery either"
                .to_string(),
            note: "no recognised server key resolved, so nothing was checked",
        });
    };
    match receipt.verify_with(key) {
        Ok(()) => None,
        Err(ReceiptError::KeyMismatch { found, pinned }) => Some(Refusal {
            code: "receipt_invalid",
            fail: true,
            message: format!(
                "the receipt names public key {}… and this build pins {}…: it was signed by \
                 somebody other than the server, and a start the participant signed \
                 themselves binds the run to nobody",
                short(&found),
                short(&pinned),
            ),
            note: "signature",
        }),
        Err(ReceiptError::SignatureInvalid) => Some(Refusal {
            code: "receipt_invalid",
            fail: true,
            message: "the receipt names the pinned server public key and its claims do not \
                      verify against it: a claim was edited after the server signed it"
                .to_string(),
            note: "signature",
        }),
        // A signature that is not 128 hex characters is not a signature the server ever
        // produced, so it is refused rather than excused: the defect is in the document
        // the participant handed over, not in this build's ability to check it.
        Err(ReceiptError::SignatureMalformed) => Some(Refusal {
            code: "receipt_invalid",
            fail: true,
            message: "the receipt's signature is not 128 hex characters, so it is not a \
                      signature the server could have produced"
                .to_string(),
            note: "signature",
        }),
        // `PinnedKeyMalformed` and `Serialise` are this build failing to perform the
        // check, not the receipt failing it.
        Err(other) => Some(Refusal {
            code: "receipt_unverified",
            fail: false,
            message: format!(
                "the receipt's signature could not be checked: {other}; a start nobody can \
                 verify is not a binding"
            ),
            note: "the signature could not be checked",
        }),
    }
}

/// Whether the trace's declared baseline is bound to anything.
///
/// Computed here and read twice: V13 renders it as a finding, and `verify_trace` consults
/// the baseline in `attribute_seal` only where this says it is `Bound`. One definition, so
/// the verdict cannot say a baseline was refused while the attributor was exempting files
/// on the strength of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineBinding {
    /// A legacy M0 trace: it declares `[[leg1]]` stores and no chain, so there is nothing a
    /// baseline could have been chained into. Exempt for the same reason it is exempt from
    /// `receipt_missing`, and its descriptor's baseline — if it has one — is read as
    /// declared.
    Legacy,
    /// Neither the descriptor nor the chain says anything about what the workspace held
    /// at t0.
    Missing,
    /// The chain's seq 0 is a `Baseline` link and `baseline.json` hashes to the digest it
    /// claims. The only state in which a sealed file may be exempted as `Baseline`.
    Bound,
    /// One of the ways the file and the chain can part company.
    NotChained(NotChained),
}

/// The baseline beside the chain, said apart — the mirror of `seal_not_chained`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotChained {
    /// The descriptor names a baseline and the chain's first link is not one.
    NotTheFirstLink,
    /// Both exist and the chain claims a digest this `baseline.json` does not hash to.
    DigestDiffers,
    /// Digest agrees, but the manifest's binding claims do not agree with its descriptor
    /// and linked event.
    ClaimsDiffer,
    /// The chain carries a baseline the participant did not hand over.
    NoManifest,
}

impl BaselineBinding {
    /// Whether `attribute_seal` may exempt a sealed file on this baseline's word.
    ///
    /// `Bound` because the chain covers the list, and `Legacy` because a trace with no
    /// chain has nothing that could have covered it — the M0 captures predate the daemon.
    /// Every other state is a list the participant wrote and nothing stands behind, and it
    /// exempts nothing.
    pub fn attributes(&self) -> bool {
        matches!(self, BaselineBinding::Legacy | BaselineBinding::Bound)
    }
}

/// Is the declared baseline the chain's first link, and is it this file?
///
/// **Only seq 0 counts.** A baseline chained anywhere else describes a workspace the agent
/// had already been writing to, and `TraceExport::write_baseline` refuses to chain one
/// there — so a chain carrying a `Baseline` link above seq 0 is one somebody edited.
pub fn baseline_binding(
    chain: Option<&VerifiedChain>,
    baseline: Option<&SealManifest>,
    trace: &crate::trace::TraceDescriptor,
) -> BaselineBinding {
    let Some(chain) = chain else {
        return BaselineBinding::Legacy;
    };
    let chained = match chain.baseline_link {
        Some(0) => chained_baseline(chain),
        _ => None,
    };
    match (baseline, chained) {
        (None, None) => BaselineBinding::Missing,
        (None, Some(_)) => BaselineBinding::NotChained(NotChained::NoManifest),
        (Some(_), None) => BaselineBinding::NotChained(NotChained::NotTheFirstLink),
        // Recomputed from the manifest this run loaded, through the one definition the
        // writer stamped the link with, so a `baseline.json` re-indented after capture
        // still yields the digest the chain claims and one whose content moved does not.
        // A manifest this build cannot re-serialise is not one it can say agrees with the
        // chain: the consequence of a wrong `Bound` is an exemption, so it fails closed.
        (Some(m), Some(chained)) => match goalcup_protocol::export::manifest_sha256(m) {
            Ok(own) if own != chained.manifest_sha256 => {
                BaselineBinding::NotChained(NotChained::DigestDiffers)
            }
            Ok(_) if !manifest_claim_mismatches(trace, m, &chained.claims).is_empty() => {
                BaselineBinding::NotChained(NotChained::ClaimsDiffer)
            }
            Ok(_) => BaselineBinding::Bound,
            Err(_) => BaselineBinding::NotChained(NotChained::DigestDiffers),
        },
    }
}

/// The baseline is the chain's first link, or it is a list the participant wrote about
/// their own workspace.
///
/// **Why seq 0 is enough.** The receipt's `chain_head` is a claim about the genesis link
/// and `chain_head_mismatch` above already compares it against `chain.genesis_link`. So a
/// baseline that is bound cannot be rewritten after the receipt was issued without moving
/// seq 0's hash — which moves `genesis_link` away from the head the server witnessed, and
/// the trace fails on `chain_head_mismatch` before it ever reaches this function. The
/// binding is the receipt's; this check is what makes the baseline part of it.
fn baseline_is_a_link(cx: &CheckContext, chain: &VerifiedChain, findings: &mut Vec<Finding>) {
    let cause = match baseline_binding(Some(chain), cx.baseline, cx.trace) {
        BaselineBinding::Legacy | BaselineBinding::Bound => return,
        BaselineBinding::Missing => {
            findings.push(fail(
                "baseline_missing",
                "this trace declares a chain and no R3 baseline, so nothing says what the \
                 workspace held before the run started: every sealed file is owed a traced \
                 write, and a workspace that legitimately began from a checkout has no way \
                 to say so"
                    .to_string(),
                Vec::new(),
            ));
            return;
        }
        BaselineBinding::NotChained(cause) => cause,
    };

    let message = match cause {
        NotChained::NotTheFirstLink => format!(
            "the descriptor names a baseline manifest and the chain's first link is not a \
             baseline: {}. A baseline nothing covers is a list the participant's own daemon \
             wrote about their own workspace, so a pre-populated directory could be entered \
             into it wholesale and every planted file would read as one the workspace \
             already held",
            match chain.baseline_link {
                Some(seq) => format!(
                    "the chain carries one at seq {seq}, and a baseline \
                     taken after the run started describes a workspace the agent has been \
                     writing to"
                ),
                None => "the chain carries no baseline link at all".to_string(),
            }
        ),
        NotChained::DigestDiffers => format!(
            "the chain's baseline link claims manifest sha256 {}, and the baseline this \
             trace declares hashes to {}: a baseline is chained, but not this one",
            chain.baseline_digest.as_deref().unwrap_or("nothing"),
            cx.baseline
                .and_then(|m| goalcup_protocol::export::manifest_sha256(m).ok())
                .unwrap_or_else(|| "a digest this build could not compute".to_string()),
        ),
        NotChained::ClaimsDiffer => {
            let fields = match (cx.baseline, chained_baseline(chain)) {
                (Some(m), Some(chained)) => manifest_claim_mismatches(cx.trace, m, &chained.claims)
                    .iter()
                    .map(ManifestClaimMismatch::field)
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            format!(
                "the baseline manifest and its linked event disagree about {}. These claims \
                 bind the manifest to this entry, workspace, timestamp and complete file \
                 list; a digest match alone cannot grant attribution exemptions",
                and_list(&fields)
            )
        }
        NotChained::NoManifest => "the chain's first link is a baseline and the descriptor \
             names no baseline manifest: the participant did not hand over the list the \
             chain covers, so there is nothing to check the claim against and nothing a \
             sealed file can be exempted by"
            .to_string(),
    };
    findings.push(fail(
        "baseline_not_chained",
        message,
        vec![Evidence {
            store: "baseline".to_string(),
            seq: chain.baseline_link,
            sha256: chain.baseline_digest.clone(),
            note: "the digest the chain claims for the baseline".to_string(),
        }],
    ));
}

#[derive(Debug, Clone)]
struct LinkedManifestClaims {
    entry_id: String,
    recorded_at: String,
    file_count: u64,
    seal_body_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestClaimMismatch {
    ManifestEntryId,
    LinkedEntryId,
    Workspace,
    RecordedAt,
    FileCount,
    SealBodyTimestamp,
}

impl ManifestClaimMismatch {
    fn field(&self) -> &'static str {
        match self {
            Self::ManifestEntryId => "manifest entry_id",
            Self::LinkedEntryId => "linked event entry_id",
            Self::Workspace => "workspace",
            Self::RecordedAt => "linked event recorded_at",
            Self::FileCount => "linked event file_count",
            Self::SealBodyTimestamp => "seal body sealed_at",
        }
    }
}

fn manifest_claim_mismatches(
    trace: &crate::trace::TraceDescriptor,
    manifest: &SealManifest,
    linked: &LinkedManifestClaims,
) -> Vec<ManifestClaimMismatch> {
    let mut mismatches = Vec::new();
    if manifest.entry_id != trace.entry_id {
        mismatches.push(ManifestClaimMismatch::ManifestEntryId);
    }
    if linked.entry_id != trace.entry_id {
        mismatches.push(ManifestClaimMismatch::LinkedEntryId);
    }
    if manifest.workspace != trace.workspace {
        mismatches.push(ManifestClaimMismatch::Workspace);
    }
    if linked.recorded_at != manifest.sealed_at {
        mismatches.push(ManifestClaimMismatch::RecordedAt);
    }
    if linked.file_count != manifest.files.len() as u64 {
        mismatches.push(ManifestClaimMismatch::FileCount);
    }
    if linked
        .seal_body_at
        .as_ref()
        .is_some_and(|sealed_at| sealed_at != &manifest.sealed_at)
    {
        mismatches.push(ManifestClaimMismatch::SealBodyTimestamp);
    }
    mismatches
}

/// A linked manifest claim, read off the envelope that carries it.
struct ChainedManifest {
    manifest_sha256: String,
    sealed_at: String,
    claims: LinkedManifestClaims,
}

fn conservative_disputed_seal_cutoff(
    declared: &SealManifest,
    chained: &ChainedManifest,
) -> (&'static str, String) {
    let candidates = [
        declared.sealed_at.as_str(),
        chained.claims.recorded_at.as_str(),
        chained.sealed_at.as_str(),
    ];

    // A malformed claim cannot be ordered safely. Keep it as the selected bound so the
    // ordinary `window_unmeasurable` path refuses to certify a widened interval.
    if let Some(unreadable) = candidates.iter().find(|at| parse_z(at).is_none()) {
        return (
            "the disputed seal timestamp claims",
            (*unreadable).to_string(),
        );
    }

    let earliest = candidates
        .into_iter()
        .min_by_key(|at| parse_z(at).expect("all disputed seal timestamps were parsed"))
        .expect("a disputed seal always has timestamp claims");
    (
        "the earliest disputed seal timestamp claim",
        earliest.to_string(),
    )
}

fn chained_baseline(chain: &VerifiedChain) -> Option<ChainedManifest> {
    let seq = chain.baseline_link?;
    chain.envelopes.iter().find_map(|e| match &e.body {
        EventBody::Baseline {
            manifest_sha256,
            file_count,
        } if e.link.seq == seq => Some(ChainedManifest {
            manifest_sha256: manifest_sha256.clone(),
            sealed_at: e.recorded_at.clone(),
            claims: LinkedManifestClaims {
                entry_id: e.entry_id.clone(),
                recorded_at: e.recorded_at.clone(),
                file_count: *file_count,
                seal_body_at: None,
            },
        }),
        _ => None,
    })
}

fn chained_seal(chain: &VerifiedChain) -> Option<ChainedManifest> {
    let seq = chain.seal_link?;
    chain.envelopes.iter().find_map(|e| match &e.body {
        EventBody::Seal {
            manifest_sha256,
            sealed_at,
            file_count,
        } if e.link.seq == seq => Some(ChainedManifest {
            manifest_sha256: manifest_sha256.clone(),
            sealed_at: sealed_at.clone(),
            claims: LinkedManifestClaims {
                entry_id: e.entry_id.clone(),
                recorded_at: e.recorded_at.clone(),
                file_count: *file_count,
                seal_body_at: Some(sealed_at.clone()),
            },
        }),
        _ => None,
    })
}

/// The seal beside the chain, said three ways apart.
fn seal_is_a_link(cx: &CheckContext, chain: &VerifiedChain, findings: &mut Vec<Finding>) {
    if cx.trace.seal.is_none() {
        return;
    }
    let Some(seq) = chain.seal_link else {
        findings.push(fail(
            "seal_not_chained",
            "the descriptor names a seal manifest and the chain carries no seal link: with \
             no seal in the chain, an acknowledgement over the head anchors whatever record \
             happened to be last and covers no byte of the manifest, so the seal is \
             self-asserted"
                .to_string(),
            Vec::new(),
        ));
        return;
    };

    // The digest the chain claims, read off the link that claims it. The same lookup the
    // window's upper bound uses, so the two can never disagree about which seal the chain
    // carries.
    let chained = chained_seal(chain);
    // Recomputed from the manifest this run loaded, not from the file's bytes:
    // `export::manifest_sha256` is the one definition, shared with the writer that stamped
    // the link, so a `seal.json` re-indented after capture still yields the digest the
    // chain claims and one whose content moved does not.
    let declared = cx
        .seal
        .and_then(|m| goalcup_protocol::export::manifest_sha256(m).ok());
    // A comparison that could not be made asserts nothing: `None` on either side is a
    // digest this run could not compute, not a disagreement it found.
    if let (Some(chained), Some(declared)) = (&chained, &declared)
        && chained.manifest_sha256 != *declared
    {
        findings.push(fail(
            "seal_not_chained",
            format!(
                "the chain's seal link at seq {seq} claims manifest sha256 {}, and \
                 the seal this trace declares hashes to {declared}: a seal is chained, but \
                 not this one",
                chained.manifest_sha256
            ),
            vec![Evidence::at(
                "seal",
                seq,
                Some(declared.clone()),
                "the declared manifest's own digest",
            )],
        ));
        return;
    }

    if let (Some(chained), Some(manifest)) = (&chained, cx.seal) {
        let mismatches = manifest_claim_mismatches(cx.trace, manifest, &chained.claims);
        if !mismatches.is_empty() {
            let fields: Vec<_> = mismatches
                .iter()
                .map(ManifestClaimMismatch::field)
                .collect();
            findings.push(fail(
                "seal_not_chained",
                format!(
                    "the seal manifest and its linked event disagree about {}. These claims \
                     bind the manifest to this entry, workspace, timestamp and complete \
                     file list; a digest match alone cannot close the run",
                    and_list(&fields)
                ),
                vec![Evidence::at(
                    "seal",
                    seq,
                    declared.clone(),
                    "manifest/link binding claims differ",
                )],
            ));
            return;
        }
    }

    if seq != chain.head_seq {
        findings.push(fail(
            "seal_not_chained",
            format!(
                "the seal is chained at seq {seq} and the chain's last link is seq {}: {} \
                 link(s) follow a seal, and a seal with records after it closes nothing",
                chain.head_seq,
                chain.head_seq.saturating_sub(seq),
            ),
            vec![Evidence::at(
                "chain",
                seq,
                None,
                format!("the seal link; the chain ends at seq {}", chain.head_seq),
            )],
        ));
    }
}

/// `a`, `a and b`, `a, b and c` — so a message names its fields the way a sentence does.
fn and_list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The first eight characters of a hex value: enough to tell two keys apart, and not a key.
fn short(hex: &str) -> String {
    hex.chars().take(8).collect()
}

/// The first 32 characters of a participant-written string. Counted in `char`s rather than
/// bytes, so a multi-byte value is truncated at a boundary and never panics.
fn first_32(value: &str) -> String {
    value.chars().take(32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{
        EnvelopeCompatibilityIssue, LegacyRedactionLogMissing, RedactionAuditIssue,
        RedactionAuditIssueReason,
    };
    use crate::checks::Severity;
    use crate::{ManifestPin, ManifestPinning};

    #[test]
    fn structural_renders_invalid_v2_audit_as_fail_and_legacy_ambiguity_as_flag() {
        let chain = VerifiedChain {
            entry_id: "e1".to_string(),
            genesis_link: "00".repeat(32),
            head_seq: 0,
            head_hash: "00".repeat(32),
            envelopes: Vec::new(),
            stores: Vec::new(),
            posts: crate::legs::Leg2Stream { posts: Vec::new() },
            events: Vec::new(),
            redactions: Vec::new(),
            seal_link: None,
            baseline_link: None,
            baseline_digest: None,
            break_at: None,
            truncated_tail: None,
            unsupported_version: None,
            incompatible_envelope: Some(EnvelopeCompatibilityIssue {
                seq: 3,
                reason:
                    goalcup_protocol::envelope::EnvelopeVersionCompatibilityError::
                        HomeSeededRequiresV3 {
                            protocol_version: goalcup_protocol::LEGACY_PROTOCOL_VERSION,
                        },
            }),
            redaction_audit_issues: vec![RedactionAuditIssue {
                seq: 4,
                store: "main".to_string(),
                reason: RedactionAuditIssueReason::EmptySpans,
            }],
            legacy_redaction_log_missing: vec![LegacyRedactionLogMissing {
                seq: 7,
                store: "hooks".to_string(),
            }],
        };
        let mut findings = Vec::new();
        structural(&chain, &mut findings);

        let invalid = findings
            .iter()
            .find(|f| f.code == "redaction_audit_invalid")
            .expect("invalid audit is rendered");
        assert_eq!(invalid.severity, Severity::Fail);
        assert_eq!(
            (invalid.evidence[0].store.as_str(), invalid.evidence[0].seq),
            ("main", Some(4))
        );

        let missing = findings
            .iter()
            .find(|f| f.code == "legacy_redaction_log_missing")
            .expect("legacy ambiguity is rendered");
        assert_eq!(missing.severity, Severity::Flag);
        assert!(missing.message.contains("missing audit evidence"));
        assert_eq!(
            (missing.evidence[0].store.as_str(), missing.evidence[0].seq),
            ("hooks", Some(7))
        );

        let incompatible = findings
            .iter()
            .find(|f| f.code == "envelope_version_body_incompatible")
            .expect("version/body incompatibility is rendered");
        assert_eq!(incompatible.severity, Severity::Fail);
        assert_eq!(incompatible.evidence[0].seq, Some(3));
    }

    /// The three readings of the pin, and the two severities each unpinned one has.
    ///
    /// `Unpinned` cannot be reached through `verify_trace` while every connector this build
    /// has a parser for is also pinned — a descriptor naming another connector is refused
    /// before any check runs — so this is where that arm is exercised.
    #[test]
    fn a_matching_pin_says_nothing_and_the_two_refusals_say_which_reading_applied() {
        const READ: &str = "abababababababababababababababababababababababababababababababab";
        const PINNED: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
        let read = READ;
        let pinned = PINNED;

        let quiet = ManifestPinning {
            state: ManifestPin::Pinned,
            pinned: None,
            allowed: false,
            tables: crate::PinTables::CURRENT_BUILD,
        };
        assert_eq!(manifest_pin_finding(quiet, read), None);

        let altered = ManifestPinning {
            state: ManifestPin::Altered,
            pinned: Some(PINNED),
            allowed: false,
            tables: crate::PinTables::CURRENT_BUILD,
        };
        let f = manifest_pin_finding(altered, read).expect("a manifest nobody pins");
        assert_eq!(f.code, "manifest_not_pinned");
        assert_eq!(f.severity, Severity::Fail);
        // Both digests are named: the one this build pins and the one that was read.
        assert!(f.message.contains(pinned), "{}", f.message);
        assert!(f.message.contains(read), "{}", f.message);
        assert!(
            !f.message.contains("--dev"),
            "a refused run must not read as a downgraded one: {}",
            f.message
        );
        assert_eq!(f.evidence[0].sha256.as_deref(), Some(read));

        let dev = manifest_pin_finding(
            ManifestPinning {
                allowed: true,
                ..altered
            },
            read,
        )
        .expect("the finding survives the downgrade");
        assert_eq!(dev.severity, Severity::Flag);
        assert!(dev.message.contains("--dev"), "{}", dev.message);

        let unpinned = ManifestPinning {
            state: ManifestPin::Unpinned,
            pinned: None,
            allowed: false,
            tables: crate::PinTables::CURRENT_BUILD,
        };
        let f = manifest_pin_finding(unpinned, read).expect("a connector nobody pins");
        assert_eq!(f.severity, Severity::Fail);
        assert!(f.message.contains("pins no manifest"), "{}", f.message);
        let dev = manifest_pin_finding(
            ManifestPinning {
                allowed: true,
                ..unpinned
            },
            read,
        )
        .expect("the finding survives the downgrade");
        assert_eq!(dev.severity, Severity::Flag);
        assert!(dev.message.contains("--dev"), "{}", dev.message);
    }

    #[test]
    fn a_field_list_reads_as_a_sentence() {
        assert_eq!(and_list(&[]), "");
        assert_eq!(and_list(&["workspace"]), "workspace");
        assert_eq!(
            and_list(&["entry_id", "workspace"]),
            "entry_id and workspace"
        );
        assert_eq!(
            and_list(&["entry_id", "connector", "workspace"]),
            "entry_id, connector and workspace"
        );
    }

    #[test]
    fn manifest_link_claims_bind_each_identity_workspace_time_and_count_field() {
        let trace = crate::checks::support::trace("codex-cli");
        let manifest = SealManifest {
            entry_id: trace.entry_id.clone(),
            sealed_at: "2026-09-10T00:00:00Z".to_string(),
            workspace: trace.workspace.clone(),
            files: Vec::new(),
            excluded: Vec::new(),
        };
        let linked = LinkedManifestClaims {
            entry_id: trace.entry_id.clone(),
            recorded_at: manifest.sealed_at.clone(),
            file_count: 0,
            seal_body_at: None,
        };
        assert!(manifest_claim_mismatches(&trace, &manifest, &linked).is_empty());

        let mut changed = manifest.clone();
        changed.entry_id = "other-entry".to_string();
        assert_eq!(
            manifest_claim_mismatches(&trace, &changed, &linked),
            vec![ManifestClaimMismatch::ManifestEntryId]
        );

        let mut changed = linked.clone();
        changed.entry_id = "other-entry".to_string();
        assert_eq!(
            manifest_claim_mismatches(&trace, &manifest, &changed),
            vec![ManifestClaimMismatch::LinkedEntryId]
        );

        let mut changed = manifest.clone();
        changed.workspace = "/other-workspace".to_string();
        assert_eq!(
            manifest_claim_mismatches(&trace, &changed, &linked),
            vec![ManifestClaimMismatch::Workspace]
        );

        let mut changed = linked.clone();
        changed.recorded_at = "2026-09-10T00:00:01Z".to_string();
        assert_eq!(
            manifest_claim_mismatches(&trace, &manifest, &changed),
            vec![ManifestClaimMismatch::RecordedAt]
        );

        let mut changed = linked.clone();
        changed.file_count = 1;
        assert_eq!(
            manifest_claim_mismatches(&trace, &manifest, &changed),
            vec![ManifestClaimMismatch::FileCount]
        );

        let mut changed = linked;
        changed.seal_body_at = Some("2026-09-10T00:00:01Z".to_string());
        assert_eq!(
            manifest_claim_mismatches(&trace, &manifest, &changed),
            vec![ManifestClaimMismatch::SealBodyTimestamp]
        );
    }

    #[test]
    fn a_digest_matching_baseline_with_wrong_binding_claims_grants_no_attribution() {
        let trace = crate::checks::support::trace("codex-cli");
        let baseline = SealManifest {
            entry_id: trace.entry_id.clone(),
            sealed_at: "2026-09-10T00:00:00Z".to_string(),
            workspace: "/wrong-workspace".to_string(),
            files: Vec::new(),
            excluded: Vec::new(),
        };
        let tmp = tempfile::tempdir().unwrap();
        let mut export = goalcup_protocol::export::TraceExport::create(tmp.path()).unwrap();
        export.write_baseline(&baseline).unwrap();
        let text = std::fs::read_to_string(tmp.path().join("chain.jsonl")).unwrap();
        let chain = crate::chain::load_and_verify_text("chain.jsonl", &text).unwrap();

        let binding = baseline_binding(Some(&chain), Some(&baseline), &trace);
        assert_eq!(
            binding,
            BaselineBinding::NotChained(NotChained::ClaimsDiffer)
        );
        assert!(!binding.attributes());
    }

    /// A key never reaches a message whole, and a short value is not padded into one.
    #[test]
    fn a_key_is_abbreviated_and_never_carried_whole() {
        let key = "0123456789abcdef".repeat(4);
        assert_eq!(short(&key), "01234567");
        assert_eq!(short("abc"), "abc");
        assert_eq!(short(""), "");
    }
}
