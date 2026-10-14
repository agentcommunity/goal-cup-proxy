//! The upload server's side of a run (docs/SERVER-DESIGN.md, docs/UPLOAD-API.md).
//!
//! Off unless `goalcup login` stored a server ([`crate::server`]): with no `server.json`, and
//! for a harness given a provider key, nothing here is called and the run is local only.
//!
//! What a run with a server does, in `goalcup start`'s order:
//!
//! 0. The participant approves the run in the browser (`account::device`); the credential that
//!    comes back is this run's alone, and every call below carries it.
//! 1. [`open_run`] before the daemon creates anything: the server issues the `entry_id`, stamps
//!    `official_start` and sets `deadline_at`. A refusal, or no answer, stops the start.
//! 2. The daemon creates the entry under that `entry_id` and chains its baseline.
//! 3. [`fetch_receipt`] with the baseline's link hash: the server signs a start receipt over
//!    the run's fields and that head. It is checked against what was sent before it is kept
//!    ([`keep_run`]): in `<cfg>/uploads/` and as the trace's `receipt.json`.
//!
//! A failure between 1 and the launch gives the run back ([`GiveBack`]), so the account is not
//! left holding an open run with nothing in it. A start killed in that window cannot, so it
//! leaves `<cfg>/uploads/pending-open.json`, and the next start gives that run back first, with
//! the credential the killed start saved in that run's directory.

pub mod api;
pub mod http;
pub mod policy;
pub mod prune;
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
    /// The run's workspace, whose repository's checked-out commit the final heartbeat names.
    #[serde(default)]
    pub workspace: String,
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
            " This run's credential was not accepted: run the command again to approve the run \
             again."
        }
        ServerError::Status { status: 409, .. } => {
            " The server says this account already has an open run: a run that is still going \
             cannot be started twice."
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

/// `POST /v1/runs/{run_id}/abandon`, once. `reason` is a fixed sentence: nothing local (a
/// path, a user name) is sent with it.
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

/// `<cfg>/uploads/pending-open.json`: a run opened on the server whose start has not launched.
pub fn pending_path(cfg: &Path) -> PathBuf {
    cfg.join(UPLOADS_DIR_NAME).join("pending-open.json")
}

/// What [`pending_path`] holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOpen {
    pub run_id: String,
    pub server_url: String,
}

/// A run opened on the server and not launched yet: given back if the start fails before the
/// launch, kept once the harness runs.
#[derive(Debug)]
pub struct GiveBack {
    pub server: ServerClient,
    pub cfg: PathBuf,
    pub run_id: String,
    pub entry_id: String,
}

impl GiveBack {
    /// Note the opened run on disk, so a start killed before its launch is given back by the
    /// next one.
    pub fn opened(server: &ServerClient, cfg: &Path, run: &OpenRun) -> Result<GiveBack, CliError> {
        let pending = PendingOpen {
            run_id: run.run_id.clone(),
            server_url: server.base().to_string(),
        };
        write_secret_file(&pending_path(cfg), &serde_json::to_vec_pretty(&pending)?)?;
        Ok(GiveBack {
            server: server.clone(),
            cfg: cfg.to_path_buf(),
            run_id: run.run_id.clone(),
            entry_id: run.entry_id.clone(),
        })
    }

    /// The start failed before its launch: give the run back, and remove what this machine
    /// kept of it. Best effort: the start's own refusal is what the participant reads; a run
    /// the server did not hear about is given back by the next start.
    pub fn give(self) {
        let given = abandon_run(
            &self.server,
            &self.run_id,
            "goalcup start failed before the launch",
        );
        let _ = std::fs::remove_file(record_path(&self.cfg, &self.entry_id));
        let _ = std::fs::remove_file(receipt_path(&self.cfg, &self.entry_id));
        if settled(&given) {
            let _ = std::fs::remove_file(pending_path(&self.cfg));
            forget_credential(&self.cfg, &self.run_id);
        }
    }

    /// The harness was launched: the run is the participant's now.
    pub fn launched(self) {
        let _ = std::fs::remove_file(pending_path(&self.cfg));
    }
}

/// Whether an abandon's answer settles the run: given back, or past giving back (gone,
/// holding chunks, or closed).
fn settled(answer: &Result<(), ServerError>) -> bool {
    match answer {
        Ok(()) => true,
        Err(ServerError::Status { status, .. }) => matches!(status, 404 | 409 | 410),
        Err(_) => false,
    }
}

/// Remove the credential a given-back run saved, and its directory if nothing else is in it.
/// A run that was given back will not be uploaded to, so its credential is of no use.
fn forget_credential(cfg: &Path, run_id: &str) {
    if !is_run_id(run_id) {
        return;
    }
    let dir = crate::run::run_dir(cfg, run_id);
    let _ = std::fs::remove_file(crate::account::credential::credential_path(&dir));
    let _ = std::fs::remove_dir(&dir);
}

/// Give back a run an earlier start opened on `server_url` and never launched, if this machine
/// noted one. The credential that start saved in the run's directory is what asks. Best effort,
/// and silent: if the server cannot be asked now, the note stays for the next start; a note
/// with no credential beside it cannot be acted on and is dropped (the server ends such a run
/// at its deadline, or when the account starts another). A note for another server, or whose
/// credential has ended, is dropped with its credential.
pub fn give_back_pending(server_url: &str, cfg: &Path) {
    let path = pending_path(cfg);
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let Ok(pending) = serde_json::from_slice::<PendingOpen>(&bytes) else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    if !is_run_id(&pending.run_id) {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if pending.server_url != server_url.trim_end_matches('/') {
        // Another server: this one cannot give that run back, and the credential kept for it
        // is of no use to anyone.
        let _ = std::fs::remove_file(&path);
        forget_credential(cfg, &pending.run_id);
        return;
    }
    let dir = crate::run::run_dir(cfg, &pending.run_id);
    let Ok(credential) = crate::account::credential::load(&dir) else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    // The server refuses a credential after its end; asking again every start would never
    // succeed, so the note and the credential go.
    if goalcup_protocol::control::parse_z(&credential.expires_at)
        .is_some_and(|ends| ends <= crate::run::now_unix())
    {
        let _ = std::fs::remove_file(&path);
        forget_credential(cfg, &pending.run_id);
        return;
    }
    let server = ServerClient::with_credential(server_url, &credential);
    let given = abandon_run(
        &server,
        &pending.run_id,
        "an earlier goalcup start ended before the launch",
    );
    if settled(&given) {
        let _ = std::fs::remove_file(&path);
        forget_credential(cfg, &pending.run_id);
    }
}

/// Wait for the harness to exit, and when `stop` says the run must stop first (its deadline, or
/// a full disk), call `on_stop` with the reason while the wait goes on. `on_stop` asks the daemon
/// to seal, which ends the harness (the daemon is the only terminator, D22), and so ends this
/// wait. `stop` is the run's one stop channel: whoever has a reason to end the run sends it there.
pub fn wait_stopping<F: FnOnce(&str) + Send>(
    child: &mut std::process::Child,
    stop: std::sync::mpsc::Receiver<String>,
    on_stop: F,
) -> std::io::Result<std::process::ExitStatus> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::RecvTimeoutError;
    let exited = AtomicBool::new(false);
    let exited = &exited;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            while !exited.load(Ordering::SeqCst) {
                match stop.recv_timeout(Duration::from_millis(200)) {
                    Ok(reason) => return on_stop(&reason),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        let status = child.wait();
        exited.store(true, Ordering::SeqCst);
        status
    })
}
