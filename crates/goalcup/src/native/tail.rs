//! Settle, generations and reads of taken session files (PRD-04 sections 4, 5, 7 and 8). Nothing
//! is held between calls: a read opens its file from the root, reads at most `room()`, closes it.
//! Content is never parsed; the file is the buffer.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde_json::json;

use super::Noted;
use super::fsx::{Dir, Stat, file_stat};
use crate::pipeline::cursors::{Cursors, Fid};
use crate::pipeline::{EndReason, GapReason, Mono, OpenFlags, Recorder, StreamHandle, StreamKind};
use EndReason::{Complete, CutAtSeal, SourceChanged};

/// Why `read` returned.
pub enum Stop {
    Done,
    Slice,
    Room,
}

#[derive(Default)]
pub struct Closed {
    pub streams: u64,
    pub cut_files: u64,
    pub cut_bytes: u64,
}

/// One taken path. An append path keeps its stream open until it changes, goes or is closed; a
/// gone one keeps its entry with no stream, so (path, gen) never repeats.
struct Entry {
    root: PathBuf,
    rel: PathBuf,
    stat: Stat,
    /// The last stat that showed unread bytes (append) or a change (snapshot): GAP start, settle
    /// clock.
    seen_at: Mono,
    generation: u64,
    off: u64,
    h: Option<StreamHandle>,
    snap: bool,
    /// A snapshot copy in progress: the stat it copies.
    copy: Option<Stat>,
    /// A snapshot's last whole copy.
    done: Option<Stat>,
    live: bool,
}

#[derive(Default)]
pub struct Tail {
    since: i64,
    cursors: Cursors,
    files: BTreeMap<String, Entry>,
    noted: Noted,
    last: Option<String>,
}

impl Tail {
    pub fn new(since: SystemTime, cursors: Cursors) -> Tail {
        let secs = since.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let since = i64::try_from(secs).unwrap_or(i64::MAX);
        Tail {
            since,
            cursors,
            ..Tail::default()
        }
    }

    /// One walked file against what was seen last (section 4).
    pub fn settle(
        &mut self,
        rec: &Recorder,
        id: String,
        root: &Path,
        rel: &Path,
        stat: Stat,
        snapshot: bool,
    ) {
        let now = rec.now();
        let Some(e) = self.files.get_mut(&id) else {
            if !self.taken(&stat) {
                return;
            }
            let (generation, off) = match self.cursors.native.get(&id) {
                Some(c) if c.fid == stat.fid() && stat.size >= c.off => (c.generation, c.off),
                Some(c) => (c.generation + 1, 0),
                None => (0, 0),
            };
            let c = self.cursors.snapshots.get(&id);
            let done = c.filter(|c| (c.size, c.mtime_ns) == (stat.size, stat.mtime_ns));
            let mut e = Entry {
                root: root.into(),
                rel: rel.into(),
                stat,
                seen_at: now,
                generation,
                off,
                h: None,
                snap: snapshot,
                copy: None,
                done: done.map(|_| stat),
                live: true,
            };
            e.h = (!snapshot).then(|| open_native(rec, &id, &e));
            self.files.insert(id, e);
            return;
        };
        e.live = true;
        if e.snap {
            if stat != e.stat {
                (e.stat, e.seen_at) = (stat, now);
            }
        } else if e.h.is_none() || stat.fid() != e.stat.fid() || stat.size < e.off.max(e.stat.size)
        {
            // A size only falls by truncation: bytes seen at or past the offset may be gone.
            if let Some(h) = e.h.take() {
                end(rec, h, e, SourceChanged, false);
            }
            (e.generation, e.off, e.stat, e.seen_at) = (e.generation + 1, 0, stat, now);
            e.h = Some(open_native(rec, &id, e));
        } else {
            if stat.size > e.off {
                e.seen_at = now;
            }
            e.stat = stat;
        }
    }

    /// The run-start rule (section 2, D57): max(mtime, ctime) in whole seconds at or after `since`.
    pub fn taken(&self, stat: &Stat) -> bool {
        stat.mtime_ns.max(stat.ctime_ns).div_euclid(1_000_000_000) >= self.since
    }

    /// Paths not settled since the last sweep are gone: GAP if unread, END `source_changed`. A gone
    /// snapshot whose last state was not copied whole gets a copy of its own (see `last_copy`) and
    /// its exact loss, since no later copy comes. A path under one of the walk's `unlisted` folders
    /// was not looked at, so it is left alone.
    pub fn sweep(&mut self, rec: &Recorder, unlisted: &[PathBuf]) {
        self.files.retain(|id, e| {
            let held = || unlisted.iter().any(|d| e.root.join(&e.rel).starts_with(d));
            if std::mem::take(&mut e.live) || held() {
                return true;
            }
            if e.snap {
                last_copy(rec, id, e);
            }
            if let Some(h) = e.h.take() {
                end(rec, h, e, SourceChanged, e.snap);
            }
            !e.snap
        });
    }

    /// Reads what is due, round robin from after the file where the last call stopped (section 5).
    pub fn read(&mut self, rec: &Recorder, slice: u64, settle: Duration) -> Stop {
        let now = rec.now().ns();
        let settle = u64::try_from(settle.as_nanos()).unwrap_or(u64::MAX);
        let is_due = |e: &Entry| match e.snap {
            false => e.h.is_some() && e.stat.size > e.off,
            true => {
                let settled = now.saturating_sub(e.seen_at.ns()) >= settle;
                e.copy.is_some() || (settled && e.done != Some(e.stat))
            }
        };
        let due = self.files.iter().filter(|(_, e)| is_due(e));
        let mut due: Vec<String> = due.map(|(id, _)| id.clone()).collect();
        let after = self
            .last
            .as_ref()
            .map(|l| due.partition_point(|id| id <= l));
        due.rotate_left(after.unwrap_or(0));
        let mut left = slice;
        for id in due {
            if left == 0 || rec.room() == 0 {
                return stop(left);
            }
            let Some(e) = self.files.get_mut(&id) else {
                continue;
            };
            let copying = e.copy.is_some();
            let mut got = read_one(rec, &id, e, &mut left);
            // A copy torn by a state already seen and settled: that state is copied now, once.
            if copying && matches!(got, Ok(None)) && e.copy.is_none() && is_due(e) {
                got = read_one(rec, &id, e, &mut left);
            }
            if let Err(err) = &got {
                let errno = err.raw_os_error().unwrap_or(0);
                let what = "native_unreadable";
                let ev = json!({"ev": "note", "what": what, "path": id, "errno": errno});
                self.noted.once(rec, ev);
            }
            self.last = Some(id);
            if let Ok(Some(stop)) = got {
                return stop;
            }
        }
        Stop::Done
    }

    /// Ends every open stream (section 8 steps 4 and 5). A snapshot whose last state was not copied
    /// whole (never opened, or its copy torn by that state) gets a copy of its own, so its loss is
    /// named. `cut`: the seal's time guard ran out, so unread bytes are `cut_at_seal`, else
    /// `source_changed`.
    pub fn close(self, rec: &Recorder, cut: bool) -> Closed {
        let mut c = Closed::default();
        for (id, mut e) in self.files {
            if e.snap {
                last_copy(rec, &id, &mut e);
            }
            let Some(h) = e.h.take() else { continue };
            let reason = match want(&e) == e.off {
                true => Complete,
                false if cut => CutAtSeal,
                false => SourceChanged,
            };
            let n = end(rec, h, &e, reason, true);
            c.streams += u64::from(!e.snap);
            if reason == CutAtSeal {
                (c.cut_files, c.cut_bytes) = (c.cut_files + 1, c.cut_bytes + n);
            }
        }
        c
    }
}

/// One file: opens it, starts a snapshot copy if none runs, reads pieces to the size seen.
fn read_one(rec: &Recorder, id: &str, e: &mut Entry, left: &mut u64) -> io::Result<Option<Stop>> {
    let f = match open_rel(&e.root, &e.rel, e.copy.unwrap_or(e.stat).fid()) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // A copy torn by a newer state ends here, as that state gets a copy of its own; else
            // the next walk says gone (sweep names the loss) or replaced (settle sees the change).
            end_torn(rec, e);
            return Ok(None);
        }
        f => f?,
    };
    if e.snap && e.copy.is_none() {
        begin_copy(rec, id, e);
    }
    let (to, Some(h)) = (want(e), e.h.as_mut()) else {
        return Ok(None);
    };
    while e.off < to {
        let n = rec.room().min(256 << 10).min(to - e.off).min(*left);
        if n == 0 {
            return Ok(Some(stop(*left)));
        }
        let mut buf = vec![0; n as usize];
        let t = rec.now();
        let k = loop {
            match f.read_at(&mut buf, e.off) {
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                k => break k?,
            }
        };
        buf.truncate(k);
        (e.off, *left) = (e.off + k as u64, *left - k as u64);
        if k > 0 {
            h.data(t, Bytes::from(buf));
        }
        if (k as u64) < n {
            break;
        }
    }
    if let Some(c) = e.copy.take()
        && let Some(h) = e.h.take()
    {
        let a = file_stat(&f);
        let whole = e.off == c.size && a.as_ref().ok() == Some(&c);
        if whole {
            e.done = Some(c);
        } else if let Ok(a) = a
            && a != e.stat
        {
            // Only a state not seen before restarts the settle clock: the one seen keeps its own.
            (e.stat, e.seen_at) = (a, rec.now());
        }
        let reason = if whole { Complete } else { SourceChanged };
        h.end(rec.now(), reason, None);
    }
    Ok(None)
}

/// A gone or sealed snapshot's last state: a copy torn by it ends, and if that state was not
/// copied whole it gets a copy of its own, which the caller ends with the exact loss.
fn last_copy(rec: &Recorder, id: &str, e: &mut Entry) {
    end_torn(rec, e);
    if e.copy.is_none() && e.done != Some(e.stat) {
        begin_copy(rec, id, e);
    }
}

/// A snapshot copy of a state older than the last seen ends `source_changed` with no GAP: the
/// newer state gets a copy of its own, which names any loss.
fn end_torn(rec: &Recorder, e: &mut Entry) {
    if e.copy.is_some_and(|c| c != e.stat)
        && let Some(h) = e.h.take()
    {
        e.copy = None;
        h.end(rec.now(), SourceChanged, None);
    }
}

/// Starts a snapshot copy of the last state seen, from byte 0.
fn begin_copy(rec: &Recorder, id: &str, e: &mut Entry) {
    (e.copy, e.off) = (Some(e.stat), 0);
    e.h = Some(open_snapshot(rec, id, &e.stat));
}

/// Ends a stream; seen bytes left unread get a GAP first, `exact` at the seal or for a gone
/// snapshot, a lower bound when the file changed (sections 4 and 8). A torn snapshot copy never
/// comes here (`end_torn`). Returns the unread.
fn end(rec: &Recorder, mut h: StreamHandle, e: &Entry, reason: EndReason, exact: bool) -> u64 {
    let n = want(e).saturating_sub(e.off);
    if n > 0 && reason == CutAtSeal {
        h.gap(GapReason::CutAtSeal, e.seen_at, Some(n), None);
    } else if n > 0 {
        let tail = Some(json!({"unread_at_least": n}));
        let lost = exact.then_some(n);
        h.gap(GapReason::SourceChanged, e.seen_at, lost, tail);
    }
    h.end(rec.now(), reason, None);
    n
}

fn open_native(rec: &Recorder, id: &str, e: &Entry) -> StreamHandle {
    let (size_at_open, fid) = (e.stat.size, fid_hex(e.stat.fid()));
    let meta = json!({"path": id, "gen": e.generation, "size_at_open": size_at_open, "from": e.off, "fid": fid});
    rec.open(StreamKind::Native, 0, 0, e.off, OpenFlags::NONE, meta)
}

/// A snapshot copy's stream (section 4, snapshot form): the copy of `st`.
fn open_snapshot(rec: &Recorder, id: &str, st: &Stat) -> StreamHandle {
    let fid = fid_hex(st.fid());
    let meta = json!({"path": id, "form": "snapshot", "size": st.size, "mtime_ns": st.mtime_ns, "fid": fid});
    rec.open(StreamKind::Snapshot, 0, 0, 0, OpenFlags::NONE, meta)
}

/// Why a read stopped short: the slice is used up, else the room.
fn stop(left: u64) -> Stop {
    if left == 0 { Stop::Slice } else { Stop::Room }
}

/// The size a read goes to: the copy's for a snapshot copy, else the last seen.
fn want(e: &Entry) -> u64 {
    e.copy.unwrap_or(e.stat).size
}

/// OPEN meta `fid`: device, inode and on macOS birth time, in hex.
pub(crate) fn fid_hex(fid: Fid) -> String {
    let birth = fid.birth_ns.map(|b| format!(".{b:x}")).unwrap_or_default();
    format!("{:x}.{:x}{birth}", fid.dev, fid.ino)
}

/// Opens `root/rel` with no symlink followed: the root, each folder, then the file with `fid`.
pub(crate) fn open_rel(root: &Path, rel: &Path, fid: Fid) -> io::Result<File> {
    let mut dir = Dir::open_root(root)?;
    for name in rel.parent().into_iter().flat_map(Path::iter) {
        dir = dir.open_child(name)?;
    }
    dir.open_file(rel.file_name().ok_or(io::ErrorKind::NotFound)?, fid)
}
