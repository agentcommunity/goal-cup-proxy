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
use crate::upload::uploader::{Policy, Uploader};
use std::path::Path;

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
        let _ = writeln!(out, "{}", participant::upload_nothing_left(&run_id));
        return Ok(());
    }
    let mut first_error = None;
    // The parts are sent one after the other; the plain word on how it went comes once, at the
    // end, because a part sent early does not say the run is done.
    let mut last = None;
    for entry_id in &unsent {
        match upload_entry(cfg, entry_id, policy, out) {
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
    if first_error.is_none()
        && let Some(sent) = last
    {
        let _ = writeln!(out, "{}", sent.plain());
    }
    first_error.map_or(Ok(()), Err)
}

/// How far one part of a run got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sent {
    /// All of it is on the server, with its seal.
    Sealed,
    /// All of it so far is on the server; the run is not sealed yet.
    NotSealedYet,
}

impl Sent {
    /// The plain line for the end of an upload, the next action first.
    fn plain(self) -> String {
        match self {
            Sent::Sealed => participant::upload_finished(),
            Sent::NotSealedYet => participant::upload_not_sealed(),
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
    let sent = upload_entry(cfg, entry_id, policy, out)?;
    let _ = writeln!(out, "{}", sent.plain());
    Ok(())
}

/// [`upload_with`] without its closing plain line: the `uploaded:` line is the technical record,
/// and the caller says how it went.
fn upload_entry(
    cfg: &Path,
    entry_id: &str,
    policy: Policy,
    out: &mut dyn std::io::Write,
) -> Result<Sent, CliError> {
    // A reserve nothing will use again goes back to the disk, whatever becomes of this upload.
    crate::run::reserve::release_stale(
        cfg,
        &crate::run::reserve::artifact_root(cfg),
        crate::run::now_unix(),
    );
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
    let receipt_path = crate::upload::receipt_path(cfg, entry_id);
    let receipt: goalcup_protocol::receipt::StartReceipt = serde_json::from_slice(
        &std::fs::read(&receipt_path).map_err(|source| CliError::Io {
            path: receipt_path.display().to_string(),
            source,
        })?,
    )?;
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
            Err(error) if !error.ends_upload() => {
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
    let report = uploader.finish(None, &mut say);
    let sent = match report {
        crate::upload::uploader::Report {
            caught_up: true,
            sealed: true,
            ..
        } => {
            say(&format!(
                "uploaded: {} bytes of entry {entry_id}, acknowledged, and its seal, on {}.",
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
            say(&format!(
                "uploaded: {} bytes of entry {entry_id}, acknowledged. The run is not sealed yet; \
                 run this again once it is.",
                report.offset
            ));
            Sent::NotSealedYet
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
