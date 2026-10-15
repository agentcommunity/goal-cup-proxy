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
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// `<cfg>/runs`: one directory per run.
pub const RUNS_DIR_NAME: &str = "runs";

/// The run's record inside its directory.
pub const RUN_FILE_NAME: &str = "run.json";

/// What the run is doing now, inside its directory.
pub const STATE_FILE_NAME: &str = "state.json";

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

/// How the tracker sees its own recording. It is what the heartbeat tells the server
/// (`recording`) and what `goalcup status` puts first when it is not `Ok`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recording {
    Ok,
    /// The daemon says the trace is missing something.
    Gaps,
    /// The daemon says capture failed.
    Failed,
    /// No answer from the daemon (yet): not a verdict, never read as "dead".
    #[default]
    Unknown,
}

impl Recording {
    /// The heartbeat's word for it.
    pub fn wire(self) -> &'static str {
        match self {
            Recording::Ok => "ok",
            Recording::Gaps => "gaps",
            Recording::Failed => "failed",
            Recording::Unknown => "unknown",
        }
    }

    /// What the daemon's capture state means for the run. A capture that has not started
    /// recording yet says nothing.
    pub fn of_capture(capture: goalcup_protocol::capture_status::CaptureState) -> Recording {
        use goalcup_protocol::capture_status::CaptureState;
        match capture {
            CaptureState::Recording => Recording::Ok,
            CaptureState::Incomplete => Recording::Gaps,
            CaptureState::Failed => Recording::Failed,
            CaptureState::Starting => Recording::Unknown,
        }
    }

    fn from_u8(n: u8) -> Recording {
        match n {
            0 => Recording::Ok,
            1 => Recording::Gaps,
            2 => Recording::Failed,
            _ => Recording::Unknown,
        }
    }
}

/// Where the agent's process is.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessState {
    #[default]
    Running,
    /// The process ended; `status` says how, in words.
    Exited { status: String },
}

/// The upload, as far as the participant needs to know.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadState {
    /// The unix second the upload last worked (a chunk or the seal acknowledged, or nothing
    /// left to send). `None` for a run with no server.
    pub last_ok_at: Option<u64>,
    /// The uploader's last failure, in a sentence; `None` while it works.
    pub problem: Option<String>,
    /// The uploader gave up on the run (a refusal that asking again does not cure).
    #[serde(default)]
    pub stopped: bool,
}

/// The disk, as the last tick measured it.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskState {
    /// Free bytes beyond the reservation.
    pub free_bytes: u64,
    /// Bytes the reservation holds.
    pub reserve_bytes: u64,
    /// Less than the warning level is free beyond the reservation.
    pub low: bool,
}

/// What a run is doing now, kept at `<run_dir>/state.json`. The watch tick writes it, and the
/// host at each change of the agent's process; `goalcup status` reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunState {
    /// The unix second of the last write.
    pub updated_at: u64,
    pub entry_id: String,
    pub harness: HarnessState,
    pub recording: Recording,
    pub upload: UploadState,
    pub disk: DiskState,
    /// `Err` says why the machine was not kept awake. The run goes on either way.
    pub sleep_lock: Result<(), String>,
    /// Set when the host ended the agent on purpose (the deadline, a newer run, a full disk):
    /// an agent that exited with this unset left on its own. Only a stop because the run is
    /// over (not a full disk) makes the agent's exit no problem.
    #[serde(default)]
    pub stopped: Option<String>,
    /// The host has finished its wrap-up and written its last word.
    #[serde(default)]
    pub ended: bool,
}

impl RunState {
    /// A run that has just launched its agent.
    pub fn launched(entry_id: &str, sleep_lock: Result<(), String>) -> RunState {
        RunState {
            updated_at: now_unix(),
            entry_id: entry_id.to_string(),
            harness: HarnessState::Running,
            recording: Recording::Unknown,
            upload: UploadState::default(),
            disk: DiskState::default(),
            sleep_lock,
            stopped: None,
            ended: false,
        }
    }

    /// Everything wrong with the run at `now` (unix seconds), one sentence each, worst first.
    /// Nothing wrong is an empty list.
    pub fn problems(&self, now: u64) -> Vec<String> {
        let mut lines = Vec::new();
        // The tick writes the state every watch period; a run not over whose state has not
        // moved for a few of them has no tracker writing it.
        let silent = now.saturating_sub(self.updated_at);
        if !self.ended && silent > STATE_STALE_AFTER_TICKS * reserve::WATCH_PERIOD.as_secs() {
            lines.push(format!(
                "run state not updated for {}: the tracer may have stopped; run goalcup continue",
                age_words(silent)
            ));
        }
        match self.recording {
            Recording::Gaps => lines.push("recording has gaps".to_string()),
            Recording::Failed => lines.push("recording failed".to_string()),
            Recording::Ok | Recording::Unknown => {}
        }
        if let Some(why) = &self.upload.problem {
            // A run that is over is told what became of its upload; one that records is told
            // only once the failure has lasted.
            let age = self
                .upload
                .last_ok_at
                .map_or(0, |last_ok| now.saturating_sub(last_ok));
            let failing = age >= reserve::UPLOAD_FAILING_AFTER.as_secs();
            if self.ended || self.upload.stopped || failing {
                lines.push(format!("last upload {} ago: {why}", age_words(age)));
            }
        }
        if self.disk.low && !self.ended {
            lines.push(format!(
                "disk: {} free beyond the reservation",
                reserve::human(self.disk.free_bytes)
            ));
        }
        if let Err(why) = &self.sleep_lock
            && !self.ended
        {
            lines.push(format!("the machine may sleep: {why}"));
        }
        // An agent the host ended because the run is over (its deadline, a newer run) is not a
        // problem; one that left while the run is open, or that a full disk stopped, is.
        let over = self
            .stopped
            .as_deref()
            .is_some_and(|why| why != watch::DISK_STOP_REASON);
        if let HarnessState::Exited { status } = &self.harness
            && !over
        {
            lines.push(format!(
                "the agent's process exited ({status}): run goalcup continue"
            ));
        }
        lines
    }
}

/// How many watch periods without a write make a run's state stale.
pub const STATE_STALE_AFTER_TICKS: u64 = 3;

/// A span in words: `45 s`, `12 min`, `3 h 5 min`, `2 d 4 h`.
fn age_words(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        3600..86_400 => format!("{} h {} min", secs / 3600, secs % 3600 / 60),
        _ => format!("{} d {} h", secs / 86_400, secs % 86_400 / 3600),
    }
}

/// What the threads of one run tell each other: the recording as the last tick saw it, the
/// proxy's count of model requests, and how the upload is doing. Cloning shares it.
#[derive(Debug, Clone)]
pub struct Vitals {
    recording: Arc<AtomicU8>,
    model_requests: Arc<AtomicU64>,
    upload: Arc<Mutex<UploadVital>>,
}

#[derive(Debug, Default)]
struct UploadVital {
    problem: Option<String>,
    stopped: bool,
}

impl Default for Vitals {
    /// Nothing known: the recording unknown, no model request.
    fn default() -> Self {
        Vitals::counting(Arc::default())
    }
}

impl Vitals {
    /// Vitals that read model requests from `counter` (the proxy's).
    pub fn counting(counter: Arc<AtomicU64>) -> Vitals {
        Vitals {
            recording: Arc::new(AtomicU8::new(Recording::Unknown as u8)),
            model_requests: counter,
            upload: Arc::default(),
        }
    }

    pub fn recording(&self) -> Recording {
        Recording::from_u8(self.recording.load(Ordering::SeqCst))
    }

    pub fn set_recording(&self, recording: Recording) {
        self.recording.store(recording as u8, Ordering::SeqCst);
    }

    /// Model requests so far, cumulative.
    pub fn model_requests(&self) -> u64 {
        self.model_requests.load(Ordering::SeqCst)
    }

    fn upload(&self) -> MutexGuard<'_, UploadVital> {
        self.upload.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The upload failed, and will be asked again.
    pub fn upload_failed(&self, why: &str) {
        self.upload().problem = Some(why.to_string());
    }

    /// The upload gave up on the run.
    pub fn upload_stopped(&self, why: &str) {
        let mut upload = self.upload();
        upload.problem = Some(why.to_string());
        upload.stopped = true;
    }

    /// The upload works.
    pub fn upload_ok(&self) {
        let mut upload = self.upload();
        if !upload.stopped {
            upload.problem = None;
        }
    }

    /// The uploader's last failure, and whether it gave up.
    pub fn upload_problem(&self) -> (Option<String>, bool) {
        let upload = self.upload();
        (upload.problem.clone(), upload.stopped)
    }
}

/// The run's `state.json`, shared by the tick and the host: every change goes through
/// [`StateKeeper::update`], which writes the whole file again, atomically and mode 0600.
#[derive(Debug, Clone)]
pub struct StateKeeper {
    inner: Arc<KeeperInner>,
}

#[derive(Debug)]
struct KeeperInner {
    path: PathBuf,
    state: Mutex<RunState>,
}

impl StateKeeper {
    /// Keep `state` at `<run_dir>/state.json`, writing it now.
    pub fn begin(run_dir: &Path, state: RunState) -> StateKeeper {
        let keeper = StateKeeper {
            inner: Arc::new(KeeperInner {
                path: run_dir.join(STATE_FILE_NAME),
                state: Mutex::new(state),
            }),
        };
        keeper.update(|_| {});
        keeper
    }

    /// Change the state and write it. A write that fails leaves the state in memory and the
    /// next change tries again: a run never stops for it.
    pub fn update(&self, change: impl FnOnce(&mut RunState)) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        change(&mut state);
        state.updated_at = now_unix();
        if let Ok(mut text) = serde_json::to_vec_pretty(&*state) {
            text.push(b'\n');
            let _ = crate::config::write_secret_file(&self.inner.path, &text);
        }
    }

    /// [`StateKeeper::update`], except that the file is not written once it names another entry:
    /// a later segment of the run has taken `state.json` over (its host started while this
    /// one was still sending), and this host's last word must not replace the live segment's.
    pub fn update_while_current(&self, change: impl FnOnce(&mut RunState)) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        change(&mut state);
        state.updated_at = now_unix();
        let superseded = self
            .inner
            .path
            .parent()
            .and_then(read_state)
            .is_some_and(|written| written.entry_id != state.entry_id);
        if superseded {
            return;
        }
        if let Ok(mut text) = serde_json::to_vec_pretty(&*state) {
            text.push(b'\n');
            let _ = crate::config::write_secret_file(&self.inner.path, &text);
        }
    }

    /// The state as last written.
    pub fn snapshot(&self) -> RunState {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The most of a `state.json` this build reads.
const STATE_FILE_MAX_BYTES: u64 = 64 * 1024;

/// The run's state as last written, or `None` when there is none or it cannot be read.
pub fn read_state(run_dir: &Path) -> Option<RunState> {
    let file = std::fs::File::open(run_dir.join(STATE_FILE_NAME)).ok()?;
    let mut text = Vec::new();
    file.take(STATE_FILE_MAX_BYTES + 1)
        .read_to_end(&mut text)
        .ok()?;
    if text.len() as u64 > STATE_FILE_MAX_BYTES {
        return None;
    }
    serde_json::from_slice(&text).ok()
}

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
    run_dir(cfg, run_id).join(STATE_FILE_NAME)
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
    fn the_state_is_kept_atomically_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_state(dir.path()).is_none(), "no file, no state");
        let keeper = StateKeeper::begin(dir.path(), RunState::launched("e1", Ok(())));
        let first = read_state(dir.path()).expect("written at once");
        assert_eq!(first.entry_id, "e1");
        assert_eq!(first.harness, HarnessState::Running);
        keeper.update(|state| {
            state.recording = Recording::Gaps;
            state.harness = HarnessState::Exited {
                status: "exit code 2".to_string(),
            };
            state.sleep_lock = Err("no inhibitor".to_string());
        });
        let second = read_state(dir.path()).unwrap();
        assert_eq!(second, keeper.snapshot());
        assert_eq!(second.recording, Recording::Gaps);
        assert_eq!(second.sleep_lock, Err("no inhibitor".to_string()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(STATE_FILE_NAME))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // No temporary file is left beside it.
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(STATE_FILE_NAME)]);
        // Something that is not a state is none.
        std::fs::write(dir.path().join(STATE_FILE_NAME), b"{ not json").unwrap();
        assert!(read_state(dir.path()).is_none());
    }

    #[test]
    fn a_host_that_was_superseded_does_not_overwrite_the_live_segments_state() {
        let dir = tempfile::tempdir().unwrap();
        let first = StateKeeper::begin(dir.path(), RunState::launched("e1", Ok(())));
        first.update_while_current(|state| state.ended = true);
        assert!(read_state(dir.path()).unwrap().ended, "still its own file");
        // The next segment's host takes the file over while the first is still sending.
        let _second = StateKeeper::begin(dir.path(), RunState::launched("e2", Ok(())));
        first.update_while_current(|state| state.ended = true);
        let written = read_state(dir.path()).unwrap();
        assert_eq!(written.entry_id, "e2");
        assert!(!written.ended, "the live segment's state is not replaced");
    }

    #[test]
    fn problems_come_in_the_order_the_participant_should_act() {
        let now = 1_000_000;
        let mut state = RunState::launched("e1", Ok(()));
        assert!(state.problems(now).is_empty());
        state.recording = Recording::Unknown;
        state.upload.problem = Some("busy".to_string());
        state.upload.last_ok_at = Some(now - 29 * 60);
        assert!(
            state.problems(now).is_empty(),
            "29 minutes is not yet a problem"
        );
        state.upload.last_ok_at = Some(now - 3 * 3600 - 5 * 60);
        assert_eq!(state.problems(now), ["last upload 3 h 5 min ago: busy"]);
        // A run that is over is told how its upload ended, whatever the age, and nothing about
        // what only matters while it records.
        state.upload.last_ok_at = Some(now - 10);
        state.ended = true;
        state.disk.low = true;
        state.sleep_lock = Err("x".into());
        assert_eq!(state.problems(now), ["last upload 10 s ago: busy"]);
    }

    #[test]
    fn a_state_nobody_has_written_for_three_ticks_is_a_problem_until_the_run_ends() {
        let mut state = RunState::launched("e1", Ok(()));
        state.updated_at = 1_000;
        let period = reserve::WATCH_PERIOD.as_secs();
        let fresh = 1_000 + STATE_STALE_AFTER_TICKS * period;
        assert!(
            state.problems(fresh).is_empty(),
            "three ticks late is not yet stale"
        );
        assert_eq!(
            state.problems(fresh + 1),
            ["run state not updated for 1 min: the tracer may have stopped; run goalcup continue"]
        );
        state.ended = true;
        assert!(state.problems(fresh + 10_000).is_empty());
    }

    #[test]
    fn an_agent_that_left_is_a_problem_unless_the_run_is_over() {
        let mut state = RunState::launched("e1", Ok(()));
        state.harness = HarnessState::Exited {
            status: "exit code 1".to_string(),
        };
        let now = state.updated_at;
        let line = "the agent's process exited (exit code 1): run goalcup continue";
        // It left while the run is open.
        assert_eq!(state.problems(now), [line]);
        // The host ended it because the run is over: the deadline, or a newer run.
        for why in [
            "the server says the run is over (its deadline was 2026-10-21T00:00:00Z)",
            "a newer run on your account replaced this one",
        ] {
            state.stopped = Some(why.to_string());
            assert!(state.problems(now).is_empty(), "{why}");
        }
        // A full disk ended it, the run is still open: it can be continued.
        state.stopped = Some(watch::DISK_STOP_REASON.to_string());
        assert_eq!(state.problems(now), [line]);
    }

    #[test]
    fn no_runs_is_none_not_an_error() {
        let cfg = tempfile::tempdir().unwrap();
        assert!(newest(cfg.path()).unwrap().is_none());
    }
}
