//! The relay pump (PRD-03 section 1 rules 1, 4 and 8, section 5a, section 10). Each direction
//! reads a chunk, writes it to the other side, and only then hands the same bytes to the
//! connection's parser, inside `catch_unwind`. No deadline, idle timeout or cap. When one leg's
//! error ends the relay, the part of a chunk the other side's writer already took is recorded
//! too: it may have reached the peer. The rest of a client chunk, never written, goes to the
//! parser at the close to be recorded as `unsent`.

use crate::pipeline::{EndReason, GapReason, Mono, Recorder, Sid, StreamHandle};
use crate::proxy::ws::Dir;
use bytes::Bytes;
use serde_json::{Value, json};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};

/// Parsing stopped for the connection (5a): the `conn` GAP's reason and tail, and the relayed
/// plaintext bytes the parser took but recorded in no stream (the rest of the faulting chunk, an
/// upgrade copy, server bytes waiting for their request, a head cut short), the first of them no
/// earlier than `since`. The pump adds every byte relayed after it. `at_close`: bytes only held
/// at the close (a head cut short); parsing never stopped, so the GAP comes without the note.
#[derive(Debug, PartialEq)]
pub struct Stop {
    pub reason: GapReason,
    pub tail: Option<Value>,
    pub held: u64,
    pub since: Mono,
    pub at_close: bool,
}

impl Stop {
    /// A parser panic (rule 4).
    pub fn panic(held: u64, since: Mono) -> Stop {
        let tail = Some(json!({"panic": true}));
        let reason = GapReason::DecodeFailed;
        Stop {
            reason,
            tail,
            held,
            since,
            at_close: false,
        }
    }
}

/// A passive parser of one connection's plaintext (HTTP/1.1, HTTP/2).
pub trait Parser: Send {
    /// Relayed bytes of one direction, called only after they were written to the other side.
    /// Err: parsing stopped for the connection (5a); every open stream already has its own GAP
    /// and ended `decode_error`. Later feeds are not made; `end` still is.
    fn feed(&mut self, dir: Dir, t: Mono, bytes: &[u8]) -> Result<(), Stop>;
    /// The connection closed. `unsent`: client bytes read but never forwarded (the upstream had
    /// closed or failed), empty after a stop; the parser records them as if fed, and each stream
    /// they reach gets its share as END tail `unsent`. A `Stop` names bytes still held
    /// unrecorded at the close. `down_cut`: the upstream closed TLS without close_notify, so only
    /// a body that ends at the server's close is not known whole; framing shows whether anything
    /// else is.
    fn end(
        self: Box<Self>,
        t: Mono,
        reason: EndReason,
        down_cut: bool,
        unsent: &[u8],
    ) -> Option<Stop>;
    /// `feed` panicked: every open stream gets its GAP and ends `decode_error`; the `Stop` counts
    /// the bytes not recorded, the panicking chunk included.
    fn panicked(self: Box<Self>, t: Mono) -> Stop;
}

/// What [`relay`] hands back: both streams, so the caller can reset the harness leg (rule 8).
pub struct Pumped<C, U> {
    pub client: C,
    pub upstream: U,
    pub plain_up: u64,
    pub plain_down: u64,
    /// `complete` when both sides closed cleanly, else `interrupted`.
    pub reason: EndReason,
    /// The upstream leg errored (reset, TLS error, read or write error), not a clean close.
    pub upstream_failed: bool,
}

/// Called after each chunk moved: its direction, time and length.
pub type Tap<'a> = &'a (dyn Fn(Dir, Mono, u64) + Sync);

/// How one direction ended. `Cut`: a TLS peer closed without close_notify, passed on as a clean
/// close (rule 8), though only close_notify proves a close-delimited body whole.
#[derive(Clone, Copy)]
enum Leg {
    Eof,
    Cut,
    ReadErr,
    WriteErr,
}

struct Pump<'a> {
    rec: &'a Recorder,
    conn: Sid,
    quiet: &'a AtomicU64,
    tap: Option<Tap<'a>>,
    /// Plaintext bytes moved, by [`Dir`]; kept outside the legs, which may be cut.
    moved: [AtomicU64; 2],
    /// By [`Dir`]: the chunk being written and how much of it the writer took.
    inflight: [Mutex<(Bytes, usize)>; 2],
    shared: Mutex<Shared>,
}

/// The parser and, once parsing stopped, its [`Stop`] with every byte relayed since added.
struct Shared {
    parser: Option<Box<dyn Parser>>,
    stop: Option<Stop>,
}

fn parse_stopped(rec: &Recorder, conn: Sid, reason: GapReason) {
    rec.event(
        json!({"ev": "note", "what": "parse_stopped", "conn": conn, "reason": reason.name()}),
    );
}

impl Pump<'_> {
    /// The parser sees the chunk under a lock that is never held across an await.
    fn feed(&self, dir: Dir, t: Mono, bytes: &[u8]) {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(stop) = &mut s.stop {
            stop.held += bytes.len() as u64;
            return;
        }
        let Some(p) = s.parser.as_mut() else { return };
        let stop = match catch_unwind(AssertUnwindSafe(|| p.feed(dir, t, bytes))) {
            Ok(Ok(())) => return,
            Ok(Err(stop)) => stop,
            Err(_) => {
                let p = s.parser.take();
                let ended = p.map(|p| catch_unwind(AssertUnwindSafe(move || p.panicked(t))));
                // A second panic drops the parser: its open streams end interrupted.
                match ended {
                    Some(Ok(stop)) => stop,
                    _ => Stop::panic(bytes.len() as u64, t),
                }
            }
        };
        let reason = stop.reason;
        s.stop = Some(stop);
        drop(s);
        parse_stopped(self.rec, self.conn, reason);
    }

    /// The written part of `dir`'s chunk in flight is moved: counted, shown to `tap`, parsed.
    /// Returns the rest, read and never written.
    fn settle(&self, dir: Dir) -> Bytes {
        let slot = &self.inflight[dir as usize];
        let (chunk, n) = std::mem::take(&mut *slot.lock().unwrap_or_else(|e| e.into_inner()));
        let rest = chunk.slice(n..);
        if n == 0 {
            return rest;
        }
        let t = self.rec.now();
        self.quiet.fetch_max(t.ns(), Relaxed);
        self.moved[dir as usize].fetch_add(n as u64, Relaxed);
        if let Some(tap) = self.tap {
            tap(dir, t, n as u64);
        }
        self.feed(dir, t, &chunk[..n]);
        rest
    }

    async fn leg<R, W>(&self, dir: Dir, prefix: Bytes, from: &mut R, to: &mut W) -> Leg
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let (mut buf, mut prefix, mut end) = (vec![0u8; 16 * 1024], Some(prefix), Leg::Eof);
        loop {
            let chunk = match prefix.take().filter(|p| !p.is_empty()) {
                Some(p) => p,
                None => match from.read(&mut buf).await {
                    Ok(0) => break,
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                        end = Leg::Cut;
                        break;
                    }
                    Ok(n) => Bytes::copy_from_slice(&buf[..n]),
                    Err(_) => return Leg::ReadErr,
                },
            };
            let slot = &self.inflight[dir as usize];
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = (chunk.clone(), 0);
            let mut off = 0;
            while off < chunk.len() {
                match to.write(&chunk[off..]).await {
                    Ok(k) if k > 0 => off += k,
                    _ => return Leg::WriteErr,
                }
                slot.lock().unwrap_or_else(|e| e.into_inner()).1 = off;
            }
            if to.flush().await.is_err() {
                return Leg::WriteErr;
            }
            self.settle(dir);
        }
        let _ = to.shutdown().await;
        end
    }
}

/// Relays both ways until both sides closed or one leg errored, first replaying the prefixes (to
/// upstream, to the client) as relayed chunks; every chunk moved is shown to `tap`. A clean EOF is
/// passed on as a shutdown of the other side's write half. `quiet` takes `rec.now()` of every
/// chunk moved. When parsing stopped, `conn` gets the 5a GAP after the parser ended.
pub async fn relay<C, U>(
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
    let shared = Mutex::new(Shared { parser, stop: None });
    let (moved, inflight) = Default::default();
    let pump = Pump {
        rec,
        conn: conn.sid(),
        quiet,
        tap,
        moved,
        inflight,
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
            x = &mut a => (x, if matches!(x, Leg::Eof | Leg::Cut) { b.await } else { Leg::Eof }),
            y = &mut b => (if matches!(y, Leg::Eof | Leg::Cut) { a.await } else { Leg::Eof }, y),
        }
    };
    // A leg that errored, or was dropped mid-write, leaves the part its writer took; the rest of
    // a client chunk never reached the upstream.
    let rest = pump.settle(Dir::Up);
    pump.settle(Dir::Down);
    let reason = match (lu, ld) {
        (Leg::Eof | Leg::Cut, Leg::Eof | Leg::Cut) => EndReason::Complete,
        _ => EndReason::Interrupted,
    };
    let down_cut = matches!(ld, Leg::Cut);
    let [up, down] = pump.moved.map(AtomicU64::into_inner);
    let Shared { parser, mut stop } = pump.shared.into_inner().unwrap_or_else(|e| e.into_inner());
    // Never relayed, so not moved or tapped; after a stop the conn GAP counts them.
    let unsent = match &mut stop {
        Some(s) => {
            s.held += rest.len() as u64;
            Bytes::new()
        }
        None => rest,
    };
    if let Some(p) = parser {
        let t = rec.now();
        let ended = catch_unwind(AssertUnwindSafe(|| p.end(t, reason, down_cut, &unsent)));
        match ended {
            Ok(Some(late)) if stop.is_none() => {
                if !late.at_close {
                    parse_stopped(rec, conn.sid(), late.reason);
                }
                stop = Some(late);
            }
            // The parser is dropped: its open streams end interrupted, and what it held is unknown.
            Err(_) if stop.is_none() => {
                parse_stopped(rec, conn.sid(), GapReason::DecodeFailed);
                let tail = Some(json!({"panic": true}));
                conn.gap(GapReason::DecodeFailed, t, None, tail);
            }
            _ => {}
        }
    }
    if let Some(s) = stop {
        conn.gap(s.reason, s.since, Some(s.held), s.tail);
    }
    Pumped {
        client: cr.unsplit(cw),
        upstream: ur.unsplit(uw),
        plain_up: up,
        plain_down: down,
        reason,
        upstream_failed: matches!(lu, Leg::WriteErr) || matches!(ld, Leg::ReadErr),
    }
}

/// Counts the bytes read from and written to `T`: the TCP wire under TLS, for the `conn` END
/// tail `up` and `down` of a decrypted connection. Wired by `serve` (the next slice).
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
