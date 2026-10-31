//! The relay pump (PRD-03 section 1 rules 1, 4 and 8, section 5a, section 10). Each direction
//! reads a chunk, writes it to the other side, and only then hands the same bytes to the
//! connection's parser, inside `catch_unwind`. No deadline, idle timeout or cap. When one leg's
//! error ends the relay, the part of a chunk the other side's writer already took is recorded
//! too: it may have reached the peer. The rest of a client chunk, never written, goes to the
//! parser at the close to be recorded as `unsent`. The rest of a server chunk, read from the
//! upstream and never written to the client (the client went away), is fed to the parser before
//! the close: it is model output we hold, and its response ends `interrupted`.

use crate::pipeline::{EndReason, GapReason, Mono, Recorder, Sid, StreamHandle};
use crate::proxy::ws::Dir;
use bytes::Bytes;
use serde_json::{Value, json};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// Parsing stopped for the connection (5a): the `conn` GAP's reason and tail, and the relayed
/// plaintext bytes the parser took but recorded in no stream (the rest of the faulting chunk, an
/// upgrade copy, server bytes waiting for their request, a head cut short), the first of them no
/// earlier than `since`. The pump adds every byte relayed after it and the chunk rests the close
/// left unwritten. `at_close`: bytes only held at the close (a head cut short); parsing never
/// stopped, so the GAP comes without the note.
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
    /// Relayed bytes of one direction, called only after they were written to the other side;
    /// at the close also the rest of a server chunk the client never took.
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
    /// The harness leg. The caller drops it (or resets it, rule 8) and never shuts it down: after
    /// an upstream cut a shutdown would send close_notify and pass a cut reply on as whole.
    pub client: C,
    pub upstream: U,
    pub plain_up: u64,
    pub plain_down: u64,
    /// `complete` when both directions ended in a close: a client's cut counts as one, and so
    /// does an upstream cut that comes after the client half-closed (`Eof` then `Cut`).
    /// `interrupted` when a leg errored, or when an upstream cut ends the relay first and leaves
    /// the client leg `Dropped`. No byte is lost and no GAP is written for a cut either way: a
    /// close-delimited body it may have shortened still ends interrupted through `down_cut`.
    pub reason: EndReason,
    /// The upstream leg errored (reset, TLS error, read or write error), not a clean close.
    pub upstream_failed: bool,
    /// Bytes read from one side and never written to the other, by [`Dir`]: the chunk rests the
    /// error cut. With a parser they are recorded or in a `conn` GAP already; a tunnel counts them.
    pub unwritten: [u64; 2],
}

/// Called after each chunk moved: its direction, time and length.
pub type Tap<'a> = &'a (dyn Fn(Dir, Mono, u64) + Sync);

/// How one direction ended. `Cut`: a TLS peer closed without close_notify. A client's cut is
/// passed on as a clean close. An upstream's cut ends the relay (at once, unless the client
/// already half-closed) with no shutdown of the client leg, so the harness gets no close_notify
/// either and sees a close-delimited body cut, as it would without us (rule 8); the caller drops
/// that leg, a plain TCP close. `Dropped`: the leg was left unfinished when the other one ended
/// the relay.
#[derive(Clone, Copy)]
enum Leg {
    Eof,
    Cut,
    ReadErr,
    WriteErr,
    Dropped,
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
                        if dir == Dir::Down {
                            return Leg::Cut;
                        }
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
        // An error on one leg, or an upstream cut, ends the other: the connection is over.
        use Leg::{Cut, Dropped, Eof};
        tokio::select! {
            x = &mut a => (x, if matches!(x, Eof | Cut) { b.await } else { Dropped }),
            y = &mut b => (if matches!(y, Eof) { a.await } else { Dropped }, y),
        }
    };
    // A leg that errored, or was dropped mid-write, leaves the part its writer took; the rest of
    // a client chunk never reached the upstream, the rest of a server chunk never the client.
    let (rest, cut) = (pump.settle(Dir::Up), pump.settle(Dir::Down));
    let unwritten = [rest.len() as u64, cut.len() as u64];
    // Model output read from the upstream: parsed, never moved; after a stop `held` counts it.
    if !cut.is_empty() {
        pump.feed(Dir::Down, rec.now(), &cut);
    }
    // Streams still open at the close end on how the legs ended; an upstream cut leaves the
    // client leg unfinished, which ends the connection, not a stream framing has shown whole.
    let streams = match (lu, ld) {
        (Leg::Eof | Leg::Cut | Leg::Dropped, Leg::Eof | Leg::Cut) => EndReason::Complete,
        _ => EndReason::Interrupted,
    };
    let reason = match lu {
        Leg::Dropped => EndReason::Interrupted,
        _ => streams,
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
        let ended = catch_unwind(AssertUnwindSafe(|| p.end(t, streams, down_cut, &unsent)));
        match ended {
            Ok(Some(late)) if stop.is_none() => {
                if !late.at_close {
                    parse_stopped(rec, conn.sid(), late.reason);
                }
                stop = Some(late);
            }
            // The parser is dropped: its open streams end interrupted, and what it held is unknown.
            Err(_) if stop.is_none() => {
                let s = Stop::panic(0, t);
                parse_stopped(rec, conn.sid(), s.reason);
                conn.gap(s.reason, t, None, s.tail);
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
        unwritten,
    }
}
