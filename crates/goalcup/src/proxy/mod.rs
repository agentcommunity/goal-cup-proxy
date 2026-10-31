//! The local recording proxy (PRD-03). The harness reaches its model hosts through
//! `HTTPS_PROXY=http://127.0.0.1:<port>`; for a listed host the proxy ends TLS with a leaf from the
//! run CA, opens its own TLS leg upstream and passes the plaintext through unchanged. Each direction
//! relays first and records second: a chunk is written to the other side before the same bytes go
//! to that connection's passive parser, which hands them to the recorder (seam S1) and never makes
//! the relay wait. Anything that cannot be decrypted or parsed still passes, as a tunnel with a
//! named GAP.
//!
//! [`serve`] owns the accept loop and the per-connection flow of section 1: the CONNECT head, the
//! upstream TCP leg (before the `200`, so an unreachable target is a `502` and a server-first
//! protocol can win the first-bytes race), the ClientHello, the decision, then a tunnel or both
//! TLS legs (upstream first, so a bad upstream certificate is never laundered into our leaf) and
//! the pump with a parser.

pub mod ca;
pub mod connect;
pub mod decode;
pub mod hosts;
pub mod http1;
pub mod pump;
pub mod tls;
pub mod tunnel;
pub mod ws;

use crate::connector::{Decision, HostList, Kind, Source};
use crate::pipeline::{
    EndReason, GapReason, Mono, OpenFlags, Recorder, StreamHandle, StreamKind, fds,
};
use bytes::Bytes;
use ca::RunCa;
use connect::{First, Hello, NoHead, PrefixedIo, RefusalNotes, fingerprint_hex16};
use hosts::Hosts;
use pump::{Parser, Pumped, Stop};
use rustls::pki_types::{CertificateDer, ServerName};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::task::{Context, Poll};
use std::time::Duration;
use tls::{Offer, Refusal, Tls, UpstreamFail};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tunnel::{ConnMeta, TunnelConn};
use ws::Dir;

/// Relay threads (rule 6): TLS, framing and decoding only.
pub const RELAY_THREADS: usize = 2;
/// The pause while `may_accept()` is false, and after a failed `accept` (rule 7).
const PAUSE: Duration = Duration::from_millis(10);

/// What one recorder boot hands the proxy (PRD-00 S8).
pub struct ProxyCtx {
    pub recorder: Recorder,
    pub hosts: HostList,
    pub ca: RunCa,
}

/// The running proxy. Dropping it stops accepting at once, then shuts the runtime down, which
/// drops every connection; the listen socket stays open in the watchdog, so the next boot's
/// [`serve`] on a clone of it serves the backlog.
pub struct ProxyHandle {
    rt: Option<tokio::runtime::Runtime>,
    quiet: Arc<AtomicU64>,
    stop: Option<oneshot::Sender<()>>,
    stopped: mpsc::Receiver<()>,
}

impl ProxyHandle {
    /// The `Recorder::now()` of the last byte any relay moved (at first, the boot). An open but
    /// silent connection never moves it (section 10).
    pub fn quiet_since(&self) -> Mono {
        Mono(self.quiet.load(Relaxed))
    }
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        // The accept loop drops this boot's listener before the runtime goes, so a connection
        // arriving now waits in the backlog for the next boot instead of being taken and cut.
        drop(self.stop.take());
        let _ = self.stopped.recv_timeout(Duration::from_secs(1));
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

/// One boot's shared state.
struct Boot {
    rec: Recorder,
    hosts: Hosts,
    tls: Tls,
    flags: tunnel::Flags,
    notes: RefusalNotes,
    quiet: Arc<AtomicU64>,
    /// `GOALCUP_TEST_UPSTREAM`; always empty without `test-hooks`.
    upstream: HashMap<String, SocketAddr>,
}

/// Accepts on the inherited `listener` (it binds nothing) on a runtime of its own with
/// [`RELAY_THREADS`] workers, and returns at once.
pub fn serve(listener: std::net::TcpListener, ctx: ProxyCtx) -> ProxyHandle {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(RELAY_THREADS)
        .thread_name("goalcup-relay")
        .enable_all()
        .build()
        .expect("the relay runtime starts");
    let (upstream, extra_roots) = test_hooks();
    let quiet = Arc::new(AtomicU64::new(ctx.recorder.now().ns()));
    let boot = Arc::new(Boot {
        hosts: Hosts::new(ctx.hosts),
        tls: Tls::new(&ctx.ca, extra_roots),
        flags: tunnel::Flags::default(),
        notes: RefusalNotes::default(),
        quiet: quiet.clone(),
        upstream,
        rec: ctx.recorder,
    });
    let _ = listener.set_nonblocking(true);
    let listener = {
        let _in = rt.enter();
        TcpListener::from_std(listener).expect("the listen socket joins the runtime")
    };
    let (stop, mut stop_rx) = oneshot::channel::<()>();
    let (done, stopped) = mpsc::channel::<()>();
    rt.spawn(async move {
        let _done = done;
        loop {
            tokio::select! {
                _ = &mut stop_rx => return,
                next = next_conn(&listener, &boot.rec) => if let Some(client) = next {
                    // A panic in one connection ends its task only.
                    tokio::spawn(connection(boot.clone(), client));
                },
            }
        }
    });
    ProxyHandle {
        rt: Some(rt),
        quiet,
        stop: Some(stop),
        stopped,
    }
}

/// Rule 7: wait while descriptors are short, never refuse; an `accept` error (EMFILE included)
/// pauses and the loop goes on.
async fn next_conn(listener: &TcpListener, rec: &Recorder) -> Option<TcpStream> {
    while !fds::may_accept() {
        if let Some((limit, open)) = fds::take_note() {
            rec.event(json!({"ev": "note", "what": "fd_limit", "limit": limit, "open": open}));
        }
        tokio::time::sleep(PAUSE).await;
    }
    match listener.accept().await {
        Ok((client, _)) => {
            let _ = client.set_nodelay(true);
            Some(client)
        }
        Err(_) => {
            tokio::time::sleep(PAUSE).await;
            None
        }
    }
}

#[cfg(feature = "test-hooks")]
fn test_hooks() -> (HashMap<String, SocketAddr>, Vec<CertificateDer<'static>>) {
    let pairs = std::env::var("GOALCUP_TEST_UPSTREAM").unwrap_or_default();
    let upstream = (pairs.split(|c: char| c == ',' || c.is_whitespace()))
        .filter_map(|p| p.split_once('='))
        .filter_map(|(name, addr)| Some((name.to_ascii_lowercase(), addr.parse().ok()?)))
        .collect();
    let roots = std::env::var_os("GOALCUP_TEST_EXTRA_ROOTS").and_then(|p| std::fs::read(p).ok());
    let roots = ca::certificates(&roots.unwrap_or_default());
    let roots = roots.into_iter().map(CertificateDer::from).collect();
    (upstream, roots)
}

#[cfg(not(feature = "test-hooks"))]
fn test_hooks() -> (HashMap<String, SocketAddr>, Vec<CertificateDer<'static>>) {
    Default::default()
}

fn listed(d: Decision) -> Option<(Source, Kind)> {
    match d {
        Decision::Listed { source, kind, .. } => Some((source, kind)),
        _ => None,
    }
}

/// A request line's start: an upper-case method and a space.
fn looks_like_request(bytes: &[u8]) -> bool {
    let method = bytes.iter().take_while(|b| b.is_ascii_uppercase()).count();
    (3..=16).contains(&method) && bytes.get(method) == Some(&b' ')
}

/// One harness connection, section 1 from the head to the close.
async fn connection(b: Arc<Boot>, mut client: TcpStream) {
    let rec = &b.rec;
    let head = match connect::read_head(&mut client, rec).await {
        Ok(head) => head,
        Err(NoHead::Refused(r)) => {
            b.notes.note(rec, r.host.as_deref(), r.status);
            let _ = connect::answer(&mut client, r.status).await;
            return;
        }
        Err(NoHead::Closed) => return,
    };
    let (host, port) = (head.host.as_str(), head.port);
    let target = b.upstream.get(host).copied();
    let connect_listed = listed(b.hosts.decide(None, host).1);
    let tc = |reason, listed, fingerprint, client_prefix, upstream_prefix| TunnelConn {
        host: host.to_owned(),
        port,
        reason,
        listed,
        fingerprint,
        client_prefix,
        upstream_prefix,
    };
    let is_listed = connect_listed.is_some();
    let Ok(mut up) = connect::connect_upstream(rec, host, port, is_listed, target).await else {
        b.notes.note(rec, Some(host), 502);
        let _ = connect::answer(&mut client, 502).await;
        return;
    };
    if connect::answer(&mut client, 200).await.is_err() {
        return;
    }
    let first = match head.rest.is_empty() {
        false => head.rest.clone(),
        true => match connect::first_bytes(&mut client, &mut up).await {
            First::Client(bytes) => bytes,
            First::Server(bytes) => {
                let c = tc("not_tls", connect_listed, None, Bytes::new(), bytes);
                return tunnel::run(rec, &b.flags, &b.quiet, c, client, up).await;
            }
            First::ClientClosed | First::ServerClosed => {
                let c = tc("not_tls", connect_listed, None, Bytes::new(), Bytes::new());
                return tunnel::run(rec, &b.flags, &b.quiet, c, client, up).await;
            }
        },
    };
    let hello = match connect::peek_client_hello(&mut client, first).await {
        Hello::Parsed(hello) => hello,
        Hello::TooBig(bytes) => {
            let c = tc("hello_too_big", connect_listed, None, bytes, Bytes::new());
            return tunnel::run(rec, &b.flags, &b.quiet, c, client, up).await;
        }
        Hello::NotTls(bytes) if is_listed && looks_like_request(&bytes) => {
            // Plain HTTP to a listed host: decrypted with no TLS; the wire is the plaintext.
            let (source, kind) = connect_listed.unzip();
            let meta = ConnMeta {
                host,
                port,
                mode: "decrypt",
                listed: source,
                kind,
                reason: None,
                alpn: None,
                tls: None,
                fingerprint: None,
            };
            let (conn, p) = decrypt(&b, meta, (client, up), bytes, false).await;
            let (reason, pu, pd) = close(p, |c| c);
            let tail = json!({"up": pu, "down": pd, "plain_up": pu, "plain_down": pd});
            return conn.end(rec.now(), reason, Some(tail));
        }
        Hello::NotTls(bytes) | Hello::Closed(bytes) => {
            let c = tc("not_tls", connect_listed, None, bytes, Bytes::new());
            return tunnel::run(rec, &b.flags, &b.quiet, c, client, up).await;
        }
    };
    let (name, decision) = b.hosts.decide(hello.sni.as_deref(), host);
    let name = name.to_owned();
    let (fp, fp16) = (hello.fingerprint, Some(fingerprint_hex16(&hello.fingerprint)));
    let offer = tls::upstream_offer(&hello.alpn);
    let reason = match decision {
        Decision::Never => Some("sign_in"),
        Decision::NotListed => Some("not_listed"),
        Decision::Listed { pinned: true, .. } => Some("pinned"),
        _ if b.hosts.refused(rec.now(), &name, &fp) => Some("tls_refused"),
        _ if offer == Offer::Other => Some("alpn_other"),
        _ => None,
    };
    let dl = listed(decision);
    if let Some(reason) = reason {
        let c = tc(reason, dl, fp16, hello.bytes, Bytes::new());
        return tunnel::run(rec, &b.flags, &b.quiet, c, client, up).await;
    }
    // Upstream first, checked against the deciding name (C10). Any failure: a fresh TCP leg and a
    // tunnel, so the harness meets the real server itself.
    let connector = b.tls.client(offer).map(tokio_rustls::TlsConnector::from);
    let upstream_tls = match (connector, ServerName::try_from(name.clone())) {
        (Some(c), Ok(server_name)) => c.connect(server_name, up).await,
        _ => Err(io::Error::other("no upstream TLS for this name")),
    };
    let up_tls = match upstream_tls {
        // An `h2`-only client whose upstream picked no ALPN: the legs would not agree.
        Ok(t) if offer == Offer::H2 && t.get_ref().1.alpn_protocol().is_none() => {
            Err(UpstreamFail::AlpnRejected)
        }
        Ok(t) => Ok(t),
        Err(e) => Err(tls::classify_upstream_handshake(&e)),
    };
    let up_tls = match up_tls {
        Ok(t) => t,
        Err(fail) => {
            let reason = match fail {
                UpstreamFail::AlpnRejected => "alpn_other",
                UpstreamFail::Untrusted => "upstream_untrusted",
            };
            let c = tc(reason, dl, fp16, hello.bytes, Bytes::new());
            return fresh_tunnel(&b, c, client, target).await;
        }
    };
    let chosen = up_tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
    let server = match hello.sni {
        Some(_) => Ok(b.tls.server(chosen.as_deref())),
        None => b.tls.server_for(chosen.as_deref(), host),
    };
    let Ok(server) = server else {
        // No leaf can be made for the name: the harness meets the real server itself.
        drop(up_tls);
        let c = tc("upstream_untrusted", dl, fp16, hello.bytes, Bytes::new());
        return fresh_tunnel(&b, c, client, target).await;
    };
    // Harness-leg TCP bytes; the ClientHello was read before the counter, so it is added.
    let wire = Arc::new([AtomicU64::new(0), AtomicU64::new(0)]);
    let hello_len = hello.bytes.len() as u64;
    let wire_now = || (hello_len + wire[0].load(Relaxed), wire[1].load(Relaxed));
    let counted = Counted {
        inner: client,
        n: wire.clone(),
    };
    let opened = rec.now();
    let acceptor = tokio_rustls::TlsAcceptor::from(server);
    let client_tls = match acceptor.accept(PrefixedIo::new(hello.bytes, counted)).await {
        Ok(t) => t,
        Err(e) => {
            // Opened only now, so its meta tells the outcome. Only an explicit alert teaches.
            let alert = match tls::classify_client_handshake(&e) {
                Refusal::Alert(alert) => {
                    b.hosts.learn(rec, rec.now(), &name, &fp);
                    json!(alert)
                }
                Refusal::NoAlert => json!(false),
            };
            let c = tc("tls_refused", dl, fp16, Bytes::new(), Bytes::new());
            let tail = Some(json!({ "alert": alert }));
            return lost_tunnel(rec, &c, opened, wire_now(), tail);
        }
    };
    b.hosts.trust(&name, &fp);
    let (source, kind) = dl.unzip();
    let meta = ConnMeta {
        host,
        port,
        mode: "decrypt",
        listed: source,
        kind,
        reason: None,
        alpn: chosen.as_deref().and_then(|a| std::str::from_utf8(a).ok()),
        tls: match client_tls.get_ref().1.protocol_version() {
            Some(rustls::ProtocolVersion::TLSv1_3) => Some("1.3"),
            Some(rustls::ProtocolVersion::TLSv1_2) => Some("1.2"),
            _ => None,
        },
        fingerprint: fp16.as_deref(),
    };
    let h2 = chosen.as_deref() == Some(b"h2");
    let (conn, p) = decrypt(&b, meta, (client_tls, up_tls), Bytes::new(), h2).await;
    let (reason, plain_up, plain_down) = close(p, |t| t.into_inner().0.into_inner().inner);
    let (up, down) = wire_now();
    let tail = json!({"up": up, "down": down, "plain_up": plain_up, "plain_down": plain_down});
    conn.end(rec.now(), reason, Some(tail));
}

/// Opens the decrypted `conn` and relays it through its parser.
async fn decrypt<C, U>(
    b: &Boot,
    meta: ConnMeta<'_>,
    legs: (C, U),
    to_up: Bytes,
    h2: bool,
) -> (StreamHandle, Pumped<C, U>)
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
    U: AsyncRead + AsyncWrite + Unpin + Send,
{
    let meta = serde_json::to_value(meta).unwrap_or_default();
    let (rec, prefixes) = (&b.rec, (to_up, Bytes::new()));
    let mut conn = rec.open(StreamKind::Conn, 0, 0, 0, OpenFlags::DECRYPTED, meta);
    let parser: Box<dyn Parser> = match h2 {
        true => Box::new(H2Unparsed),
        false => Box::new(http1::Http1::new(rec, conn.sid())),
    };
    let p = pump::relay(rec, &mut conn, legs, prefixes, Some(parser), &b.quiet, None).await;
    (conn, p)
}

/// Rule 8: after an upstream failure the harness leg is reset; otherwise both legs are dropped,
/// the harness leg never shut down (`Pumped::client`). Gives the END reason and plaintext counts.
fn close<C, U>(p: Pumped<C, U>, tcp: impl FnOnce(C) -> TcpStream) -> (EndReason, u64, u64) {
    let out = (p.reason, p.plain_up, p.plain_down);
    if p.upstream_failed {
        connect::reset(tcp(p.client));
    }
    out
}

/// The upstream handshake failed: a fresh TCP connection and a tunnel. If that connect fails
/// too, the harness leg is reset and the conn recorded as the tunnel with its loss.
async fn fresh_tunnel(b: &Boot, c: TunnelConn, client: TcpStream, target: Option<SocketAddr>) {
    let rec = &b.rec;
    let opened = rec.now();
    match connect::connect_upstream(rec, &c.host, c.port, true, target).await {
        Ok(up) => tunnel::run(rec, &b.flags, &b.quiet, c, client, up).await,
        Err(_) => {
            connect::reset(client);
            let up = c.client_prefix.len() as u64;
            lost_tunnel(rec, &c, opened, (up, 0), None);
        }
    }
}

/// A tunnel `conn` whose bytes never passed: the GAP its reason names on a listed host, then END.
fn lost_tunnel(rec: &Recorder, c: &TunnelConn, at: Mono, (up, down): (u64, u64), tail: Option<Value>) {
    let meta = ConnMeta {
        host: &c.host,
        port: c.port,
        mode: "tunnel",
        listed: c.listed.map(|l| l.0),
        kind: c.listed.map(|l| l.1),
        reason: Some(c.reason),
        alpn: None,
        tls: None,
        fingerprint: c.fingerprint.as_deref(),
    };
    let meta = serde_json::to_value(meta).unwrap_or_default();
    let mut conn = rec.open(StreamKind::Conn, 0, 0, 0, OpenFlags::TUNNEL, meta);
    if c.listed.is_some()
        && let Some(reason) = goalcup_format::tunnel_gap(c.reason)
    {
        conn.gap(reason, at, Some(up + down), tail);
    }
    let tail = Some(json!({"up": up, "down": down}));
    conn.end(rec.now(), EndReason::Interrupted, tail);
}

/// Until the HTTP/2 parser lands: an `h2` connection relays every byte and names its loss (5a
/// `h2_unsupported`), never silently.
struct H2Unparsed;

impl Parser for H2Unparsed {
    fn feed(&mut self, _: Dir, t: Mono, bytes: &[u8]) -> Result<(), Stop> {
        Err(stop_h2(t, bytes.len(), false))
    }

    fn end(self: Box<Self>, t: Mono, _: EndReason, _: bool, unsent: &[u8]) -> Option<Stop> {
        (!unsent.is_empty()).then(|| stop_h2(t, unsent.len(), true))
    }

    fn panicked(self: Box<Self>, t: Mono) -> Stop {
        Stop::panic(0, t)
    }
}

fn stop_h2(since: Mono, held: usize, at_close: bool) -> Stop {
    let (reason, tail, held) = (GapReason::H2Unsupported, None, held as u64);
    Stop {
        reason,
        tail,
        held,
        since,
        at_close,
    }
}

/// The harness leg's TCP bytes: read (`n[0]`) and written (`n[1]`).
struct Counted<T> {
    inner: T,
    n: Arc<[AtomicU64; 2]>,
}

impl<T: AsyncRead + Unpin> AsyncRead for Counted<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let r = Pin::new(&mut self.inner).poll_read(cx, buf);
        self.n[0].fetch_add((buf.filled().len() - before) as u64, Relaxed);
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
        if let Poll::Ready(Ok(k)) = r {
            self.n[1].fetch_add(k as u64, Relaxed);
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
