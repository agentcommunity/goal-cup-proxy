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
use crate::commands::say;
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

/// Refuse `goalcup seal` run from inside the harness: `in_harness` is
/// [`crate::launch::in_harness`], which every recording launch makes true in the harness's
/// environment, so the agent cannot end its own segment by accident. A seal typed in another
/// terminal is not in the harness, and the host's own seal at the end of a run does not come
/// here.
pub fn refuse_inside_harness(in_harness: bool) -> Result<(), CliError> {
    if in_harness {
        return Err(CliError::Refused(
            crate::participant::seal_inside_harness().to_string(),
        ));
    }
    Ok(())
}

/// `goalcup seal` as a person types it in their own terminal: ask first unless `yes`, seal, keep
/// the technical lines in the run's `host.log`, and say in plain words what happened and what
/// to do next. `input` is where the answer is read from (the terminal).
pub fn seal_for_person(
    client: &dyn DaemonClient,
    connectors: &Path,
    cfg: &Path,
    entry_id: &str,
    yes: bool,
    input: &mut dyn std::io::BufRead,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let facts = PartFacts::of(cfg, connectors, entry_id);
    if !yes {
        confirm(
            &crate::participant::seal_question(&facts.harness, facts.open_until.as_deref()),
            input,
            out,
        )?;
    }
    let mut details = Vec::new();
    let sealed = seal_entry(client, connectors, cfg, entry_id, &mut details);
    // The technical lines go to the run's log; with no run folder here, they are shown.
    let log = facts.log_path.as_deref().filter(|log| {
        crate::host_log::open_append(log)
            .and_then(|mut file| std::io::Write::write_all(&mut file, &details))
            .is_ok()
    });
    if log.is_none() {
        let _ = out.write_all(&details);
    }
    let outcome = sealed?;
    let lines = crate::participant::seal_done(&crate::participant::SealDone {
        harness: &facts.harness,
        part_id: entry_id,
        sealed: outcome != Some(LocalSealOutcome::SealWriteFailed),
        go_on: facts
            .go_on
            .as_ref()
            .map(|(run, until)| (run.as_str(), until.as_str())),
        open_until: facts.open_until.as_deref(),
        details: log,
    });
    for line in lines {
        say(out, &line)?;
    }
    Ok(())
}

/// Ask `question` and read one answer from `input`: only "yes" goes on. Anything else, or no
/// answer (no terminal), stops here with nothing done.
pub fn confirm(
    question: &str,
    input: &mut dyn std::io::BufRead,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    write!(out, "{question} ")
        .and_then(|()| out.flush())
        .map_err(|source| CliError::Io {
            path: "stdout".to_string(),
            source,
        })?;
    let mut answer = String::new();
    let _ = input.read_line(&mut answer);
    if answer.trim().eq_ignore_ascii_case("yes") {
        return Ok(());
    }
    // The answer ended no line of its own when there was none.
    let _ = writeln!(out);
    Err(CliError::Refused(crate::participant::seal_not_confirmed()))
}

/// What the plain words of a seal name: the harness, the run's end while it stays open, and
/// where the run's log is.
struct PartFacts {
    harness: String,
    /// The run's end ([`crate::participant::end_words`]) while the run stays open.
    open_until: Option<String>,
    /// The run and its end, when `goalcup continue` can start its next part.
    go_on: Option<(String, String)>,
    log_path: Option<std::path::PathBuf>,
}

impl PartFacts {
    fn of(cfg: &Path, connectors: &Path, entry_id: &str) -> PartFacts {
        let record = read_entry(cfg, entry_id).ok();
        let harness = record.as_ref().map_or_else(
            || "the agent".to_string(),
            |r| crate::commands::status::harness_name(Some(connectors), &r.connector_id, r.dev),
        );
        let Ok(Some(run)) = crate::run::of_entry(cfg, entry_id) else {
            return PartFacts {
                harness,
                open_until: None,
                go_on: None,
                log_path: None,
            };
        };
        let run_dir = crate::run::run_dir(cfg, &run.run_id);
        let state = crate::run::read_state(&run_dir);
        let now = crate::run::now_unix();
        let over = crate::run::is_over(&run, state.as_ref(), now);
        let left = crate::participant::run_seconds_left(&run, state.as_ref(), now);
        let open_until = run
            .deadline_at
            .as_deref()
            .filter(|_| run.server_url.is_some() && !over)
            .map(|deadline| crate::participant::end_words(deadline, left));
        let continuable = state.as_ref().is_some_and(|state| state.continuable);
        let go_on = open_until
            .clone()
            .filter(|_| crate::run::can_continue(cfg, &run, continuable, over))
            .map(|until| (run.run_id.clone(), until));
        PartFacts {
            harness,
            open_until,
            go_on,
            log_path: run_dir
                .is_dir()
                .then(|| run_dir.join(crate::host_log::HOST_LOG_NAME)),
        }
    }
}

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
    seal_entry(client, connectors, cfg, entry_id, out).map(|_| ())
}

/// [`seal_with`], answering how the seal was written (`None`: no local finish was reported).
fn seal_entry(
    client: &dyn DaemonClient,
    connectors: &Path,
    cfg: &Path,
    entry_id: &str,
    out: &mut dyn std::io::Write,
) -> Result<Option<LocalSealOutcome>, CliError> {
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
        // A standalone seal sends no counters.
        let finish = finish_locally(client, entry_id, None, deadline, allowance, None, out)?;
        // The entry is sealed: the run's reserve is released if nothing will use it again (a run
        // with no server and no host holding it; a run all of whose entries are sent).
        if let Ok(Some(run)) = crate::run::of_entry(cfg, entry_id) {
            crate::run::reserve::release_if_done(
                cfg,
                &crate::run::reserve::artifact_root(cfg),
                &run.run_id,
                false,
                crate::run::now_unix(),
            );
        }
        return Ok(Some(finish.seal));
    }

    // No capture status: the bare seal is the only one this command can send. The daemon
    // answers it `malformed` for a capture row it could not reopen, and `missing` for an id
    // it does not hold; it never answers it with a report this command could print, and never
    // with `busy` (the route reads only the registry row), so the generic retry is enough.
    match retry_busy(|| client.seal(entry_id)) {
        Ok(_) => Err(CliError::Control(ControlError::Malformed(format!(
            "goalcup's recorder answered the seal of part {entry_id} with a pre-v5 seal report, which \
             this build does not read"
        )))),
        Err(ControlError::Busy(said)) => Err(CliError::Refused(format!(
            "goalcup's recorder is still busy with part {entry_id} while retrying (up to {} attempts): {said}. \
                 The part is held, not broken, and no seal manifest was returned to this command. \
                 The daemon may still be completing a seal — wait a moment and \
                 run `goalcup seal {entry_id}` again.",
            BUSY_ATTEMPTS
        ))),
        // The daemon has the entry's row as a v5 capture but no status and no open capture: its
        // chain could not be reopened (a torn tail, an unreadable file), so there is nothing a
        // capture seal could finish. The CLI sent the only seal its status let it choose, and the
        // daemon's `malformed` is about that choice, not about a response this CLI cannot read.
        Err(ControlError::Malformed(said)) if status_unknown => {
            Err(torn_entry_refusal(cfg, entry_id, &said))
        }
        Err(other) => Err(CliError::Control(other)),
    }
}

/// Finish a local capture entry and say how it finished, where its trace is, and the one
/// command that checks it. `proxy` is the launcher's proxy report: `goalcup start` hands it
/// over when its harness exits, and this command has none.
///
/// The finish runs inside `deadline`, the one local-finish budget its caller started
/// (CONTRACTS.md, C6 local finish budget). Every attempt states what remains of it, and its transport timeout
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
    deadline: std::time::Instant,
    termination_allowance: std::time::Duration,
    reconnect: Option<&Path>,
    out: &mut dyn std::io::Write,
) -> Result<CaptureFinishReport, CliError> {
    let finish = seal_locally(
        client,
        entry_id,
        proxy,
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
    deadline: std::time::Instant,
    termination_allowance: std::time::Duration,
    reconnect: Option<&Path>,
) -> Result<CaptureFinishReport, CliError> {
    seal_locally_until(
        client,
        entry_id,
        proxy,
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
/// (and shared with the recording proxy's stop and the tees); each later attempt has a whole budget
/// of its own, and follows a daemon restarted since the one before.
#[allow(clippy::too_many_arguments)]
pub(crate) fn seal_locally_until(
    client: &dyn DaemonClient,
    entry_id: &str,
    proxy: Option<ProxyCounters>,
    first_deadline: std::time::Instant,
    until: WrapUp,
    termination_allowance: std::time::Duration,
    reconnect: Option<&Path>,
) -> Result<CaptureFinishReport, CliError> {
    let mut waited = std::time::Duration::ZERO;
    let mut asked = 0u32;
    let mut say = crate::host_log::say;
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
        LocalSealOutcome::SealWriteFailed => {
            // With its terminal chained, the tracker writes seal.json again on the next seal.
            let again = if finish.terminal_seq.is_some() {
                format!(" Once there is disk space, run `goalcup seal {entry_id}` again.")
            } else {
                String::new()
            };
            say(
                out,
                &format!(
                    "entry {entry_id}: finished locally, but seal.json could not be written \
                     ({terminal}), so the trace reads incomplete.{again}"
                ),
            )?
        }
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
fn torn_entry_refusal(cfg: &Path, entry_id: &str, daemon_said: &str) -> CliError {
    // What to do is what `goalcup doctor` says for the same entry (same words, same test of
    // whether `goalcup continue` can still restart its run). An entry of a run that is over,
    // sent, or not held here is history: then there is only the plain fact.
    let todo = match crate::run::reopen_failed_actions(
        cfg,
        &[entry_id.to_string()],
        crate::run::now_unix(),
    )
    .pop()
    {
        Some((entry_id, run_id)) => format!(
            "{} {}",
            crate::run::reopen_failed_sentence(&entry_id),
            crate::run::reopen_failed_next(&run_id)
        ),
        None => "This part can be neither sealed nor resumed, so start a new run for new work."
            .to_string(),
    };
    CliError::Refused(format!(
        "Part {entry_id} cannot be sealed: goalcup holds its record but could not reopen its \
         trace (it is torn or unreadable, for example a write cut short), so there is no open \
         capture to finish. Nothing was sealed, written or repaired: the trace bytes are preserved \
         exactly as the crash left them. Read what survives with `{VERIFIER_BIN} \
         --capture-integrity <the part's trace directory>` (goalcup keeps it under its \
         artifact root, in a folder named {entry_id}). {todo} (goalcup's recorder said: {daemon_said})"
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
            "Another finish of part {entry_id} is still in progress. Run `goalcup seal \
             {entry_id}` again to read its result."
        )),
        ControlError::FinishDeadline(stage) => CliError::Refused(format!(
            "Part {entry_id} did not finish within {} seconds (stopped before {stage}). It is \
             still open. Run `goalcup seal {entry_id}` to finish it; it will be marked \
             incomplete.",
            crate::finish_budget().as_secs()
        )),
        // A transport failure or a read timeout after the request was written: the daemon may
        // still commit this finish, and a later seal answers a committed one unchanged.
        ControlError::Unreachable(_) if admitted => CliError::Refused(format!(
            "goalcup's recorder did not answer the finish of part {entry_id} within {}s. It may still \
             complete it. Run `goalcup seal {entry_id}`: a finish it already committed is \
             answered unchanged.",
            waited.as_secs()
        )),
        error @ (ControlError::Unreachable(_) | ControlError::Unauthorized(_)) => {
            CliError::Refused(format!(
                "Part {entry_id} was not finished: goalcup's recorder refused this command's \
                 connection or credentials ({error}) before the finish began. Run `goalcup seal \
                 {entry_id}`."
            ))
        }
        ControlError::Busy(said) => CliError::Refused(format!(
            "goalcup's recorder is still busy with part {entry_id} while retrying (the {}-second \
             finish budget leaves no room for another attempt): {said}. The part is held, not \
             broken. Run `goalcup seal {entry_id}` again.",
            crate::finish_budget().as_secs()
        )),
        other => CliError::Control(other),
    }
}

/// What goalcup says of the harness's end, in words. The variant name leads (it is what the
/// record and the verifier call the outcome), and a stop that could not be confirmed says what
/// to check.
pub(crate) fn render_termination(outcome: &TerminationOutcomeV1) -> String {
    match outcome {
        TerminationOutcomeV1::AlreadyStopped { checked_at } => format!(
            "AlreadyStopped, the harness had already ended when goalcup checked at {}",
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
            "Unresolved, the harness was asked to stop at {} and goalcup gave up waiting at \
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

#[cfg(test)]
mod tests {
    #[test]
    fn seal_refuses_inside_the_harness_and_runs_without_the_marker() {
        let refused = super::refuse_inside_harness(true);
        assert!(
            matches!(&refused, Err(crate::CliError::Refused(said)) if said.contains("another terminal")),
            "{refused:?}"
        );
        assert!(super::refuse_inside_harness(false).is_ok());
    }

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

    /// A person's `goalcup seal` asks first, and only "yes" stops anything; `--yes` asks
    /// nothing. Once sealed it says, in plain words, that the run stays open and how to go on,
    /// and the technical lines go to the run's host.log, not the terminal.
    #[test]
    fn a_persons_seal_asks_first_unless_yes_and_says_how_to_go_on() {
        use crate::daemon::MockDaemonClient;
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("cfg");
        let mock = MockDaemonClient::armed_ok(tmp.path(), "fake");
        let part = mock.info().entry_id;
        crate::entries::write_entry(&cfg, &mock.info()).unwrap();
        let far = crate::run::now_unix() + 3_600;
        crate::run::tests_support::continuable_run(&cfg, "run-open", &part, far);
        let connectors = tmp.path().join("no-connectors");
        let seals =
            |mock: &MockDaemonClient| mock.calls().iter().filter(|c| *c == "seal_capture").count();

        // Anything but yes: nothing is stopped.
        for answer in ["no\n", "", "y\n"] {
            let mut out = Vec::new();
            let refused = super::seal_for_person(
                &mock,
                &connectors,
                &cfg,
                &part,
                false,
                &mut answer.as_bytes(),
                &mut out,
            )
            .unwrap_err();
            let asked = String::from_utf8(out).unwrap();
            assert!(asked.contains("This stops"), "{asked}");
            assert!(asked.contains("Your run stays open until"), "{asked}");
            assert!(asked.contains("Type yes to stop it."), "{asked}");
            assert!(
                refused.to_string().contains("Nothing was stopped"),
                "{refused}"
            );
            assert_eq!(seals(&mock), 0, "{answer:?}");
        }

        // `--yes`: no question, and nothing is read.
        let mut out = Vec::new();
        super::seal_for_person(
            &mock,
            &connectors,
            &cfg,
            &part,
            true,
            &mut "no\n".as_bytes(),
            &mut out,
        )
        .unwrap();
        assert_eq!(seals(&mock), 1);
        let said = String::from_utf8(out).unwrap();
        assert!(!said.contains("Type yes"), "{said}");
        assert!(
            said.trim_end().ends_with(
                "To go on: goalcup continue run-open. If your work is done, nothing else is \
                 needed."
            ),
            "{said}"
        );
        for word in ["anchor", "trace", "verify", "chain", "entry", "tracker"] {
            assert!(!said.contains(word), "{word}: {said}");
        }
        let log = std::fs::read_to_string(
            crate::run::run_dir(&cfg, "run-open").join(crate::host_log::HOST_LOG_NAME),
        )
        .unwrap();
        assert!(log.contains("--capture-integrity"), "{log}");

        // A typed yes seals too.
        super::seal_for_person(
            &mock,
            &connectors,
            &cfg,
            &part,
            false,
            &mut "yes\n".as_bytes(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(seals(&mock), 2);
    }

    /// `goalcup seal` on an entry the tracker could not reopen says what `goalcup doctor` says:
    /// the run to continue. For an entry of no run here it gives only the plain fact.
    #[test]
    fn a_torn_entry_of_a_live_run_names_the_run_to_continue() {
        let cfg = tempfile::tempdir().unwrap();
        let far = crate::run::now_unix() + 3_600;
        crate::run::tests_support::continuable_run(cfg.path(), "run-open", "entry-aaa", far);

        let said = super::torn_entry_refusal(cfg.path(), "entry-aaa", "malformed").to_string();
        assert!(said.contains("goalcup continue run-open"), "{said}");

        let said = super::torn_entry_refusal(cfg.path(), "entry-zzz", "malformed").to_string();
        assert!(!said.contains("goalcup continue"), "{said}");
    }
}
