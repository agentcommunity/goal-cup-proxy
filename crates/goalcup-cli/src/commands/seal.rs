//! `goalcup seal` — end the run early, report what the tracker recorded, and say where the
//! trace is.
//!
//! # The three things this command does not do
//!
//! **It does not seal.** The daemon does: `POST /v0/entries/{id}/seal` finishes the entry and
//! answers with the workspace submission outcome and the harness termination, or with a
//! legacy sealed manifest. This command asks for that and reports it.
//!
//! **It does not signal anything.** D22: the daemon terminates the registered process tree
//! per `kill.strategy`, because a seal can come when the participant's terminal is long gone
//! (the V1 deadline seal did, until D48 removed it) and a kill that only worked from the
//! terminal would be a kill that only worked when somebody was watching. There is exactly
//! one terminator, and it is not here.
//!
//! **It does not write the trace.** `<artifact_root>/trace` belongs to the daemon's export,
//! start to finish. This command's half of exporting is [`crate::seal::trace_dir`] and the
//! printed `goalcup-verify` line — which is also why there is no `goalcup export`.
//!
//! # What it does own: the self-check
//!
//! [`crate::seal::scan_workspace`] takes the same walk with the same exclusions from this
//! side, and [`crate::seal::compare`] prints how the two documents differ. The self-check
//! runs for `--dry-run`, for a legacy sealed manifest and for an entry the daemon had already
//! sealed; a workspace submission is reported as the daemon gave it, and this side does not
//! walk the live workspace for it. The differences are information and never a refusal: the
//! tracker's manifest is the record either way, and a participant who can see what their own
//! machine thinks is in the workspace can tell a file written between the two walks from a
//! file that was never captured.
//!
//! # It reads the entry cache first, and `--dry-run` reads it too
//!
//! `goalcup seal` runs in a process that never saw a `WorkspaceInfo`. The workspace to scan
//! and the tracker's own file list — one half of the exclusion set — reach it only through
//! the record `goalcup start` wrote ([`crate::entries::read_entry`]), so an entry this
//! machine has no record of is rung 1 naming the id and nothing is asked of the daemon. A
//! `--dry-run` reads the same record for a sharper reason: a dry run that guessed the
//! workspace would print a difference report about the wrong directory, which is worse than
//! refusing.
//!
//! # A dry run builds no client, because it needs none
//!
//! [`dry_run_with`] is a separate entry point, and `main.rs` reaches it before it resolves a
//! `control.json`. A dry run makes no call: it reads this machine's record, walks this
//! machine's workspace and prints what it found, so `goalcup seal --dry-run` answers on a
//! machine with no daemon running.
//!
//! # `busy` and `conflict` share a `409` and are opposite answers
//!
//! Which is why this command branches on the [`ControlError`] and never on a status. `busy`
//! is the daemon holding the entry for the length of one asynchronous call — ridden out by
//! [`retry_busy_seal`], and reported at rung 1 if it is still busy when that is spent.
//! `conflict` is "already sealed", which is the **promised** behaviour and not a failure: the
//! daemon seals at `deadline_at` on its own, so a participant who runs this after the clock
//! ran out meets it. That prints what happened, still shows the self-check and the trace, and
//! exits 0.
//!
//! The conflict branch also **reads the manifest that seal produced**, out of
//! `<artifact_root>/trace/seal.json`, and runs the same comparison the legacy branch runs:
//! the daemon sealed at the deadline, minutes or hours ago, and everything the agent wrote
//! since is a difference the participant has a right to see. The file is opened read-only and
//! this command writes nothing in that directory; a `seal.json` that is missing or does not
//! parse is said out loud and is not a refusal, because the entry is sealed either way and
//! the exported trace is the daemon's to explain.

use crate::CliError;
use crate::entries::EntryRecord;
use crate::entries::read_entry;
use crate::retry::{SEAL_BUSY_ATTEMPTS, retry_busy, retry_busy_seal, retry_capture_seal};
use crate::seal::{
    Differences, compare_parts, exclusion_set, scan_workspace, scanned_at_now, trace_dir,
};
use goalcup_protocol::control::{
    CaptureFinishReport, CaptureSealError, CaptureSealRequest, ControlError, DaemonClient,
    LOCAL_CAPTURE_FINISH_BUDGET, LocalSealOutcome, ProxyCounters, SealResponse,
};
use goalcup_protocol::export::SEAL_FILE;
use goalcup_protocol::process::termination_bound;
use goalcup_protocol::provenance::{TerminationFailureV1, TerminationOutcomeV1};
use goalcup_protocol::seal::SealManifest;
use goalcup_protocol::submission::SubmissionOutcomeV1;
use std::path::Path;

/// The verifier, which is a separate binary and not a sub-command of this one.
///
/// A constant because it is printed to a participant as a command to run, and a binary
/// spelled twice is a binary that is one day spelled wrong in the place somebody copies from.
pub const VERIFIER_BIN: &str = "goalcup-verify";

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
    dry_run: bool,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    if dry_run {
        return dry_run_with(connectors, cfg, entry_id, out);
    }
    // The cached record establishes which entry this machine means, but a workspace
    // submission finish does not walk its live workspace. The daemon's captured submission
    // is already immutable and bound to the signed cutoff; scanning here would admit later
    // bytes into the report.
    let record = read_entry(cfg, entry_id)?;
    let status = retry_busy(|| client.status(entry_id));
    // An entry this machine recorded and the daemon has no status for (H-15): the one case the
    // legacy seal below can be answered `malformed` by a daemon that holds the row as a capture.
    let status_unknown = matches!(status, Err(ControlError::NotFound(_)));

    // 3a. A local capture entry finishes locally. The launcher that ran its proxy is gone
    //     or still waiting on the harness, so this seal carries no proxy counters; the daemon
    //     ends the harness it owns. The one local-finish budget starts here, after the status
    //     read, and the harness's termination keeps its own bound outside it: the allowance is
    //     the connector's own grace (from its cached manifest), or the slow-call timeout when
    //     that manifest cannot be read.
    if status
        .as_ref()
        .is_ok_and(|status| status.payload.capture.is_some())
    {
        let deadline = std::time::Instant::now() + LOCAL_CAPTURE_FINISH_BUDGET;
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

    // 3b. A run that never started still seals, and the seal is a Fail. The participant is
    //     owed the sentence saying what it will be worth before the seal, while stopping is
    //     still a choice they have. Asked of the daemon and not of the local record, because
    //     binding a session is the daemon's event and nothing on this machine's disk knows
    //     about it. A status this command could not read is not a reason to refuse a seal:
    //     the warning is dropped and the seal goes ahead.
    if let Ok(status) = status
        && status.payload.session_id.is_none()
    {
        say(
            out,
            &format!(
                "entry {entry_id} has no session: the tracker never saw a harness start in \
                 this workspace, so there is nothing captured to judge. Sealing is still the \
                 right way to end it, and the verdict will be a Fail — `no_session` — rather \
                 than a trace anybody has to read."
            ),
        )?;
    }

    // 4. The one call, and the only thing this command puts on the wire.
    let trace = match retry_busy_seal(|| client.seal(entry_id)) {
        Ok(SealResponse::Workspace(finish)) => {
            if finish.entry_id != entry_id {
                return Err(CliError::Control(ControlError::Malformed(format!(
                    "seal response belongs to entry {}, not requested entry {entry_id}",
                    finish.entry_id
                ))));
            }
            match &finish.submission {
                SubmissionOutcomeV1::Captured(captured) => say(
                    out,
                    &format!(
                        "entry {entry_id}: workspace submission captured (manifest {}).",
                        captured.manifest_sha256.as_str()
                    ),
                )?,
                SubmissionOutcomeV1::Incomplete { reason } => say(
                    out,
                    &format!(
                        "entry {entry_id}: workspace submission incomplete ({}).",
                        wire_words(reason)
                    ),
                )?,
            }
            say(
                out,
                &format!(
                    "harness termination: {}.",
                    render_termination(&finish.termination)
                ),
            )?;
            std::path::PathBuf::from(finish.trace_dir)
        }
        Ok(SealResponse::Legacy(sealed)) => {
            let local = scan_record(connectors, &record)?;
            say(
                out,
                &format!(
                    "entry {entry_id}: sealed at {}. The tracker's manifest is the record: \
                     {}, {} applied.",
                    sealed.sealed_at,
                    count(sealed.files.len(), "file"),
                    count(sealed.excluded.len(), "exclusion")
                ),
            )?;
            say(out, &difference_report(&local, &sealed))?;
            trace_dir(Path::new(&record.artifact_root))
        }
        Err(ControlError::Conflict(said)) => {
            let local = scan_record(connectors, &record)?;
            let trace = trace_dir(Path::new(&record.artifact_root));
            say(
                out,
                &format!(
                    "entry {entry_id} was already sealed at its deadline, or by an earlier \
                     `goalcup seal`: {said}. That seal is the record and nothing was lost by \
                     it — the tracker seals at the deadline on its own, which is the promised \
                     behaviour and not a failure of this run."
                ),
            )?;
            // The manifest that seal produced is on disk, in the trace the daemon exported.
            // Read it and hold this scan up against it, the same comparison the legacy branch
            // makes and the case it matters most in, because that seal may be hours old.
            // Read-only: this command opens the file and writes nothing beside it.
            match exported_seal(&trace, entry_id) {
                Ok(sealed) => say(out, &difference_report(&local, &sealed))?,
                Err(why) => {
                    say(out, &why)?;
                    say(out, &scan_report(&local))?;
                }
            }
            trace
        }
        Err(ControlError::Busy(said)) => {
            return Err(CliError::Refused(format!(
                "the tracker is still busy with entry {entry_id} while retrying (up to {} attempts): {said}. \
                 The entry is held, not broken, and no seal manifest was returned to this command. \
                 The daemon may still be completing a seal — wait a moment and \
                 run `goalcup seal {entry_id}` again.",
                SEAL_BUSY_ATTEMPTS
            )));
        }
        // The daemon has the entry's row as a v5 capture but no status and no open capture: its
        // chain could not be reopened (a torn tail, an unreadable file), so there is nothing a
        // capture seal could finish. The CLI sent the only seal its status let it choose, and the
        // daemon's `malformed` is about that choice, not about a response this CLI cannot read.
        Err(ControlError::Malformed(said)) if status_unknown => {
            return Err(torn_entry_refusal(entry_id, &said));
        }
        Err(other) => return Err(CliError::Control(other)),
    };

    // 5. Where the tracker exported this entry, and the one command that checks it. This CLI
    //    reads the path and prints it; it writes not one byte inside it.
    say(out, &format!("trace:  {}", trace.display()))?;
    say(
        out,
        &format!("verify: {}", verify_command(&trace, record.dev)),
    )?;
    Ok(())
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
/// Nothing is printed here.
pub(crate) fn seal_locally(
    client: &dyn DaemonClient,
    entry_id: &str,
    proxy: Option<ProxyCounters>,
    native_scope: Option<String>,
    deadline: std::time::Instant,
    termination_allowance: std::time::Duration,
    reconnect: Option<&Path>,
) -> Result<CaptureFinishReport, CliError> {
    let mut waited = std::time::Duration::ZERO;
    let mut fresh = None;
    let finish: CaptureFinishReport = retry_capture_seal(deadline, |remaining| {
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
        // A daemon restarted since the caller's client was built is at another port or token:
        // `reconnect` is the configuration directory to read that from (H-14).
        crate::daemon::seal_capture_following_restart(
            client, &mut fresh, reconnect, entry_id, &request, waited,
        )
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
    say(
        out,
        "anchor: local only. No receipt, acknowledgement or upload exists for this run.",
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
            LOCAL_CAPTURE_FINISH_BUDGET.as_secs()
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
            LOCAL_CAPTURE_FINISH_BUDGET.as_secs()
        )),
        other => CliError::Control(other),
    }
}

/// `goalcup seal --dry-run`: this machine's record, this machine's walk, and no client.
///
/// Takes no [`DaemonClient`] — not an unused one, none — which is the whole point: nothing
/// below this line can call a daemon, so `main.rs` has nothing to resolve before it gets
/// here and a machine with no `control.json` can still answer "what is in my workspace".
pub fn dry_run_with(
    connectors: &Path,
    cfg: &Path,
    entry_id: &str,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let (_record, local) = record_and_scan(connectors, cfg, entry_id)?;
    say(
        out,
        &format!("entry {entry_id}: a dry run. Nothing was sealed and the tracker was not called."),
    )?;
    say(out, &scan_report(&local))?;
    say(
        out,
        &format!(
            "Run `goalcup seal {entry_id}` to record the tracker's own manifest and end the \
             run."
        ),
    )?;
    Ok(())
}

/// The two things both entry points start from: the record this machine pinned at creation,
/// and this machine's own walk of the workspace it names.
///
/// The exclusion set is the connector's patterns plus the tracker's own workspace files, as
/// the daemon listed them. The record carries that list unchanged; sorting it is the
/// normalisation of the daemon's own answer, not a second derivation of it. The walk is
/// stamped with this machine's clock and never the daemon's.
fn record_and_scan(
    connectors: &Path,
    cfg: &Path,
    entry_id: &str,
) -> Result<(EntryRecord, SealManifest), CliError> {
    let record = read_entry(cfg, entry_id)?;
    let local = scan_record(connectors, &record)?;
    Ok((record, local))
}

/// The self-check scan, reached from a dry run, a legacy sealed manifest and an already-sealed
/// entry.
fn scan_record(connectors: &Path, record: &EntryRecord) -> Result<SealManifest, CliError> {
    let c = crate::connector::load(connectors, &record.connector_id)?;
    let mut tracker_files = record.workspace_relative.clone();
    tracker_files.sort();
    let exclusions = exclusion_set(&c.manifest, &tracker_files);
    let local = scan_workspace(
        Path::new(&record.workspace),
        &exclusions,
        &record.entry_id,
        &scanned_at_now(),
    )?;
    Ok(local)
}

/// The [`SealManifest`] the daemon wrote into the exported trace **for `entry_id`**, or a
/// sentence saying why there is none to read.
///
/// `Err` here is a line to print and not a [`CliError`]: the entry is sealed whatever this
/// file says, and a command that refused because a document it only wanted for a comparison
/// was unreadable would be turning the tracker's own export problem into the participant's
/// failed seal. Opened read-only; nothing in this crate writes under that directory.
///
/// # It refuses a manifest belonging to another entry
///
/// The path this reads is built by pure arithmetic — [`crate::seal::trace_dir`] over the
/// `artifact_root` the entry cache holds — and the cache is a file on the participant's own
/// disk. An artifact root copied from another run, an entry record edited by hand, a
/// `{artifact_root}` two entries were pointed at: any of those puts one entry's `seal.json`
/// under another entry's path, and the difference report would then present a stranger's
/// manifest as "the record" this scan disagrees with.
///
/// So the id is compared, and a mismatch is a **named refusal** rather than a silent
/// comparison: it goes down the same path an unreadable file takes — said out loud, the local
/// scan printed on its own, exit 0 — because the entry is sealed either way and the exported
/// trace is the daemon's to explain.
fn exported_seal(trace: &Path, entry_id: &str) -> Result<SealManifest, String> {
    let path = trace.join(SEAL_FILE);
    let unusable = |why: String| {
        format!("{why} The scan below is this machine's alone, with nothing to hold it up against.")
    };
    let text = read_capped(&path).map_err(unusable)?;
    let sealed: SealManifest = serde_json::from_str(&text).map_err(|e| {
        unusable(format!(
            "that seal's own manifest at {} could not be read as one: {e}.",
            path.display()
        ))
    })?;
    if sealed.entry_id != entry_id {
        return Err(unusable(format!(
            "the manifest at {} is entry {}'s, not entry {entry_id}'s, so it is not this \
             entry's record and nothing here compares the two.",
            path.display(),
            sealed.entry_id
        )));
    }
    Ok(sealed)
}

/// The exact invocation, quoted the way [`crate::launch::shell_line`] quotes an argument, so
/// a participant whose artifact root has a space in it can paste the line as it stands.
///
/// `--dev` when, and only when, the run was started with `goalcup start --dev`. A development
/// run is one whose connector manifest this build does not pin, and `goalcup-verify` fails
/// such a trace outright without the flag. The flag is not added on any other ground: a
/// judged entry's verification must never carry it, and the record says which run this was.
pub fn verify_command(trace: &Path, dev: bool) -> String {
    let flag = if dev { " --dev" } else { "" };
    format!(
        "{VERIFIER_BIN}{flag} {}",
        crate::launch::quote(&trace.display().to_string())
    )
}

/// What this machine's own walk found, with the listing bounded and the count exact.
///
/// Printed where there is no daemon manifest to hold it up against: a dry run, and an entry
/// the daemon had already sealed.
fn scan_report(local: &SealManifest) -> String {
    let mut out = format!(
        "this machine's scan of {} at {}: {}, {} applied.",
        local.workspace,
        local.sealed_at,
        count(local.files.len(), "file"),
        count(local.excluded.len(), "exclusion")
    );
    let named: Vec<String> = local
        .files
        .iter()
        .map(|f| format!("{}  ({} bytes)", f.path, f.bytes))
        .collect();
    listing(&mut out, &named);
    out
}

/// The self-check, laid beside the record: how the two walks disagree, and what that means.
///
/// A difference is a statement about two walks taken at two moments, not a verdict. The
/// closing sentence says so out loud, because a participant reading a list of differences
/// under the word "seal" will otherwise read it as damage.
fn difference_report(local: &SealManifest, sealed: &SealManifest) -> String {
    let differences: Differences = compare_parts(local, sealed);
    if differences.is_empty() {
        return format!(
            "this machine's own scan, taken at {}, agrees with it in every file.",
            local.sealed_at
        );
    }
    let mut out = format!(
        "this machine's own scan, taken at {}, disagrees with it in {}:",
        local.sealed_at,
        count(differences.len(), "place")
    );
    // The workspace line is at most one and explains every line under it, so it is never
    // the thing a bound drops. The other two kinds are bounded separately: one edited
    // connector manifest can produce more exclusion lines than the whole listing holds, and
    // a single bound over the concatenation would spend it on them and print not one file.
    if let Some(line) = &differences.workspace {
        out.push_str(&format!("\n  {line}"));
    }
    listing(&mut out, &differences.exclusions);
    listing(&mut out, &differences.files);
    out.push_str(
        "\nA difference is information and not a refusal: the two walks were taken at two \
         moments, and the tracker's manifest is what this entry is judged on.",
    );
    out
}

/// A document this process did not write, read with a bound on it.
///
/// The cap is [`crate::config::CONTROL_FILE_MAX_BYTES`], taken from there rather than named
/// again so the two cannot drift, and the reason is that file's: **the size of a document
/// this process did not write is not this process's to trust**. `<trace>/seal.json` sits
/// under an `artifact_root` the entry cache names (the same cache whose `status_url` the V1
/// `goalcup ui` checked before a browser was handed it), so an edited record, or two entries
/// pointed at one artifact root, decides which file this reads.
///
/// One byte past the cap, so a file exactly at it still reads and anything larger is caught
/// having read the cap and no more — the shape `config::read_control_file` uses.
///
/// `Err` is the sentence for the participant, because every caller wraps it in the same
/// "the scan below is this machine's alone" line.
fn read_capped(path: &Path) -> Result<String, String> {
    use std::io::Read;

    let cap = crate::config::CONTROL_FILE_MAX_BYTES;
    let file = std::fs::File::open(path).map_err(|e| {
        format!(
            "that seal's own manifest could not be read at {}: {e}.",
            path.display()
        )
    })?;
    let mut text = String::new();
    let read = file.take(cap + 1).read_to_string(&mut text).map_err(|e| {
        format!(
            "that seal's own manifest could not be read at {}: {e}.",
            path.display()
        )
    })?;
    if read as u64 > cap {
        return Err(format!(
            "that seal's own manifest at {} is larger than {cap} bytes, which is more than \
             any manifest a daemon exports.",
            path.display()
        ));
    }
    Ok(text)
}

/// At most [`crate::seal::MAX_LISTED`] of `lines`, then how many were not shown.
fn listing(out: &mut String, lines: &[String]) {
    for line in lines.iter().take(crate::seal::MAX_LISTED) {
        out.push_str(&format!("\n  {line}"));
    }
    if lines.len() > crate::seal::MAX_LISTED {
        out.push_str(&format!(
            "\n  … and {} more.",
            lines.len() - crate::seal::MAX_LISTED
        ));
    }
}

/// `n` of `noun`, pluralised the one way English does it for these three nouns.
///
/// "1 files" is the kind of line a participant reads as a machine talking rather than as
/// something anybody checked, and every count this command prints can be one.
fn count(n: usize, noun: &str) -> String {
    match n {
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
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
