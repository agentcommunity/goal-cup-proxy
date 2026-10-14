//! The uploader: the entry's `chain.jsonl` to the server in chunks, each acknowledged.
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
//!   recording. After the stop the same asking goes on for the wrap-up ([`Policy::wrap_up`],
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
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::export::{ACKS_FILE, BASELINE_FILE, CHAIN_FILE, SEAL_FILE};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// How many complete-line bytes of `chain` lie past `offset` (a torn last line is not
/// counted, but neither is it looked for: this is the file's length, a cheap upper bound).
pub fn waiting_bytes(chain: &Path, offset: u64) -> u64 {
    std::fs::metadata(chain)
        .map(|m| m.len().saturating_sub(offset))
        .unwrap_or(0)
}

/// The next chunk of `chain` from `pos`, compressed into `spool`, or `None` when no complete
/// line waits. Every line's link is checked to continue from `pos`, so a chunk can only ever
/// extend what the server holds: a local chain that does not is refused, not uploaded.
pub fn build_chunk(chain: &Path, pos: &Position, spool: &Path) -> Result<Option<Chunk>, CliError> {
    let mut file = std::fs::File::open(chain).map_err(io_error(chain))?;
    file.seek(SeekFrom::Start(pos.offset))
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
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: `flock` on a descriptor this function owns; it reads and writes no memory.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(CliError::Refused(format!(
                "entry {entry_id} is being uploaded by another goalcup: its `goalcup start` is \
                 still running, or another `goalcup upload` is. Run this again once it ends."
            )));
        }
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
        let position: RunPosition =
            server.json("GET", &format!("/v1/runs/{}", record.run_id), None::<&()>)?;
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
        };
        uploader.adopt_server_ack(&position);
        Ok(uploader)
    }

    fn chain(&self) -> PathBuf {
        Path::new(&self.record.trace_dir).join(CHAIN_FILE)
    }

    /// Bytes of chain waiting to be sent (an upper bound).
    pub fn waiting(&self) -> u64 {
        waiting_bytes(&self.chain(), self.pos.offset)
    }

    /// Keep the server's last acknowledgement if the local chain's record ending at the
    /// server's offset is the one it names. An acknowledgement of anything else would make
    /// the trace fail verification, and dropping one is always safe.
    fn adopt_server_ack(&mut self, position: &RunPosition) {
        let Some(ack) = &position.last_ack else {
            return;
        };
        let local = local_link_before(&self.chain(), position.offset);
        if local
            .as_ref()
            .map(|(seq, hash)| (Some(*seq), hash.as_str()))
            == Some((position.next_seq.checked_sub(1), position.head.as_str()))
        {
            let _ = self.keep_ack(ack, position.next_seq.checked_sub(1), &position.head);
        }
    }

    /// Send every complete line, chunk by chunk, once, within the wrap-up `until` if one is
    /// given.
    fn attempt(&mut self, until: Option<WrapUp>) -> Attempt {
        loop {
            // A background upload stops between chunks once the run must stop.
            if until.is_none() && self.halt.as_ref().is_some_and(|h| h.load(Ordering::SeqCst)) {
                return Attempt::Retry(None, "the run is stopping".to_string());
            }
            // Only a finish has a wrap-up; once it has run out, the rest is `goalcup upload`'s.
            let Some(timeout) = left(until, self.policy.put_timeout) else {
                return Attempt::RanOut;
            };
            let chunk = match build_chunk(&self.chain(), &self.pos, &self.spool) {
                Ok(Some(chunk)) => chunk,
                Ok(None) => return Attempt::CaughtUp,
                Err(error) => {
                    let _ = std::fs::remove_file(&self.spool);
                    return Attempt::Stop(error.to_string());
                }
            };
            let sent = self.put(&chunk, timeout);
            let _ = std::fs::remove_file(&chunk.spool);
            match sent {
                Ok(answer) => {
                    if let Err(why) = self.keep_ack(&answer.ack, Some(chunk.last_seq), &chunk.head)
                    {
                        return Attempt::Stop(why);
                    }
                    self.pos = Position {
                        offset: chunk.offset + chunk.length,
                        next_seq: chunk.last_seq + 1,
                        head: chunk.head.clone(),
                    };
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
        let position: RunPosition = self.server.json(
            "GET",
            &format!("/v1/runs/{}", self.record.run_id),
            None::<&()>,
        )?;
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
    ) -> Result<(), String> {
        let at = goalcup_protocol::provenance::Rfc3339Utc::try_from(ack.acked_at.as_str())
            .ok()
            .and(ack.acked_at_secs());
        let fits = ack.entry_id == self.record.entry_id && Some(ack.seq) == seq && ack.hash == head;
        let (Some(at), true, Ok(())) = (at, fits, ack.verify(&self.key)) else {
            return Err(format!(
                "the server's acknowledgement of seq {} does not match the chunk it answers, is \
                 not signed by the key of this run's start receipt, or has no valid time",
                ack.seq
            ));
        };
        if self
            .acked
            .is_some_and(|(last_seq, last_at)| ack.seq <= last_seq || at < last_at)
        {
            return Ok(());
        }
        append_ack(&Path::new(&self.record.trace_dir).join(ACKS_FILE), ack)
            .map_err(|e| e.to_string())?;
        self.acked = Some((ack.seq, at));
        Ok(())
    }

    /// The final heartbeat: commit the sealed chain's last record as the run's end, asking
    /// again after each failure that does not end the upload, within the wrap-up `until`.
    fn commit_end(
        &mut self,
        until: Option<WrapUp>,
        say: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        let mut failures = 0;
        let mut last = None::<String>;
        loop {
            let Some(timeout) = left(until, super::http::CALL_TIMEOUT) else {
                return Err(ran_out(last.as_deref()));
            };
            let repo_head = noted_repo_head(&self.spool, &self.record.entry_id);
            match heartbeat(&self.server, &self.record, true, repo_head, timeout) {
                None => return Err("the chain holds no whole record".to_string()),
                Some(Ok(_)) => return Ok(()),
                Some(Err(error)) if !error.ends_upload() => {
                    failures += 1;
                    let wait = backoff(
                        &self.policy,
                        failures,
                        error.retry_after().map(Duration::from_secs),
                    );
                    if past(until, wait) {
                        return Err(ran_out(Some(&error.to_string())));
                    }
                    say(&format!(
                        "upload: the final heartbeat was not stored ({error}); trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    last = Some(error.to_string());
                    std::thread::sleep(wait);
                }
                Some(Err(error)) => return Err(error.to_string()),
            }
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
    fn seal(&self, documents: &(String, String), timeout: Duration) -> Result<(), ServerError> {
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
            .map(|_| ())
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

/// The `seq` and `hash` of the chain record whose line ends exactly at byte `offset`, or
/// `None` when no whole record ends there.
pub fn local_link_before(chain: &Path, offset: u64) -> Option<(u64, String)> {
    if offset == 0 {
        return None;
    }
    let start = offset.saturating_sub(MAX_LINE_BYTES + 1);
    let mut file = std::fs::File::open(chain).ok()?;
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = vec![0; (offset - start) as usize];
    file.read_exact(&mut bytes).ok()?;
    let body = bytes.strip_suffix(b"\n")?;
    let line = match body.iter().rposition(|b| *b == b'\n') {
        Some(at) => &body[at + 1..],
        None if start == 0 => body,
        None => return None,
    };
    let link = serde_json::from_slice::<LinkOnly>(line).ok()?.link;
    Some((link.seq, link.hash))
}

/// The last whole record of `chain`: the bytes up to the end of it, its `seq` and its `hash`.
/// `None` for a chain with no whole record.
pub fn local_end(chain: &Path) -> Option<(u64, u64, String)> {
    let len = std::fs::metadata(chain).ok()?.len();
    let start = len.saturating_sub(2 * (MAX_LINE_BYTES + 1));
    let mut file = std::fs::File::open(chain).ok()?;
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    file.take(len - start).read_to_end(&mut tail).ok()?;
    // The end of the last whole line; a torn tail after it is not a record yet.
    let end = start + tail.iter().rposition(|b| *b == b'\n')? as u64 + 1;
    let (seq, hash) = local_link_before(chain, end)?;
    Some((end, seq, hash))
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

/// `<cfg>/uploads/<entry>.repo-head`: the commit noted when the run stopped.
fn repo_head_path(spool_dir: &Path, entry_id: &str) -> PathBuf {
    spool_dir.join(format!("{entry_id}.repo-head"))
}

/// Note the workspace repository's checked-out commit now, the moment the run stops, for the
/// final heartbeat to send whenever it goes (a later `goalcup upload` included). Nothing is
/// noted when there is no repository.
pub fn note_repo_head(spool_dir: &Path, record: &UploadRecord) {
    if record.workspace.is_empty() {
        return;
    }
    if let Some(commit) = repo_head(Path::new(&record.workspace)) {
        let _ = crate::config::write_secret_file(
            &repo_head_path(spool_dir, &record.entry_id),
            commit.as_bytes(),
        );
    }
}

/// The commit [`note_repo_head`] noted for the entry whose spool is `spool`, if any.
fn noted_repo_head(spool: &Path, entry_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(repo_head_path(spool.parent()?, entry_id)).ok()?;
    let text = text.trim();
    (text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())).then(|| text.to_string())
}

/// `POST /v1/runs/{id}/heartbeat` with the local chain's last whole record: `final` once the
/// run has ended, with the workspace repository's commit. `None` when the chain holds no
/// whole record yet.
fn heartbeat(
    server: &ServerClient,
    record: &UploadRecord,
    is_final: bool,
    repo_head: Option<String>,
    timeout: Duration,
) -> Option<Result<HeartbeatAnswer, ServerError>> {
    let (offset_local, seq, head) = local_end(&Path::new(&record.trace_dir).join(CHAIN_FILE))?;
    let request = HeartbeatRequest {
        entry_id: record.entry_id.clone(),
        seq,
        head,
        offset_local,
        is_final,
        repo_head,
    };
    Some(server.json_timed(
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

/// Send a heartbeat every `policy.heartbeat` and watch the run's own deadline, until the run
/// must stop (said once on `stopping`, and `over` set) or the uploader is `done`.
fn watch_deadline(
    server: &ServerClient,
    record: &UploadRecord,
    policy: Policy,
    local_deadline: Option<LocalDeadline>,
    stopping: &std::sync::mpsc::Sender<String>,
    over: &AtomicBool,
    done: &AtomicBool,
) {
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
            match heartbeat(server, record, false, None, HEARTBEAT_TIMEOUT) {
                Some(Ok(answer)) if answer.stop => Some(format!(
                    "the server says the run is over (its deadline was {})",
                    answer.deadline_at
                )),
                _ => None,
            }
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

/// Append one acknowledgement as one line, in one write, and make it durable.
fn append_ack(acks: &Path, ack: &AckCheckpoint) -> Result<(), CliError> {
    let mut line = serde_json::to_vec(ack)?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(acks)
        .map_err(io_error(acks))?;
    file.write_all(&line).map_err(io_error(acks))?;
    file.sync_data().map_err(io_error(acks))
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
}

impl Report {
    fn not(why: impl Into<String>, offset: u64) -> Report {
        Report {
            caught_up: false,
            sealed: false,
            why: Some(why.into()),
            offset,
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
        // past it, and nothing at all until it is committed.
        if documents.is_some()
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
        let caught_up = |sealed, why| Report {
            caught_up: true,
            sealed,
            why,
            offset: self.pos.offset,
        };
        let Some(documents) = documents else {
            return caught_up(false, None);
        };
        let mut failures = 0;
        let mut last = None::<String>;
        loop {
            let Some(timeout) = left(until, super::http::CALL_TIMEOUT) else {
                return caught_up(false, Some(ran_out(last.as_deref())));
            };
            match self.seal(&documents, timeout) {
                Ok(()) => return caught_up(true, None),
                Err(error) if !error.ends_upload() => {
                    failures += 1;
                    let wait = backoff(
                        &self.policy,
                        failures,
                        error.retry_after().map(Duration::from_secs),
                    );
                    if past(until, wait) {
                        return caught_up(false, Some(ran_out(Some(&error.to_string()))));
                    }
                    say(&format!(
                        "upload: the seal was not stored ({error}); trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    last = Some(error.to_string());
                    std::thread::sleep(wait);
                }
                Err(error) => return caught_up(false, Some(error.to_string())),
            }
        }
    }
}

/// The uploader running beside `goalcup start`, on a thread of its own.
pub struct Running {
    finish: std::sync::mpsc::Sender<WrapUp>,
    report: std::sync::mpsc::Receiver<Report>,
    /// Says, once, why the run must stop now: the server's deadline, or this machine's count of
    /// the run's length when the server cannot be reached.
    stop: Option<std::sync::mpsc::Receiver<String>>,
}

/// The run's own deadline, for when the server cannot say it: its length
/// (`deadline_at − official_start`) counted from when it opened on a clock that keeps running
/// while the machine sleeps.
#[derive(Debug, Clone, Copy)]
pub struct LocalDeadline {
    pub opened: goalcup_protocol::clock::ContinuousOrigin,
    pub length: Duration,
}

impl LocalDeadline {
    /// The run's length from the record's two server times, from `opened`.
    pub fn of(
        record: &UploadRecord,
        opened: goalcup_protocol::clock::ContinuousOrigin,
    ) -> Option<LocalDeadline> {
        let start = goalcup_protocol::control::parse_z(&record.official_start)?;
        let deadline = goalcup_protocol::control::parse_z(&record.deadline_at)?;
        Some(LocalDeadline {
            opened,
            length: Duration::from_secs(deadline.checked_sub(start)?),
        })
    }

    fn passed(&self) -> bool {
        u128::from(self.opened.elapsed_ns()) >= self.length.as_nanos()
    }
}

/// How long past the wrap-up `goalcup start` waits for the uploader's own last word.
const FINISH_GRACE: Duration = Duration::from_secs(10);

/// How often the wait for the uploader's last word looks at the continuous clock again.
const FINISH_SLICE: Duration = Duration::from_secs(5);

/// Start uploading `record` beside the run: the first chunk at once, then on [`Policy`]'s
/// cadence. Its waits and failures are said on stderr, prefixed `goalcup: upload:`; nothing it
/// does can stop or slow the recording.
pub fn begin(
    server: ServerClient,
    record: UploadRecord,
    receipt_key: String,
    policy: Policy,
    spool_dir: PathBuf,
    local_deadline: Option<LocalDeadline>,
) -> Result<Running, CliError> {
    let lock = lock_entry(&spool_dir, &record.entry_id)?;
    let (finish, finishing) = std::sync::mpsc::channel::<WrapUp>();
    let (reported, report) = std::sync::mpsc::channel::<Report>();
    let (stopping, stop) = std::sync::mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("goalcup-upload".into())
        .spawn(move || {
            let _lock = lock;
            let beside = Beside {
                finishing,
                stopping,
                local_deadline,
            };
            let ended = run(server, record, receipt_key, policy, &spool_dir, beside);
            let _ = reported.send(ended);
        })
        .map_err(|source| CliError::Io {
            path: "the upload thread".to_string(),
            source,
        })?;
    Ok(Running {
        finish,
        report,
        stop: Some(stop),
    })
}

/// What the upload thread hears from, and says to, `goalcup start`.
struct Beside {
    /// The run is sealed: finish within this wrap-up.
    finishing: std::sync::mpsc::Receiver<WrapUp>,
    /// The run must stop now, and why.
    stopping: std::sync::mpsc::Sender<String>,
    local_deadline: Option<LocalDeadline>,
}

impl Running {
    /// The channel that says when the run must stop (taken once).
    pub fn take_stop(&mut self) -> Option<std::sync::mpsc::Receiver<String>> {
        self.stop.take()
    }

    /// The run is sealed: send the rest and the seal, asking again after every failure that
    /// does not end the upload, until `wrap_up` has run out, and say how it went. Returns by
    /// then (and a short grace) whatever the server does: an upload still under way is left
    /// to `goalcup upload`.
    ///
    /// The wait is counted on the continuous clock, in slices: a machine that sleeps through
    /// the wrap-up does not owe it again on waking.
    pub fn finish(self, wrap_up: WrapUp) -> Report {
        let _ = self.finish.send(wrap_up);
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
        stopping,
        local_deadline,
    } = beside;
    let mut say = |line: &str| eprintln!("goalcup: {line}");
    let mut uploader: Option<Uploader> = None;
    let mut stopped: Option<String> = None;
    let mut failures = 0u32;
    let mut wait_until = Instant::now();
    let mut next_due = Instant::now();
    let mut cadence = Cadence::new(&policy);
    // Heartbeats and the deadline run on a thread of their own, so a slow chunk upload never
    // delays a stop. Once the run must stop, uploading waits for the finish.
    let over = std::sync::Arc::new(AtomicBool::new(false));
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let _done_on_return = SetOnDrop(done.clone());
    {
        let (server, record, over, done) = (server.clone(), record.clone(), over.clone(), done);
        let _ = std::thread::Builder::new()
            .name("goalcup-heartbeat".into())
            .spawn(move || {
                watch_deadline(
                    &server,
                    &record,
                    policy,
                    local_deadline,
                    &stopping,
                    &over,
                    &done,
                )
            });
    }
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
                        wait_until = now + wait;
                    }
                    Err(error) => {
                        say(&format!(
                            "upload stopped: {error}. The run goes on recording on this machine."
                        ));
                        stopped = Some(error.to_string());
                    }
                }
            }
            if let Some(up) = uploader.as_mut()
                && (now >= next_due || up.waiting() >= policy.eager_bytes)
            {
                match up.attempt(None) {
                    Attempt::CaughtUp => {
                        failures = 0;
                        next_due = cadence.next_due(now);
                    }
                    Attempt::Retry(after, why) => {
                        failures += 1;
                        let wait = backoff(&policy, failures, after);
                        say(&format!(
                            "upload: {why}; trying again in {}s",
                            wait.as_secs().max(1)
                        ));
                        wait_until = now + wait;
                    }
                    Attempt::Stop(why) => {
                        say(&format!(
                            "upload stopped: {why}. The run goes on recording on this machine."
                        ));
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
                        Ok(up) => up,
                        Err(error) => return Report::not(error.to_string(), 0),
                    },
                };
                return up.finish(Some(wrap_up), &mut say);
            }
        }
    }
}
