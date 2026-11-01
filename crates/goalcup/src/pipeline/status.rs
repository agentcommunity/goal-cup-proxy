//! `status.json` (seam S4, PRD-02 section 6). Only the recorder writes it, each second, by atomic
//! rename; `goalcup status`, the end screen (PRD-10) and the uploader (PRD-06) read it.

use super::counts::{Counts, EntryTotals};
use super::{EXIT_PIPELINE_GONE, Recorder, clock, secure};
use crate::watchdog::{Exit, SealReason};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// The schema version.
pub const STATUS_V: u32 = 1;

/// The stall exit's code (PRD-02 section 3); the next boot names the span `writer_stalled`.
pub(super) const EXIT_WRITER_STALLED: i32 = 75;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
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
    /// Bytes offered and not yet written, plus reservations. Each queued item also counts its
    /// cost in memory: 128 per DATA piece, 384 plus its JSON per control item (`Item::held`).
    /// The boot's peak and the soft guard are not here: the status thread reads them from the
    /// recorder, so this shape stays the one PRD-06 builds.
    pub held_bytes: u64,
    /// While records wait: awake time since the later of the writer's last write and the moment
    /// records began to wait. 0 while none wait; a reservation is never a record waiting.
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

/// Writes `bytes` to `path` (a file in the entry folder) by atomic rename: the folder is created
/// 0700 when missing, a temp file in it is created new with mode 0600, written, synced when
/// `sync`, and renamed over `path`; then the folder is synced when `sync`. `counts.jsonl`,
/// `cursors.json` and `status.json` all use it.
pub(crate) fn write_private(path: &Path, bytes: &[u8], sync: bool) -> io::Result<()> {
    static N: AtomicU64 = AtomicU64::new(0);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no file name"));
    };
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
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
pub fn mode(live: &Live, sealed: bool) -> Mode {
    if sealed {
        Mode::Sealed
    } else if live.off.since.is_some() {
        Mode::Null
    } else if live.writer_idle_ms >= 10_000 {
        Mode::WriterSlow
    } else if live.losing.streams_in_gap > 0 {
        Mode::Losing
    } else {
        Mode::Recording
    }
}

/// The stall exit (PRD-02 section 3): records wait and nothing was written for 60 s of awake time
/// (a suspend is not a stall), or held bytes reached twice the soft guard `guard`.
pub fn stalled(live: &Live, guard: u64) -> bool {
    live.writer_idle_ms >= 60_000 || live.held_bytes >= guard.saturating_mul(2)
}

/// The facts `status.json` carries that the live atomics do not, as a `Status` the status thread
/// copies and overlays with the live fields. Cheap to clone; every call takes the lock for plain
/// stores and no lock is held across I/O.
#[derive(Clone, Default)]
pub struct Board(Arc<Mutex<Facts>>);

#[derive(Default)]
struct Facts {
    status: Status,
    /// The first note of each `what`, with its number.
    first_notes: Vec<(u64, Value)>,
    /// The last 20 notes, with their numbers.
    last_notes: VecDeque<(u64, Value)>,
    notes_seen: u64,
    stop: bool,
}

/// How many recent notes `status.json` keeps beside the first of each `what`.
const LAST_NOTES: usize = 20;

impl Board {
    /// Sets facts the live atomics do not hold: `pid`, `start_id`, `port`, `deadline`, `harness`,
    /// `last_request_at`, `last_native_at`, `sealing`, `after_seal`, `cut_files`, `outlived`.
    /// The live fields are overwritten at each write.
    pub fn set(&self, f: impl FnOnce(&mut Status)) {
        f(&mut super::lock(&self.0).status);
    }

    /// A `note` event as written.
    pub fn note(&self, note: Value) {
        let mut f = super::lock(&self.0);
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

    /// The seal: from now on `mode` is sealed and `written` and `off` are the seal's totals.
    pub fn sealed(&self, seal: SealStatus, totals: &EntryTotals) {
        let s = &mut super::lock(&self.0).status;
        s.seal = Some(seal);
        s.written = Written::new(totals.counts.clone());
        s.off = Off {
            spans: totals.off_spans,
            ms: totals.counts.off_ms,
            since: None,
        };
    }

    /// Stops the status thread after its current write.
    pub fn stop(&self) {
        super::lock(&self.0).stop = true;
    }

    fn stopped(&self) -> bool {
        super::lock(&self.0).stop
    }

    /// These facts with the live atomics, this entry's written counts and the durable point over
    /// them. In `mode: sealed`, `written` and `off` stay the seal's totals. `held_peak` is the
    /// larger of the boot's live peak `held_peak` and the written one, so a stuck writer never
    /// freezes it.
    pub fn status(
        &self,
        live: &Live,
        held_peak: u64,
        counts: Counts,
        durable: DurableStatus,
    ) -> Status {
        let f = super::lock(&self.0);
        let mut s = f.status.clone();
        let sealed = s.seal.is_some();
        if !sealed {
            s.written = Written::new(counts);
            s.off = live.off.clone();
        }
        let first: Vec<u64> = f.first_notes.iter().map(|(n, _)| *n).collect();
        s.notes = f
            .first_notes
            .iter()
            .chain(f.last_notes.iter().filter(|(n, _)| !first.contains(n)))
            .map(|(_, v)| v.clone())
            .collect();
        s.v = STATUS_V;
        s.updated = goalcup_format::dump::wall(clock::wall_ns() / 1_000_000_000 * 1_000_000_000);
        s.boot = live.boot;
        s.mode = mode(live, sealed);
        s.losing = live.losing;
        s.held_bytes = live.held_bytes;
        s.held_peak = held_peak.max(s.written.counts.held_peak);
        s.writer_idle_ms = live.writer_idle_ms;
        s.durable = durable;
        s
    }
}

/// The status thread (PRD-02 sections 3 and 5 step 5): every second it writes `status.json` from
/// the live atomics, the written counts, the durable point and `board`, and checks the stall
/// exit. A stall writes one fixed line and ends the process with 75; the next boot names the span
/// `writer_stalled`. A panic in this thread ends the process with 70, since the stall exit would
/// go unchecked. It runs until [`Board::stop`].
pub fn spawn(rec: Recorder, entry_dir: PathBuf, board: Board) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("goalcup-status".into())
        .spawn(move || {
            let _exit = ExitOnPanic;
            let guard = rec.inner.guard;
            board.set(|s| s.guard = guard);
            while !board.stopped() {
                let live = write(&rec, &entry_dir, &board);
                if stalled(&live, guard) {
                    secure::exit(EXIT_WRITER_STALLED, "writer_stalled");
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("the status thread starts")
}

/// Writes `status.json` once from the live atomics, the written counts, the durable point and
/// `board`; the status thread each second, and the seal once its fields are set. Returns the live
/// state it wrote.
pub(crate) fn write(rec: &Recorder, entry_dir: &Path, board: &Board) -> Live {
    let live = rec.live();
    let d = (rec.inner.durable.borrow().point.as_ref())
        .map(|p| DurableStatus {
            seq: p.line.seq,
            raw_end: p.line.raw_end,
        })
        .unwrap_or_default();
    let peak = rec.inner.held_peak.load(Ordering::Relaxed);
    let _ = board.status(&live, peak, rec.counts(), d).write(entry_dir);
    live
}

/// Ends the process with 70 when the thread holding it unwinds from a panic.
struct ExitOnPanic;

impl Drop for ExitOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            secure::exit(EXIT_PIPELINE_GONE, "status_thread_gone");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stall_exit_at_its_two_marks() {
        let guard = 512 << 20;
        let stalled = |held_bytes, writer_idle_ms| {
            let live = Live {
                held_bytes,
                writer_idle_ms,
                ..Live::default()
            };
            stalled(&live, guard)
        };
        assert!(!stalled(1, 59_999));
        assert!(stalled(1, 60_000));
        assert!(!stalled(2 * guard - 1, 0));
        assert!(stalled(2 * guard, 0));
    }
}
