//! The uploader of one entry: where the server is, every complete line sent chunk by chunk,
//! and each acknowledgement checked and kept.

use super::local::{last_ack, sent_path};
use super::{
    Chunk, EndedGate, Policy, Position, WrapUp, build_chunk, io_error, local_link_before,
    waiting_bytes,
};
use crate::CliError;
use crate::run::Vitals;
use crate::upload::UploadRecord;
use crate::upload::api::{ChunkAnswer, RunPosition};
use crate::upload::http::{ServerClient, ServerError};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::export::{ACKS_FILE, ChainPart, append_ack_to};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// What one attempt to send everything waiting came to.
#[derive(Debug)]
pub(super) enum Attempt {
    /// Nothing complete is left to send.
    CaughtUp,
    /// Ask again after at least this long (the server's `Retry-After`, if it sent one).
    Retry(Option<Duration>, String),
    /// Stop uploading this run, and say why.
    Stop(String),
    /// The finish's wrap-up ran out before a request could begin.
    RanOut,
}

/// Why an acknowledgement was not kept.
enum AckError {
    /// It does not match the chunk, or is not signed by this run's key: the server's fault, and
    /// the end of the upload.
    Rejected(String),
    /// `acks.jsonl` could not be written: this machine's trouble, asked again later.
    Local(String),
}

/// The sentence for an acknowledgement that could not be written to `acks.jsonl` yet.
fn ack_not_kept(why: &str) -> String {
    format!("the server's acknowledgement could not be written to acks.jsonl ({why})")
}

/// The least of the wrap-up that is worth a request.
const LEAST_CALL: Duration = Duration::from_millis(100);

/// What is left of the wrap-up `until`, or `cap` with none; `None` once it has run out.
pub(super) fn left(until: Option<WrapUp>, cap: Duration) -> Option<Duration> {
    match until {
        None => Some(cap),
        Some(until) => {
            let left = until.remaining();
            (left >= LEAST_CALL).then(|| left.min(cap))
        }
    }
}

/// The sentence for a wrap-up that ran out, with the last answer that was not an end of the
/// upload when there was one: what the participant needs to know is why it never got through.
pub(super) fn ran_out(last: Option<&str>) -> String {
    match last {
        Some(last) => format!("{RAN_OUT}; the last answer was: {last}"),
        None => RAN_OUT.to_string(),
    }
}

/// What a background upload says when it stops between chunks because the run must stop.
pub(super) const STOPPING: &str = "the run is stopping";

/// What a finish says when the wrap-up ran out before an attempt could begin.
const RAN_OUT: &str = "the time left to finish ran out";

/// Whether waiting `wait` would run past the wrap-up `until` (never, with none).
pub(super) fn past(until: Option<WrapUp>, wait: Duration) -> bool {
    until.is_some_and(|until| wait > until.remaining())
}

/// The uploader of one entry. Its caller holds the entry's lock ([`lock_entry`]).
pub struct Uploader {
    pub(super) server: ServerClient,
    pub(super) record: UploadRecord,
    /// The key the start receipt was signed with; every acknowledgement must verify under it.
    pub(super) key: String,
    pub(super) policy: Policy,
    pub(super) pos: Position,
    /// The last acknowledgement in the trace's `acks.jsonl`: its `seq` and `acked_at`.
    pub(super) acked: Option<(u64, u64)>,
    pub(super) spool: PathBuf,
    /// Beside a recording run only: set once the run must stop, after which this background
    /// upload sends nothing more (the finish takes over).
    pub(super) halt: Option<std::sync::Arc<AtomicBool>>,
    /// The unix second of the last time this upload worked: a chunk or the seal acknowledged,
    /// or nothing left to send. The run's disk watch reads it.
    pub(super) last_ok: std::sync::Arc<AtomicU64>,
    /// Bytes of the chain the server holds, as of the last acknowledgement.
    pub(super) acked_offset: std::sync::Arc<AtomicU64>,
    /// Beside a recording run only: what the run tells its heartbeats.
    pub(super) vitals: Option<Vitals>,
    /// A segment closed from its tail (no seal): [`Uploader::finish`] still commits its end in
    /// a final heartbeat, and sends the chain up to it, but has no seal to send.
    pub(super) close_unsealed: bool,
    /// An acknowledgement the server gave and this machine could not write to `acks.jsonl`
    /// yet, with the `seq` and head of the chunk it answers: written before anything else is
    /// sent, so the chunk it belongs to is not sent again for it.
    pub(super) owed_ack: Option<(AckCheckpoint, u64, String)>,
    /// Whether the run's final heartbeat has been sent (see [`EndedGate`]).
    pub(super) ended: EndedGate,
    /// A run's own upload (beside `goalcup start` or `goalcup continue`): nothing ends it, every
    /// failure is asked again (D64). Unset for `goalcup upload`, which gives up on an answer
    /// asking again does not cure ([`ServerError::ends_manual_upload`]) and says why.
    pub(super) persist: bool,
    /// The server's position is read again before the next chunk is built: a run's upload met a
    /// disagreement with the server's copy, which a fixed server may have cured.
    pub(super) reread_first: bool,
}

/// `GET /v1/runs/{run}?entry_id={entry}`: where the server is for the entry this record names,
/// which is not the run's latest once the run has had a segment after it.
fn position_path(record: &UploadRecord) -> String {
    format!("/v1/runs/{}?entry_id={}", record.run_id, record.entry_id)
}

impl Uploader {
    /// An uploader for `record`, at the position the server reports. A newer acknowledgement
    /// the server holds than the trace has (a reply lost in transit) is kept, once it is shown
    /// to name a record of the local chain.
    pub fn resume(
        server: ServerClient,
        record: UploadRecord,
        receipt_key: String,
        policy: Policy,
        spool_dir: &Path,
    ) -> Result<Uploader, ServerError> {
        let position: RunPosition = server.json("GET", &position_path(&record), None::<&()>)?;
        let spool = spool_dir.join(format!("{}.chunk.zst", record.entry_id));
        // A spool a killed upload left behind is never sent: chunks are rebuilt from the chain.
        let _ = std::fs::remove_file(&spool);
        let mut uploader = Uploader {
            server,
            acked: last_ack(&Path::new(&record.trace_dir).join(ACKS_FILE)),
            record,
            key: receipt_key,
            policy,
            pos: Position::from(&position),
            spool,
            halt: None,
            last_ok: std::sync::Arc::new(AtomicU64::new(crate::run::now_unix())),
            acked_offset: std::sync::Arc::new(AtomicU64::new(0)),
            vitals: None,
            close_unsealed: false,
            owed_ack: None,
            ended: EndedGate::default(),
            persist: false,
            reread_first: false,
        };
        uploader.adopt_server_ack(&position);
        uploader.adopt_verified_offset(&position);
        Ok(uploader)
    }

    /// The entry's chain parts as they are on disk now.
    fn parts(&self) -> Result<Vec<ChainPart>, CliError> {
        let dir = Path::new(&self.record.trace_dir);
        crate::upload::prune::parts_now(dir).map_err(io_error(dir))
    }

    /// This upload worked just now.
    pub(super) fn mark_ok(&self) {
        self.last_ok.store(crate::run::now_unix(), Ordering::SeqCst);
    }

    /// Note that everything of this entry that will ever be sent has been: a later
    /// `goalcup continue` does not close and send it again.
    pub(super) fn note_sent(&self) {
        if let Some(dir) = self.spool.parent() {
            let _ =
                crate::config::write_secret_file(&sent_path(dir, &self.record.entry_id), b"sent\n");
        }
    }

    /// Hand the run's two shared counters to this uploader, replacing its own.
    pub(super) fn share(
        &mut self,
        last_ok: &std::sync::Arc<AtomicU64>,
        acked: &std::sync::Arc<AtomicU64>,
    ) {
        let verified = self.acknowledged();
        self.last_ok = last_ok.clone();
        self.acked_offset = acked.clone();
        self.acked_offset.store(verified, Ordering::SeqCst);
    }

    /// Bytes of the chain this uploader has verified the server holds: what the disk watch may
    /// delete behind. It is raised only to an offset a signed chunk acknowledgement proved, or
    /// to a server position the local chain confirms ([`Self::adopt_verified_offset`]); never
    /// to a number the server merely reports.
    pub fn acknowledged(&self) -> u64 {
        self.acked_offset.load(Ordering::SeqCst)
    }

    /// Take the server's reported position as the acknowledged offset only if it is shown
    /// true: offset 0, or the local record ending exactly at `offset` has the `seq` and hash
    /// the server names. Anything else leaves the offset as it was (lowering is always safe;
    /// raising to an unverified value would let the watch delete bytes the server never held).
    fn adopt_verified_offset(&self, position: &RunPosition) {
        let verified = position.offset == 0
            || self
                .parts()
                .ok()
                .and_then(|parts| local_link_before(&parts, position.offset))
                .is_some_and(|(seq, hash)| {
                    position.next_seq.checked_sub(1) == Some(seq) && hash == position.head
                });
        if verified {
            self.acked_offset.store(position.offset, Ordering::SeqCst);
        }
    }

    /// Bytes of chain waiting to be sent (an upper bound).
    pub fn waiting(&self) -> u64 {
        self.parts()
            .map_or(0, |parts| waiting_bytes(&parts, self.pos.offset))
    }

    /// Keep the server's last acknowledgement if the local chain's record ending at the
    /// server's offset is the one it names. An acknowledgement of anything else would make
    /// the trace fail verification, and dropping one is always safe.
    fn adopt_server_ack(&mut self, position: &RunPosition) {
        let Some(ack) = &position.last_ack else {
            return;
        };
        let local = self
            .parts()
            .ok()
            .and_then(|parts| local_link_before(&parts, position.offset));
        if local
            .as_ref()
            .map(|(seq, hash)| (Some(*seq), hash.as_str()))
            == Some((position.next_seq.checked_sub(1), position.head.as_str()))
        {
            let seq = position.next_seq.checked_sub(1);
            // A rejected one is dropped (dropping is always safe); one that could not be
            // written is held, like any other, and written before anything is sent.
            if let (Err(AckError::Local(_)), Some(seq)) =
                (self.keep_ack(ack, seq, &position.head), seq)
            {
                self.owed_ack = Some((ack.clone(), seq, position.head.clone()));
            }
        }
    }

    /// Send every complete line, chunk by chunk, once, within the wrap-up `until` if one is
    /// given. A run's own upload never stops (D64): what would end `goalcup upload` is asked
    /// again, and the server's position is read again before the next try.
    pub(super) fn attempt(&mut self, until: Option<WrapUp>) -> Attempt {
        match self.send_waiting(until) {
            Attempt::Stop(why) if self.persist => {
                self.reread_first = true;
                Attempt::Retry(None, why)
            }
            other => other,
        }
    }

    /// [`Self::attempt`], as `goalcup upload` has it: [`Attempt::Stop`] on an answer asking
    /// again does not cure.
    fn send_waiting(&mut self, until: Option<WrapUp>) -> Attempt {
        loop {
            // A background upload stops between chunks once the run must stop.
            if until.is_none() && self.halt.as_ref().is_some_and(|h| h.load(Ordering::SeqCst)) {
                return Attempt::Retry(None, STOPPING.to_string());
            }
            // Only a finish has a wrap-up; once it has run out, the rest is `goalcup upload`'s.
            let Some(timeout) = left(until, self.policy.put_timeout) else {
                return Attempt::RanOut;
            };
            // An acknowledgement the last chunk left unwritten is written first.
            if let Some(retry) = self.write_owed_ack() {
                return retry;
            }
            // After a disagreement, where the server is now: a fixed server may say otherwise.
            if self.reread_first {
                match self.reread_position(false) {
                    Ok(()) => {
                        self.note_answered();
                        self.reread_first = false;
                    }
                    Err(error) => return self.failed(error),
                }
            }
            let built = self
                .parts()
                .and_then(|parts| build_chunk(&parts, &self.pos, &self.spool));
            let chunk = match built {
                Ok(Some(chunk)) => chunk,
                Ok(None) => return Attempt::CaughtUp,
                Err(error) => {
                    let _ = std::fs::remove_file(&self.spool);
                    return match error {
                        // The local chain and the server's copy disagree: asking again
                        // changes neither, so this is the one build error that ends the upload.
                        CliError::Refused(_) => Attempt::Stop(error.to_string()),
                        // Anything else is this machine's trouble of the moment (the disk
                        // full, a part briefly missing): the server has said nothing against
                        // the run, so the chunk is built again after a backoff.
                        local => Attempt::Retry(
                            None,
                            format!("the next chunk could not be built ({local})"),
                        ),
                    };
                }
            };
            let sent = self.put(&chunk, timeout);
            let _ = std::fs::remove_file(&chunk.spool);
            match sent {
                Ok(answer) => {
                    self.note_answered();
                    // Sizes only, for measuring the upload later: no content, no key.
                    crate::host_log::log_only(&format!(
                        "upload: chunk at byte {} stored: {} bytes of chain, {} bytes compressed",
                        chunk.offset, chunk.length, chunk.compressed
                    ));
                    let kept = self.keep_ack(&answer.ack, Some(chunk.last_seq), &chunk.head);
                    let owed = match kept {
                        Ok(()) => None,
                        Err(AckError::Rejected(why)) => return Attempt::Stop(why),
                        // The server has the chunk: it is not sent again for the file's
                        // sake. The signed acknowledgement waits, and is written before
                        // anything else is.
                        Err(AckError::Local(why)) => {
                            self.owed_ack = Some((answer.ack, chunk.last_seq, chunk.head.clone()));
                            Some(why)
                        }
                    };
                    self.pos = Position {
                        offset: chunk.offset + chunk.length,
                        next_seq: chunk.last_seq + 1,
                        head: chunk.head.clone(),
                    };
                    self.acked_offset.store(self.pos.offset, Ordering::SeqCst);
                    self.mark_ok();
                    if let Some(why) = owed {
                        return Attempt::Retry(None, ack_not_kept(&why));
                    }
                }
                // The server is somewhere else: read where, and go on from there.
                Err(error) if error.is_conflict() => match self.reread_position(true) {
                    Ok(()) => continue,
                    Err(error) => return self.failed(error),
                },
                Err(error) => return self.failed(error),
            }
        }
    }

    fn failed(&self, error: ServerError) -> Attempt {
        self.note_refused(&error);
        // Past the deadline the server takes nothing before the final heartbeat, which only a
        // sealed run sends. Beside a recording run the stop is on its way, so this waits; for
        // `goalcup upload` of a run that was never sealed it is the end of the road.
        if self.halt.is_none()
            && !self.persist
            && matches!(&error, ServerError::Status { code, .. } if code == "awaiting_final")
        {
            return Attempt::Stop(format!(
                "the run is past its deadline and is not sealed on this machine, so its end cannot \
                 be committed: run `goalcup seal {}` first, then upload again",
                self.record.entry_id
            ));
        }
        if self.gives_up(&error) {
            Attempt::Stop(error.to_string())
        } else {
            Attempt::Retry(
                error.retry_after().map(Duration::from_secs),
                error.to_string(),
            )
        }
    }

    /// Whether this upload ends on `error`: only `goalcup upload` ever does (D64).
    pub(super) fn gives_up(&self, error: &ServerError) -> bool {
        !self.persist && error.ends_manual_upload()
    }

    /// The server answered one of this upload's calls normally: for the run's status, its run
    /// of refusals, if any, is over.
    pub(super) fn note_answered(&self) {
        if let Some(vitals) = &self.vitals {
            vitals.server_answered(crate::run::Channel::Upload);
        }
    }

    /// The server failed one of this upload's calls: a refusal (`4xx`) is counted for the run's
    /// status, and said once when it has repeated ([`crate::run::REFUSALS_SAID_AFTER`]).
    pub(super) fn note_refused(&self, error: &ServerError) {
        note_refusal(self.vitals.as_ref(), error);
    }

    /// Read the server's position again. After a `409` (`must_move`) a position that has not
    /// moved is a conflict asking again cannot cure; after a disagreement it is taken as it is.
    /// A position behind ours is taken as given (the chunk builder re-checks every link); one
    /// past what is on disk waits.
    fn reread_position(&mut self, must_move: bool) -> Result<(), ServerError> {
        let position: RunPosition =
            self.server
                .json("GET", &position_path(&self.record), None::<&()>)?;
        self.adopt_server_ack(&position);
        let moved = Position::from(&position);
        if must_move && moved == self.pos {
            // A conflict that is not about where the server is cannot be cured by asking.
            return Err(ServerError::Status {
                status: 409,
                code: "conflict".into(),
                message: "the server refused a chunk at its own position".into(),
                retry_after: None,
            });
        }
        self.pos = moved;
        self.adopt_verified_offset(&position);
        Ok(())
    }

    fn put(&self, chunk: &Chunk, timeout: Duration) -> Result<ChunkAnswer, ServerError> {
        let body = std::fs::File::open(&chunk.spool)
            .map_err(|e| ServerError::Transport(format!("reading the chunk spool: {e}")))?;
        let headers = [
            ("Content-Type", "application/zstd".to_string()),
            ("Goalcup-Entry-Id", self.record.entry_id.clone()),
            ("Goalcup-Length", chunk.length.to_string()),
            ("Goalcup-Sha256", chunk.sha256.clone()),
            ("Goalcup-First-Seq", chunk.first_seq.to_string()),
            ("Goalcup-Last-Seq", chunk.last_seq.to_string()),
            ("Goalcup-Prev", chunk.prev.clone()),
            ("Goalcup-Head", chunk.head.clone()),
        ];
        self.server.put_stream(
            &format!("/v1/runs/{}/chunks/{}", self.record.run_id, chunk.offset),
            &headers,
            chunk.compressed,
            body,
            timeout,
        )
    }

    /// Check `ack` against the chunk it answers (`seq`, `head`) and append it to the trace's
    /// `acks.jsonl`, where the verifier requires every acknowledgement to name a record of the
    /// chain, verify, carry a valid time, and come after the one before it in both `seq` and
    /// time. One that would break the order is not kept: dropping an acknowledgement only
    /// weakens what the trace can show, and keeping it would make the trace invalid.
    fn keep_ack(
        &mut self,
        ack: &AckCheckpoint,
        seq: Option<u64>,
        head: &str,
    ) -> Result<(), AckError> {
        let at = goalcup_protocol::provenance::Rfc3339Utc::try_from(ack.acked_at.as_str())
            .ok()
            .and(ack.acked_at_secs());
        let fits = ack.entry_id == self.record.entry_id && Some(ack.seq) == seq && ack.hash == head;
        let (Some(at), true, Ok(())) = (at, fits, ack.verify(&self.key)) else {
            return Err(AckError::Rejected(format!(
                "the server's acknowledgement of seq {} does not match the chunk it answers, is \
                 not signed by the key of this run's start receipt, or has no valid time",
                ack.seq
            )));
        };
        if self
            .acked
            .is_some_and(|(last_seq, last_at)| ack.seq <= last_seq || at < last_at)
        {
            return Ok(());
        }
        append_ack_to(Path::new(&self.record.trace_dir), ack)
            .map_err(|e| AckError::Local(e.to_string()))?;
        self.acked = Some((ack.seq, at));
        Ok(())
    }

    /// Write the acknowledgement [`Self::owed_ack`] holds, if any. `Some` is the attempt that
    /// must end here: it could not be written yet, and is asked again after a backoff.
    fn write_owed_ack(&mut self) -> Option<Attempt> {
        let (ack, seq, head) = self.owed_ack.take()?;
        match self.keep_ack(&ack, Some(seq), &head) {
            Ok(()) => None,
            Err(AckError::Local(why)) => {
                self.owed_ack = Some((ack, seq, head));
                Some(Attempt::Retry(None, ack_not_kept(&why)))
            }
            // It verified when it came, and nothing it is checked against has changed.
            Err(AckError::Rejected(why)) => Some(Attempt::Stop(why)),
        }
    }
}

/// A failed call of a run's upload: a refusal (`4xx`) is counted in `vitals` for the run's
/// status, and said once when it has repeated ([`crate::run::REFUSALS_SAID_AFTER`]). Nothing
/// without `vitals` (`goalcup upload`, an earlier segment's drain).
pub(super) fn note_refusal(vitals: Option<&Vitals>, error: &ServerError) {
    if let (Some(vitals), Some(status)) = (vitals, error.refusal_status())
        && vitals.server_refused(crate::run::Channel::Upload, status)
    {
        crate::host_log::say(&format!(
            "upload: {}",
            crate::participant::problem_server_refuses(status)
        ));
    }
}
