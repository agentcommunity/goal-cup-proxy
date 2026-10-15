//! The run's one disk reservation, and the rules that resize it.
//!
//! A run holds one physically allocated reserve file for as long as it records, not one per
//! chain. It is a **floor, not a cap**: the reserve plus the run's chain parts on disk are kept
//! at 3 GiB ([`target`]), so the reserve gives space back as the chain grows and takes it again
//! when parts are removed; a run may grow past 3 GiB while the disk has room. The reserve also
//! gives space back when something else fills the disk (the agent's own build), before the
//! recording suffers. A run stops cleanly only when the disk itself is nearly full, the reserve
//! has nothing left to give back, and uploads are failing ([`verdict`]).
//!
//! The file sits at `<artifact_root>/reserve/<run_id>`, on the same volume as the traces.

use super::EntryRef;
use crate::CliError;
use goalcup_platform::disk::{ReserveFile, free_bytes};
use goalcup_protocol::capture::CHAIN_QUOTA_BYTES;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What every run keeps free for its recording: the reserve and the chain on disk together.
pub const RUN_RESERVE_BYTES: u64 = 3 << 30;

/// What a run needs free beyond the reserve to start at all.
pub const START_MARGIN: u64 = 1 << 30;

/// Below this much free disk beyond the reserve, the participant is warned.
pub const LOW_DISK_BYTES: u64 = 5 << 30;

/// The free space the reserve gives back to keep: below it the disk is full for this purpose.
pub const FREE_FLOOR: u64 = 256 << 20;

/// Uploads are failing when none was acknowledged for this long.
pub const UPLOAD_FAILING_AFTER: Duration = Duration::from_secs(30 * 60);

/// How long past its deadline a run's reserve is kept, for the last upload.
pub const KEEP_AFTER_DEADLINE: Duration = Duration::from_secs(24 * 60 * 60);

/// How often the watch looks at the disk.
pub const WATCH_PERIOD: Duration = Duration::from_secs(30);

/// The reserve gives space back at once, but takes it again only when this much is missing, so
/// that it does not take freed space straight back on every tick.
pub const GROW_STEP: u64 = 256 << 20;

/// The sizes one start works with. Shipped builds use [`DiskPolicy::default`]; a test build can
/// name smaller ones ([`DiskPolicy::from_test_spec`]) so no test allocates 3 GiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskPolicy {
    /// What the reserve and the chain on disk add up to ([`RUN_RESERVE_BYTES`]).
    pub reserve_bytes: u64,
    /// What a start needs free beyond that ([`START_MARGIN`]).
    pub start_margin: u64,
    /// The free space the reserve keeps for the disk ([`FREE_FLOOR`]).
    pub free_floor: u64,
    /// The logical size a chain may grow to ([`CHAIN_QUOTA_BYTES`]): a limit on paper, not a
    /// reservation of any disk.
    pub chain_quota: u64,
    /// How often the watch ticks ([`WATCH_PERIOD`]).
    pub tick: Duration,
}

impl Default for DiskPolicy {
    fn default() -> Self {
        DiskPolicy {
            reserve_bytes: RUN_RESERVE_BYTES,
            start_margin: START_MARGIN,
            free_floor: FREE_FLOOR,
            chain_quota: CHAIN_QUOTA_BYTES,
            tick: WATCH_PERIOD,
        }
    }
}

impl DiskPolicy {
    /// A test build's sizes from `spec`: `reserve_bytes,start_margin_bytes,tick_ms`, and
    /// optionally `free_floor_bytes` and then `chain_quota_bytes` after them. A shipped build
    /// has no such door.
    #[cfg(feature = "test-support")]
    pub fn from_test_spec(spec: &str) -> Option<DiskPolicy> {
        let n: Vec<u64> = spec
            .split(',')
            .map(|v| v.trim().parse().ok())
            .collect::<Option<_>>()?;
        let (reserve_bytes, start_margin, tick_ms, free_floor, chain_quota) = match n[..] {
            [a, b, c] => (a, b, c, FREE_FLOOR, CHAIN_QUOTA_BYTES),
            [a, b, c, d] => (a, b, c, d, CHAIN_QUOTA_BYTES),
            [a, b, c, d, e] => (a, b, c, d, e),
            _ => return None,
        };
        Some(DiskPolicy {
            reserve_bytes,
            start_margin,
            free_floor,
            chain_quota,
            tick: Duration::from_millis(tick_ms),
        })
    }
}

/// `<artifact_root>/reserve/<run_id>`: on the volume that holds the traces.
pub fn reserve_path(artifact_root: &Path, run_id: &str) -> PathBuf {
    artifact_root.join("reserve").join(run_id)
}

/// The directory traces are kept under: the daemon's `artifact_root` as `<cfg>/daemon.toml`
/// names it, else the default beside `cfg`.
pub fn artifact_root(cfg: &Path) -> PathBuf {
    crate::daemon::spawn::artifact_root(cfg)
}

/// What the reserve should hold: whatever keeps it and the chain on disk at 3 GiB, and never
/// more than the disk can give once the floor is kept free. `footprint` is the chain parts on
/// disk, `free` the volume's free bytes, `reserve` what the reserve holds now.
pub fn target(footprint: u64, free: u64, reserve: u64) -> u64 {
    target_with(&DiskPolicy::default(), footprint, free, reserve)
}

/// [`target`] at the sizes of `policy`.
pub fn target_with(policy: &DiskPolicy, footprint: u64, free: u64, reserve: u64) -> u64 {
    policy.reserve_bytes.saturating_sub(footprint).min(
        reserve
            .saturating_add(free)
            .saturating_sub(policy.free_floor),
    )
}

/// What the watch makes of the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskVerdict {
    Ok,
    /// Less than [`LOW_DISK_BYTES`] free beyond the reservation.
    Low {
        free: u64,
    },
    /// The disk is full, nothing is left to give back, and uploads are failing: stop cleanly.
    Stop,
}

/// The verdict on one reading.
///
/// `held` is what the reserve holds and `free_beyond_reserve` what the volume has beyond it,
/// both read after the reserve was resized to [`target`]. `Stop` needs all three: the reserve
/// drawn down to nothing (`held == 0`), the free space below [`FREE_FLOOR`], and uploads
/// failing. Free space just under the floor while the reserve still holds bytes is a reserve
/// that has more to give back, not a full disk. The footprint alone never stops a run: it may
/// grow past 3 GiB while the disk has room, which is why it does not enter the verdict.
pub fn verdict(
    footprint: u64,
    held: u64,
    free_beyond_reserve: u64,
    uploads_failing: bool,
) -> DiskVerdict {
    verdict_with(
        &DiskPolicy::default(),
        footprint,
        held,
        free_beyond_reserve,
        uploads_failing,
    )
}

/// [`verdict`] at the floor of `policy`.
pub fn verdict_with(
    policy: &DiskPolicy,
    _footprint: u64,
    held: u64,
    free_beyond_reserve: u64,
    uploads_failing: bool,
) -> DiskVerdict {
    if held == 0 && free_beyond_reserve < policy.free_floor && uploads_failing {
        DiskVerdict::Stop
    } else if free_beyond_reserve < LOW_DISK_BYTES {
        DiskVerdict::Low {
            free: free_beyond_reserve,
        }
    } else {
        DiskVerdict::Ok
    }
}

/// Whether the run's chain parts alone are past the reservation: parts that are acknowledged
/// may then go.
pub fn pressure(footprint: u64) -> bool {
    footprint > RUN_RESERVE_BYTES
}

/// The bytes of one entry's chain on disk: the sum of its parts' lengths
/// ([`goalcup_protocol::export::chain_parts`]). A missing directory, or a chain that cannot be
/// listed, counts for nothing.
pub fn chain_footprint(trace_dir: &Path) -> u64 {
    crate::upload::prune::parts_now(trace_dir)
        .map(|parts| parts.iter().map(|part| part.len).sum())
        .unwrap_or(0)
}

/// The chain bytes on disk across all of a run's entries.
pub fn footprint(entries: &[EntryRef]) -> u64 {
    entries
        .iter()
        .map(|entry| chain_footprint(&entry.trace_dir))
        .sum()
}

/// `bytes` in the largest unit that keeps it short: `3 GiB`, `2.4 GiB`, `100 MiB`.
pub fn human(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")];
    for (size, name) in UNITS {
        if bytes >= size {
            return if bytes.is_multiple_of(size) {
                format!("{} {name}", bytes / size)
            } else {
                format!("{:.1} {name}", bytes as f64 / size as f64)
            };
        }
    }
    format!("{bytes} B")
}

/// The run's reserve file, held for as long as the run records, and locked (`flock`, held by
/// this open file) for as long as this value lives. A process that dies takes the lock with it,
/// which is how a later start tells a reserve nobody holds from one a live run does.
#[derive(Debug)]
pub struct RunReserve {
    file: ReserveFile,
    dir: PathBuf,
    /// Holds the lock; never read.
    _lock: std::fs::File,
}

/// Open `path` for locking, never through a symlink, creating it when asked.
fn open_lockable(path: &Path, create: bool) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

/// Open `path` for locking, and say whether this call made it. A reserve that was already there
/// is a continued run's, reused, and must outlive a failure to hold it.
fn open_or_create_lockable(path: &Path) -> std::io::Result<(std::fs::File, bool)> {
    use std::os::unix::fs::OpenOptionsExt;
    let made = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);
    match made {
        Ok(file) => Ok((file, true)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            open_lockable(path, false).map(|file| (file, false))
        }
        Err(error) => Err(error),
    }
}

/// Whether `path` still names the very file `locked` has open: the lock was taken on it, not on
/// a file another process has since unlinked.
fn names_the_file(locked: &std::fs::File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (locked.metadata(), std::fs::symlink_metadata(path)) {
        (Ok(open), Ok(named)) => open.dev() == named.dev() && open.ino() == named.ino(),
        _ => false,
    }
}

/// Refuse, before anything is created, a volume that cannot hold the reserve and the start
/// margin. `dir` need not exist yet: the nearest directory above it that does is measured.
pub fn check_room(dir: &Path, policy: &DiskPolicy) -> Result<(), CliError> {
    room(dir, policy).map(|_| ())
}

/// The volume's free bytes under `dir`, when they are enough to start; else the refusal.
fn room(dir: &Path, policy: &DiskPolicy) -> Result<u64, CliError> {
    room_for(dir, policy, policy.reserve_bytes)
}

/// [`room`] for a reserve that still has to allocate `needed` bytes: a continued run's reserve
/// file keeps the blocks it has, so only the missing ones must be free.
fn room_for(dir: &Path, policy: &DiskPolicy, needed: u64) -> Result<u64, CliError> {
    let near = dir.ancestors().find(|above| above.is_dir()).unwrap_or(dir);
    let free = free_bytes(near).map_err(|source| CliError::Io {
        path: near.display().to_string(),
        source,
    })?;
    if free < needed.saturating_add(policy.start_margin) {
        return Err(refusal(dir, policy, free));
    }
    Ok(free)
}

fn refusal(dir: &Path, policy: &DiskPolicy, free: u64) -> CliError {
    CliError::Refused(format!(
        "this run needs {} free plus {}; {} are free on the volume that holds {}. Make room and \
         start again.",
        human(policy.reserve_bytes),
        human(policy.start_margin),
        human(free),
        dir.display()
    ))
}

impl RunReserve {
    /// Hold `policy.reserve_bytes` at `path`, or refuse: the volume must have that much free
    /// plus `policy.start_margin`. Nothing is left behind on a refusal.
    ///
    /// A reserve file that is already there is a continued run's: it is reused, keeps the blocks
    /// it has (only the missing ones must be free), and is never removed by a failure to hold
    /// it. Another process's [`release_stale`] may unlink the path between the open and the
    /// lock; the lock is only good if the path still names the file that was locked, so a lock
    /// on an unlinked file is dropped and the take is tried once more.
    pub fn take(path: &Path, policy: &DiskPolicy) -> Result<RunReserve, CliError> {
        use std::os::unix::fs::MetadataExt;
        let dir = path.parent().ok_or_else(|| {
            CliError::Internal(format!(
                "the reserve path {} has no directory",
                path.display()
            ))
        })?;
        crate::config::create_private_dir(dir)?;
        let io = |source| CliError::Io {
            path: path.display().to_string(),
            source,
        };
        for _ in 0..2 {
            // The file exists and is locked before any block is allocated, so a start looking
            // for stale reserves never sees this one unlocked.
            let (lock, created) = open_or_create_lockable(path).map_err(io)?;
            match lock.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(CliError::Refused(format!(
                        "another goalcup is holding the reservation {}.",
                        path.display()
                    )));
                }
                Err(std::fs::TryLockError::Error(source)) => return Err(io(source)),
            }
            if !names_the_file(&lock, path) {
                continue;
            }
            let have = lock.metadata().map_or(0, |meta| meta.blocks() * 512);
            let free = match room_for(dir, policy, policy.reserve_bytes.saturating_sub(have)) {
                Ok(free) => free,
                Err(refused) => {
                    if created {
                        let _ = std::fs::remove_file(path);
                    }
                    return Err(refused);
                }
            };
            return match ReserveFile::hold(path, policy.reserve_bytes) {
                Ok(file) => Ok(RunReserve {
                    file,
                    dir: dir.to_path_buf(),
                    _lock: lock,
                }),
                Err(source) => {
                    if created {
                        let _ = std::fs::remove_file(path);
                    }
                    if source.kind() == std::io::ErrorKind::StorageFull {
                        Err(refusal(dir, policy, free))
                    } else {
                        Err(io(source))
                    }
                }
            };
        }
        Err(CliError::Refused(format!(
            "the reservation {} was removed by another goalcup while this one was taking it. \
             Try again.",
            path.display()
        )))
    }

    /// Bytes the reserve physically holds now.
    pub fn allocated(&self) -> u64 {
        self.file.allocated()
    }

    /// The volume's free bytes, beyond what the reserve holds.
    pub fn free(&self) -> std::io::Result<u64> {
        free_bytes(&self.dir)
    }

    /// Give back or take blocks so the reserve holds `bytes`.
    pub fn resize(&mut self, bytes: u64) -> std::io::Result<()> {
        self.file.resize(bytes)
    }

    /// Give the whole reserve back to the volume.
    pub fn release(self) -> std::io::Result<()> {
        let RunReserve { file, _lock, .. } = self;
        let released = file.release();
        drop(_lock);
        released
    }
}

/// Remove the reserves nobody will use again, and say how many went. Called when a start
/// begins.
///
/// A run is known by its record under `cfg`.
/// - A run opened on a server keeps its reserve until its deadline and a day ([`KEEP_AFTER_DEADLINE`])
///   have passed, whether or not anything holds it: its harness may have exited early and
///   `continue` will want the reserve again.
/// - A local run has no deadline. Its reserve goes when no process holds its lock (the run's
///   `goalcup start` died; a live one holds it for as long as it records).
/// - A reserve with no run record goes the same way, when its lock is free.
pub fn release_stale(cfg: &Path, artifact_root: &Path, now: u64) -> usize {
    let Ok(files) = std::fs::read_dir(artifact_root.join("reserve")) else {
        return 0;
    };
    let mut released = 0;
    for file in files.flatten() {
        let Some(run_id) = file.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(record) = super::read_if_any(cfg, &run_id) else {
            continue;
        };
        let gone = match record.as_ref().and_then(|r| r.deadline_at.as_deref()) {
            Some(deadline) => goalcup_protocol::control::parse_z(deadline).is_some_and(|d| {
                now >= d.saturating_add(KEEP_AFTER_DEADLINE.as_secs())
                    && std::fs::remove_file(file.path()).is_ok()
            }),
            None => remove_if_unheld(&file.path()),
        };
        if gone {
            released += 1;
        }
    }
    released
}

/// Unlink `path` when no process holds its lock; the lock is taken for the unlink, so a run
/// that takes the file at that moment finds it locked or gone, never half-removed.
fn remove_if_unheld(path: &Path) -> bool {
    let Ok(lock) = open_lockable(path, false) else {
        return false;
    };
    lock.try_lock().is_ok() && std::fs::remove_file(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;

    #[test]
    fn the_reserve_is_three_gib_minus_the_parts_on_disk() {
        assert_eq!(target(0, 100 * GIB, 0), 3 * GIB);
        assert_eq!(target(GIB, 100 * GIB, 0), 2 * GIB);
        assert_eq!(target(3 * GIB, 100 * GIB, 0), 0);
    }

    #[test]
    fn a_run_may_grow_past_three_gib_while_the_disk_has_room() {
        // Five GiB of parts: the reserve is gone, and nothing stops the run.
        assert_eq!(target(5 * GIB, 50 * GIB, 0), 0);
        assert!(pressure(5 * GIB));
        assert!(!pressure(3 * GIB));
        assert_eq!(verdict(5 * GIB, 0, 50 * GIB, true), DiskVerdict::Ok);
        assert_eq!(verdict(5 * GIB, 0, 50 * GIB, false), DiskVerdict::Ok);
    }

    #[test]
    fn the_agent_filling_the_disk_draws_down_the_reserve_before_any_stop() {
        // 100 MiB free, 2 GiB reserved: the reserve gives back all but the floor.
        let held = target(0, 100 * MIB, 2 * GIB);
        assert_eq!(held, 2 * GIB + 100 * MIB - FREE_FLOOR);
        assert!(held < 2 * GIB);
        // After it did, the disk has the floor free: low, but not a stop, even with the
        // uploads failing.
        let freed = 100 * MIB + (2 * GIB - held);
        assert_eq!(freed, FREE_FLOOR);
        assert_eq!(
            verdict(0, held, freed, true),
            DiskVerdict::Low { free: FREE_FLOOR }
        );
    }

    #[test]
    fn a_reserve_never_grows_past_the_floor_the_disk_can_keep() {
        assert_eq!(target(0, 100 * MIB, 0), 0);
        assert_eq!(target(0, FREE_FLOOR + MIB, 0), MIB);
    }

    #[test]
    fn a_full_disk_with_failing_uploads_stops() {
        assert_eq!(verdict(GIB, 0, 100 * MIB, true), DiskVerdict::Stop);
        assert_eq!(verdict(0, 0, 0, true), DiskVerdict::Stop);
    }

    #[test]
    fn a_reserve_that_still_holds_bytes_is_never_a_full_disk() {
        // Free space just under the floor with the reserve holding GiB: it has more to give.
        assert_eq!(
            verdict(0, 2 * GIB, FREE_FLOOR - 4096, true),
            DiskVerdict::Low {
                free: FREE_FLOOR - 4096
            }
        );
        assert_eq!(
            verdict(0, 1, 0, true),
            DiskVerdict::Low { free: 0 },
            "even one byte held"
        );
    }

    #[test]
    fn a_full_disk_with_working_uploads_does_not_stop() {
        assert_eq!(
            verdict(GIB, 0, 100 * MIB, false),
            DiskVerdict::Low { free: 100 * MIB }
        );
    }

    #[test]
    fn low_below_five_gib_beyond_the_reservation() {
        assert_eq!(verdict(0, 0, 5 * GIB, true), DiskVerdict::Ok);
        assert_eq!(
            verdict(0, 0, 5 * GIB - 1, true),
            DiskVerdict::Low { free: 5 * GIB - 1 }
        );
    }

    #[test]
    fn the_footprint_counts_the_chain_and_its_parts_only() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, len: usize| std::fs::write(dir.path().join(name), vec![0; len]);
        write("chain.jsonl", 100).unwrap();
        write("chain.00000000000000000100.jsonl", 40).unwrap();
        write("chain.jsonl.tmp", 7).unwrap();
        write("seal.json", 9).unwrap();
        write("chain.x.jsonl", 5).unwrap();
        assert_eq!(chain_footprint(dir.path()), 140);
        // The first parts deleted after their acknowledgement: what is left is counted.
        std::fs::remove_file(dir.path().join("chain.jsonl")).unwrap();
        assert_eq!(chain_footprint(dir.path()), 40);
        assert_eq!(chain_footprint(&dir.path().join("missing")), 0);
        assert_eq!(chain_footprint(Path::new("")), 0);
    }

    #[test]
    fn sizes_read_as_a_person_would() {
        assert_eq!(human(3 * GIB), "3 GiB");
        assert_eq!(human(GIB + GIB / 2), "1.5 GiB");
        assert_eq!(human(4 * MIB), "4 MiB");
        assert_eq!(human(12), "12 B");
    }

    #[test]
    fn a_run_that_cannot_get_its_disk_is_refused_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = reserve_path(dir.path(), "r1");
        let policy = DiskPolicy {
            reserve_bytes: 1 << 60,
            start_margin: GIB,
            free_floor: FREE_FLOOR,
            chain_quota: CHAIN_QUOTA_BYTES,
            tick: Duration::from_millis(50),
        };
        let refused = RunReserve::take(&path, &policy).unwrap_err();
        assert_eq!(refused.exit_code(), 1);
        assert!(refused.to_string().contains("this run needs"), "{refused}");
        assert!(!path.exists());
    }

    #[test]
    fn a_reserve_is_held_resized_and_released() {
        let dir = tempfile::tempdir().unwrap();
        let path = reserve_path(dir.path(), "r1");
        let policy = DiskPolicy {
            reserve_bytes: 8 * MIB,
            start_margin: 0,
            free_floor: FREE_FLOOR,
            chain_quota: CHAIN_QUOTA_BYTES,
            tick: Duration::from_millis(50),
        };
        let mut reserve = RunReserve::take(&path, &policy).unwrap();
        assert!(reserve.allocated() >= 8 * MIB);
        reserve.resize(2 * MIB).unwrap();
        assert!(reserve.allocated() < 4 * MIB);
        reserve.resize(6 * MIB).unwrap();
        assert!(reserve.allocated() >= 6 * MIB);
        reserve.release().unwrap();
        assert!(!path.exists());
    }

    fn record(cfg: &Path, id: &str, deadline_at: Option<String>) {
        super::super::write(
            cfg,
            &super::super::RunRecord {
                run_id: id.to_string(),
                server_url: deadline_at
                    .as_ref()
                    .map(|_| "https://server.example".into()),
                practice: false,
                connector_id: "c".to_string(),
                workspace: PathBuf::from("/w"),
                started_at_unix: 0,
                deadline_at,
                entries: Vec::new(),
            },
        )
        .unwrap();
    }

    fn small() -> DiskPolicy {
        DiskPolicy {
            reserve_bytes: MIB,
            start_margin: 0,
            free_floor: FREE_FLOOR,
            chain_quota: CHAIN_QUOTA_BYTES,
            tick: Duration::from_millis(50),
        }
    }

    #[test]
    fn a_server_run_keeps_its_reserve_until_its_deadline_and_a_day_have_passed() {
        let cfg = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let deadline = |at: u64| Some(goalcup_protocol::control::format_z(at));
        for (id, deadline_at) in [("r-old", deadline(1_000)), ("r-new", deadline(1_000_000))] {
            record(cfg.path(), id, deadline_at);
            // No process holds either lock after this: only the deadline decides.
            drop(RunReserve::take(&reserve_path(root.path(), id), &small()).unwrap());
        }
        let now = 1_000 + KEEP_AFTER_DEADLINE.as_secs() - 1;
        assert_eq!(release_stale(cfg.path(), root.path(), now), 0);
        assert_eq!(release_stale(cfg.path(), root.path(), now + 1), 1);
        let left = |id: &str| reserve_path(root.path(), id).exists();
        assert!(!left("r-old") && left("r-new"));
    }

    #[test]
    fn a_local_reserve_left_by_a_killed_run_is_removed_and_a_held_one_is_not() {
        let cfg = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        record(cfg.path(), "r-dead", None);
        record(cfg.path(), "r-live", None);
        // The killed run: its file is there and nothing holds the lock any more.
        drop(RunReserve::take(&reserve_path(root.path(), "r-dead"), &small()).unwrap());
        // No record either, and no holder.
        drop(RunReserve::take(&reserve_path(root.path(), "r-orphan"), &small()).unwrap());
        // The live runs: a run with a record, and one whose record is not written yet.
        let live = RunReserve::take(&reserve_path(root.path(), "r-live"), &small()).unwrap();
        let starting =
            RunReserve::take(&reserve_path(root.path(), "r-starting"), &small()).unwrap();

        assert_eq!(release_stale(cfg.path(), root.path(), 1), 2);
        let left = |id: &str| reserve_path(root.path(), id).exists();
        assert!(!left("r-dead") && !left("r-orphan"));
        assert!(left("r-live") && left("r-starting"));
        // A second holder cannot take a held reserve.
        let again = RunReserve::take(&reserve_path(root.path(), "r-live"), &small());
        assert!(again.is_err());
        // Once their holders are done, the next start takes them too.
        drop((live, starting));
        assert_eq!(release_stale(cfg.path(), root.path(), 1), 2);
    }

    #[test]
    fn a_reserve_that_is_there_survives_a_failed_hold() {
        // A continued run takes its reserve file again. When the volume cannot hold what it
        // asks, the file the run already had stays where it is.
        let dir = tempfile::tempdir().unwrap();
        let path = reserve_path(dir.path(), "r1");
        drop(RunReserve::take(&path, &small()).unwrap());
        let before = std::fs::metadata(&path).unwrap();
        let too_much = DiskPolicy {
            reserve_bytes: 1 << 60,
            ..small()
        };
        let refused = RunReserve::take(&path, &too_much).unwrap_err();
        assert!(refused.to_string().contains("this run needs"), "{refused}");
        let after = std::fs::metadata(&path).expect("the reserve file is still there");
        use std::os::unix::fs::MetadataExt;
        assert_eq!(before.ino(), after.ino());
        // And a reserve that was not there before is not left behind by the same refusal.
        let fresh = reserve_path(dir.path(), "r2");
        RunReserve::take(&fresh, &too_much).unwrap_err();
        assert!(!fresh.exists());
    }

    #[test]
    fn a_reserve_takes_only_the_blocks_it_is_missing() {
        // The volume has to hold the reserve plus the margin, less what the file already holds:
        // a margin larger than the free space refuses a new reserve, not a reused full one.
        let dir = tempfile::tempdir().unwrap();
        let path = reserve_path(dir.path(), "r1");
        drop(RunReserve::take(&path, &small()).unwrap());
        let free = free_bytes(dir.path()).unwrap();
        let tight = DiskPolicy {
            start_margin: free,
            ..small()
        };
        // Nothing to allocate: only the margin is asked of the volume, and it is there.
        assert!(RunReserve::take(&path, &tight).is_ok());
        let other = reserve_path(dir.path(), "r2");
        assert!(RunReserve::take(&other, &tight).is_err());
    }

    #[test]
    fn a_lock_is_only_good_on_the_file_the_path_still_names() {
        // Another process's `release_stale` unlinks the path between the open and the lock: the
        // lock is then on a file nothing names, and `take` must not trust it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reserve");
        let (locked, created) = open_or_create_lockable(&path).unwrap();
        assert!(created);
        assert!(names_the_file(&locked, &path));
        let (_, created) = open_or_create_lockable(&path).unwrap();
        assert!(!created, "an existing file is opened, not made");
        std::fs::remove_file(&path).unwrap();
        assert!(!names_the_file(&locked, &path), "unlinked");
        std::fs::write(&path, b"another file").unwrap();
        assert!(!names_the_file(&locked, &path), "replaced by another file");
    }
}
