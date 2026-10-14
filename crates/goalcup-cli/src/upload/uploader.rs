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
//! - **When:** at once when the run launches, then once an hour, or sooner once
//!   [`Policy::eager_bytes`] of chain is waiting, and everything at the end.
//! - **Failures:** no answer, a server error, a rate limit or the maintenance switch is asked
//!   again after a backoff that starts at a minute and doubles to fifteen, with jitter, never
//!   sooner than the server's `Retry-After`. A `409` means the server is somewhere else: its
//!   position is read again and the upload continues from there. Any other refusal stops the
//!   uploader and says why; the run goes on recording.
//! - **Acknowledgements:** each one is checked (this entry, the chunk's last `seq` and head,
//!   a signature under the start receipt's key) and appended to the trace's `acks.jsonl`,
//!   where `goalcup-verify` reads it.

use super::UploadRecord;
use super::api::{ChunkAnswer, RunPosition, SealRequest, StatusAnswer};
use super::http::{ServerClient, ServerError};
use crate::CliError;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::export::{ACKS_FILE, BASELINE_FILE, CHAIN_FILE, SEAL_FILE};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
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
/// How long one chunk upload may take.
const PUT_TIMEOUT: Duration = Duration::from_secs(600);

/// When chunks go, and how failures wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// The regular cadence.
    pub interval: Duration,
    /// How often the chain is looked at.
    pub poll: Duration,
    /// A chunk goes before its hour once this much chain is waiting (about 25 MB compressed).
    pub eager_bytes: u64,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// How long `goalcup start` keeps sending after the seal before it leaves the rest to
    /// `goalcup upload`.
    pub finish_budget: Duration,
    /// The longest one chunk upload may take.
    pub put_timeout: Duration,
}

impl Policy {
    /// A test build's policy from `spec`: `interval,poll,backoff_min,backoff_max,finish`
    /// milliseconds, `eager_bytes`, and optionally the chunk upload timeout in milliseconds,
    /// comma separated. A shipped build has no such door.
    #[cfg(feature = "test-support")]
    pub fn from_test_spec(spec: &str) -> Option<Policy> {
        let n: Vec<u64> = spec
            .split(',')
            .map(|v| v.trim().parse().ok())
            .collect::<Option<_>>()?;
        let (interval, poll, backoff_min, backoff_max, finish, eager, put) = match n[..] {
            [a, b, c, d, e, f] => (a, b, c, d, e, f, 600_000),
            [a, b, c, d, e, f, g] => (a, b, c, d, e, f, g),
            _ => return None,
        };
        let ms = Duration::from_millis;
        Some(Policy {
            interval: ms(interval),
            poll: ms(poll),
            eager_bytes: eager,
            backoff_min: ms(backoff_min),
            backoff_max: ms(backoff_max),
            finish_budget: ms(finish),
            put_timeout: ms(put),
        })
    }
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            interval: Duration::from_secs(3600),
            poll: Duration::from_secs(30),
            eager_bytes: 256 * 1024 * 1024,
            backoff_min: Duration::from_secs(60),
            backoff_max: Duration::from_secs(15 * 60),
            finish_budget: Duration::from_secs(120),
            put_timeout: PUT_TIMEOUT,
        }
    }
}

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
}

/// What is left of `deadline`, or `cap` with none; `None` once it has passed.
fn left(deadline: Option<Instant>, cap: Duration) -> Option<Duration> {
    match deadline {
        None => Some(cap),
        Some(deadline) => {
            let left = deadline.saturating_duration_since(Instant::now());
            (left >= Duration::from_secs(1)).then(|| left.min(cap))
        }
    }
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

    /// Send every complete line, chunk by chunk, once, within `deadline` if one is given.
    fn attempt(&mut self, deadline: Option<Instant>) -> Attempt {
        loop {
            // Only a finish has a deadline; once it has passed, the rest is `goalcup upload`'s.
            let Some(timeout) = left(deadline, self.policy.put_timeout) else {
                return Attempt::Stop("the time left to finish ran out".to_string());
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
        if error.is_retryable() {
            Attempt::Retry(
                error.retry_after().map(Duration::from_secs),
                error.to_string(),
            )
        } else {
            Attempt::Stop(error.to_string())
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

/// The wait before asking again: from `min`, doubling per consecutive failure to `max`, plus
/// up to a quarter more at random so a fleet of tracers does not ask in step, and never
/// less than what the server asked for.
pub fn backoff(policy: &Policy, failures: u32, retry_after: Option<Duration>) -> Duration {
    use rand::Rng;
    let doubled = policy
        .backoff_min
        .saturating_mul(1u32 << failures.saturating_sub(1).min(16))
        .min(policy.backoff_max);
    let jitter = doubled.mul_f64(rand::thread_rng().gen_range(0.0..0.25));
    (doubled + jitter).max(retry_after.unwrap_or_default())
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
    /// Send everything and then the seal, asking again after each retryable failure, all
    /// within `deadline` (or for as long as it takes, with `None`). `say` hears each wait.
    ///
    /// The seal's documents are read before the chain is drained: the daemon writes them
    /// after the chain's last record, so the drain that follows sends every record they seal,
    /// and the server is never sealed on a copy missing its tail.
    pub fn finish(&mut self, deadline: Option<Instant>, say: &mut dyn FnMut(&str)) -> Report {
        let documents = self.seal_documents();
        let mut failures = 0;
        loop {
            match self.attempt(deadline) {
                Attempt::CaughtUp => break,
                Attempt::Stop(why) => return Report::not(why, self.pos.offset),
                Attempt::Retry(after, why) => {
                    failures += 1;
                    let wait = backoff(&self.policy, failures, after);
                    if deadline.is_some_and(|d| Instant::now() + wait > d) {
                        return Report::not(why, self.pos.offset);
                    }
                    say(&format!(
                        "upload: {why}; trying again in {}s",
                        wait.as_secs().max(1)
                    ));
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
        loop {
            let Some(timeout) = left(deadline, super::http::CALL_TIMEOUT) else {
                return caught_up(false, Some("the time left to finish ran out".into()));
            };
            match self.seal(&documents, timeout) {
                Ok(()) => return caught_up(true, None),
                Err(error) if error.is_retryable() => {
                    failures += 1;
                    let wait = backoff(
                        &self.policy,
                        failures,
                        error.retry_after().map(Duration::from_secs),
                    );
                    if deadline.is_some_and(|d| Instant::now() + wait > d) {
                        return caught_up(false, Some(error.to_string()));
                    }
                    say(&format!(
                        "upload: the seal was not stored ({error}); trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    std::thread::sleep(wait);
                }
                Err(error) => return caught_up(false, Some(error.to_string())),
            }
        }
    }
}

/// The uploader running beside `goalcup start`, on a thread of its own.
pub struct Running {
    finish: std::sync::mpsc::Sender<Instant>,
    report: std::sync::mpsc::Receiver<Report>,
    budget: Duration,
}

/// How long past its budget `goalcup start` waits for the uploader's own last word.
const FINISH_GRACE: Duration = Duration::from_secs(10);

/// Start uploading `record` beside the run: the first chunk at once, then on [`Policy`]'s
/// cadence. Its waits and failures are said on stderr, prefixed `goalcup: upload:`; nothing it
/// does can stop or slow the recording.
pub fn begin(
    server: ServerClient,
    record: UploadRecord,
    receipt_key: String,
    policy: Policy,
    spool_dir: PathBuf,
) -> Result<Running, CliError> {
    let lock = lock_entry(&spool_dir, &record.entry_id)?;
    let (finish, finishing) = std::sync::mpsc::channel::<Instant>();
    let (reported, report) = std::sync::mpsc::channel::<Report>();
    std::thread::Builder::new()
        .name("goalcup-upload".into())
        .spawn(move || {
            let _lock = lock;
            let ended = run(server, record, receipt_key, policy, &spool_dir, finishing);
            let _ = reported.send(ended);
        })
        .map_err(|source| CliError::Io {
            path: "the upload thread".to_string(),
            source,
        })?;
    Ok(Running {
        finish,
        report,
        budget: policy.finish_budget,
    })
}

impl Running {
    /// The run is sealed: send the rest and the seal within the policy's finish budget, and
    /// say how it went. Returns by the budget (and a short grace) whatever the server does:
    /// an upload still under way then is left to `goalcup upload`.
    pub fn finish(self) -> Report {
        let _ = self.finish.send(Instant::now() + self.budget);
        self.report
            .recv_timeout(self.budget + FINISH_GRACE)
            .unwrap_or_else(|_| Report::not("the upload did not finish in time", 0))
    }
}

fn run(
    server: ServerClient,
    record: UploadRecord,
    key: String,
    policy: Policy,
    spool_dir: &Path,
    finishing: std::sync::mpsc::Receiver<Instant>,
) -> Report {
    use std::sync::mpsc::RecvTimeoutError;
    let mut say = |line: &str| eprintln!("goalcup: {line}");
    let mut uploader: Option<Uploader> = None;
    let mut stopped: Option<String> = None;
    let mut failures = 0u32;
    let mut wait_until = Instant::now();
    let mut next_due = Instant::now();
    loop {
        let now = Instant::now();
        if stopped.is_none() && now >= wait_until {
            if uploader.is_none() {
                match Uploader::resume(
                    server.clone(),
                    record.clone(),
                    key.clone(),
                    policy,
                    spool_dir,
                ) {
                    Ok(resumed) => uploader = Some(resumed),
                    Err(error) if error.is_retryable() => {
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
                        next_due = now + policy.interval;
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
            Ok(deadline) => {
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
                return up.finish(Some(deadline), &mut say);
            }
        }
    }
}
