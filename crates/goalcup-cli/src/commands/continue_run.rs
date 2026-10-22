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
//!   `resume_args` when the run has a saved session (a file in one of the connector's native
//!   stores; otherwise with its `args` alone), and the run's one disk reservation (a reserve
//!   file is not taken again).
//!
//! In this order, each step settled before the next: the run and its credential are found; every
//! refusal that needs no server is made (the connector is a chat-mode one, the terminal is
//! there, the harness and its pin are usable); the shared harness home is locked (another run
//! recording there is refused, by name) and the run's reserve is looked at (a host still
//! holding it is refused); each earlier segment not yet sent is looked at, and anything the
//! tracker's daemon answers other than "not held" or "not open" is a refusal; the segments the
//! daemon still holds are sealed, and the ones it cannot hold are closed from the tail of their
//! chain, incomplete segments that say so, with no repair of a torn tail; each is sent on a
//! thread of its own; only then is the server asked for the next segment, the harness launched,
//! and the segment appended to the run's record. Nothing is sent to the server before every
//! refusal that can be made without it has been. From there the segment runs and ends as any
//! run does ([`super::start`]).

use crate::CliError;
use crate::commands::say;
use crate::commands::start::{
    Continuing, SleepLockFn, StartOptions, check_run_folders, continue_preflight, continue_segment,
    run_folder_for_continue,
};
use crate::connector::HarnessConnector;
use crate::entries::{EntryRecord, read_entry};
use crate::home::lock_home_for_continue;
use crate::retry::retry_busy;
use crate::run::reserve::{DiskPolicy, artifact_root, held_by_another, reserve_path};
use crate::run::{HarnessState, RunRecord};
use crate::upload::UploadRecord;
use crate::upload::http::ServerClient;
use crate::upload::policy::WrapUp;
use crate::upload::uploader::{
    Closing, Policy, drain, is_sent, lock_entry, note_repo_head_unless_noted,
};
use goalcup_protocol::control::{ControlError, DaemonClient};
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
    let not_here = || CliError::Refused(crate::participant::not_started_here(run));
    let record = match run {
        Some(id) => crate::run::read_if_any(cfg, id)?,
        None => crate::run::newest(cfg)?,
    }
    .ok_or_else(not_here)?;
    if record.server_url.is_none() {
        return Err(CliError::Refused(crate::participant::local_only(
            &record.run_id,
        )));
    }
    let run_dir = crate::run::run_dir(cfg, &record.run_id);
    if !crate::account::credential::credential_path(&run_dir).exists() {
        return Err(not_here());
    }
    let credential = crate::account::credential::load(&run_dir)?;
    Ok((record, credential))
}

/// How an earlier segment is to be closed, settled by looking at the daemon and changing
/// nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum How {
    /// Its host sealed it.
    Sealed,
    /// The daemon holds it open: it is sealed now.
    SealIt,
    /// The daemon does not hold it (it has no such entry, or the entry is not open): it ends at
    /// the last whole record of its chain.
    FromTail,
}

/// An earlier segment of the run whose end is still to be sent.
struct Earlier {
    entry: EntryRecord,
    upload: UploadRecord,
    receipt_key: String,
    how: How,
}

/// The refusal for an earlier segment that cannot be closed: before anything is sent.
fn unsealable(entry_id: &str, why: &str) -> CliError {
    CliError::Refused(format!(
        "Run goalcup seal {entry_id}, then goalcup continue. The earlier part of the run (part \
         {entry_id}) could not be sealed ({why}). Nothing was sent."
    ))
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
    // The run may still be open, so a new start (which would replace it) is not advised.
    let previous = record.entries.last().ok_or_else(|| {
        CliError::Refused(format!(
            "Write to {} and name run {run_id}. It has nothing recorded on this computer to go \
             on from, so it cannot be continued here. Nothing was changed.",
            crate::participant::SUPPORT_ADDRESS
        ))
    })?;
    let previous_entry = read_entry(cfg, &previous.entry_id)?;
    let connector =
        crate::connector::load_harness(connectors, &record.connector_id, previous_entry.dev)?;
    let folder = run_folder_for_continue(opts.run_folder.as_deref(), &record.workspace)?;
    // WSL1 cannot run this, and WSL2 cannot keep a run on a Windows drive: judged here, over every
    // folder the segment writes to, before the home is locked or anything is closed or changed.
    // A folder on a filesystem a run cannot keep its records on is only warned about.
    check_run_folders(
        env,
        cfg,
        &folder,
        record.workspace.parent(),
        "Nothing was changed.",
    )?;

    // 0. Every refusal that needs no server and changes nothing, before the home is locked and
    //    before any segment is sealed or sent.
    continue_preflight(cfg, &connector, previous_entry.dev, env)?;

    // 1. Another run recording on the shared harness home is refused, by name, before anything
    //    is closed or changed. Its host holds the lock until it has sealed.
    let home_lock = lock_home_for_continue(cfg, &connector, &run_id)?;

    // The run's one reserve is handed over at a segment's seal. One still held is a host that
    // has not got that far, or a run that is not what it looks like: refused here, because past
    // `POST continue` the server's new segment cannot be taken back.
    if held_by_another(&reserve_path(&artifact_root(cfg), &run_id)) {
        return Err(CliError::Refused(crate::participant::last_part_closing(
            &run_id,
        )));
    }

    // 2. The earlier segments: some host may still be sending one (its wrap-up), or it may be
    //    gone (a killed host, a reboot).
    let spool = cfg.join(crate::upload::UPLOADS_DIR_NAME);
    let mut earlier = Vec::new();
    for (index, reference) in record.entries.iter().enumerate() {
        let last = index + 1 == record.entries.len();
        let entry_id = &reference.entry_id;
        let Some(upload) = crate::upload::read_record(cfg, entry_id)? else {
            if last {
                return Err(CliError::Refused(format!(
                    "Write to {} and name run {run_id}. Its last part ({entry_id}) has no \
                     upload record on this computer, so it cannot be continued here. Nothing \
                     was changed.",
                    crate::participant::SUPPORT_ADDRESS
                )));
            }
            continue;
        };
        // Sent and sealed (or closed from its tail and sent) already: nothing is left of it.
        if is_sent(&spool, entry_id) {
            continue;
        }
        match lock_entry(&spool, entry_id) {
            // Nobody sends it: this process closes it and sends it.
            Ok(lock) => drop(lock),
            // Another goalcup sends it. If its agent still runs, the run is recording; if the
            // agent is gone, it is the host's wrap-up and needs nothing from here.
            Err(_) => {
                let running = last
                    && crate::run::read_state(&crate::run::run_dir(cfg, &run_id)).is_some_and(
                        |state| state.harness == HarnessState::Running && !state.ended,
                    );
                if running {
                    return Err(CliError::Refused(crate::participant::still_recording(
                        connector.display_name(),
                        &run_id,
                    )));
                }
                continue;
            }
        }
        let receipt = crate::upload::read_receipt(cfg, entry_id)?;
        let entry = read_entry(cfg, entry_id)?;
        let how = how_to_close(client, &entry)?;
        earlier.push(Earlier {
            entry,
            upload,
            receipt_key: receipt.public_key,
            how,
        });
    }

    // 3. Close them. A seal that does not take is a refusal, before the server is asked
    //    anything: the daemon is waited for, never read as gone.
    let mut closings = Vec::new();
    for segment in &earlier {
        closings.push(close_previous(
            client, &connector, cfg, segment, &spool, out,
        )?);
    }

    // 4. Send them, each on a thread of its own, beside the segment that follows.
    let mut draining = Vec::new();
    for (segment, closing) in earlier.into_iter().zip(closings) {
        let entry_id = segment.entry.entry_id.clone();
        let server = ServerClient::with_credential(&server_url, &credential);
        match drain(
            server,
            segment.upload,
            segment.receipt_key,
            opts.upload_policy,
            spool.clone(),
            closing,
            WrapUp::after(opts.upload_policy.wrap_up),
        ) {
            Ok(started) => {
                draining.push(started);
                say(out, crate::participant::EARLIER_PART_UPLOADING)?;
            }
            // Another goalcup began to send it since it was looked at: it needs nothing here.
            Err(error) => log_line(cfg, &entry_id, &format!("part {entry_id}: {error}")),
        }
    }

    // 5. The next segment, launched as a run is.
    let start = StartOptions {
        harness: record.connector_id.clone(),
        dev: previous_entry.dev,
        run_folder: Some(folder),
        disk: opts.disk,
        mode: record.mode(),
        open_url: |_| false,
        upload_policy: opts.upload_policy,
        sleep_lock: opts.sleep_lock,
    };
    let previous_entry_id = record
        .entries
        .last()
        .map(|entry| entry.entry_id.clone())
        .unwrap_or_default();
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

/// Look at what the daemon says of an earlier segment, and change nothing. It is not held when
/// the daemon has no such entry, or the entry is not open there; any other answer (the daemon
/// unreachable or failing, or an answer that does not verify) is not "not held" and refuses.
fn how_to_close(client: &(dyn DaemonClient + Sync), entry: &EntryRecord) -> Result<How, CliError> {
    let entry_id = &entry.entry_id;
    let status = match retry_busy(|| client.status(entry_id)) {
        Ok(status) => status,
        Err(ControlError::NotFound(_)) => return Ok(How::FromTail),
        Err(error) => return Err(unsealable(entry_id, &error.to_string())),
    };
    if status.verify_with(&entry.run_public_key).is_err() || status.payload.entry_id != *entry_id {
        return Err(unsealable(
            entry_id,
            "goalcup's recorder gave an answer for it that did not verify",
        ));
    }
    Ok(if status.payload.sealed_at.is_some() {
        // Its host sealed it, and noted the commit its workspace had then.
        How::Sealed
    } else if status.payload.capture.is_none() {
        How::FromTail
    } else {
        How::SealIt
    })
}

/// End an earlier segment on this machine, and say how it ended. A segment the daemon holds
/// open (the host was killed, the machine rebooted and the daemon reopened it) is sealed. One
/// the daemon cannot hold (its chain is torn, or it is not known) is closed from the tail of
/// its chain, as an incomplete segment: its last whole record is the end, and nothing is
/// repaired.
fn close_previous(
    client: &(dyn DaemonClient + Sync),
    connector: &HarnessConnector,
    cfg: &Path,
    segment: &Earlier,
    spool: &Path,
    out: &mut dyn std::io::Write,
) -> Result<Closing, CliError> {
    let entry_id = &segment.entry.entry_id;
    match segment.how {
        // Its host sealed it and noted the commit its workspace had then; one that did not (the
        // host died after the seal, or `goalcup seal` sealed it) is noted now.
        How::Sealed => {
            note_repo_head_unless_noted(spool, &segment.upload);
            Ok(Closing::Sealed)
        }
        How::FromTail => {
            note_repo_head_unless_noted(spool, &segment.upload);
            log_line(
                cfg,
                entry_id,
                &format!(
                    "part {entry_id}: the recorder no longer holds it open, so it is closed from \
                     the end of its chain as an incomplete part."
                ),
            );
            let _ = say(
                out,
                "The earlier part of this run ended without a clean seal. What was recorded is \
                 kept and uploads now. Nothing to do.",
            );
            Ok(Closing::FromTail)
        }
        How::SealIt => {
            note_repo_head_unless_noted(spool, &segment.upload);
            let allowance =
                termination_bound(connector.loaded.manifest.termination().grace_seconds());
            let deadline = std::time::Instant::now() + crate::finish_budget();
            crate::commands::seal::seal_locally(
                client,
                entry_id,
                None,
                deadline,
                allowance,
                Some(cfg),
            )
            .map_err(|error| unsealable(entry_id, &error.to_string()))?;
            log_line(
                cfg,
                entry_id,
                &format!("part {entry_id}: its host had gone, so it was sealed now."),
            );
            let _ = say(
                out,
                "The earlier part of this run was still open, so it was sealed now.",
            );
            Ok(Closing::Sealed)
        }
    }
}

/// One technical line in the `host.log` of the run that holds `entry_id`, for support: what a
/// participant need not read. Nothing happens when the log cannot be written.
fn log_line(cfg: &Path, entry_id: &str, line: &str) {
    let Ok(Some(run)) = crate::run::of_entry(cfg, entry_id) else {
        return;
    };
    let log = crate::run::run_dir(cfg, &run.run_id).join(crate::host_log::HOST_LOG_NAME);
    if let Ok(mut file) = crate::host_log::open_append(&log) {
        let _ = std::io::Write::write_all(&mut file, format!("goalcup: {line}\n").as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::MockDaemonClient;
    use goalcup_protocol::control::ControlError;

    /// The mock's one entry, as this machine's record of it.
    fn entry_of(mock: &MockDaemonClient) -> EntryRecord {
        let key = MockDaemonClient::signing_key();
        EntryRecord {
            entry_id: mock.status("e").unwrap().payload.entry_id,
            process_generation: 1,
            connector_id: "fake".to_string(),
            run_public_key: key
                .verifying_key()
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            workspace: String::new(),
            artifact_root: String::new(),
            harness_home: String::new(),
            session_id: String::new(),
            last_pid: None,
            created_at: String::new(),
            dev: true,
        }
    }

    /// Only "the daemon has no such entry" lets go of an earlier segment; an open one is
    /// sealed, and a daemon that failed to answer, or answered what does not verify, is not a
    /// daemon that does not hold it.
    #[test]
    fn only_a_daemon_that_does_not_hold_the_segment_lets_it_close_from_its_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let mock = MockDaemonClient::armed_ok(tmp.path(), "fake");
        let entry = entry_of(&mock);
        // Open there (the capture status the daemon sends): seal it.
        assert!(matches!(how_to_close(&mock, &entry), Ok(How::SealIt)));
        // Not known there: from the tail.
        mock.fail_always("status", ControlError::NotFound("no such entry".into()));
        assert!(matches!(how_to_close(&mock, &entry), Ok(How::FromTail)));
        // Anything else is a refusal that says what to do, and that nothing was sent.
        for error in [
            ControlError::Internal("the registry is damaged".into()),
            ControlError::Unreachable("connection refused".into()),
            ControlError::Malformed("not json".into()),
            ControlError::Unauthorized("bad token".into()),
        ] {
            let shown = error.to_string();
            mock.fail_always("status", error);
            let refused = how_to_close(&mock, &entry).err().expect("a refusal");
            let said = refused.to_string();
            assert!(said.contains("could not be sealed"), "{said}");
            assert!(said.contains(&shown), "{said}");
            assert!(
                said.contains(&format!("goalcup seal {}", entry.entry_id)),
                "{said}"
            );
            assert!(said.contains("Nothing was sent"), "{said}");
        }
    }

    /// An answer signed by another key is not the daemon's answer for this entry.
    #[test]
    fn an_answer_that_does_not_verify_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let mock = MockDaemonClient::armed_ok(tmp.path(), "fake");
        let mut entry = entry_of(&mock);
        entry.run_public_key = "00".repeat(32);
        let said = how_to_close(&mock, &entry)
            .err()
            .expect("a refusal")
            .to_string();
        assert!(said.contains("did not verify"), "{said}");
    }
}
