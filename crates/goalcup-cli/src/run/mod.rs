//! A run on this machine: the record of it, its disk reservation, and the tick that watches
//! the disk while it records.
//!
//! One `goalcup start` hosts one run. Its record is `<cfg>/runs/<run_id>/run.json` ([`RunRecord`]),
//! written when the daemon has created the entry and read by the commands that come after it.
//! A run opened on a server uses the server's run id; a local run uses its entry id.
//!
//! [`reserve`] holds the run's one disk reservation and the rules for resizing it, [`watch`] is
//! the 30-second tick that applies them.

pub mod reserve;
pub mod watch;

use crate::CliError;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

/// `<cfg>/runs`: one directory per run.
pub const RUNS_DIR_NAME: &str = "runs";

/// The run's record inside its directory.
pub const RUN_FILE_NAME: &str = "run.json";

/// The most of a run record this build reads: it is a few hundred bytes, plus one line per
/// segment of the run.
const RUN_FILE_MAX_BYTES: u64 = 1024 * 1024;

/// One entry (one recording segment) of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRef {
    pub entry_id: String,
    /// The entry's trace directory, holding its chain.
    pub trace_dir: PathBuf,
}

/// What this machine knows of a run: `<cfg>/runs/<run_id>/run.json`.
///
/// Only what is known when the run starts is here; later steps add what they bring (the run
/// credential, the clone of the repository, a segment per `continue`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    /// The server's run id for a run opened on a server, the entry id for a local one.
    pub run_id: String,
    /// The upload server the run was opened on; `None` for a local run.
    pub server_url: Option<String>,
    /// A practice run (not a competition entry). No run is a practice run yet.
    pub practice: bool,
    pub connector_id: String,
    pub workspace: PathBuf,
    /// When `goalcup start` made the run, on this machine's clock.
    pub started_at_unix: u64,
    /// The run's deadline as the server stamped it (`...Z`); `None` for a local run, which has
    /// none.
    pub deadline_at: Option<String>,
    pub entries: Vec<EntryRef>,
}

/// What a run is doing now, kept at `<run_dir>/state.json`. The watch tick writes it and
/// `goalcup status` reads it; it has no fields yet, and the task that adds them fills this in.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunState {}

/// `<cfg>/runs`.
pub fn runs_dir(cfg: &Path) -> PathBuf {
    cfg.join(RUNS_DIR_NAME)
}

/// `<cfg>/runs/<run_id>`.
pub fn run_dir(cfg: &Path, run_id: &str) -> PathBuf {
    runs_dir(cfg).join(run_id)
}

/// `<cfg>/runs/<run_id>/state.json`.
pub fn state_path(cfg: &Path, run_id: &str) -> PathBuf {
    run_dir(cfg, run_id).join("state.json")
}

/// Whether `id` can be a directory name here: ids come from the server and the daemon, and a
/// path separator or a dot must not make one a path.
fn is_safe_name(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn checked_name(run_id: &str) -> Result<(), CliError> {
    if is_safe_name(run_id) {
        Ok(())
    } else {
        Err(CliError::Internal(format!(
            "`{run_id}` cannot name a run directory"
        )))
    }
}

/// Write `record` to its `run.json` (mode 0600, replacing any earlier copy whole).
pub fn write(cfg: &Path, record: &RunRecord) -> Result<(), CliError> {
    checked_name(&record.run_id)?;
    let mut text = serde_json::to_vec_pretty(record)?;
    text.push(b'\n');
    crate::config::write_secret_file(&run_dir(cfg, &record.run_id).join(RUN_FILE_NAME), &text)
}

/// The record of `run_id`, or `None` when this machine has none.
pub(crate) fn read_if_any(cfg: &Path, run_id: &str) -> Result<Option<RunRecord>, CliError> {
    if !is_safe_name(run_id) {
        return Ok(None);
    }
    let path = run_dir(cfg, run_id).join(RUN_FILE_NAME);
    let io = |source| CliError::Io {
        path: path.display().to_string(),
        source,
    };
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io(source)),
    };
    let mut text = Vec::new();
    file.take(RUN_FILE_MAX_BYTES + 1)
        .read_to_end(&mut text)
        .map_err(io)?;
    if text.len() as u64 > RUN_FILE_MAX_BYTES {
        return Err(CliError::Refused(format!(
            "{} is larger than {RUN_FILE_MAX_BYTES} bytes, which is more than any run record this \
             build writes.",
            path.display()
        )));
    }
    Ok(Some(serde_json::from_slice(&text)?))
}

/// The record of `run_id`; a refusal naming where it would be when this machine has none.
pub fn read(cfg: &Path, run_id: &str) -> Result<RunRecord, CliError> {
    read_if_any(cfg, run_id)?.ok_or_else(|| {
        CliError::Refused(format!(
            "no run `{run_id}` was started from this machine: there is no {} under {}.",
            RUN_FILE_NAME,
            runs_dir(cfg).display()
        ))
    })
}

/// Every run record under `<cfg>/runs`, in no order. A directory without a readable record is
/// skipped: it is not a run this build can say anything about.
fn all(cfg: &Path) -> Vec<RunRecord> {
    let Ok(dirs) = std::fs::read_dir(runs_dir(cfg)) else {
        return Vec::new();
    };
    dirs.flatten()
        .filter_map(|dir| dir.file_name().to_str().map(str::to_owned))
        .filter_map(|name| read_if_any(cfg, &name).ok().flatten())
        .collect()
}

/// The run most recently started from this machine, if any.
pub fn newest(cfg: &Path) -> Result<Option<RunRecord>, CliError> {
    Ok(all(cfg)
        .into_iter()
        .max_by(|a, b| (a.started_at_unix, &a.run_id).cmp(&(b.started_at_unix, &b.run_id))))
}

/// The run that holds `entry_id`: the run of that id (a local run), or the one whose segments
/// name it.
pub fn of_entry(cfg: &Path, entry_id: &str) -> Result<Option<RunRecord>, CliError> {
    if let Some(record) = read_if_any(cfg, entry_id)? {
        return Ok(Some(record));
    }
    Ok(all(cfg)
        .into_iter()
        .find(|record| record.entries.iter().any(|e| e.entry_id == entry_id)))
}

/// Remove a run's directory (its record and anything beside it). Missing is fine.
pub fn remove(cfg: &Path, run_id: &str) {
    if is_safe_name(run_id) {
        let _ = std::fs::remove_dir_all(run_dir(cfg, run_id));
    }
}

/// Seconds since the epoch on this machine's clock.
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(run_id: &str, started: u64, entry: &str) -> RunRecord {
        RunRecord {
            run_id: run_id.to_string(),
            server_url: None,
            practice: false,
            connector_id: "c".to_string(),
            workspace: PathBuf::from("/w"),
            started_at_unix: started,
            deadline_at: None,
            entries: vec![EntryRef {
                entry_id: entry.to_string(),
                trace_dir: PathBuf::from("/t"),
            }],
        }
    }

    #[test]
    fn a_record_is_read_back_and_found_by_run_newest_and_entry() {
        let cfg = tempfile::tempdir().unwrap();
        write(cfg.path(), &record("r1", 10, "e1")).unwrap();
        write(cfg.path(), &record("r2", 20, "e2")).unwrap();
        assert_eq!(read(cfg.path(), "r1").unwrap().started_at_unix, 10);
        assert_eq!(newest(cfg.path()).unwrap().unwrap().run_id, "r2");
        assert_eq!(of_entry(cfg.path(), "e1").unwrap().unwrap().run_id, "r1");
        assert_eq!(of_entry(cfg.path(), "r2").unwrap().unwrap().run_id, "r2");
        assert!(of_entry(cfg.path(), "nope").unwrap().is_none());
        assert_eq!(read(cfg.path(), "nope").unwrap_err().exit_code(), 1);
    }

    #[test]
    fn an_id_that_is_a_path_names_no_run() {
        let cfg = tempfile::tempdir().unwrap();
        assert!(write(cfg.path(), &record("../x", 1, "e")).is_err());
        assert!(read_if_any(cfg.path(), "../x").unwrap().is_none());
    }

    #[test]
    fn no_runs_is_none_not_an_error() {
        let cfg = tempfile::tempdir().unwrap();
        assert!(newest(cfg.path()).unwrap().is_none());
    }
}
