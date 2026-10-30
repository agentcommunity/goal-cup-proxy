//! The relay pump (PRD-03 section 1 rules 1, 4 and 8, section 5a, section 10). Each direction
//! reads a chunk, writes it to the other side, and only then hands the same bytes to the
//! connection's parser, inside `catch_unwind`. No deadline, idle timeout or cap.

use crate::pipeline::{EndReason, GapReason, Mono, Recorder, Sid, StreamHandle};
use crate::proxy::ws::Dir;
use bytes::Bytes;
use serde_json::json;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};

/// A passive parser of one connection's plaintext (HTTP/1.1, HTTP/2).
pub trait Parser: Send {
    /// Relayed bytes of one direction, called only after they were written to the other side.
    /// Err(reason): parsing stopped for the connection (5a); the parser already gave its open
    /// stream its own GAP and ended it; later feeds are not made.
    fn feed(&mut self, dir: Dir, t: Mono, bytes: &[u8]) -> Result<(), GapReason>;
    /// The connection closed. unsent_up: client bytes read but never forwarded (upstream had
    /// closed).
    fn end(self: Box<Self>, t: Mono, reason: EndReason, unsent_up: u64);
}

/// What [`run`] hands back: both streams, so the caller can reset the harness leg (rule 8).
pub struct Pumped<C, U> {
    pub client: C,
    pub upstream: U,
    pub plain_up: u64,
    pub plain_down: u64,
    pub unsent_up: u64,
    /// The upstream leg errored (reset, TLS error, read or write error), not a clean EOF.
    pub upstream_failed: bool,
}

/// Called after each chunk moved: its direction, time and length.
pub(crate) type Tap<'a> = &'a (dyn Fn(Dir, Mono, u64) + Sync);

/// How one direction ended. `WriteErr` carries the chunk read and not forwarded.
#[derive(Clone, Copy)]
enum Leg {
    Eof,
    ReadErr,
    WriteErr(u64),
}

struct Pump<'a> {
    rec: &'a Recorder,
    conn: Sid,
    quiet: &'a AtomicU64,
    tap: Option<Tap<'a>>,
    /// Plaintext bytes moved, by [`Dir`]; kept outside the legs, which may be cut.
    moved: [AtomicU64; 2],
    shared: Mutex<Shared>,
}

/// The parser; the 5a fault (reason, time, panic); plaintext bytes relayed after it.
type Shared = (Option<Box<dyn Parser>>, Option<(GapReason, Mono, bool)>, u64);

impl Pump<'_> {
    /// The parser sees the chunk under a lock that is never held across an await.
    fn feed(&self, dir: Dir, t: Mono, bytes: &[u8]) {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let (parser, fault, after) = &mut *s;
        if fault.is_some() {
            *after += bytes.len() as u64;
            return;
        }
        let Some(p) = parser.as_mut() else { return };
        let (reason, panic) = match catch_unwind(AssertUnwindSafe(|| p.feed(dir, t, bytes))) {
            Ok(Ok(())) => return,
            Ok(Err(reason)) => (reason, false),
            Err(_) => {
                // Dropped: its open streams end interrupted.
                *parser = None;
                (GapReason::DecodeFailed, true)
            }
        };
        *fault = Some((reason, t, panic));
        drop(s);
        let what = "parse_stopped";
        let ev = json!({"ev": "note", "what": what, "conn": self.conn, "reason": reason.name()});
        self.rec.event(ev);
    }

    async fn leg<R, W>(&self, dir: Dir, prefix: Bytes, from: &mut R, to: &mut W) -> Leg
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let (mut buf, mut prefix) = (vec![0u8; 16 * 1024], Some(prefix));
        loop {
            let chunk = match prefix.take().filter(|p| !p.is_empty()) {
                Some(p) => p,
                None => match from.read(&mut buf).await {
                    Ok(0) => {
                        let _ = to.shutdown().await;
                        return Leg::Eof;
                    }
                    Ok(n) => Bytes::copy_from_slice(&buf[..n]),
                    Err(_) => return Leg::ReadErr,
                },
            };
            let n = chunk.len() as u64;
            if to.write_all(&chunk).await.is_err() || to.flush().await.is_err() {
                return Leg::WriteErr(n);
            }
            let t = self.rec.now();
            self.quiet.fetch_max(t.ns(), Relaxed);
            self.moved[dir as usize].fetch_add(n, Relaxed);
            if let Some(tap) = self.tap {
                tap(dir, t, n);
            }
            self.feed(dir, t, &chunk);
        }
    }
}

/// Relays both ways until both sides closed or one leg errored. A clean EOF is passed on as a
/// shutdown of the other side's write half. `quiet` takes `rec.now()` of every chunk moved.
pub async fn run<C, U>(
    rec: &Recorder,
    conn: &mut StreamHandle,
    client: C,
    upstream: U,
    parser: Option<Box<dyn Parser>>,
    quiet: &AtomicU64,
) -> Pumped<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
    U: AsyncRead + AsyncWrite + Unpin + Send,
{
    let none = (Bytes::new(), Bytes::new());
    relay(rec, conn, (client, upstream), none, parser, quiet, None).await
}

/// [`run`], first replaying the prefixes (to upstream, to the client) as relayed chunks, and
/// showing every chunk moved to `tap`.
pub(crate) async fn relay<C, U>(
    rec: &Recorder,
    conn: &mut StreamHandle,
    (client, upstream): (C, U),
    (to_up, to_client): (Bytes, Bytes),
    parser: Option<Box<dyn Parser>>,
    quiet: &AtomicU64,
    tap: Option<Tap<'_>>,
) -> Pumped<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
    U: AsyncRead + AsyncWrite + Unpin + Send,
{
    let shared = Mutex::new((parser, None, 0));
    let moved = Default::default();
    let pump = Pump {
        rec,
        conn: conn.sid(),
        quiet,
        tap,
        moved,
        shared,
    };
    let ((mut cr, mut cw), (mut ur, mut uw)) =
        (tokio::io::split(client), tokio::io::split(upstream));
    let (lu, ld) = {
        let a = pump.leg(Dir::Up, to_up, &mut cr, &mut uw);
        let b = pump.leg(Dir::Down, to_client, &mut ur, &mut cw);
        tokio::pin!(a, b);
        // An error on one leg ends the other: the connection is over.
        tokio::select! {
            x = &mut a => (x, if matches!(x, Leg::Eof) { b.await } else { Leg::Eof }),
            y = &mut b => (if matches!(y, Leg::Eof) { a.await } else { Leg::Eof }, y),
        }
    };
    let unsent_up = if let Leg::WriteErr(n) = lu { n } else { 0 };
    let reason = match (lu, ld) {
        (Leg::Eof, Leg::Eof) => EndReason::Complete,
        _ => EndReason::Interrupted,
    };
    let [up, down] = pump.moved.map(AtomicU64::into_inner);
    let (parser, fault, after) = pump.shared.into_inner().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = parser {
        let t = rec.now();
        // A panic here drops the parser: its open streams end interrupted, nothing more relays.
        let _ = catch_unwind(AssertUnwindSafe(|| p.end(t, reason, unsent_up)));
    }
    if let Some((reason, at, panic)) = fault {
        conn.gap(
            reason,
            at,
            Some(after),
            panic.then(|| json!({"panic": true})),
        );
    }
    Pumped {
        client: cr.unsplit(cw),
        upstream: ur.unsplit(uw),
        plain_up: up,
        plain_down: down,
        unsent_up,
        upstream_failed: matches!(lu, Leg::WriteErr(_)) || matches!(ld, Leg::ReadErr),
    }
}

/// Counts the bytes read from and written to `T`: the TCP wire under TLS, for the `conn` END
/// tail `up` and `down` of a decrypted connection.
pub struct Counted<T> {
    pub inner: T,
    /// Bytes read, bytes written.
    pub counts: Arc<[AtomicU64; 2]>,
}

impl<T: AsyncRead + Unpin> AsyncRead for Counted<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let r = Pin::new(&mut self.inner).poll_read(cx, buf);
        self.counts[0].fetch_add((buf.filled().len() - before) as u64, Relaxed);
        r
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Counted<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let r = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = r {
            self.counts[1].fetch_add(n as u64, Relaxed);
        }
        r
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
