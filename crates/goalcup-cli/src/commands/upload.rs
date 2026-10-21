//! `goalcup upload [<entry>]` — finish uploading a run whose `goalcup start` could not: the
//! server was away at the end, the laptop slept, or the process was killed.
//!
//! It asks the server where it is (`GET /v1/runs/{id}`), sends every complete line of the
//! local chain from there (across its parts; the bytes below the server's offset may already be
//! deleted here), and the seal once the run is sealed, waiting out a server that is
//! away with the same backoff the run uses. Safe to run again: the server takes nothing twice.
//! It is refused while another process (the run's own `goalcup start`) uploads the entry.
//!
//! With no entry id it uploads each entry of the newest run started on this machine whose upload
//! has not finished ([`upload_newest_with`]).

use crate::CliError;
use crate::participant;
use crate::upload::http::ServerClient;
use crate::upload::policy::WrapUp;
use crate::upload::uploader::{Policy, Uploader};
use std::path::Path;
use std::time::Duration;

/// How long a command waits for [`send_leftovers`] before it goes on without it.
pub const LEFTOVERS_WAIT: Duration = Duration::from_secs(3);

/// The finished parts of runs on this computer that are not on the server yet: sealed here, not
/// sent, of a run opened on a server whose credential is here. The run `skip` is left out (the
/// command that called this sends that one itself). Oldest run first.
pub fn leftovers(cfg: &Path, skip: Option<&str>) -> Vec<String> {
    let spool = cfg.join(crate::upload::UPLOADS_DIR_NAME);
    let mut runs = crate::run::all(cfg);
    runs.sort_by_key(|run| run.started_at_unix);
    runs.into_iter()
        .filter(|run| run.server_url.is_some() && skip != Some(run.run_id.as_str()))
        .filter(|run| {
            crate::account::credential::credential_path(&crate::run::run_dir(cfg, &run.run_id))
                .exists()
        })
        .flat_map(|run| run.entries)
        .filter(|part| {
            !crate::upload::uploader::is_sent(&spool, &part.entry_id)
                && crate::upload::prune::is_sealed(&part.trace_dir, &part.entry_id)
        })
        .map(|part| part.entry_id)
        .collect()
}

/// Send the [`leftovers`] by themselves, as a participant's `goalcup` command begins: best
/// effort, on a thread of its own, waited for at most `wait`. A part another goalcup is sending,
/// a server that is away, anything that fails, is left for the next time; nothing is printed
/// (what the upload says goes to a run's log, when one is open). A part still being sent when
/// the wait is over goes on in the background while the command runs, and is cut off when it
/// ends, which is safe: the server takes nothing twice, and the next send goes on from there.
pub fn send_leftovers(cfg: &Path, policy: Policy, skip: Option<&str>, wait: Duration) {
    let parts = leftovers(cfg, skip);
    if parts.is_empty() {
        return;
    }
    let cfg = cfg.to_path_buf();
    within(wait, move || {
        let mut log = LogOnly;
        for part in parts {
            let _ = upload_entry(&cfg, &part, policy, Some(WrapUp::after(wait)), &mut log);
        }
    });
}

/// Run `job` on a thread of its own and wait for it `wait` at most: past that it goes on
/// without being waited for. A thread that cannot be started is a job not done.
fn within(wait: Duration, job: impl FnOnce() + Send + 'static) {
    let (done, finished) = std::sync::mpsc::channel();
    let started = std::thread::Builder::new()
        .name("goalcup-leftovers".to_string())
        .spawn(move || {
            job();
            let _ = done.send(());
        });
    if started.is_ok() {
        let _ = finished.recv_timeout(wait);
    }
}

/// What a background upload says: to the run's log while one is open, never to the terminal.
struct LogOnly;

impl std::io::Write for LogOnly {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let line = text.trim_end();
        if !line.is_empty() {
            crate::host_log::log_only(line);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The unsent entries of the newest run started on this machine, oldest first (a continued run
/// has one entry per segment), with the run's id. Refuses when there is no run, or when the
/// newest run is local (it was never opened on a server, so it has nothing to send).
pub fn unsent_of_newest_run(cfg: &Path) -> Result<(String, Vec<String>), CliError> {
    let run =
        crate::run::newest(cfg)?.ok_or_else(|| CliError::Refused(participant::upload_no_run()))?;
    if run.server_url.is_none() {
        return Err(CliError::Refused(participant::upload_local_run(
            &run.run_id,
        )));
    }
    let spool = cfg.join(crate::upload::UPLOADS_DIR_NAME);
    let unsent = run
        .entries
        .iter()
        .map(|e| e.entry_id.clone())
        .filter(|id| !crate::upload::uploader::is_sent(&spool, id))
        .collect();
    Ok((run.run_id, unsent))
}

/// `goalcup upload` with no entry id: [`upload_with`] for each unsent entry of the newest run.
/// Every entry is tried; the first refusal is returned at the end, so one bad segment does not
/// hide the others.
pub fn upload_newest_with(
    cfg: &Path,
    policy: Policy,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let (run_id, unsent) = unsent_of_newest_run(cfg)?;
    if unsent.is_empty() {
        // `upload_with` does this first; a run with nothing to send must not skip it.
        crate::run::reserve::release_stale(
            cfg,
            &crate::run::reserve::artifact_root(cfg),
            crate::run::now_unix(),
        );
        crate::upload::prune::clean_finished_runs(cfg);
        let _ = writeln!(out, "{}", participant::upload_nothing_left(&run_id));
        return Ok(());
    }
    let mut first_error = None;
    // The parts are sent one after the other; the plain word on how it went comes once, at the
    // end, because a part sent early does not say the run is done.
    let mut last = None;
    for entry_id in &unsent {
        match upload_entry(cfg, entry_id, policy, None, out) {
            Ok(sent) => last = Some(sent),
            Err(error) => {
                // The first refusal is returned, and so printed by the caller; the others are
                // said here so that one bad part does not hide them.
                if first_error.is_some() {
                    let _ = writeln!(out, "{error}");
                }
                first_error.get_or_insert(error);
            }
        }
    }
    // What is sealed and sent, of this run or an older one, need not stay on this machine.
    crate::upload::prune::clean_finished_runs(cfg);
    if first_error.is_none()
        && let Some(sent) = last
    {
        let _ = writeln!(out, "{}", sent.plain());
    }
    first_error.map_or(Ok(()), Err)
}

/// How far one part of a run got.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sent {
    /// All of it is on the server, with its seal.
    Sealed,
    /// All of it so far is on the server; the part is still recording.
    NotSealedYet,
    /// All of it is on the server, and nothing records the part any more, but it was never
    /// sealed (its host died first, or its `seal.json` could not be written): only `goalcup seal`
    /// can seal it now.
    SealMissing(String),
}

impl Sent {
    /// The plain line for the end of an upload, the next action first.
    fn plain(&self) -> String {
        match self {
            Sent::Sealed => participant::upload_finished(),
            Sent::NotSealedYet => participant::upload_not_sealed(),
            Sent::SealMissing(entry_id) => participant::upload_seal_missing(entry_id),
        }
    }
}

/// The testable half: `entry_id` uploaded to the server it was opened on, with its run's own
/// credential.
pub fn upload_with(
    cfg: &Path,
    entry_id: &str,
    policy: Policy,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let sent = upload_entry(cfg, entry_id, policy, None, out);
    // What is sealed and sent, of this run or an older one, need not stay on this machine.
    crate::upload::prune::clean_finished_runs(cfg);
    let _ = writeln!(out, "{}", sent?.plain());
    Ok(())
}

/// [`upload_with`] without its closing plain line: the `uploaded:` line is the technical record,
/// and the caller says how it went. `until` bounds a send nobody waits for
/// ([`send_leftovers`]): one try to reach the server, and the upload within it. `None` is
/// `goalcup upload`, which waits for a server that is away.
fn upload_entry(
    cfg: &Path,
    entry_id: &str,
    policy: Policy,
    until: Option<WrapUp>,
    out: &mut dyn std::io::Write,
) -> Result<Sent, CliError> {
    // A reserve nothing will use again goes back to the disk, whatever becomes of this upload.
    // Not from a send in the background: the command beside it may be taking a reserve.
    if until.is_none() {
        crate::run::reserve::release_stale(
            cfg,
            &crate::run::reserve::artifact_root(cfg),
            crate::run::now_unix(),
        );
    }
    let record = crate::upload::read_record(cfg, entry_id)?
        .ok_or_else(|| CliError::Refused(participant::upload_not_opened(entry_id)))?;
    // The run holds the credential its approval returned, in its own directory: found from the
    // entry, so the participant need not be logged in to, or name, anything.
    let not_here = || CliError::Refused(participant::upload_not_here(entry_id));
    let run = crate::run::of_entry(cfg, entry_id)?.ok_or_else(not_here)?;
    let run_dir = crate::run::run_dir(cfg, &run.run_id);
    if !crate::account::credential::credential_path(&run_dir).exists() {
        return Err(not_here());
    }
    let credential = crate::account::credential::load(&run_dir)?;
    let receipt = crate::upload::read_receipt(cfg, entry_id)?;
    let server = ServerClient::with_credential(&record.server_url, &credential);
    let spool = cfg.join(crate::upload::UPLOADS_DIR_NAME);
    // One uploader per entry: refused while the run's own `goalcup start` still uploads it.
    let _lock = crate::upload::uploader::lock_entry(&spool, entry_id)?;
    let mut say = |line: &str| {
        let _ = writeln!(out, "{line}");
    };
    let mut failures = 0;
    let mut uploader = loop {
        match Uploader::resume(
            server.clone(),
            record.clone(),
            receipt.public_key.clone(),
            policy,
            &spool,
        ) {
            Ok(uploader) => break uploader,
            Err(error) if !error.ends_manual_upload() && until.is_none() => {
                failures += 1;
                let wait = crate::upload::uploader::backoff(
                    &policy,
                    failures,
                    error.retry_after().map(std::time::Duration::from_secs),
                );
                say(&participant::upload_waiting(
                    &error.to_string(),
                    wait.as_secs().max(1),
                ));
                std::thread::sleep(wait);
            }
            Err(error) => {
                return Err(CliError::Refused(participant::upload_refused(
                    &record.server_url,
                    entry_id,
                    &error.to_string(),
                )));
            }
        }
    };
    let report = uploader.finish(until, &mut say);
    let sent = match report {
        crate::upload::uploader::Report {
            caught_up: true,
            sealed: true,
            ..
        } => {
            say(&format!(
                "uploaded: {} bytes of part {entry_id}, acknowledged, and its seal, on {}.",
                report.offset, record.server_url
            ));
            // The last entry of the run is on the server: if the run is over (or cannot be
            // continued), its reserve has no more use.
            crate::run::reserve::release_if_done(
                cfg,
                &crate::run::reserve::artifact_root(cfg),
                &run.run_id,
                report.run_ended,
                crate::run::now_unix(),
            );
            Sent::Sealed
        }
        crate::upload::uploader::Report {
            caught_up: true,
            why: None,
            ..
        } => {
            // Only a host that still records this part seals it later. One that has finished, or
            // that nothing has heard from for a while (it died), left no seal and none is coming:
            // "run upload again" would be said for ever, so the seal is asked for instead.
            let recording = crate::run::read_state(&run_dir).is_some_and(|state| {
                state.entry_id == entry_id
                    && !state.ended
                    && !state.is_stale(crate::run::now_unix())
            });
            if crate::upload::uploader::is_sent(&spool, entry_id) {
                // A part closed from the end of its chain has no seal, and is sent all the same.
                Sent::Sealed
            } else if !recording
                && !crate::upload::prune::is_sealed(Path::new(&record.trace_dir), entry_id)
            {
                say(&format!(
                    "uploaded: {} bytes of part {entry_id}, acknowledged. Its seal.json was not \
                     written, so the part is not sealed.",
                    report.offset
                ));
                Sent::SealMissing(entry_id.to_string())
            } else {
                say(&format!(
                    "uploaded: {} bytes of part {entry_id}, acknowledged. The part is still \
                     recording; its own goalcup sends the rest.",
                    report.offset
                ));
                Sent::NotSealedYet
            }
        }
        crate::upload::uploader::Report { why, .. } => {
            return Err(CliError::Refused(participant::upload_incomplete(
                entry_id,
                report.offset,
                &why.unwrap_or_default(),
            )));
        }
    };
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A run of one part under `cfg`, on `server` (with its credential) or local; its part sealed
    /// when `sealed`.
    fn run_of_one_part(cfg: &Path, run_id: &str, server: Option<&str>, started: u64, sealed: bool) {
        let part = format!("{run_id}-part");
        let trace_dir = cfg.join("traces").join(&part);
        std::fs::create_dir_all(&trace_dir).unwrap();
        if sealed {
            let seal = serde_json::json!({
                "entry_id": part, "sealed_at": "2026-10-21T00:00:00Z", "workspace": "/w",
                "files": [], "excluded": [],
            });
            std::fs::write(trace_dir.join("seal.json"), seal.to_string()).unwrap();
        }
        let record = crate::run::RunRecord {
            run_id: run_id.to_string(),
            server_url: server.map(str::to_string),
            practice: false,
            test: false,
            connector_id: "c".to_string(),
            workspace: PathBuf::from("/w"),
            started_at_unix: started,
            deadline_at: None,
            entries: vec![crate::run::EntryRef {
                entry_id: part,
                trace_dir,
            }],
        };
        crate::run::write(cfg, &record).unwrap();
        if server.is_some() {
            let credential = crate::account::credential::RunCredential::new(
                format!("gcr_{}", "ab".repeat(32)),
                "2099-01-01T00:00:00Z".to_string(),
            )
            .unwrap();
            crate::account::credential::store(&crate::run::run_dir(cfg, run_id), &credential)
                .unwrap();
        }
    }

    /// What goes up by itself is the sealed, unsent parts of runs on a server, but the run the
    /// command sends itself. The send is waited for a bounded time only, and a part that cannot
    /// be sent is left as it was, with nothing failing.
    #[test]
    fn leftovers_go_up_by_themselves_best_effort_and_bounded() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path();
        let server = Some("https://server.example");
        run_of_one_part(cfg, "old", server, 10, true);
        run_of_one_part(cfg, "new", server, 20, true);
        // Not leftovers: a part not sealed (its host may still record it), a local run, a part
        // already sent.
        run_of_one_part(cfg, "open", server, 30, false);
        run_of_one_part(cfg, "local", None, 40, true);
        run_of_one_part(cfg, "sent", server, 50, true);
        let spool = cfg.join(crate::upload::UPLOADS_DIR_NAME);
        std::fs::create_dir_all(&spool).unwrap();
        std::fs::write(spool.join("sent-part.sent"), "sent\n").unwrap();

        assert_eq!(leftovers(cfg, None), ["old-part", "new-part"]);
        assert_eq!(leftovers(cfg, Some("new")), ["old-part"]);

        // A send that takes far longer than the wait is not waited for.
        let began = std::time::Instant::now();
        within(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(30))
        });
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "{:?}",
            began.elapsed()
        );

        // Best effort: these parts have no upload record here, so nothing can be sent. Nothing
        // fails, nothing is printed, and they are still there for the next time.
        send_leftovers(cfg, Policy::default(), None, Duration::from_secs(2));
        assert_eq!(leftovers(cfg, None), ["old-part", "new-part"]);
    }
}
