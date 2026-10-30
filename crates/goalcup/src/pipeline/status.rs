//! `status.json` (seam S4, PRD-02 section 6). Only the recorder writes it, each second, by atomic
//! rename; `goalcup status`, the end screen (PRD-10) and the uploader (PRD-06) read it.

use super::Recorder;
use super::counts::{Counts, EntryTotals};
use crate::watchdog::{Exit, SealReason};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The schema version.
pub const STATUS_V: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub v: u32,
    /// Wall time of this write, RFC 3339 UTC.
    pub updated: String,
    /// The recorder's pid.
    pub pid: u32,
    /// The recorder's OS start time (as `HarnessFacts::start_id`), so `goalcup recover` never
    /// signals a process that reused its pid.
    pub start_id: u64,
    pub boot: u64,
    /// The listen port, so `goalcup continue` and `goalcup recover` can ask for it again.
    pub port: u16,
    pub mode: Mode,
    pub harness: Option<HarnessStatus>,
    /// Entry totals at the last LINK whose index line is written.
    pub written: Written,
    pub last_request_at: Option<String>,
    pub last_native_at: Option<String>,
    pub losing: Losing,
    pub off: Off,
    /// The first note of each `what`, then the last 20 `note` events.
    pub notes: Vec<serde_json::Value>,
    pub held_bytes: u64,
    pub held_peak: u64,
    /// The soft guard in bytes.
    pub guard: u64,
    pub writer_idle_ms: u64,
    pub durable: DurableStatus,
    pub deadline: Option<String>,
    /// `reading session files` while the seal reads them with no time guard; null otherwise.
    pub sealing: Option<String>,
    // Sealed only (`mode: sealed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seal: Option<SealStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cut_files: Option<CutFiles>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outlived: Option<Outlived>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_seal: Option<AfterSeal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Recording,
    /// Some stream is in a gap, or a `parse_stopped` connection's closing GAP is not yet written.
    Losing,
    /// Records wait and nothing was written for 10 s (display only).
    WriterSlow,
    /// Records nothing; `off.since` is set.
    Null,
    Sealed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessStatus {
    pub pid: u32,
    /// Wall time, RFC 3339 UTC, for display only.
    pub started: String,
    /// `HarnessFacts::start_id`: the OS start time of `pid`. The harness is alive only while
    /// `pid` answers and this still matches; `started` is never compared.
    pub start_id: u64,
    pub exit: Option<Exit>,
}

/// `written`: every COUNT key as entry totals, plus `model_requests`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Written {
    pub model_requests: u64,
    #[serde(flatten)]
    pub counts: Counts,
}

impl Written {
    pub fn new(counts: Counts) -> Written {
        Written {
            model_requests: counts.model_requests(),
            counts,
        }
    }
}

/// `Recorder::live()` (seam S4): the live atomics `status.json` is written from each second.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub boot: u64,
    /// Bytes offered and not yet written, plus reservations. Records wait while this is not 0.
    pub held_bytes: u64,
    /// Boot time since the writer last wrote.
    pub writer_idle_ms: u64,
    pub losing: Losing,
    /// Entry totals over boots, the open null span included.
    pub off: Off,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Losing {
    /// Streams in a gap, plus `parse_stopped` connections whose closing GAP is not yet written.
    pub streams_in_gap: u64,
    /// Bytes dropped this boot.
    pub dropped_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Off {
    /// `recorder_down` spans on `sid 0`.
    pub spans: u64,
    pub ms: u64,
    /// Wall time a null recorder started, RFC 3339 UTC.
    pub since: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableStatus {
    pub seq: u64,
    pub raw_end: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealStatus {
    pub reason: SealReason,
    /// `git rev-parse HEAD` of the workspace, null on failure.
    pub workspace_head: Option<String>,
}

/// From `NativeFinish` (PRD-04): session files cut by the seal's time guard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutFiles {
    pub files: u64,
    pub bytes: u64,
}

/// Processes the harness left running at the seal (note `outlived`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outlived {
    pub count: u64,
    pub programs: Vec<String>,
}

/// Bytes relayed after the seal's LINK, which cannot be recorded, and their hosts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfterSeal {
    pub bytes: u64,
    pub hosts: Vec<String>,
}

impl Status {
    const NAME: &'static str = "status.json";

    /// Writes `status.json` in `entry_dir` by atomic rename, mode 0600, without fsync (every
    /// second; a crash loses at most a second of display).
    pub fn write(&self, entry_dir: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        write_private(&entry_dir.join(Self::NAME), &bytes, false)
    }

    /// Reads `status.json` in `entry_dir` (PRD-10). None when it is missing or does not parse.
    pub fn read(entry_dir: &Path) -> Option<Status> {
        serde_json::from_slice(&fs::read(entry_dir.join(Self::NAME)).ok()?).ok()
    }
}

/// Writes `bytes` to `path` by atomic rename: the parent folder is created 0700 when missing, a
/// temp file in the same folder is created new with mode 0600, written, synced when `sync`, and
/// renamed over `path`; then the folder is synced when `sync`. `counts.jsonl`, `cursors.json` and
/// `status.json` all use it.
pub(crate) fn write_private(path: &Path, bytes: &[u8], sync: bool) -> io::Result<()> {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    if !dir.exists() {
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let tmp = dir.join(format!(
        ".{}.{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let put = || -> io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        if sync {
            f.sync_all()?;
        }
        drop(f);
        fs::rename(&tmp, path)?;
        if sync {
            File::open(dir)?.sync_all()?;
        }
        Ok(())
    };
    let r = put();
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r
}

/// The mode `status.json` shows: sealed, then null, then `writer_slow` (records wait and nothing
/// was written for 10 s), then `losing` (some stream in a gap), then recording.
pub fn mode(live: &Live, null: bool, sealed: bool) -> Mode {
    if sealed {
        Mode::Sealed
    } else if null {
        Mode::Null
    } else if live.held_bytes > 0 && live.writer_idle_ms >= 10_000 {
        Mode::WriterSlow
    } else if live.losing.streams_in_gap > 0 {
        Mode::Losing
    } else {
        Mode::Recording
    }
}

/// The stall exit (PRD-02 section 3): records wait and nothing was written for 60 s of boot time,
/// or held bytes reached twice the soft guard.
pub fn stalled(live: &Live, guard: u64) -> bool {
    (live.held_bytes > 0 && live.writer_idle_ms >= 60_000)
        || live.held_bytes >= guard.saturating_mul(2)
}

/// The facts `status.json` carries that the live atomics do not. Cheap to clone; every setter
/// takes the lock for a plain store and no lock is held across I/O.
#[derive(Clone, Default)]
pub struct Board(Arc<Mutex<Facts>>);

#[derive(Default)]
struct Facts {
    pid: u32,
    start_id: u64,
    port: u16,
    deadline: Option<String>,
    harness: Option<HarnessStatus>,
    /// The first note of each `what`, with its number.
    first_notes: Vec<(u64, Value)>,
    /// The last 20 notes, with their numbers.
    last_notes: VecDeque<(u64, Value)>,
    notes_seen: u64,
    last_request_at: Option<String>,
    last_native_at: Option<String>,
    null_since: Option<String>,
    sealing: Option<String>,
    after_seal: Option<AfterSeal>,
    seal: Option<SealStatus>,
    totals: Option<EntryTotals>,
    cut_files: Option<CutFiles>,
    outlived: Option<Outlived>,
    stop: bool,
}

/// How many recent notes `status.json` keeps beside the first of each `what`.
const LAST_NOTES: usize = 20;

impl Board {
    fn facts(&self) -> MutexGuard<'_, Facts> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The recorder's pid, its OS start time and the listen port.
    pub fn process(&self, pid: u32, start_id: u64, port: u16) {
        let mut f = self.facts();
        (f.pid, f.start_id, f.port) = (pid, start_id, port);
    }

    pub fn deadline(&self, deadline: Option<String>) {
        self.facts().deadline = deadline;
    }

    /// The harness as the watchdog launched it.
    pub fn launch(&self, harness: HarnessStatus) {
        self.facts().harness = Some(harness);
    }

    /// How the harness ended.
    pub fn exit(&self, exit: Exit) {
        if let Some(h) = &mut self.facts().harness {
            h.exit = Some(exit);
        }
    }

    /// A `note` event as written.
    pub fn note(&self, note: Value) {
        let mut f = self.facts();
        let n = f.notes_seen;
        f.notes_seen += 1;
        let what = &note["what"];
        if !f.first_notes.iter().any(|(_, v)| v["what"] == *what) {
            f.first_notes.push((n, note.clone()));
        }
        f.last_notes.push_back((n, note));
        if f.last_notes.len() > LAST_NOTES {
            f.last_notes.pop_front();
        }
    }

    /// Wall time (RFC 3339 UTC) of the last written model request.
    pub fn request_at(&self, at: String) {
        self.facts().last_request_at = Some(at);
    }

    /// Wall time (RFC 3339 UTC) of the last written native DATA.
    pub fn native_at(&self, at: String) {
        self.facts().last_native_at = Some(at);
    }

    /// Set while a null recorder runs: when it started, RFC 3339 UTC.
    pub fn null_since(&self, since: Option<String>) {
        self.facts().null_since = since;
    }

    /// `reading session files` while the seal reads them with no time guard; None otherwise.
    pub fn sealing(&self, sealing: Option<String>) {
        self.facts().sealing = sealing;
    }

    pub fn after_seal(&self, after: AfterSeal) {
        self.facts().after_seal = Some(after);
    }

    /// The seal: from now on `mode` is sealed and `written` and `off` are the seal's totals.
    pub fn sealed(
        &self,
        seal: SealStatus,
        totals: EntryTotals,
        cut_files: Option<CutFiles>,
        outlived: Option<Outlived>,
    ) {
        let mut f = self.facts();
        f.seal = Some(seal);
        f.totals = Some(totals);
        f.cut_files = cut_files;
        f.outlived = outlived;
    }

    /// Stops the status thread after its current write.
    pub fn stop(&self) {
        self.facts().stop = true;
    }

    fn stopped(&self) -> bool {
        self.facts().stop
    }

    /// The status from the live atomics, this entry's written counts, the durable point and these
    /// facts. In `mode: sealed`, `written` and `off` are the seal's totals.
    pub fn status(
        &self,
        live: &Live,
        counts: Counts,
        durable: DurableStatus,
        guard: u64,
    ) -> Status {
        let f = self.facts();
        let sealed = f.seal.is_some();
        let (counts, off) = match (&f.totals, sealed) {
            (Some(t), true) => (
                t.counts.clone(),
                Off {
                    spans: t.off_spans,
                    ms: t.counts.off_ms,
                    since: None,
                },
            ),
            _ => (
                counts,
                Off {
                    since: live.off.since.clone().or_else(|| f.null_since.clone()),
                    ..live.off.clone()
                },
            ),
        };
        let first: Vec<u64> = f.first_notes.iter().map(|(n, _)| *n).collect();
        let notes = f
            .first_notes
            .iter()
            .chain(f.last_notes.iter().filter(|(n, _)| !first.contains(n)))
            .map(|(_, v)| v.clone())
            .collect();
        Status {
            v: STATUS_V,
            updated: rfc3339_now(),
            pid: f.pid,
            start_id: f.start_id,
            boot: live.boot,
            port: f.port,
            mode: mode(live, f.null_since.is_some(), sealed),
            harness: f.harness.clone(),
            held_peak: counts.held_peak,
            written: Written::new(counts),
            last_request_at: f.last_request_at.clone(),
            last_native_at: f.last_native_at.clone(),
            losing: live.losing,
            off,
            notes,
            held_bytes: live.held_bytes,
            guard,
            writer_idle_ms: live.writer_idle_ms,
            durable,
            deadline: f.deadline.clone(),
            sealing: f.sealing.clone(),
            seal: f.seal.clone(),
            cut_files: f.cut_files,
            outlived: f.outlived.clone(),
            after_seal: f.after_seal.clone(),
        }
    }
}

/// The status thread (PRD-02 sections 3 and 5 step 5): every second it writes `status.json` from
/// the live atomics, the written counts, the durable point and `board`, and checks the stall
/// exit. A stall writes one fixed line to stderr and ends the process with 75; the next boot names
/// the span `writer_stalled`. It runs until [`Board::stop`].
pub fn spawn(rec: Recorder, entry_dir: PathBuf, board: Board, guard: u64) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("goalcup-status".into())
        .spawn(move || {
            let durable = rec.durable();
            while !board.stopped() {
                let live = rec.live();
                let d = durable
                    .borrow()
                    .point
                    .as_ref()
                    .map(|p| DurableStatus {
                        seq: p.line.seq,
                        raw_end: p.line.raw_end,
                    })
                    .unwrap_or_default();
                let _ = board
                    .status(&live, rec.counts(), d, guard)
                    .write(&entry_dir);
                if stalled(&live, guard) {
                    let _ = io::stderr()
                        .write_all(b"goalcup recorder: the writer stalled; exiting to restart\n");
                    std::process::exit(75);
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("the status thread starts")
}

/// Wall time now, RFC 3339 UTC, whole seconds.
pub(crate) fn rfc3339_now() -> String {
    let s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    rfc3339(s)
}

/// Unix seconds as RFC 3339 UTC (`2026-11-07T10:00:01Z`), by the days-to-civil rule.
pub(crate) fn rfc3339(unix_s: u64) -> String {
    let days = (unix_s / 86_400) as i64;
    let rem = unix_s % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stall_exit_at_its_two_marks() {
        let guard = 512 << 20;
        let live = |held_bytes, writer_idle_ms| Live {
            held_bytes,
            writer_idle_ms,
            ..Live::default()
        };
        assert!(!stalled(&live(1, 59_999), guard));
        assert!(stalled(&live(1, 60_000), guard));
        assert!(!stalled(&live(0, 600_000), guard));
        assert!(!stalled(&live(2 * guard - 1, 0), guard));
        assert!(stalled(&live(2 * guard, 0), guard));
    }
}
