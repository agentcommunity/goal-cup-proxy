//! `goalcup seal` — end a capture run early, report how it finished, and say where the trace
//! is.
//!
//! # The three things this command does not do
//!
//! **It does not seal.** The daemon does: `POST /v0/entries/{id}/seal` with a
//! [`CaptureSealRequest`] finishes the capture and answers with its [`CaptureFinishReport`].
//! This command asks for that and reports it.
//!
//! **It does not signal anything.** D22: the daemon terminates the registered process tree
//! per `kill.strategy`, because a seal can come when the participant's terminal is long gone,
//! and a kill that only worked from the terminal would be a kill that only worked when
//! somebody was watching. There is exactly one terminator, and it is not here.
//!
//! **It does not write the trace.** The trace belongs to the daemon's export, start to
//! finish. This command prints its path and the `goalcup-verify --capture-integrity` line —
//! which is also why there is no `goalcup export`.
//!
//! # `busy` is ridden out
//!
//! `busy` is the daemon holding the entry for the length of one asynchronous call — ridden out
//! by [`retry_capture_seal`] within the local-finish budget, and reported at rung 1 if it is
//! still busy when that is spent.

use crate::CliError;
use crate::entries::read_entry;
use crate::retry::{BUSY_ATTEMPTS, retry_busy, retry_capture_seal, retry_capture_seal_until};
use crate::upload::policy::WrapUp;
use goalcup_protocol::control::{
    CaptureFinishReport, CaptureSealError, CaptureSealRequest, ControlError, DaemonClient,
    LocalSealOutcome, ProxyCounters,
};
use goalcup_protocol::process::termination_bound;
use goalcup_protocol::provenance::{TerminationFailureV1, TerminationOutcomeV1};
use std::path::Path;

/// The verifier, which is a separate binary and not a sub-command of this one.
///
/// A constant because it is printed to a participant as a command to run, and a binary
/// spelled twice is a binary that is one day spelled wrong in the place somebody copies from.
pub(crate) const VERIFIER_BIN: &str = "goalcup-verify";

/// The testable half: everything except resolving a real client and a real config dir.
///
/// `connectors`, `cfg` and `out` are parameters and not lookups for the reason the rest of
/// this crate takes them that way — a command that printed with `println!` could not be
/// asserted at all.
pub fn seal_with(
    client: &dyn DaemonClient,
    connectors: &Path,
    cfg: &Path,
    entry_id: &str,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    // The cached record establishes which entry this machine means, and names its connector.
    let record = read_entry(cfg, entry_id)?;
    let status = retry_busy(|| client.status(entry_id));
    // An entry this machine recorded and the daemon has no status for (H-15): the one case the
    // bare seal below can be answered `malformed` by a daemon that holds the row as a capture.
    let status_unknown = matches!(status, Err(ControlError::NotFound(_)));

    // A local capture entry finishes locally. The launcher that ran its proxy is gone
    // or still waiting on the harness, so this seal carries no proxy counters; the daemon
    // ends the harness it owns. The one local-finish budget starts here, after the status
    // read, and the harness's termination keeps its own bound outside it: the allowance is
    // the connector's own grace (from its cached manifest), or the slow-call timeout when
    // that manifest cannot be read.
    if status
        .as_ref()
        .is_ok_and(|status| status.payload.capture.is_some())
    {
        let deadline = std::time::Instant::now() + crate::finish_budget();
        let allowance =
            match crate::connector::load_harness(connectors, &record.connector_id, record.dev) {
                Ok(connector) => {
                    termination_bound(connector.loaded.manifest.termination().grace_seconds())
                }
                Err(_) => crate::daemon::SLOW_CALL_TIMEOUT,
            };
        // A standalone seal sends no counters and no native scope: whatever identity the
        // harness printed after it was asked to stop is a later fact (seams design v2, 3).
        return finish_locally(client, entry_id, None, None, deadline, allowance, None, out)
            .map(|_| ());
    }

    // No capture status: the bare seal is the only one this command can send. The daemon
    // answers it `malformed` for a capture row it could not reopen, and `missing` for an id
    // it does not hold; it never answers it with a report this command could print, and never
    // with `busy` (the route reads only the registry row), so the generic retry is enough.
    match retry_busy(|| client.seal(entry_id)) {
        Ok(_) => Err(CliError::Control(ControlError::Malformed(format!(
            "the tracker answered the seal of entry {entry_id} with a pre-v5 seal report, which \
             this build does not read"
        )))),
        Err(ControlError::Busy(said)) => Err(CliError::Refused(format!(
            "the tracker is still busy with entry {entry_id} while retrying (up to {} attempts): {said}. \
                 The entry is held, not broken, and no seal manifest was returned to this command. \
                 The daemon may still be completing a seal — wait a moment and \
                 run `goalcup seal {entry_id}` again.",
            BUSY_ATTEMPTS
        ))),
        // The daemon has the entry's row as a v5 capture but no status and no open capture: its
        // chain could not be reopened (a torn tail, an unreadable file), so there is nothing a
        // capture seal could finish. The CLI sent the only seal its status let it choose, and the
        // daemon's `malformed` is about that choice, not about a response this CLI cannot read.
        Err(ControlError::Malformed(said)) if status_unknown => {
            Err(torn_entry_refusal(entry_id, &said))
        }
        Err(other) => Err(CliError::Control(other)),
    }
}

/// Finish a local capture entry and say how it finished, where its trace is, and the one
/// command that checks it. `proxy` is the launcher's proxy report: `goalcup start
/// --goal-file` hands it over when its harness exits, and this command has none.
///
/// The finish runs inside `deadline`, the one local-finish budget its caller started
/// (CONTRACTS.md, H-10 4a). Every attempt states what remains of it, and its transport timeout
/// is that plus `termination_allowance` (the harness's termination is outside the budget) plus
/// the connect slack. A refusal or an ambiguous outcome is an exit-1 refusal with a fixed
/// sentence; the daemon answers a repeated finish with the one it committed.
///
/// `reconnect` is the configuration directory to re-read `control.json` from when the daemon was
/// restarted since `client` was built (H-14, [`crate::daemon::seal_capture_following_restart`]);
/// a standalone seal builds its client moments before and passes `None`.
///
/// A local finish has no receipt, acknowledgement or upload, and says so: its anchor is this
/// machine.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_locally(
    client: &dyn DaemonClient,
    entry_id: &str,
    proxy: Option<ProxyCounters>,
    native_scope: Option<String>,
    deadline: std::time::Instant,
    termination_allowance: std::time::Duration,
    reconnect: Option<&Path>,
    out: &mut dyn std::io::Write,
) -> Result<CaptureFinishReport, CliError> {
    let finish = seal_locally(
        client,
        entry_id,
        proxy,
        native_scope,
        deadline,
        termination_allowance,
        reconnect,
    )?;
    report_local_finish(entry_id, &finish, out)?;
    Ok(finish)
}

/// The seal request of [`finish_locally`] without its report: what a caller needs when the seal
/// has to count as committed whether or not this command's own output can still be written.
/// Nothing is printed here. One attempt, within `deadline`.
pub(crate) fn seal_locally(
    client: &dyn DaemonClient,
    entry_id: &str,
    proxy: Option<ProxyCounters>,
    native_scope: Option<String>,
    deadline: std::time::Instant,
    termination_allowance: std::time::Duration,
    reconnect: Option<&Path>,
) -> Result<CaptureFinishReport, CliError> {
    seal_locally_until(
        client,
        entry_id,
        proxy,
        native_scope,
        deadline,
        WrapUp::one_attempt(),
        termination_allowance,
        reconnect,
    )
}

/// [`seal_locally`] asked again, with growing waits, until the wrap-up `until` has run out: the
/// stop of a run must end in a seal, and one attempt's ten seconds are not the whole of what a
/// busy or restarting daemon gets (`retry_capture_seal_until` says which failures are asked
/// again).
///
/// The first attempt is within `first_deadline`, the one local-finish budget its caller took
/// (and shared with the proxy's stop and the tees); each later attempt has a whole budget of its
/// own, and follows a daemon restarted since the one before.
#[allow(clippy::too_many_arguments)]
pub(crate) fn seal_locally_until(
    client: &dyn DaemonClient,
    entry_id: &str,
    proxy: Option<ProxyCounters>,
    native_scope: Option<String>,
    first_deadline: std::time::Instant,
    until: WrapUp,
    termination_allowance: std::time::Duration,
    reconnect: Option<&Path>,
) -> Result<CaptureFinishReport, CliError> {
    let mut waited = std::time::Duration::ZERO;
    let mut asked = 0u32;
    let mut say = |line: &str| eprintln!("goalcup: {line}");
    let finish: CaptureFinishReport = retry_capture_seal_until(until, &mut say, || {
        asked += 1;
        let deadline = if asked == 1 {
            first_deadline
        } else {
            std::time::Instant::now() + crate::finish_budget()
        };
        let mut fresh = None;
        retry_capture_seal(deadline, |remaining| {
            // Whole milliseconds, rounded down: the request and the timeout state the same number.
            let remaining = std::time::Duration::from_millis(
                u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX),
            );
            let request = CaptureSealRequest {
                proxy,
                remaining_finish_ms: Some(u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX)),
                native_scope: native_scope.clone(),
            };
            waited = crate::daemon::capture_seal_read_timeout(remaining, termination_allowance);
            // A daemon restarted since the caller's client was built is at another port or
            // token: `reconnect` is the configuration directory to read that from (H-14).
            crate::daemon::seal_capture_following_restart(
                client, &mut fresh, reconnect, entry_id, &request, waited,
            )
        })
    })
    .map_err(|failure| capture_finish_refusal(entry_id, failure, waited))?;
    if finish.entry_id != entry_id {
        return Err(CliError::Control(ControlError::Malformed(format!(
            "seal response belongs to entry {}, not requested entry {entry_id}",
            finish.entry_id
        ))));
    }
    Ok(finish)
}

/// The lines a local finish tells the participant, from the report the daemon committed.
pub(crate) fn report_local_finish(
    entry_id: &str,
    finish: &CaptureFinishReport,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let terminal = match finish.terminal_seq {
        Some(seq) => format!("its terminal record is {seq}"),
        None => "no terminal record was written".to_string(),
    };
    match finish.seal {
        LocalSealOutcome::SealWritten => say(
            out,
            &format!("entry {entry_id}: finished locally; {terminal} and seal.json binds it."),
        )?,
        LocalSealOutcome::SealWriteFailed => say(
            out,
            &format!(
                "entry {entry_id}: finished locally, but seal.json could not be written \
                 ({terminal}), so the trace reads incomplete."
            ),
        )?,
    }
    // A run opened on an upload server carries its signed start receipt in the trace.
    let receipt =
        std::path::Path::new(&finish.trace_dir).join(goalcup_protocol::export::RECEIPT_FILE);
    say(
        out,
        if receipt.is_file() {
            "anchor: the upload server's signed start receipt; its acknowledgements are added to \
             acks.jsonl as the trace uploads."
        } else {
            "anchor: local only. No receipt, acknowledgement or upload exists for this run."
        },
    )?;
    say(
        out,
        &format!(
            "harness termination: {}.",
            render_termination(&finish.termination)
        ),
    )?;
    say(out, &format!("trace:  {}", finish.trace_dir))?;
    say(
        out,
        &format!(
            "verify: {VERIFIER_BIN} --capture-integrity {}",
            crate::launch::quote(&finish.trace_dir)
        ),
    )
}

/// What `goalcup seal` says of an entry the tracker holds but cannot read the trace of. A safe
/// refusal at rung 1: nothing was sealed, nothing was written, nothing was repaired. A local
/// capture's record names no artifact root, so the trace directory is described and not printed.
fn torn_entry_refusal(entry_id: &str, daemon_said: &str) -> CliError {
    CliError::Refused(format!(
        "Entry {entry_id} cannot be sealed: the tracker holds its record but could not reopen its \
         trace (it is torn or unreadable, for example a write cut short), so there is no open \
         capture to finish. Nothing was sealed, written or repaired: the trace bytes are preserved \
         exactly as the crash left them. Read what survives with `{VERIFIER_BIN} \
         --capture-integrity <the entry's trace directory>` (the tracker keeps it under its \
         artifact root, in a folder named {entry_id}); this entry can be neither sealed nor \
         resumed, so start a new run for new work. (The tracker said: {daemon_said})"
    ))
}

/// What a capture seal that did not return a report means, in one sentence the participant
/// can act on. `waited` is the read timeout of the last request.
fn capture_finish_refusal(
    entry_id: &str,
    failure: CaptureSealError,
    waited: std::time::Duration,
) -> CliError {
    let admitted = !failure.not_admitted();
    match failure.into_error() {
        ControlError::FinishDeadline(stage) if stage == "run_lock" => CliError::Refused(format!(
            "Another finish of entry {entry_id} is still in progress. Run `goalcup seal \
             {entry_id}` again to read its result."
        )),
        ControlError::FinishDeadline(stage) => CliError::Refused(format!(
            "Entry {entry_id} did not finish within its {}-second local budget (stopped before \
             {stage}). Nothing terminal was written and the entry is still open. Run `goalcup \
             seal {entry_id}` to finish it; its trace will read incomplete.",
            crate::finish_budget().as_secs()
        )),
        // A transport failure or a read timeout after the request was written: the daemon may
        // still commit this finish, and a later seal answers a committed one unchanged.
        ControlError::Unreachable(_) if admitted => CliError::Refused(format!(
            "The tracker did not answer the finish of entry {entry_id} within {}s. It may still \
             complete it. Run `goalcup seal {entry_id}`: a finish it already committed is \
             answered unchanged.",
            waited.as_secs()
        )),
        error @ (ControlError::Unreachable(_) | ControlError::Unauthorized(_)) => {
            CliError::Refused(format!(
                "Entry {entry_id} was not finished: the tracker refused this command's \
                 connection or credentials ({error}) before the finish began. Run `goalcup seal \
                 {entry_id}`."
            ))
        }
        ControlError::Busy(said) => CliError::Refused(format!(
            "the tracker is still busy with entry {entry_id} while retrying (the {}-second \
             finish budget leaves no room for another attempt): {said}. The entry is held, not \
             broken. Run `goalcup seal {entry_id}` again.",
            crate::finish_budget().as_secs()
        )),
        other => CliError::Control(other),
    }
}

/// What the tracker says of the harness's end, in words. The variant name leads (it is what the
/// record and the verifier call the outcome), and a stop that could not be confirmed says what
/// to check.
pub(crate) fn render_termination(outcome: &TerminationOutcomeV1) -> String {
    match outcome {
        TerminationOutcomeV1::AlreadyStopped { checked_at } => format!(
            "AlreadyStopped, the harness had already ended when the tracker checked at {}",
            checked_at.as_str()
        ),
        TerminationOutcomeV1::Confirmed {
            requested_at,
            confirmed_at,
            method,
        } => format!(
            "Confirmed, the harness was asked to stop at {} and seen stopped at {} ({})",
            requested_at.as_str(),
            confirmed_at.as_str(),
            wire_words(method)
        ),
        TerminationOutcomeV1::Unresolved {
            requested_at,
            stopped_waiting_at,
            reason,
        } => format!(
            "Unresolved, the harness was asked to stop at {} and the tracker gave up waiting at \
             {} (reason: {}), so its stop is not confirmed; check that no process of this run \
             is still running before you start another in this folder",
            requested_at.as_str(),
            stopped_waiting_at.as_str(),
            failure_name(*reason)
        ),
    }
}

/// The name the record and the verifier give a failed stop. It stays an identifier, not words,
/// because it is what a participant searches the evidence and the docs for.
fn failure_name(reason: TerminationFailureV1) -> &'static str {
    match reason {
        TerminationFailureV1::ProcessAuthorityUnavailable => "ProcessAuthorityUnavailable",
        TerminationFailureV1::RequestFailed => "RequestFailed",
        TerminationFailureV1::ConfirmationTimedOut => "ConfirmationTimedOut",
        TerminationFailureV1::ProcessStillRunning => "ProcessStillRunning",
        TerminationFailureV1::UnsupportedPlatform => "UnsupportedPlatform",
    }
}

/// A protocol enum as the words of its wire name (`process_signal` is "process signal").
fn wire_words<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(word)) => word.replace('_', " "),
        _ => "unspecified".to_string(),
    }
}

/// One line to the participant, or the write error naming the stream.
fn say(out: &mut dyn std::io::Write, line: &str) -> Result<(), CliError> {
    writeln!(out, "{line}").map_err(|source| CliError::Io {
        path: "stdout".to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::render_termination;
    use goalcup_protocol::provenance::{
        Rfc3339Utc, TerminationFailureV1, TerminationMethodV1, TerminationOutcomeV1,
    };

    fn at(text: &str) -> Rfc3339Utc {
        Rfc3339Utc::try_from(text).unwrap()
    }

    /// The participant reads words, never the Rust debug form of the record.
    #[test]
    fn a_termination_reads_as_words_with_the_outcome_first() {
        let stopped = render_termination(&TerminationOutcomeV1::AlreadyStopped {
            checked_at: at("2026-10-02T10:00:00Z"),
        });
        assert!(stopped.starts_with("AlreadyStopped, "), "{stopped}");
        assert!(stopped.contains("2026-10-02T10:00:00Z"), "{stopped}");

        let confirmed = render_termination(&TerminationOutcomeV1::Confirmed {
            requested_at: at("2026-10-02T10:00:00Z"),
            confirmed_at: at("2026-10-02T10:00:01Z"),
            method: TerminationMethodV1::ProcessSignal,
        });
        assert!(confirmed.starts_with("Confirmed, "), "{confirmed}");
        assert!(confirmed.contains("(process signal)"), "{confirmed}");

        let unresolved = render_termination(&TerminationOutcomeV1::Unresolved {
            requested_at: at("2026-10-02T10:00:00Z"),
            stopped_waiting_at: at("2026-10-02T10:00:05Z"),
            reason: TerminationFailureV1::ConfirmationTimedOut,
        });
        assert!(unresolved.starts_with("Unresolved, "), "{unresolved}");
        assert!(unresolved.contains("ConfirmationTimedOut"), "{unresolved}");
        assert!(unresolved.contains("check that no process"), "{unresolved}");

        for text in [&stopped, &confirmed, &unresolved] {
            assert!(
                !text.contains("Rfc3339Utc")
                    && !text.contains('{')
                    && !text.contains("requested_at"),
                "{text}"
            );
        }
    }
}
