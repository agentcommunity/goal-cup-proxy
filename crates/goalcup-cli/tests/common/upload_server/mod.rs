//! A local stand-in for the Goal Cup upload server, for tests only.
//!
//! It implements `UPLOAD-API.md` (version 1) over loopback HTTP so the Rust tests of the tracer's
//! uploader run without Cloudflare. A TypeScript Worker implements the same contract; where the
//! two ever disagree the contract file decides, and a test here is the cheap way to find out.
//!
//! * State is a filesystem store: one JSON file per run (`state.rs`), chunk bodies create-only
//!   under `<run>/<entry>/`. `stop` leaves the store alone, so `start` on the same directory and
//!   port resumes the same state. That is how a test says "the server went down mid-run".
//! * The server runs on its own thread with its own tokio runtime, so a test thread stays
//!   synchronous and can call it with `ureq`.
//! * It checks exactly what the contract says the server checks, and nothing more. It does not
//!   decompress a chunk and does not look inside the chain.
//!
//! Test hooks (not part of the contract): `set_maintenance`, `set_maintenance_retry_after`,
//! `fail_next_chunks` and `drop_next_chunk_reply`.

mod handlers;
mod state;

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread::JoinHandle;

use axum::serve::Listener;
use ed25519_dalek::SigningKey;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::receipt::StartReceipt;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};

pub use state::{Position, StoredChunk};
use state::{RunStatus, Shared};

/// What the stand-in is configured with.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// `(token, member)` pairs: the bearer tokens the server knows and whose they are.
    pub tokens: Vec<(String, String)>,
    /// The event's run length; `deadline_at` is `official_start` plus this.
    pub run_hours: u64,
    /// Seed of the Ed25519 key that signs receipts and acknowledgements.
    pub signing_seed: [u8; 32],
    /// A fixed port, so a test can restart on the same one. `None` picks a free port.
    pub port: Option<u16>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            tokens: vec![("test-token".to_string(), "member-1".to_string())],
            run_hours: 168,
            signing_seed: [7u8; 32],
            port: None,
        }
    }
}

/// A running stand-in. Dropping it stops it; `stop` does the same and says so.
pub struct UploadServer {
    shared: Arc<Shared>,
    port: u16,
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl UploadServer {
    /// Bind `127.0.0.1` (port `config.port`, or any free one), load whatever `store_dir` already
    /// holds, and return once the listener is accepting.
    pub fn start(store_dir: &Path, config: ServerOptions) -> UploadServer {
        let key = SigningKey::from_bytes(&config.signing_seed);
        let shared = Arc::new(Shared {
            store_dir: store_dir.to_path_buf(),
            tokens: config.tokens.into_iter().collect::<HashMap<_, _>>(),
            run_hours: config.run_hours,
            key,
            runs: Mutex::new(Shared::load_runs(store_dir)),
            calls: Mutex::new(Vec::new()),
            maintenance: AtomicBool::new(false),
            retry_after: AtomicU64::new(5),
            fail_chunks: AtomicU64::new(0),
            drop_replies: AtomicU64::new(0),
        });
        let app = handlers::router(shared.clone());
        let bind_port = config.port.unwrap_or(0);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u16, String>>();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("upload-standin".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("build the stand-in's tokio runtime");
                rt.block_on(async move {
                    let listener = match TcpListener::bind(("127.0.0.1", bind_port)).await {
                        Ok(l) => l,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e.to_string()));
                            return;
                        }
                    };
                    let port = listener.local_addr().expect("listener address").port();
                    let _ = ready_tx.send(Ok(port));
                    let serve = axum::serve(
                        KillListener(listener),
                        app.into_make_service_with_connect_info::<handlers::Conn>(),
                    );
                    tokio::select! {
                        _ = serve => {}
                        _ = stop_rx => {}
                    }
                });
                // Dropping the runtime cancels every connection task still alive, so a pooled
                // keep-alive connection cannot outlive `stop` and keep answering.
                drop(rt);
            })
            .expect("spawn the stand-in thread");
        let port = ready_rx
            .recv()
            .expect("the stand-in thread ended before it listened")
            .unwrap_or_else(|e| panic!("the upload stand-in could not bind port {bind_port}: {e}"));
        UploadServer {
            shared,
            port,
            stop_tx: Some(stop_tx),
            thread: Some(thread),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port>`, what `goalcup login --server` would be given.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Hex of the Ed25519 public key that signs receipts and acknowledgements.
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.shared.key.verifying_key().to_bytes())
    }

    /// Shut the listener down so connections are refused. The store stays on disk. Blocks until
    /// the server thread has ended, so the port is free when this returns.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().expect("join the stand-in thread");
        }
    }

    /// While on, every route answers `503 maintenance` with a `Retry-After` header.
    pub fn set_maintenance(&self, on: bool) {
        self.shared.maintenance.store(on, Ordering::SeqCst);
    }

    /// The `Retry-After` seconds a maintenance answer carries (default 5).
    pub fn set_maintenance_retry_after(&self, seconds: u64) {
        self.shared.retry_after.store(seconds, Ordering::SeqCst);
    }

    /// The next `n` chunk PUTs answer `500` and store nothing.
    pub fn fail_next_chunks(&self, n: u64) {
        self.shared.fail_chunks.store(n, Ordering::SeqCst);
    }

    /// The next chunk PUT is stored and committed, then the connection is cut without a reply:
    /// the tracer sees a transport error for a chunk the server holds, and its retry must be
    /// answered from the stored acknowledgement.
    pub fn drop_next_chunk_reply(&self) {
        self.shared.drop_replies.store(1, Ordering::SeqCst);
    }

    fn with_run<T>(&self, run_id: &str, f: impl FnOnce(&state::RunRecord) -> T) -> Option<T> {
        let runs = self.shared.runs.lock().unwrap_or_else(|e| e.into_inner());
        runs.get(run_id).map(f)
    }

    /// Every run the server holds, ordered by id.
    pub fn run_ids(&self) -> Vec<String> {
        let runs = self.shared.runs.lock().unwrap_or_else(|e| e.into_inner());
        runs.keys().cloned().collect()
    }

    pub fn position(&self, run_id: &str) -> Option<Position> {
        self.with_run(run_id, |r| Position {
            offset: r.offset(),
            next_seq: r.next_seq(),
            head: r.head(),
        })
    }

    pub fn chunks(&self, run_id: &str) -> Vec<StoredChunk> {
        self.with_run(run_id, |r| r.chunks.clone())
            .unwrap_or_default()
    }

    /// `(seal_json, baseline_json)` once sealed.
    pub fn seal(&self, run_id: &str) -> Option<(String, String)> {
        self.with_run(run_id, |r| r.seal.clone()).flatten()
    }

    pub fn receipt(&self, run_id: &str) -> Option<StartReceipt> {
        self.with_run(run_id, |r| r.receipt.clone()).flatten()
    }

    pub fn last_ack(&self, run_id: &str) -> Option<AckCheckpoint> {
        self.with_run(run_id, |r| r.chunks.last().map(|c| c.ack.clone()))
            .flatten()
    }

    /// `open`, `sealed` or `abandoned`.
    pub fn status(&self, run_id: &str) -> Option<String> {
        self.with_run(run_id, |r: &state::RunRecord| {
            let s: RunStatus = r.status;
            s.as_str().to_string()
        })
    }

    /// Every request the server saw, in order, as `"<METHOD> <path>"` (maintenance answers
    /// included).
    pub fn calls(&self) -> Vec<String> {
        self.shared
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for UploadServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A TCP listener whose connections can be cut from inside a handler.
///
/// axum gives a handler no way to drop the connection without replying, which is exactly what a
/// lost reply is. Each accepted connection carries a flag (`handlers::Conn`, reachable through
/// `ConnectInfo`); once a handler sets it, the connection's writes fail, hyper gives up on the
/// connection and closes the socket, and the client sees the stream end with no response.
struct KillListener(TcpListener);

impl Listener for KillListener {
    type Io = KillIo;
    type Addr = handlers::Conn;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.0.accept().await {
                Ok((stream, _)) => {
                    let kill = Arc::new(AtomicBool::new(false));
                    return (
                        KillIo {
                            inner: stream,
                            kill: kill.clone(),
                        },
                        handlers::Conn(kill),
                    );
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(handlers::Conn(Arc::new(AtomicBool::new(false))))
    }
}

struct KillIo {
    inner: TcpStream,
    kill: Arc<AtomicBool>,
}

impl KillIo {
    fn killed(&self) -> bool {
        self.kill.load(Ordering::SeqCst)
    }
}

fn cut() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "connection cut by a test hook")
}

impl AsyncRead for KillIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for KillIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.killed() {
            return Poll::Ready(Err(cut()));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.killed() {
            return Poll::Ready(Err(cut()));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
