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
//! # What it prints
//!
//! First the participant block ([`crate::participant`] has its words): the run (practice or
//! competition) with the harness's display name and its state, when it ends and how long is
//! left, whether the agent runs, how the upload stands, and one `PROBLEM:` line for each thing
//! wrong, every one ending with its fix. The block ends with the one thing to do next. Then, under
//! `Details:`, the technical status of the entry: every document the daemon signs is a v5 local
//! capture's, so that is the capture block with its states, the daemon's sentence, storage, gaps
//! and admissions. Staleness is [`goalcup_protocol::control::STALE_AFTER_SECONDS`], the one
//! threshold every reader of the status document uses, and `now` is a parameter so the rendering
//! is deterministic. `--json` prints the signed document, with this machine's own facts
//! beside it under `local`, outside what is signed.

use crate::CliError;
use crate::entries::{self, EntryRecord};
use crate::participant;
use crate::retry::retry_busy;
use goalcup_protocol::capture::M2Status;
use goalcup_protocol::control::{
    DaemonClient, STALE_AFTER_SECONDS, SignedStatus, StatusPayload, parse_z,
};
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
    status_in(client, cfg, None, entry, json, now, out)
}

/// [`status_with`] with the directory the connectors are in, so the participant block names the
/// harness by its display name. Without it, or when the connector cannot be loaded, the
/// connector id is named instead.
pub fn status_in(
    client: &dyn DaemonClient,
    cfg: &Path,
    connectors: Option<&Path>,
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
    // What is sealed and sent need not stay on this machine: its acknowledged parts go.
    crate::upload::prune::clean_finished_runs(cfg);
    // First of everything, before the daemon is even asked: where the run is and what to do
    // about anything wrong with it. A `--json` caller gets the document.
    let mut run_id = None;
    if !json {
        let (report, run) = run_report(cfg, connectors, &record, now);
        put(out, &report)?;
        run_id = run;
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
        // The document as the daemon signed it, field for field, so a participant can hand it
        // to an organiser and it still verifies. It is reached only once both checks above
        // have passed, so a `--json` caller never has to decide whether to trust it.
        // Beside it, outside what is signed, `local`: this machine's own facts.
        let shown = WithLocal {
            document: &document,
            local: local_facts(cfg, &record),
        };
        put(out, &format!("{}\n", serde_json::to_string_pretty(&shown)?))?;
    } else {
        // The technical block follows the participant block, under its own heading.
        if let Some(run_id) = &run_id {
            put(
                out,
                &format!("\n{}\nrun {run_id}\n", participant::STATUS_DETAILS),
            )?;
        }
        put(out, &render_status(&document, now))?;
    }
    Ok(())
}

/// The key of `goalcup status --json` beside the signed document that holds this machine's own
/// facts ([`local_vitals`]). It is not signed: it comes from the run's files, not the daemon.
pub const LOCAL_KEY: &str = "local";

/// The signed document, its fields as they are, and [`LOCAL_KEY`] after them.
#[derive(serde::Serialize)]
struct WithLocal<'a> {
    #[serde(flatten)]
    document: &'a SignedStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    local: Option<crate::upload::api::HeartbeatVitals>,
}

/// [`local_vitals`] for the run of `record`; `None` when this machine has no state for it.
fn local_facts(cfg: &Path, record: &EntryRecord) -> Option<crate::upload::api::HeartbeatVitals> {
    let run = crate::run::of_entry(cfg, &record.entry_id).ok().flatten()?;
    let run_dir = crate::run::run_dir(cfg, &run.run_id);
    let state = crate::run::read_state(&run_dir)?;
    Some(local_vitals(&run_dir, &state))
}

/// The run's vitals in the heartbeats' shape: the free disk as the watch tick last measured it,
/// whether the agent is paused for it, and what the tracer's own processes used at the last
/// heartbeat (none for a run with no server).
fn local_vitals(
    run_dir: &Path,
    state: &crate::run::RunState,
) -> crate::upload::api::HeartbeatVitals {
    let mut vitals = crate::tracer_vitals::last(run_dir).unwrap_or_default();
    if state.disk != crate::run::DiskState::default() {
        vitals.disk_free_bytes = Some(state.disk.free_bytes);
    }
    vitals.agent_paused =
        crate::tracer_vitals::paused_for_disk(run_dir).then(|| "disk".to_string());
    vitals
}

/// The first line of the run section of an organizer test run (`goalcup start --test`).
const TEST_RUN_BANNER: &str = "TEST RUN: an organizer test run, not a competition entry\n";

/// What this machine knows of the run the entry belongs to (its `state.json`), in the
/// participant's words: where the run is, one `PROBLEM:` line for each thing that is wrong with
/// the fix after it, and the one thing to do next. A run with no state yet is not a problem, and
/// is said in plain words; a machine with no run record for the entry says nothing. The second
/// value is the run's id, when the machine has the run.
fn run_report(
    cfg: &Path,
    connectors: Option<&Path>,
    record: &EntryRecord,
    now: &str,
) -> (String, Option<String>) {
    // The run of the entry that was pinned, never the newest run: the newest entry and the
    // newest run can belong to different runs (a practice run continued after a competition
    // run was started).
    let Ok(Some(run)) = crate::run::of_entry(cfg, &record.entry_id) else {
        return (String::new(), None);
    };
    // An organizer test run says so first, before anything else in the run section.
    let banner = if run.test { TEST_RUN_BANNER } else { "" };
    let name = harness_name(connectors, &run.connector_id, record.dev);
    let now = parse_z(now).unwrap_or_else(crate::run::now_unix);
    let state = crate::run::read_state(&crate::run::run_dir(cfg, &run.run_id));
    let over = crate::run::is_over(&run, state.as_ref(), now);
    let left = participant::seconds_left(
        state.as_ref().and_then(|state| state.time_left.as_ref()),
        run.deadline_at.as_deref(),
        now,
    );
    // Over by a stop (a newer run, the server's close) is not over by the deadline.
    let early = state.as_ref().is_some_and(|state| {
        state.stopped_for_its_end()
            && !crate::run::time_left::time_up(
                state.time_left.as_ref(),
                run.deadline_at.as_deref(),
                now,
            )
    });
    let end = match (over, early) {
        (false, _) => participant::End::Open,
        (true, false) => participant::End::Over,
        (true, true) => participant::End::Early,
    };
    let ends = run
        .deadline_at
        .as_deref()
        .map(|deadline| participant::ends_line(deadline, left, end));
    let on_server = run.server_url.is_some();
    let mode = on_server.then(|| run.mode());
    let Some(state) = state else {
        let young = now.saturating_sub(run.started_at_unix) < NEW_RUN_SECS;
        let header = participant::status_header(&participant::StatusHeader {
            harness: &name,
            mode,
            phase: None,
            ends,
            agent: participant::Agent::Running,
            upload: participant::UploadNow::NotYet,
            problems: 0,
            stale: false,
            early: false,
            young,
        });
        // A run that is a few minutes old is still starting; an older one with no state did not
        // start (a start that failed before the launch leaves a record and no state).
        let next = if young {
            participant::Next::Wait
        } else {
            participant::Next::Restart {
                connector_id: &run.connector_id,
                mode,
            }
        };
        let text = format!(
            "{banner}{}\n{}\n",
            header.join("\n"),
            participant::status_next(&next)
        );
        return (plain_lines(&text), Some(run.run_id));
    };

    // `goalcup continue` is advised only for a run it can restart.
    let advice = crate::run::advice(cfg, &run, &state, now);
    let until = run
        .deadline_at
        .as_deref()
        .map(|deadline| participant::end_words(deadline, None));
    let who = participant::Who {
        harness: &name,
        run_id: &run.run_id,
        until: until.as_deref(),
    };
    let problems = state.problems(now, advice, &who);
    // A normal end of a run that cannot be continued is said plainly, after any problem.
    let finished = state.finished(advice, &who);

    let spool = cfg.join(crate::upload::UPLOADS_DIR_NAME);
    let all_sent = on_server
        && run
            .entries
            .iter()
            .all(|e| crate::upload::uploader::is_sent(&spool, &e.entry_id));
    let exited = matches!(state.harness, crate::run::HarnessState::Exited { .. });
    // Done: its time is up, or the agent left a run nothing can restart.
    let done = over || (state.ended && exited && advice == crate::run::Advice::Cannot);
    // Sealed only with a valid `seal.json`: a host can finish without having written one.
    let sealed = run
        .entries
        .last()
        .is_some_and(|e| crate::upload::prune::is_sealed(&e.trace_dir, &e.entry_id));
    let phase = if done {
        if !state.ended {
            participant::Phase::Finishing
        } else if all_sent {
            participant::Phase::Uploaded
        } else if sealed {
            participant::Phase::Sealed
        } else {
            participant::Phase::NotSealed
        }
    } else if exited {
        participant::Phase::AgentExited
    } else {
        participant::Phase::Recording
    };
    let agent = match &state.harness {
        crate::run::HarnessState::Running => participant::Agent::Running,
        crate::run::HarnessState::Exited { .. } if state.stopped_for_its_end() => {
            participant::Agent::Stopped
        }
        crate::run::HarnessState::Exited { .. }
            if state.stopped.as_deref() == Some(crate::run::watch::DISK_STOP_REASON) =>
        {
            participant::Agent::StoppedForDisk
        }
        crate::run::HarnessState::Exited { status } => participant::Agent::Exited { how: status },
    };
    let ago = state
        .upload
        .last_ok_at
        .map(|last_ok| now.saturating_sub(last_ok));
    let stale = state.is_stale(now);
    let upload = if !on_server {
        participant::UploadNow::NotUsed
    } else if phase == participant::Phase::Uploaded {
        participant::UploadNow::Done
    } else if state.ended && !all_sent {
        // Nothing is sending now: the host is done, and what is left waits for `goalcup upload`
        // (or, for an open run, for `goalcup continue`).
        participant::UploadNow::Unfinished
    } else if state.upload.stopped {
        participant::UploadNow::GaveUp
    } else if state.upload.problem.is_some() {
        participant::UploadNow::Retrying { ago }
    } else if ago.is_some() {
        participant::UploadNow::UpToDate { ago }
    } else {
        participant::UploadNow::NotYet
    };
    let next = match phase {
        participant::Phase::Recording if problems.is_empty() => participant::Next::NothingEnds,
        participant::Phase::Recording | participant::Phase::AgentExited
            if stale && advice == crate::run::Advice::Continue =>
        {
            participant::Next::Continue {
                run_id: &run.run_id,
            }
        }
        participant::Phase::Recording => participant::Next::Problems,
        participant::Phase::AgentExited if advice == crate::run::Advice::Continue => {
            participant::Next::Continue {
                run_id: &run.run_id,
            }
        }
        participant::Phase::AgentExited | participant::Phase::Uploaded => {
            participant::Next::Nothing
        }
        // Time is up and the state is moving: goalcup is finishing the run. If nothing writes it
        // any more (the computer restarted, the process was killed), nobody will: seal it here.
        participant::Phase::Finishing if stale => participant::Next::Seal { upload: on_server },
        participant::Phase::NotSealed => participant::Next::Seal { upload: on_server },
        participant::Phase::Finishing => participant::Next::Wait,
        participant::Phase::Sealed if on_server => participant::Next::Upload,
        participant::Phase::Sealed => participant::Next::Nothing,
    };
    let mut header = participant::status_header(&participant::StatusHeader {
        harness: &name,
        mode,
        phase: Some(phase),
        ends,
        agent,
        upload,
        problems: problems.len(),
        stale,
        early,
        young: false,
    });
    // While goalcup records: the disk, and what its own processes use.
    if !state.ended
        && let Some(line) = participant::this_computer(&local_vitals(
            &crate::run::run_dir(cfg, &run.run_id),
            &state,
        ))
    {
        header.push(line);
    }
    let problems: String = problems
        .iter()
        .map(|problem| format!("{} {problem}\n", participant::PROBLEM))
        .collect();
    let finished = finished.map(|line| format!("{line}\n")).unwrap_or_default();
    let text = format!(
        "{banner}{}\n{problems}{finished}{}\n",
        header.join("\n"),
        participant::status_next(&next)
    );
    (plain_lines(&text), Some(run.run_id))
}

/// How long after its start a run may be without a state before it is called not started.
const NEW_RUN_SECS: u64 = 5 * 60;

/// The harness's display name from its connector, or the connector id when the connector cannot
/// be loaded here.
fn harness_name(connectors: Option<&Path>, connector_id: &str, dev: bool) -> String {
    connectors
        .and_then(|dir| crate::connector::load_harness(dir, connector_id, dev).ok())
        .map_or_else(
            || connector_id.to_string(),
            |connector| connector.display_name().to_string(),
        )
}

/// A block of lines, with anything a terminal would obey taken out of each ([`plain`]) but the
/// line ends.
fn plain_lines(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| match line.strip_suffix('\n') {
            Some(line) => format!("{}\n", plain(line)),
            None => plain(line),
        })
        .collect()
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
        None => {
            entries::latest_entry(cfg)?.ok_or_else(|| CliError::Refused(participant::no_run_yet()))
        }
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
    let workspace = plain(&p.workspace);
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("entry {} ({})", p.entry_id, p.connector_id));

    // First, before anything a reader might act on: whether anything has published a
    // document for this entry lately.
    lines.push(freshness(s, now));

    match &p.capture {
        Some(capture) => lines.extend(capture_lines(p, capture, &detail, &workspace)),
        // The daemon always sends a capture block. A document without one is still signed by
        // the pinned key, so its sentence is shown, and nothing is guessed for the rest.
        None => {
            lines.push(format!("  {detail}"));
            lines.push(format!("  workspace: {workspace}"));
        }
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
