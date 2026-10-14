//! `goalcup upload <entry>` — finish uploading a run whose `goalcup start` could not: the
//! server was away at the end, the laptop slept, or the process was killed.
//!
//! It asks the server where it is (`GET /v1/runs/{id}`), sends every complete line of the
//! local chain from there, and the seal once the run is sealed, waiting out a server that is
//! away with the same backoff the run uses. Safe to run again: the server takes nothing twice.
//! It is refused while another process (the run's own `goalcup start`) uploads the entry.

use crate::CliError;
use crate::upload::http::ServerClient;
use crate::upload::uploader::{Policy, Uploader};
use std::path::Path;

/// The testable half: `entry_id` uploaded to the server `goalcup login` stored.
pub fn upload_with(
    cfg: &Path,
    entry_id: &str,
    policy: Policy,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let record = crate::upload::read_record(cfg, entry_id)?.ok_or_else(|| {
        CliError::Refused(format!(
            "entry {entry_id} was not opened on a server from this machine (there is no {}), \
             so there is nothing to upload.",
            crate::upload::record_path(cfg, entry_id).display()
        ))
    })?;
    let config = crate::server::read_server_config(cfg)?.ok_or_else(|| {
        CliError::Refused(format!(
            "this tracker is not logged in. Run `goalcup login --server {}` first.",
            record.server_url
        ))
    })?;
    if config.url != record.server_url {
        return Err(CliError::Refused(format!(
            "entry {entry_id} was opened on {}, and this tracker is logged in to {}. Log in to \
             {} to upload it.",
            record.server_url, config.url, record.server_url
        )));
    }
    let receipt_path = crate::upload::receipt_path(cfg, entry_id);
    let receipt: goalcup_protocol::receipt::StartReceipt = serde_json::from_slice(
        &std::fs::read(&receipt_path).map_err(|source| CliError::Io {
            path: receipt_path.display().to_string(),
            source,
        })?,
    )?;
    let server = ServerClient::new(&config);
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
                say(&format!(
                    "upload: {error}; trying again in {}s",
                    wait.as_secs().max(1)
                ));
                std::thread::sleep(wait);
            }
            Err(error) => {
                return Err(CliError::Refused(format!(
                    "the server at {} refused entry {entry_id}: {error}",
                    record.server_url
                )));
            }
        }
    };
    let report = uploader.finish(None, &mut say);
    match report {
        crate::upload::uploader::Report {
            caught_up: true,
            sealed: true,
            ..
        } => say(&format!(
            "uploaded: {} bytes of entry {entry_id}, acknowledged, and its seal, on {}.",
            report.offset, record.server_url
        )),
        crate::upload::uploader::Report {
            caught_up: true,
            why: None,
            ..
        } => say(&format!(
            "uploaded: {} bytes of entry {entry_id}, acknowledged. The run is not sealed yet; \
             run this again once it is.",
            report.offset
        )),
        crate::upload::uploader::Report { why, .. } => {
            return Err(CliError::Refused(format!(
                "entry {entry_id} is not fully uploaded ({} bytes are): {}",
                report.offset,
                why.unwrap_or_default()
            )));
        }
    }
    Ok(())
}
