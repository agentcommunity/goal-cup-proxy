//! The heartbeat (PRD-06 section 6): the flat `counts`, the recording state, the answer as
//! [`ServerWord`] and `upload.json` `server_word`, the ordinary beat, the final heartbeat, and the
//! on-disk point a beat names when no live writer publishes one.

use super::SealFacts;
use super::ServerWord;
use super::client::{HeartbeatAnswer, HeartbeatRequest, Recording, ServerClient, ServerError};
use super::policy::{Backoff, Class, Policy, Timer, class_of_status};
use super::state::{self, FileWord, Final, UploadJson};
use crate::cli::log::{Fact, line};
use crate::pipeline::status::Mode;
use crate::pipeline::{Durable, EntryTotals, Live, Mono, Status};
use goalcup_format::IndexLine;
use goalcup_format::read::{part_files, read_index};
use serde_json::{Map, Value, json};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tokio::sync::watch;

/// The recorder as the beat sees it: its mode and its live facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Health(pub Mode, pub Live);

/// The health of the live recorder, or else of the entry in `entry_dir` as its `status.json` says
/// (null mode, `goalcup upload`, a dead entry); a missing one reads as null mode with the totals'
/// off time.
pub fn health(live: Option<&Live>, entry_dir: &Path, t: &EntryTotals) -> Health {
    if let Some(l) = live {
        let mode = match (l.held_bytes, l.writer_idle_ms) {
            (1.., 10_000..) => Mode::WriterSlow,
            _ if l.losing.streams_in_gap > 0 => Mode::Losing,
            _ => Mode::Recording,
        };
        return Health(mode, l.clone());
    }
    let mut l = Live::default();
    let Some(s) = crate::cli::read_json::<Status>(&crate::cli::status_json(entry_dir)) else {
        (l.off.ms, l.off.spans) = (t.counts.off_ms, t.off_spans);
        return Health(Mode::Null, l);
    };
    (l.boot, l.held_bytes, l.writer_idle_ms) = (s.boot, s.held_bytes, s.writer_idle_ms);
    (l.losing, l.off) = (s.losing, s.off);
    Health(s.mode, l)
}

/// The point a beat names when no live writer publishes one (null mode, `goalcup upload`, a dead
/// entry): the last whole index line of the last part file whose `c_end` is within that file, its
/// part and index synced (section 3). A part with no `.idx` has no lines.
pub fn point_on_disk(rec_dir: &Path) -> io::Result<Option<IndexLine>> {
    for part in part_files(rec_dir)?.iter().rev() {
        let name = part
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let idx = part.with_file_name(name.replace(".gcr.zst", ".idx"));
        let lines = match read_index(&idx) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            r => r?,
        };
        let len = fs::metadata(part)?.len();
        if let Some(l) = lines.iter().rev().find(|l| l.c_end <= len) {
            File::open(part)?.sync_data()?;
            File::open(&idx)?.sync_data()?;
            return Ok(Some(*l));
        }
    }
    Ok(None)
}

/// The heartbeat's `counts`: one flat object, every whole-number COUNT counter as entry totals
/// (the maps are left out), plus the keys of PRD-06 section 6.
pub fn counts(Health(mode, l): &Health, t: &EntryTotals, waiting_bytes: u64) -> Map<String, Value> {
    let all = serde_json::to_value(&t.counts).unwrap_or_default();
    let mut m = all.as_object().cloned().unwrap_or_default();
    m.retain(|_, v| !v.is_object() && !v.is_array());
    let mode = (*mode != Mode::Null).then(|| json!(mode));
    let keys = json!({"v": 1, "boot": l.boot, "mode": mode, "off_ms": l.off.ms,
        "off_spans": l.off.spans, "off_since": l.off.since, "waiting_bytes": waiting_bytes});
    m.extend(keys.as_object().cloned().unwrap_or_default());
    m
}

/// The recording state of section 6's table. Never `unknown`.
pub fn recording(Health(mode, l): &Health, t: &EntryTotals) -> Recording {
    let c = &t.counts;
    let lost = [
        c.gaps,
        c.lost_bytes,
        c.lost_unknown,
        l.off.ms,
        l.losing.streams_in_gap,
    ];
    match mode {
        Mode::Null => Recording::Failed,
        _ if l.held_bytes > 0 && l.writer_idle_ms > 60_000 => Recording::Failed,
        Mode::Recording | Mode::Sealed if lost.iter().all(|n| *n == 0) => Recording::Ok,
        _ => Recording::Gaps,
    }
}

/// The heartbeat for the LINK `point`; `repo_head` goes on the final heartbeat only.
pub fn request(
    entry_id: &str,
    point: &IndexLine,
    t: &EntryTotals,
    recording: Recording,
    counts: Map<String, Value>,
    is_final: bool,
    repo_head: Option<String>,
) -> HeartbeatRequest {
    HeartbeatRequest {
        entry_id: entry_id.to_owned(),
        seq: point.seq,
        head: goalcup_format::hex(&point.hash),
        offset_local: point.raw_end,
        is_final,
        repo_head,
        recording,
        model_requests: t.counts.model_requests(),
        counts,
    }
}

/// Takes a heartbeat answer into `upload.json`. `asked_at` and `asked_wall` were read before the
/// call; the deadline counts from `asked_at`, boot time, so a wall clock set back never moves it.
/// `upload_by` after `replaced` only ever moves earlier: every later answer says `replaced` again,
/// and the server closes the run a day after the replacement itself.
pub fn take_answer(
    j: &mut UploadJson,
    a: &HeartbeatAnswer,
    sent: Recording,
    asked_at: Mono,
    asked_wall: SystemTime,
) -> ServerWord {
    let word = ServerWord {
        stop: a.stop,
        reason: a.reason.clone(),
        seconds_left: a.seconds_left,
        asked_at,
    };
    let file = FileWord::new(&word, asked_wall);
    j.beat.last_ok = Some(file.asked_at.clone());
    j.server_word = Some(file);
    j.beat.failures = 0;
    j.beat.recording = Some(sent);
    if a.sealed {
        j.final_ = Final::Sealed;
    }
    if a.stop && a.reason.as_deref() == Some("replaced") {
        let unix = |s: &Option<String>| s.as_deref().and_then(crate::cli::unix_of);
        let by = asked_wall
            .checked_add(Duration::from_secs(86_400))
            .map(state::rfc3339);
        if unix(&by).is_some_and(|n| unix(&j.upload_by).is_none_or(|o| n < o)) {
            j.upload_by = by;
        }
    }
    word
}

/// What the final heartbeat names: the seal LINK (the last `E` or `R` index line), the seal's
/// facts, and the recorder's live facts, sent as `mode: sealed`.
pub struct FinalFacts<'a> {
    pub point: &'a IndexLine,
    pub seal: &'a SealFacts,
    pub live: &'a Live,
    pub waiting_bytes: u64,
}

/// The beat of one entry, ordinary and final, with its own backoff.
pub struct Beat {
    client: ServerClient,
    run: String,
    entry: String,
    entry_dir: PathBuf,
    timer: Timer,
    pub backoff: Backoff,
    /// The recording state last answered.
    last: Option<Recording>,
    /// A final heartbeat was tried: no ordinary beat follows (PRD-17 S4).
    finals: bool,
}

impl Beat {
    pub fn new(p: &Policy, client: ServerClient, run: &str, entry: &str, entry_dir: &Path) -> Beat {
        Beat {
            client,
            run: run.to_owned(),
            entry: entry.to_owned(),
            entry_dir: entry_dir.to_path_buf(),
            timer: Timer::new(p.beat, p.spread_first),
            backoff: Backoff::new(p),
            last: None,
            finals: false,
        }
    }

    /// At once when the state changed, else on the timer, and never while its backoff waits;
    /// `tick` beats only with a durable point. Never once a final heartbeat was tried.
    pub fn due(&self, now: Mono, recording: Recording) -> bool {
        let changed = self.last != Some(recording);
        !self.finals && self.backoff.due(now) && (changed || self.timer.due(now))
    }

    /// Whether the final heartbeat may go (again): only its backoff waits. The caller repeats it
    /// until `upload.json` `final` says sealed or the wrap-up ends.
    pub fn final_due(&self, now: Mono) -> bool {
        self.backoff.due(now)
    }

    /// Sends the final heartbeat once: `final: true` at the seal LINK with the seal's
    /// `workspace_head` and totals. From here on `tick` sends nothing.
    pub fn final_beat(
        &mut self,
        now: Mono,
        f: &FinalFacts<'_>,
        j: &Mutex<UploadJson>,
        word: &watch::Sender<Option<ServerWord>>,
    ) -> Option<ServerWord> {
        self.finals = true;
        let (h, t) = (Health(Mode::Sealed, f.live.clone()), &f.seal.totals);
        let c = counts(&h, t, f.waiting_bytes);
        let head = f.seal.workspace_head.clone();
        let req = request(&self.entry, f.point, t, recording(&h, t), c, true, head);
        self.send(now, &req, j, word)
    }

    /// One ordinary tick: beats from `durable` if due. True when a beat was answered.
    pub fn tick(
        &mut self,
        now: Mono,
        durable: &Durable,
        h: &Health,
        waiting_bytes: u64,
        j: &Mutex<UploadJson>,
        word: &watch::Sender<Option<ServerWord>>,
    ) -> bool {
        let (t, rec) = (&durable.totals, recording(h, &durable.totals));
        if !self.due(now, rec) {
            return false;
        }
        let Some(point) = &durable.point else {
            return false;
        };
        let req = request(
            &self.entry,
            &point.line,
            t,
            rec,
            counts(h, t, waiting_bytes),
            false,
            None,
        );
        self.send(now, &req, j, word).is_some()
    }

    /// Sends `req` once, `now` read before the call. An answer goes into `upload.json` and onto
    /// `word`; a failure backs off and writes one `goalcup.log` line.
    pub fn send(
        &mut self,
        now: Mono,
        req: &HeartbeatRequest,
        j: &Mutex<UploadJson>,
        word: &watch::Sender<Option<ServerWord>>,
    ) -> Option<ServerWord> {
        let wall = SystemTime::now();
        let answer = self.client.heartbeat(&self.run, req);
        let mut j = j.lock().unwrap_or_else(|e| e.into_inner());
        let out = match answer {
            Ok(a) => {
                let w = take_answer(&mut j, &a, req.recording, now, wall);
                if req.is_final && j.final_ == Final::None {
                    j.final_ = Final::Sent;
                }
                self.backoff.progress();
                self.timer.started(now);
                self.last = Some(req.recording);
                word.send_replace(Some(w.clone()));
                Some(w)
            }
            Err(e) => {
                let wait = self
                    .backoff
                    .fail(now, e.retry_after().map(Duration::from_secs));
                j.beat.failures = self.backoff.failures;
                self.log_failure(&e, wait);
                None
            }
        };
        let _ = state::write(&self.entry_dir, &j);
        out
    }

    /// `upload what=beat class=<class> status=<n or -> wait_s=<n>`: fixed words and numbers only.
    /// A 409 has no class (section 5) and logs `none`; an unreadable success is `internal`.
    fn log_failure(&self, e: &ServerError, wait: Duration) {
        let (class, status) = match e {
            ServerError::Status { status, .. } => (
                class_of_status(*status).map_or("none", Class::word),
                Fact::N((*status).into()),
            ),
            ServerError::Malformed(_) => (Class::Internal.word(), Fact::Word("-")),
            ServerError::Transport(_) => (Class::Network.word(), Fact::Word("-")),
        };
        let facts = [
            ("what", Fact::Word("beat")),
            ("class", Fact::Word(class)),
            ("status", status),
            ("wait_s", Fact::N(wait.as_secs())),
        ];
        line(&self.entry_dir, "upload", &facts);
    }
}
