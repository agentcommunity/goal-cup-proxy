//! `goalcup upload` (PRD-06 section 9): sends what is left of one sealed entry by hand, in the
//! wrap-up's order (final heartbeat, chunks, the final again until sealed), then prunes. It gives
//! up only on 401, 404, 410 or `stuck`; a network or server failure waits on section 5's backoff,
//! with no cap of ours. The words are PRD-10's; [`run`] returns the facts.

use super::beat::{Beat, FinalFacts, health};
use super::chunk::Bound;
use super::client::{RunPosition, ServerClient, ServerError};
use super::policy::{Backoff, Class, Policy, ends_manual_upload};
use super::send::{Sender, Step, lock, log_failure, words};
use super::state::{self, Confirmed, Final, State, UploadJson, Writer};
use super::{SealFacts, TICK, connect, final_due, prune::prune, seal_link};
use crate::cli;
use crate::pipeline::status::{Mode, SealStatus};
use crate::pipeline::{self, Mono, Status};
use crate::watchdog::SealReason;
use goalcup_format::hex;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
/// with its seal, or `upload.json` says the server sealed it; any other is `NotClosed`.
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
    let status = cli::read_json::<Status>(&cli::status_json(entry_dir));
    let seal = status
        .filter(|s| s.mode == Mode::Sealed)
        .and_then(|s| s.seal);
    let stored = old.final_ == Final::Sealed;
    if seal.is_none() && !stored {
        return Outcome::NotClosed;
    }
    let link = seal_link(&entry_dir.join("rec"));
    // Every part pruned: the confirmed end of an entry the server sealed stands for the seal LINK.
    let kept = old.confirmed.clone().filter(|_| stored);
    let end = link.map(|l| (l.raw_end, l.seq, hex(&l.hash)));
    let Some(end) = end.or(kept.map(|c| (c.raw, c.seq, c.hash))) else {
        return Outcome::NotClosed;
    };
    let keep = keep || old.keep;
    let (json, client) = connect(run_dir, entry_dir, ids, keep, old);
    let Some(client) = client else {
        return match json.state {
            State::Local => Outcome::Local,
            _ => Outcome::NoServer,
        };
    };
    let (j, file) = (Mutex::new(json), Mutex::new(Writer::new(entry_dir.into())));
    let out = match position(&client, entry_dir, ids, &j) {
        Err(o) => o,
        Ok(p) if p.sealed && p.offset == end.0 => already(&p, end, entry_dir, (&j, &file)),
        Ok(_) => match link {
            Some(point) => send(entry_dir, client, ids, &point, seal, (&j, &file)),
            None => Outcome::Stuck,
        },
    };
    let _ = lock(&file).put(Mono::now(), &lock(&j).clone(), true);
    out
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

/// The wrap-up's order with the seal's facts from `status.json` (a seal not known there is
/// `recovered`) and the totals from
/// `pipeline::entry_totals` (`counts.jsonl` for pruned boots), then prune.
fn send(
    entry_dir: &Path,
    client: ServerClient,
    (run, entry): (&str, &str),
    point: &goalcup_format::IndexLine,
    seal: Option<SealStatus>,
    (j, file): (&Mutex<UploadJson>, &Mutex<Writer>),
) -> Outcome {
    let p = Policy::from_env();
    let totals = pipeline::entry_totals(entry_dir);
    let live = health(None, entry_dir, &totals).1;
    let seal = SealFacts {
        // Not known: a dead entry's, which never says `ok` for what it cannot vouch for.
        reason: seal.as_ref().map_or(SealReason::Recovered, |s| s.reason),
        workspace_head: seal.and_then(|s| s.workspace_head),
        totals,
    };
    let mut s = Sender::new(&p, client.clone(), run, entry, entry_dir);
    let mut b = Beat::new(&p, client, run, entry, entry_dir);
    let (word, _) = tokio::sync::watch::channel(None);
    let mut tried = false;
    loop {
        let now = Mono::now();
        let snap = lock(j).clone();
        if final_due(&b, now, tried, &snap, point) {
            let waiting_bytes = snap.waiting_bytes;
            let f = FinalFacts {
                point,
                seal: &seal,
                live: &live,
                waiting_bytes,
            };
            b.final_beat(now, &f, j, &word);
            tried = true;
        }
        while s.tick(&Mono::now, &Bound::OnDisk, true, j) == Step::Again {}
        let snap = lock(j).clone();
        let _ = lock(file).put(Mono::now(), &snap, false);
        if snap.final_ == Final::Sealed && snap.state == State::Done {
            prune(entry_dir, j, file);
            return Outcome::Done;
        }
        if snap.state == State::Stuck {
            return Outcome::Stuck;
        }
        let refused = snap.last_error.filter(|e| e.class == Class::Refused);
        if let Some(status) = refused
            .and_then(|e| e.status)
            .filter(|s| ends_manual_upload(*s))
        {
            return give_up(status, j);
        }
        std::thread::sleep(TICK);
    }
}

/// `Refused`, or `Closed` for a 401 once `upload_by` has passed (`upload.json` then says `closed`).
fn give_up(status: u16, j: &Mutex<UploadJson>) -> Outcome {
    let mut j = lock(j);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let by = j.upload_by.clone();
    match by {
        Some(by) if status == 401 && cli::unix_of(&by).is_some_and(|t| t <= now) => {
            j.state = State::Closed;
            Outcome::Closed { upload_by: by }
        }
        _ => Outcome::Refused(status),
    }
}
