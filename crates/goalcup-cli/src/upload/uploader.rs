//! The uploader: the entry's chain to the server in chunks, each acknowledged.
//!
//! The chain is one or more part files (`chain.jsonl`, then `chain.<start>.jsonl`; D56), and
//! every offset here is logical: the byte in the whole chain, wherever its part starts. A
//! chunk never crosses the end of a part. Parts the server acknowledged long ago may be
//! deleted ([`super::prune`]); the uploader only ever reads from the server's offset on.
//!
//! It reads the chain the daemon writes and never touches it: recording never waits on the
//! network, and a run whose server is away for a day loses nothing, because every byte is
//! still on disk and is sent when the server is back. What it sends and when
//! (docs/UPLOAD-API.md, docs/SERVER-DESIGN.md section 3, step 4):
//!
//! - **What:** whole lines from where the server is (`offset`, `next_seq`, `head`), each
//!   chunk one zstd frame of at most [`MAX_CHUNK_BYTES`] compressed, built in a spool file
//!   beside the trace and streamed from it, so a chunk is never held in memory.
//! - **When:** at once when the run launches, then every [`Policy::interval`] (thirty
//!   minutes; the first interval after the opening chunk is a random share of it, so that
//!   runs started together do not send together), or sooner once [`Policy::eager_bytes`] of
//!   chain is waiting, and everything at the end.
//! - **Failures:** one failed attempt never ends an upload. No answer, a server error, a rate
//!   limit, the maintenance switch, a rejected chunk (`400`, `413`, `422`) or any other answer
//!   is asked again after a backoff that starts at a minute and doubles to fifteen, with
//!   jitter, never sooner than the server's `Retry-After`. A `409` means the server is
//!   somewhere else: its position is read again and the upload continues from there. Only
//!   `401`, `404` and `410` ([`ServerError::ends_upload`]: the token, the run, or the run's
//!   being closed) stop the uploader and say why, and so do a local chain that does not
//!   continue the server's copy and an acknowledgement that does not verify; the run goes on
//!   recording. A local error (the disk full, a part briefly missing, `acks.jsonl` not
//!   writable) is this machine's trouble of the moment, not the server's verdict: it is said
//!   and asked again after the same backoff. After the stop the same asking goes on for the wrap-up ([`Policy::wrap_up`],
//!   an hour, counted through machine sleep): the final heartbeat, the rest of the chain and
//!   the seal. What is still unsent then is `goalcup upload`'s.
//! - **Acknowledgements:** each one is checked (this entry, the chunk's last `seq` and head,
//!   a signature under the start receipt's key) and appended to the trace's `acks.jsonl`,
//!   where `goalcup-verify` reads it.

use super::UploadRecord;
use super::api::{
    ChunkAnswer, HeartbeatAnswer, HeartbeatRequest, RunPosition, SealRequest, StatusAnswer,
};
use super::http::{ServerClient, ServerError};
use super::policy::Cadence;
pub use super::policy::{Policy, WrapUp, backoff};
use crate::CliError;
use crate::run::Vitals;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::export::{ACKS_FILE, BASELINE_FILE, ChainPart, SEAL_FILE};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The most a chunk body may be, compressed: the server's cap.
pub const MAX_CHUNK_BYTES: u64 = 32 * 1024 * 1024;
/// A chunk stops taking lines once its spool reaches this, leaving room for the frame's end
/// and the encoder's buffered block.
const CHUNK_SOFT_BYTES: u64 = MAX_CHUNK_BYTES - 2 * 1024 * 1024;
/// The zstd level: the library's default, fast and about 10x on a measured chain. No long
/// window (docs/SERVER-DESIGN.md section 17).
const ZSTD_LEVEL: i32 = 3;
/// The longest line read: one chain record is at most this (`MAX_ENVELOPE_BYTES`).
const MAX_LINE_BYTES: u64 = goalcup_protocol::capture::MAX_ENVELOPE_BYTES as u64;
/// Where the server is: the bytes it holds, the `seq` the next chunk starts at, and the hash
/// the next chunk's first record must extend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub offset: u64,
    pub next_seq: u64,
    pub head: String,
}

impl From<&RunPosition> for Position {
    fn from(p: &RunPosition) -> Self {
        Position {
            offset: p.offset,
            next_seq: p.next_seq,
            head: p.head.clone(),
        }
    }
}

/// One chunk, built and waiting in its spool file.
#[derive(Debug)]
pub struct Chunk {
    pub offset: u64,
    /// Uncompressed bytes of the chain it holds.
    pub length: u64,
    pub first_seq: u64,
    pub last_seq: u64,
    pub prev: String,
    pub head: String,
    /// SHA-256 of the compressed body.
    pub sha256: String,
    pub compressed: u64,
    pub spool: PathBuf,
}

/// Only the link of a chain record, whose fields an envelope carries at its top level
/// (`Envelope::link` is flattened); the rest of the line is not read here.
#[derive(serde::Deserialize)]
struct LinkOnly {
    #[serde(flatten)]
    link: goalcup_protocol::chain::Link,
}

/// A writer that counts and hashes what passes through it.
struct Tally<W> {
    inner: W,
    bytes: u64,
    sha: Sha256,
}

impl<W: Write> Write for Tally<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.bytes += n as u64;
        self.sha.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> CliError + '_ {
    move |source| CliError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// How many bytes of `parts` lie past the logical `offset` (a torn last line is not counted,
/// but neither is it looked for: this is the length of the chain, a cheap upper bound).
pub fn waiting_bytes(parts: &[ChainPart], offset: u64) -> u64 {
    parts
        .last()
        .map_or(0, |last| last.start.saturating_add(last.len))
        .saturating_sub(offset)
}

/// The next chunk of the chain `parts` from `pos`, compressed into `spool`, or `None` when no
/// complete line waits. The chunk is cut from the one part that holds `pos.offset` and stops
/// at the end of it; the next chunk starts in the next part. Every line's link is checked to
/// continue from `pos`, so a chunk can only ever extend what the server holds: a local chain
/// that does not is refused, not uploaded.
///
/// A server position below the first surviving part is refused too: those bytes were deleted
/// here, which only happens to bytes the server acknowledged, so the two copies disagree and
/// nothing is sent across the gap.
pub fn build_chunk(
    parts: &[ChainPart],
    pos: &Position,
    spool: &Path,
) -> Result<Option<Chunk>, CliError> {
    let Some(first) = parts.first() else {
        return Err(CliError::Internal(
            "the entry has no chain part to upload from".to_string(),
        ));
    };
    if pos.offset < first.start {
        return Err(CliError::Refused(format!(
            "the server holds {} bytes of this entry's chain, but the first {} bytes were \
             already deleted from this machine, so the bytes in between cannot be sent. Nothing \
             more is uploaded for this run; ask the organisers.",
            pos.offset, first.start
        )));
    }
    // The part that holds the next byte; none when everything on disk is already sent.
    let Some(part) = parts
        .iter()
        .find(|part| pos.offset < part.start.saturating_add(part.len))
    else {
        let _ = std::fs::remove_file(spool);
        return Ok(None);
    };
    let chain = part.path.as_path();
    let mut file = std::fs::File::open(chain).map_err(io_error(chain))?;
    file.seek(SeekFrom::Start(pos.offset - part.start))
        .map_err(io_error(chain))?;
    let mut reader = BufReader::new(file);
    let out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(spool)
        .map_err(io_error(spool))?;
    let tally = Tally {
        inner: out,
        bytes: 0,
        sha: Sha256::new(),
    };
    let mut encoder = zstd::stream::Encoder::new(tally, ZSTD_LEVEL).map_err(io_error(spool))?;
    let (mut length, mut seq, mut head) = (0u64, pos.next_seq, pos.head.clone());
    let mut lines = 0u64;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut reader)
            .take(MAX_LINE_BYTES + 1)
            .read_until(b'\n', &mut line)
            .map_err(io_error(chain))?;
        if read == 0 || line.last() != Some(&b'\n') {
            if read as u64 > MAX_LINE_BYTES {
                return Err(CliError::Internal(format!(
                    "{} holds a line longer than any chain record at byte {}",
                    chain.display(),
                    pos.offset + length
                )));
            }
            // The end, or a line the daemon is still writing: it goes in a later chunk.
            break;
        }
        let link = serde_json::from_slice::<LinkOnly>(&line[..line.len() - 1])
            .map_err(|e| {
                CliError::Internal(format!(
                    "{} has a record that is not a chain record at byte {}: {e}",
                    chain.display(),
                    pos.offset + length
                ))
            })?
            .link;
        if link.seq != seq || link.prev != head {
            return Err(CliError::Refused(format!(
                "{} does not continue where the server's copy ends (seq {seq}): its record at \
                 byte {} is seq {}. The local chain and the server's copy differ; nothing more \
                 is uploaded for this run.",
                chain.display(),
                pos.offset + length,
                link.seq
            )));
        }
        encoder.write_all(&line).map_err(io_error(spool))?;
        length += line.len() as u64;
        lines += 1;
        seq += 1;
        head = link.hash;
        if encoder.get_ref().bytes >= CHUNK_SOFT_BYTES {
            break;
        }
    }
    let tally = encoder.finish().map_err(io_error(spool))?;
    tally.inner.sync_all().map_err(io_error(spool))?;
    if lines == 0 {
        let _ = std::fs::remove_file(spool);
        // A closed part ends at a whole record (it is closed at a durable frontier), so bytes
        // before its end that hold no whole line mean the part is damaged: waiting would
        // stall the upload for good, silently.
        if parts.last().is_some_and(|last| last.start != part.start) {
            return Err(CliError::Internal(format!(
                "{} is closed but holds no whole record at byte {}, so the upload cannot go on \
                 past it",
                chain.display(),
                pos.offset
            )));
        }
        return Ok(None);
    }
    if tally.bytes > MAX_CHUNK_BYTES {
        let _ = std::fs::remove_file(spool);
        return Err(CliError::Internal(format!(
            "a chunk compressed to {} bytes, over the {MAX_CHUNK_BYTES}-byte cap",
            tally.bytes
        )));
    }
    Ok(Some(Chunk {
        offset: pos.offset,
        length,
        first_seq: pos.next_seq,
        last_seq: seq - 1,
        prev: pos.head.clone(),
        head,
        sha256: hex::encode(tally.sha.finalize()),
        compressed: tally.bytes,
        spool: spool.to_path_buf(),
    }))
}

/// What one attempt to send everything waiting came to.
#[derive(Debug)]
enum Attempt {
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
fn left(until: Option<WrapUp>, cap: Duration) -> Option<Duration> {
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
fn ran_out(last: Option<&str>) -> String {
    match last {
        Some(last) => format!("{RAN_OUT}; the last answer was: {last}"),
        None => RAN_OUT.to_string(),
    }
}

/// What a background upload says when it stops between chunks because the run must stop.
const STOPPING: &str = "the run is stopping";

/// What a finish says when the wrap-up ran out before an attempt could begin.
const RAN_OUT: &str = "the time left to finish ran out";

/// Whether waiting `wait` would run past the wrap-up `until` (never, with none).
fn past(until: Option<WrapUp>, wait: Duration) -> bool {
    until.is_some_and(|until| wait > until.remaining())
}

/// `<cfg>/uploads/<entry>.lock`: held by whichever process uploads the entry.
pub fn lock_path(spool_dir: &Path, entry_id: &str) -> PathBuf {
    spool_dir.join(format!("{entry_id}.lock"))
}

/// Take the entry's upload lock, held until the returned file is dropped. Only one process
/// uploads an entry at a time: two would truncate each other's spool and race on
/// `acks.jsonl`.
pub fn lock_entry(spool_dir: &Path, entry_id: &str) -> Result<std::fs::File, CliError> {
    crate::config::create_private_dir(spool_dir)?;
    let path = lock_path(spool_dir, entry_id);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(io_error(&path))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(CliError::Refused(format!(
                "entry {entry_id} is being uploaded by another goalcup: its `goalcup start` is \
                 still running, or another `goalcup upload` is. Run this again once it ends."
            )));
        }
        Err(std::fs::TryLockError::Error(source)) => return Err(io_error(&path)(source)),
    }
    Ok(file)
}

/// The uploader of one entry. Its caller holds the entry's lock ([`lock_entry`]).
pub struct Uploader {
    server: ServerClient,
    record: UploadRecord,
    /// The key the start receipt was signed with; every acknowledgement must verify under it.
    key: String,
    policy: Policy,
    pos: Position,
    /// The last acknowledgement in the trace's `acks.jsonl`: its `seq` and `acked_at`.
    acked: Option<(u64, u64)>,
    spool: PathBuf,
    /// Beside a recording run only: set once the run must stop, after which this background
    /// upload sends nothing more (the finish takes over).
    halt: Option<std::sync::Arc<AtomicBool>>,
    /// The unix second of the last time this upload worked: a chunk or the seal acknowledged,
    /// or nothing left to send. The run's disk watch reads it.
    last_ok: std::sync::Arc<AtomicU64>,
    /// Bytes of the chain the server holds, as of the last acknowledgement.
    acked_offset: std::sync::Arc<AtomicU64>,
    /// Beside a recording run only: what the run tells its heartbeats.
    vitals: Option<Vitals>,
    /// A segment closed from its tail (no seal): [`Uploader::finish`] still commits its end in
    /// a final heartbeat, and sends the chain up to it, but has no seal to send.
    close_unsealed: bool,
    /// An acknowledgement the server gave and this machine could not write to `acks.jsonl`
    /// yet, with the `seq` and head of the chunk it answers: written before anything else is
    /// sent, so the chunk it belongs to is not sent again for it.
    owed_ack: Option<(AckCheckpoint, u64, String)>,
    /// Whether the run's final heartbeat has been sent (see [`EndedGate`]).
    ended: EndedGate,
}

/// Set as soon as the first final heartbeat has been sent, whatever came of it (a lost answer
/// or a `5xx` is asked again after a backoff of minutes, and the run's ordinary heartbeats must
/// not go in between). The run's heartbeat thread sends an ordinary heartbeat only while it is
/// unset, holding the lock for the whole call; the final heartbeat is sent under the same lock,
/// so an ordinary one is never in flight behind it or after it.
pub type EndedGate = std::sync::Arc<Mutex<bool>>;

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
        };
        uploader.adopt_server_ack(&position);
        uploader.adopt_verified_offset(&position);
        Ok(uploader)
    }

    /// The entry's chain parts as they are on disk now.
    fn parts(&self) -> Result<Vec<ChainPart>, CliError> {
        let dir = Path::new(&self.record.trace_dir);
        super::prune::parts_now(dir).map_err(io_error(dir))
    }

    /// This upload worked just now.
    fn mark_ok(&self) {
        self.last_ok.store(crate::run::now_unix(), Ordering::SeqCst);
    }

    /// Note that everything of this entry that will ever be sent has been: a later
    /// `goalcup continue` does not close and send it again.
    fn note_sent(&self) {
        if let Some(dir) = self.spool.parent() {
            let _ =
                crate::config::write_secret_file(&sent_path(dir, &self.record.entry_id), b"sent\n");
        }
    }

    /// Hand the run's two shared counters to this uploader, replacing its own.
    fn share(&mut self, last_ok: &std::sync::Arc<AtomicU64>, acked: &std::sync::Arc<AtomicU64>) {
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
    /// given.
    fn attempt(&mut self, until: Option<WrapUp>) -> Attempt {
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
                Err(error) if error.is_conflict() => match self.reread() {
                    Ok(()) => continue,
                    Err(error) => return self.failed(error),
                },
                Err(error) => return self.failed(error),
            }
        }
    }

    fn failed(&self, error: ServerError) -> Attempt {
        // Past the deadline the server takes nothing before the final heartbeat, which only a
        // sealed run sends. Beside a recording run the stop is on its way, so this waits; for
        // `goalcup upload` of a run that was never sealed it is the end of the road.
        if self.halt.is_none()
            && matches!(&error, ServerError::Status { code, .. } if code == "awaiting_final")
        {
            return Attempt::Stop(format!(
                "the run is past its deadline and is not sealed on this machine, so its end cannot \
                 be committed: run `goalcup seal {}` first, then upload again",
                self.record.entry_id
            ));
        }
        if error.ends_upload() {
            Attempt::Stop(error.to_string())
        } else {
            Attempt::Retry(
                error.retry_after().map(Duration::from_secs),
                error.to_string(),
            )
        }
    }

    /// Read the server's position again after a `409`. A position behind ours is taken as
    /// given (the chunk builder re-checks every link); one past what is on disk waits.
    fn reread(&mut self) -> Result<(), ServerError> {
        let position: RunPosition =
            self.server
                .json("GET", &position_path(&self.record), None::<&()>)?;
        self.adopt_server_ack(&position);
        let moved = Position::from(&position);
        if moved == self.pos {
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
        append_ack(Path::new(&self.record.trace_dir), ack)
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

    /// The final heartbeat: commit the sealed chain's last record as the run's end, asking
    /// again after each failure that does not end the upload, within the wrap-up `until`. A
    /// chain that cannot be read just now is a failure of that kind too: it is read again after
    /// the same backoff, not taken for a chain with no whole record.
    fn commit_end(
        &mut self,
        until: Option<WrapUp>,
        say: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        let mut failures = 0;
        let mut last = None::<String>;
        loop {
            let repo_head = noted_repo_head(&self.spool, &self.record.entry_id);
            // Under the lock the run's ordinary heartbeats take, so none is in flight behind
            // this one, and none is sent once this one has gone. The time left is measured
            // after the lock is taken: an ordinary heartbeat in flight holds it for as long as
            // it takes, and must not come out of this call's time.
            let (retry_after, why) = {
                let mut ended = self.ended.lock().unwrap_or_else(PoisonError::into_inner);
                let Some(timeout) = left(until, super::http::CALL_TIMEOUT) else {
                    return Err(ran_out(last.as_deref()));
                };
                let beat = heartbeat(
                    &self.server,
                    &self.record,
                    true,
                    repo_head,
                    self.vitals.as_ref(),
                    timeout,
                );
                // Once it has been sent, whatever the answer, the run's end is the final
                // heartbeat's to commit: no ordinary one follows it.
                if matches!(beat, Beat::Answer(_)) {
                    *ended = true;
                }
                match beat {
                    Beat::Empty => return Err("the chain holds no whole record".to_string()),
                    Beat::Answer(Ok(_)) => return Ok(()),
                    Beat::Answer(Err(error)) if error.ends_upload() => {
                        return Err(error.to_string());
                    }
                    Beat::Answer(Err(error)) => (
                        error.retry_after().map(Duration::from_secs),
                        format!("the final heartbeat was not stored ({error})"),
                    ),
                    Beat::Unreadable(error) => (
                        None,
                        format!("the final heartbeat could not read the chain ({error})"),
                    ),
                }
            };
            failures += 1;
            let wait = backoff(&self.policy, failures, retry_after);
            if past(until, wait) {
                return Err(ran_out(Some(&why)));
            }
            say(&format!(
                "upload: {why}; trying again in {}s",
                wait.as_secs().max(1)
            ));
            last = Some(why);
            std::thread::sleep(wait);
        }
    }

    /// The trace's `seal.json` and `baseline.json`, once both exist.
    fn seal_documents(&self) -> Option<(String, String)> {
        let trace = Path::new(&self.record.trace_dir);
        Some((
            read_document(&trace.join(SEAL_FILE))?,
            read_document(&trace.join(BASELINE_FILE))?,
        ))
    }

    /// `POST /v1/runs/{id}/seal` with the two documents: the run is over.
    /// Answers whether the server says the run itself is over.
    fn seal(&self, documents: &(String, String), timeout: Duration) -> Result<bool, ServerError> {
        let request = SealRequest {
            entry_id: self.record.entry_id.clone(),
            seal_json: documents.0.clone(),
            baseline_json: documents.1.clone(),
        };
        self.server
            .json_timed::<StatusAnswer>(
                "POST",
                &format!("/v1/runs/{}/seal", self.record.run_id),
                Some(&request),
                timeout,
            )
            .map(|answer| answer.run_ended())
    }
}

/// The largest document sent with the seal: the verifier's own bound on `seal.json`.
const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;

/// A non-empty document of at most [`MAX_DOCUMENT_BYTES`], or `None`.
fn read_document(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_DOCUMENT_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    (!text.is_empty() && text.len() as u64 <= MAX_DOCUMENT_BYTES).then_some(text)
}

/// The `seq` and `acked_at` of the last acknowledgement in `acks`, or `None` for none.
fn last_ack(acks: &Path) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(acks).ok()?;
    text.lines()
        .rev()
        .find_map(|line| serde_json::from_str::<AckCheckpoint>(line).ok())
        .and_then(|ack| Some((ack.seq, ack.acked_at_secs()?)))
}

/// The `seq` and `hash` of the chain record whose line ends exactly at the logical byte
/// `offset`, or `None` when no whole record ends there (or the part holding it is gone).
pub fn local_link_before(parts: &[ChainPart], offset: u64) -> Option<(u64, String)> {
    // The part whose last byte is at `offset - 1`; the last part may have grown since it was
    // measured, and a record that ends past what was measured is then just not found.
    let last_start = parts.last()?.start;
    let part = parts.iter().find(|part| {
        part.start < offset
            && (offset <= part.start.saturating_add(part.len) || part.start == last_start)
    })?;
    link_before_in(&part.path, offset - part.start)
}

/// [`local_link_before`] within one part file: the record whose line ends at `offset` in it.
fn link_before_in(file_path: &Path, offset: u64) -> Option<(u64, String)> {
    link_before_io(file_path, offset).ok().flatten()
}

/// [`link_before_in`], telling a file that could not be read (`Err`) from one that holds no
/// record there (`Ok(None)`).
fn link_before_io(file_path: &Path, offset: u64) -> std::io::Result<Option<(u64, String)>> {
    if offset == 0 {
        return Ok(None);
    }
    let start = offset.saturating_sub(MAX_LINE_BYTES + 1);
    let mut file = std::fs::File::open(file_path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (offset - start) as usize];
    file.read_exact(&mut bytes)?;
    let Some(body) = bytes.strip_suffix(b"\n") else {
        return Ok(None);
    };
    let line = match body.iter().rposition(|b| *b == b'\n') {
        Some(at) => &body[at + 1..],
        None if start == 0 => body,
        None => return Ok(None),
    };
    Ok(serde_json::from_slice::<LinkOnly>(line)
        .ok()
        .map(|record| (record.link.seq, record.link.hash)))
}

/// The last whole record of the chain `parts`: the logical bytes up to the end of it, its
/// `seq` and its `hash`. `None` for a chain with no whole record. A last part that holds no
/// whole record yet (just rotated) answers from the part before it.
pub fn local_end(parts: &[ChainPart]) -> Option<(u64, u64, String)> {
    local_end_io(parts).ok().flatten()
}

/// [`local_end`], telling a chain that could not be read (`Err`) from one with no whole record
/// (`Ok(None)`): the first is asked again, the second is just not there yet.
fn local_end_io(parts: &[ChainPart]) -> std::io::Result<Option<(u64, u64, String)>> {
    for part in parts.iter().rev() {
        match end_in_io(&part.path) {
            Ok(Some((end, seq, hash))) => return Ok(Some((part.start + end, seq, hash))),
            Ok(None) => {}
            // A part deleted since it was listed holds no record any more: the one before it
            // answers, as for a part that holds none yet.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

/// The end of the last whole line of one part file, with the record that ends there.
fn end_in_io(file_path: &Path) -> std::io::Result<Option<(u64, u64, String)>> {
    let len = std::fs::metadata(file_path)?.len();
    let start = len.saturating_sub(2 * (MAX_LINE_BYTES + 1));
    let mut file = std::fs::File::open(file_path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.take(len - start).read_to_end(&mut tail)?;
    // The end of the last whole line; a torn tail after it is not a record yet.
    let Some(newline) = tail.iter().rposition(|b| *b == b'\n') else {
        return Ok(None);
    };
    let end = start + newline as u64 + 1;
    Ok(link_before_io(file_path, end)?.map(|(seq, hash)| (end, seq, hash)))
}

/// The 40-hex commit `workspace`'s repository has checked out, read from its `.git` files (no
/// `git` process): `HEAD` naming a commit, or a branch found under `refs/heads/` or in
/// `packed-refs`. `None` when there is no repository or nothing is committed.
pub fn repo_head(workspace: &Path) -> Option<String> {
    let git = workspace.join(".git");
    // The agent controls this directory: only regular files are read (a FIFO would block), and
    // never more than a bound.
    let read = |path: PathBuf| regular_file_text(&path, 64 * 1024);
    let is_commit = |s: &str| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
    let head = read(git.join("HEAD"))?;
    let head = head.trim();
    if is_commit(head) {
        return Some(head.to_ascii_lowercase());
    }
    let reference = head.strip_prefix("ref: ")?.trim();
    // A reference names a path under `.git`; one that tries to leave it is not followed.
    if !reference.starts_with("refs/") || reference.split('/').any(|part| part == "..") {
        return None;
    }
    if let Some(commit) = read(git.join(reference)) {
        let commit = commit.trim();
        return is_commit(commit).then(|| commit.to_ascii_lowercase());
    }
    regular_file_text(&git.join("packed-refs"), 64 * 1024 * 1024)?
        .lines()
        .find_map(|line| {
            let (commit, name) = line.split_once(' ')?;
            (name.trim() == reference && is_commit(commit)).then(|| commit.to_ascii_lowercase())
        })
}

/// The text of `path` if it is a regular file (never a FIFO, device or directory, and opened
/// without blocking), at most `max` bytes, or `None`.
fn regular_file_text(path: &Path, max: u64) -> Option<String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    file.take(max + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= max).then_some(text)
}

/// `<cfg>/uploads/<entry>.sent`: written when the entry's upload is complete (the whole chain
/// and its seal; for a segment closed from its tail, the chain up to its committed end).
fn sent_path(spool_dir: &Path, entry_id: &str) -> PathBuf {
    spool_dir.join(format!("{entry_id}.sent"))
}

/// Whether the entry's upload finished: nothing of it is left to close or send.
pub fn is_sent(spool_dir: &Path, entry_id: &str) -> bool {
    sent_path(spool_dir, entry_id).is_file()
}

/// `<cfg>/uploads/<entry>.repo-head`: the commit noted when the run stopped.
fn repo_head_path(spool_dir: &Path, entry_id: &str) -> PathBuf {
    spool_dir.join(format!("{entry_id}.repo-head"))
}

/// Note the workspace repository's checked-out commit now, the moment the run stops, for the
/// final heartbeat to send whenever it goes (a later `goalcup upload` included). Nothing is
/// noted when there is no repository.
pub fn note_repo_head(spool_dir: &Path, record: &UploadRecord) {
    note_repo_head_saying(spool_dir, record, &mut crate::host_log::say);
}

/// [`note_repo_head`], saying through `say` when the note cannot be written: the final
/// heartbeat then goes without the commit, and the participant is told so, not left to find the
/// commit missing from the entry.
fn note_repo_head_saying(spool_dir: &Path, record: &UploadRecord, say: &mut dyn FnMut(&str)) {
    if record.workspace.is_empty() {
        return;
    }
    if let Some(commit) = repo_head(Path::new(&record.workspace))
        && let Err(error) = crate::config::write_secret_file(
            &repo_head_path(spool_dir, &record.entry_id),
            commit.as_bytes(),
        )
    {
        say(&format!(
            "upload: the workspace's commit could not be noted ({error}); the final heartbeat \
             will go without it"
        ));
    }
}

/// [`note_repo_head`], unless a commit was already noted for the entry. A `goalcup continue`
/// notes the commit of every earlier segment it is about to send before the next segment's
/// agent can change the workspace; a host that noted the commit as its segment ended has said
/// it better than this can.
pub fn note_repo_head_unless_noted(spool_dir: &Path, record: &UploadRecord) {
    if noted_repo_head_in(spool_dir, &record.entry_id).is_none() {
        note_repo_head(spool_dir, record);
    }
}

/// The commit [`note_repo_head`] noted for the entry whose spool is `spool`, if any.
fn noted_repo_head(spool: &Path, entry_id: &str) -> Option<String> {
    noted_repo_head_in(spool.parent()?, entry_id)
}

/// [`noted_repo_head`] given the spool directory itself.
fn noted_repo_head_in(spool_dir: &Path, entry_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(repo_head_path(spool_dir, entry_id)).ok()?;
    let text = text.trim();
    (text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())).then(|| text.to_string())
}

/// What sending a heartbeat came to.
enum Beat {
    /// The server answered, or did not.
    Answer(Result<HeartbeatAnswer, ServerError>),
    /// The chain holds no whole record yet: there is nothing to say.
    Empty,
    /// The chain could not be read just now (the error): not the same as having no record.
    Unreadable(String),
}

/// `POST /v1/runs/{id}/heartbeat` with the local chain's last whole record: `final` once the
/// run has ended, with the workspace repository's commit. `vitals` adds how the recording is
/// and how many model requests were seen; a heartbeat without them (`goalcup upload`) says
/// neither.
fn heartbeat(
    server: &ServerClient,
    record: &UploadRecord,
    is_final: bool,
    repo_head: Option<String>,
    vitals: Option<&Vitals>,
    timeout: Duration,
) -> Beat {
    let parts = match super::prune::parts_now(Path::new(&record.trace_dir)) {
        Ok(parts) => parts,
        Err(error) => return Beat::Unreadable(error.to_string()),
    };
    let (offset_local, seq, head) = match local_end_io(&parts) {
        Ok(Some(end)) => end,
        Ok(None) => return Beat::Empty,
        Err(error) => return Beat::Unreadable(error.to_string()),
    };
    let request = HeartbeatRequest {
        entry_id: record.entry_id.clone(),
        seq,
        head,
        offset_local,
        is_final,
        repo_head,
        recording: vitals.map(|vitals| vitals.recording().wire().to_string()),
        model_requests: vitals.map(Vitals::model_requests),
    };
    Beat::Answer(server.json_timed(
        "POST",
        &format!("/v1/runs/{}/heartbeat", record.run_id),
        Some(&request),
        timeout,
    ))
}

/// Sets its flag when dropped: the watcher thread's signal that the uploader has returned.
struct SetOnDrop(std::sync::Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// How long one ordinary heartbeat may take: well inside the 5-minute cadence, so a server that
/// hangs cannot hold the local deadline check back for long.
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);

/// What the run is told when a newer run on the account has replaced it.
pub const REPLACED_REASON: &str = "a newer run on your account replaced this one";

/// Why the run must stop, from a heartbeat answer that says `stop`: a newer run replaced it,
/// or (also from a server that gives no reason) its deadline came.
pub fn stop_reason(answer: &HeartbeatAnswer) -> Option<String> {
    if !answer.stop {
        return None;
    }
    Some(match answer.reason.as_deref() {
        Some("replaced") => REPLACED_REASON.to_string(),
        _ => format!(
            "the server says the run is over (its deadline was {})",
            answer.deadline_at
        ),
    })
}

/// What the run is told when the server refuses its heartbeat for good (`401`, or `410`): the
/// reason starts with this, which `goalcup start` words its stop by ([`refused_run`]).
pub const HEARTBEAT_REFUSED: &str = "the server refused the run's heartbeat";

/// Whether `reason` (as sent on the run's stop channel) is the server's refusal of the run's
/// heartbeat.
pub fn refused_run(reason: &str) -> bool {
    reason.starts_with(HEARTBEAT_REFUSED)
}

/// The sentence that stops the run when a heartbeat is answered `401` (the token is no longer
/// accepted) or `410` (the run is closed): what every other call ends the upload for, so the run
/// stops here too. Any other failure is asked again at the next beat.
fn heartbeat_refusal(error: &ServerError) -> Option<String> {
    matches!(
        error,
        ServerError::Status {
            status: 401 | 410,
            ..
        }
    )
    .then(|| format!("{HEARTBEAT_REFUSED} ({error}), so this run can no longer be uploaded"))
}

/// One ordinary heartbeat, counted in `vitals` and said when it fails; `Some` is why the run
/// must stop. Nothing is sent once the final heartbeat has been sent (`ended`), and none is in
/// flight while it goes: the lock is held for the call.
fn ordinary_beat(
    server: &ServerClient,
    record: &UploadRecord,
    vitals: &Vitals,
    ended: &EndedGate,
    count: &mut Option<LocalDeadline>,
) -> Option<String> {
    let ended = ended.lock().unwrap_or_else(PoisonError::into_inner);
    if *ended {
        return None;
    }
    let failed = |why: String| {
        let failures = vitals.heartbeat_failed(&why);
        // Said when it starts and when it has lasted; `goalcup status` shows it from then on.
        if failures == 1 || failures == crate::run::HEARTBEATS_FAILING_AFTER {
            crate::host_log::say(&format!(
                "heartbeat: {why} ({failures} in a row); it is sent again at the next beat"
            ));
        }
    };
    // Taken before the call, so the round trip makes the count end earlier, not later. (The
    // server's whole seconds can still leave it up to a second late.)
    let asked = goalcup_protocol::clock::ContinuousOrigin::now();
    match heartbeat(server, record, false, None, Some(vitals), HEARTBEAT_TIMEOUT) {
        Beat::Answer(Ok(answer)) => {
            vitals.heartbeat_ok();
            LocalDeadline::heard(count, &answer, asked);
            stop_reason(&answer)
        }
        Beat::Answer(Err(error)) => {
            let refusal = heartbeat_refusal(&error);
            if refusal.is_none() {
                failed(error.to_string());
            }
            refusal
        }
        Beat::Unreadable(error) => {
            failed(format!("the chain could not be read ({error})"));
            None
        }
        // Nothing recorded yet: nothing to say, and nothing failed.
        Beat::Empty => None,
    }
}

/// Send a heartbeat every `policy.heartbeat` and watch the run's own deadline, until the run
/// must stop (said once on `stopping`, and `over` set) or the uploader is `done`. Each answer
/// that says how much time is left sets the count again ([`LocalDeadline`]).
#[allow(clippy::too_many_arguments)]
fn watch_deadline(
    server: &ServerClient,
    record: &UploadRecord,
    policy: Policy,
    local_deadline: Option<LocalDeadline>,
    vitals: &Vitals,
    stopping: &std::sync::mpsc::Sender<String>,
    over: &AtomicBool,
    done: &AtomicBool,
    ended: &EndedGate,
) {
    let mut local_deadline = local_deadline;
    let mut next_beat = Instant::now();
    let tick = policy.poll.min(Duration::from_secs(1));
    while !done.load(Ordering::SeqCst) {
        // This machine's clock first: it needs no network, so a hanging server cannot hold it.
        let local = local_deadline.filter(LocalDeadline::passed).map(|_| {
            format!(
                "the run's time is up by this machine's clock (its deadline was {})",
                record.deadline_at
            )
        });
        let reason = if local.is_some() {
            local
        } else if Instant::now() >= next_beat {
            next_beat = Instant::now() + policy.heartbeat;
            ordinary_beat(server, record, vitals, ended, &mut local_deadline)
        } else {
            None
        };
        if let Some(reason) = reason {
            over.store(true, Ordering::SeqCst);
            let _ = stopping.send(reason);
            return;
        }
        std::thread::sleep(tick);
    }
}

/// Append one acknowledgement as one line, in one write, and make it durable. The file is opened
/// through the trace directory's capability ([`goalcup_platform::tracecap`]): `acks.jsonl` is
/// reached by its own name from a retained directory handle, never through a link, and must be
/// a regular file with one name, so an agent that left a link in its place cannot have the
/// acknowledgement written elsewhere. The handle is fresh for each acknowledgement: a refused
/// or failed write poisons a capability writer for good, and the next try must be able to start
/// again.
fn append_ack(trace_dir: &Path, ack: &AckCheckpoint) -> Result<(), CliError> {
    let acks = trace_dir.join(ACKS_FILE);
    let refused = |error: goalcup_platform::tracecap::TraceCapError| CliError::Io {
        path: acks.display().to_string(),
        source: std::io::Error::other(error.to_string()),
    };
    let mut line = serde_json::to_vec(ack)?;
    line.push(b'\n');
    let dir = goalcup_platform::tracecap::TraceDir::open(trace_dir).map_err(refused)?;
    let mut file = dir.open_acks_for_resume().map_err(refused)?;
    file.append_checked(&line).map_err(refused)?;
    dir.sync_append(&mut file).map_err(refused)
}

/// How an upload ended, for the participant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Every complete line of the chain is on the server, acknowledged.
    pub caught_up: bool,
    /// The server has the run's `seal.json` and `baseline.json`.
    pub sealed: bool,
    /// Why not, when either is false.
    pub why: Option<String>,
    /// Bytes of the chain the server holds.
    pub offset: u64,
    /// The server's answer to the seal says the run itself is over (sealed or incomplete).
    pub run_ended: bool,
}

impl Report {
    fn not(why: impl Into<String>, offset: u64) -> Report {
        Report {
            caught_up: false,
            sealed: false,
            why: Some(why.into()),
            offset,
            run_ended: false,
        }
    }
}

impl Uploader {
    /// Send everything and then the seal, asking again after each failure that does not end
    /// the upload, all within the wrap-up `until` (or for as long as it takes, with `None`).
    /// `say` hears each wait.
    ///
    /// The seal's documents are read before the chain is drained: the daemon writes them
    /// after the chain's last record, so the drain that follows sends every record they seal,
    /// and the server is never sealed on a copy missing its tail.
    pub fn finish(&mut self, until: Option<WrapUp>, say: &mut dyn FnMut(&str)) -> Report {
        let documents = self.seal_documents();
        // A sealed run commits its end first: after the deadline, the server accepts nothing
        // past it, and nothing at all until it is committed. So does a segment closed from its
        // tail, which has the end to commit and no seal to send.
        if (documents.is_some() || self.close_unsealed)
            && let Err(why) = self.commit_end(until, say)
        {
            return Report::not(why, self.pos.offset);
        }
        let mut failures = 0;
        let mut last = None::<String>;
        loop {
            match self.attempt(until) {
                Attempt::CaughtUp => break,
                Attempt::RanOut => {
                    return Report::not(ran_out(last.as_deref()), self.pos.offset);
                }
                Attempt::Stop(why) => return Report::not(why, self.pos.offset),
                Attempt::Retry(after, why) => {
                    failures += 1;
                    let wait = backoff(&self.policy, failures, after);
                    if past(until, wait) {
                        return Report::not(ran_out(Some(&why)), self.pos.offset);
                    }
                    say(&format!(
                        "upload: {why}; trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    last = Some(why);
                    std::thread::sleep(wait);
                }
            }
        }
        let caught_up = |sealed, why, run_ended| Report {
            caught_up: true,
            sealed,
            why,
            offset: self.pos.offset,
            run_ended,
        };
        let Some(documents) = documents else {
            // A segment closed from its tail has no seal to send: it is done when its chain,
            // up to its committed end, is.
            if self.close_unsealed {
                self.note_sent();
            }
            return caught_up(false, None, false);
        };
        let mut failures = 0;
        let mut last = None::<String>;
        loop {
            let Some(timeout) = left(until, super::http::CALL_TIMEOUT) else {
                return caught_up(false, Some(ran_out(last.as_deref())), false);
            };
            match self.seal(&documents, timeout) {
                Ok(run_ended) => {
                    self.mark_ok();
                    self.note_sent();
                    return caught_up(true, None, run_ended);
                }
                Err(error) if !error.ends_upload() => {
                    failures += 1;
                    let wait = backoff(
                        &self.policy,
                        failures,
                        error.retry_after().map(Duration::from_secs),
                    );
                    if past(until, wait) {
                        return caught_up(false, Some(ran_out(Some(&error.to_string()))), false);
                    }
                    say(&format!(
                        "upload: the seal was not stored ({error}); trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    last = Some(error.to_string());
                    std::thread::sleep(wait);
                }
                Err(error) => return caught_up(false, Some(error.to_string()), false),
            }
        }
    }
}

/// The uploader running beside `goalcup start`, on a thread of its own.
pub struct Running {
    entry_id: String,
    /// `None` once dropped: the upload thread then sees the run's end.
    finish: Option<std::sync::mpsc::Sender<WrapUp>>,
    /// The flag that keeps the background upload from starting anything more.
    quit: std::sync::Arc<AtomicBool>,
    report: std::sync::mpsc::Receiver<Report>,
    last_ok: std::sync::Arc<AtomicU64>,
    acked_offset: std::sync::Arc<AtomicU64>,
}

impl Drop for Running {
    /// A `Running` dropped without a finish (the run ended some other way) stops its upload
    /// thread rather than leaving it to retry in the background: nothing more is started, and
    /// the thread gets a moment to say it has returned.
    fn drop(&mut self) {
        self.quit.store(true, Ordering::SeqCst);
        // Its sender gone, the upload thread sees the end of the run and returns.
        drop(self.finish.take());
        let _ = self.report.recv_timeout(Duration::from_secs(2));
    }
}

/// The run's countdown to its deadline, for when the server cannot say stop. The server is the
/// clock (WP11): the count is set from the `seconds_left` of the open or continue answer, and set
/// again from every heartbeat answer that carries it, whatever it says (the server's answer
/// wins). It runs on a clock that keeps running while the machine sleeps
/// ([`goalcup_protocol::clock::ContinuousOrigin`]), and never reads this machine's date, which
/// the participant can change. During an outage it goes on counting down from the last answer.
///
/// Only an older server, whose answers carry no `seconds_left`, leaves the count to the rules
/// it had before: the run's length from when it opened ([`LocalDeadline::of`]), and for a
/// continued segment `deadline_at` less this machine's date ([`LocalDeadline::until`]).
#[derive(Debug, Clone, Copy)]
pub struct LocalDeadline {
    /// When the count was last set: taken before the call whose answer set it, so a slow or
    /// retried answer makes the count end earlier. It is never more than a second late: the
    /// server counts its own clock in whole seconds.
    pub from: goalcup_protocol::clock::ContinuousOrigin,
    /// What was left of the run at `from`.
    pub left: Duration,
}

impl LocalDeadline {
    /// `seconds_left` as a server answered it, counted from `asked`, which was taken before the
    /// call.
    pub fn left_from(
        seconds_left: u64,
        asked: goalcup_protocol::clock::ContinuousOrigin,
    ) -> LocalDeadline {
        LocalDeadline {
            from: asked,
            left: Duration::from_secs(seconds_left),
        }
    }

    /// The count for an entry that `goalcup start` (or `continue`, `continued`) has just had
    /// opened: the answer's `seconds_left` from `asked`, taken before the call. An older server's
    /// answer has none: then the run's length ([`LocalDeadline::of`]), or for a continued
    /// segment what this machine's date says is left ([`LocalDeadline::until`]).
    pub fn at_start(
        record: &UploadRecord,
        seconds_left: Option<u64>,
        asked: goalcup_protocol::clock::ContinuousOrigin,
        continued: bool,
    ) -> Option<LocalDeadline> {
        match (seconds_left, continued) {
            (Some(seconds), _) => Some(LocalDeadline::left_from(seconds, asked)),
            (None, false) => LocalDeadline::of(record, asked),
            (None, true) => LocalDeadline::until(&record.deadline_at),
        }
    }

    /// An older server only: the run's length from the record's two server times, from `opened`.
    pub fn of(
        record: &UploadRecord,
        opened: goalcup_protocol::clock::ContinuousOrigin,
    ) -> Option<LocalDeadline> {
        let start = goalcup_protocol::control::parse_z(&record.official_start)?;
        let deadline = goalcup_protocol::control::parse_z(&record.deadline_at)?;
        Some(LocalDeadline {
            from: opened,
            left: Duration::from_secs(deadline.checked_sub(start)?),
        })
    }

    /// An older server only: what is left of a run that is already under way, its `deadline_at`
    /// less this machine's date now (nothing if it has passed), counted from now on the
    /// continuous clock. This is the one place the date is read, and only because such a server
    /// says nothing better.
    pub fn until(deadline_at: &str) -> Option<LocalDeadline> {
        LocalDeadline::until_at(deadline_at, crate::run::now_unix())
    }

    /// [`LocalDeadline::until`] with this machine's date given (`now_unix`, epoch seconds).
    fn until_at(deadline_at: &str, now_unix: u64) -> Option<LocalDeadline> {
        let deadline = goalcup_protocol::control::parse_z(deadline_at)?;
        Some(LocalDeadline {
            from: goalcup_protocol::clock::ContinuousOrigin::now(),
            left: Duration::from_secs(deadline.saturating_sub(now_unix)),
        })
    }

    /// A heartbeat answer came, to a call made at `asked`: its `seconds_left` is the count from
    /// then on. An answer without one (an older server) leaves the count as it is.
    fn heard(
        count: &mut Option<LocalDeadline>,
        answer: &HeartbeatAnswer,
        asked: goalcup_protocol::clock::ContinuousOrigin,
    ) {
        if let Some(seconds) = answer.seconds_left {
            *count = Some(LocalDeadline::left_from(seconds, asked));
        }
    }

    fn passed(&self) -> bool {
        u128::from(self.from.elapsed_ns()) >= self.left.as_nanos()
    }
}

/// How long past the wrap-up `goalcup start` waits for the uploader's own last word.
const FINISH_GRACE: Duration = Duration::from_secs(10);

/// How often the wait for the uploader's last word looks at the continuous clock again.
const FINISH_SLICE: Duration = Duration::from_secs(5);

/// Start uploading `record` beside the run: the first chunk at once, then on [`Policy`]'s
/// cadence. Its waits and failures are said on stderr, prefixed `goalcup: upload:`; nothing it
/// does can stop or slow the recording.
///
/// `stop` is the run's one stop channel: when the server says the run is over (its deadline,
/// or a newer run replacing this one), or this machine's count of the run's length runs out,
/// the reason is sent on it once. `vitals` is what the heartbeats say about the recording, and
/// where the uploader leaves its own trouble for `goalcup status`.
#[allow(clippy::too_many_arguments)]
pub fn begin(
    server: ServerClient,
    record: UploadRecord,
    receipt_key: String,
    policy: Policy,
    spool_dir: PathBuf,
    local_deadline: Option<LocalDeadline>,
    stop: std::sync::mpsc::Sender<String>,
    vitals: Vitals,
) -> Result<Running, CliError> {
    begin_spawning(
        &spawn_thread,
        server,
        record,
        receipt_key,
        policy,
        spool_dir,
        local_deadline,
        stop,
        vitals,
    )
}

/// A thread named `name` running `work`, detached.
type Spawn<'a> = &'a dyn Fn(&str, Box<dyn FnOnce() + Send>) -> std::io::Result<()>;

fn spawn_thread(name: &str, work: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(work)
        .map(|_| ())
}

/// [`begin`], starting its threads with `spawn`. The heartbeat thread starts first, and a
/// thread that cannot start is an error of `begin`: a run whose heartbeats and deadline watch
/// cannot run is not begun, because nothing would tell the server it was alive, or stop it at
/// its deadline.
#[allow(clippy::too_many_arguments)]
fn begin_spawning(
    spawn: Spawn<'_>,
    server: ServerClient,
    record: UploadRecord,
    receipt_key: String,
    policy: Policy,
    spool_dir: PathBuf,
    local_deadline: Option<LocalDeadline>,
    stop: std::sync::mpsc::Sender<String>,
    vitals: Vitals,
) -> Result<Running, CliError> {
    let lock = lock_entry(&spool_dir, &record.entry_id)?;
    let entry_id = record.entry_id.clone();
    let (finish, finishing) = std::sync::mpsc::channel::<WrapUp>();
    let (reported, report) = std::sync::mpsc::channel::<Report>();
    let last_ok = std::sync::Arc::new(AtomicU64::new(crate::run::now_unix()));
    let acked_offset = std::sync::Arc::new(AtomicU64::new(0));
    let shared = (last_ok.clone(), acked_offset.clone());
    // Heartbeats and the deadline run on a thread of their own, so a slow chunk upload never
    // delays a stop. Once the run must stop, uploading waits for the finish.
    let over = std::sync::Arc::new(AtomicBool::new(false));
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let ended = EndedGate::default();
    let spawn_error = |what: &str| {
        let what = what.to_string();
        move |source| CliError::Io {
            path: format!("the {what} thread"),
            source,
        }
    };
    {
        let (server, record, vitals) = (server.clone(), record.clone(), vitals.clone());
        let (over, done, ended) = (over.clone(), done.clone(), ended.clone());
        spawn(
            "goalcup-heartbeat",
            Box::new(move || {
                watch_deadline(
                    &server,
                    &record,
                    policy,
                    local_deadline,
                    &vitals,
                    &stop,
                    &over,
                    &done,
                    &ended,
                )
            }),
        )
        .map_err(spawn_error("heartbeat"))?;
    }
    let quit = over.clone();
    let beside = Beside {
        finishing,
        over,
        ended,
        shared,
        vitals,
    };
    let heartbeats_done = done.clone();
    spawn(
        "goalcup-upload",
        Box::new(move || {
            let _lock = lock;
            let _done_on_return = SetOnDrop(done);
            let ended = run(server, record, receipt_key, policy, &spool_dir, beside);
            let _ = reported.send(ended);
        }),
    )
    .map_err(|source| {
        // The heartbeat thread is already running: it is told to end, as the upload thread
        // would have told it.
        heartbeats_done.store(true, Ordering::SeqCst);
        spawn_error("upload")(source)
    })?;
    Ok(Running {
        entry_id,
        finish: Some(finish),
        quit,
        report,
        last_ok,
        acked_offset,
    })
}

/// What the upload thread hears from, and says to, `goalcup start`.
struct Beside {
    /// The run is sealed: finish within this wrap-up.
    finishing: std::sync::mpsc::Receiver<WrapUp>,
    /// Set by the heartbeat thread once the run must stop.
    over: std::sync::Arc<AtomicBool>,
    /// Set when the final heartbeat is stored, after which no ordinary one is sent.
    ended: EndedGate,
    /// The counters [`Running::last_ok`] and [`Running::acked_offset`] read.
    shared: (std::sync::Arc<AtomicU64>, std::sync::Arc<AtomicU64>),
    vitals: Vitals,
}

impl Running {
    /// The unix second of the last time this upload worked: it starts at the run's launch, and
    /// is set again by each acknowledged chunk, by the seal, and whenever there is nothing
    /// left to send. The run's disk watch calls uploads failing when it is half an hour old.
    pub fn last_ok(&self) -> std::sync::Arc<AtomicU64> {
        self.last_ok.clone()
    }

    /// The entry this upload sends.
    pub fn entry_id(&self) -> &str {
        &self.entry_id
    }

    /// Bytes of the chain the server has acknowledged.
    pub fn acked_offset(&self) -> std::sync::Arc<AtomicU64> {
        self.acked_offset.clone()
    }

    /// The run is sealed: send the rest and the seal, asking again after every failure that
    /// does not end the upload, until `wrap_up` has run out, and say how it went. Returns by
    /// then (and a short grace) whatever the server does: an upload still under way is left
    /// to `goalcup upload`.
    ///
    /// The wait is counted on the continuous clock, in slices: a machine that sleeps through
    /// the wrap-up does not owe it again on waking.
    pub fn finish(self, wrap_up: WrapUp) -> Report {
        if let Some(finish) = &self.finish {
            let _ = finish.send(wrap_up);
        }
        let last_word = WrapUp::after(wrap_up.remaining() + FINISH_GRACE);
        loop {
            let left = last_word.remaining();
            if left.is_zero() {
                return Report::not("the upload did not finish in time", 0);
            }
            match self.report.recv_timeout(left.min(FINISH_SLICE)) {
                Ok(report) => return report,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Report::not("the upload thread ended without a report", 0);
                }
            }
        }
    }
}

/// How the end of an entry that is no longer recording is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closing {
    /// The entry is sealed locally: its end, the rest of its chain, then the seal.
    Sealed,
    /// The entry could not be sealed (the host died, the machine rebooted, the chain's tail is
    /// torn): its end is the last whole record of its chain, committed in a final heartbeat, and
    /// the chain is sent up to it. No seal goes; the segment is incomplete, and says so.
    FromTail,
}

/// An entry that is no longer recording, being closed and sent on a thread of its own
/// ([`drain`]), beside the segment that came after it.
pub struct Draining {
    entry_id: String,
    closing: Closing,
    last_ok: std::sync::Arc<AtomicU64>,
    acked_offset: std::sync::Arc<AtomicU64>,
    report: std::sync::mpsc::Receiver<Report>,
}

/// Close `record`'s entry on the server on a thread of its own: its final heartbeat, the rest of
/// its chain and (when it is [`Closing::Sealed`]) its seal, asking again after every failure
/// that does not end the upload, until `wrap_up` has run out. It holds the entry's upload lock
/// ([`lock_entry`]) until it is done, so a refusal here means another goalcup uploads the entry.
pub fn drain(
    server: ServerClient,
    record: UploadRecord,
    receipt_key: String,
    policy: Policy,
    spool_dir: PathBuf,
    closing: Closing,
    wrap_up: WrapUp,
) -> Result<Draining, CliError> {
    let lock = lock_entry(&spool_dir, &record.entry_id)?;
    let entry_id = record.entry_id.clone();
    let (reported, report) = std::sync::mpsc::channel::<Report>();
    let last_ok = std::sync::Arc::new(AtomicU64::new(crate::run::now_unix()));
    let acked_offset = std::sync::Arc::new(AtomicU64::new(0));
    let shared = (last_ok.clone(), acked_offset.clone());
    std::thread::Builder::new()
        .name("goalcup-drain".into())
        .spawn(move || {
            let _lock = lock;
            let mut say = crate::host_log::say;
            let mut failures = 0u32;
            let ended = loop {
                match Uploader::resume(
                    server.clone(),
                    record.clone(),
                    receipt_key.clone(),
                    policy,
                    &spool_dir,
                ) {
                    Ok(mut up) => {
                        up.share(&shared.0, &shared.1);
                        up.close_unsealed = closing == Closing::FromTail;
                        break up.finish(Some(wrap_up), &mut say);
                    }
                    Err(error) if !error.ends_upload() => {
                        failures += 1;
                        let wait = backoff(
                            &policy,
                            failures,
                            error.retry_after().map(Duration::from_secs),
                        );
                        if past(Some(wrap_up), wait) {
                            break Report::not(ran_out(Some(&error.to_string())), 0);
                        }
                        say(&format!(
                            "upload of entry {}: {error}; trying again in {}s",
                            record.entry_id,
                            wait.as_secs().max(1)
                        ));
                        std::thread::sleep(wait);
                    }
                    Err(error) => break Report::not(error.to_string(), 0),
                }
            };
            let _ = reported.send(ended);
        })
        .map_err(|source| CliError::Io {
            path: "the upload thread".to_string(),
            source,
        })?;
    Ok(Draining {
        entry_id,
        closing,
        last_ok,
        acked_offset,
        report,
    })
}

/// What to do about an earlier segment's unfinished upload. One sealed by its host is finished
/// by `goalcup upload`; one closed from its tail has an end `goalcup upload` cannot commit, and
/// the next `goalcup continue` closes and sends it again.
pub fn finish_advice(closing: Closing, entry_id: &str) -> String {
    match closing {
        Closing::Sealed => format!("Run `goalcup upload {entry_id}` to finish sending it."),
        Closing::FromTail => {
            "Run `goalcup continue` (it finishes the earlier segment's upload).".to_string()
        }
    }
}

/// What the disk watch reads of an entry that is being drained: the same two counters as of a
/// recording entry's upload ([`Running::last_ok`], [`Running::acked_offset`]).
#[derive(Debug, Clone)]
pub struct DrainHandles {
    pub entry_id: String,
    pub last_ok: std::sync::Arc<AtomicU64>,
    pub acked_offset: std::sync::Arc<AtomicU64>,
}

impl Draining {
    /// The entry this thread sends.
    pub fn entry_id(&self) -> &str {
        &self.entry_id
    }

    /// How the entry was closed: a segment closed from its tail cannot be finished by
    /// `goalcup upload`, only by a `goalcup continue`.
    pub fn closing(&self) -> Closing {
        self.closing
    }

    /// What to tell the participant to do about this entry's upload when it did not finish.
    pub fn how_to_finish(&self) -> String {
        finish_advice(self.closing, &self.entry_id)
    }

    /// The counters the disk watch reads, for the watch to cover this entry too.
    pub fn handles(&self) -> DrainHandles {
        DrainHandles {
            entry_id: self.entry_id.clone(),
            last_ok: self.last_ok.clone(),
            acked_offset: self.acked_offset.clone(),
        }
    }

    /// Wait for the thread's report for as long as `until` leaves (and a short grace). The
    /// thread has a wrap-up of its own and stops by it; a report that is not there by then is
    /// "did not finish", and what is unsent is `goalcup upload`'s.
    pub fn wait(self, until: WrapUp) -> Report {
        let last_word = WrapUp::after(until.remaining() + FINISH_GRACE);
        loop {
            let left = last_word.remaining();
            if left.is_zero() {
                return Report::not("the upload did not finish in time", 0);
            }
            match self.report.recv_timeout(left.min(FINISH_SLICE)) {
                Ok(report) => return report,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Report::not("the upload thread ended without a report", 0);
                }
            }
        }
    }
}

fn run(
    server: ServerClient,
    record: UploadRecord,
    key: String,
    policy: Policy,
    spool_dir: &Path,
    beside: Beside,
) -> Report {
    use std::sync::mpsc::RecvTimeoutError;
    let Beside {
        finishing,
        over,
        ended,
        shared,
        vitals,
    } = beside;
    let mut say = crate::host_log::say;
    let mut uploader: Option<Uploader> = None;
    let mut stopped: Option<String> = None;
    let mut failures = 0u32;
    let mut wait_until = Instant::now();
    let mut next_due = Instant::now();
    let mut cadence = Cadence::new(&policy);
    loop {
        let now = Instant::now();
        if !over.load(Ordering::SeqCst) && stopped.is_none() && now >= wait_until {
            if uploader.is_none() {
                match Uploader::resume(
                    server.clone(),
                    record.clone(),
                    key.clone(),
                    policy,
                    spool_dir,
                ) {
                    Ok(mut resumed) => {
                        resumed.halt = Some(over.clone());
                        resumed.share(&shared.0, &shared.1);
                        resumed.vitals = Some(vitals.clone());
                        resumed.ended = ended.clone();
                        uploader = Some(resumed);
                    }
                    Err(error) if !error.ends_upload() => {
                        failures += 1;
                        let wait = backoff(
                            &policy,
                            failures,
                            error.retry_after().map(Duration::from_secs),
                        );
                        say(&format!(
                            "upload: {error}; trying again in {}s",
                            wait.as_secs().max(1)
                        ));
                        vitals.upload_failed(&error.to_string());
                        wait_until = now + wait;
                    }
                    Err(error) => {
                        say(&format!(
                            "upload stopped: {error}. The run goes on recording on this machine."
                        ));
                        vitals.upload_stopped(&error.to_string());
                        stopped = Some(error.to_string());
                    }
                }
            }
            // Nothing waiting is an upload with nothing to complain of.
            if let Some(up) = uploader.as_ref()
                && stopped.is_none()
                && up.waiting() == 0
                // An acknowledgement still to be written is trouble, not an idle upload.
                && up.owed_ack.is_none()
            {
                up.mark_ok();
                vitals.upload_ok();
            }
            if let Some(up) = uploader.as_mut()
                && (now >= next_due || up.waiting() >= policy.eager_bytes)
            {
                match up.attempt(None) {
                    Attempt::CaughtUp => {
                        failures = 0;
                        next_due = cadence.next_due(now);
                        vitals.upload_ok();
                    }
                    Attempt::Retry(after, why) => {
                        failures += 1;
                        let wait = backoff(&policy, failures, after);
                        say(&format!(
                            "upload: {why}; trying again in {}s",
                            wait.as_secs().max(1)
                        ));
                        if why != STOPPING {
                            vitals.upload_failed(&why);
                        }
                        wait_until = now + wait;
                    }
                    Attempt::Stop(why) => {
                        say(&format!(
                            "upload stopped: {why}. The run goes on recording on this machine."
                        ));
                        vitals.upload_stopped(&why);
                        stopped = Some(why);
                    }
                    // Only a finish has a wrap-up to run out; a background attempt has none.
                    Attempt::RanOut => {}
                }
            }
        }
        let offset = || uploader.as_ref().map(|u| u.pos.offset).unwrap_or(0);
        match finishing.recv_timeout(policy.poll) {
            Err(RecvTimeoutError::Timeout) => continue,
            // `goalcup start` ended without a finish (a refusal after the launch): stop here.
            Err(RecvTimeoutError::Disconnected) => {
                return Report::not("the run ended before its upload finished", offset());
            }
            Ok(wrap_up) => {
                if let Some(why) = stopped {
                    return Report::not(why, offset());
                }
                let mut up = match uploader {
                    Some(up) => up,
                    None => match Uploader::resume(server, record, key, policy, spool_dir) {
                        Ok(mut up) => {
                            up.share(&shared.0, &shared.1);
                            up.vitals = Some(vitals.clone());
                            up.ended = ended.clone();
                            up
                        }
                        Err(error) => return Report::not(error.to_string(), 0),
                    },
                };
                return up.finish(Some(wrap_up), &mut say);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A segment closed from its tail cannot be finished by `goalcup upload`; the advice for
    /// it is `goalcup continue`, which closes and sends it again.
    #[test]
    fn a_tail_closed_segment_is_finished_by_continue_not_upload() {
        let sealed = finish_advice(Closing::Sealed, "e1");
        assert!(sealed.contains("goalcup upload e1"), "{sealed}");
        let tail = finish_advice(Closing::FromTail, "e1");
        assert!(tail.contains("goalcup continue"), "{tail}");
        assert!(!tail.contains("goalcup upload"), "{tail}");
    }

    #[test]
    fn a_commit_noted_as_a_segment_ended_is_not_replaced_by_a_later_one() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("work");
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        let head = |commit: &str| {
            std::fs::write(workspace.join(".git/HEAD"), format!("{commit}\n")).unwrap();
        };
        let record = UploadRecord {
            run_id: "r0123456789abcdef".into(),
            entry_id: "e1".into(),
            server_url: String::new(),
            trace_dir: String::new(),
            official_start: String::new(),
            deadline_at: String::new(),
            workspace: workspace.display().to_string(),
        };
        let (first, later) = ("a".repeat(40), "b".repeat(40));
        head(&first);
        note_repo_head_unless_noted(dir.path(), &record);
        assert_eq!(noted_repo_head_in(dir.path(), "e1"), Some(first.clone()));
        // The next segment's agent commits; the earlier segment's note stands.
        head(&later);
        note_repo_head_unless_noted(dir.path(), &record);
        assert_eq!(noted_repo_head_in(dir.path(), "e1"), Some(first));
        // An entry with no note takes the commit the workspace has.
        let other = UploadRecord {
            entry_id: "e2".into(),
            ..record
        };
        note_repo_head_unless_noted(dir.path(), &other);
        assert_eq!(noted_repo_head_in(dir.path(), "e2"), Some(later));
    }

    // ---- the server is the clock (WP11)

    fn answer_with(seconds_left: Option<u64>) -> HeartbeatAnswer {
        HeartbeatAnswer {
            stop: false,
            deadline_at: "2026-11-14T00:00:00Z".into(),
            seconds_left,
            reason: None,
        }
    }

    /// Each answer that says how much is left sets the count again, also to more time than the
    /// count had (the server's answer wins). An older server's answer leaves it alone, and with
    /// no answer at all (an outage) it goes on counting down from the last one.
    #[test]
    fn each_answer_sets_the_count_again_and_an_outage_counts_on_from_the_last() {
        use goalcup_protocol::clock::ContinuousOrigin;
        let mut count = Some(LocalDeadline::left_from(3600, ContinuousOrigin::now()));
        assert!(!count.unwrap().passed());
        LocalDeadline::heard(&mut count, &answer_with(Some(0)), ContinuousOrigin::now());
        assert!(
            count.unwrap().passed(),
            "nothing left: the count has run out"
        );
        LocalDeadline::heard(
            &mut count,
            &answer_with(Some(7200)),
            ContinuousOrigin::now(),
        );
        assert!(
            !count.unwrap().passed(),
            "the next answer wins, with more time"
        );
        assert_eq!(count.unwrap().left, Duration::from_secs(7200));
        LocalDeadline::heard(&mut count, &answer_with(None), ContinuousOrigin::now());
        assert_eq!(
            count.unwrap().left,
            Duration::from_secs(7200),
            "an older server's answer changes nothing"
        );
        // A run whose record gave no count takes the first answer's.
        let mut none = None;
        LocalDeadline::heard(&mut none, &answer_with(Some(5)), ContinuousOrigin::now());
        assert_eq!(none.unwrap().left, Duration::from_secs(5));

        // The outage: no answer comes, and the count runs out on its own.
        let last = LocalDeadline {
            from: ContinuousOrigin::now(),
            left: Duration::from_millis(200),
        };
        assert!(!last.passed());
        std::thread::sleep(Duration::from_millis(300));
        assert!(last.passed(), "the count went on through the outage");
    }

    /// An entry's first count is the server's `seconds_left`, whatever this machine's date says,
    /// for a new run and a continued segment alike. Only an older server's answer, which has
    /// none, falls back to the run's length or (continued) to the date.
    #[test]
    fn the_first_count_is_the_servers_time_left_and_never_the_date() {
        use goalcup_protocol::clock::ContinuousOrigin;
        let record = UploadRecord {
            official_start: "2099-01-01T00:00:00Z".into(),
            deadline_at: "2099-01-08T00:00:00Z".into(),
            ..record_in(Path::new("/nowhere"))
        };
        let asked = ContinuousOrigin::now();
        for continued in [false, true] {
            let count = LocalDeadline::at_start(&record, Some(90), asked, continued).unwrap();
            assert_eq!(count.left, Duration::from_secs(90));
            assert_eq!(count.from, asked, "counted from when the server was asked");
        }
        let fresh = LocalDeadline::at_start(&record, None, asked, false).unwrap();
        assert_eq!(
            fresh.left,
            Duration::from_secs(168 * 3600),
            "the run's length"
        );
        // A continued segment on an older server reads the date (now, long before the deadline).
        let continued = LocalDeadline::at_start(&record, None, asked, true).unwrap();
        assert!(continued.left > Duration::ZERO);
    }

    /// The older server's rule for a continued segment, on a skewed date: a deadline the date
    /// says has passed gives nothing left, and one in the future is counted on the continuous
    /// clock from now. (A date set a day back gives a day more: why the server's count wins.)
    #[test]
    fn until_on_a_skewed_date_counts_what_the_date_says_on_the_continuous_clock() {
        let deadline = "2026-11-14T00:00:00Z";
        let at = goalcup_protocol::control::parse_z(deadline).unwrap();
        let past = LocalDeadline::until_at(deadline, at + 24 * 3600).unwrap();
        assert_eq!(past.left, Duration::ZERO);
        assert!(past.passed(), "a deadline in the past has run out at once");
        let future = LocalDeadline::until_at(deadline, at - 90).unwrap();
        assert_eq!(future.left, Duration::from_secs(90));
        assert!(
            future.from.elapsed_ns() < 5_000_000_000,
            "counted from now on the continuous clock"
        );
        assert!(!future.passed());
        let set_back = LocalDeadline::until_at(deadline, at - 90 - 24 * 3600).unwrap();
        assert_eq!(set_back.left, Duration::from_secs(90 + 24 * 3600));
    }

    #[test]
    fn a_sent_entry_is_noted_and_only_that_one() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_sent(dir.path(), "e1"));
        std::fs::write(sent_path(dir.path(), "e1"), b"sent\n").unwrap();
        assert!(is_sent(dir.path(), "e1"));
        assert!(!is_sent(dir.path(), "e2"));
    }

    fn record_in(dir: &Path) -> UploadRecord {
        UploadRecord {
            run_id: "r0123456789abcdef".into(),
            entry_id: "e1".into(),
            server_url: String::new(),
            trace_dir: dir.join("trace").display().to_string(),
            official_start: String::new(),
            deadline_at: String::new(),
            workspace: String::new(),
        }
    }

    /// A heartbeat thread that cannot start is an error of `begin`, and the upload thread is
    /// never started behind it.
    #[test]
    fn a_heartbeat_thread_that_cannot_start_is_an_error_of_begin() {
        let dir = tempfile::tempdir().unwrap();
        let spawned = Mutex::new(Vec::<String>::new());
        let spawn = |name: &str, _work: Box<dyn FnOnce() + Send>| {
            spawned.lock().unwrap().push(name.to_string());
            Err(std::io::Error::other("no threads left"))
        };
        let (stop, _stopped) = std::sync::mpsc::channel();
        let result = begin_spawning(
            &spawn,
            ServerClient::anonymous("http://127.0.0.1:1"),
            record_in(dir.path()),
            "key".into(),
            Policy::default(),
            dir.path().join("uploads"),
            None,
            stop,
            Vitals::default(),
        );
        match result {
            Err(CliError::Io { path, source }) => {
                assert!(path.contains("heartbeat"), "{path}");
                assert!(source.to_string().contains("no threads left"), "{source}");
            }
            Err(other) => panic!("a different error: {other}"),
            Ok(_) => panic!("begin went on without its heartbeat thread"),
        }
        assert_eq!(*spawned.lock().unwrap(), ["goalcup-heartbeat"]);
    }

    /// An upload thread that cannot start is an error too, and the heartbeat thread that is
    /// already running is told to end rather than left beating for a run that never began.
    #[test]
    fn an_upload_thread_that_cannot_start_ends_the_heartbeat_thread_it_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let heartbeat = Mutex::new(None);
        let spawn = |name: &str, work: Box<dyn FnOnce() + Send>| {
            if name == "goalcup-heartbeat" {
                *heartbeat.lock().unwrap() = Some(std::thread::spawn(work));
                Ok(())
            } else {
                Err(std::io::Error::other("no threads left"))
            }
        };
        let (stop, _stopped) = std::sync::mpsc::channel();
        let result = begin_spawning(
            &spawn,
            ServerClient::anonymous("http://127.0.0.1:1"),
            record_in(dir.path()),
            "key".into(),
            Policy::default(),
            dir.path().join("uploads"),
            None,
            stop,
            Vitals::default(),
        );
        assert!(
            matches!(result, Err(CliError::Io { ref path, .. }) if path.contains("upload")),
            "begin refused"
        );
        // Joins only if the thread was told to end.
        heartbeat.lock().unwrap().take().unwrap().join().unwrap();
    }

    /// A commit that cannot be noted is said, so the participant is not left to find it missing.
    #[test]
    fn a_commit_that_cannot_be_noted_is_said() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("work");
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        std::fs::write(workspace.join(".git/HEAD"), format!("{}\n", "a".repeat(40))).unwrap();
        let record = UploadRecord {
            workspace: workspace.display().to_string(),
            ..record_in(dir.path())
        };
        // A spool directory that is a file: nothing can be written beneath it.
        let spool = dir.path().join("uploads");
        std::fs::write(&spool, b"not a directory").unwrap();
        let mut said = Vec::new();
        note_repo_head_saying(&spool, &record, &mut |line| said.push(line.to_string()));
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("could not be noted") && said[0].contains("without it"),
            "{said:?}"
        );
        assert_eq!(noted_repo_head_in(&spool, "e1"), None);

        // Written, nothing is said.
        let spool = dir.path().join("spool");
        std::fs::create_dir_all(&spool).unwrap();
        let mut said = Vec::new();
        note_repo_head_saying(&spool, &record, &mut |line| said.push(line.to_string()));
        assert!(said.is_empty(), "{said:?}");
        assert_eq!(noted_repo_head_in(&spool, "e1"), Some("a".repeat(40)));
    }
}
