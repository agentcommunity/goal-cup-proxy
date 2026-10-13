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

use super::api::{ChunkAnswer, RunPosition, SealRequest, StatusAnswer};
use super::http::{ServerClient, ServerError};
use super::UploadRecord;
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
}

impl Policy {
    /// A test build's policy from `spec`: `interval,poll,backoff_min,backoff_max,finish`
    /// milliseconds and `eager_bytes`, comma separated. A shipped build has no such door.
    #[cfg(feature = "test-support")]
    pub fn from_test_spec(spec: &str) -> Option<Policy> {
        let n: Vec<u64> = spec
            .split(',')
            .map(|v| v.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [interval, poll, backoff_min, backoff_max, finish, eager] = n[..] else {
            return None;
        };
        let ms = Duration::from_millis;
        Some(Policy {
            interval: ms(interval),
            poll: ms(poll),
            eager_bytes: eager,
            backoff_min: ms(backoff_min),
            backoff_max: ms(backoff_max),
            finish_budget: ms(finish),
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

/// The uploader of one entry.
pub struct Uploader {
    server: ServerClient,
    record: UploadRecord,
    /// The key the start receipt was signed with; every acknowledgement must verify under it.
    key: String,
    policy: Policy,
    pos: Position,
    /// The highest `seq` already in the trace's `acks.jsonl`.
    acked: Option<u64>,
    spool: PathBuf,
}

impl Uploader {
    /// An uploader for `record`, at the position the server reports. A newer acknowledgement
    /// the server holds than the trace has (a reply lost in transit) is checked and kept.
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
        let mut uploader = Uploader {
            server,
            acked: last_acked_seq(&Path::new(&record.trace_dir).join(ACKS_FILE)),
            record,
            key: receipt_key,
            policy,
            pos: Position::from(&position),
            spool,
        };
        if let Some(ack) = position.last_ack {
            // A lost reply's acknowledgement: the chunk it names is the server's last.
            let _ = uploader.keep_ack(&ack, position.next_seq.checked_sub(1), &position.head);
        }
        Ok(uploader)
    }

    fn chain(&self) -> PathBuf {
        Path::new(&self.record.trace_dir).join(CHAIN_FILE)
    }

    /// Bytes of chain waiting to be sent (an upper bound).
    pub fn waiting(&self) -> u64 {
        waiting_bytes(&self.chain(), self.pos.offset)
    }

    /// Send every complete line, chunk by chunk, once.
    fn attempt(&mut self) -> Attempt {
        loop {
            let chunk = match build_chunk(&self.chain(), &self.pos, &self.spool) {
                Ok(Some(chunk)) => chunk,
                Ok(None) => return Attempt::CaughtUp,
                Err(error) => return Attempt::Stop(error.to_string()),
            };
            let sent = self.put(&chunk);
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
            Attempt::Retry(error.retry_after().map(Duration::from_secs), error.to_string())
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
        if let Some(ack) = &position.last_ack {
            let _ = self.keep_ack(ack, position.next_seq.checked_sub(1), &position.head);
        }
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

    fn put(&self, chunk: &Chunk) -> Result<ChunkAnswer, ServerError> {
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
            PUT_TIMEOUT,
        )
    }

    /// Check `ack` against the chunk it answers (`seq`, `head`) and keep it in the trace's
    /// `acks.jsonl` unless an acknowledgement at or past its `seq` is already there.
    fn keep_ack(&mut self, ack: &AckCheckpoint, seq: Option<u64>, head: &str) -> Result<(), String> {
        let fits = ack.entry_id == self.record.entry_id
            && Some(ack.seq) == seq
            && ack.hash == head
            && ack.acked_at_secs().is_some();
        if !fits || ack.verify(&self.key).is_err() {
            return Err(format!(
                "the server's acknowledgement of seq {} does not match the chunk it answers, or \
                 is not signed by the key of this run's start receipt",
                ack.seq
            ));
        }
        if self.acked.is_some_and(|acked| acked >= ack.seq) {
            return Ok(());
        }
        append_ack(&Path::new(&self.record.trace_dir).join(ACKS_FILE), ack)
            .map_err(|e| e.to_string())?;
        self.acked = Some(ack.seq);
        Ok(())
    }

    /// `POST /v1/runs/{id}/seal` with the trace's `seal.json` and `baseline.json`, once both
    /// exist: the run is over.
    fn seal(&self) -> Result<bool, ServerError> {
        let trace = Path::new(&self.record.trace_dir);
        let read = |name: &str| read_document(&trace.join(name));
        let (Some(seal_json), Some(baseline_json)) = (read(SEAL_FILE), read(BASELINE_FILE)) else {
            return Ok(false);
        };
        let request = SealRequest {
            entry_id: self.record.entry_id.clone(),
            seal_json,
            baseline_json,
        };
        self.server.json::<StatusAnswer>(
            "POST",
            &format!("/v1/runs/{}/seal", self.record.run_id),
            Some(&request),
        )?;
        Ok(true)
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

/// The `seq` of the last acknowledgement in `acks`, or `None` for none (or none readable).
fn last_acked_seq(acks: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(acks).ok()?;
    text.lines()
        .rev()
        .find_map(|line| serde_json::from_str::<AckCheckpoint>(line).ok())
        .map(|ack| ack.seq)
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

impl Uploader {
    /// Send everything and then the seal, asking again after each retryable failure until
    /// `deadline` (or for as long as it takes, with `None`). `say` hears each wait.
    pub fn finish(&mut self, deadline: Option<Instant>, say: &mut dyn FnMut(&str)) -> Report {
        let mut failures = 0;
        let mut report = |caught_up, sealed, why: Option<String>, offset| Report {
            caught_up,
            sealed,
            why,
            offset,
        };
        loop {
            match self.attempt() {
                Attempt::CaughtUp => break,
                Attempt::Stop(why) => return report(false, false, Some(why), self.pos.offset),
                Attempt::Retry(after, why) => {
                    failures += 1;
                    let wait = backoff(&self.policy, failures, after);
                    if deadline.is_some_and(|d| Instant::now() + wait > d) {
                        return report(false, false, Some(why), self.pos.offset);
                    }
                    say(&format!(
                        "upload: {why}; trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    std::thread::sleep(wait);
                }
            }
        }
        let mut failures = 0;
        loop {
            match self.seal() {
                Ok(sealed) => return report(true, sealed, None, self.pos.offset),
                Err(error) if error.is_retryable() => {
                    failures += 1;
                    let wait = backoff(
                        &self.policy,
                        failures,
                        error.retry_after().map(Duration::from_secs),
                    );
                    if deadline.is_some_and(|d| Instant::now() + wait > d) {
                        return report(true, false, Some(error.to_string()), self.pos.offset);
                    }
                    say(&format!(
                        "upload: the seal was not stored ({error}); trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    std::thread::sleep(wait);
                }
                Err(error) => {
                    return report(true, false, Some(error.to_string()), self.pos.offset);
                }
            }
        }
    }
}

/// The uploader running beside `goalcup start`, on a thread of its own.
pub struct Running {
    finish: std::sync::mpsc::Sender<Instant>,
    thread: std::thread::JoinHandle<Report>,
    budget: Duration,
}

/// Start uploading `record` beside the run: the first chunk at once, then on [`Policy`]'s
/// cadence. Its waits and failures are said on stderr, prefixed `goalcup: upload:`; nothing it
/// does can stop or slow the recording.
pub fn spawn(
    server: ServerClient,
    record: UploadRecord,
    receipt_key: String,
    policy: Policy,
    spool_dir: PathBuf,
) -> Result<Running, CliError> {
    let (finish, finishing) = std::sync::mpsc::channel::<Instant>();
    let thread = std::thread::Builder::new()
        .name("goalcup-upload".into())
        .spawn(move || run(server, record, receipt_key, policy, &spool_dir, finishing))
        .map_err(|source| CliError::Io {
            path: "the upload thread".to_string(),
            source,
        })?;
    Ok(Running {
        finish,
        thread,
        budget: policy.finish_budget,
    })
}

impl Running {
    /// The run is sealed: send the rest and the seal within the policy's finish budget, and
    /// say how it went.
    pub fn finish(self) -> Report {
        let _ = self.finish.send(Instant::now() + self.budget);
        self.thread.join().unwrap_or_else(|_| Report {
            caught_up: false,
            sealed: false,
            why: Some("the upload thread failed".to_string()),
            offset: 0,
        })
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
                match Uploader::resume(server.clone(), record.clone(), key.clone(), policy, spool_dir) {
                    Ok(resumed) => uploader = Some(resumed),
                    Err(error) if error.is_retryable() => {
                        failures += 1;
                        let wait = backoff(&policy, failures, error.retry_after().map(Duration::from_secs));
                        say(&format!("upload: {error}; trying again in {}s", wait.as_secs().max(1)));
                        wait_until = now + wait;
                    }
                    Err(error) => {
                        say(&format!("upload stopped: {error}. The run goes on recording on this machine."));
                        stopped = Some(error.to_string());
                    }
                }
            }
            if let Some(up) = uploader.as_mut()
                && (now >= next_due || up.waiting() >= policy.eager_bytes)
            {
                match up.attempt() {
                    Attempt::CaughtUp => {
                        failures = 0;
                        next_due = now + policy.interval;
                    }
                    Attempt::Retry(after, why) => {
                        failures += 1;
                        let wait = backoff(&policy, failures, after);
                        say(&format!("upload: {why}; trying again in {}s", wait.as_secs().max(1)));
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
        match finishing.recv_timeout(policy.poll) {
            Err(RecvTimeoutError::Timeout) => continue,
            // `goalcup start` ended without a finish (a refusal after the launch): stop here.
            Err(RecvTimeoutError::Disconnected) => {
                return Report {
                    caught_up: false,
                    sealed: false,
                    why: Some("the run ended before its upload finished".to_string()),
                    offset: uploader.as_ref().map(|u| u.pos.offset).unwrap_or(0),
                };
            }
            Ok(deadline) => {
                if let Some(why) = stopped {
                    return Report {
                        caught_up: false,
                        sealed: false,
                        why: Some(why),
                        offset: uploader.as_ref().map(|u| u.pos.offset).unwrap_or(0),
                    };
                }
                let mut up = match uploader {
                    Some(up) => up,
                    None => match Uploader::resume(server, record, key, policy, spool_dir) {
                        Ok(up) => up,
                        Err(error) => {
                            return Report {
                                caught_up: false,
                                sealed: false,
                                why: Some(error.to_string()),
                                offset: 0,
                            };
                        }
                    },
                };
                return up.finish(Some(deadline), &mut say);
            }
        }
    }
}
