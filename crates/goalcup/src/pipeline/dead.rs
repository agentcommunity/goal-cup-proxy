//! Closing an entry no recorder will finish (PRD-02 section 9.4): `goalcup continue`, `seal` and
//! `upload` after both processes died, and the null recorder's seal (section 11 "In null mode").
//! One recovery frame holds the dead boot's COUNT, one `recorder_down` GAP, the `seal` event, a
//! LINK and the `R` line (PRD-01 section 12); nothing is appended after it. An entry that left no
//! index line at all has no frame to recover: boot 0 is written afresh (`start`, the gap, a COUNT,
//! the seal) and closed by an ordinary seal, so its index ends with an `E` line, and with no part
//! file left the gap's start is its end.

use super::boot::{WINDOW_LOG, down_tail};
use super::clock::{self, now_pair};
use super::counts::{self, Counter, CountsLog, EntryTotals};
use super::cursors::Cursors;
use super::status::{Board, DurableStatus, Live, Mode, SealStatus, Status};
use super::{OpenFlags, seal, writer};
use crate::cli::log::{self, Fact};
use crate::upload::SealFacts;
use crate::watchdog::{Exit, HarnessFacts, Prev, SealReason};
use goalcup_format::read::{decode, part_files, read_index};
use goalcup_format::{GapReason, Now, PartWriter, Record, Recover, Recovered};
use serde_json::{Map, Value, json};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// A recorder holds `rec/lock`: nothing was written.
#[derive(Debug)]
pub struct StillRecording;

/// What `close_dead_entry` did. An I/O failure is never a live recorder.
#[derive(Debug)]
// The seam shape (PRD-00): a close happens once per entry, so the size is no cost.
#[allow(clippy::large_enum_variant)]
pub enum Closed {
    Sealed(SealFacts),
    AlreadySealed,
    Failed(io::ErrorKind),
}

/// Closes a dead entry with seal reason `recovered` (PRD-02 section 9.4): takes `rec/lock` once,
/// with no wait, and holds it until it returns; recovers the last part file with the `recovered`
/// seal in the recovery frame; syncs `counts.jsonl`; writes the sealed `status.json`, keeping the
/// dead recorder's `pid`, `start_id` and `port`; removes `rec/run-ca.key`.
pub fn close_dead_entry(entry_dir: &Path) -> Result<Closed, StillRecording> {
    let r = try_lock(entry_dir).and_then(|lock| {
        let Some(_lock) = lock else { return Ok(None) };
        let mut tail = Map::new();
        tail.insert("cause".into(), "abandoned".into());
        let ev = json!({"ev": "seal", "reason": "recovered", "harness": "unknown", "exit": null,
            "workspace_head": null, "dirty": null});
        let (facts, fresh, durable, boot) = close(entry_dir, |_| tail, ev)?;
        let old = Status::read(entry_dir);
        if fresh || old.as_ref().is_none_or(|s| s.mode != Mode::Sealed) {
            let old = old.unwrap_or_default();
            let board = Board::default();
            board.set(|s| {
                *s = old.clone();
                s.sealing = None;
            });
            board.sealed(seal_status(&facts), &facts.totals);
            let live = Live {
                boot,
                ..Live::default()
            };
            let mut s = board.status(&live, 0, Default::default(), durable);
            s.notes = old.notes;
            s.write(entry_dir)?;
        }
        seal::remove_key(entry_dir);
        Ok(Some(fresh.then_some(facts)))
    });
    match r {
        Ok(None) => Err(StillRecording),
        Ok(Some(Some(facts))) => Ok(Closed::Sealed(facts)),
        Ok(Some(None)) => Ok(Closed::AlreadySealed),
        Err(e) => {
            log::line(entry_dir, "close_dead_entry", &[("io", Fact::Io(e.kind()))]);
            Ok(Closed::Failed(e.kind()))
        }
    }
}

/// What the sealed `status.json` says of a seal.
pub(super) fn seal_status(facts: &SealFacts) -> SealStatus {
    SealStatus {
        reason: facts.reason,
        workspace_head: facts.workspace_head.clone(),
    }
}

/// The null recorder's seal (section 11 "In null mode"), under the `rec/lock` its boot holds: the
/// same recovery with the `null_mode` cause from `prev` and the real reason, harness state, exit,
/// `server_reason` and workspace state. `rec/run-ca.key` is gone when it returns `Ok`; the caller
/// writes the sealed `status.json`.
pub(super) fn seal_null(
    entry_dir: &Path,
    reason: SealReason,
    harness: Option<&HarnessFacts>,
    exit: Option<Exit>,
    server_reason: Option<String>,
    prev: Prev,
    workspace: Option<&Path>,
) -> io::Result<SealFacts> {
    let exited = reason == SealReason::HarnessExit || exit.is_some();
    let harness = match (exited, harness) {
        (true, _) => "exited",
        (false, Some(_)) => "running",
        (false, None) => "unknown",
    };
    let head = workspace.and_then(|w| seal::git(w, &["rev-parse", "HEAD"]));
    let head = head.and_then(|b| Some(String::from_utf8(b).ok()?.trim().to_string()));
    let dirty = workspace.and_then(|w| seal::git(w, &["status", "--porcelain", "-z"]));
    let dirty = dirty.map(|b| hex::encode(<sha2::Sha256 as sha2::Digest>::digest(b)));
    let mut ev = json!({"ev": "seal", "reason": reason, "exit": exit, "harness": harness,
        "workspace_head": head, "dirty": dirty});
    if let Some(r) = server_reason.filter(|_| reason == SealReason::ServerStop) {
        ev["server_reason"] = r.into();
    }
    let (facts, ..) = close(entry_dir, |lineless| down_tail(Some(prev), lineless), ev)?;
    seal::remove_key(entry_dir);
    Ok(facts)
}

/// The recovery with `seal` (given `boots` and the entry totals here), then `counts.jsonl`
/// synced. Returns the seal's facts, whether this call wrote it (false when the files say the
/// entry was already sealed, which appends nothing), the durable point and the last boot.
fn close(
    dir: &Path,
    tail: impl FnOnce(bool) -> Map<String, Value>,
    mut seal: Value,
) -> io::Result<(SealFacts, bool, DurableStatus, u64)> {
    let rv = Recovery::of(dir);
    // Recovery's first step, done first, so the totals and the gap start see what it keeps.
    if let Some((part, len)) = &rv.durable {
        for p in part_files(&dir.join("rec"))? {
            let cut = match name(&p).as_str().cmp(part.as_str()) {
                std::cmp::Ordering::Greater => 0,
                std::cmp::Ordering::Equal => *len,
                std::cmp::Ordering::Less => continue,
            };
            match OpenOptions::new().write(true).open(idx(&p)) {
                Ok(f) if f.metadata()?.len() > cut => f.set_len(cut)?,
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
    }
    let entry = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let kept = last_kept(&dir.join("rec"))?;
    // The dead boot's COUNT recovery writes, put in `counts.jsonl` first when its line at the
    // last LINK is missing, so the totals here and that COUNT are one figure.
    if let Some(k) = kept.as_ref().filter(|k| k.seal.is_none())
        && counts::count_at(dir, k.durable.seq).is_none()
        && let Ok(c) = serde_json::from_slice::<Value>(&counts::recount_at(dir, k.durable.seq))
    {
        CountsLog::open(dir).append(k.durable.seq, &c, 0)?;
    }
    let mut totals = counts::entry_totals(dir);
    let tail = tail(rv.lineless);
    if let Some(k @ Kept { seal: Some(s), .. }) = &kept {
        let reason = serde_json::from_value(s["reason"].clone()).unwrap_or(SealReason::Recovered);
        let workspace_head = s["workspace_head"].as_str().map(str::to_string);
        let facts = SealFacts {
            reason,
            workspace_head,
            totals,
        };
        return Ok((facts, false, k.durable, k.boot));
    }
    let mut counter = Counter::new(&[]);
    let boot = kept.as_ref().map_or(0, |k| k.boot);
    let reason = serde_json::from_value(seal["reason"].clone()).unwrap_or(SealReason::Recovered);
    let workspace_head = seal["workspace_head"].as_str().map(str::to_string);
    let mut seal_bytes = |counter: &Counter, totals: &mut EntryTotals| {
        totals.counts.add(&counter.counts());
        totals.off_spans += counter.off_spans();
        seal["boots"] = (boot + 1).into();
        seal["totals"] = serde_json::to_value(&totals.counts).unwrap_or_default();
        seal.to_string().into_bytes()
    };
    let line = match kept {
        Some(k) => {
            // The GAP recovery writes, from the last LINK to now (PRD-01 section 12 step 3).
            let (mut sw, mut sm) = (k.at.wall_ns.min(rv.now.wall_ns), k.at.mono_ns);
            let mut t = tail.clone();
            if k.boot_id == rv.id {
                sm = sm.min(rv.now.mono_ns.saturating_sub(1));
            } else {
                t.insert("boot_id".into(), hex::encode(rv.id).into());
                sw = sw.min(rv.now.wall_ns.saturating_sub(1));
            }
            counter.written(&gap(sw, sm, rv.now, &t), OpenFlags::NONE, None);
            let ev = seal_bytes(&counter, &mut totals);
            rv.recover(dir, entry, tail, None, Some(ev))?.line
        }
        None => {
            // No index line anywhere: boot 0 afresh with its `start`, the gap and the seal.
            let r = rv.recover(dir, entry, tail, None, None)?;
            let (Some(mut w), Some(down)) = (r.writer, r.gap_after_start) else {
                return Err(io::Error::other("no boot 0 after recovery"));
            };
            let t = rv.now.mono_ns;
            let start = json!({"ev": "start", "entry": entry, "goalcup": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "resume": false});
            for r in [event(t, &start), down] {
                counter.written(&r, OpenFlags::NONE, None);
                w.write(&r)?;
            }
            w.write(&event(t, &counter.count_event(0, 0, 0, 0)))?;
            let ev = seal_bytes(&counter, &mut totals);
            w.write(&Record::Event { t, json: ev })?;
            Some(w.seal(now_pair())?.line)
        }
    };
    let totals = CountsLog::open(dir).sync(dir)?;
    let d = line.map_or_else(DurableStatus::default, |l| DurableStatus {
        seq: l.seq,
        raw_end: l.raw_end,
    });
    let facts = SealFacts {
        reason,
        workspace_head,
        totals,
    };
    Ok((facts, true, d, boot))
}

fn event(t: u64, v: &Value) -> Record {
    Record::Event {
        t,
        json: v.to_string().into_bytes(),
    }
}

fn gap(start_wall: u64, start_mono: u64, end: Now, tail: &Map<String, Value>) -> Record {
    Record::Gap {
        sid: 0,
        reason: GapReason::RecorderDown,
        start_wall,
        end_wall: end.wall_ns,
        start_mono,
        end_mono: end.mono_ns,
        off: 0,
        lost: 0,
        tail: serde_json::to_vec(tail).unwrap_or_default(),
    }
}

/// The last part file recovery keeps, read up to its last whole index line.
struct Kept {
    boot: u64,
    boot_id: [u8; 16],
    /// Its last LINK's time.
    at: Now,
    durable: DurableStatus,
    /// The seal event right before that LINK: the entry is sealed.
    seal: Option<Value>,
}

/// None when no part file has an index line.
fn last_kept(rec: &Path) -> io::Result<Option<Kept>> {
    for p in part_files(rec)?.iter().rev() {
        let flen = std::fs::metadata(p)?.len();
        let lines = match read_index(&idx(p)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            l => l?,
        };
        let Some(line) = lines.into_iter().take_while(|l| l.c_end <= flen).last() else {
            continue;
        };
        let (seq, raw_end) = (line.seq, line.raw_end);
        let mut k = Kept {
            boot: 0,
            boot_id: [0; 16],
            at: Now {
                wall_ns: 0,
                mono_ns: 0,
            },
            durable: DurableStatus { seq, raw_end },
            seal: None,
        };
        let mut seal_last = None;
        for it in decode(File::open(p)?.take(line.c_end))? {
            let rec = it?.rec;
            let ev = match &rec {
                Record::Event { json, .. } => serde_json::from_slice::<Value>(json).ok(),
                _ => None,
            };
            match rec {
                Record::Part { boot, boot_id, .. } => (k.boot, k.boot_id) = (boot, boot_id),
                Record::Link {
                    wall_ns, mono_ns, ..
                } => (k.at, k.seal) = (Now { wall_ns, mono_ns }, seal_last.take()),
                _ => {}
            }
            seal_last = ev.filter(|v| v["ev"] == "seal");
        }
        return Ok(Some(k));
    }
    Ok(None)
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A part file's index.
fn idx(p: &Path) -> PathBuf {
    p.with_file_name(format!("{}.idx", name(p).trim_end_matches(".gcr.zst")))
}

/// Takes `rec/lock` with `flock` once, with no wait: None while another process holds it.
pub(super) fn try_lock(entry_dir: &Path) -> io::Result<Option<File>> {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(entry_dir.join("rec").join("lock"))?;
    // SAFETY: `f` is an open file for the whole call; flock changes no memory.
    let held = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0;
    Ok((!held).then_some(f))
}

/// What `PartWriter::recover` takes from the entry and this machine boot (PRD-02 section 9.2 step
/// 4), built one way for a boot's recovery and for a dead entry.
pub(super) struct Recovery {
    pub id: [u8; 16],
    pub now: Now,
    /// `cursors.json`'s durable index length, only under a new `boot_id`: it cuts index lines a
    /// reboot may have lost.
    pub durable: Option<(String, u64)>,
    /// No part file has an index line.
    pub lineless: bool,
}

impl Recovery {
    pub(super) fn of(entry_dir: &Path) -> Recovery {
        let (now, id) = (now_pair(), clock::boot_id());
        let cursors = Cursors::read(entry_dir);
        let me = hex::encode(id);
        let durable = (cursors.boot_id.as_ref())
            .filter(|b| **b != me)
            .and(cursors.durable)
            .map(|d| (d.part, d.index_len));
        let lineless = !counts::has_index_line(&entry_dir.join("rec"));
        Recovery {
            id,
            now,
            durable,
            lineless,
        }
    }

    /// Recovers `entry_dir/rec`. The dead boot's COUNT is its `counts.jsonl` line at that LINK,
    /// or a recount when the line is gone (a torn tail).
    pub(super) fn recover(
        &self,
        entry_dir: &Path,
        entry: &str,
        tail: Map<String, Value>,
        dead_start: Option<(Now, [u8; 16])>,
        seal: Option<Vec<u8>>,
    ) -> io::Result<Recovered> {
        let count_at = |seq| {
            counts::count_at(entry_dir, seq).unwrap_or_else(|| counts::recount_at(entry_dir, seq))
        };
        let rc = Recover {
            entry,
            boot_id: self.id,
            now: self.now,
            durable: (self.durable.as_ref()).map(|(p, l)| (p.as_str(), *l)),
            count_at: &count_at,
            tail,
            dead_start,
            seal,
        };
        PartWriter::recover(&entry_dir.join("rec"), writer::part_options(WINDOW_LOG), rc)
    }
}

/// The null recorder's seal on a dead entry in `entry_dir`, for tests: no harness, exit,
/// server reason or workspace.
#[cfg(feature = "test-hooks")]
pub fn seal_null_for_tests(
    entry_dir: &Path,
    reason: SealReason,
    prev: Prev,
) -> io::Result<SealFacts> {
    seal_null(entry_dir, reason, None, None, None, prev, None)
}
