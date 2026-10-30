//! Settle, generations and reads of taken session files (PRD-04 sections 4, 5, 7 and 8). Nothing
//! is held between calls: a read opens its file from the root, reads at most `room()`, closes it.
//! Content is never parsed; the file is the buffer.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde_json::json;

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
    /// When the walk last saw it grow (append) or change (snapshot): GAP start, settle clock.
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
    noted: HashSet<(String, i32)>,
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
            if !taken(&stat, self.since) {
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
        } else if e.h.is_none() || stat.fid() != e.stat.fid() || stat.size < e.off {
            if let Some(h) = e.h.take() {
                end(rec, h, e, SourceChanged, false);
            }
            (e.generation, e.off, e.stat, e.seen_at) = (e.generation + 1, 0, stat, now);
            e.h = Some(open_native(rec, &id, e));
        } else {
            if stat.size > e.stat.size {
                e.seen_at = now;
            }
            e.stat = stat;
        }
    }

    /// Paths not settled since the last sweep are gone: GAP if unread, END `source_changed`.
    pub fn sweep(&mut self, rec: &Recorder) {
        self.files.retain(|_, e| {
            if !std::mem::take(&mut e.live) {
                if let Some(h) = e.h.take() {
                    end(rec, h, e, SourceChanged, false);
                }
                return !e.snap;
            }
            true
        });
    }

    /// Reads what is due, round robin from after the file where the last call stopped (section 5).
    pub fn read(&mut self, rec: &Recorder, slice: u64, settle: Duration) -> Stop {
        let now = rec.now().ns();
        let settle = u64::try_from(settle.as_nanos()).unwrap_or(u64::MAX);
        let mut due: Vec<String> = self
            .files
            .iter()
            .filter(|(_, e)| match e.snap {
                false => e.h.is_some() && e.stat.size > e.off,
                true => {
                    let settled = now.saturating_sub(e.seen_at.ns()) >= settle;
                    e.copy.is_some() || (settled && e.done != Some(e.stat))
                }
            })
            .map(|(id, _)| id.clone())
            .collect();
        let after = self
            .last
            .as_ref()
            .map(|l| due.partition_point(|id| id <= l));
        due.rotate_left(after.unwrap_or(0));
        let mut left = slice;
        for id in due {
            if left == 0 || rec.room() == 0 {
                return if left == 0 { Stop::Slice } else { Stop::Room };
            }
            let Some(e) = self.files.get_mut(&id) else {
                continue;
            };
            let got = read_one(rec, &id, e, &mut left);
            if let Err(err) = &got {
                let errno = err.raw_os_error().unwrap_or(0);
                if self.noted.insert((id.clone(), errno)) {
                    let what = "native_unreadable";
                    rec.event(json!({"ev": "note", "what": what, "path": id, "errno": errno}));
                }
            }
            self.last = Some(id);
            if let Ok(Some(stop)) = got {
                return stop;
            }
        }
        Stop::Done
    }

    /// Ends every open stream (section 8 steps 4 and 5). `cut`: the seal's time guard ran out.
    pub fn close(self, rec: &Recorder, cut: bool) -> Closed {
        let mut c = Closed::default();
        for mut e in self.files.into_values() {
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
            if e.copy.take().is_some()
                && let Some(h) = e.h.take()
            {
                h.end(rec.now(), SourceChanged, None);
            }
            return Ok(None);
        }
        f => f?,
    };
    if e.snap && e.copy.is_none() {
        let st = e.stat;
        let fid = fid_hex(st.fid());
        let meta = json!({"path": id, "form": "snapshot", "size": st.size, "mtime_ns": st.mtime_ns, "fid": fid});
        (e.copy, e.off) = (Some(st), 0);
        e.h = Some(rec.open(StreamKind::Snapshot, 0, 0, 0, OpenFlags::NONE, meta));
    }
    let (to, Some(h)) = (want(e), e.h.as_mut()) else {
        return Ok(None);
    };
    while e.off < to {
        let n = rec.room().min(256 << 10).min(to - e.off).min(*left);
        if n == 0 {
            return Ok(Some(if *left == 0 { Stop::Slice } else { Stop::Room }));
        }
        let mut buf = vec![0; n as usize];
        let t = rec.now();
        let k = f.read_at(&mut buf, e.off)?;
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
        } else if let Ok(a) = a {
            (e.stat, e.seen_at) = (a, rec.now());
        }
        let reason = if whole { Complete } else { SourceChanged };
        h.end(rec.now(), reason, None);
    }
    Ok(None)
}

/// Ends a stream; seen bytes left unread get a GAP first, `exact` at the seal and a lower bound
/// when the file changed (sections 4 and 8). A torn snapshot copy needs none. Returns the unread.
fn end(rec: &Recorder, mut h: StreamHandle, e: &Entry, reason: EndReason, exact: bool) -> u64 {
    let n = want(e).saturating_sub(e.off);
    if n > 0 && reason == CutAtSeal {
        h.gap(GapReason::CutAtSeal, e.seen_at, Some(n), None);
    } else if n > 0 && !e.snap {
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

/// The size a read goes to: the copy's for a snapshot copy, else the last seen.
fn want(e: &Entry) -> u64 {
    e.copy.unwrap_or(e.stat).size
}

/// OPEN meta `fid`: device, inode and on macOS birth time, in hex.
pub fn fid_hex(fid: Fid) -> String {
    let birth = fid.birth_ns.map(|b| format!(".{b:x}")).unwrap_or_default();
    format!("{:x}.{:x}{birth}", fid.dev, fid.ino)
}

/// The run-start rule (section 2, D57): max(mtime, ctime) in whole seconds at or after `since`.
pub(crate) fn taken(stat: &Stat, since_secs: i64) -> bool {
    stat.mtime_ns.max(stat.ctime_ns).div_euclid(1_000_000_000) >= since_secs
}

/// Opens `root/rel` with no symlink followed: the root, each folder, then the file with `fid`.
pub(crate) fn open_rel(root: &Path, rel: &Path, fid: Fid) -> io::Result<File> {
    let mut dir = Dir::open_root(root)?;
    for name in rel.parent().into_iter().flat_map(Path::iter) {
        dir = dir.open_child(name)?;
    }
    dir.open_file(rel.file_name().ok_or(io::ErrorKind::NotFound)?, fid)
}
