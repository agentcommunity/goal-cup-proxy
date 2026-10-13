//! The upload server's side of a run (docs/SERVER-DESIGN.md, docs/UPLOAD-API.md).
//!
//! Off unless `goalcup login` stored a server ([`crate::server`]): with no `server.json`, and
//! for a `--dev` run, nothing here is called and the run is local only.
//!
//! What a run with a server does, in `goalcup start`'s order:
//!
//! 1. [`open_run`] before anything is created: the server issues the `entry_id`, stamps
//!    `official_start` and sets `deadline_at`. A refusal, or no answer, stops the start.
//! 2. The daemon creates the entry under that `entry_id` and chains its baseline.
//! 3. [`fetch_receipt`] with the baseline's link hash: the server signs a start receipt over
//!    the run's fields and that head. It is checked against what was sent before it is kept
//!    ([`keep_run`]): in `<cfg>/uploads/` and as the trace's `receipt.json`.
//!
//! A failure between 1 and the launch gives the run back ([`abandon_run`]), so the account
//! is not left holding an open run with nothing in it.

pub mod api;
pub mod http;
pub mod uploader;

use crate::CliError;
use crate::config::write_secret_file;
use api::{AbandonRequest, OpenRun, OpenRunRequest, ReceiptAnswer, ReceiptRequest, StatusAnswer};
use goalcup_protocol::receipt::StartReceipt;
use http::{ServerClient, ServerError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `<cfg>/uploads`: one record and one receipt per entry opened with the server.
pub const UPLOADS_DIR_NAME: &str = "uploads";

/// How many times a start asks before it gives up, and how long it waits between asks.
const START_ATTEMPTS: u32 = 3;
const START_RETRY_WAIT: Duration = Duration::from_secs(3);

/// What a later `goalcup upload` needs to find the run again: `<cfg>/uploads/<entry>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadRecord {
    pub run_id: String,
    pub entry_id: String,
    pub server_url: String,
    /// The entry's trace directory, holding `chain.jsonl`.
    pub trace_dir: String,
    pub official_start: String,
    pub deadline_at: String,
}

/// `<cfg>/uploads/<entry_id>.json`.
pub fn record_path(cfg: &Path, entry_id: &str) -> PathBuf {
    cfg.join(UPLOADS_DIR_NAME).join(format!("{entry_id}.json"))
}

/// `<cfg>/uploads/<entry_id>.receipt.json`: the start receipt as the server signed it.
pub fn receipt_path(cfg: &Path, entry_id: &str) -> PathBuf {
    cfg.join(UPLOADS_DIR_NAME)
        .join(format!("{entry_id}.receipt.json"))
}

/// The record `goalcup start` kept for `entry_id`, or `None` for a local-only run.
pub fn read_record(cfg: &Path, entry_id: &str) -> Result<Option<UploadRecord>, CliError> {
    let path = record_path(cfg, entry_id);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(CliError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// `url` checked: `https://github.com/<owner>/<repo>`, the shape the server accepts. A
/// trailing `/` or `.git` is dropped.
pub fn checked_repo_url(url: &str) -> Result<String, CliError> {
    let trimmed = url.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let ok = trimmed
        .strip_prefix("https://github.com/")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(owner, repo)| {
            let name = |s: &str, extra: &[u8]| {
                !s.is_empty()
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || extra.contains(&b))
            };
            name(owner, b"") && name(repo, b"._") && !repo.contains('/')
        });
    if !ok {
        return Err(CliError::Refused(format!(
            "--repo {url:?} is not a GitHub repository address. Give the public repository \
             your agent pushes to, as https://github.com/<owner>/<repo>. Nothing was created."
        )));
    }
    Ok(trimmed.to_string())
}

/// `call`, asked again up to [`START_ATTEMPTS`] times while the answer is retryable. A start
/// is a person waiting at a terminal, so this does not back off for minutes: a server that
/// stays away refuses the start, and the participant can try again.
fn with_retries<T>(mut call: impl FnMut() -> Result<T, ServerError>) -> Result<T, ServerError> {
    let mut attempt = 1;
    loop {
        match call() {
            Err(error) if error.is_retryable() && attempt < START_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(START_RETRY_WAIT);
            }
            answer => return answer,
        }
    }
}

fn refused(what: &str, server: &ServerClient, error: ServerError) -> CliError {
    let hint = match &error {
        ServerError::Status { status: 401, .. } => {
            " Your token was not accepted: run `goalcup login` again with a token from the site."
        }
        ServerError::Transport(_) => " Check your network, or try again in a few minutes.",
        _ => "",
    };
    CliError::Refused(format!(
        "{what}: {} ({}).{hint} Nothing was started.",
        error,
        server.base()
    ))
}

/// `POST /v1/runs`. Repeats are safe: the server answers a repeat of the same `session_id`
/// with the same run.
pub fn open_run(server: &ServerClient, request: &OpenRunRequest) -> Result<OpenRun, CliError> {
    let run: OpenRun = with_retries(|| server.json("POST", "/v1/runs", Some(request)))
        .map_err(|e| refused("the Goal Cup server did not open the run", server, e))?;
    let well_formed = goalcup_protocol::control::is_entry_id(&run.entry_id)
        && is_run_id(&run.run_id)
        && goalcup_protocol::control::parse_z(&run.official_start).is_some()
        && matches!(
            (
                goalcup_protocol::control::parse_z(&run.official_start),
                goalcup_protocol::control::parse_z(&run.deadline_at),
            ),
            (Some(start), Some(deadline)) if start < deadline
        );
    if !well_formed {
        return Err(CliError::Refused(format!(
            "the Goal Cup server at {} answered a run this tracker cannot use: {run:?}. \
             Nothing was started.",
            server.base()
        )));
    }
    Ok(run)
}

/// Whether `id` is a run id: `r` and 16 lowercase hex digits.
pub fn is_run_id(id: &str) -> bool {
    id.len() == 17
        && id.starts_with('r')
        && id[1..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `POST /v1/runs/{run_id}/receipt`, and the answer checked: its claims are exactly the run
/// that was opened and the head that was sent, and its signature verifies under the key it
/// names. Which keys to trust is the verifier's question, not this one's; this check only
/// stops a receipt that could never bind this trace from being kept.
pub fn fetch_receipt(
    server: &ServerClient,
    run: &OpenRun,
    opened: &OpenRunRequest,
    chain_head: &str,
) -> Result<StartReceipt, CliError> {
    let request = ReceiptRequest {
        entry_id: run.entry_id.clone(),
        chain_head: chain_head.to_string(),
    };
    let answer: ReceiptAnswer = with_retries(|| {
        server.json(
            "POST",
            &format!("/v1/runs/{}/receipt", run.run_id),
            Some(&request),
        )
    })
    .map_err(|e| {
        refused(
            "the Goal Cup server did not sign the start receipt",
            server,
            e,
        )
    })?;
    let receipt = answer.start_receipt;
    let c = &receipt.claims;
    let matches = c.entry_id == run.entry_id
        && c.connector_id == opened.connector_id
        && c.workspace == opened.workspace
        && c.session_id == opened.session_id
        && c.started_at == run.official_start
        && c.deadline_at == run.deadline_at
        && c.manifest_sha256 == opened.manifest_sha256
        && c.chain_head == chain_head
        && c.connector_dir_sha256.as_deref() == Some(opened.connector_dir_sha256.as_str())
        && c.run_public_key.is_none()
        && c.questionnaire_version.is_none();
    if !matches {
        return Err(CliError::Refused(format!(
            "the start receipt from {} does not describe this run. Nothing was started.",
            server.base()
        )));
    }
    if receipt.verify_with(&receipt.public_key).is_err() {
        return Err(CliError::Refused(format!(
            "the start receipt from {} does not carry a valid signature. Nothing was started.",
            server.base()
        )));
    }
    Ok(receipt)
}

/// `POST /v1/runs/{run_id}/abandon`, once, for a start that failed before its launch. Best
/// effort: the start is already failing, and its own refusal is what the participant reads.
pub fn abandon_run(server: &ServerClient, run_id: &str, reason: &str) -> Result<(), ServerError> {
    let reason: String = reason.chars().take(500).collect();
    server
        .json::<StatusAnswer>(
            "POST",
            &format!("/v1/runs/{run_id}/abandon"),
            Some(&AbandonRequest { reason }),
        )
        .map(|_| ())
}

/// Keep the run: its record and receipt under `<cfg>/uploads/`, and the receipt as the
/// trace's `receipt.json`, which `goalcup-verify` reads.
pub fn keep_run(cfg: &Path, record: &UploadRecord, receipt: &StartReceipt) -> Result<(), CliError> {
    let receipt_bytes = serde_json::to_vec_pretty(receipt)?;
    write_secret_file(
        &record_path(cfg, &record.entry_id),
        &serde_json::to_vec_pretty(record)?,
    )?;
    write_secret_file(&receipt_path(cfg, &record.entry_id), &receipt_bytes)?;
    let in_trace = Path::new(&record.trace_dir).join(goalcup_protocol::export::RECEIPT_FILE);
    write_new_file(&in_trace, &receipt_bytes)
}

/// `bytes` at `path`, which must not exist yet, made durable.
fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    use std::io::Write;
    let io = |source| CliError::Io {
        path: path.display().to_string(),
        source,
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}
