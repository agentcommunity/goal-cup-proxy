//! Honesty (PRD-08 section 3): `listed_hosts_decrypted`, `conn_bytes` and `mask_failclosed`, fed
//! one record at a time from integrity's pass. State is a few words per open connection or stream,
//! dropped at its END and at a new boot. A record of a stream never seen open is ignored here
//! (integrity reports it); a crafted record must not panic (L-VR3).

use crate::integrity::{At, Check, Res, has_extent};
use crate::legs::Cover;
use goalcup_format::{EndReason, GapReason, Item, Record, StreamKind};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const MIB: u64 = 1 << 20;

/// One open client connection.
#[derive(Default)]
struct Conn {
    host: String,
    reason: String,
    fp: Option<String>,
    tunnel: bool,
    listed: bool,
    /// A model host: listed with kind provider or gateway, or named by the participant or the
    /// config. A GAP on this connection covers harness calls.
    model: bool,
    /// A tunnel to a host that earlier finished a decrypted connection with this fingerprint.
    after_trust: bool,
    /// GAPs on the connection or on one of its streams: a loss `conn_bytes` need not guess at.
    gaps: u64,
    lost: u64,
    /// A GAP names this tunnel's own reason (PRD-01 section 4).
    gap_named: bool,
    /// A GAP tail says `alert: false`: nothing was learned from the refusal.
    quiet: bool,
    /// Plain bytes its streams account for: up, down.
    acc: [u64; 2],
}

/// An open stream of a connection: the connection's sid, direction (0 up, 1 down), WebSocket.
struct Sub {
    conn: u64,
    dir: usize,
    ws: bool,
}

#[derive(Default)]
struct Counts {
    listed: u64,
    decrypted: u64,
    sign_in: u64,
    conn_gaps: u64,
    unnamed: u64,
    checked: u64,
    warn: u64,
    fail: u64,
    no_plain_tail: u64,
    unaccounted: u64,
    streams: u64,
    mask_bytes: u64,
}

struct Checks {
    listed: Check,
    bytes: Check,
    mask: Check,
}

impl Default for Checks {
    fn default() -> Self {
        let new = |id| Check::new(id, None, Res::Pass, "");
        Checks {
            listed: new("listed_hosts_decrypted"),
            bytes: new("conn_bytes"),
            mask: new("mask_failclosed"),
        }
    }
}

#[derive(Default)]
pub struct Honesty {
    boot: Option<u64>,
    conns: HashMap<u64, Conn>,
    subs: HashMap<u64, Sub>,
    /// Fail-closed MASK bytes per open stream.
    masks: HashMap<u64, u64>,
    /// The `start` event's host list: exact names and `*.suffix` patterns, with their kinds and
    /// the source that listed them.
    on_list: Vec<(String, String, String)>,
    /// Wall time in which a loss is named, for the legs checks.
    pub(crate) covers: Vec<Cover>,
    trusted: HashSet<(String, String)>,
    listed: BTreeSet<String>,
    long: BTreeSet<String>,
    refused: BTreeSet<(String, String)>,
    hosts: BTreeSet<String>,
    tunnels_named: BTreeMap<String, u64>,
    n: Counts,
    checks: Checks,
}

fn obj(b: &[u8]) -> Map<String, Value> {
    serde_json::from_slice(b).unwrap_or_default()
}

fn inc(x: &mut u64, by: u64) {
    *x = x.saturating_add(by);
}

/// A head is approximate on purpose: each name and value, 4 bytes of framing, 32 for the start
/// line.
fn head_size(m: &Map<String, Value>) -> u64 {
    let len = |v: &Value| v.as_str().map_or(0, |s| s.len() as u64);
    let pair = |p: &Value| p.as_array().map_or(0, |p| p.iter().map(len).sum::<u64>()) + 4;
    let heads = m.get("headers").and_then(Value::as_array);
    heads.into_iter().flatten().map(pair).sum::<u64>() + 32
}

/// Whether a GAP of `gap` names a listed tunnel of `reason` (PRD-01 section 4). A reason this
/// build does not know takes any of the five.
fn names_tunnel(reason: &str, gap: GapReason) -> bool {
    use GapReason::*;
    match reason {
        "tls_refused" => gap == TlsRefused,
        "pinned" => gap == Pinned,
        "upstream_untrusted" => gap == UpstreamUntrusted,
        "alpn_other" | "not_tls" => gap == ProtocolOther,
        "hello_too_big" => gap == MemoryGuard,
        _ => matches!(
            gap,
            MemoryGuard | TlsRefused | Pinned | UpstreamUntrusted | ProtocolOther
        ),
    }
}

impl Honesty {
    pub(crate) fn item(&mut self, it: &Item, at: At, ev: &Map<String, Value>) {
        match &it.rec {
            Record::Part { boot, .. } if self.boot != Some(*boot) => {
                self.boot = Some(*boot);
                (self.conns, self.subs, self.masks) = Default::default();
            }
            Record::Open {
                sid,
                skind,
                conn,
                meta,
                ..
            } => self.open(*sid, *skind, *conn, &obj(meta)),
            Record::Data { sid, bytes, .. } => {
                if let Some(&Sub {
                    conn,
                    dir,
                    ws: true,
                }) = self.subs.get(sid)
                    && let Some(c) = self.conns.get_mut(&conn)
                {
                    inc(&mut c.acc[dir], bytes.len() as u64);
                }
            }
            Record::Gap {
                sid,
                reason,
                start_wall,
                end_wall,
                start_mono,
                end_mono,
                lost,
                tail,
                ..
            } => {
                let tail = obj(tail);
                // A GAP with no extent (gap_has_end) covers nothing.
                let span = has_extent(
                    &tail,
                    (*start_wall, *end_wall),
                    (*start_mono, *end_mono),
                    *lost,
                )
                .then_some(Cover {
                    start_wall: *start_wall,
                    end_wall: *end_wall,
                    down: false,
                });
                self.gap(*sid, *reason, *lost, &tail, at, span);
            }
            Record::End {
                sid,
                reason,
                seen,
                tail,
                ..
            } => self.end(*sid, *reason, *seen, &obj(tail), at),
            Record::Event { .. } => self.event(ev),
            _ => {}
        }
    }

    fn event(&mut self, e: &Map<String, Value>) {
        let is = |k: &str, v: &str| e.get(k).and_then(Value::as_str) == Some(v);
        if is("ev", "note")
            && is("what", "long_tunnel")
            && let Some(h) = e.get("host").and_then(Value::as_str)
        {
            self.long.insert(h.into());
        } else if is("ev", "start")
            && let Some(Value::Object(by_source)) = e.get("hosts")
        {
            let all = by_source
                .iter()
                .filter_map(|(src, hs)| Some((src, hs.as_array()?)))
                .flat_map(|(src, hs)| hs.iter().map(move |h| (src, h)));
            let names = all.filter_map(|(src, h)| {
                let kind = h.get("kind").and_then(Value::as_str);
                Some((h.get("host")?.as_str()?, kind.unwrap_or_default(), src))
            });
            self.on_list = (names.map(|(h, k, s)| (h.into(), k.into(), s.clone()))).collect();
        }
    }

    /// A fail-closed MASK of `len` bytes on stream `sid`; integrity has resolved the rule.
    pub(crate) fn mask(&mut self, sid: u64, len: u64, at: At) {
        let s = self.masks.entry(sid).or_default();
        let before = *s;
        *s = s.saturating_add(len);
        inc(&mut self.n.mask_bytes, len);
        if before <= MIB && *s > MIB {
            inc(&mut self.n.streams, 1);
            self.checks.mask.raise(Res::Warn, "", Some(at));
        }
    }

    /// The kind and source the `start` event's host list gives `host` (the kind is empty when it
    /// says none), or `None` when the list does not hold it.
    fn list_kind(&self, host: &str) -> Option<(&str, &str)> {
        let hit = |(p, ..): &&(String, String, String)| {
            let suffix = p.strip_prefix('*');
            p == host || suffix.is_some_and(|s| host.len() > s.len() && host.ends_with(s))
        };
        (self.on_list.iter().find(hit)).map(|(_, k, s)| (k.as_str(), s.as_str()))
    }

    fn open(&mut self, sid: u64, kind: StreamKind, conn: u64, m: &Map<String, Value>) {
        let s = |k: &str| m.get(k).and_then(Value::as_str);
        if kind == StreamKind::Conn {
            let host = s("host").unwrap_or_default();
            let (on, listed) = (self.list_kind(host), s("listed").is_some());
            let (on_kind, on_src) = on.unwrap_or_default();
            let kind = s("kind").unwrap_or(on_kind);
            let src = s("listed").unwrap_or(on_src);
            // A host the participant or the config listed is the harness's model host by their
            // own word, whatever kind it carries (PRD-01 section 7: always `other`).
            let model = (listed || on.is_some())
                && (matches!(kind, "provider" | "gateway")
                    || matches!(src, "participant" | "config"));
            let mut c = Conn {
                host: host.into(),
                reason: s("reason").unwrap_or("unknown").into(),
                fp: s("fingerprint").map(Into::into),
                tunnel: s("mode") == Some("tunnel"),
                // The conn's own word, or the start event's list: a recorder that mislabels a
                // listed host as unlisted must not hide a blind relay.
                listed: listed || on.is_some(),
                model,
                ..Default::default()
            };
            if c.listed {
                inc(&mut self.n.listed, 1);
                self.listed.insert(c.host.clone());
                if !c.tunnel {
                    inc(&mut self.n.decrypted, 1);
                } else if c.reason == "sign_in" {
                    inc(&mut self.n.sign_in, 1);
                }
                let key = c.fp.clone().map(|f| (c.host.clone(), f));
                let seen = key.is_some_and(|k| self.trusted.contains(&k));
                c.after_trust = c.reason == "tls_refused" && seen;
            }
            self.conns.insert(sid, c);
            return;
        }
        let dir = match kind {
            StreamKind::HttpReq | StreamKind::WsUp => 0,
            StreamKind::HttpResp | StreamKind::WsDown => 1,
            _ => return,
        };
        let ws = matches!(kind, StreamKind::WsUp | StreamKind::WsDown);
        let Some(c) = self.conns.get_mut(&conn) else {
            return;
        };
        if !ws {
            inc(&mut c.acc[dir], head_size(m));
        }
        self.subs.insert(sid, Sub { conn, dir, ws });
    }

    fn gap(
        &mut self,
        sid: u64,
        reason: GapReason,
        lost: u64,
        tail: &Map<String, Value>,
        at: At,
        span: Option<Cover>,
    ) {
        if sid == 0 && reason == GapReason::RecorderDown {
            self.covers.extend(span.map(|c| Cover { down: true, ..c }));
        }
        // A GAP on a stream of a connection names only its own bytes, so it never waives the
        // `conn_bytes` fail rule for other requests. A WebSocket stream has no END `wire`: the
        // bytes it lost count as accounted for, and a loss of unknown size waives the rule.
        if let Some(&Sub { conn, dir, ws }) = self.subs.get(&sid) {
            if let (true, Some(c)) = (ws, self.conns.get_mut(&conn)) {
                inc(&mut c.acc[dir], lost.saturating_sub(1));
                if lost == 0 {
                    inc(&mut c.gaps, 1);
                }
            }
            return;
        }
        let Some(c) = self.conns.get_mut(&sid) else {
            return;
        };
        inc(&mut c.gaps, 1);
        inc(&mut c.lost, lost.saturating_sub(1));
        c.gap_named |= names_tunnel(&c.reason, reason);
        c.quiet |= tail.get("alert") == Some(&Value::Bool(false));
        // A conn GAP names the harness calls of a model host: a decrypted connection's, or a
        // tunnel's own (PRD-08 section 3). Not a stream GAP, not a vendor's. It names wire calls
        // of that connection, never a subagent with no stream at all (`Cover::down`).
        if c.model
            && reason != GapReason::AfterStop
            && (!c.tunnel || names_tunnel(&c.reason, reason))
        {
            self.covers.extend(span);
        }
        if c.listed && !c.tunnel {
            self.hosts.insert(c.host.clone());
            inc(&mut self.n.conn_gaps, 1);
            self.checks.listed.raise(Res::Named, "", Some(at));
        }
    }

    fn end(&mut self, sid: u64, reason: EndReason, seen: u64, tail: &Map<String, Value>, at: At) {
        self.masks.remove(&sid);
        if let Some(Sub { conn, dir, ws }) = self.subs.remove(&sid) {
            if let (false, Some(c)) = (ws, self.conns.get_mut(&conn)) {
                let wire = tail.get("wire").and_then(Value::as_u64).unwrap_or(seen);
                inc(&mut c.acc[dir], wire);
            }
            return;
        }
        let Some(c) = self.conns.remove(&sid) else {
            return;
        };
        if c.tunnel {
            self.tunnel_end(&c, at);
        } else {
            if let (EndReason::Complete, Some(fp)) = (reason, &c.fp) {
                self.trusted.insert((c.host.clone(), fp.clone()));
            }
            self.plain_end(&c, tail, at);
        }
    }

    /// A listed tunnel is judged at its END: a sign-in is no loss, a GAP of its own reason is
    /// `named`, none is a silent loss.
    fn tunnel_end(&mut self, c: &Conn, at: At) {
        let check = &mut self.checks.listed;
        if c.after_trust && !c.quiet {
            check.raise(Res::Warn, "", Some(at));
            let fp = c.fp.clone().unwrap_or_default();
            self.refused.insert((c.host.clone(), fp));
        }
        if !c.listed || c.reason == "sign_in" {
            return;
        }
        self.hosts.insert(c.host.clone());
        if c.gap_named {
            *self.tunnels_named.entry(c.reason.clone()).or_default() += 1;
            check.raise(Res::Named, "", Some(at));
        } else {
            inc(&mut self.n.unnamed, 1);
            check.raise(Res::Fail, "", Some(at));
        }
    }

    /// A decrypted connection's plain bytes against what its streams account for, per direction.
    fn plain_end(&mut self, c: &Conn, tail: &Map<String, Value>, at: At) {
        let plain = ["plain_up", "plain_down"].map(|k| tail.get(k).and_then(Value::as_u64));
        if plain.contains(&None) {
            inc(&mut self.n.no_plain_tail, 1);
        }
        let mut worst = None;
        for (d, plain) in plain.into_iter().enumerate() {
            let Some(plain) = plain else {
                continue;
            };
            let un = plain.saturating_sub(c.acc[d].saturating_add(c.lost));
            inc(&mut self.n.unaccounted, un);
            let big = un > MIB;
            let res = if big && un > plain / 2 && c.gaps == 0 {
                Res::Fail
            } else if big && un > plain / 10 {
                Res::Warn
            } else {
                Res::Pass
            };
            worst = worst.max(Some(res));
            self.checks.bytes.raise(res, "", Some(at));
        }
        if let Some(w) = worst {
            inc(&mut self.n.checked, 1);
            match w {
                Res::Fail => inc(&mut self.n.fail, 1),
                Res::Warn => inc(&mut self.n.warn, 1),
                _ => {}
            }
        }
    }

    pub fn finish(self) -> Vec<Check> {
        let n = &self.n;
        let long: Vec<&String> = (self.long.iter())
            .filter(|h| !self.listed.contains(*h))
            .collect();
        let Checks {
            mut listed,
            mut bytes,
            mut mask,
        } = self.checks;
        if !long.is_empty() {
            listed.raise(Res::Warn, "", None);
        }
        // Nothing judged, or a decrypted connection with no plain byte count, is no evidence.
        if n.checked == 0 || n.no_plain_tail > 0 {
            let why = match n.no_plain_tail {
                0 => "no decrypted connection to judge".to_string(),
                k => format!("{k} decrypted connections ended with no plain_up or plain_down"),
            };
            bytes.raise(Res::Skip, &why, None);
        }
        let refused: Vec<Value> = (self.refused.iter())
            .map(|(host, fingerprint)| json!({"host": host, "fingerprint": fingerprint}))
            .collect();
        let hosts = (self.hosts.iter().chain(long.iter().copied())).cloned();
        let hosts = hosts.collect::<Vec<_>>().join(", ");
        let blind = self.tunnels_named.values().sum::<u64>() + n.unnamed;
        let why = format!(
            "{} listed connections: {} decrypted, {} sign-in, {blind} blind ({} with no GAP of \
             their reason), {} conn GAPs on decrypted ones, {} long tunnels to unlisted hosts, \
             {} refusals after trust; hosts: {hosts}",
            n.listed,
            n.decrypted,
            n.sign_in,
            n.unnamed,
            n.conn_gaps,
            long.len(),
            refused.len()
        );
        let facts = json!({"listed_conns": n.listed, "decrypted": n.decrypted,
            "sign_in": n.sign_in, "tunnels_named": self.tunnels_named,
            "tunnels_unnamed": n.unnamed, "conn_gaps": n.conn_gaps,
            "long_tunnel_unlisted": long, "refusal_after_trust": refused});
        fill(&mut listed, why, facts);
        let mut why = format!(
            "{} of {} decrypted connections have bytes no stream accounts for ({} bytes)",
            n.warn + n.fail,
            n.checked,
            n.unaccounted
        );
        if n.no_plain_tail > 0 {
            why += &format!(
                "; {} decrypted connections ended with no plain_up or plain_down",
                n.no_plain_tail
            );
        }
        let facts = json!({"checked": n.checked, "warn": n.warn, "fail": n.fail,
            "unaccounted_bytes": n.unaccounted, "no_plain_tail": n.no_plain_tail});
        fill(&mut bytes, why, facts);
        let why = format!(
            "{} streams have over 1 MiB of fail-closed masking ({} bytes in all)",
            n.streams, n.mask_bytes
        );
        fill(
            &mut mask,
            why,
            json!({"streams": n.streams, "bytes": n.mask_bytes}),
        );
        vec![listed, bytes, mask]
    }
}

/// Sets a check's facts, and its one plain line when it is not a pass.
fn fill(c: &mut Check, why: String, facts: Value) {
    c.facts = facts.as_object().cloned().unwrap_or_default();
    if c.result > Res::Skip {
        c.why = why;
    }
}
