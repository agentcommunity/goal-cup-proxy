//! `goalcup upload` (PRD-06 section 9): sends what is left of one sealed entry by hand, in the
//! wrap-up's order (final heartbeat, chunks, the final again until sealed), then prunes. It gives
//! up only on 401, 404, 410 or `stuck`; a network or server failure waits on section 5's backoff,
//! with no cap of ours. The words are PRD-10's; [`run`] returns the facts. The same step, one
//! [`Entry`] round, serves the upload thread's [`Queue`] of older sealed entries (section 1).

use super::beat::{Beat, FinalFacts, health};
use super::chunk::Bound;
use super::client::{RunPosition, ServerClient, ServerError};
use super::policy::{Backoff, Class, Policy, ends_manual_upload};
use super::send::{Sender, Step, lock, log_failure, words};
use super::state::{self, Confirmed, Final, State, UploadJson, UploadLock, Writer};
use super::{SealFacts, ServerWord, TICK, connect, final_due, policy, prune::prune, seal_link};
use crate::cli;
use crate::pipeline::status::{Mode, SealStatus};
use crate::pipeline::{self, Live, Mono, Status};
use crate::watchdog::SealReason;
use goalcup_format::{IndexLine, hex};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

/// What `goalcup upload` found or did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The server holds the whole entry, sealed.
    Done,
    /// The server already held it, sealed: nothing was sent.
    AlreadyThere,
    /// A recorder holds `rec/lock`: the entry sends itself.
    StillRecording,
    /// The entry has no seal and no recorder: `pipeline::close_dead_entry` (PRD-02 9.4) closes it.
    NotClosed,
    /// The run has no server.
    Local,
    /// `server.json` is unreadable or the run credential is missing.
    NoServer,
    /// The server refused the run with this status (401, 404 or 410).
    Refused(u16),
    /// A 401 after `upload_by`: the server closed the run then.
    Closed { upload_by: String },
    /// The server's copy and this computer's differ; nothing is sent across it.
    Stuck,
}

/// Sends the entry in `entry_dir` of the run in `run_dir`. Says `StillRecording` at once while a
/// recorder holds `rec/lock`, checked before `upload.lock` and on every 1 s tick spent waiting for
/// another uploader to let `upload.lock` go. An entry is sealed when `status.json` says `sealed`
/// with its seal, or `upload.json` says the server sealed it; any other is `NotClosed`. A sealed
/// entry with nothing left on disk to check against (every part pruned and `upload.json` lost) is
/// decided by the server: `AlreadyThere` when it holds the entry sealed, `Stuck` otherwise, so a
/// sealed entry is never handed to `pipeline::close_dead_entry`.
pub fn run(entry_dir: &Path, run_dir: &Path, ids: (&str, &str), keep: bool) -> Outcome {
    let _held = loop {
        if recording(entry_dir) {
            return Outcome::StillRecording;
        }
        match state::lock_upload(entry_dir) {
            Some(held) => break held,
            None => std::thread::sleep(TICK),
        }
    };
    if recording(entry_dir) {
        return Outcome::StillRecording;
    }
    let old = state::read(entry_dir).unwrap_or_default();
    let seal = status_seal(entry_dir);
    let stored = old.final_ == Final::Sealed;
    if seal.is_none() && !stored {
        return Outcome::NotClosed;
    }
    let link = seal_link(&entry_dir.join("rec"));
    // Every part pruned: the confirmed end of an entry the server sealed stands for the seal LINK.
    let kept = old.confirmed.clone().filter(|_| stored);
    let end = link.map(|l| (l.raw_end, l.seq, hex(&l.hash)));
    let end = end.or(kept.map(|c| (c.raw, c.seq, c.hash)));
    if end.is_none() && seal.is_none() {
        return Outcome::NotClosed;
    }
    let keep = keep || old.keep;
    let (json, client) = connect(run_dir, entry_dir, ids, keep, old);
    let Some(client) = client else {
        return match json.state {
            State::Local => Outcome::Local,
            _ => Outcome::NoServer,
        };
    };
    let (j, file) = (Mutex::new(json), Mutex::new(Writer::new(entry_dir.into())));
    let out = match (position(&client, entry_dir, ids, &j), end, link) {
        (Err(o), ..) => o,
        (Ok(p), None, _) if p.sealed => Outcome::AlreadyThere,
        (Ok(p), Some(e), _) if p.sealed && p.offset == e.0 => {
            already(&p, e, entry_dir, (&j, &file))
        }
        (Ok(_), Some(_), Some(point)) => {
            let mut e = Entry::new(entry_dir, client, ids, point, seal, (j, file), None);
            let out = loop {
                match e.step(true) {
                    Some(o) => break o,
                    None => std::thread::sleep(TICK),
                }
            };
            e.put();
            return out;
        }
        _ => Outcome::Stuck,
    };
    let _ = lock(&file).put(Mono::now(), &lock(&j).clone(), true);
    out
}

/// `status.json`'s seal, when it says `sealed`.
fn status_seal(entry_dir: &Path) -> Option<SealStatus> {
    let status = cli::read_json::<Status>(&cli::status_json(entry_dir));
    status.filter(|s| s.mode == Mode::Sealed).and_then(|s| s.seal)
}

/// The server holds a sealed entry ending at the seal LINK's offset: when its `next_seq - 1` and
/// `head` match that LINK too, the end is confirmed (section 7), written, and the entry pruned;
/// otherwise the copies differ.
fn already(
    p: &RunPosition,
    (raw, seq, hash): (u64, u64, String),
    entry_dir: &Path,
    (j, file): (&Mutex<UploadJson>, &Mutex<Writer>),
) -> Outcome {
    if p.next_seq.checked_sub(1) != Some(seq) || !p.head.eq_ignore_ascii_case(&hash) {
        lock(j).state = State::Stuck;
        return Outcome::Stuck;
    }
    {
        let mut j = lock(j);
        let at = state::rfc3339(SystemTime::now());
        j.confirmed = Some(Confirmed { raw, seq, hash, at });
        (j.final_, j.state) = (Final::Sealed, State::Done);
    }
    prune(entry_dir, j, file);
    Outcome::AlreadyThere
}

/// Whether a recorder holds `rec/lock`. The probe takes a shared lock for a moment and lets it go
/// with the file; a recorder starting in that moment waits for it (PRD-02 9.2 step 2).
fn recording(entry_dir: &Path) -> bool {
    let f = std::fs::File::open(entry_dir.join("rec").join("lock"));
    f.is_ok_and(|f| matches!(f.try_lock_shared(), Err(std::fs::TryLockError::WouldBlock)))
}

/// The server's position, read again on the backoff until it answers or gives up.
fn position(
    c: &ServerClient,
    entry_dir: &Path,
    (run, entry): (&str, &str),
    j: &Mutex<UploadJson>,
) -> Result<RunPosition, Outcome> {
    let mut b = Backoff::new(&Policy::from_env());
    loop {
        let e = match c.position(run, entry) {
            Ok(p) => return Ok(p),
            Err(ServerError::Status { status, .. }) if ends_manual_upload(status) => {
                return Err(give_up(status, j));
            }
            Err(e) => e,
        };
        let (class, status, _) = words(&e);
        let wait = b.fail(Mono::now(), e.retry_after().map(Duration::from_secs));
        log_failure(entry_dir, "position", class.word(), status, wait);
        while !b.due(Mono::now()) {
            std::thread::sleep(TICK);
        }
    }
}

/// One sealed entry being sent, by `goalcup upload` or by the [`Queue`], with the seal's facts
/// from `status.json` (a seal not known there is `recovered`) and the totals from
/// `pipeline::entry_totals` (`counts.jsonl` for pruned boots).
pub struct Entry {
    dir: PathBuf,
    s: Sender,
    b: Beat,
    tried: bool,
    j: Mutex<UploadJson>,
    file: Mutex<Writer>,
    point: IndexLine,
    seal: SealFacts,
    live: Live,
    word: watch::Sender<Option<ServerWord>>,
    _held: Option<UploadLock>,
}

impl Entry {
    fn new(
        dir: &Path,
        client: ServerClient,
        (run, entry): (&str, &str),
        point: IndexLine,
        seal: Option<SealStatus>,
        (j, file): (Mutex<UploadJson>, Mutex<Writer>),
        held: Option<UploadLock>,
    ) -> Entry {
        let p = Policy::from_env();
        let totals = pipeline::entry_totals(dir);
        let live = health(None, dir, &totals).1;
        let seal = SealFacts {
            // Not known: a dead entry's, which never says `ok` for what it cannot vouch for.
            reason: seal.as_ref().map_or(SealReason::Recovered, |s| s.reason),
            workspace_head: seal.and_then(|s| s.workspace_head),
            totals,
        };
        Entry {
            dir: dir.into(),
            s: Sender::new(&p, client.clone(), run, entry, dir),
            b: Beat::new(&p, client, run, entry, dir),
            tried: false,
            j,
            file,
            point,
            seal,
            live,
            word: watch::channel(None).0,
            _held: held,
        }
    }

    /// One round in the wrap-up's order: the final heartbeat when due, then one sender step, or
    /// every step while it says `Again` (`all`), then prune once sealed and stored. Some once the
    /// entry is finished: done, stuck, or refused with 401, 404 or 410.
    pub fn step(&mut self, all: bool) -> Option<Outcome> {
        let now = Mono::now();
        let snap = lock(&self.j).clone();
        if final_due(&self.b, now, self.tried, &snap, &self.point) {
            let f = FinalFacts {
                point: &self.point,
                seal: &self.seal,
                live: &self.live,
                waiting_bytes: snap.waiting_bytes,
            };
            self.b.final_beat(now, &f, &self.j, &self.word);
            self.tried = true;
        }
        while self.s.tick(&Mono::now, &Bound::OnDisk, true, &self.j) == Step::Again && all {}
        let snap = lock(&self.j).clone();
        let _ = lock(&self.file).put(Mono::now(), &snap, false);
        if snap.final_ == Final::Sealed && snap.state == State::Done {
            prune(&self.dir, &self.j, &self.file);
            return Some(Outcome::Done);
        }
        if snap.state == State::Stuck {
            return Some(Outcome::Stuck);
        }
        let refused = snap.last_error.filter(|e| e.class == Class::Refused);
        let status = refused.and_then(|e| e.status);
        status.filter(|s| ends_manual_upload(*s)).map(|s| give_up(s, &self.j))
    }

    /// Writes `upload.json` now.
    fn put(&self) {
        let _ = lock(&self.file).put(Mono::now(), &lock(&self.j).clone(), true);
    }
}

/// The upload thread's older entries (section 1): every sealed entry under `$GOALCUP_HOME/runs`
/// that is not done or closed, whose run has a credential, that no recorder holds and whose
/// `upload.lock` it could take. Listed at most once a minute; each tick gives each one round, so
/// none holds another or the live entry. An unsealed dead entry is left for `goalcup continue` or
/// `goalcup upload`.
#[derive(Default)]
pub struct Queue {
    entries: Vec<Entry>,
    /// Entries that left the queue unfinished (refused, stuck, nothing to send with): not taken
    /// again while this uploader runs.
    left: Vec<PathBuf>,
    listed: Option<Mono>,
}

impl Queue {
    /// One tick: lists the entries again when due, then gives each one round. An entry past
    /// `upload_by` is dropped with `upload.json` saying `closed`; a finished one lets its
    /// `upload.lock` go.
    pub fn serve(&mut self, home: &Path, live: &Path) {
        let now = Mono::now();
        if self.listed.is_none_or(|t| policy::after(t, 60 * TICK) <= now) {
            self.listed = Some(now);
            self.list(home, live);
        }
        let left = &mut self.left;
        self.entries.retain_mut(|e| {
            if past(lock(&e.j).upload_by.as_deref()) {
                lock(&e.j).state = State::Closed;
            } else if e.step(false).is_none() {
                return true;
            }
            e.put();
            left.push(e.dir.clone());
            false
        });
    }

    /// Adds the older entries not yet queued, oldest run first (by `server.json` time, else the
    /// folder's), each run's entries oldest first.
    fn list(&mut self, home: &Path, live: &Path) {
        for run_dir in by_age(&home.join("runs"), 'r', |p| cli::server_json(p)) {
            for dir in by_age(&run_dir, 'e', Path::to_path_buf) {
                let queued = self.entries.iter().any(|e| e.dir == dir);
                if dir == live || queued || self.left.contains(&dir) {
                    continue;
                }
                match older(&run_dir, &dir) {
                    Some(Ok(e)) => self.entries.push(e),
                    Some(Err(())) => self.left.push(dir),
                    None => {}
                }
            }
        }
    }
}

/// The folders in `dir` whose name starts with `first`, oldest first by the time of `at(folder)`,
/// else the folder's.
fn by_age(dir: &Path, first: char, at: fn(&Path) -> PathBuf) -> Vec<PathBuf> {
    let time = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut found: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(first)))
        .map(|p| (time(&at(&p)).or_else(|| time(&p)), p))
        .collect();
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

/// The older entry in `dir` ready to send; None while it is not (yet) one, `Err` when it is
/// closed now (past `upload_by`) or has nothing to send with.
fn older(run_dir: &Path, dir: &Path) -> Option<Result<Entry, ()>> {
    let old = state::read(dir).unwrap_or_default();
    let seal = status_seal(dir);
    let cred = std::fs::read_to_string(cli::credential(run_dir)).unwrap_or_default();
    let open = !matches!(old.state, State::Done | State::Closed) && !cred.trim().is_empty();
    if !open || (seal.is_none() && old.final_ != Final::Sealed) {
        return None;
    }
    let held = state::lock_upload(dir)?;
    let point = seal_link(&dir.join("rec")).filter(|_| !recording(dir))?;
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    let (run, entry) = (name(run_dir)?, name(dir)?);
    let (mut json, client) = connect(run_dir, dir, (&run, &entry), old.keep, old);
    if past(json.upload_by.as_deref()) {
        json.state = State::Closed;
        let _ = state::write(dir, &json);
        return Some(Err(()));
    }
    let Some(client) = client else {
        return Some(Err(()));
    };
    let j = (Mutex::new(json), Mutex::new(Writer::new(dir.into())));
    let ids = (&*run, &*entry);
    Some(Ok(Entry::new(dir, client, ids, point, seal, j, Some(held))))
}

/// Whether `upload_by` has passed.
fn past(upload_by: Option<&str>) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    upload_by.and_then(cli::unix_of).is_some_and(|t| t <= now)
}

/// `Refused`, or `Closed` for a 401 once `upload_by` has passed (`upload.json` then says `closed`).
fn give_up(status: u16, j: &Mutex<UploadJson>) -> Outcome {
    let mut j = lock(j);
    let by = j.upload_by.clone();
    match by {
        Some(by) if status == 401 && past(Some(&by)) => {
            j.state = State::Closed;
            Outcome::Closed { upload_by: by }
        }
        _ => Outcome::Refused(status),
    }
}
