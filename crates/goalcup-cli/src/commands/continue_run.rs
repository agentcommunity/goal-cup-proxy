//! `goalcup continue [RUN_ID]` — restart a run's agent as its next segment.
//!
//! A run lasts 168 hours on the server's clock, and its agent may end long before that: the
//! participant closes the window, the agent crashes, the machine reboots. That ends one
//! *segment* (one entry): `goalcup start` sealed it, sent its final heartbeat and said the run
//! stays open. This command opens the next one, on the same machine, with what the run kept:
//!
//! - the run's saved credential (`<cfg>/runs/<run_id>/credential`). There is no browser and no
//!   approval: a run stays on the machine that started it, and a run with no record or no
//!   credential here is refused, whoever asks;
//! - the run's clone as the workspace, the harness started with its `args` followed by its
//!   `resume_args`, and the run's one disk reservation (a reserve file is not taken again).
//!
//! In this order, each step settled before the next: the run and its credential are found; the
//! shared harness home is locked (another run recording there is refused, by name, before
//! anything is changed); the previous segment is closed, by the tracker's seal if the daemon
//! still holds it, otherwise from the tail of its chain, an incomplete segment that says so, with
//! no repair of a torn tail; the previous segment is sent on a thread of its own; the server is
//! asked for the next segment; the harness is launched; and the segment is appended to the run's
//! record. From there the segment runs and ends as any run does ([`super::start`]).

use crate::CliError;
use crate::commands::start::{
    Continuing, SleepLockFn, StartOptions, continue_segment, lock_home_for_continue,
    run_folder_for_continue, still_recording,
};
use crate::connector::HarnessConnector;
use crate::entries::{EntryRecord, read_entry};
use crate::retry::retry_busy;
use crate::run::reserve::DiskPolicy;
use crate::run::{HarnessState, RunRecord};
use crate::upload::UploadRecord;
use crate::upload::http::ServerClient;
use crate::upload::policy::WrapUp;
use crate::upload::uploader::{Closing, Policy, drain, lock_entry, note_repo_head};
use goalcup_protocol::control::DaemonClient;
use goalcup_protocol::process::termination_bound;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What one `goalcup continue` was asked to do.
pub struct ContinueOptions {
    /// The run to continue; `None` is the run most recently started on this machine.
    pub run: Option<String>,
    /// Where the segment's own folder is made: the directory `goalcup continue` was run from,
    /// which `main.rs` reads. `None` reads this process's current directory.
    pub run_folder: Option<PathBuf>,
    /// The sizes of the run's disk reservation and how often it is watched
    /// ([`DiskPolicy::default`] in every shipped build).
    pub disk: DiskPolicy,
    /// When the run's chunks go and how failures wait.
    pub upload_policy: Policy,
    /// How the machine is kept awake for the segment's life.
    pub sleep_lock: SleepLockFn,
}

/// The run `run` names (else the newest this machine started) and the credential it saved here,
/// or the one refusal for a run this machine did not start. Nothing is started or changed.
pub fn resolve(
    cfg: &Path,
    run: Option<&str>,
) -> Result<(RunRecord, crate::account::credential::RunCredential), CliError> {
    let not_here = || {
        CliError::Refused(format!(
            "this run was not started on this machine: there is no record of {} under {}, and a \
             run can be continued only on the machine that started it. Nothing was changed.",
            run.map_or_else(|| "any run".to_string(), |id| format!("run {id}")),
            crate::run::runs_dir(cfg).display()
        ))
    };
    let record = match run {
        Some(id) => crate::run::read_if_any(cfg, id)?,
        None => crate::run::newest(cfg)?,
    }
    .ok_or_else(not_here)?;
    if record.server_url.is_none() {
        return Err(CliError::Refused(format!(
            "run {} was recorded on this machine only (it was never opened on the Goal Cup \
             server), so it has no segments to continue. Start a new run with `goalcup start`.",
            record.run_id
        )));
    }
    let run_dir = crate::run::run_dir(cfg, &record.run_id);
    if !crate::account::credential::credential_path(&run_dir).exists() {
        return Err(not_here());
    }
    let credential = crate::account::credential::load(&run_dir)?;
    Ok((record, credential))
}

/// The testable half: everything except resolving a real client and a real config dir.
pub fn continue_with(
    client: &(dyn DaemonClient + Sync),
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &ContinueOptions,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let (record, credential) = resolve(cfg, opts.run.as_deref())?;
    let run_id = record.run_id.clone();
    let server_url = record.server_url.clone().unwrap_or_default();
    let previous = record.entries.last().ok_or_else(|| {
        CliError::Refused(format!(
            "run {run_id} has no entry on this machine to follow. Nothing was changed."
        ))
    })?;
    let previous_entry = read_entry(cfg, &previous.entry_id)?;
    let connector =
        crate::connector::load_harness(connectors, &record.connector_id, previous_entry.dev)?;
    let folder = run_folder_for_continue(opts.run_folder.as_deref(), &record.workspace)?;

    // 1. Another run recording on the shared harness home is refused, by name, before anything
    //    is closed or changed. Its host holds the lock until it has sealed.
    let home_lock = lock_home_for_continue(cfg, &connector, &run_id)?;

    // 2. The previous segment: its host may still be sending it (it is sealed, and nothing is
    //    left to close), or may be gone (a killed host, a reboot).
    let spool = cfg.join(crate::upload::UPLOADS_DIR_NAME);
    let upload_record = crate::upload::read_record(cfg, &previous.entry_id)?.ok_or_else(|| {
        CliError::Refused(format!(
            "entry {} of run {run_id} has no upload record on this machine. Nothing was \
             changed.",
            previous.entry_id
        ))
    })?;
    let receipt: goalcup_protocol::receipt::StartReceipt = {
        let path = crate::upload::receipt_path(cfg, &previous.entry_id);
        serde_json::from_slice(&std::fs::read(&path).map_err(|source| CliError::Io {
            path: path.display().to_string(),
            source,
        })?)?
    };
    let sender = match lock_entry(&spool, &previous.entry_id) {
        // Nobody sends it: this process closes it and sends it.
        Ok(lock) => {
            drop(lock);
            true
        }
        // Another goalcup sends it. If its agent still runs, the run is recording; if the agent
        // is gone, it is the host's wrap-up and needs nothing from here.
        Err(_) => {
            let running = crate::run::read_state(&crate::run::run_dir(cfg, &run_id))
                .is_some_and(|state| state.harness == HarnessState::Running && !state.ended);
            if running {
                return Err(CliError::Refused(still_recording(
                    &run_id,
                    &record.connector_id,
                )));
            }
            false
        }
    };
    let mut draining = Vec::new();
    if sender {
        let closing = close_previous(
            client,
            &connector,
            cfg,
            &previous_entry,
            &upload_record,
            &spool,
            out,
        );
        let server = ServerClient::with_credential(&server_url, &credential);
        draining.push(drain(
            server,
            upload_record,
            receipt.public_key,
            opts.upload_policy,
            spool,
            closing,
            WrapUp::after(opts.upload_policy.wrap_up),
        )?);
        say(
            out,
            &format!(
                "run {run_id}: the earlier segment (entry {}) is being sent in the background.",
                previous.entry_id
            ),
        )?;
    }

    // 3. The next segment, launched as a run is.
    let start = StartOptions {
        harness: record.connector_id.clone(),
        dev: previous_entry.dev,
        goal_file: None,
        run_folder: Some(folder),
        disk: opts.disk,
        practice: record.practice,
        open_url: |_| false,
        upload_policy: opts.upload_policy,
        sleep_lock: opts.sleep_lock,
    };
    let previous_entry_id = previous.entry_id.clone();
    continue_segment(
        client,
        connectors,
        cfg,
        env,
        &start,
        Continuing {
            record,
            credential,
            previous_entry_id,
            home_lock,
            draining,
        },
        out,
    )
    .map(|_| ())
}

/// End the previous segment on this machine, and say how it ended. If the daemon holds it open
/// (the host was killed, the machine rebooted and the daemon reopened it) it is sealed. If the
/// daemon cannot (its chain is torn, or it is not known), the segment is closed from the tail of
/// its chain, as an incomplete segment: its last whole record is the end, and nothing is
/// repaired.
fn close_previous(
    client: &(dyn DaemonClient + Sync),
    connector: &HarnessConnector,
    cfg: &Path,
    entry: &EntryRecord,
    upload: &UploadRecord,
    spool: &Path,
    out: &mut dyn std::io::Write,
) -> Closing {
    let entry_id = &entry.entry_id;
    let status = retry_busy(|| client.status(entry_id))
        .ok()
        .filter(|document| {
            document.verify_with(&entry.run_public_key).is_ok()
                && document.payload.entry_id == *entry_id
        });
    let Some(status) = status else {
        let _ = say(
            out,
            &format!(
                "entry {entry_id}: the tracker no longer holds it open, so it is closed from the \
                 end of its chain as an incomplete segment."
            ),
        );
        note_repo_head(spool, upload);
        return Closing::FromTail;
    };
    if status.payload.sealed_at.is_some() {
        // Its host sealed it, and noted the commit its workspace had then.
        return Closing::Sealed;
    }
    note_repo_head(spool, upload);
    if status.payload.capture.is_none() {
        return Closing::FromTail;
    }
    let allowance = termination_bound(connector.loaded.manifest.termination().grace_seconds());
    let deadline = std::time::Instant::now() + crate::finish_budget();
    match crate::commands::seal::seal_locally(
        client,
        entry_id,
        None,
        None,
        deadline,
        allowance,
        Some(cfg),
    ) {
        Ok(_) => {
            let _ = say(
                out,
                &format!("entry {entry_id}: its host had gone, so it was sealed now."),
            );
            Closing::Sealed
        }
        Err(error) => {
            let _ = say(
                out,
                &format!(
                    "entry {entry_id}: it could not be sealed ({error}), so it is closed from \
                     the end of its chain as an incomplete segment."
                ),
            );
            Closing::FromTail
        }
    }
}

/// One line to the participant, or the write error naming the stream.
fn say(out: &mut dyn std::io::Write, line: &str) -> Result<(), CliError> {
    writeln!(out, "{line}").map_err(|source| CliError::Io {
        path: "stdout".to_string(),
        source,
    })
}
