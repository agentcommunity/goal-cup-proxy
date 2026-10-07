//! The uploader: signed chunks out, verified acknowledgements in (V12).
//!
//! This is the module that turns a recorder into an anchored one. Everything else in the
//! evidence — the chain, the buffer, the encrypted spool — sits on the participant's own
//! machine under the participant's own account; the acknowledgements do not. So there are
//! exactly two cryptographic obligations here, and they run in opposite directions:
//!
//!   * **the daemon signs, and never verifies its own work.** Each chunk is signed with the
//!     entry's run key over [`ChunkClaims`]. `Chunk::verify_with` is the *server's* job — a
//!     document that names the key that checks it checks nothing — so this module calls it
//!     nowhere.
//!   * **the daemon verifies, and never believes.** Every `AckCheckpoint` that comes back is
//!     checked against the server key this build **pinned** before anything is written or
//!     advanced. Storing whatever arrived would let a hostile or broken server hand a
//!     participant an anchor for a chain it never saw. A failure here is
//!     [`PumpOutcome::ack_unverified`]: nothing appended, nothing truncated, and a health fact
//!     rather than a silent success.
//!
//! **`chunk_id` is derived, never chosen.** Claims are built with `ChunkClaims::new`, which
//! computes the id from `(entry_id, first_seq, last_seq, head)`. That is what makes a
//! re-upload idempotent — the same range is the same document, and a server that already
//! stored it answers the same acknowledgement — and it is why no struct literal of that type
//! is written anywhere in this crate: supplying an id would only ever be a way for it to be
//! wrong.
//!
//! **The range is the buffer's, not this module's.** [`Buffer::read_from`] resumes at
//! `floor() + 1` and stops at the first hole, which is exactly the contract the receiving
//! server enforces: a chunk starts at `last_acked + 1`, carries an unbroken range, and is
//! refused *permanently* otherwise. So the uploader never re-sends below the watermark —
//! even when a caller's [`Uploader::resume`] asks it to, which is clamped rather than
//! obeyed.
//!
//! **A hole is asked of the chain once, and is otherwise a fact**. Every chaining
//! call appends to `chain.jsonl` before the buffer, so `kill -9` between the two leaves a
//! seq the chain holds and the buffer does not — and `read_from` stops there forever,
//! capping the whole 168-hour run at `gap_at - 1`. The chain is the authority on what was
//! recorded, so [`Uploader::refill_gap`] streams that one line back into the buffer, once
//! per observed hole. What that cannot answer for stays `PumpOutcome::gap_at`: a health
//! fact of its own, because `HealthState::UploadLagging`'s words are "the upload is behind
//! … nothing is lost", which is actively false about a hole.
//!
//! **Recovery is a state, not an error** (D9). A closed laptop is the normal path, so an
//! unreachable server is [`PumpOutcome::offline`] and not a `DaemonError`; the next attempt
//! waits out a backoff that doubles to five minutes with ±25% jitter, so a fleet coming back
//! from the same outage does not retry in lockstep. [`PumpOutcome::backing_off`] is reported
//! separately from `offline` because "we did not try" and "we tried and could not reach it"
//! are different facts, and only one of them is evidence about the network. The first pump
//! that succeeds after a streak carries the sentence `back online, N behind`.
//!
//! **A refusal is not an outage.** The server refuses a chunk it cannot place — a range it
//! has moved past, a chain that does not continue the one it acknowledged, an entry with no
//! receipt — and it refuses the same bytes forever, so re-sending them is an infinite loop
//! with a network in it. A refusal therefore backs off like an outage *and* drops any
//! in-memory resume position, so the next attempt re-derives its start from the durable
//! watermark: the last acknowledgement this daemon actually verified. A caller that learns
//! the server's position by some other route says so with [`Uploader::resume`].
//!
//! **Clock skew is reported and never corrected.** The local clock belongs to the
//! participant; the server's `acked_at` is the only external time reference either side has,
//! and it is inside the acknowledgement's signature. The distance between them is a number
//! for the health surface, not an adjustment to make.
//!
//! **Bounded memory**: one chunk in flight, at most `chunk_max`
//! envelopes in it, whatever the length of the outage behind it.

use crate::buffer::Buffer;
use crate::chain_writer::ChainWriter;
use crate::receipt::{NETWORK_TIMEOUT, post, split_url};
use crate::registry::RunKeys;
use crate::{DaemonError, now_unix};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chunk::{Chunk, ChunkClaims};
use goalcup_protocol::envelope::Envelope;
use goalcup_protocol::export::{CHAIN_FILE, TraceExport};
use std::path::Path;

/// The two trace operations an uploader needs. A live writer can refill a torn buffer gap;
/// a sealed drain uses the exact retained terminal export and only appends acknowledgements.
/// Keeping this boundary small means terminal delivery never needs to parse or compile an
/// untrusted/current connector manifest merely to obtain a redactor it will not use.
pub trait UploadReader: std::io::Read + std::io::Seek + Send {}

impl<T: std::io::Read + std::io::Seek + Send> UploadReader for T {}

pub trait UploadTrace {
    fn root(&self) -> &Path;
    fn independent_chain_reader(&mut self) -> Result<Box<dyn UploadReader>, DaemonError>;
    fn append_ack(&mut self, ack: &AckCheckpoint) -> Result<(), DaemonError>;
}

impl UploadTrace for ChainWriter {
    fn root(&self) -> &Path {
        self.root()
    }

    fn independent_chain_reader(&mut self) -> Result<Box<dyn UploadReader>, DaemonError> {
        Ok(Box::new(self.independent_chain_reader()?))
    }

    fn append_ack(&mut self, ack: &AckCheckpoint) -> Result<(), DaemonError> {
        self.append_ack(ack)
    }
}

impl UploadTrace for TraceExport {
    fn root(&self) -> &Path {
        self.root()
    }

    fn independent_chain_reader(&mut self) -> Result<Box<dyn UploadReader>, DaemonError> {
        Ok(Box::new(self.independent_chain_reader()?))
    }

    fn append_ack(&mut self, ack: &AckCheckpoint) -> Result<(), DaemonError> {
        Ok(self.append_ack(ack)?)
    }
}

/// The first delay after a failure, in seconds. Every later one is this doubled.
pub const BACKOFF_BASE_SECONDS: f64 = 1.0;

/// Five minutes, and no longer. A daemon that backed off further would stop being a
/// recorder whose lag is visible and start being one that is quietly asleep.
pub const BACKOFF_CAP_SECONDS: f64 = 300.0;

/// ±25%, so a fleet that lost the same server does not come back in lockstep.
pub const BACKOFF_JITTER: f64 = 0.25;

/// The most characters of a server's refusal that are carried into an error.
///
/// A refusal is a sentence a participant has to act on, and a body is otherwise whatever the
/// peer felt like sending — including a megabyte of it into a health surface.
const MAX_REFUSAL_CHARS: usize = 512;

/// Why one upload did not produce an acknowledgement.
///
/// Two variants, and the distinction is the whole of the recovery policy. [`Self::Offline`]
/// is "no answer came back from a server": nothing reached it, or it said it was
/// unavailable. Retrying the same bytes later is the right move. [`Self::Rejected`] is "a
/// server read this and would not have it", and the same bytes will be refused again for as
/// long as the server holds its position, so a retry is a loop rather than a recovery.
///
/// Neither ever carries key material. The request body holds a public key and a signature —
/// neither secret — but this crate holds the per-run secrets in the same
/// process, and an error type that can quote a request is one that will one day quote the
/// wrong one.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// Nothing was acknowledged because nothing answered: a socket that would not connect,
    /// a peer that went silent inside the timeout, a server reporting itself unavailable —
    /// and also the two ways a request cannot be *made* at all, an upload url this build
    /// refuses to speak cleartext to and a chunk that would not serialise. All of them are
    /// "no request reached a server", which is the fact the retry policy turns on.
    #[error("offline: {0}")]
    Offline(String),
    /// A server answered, and the answer was not an acknowledgement.
    #[error("the upload server answered {status}: {body}")]
    Rejected { status: u16, body: String },
}

/// The HTTP client for one entry's chunks.
///
/// Deliberately the same transport as [`crate::receipt::ReceiptClient`], through the same
/// two functions: `split_url` decides what this build will speak to, and `post` is the one
/// bounded HTTP/1.1 request in this crate. Loopback cleartext or refused (nothing here
/// terminates TLS), ten seconds on connect, read and write, and no logging anywhere on the
/// path. A second copy of those rules is how the receipt request and the upload request
/// drift into having different ones.
#[derive(Debug)]
pub struct UploadClient {
    upload_url: String,
    timeout: std::time::Duration,
}

impl UploadClient {
    pub fn new(base_url: impl Into<String>) -> UploadClient {
        UploadClient::with_timeout(base_url, NETWORK_TIMEOUT)
    }

    /// The same client on a shorter clock, for a test that proves the timeout without
    /// waiting out the shipped one.
    pub fn with_timeout(base_url: impl Into<String>, timeout: std::time::Duration) -> UploadClient {
        UploadClient {
            upload_url: base_url.into(),
            timeout,
        }
    }

    /// `POST /v0/entries/{id}/chunks`, and the acknowledgement it answered with — **not yet
    /// verified**. Verification is [`Uploader::pump`]'s, against the key the config pinned,
    /// because a client that verified with a key it was handed beside the document would be
    /// checking nothing.
    ///
    /// 503 is [`UploadError::Offline`] and not a rejection: the server said it is
    /// unavailable, which is a sentence about now and not about these bytes. A 200 whose
    /// body is not an acknowledgement is a rejection, because the server answered and the
    /// answer is unusable.
    pub async fn send(&self, chunk: &Chunk) -> Result<AckCheckpoint, UploadError> {
        let body = serde_json::to_string(chunk)
            .map_err(|e| UploadError::Offline(format!("the chunk was not put on the wire: {e}")))?;
        let (authority, prefix) =
            split_url(&self.upload_url).map_err(|e| UploadError::Offline(e.to_string()))?;
        let path = format!("{prefix}/v0/entries/{}/chunks", chunk.claims.entry_id);
        let timeout = self.timeout;
        let sent = tokio::task::spawn_blocking(move || post(&authority, &path, &body, timeout))
            .await
            .map_err(|e| UploadError::Offline(format!("the upload did not run: {e}")))?;
        let (status, text) = sent.map_err(|e| UploadError::Offline(e.to_string()))?;
        if status == 503 {
            return Err(UploadError::Offline(clip(&text)));
        }
        if status != 200 {
            return Err(UploadError::Rejected {
                status,
                body: clip(&text),
            });
        }
        serde_json::from_str(&text).map_err(|e| UploadError::Rejected {
            status,
            body: format!("the acknowledgement did not parse: {e}"),
        })
    }
}

/// What one [`Uploader::pump`] did, and what the health surface should say about it.
///
/// Every field is a fact rather than a judgement (D3): how many envelopes were acknowledged,
/// where the anchor is, whether the server was reachable, whether this attempt was even
/// made. Nothing here decides what a participant's run is worth.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PumpOutcome {
    /// Envelopes this pump got a **verified** acknowledgement for. Zero on every failure
    /// path, including one where a chunk was sent and the answer was not believed.
    pub uploaded: u64,
    /// The highest sequence this entry holds a verified acknowledgement for, after this
    /// pump. Unchanged by a failure, which is the point: a forged answer does not move the
    /// anchor.
    pub acked_seq: Option<u64>,
    /// The first success after a failure streak. Carries the catch-up sentence in `message`.
    pub back_online: bool,
    /// A request was made and no server answered it.
    pub offline: bool,
    /// No request was made, because the backoff from an earlier failure has not run out.
    /// Reported separately from `offline` because "we did not try" is not evidence about
    /// the network.
    pub backing_off: bool,
    /// The status a server refused with, when it refused. `None` when no server answered
    /// and `None` when one acknowledged: a refusal is neither an outage nor a success.
    pub rejected: Option<u16>,
    /// One sentence for the participant: what came back, or how far behind the catch-up is.
    pub message: Option<String>,
    /// `acked_at` minus local time, when the two differ by more than the configured
    /// threshold. Reported, never corrected — the local clock belongs to the participant.
    pub clock_skew_seconds: Option<f64>,
    /// The server answered with an acknowledgement whose signature did not verify against
    /// the pinned key. Not stored, not chained, and a health detail rather than a silent
    /// success.
    pub ack_unverified: bool,
    /// The acknowledgement's **signature was good** and its contents were not: it named a
    /// different entry, or a sequence and chain hash other than the ones this chunk carried.
    /// A replayed acknowledgement for an earlier range looks exactly like this, and so does
    /// a server that has lost its place.
    ///
    /// Kept apart from [`Self::ack_unverified`] because they are different accusations —
    /// "this is not the server's" against "this is the server's and it is about something
    /// else" — and apart from [`Self::offline`] because a status surface that could not tell
    /// them apart would report a server answering the wrong questions as bad wifi. The
    /// handling is the same either way: nothing appended, nothing truncated, and a backoff.
    pub ack_mismatched: bool,
    /// The first sequence the buffer wanted and did not find, when something above it is
    /// buffered — as it stood after [`Uploader::refill_gap`] tried the chain.
    ///
    /// `None` when the refill closed it, which is the ordinary `kill -9` case: the chain
    /// and the buffer are written in that order, so a process killed between the two
    /// leaves a seq the chain holds and the buffer does not. Still `Some` when the chain
    /// could not answer either, and that is the fact the run is capped by: it reaches the
    /// health surface as `buffer_gap_at` rather than as `UploadLagging`, whose sentence
    /// ("the upload is behind … nothing is lost") is a lie about a hole.
    pub gap_at: Option<u64>,
    /// The refill put this many envelopes back into the buffer off the chain, this pump.
    /// Zero on every pump that met no hole, and zero on one whose hole the chain could not
    /// answer for.
    pub refilled: u64,
}

/// One entry's upload loop, one chunk at a time.
///
/// Holds the entry's run key (to sign with) and the pinned server key (to check answers
/// against), and nothing else durable: the position it resumes from is the buffer's
/// watermark on disk, not a number this struct remembers across a restart.
#[derive(Debug)]
pub struct Uploader {
    entry_id: String,
    client: UploadClient,
    chunk_max: usize,
    run_keys: RunKeys,
    pinned_server_key: String,
    /// A caller's resume position, from [`Uploader::resume`]. `None` — the normal case —
    /// means "wherever the buffer is up to". Clamped up to the buffer's own watermark on
    /// every pump, so it can move the start forward and never backward.
    start_after: Option<u64>,
    /// The seq the last refill was attempted at, so one hole costs one read of the chain
    /// and not one per pump.
    ///
    /// A hole the chain cannot answer for is permanent — the record was never written —
    /// and a pump that re-read a chain reaching tens of gigabytes every upload interval
    /// for the rest of a 168-hour run would be paying that price forever to learn the same
    /// thing. Keyed by seq rather than a flag, so a second hole further up is refilled too.
    refilled_at: Option<u64>,
    /// The byte offset in `chain.jsonl` the last refill scan reached, so the next one
    /// resumes there.
    ///
    /// Forward-only, and sound because the chain is append-only and its seqs are monotonic:
    /// a hole at a higher seq than the last one searched for is further into the file than
    /// the offset that search stopped at. Without it a run that met a hole every few hours
    /// would re-read the whole chain each time, on the cadence's own thread.
    ///
    /// In memory only. A restart re-reads from zero, which costs one scan and is the honest
    /// starting point for a process that has not read the file at all.
    scanned_to: u64,
    /// Test-only: the bytes every refill scan on this uploader has read, which is the only
    /// externally visible difference between resuming and re-reading.
    #[cfg(any(test, feature = "test-hooks"))]
    bytes_scanned: u64,
    last_ack: Option<AckCheckpoint>,
    /// Consecutive failures. The backoff's exponent, and what makes the next success a
    /// `back_online` rather than an ordinary pump.
    failures: u32,
    /// The instant before which no request is made, as `now_unix()` reads it.
    retry_after: Option<f64>,
    clock_skew_threshold: f64,
}

impl Uploader {
    /// `run_keys` because the per-run signature is over every chunk,
    /// `pinned_server_key` because an unverified acknowledgement anchors nothing, and
    /// `clock_skew_seconds` because it is how far the server's `acked_at` may differ from
    /// local time before it is reported: `DaemonConfig::clock_skew_seconds`, handed in rather
    /// than read here, since this module owns no configuration.
    pub fn new(
        entry_id: impl Into<String>,
        client: UploadClient,
        chunk_max: usize,
        run_keys: RunKeys,
        pinned_server_key: String,
        clock_skew_seconds: f64,
    ) -> Uploader {
        Uploader {
            entry_id: entry_id.into(),
            client,
            chunk_max,
            run_keys,
            pinned_server_key,
            start_after: None,
            refilled_at: None,
            scanned_to: 0,
            #[cfg(any(test, feature = "test-hooks"))]
            bytes_scanned: 0,
            last_ack: None,
            failures: 0,
            retry_after: None,
            clock_skew_threshold: clock_skew_seconds,
        }
    }

    /// Say where the server is, and try again now.
    ///
    /// This is the resync path. `Some(n)` names the last sequence the server acknowledged —
    /// which the next chunk starts one above — and `None` means "wherever the buffer's own
    /// watermark is", which is what a restart and a refusal both fall back to. Either way
    /// the outstanding backoff is dropped, because new information about the server's
    /// position is a reason to attempt rather than to keep waiting; the failure streak is
    /// **not** dropped, so the attempt that succeeds still reports itself as the moment the
    /// run came back online.
    ///
    /// **A position the buffer disagrees with is not obeyed, in either direction.** Below
    /// the watermark, the server refuses a range it has already acknowledged and refuses it
    /// permanently, so a caller who asked for one would be asking for a dead upload rather
    /// than a slow one. Above it, the start would step over envelopes this daemon holds no
    /// acknowledgement for, and the [`Buffer::ack`] that followed a success would delete
    /// them unread. Since both bounds are the watermark, `Some(n)` changes what is sent only
    /// on a buffer that has acknowledged nothing yet; on any other, the effect that remains
    /// is the one every caller of this method is really after, which is *try again now*.
    pub fn resume(&mut self, last_ack_seq: Option<u64>) {
        self.start_after = last_ack_seq;
        self.retry_after = None;
    }

    /// The last acknowledgement this uploader **verified**. Never one that failed the pinned
    /// key, which is why the accessor is worth having: it is the anchor, not the last thing
    /// a server said.
    pub fn last_ack(&self) -> Option<&AckCheckpoint> {
        self.last_ack.as_ref()
    }

    /// Seconds before the next attempt is allowed. `0.0` when the next pump will try
    /// immediately, which is the state a success and a [`Uploader::resume`] both leave
    /// behind.
    pub fn backoff_seconds(&self) -> f64 {
        match self.retry_after {
            Some(at) => (at - now_unix()).max(0.0),
            None => 0.0,
        }
    }

    /// Read one chunk from the buffer, sign it, send it, verify what comes back, and only
    /// then write the acknowledgement into the trace and truncate the buffer behind it.
    ///
    /// The order is the whole safety argument and it does not have a shortcut in it:
    ///
    ///  1. read at most `chunk_max` **contiguous** envelopes from `floor() + 1`, so a hole
    ///     is reported by every pump and not only by the ones that made a request;
    ///  2. wait out any backoff, so a refused or unreachable server is not hammered;
    ///  3. build claims with `ChunkClaims::new`, so the id is derived from the range;
    ///  4. sign with the run key — the key never leaves this process any other way;
    ///  5. `AckCheckpoint::verify(pinned)`, and stop here if it fails;
    ///  6. check the acknowledgement describes the range that was actually sent;
    ///  7. `ChainWriter::append_ack`, then `Buffer::ack`.
    ///
    /// Steps 7 in that order: if the append fails, the buffer keeps the envelopes and the
    /// range is offered again, which the server answers idempotently. The reverse order
    /// would truncate a range whose anchor never reached `acks.jsonl`. Either of those two
    /// failing arms the backoff before it returns, because "the disk will not take it" is a
    /// condition that persists across ticks and re-offering the range on the next one is a
    /// hot loop the server would answer politely forever.
    ///
    /// A failure to reach the server is not a `DaemonError` — a closed laptop is the normal
    /// path (D9). The `Err` arm of this function is for the two things that really are
    /// broken: a buffer or a trace directory that will not answer, and a run key that does
    /// not decode.
    pub async fn pump<T: UploadTrace>(
        &mut self,
        buffer: &Buffer,
        writer: &mut T,
    ) -> Result<PumpOutcome, DaemonError> {
        self.pump_through(buffer, writer, None).await
    }

    /// Pump only through a durable terminal sequence when one exists.  A live runtime may
    /// still own its upload state after it seals, so enforcing the ceiling only in the
    /// restart drain would leave an in-process post-terminal buffer tail sendable.
    pub async fn pump_through<T: UploadTrace>(
        &mut self,
        buffer: &Buffer,
        writer: &mut T,
        terminal_seq: Option<u64>,
    ) -> Result<PumpOutcome, DaemonError> {
        // The durable buffer watermark is an acknowledgement authority too.  A corrupt
        // value above the terminal would otherwise make a restart report that impossible
        // sequence as acknowledged without constructing or sending another chunk.
        if let Some(terminal) = terminal_seq
            && buffer.floor()?.is_some_and(|floor| floor > terminal)
        {
            return Err(DaemonError::Refused(format!(
                "buffer acknowledgement watermark exceeds terminal seq {terminal}"
            )));
        }
        let mut out = PumpOutcome {
            acked_seq: self.anchor(buffer)?,
            ..PumpOutcome::default()
        };
        if let Some(terminal) = terminal_seq
            && out.acked_seq.is_some_and(|acked| acked > terminal)
        {
            return Err(DaemonError::Refused(format!(
                "upload acknowledgement exceeds terminal seq {terminal}"
            )));
        }

        // The buffer is read before the backoff gate, not after it, because `gap_at` is a
        // fact about this machine's own write-ahead log and not about the network. A pump
        // that skipped the read while it was waiting would report the hole, then not report
        // it, then report it again, and a health surface polling through an outage would
        // watch a permanent condition blink.
        //
        // The start is the durable watermark whenever there is one. Never *below* it,
        // because the server refuses an already-acknowledged range and refuses it for the
        // rest of the run; and never *above* it, because a start above the watermark steps
        // over envelopes this daemon holds no acknowledgement for, and the `Buffer::ack`
        // that followed a success would delete them unread — a hole punched in the run by
        // the resync path that exists to repair one. Both bounds are the same number, so a
        // caller's `resume` position takes effect only on a buffer that has acknowledged
        // nothing yet.
        let floor = buffer.floor()?;
        let after = match floor {
            Some(watermark) => Some(watermark),
            None => self.start_after,
        };
        let mut read = buffer.read_from_through(after, self.chunk_max, terminal_seq)?;
        // **The hole, once, off the chain**. Every chaining call in this crate
        // appends to `chain.jsonl` and then to the buffer, and `export.rs` does not fsync,
        // so an ordinary `kill -9` of the daemon leaves a seq the chain holds and the
        // buffer does not. `read_from` stops at that seq forever and the run is capped at
        // `gap_at - 1`: every record for the rest of the 168 hours is unanchored.
        //
        // The chain is the authority on what was recorded — the buffer is a write-ahead
        // spool in front of it, encrypted for confidentiality and nothing else — so the
        // line is read back and put where the buffer lost it. This adds no trust: the
        // devserver re-hashes and re-links every envelope it accepts, so a refilled record
        // that is not the chain's is refused rather than anchored.
        if let Some(gap) = read.gap_at
            && self.refilled_at != Some(gap)
        {
            self.refilled_at = Some(gap);
            out.refilled = self.refill_gap(buffer, writer, gap).await?;
            if out.refilled > 0 {
                read = buffer.read_from_through(after, self.chunk_max, terminal_seq)?;
            }
        }
        out.gap_at = read.gap_at;

        let waiting = self.backoff_seconds();
        if waiting > 0.0 {
            out.backing_off = true;
            out.message = Some(format!(
                "waiting {waiting:.0}s before the next upload attempt"
            ));
            return Ok(out);
        }

        let Some(last) = read.envelopes.last() else {
            // Nothing to send. Not a success and not a failure: the run is simply caught up,
            // or the only thing above the watermark is on the far side of a hole.
            return Ok(out);
        };

        let first_seq = read.envelopes[0].link.seq;
        let last_seq = last.link.seq;
        let head = last.link.hash.clone();
        let behind = buffer.pending(after)?;
        let claims = ChunkClaims::new(&self.entry_id, first_seq, last_seq, &head);
        let count = read.envelopes.len() as u64;
        let chunk = Chunk::sign(claims, read.envelopes, &self.run_keys.signing_key()?)?;

        let ack = match self.client.send(&chunk).await {
            Ok(ack) => ack,
            Err(UploadError::Offline(why)) => {
                self.fail();
                out.offline = true;
                out.message = Some(why);
                return Ok(out);
            }
            Err(UploadError::Rejected { status, body }) => {
                self.fail();
                // The bytes are refused, and will be refused again. Drop any caller-supplied
                // resume position so the next attempt re-derives its start from the durable
                // watermark — the last acknowledgement this daemon actually verified.
                self.start_after = None;
                out.rejected = Some(status);
                out.message = Some(format!("the upload server refused {status}: {body}"));
                return Ok(out);
            }
        };

        if let Err(e) = ack.verify(&self.pinned_server_key) {
            self.fail();
            out.ack_unverified = true;
            out.message = Some(format!(
                "the acknowledgement for seqs {first_seq}..={last_seq} is not signed by the \
                 key this build pins, so it anchors nothing: {e}"
            ));
            return Ok(out);
        }
        if ack.entry_id != self.entry_id || ack.seq != last_seq || ack.hash != head {
            self.fail();
            out.ack_mismatched = true;
            out.message = Some(format!(
                "the acknowledgement names entry {} at seq {}, and this chunk carried entry \
                 {} through seq {last_seq}",
                ack.entry_id, ack.seq, self.entry_id
            ));
            return Ok(out);
        }
        if let Some(terminal) = terminal_seq
            && ack.seq > terminal
        {
            self.fail();
            return Err(DaemonError::Refused(format!(
                "upload acknowledgement exceeds terminal seq {terminal}"
            )));
        }

        // Both of these are real errors and leave this function through `Err` — but they
        // arm the backoff on the way out, because the conditions behind them (a full disk,
        // a read-only artifact root, a buffer that will not commit) do not clear inside one
        // tick. Without that, a caller looping on `pump` would re-sign and re-post the same
        // chunk every tick for as long as the disk stayed full: the server answers each one
        // idempotently, so nothing would break loudly and nothing would stop.
        if let Err(e) = writer.append_ack(&ack) {
            self.fail();
            return Err(e);
        }
        if let Err(e) = buffer.ack(ack.seq) {
            self.fail();
            return Err(e);
        }
        out.clock_skew_seconds = self.skew(&ack);
        out.uploaded = count;
        out.acked_seq = Some(ack.seq);
        if self.failures > 0 {
            out.back_online = true;
            out.message = Some(format!("back online, {behind} behind"));
        }
        self.failures = 0;
        self.retry_after = None;
        self.start_after = None;
        self.last_ack = Some(ack);
        Ok(out)
    }

    /// Put the envelope at `gap` back into the buffer off `chain.jsonl`.
    ///
    /// **Streamed, one line at a time.** A chain reaches tens of gigabytes over 168 hours
    /// and daemon memory is bounded, so this is a `BufReader` line loop that
    /// stops at the first link whose seq matches — never a `read_to_string`. Every reader
    /// of a whole chain in this workspace holds one line: `chain_line_at`, rebuilt-entry
    /// validation, and protocol resume preparation are the others.
    ///
    /// **One seq, not a range.** The hole this repairs is a torn write at the tail: the
    /// chain took a link and the process died before the buffer commit. Refilling the one
    /// seq is enough for `read_from` to step past it on the re-read, and the next hole — if
    /// the crash tore more than one — is the next pump's, at its own seq, once.
    ///
    /// A chain that has no such link is the other shape of the same fact: the record was
    /// never recorded anywhere, `gap_at` survives the refill, and the health surface says
    /// so. Not an error: a missing line is data about this run, and a pump that returned
    /// `Err` over it would stop the upload of everything below the hole as well.
    ///
    /// **Off the cadence's own thread, and forward-only.** The scan is a synchronous walk of a
    /// file that reaches tens of gigabytes, so it runs on [`tokio::task::spawn_blocking`]
    /// rather than inside `pump` on the task the four cadences share: otherwise one hole would
    /// stop the poll, the status and the deadline check for as long as the read took. The
    /// byte offset it reached is kept, so a later hole resumes where the last one stopped
    /// rather than re-reading the whole file. That is sound because the chain is append-only
    /// and its seqs are monotonic: a seq above the last one searched for cannot be behind the
    /// offset that search reached.
    ///
    /// A join failure — a panicked or cancelled blocking worker — is `Ok(0)`, the same
    /// answer as a chain that had no such line: the hole survives as `gap_at` and the next
    /// hole gets its own attempt.
    async fn refill_gap<T: UploadTrace>(
        &mut self,
        buffer: &Buffer,
        writer: &mut T,
        gap: u64,
    ) -> Result<u64, DaemonError> {
        let path = writer.root().join(CHAIN_FILE);
        let reader = writer.independent_chain_reader()?;
        let from = self.scanned_to;
        let Ok(scanned) =
            tokio::task::spawn_blocking(move || scan_chain(reader, &path, gap, from)).await
        else {
            return Ok(0);
        };
        let (found, to) = scanned?;
        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.bytes_scanned = self.bytes_scanned.saturating_add(to.saturating_sub(from));
        }
        self.scanned_to = to;
        match found {
            Some(envelope) => {
                buffer.append(&envelope)?;
                Ok(1)
            }
            None => Ok(0),
        }
    }

    /// Test-only: how many bytes of `chain.jsonl` every refill on this uploader has read.
    ///
    /// The forward-only offset is invisible from the outside — a refill that re-read the
    /// whole file every time returns exactly the same envelope — so this is the only way to
    /// assert it. Behind the same off-by-default `test-hooks` feature the chain writer's
    /// fault injection uses, so no shipped build carries the counter.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn chain_bytes_scanned(&self) -> u64 {
        self.bytes_scanned
    }

    /// The highest sequence this entry holds a verified acknowledgement for: this process's
    /// own, or — across a restart, where the in-memory one is gone — the buffer's durable
    /// watermark, which only ever moved because an acknowledgement verified.
    fn anchor(&self, buffer: &Buffer) -> Result<Option<u64>, DaemonError> {
        match self.last_ack.as_ref() {
            Some(ack) => Ok(Some(ack.seq)),
            None => buffer.floor(),
        }
    }

    /// One more consecutive failure, and the delay before the next attempt.
    fn fail(&mut self) {
        self.failures = self.failures.saturating_add(1);
        self.retry_after = Some(now_unix() + backoff_delay(self.failures));
    }

    /// The server's `acked_at` minus local time, when the two differ by more than the
    /// threshold. `None` when they agree closely enough, and `None` when `acked_at` does not
    /// parse — an unreadable timestamp is reported as unknown by whoever reads the
    /// acknowledgement, never as a skew of zero.
    fn skew(&self, ack: &AckCheckpoint) -> Option<f64> {
        let acked = ack.acked_at_secs()? as f64;
        let skew = acked - now_unix();
        (skew.abs() > self.clock_skew_threshold).then_some(skew)
    }
}

/// Read `chain.jsonl` forward from byte `from` looking for the link at `gap`, and say where
/// the reading stopped.
///
/// Free-standing and synchronous, because it runs on a blocking worker and so may borrow
/// nothing from the [`Uploader`]. It returns `(the envelope or None, the byte offset
/// reached)`, and the offset is always at a line boundary — the next scan may start there
/// without landing mid-record.
///
/// **Streamed, one line at a time.** A chain reaches tens of gigabytes over 168 hours and
/// daemon memory is bounded, so this is a `BufReader` line loop and never a
/// `read_to_string`. At most one line is held.
///
/// A `from` past the end of the file — which nothing append-only should produce, and a
/// truncated or replaced chain would — restarts at zero rather than reading nothing
/// forever.
fn scan_chain(
    mut file: impl std::io::Read + std::io::Seek,
    diagnostic_path: &std::path::Path,
    gap: u64,
    from: u64,
) -> Result<(Option<Envelope>, u64), DaemonError> {
    use std::io::{BufRead, SeekFrom};
    let io = |source: std::io::Error| DaemonError::Io {
        path: diagnostic_path.display().to_string(),
        source,
    };
    let length = file.seek(SeekFrom::End(0)).map_err(io)?;
    let start = if from <= length { from } else { 0 };
    file.seek(SeekFrom::Start(start)).map_err(io)?;
    let mut at = start;
    let mut reader = std::io::BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok((None, at)),
            Err(source) => return Err(io(source)),
            Ok(read) => {
                let read = u64::try_from(read).map_err(|_| {
                    DaemonError::Refused("the chain scan byte count does not fit u64".to_string())
                })?;
                at = at.checked_add(read).ok_or_else(|| {
                    DaemonError::Refused("the chain scan byte offset overflowed u64".to_string())
                })?;
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        // A line this cannot parse is one line, skipped: the chain is the authority on what
        // it holds and a reader that refused the whole file over one bad line would turn a
        // torn tail into a dead upload.
        let Ok(envelope) = serde_json::from_str::<Envelope>(&line) else {
            continue;
        };
        if envelope.link.seq == gap {
            return Ok((Some(envelope), at));
        }
    }
}

/// The delay after `failures` consecutive failures: `1s`, doubling, capped at five minutes,
/// then jittered ±25% and clamped back under the cap.
///
/// Jitter after the cap rather than before it, so the cap is a real ceiling and not a
/// ceiling plus a quarter — and so a fleet that all reached the cap during the same outage
/// still comes back spread out rather than together, which is the only reason the jitter is
/// there.
fn backoff_delay(failures: u32) -> f64 {
    // 2^32 seconds is longer than the cap by every order of magnitude that matters; the
    // clamp is what keeps `powi` from reaching infinity on a very long outage.
    let steps = failures.saturating_sub(1).min(32) as i32;
    let base = (BACKOFF_BASE_SECONDS * 2f64.powi(steps)).min(BACKOFF_CAP_SECONDS);
    (base * (1.0 + BACKOFF_JITTER * unit_noise())).clamp(0.0, BACKOFF_CAP_SECONDS)
}

/// A number in `[-1, 1]` from the operating system's entropy source. `OsRng` because it is
/// the one this crate already draws its run secrets from; a second generator seeded from the
/// clock would give a fleet that lost the same server the same jitter.
fn unit_noise() -> f64 {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    (u64::from_le_bytes(bytes) as f64 / u64::MAX as f64).mul_add(2.0, -1.0)
}

/// A server's own words, bounded. The peer chooses this text, so the daemon chooses how much
/// of it travels into an error and a health surface.
fn clip(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(MAX_REFUSAL_CHARS) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The doubling, the cap and the jitter, without a network under them.
    #[test]
    fn the_backoff_doubles_to_five_minutes_and_never_past_it() {
        for failures in 1..40u32 {
            let d = backoff_delay(failures);
            assert!((0.0..=BACKOFF_CAP_SECONDS).contains(&d), "{failures}: {d}");
        }
        // The un-jittered midpoint doubles: the widest a jittered 2^(k-1) can be is
        // 1.25 × 2^(k-1), and the narrowest a jittered 2^k can be is 0.75 × 2^k = 1.5 ×
        // 2^(k-1), so the bands do not overlap however the noise falls.
        for failures in 1..9u32 {
            assert!(backoff_delay(failures) < backoff_delay(failures + 1));
        }
        assert!(backoff_delay(40) >= BACKOFF_CAP_SECONDS * (1.0 - BACKOFF_JITTER));
    }

    /// Jitter that always returned the same number would not be jitter.
    #[test]
    fn the_noise_is_noise_and_stays_inside_the_unit_interval() {
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..64 {
            let n = unit_noise();
            assert!((-1.0..=1.0).contains(&n), "{n}");
            seen.insert(n.to_bits());
        }
        assert!(seen.len() > 32, "the jitter did not vary");
    }

    /// A peer's own words, bounded — and a boundary that lands inside a multi-byte character
    /// does not panic on a byte index.
    #[test]
    fn a_refusal_is_clipped_on_a_character_boundary() {
        assert_eq!(clip("  refused  "), "refused");
        let wide = "é".repeat(MAX_REFUSAL_CHARS + 10);
        let clipped = clip(&wide);
        assert!(clipped.ends_with('…'));
        assert_eq!(clipped.chars().count(), MAX_REFUSAL_CHARS + 1);
    }
}
