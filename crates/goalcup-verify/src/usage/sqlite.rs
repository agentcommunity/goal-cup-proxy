//! The Hermes native leg (PRD-09 sections 5 and 6): `state.db` read back out of the record's
//! database copies. A copy is a `set` of `state.db`, `-wal` and `-shm` snapshot streams (PRD-01
//! section 4); the newest set that opens after the WAL checksum repair is read, and only session
//! ids, parent ids, model names and numbers leave this file, never `title`, prompts or messages.
//! Vendor pages: Hermes "Session Storage" at v0.22.5 (`sessions`, `messages`) and the SQLite
//! file format page, "WAL format" (header, frame header, checksum).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, Row};
use serde_json::Value;

use super::shapes::model_name;
use super::{CallRow, Kind, Leg, Outcome, Thread, hints};

/// What the Hermes leg gives: native call rows, threads, parent to child ids, whether the rows
/// have usable times and counts (`per_call`, else the totals form), and whether no copy opened.
#[derive(Default)]
pub(super) struct HermesLeg {
    pub(super) rows: Vec<CallRow>,
    pub(super) threads: Vec<Thread>,
    pub(super) children: BTreeMap<String, Vec<String>>,
    pub(super) per_call: bool,
    pub(super) unreadable: bool,
}

/// One file of a copy: its bytes (`None` after a hole), the size and mtime the stream named.
#[derive(Default)]
struct File {
    bytes: Option<Vec<u8>>,
    size: Option<u64>,
    mtime: Option<u64>,
}

#[derive(Default)]
struct Pair {
    db: Option<File>,
    wal: Option<File>,
}

/// The database streams of an entry, by copy `set` (boot, n). No `Debug`: file bytes (PRD-17 VR2).
#[derive(Default)]
pub(super) struct Sets {
    /// (boot, sid) to its set and whether it is the `-wal`.
    streams: HashMap<(u64, u64), ((u64, u64), bool)>,
    copies: BTreeMap<(u64, u64), Pair>,
}

impl Sets {
    /// A snapshot stream of a database file; `-shm` and a stream with no `set` are not kept.
    pub(super) fn open(&mut self, key: (u64, u64), meta: &Value) {
        let set = meta["set"].as_str().and_then(|s| s.split_once('.'));
        let set = set.and_then(|(b, n)| Some((b.parse().ok()?, n.parse().ok()?)));
        let name = meta["path"]
            .as_str()
            .and_then(|p| p.rsplit(['/', ':']).next());
        let wal = name.is_some_and(|n| n.ends_with("-wal"));
        let Some(set) = set.filter(|_| wal || name.is_some_and(|n| n.ends_with(".db"))) else {
            return;
        };
        let f = File {
            bytes: Some(Vec::new()),
            size: meta["size"].as_u64(),
            mtime: meta["mtime_ns"].as_u64(),
        };
        let pair = self.copies.entry(set).or_default();
        *(if wal { &mut pair.wal } else { &mut pair.db }) = Some(f);
        self.streams.insert(key, (set, wal));
    }

    fn file(&mut self, key: (u64, u64)) -> Option<&mut File> {
        let (set, wal) = *self.streams.get(&key)?;
        let pair = self.copies.get_mut(&set)?;
        (if wal { &mut pair.wal } else { &mut pair.db }).as_mut()
    }

    /// A piece at file offset `off`; one that is not the next breaks the file, and its set.
    pub(super) fn data(&mut self, key: (u64, u64), off: u64, bytes: &[u8]) {
        if let Some(f) = self.file(key) {
            match &mut f.bytes {
                Some(b) if off == b.len() as u64 => b.extend_from_slice(bytes),
                other => *other = None,
            }
        }
    }

    pub(super) fn gap(&mut self, key: (u64, u64)) {
        if let Some(f) = self.file(key) {
            f.bytes = None;
        }
    }

    /// `None` when no database stream was seen. Else the newest set that opens, or `unreadable`.
    /// `start` is the wall ns of the entry's first PART: older sessions are not this run's.
    pub(super) fn finish(self, entry: &str, start: Option<u64>) -> Option<HermesLeg> {
        if self.copies.is_empty() {
            return None;
        }
        let read = |(k, p): (&(u64, u64), &Pair)| read(p, k.0, entry, start);
        let found = self.copies.iter().rev().find_map(read);
        Some(found.unwrap_or_else(|| HermesLeg {
            unreadable: true,
            ..HermesLeg::default()
        }))
    }
}

type Sess = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<f64>,
    Option<u64>,
);

/// The bytes of a file kept whole: no hole, and the size its stream named.
fn whole(f: &File) -> Option<&[u8]> {
    (f.bytes.as_deref()).filter(|b| f.size.is_none_or(|s| s == b.len() as u64))
}

/// The assistant `messages` rows: session id and `timestamp`.
type Msg = (Option<String>, Option<f64>);
type Rows = (Vec<Sess>, Vec<Msg>);

fn read(p: &Pair, boot: u64, entry: &str, start: Option<u64>) -> Option<HermesLeg> {
    let db = whole(p.db.as_ref()?)?;
    let wal = p
        .wal
        .as_ref()
        .map_or(Some(Vec::new()), |f| repair(whole(f)?))?;
    let until = [&p.db, &p.wal]
        .into_iter()
        .flatten()
        .filter_map(|f| f.mtime)
        .max();
    let (sessions, msgs) = query(db, &wal)?;
    let ns = |x: Option<f64>| {
        x.filter(|f| f.is_finite() && *f >= 0.0)
            .map(|f| (f * 1e9) as u64)
    };
    let mut leg = HermesLeg {
        per_call: true,
        ..HermesLeg::default()
    };
    // Session id to (thread, model, assistant rows seen).
    let mut kept: BTreeMap<String, (Thread, Option<String>, u64)> = BTreeMap::new();
    for (id, parent, model, at, calls) in sessions {
        let Some(id) = id else { continue };
        if start.zip(ns(at)).is_some_and(|(s, a)| a < s) {
            continue;
        }
        let th = Thread {
            parent,
            native_total: calls,
            native_until: until,
            reason: None,
            ..Thread::new(&id)
        };
        kept.insert(id, (th, model, 0));
    }
    for (sid, at) in msgs {
        let Some((th, model, n)) = sid.and_then(|s| kept.get_mut(&s)) else {
            continue;
        };
        let at = ns(at);
        *n += 1;
        leg.per_call &= at.is_some();
        // Rows after the copy's time are not known, so the join does not compare them.
        if at.zip(until).is_some_and(|(a, u)| a > u) {
            continue;
        }
        let mut r = CallRow::new(Leg::Native, entry, boot, Kind::Call);
        (r.thread, r.parent, r.model) = (Some(th.id.clone()), th.parent.clone(), model.clone());
        (r.wall_start, r.wall_end, r.outcome, r.whole) = (at, at, Outcome::Ok, true);
        leg.rows.push(r);
    }
    for (th, _, n) in kept.into_values() {
        leg.per_call &= th.native_total.is_none_or(|t| t == n);
        if let Some(p) = &th.parent {
            leg.children
                .entry(p.clone())
                .or_default()
                .push(th.id.clone());
        }
        leg.threads.push(th);
    }
    Some(leg)
}

/// The `sessions` rows (id, parent, model, `started_at`, `api_call_count`) and the assistant
/// `messages` rows (session, `timestamp`) of a database and WAL, opened from a private copy.
fn query(db: &[u8], wal: &[u8]) -> Option<Rows> {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("goalcup-sqlite-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let got = std::fs::create_dir(&dir)
        .ok()
        .and_then(|()| rows(&dir, db, wal));
    let _ = std::fs::remove_dir_all(&dir);
    got
}

fn rows(dir: &Path, db: &[u8], wal: &[u8]) -> Option<Rows> {
    let path = dir.join("state.db");
    std::fs::write(&path, db).ok()?;
    if !wal.is_empty() {
        std::fs::write(dir.join("state.db-wal"), wal).ok()?;
    }
    let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE).ok()?;
    c.execute_batch("PRAGMA query_only=1").ok()?;
    let sessions = all(
        &c,
        "SELECT id, parent_session_id, model, started_at, api_call_count FROM sessions",
        |r| {
            let model = text(r, 2)?.and_then(|s| model_name(Some(&Value::String(s))));
            Ok((
                ident(r, 0)?,
                ident(r, 1)?,
                model,
                num(r, 3)?,
                num(r, 4)?.map(|n| n as u64),
            ))
        },
    )?;
    let msgs = all(
        &c,
        "SELECT session_id, timestamp FROM messages WHERE role = 'assistant'",
        |r| Ok((ident(r, 0)?, num(r, 1)?)),
    )?;
    Some((sessions, msgs))
}

fn all<T>(c: &Connection, sql: &str, f: impl FnMut(&Row) -> rusqlite::Result<T>) -> Option<Vec<T>> {
    let mut st = c.prepare(sql).ok()?;
    let got = st
        .query_map([], f)
        .ok()?
        .collect::<Result<Vec<T>, _>>()
        .ok()?;
    Some(got)
}

/// An id column, kept only as [`hints::id`] allows.
fn ident(r: &Row, i: usize) -> rusqlite::Result<Option<String>> {
    Ok(text(r, i)?.and_then(|s| hints::id(&s)))
}

fn text(r: &Row, i: usize) -> rusqlite::Result<Option<String>> {
    Ok(match r.get_ref(i)? {
        ValueRef::Text(b) => std::str::from_utf8(b).ok().map(str::to_string),
        _ => None,
    })
}

/// A number column; a null or non-numeric value is no value, never a stop.
fn num(r: &Row, i: usize) -> rusqlite::Result<Option<f64>> {
    Ok(match r.get_ref(i)? {
        ValueRef::Integer(n) => Some(n as f64),
        ValueRef::Real(f) => Some(f),
        _ => None,
    })
}

/// The WAL with its header checksum and every valid frame's cumulative checksum recomputed
/// (SQLite file format, WAL format): a mask keeps lengths, so only the checksums break. Frames
/// whose salts differ from the header's are stale and left alone; a partial last frame is cut.
/// `None` when the header is not a WAL header (magic or page size masked).
fn repair(w: &[u8]) -> Option<Vec<u8>> {
    if w.is_empty() {
        return Some(Vec::new());
    }
    let h = w.get(..32)?;
    let word = |i: usize| u32::from_be_bytes(h[i..i + 4].try_into().unwrap_or_default());
    // The words are summed big-endian for magic 0x377f0683, little-endian for 0x377f0682.
    let big = match word(0) {
        0x377f_0682 => false,
        0x377f_0683 => true,
        _ => return None,
    };
    let ps = usize::try_from(word(8))
        .ok()
        .filter(|p| (512..=65536).contains(p) && p.is_power_of_two())?;
    let frame = 24 + ps;
    let n = (w.len() - 32) / frame;
    let mut out = w[..32 + n * frame].to_vec();
    let mut s = sum((0, 0), &w[..24], big);
    out[24..28].copy_from_slice(&s.0.to_be_bytes());
    out[28..32].copy_from_slice(&s.1.to_be_bytes());
    for f in (0..n).map(|i| 32 + i * frame) {
        if w[f + 8..f + 16] != w[16..24] {
            continue;
        }
        s = sum(sum(s, &w[f..f + 8], big), &w[f + 24..f + frame], big);
        out[f + 16..f + 20].copy_from_slice(&s.0.to_be_bytes());
        out[f + 20..f + 24].copy_from_slice(&s.1.to_be_bytes());
    }
    Some(out)
}

/// The WAL checksum over a multiple of 8 bytes, continuing from `(a, b)`.
fn sum((mut a, mut b): (u32, u32), d: &[u8], big: bool) -> (u32, u32) {
    let word = |x: &[u8]| {
        let x: [u8; 4] = x.try_into().unwrap_or_default();
        if big {
            u32::from_be_bytes(x)
        } else {
            u32::from_le_bytes(x)
        }
    };
    for c in d.as_chunks::<8>().0 {
        a = a.wrapping_add(word(&c[..4])).wrapping_add(b);
        b = b.wrapping_add(word(&c[4..])).wrapping_add(a);
    }
    (a, b)
}
