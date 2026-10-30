//! Tunnels (PRD-03 section 9): every connection that is not decrypted is a `conn` stream with
//! `mode: "tunnel"` and a reason, opened when the tunnel starts and ended at close with its byte
//! counts. A tunnel to a listed host is a named loss. Tunnels to unlisted hosts feed the
//! per-host `long_tunnel` flag. Bytes pass through [`pump::run`] with no parser.

use crate::connector::hosts::{Kind, Source};
use crate::pipeline::{EndReason, Mono, OpenFlags, Recorder, StreamKind};
use crate::proxy::connect::{self, CLIENT_HELLO_GUARD};
use crate::proxy::pump;
use crate::proxy::ws::Dir;
use bytes::Bytes;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use tokio::net::TcpStream;

/// The flag's sliding window, in seconds.
const WINDOW: u64 = 600;
/// Rule 1: one tunnel open this long ...
const STREAM_OPEN_NS: u64 = 60_000_000_000;
/// ... that received bytes in at least this many different seconds.
const STREAM_SECS: usize = 10;
/// Rule 2: bytes sent up to one host within the window.
const UPLOAD: u64 = 1 << 20;

/// One tunnel as the decision left it.
pub struct TunnelConn {
    pub host: String,
    pub port: u16,
    /// A `conn` reason from PRD-01's list (`not_listed`, `sign_in`, `hello_too_big`, ...).
    pub reason: &'static str,
    pub listed: Option<(Source, Kind)>,
    /// The first 16 hex digits of the ClientHello fingerprint, when one was read.
    pub fingerprint: Option<String>,
}

/// The `conn` OPEN meta (PRD-01 section 4), for both modes.
#[allow(clippy::too_many_arguments)]
pub fn conn_meta(
    host: &str,
    port: u16,
    mode: &str,
    listed: Option<(Source, Kind)>,
    reason: Option<&str>,
    alpn: Option<&str>,
    tls: Option<&str>,
    fingerprint: Option<&str>,
) -> Value {
    json!({"host": host, "port": port, "mode": mode, "listed": listed.map(|l| l.0),
        "kind": listed.map(|l| l.1), "reason": reason, "alpn": alpn, "tls": tls,
        "fingerprint": fingerprint})
}

/// The per-host `long_tunnel` flag, per boot, shared by every tunnel.
#[derive(Default)]
pub struct Flags {
    hosts: Mutex<HashMap<String, HostWindow>>,
}

#[derive(Default)]
struct HostWindow {
    noted: bool,
    conns: u64,
    /// Per second within the window: (second, bytes up, bytes down).
    secs: VecDeque<(u64, u64, u64)>,
}

/// One tunnel's own view for rule 1: when it opened and the seconds it received bytes in.
pub struct Watch {
    host: String,
    opened: Mono,
    secs: VecDeque<u64>,
}

impl Flags {
    /// Starts watching one tunnel to `host` that opened at `opened`.
    pub fn watch(&self, host: &str, opened: Mono) -> Watch {
        let mut hosts = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        hosts.entry(host.to_owned()).or_default().conns += 1;
        Watch { host: host.to_owned(), opened, secs: VecDeque::new() }
    }

    /// `n` bytes went `dir` on the watched tunnel at `now` (rule 9: `Recorder::now()`, read on
    /// the relay's own events). Writes at most one note per host per boot.
    pub fn observe(&self, rec: &Recorder, w: &mut Watch, now: Mono, dir: Dir, n: u64) {
        let sec = now.ns() / 1_000_000_000;
        let mut hosts = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        let Some(h) = hosts.get_mut(&w.host).filter(|h| !h.noted) else {
            return;
        };
        let (up, down) = if dir == Dir::Up { (n, 0) } else { (0, n) };
        if down > 0 && w.secs.back() != Some(&sec) {
            w.secs.push_back(sec);
        }
        match h.secs.back_mut() {
            Some(last) if last.0 == sec => {
                last.1 += up;
                last.2 += down;
            }
            _ => h.secs.push_back((sec, up, down)),
        }
        while w.secs.front().is_some_and(|&s| s + WINDOW <= sec) {
            w.secs.pop_front();
        }
        while h.secs.front().is_some_and(|s| s.0 + WINDOW <= sec) {
            h.secs.pop_front();
        }
        let (up, down) = h.secs.iter().fold((0, 0), |a, s| (a.0 + s.1, a.1 + s.2));
        let why = if now.ns().saturating_sub(w.opened.ns()) >= STREAM_OPEN_NS
            && w.secs.len() >= STREAM_SECS
        {
            "stream"
        } else if up >= UPLOAD {
            "upload"
        } else {
            return;
        };
        let secs = h.secs.iter().filter(|s| s.2 > 0).count();
        h.noted = true;
        h.secs = VecDeque::new();
        let ev = json!({"ev": "note", "what": "long_tunnel", "host": w.host, "why": why,
            "conns": h.conns, "up": up, "down": down, "secs": secs});
        drop(hosts);
        rec.event(ev);
    }
}

/// Relays one tunnel to its end: the prefixes are replayed first (`client_prefix` upstream,
/// `upstream_prefix` to the client); an upstream failure reaches the client as a reset (rule 8).
/// The END tail counts the TCP bytes each way, prefixes included.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    rec: &Recorder,
    flags: &Flags,
    quiet: &AtomicU64,
    c: TunnelConn,
    client: TcpStream,
    client_prefix: Bytes,
    upstream: TcpStream,
    upstream_prefix: Bytes,
) {
    let opened = rec.now();
    let meta = conn_meta(
        &c.host,
        c.port,
        "tunnel",
        c.listed,
        Some(c.reason),
        None,
        None,
        c.fingerprint.as_deref(),
    );
    let mut conn = rec.open(StreamKind::Conn, 0, 0, 0, OpenFlags::TUNNEL, meta);
    let watch = (c.listed.is_none() && c.reason != "sign_in")
        .then(|| Mutex::new(flags.watch(&c.host, opened)));
    let tap = |dir, t, n| {
        if let Some(w) = &watch {
            flags.observe(rec, &mut w.lock().unwrap_or_else(|e| e.into_inner()), t, dir, n);
        }
    };
    let prefixes = (client_prefix, upstream_prefix);
    let legs = (client, upstream);
    let p = pump::relay(rec, &mut conn, legs, prefixes, None, quiet, Some(&tap)).await;
    let (up, down) = (p.plain_up, p.plain_down);
    if p.upstream_failed {
        connect::reset(p.client);
    } else {
        drop(p.client);
    }
    drop(p.upstream);
    if c.listed.is_some()
        && let Some(reason) = goalcup_format::tunnel_gap(c.reason)
    {
        let tail = (c.reason == "hello_too_big")
            .then(|| json!({"guard": "client_hello", "limit": CLIENT_HELLO_GUARD}));
        conn.gap(reason, opened, Some(up + down), tail);
    }
    let reason = if p.upstream_failed { EndReason::Interrupted } else { EndReason::Complete };
    conn.end(rec.now(), reason, Some(json!({"up": up, "down": down})));
}
