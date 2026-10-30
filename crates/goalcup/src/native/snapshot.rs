//! Harness databases (PRD-04 sections 6 and 8): the files of a `final` row that share a name once
//! a `-wal`, `-shm` or `-journal` suffix is removed are one set, copied whole as raw bytes during
//! the run (when it changed and its interval passed) and at every seal.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use super::fsx::{Kind, Stat};
use super::tail::{self, Closed, Stop};
use super::{Noted, elapsed, pattern};
use crate::pipeline::{EndReason, GapReason, Mono, OpenFlags, Recorder, StreamHandle, StreamKind};
use EndReason::{Complete, CutAtSeal, SourceChanged};

/// A changed set is due `base` after its last copy (the first after the reader starts), or
/// `per_2mib` per 2 MiB of the set if that is longer.
#[derive(Clone, Copy, Default)]
pub struct Interval {
    pub base: Duration,
    pub per_2mib: Duration,
}

/// (row name, root, path below it with the suffix removed).
type Key = (String, PathBuf, PathBuf);
type Files = BTreeMap<PathBuf, Stat>;

/// One database: its files at the last walk, in this walk and at its last copy (`at`, or 0).
#[derive(Default)]
struct Set {
    files: Files,
    next: Files,
    copied: Files,
    at: Mono,
    taken: bool,
}

/// One file of a copy.
struct Part {
    rel: PathBuf,
    /// The stat it copies.
    st: Stat,
    off: u64,
    h: StreamHandle,
    /// The read failed or stopped short: the file went, shrank or was replaced.
    moved: bool,
    /// The read failed for a reason other than the file going away.
    unreadable: bool,
}

/// A copy in progress.
struct Copying {
    key: Key,
    parts: Vec<Part>,
    began: Mono,
    retry: bool,
    seal: bool,
}

#[derive(Default)]
pub struct Sets {
    iv: Interval,
    born: Mono,
    sets: BTreeMap<Key, Set>,
    copy: Option<Copying>,
    n: u64,
    noted: Noted,
    /// Once sealing: the sets still to copy, `harness_exited`, the streams the seal opened.
    seal: Option<(Vec<Key>, bool, u64)>,
}

impl Sets {
    pub fn new(iv: Interval) -> Sets {
        Sets {
            iv,
            born: Mono::now(),
            ..Sets::default()
        }
    }

    /// One walked file of a `final` row, `taken` by the run-start rule: a set is taken whole
    /// once any of its files is (section 6 step 2).
    pub fn see(&mut self, row: &str, root: &Path, rel: &Path, st: Stat, taken: bool) {
        let b = rel.as_os_str().as_bytes();
        let strip = |x: &&str| b.strip_suffix(x.as_bytes());
        let b = ["-wal", "-shm", "-journal"]
            .iter()
            .find_map(strip)
            .unwrap_or(b);
        let key = (row.into(), root.into(), OsStr::from_bytes(b).into());
        let s = self.sets.entry(key).or_default();
        s.taken |= taken;
        s.next.insert(rel.into(), st);
    }

    /// Ends a walk: a set seen in it takes its files from it, so a file that appeared or vanished
    /// is a change; a set not seen keeps its last files.
    pub fn walked(&mut self) {
        for s in self.sets.values_mut().filter(|s| !s.next.is_empty()) {
            s.files = std::mem::take(&mut s.next);
        }
    }

    /// Section 8 step 3: from now on `read` lets the copy in progress finish, then copies every
    /// taken set once, changed or not.
    pub fn seal(&mut self, harness_exited: bool) {
        let taken = self.sets.iter().filter(|(_, s)| s.taken);
        self.seal = Some((taken.map(|(k, _)| k.clone()).collect(), harness_exited, 0));
    }

    /// Copies what is due within `slice` and the room (section 5). A copy carries over to the
    /// next call, so the loop keeps its slices, its room wait and its walks.
    pub fn read(&mut self, rec: &Recorder, slice: u64) -> Stop {
        let (mut left, iv, born) = (slice, self.iv, self.born);
        loop {
            if self.copy.is_none() {
                let mut ready = self
                    .sets
                    .iter()
                    .filter(|(_, s)| due(s, iv, born, rec.now()));
                let next = match &mut self.seal {
                    Some((todo, ..)) => todo.pop(),
                    None => ready.next().map(|(k, _)| k.clone()),
                };
                let Some(key) = next else {
                    return Stop::Done;
                };
                self.begin(rec, key, false, self.seal.is_some());
            }
            if let Some(stop) = self.step(rec, &mut left) {
                return stop;
            }
        }
    }

    /// Section 8 step 5: the copy in progress and every set the seal did not reach are cut for
    /// every file, each with its exact loss. `streams` counts what the seal opened.
    pub fn close(mut self, rec: &Recorder) -> Closed {
        let mut c = Closed::default();
        let mut todo = self
            .seal
            .as_mut()
            .map(|s| std::mem::take(&mut s.0))
            .unwrap_or_default();
        loop {
            if let Some(Copying { parts, began, .. }) = self.copy.take() {
                for Part { st, off, mut h, .. } in parts {
                    let n = st.size.saturating_sub(off);
                    if n > 0 {
                        h.gap(GapReason::CutAtSeal, began, Some(n), None);
                        (c.cut_files, c.cut_bytes) = (c.cut_files + 1, c.cut_bytes + n);
                    }
                    h.end(rec.now(), CutAtSeal, None);
                }
            }
            let Some(key) = todo.pop() else { break };
            self.begin(rec, key, false, true);
        }
        c.streams = self.seal.map_or(0, |s| s.2);
        c
    }

    /// Opens a stream per file of the set, all with one new `set` id (section 6 step 2).
    fn begin(&mut self, rec: &Recorder, key: Key, retry: bool, seal: bool) {
        let Some(s) = self.sets.get_mut(&key) else {
            return;
        };
        let exited = seal && self.seal.as_ref().is_some_and(|x| x.1);
        let set = format!("{}.{}", rec.live().boot, self.n);
        (self.n, s.copied, s.at) = (self.n + 1, s.files.clone(), rec.now());
        let open = |(rel, st): (&PathBuf, &Stat)| {
            let (path, fid) = (pattern::path_id(&key.0, rel), tail::fid_hex(st.fid()));
            let meta = json!({"path": path, "form": "final", "size": st.size, "mtime_ns": st.mtime_ns, "fid": fid, "set": set, "harness_exited": exited});
            let h = rec.open(StreamKind::Snapshot, 0, 0, 0, OpenFlags::NONE, meta);
            Part {
                rel: rel.clone(),
                st: *st,
                off: 0,
                h,
                moved: false,
                unreadable: false,
            }
        };
        let parts: Vec<Part> = s.files.iter().map(open).collect();
        if let Some(x) = self.seal.as_mut().filter(|_| seal) {
            x.2 += parts.len() as u64;
        }
        if seal && !exited {
            rec.event(json!({"ev": "note", "what": "final_snapshot_live", "set": set}));
        }
        let began = rec.now();
        self.copy = Some(Copying {
            key,
            parts,
            began,
            retry,
            seal,
        });
    }

    /// Reads the copy in progress to the sizes seen, then stats each file again by path. A set
    /// that moved ends `source_changed` and is copied once more; a retry that moved too ends
    /// `complete`, with one `snapshot_unstable` per moved path (section 6 step 3). A file whose
    /// bytes were not all read gets a GAP, exact when the file is gone, so no copy ends short
    /// unnamed.
    fn step(&mut self, rec: &Recorder, left: &mut u64) -> Option<Stop> {
        let Copying { key, parts, .. } = self.copy.as_mut()?;
        for p in parts.iter_mut().filter(|p| !p.moved && p.off < p.st.size) {
            let f = tail::open_rel(&key.1, &p.rel, p.st.fid());
            match f.and_then(|f| tail::pump(rec, &f, &mut p.h, &mut p.off, p.st.size, left)) {
                Ok(Some(stop)) => return Some(stop),
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    let errno = e.raw_os_error().unwrap_or(0);
                    let path = pattern::path_id(&key.0, &p.rel);
                    let ev = json!({"ev": "note", "what": "native_unreadable", "path": path, "errno": errno});
                    self.noted.once(rec, ev);
                    (p.moved, p.unreadable) = (true, true);
                }
                got => p.moved = got.is_err() || p.off < p.st.size,
            }
        }
        let Copying {
            key,
            parts,
            began,
            retry,
            seal,
        } = self.copy.take()?;
        let now: Vec<_> = parts
            .iter()
            .map(|p| tail::stat_rel(&key.1, &p.rel))
            .collect();
        let moved = |(p, now): (&Part, &io::Result<Stat>)| p.moved || now.as_ref().ok() != Some(&p.st);
        let any = parts.iter().zip(&now).any(moved);
        let fresh = parts.iter().zip(&now).filter_map(|(p, st)| {
            let st = *st.as_ref().ok()?;
            (st.kind == Kind::File).then(|| (p.rel.clone(), st))
        });
        let fresh: Files = fresh.collect();
        let reason = if retry || !any { Complete } else { SourceChanged };
        for (mut p, now) in parts.into_iter().zip(&now) {
            let path = pattern::path_id(&key.0, &p.rel);
            if retry && moved((&p, now)) && !p.unreadable {
                rec.event(json!({"ev": "note", "what": "snapshot_unstable", "path": path}));
            }
            let n = p.st.size.saturating_sub(p.off);
            if n > 0 {
                let gone = now.as_ref().is_err_and(|e| e.kind() == io::ErrorKind::NotFound);
                let tail = json!({"unread_at_least": n});
                p.h.gap(GapReason::SourceChanged, began, gone.then_some(n), Some(tail));
            }
            p.h.end(rec.now(), reason, None);
        }
        if any && !retry {
            self.sets.get_mut(&key)?.files = fresh;
            self.begin(rec, key, true, seal);
        }
        None
    }
}

/// A taken set changed since its last copy and its interval passed (section 6 step 1).
fn due(s: &Set, iv: Interval, born: Mono, now: Mono) -> bool {
    let size: u64 = s.files.values().map(|st| st.size).sum();
    let every = iv
        .per_2mib
        .saturating_mul(u32::try_from(size >> 21).unwrap_or(u32::MAX));
    s.taken && s.files != s.copied && elapsed(s.at.max(born), now) >= iv.base.max(every)
}
