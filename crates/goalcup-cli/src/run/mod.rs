//! A run on this machine: the record of it, its disk reservation, and the tick that watches
//! the disk while it records.
//!
//! One `goalcup start` hosts one run. Its record is `<cfg>/runs/<run_id>/run.json` ([`RunRecord`]),
//! written when the daemon has created the entry and read by the commands that come after it.
//! A run opened on a server uses the server's run id; a local run uses its entry id.
//!
//! [`reserve`] holds the run's one disk reservation and the rules for resizing it, [`watch`] is
//! the 30-second tick that applies them, and [`pause`] pauses the agent while the disk is nearly
//! full (D65).

pub mod pause;
pub mod reserve;
pub mod time_left;
pub mod watch;

use crate::CliError;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
pub use time_left::TimeLeft;

use crate::participant::{self, Restart, Who};

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

impl RunRecord {
    /// The mode the run was started in.
    pub fn mode(&self) -> crate::account::device::RunMode {
        use crate::account::device::RunMode;
        if self.test {
            RunMode::Test
        } else if self.practice {
            RunMode::Practice
        } else {
            RunMode::Competition
        }
    }
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
    /// A practice run (not a competition entry): `goalcup start --practice`.
    pub practice: bool,
    /// An organizer test run (`goalcup start --test`). Absent from a `run.json` written before
    /// the flag existed, which reads as `false`.
    #[serde(default)]
    pub test: bool,
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
    /// The uploader gave up on the run. Only an older build's uploader did; a run's own upload
    /// never gives up now (D64).
    #[serde(default)]
    pub stopped: bool,
    /// The server has refused this run's calls with this status [`REFUSALS_SAID_AFTER`] times in
    /// a row (a `4xx`, such as a bad deploy's `401`, `404` or `410`): recording goes on, and the
    /// calls are asked again until it answers normally, which clears it.
    #[serde(default)]
    pub refused: Option<u16>,
}

/// How many refusals in a row, of the same status, the run says once in the terminal and
/// `goalcup status` keeps as a problem until the server answers normally again.
pub const REFUSALS_SAID_AFTER: u32 = 3;

/// Which of the run's calls the server answered: the upload's (its position, chunks, the final
/// heartbeat and the seal) or the ordinary heartbeats. Each counts its refusals on its own, so a
/// heartbeat that works does not hide chunks that keep being refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Upload = 0,
    Heartbeat = 1,
}

/// The run's heartbeats to the server, as far as the participant needs to know.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatState {
    /// How many heartbeats in a row failed; `0` once one is stored.
    #[serde(default)]
    pub failures: u32,
    /// Why the last one failed, in a sentence.
    #[serde(default)]
    pub last_failure: Option<String>,
    /// The unix second the first of these failures came: how long the server has not heard
    /// the run is counted from it, never from the count (heartbeats are not always minutes
    /// apart).
    #[serde(default)]
    pub failing_since: Option<u64>,
}

/// How many heartbeats in a row must fail before `goalcup status` calls it a problem: three,
/// which at the five-minute cadence is a quarter of an hour without the server hearing the run.
pub const HEARTBEATS_FAILING_AFTER: u32 = 3;

/// The disk, as the last tick measured it.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskState {
    /// Free bytes beyond the reservation.
    pub free_bytes: u64,
    /// Bytes the reservation holds.
    pub reserve_bytes: u64,
    /// Less than the warning level is free beyond the reservation.
    pub low: bool,
    /// The agent is paused because the disk is nearly full (D65). Absent from a `state.json` an
    /// older build wrote, which never paused it.
    #[serde(default)]
    pub disk_paused: bool,
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
    /// Absent from a `state.json` an older build wrote.
    #[serde(default)]
    pub heartbeat: HeartbeatState,
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
    /// `goalcup continue` could restart this run once its agent is gone: it is a run on a
    /// server and its connector is a chat-mode one. Absent from a `state.json` an older build
    /// wrote, which had no `continue`.
    #[serde(default)]
    pub continuable: bool,
    /// What the server last said was left of the run, counted on the continuous clock: what
    /// says whether the run is over ([`is_over`]). Absent for a run with no server, and from a
    /// `state.json` an older build wrote.
    #[serde(default)]
    pub time_left: Option<TimeLeft>,
}

/// What `goalcup status` may tell the participant about `goalcup continue` for a run: the advice
/// is given only when the command can actually restart the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advice {
    /// A run on a server, chat-mode, not over, started here: `goalcup continue` restarts it.
    Continue,
    /// The run is over (its deadline passed, a newer run replaced it, or it was stopped for its
    /// end): there is nothing left to continue.
    Over,
    /// The run is not over, but it cannot be continued: it has no server, its connector runs
    /// headless, or this machine holds no credential for it.
    Cannot,
}

impl Advice {
    /// What a problem line says about restarting the agent, for the run `who`.
    fn restart<'a>(self, who: &Who<'a>) -> Restart<'a> {
        match self {
            Advice::Continue => Restart::Continue { run_id: who.run_id },
            Advice::Over => Restart::Over,
            Advice::Cannot => Restart::Cannot,
        }
    }
}

/// Whether the run is over: its host stopped it for its end (the server's stop, a newer run
/// replacing it), or its time is up by the count of the server's last answer
/// ([`RunState::time_left`]). This machine's date is never read for it, except for a state with
/// no count, against `now` (unix seconds; [`time_left::time_up`]). A full disk stops the agent
/// and leaves the run open.
pub fn is_over(record: &RunRecord, state: Option<&RunState>, now: u64) -> bool {
    state.is_some_and(RunState::stopped_for_its_end)
        || time_left::time_up(
            state.and_then(|state| state.time_left.as_ref()),
            record.deadline_at.as_deref(),
            now,
        )
}

/// What a run's state says once the server refused `goalcup continue` because the run is over.
pub const SERVER_SAYS_OVER: &str = "the server says the run is over (it refused goalcup continue)";

/// The server said the run is over: say so in the run's `state.json` ([`RunState::stopped`]), so
/// that `goalcup status` and the reserve's release follow the server at once, whatever the count
/// or the date say. A run with no state is left as it is.
pub fn note_over(cfg: &Path, run_id: &str) {
    let dir = run_dir(cfg, run_id);
    if let Some(mut state) = read_state(&dir) {
        if !state.stopped_for_its_end() {
            state.stopped = Some(SERVER_SAYS_OVER.to_string());
        }
        StateKeeper::begin(&dir, state);
    }
}

/// Whether `goalcup continue` can restart the run: it is on a server, its connector is a
/// chat-mode one (`continuable`), it is not `over`, and this machine holds the run's credential.
/// The one answer the host, `goalcup status` and the reserve's release all use.
pub fn can_continue(cfg: &Path, record: &RunRecord, continuable: bool, over: bool) -> bool {
    !over
        && record.server_url.is_some()
        && continuable
        && crate::account::credential::credential_path(&run_dir(cfg, &record.run_id)).exists()
}

/// What `goalcup status` may say about `goalcup continue` for `record`, whose state is `state`.
pub fn advice(cfg: &Path, record: &RunRecord, state: &RunState, now: u64) -> Advice {
    let over = is_over(record, Some(state), now);
    if can_continue(cfg, record, state.continuable, over) {
        Advice::Continue
    } else if over {
        Advice::Over
    } else {
        Advice::Cannot
    }
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
            heartbeat: HeartbeatState::default(),
            disk: DiskState::default(),
            sleep_lock,
            stopped: None,
            ended: false,
            continuable: false,
            time_left: None,
        }
    }

    /// The host ended the agent on purpose because the run is over (the deadline, a newer run);
    /// a full disk is not an end.
    pub(crate) fn stopped_for_its_end(&self) -> bool {
        self.stopped
            .as_deref()
            .is_some_and(|why| why != watch::DISK_STOP_REASON)
    }

    /// The agent's process exited with status 0.
    fn exited_cleanly(&self) -> bool {
        matches!(&self.harness, HarnessState::Exited { status } if status == "exit code 0")
    }

    /// The plain line for a run whose agent finished normally and which `goalcup continue` cannot
    /// restart: a normal end, not a problem.
    pub fn finished(&self, advice: Advice, who: &Who<'_>) -> Option<String> {
        (self.exited_cleanly() && !self.stopped_for_its_end() && advice != Advice::Continue)
            .then(|| participant::agent_finished(who.harness, advice == Advice::Over))
    }

    /// Whether no tracker writes this state any more: the host has not finished, and the tick has
    /// not written it for a few watch periods.
    pub fn is_stale(&self, now: u64) -> bool {
        !self.ended
            && now.saturating_sub(self.updated_at)
                > STATE_STALE_AFTER_TICKS * reserve::WATCH_PERIOD.as_secs()
    }

    /// Everything wrong with the run at `now` (unix seconds), one sentence each, worst first, each
    /// ending with what to do about it. Nothing wrong is an empty list. `advice` says whether
    /// `goalcup continue` can restart the run: the lines say to run it only when it can. `who`
    /// names the harness and the run in the words.
    pub fn problems(&self, now: u64, advice: Advice, who: &Who<'_>) -> Vec<String> {
        let mut lines = Vec::new();
        // The tick writes the state every watch period; a run not over whose state has not
        // moved for a few of them has no tracker writing it.
        if self.is_stale(now) {
            let silent = now.saturating_sub(self.updated_at);
            lines.push(participant::problem_state_stale(
                &age_words(silent),
                advice.restart(who),
            ));
        }
        match self.recording {
            Recording::Gaps => lines.push(participant::problem_recording_gaps()),
            Recording::Failed => lines.push(participant::problem_recording_failed(who.run_id)),
            Recording::Ok | Recording::Unknown => {}
        }
        // The server keeps refusing the run's calls (a bad deploy): the run goes on recording
        // and asking, and the participant is told nothing is lost (D64).
        if let Some(status) = self.upload.refused
            && !self.ended
        {
            lines.push(participant::problem_server_refuses(status));
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
                lines.push(participant::problem_upload(
                    &age_words(age),
                    why,
                    self.ended || self.upload.stopped,
                ));
            }
        }
        if self.heartbeat.failures >= HEARTBEATS_FAILING_AFTER
            && !self.ended
            && let Some(why) = &self.heartbeat.last_failure
        {
            // A state an older build wrote has no start: the count at the usual cadence.
            let silent = self.heartbeat.failing_since.map_or_else(
                || u64::from(self.heartbeat.failures) * HEARTBEAT_EVERY_SECS,
                |since| now.saturating_sub(since),
            );
            lines.push(participant::problem_heartbeats(&age_words(silent), why));
        }
        // A paused agent's line says what to do about the disk, so the low-space line is not
        // said beside it.
        if self.disk.disk_paused && !self.ended {
            lines.push(participant::problem_disk_paused().to_string());
        } else if self.disk.low && !self.ended {
            lines.push(participant::problem_disk(&reserve::human(
                self.disk.free_bytes,
            )));
        }
        if let Err(why) = &self.sleep_lock
            && !self.ended
        {
            lines.push(participant::problem_sleep(why));
        }
        // An agent the host ended because the run is over (its deadline, a newer run) is not a
        // problem; one that left while the run is open, or that a full disk stopped, is.
        if let HarnessState::Exited { status } = &self.harness
            && !self.stopped_for_its_end()
            // A clean exit of a run that cannot be continued is a normal end ([`Self::finished`]).
            && !(self.exited_cleanly() && advice != Advice::Continue)
        {
            lines.push(participant::problem_agent_exited(
                who,
                status,
                self.stopped.as_deref() == Some(watch::DISK_STOP_REASON),
                advice.restart(who),
            ));
        }
        lines
    }
}

/// The usual time between two heartbeats.
const HEARTBEAT_EVERY_SECS: u64 = 5 * 60;

/// How many watch periods without a write make a run's state stale.
pub const STATE_STALE_AFTER_TICKS: u64 = 3;

/// A span in words: `45 s`, `12 min`, `3 h 5 min`, `2 d 4 h`.
pub(crate) fn age_words(secs: u64) -> String {
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
    /// The uploader's count of what is left of the run, each time it is set.
    time_left: Arc<Mutex<Option<TimeLeft>>>,
    /// What the heartbeats say of the disk and the tracer's own processes; `None` says nothing.
    probe: Option<crate::tracer_vitals::Probe>,
}

#[derive(Debug, Default)]
struct UploadVital {
    problem: Option<String>,
    stopped: bool,
    /// Heartbeats in a row that failed, and why the last one did.
    heartbeat_failures: u32,
    heartbeat_last: Option<String>,
    heartbeat_failing_since: Option<u64>,
    /// Per [`Channel`]: the status the server last refused that channel's call with, and how
    /// many times in a row it did. `None` once it answers normally.
    refusals: [Option<(u16, u32)>; 2],
    /// The refusal now standing has been said in the terminal.
    refusal_said: bool,
}

impl UploadVital {
    /// The status the server has refused one channel's calls with [`REFUSALS_SAID_AFTER`] times
    /// in a row, the upload's first.
    fn refused(&self) -> Option<u16> {
        self.refusals
            .iter()
            .flatten()
            .find(|(_, times)| *times >= REFUSALS_SAID_AFTER)
            .map(|(status, _)| *status)
    }
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
            time_left: Arc::default(),
            probe: None,
        }
    }

    /// These vitals, with the heartbeats' `vitals` read by `probe`.
    pub fn probing(mut self, probe: crate::tracer_vitals::Probe) -> Vitals {
        self.probe = Some(probe);
        self
    }

    /// The heartbeat's `vitals`, read now; `None` without a probe.
    pub fn heartbeat_vitals(&self) -> Option<crate::upload::api::HeartbeatVitals> {
        self.probe.as_ref().map(crate::tracer_vitals::Probe::read)
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

    /// The uploader set its count of what is left of the run.
    pub fn set_time_left(&self, left: TimeLeft) {
        *self
            .time_left
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(left);
    }

    /// The uploader's count as last set; `None` before it is set.
    pub fn time_left(&self) -> Option<TimeLeft> {
        self.time_left
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

    /// A heartbeat failed: counted until one is stored. The count so far, this one included.
    pub fn heartbeat_failed(&self, why: &str) -> u32 {
        let mut upload = self.upload();
        upload.heartbeat_failures = upload.heartbeat_failures.saturating_add(1);
        upload.heartbeat_last = Some(why.to_string());
        upload.heartbeat_failing_since.get_or_insert_with(now_unix);
        upload.heartbeat_failures
    }

    /// A heartbeat was stored.
    pub fn heartbeat_ok(&self) {
        let mut upload = self.upload();
        upload.heartbeat_failures = 0;
        upload.heartbeat_last = None;
        upload.heartbeat_failing_since = None;
    }

    /// How the heartbeats are doing: the failures in a row, and why the last one failed.
    pub fn heartbeat_trouble(&self) -> HeartbeatState {
        let upload = self.upload();
        HeartbeatState {
            failures: upload.heartbeat_failures,
            last_failure: upload.heartbeat_last.clone(),
            failing_since: upload.heartbeat_failing_since,
        }
    }

    /// The uploader's last failure, and whether it gave up.
    pub fn upload_problem(&self) -> (Option<String>, bool) {
        let upload = self.upload();
        (upload.problem.clone(), upload.stopped)
    }

    /// The server refused a call of `channel` with `status` (a `4xx`). Answers `true` exactly
    /// once per standing refusal: when one channel has had [`REFUSALS_SAID_AFTER`] refusals of
    /// the same status in a row, and it is not said already. The caller says it then.
    pub fn server_refused(&self, channel: Channel, status: u16) -> bool {
        let mut upload = self.upload();
        let slot = &mut upload.refusals[channel as usize];
        *slot = match *slot {
            Some((same, times)) if same == status => Some((same, times.saturating_add(1))),
            _ => Some((status, 1)),
        };
        let standing = upload.refused().is_some();
        let say = standing && !upload.refusal_said;
        upload.refusal_said = standing;
        say
    }

    /// The server answered a call of `channel` normally: that channel's run of refusals is
    /// over. A call that failed some other way (no answer, a `5xx`) leaves the run as it is.
    pub fn server_answered(&self, channel: Channel) {
        let mut upload = self.upload();
        upload.refusals[channel as usize] = None;
        if upload.refused().is_none() {
            upload.refusal_said = false;
        }
    }

    /// The status the server keeps refusing the run's calls with, once it has
    /// [`REFUSALS_SAID_AFTER`] times in a row; `None` while it answers.
    pub fn server_refusing(&self) -> Option<u16> {
        self.upload().refused()
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
pub(crate) fn all(cfg: &Path) -> Vec<RunRecord> {
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

/// Where a participant writes when the tracker cannot help.
const SUPPORT_ADDRESS: &str = "goalcup@agentcommunity.org";

/// What the tracker says of a capture entry its daemon could not reopen. `goalcup doctor` and
/// `goalcup seal` both say it, so the two cannot give different advice for the one condition.
pub fn reopen_failed_sentence(entry_id: &str) -> String {
    format!("Recording {entry_id} could not be reopened.")
}

/// What to do about it: `goalcup continue` once, for the run the entry belongs to.
pub fn reopen_failed_next(run_id: &str) -> String {
    format!(
        "Run `goalcup continue {run_id}` once. If this stays after that, write to \
         {SUPPORT_ADDRESS}."
    )
}

/// The entries of `reopen_failed` (a daemon's health answer) that need that advice, each with
/// its run id. An entry of a run that is over, whose upload is finished, that `goalcup continue`
/// cannot restart, or that no run here holds, is history and not an action, so it is left out.
pub fn reopen_failed_actions(cfg: &Path, entry_ids: &[String], now: u64) -> Vec<(String, String)> {
    let spool = cfg.join(crate::upload::UPLOADS_DIR_NAME);
    entry_ids
        .iter()
        .filter_map(|entry_id| {
            let record = of_entry(cfg, entry_id).ok().flatten()?;
            let state = read_state(&run_dir(cfg, &record.run_id))?;
            let open = advice(cfg, &record, &state, now) == Advice::Continue;
            let sent = crate::upload::uploader::is_sent(&spool, entry_id);
            (open && !sent).then(|| (entry_id.clone(), record.run_id))
        })
        .collect()
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
pub(crate) mod tests_support {
    use super::*;

    /// A run on a server, with one entry, that `goalcup continue` can restart: the run has a
    /// credential and a state that says it is continuable. `deadline` is its `deadline_at`.
    pub(crate) fn continuable_run(cfg: &Path, run_id: &str, entry_id: &str, deadline: u64) {
        let record = crate::run::RunRecord {
            run_id: run_id.to_string(),
            server_url: Some("https://server.example".to_string()),
            practice: false,
            test: false,
            connector_id: "c".to_string(),
            workspace: PathBuf::from("/w"),
            started_at_unix: 1,
            deadline_at: Some(goalcup_protocol::control::format_z(deadline)),
            entries: vec![crate::run::EntryRef {
                entry_id: entry_id.to_string(),
                trace_dir: PathBuf::from("/t"),
            }],
        };
        crate::run::write(cfg, &record).unwrap();
        let dir = crate::run::run_dir(cfg, run_id);
        let credential = crate::account::credential::RunCredential::new(
            format!("gcr_{}", "ab".repeat(32)),
            "2099-01-01T00:00:00Z".to_string(),
        )
        .unwrap();
        crate::account::credential::store(&dir, &credential).unwrap();
        let mut state = crate::run::RunState::launched(entry_id, Ok(()));
        state.continuable = true;
        crate::run::StateKeeper::begin(&dir, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(run_id: &str, started: u64, entry: &str) -> RunRecord {
        RunRecord {
            run_id: run_id.to_string(),
            server_url: None,
            practice: false,
            test: false,
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

    fn who() -> Who<'static> {
        Who {
            harness: "Some Agent",
            run_id: "r0123456789abcdef",
            until: Some("Sat 14 Nov, 10:12 UTC"),
        }
    }

    #[test]
    fn problems_come_in_the_order_the_participant_should_act() {
        let now = 1_000_000;
        let who = who();
        let mut state = RunState::launched("e1", Ok(()));
        assert!(state.problems(now, Advice::Continue, &who).is_empty());
        state.recording = Recording::Unknown;
        state.upload.problem = Some("busy".to_string());
        state.upload.last_ok_at = Some(now - 29 * 60);
        assert!(
            state.problems(now, Advice::Continue, &who).is_empty(),
            "29 minutes is not yet a problem"
        );
        state.upload.last_ok_at = Some(now - 3 * 3600 - 5 * 60);
        let lines = state.problems(now, Advice::Continue, &who);
        assert_eq!(lines.len(), 1, "{lines:?}");
        // The age, the cause, that nothing is lost, and what to do.
        assert!(lines[0].contains("3 h 5 min"), "{lines:?}");
        assert!(lines[0].contains("busy"), "{lines:?}");
        assert!(lines[0].contains("Nothing is lost"), "{lines:?}");
        assert!(lines[0].contains("Check the network"), "{lines:?}");
        // A run that is over is told how its upload ended, whatever the age, and nothing about
        // what only matters while it records. The fix is the command.
        state.upload.last_ok_at = Some(now - 10);
        state.ended = true;
        state.disk.low = true;
        state.sleep_lock = Err("x".into());
        let lines = state.problems(now, Advice::Continue, &who);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("10 s"), "{lines:?}");
        assert!(lines[0].contains("run: goalcup upload"), "{lines:?}");
    }

    /// D65: a paused agent is one problem line, in the owner's words, instead of the low-disk
    /// line, and it goes once the agent resumes. A state an older build wrote reads as not paused.
    #[test]
    fn a_paused_agent_is_a_problem_until_it_resumes() {
        let now = 1_000_000;
        let who = who();
        let mut state = RunState::launched("e1", Ok(()));
        state.updated_at = now;
        state.disk.low = true;
        state.disk.disk_paused = true;
        let lines = state.problems(now, Advice::Continue, &who);
        assert_eq!(lines, [participant::problem_disk_paused()]);
        state.disk.disk_paused = false;
        state.disk.free_bytes = 4 << 30;
        let lines = state.problems(now, Advice::Continue, &who);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("4 GiB"), "{lines:?}");
        let old: DiskState =
            serde_json::from_str(r#"{"free_bytes":1,"reserve_bytes":2,"low":true}"#).unwrap();
        assert!(!old.disk_paused);
    }

    /// Each problem line ends with what to do, and says nothing a participant cannot act on.
    #[test]
    fn every_problem_line_carries_its_fix() {
        let now = 1_000_000;
        let who = who();
        let mut state = RunState::launched("e1", Err("no sleep inhibitor".into()));
        state.updated_at = now;
        state.recording = Recording::Gaps;
        state.heartbeat = HeartbeatState {
            failures: HEARTBEATS_FAILING_AFTER,
            last_failure: Some("the server answered HTTP 503".to_string()),
            ..HeartbeatState::default()
        };
        state.disk = DiskState {
            free_bytes: 3 << 30,
            reserve_bytes: 0,
            low: true,
            disk_paused: false,
        };
        let lines = state.problems(now, Advice::Continue, &who);
        let said = lines.join("\n");
        assert_eq!(lines.len(), 4, "{said}");
        assert!(lines[0].contains("gaps") && lines[0].contains(participant::SUPPORT_ADDRESS));
        assert!(lines[1].contains("15 min") && lines[1].contains("Check the network"));
        assert!(lines[2].contains("3 GiB") && lines[2].contains("Free some space"));
        assert!(lines[3].contains("sleep") && lines[3].contains("power"));
        state.recording = Recording::Failed;
        let lines = state.problems(now, Advice::Continue, &who);
        assert!(lines[0].contains(who.run_id), "{lines:?}");
        assert!(lines[0].contains(participant::SUPPORT_ADDRESS), "{lines:?}");
        for word in ["tracer", "heartbeat", "entry", "segment", "daemon"] {
            assert!(!said.contains(word), "{word}: {said}");
        }
    }

    #[test]
    fn a_state_nobody_has_written_for_three_ticks_is_a_problem_until_the_run_ends() {
        let who = who();
        let mut state = RunState::launched("e1", Ok(()));
        state.updated_at = 1_000;
        let period = reserve::WATCH_PERIOD.as_secs();
        let fresh = 1_000 + STATE_STALE_AFTER_TICKS * period;
        assert!(
            state.problems(fresh, Advice::Continue, &who).is_empty(),
            "three ticks late is not yet stale"
        );
        let lines = state.problems(fresh + 1, Advice::Continue, &who);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("1 min"), "{lines:?}");
        assert!(
            lines[0].contains("goalcup continue r0123456789abcdef"),
            "{lines:?}"
        );
        // A run nothing can continue is not told to.
        for advice in [Advice::Over, Advice::Cannot] {
            let lines = state.problems(fresh + 1, advice, &who);
            assert!(!lines[0].contains("goalcup continue"), "{lines:?}");
        }
        state.ended = true;
        assert!(
            state
                .problems(fresh + 10_000, Advice::Continue, &who)
                .is_empty()
        );
    }

    #[test]
    fn an_agent_that_left_is_a_problem_unless_the_run_is_over() {
        let who = who();
        let mut state = RunState::launched("e1", Ok(()));
        state.harness = HarnessState::Exited {
            status: "exit code 1".to_string(),
        };
        let now = state.updated_at;
        // It left while the run is open: who left, how, until when, and the command with the id.
        let lines = state.problems(now, Advice::Continue, &who);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("Some Agent has exited (exit code 1)."),
            "{lines:?}"
        );
        assert!(lines[0].contains(participant::STILL_OPEN), "{lines:?}");
        assert!(lines[0].contains("Sat 14 Nov, 10:12 UTC"), "{lines:?}");
        assert!(
            lines[0].ends_with("goalcup continue r0123456789abcdef"),
            "{lines:?}"
        );
        // The host ended it because the run is over: the deadline, or a newer run.
        for why in [
            "the server says the run is over (its deadline was 2026-10-21T00:00:00Z)",
            "a newer run on your account replaced this one",
        ] {
            state.stopped = Some(why.to_string());
            assert!(
                state.problems(now, Advice::Continue, &who).is_empty(),
                "{why}"
            );
        }
        // A full disk ended it, the run is still open: free space first, then continue.
        state.stopped = Some(watch::DISK_STOP_REASON.to_string());
        let lines = state.problems(now, Advice::Continue, &who);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("disk"), "{lines:?}");
        assert!(
            lines[0].contains("goalcup continue r0123456789abcdef"),
            "{lines:?}"
        );
    }

    /// WP11: the run is over by the count of the server's last answer, never by this machine's
    /// date. A date set a day past the deadline neither stops the advice to `continue` nor gives
    /// the run's reserve back; a count that has run out is over whatever the date says.
    #[test]
    fn a_date_set_forward_neither_ends_the_run_nor_the_advice_to_continue() {
        use goalcup_protocol::clock::continuous_ns;
        let cfg = tempfile::tempdir().unwrap();
        let mut run = record("r0123456789abcdef", 1, "e1");
        run.server_url = Some("https://server.example".to_string());
        run.deadline_at = Some(goalcup_protocol::control::format_z(10_000));
        write(cfg.path(), &run).unwrap();
        let credential = crate::account::credential::RunCredential::new(
            format!("gcr_{}", "ab".repeat(32)),
            "2099-01-01T00:00:00Z".to_string(),
        )
        .unwrap();
        let dir = run_dir(cfg.path(), &run.run_id);
        crate::account::credential::store(&dir, &credential).unwrap();
        let mut state = RunState::launched("e1", Ok(()));
        state.continuable = true;
        state.harness = HarnessState::Exited {
            status: "exit code 1".to_string(),
        };
        // The server said an hour is left; the date says the deadline was a day ago.
        state.time_left = Some(TimeLeft::counted(3_600, continuous_ns()));
        let a_day_on = 10_000 + 86_400;
        assert!(!is_over(&run, Some(&state), a_day_on));
        assert_eq!(advice(cfg.path(), &run, &state, a_day_on), Advice::Continue);
        StateKeeper::begin(&dir, state.clone());
        assert!(!reserve::run_is_done(cfg.path(), &run, false, a_day_on));

        // The count has run out: over, though the date says the deadline is far off.
        state.time_left = Some(TimeLeft::counted(0, continuous_ns()));
        assert!(is_over(&run, Some(&state), 5_000));
        assert_eq!(advice(cfg.path(), &run, &state, 5_000), Advice::Over);
        StateKeeper::begin(&dir, state);
        assert!(reserve::run_is_done(cfg.path(), &run, false, 5_000));
    }

    /// `goalcup continue` is advised only when it can restart the run.
    #[test]
    fn the_advice_to_continue_is_given_only_when_continue_can_restart_the_run() {
        let cfg = tempfile::tempdir().unwrap();
        let mut run = record("r0123456789abcdef", 1, "e1");
        run.server_url = Some("https://server.example".to_string());
        run.deadline_at = Some(goalcup_protocol::control::format_z(10_000));
        write(cfg.path(), &run).unwrap();
        let credential = crate::account::credential::RunCredential::new(
            format!("gcr_{}", "ab".repeat(32)),
            "2099-01-01T00:00:00Z".to_string(),
        )
        .unwrap();
        crate::account::credential::store(&run_dir(cfg.path(), &run.run_id), &credential).unwrap();
        let mut state = RunState::launched("e1", Ok(()));
        state.continuable = true;
        state.harness = HarnessState::Exited {
            status: "exit code 1".to_string(),
        };
        let now = 5_000;
        let who = who();
        let has_exited = "Some Agent has exited (exit code 1)";

        // A server run, a chat-mode connector, a credential here, the deadline ahead.
        assert_eq!(advice(cfg.path(), &run, &state, now), Advice::Continue);
        let lines = state.problems(now, Advice::Continue, &who);
        assert!(lines[0].contains(has_exited), "{lines:?}");
        assert!(
            lines[0].contains("goalcup continue r0123456789abcdef"),
            "{lines:?}"
        );

        // The deadline has passed: the run is over, and the line says so.
        let over = advice(cfg.path(), &run, &state, 10_000);
        assert_eq!(over, Advice::Over);
        let lines = state.problems(10_000, over, &who);
        assert!(lines[0].contains(has_exited), "{lines:?}");
        assert!(lines[0].contains("the run is over"), "{lines:?}");
        assert!(!lines[0].contains("goalcup continue"), "{lines:?}");

        // A newer run replaced it.
        let mut replaced = state.clone();
        replaced.stopped = Some("a newer run on your account replaced this one".to_string());
        assert_eq!(advice(cfg.path(), &run, &replaced, now), Advice::Over);

        // A headless connector: not over, and not continuable.
        let mut headless = state.clone();
        headless.continuable = false;
        let cannot = advice(cfg.path(), &run, &headless, now);
        assert_eq!(cannot, Advice::Cannot);
        let lines = headless.problems(now, cannot, &who);
        assert!(lines[0].contains(has_exited), "{lines:?}");
        assert!(lines[0].contains("cannot be continued"), "{lines:?}");
        assert!(!lines[0].contains("goalcup continue"), "{lines:?}");

        // A run with no server.
        let mut local = run.clone();
        local.server_url = None;
        assert_eq!(advice(cfg.path(), &local, &state, now), Advice::Cannot);

        // `can_continue` is the one answer: server run, chat-mode, not over, credential here.
        assert!(can_continue(cfg.path(), &run, true, false));
        assert!(!can_continue(cfg.path(), &run, false, false), "headless");
        assert!(!can_continue(cfg.path(), &run, true, true), "over");
        assert!(!can_continue(cfg.path(), &local, true, false), "no server");

        // No credential on this machine: all three places that ask say no.
        std::fs::remove_file(crate::account::credential::credential_path(&run_dir(
            cfg.path(),
            &run.run_id,
        )))
        .unwrap();
        assert!(!can_continue(cfg.path(), &run, true, false));
        assert_eq!(advice(cfg.path(), &run, &state, now), Advice::Cannot);

        // No line ever tells a run that cannot be continued to continue it.
        for advice in [Advice::Over, Advice::Cannot] {
            let mut stale = RunState::launched("e1", Ok(()));
            stale.updated_at = 0;
            let lines = stale.problems(10_000, advice, &who);
            assert!(
                lines.iter().all(|line| !line.contains("goalcup continue")),
                "{lines:?}"
            );
        }
    }

    /// A normal end of a run that cannot be continued is a plain line, not a problem; a
    /// non-zero exit is still one.
    #[test]
    fn a_clean_exit_of_a_run_that_cannot_be_continued_is_not_a_problem() {
        let who = who();
        let mut state = RunState::launched("e1", Ok(()));
        state.harness = HarnessState::Exited {
            status: "exit code 0".to_string(),
        };
        let now = state.updated_at;
        for advice in [Advice::Cannot, Advice::Over] {
            assert!(state.problems(now, advice, &who).is_empty(), "{advice:?}");
        }
        let cannot = state.finished(Advice::Cannot, &who).unwrap();
        assert!(cannot.starts_with("Some Agent finished"), "{cannot}");
        assert!(cannot.contains("cannot be continued"), "{cannot}");
        let over = state.finished(Advice::Over, &who).unwrap();
        assert!(over.contains("the run is over"), "{over}");
        // A run `continue` can restart: the participant is still told, as a problem.
        let lines = state.problems(now, Advice::Continue, &who);
        assert!(lines[0].contains("exit code 0"), "{lines:?}");
        assert!(
            lines[0].contains("goalcup continue r0123456789abcdef"),
            "{lines:?}"
        );
        assert_eq!(state.finished(Advice::Continue, &who), None);
        // A non-zero exit is a problem whatever the advice.
        state.harness = HarnessState::Exited {
            status: "exit code 1".to_string(),
        };
        let lines = state.problems(now, Advice::Cannot, &who);
        assert!(lines[0].contains("exit code 1"), "{lines:?}");
        assert!(lines[0].contains("cannot be continued"), "{lines:?}");
        assert_eq!(state.finished(Advice::Cannot, &who), None);
    }

    #[test]
    fn no_runs_is_none_not_an_error() {
        let cfg = tempfile::tempdir().unwrap();
        assert!(newest(cfg.path()).unwrap().is_none());
    }

    #[test]
    fn heartbeat_failures_count_in_a_row_until_one_is_stored() {
        let vitals = Vitals::default();
        assert_eq!(vitals.heartbeat_trouble(), HeartbeatState::default());
        assert_eq!(vitals.heartbeat_failed("503"), 1);
        assert_eq!(vitals.heartbeat_failed("timeout"), 2);
        let trouble = vitals.heartbeat_trouble();
        assert_eq!(trouble.failures, 2);
        assert_eq!(trouble.last_failure.as_deref(), Some("timeout"));
        assert!(trouble.failing_since.is_some());
        // The upload's own trouble is a separate count.
        vitals.upload_ok();
        assert_eq!(vitals.heartbeat_trouble().failures, 2);
        vitals.heartbeat_ok();
        assert_eq!(vitals.heartbeat_trouble(), HeartbeatState::default());
    }

    /// How long the server has not heard the run is the time since the first failure, however
    /// many heartbeats failed in it: half an hour of beats ten seconds apart is 30 min, not 900.
    #[test]
    fn heartbeat_silence_is_counted_in_time_not_in_failures() {
        let now = 1_000_000;
        let mut state = RunState::launched("e1", Ok(()));
        state.updated_at = now;
        state.heartbeat = HeartbeatState {
            failures: 180,
            last_failure: Some("no answer".to_string()),
            failing_since: Some(now - 30 * 60),
        };
        let lines = state.problems(now, Advice::Cannot, &who());
        let line = lines.iter().find(|l| l.contains("not heard")).unwrap();
        assert!(line.contains("about 30 min"), "{line}");
    }

    /// D64: three refusals of one status in a row are said once and named until the server
    /// answers that channel normally; another status starts the count again, and one channel
    /// answering does not hide the other's refusals.
    #[test]
    fn a_refusal_is_named_after_three_in_a_row_said_once_and_cleared_by_an_answer() {
        let vitals = Vitals::default();
        assert!(!vitals.server_refused(Channel::Upload, 401));
        assert!(!vitals.server_refused(Channel::Upload, 401));
        assert_eq!(vitals.server_refusing(), None, "two are a blip");
        assert!(vitals.server_refused(Channel::Upload, 401), "said at three");
        assert_eq!(vitals.server_refusing(), Some(401));
        assert!(
            !vitals.server_refused(Channel::Upload, 401),
            "and only once"
        );
        // The heartbeats working does not clear what the chunks meet.
        vitals.server_answered(Channel::Heartbeat);
        assert_eq!(vitals.server_refusing(), Some(401));
        vitals.server_answered(Channel::Upload);
        assert_eq!(vitals.server_refusing(), None);
        // A different status in between starts the count again; a new episode is said again.
        for status in [404, 410, 404, 404] {
            assert!(
                !vitals.server_refused(Channel::Heartbeat, status),
                "{status}"
            );
        }
        assert!(vitals.server_refused(Channel::Heartbeat, 404));
        assert_eq!(vitals.server_refusing(), Some(404));
    }
}
