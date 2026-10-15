//! The watch tick: while a run records and while its upload wraps up, every
//! [`super::reserve::WATCH_PERIOD`] it deletes the chain parts the server acknowledged long
//! ago ([`Pruner`]), measures the disk, resizes the run's reserve, and stops the run cleanly if
//! the disk is full and uploads are failing.
//!
//! The tick runs on a thread of its own, started before the wait for the harness and ended just
//! before the reserve is released ([`Watching::end`]). It never touches the recording; its only
//! way to end a run is to send on the run's one stop channel, which takes the same path as the
//! run's deadline (the daemon seals, the uploader sends the seal). After the harness has gone
//! nothing listens on that channel and the tick only keeps the reserve sized.
//!
//! Each tick also asks the daemon how the recording is (a bounded wait: no answer is
//! [`Recording::Unknown`], never a failure), and writes the run's `state.json` ([`Beat`]).
//! That is where the disk's low-space warning, the upload's trouble and the recording's gaps
//! are said; `goalcup status` reads them back.

use super::reserve::{
    DiskPolicy, DiskVerdict, GROW_STEP, RunReserve, UPLOAD_FAILING_AFTER, human, pressure,
    target_with, verdict_with,
};
use super::{DiskState, EntryRef, Recording, RunRecord, StateKeeper, UploadState, Vitals};
use crate::upload::prune::{AckAges, deletable, parts_now};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What is said when the disk ends a run.
pub const DISK_STOP_REASON: &str = "the disk is nearly full and uploads are failing";

/// How often a sleeping tick looks at `done`, so a run that ends is not kept waiting on it.
const LOOK_EVERY: Duration = Duration::from_millis(100);

/// How long one tick waits for the daemon to say how the recording is. A daemon that is slow
/// is waited for this long and then read as not having answered.
pub const RECORDING_WAIT: Duration = crate::daemon::CALL_TIMEOUT;

/// Asks the daemon how the recording is: `None` when it cannot say.
pub type AskRecording = Arc<dyn Fn() -> Option<Recording> + Send + Sync>;

/// What the tick writes to `state.json` and shares with the heartbeat.
pub struct Beat {
    pub keeper: StateKeeper,
    pub vitals: Vitals,
    pub ask: AskRecording,
}

/// Ask for the recording, waiting at most `within` (and not past `done`). The question runs on
/// a thread of its own, so a daemon that does not answer holds nothing but its own call; no
/// answer in time is [`Recording::Unknown`], never a verdict.
pub fn recording_within(within: Duration, done: &AtomicBool, ask: AskRecording) -> Recording {
    let (tell, answer) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("goalcup-recording".into())
        .spawn(move || {
            let _ = tell.send(ask());
        });
    if spawned.is_err() {
        return Recording::Unknown;
    }
    let wake = Instant::now() + within;
    while Instant::now() < wake && !done.load(Ordering::SeqCst) {
        match answer.recv_timeout(LOOK_EVERY.min(wake.saturating_duration_since(Instant::now()))) {
            Ok(said) => return said.unwrap_or(Recording::Unknown),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Recording::Unknown
}

/// Everything the tick needs.
pub struct Watch {
    pub reserve: RunReserve,
    pub run: RunRecord,
    /// The run's one stop channel: the same one the uploader's deadline stop sends on.
    pub stop: Sender<String>,
    /// For each entry of a run with an upload server, the unix second of its last acknowledged
    /// upload ([`crate::upload::uploader::Running::last_ok`]). Empty when nothing is being
    /// uploaded (a local run, or an upload that could not start): nothing will ever free space
    /// then, so uploads count as failing.
    pub uploads: Vec<Arc<AtomicU64>>,
    /// For each entry that is uploaded, its id and the bytes of its chain the server holds
    /// ([`crate::upload::uploader::Running::acked_offset`]). An entry not listed (a run with no
    /// server, an earlier segment) has nothing acknowledged: none of its parts is ever deleted.
    pub acked: Vec<(String, Arc<AtomicU64>)>,
    pub policy: DiskPolicy,
    /// The run's state file and what the tick tells the heartbeat. `None` writes nothing.
    pub beat: Option<Beat>,
}

/// Whether no upload was acknowledged for [`UPLOAD_FAILING_AFTER`] at `now` (unix seconds).
pub fn uploads_failing(uploads: &[Arc<AtomicU64>], now: u64) -> bool {
    uploads.iter().all(|last| {
        now.saturating_sub(last.load(Ordering::SeqCst)) >= UPLOAD_FAILING_AFTER.as_secs()
    })
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
        stop,
        uploads,
        acked,
        policy,
        beat,
    } = watch;
    let done = Arc::new(AtomicBool::new(false));
    let reserve = Arc::new(Mutex::new(reserve));
    let beside = (Arc::clone(&done), Arc::clone(&reserve));
    let thread = std::thread::Builder::new()
        .name("goalcup-disk-watch".into())
        .spawn(move || {
            let (done, reserve) = beside;
            let mut tick = Tick::default();
            let mut pruner = Pruner::within(policy.reserve_bytes);
            while !done.load(Ordering::SeqCst) {
                let failing = uploads_failing(&uploads, super::now_unix());
                let on_disk = pruner.sweep(
                    &run.entries,
                    &|entry| {
                        acked
                            .iter()
                            .find(|(id, _)| id == entry)
                            .map_or(0, |(_, offset)| offset.load(Ordering::SeqCst))
                    },
                    Instant::now(),
                    &mut crate::host_log::say,
                );
                let stop_now = tick.step(
                    &mut *reserve.lock().unwrap_or_else(PoisonError::into_inner),
                    &policy,
                    on_disk,
                    failing,
                    &mut crate::host_log::say,
                );
                if stop_now {
                    let _ = stop.send(DISK_STOP_REASON.to_string());
                }
                if let Some(beat) = &beat {
                    let recording = recording_within(RECORDING_WAIT, &done, beat.ask.clone());
                    beat.vitals.set_recording(recording);
                    let (problem, stopped) = beat.vitals.upload_problem();
                    let last_ok_at = uploads.iter().map(|last| last.load(Ordering::SeqCst)).max();
                    let disk = tick.disk();
                    let heartbeat = beat.vitals.heartbeat_trouble();
                    let time_left = beat.vitals.time_left();
                    beat.keeper.update(|state| {
                        if time_left.is_some() {
                            state.time_left = time_left;
                        }
                        state.recording = recording;
                        state.heartbeat = heartbeat;
                        state.upload = UploadState {
                            last_ok_at,
                            problem,
                            stopped,
                        };
                        state.disk = disk;
                    });
                }
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

/// The most chain parts deleted from one entry in one sweep. A daemon that reopens the entry
/// lists its parts and opens them one by one, and lists once more when one has vanished
/// (D55); with deletions a whole tick apart, a second miss cannot happen.
const PARTS_PER_SWEEP: usize = 1;

/// Deletes the chain parts the server acknowledged [`crate::upload::prune::PRUNE_AFTER`] ago,
/// or at once while the parts alone are past the reservation.
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
    /// disk. `acked` gives an entry's acknowledged offset. A failed delete is said once and
    /// never stops the run.
    pub fn sweep(
        &mut self,
        entries: &[EntryRef],
        acked: &dyn Fn(&str) -> u64,
        now: Instant,
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
        let under_pressure = pressure(total, self.reserve_bytes);
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
    /// The stop has been asked for.
    stopped: bool,
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
    /// `say`; low space does not (it is [`Tick::disk`]).
    /// Answers `true` exactly once: the reserve is drawn down to nothing, the disk is full,
    /// uploads are failing, and the run must stop.
    pub fn step(
        &mut self,
        disk: &mut dyn Disk,
        policy: &DiskPolicy,
        footprint: u64,
        uploads_failing: bool,
        say: &mut dyn FnMut(&str),
    ) -> bool {
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
                return false;
            }
        };
        self.trouble_said = false;
        let verdict = verdict_with(policy, footprint, held, free, uploads_failing);
        self.disk = DiskState {
            free_bytes: free,
            reserve_bytes: held,
            low: matches!(verdict, DiskVerdict::Low { .. }),
        };
        match verdict {
            DiskVerdict::Ok | DiskVerdict::Low { .. } => false,
            DiskVerdict::Stop => !std::mem::replace(&mut self.stopped, true),
        }
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

    /// One tick: whether it stops the run, what it said, and the disk it found.
    fn run(volume: &mut Volume, footprint: u64, failing: bool) -> (bool, Vec<String>, DiskState) {
        let mut said = Vec::new();
        let mut tick = Tick::default();
        let stop = tick.step(
            volume,
            &DiskPolicy::default(),
            footprint,
            failing,
            &mut |l| said.push(l.to_string()),
        );
        (stop, said, tick.disk())
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
        let (stop, said, disk) = run(&mut volume, GIB, true);
        assert!(!stop && said.is_empty() && !disk.low, "{said:?}");
        assert_eq!(volume.reserve, 2 * GIB);
        // Past 3 GiB of parts there is no reserve left, and the roomy disk is not a problem.
        let (stop, said, disk) = run(&mut volume, 5 * GIB, true);
        assert!(!stop && said.is_empty() && !disk.low, "{said:?}");
        assert_eq!(volume.reserve, 0);
    }

    #[test]
    fn the_agent_filling_the_disk_draws_down_the_reserve_before_any_stop() {
        // 100 MiB free, 2 GiB reserved, uploads failing: the reserve gives back, the disk is
        // low, the run is not stopped.
        let mut volume = Volume {
            capacity: 20 * GIB,
            others: 20 * GIB - 2 * GIB - 100 * MIB,
            reserve: 2 * GIB,
            resizes: Vec::new(),
            leak: 0,
        };
        let (stop, said, disk) = run(&mut volume, 0, true);
        assert!(!stop);
        assert_eq!(volume.reserve, 2 * GIB + 100 * MIB - FREE_FLOOR);
        assert_eq!(volume.free().unwrap(), FREE_FLOOR);
        // Low space is not said on the terminal: it is the state file's (and `status`'s).
        assert!(said.is_empty(), "{said:?}");
        assert_eq!(
            disk,
            DiskState {
                free_bytes: FREE_FLOOR,
                reserve_bytes: volume.reserve,
                low: true
            }
        );
    }

    #[test]
    fn a_full_disk_with_failing_uploads_stops_once() {
        let mut volume = Volume {
            capacity: 20 * GIB,
            others: 0,
            reserve: GIB / 2,
            resizes: Vec::new(),
            leak: 0,
        };
        let mut tick = Tick::default();
        let mut said = Vec::new();
        let mut stops = Vec::new();
        for round in 0..8 {
            // The agent keeps filling whatever the reserve gave back.
            volume.others = volume.capacity - volume.reserve - 10 * MIB;
            if tick.step(&mut volume, &DiskPolicy::default(), 0, true, &mut |l| {
                said.push(l.to_string())
            }) {
                stops.push((round, volume.reserve));
            }
        }
        // Not at the first full reading: the reserve gives back, round after round, and the
        // run stops only when it has nothing left. And then it is asked once.
        assert_eq!(stops.len(), 1, "{stops:?}");
        assert!(stops[0].0 > 0, "{stops:?}");
        assert_eq!(stops[0].1, 0, "{stops:?}");
    }

    #[test]
    fn free_space_lost_at_each_resize_never_stops_a_run_that_still_holds_a_reserve() {
        // A few KiB of free space go missing after every give-back, so each reading is just under
        // the floor while the reserve still holds hundreds of MiB.
        let mut volume = Volume {
            capacity: 20 * GIB,
            others: 20 * GIB - 2 * GIB - 100 * MIB,
            reserve: 2 * GIB,
            resizes: Vec::new(),
            leak: 4096,
        };
        let mut tick = Tick::default();
        let mut said = Vec::new();
        for _ in 0..6 {
            let stop = tick.step(&mut volume, &DiskPolicy::default(), 0, true, &mut |l| {
                said.push(l.to_string())
            });
            assert!(!stop, "stopped holding {} bytes", volume.reserve);
            assert!(volume.reserve > 0);
        }
        // The agent now takes everything the reserve gives back: it is drawn down, and only
        // then does the run stop, once.
        let mut stops = Vec::new();
        for _ in 0..8 {
            volume.others = volume.capacity - volume.reserve - 10 * MIB;
            if tick.step(&mut volume, &DiskPolicy::default(), 0, true, &mut |l| {
                said.push(l.to_string())
            }) {
                stops.push(volume.reserve);
            }
        }
        assert_eq!(stops, vec![0]);
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
            Tick::default().step(volume, &policy, footprint, false, &mut |_| {});
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
    fn a_full_disk_with_working_uploads_does_not_stop() {
        let mut volume = Volume {
            capacity: 20 * GIB,
            others: 20 * GIB - 10 * MIB,
            reserve: 0,
            resizes: Vec::new(),
            leak: 0,
        };
        let (stop, said, disk) = run(&mut volume, 4 * GIB, false);
        assert!(!stop);
        assert!(said.is_empty() && disk.low, "{said:?} {disk:?}");
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
            tick.step(
                &mut volume(others),
                &DiskPolicy::default(),
                0,
                false,
                &mut |l| said.push(l.to_string()),
            );
            lows.push(tick.disk().low);
        }
        assert_eq!(lows, [true, true, false, true]);
        assert!(said.is_empty(), "{said:?}");
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
            pruner.sweep(std::slice::from_ref(&entry), &acked, at, &mut |l| {
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
            pruner.sweep(std::slice::from_ref(entry), &|_| 25, at, &mut |_| {})
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
        pruner.sweep(std::slice::from_ref(&entry), &|_| 25, t0, &mut |l| {
            said.push(l.to_string())
        });
        pruner.sweep(std::slice::from_ref(&entry), &|_| 25, due, &mut |l| {
            said.push(l.to_string())
        });
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
            let left = pruner.sweep(std::slice::from_ref(&entry), &|_| 0, far, &mut |_| {});
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
            let left = pruner.sweep(std::slice::from_ref(&entry), &|_| 25, at, &mut |l| {
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
        pruner.sweep(std::slice::from_ref(&entry), &|_| 25, due, &mut |l| {
            said.push(l.to_string())
        });
        assert_eq!(part_starts(dir.path()), [10, 20]);
    }

    #[test]
    fn a_daemon_that_does_not_answer_is_unknown_not_failed() {
        let done = AtomicBool::new(false);
        let ask = |answer: fn() -> Option<Recording>| -> AskRecording { Arc::new(answer) };
        // Nobody to ask, or no answer: not a verdict.
        assert_eq!(
            recording_within(Duration::from_secs(5), &done, ask(|| None)),
            Recording::Unknown
        );
        // A daemon that is slow is waited for, but only so long.
        let slow: AskRecording = Arc::new(|| {
            std::thread::sleep(Duration::from_secs(3));
            Some(Recording::Failed)
        });
        let began = Instant::now();
        assert_eq!(
            recording_within(Duration::from_millis(200), &done, slow.clone()),
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
            recording_within(Duration::from_secs(60), &ended, slow),
            Recording::Unknown
        );
        assert!(began.elapsed() < Duration::from_secs(2));
        // What the daemon says is what is recorded.
        for (said, recording) in [
            (Recording::Ok, Recording::Ok),
            (Recording::Gaps, Recording::Gaps),
            (Recording::Failed, Recording::Failed),
        ] {
            let answer: AskRecording = Arc::new(move || Some(said));
            assert_eq!(
                recording_within(Duration::from_secs(5), &done, answer),
                recording
            );
        }
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

    #[test]
    fn uploads_are_failing_after_half_an_hour_without_an_acknowledgement() {
        let now = 10_000;
        let at = |secs: u64| Arc::new(AtomicU64::new(secs));
        assert!(
            uploads_failing(&[], now),
            "nothing uploads: nothing frees space"
        );
        assert!(!uploads_failing(&[at(now - 60)], now));
        assert!(!uploads_failing(&[at(now - 1799)], now));
        assert!(uploads_failing(&[at(now - 1800)], now));
        // A later segment that is acknowledging keeps the run's uploads working.
        assert!(!uploads_failing(&[at(0), at(now - 5)], now));
    }
}
