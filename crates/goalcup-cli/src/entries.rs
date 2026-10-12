//! The entry cache: what one `goalcup start` leaves behind for the processes that come
//! after it.
//!
//! `goalcup start` is a process that exits. `goalcup status`, `goalcup seal`, `goalcup
//! resume` and `goalcup-mcp` are fresh processes with no memory of it, and the one thing
//! they cannot do without is the run public key the participant pinned from
//! `WorkspaceInfo.run_public_key` at creation. Without this file that key lives only in the
//! memory of a process that has already gone, and every later command would have to trust
//! the key named **inside the document it is checking** — which is the one thing D25
//! forbids. So the key is written down here, once, at the moment it was handed over, and a
//! later command verifies against this record and never against `SignedStatus.public_key`.
//!
//! # It is not a second registry
//!
//! The daemon owns the entries. This is the participant's own bookkeeping: the handful of
//! strings a later command on **this machine** needs before it can ask the daemon anything —
//! which entry was last started, where its workspace is, which connector it was, and the
//! three values a resume resolves a lock path from (`harness_home`, `session_id`,
//! `last_pid`). Nothing here is authoritative about the run; everything here is a pin taken
//! at creation so a later process can tell whether the answer it got is about the entry it
//! asked for.
//!
//! # It holds no secret, and is written like one
//!
//! There is no token and no grant in an [`EntryRecord`] — a status URL and a public key are
//! not secrets, and `tests/it/start_launch.rs` greps the written bytes for the grant token
//! to keep it that way. It still goes through [`crate::config::write_secret_file`] at
//! `0600`, under a directory at `0700`: it names this participant's runs and where on their
//! disk the work is, and the rule for a file this CLI writes into the tracker's own
//! configuration directory is one rule, not one per sensitivity judgement.

use crate::CliError;
use crate::config::write_secret_file;
use goalcup_protocol::process::ProcessRegistration;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `<config_dir>/entries` — one JSON document per entry this machine started.
pub const ENTRIES_DIR_NAME: &str = "entries";

/// A local CLI's durable proof that it may recover the `/resume` request it was about to
/// send.  It is deliberately only an expected daemon generation: a random nonce would have
/// to be issued, persisted and authenticated by the daemon, while the daemon already makes
/// generation transitions one-winner.  The per-entry OS lock says which local CLI owns this
/// record while it is alive; after a crash its successor may cancel only the next generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRecoveryIntent {
    pub expected_generation: u64,
}

/// Which daemon authority, if any, must be settled before a rejected process cleanup can
/// complete. Missing on the short-lived earlier cache shape means `Resume`: that shape only ever
/// wrote cleanup markers while retiring a resumed registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessCleanupOrigin {
    Initial,
    Resume,
}

/// A group-less native-pid launch whose wrapper was observed but whose manifest-pinned
/// native process was not. No later process may turn the binary name into signal authority:
/// this marker is a manual-cleanup latch until an exact handoff is acknowledged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedNativeCleanup {
    pub origin: ProcessCleanupOrigin,
    pub native_binary: String,
}

/// What `goalcup start` pinned, and what every later process on this machine reads.
///
/// `Serialize`/`Deserialize` are the file format: this struct **is**
/// `<config_dir>/entries/<entry_id>.json`, and a field added later is added here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRecord {
    pub entry_id: String,
    /// The exact daemon-issued generation a later `resume` must compare-and-swap. Missing
    /// pre-v1 cache records read as zero, which can only exercise started-legacy recovery.
    #[serde(default)]
    pub process_generation: u64,
    /// A `/process` handoff whose response was transport-ambiguous after at least one
    /// attempt.  It is retained only in this private 0600 cache so a later invocation can
    /// replay the same canonical body; it is never a second source of authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_process_registration: Option<ProcessRegistration>,
    /// The opening that issued the replayable body. Only Resume has a daemon permit that
    /// rejected-registration cleanup may cancel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_process_registration_origin: Option<ProcessCleanupOrigin>,
    /// A process body the daemon has explicitly proven it refused before mutation.  It is
    /// no longer replayable, but its exact instance identity remains durable until this
    /// machine verifies the rejected child stopped.  Keeping cleanup separate prevents a
    /// crash from turning a rejected body back into a registration retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_process_cleanup: Option<ProcessRegistration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_process_cleanup_origin: Option<ProcessCleanupOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_unresolved_native_cleanup: Option<UnresolvedNativeCleanup>,
    /// Written before a non-preview `/resume` request.  It survives a local crash long
    /// enough for the next lock holder to tell its own interrupted request from another
    /// caller's open permit; it is cleared only after a preview proves no permit is open,
    /// cancellation, or process registration acknowledgement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_recovery: Option<ResumeRecoveryIntent>,
    pub connector_id: String,
    /// Pinned at creation from `WorkspaceInfo.run_public_key`. Every later command verifies
    /// against this, never against `SignedStatus.public_key`.
    pub run_public_key: String,
    pub workspace: String,
    pub artifact_root: String,
    /// `harness_home`, `session_id` and `last_pid` are here because the V1 `goalcup resume`
    /// (removed by D48) ran `check_locks` before it posted the resume: the daemon refuses a
    /// held lock itself and returns an error carrying no findings, so the CLI had to be able
    /// to resolve `{config_dir}`/`{home}`/`{workspace}` and ask whether the holder was still
    /// alive from its own record. None of the three is a secret.
    pub harness_home: String,
    pub session_id: String,
    /// The pid of the last `ProcessHandle` this CLI registered — written after
    /// `register_process` by `goalcup start` (and, in V1, rewritten by `goalcup resume` after
    /// its own). `None` until something has been spawned.
    pub last_pid: Option<u32>,
    /// The daemon's own list, kept so `goalcup seal` can build the exclusion set after
    /// `start` has exited.
    pub workspace_relative: Vec<String>,
    pub status_url: String,
    /// The daemon's stamp on `WorkspaceInfo.created_at`, carried across unchanged. This CLI
    /// reads no clock for it: two records are ordered by the one clock that made them.
    pub created_at: String,
    /// This run was started with `goalcup start --dev`.
    ///
    /// The one thing a later process cannot re-derive: `--dev` is a flag on a command that
    /// has exited, and `goalcup seal` prints a `goalcup-verify` line the participant is
    /// meant to paste. A development run's trace names a connector manifest this build does
    /// not pin, and `goalcup-verify` fails it outright without `--dev` — so a seal that
    /// printed the bare command would hand exactly the wrong participant a line that cannot
    /// work. It is remembered here rather than guessed from the trace, because the trace is
    /// the document being checked and a flag read out of it would be the document choosing
    /// how strictly it is read.
    ///
    /// `serde(default)` so a record written before this field existed still reads, as
    /// `false` — which is the safe reading: a run this machine has no `--dev` note for gets
    /// the strict verify line.
    #[serde(default)]
    pub dev: bool,
}

/// `<cfg>/entries`.
pub fn entries_dir(cfg: &Path) -> PathBuf {
    cfg.join(ENTRIES_DIR_NAME)
}

/// `<cfg>/entries/<entry_id>.json`.
///
/// A plain join, so a caller that has an id can name the file without a `Result` in the way.
/// The id is checked where it is *used* — [`write_entry`], [`read_entry`] and
/// [`remove_entry`] all go through [`one_path_component`] first — because a check that only
/// this function made would be a check a caller could skip by joining the path themselves.
pub fn entry_path(cfg: &Path, entry_id: &str) -> PathBuf {
    entries_dir(cfg).join(format!("{entry_id}.json"))
}

/// The record, written at `0600` under a directory at `0700`.
pub fn write_entry(cfg: &Path, rec: &EntryRecord) -> Result<(), CliError> {
    one_path_component(&rec.entry_id)?;
    let bytes = serde_json::to_vec_pretty(rec)?;
    write_secret_file(&entry_path(cfg, &rec.entry_id), &bytes)
}

/// The record this machine holds for `entry_id`.
///
/// A missing file is a refusal the participant can act on — rung 1, naming the path — and
/// not this CLI failing: "this machine has no record of that entry" is an answer `goalcup
/// status` and `goalcup seal` both came for, and the fix is to name an entry this machine
/// started.
pub fn read_entry(cfg: &Path, entry_id: &str) -> Result<EntryRecord, CliError> {
    one_path_component(entry_id)?;
    let path = entry_path(cfg, entry_id);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Err(CliError::Refused(format!(
                "this machine has no record of entry `{entry_id}`: there is nothing at {}. \
                 `goalcup entries` lists the entries the daemon knows about, and only a run \
                 started from this machine leaves a record here.",
                path.display()
            )))
        }
        Err(source) => Err(CliError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// The newest record by `created_at`, for the commands that take no `<entry>`.
///
/// A configuration directory with no `entries/` in it is `None` and not an error: a machine
/// that has never started a run is the ordinary case for `goalcup status` on a fresh
/// install. A file in there that does not parse is skipped rather than fatal, for the
/// reason `connector::load` skips a neighbour that does not parse: one unreadable record is
/// not a reason the participant cannot reach the other four.
pub fn latest_entry(cfg: &Path) -> Result<Option<EntryRecord>, CliError> {
    let dir = entries_dir(cfg);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(CliError::Io {
                path: dir.display().to_string(),
                source,
            });
        }
    };
    let mut newest: Option<EntryRecord> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(rec) = serde_json::from_str::<EntryRecord>(&text) else {
            continue;
        };
        // RFC-3339 UTC, which the whole tree spells one way (`control::format_z`), so the
        // lexical order is the chronological one.
        let newer = match &newest {
            Some(held) => rec.created_at > held.created_at,
            None => true,
        };
        if newer {
            newest = Some(rec);
        }
    }
    Ok(newest)
}

/// Remove the record for `entry_id`. Removing one that is not there is not a failure.
///
/// This is the other half of `abandon`: the daemon deletes the entry and its artifact
/// directory, and a record left behind here would be a pin every later command refuses on,
/// for a run that no longer exists.
pub fn remove_entry(cfg: &Path, entry_id: &str) -> Result<(), CliError> {
    one_path_component(entry_id)?;
    let path = entry_path(cfg, entry_id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(CliError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// An entry id is one path component, or it does not name a file in this directory.
///
/// The id comes from the daemon, over the loopback, and is interpolated into a path. That is
/// the shape that has to be checked rather than trusted: `..` in it would write outside the
/// tracker's own configuration directory, and a separator in it would write into a directory
/// nothing here created. Refused rather than sanitised — an id this CLI quietly rewrote
/// would be a record the next command looks for under the id it was actually given.
fn one_path_component(entry_id: &str) -> Result<(), CliError> {
    let bad = entry_id.is_empty()
        || entry_id == "."
        || entry_id == ".."
        || entry_id.contains('/')
        || entry_id.contains('\\')
        || entry_id.contains('\0');
    if bad {
        return Err(CliError::Refused(format!(
            "`{entry_id}` does not name an entry: an entry id is one path component, and \
             this one would name a file outside the tracker's own entries directory."
        )));
    }
    Ok(())
}
