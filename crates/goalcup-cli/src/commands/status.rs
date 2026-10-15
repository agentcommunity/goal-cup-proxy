//! `goalcup status` — one entry's signed status, checked against the key this machine
//! pinned when the run was created.
//!
//! # The key is the caller's, and it comes off the disk
//!
//! [`goalcup_protocol::control::SignedStatus::verify_with`] takes a run public key, and
//! there is deliberately no `verify(&self)` on that type: the status route is
//! unauthenticated, so a local process that bound the control port first could
//! serve a document signed with its own key, and a document checked against the key it
//! carries would report a forged deadline as trusted. The key this command checks with is
//! the one `goalcup start` wrote into `<config_dir>/entries/<id>.json`
//! ([`crate::entries`]) at the moment the daemon handed it over. That record is the whole
//! reason it survives: `start` is a process that exited, `status` is a fresh one, and a
//! command with no pinned key has nothing to verify against at all. An entry this machine
//! has no record of is refused by name (rung 1), never displayed on its own document's
//! word.
//!
//! # A document that does not check out is not rendered at all
//!
//! Two questions are asked before a single field is printed: does the signature verify
//! against the pinned run key, and is the payload about the entry that was asked for. A
//! `no` to either is a refusal (rung 1) that prints one headline naming the entry and what
//! failed, and nothing out of the payload — not the deadline, not the health sentence, not
//! the workspace path.
//!
//! A document that does not check out is not printed with a "did not verify" caveat either,
//! because an unverifiable document is not evidence of anything: the status route is
//! unauthenticated, so the bytes may be a rogue process's invention, and rendering them
//! would put a deadline and a health state on a participant's screen in the daemon's voice
//! while a sentence above says not to believe any of it. The evidence a participant acts on
//! is the sealed trace the verifier reads, which this command is not. A headline and rung 1
//! is the whole answer.
//!
//! The entry-id check is the second half of the same rule and is not covered by the
//! signature: one run key signs every document of that run, so a daemon answering `status
//! <a>` with `<b>`'s payload — a routing bug, or an id this CLI resolved and the daemon did
//! not — hands over a document that verifies perfectly and is about the wrong run.
//!
//! Under `--json` both refusals print a small error object and never the document: a caller
//! parsing stdout gets one shape for "here is a status" and another for "there is none",
//! rather than a document it would have to know not to trust.
//!
//! # What only this surface prints
//!
//! The bundled page carries `cycles` and `event_status_source` in its JSON
//! and shows neither, so `goalcup status` is the only human surface for the
//! daemon's own lost work — and for whether the round's dates were configured by an
//! organiser or are the entry's own timeline standing in for them
//! ([`EventStatusSource::Fallback`]). Staleness is
//! [`goalcup_protocol::control::STALE_AFTER_SECONDS`], the one threshold every reader of the
//! status document uses, and `now` is a parameter so the rendering is
//! deterministic.

use crate::CliError;
use crate::entries::{self, EntryRecord};
use crate::retry::retry_busy;
use goalcup_protocol::capture::M2Status;
use goalcup_protocol::control::{
    ArmingState, DaemonClient, EventStatusSource, STALE_AFTER_SECONDS, SignedStatus, StatusPayload,
    WorkspaceFinalizationStatusV1, parse_z,
};
use goalcup_protocol::events::HealthState;
use goalcup_protocol::provenance::TerminationOutcomeV1;
use std::io::Write;
use std::path::Path;

/// `goalcup status [<entry>] [--json]`, with everything it reads handed in.
///
/// The pinned run key is read and never passed: [`entries::read_entry`] when `entry` names
/// one, [`entries::latest_entry`] when it does not. Both answers are this machine's own
/// bookkeeping, and neither asks the daemon which entry it thinks is current — a document
/// checked against a key that came back with it is a document checked against itself.
pub fn status_with(
    client: &dyn DaemonClient,
    cfg: &Path,
    entry: Option<&str>,
    json: bool,
    now: &str,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let record = pinned(cfg, entry)?;
    // A reserve nothing will use again goes back to the disk, whatever the daemon answers below.
    crate::run::reserve::release_stale(
        cfg,
        &crate::run::reserve::artifact_root(cfg),
        crate::run::now_unix(),
    );
    // First of everything, before the daemon is even asked: what is wrong with the run. A
    // `--json` caller gets the document alone.
    if !json {
        put(out, &run_report(cfg, entry, &record, now))?;
    }
    let document = retry_busy(|| client.status(&record.entry_id)).map_err(CliError::Control)?;
    if document.verify_with(&record.run_public_key).is_err() {
        // Something other than the daemon that opened this run may be answering on an
        // unauthenticated port.
        return refuse(
            out,
            json,
            &record.entry_id,
            "signature did not verify".to_string(),
            format!(
                "the status document for entry {} did not verify against the run key \
                 this machine pinned when the entry was created, so nothing in it has \
                 been shown. The status route is unauthenticated by ruling: something \
                 other than the daemon that opened this run may be answering on the \
                 control port, and a deadline printed off that document would be its \
                 invention rather than this run's.",
                record.entry_id
            ),
        );
    }
    // Not covered by the check above: one run key signs every document of the run, so
    // another entry's payload verifies just as well as this one's.
    if document.payload.entry_id != record.entry_id {
        return refuse(
            out,
            json,
            &record.entry_id,
            format!(
                "the document that came back is for entry {}",
                document.payload.entry_id
            ),
            format!(
                "entry {} was asked for and the daemon answered with a document for entry \
                 {}, so nothing in it has been shown. It carries a good signature — the run \
                 key this machine pinned signs every document of this run — and it is still \
                 about a different run than the one the question was about.",
                record.entry_id, document.payload.entry_id
            ),
        );
    }
    if json {
        // The document as the daemon signed it, byte for byte, so a participant can hand it
        // to an organiser and it still verifies. It is reached only once both checks above
        // have passed, so a `--json` caller never has to decide whether to trust it.
        put(
            out,
            &format!("{}\n", serde_json::to_string_pretty(&document)?),
        )?;
    } else {
        put(out, &render_status(&document, now))?;
    }
    Ok(())
}

/// What this machine knows of the run the entry belongs to (its `state.json`): one `PROBLEM:`
/// line for each thing that is wrong, first of all, and nothing when nothing is. A run with no
/// state yet is not a problem, and is said in plain words; a machine with no run record for
/// the entry says nothing.
fn run_report(cfg: &Path, entry: Option<&str>, record: &EntryRecord, now: &str) -> String {
    let run = match entry {
        None => crate::run::newest(cfg),
        Some(_) => crate::run::of_entry(cfg, &record.entry_id),
    };
    let Ok(Some(run)) = run else {
        return String::new();
    };
    let Some(state) = crate::run::read_state(&crate::run::run_dir(cfg, &run.run_id)) else {
        return "run state: no run state yet\n".to_string();
    };
    let now = parse_z(now).unwrap_or_else(crate::run::now_unix);
    // `goalcup continue` is advised only for a run it can restart.
    let advice = crate::run::advice(cfg, &run, &state, now);
    let problems = state
        .problems(now, advice)
        .iter()
        .map(|problem| format!("PROBLEM: {}\n", plain(problem)))
        .collect::<String>();
    // A normal end of a run that cannot be continued is said plainly, after any problem.
    let finished = state
        .finished(advice)
        .map(|line| format!("{}\n", plain(&line)))
        .unwrap_or_default();
    problems + &finished
}

/// One headline on stdout, and the reason on rung 1.
///
/// The headline is deliberately short and carries nothing out of the payload: the point of
/// both refusals is that the payload is not a thing this command may repeat. Under `--json`
/// it is a small object with a stable `error` code, so a caller can branch on the shape
/// rather than on the absence of a document.
fn refuse(
    out: &mut dyn Write,
    json: bool,
    entry_id: &str,
    headline: String,
    said: String,
) -> Result<(), CliError> {
    if json {
        let document = serde_json::json!({
            "error": "status_not_trusted",
            "entry_id": entry_id,
            "message": headline,
        });
        put(
            out,
            &format!("{}\n", serde_json::to_string_pretty(&document)?),
        )?;
    } else {
        put(out, &format!("entry {entry_id}: {headline}\n"))?;
    }
    Err(CliError::Refused(said))
}

/// The record this machine holds for the entry that was named, or for the newest run it
/// started.
///
/// A configuration directory with no run in it at all is a refusal the participant can act
/// on rather than an empty render: there is no entry to ask about and no key to check an
/// answer with, and the fix is to start one.
///
/// `main` asks this first, before it starts a daemon: a run this machine has no record of is
/// an answer that needs no daemon, and a look on an unused machine must not create one.
pub fn pinned(cfg: &Path, entry: Option<&str>) -> Result<EntryRecord, CliError> {
    match entry {
        Some(entry_id) => entries::read_entry(cfg, entry_id),
        None => entries::latest_entry(cfg)?.ok_or_else(|| {
            CliError::Refused(format!(
                "no run has been started from this machine: there is no record under {}. \
                 Run `goalcup start --harness <id> --goal-file <file>`, or name an entry \
                 that was started here.",
                entries::entries_dir(cfg).display()
            ))
        }),
    }
}

/// One signed status, rendered for a terminal.
///
/// Reached only for a document that verified against the pinned run key and is about the
/// entry that was asked for: see the module header. There is no `verified` parameter and no
/// "this did not check out" line, because a document that did not check out never gets here.
/// `now` is a parameter for the same reason every other clock in this crate is — a render
/// that read the clock itself would be a different render tomorrow.
pub fn render_status(s: &SignedStatus, now: &str) -> String {
    let p = &s.payload;
    // Every string below that the daemon wrote goes through [`plain`] first. This is the one
    // place in this crate where bytes it did not author
    // reach a terminal, and a terminal reads control characters as instructions: an escape
    // sequence inside `detail` could move the cursor over the freshness line and repaint it.
    // The document is signed against the pinned run key by the time this is reached, so this
    // is the last inch of a chain that is otherwise sound — which is exactly why it is one
    // function and not a judgement call at every call site.
    let detail = plain(&p.detail);
    let finalization_needs_review = finalization_needs_harness_review(p.finalization.as_ref());
    let displayed_health = if finalization_needs_review {
        HealthState::SealingRecoveryNeeded
    } else {
        p.health
    };
    let workspace = plain(&p.workspace);
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("entry {} ({})", p.entry_id, p.connector_id));

    // First, before anything a reader might act on: whether anything has published a
    // document for this entry lately.
    lines.push(freshness(s, now));

    // A v5 local capture has no arming gate, upload or deadline: its capture block is the
    // whole story, and the legacy lines below would describe a run it is not.
    if let Some(capture) = &p.capture {
        lines.extend(capture_lines(p, capture, &detail, &workspace));
        return lines.join("\n") + "\n";
    }

    match p.arming {
        // The fourth arming state, and the reason it exists: "will never arm" must not read as
        // "still waiting to arm". The reason is the daemon's own words.
        ArmingState::Refused => lines.push(format!("  arming: refused — {detail}")),
        other => lines.push(format!("  arming: {}", wire_word(&other))),
    }
    // D25: the health state in the daemon's own plain words, which is the same sentence the
    // page shows. A second vocabulary here would be a second story about one daemon.
    lines.push(format!(
        "  health: {} — {detail}",
        wire_word(&displayed_health)
    ));
    lines.push(format!("  workspace: {workspace}"));
    if let Some(session) = &p.session_id {
        lines.push(format!("  session: {}", plain(session)));
    }
    if let Some(version) = &p.harness_version {
        // A daemon that has the field but not the number answers `Some("")` — a harness
        // whose version probe printed nothing — and that would render as `  harness: ` with
        // a dangling colon, which reads as a rendering fault rather than as an answer. The
        // word is the answer.
        let version = plain(version);
        let version = if version.trim().is_empty() {
            "unknown".to_string()
        } else {
            version
        };
        lines.push(format!("  harness: {version}"));
    }
    if let Some(started) = &p.started_at {
        lines.push(format!("  started: {}", plain(started)));
    }
    // A sealed run has no time remaining, it has an ending. The countdown line
    // is the one a participant acts on, and printing `time remaining: 167h 51m` under a run
    // whose chain closed an hour ago is the surface telling them to hurry over something
    // that is over.
    lines.push(match &p.sealed_at {
        Some(at) => format!("  sealed: {} — the chain is closed.", plain(at)),
        None => match (p.seconds_remaining, &p.deadline_at) {
            (Some(left), Some(at)) => format!("  time remaining: {} (deadline {at})", spell(left)),
            (Some(left), None) => format!("  time remaining: {}", spell(left)),
            (None, Some(at)) => format!("  deadline {at}, and the daemon reported no time left"),
            (None, None) => "  time remaining: this run has no deadline on it".to_string(),
        },
    });
    lines.push(format!(
        "  events: {} uploaded, {} pending",
        p.events_uploaded, p.events_pending
    ));
    if let Some(sync) = &p.last_sync_at {
        lines.push(format!("  last sync: {}", plain(sync)));
    }

    // The two facts the page carries in JSON alone, and the two beside them that nothing
    // else says either. Each is printed only when it has something to report: a run with
    // none of them is a run whose participant should not have to read four zeroes.
    if p.event_status_source == EventStatusSource::Fallback {
        lines.push(format!(
            "  round dates: not configured (fallback) — t0 {} and the close at {} are this \
             entry's own timeline standing in for an organiser's.",
            p.event.t0, p.event.publication_closes_at
        ));
    }
    let c = &p.cycles;
    // When the daemon's loops last turned,
    // beside what they have lost. Four zeroes under `cycle errors` is what a healthy daemon
    // looks like and reads like a daemon that has stopped; this line is the difference.
    // Printed only when the document carries the stamp — one written before it, or by a
    // daemon whose loops have not run yet, says nothing here rather than inventing a time.
    if let Some(at) = &c.last_pass_at {
        lines.push(format!("  cycles last ran: {}", plain(at)));
    }
    if c.tick + c.upload + c.status + c.deadline > 0 {
        lines.push(format!(
            "  cycle errors: tick {}, upload {}, status {}, deadline {}",
            c.tick, c.upload, c.status, c.deadline
        ));
    }
    if let Some(last) = &c.last {
        lines.push(format!("  last cycle error: {}", plain(last)));
    }
    if p.files_capped > 0 {
        lines.push(format!(
            "  files not captured: {} — a whole-file store went past the daemon's seen-set \
             bound, and every file past it is evidence that will not exist.",
            p.files_capped
        ));
    }
    if p.harness_unverified || finalization_needs_review {
        lines.push(
            "  the seal could not vouch for the harness: this daemon could not prove the \
             managed process tree was fully and safely stopped when the chain ended. \
             Review the recorded termination outcome."
                .to_string(),
        );
    }
    lines.join("\n") + "\n"
}

/// The capture block of a v5 status, in the wire's own words.
fn capture_lines(p: &StatusPayload, c: &M2Status, detail: &str, workspace: &str) -> Vec<String> {
    let storage = &c.storage;
    let durable = match &storage.durable {
        Some(d) => format!("durable through seq {} ({} bytes)", d.seq, d.chain_bytes),
        None => "nothing durable yet".to_string(),
    };
    let accounting = if storage.accounting_available {
        ""
    } else {
        "; byte accounting unavailable"
    };
    let mut lines = vec![
        format!(
            "  capture: {} — run {}, delivery {}, anchor {}, analysis {}",
            wire_word(&c.capture),
            wire_word(&c.run),
            wire_word(&c.delivery),
            wire_word(&c.anchor),
            wire_word(&c.analysis)
        ),
        format!("  {detail}"),
        format!("  workspace: {workspace}"),
        format!(
            "  storage: {} of {} bytes used, {} reserved, {durable}{accounting}",
            storage.used_bytes, storage.quota_bytes, storage.reserved_bytes
        ),
        format!(
            "  gaps: {}; assistance recorded: {}",
            c.gap_count,
            if c.assistance_observed { "yes" } else { "no" }
        ),
    ];
    if let (Some(consumed), Some(refused)) = (p.producers_consumed, p.admissions_refused) {
        lines.push(format!(
            "  ingest: {consumed} producer(s) consumed, {refused} refused admission"
        ));
    }
    lines
}

fn finalization_needs_harness_review(finalization: Option<&WorkspaceFinalizationStatusV1>) -> bool {
    match finalization {
        None | Some(WorkspaceFinalizationStatusV1::Finishing {}) => false,
        Some(WorkspaceFinalizationStatusV1::Incomplete { termination, .. }) => termination
            .as_ref()
            .is_some_and(|outcome| !matches!(outcome, TerminationOutcomeV1::Confirmed { .. })),
        Some(WorkspaceFinalizationStatusV1::Terminal { termination, .. }) => {
            !matches!(termination, TerminationOutcomeV1::Confirmed { .. })
        }
    }
}

/// How old this document is, in the words a reader acts on.
///
/// The threshold is [`STALE_AFTER_SECONDS`], read from the protocol crate, so every reader
/// of the status document calls the same document stale. A `cached_at`
/// that does not parse is "unknown", never "fresh" — the one answer that would be a guess.
///
/// The two timestamps are subtracted here rather than through
/// [`goalcup_protocol::control::SignedStatus::stale_seconds`], which saturates: a document
/// stamped ahead of this machine's clock would come back from it as an age of zero, which reads
/// as "published just now" and is the one thing it is not. A stamp in the future is a clock
/// that disagrees with the daemon's, and it is said out loud — that is a fact a participant
/// can act on, and a run whose deadline is enforced by one of the two clocks is a run where
/// they should.
fn freshness(s: &SignedStatus, now: &str) -> String {
    let (Some(cached), Some(asked)) = (parse_z(&s.cached_at), parse_z(now)) else {
        return format!(
            "  published at {}, which did not read as a time — how old this document is is \
             unknown rather than fresh.",
            plain(&s.cached_at)
        );
    };
    if cached > asked {
        return format!(
            "  published in the future by {}s (clock skew) ({}), so how old this document \
             is cannot be said: this machine's clock and the one that stamped it disagree.",
            cached - asked,
            plain(&s.cached_at)
        );
    }
    let age = asked - cached;
    if age > STALE_AFTER_SECONDS {
        return format!(
            "  offline: nothing has published a status for this entry in {age}s, and a \
             running daemon publishes one at least every {STALE_AFTER_SECONDS}s. What \
             follows is the last document there was ({}).",
            plain(&s.cached_at)
        );
    }
    format!("  published {age}s ago ({})", plain(&s.cached_at))
}

/// One daemon-supplied string, with anything a terminal would obey taken out of it.
///
/// Every ASCII control character and every C1 control is replaced by U+FFFD, including tab
/// and newline: this renderer builds one line per fact, and a `detail` carrying a newline
/// would draw a second line the reader cannot tell from one this command wrote. Replaced
/// rather than dropped, so a string that carried one says on the screen that something was
/// there — a silent deletion is a string this CLI rewrote without saying so.
///
/// Everything above U+009F is left alone. A workspace path may legitimately hold any
/// character a filesystem accepts, and a renderer that stripped non-ASCII would print a
/// participant's own directory back at them wrong.
pub(crate) fn plain(said: &str) -> String {
    said.chars()
        .map(|c| match c.is_control() {
            true => char::REPLACEMENT_CHARACTER,
            false => c,
        })
        .collect()
}

/// A wire enum in the one word the wire spells it with.
///
/// `serde` and not a match, so this file cannot grow a second name for a state the JSON
/// already has one for: a reader comparing the terminal against `--json` sees the same word
/// in both, and a variant added later needs no edit here. Anything that is not a plain
/// string — which no `#[serde(rename_all = "snake_case")]` unit enum produces — reads as
/// `unknown` rather than panicking.
pub(crate) fn wire_word<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(word)) => word,
        _ => "unknown".to_string(),
    }
}

/// Seconds as hours, minutes and seconds. Nothing here is a threshold; it is arithmetic.
fn spell(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    format!("{h}h {m:02}m {s:02}s")
}

/// One write, and the error naming the stream it could not be made on.
///
/// Shared with [`crate::commands::entries`]: two surfaces over one document, and one
/// sentence for a terminal that closed under them.
pub(crate) fn put(out: &mut dyn Write, text: &str) -> Result<(), CliError> {
    out.write_all(text.as_bytes())
        .map_err(|source| CliError::Io {
            path: "standard output".to_string(),
            source,
        })
}
