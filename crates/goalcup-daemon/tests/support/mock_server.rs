//! The upload harness: a real `goalcup_devserver::DevServer` behind a recording proxy.
//!
//! The proxy is deliberately thin. Everything that decides whether a chunk is acceptable —
//! the run key the receipt pinned, the link-by-link re-hash, the contiguity rule, the
//! `chunk_id` re-derivation — stays in the devserver, because a mock that made those
//! decisions itself would agree with whatever the uploader sent and the uploader would be tested
//! against its own opinion. What the proxy adds is the two things a test needs and a server
//! has no reason to offer:
//!
//!   * [`MockServer::last_chunk`] — the exact `Chunk` the daemon put on the wire, so the
//!     signature, the derived id and the range can be asserted against the bytes that
//!     travelled rather than against the uploader's intentions;
//!   * [`MockServer::sign_next_with`] — one acknowledgement re-signed under a key nobody
//!     pinned: a hostile or broken server handing the
//!     participant an anchor for a chain it never saw. The daemon must store nothing.
//!
//! `set_offline` and `set_clock` are passed straight through to the devserver, which already
//! owns both hooks.

#![allow(dead_code)]

use axum::Router;
use axum::extract::{Path, State};
use axum::routing::post;
use ed25519_dalek::{Signer, SigningKey};
use goalcup_daemon::buffer::Buffer;
use goalcup_daemon::chain_writer::ChainWriter;
use goalcup_daemon::redact::Redactor;
use goalcup_daemon::registry::RunKeys;
use goalcup_daemon::upload::{UploadClient, Uploader};
use goalcup_devserver::{DevError, DevServer, ReceiptRequest};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chunk::Chunk;
use goalcup_protocol::receipt::StartReceipt;
use std::sync::{Arc, Mutex};

/// The entry every fixture in this file uses. `support::envelope` stamps `entry_id: "e1"`
/// into every envelope it builds, and the devserver refuses a chunk filed under any other
/// entry, so the two have to agree.
pub const ENTRY_ID: &str = "e1";

pub struct MockServer {
    inner: Arc<DevServer>,
    chunks: Mutex<Vec<Chunk>>,
    /// Set by [`MockServer::sign_next_with`] and consumed by the next chunk: the key the
    /// *next* acknowledgement is re-signed under. One-shot, because a test that wants two
    /// forged acknowledgements should have to say so twice.
    forge_next: Mutex<Option<SigningKey>>,
    /// Set by [`MockServer::ack_next_at`]: the seq the *next* acknowledgement claims,
    /// re-signed under the server's own key so the document is genuine and only the range
    /// is wrong. One-shot, for the same reason.
    misdirect_next: Mutex<Option<u64>>,
}

impl MockServer {
    pub fn new(inner: Arc<DevServer>) -> MockServer {
        MockServer {
            inner,
            chunks: Mutex::new(Vec::new()),
            forge_next: Mutex::new(None),
            misdirect_next: Mutex::new(None),
        }
    }

    /// The last chunk that reached this server, exactly as it was serialised by the daemon
    /// and deserialised by axum.
    pub fn last_chunk(&self) -> Option<Chunk> {
        self.lock(&self.chunks).last().cloned()
    }

    pub fn chunks(&self) -> Vec<Chunk> {
        self.lock(&self.chunks).clone()
    }

    pub fn chunk_count(&self) -> usize {
        self.lock(&self.chunks).len()
    }

    /// Re-sign the next acknowledgement with `key`, whoever's it is. The document is
    /// otherwise untouched — same entry, same seq, same hash, same `acked_at` — so the only
    /// thing wrong with it is the one thing `AckCheckpoint::verify` is for.
    pub fn sign_next_with(&self, key: &SigningKey) {
        *self.lock(&self.forge_next) = Some(key.clone());
    }

    /// Answer the next chunk with an acknowledgement that names `seq` instead of the range
    /// that was sent, signed with the server's real key so it verifies.
    ///
    /// This is the case [`MockServer::sign_next_with`] cannot reach: a document that is
    /// genuinely the server's and still does not acknowledge what this daemon sent — a
    /// replayed acknowledgement for an earlier range, or a server that lost its place. The
    /// pinned key says yes and the range says no, and the daemon has to believe the range.
    pub fn ack_next_at(&self, seq: u64) {
        *self.lock(&self.misdirect_next) = Some(seq);
    }

    pub fn set_offline(&self, offline: bool) {
        self.inner.set_offline(offline);
    }

    pub fn set_clock(&self, rfc3339: &str) {
        self.inner.set_clock(rfc3339);
    }

    pub fn public_key_hex(&self) -> String {
        self.inner.public_key_hex()
    }

    pub fn acked_seq(&self, entry_id: &str) -> Option<u64> {
        self.inner.acked_seq(entry_id)
    }

    pub fn issue_receipt(&self, request: ReceiptRequest) -> Result<StartReceipt, DevError> {
        self.inner.issue_receipt(request)
    }

    /// Record, delegate, and — if a test asked for it — hand back an acknowledgement the
    /// pinned key does not vouch for.
    fn accept(&self, chunk: &Chunk) -> Result<AckCheckpoint, DevError> {
        self.lock(&self.chunks).push(chunk.clone());
        let mut ack = self.inner.accept_chunk(chunk)?;
        if let Some(seq) = self.lock(&self.misdirect_next).take() {
            ack.seq = seq;
            let bytes = ack
                .signing_bytes()
                .expect("an acknowledgement of strings and integers serialises");
            let key = goalcup_devserver::dev_server_signing_key();
            ack.server_signature = hex::encode(key.sign(&bytes).to_bytes());
        }
        if let Some(key) = self.lock(&self.forge_next).take() {
            let bytes = ack
                .signing_bytes()
                .expect("an acknowledgement of strings and integers serialises");
            ack.server_signature = hex::encode(key.sign(&bytes).to_bytes());
        }
        Ok(ack)
    }

    fn lock<'a, T>(&self, m: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The devserver's two routes, with the chunk route going through [`MockServer::accept`].
/// `DevError` keeps its own `IntoResponse`, so the statuses a client sees here — 503 while
/// offline, 400 for a chain that does not stand, 404 for an entry with no receipt — are the
/// server's, not this file's.
pub fn mock_router(server: Arc<MockServer>) -> Router {
    Router::new()
        .route("/v0/entries/{id}/receipt", post(receipt))
        .route("/v0/entries/{id}/chunks", post(chunks))
        .with_state(server)
}

async fn receipt(
    State(server): State<Arc<MockServer>>,
    Path(id): Path<String>,
    axum::Json(request): axum::Json<ReceiptRequest>,
) -> Result<axum::Json<StartReceipt>, DevError> {
    if request.claims.entry_id != id {
        return Err(DevError::UnknownEntry(id));
    }
    server.issue_receipt(request).map(axum::Json)
}

async fn chunks(
    State(server): State<Arc<MockServer>>,
    Path(id): Path<String>,
    axum::Json(chunk): axum::Json<Chunk>,
) -> Result<axum::Json<AckCheckpoint>, DevError> {
    if chunk.claims.entry_id != id {
        return Err(DevError::UnknownEntry(id));
    }
    server.accept(&chunk).map(axum::Json)
}

/// One entry's buffer, chain writer and uploader, pointed at a mock server on a loopback
/// port this process owns.
pub struct UploadHarness {
    pub buffer: Buffer,
    pub writer: ChainWriter,
    pub uploader: Uploader,
    pub devserver: Arc<MockServer>,
    pub run_keys: RunKeys,
    pub trace_dir: std::path::PathBuf,
    pub url: String,
}

/// The default harness: `upload_chunk_max` as a machine that configured nothing gets it.
pub async fn harness_with_devserver(root: &std::path::Path) -> UploadHarness {
    harness_with_chunk_max(root, 256).await
}

/// The same harness on a smaller chunk, for the catch-up case: an outage that produced more
/// than one chunk's worth has to drain in chunk-sized steps rather than in one allocation
/// the size of the run.
pub async fn harness_with_chunk_max(root: &std::path::Path, chunk_max: usize) -> UploadHarness {
    let run_keys = RunKeys::from_seeds([3u8; 32], [8u8; 32], [1u8; 16]);
    let buffer = Buffer::open(&root.join("buffer.redb"), &run_keys.buffer_key_hex)
        .expect("the buffer opens");
    let trace_dir = root.join("trace");
    std::fs::create_dir_all(&trace_dir).expect("the trace directory");
    let writer =
        ChainWriter::create(ENTRY_ID, &trace_dir, Redactor::new()).expect("a chain writer");

    let inner = Arc::new(DevServer::new());
    let server = Arc::new(MockServer::new(inner));
    // The receipt has to exist before the first chunk — a chunk for an entry no receipt was
    // issued for is refused — and its `chain_head` has to witness seq 0, which is the
    // baseline `support::envelope(0, …)` builds.
    let genesis = super::envelope(0, "main");
    let mut claims = super::claims(ENTRY_ID, "2026-09-01T00:00:00Z", "2026-09-08T00:00:00Z");
    claims.chain_head = genesis.link.hash.clone();
    server
        .issue_receipt(ReceiptRequest {
            claims,
            run_public_key: run_keys.public_key_hex().expect("the run key"),
        })
        .expect("the entry is opened by a receipt");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    let routed = server.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, mock_router(routed)).await;
    });
    let url = format!("http://{addr}");

    let uploader = Uploader::new(
        ENTRY_ID,
        UploadClient::new(url.clone()),
        chunk_max,
        run_keys.clone(),
        server.public_key_hex(),
        // `clock_skew_seconds` as a machine that configured nothing gets it.
        120.0,
    );
    UploadHarness {
        buffer,
        writer,
        uploader,
        devserver: server,
        run_keys,
        trace_dir,
        url,
    }
}

/// How many lines `acks.jsonl` holds. `0` when the file was never written, which is what a
/// run whose acknowledgements all failed verification leaves behind.
pub fn ack_lines(trace_dir: &std::path::Path) -> usize {
    match std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::ACKS_FILE)) {
        Ok(text) => text.lines().filter(|l| !l.trim().is_empty()).count(),
        Err(_) => 0,
    }
}

/// A `ChainWriter` whose `acks.jsonl` cannot be written to, for the one failure the
/// uploader's ordering is built around: the acknowledgement verified, and the trace would
/// not take it.
///
/// A full disk and a read-only trace directory are the real cases, and neither is
/// reproducible in a test — `TraceExport` opens `acks.jsonl` once and keeps the handle, so
/// re-permissioning the directory afterwards changes nothing about a descriptor that is
/// already open. What does is a pipe: the read end is opened first so the export's
/// `O_WRONLY` open succeeds, and then closed, so every `write` on the handle the writer is
/// holding fails with `EPIPE`. Unix only, hence the gate — the property being pinned (a
/// persist failure arms the backoff instead of re-posting the same chunk every tick) is not
/// platform-specific, but the only cheap way to *cause* one is.
#[cfg(unix)]
pub fn writer_with_unwritable_acks(trace_dir: &std::path::Path) -> ChainWriter {
    use std::os::unix::ffi::OsStrExt;

    std::fs::create_dir_all(trace_dir).expect("the trace directory");
    let acks = trace_dir.join(goalcup_protocol::export::ACKS_FILE);
    let path = std::ffi::CString::new(acks.as_os_str().as_bytes()).expect("a path without a NUL");

    // SAFETY: `path` is a NUL-terminated C string that outlives both calls, and neither
    // call is given a buffer to write into. `O_NONBLOCK` is what keeps the read-end open
    // from blocking until a writer arrives, so this needs no second thread and cannot hang.
    let reader = unsafe {
        assert_eq!(
            libc::mkfifo(path.as_ptr(), 0o600),
            0,
            "acks.jsonl as a pipe"
        );
        libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK)
    };
    assert!(reader >= 0, "the read end of the pipe opens");

    let writer = ChainWriter::create(ENTRY_ID, trace_dir, Redactor::new())
        .expect("a chain writer over the pipe");
    // From here the export holds the only handle on the pipe and there is no reader, so
    // `append_ack` fails the way a full disk would.
    assert_eq!(unsafe { libc::close(reader) }, 0, "the read end closes");
    writer
}
