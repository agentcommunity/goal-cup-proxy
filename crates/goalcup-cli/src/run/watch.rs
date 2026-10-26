//! The watch tick: while a run records and while its upload wraps up, every
//! [`super::reserve::WATCH_PERIOD`] it deletes the chain parts the server acknowledged long
//! ago, or at once when space is short ([`Pruner`]), measures the disk, resizes the run's
//! reserve, and pauses the agent while the disk is nearly full, resuming it when space returns
//! ([`super::pause`], D65). It never ends a run.
//!
//! The tick runs on a thread of its own, started before the wait for the harness and ended just
//! before the reserve is released ([`Watching::end`]). It never touches the recording. Once the
//! host has ended the pause (the run stopped, or the harness has gone) the tick only keeps the
//! reserve sized.
//!
//! Each tick also asks the daemon how the recording is (a bounded wait: no answer is
//! [`Recording::Unknown`], never a failure), and writes the run's `state.json` ([`Beat`]).
//! That is where the disk's low-space warning, the upload's trouble and the recording's gaps
//! are said; `goalcup status` reads them back. It also keeps the run's time-left file, which
//! the agent may read with `goalcup time-left` (D66).
//!
//! Twice the tick also says something once per start or continue (in `host.log` while the harness
//! owns the terminal) and keeps it in the state: when the harness has written no session file
//! while its model requests are recorded ([`super::NativeSilence`]), and when no model call has
//! been recorded at all after a wait ([`super::no_model_calls_after`]).

use super::pause::{AgentPause, Change};
use super::reserve::{
    DiskPolicy, GROW_STEP, RunReserve, human, is_low, pressure, short_of_disk, target_with,
};
use super::{DiskState, EntryRef, Recording, RunRecord, RunSignals, StateKeeper, UploadState};
use crate::upload::prune::{AckAges, deletable, parts_now};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What a build before D65 said when the disk ended a run. Nothing sends it now; it is still
/// read, so such a run's `state.json` is told right.
pub const DISK_STOP_REASON: &str = "the disk is nearly full and uploads are failing";

/// How often a sleeping tick looks at `done`, so a run that ends is not kept waiting on it.
const LOOK_EVERY: Duration = Duration::from_millis(100);

/// How long one tick waits for the daemon to say how the recording is. A daemon that is slow
/// is waited for this long and then read as not having answered.
pub const RECORDING_WAIT: Duration = crate::daemon::CALL_TIMEOUT;

/// What the daemon's status said to one tick. The default is no answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Heard {
    /// A status came back, signed for the entry.
    pub answered: bool,
    pub recording: Recording,
    /// The files the entry's admitted native reader has taken; `None` when the status does not
    /// say (no reader, or one that is not admitted).
    pub native_files: Option<u64>,
}

/// Asks the daemon how the recording is: `None` when it cannot say.
pub type AskRecording = Arc<dyn Fn() -> Option<Heard> + Send + Sync>;

/// When the tick calls the native leg silent, and the harness's words for the line it says.
pub struct NativeWatch {
    pub silence: super::NativeSilence,
    /// The harness's display name.
    pub harness: String,
    /// The harness's section of the setup guide.
    pub guide: String,
}

/// What the tick writes to `state.json` and shares with the heartbeat.
pub struct Beat {
    pub keeper: StateKeeper,
    pub vitals: RunSignals,
    pub ask: AskRecording,
    /// The run's time-left file the agent may read (D66), written each tick from the count in
    /// `state.json` and removed when the count says nothing. `None` writes none.
    pub time_left_file: Option<std::path::PathBuf>,
    /// Whether the native leg is watched for silence. `None` never says it.
    pub native: Option<NativeWatch>,
    /// When the harness has run this long with no model call recorded, the tick says so
    /// ([`NoCallsTick`]). `None` never says it.
    pub no_calls: Option<NoCallsWatch>,
}

/// When the tick calls a harness silent for want of any recorded model call, and the harness's
/// display name for the line it says.
pub struct NoCallsWatch {
    /// How long the harness may run with none recorded.
    pub after: Duration,
    /// The harness's display name.
    pub harness: String,
}

/// Ask for the recording, waiting at most `within` (and not past `done`). The question runs on
/// a thread of its own, so a daemon that does not answer holds nothing but its own call; no
/// answer in time is [`Recording::Unknown`] with no native count, never a verdict.
pub fn recording_within(within: Duration, done: &AtomicBool, ask: AskRecording) -> Heard {
    let (tell, answer) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("goalcup-recording".into())
        .spawn(move || {
            let _ = tell.send(ask());
        });
    if spawned.is_err() {
        return Heard::default();
    }
    let wake = Instant::now() + within;
    while Instant::now() < wake && !done.load(Ordering::SeqCst) {
        match answer.recv_timeout(LOOK_EVERY.min(wake.saturating_duration_since(Instant::now()))) {
            Ok(said) => return said.unwrap_or_default(),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Heard::default()
}

/// Everything the tick needs.
pub struct Watch {
    pub reserve: RunReserve,
    pub run: RunRecord,
    /// For each entry of a run with an upload server, the unix second of its last acknowledged
    /// upload ([`crate::upload::uploader::Running::last_ok`]), for `state.json`.
    pub uploads: Vec<Arc<AtomicU64>>,
    /// For each entry that is uploaded, its id and the bytes of its chain the server holds
    /// ([`crate::upload::uploader::Running::acked_offset`]). An entry not listed (a run with no
    /// server, an earlier segment) has nothing acknowledged: none of its parts is ever deleted.
    pub acked: Vec<(String, Arc<AtomicU64>)>,
    pub policy: DiskPolicy,
    /// The run's state file and what the tick tells the heartbeat. `None` writes nothing.
    pub beat: Option<Beat>,
    /// The agent's pause on a nearly full disk. `None` never pauses anything.
    pub pause: Option<AgentPause>,
}

/// A running tick. Ending it ([`Watching::end`]) hands the reserve back; dropping it stops the
/// tick and leaves the reserve with whoever drops it last (the file stays, its lock goes).
pub struct Watching {
    done: Arc<AtomicBool>,
    reserve: Option<Arc<Mutex<RunReserve>>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
    }
}

impl Watching {
    /// Stop the tick, wait for it, and hand the reserve back, whether or not the tick thread
    /// panicked: what becomes of the reserve is the run's end to decide.
    pub fn end(mut self) -> Option<RunReserve> {
        self.done.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            crate::host_log::say("disk: the disk watch stopped unexpectedly");
        }
        let reserve = self.reserve.take()?;
        Arc::try_unwrap(reserve)
            .ok()
            .map(|held| held.into_inner().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Start the tick on a thread of its own. It measures at once and then every `policy.tick`
/// until it is ended.
pub fn watch_beside(watch: Watch) -> Watching {
    let Watch {
        reserve,
        run,
        uploads,
        acked,
        policy,
        beat,
        pause,
    } = watch;
    let done = Arc::new(AtomicBool::new(false));
    let reserve = Arc::new(Mutex::new(reserve));
    let beside = (Arc::clone(&done), Arc::clone(&reserve));
    let mut ticker = Ticker {
        run,
        uploads,
        acked,
        policy,
        beat,
        pause,
        tick: Tick::default(),
        pruner: Pruner::within(policy.reserve_bytes),
        native: NativeTick::default(),
        no_calls: NoCallsTick::begin(Instant::now()),
    };
    let thread = std::thread::Builder::new()
        .name("goalcup-disk-watch".into())
        .spawn(move || {
            let (done, reserve) = beside;
            while !done.load(Ordering::SeqCst) {
                ticker.once(&reserve, &done);
                let wake = Instant::now() + policy.tick;
                while Instant::now() < wake && !done.load(Ordering::SeqCst) {
                    std::thread::sleep(
                        LOOK_EVERY.min(wake.saturating_duration_since(Instant::now())),
                    );
                }
            }
        });
    let thread = match thread {
        Ok(thread) => Some(thread),
        Err(error) => {
            crate::host_log::say(&format!("disk: the disk watch could not start ({error})"));
            None
        }
    };
    Watching {
        done,
        reserve: Some(reserve),
        thread,
    }
}

/// What the tick thread works with, and keeps from one tick to the next.
struct Ticker {
    run: RunRecord,
    uploads: Vec<Arc<AtomicU64>>,
    acked: Vec<(String, Arc<AtomicU64>)>,
    policy: DiskPolicy,
    beat: Option<Beat>,
    pause: Option<AgentPause>,
    tick: Tick,
    pruner: Pruner,
    native: NativeTick,
    no_calls: NoCallsTick,
}

impl Ticker {
    /// One tick: delete the parts that may go, look at the disk (resizing the reserve and
    /// following the pause, [`look_at_disk`]), and write the beat ([`Ticker::write_beat`]).
    fn once(&mut self, reserve: &Mutex<RunReserve>, done: &AtomicBool) {
        let held = || reserve.lock().unwrap_or_else(PoisonError::into_inner);
        let short = disk_short(&*held());
        let acked = &self.acked;
        let on_disk = self.pruner.sweep(
            &self.run.entries,
            &|entry| {
                acked
                    .iter()
                    .find(|(id, _)| id == entry)
                    .map_or(0, |(_, offset)| offset.load(Ordering::SeqCst))
            },
            Instant::now(),
            short,
            &mut crate::host_log::say,
        );
        look_at_disk(
            &mut self.tick,
            &mut *held(),
            &self.policy,
            on_disk,
            self.pause.as_ref(),
            &mut crate::host_log::say,
        );
        if let Some(beat) = self.beat.take() {
            self.write_beat(&beat, done);
            self.beat = Some(beat);
        }
    }

    /// Ask the daemon how the recording is, write it with the upload, the heartbeats and the
    /// disk to `state.json`, and keep the time-left file from the count written there.
    fn write_beat(&mut self, beat: &Beat, done: &AtomicBool) {
        let heard = recording_within(RECORDING_WAIT, done, beat.ask.clone());
        let recording = heard.recording;
        beat.vitals.set_recording(recording);
        // Said once, the way every other line of the tick is: in `host.log` while the harness
        // owns the terminal. `goalcup status` is where the participant reads it.
        let native = beat.native.as_ref().and_then(|watch| {
            self.native.see(
                watch,
                beat.vitals.recorded_requests(),
                heard,
                Instant::now(),
                &mut crate::host_log::say,
            )
        });
        // Once the harness has exited, the wait is over: a harness that left before the wait ran
        // out never gets the line, and what the tick last said stands.
        let exited = matches!(
            beat.keeper.snapshot().harness,
            super::HarnessState::Exited { .. }
        );
        let no_calls = beat.no_calls.as_ref().filter(|_| !exited).map(|watch| {
            self.no_calls.see(
                watch,
                // Model calls, not every request under the base path: a harness that only lists
                // models through the proxy has made no call Goal Cup could read.
                beat.vitals.generation_requests(),
                beat.vitals.client_tls_failures(),
                Instant::now(),
                &mut crate::host_log::say,
            )
        });
        let problem = beat.vitals.upload_problem();
        let refused = beat.vitals.server_refusing();
        let last_ok_at = self
            .uploads
            .iter()
            .map(|last| last.load(Ordering::SeqCst))
            .max();
        let mut disk = self.tick.disk();
        disk.disk_paused = self.pause.as_ref().is_some_and(AgentPause::is_paused);
        let heartbeat = beat.vitals.heartbeat_trouble();
        let time_left = beat.vitals.time_left();
        let mut count = None;
        beat.keeper.update(|state| {
            if time_left.is_some() {
                state.time_left = time_left;
            }
            count.clone_from(&state.time_left);
            state.recording = recording;
            state.heartbeat = heartbeat;
            state.upload = UploadState {
                last_ok_at,
                problem,
                // Only an older build's uploader gave up (D64).
                stopped: false,
                refused,
            };
            state.disk = disk;
            if let Some(silent) = native {
                state.native_silent = silent;
            }
            if let Some(silent) = no_calls {
                state.no_model_calls = silent;
            }
        });
        if let Some(file) = &beat.time_left_file
            && let Err(error) = super::time_left::keep_file(
                file,
                count.as_ref(),
                self.run.deadline_at.as_deref(),
                super::now_unix(),
                self.run.practice,
            )
        {
            crate::host_log::log_only(&format!(
                "time-left: the file could not be written ({error})"
            ));
        }
    }
}

/// What the tick keeps from one tick to the next about the native leg. It lives as long as one
/// `goalcup start` or `goalcup continue`, so each of them says the line at most once.
#[derive(Debug, Default)]
struct NativeTick {
    /// When the first recorded model request was seen: a harness writes its session at the
    /// first prompt, so the wait starts there, not at the launch.
    first_request: Option<Instant>,
    /// The native reader has taken a file: its leg is not silent, for good.
    seen: bool,
    /// The silent native leg has been said.
    said: bool,
}

impl NativeTick {
    /// Whether the native leg is silent at `now` ([`super::NativeSilence`]), after `requests`
    /// recorded model requests, with `heard` the daemon's answer: `Some(Some(requests))` when
    /// it is, `Some(None)` when it is not (or the daemon does not say, such as for a reader that
    /// is not admitted, whose loss the recording lines already say), and `None` when the
    /// daemon did not answer, which keeps what the state says. The first time it is silent it
    /// goes to `say`, once.
    fn see(
        &mut self,
        watch: &NativeWatch,
        requests: u64,
        heard: Heard,
        now: Instant,
        say: &mut dyn FnMut(&str),
    ) -> Option<Option<u64>> {
        if requests > 0 && self.first_request.is_none() {
            self.first_request = Some(now);
        }
        if !heard.answered {
            return None;
        }
        let Some(files) = heard.native_files else {
            return Some(None);
        };
        self.seen |= files > 0;
        let waited = self
            .first_request
            .map_or(Duration::ZERO, |first| now.saturating_duration_since(first));
        let silent = !self.seen && requests > 0 && watch.silence.holds(requests, waited, files);
        if silent && !self.said {
            self.said = true;
            say(&crate::participant::problem_native_silent(
                &watch.harness,
                requests,
                &watch.guide,
            ));
        }
        Some(silent.then_some(requests))
    }
}

/// What the tick keeps from one tick to the next about a harness whose model calls are not
/// recorded. It lives as long as one `goalcup start` or `goalcup continue`, and its wait starts
/// with the tick, so a continued segment is timed from its own launch.
#[derive(Debug)]
struct NoCallsTick {
    /// When the tick began: the harness is launched at about this time.
    began: Instant,
    /// The line has been said.
    said: bool,
}

impl NoCallsTick {
    fn begin(now: Instant) -> NoCallsTick {
        NoCallsTick {
            began: now,
            said: false,
        }
    }

    /// Whether the harness is silent at `now`: `Some(..)` when the tick began at least
    /// `watch.after` ago and `requests` (the model calls recorded) is zero. The first time it
    /// is, the line goes to `say`, once. It clears as soon as a request is recorded.
    fn see(
        &mut self,
        watch: &NoCallsWatch,
        requests: u64,
        client_tls: u64,
        now: Instant,
        say: &mut dyn FnMut(&str),
    ) -> Option<crate::run::NoModelCalls> {
        let waited = now.saturating_duration_since(self.began);
        if requests > 0 || waited < watch.after {
            return None;
        }
        let silent = crate::run::NoModelCalls {
            minutes: waited.as_secs() / 60,
            tls_failing: client_tls > 0,
        };
        if !self.said {
            self.said = true;
            // The restart words are `goalcup status`'s to give: it knows whether the run can be
            // continued. Here only that nothing is recorded, and why when the proxy knows.
            say(&crate::participant::problem_no_model_calls(
                &watch.harness,
                silent.minutes,
                silent.tls_failing,
                crate::participant::Restart::Cannot,
                true,
            ));
        }
        Some(silent)
    }
}

/// One look at the disk ([`Tick::step`]) and what the agent's pause makes of it. The free bytes
/// for the pause are read again on their own when the step could not finish (a resize that
/// failed), so trouble keeping the reservation never holds a pause. A disk that cannot be read
/// at all resumes a paused agent: nothing then says the disk is still full, and a pause must not
/// freeze the agent until the run ends. The next low reading pauses it again.
pub fn look_at_disk(
    tick: &mut Tick,
    disk: &mut dyn Disk,
    policy: &DiskPolicy,
    footprint: u64,
    pause: Option<&AgentPause>,
    say: &mut dyn FnMut(&str),
) {
    let stepped = tick.step(disk, policy, footprint, say);
    if let Some(pause) = pause {
        follow_disk(pause, stepped.or_else(|| disk.free().ok()), policy, say);
    }
}

/// One reading of the free disk beyond the reservation for the agent's pause; `None` when the
/// disk could not be read. The pause is said on the terminal even while a chat-mode harness owns
/// it (the harness is frozen and draws nothing); the resume goes to `say`, where every other
/// line goes.
pub fn follow_disk(
    pause: &AgentPause,
    free: Option<u64>,
    policy: &DiskPolicy,
    say: &mut dyn FnMut(&str),
) {
    let change = match free {
        Some(free) => pause.follow(free, policy),
        None => pause.follow_unread(),
    };
    match (change, free) {
        (Some(Change::Paused), _) => {
            crate::host_log::say_loud(crate::participant::problem_disk_paused())
        }
        (Some(Change::Resumed), Some(free)) => say(&crate::participant::disk_resumed(&human(free))),
        (Some(Change::Resumed), None) => say(crate::participant::disk_resumed_unread()),
        (None, _) => {}
    }
}

/// The most chain parts deleted from one entry in one sweep. A daemon that reopens the entry
/// lists its parts and opens them one by one, and lists once more when one has vanished
/// (D55); with deletions a whole tick apart, a second miss cannot happen.
///
/// One a tick is fast enough even when the disk is short: a part closes at 256 MiB or 30
/// minutes ([`goalcup_protocol::export::PART_MAX_BYTES`]), so a recording of 1 GB an hour makes
/// about four parts an hour, and one part every [`super::reserve::WATCH_PERIOD`] (30 s) deletes
/// up to 120 an hour, about 30 GiB, from each entry. What holds parts back is the server's
/// acknowledgement, not this pace.
const PARTS_PER_SWEEP: usize = 1;

/// Whether the disk under `disk` is short of space ([`short_of_disk`]): acknowledged parts then
/// go at once. A disk that cannot be read is not short: the parts keep their safety time, and
/// the tick says the trouble.
pub fn disk_short(disk: &dyn Disk) -> bool {
    disk.free()
        .is_ok_and(|free| short_of_disk(disk.held(), free))
}

/// Deletes the chain parts the server acknowledged [`crate::upload::prune::PRUNE_AFTER`] ago,
/// or at once while the parts alone are past the reservation or the disk is short
/// ([`disk_short`]).
pub struct Pruner {
    ages: AckAges,
    /// A part could not be deleted, and the participant has been told so.
    trouble_said: bool,
    /// The run's reservation ([`DiskPolicy::reserve_bytes`]): parts past it are under pressure.
    reserve_bytes: u64,
}

impl Default for Pruner {
    /// A pruner for the shipped reservation ([`super::reserve::RUN_RESERVE_BYTES`]).
    fn default() -> Self {
        Pruner::within(super::reserve::RUN_RESERVE_BYTES)
    }
}

impl Pruner {
    /// A pruner whose parts are under pressure past `reserve_bytes`.
    pub fn within(reserve_bytes: u64) -> Pruner {
        Pruner {
            ages: AckAges::default(),
            trouble_said: false,
            reserve_bytes,
        }
    }

    /// One sweep over `entries`: delete what [`deletable`] allows (at most
    /// [`PARTS_PER_SWEEP`] parts of each entry, oldest first, stopping at the first failure so
    /// that the parts left are always a contiguous suffix) and answer the chain bytes still on
    /// disk. `acked` gives an entry's acknowledged offset; `disk_short` ([`disk_short`]) lets
    /// acknowledged parts go at once. A failed delete is said once and never stops the run.
    pub fn sweep(
        &mut self,
        entries: &[EntryRef],
        acked: &dyn Fn(&str) -> u64,
        now: Instant,
        disk_short: bool,
        say: &mut dyn FnMut(&str),
    ) -> u64 {
        let listed: Vec<_> = entries
            .iter()
            .map(|entry| (entry, parts_now(&entry.trace_dir).ok()))
            .collect();
        let total: u64 = listed
            .iter()
            .flat_map(|(_, parts)| parts.iter().flatten())
            .map(|part| part.len)
            .sum();
        let under_pressure = disk_short || pressure(total, self.reserve_bytes);
        let mut left = total;
        let mut freed = 0u64;
        for (entry, parts) in &listed {
            // Parts that could not be listed this tick are left alone: they are looked at again
            // at the next one, and their ages are kept.
            let Some(parts) = parts else { continue };
            let due = deletable(
                &entry.entry_id,
                parts,
                acked(&entry.entry_id),
                &mut self.ages,
                now,
                under_pressure,
            );
            for path in due.into_iter().take(PARTS_PER_SWEEP) {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    // Gone already: the same as deleted.
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => {
                        if !std::mem::replace(&mut self.trouble_said, true) {
                            say(&format!(
                                "disk: {} could not be deleted ({error}); it was uploaded and \
                                 can be deleted by hand",
                                path.display()
                            ));
                        }
                        break;
                    }
                }
                self.trouble_said = false;
                if let Some(part) = parts.iter().find(|part| part.path == path) {
                    left = left.saturating_sub(part.len);
                    freed += part.len;
                }
            }
        }
        if freed > 0 {
            say(&if under_pressure {
                format!(
                    "disk: {} of the recording, already uploaded, was deleted from this machine \
                     to make room",
                    human(freed)
                )
            } else {
                format!(
                    "disk: {} of the recording, uploaded over two hours ago, was deleted from \
                     this machine",
                    human(freed)
                )
            });
        }
        left
    }
}

/// What the tick reads and changes: the reserve and the volume under it.
pub trait Disk {
    /// Bytes the reserve holds now.
    fn held(&self) -> u64;
    /// The volume's free bytes, beyond what the reserve holds.
    fn free(&self) -> io::Result<u64>;
    fn resize(&mut self, bytes: u64) -> io::Result<()>;
}

impl Disk for RunReserve {
    fn held(&self) -> u64 {
        self.allocated()
    }
    fn free(&self) -> io::Result<u64> {
        RunReserve::free(self)
    }
    fn resize(&mut self, bytes: u64) -> io::Result<()> {
        RunReserve::resize(self, bytes)
    }
}

/// What the ticks so far remember, so each thing is said once.
#[derive(Default)]
pub struct Tick {
    /// The last measurement, for `state.json`: low space is told there and by `goalcup status`,
    /// not on the terminal the agent is using.
    disk: DiskState,
    /// A measurement or resize failed, and the participant has been told so.
    trouble_said: bool,
}

impl Tick {
    /// The disk as the last look found it.
    pub fn disk(&self) -> DiskState {
        self.disk.clone()
    }

    /// One look at the disk. Resizes the reserve to what [`target_with`] says (giving back at
    /// once, taking back only when [`GROW_STEP`] is missing), reads what the reserve holds and
    /// what the volume has free again, and judges them. Trouble keeping the reservation goes to
    /// `say`; low space does not (it is [`Tick::disk`]). Answers the free bytes beyond the
    /// reserve, or `None` when the disk could not be read or the reserve could not be resized.
    /// It never stops the run (D65).
    pub fn step(
        &mut self,
        disk: &mut dyn Disk,
        policy: &DiskPolicy,
        footprint: u64,
        say: &mut dyn FnMut(&str),
    ) -> Option<u64> {
        let measured = disk.free().and_then(|free| {
            let held = disk.held();
            let wanted = target_with(policy, footprint, free, held);
            if wanted < held || wanted - held >= GROW_STEP {
                disk.resize(wanted).or_else(|error| match error.kind() {
                    // The disk filled between the two looks: the next tick gives back instead.
                    io::ErrorKind::StorageFull => Ok(()),
                    _ => Err(error),
                })?;
            }
            Ok((disk.held(), disk.free()?))
        });
        let (held, free) = match measured {
            Ok(reading) => reading,
            Err(error) => {
                if !std::mem::replace(&mut self.trouble_said, true) {
                    say(&format!(
                        "disk: the reservation could not be kept ({error})"
                    ));
                }
                return None;
            }
        };
        self.trouble_said = false;
        self.disk = DiskState {
            free_bytes: free,
            reserve_bytes: held,
            low: is_low(free),
            disk_paused: false,
        };
        Some(free)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::reserve::FREE_FLOOR;

    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;

    /// A volume with `used` bytes taken by others, `capacity` in all, and a reserve in it.
    struct Volume {
        capacity: u64,
        others: u64,
        reserve: u64,
        resizes: Vec<u64>,
        /// Free space lost after each resize: the agent's writes in between, or block
        /// accounting (extent-tree blocks) that the reserve's own size does not show.
        leak: u64,
    }

    impl Disk for Volume {
        fn held(&self) -> u64 {
            self.reserve
        }
        fn free(&self) -> io::Result<u64> {
            Ok(self.capacity - self.others - self.reserve)
        }
        fn resize(&mut self, bytes: u64) -> io::Result<()> {
            self.resizes.push(bytes);
            self.reserve = bytes;
            self.others += self.leak;
            Ok(())
        }
    }

    /// One tick: what it said, and the disk it found.
    fn run(volume: &mut Volume, footprint: u64) -> (Vec<String>, DiskState) {
        let mut said = Vec::new();
        let mut tick = Tick::default();
        tick.step(volume, &DiskPolicy::default(), footprint, &mut |l| {
            said.push(l.to_string())
        });
        (said, tick.disk())
    }

    #[test]
    fn the_reserve_follows_the_chain_on_a_roomy_disk() {
        let mut volume = Volume {
            capacity: 500 * GIB,
            others: 10 * GIB,
            reserve: 3 * GIB,
            resizes: Vec::new(),
            leak: 0,
        };
        let (said, disk) = run(&mut volume, GIB);
        assert!(said.is_empty() && !disk.low, "{said:?}");
        assert_eq!(volume.reserve, 2 * GIB);
        // Past 3 GiB of parts there is no reserve left, and the roomy disk is not a problem.
        let (said, disk) = run(&mut volume, 5 * GIB);
        assert!(said.is_empty() && !disk.low, "{said:?}");
        assert_eq!(volume.reserve, 0);
    }

    #[test]
    fn the_agent_filling_the_disk_draws_down_the_reserve() {
        // 100 MiB free, 2 GiB reserved: the reserve gives back and the disk is low.
        let mut volume = Volume {
            capacity: 20 * GIB,
            others: 20 * GIB - 2 * GIB - 100 * MIB,
            reserve: 2 * GIB,
            resizes: Vec::new(),
            leak: 0,
        };
        let (said, disk) = run(&mut volume, 0);
        assert_eq!(volume.reserve, 2 * GIB + 100 * MIB - FREE_FLOOR);
        assert_eq!(volume.free().unwrap(), FREE_FLOOR);
        // Low space is not said on the terminal: it is the state file's (and `status`'s).
        assert!(said.is_empty(), "{said:?}");
        assert_eq!(
            disk,
            DiskState {
                free_bytes: FREE_FLOOR,
                reserve_bytes: volume.reserve,
                low: true,
                disk_paused: false,
            }
        );
    }

    #[test]
    fn a_full_disk_never_stops_the_run() {
        // D65: the agent keeps filling whatever the reserve gives back. The reserve is drawn
        // down to nothing, each tick answers what is free, and nothing ever asks for a stop
        // (there is no stop to ask for: the tick has no stop channel).
        let mut volume = Volume {
            capacity: 20 * GIB,
            others: 0,
            reserve: GIB / 2,
            resizes: Vec::new(),
            leak: 0,
        };
        let mut tick = Tick::default();
        let mut said = Vec::new();
        for _ in 0..8 {
            volume.others = volume.capacity - volume.reserve - 10 * MIB;
            let free = tick.step(&mut volume, &DiskPolicy::default(), 0, &mut |l| {
                said.push(l.to_string())
            });
            assert_eq!(free, Some(volume.free().unwrap()));
        }
        assert_eq!(volume.reserve, 0);
        assert!(tick.disk().low && said.is_empty(), "{said:?}");
    }

    #[test]
    fn the_reserve_gives_back_at_once_and_grows_only_by_a_step() {
        let volume = |reserve: u64| Volume {
            capacity: 500 * GIB,
            others: 10 * GIB,
            reserve,
            resizes: Vec::new(),
            leak: 0,
        };
        let policy = DiskPolicy::default();
        let step = |volume: &mut Volume, footprint: u64| {
            Tick::default().step(volume, &policy, footprint, &mut |_| {});
        };
        // A byte too much is given back on the spot.
        let mut v = volume(3 * GIB);
        step(&mut v, 1);
        assert_eq!(v.resizes, vec![3 * GIB - 1]);
        // Space freed (parts removed) is not taken straight back for less than a step...
        let mut v = volume(3 * GIB - GROW_STEP + 1);
        step(&mut v, 0);
        assert!(v.resizes.is_empty(), "{:?}", v.resizes);
        // ...and is at a step.
        let mut v = volume(3 * GIB - GROW_STEP);
        step(&mut v, 0);
        assert_eq!(v.resizes, vec![3 * GIB]);
    }

    #[test]
    fn low_follows_each_reading_and_is_never_said_on_the_terminal() {
        let volume = |others: u64| Volume {
            capacity: 100 * GIB,
            others,
            reserve: 3 * GIB,
            resizes: Vec::new(),
            leak: 0,
        };
        let low = 100 * GIB - 3 * GIB - GIB;
        let mut tick = Tick::default();
        let mut said = Vec::new();
        let mut lows = Vec::new();
        for others in [low, low, 10 * GIB, low] {
            tick.step(&mut volume(others), &DiskPolicy::default(), 0, &mut |l| {
                said.push(l.to_string())
            });
            lows.push(tick.disk().low);
        }
        assert_eq!(lows, [true, true, false, true]);
        assert!(said.is_empty(), "{said:?}");
    }

    /// A disk whose readings and resizes can fail.
    struct Flaky {
        free: Option<u64>,
        resize_fails: bool,
        reserve: u64,
    }

    impl Disk for Flaky {
        fn held(&self) -> u64 {
            self.reserve
        }
        fn free(&self) -> io::Result<u64> {
            self.free
                .ok_or_else(|| io::ErrorKind::PermissionDenied.into())
        }
        fn resize(&mut self, _: u64) -> io::Result<()> {
            if self.resize_fails {
                Err(io::ErrorKind::PermissionDenied.into())
            } else {
                Ok(())
            }
        }
    }

    /// The signals a fake harness was sent.
    #[derive(Clone, Default)]
    struct Sent(Arc<Mutex<Vec<i32>>>);

    impl crate::run::pause::Freeze for Sent {
        fn send(&mut self, sig: i32) {
            self.0.lock().unwrap().push(sig);
        }
    }

    #[test]
    fn a_failed_resize_still_pauses_and_an_unreadable_disk_resumes() {
        let policy = DiskPolicy::default();
        let sent = Sent::default();
        let pause = AgentPause::new(sent.clone());
        let mut tick = Tick::default();
        let mut said = Vec::new();
        let mut look = |disk: &mut Flaky| {
            look_at_disk(&mut tick, disk, &policy, 0, Some(&pause), &mut |l| {
                said.push(l.to_string())
            })
        };
        // The resize fails, but the disk can be read and is nearly full: the agent is paused.
        let mut disk = Flaky {
            free: Some(100 * MIB),
            resize_fails: true,
            reserve: 2 * GIB,
        };
        look(&mut disk);
        assert!(pause.is_paused());
        assert_eq!(*sent.0.lock().unwrap(), [libc::SIGSTOP]);
        // A failed resize with room again resumes it.
        disk.free = Some(10 * GIB);
        look(&mut disk);
        assert!(!pause.is_paused());
        // Paused again, then the disk cannot be read at all: the agent is resumed, not left
        // frozen until the run ends.
        disk.free = Some(0);
        look(&mut disk);
        assert!(pause.is_paused());
        disk.free = None;
        look(&mut disk);
        assert!(!pause.is_paused());
        assert_eq!(
            *sent.0.lock().unwrap(),
            [libc::SIGSTOP, libc::SIGCONT, libc::SIGSTOP, libc::SIGCONT]
        );
        assert!(
            said.iter().any(|l| l.contains("could not read")),
            "{said:?}"
        );
    }

    #[test]
    fn a_tick_that_panics_still_hands_the_reserve_back() {
        let dir = tempfile::tempdir().unwrap();
        let policy = DiskPolicy {
            reserve_bytes: MIB,
            start_margin: 0,
            ..DiskPolicy::default()
        };
        let path = crate::run::reserve::reserve_path(dir.path(), "r1");
        let reserve = Arc::new(Mutex::new(RunReserve::take(&path, &policy).unwrap()));
        let beside = Arc::clone(&reserve);
        let thread = std::thread::spawn(move || {
            let _held = beside.lock().unwrap();
            panic!("the tick died while holding the reserve");
        });
        let watching = Watching {
            done: Arc::new(AtomicBool::new(false)),
            reserve: Some(reserve),
            thread: Some(thread),
        };
        let back = watching.end().expect("the reserve comes back");
        back.release().unwrap();
        assert!(!path.exists());
    }

    /// A trace directory of `n` parts of `len` bytes each, and its entry.
    fn entry_with_parts(dir: &std::path::Path, n: u64, len: u64) -> EntryRef {
        for i in 0..n {
            std::fs::write(
                dir.join(goalcup_protocol::export::part_file_name(i * len)),
                vec![b'x'; len as usize],
            )
            .unwrap();
        }
        EntryRef {
            entry_id: "e1".to_string(),
            trace_dir: dir.to_path_buf(),
        }
    }

    fn part_starts(dir: &std::path::Path) -> Vec<u64> {
        goalcup_protocol::export::chain_parts(dir)
            .unwrap()
            .iter()
            .map(|part| part.start)
            .collect()
    }

    #[test]
    fn the_sweep_waits_two_hours_then_deletes_one_part_a_tick_from_the_front() {
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_parts(dir.path(), 5, 10);
        let (mut pruner, t0) = (Pruner::default(), Instant::now());
        let acked = |_: &str| 45;
        let mut said = Vec::new();
        let mut sweep = |pruner: &mut Pruner, at: Instant| {
            pruner.sweep(std::slice::from_ref(&entry), &acked, at, false, &mut |l| {
                said.push(l.to_string())
            })
        };
        // Four parts end before byte 45, and the fifth is live. None is two hours old.
        assert_eq!(sweep(&mut pruner, t0), 50);
        assert_eq!(part_starts(dir.path()), [0, 10, 20, 30, 40]);
        // Two hours on: one part per tick, the oldest first, and the bytes left are answered.
        let due = t0 + crate::upload::prune::PRUNE_AFTER;
        assert_eq!(sweep(&mut pruner, due), 40);
        assert_eq!(part_starts(dir.path()), [10, 20, 30, 40]);
        assert_eq!(sweep(&mut pruner, due + Duration::from_secs(30)), 30);
        assert_eq!(part_starts(dir.path()), [20, 30, 40]);
        // The record ending at 45 is in the live part (40..50), which is never deleted: the
        // deletions go on until only it is left, and then nothing more.
        sweep(&mut pruner, due + Duration::from_secs(60));
        sweep(&mut pruner, due + Duration::from_secs(90));
        sweep(&mut pruner, due + Duration::from_secs(120));
        assert_eq!(part_starts(dir.path()), [40]);
        assert!(
            said.iter().all(|l| l.contains("deleted from this machine")),
            "{said:?}"
        );
    }

    #[test]
    fn an_entry_whose_parts_cannot_be_listed_keeps_its_clocks() {
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_parts(dir.path(), 3, 10);
        let gone = EntryRef {
            entry_id: "e1".to_string(),
            trace_dir: dir.path().join("moved"),
        };
        let (mut pruner, t0) = (Pruner::default(), Instant::now());
        let sweep = |pruner: &mut Pruner, entry: &EntryRef, at: Instant| {
            pruner.sweep(std::slice::from_ref(entry), &|_| 25, at, false, &mut |_| {})
        };
        sweep(&mut pruner, &entry, t0);
        // One tick where the directory cannot be listed must not restart the two hours.
        assert_eq!(sweep(&mut pruner, &gone, t0 + Duration::from_secs(3600)), 0);
        sweep(&mut pruner, &entry, t0 + crate::upload::prune::PRUNE_AFTER);
        assert_eq!(part_starts(dir.path()), [10, 20]);
    }

    #[test]
    fn the_deletion_line_says_why_the_parts_went() {
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_parts(dir.path(), 3, 10);
        let (mut pruner, t0) = (Pruner::default(), Instant::now());
        let mut said = Vec::new();
        let due = t0 + crate::upload::prune::PRUNE_AFTER;
        pruner.sweep(std::slice::from_ref(&entry), &|_| 25, t0, false, &mut |l| {
            said.push(l.to_string())
        });
        pruner.sweep(
            std::slice::from_ref(&entry),
            &|_| 25,
            due,
            false,
            &mut |l| said.push(l.to_string()),
        );
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("uploaded over two hours ago"), "{said:?}");
    }

    #[test]
    fn nothing_is_deleted_for_a_run_with_nothing_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_parts(dir.path(), 4, 10);
        let mut pruner = Pruner::default();
        let far = Instant::now() + Duration::from_secs(100 * 3600);
        for _ in 0..3 {
            let left = pruner.sweep(
                std::slice::from_ref(&entry),
                &|_| 0,
                far,
                false,
                &mut |_| {},
            );
            assert_eq!(left, 40);
        }
        assert_eq!(part_starts(dir.path()), [0, 10, 20, 30]);
    }

    #[test]
    fn parts_are_deleted_at_once_while_the_parts_are_past_the_reservation() {
        use crate::run::reserve::RUN_RESERVE_BYTES;
        let dir = tempfile::tempdir().unwrap();
        // Two sparse parts of 2 GiB each and a small live part: 4 GiB on the books.
        let big = RUN_RESERVE_BYTES * 2 / 3;
        for (start, len) in [(0, big), (big, big), (2 * big, 10)] {
            let file = std::fs::File::create(
                dir.path()
                    .join(goalcup_protocol::export::part_file_name(start)),
            )
            .unwrap();
            file.set_len(len).unwrap();
        }
        let entry = EntryRef {
            entry_id: "e1".to_string(),
            trace_dir: dir.path().to_path_buf(),
        };
        let mut pruner = Pruner::default();
        let mut said = Vec::new();
        let left = pruner.sweep(
            std::slice::from_ref(&entry),
            &|_| 3 * big,
            Instant::now(),
            false,
            &mut |l| said.push(l.to_string()),
        );
        assert_eq!(
            left,
            big + 10,
            "young, yet deleted: the parts are past the reservation"
        );
        assert_eq!(part_starts(dir.path()), [big, 2 * big]);
        assert!(said.iter().any(|l| l.contains("to make room")), "{said:?}");
    }

    /// A 100 GiB volume with `free` bytes free beyond a reserve that holds `reserve`.
    fn volume_with(free: u64, reserve: u64) -> Volume {
        Volume {
            capacity: 100 * GIB,
            others: 100 * GIB - free - reserve,
            reserve,
            resizes: Vec::new(),
            leak: 0,
        }
    }

    /// One sweep of `entry` with `acked` bytes acknowledged, judging the disk by `volume`.
    fn sweep_on(pruner: &mut Pruner, entry: &EntryRef, acked: u64, at: Instant, volume: &Volume) {
        pruner.sweep(
            std::slice::from_ref(entry),
            &|_| acked,
            at,
            disk_short(volume),
            &mut |_| {},
        );
    }

    #[test]
    fn a_disk_short_of_space_deletes_acknowledged_parts_before_two_hours() {
        // 50 bytes of chain, far under the 3 GiB reservation: only the disk calls for the room.
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_parts(dir.path(), 5, 10);
        let (mut pruner, t0) = (Pruner::default(), Instant::now());
        let tick = Duration::from_secs(30);
        // 4 GiB free beyond a whole reserve: the parts acknowledged a moment ago go, one a tick.
        let low = volume_with(4 * GIB, 3 * GIB);
        sweep_on(&mut pruner, &entry, 45, t0, &low);
        assert_eq!(part_starts(dir.path()), [10, 20, 30, 40]);
        sweep_on(&mut pruner, &entry, 45, t0 + tick, &low);
        assert_eq!(part_starts(dir.path()), [20, 30, 40]);
        // Space came back, but the reserve was drawn down to nothing: still short.
        let drawn = volume_with(50 * GIB, 0);
        sweep_on(&mut pruner, &entry, 45, t0 + 2 * tick, &drawn);
        assert_eq!(part_starts(dir.path()), [30, 40]);
    }

    #[test]
    fn a_comfortable_disk_keeps_acknowledged_parts_for_two_hours() {
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_parts(dir.path(), 5, 10);
        let (mut pruner, t0) = (Pruner::default(), Instant::now());
        let roomy = volume_with(5 * GIB, 3 * GIB);
        let due = t0 + crate::upload::prune::PRUNE_AFTER;
        for at in [
            t0,
            t0 + Duration::from_secs(3600),
            due - Duration::from_secs(1),
        ] {
            sweep_on(&mut pruner, &entry, 45, at, &roomy);
            assert_eq!(part_starts(dir.path()), [0, 10, 20, 30, 40]);
        }
        sweep_on(&mut pruner, &entry, 45, due, &roomy);
        assert_eq!(part_starts(dir.path()), [10, 20, 30, 40]);
    }

    #[test]
    fn a_short_disk_never_takes_the_live_part_or_the_last_acknowledged_record() {
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_parts(dir.path(), 5, 10);
        let (mut pruner, t0) = (Pruner::default(), Instant::now());
        let full = volume_with(0, 0);
        // The server holds bytes 0..30: the last acknowledged record ends part 20..30.
        for i in 0..6 {
            sweep_on(
                &mut pruner,
                &entry,
                30,
                t0 + Duration::from_secs(30 * i),
                &full,
            );
        }
        assert_eq!(part_starts(dir.path()), [20, 30, 40]);
        // The server holds everything: only the live part, which also holds that record, stays.
        for i in 6..12 {
            sweep_on(
                &mut pruner,
                &entry,
                50,
                t0 + Duration::from_secs(30 * i),
                &full,
            );
        }
        assert_eq!(part_starts(dir.path()), [40]);
    }

    #[test]
    fn a_disk_that_cannot_be_read_is_not_short() {
        struct Unreadable;
        impl Disk for Unreadable {
            fn held(&self) -> u64 {
                0
            }
            fn free(&self) -> io::Result<u64> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
            fn resize(&mut self, _: u64) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(!disk_short(&Unreadable));
    }

    /// The pressure is the run's own reservation's: a smaller one (a test build's) deletes the
    /// acknowledged parts at once as soon as the parts are past it.
    #[test]
    fn the_pressure_is_past_the_runs_own_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_parts(dir.path(), 4, 10);
        let sweep = |pruner: &mut Pruner| {
            pruner.sweep(
                std::slice::from_ref(&entry),
                &|_| 35,
                Instant::now(),
                false,
                &mut |_| {},
            )
        };
        // 40 bytes of parts are far below the shipped reservation: nothing goes yet.
        sweep(&mut Pruner::default());
        assert_eq!(part_starts(dir.path()), [0, 10, 20, 30]);
        // Past a 25-byte one, one acknowledged part goes each tick until the parts are not.
        let mut pruner = Pruner::within(25);
        assert_eq!(sweep(&mut pruner), 30);
        assert_eq!(sweep(&mut pruner), 20);
        assert_eq!(sweep(&mut pruner), 20);
        assert_eq!(part_starts(dir.path()), [20, 30]);
    }

    #[cfg(unix)]
    #[test]
    fn a_part_that_cannot_be_deleted_is_said_once_and_stops_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_parts(dir.path(), 3, 10);
        let mode =
            |bits| std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(bits));
        mode(0o555).unwrap();
        if std::fs::File::create(dir.path().join("probe")).is_ok() {
            return; // running as a user the mode does not bind
        }
        let (mut pruner, mut said) = (Pruner::default(), Vec::new());
        let t0 = Instant::now();
        let due = t0 + crate::upload::prune::PRUNE_AFTER;
        // The first sweep starts the clocks; the later ones find the first part due and fail.
        for at in [t0, due, due, due] {
            let left = pruner.sweep(std::slice::from_ref(&entry), &|_| 25, at, false, &mut |l| {
                said.push(l.to_string())
            });
            assert_eq!(left, 30, "nothing was deleted, so nothing is subtracted");
        }
        mode(0o755).unwrap();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("could not be deleted"), "{said:?}");
        assert_eq!(part_starts(dir.path()), [0, 10, 20]);
        // Once the directory allows it, the same part goes at the next tick, and the trouble
        // is forgotten.
        pruner.sweep(
            std::slice::from_ref(&entry),
            &|_| 25,
            due,
            false,
            &mut |l| said.push(l.to_string()),
        );
        assert_eq!(part_starts(dir.path()), [10, 20]);
    }

    #[test]
    fn a_daemon_that_does_not_answer_is_unknown_not_failed() {
        let done = AtomicBool::new(false);
        let ask = |answer: fn() -> Option<Heard>| -> AskRecording { Arc::new(answer) };
        let said = |recording| Heard {
            answered: true,
            recording,
            native_files: None,
        };
        // Nobody to ask, or no answer: not a verdict.
        assert_eq!(
            recording_within(Duration::from_secs(5), &done, ask(|| None)).recording,
            Recording::Unknown
        );
        // A daemon that is slow is waited for, but only so long.
        let slow: AskRecording = Arc::new(move || {
            std::thread::sleep(Duration::from_secs(3));
            Some(said(Recording::Failed))
        });
        let began = Instant::now();
        assert_eq!(
            recording_within(Duration::from_millis(200), &done, slow.clone()).recording,
            Recording::Unknown
        );
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "{:?}",
            began.elapsed()
        );
        // A run that ends does not wait for the answer at all.
        let ended = AtomicBool::new(true);
        let began = Instant::now();
        assert_eq!(
            recording_within(Duration::from_secs(60), &ended, slow).recording,
            Recording::Unknown
        );
        assert!(began.elapsed() < Duration::from_secs(2));
        // What the daemon says is what is recorded.
        for (answered, recording) in [
            (Recording::Ok, Recording::Ok),
            (Recording::Gaps, Recording::Gaps),
            (Recording::Failed, Recording::Failed),
        ] {
            let answer: AskRecording = Arc::new(move || Some(said(answered)));
            assert_eq!(
                recording_within(Duration::from_secs(5), &done, answer).recording,
                recording
            );
        }
    }

    /// The native leg is called silent only past both thresholds with no file admitted, said
    /// once however many ticks it lasts, and cleared for good once a file is admitted; a tick
    /// with no count from the daemon changes nothing.
    #[test]
    fn a_silent_native_leg_is_said_once_and_cleared_by_a_session_file() {
        let watch = NativeWatch {
            silence: crate::run::NativeSilence {
                requests: 3,
                after: Duration::from_secs(60),
            },
            harness: "Some Agent".to_string(),
            guide: "https://example.test/guide#some-agent".to_string(),
        };
        let count = |files| Heard {
            answered: true,
            recording: Recording::Ok,
            native_files: files,
        };
        let mut said = Vec::new();
        let mut say = |line: &str| said.push(line.to_string());
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(61);
        let mut tick = NativeTick::default();
        // No recorded request yet: the wait has not begun, however long the agent idles.
        assert_eq!(
            tick.see(&watch, 0, count(Some(0)), later, &mut say),
            Some(None)
        );
        // The first request starts the wait: too early, then too few requests.
        assert_eq!(
            tick.see(&watch, 5, count(Some(0)), t0, &mut say),
            Some(None)
        );
        let mut few = NativeTick::default();
        few.see(&watch, 1, count(Some(0)), t0, &mut say);
        assert_eq!(
            few.see(&watch, 2, count(Some(0)), later, &mut say),
            Some(None)
        );
        // Past both: silent, with the requests so far, on every tick; said once.
        assert_eq!(
            tick.see(&watch, 6, count(Some(0)), later, &mut say),
            Some(Some(6))
        );
        assert_eq!(
            tick.see(&watch, 7, count(Some(0)), later, &mut say),
            Some(Some(7))
        );
        // No answer keeps what the state says; an answer with no admitted reader clears it.
        assert_eq!(tick.see(&watch, 8, Heard::default(), later, &mut say), None);
        assert_eq!(
            tick.see(&watch, 8, count(None), later, &mut say),
            Some(None)
        );
        // A session file clears it, and it does not come back.
        assert_eq!(
            tick.see(&watch, 9, count(Some(1)), later, &mut say),
            Some(None)
        );
        assert_eq!(
            tick.see(&watch, 20, count(Some(0)), later, &mut say),
            Some(None)
        );
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("6 "), "{said:?}");
        assert!(said[0].contains("Some Agent"), "{said:?}");
        assert!(said[0].ends_with(&watch.guide), "{said:?}");
    }

    /// A harness with no recorded model call is called silent once it has run for the wait, and
    /// not before; one recorded request clears it; the line is said once, with the TLS sentence
    /// only when the proxy has seen the harness fail its handshake.
    #[test]
    fn no_recorded_model_call_is_called_silent_after_the_wait() {
        let watch = NoCallsWatch {
            after: Duration::from_secs(180),
            harness: "Some Agent".to_string(),
        };
        let t0 = Instant::now();
        let mut said = Vec::new();
        let mut say = |line: &str| said.push(line.to_string());
        let mut tick = NoCallsTick::begin(t0);
        assert_eq!(
            tick.see(&watch, 0, 0, t0 + Duration::from_secs(179), &mut say),
            None,
            "too early"
        );
        let at = t0 + Duration::from_secs(185);
        let silent = tick.see(&watch, 0, 0, at, &mut say).expect("silent");
        assert_eq!(silent.minutes, 3);
        assert!(!silent.tls_failing);
        assert_eq!(
            tick.see(&watch, 1, 0, at, &mut say),
            None,
            "one request clears it"
        );
        // Failed handshakes show in the verdict, and the line is said once however many ticks.
        let later = t0 + Duration::from_secs(250);
        assert!(
            tick.see(&watch, 0, 5, later, &mut say)
                .expect("silent")
                .tls_failing
        );
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("Some Agent"), "{said:?}");
        assert!(said[0].contains("3 minutes"), "{said:?}");
    }

    #[test]
    fn the_daemons_capture_states_map_to_recording() {
        use goalcup_protocol::capture_status::CaptureState;
        assert_eq!(
            Recording::of_capture(CaptureState::Recording),
            Recording::Ok
        );
        assert_eq!(
            Recording::of_capture(CaptureState::Incomplete),
            Recording::Gaps
        );
        assert_eq!(
            Recording::of_capture(CaptureState::Failed),
            Recording::Failed
        );
        assert_eq!(
            Recording::of_capture(CaptureState::Starting),
            Recording::Unknown
        );
    }
}
