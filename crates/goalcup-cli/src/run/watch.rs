//! The watch tick: while a run records, every [`super::reserve::WATCH_PERIOD`] it measures the
//! disk, resizes the run's reserve, and stops the run cleanly if the disk is full and uploads
//! are failing.
//!
//! The tick runs on a thread beside the harness's wait. It never touches the recording; its
//! only way to end a run is to send on the run's one stop channel, which takes the same path as
//! the run's deadline (the daemon seals, the uploader sends the seal).

use super::RunRecord;
use super::reserve::{
    DiskPolicy, DiskVerdict, RunReserve, UPLOAD_FAILING_AFTER, footprint, human, target_with,
    verdict_with,
};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

/// What is said when the disk ends a run.
pub const DISK_STOP_REASON: &str = "the disk is nearly full and uploads are failing";

/// How often a sleeping tick looks at `done`, so a run that ends is not kept waiting on it.
const LOOK_EVERY: Duration = Duration::from_millis(100);

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
    /// Set when the harness has gone and the recording is over.
    pub done: Arc<AtomicBool>,
    pub policy: DiskPolicy,
}

/// Whether no upload was acknowledged for [`UPLOAD_FAILING_AFTER`] at `now` (unix seconds).
pub fn uploads_failing(uploads: &[Arc<AtomicU64>], now: u64) -> bool {
    uploads.iter().all(|last| {
        now.saturating_sub(last.load(Ordering::SeqCst)) >= UPLOAD_FAILING_AFTER.as_secs()
    })
}

/// Start the tick on `scope`. It measures at once and then every `policy.tick` until `done`,
/// and hands the reserve back when it ends: what becomes of the reserve is the run's end to
/// decide.
pub fn watch_beside<'scope, 'env>(
    scope: &'scope Scope<'scope, 'env>,
    watch: Watch,
) -> ScopedJoinHandle<'scope, RunReserve> {
    scope.spawn(move || {
        let Watch {
            mut reserve,
            run,
            stop,
            uploads,
            done,
            policy,
        } = watch;
        let mut tick = Tick::default();
        while !done.load(Ordering::SeqCst) {
            let failing = uploads_failing(&uploads, super::now_unix());
            let stop_now = tick.step(
                &mut reserve,
                &policy,
                footprint(&run.entries),
                failing,
                &mut |line| eprintln!("goalcup: {line}"),
            );
            if stop_now {
                let _ = stop.send(DISK_STOP_REASON.to_string());
            }
            let wake = Instant::now() + policy.tick;
            while Instant::now() < wake && !done.load(Ordering::SeqCst) {
                std::thread::sleep(LOOK_EVERY.min(wake.saturating_duration_since(Instant::now())));
            }
        }
        reserve
    })
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
    /// The disk is low, and the participant has been told so.
    low_said: bool,
    /// The stop has been asked for.
    stopped: bool,
    /// A measurement or resize failed, and the participant has been told so.
    trouble_said: bool,
}

impl Tick {
    /// One look at the disk. Resizes the reserve to what [`target_of`] says, reads the free
    /// space again (so it is what is left once the reserve has given what it can), and judges
    /// it. Lines for the participant go to `say`. Answers `true` exactly once: the disk is
    /// full, uploads are failing, and the run must stop.
    pub fn step(
        &mut self,
        disk: &mut dyn Disk,
        policy: &DiskPolicy,
        footprint: u64,
        uploads_failing: bool,
        say: &mut dyn FnMut(&str),
    ) -> bool {
        let measured = disk.free().and_then(|free| {
            let wanted = target_with(policy, footprint, free, disk.held());
            disk.resize(wanted).or_else(|error| match error.kind() {
                // The disk filled between the two looks: the next tick gives back instead.
                io::ErrorKind::StorageFull => Ok(()),
                _ => Err(error),
            })?;
            disk.free()
        });
        let free = match measured {
            Ok(free) => free,
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
        match verdict_with(policy, footprint, free, uploads_failing) {
            DiskVerdict::Ok => {
                self.low_said = false;
                false
            }
            DiskVerdict::Low { free } => {
                if !std::mem::replace(&mut self.low_said, true) {
                    say(&format!(
                        "disk: {} free beyond the reservation",
                        human(free)
                    ));
                }
                false
            }
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
            Ok(())
        }
    }

    fn run(volume: &mut Volume, footprint: u64, failing: bool) -> (bool, Vec<String>) {
        let mut said = Vec::new();
        let stop = Tick::default().step(
            volume,
            &DiskPolicy::default(),
            footprint,
            failing,
            &mut |l| said.push(l.to_string()),
        );
        (stop, said)
    }

    #[test]
    fn the_reserve_follows_the_chain_on_a_roomy_disk() {
        let mut volume = Volume {
            capacity: 500 * GIB,
            others: 10 * GIB,
            reserve: 3 * GIB,
            resizes: Vec::new(),
        };
        let (stop, said) = run(&mut volume, GIB, true);
        assert!(!stop && said.is_empty(), "{said:?}");
        assert_eq!(volume.reserve, 2 * GIB);
        // Past 3 GiB of parts there is no reserve left, and the roomy disk is not a problem.
        let (stop, said) = run(&mut volume, 5 * GIB, true);
        assert!(!stop && said.is_empty(), "{said:?}");
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
        };
        let (stop, said) = run(&mut volume, 0, true);
        assert!(!stop);
        assert_eq!(volume.reserve, 2 * GIB + 100 * MIB - FREE_FLOOR);
        assert_eq!(volume.free().unwrap(), FREE_FLOOR);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("free beyond the reservation"), "{said:?}");
    }

    #[test]
    fn a_full_disk_with_failing_uploads_stops_once() {
        let mut volume = Volume {
            capacity: 20 * GIB,
            others: 0,
            reserve: GIB / 2,
            resizes: Vec::new(),
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
    fn a_full_disk_with_working_uploads_does_not_stop() {
        let mut volume = Volume {
            capacity: 20 * GIB,
            others: 20 * GIB - 10 * MIB,
            reserve: 0,
            resizes: Vec::new(),
        };
        let (stop, said) = run(&mut volume, 4 * GIB, false);
        assert!(!stop);
        assert_eq!(said.len(), 1, "{said:?}");
    }

    #[test]
    fn low_is_said_once_for_each_low_period() {
        let volume = |others: u64| Volume {
            capacity: 100 * GIB,
            others,
            reserve: 3 * GIB,
            resizes: Vec::new(),
        };
        let low = 100 * GIB - 3 * GIB - GIB;
        let mut tick = Tick::default();
        let mut said = Vec::new();
        for others in [low, low, 10 * GIB, low] {
            tick.step(
                &mut volume(others),
                &DiskPolicy::default(),
                0,
                false,
                &mut |l| said.push(l.to_string()),
            );
            // The first two readings are one low period; the third ends it; the fourth is new.
            if others == 10 * GIB {
                assert_eq!(said.len(), 1, "{said:?}");
            }
        }
        assert_eq!(said.len(), 2, "{said:?}");
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
