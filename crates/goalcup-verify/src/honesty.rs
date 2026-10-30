//! Honesty (PRD-08 section 3): `listed_hosts_decrypted`, `conn_bytes` and `mask_failclosed`, fed
//! one record at a time from integrity's pass. State is a few words per open connection or stream,
//! dropped at its END and at a new boot. A record of a stream never seen open is ignored here
//! (integrity reports it); a crafted record must not panic (L-VR3).

use crate::integrity::{At, Check, Res};
use goalcup_format::{GapReason, Item, Record, StreamKind};
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
    /// A tunnel to a host that earlier finished a decrypted connection with this fingerprint.
    after_trust: bool,
    gaps: u64,
    lost: u64,
    /// A GAP whose reason names a tunnel's loss.
    named: bool,
    /// A GAP tail says `alert: false`: nothing was learned from the refusal.
    quiet: bool,
    /// Plain bytes its streams account for: up, down.
    acc: [u64; 2],
}

#[derive(Default)]
pub struct Honesty {
    boot: Option<u64>,
    conns: HashMap<u64, Conn>,
    /// Open streams of a connection: its sid, direction (0 up, 1 down), WebSocket.
    subs: HashMap<u64, (u64, usize, bool)>,
    masks: HashMap<u64, u64>,
    trusted: HashSet<(String, String)>,
    listed: BTreeSet<String>,
    long: BTreeSet<String>,
    refused: BTreeSet<(String, String)>,
    hosts: BTreeSet<String>,
    named: BTreeMap<String, u64>,
    n: BTreeMap<&'static str, u64>,
    lh: Check,
    cb: Check,
    mf: Check,
}

fn obj(b: &[u8]) -> Map<String, Value> {
    serde_json::from_slice(b).unwrap_or_default()
}

impl Honesty {
    fn add(&mut self, k: &'static str, by: u64) {
        let v = self.n.entry(k).or_default();
        *v = v.saturating_add(by);
    }

    pub(crate) fn item(&mut self, it: &Item, at: At, mask_rules: &[String]) {
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
                if let Some(&(c, d, true)) = self.subs.get(sid)
                    && let Some(c) = self.conns.get_mut(&c)
                {
                    c.acc[d] = c.acc[d].saturating_add(bytes.len() as u64);
                }
            }
            Record::Gap {
                sid,
                reason,
                lost,
                tail,
                ..
            } => self.gap(*sid, *reason, *lost, &obj(tail), at),
            Record::End {
                sid, seen, tail, ..
            } => self.end(*sid, *seen, &obj(tail), at),
            Record::Mask { sid, len, rule, .. } => {
                let rule = usize::try_from(*rule).ok().and_then(|i| mask_rules.get(i));
                if rule.is_some_and(|r| r == "fail_closed") {
                    self.add("bytes", *len);
                    let s = self.masks.entry(*sid).or_default();
                    let (before, now) = (*s, s.saturating_add(*len));
                    *s = now;
                    let top = self.n.entry("largest").or_default();
                    *top = (*top).max(now);
                    if before <= MIB && now > MIB {
                        self.add("streams", 1);
                        self.mf.raise(Res::Warn, "", Some(at));
                    }
                }
            }
            Record::Event { json, .. } => {
                let e = obj(json);
                let is = |k: &str, v: &str| e.get(k).and_then(Value::as_str) == Some(v);
                if is("ev", "note")
                    && is("what", "long_tunnel")
                    && let Some(h) = e.get("host").and_then(Value::as_str)
                {
                    self.long.insert(h.into());
                }
            }
            _ => {}
        }
    }

    fn open(&mut self, sid: u64, kind: StreamKind, conn: u64, m: &Map<String, Value>) {
        let s = |k: &str| m.get(k).and_then(Value::as_str);
        if kind == StreamKind::Conn {
            let mut c = Conn {
                host: s("host").unwrap_or_default().into(),
                reason: s("reason").unwrap_or("unknown").into(),
                fp: s("fingerprint").map(Into::into),
                tunnel: s("mode") == Some("tunnel"),
                listed: s("listed").is_some(),
                ..Default::default()
            };
            if c.listed {
                self.add("listed_conns", 1);
                self.listed.insert(c.host.clone());
                if !c.tunnel {
                    self.add("decrypted", 1);
                } else if c.reason == "sign_in" {
                    self.add("sign_in", 1);
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
        // A head is approximate on purpose: each name and value, 4 bytes of framing, 32 for the
        // start line.
        let len = |v: &Value| v.as_str().map_or(0, |s| s.len() as u64);
        let pair = |p: &Value| {
            let both = p.as_array().map_or(0, |p| p.iter().map(len).sum::<u64>());
            both.saturating_add(4)
        };
        let heads = m.get("headers").and_then(Value::as_array);
        let head = heads.map_or(0, |h| h.iter().map(pair).sum::<u64>());
        c.acc[dir] = c.acc[dir].saturating_add(if ws { 0 } else { head.saturating_add(32) });
        self.subs.insert(sid, (conn, dir, ws));
    }

    fn gap(&mut self, sid: u64, reason: GapReason, lost: u64, tail: &Map<String, Value>, at: At) {
        let Some(c) = self.conns.get_mut(&sid) else {
            return;
        };
        c.gaps = c.gaps.saturating_add(1);
        c.lost = c.lost.saturating_add(lost.saturating_sub(1));
        c.named |= matches!(
            reason,
            GapReason::MemoryGuard
                | GapReason::TlsRefused
                | GapReason::Pinned
                | GapReason::UpstreamUntrusted
                | GapReason::ProtocolOther
        );
        c.quiet |= tail.get("alert") == Some(&Value::Bool(false));
        if c.listed {
            if !c.tunnel {
                self.hosts.insert(c.host.clone());
                self.lh.raise(Res::Named, "", Some(at));
            }
            self.add("conn_gaps", 1);
        }
    }

    fn end(&mut self, sid: u64, seen: u64, tail: &Map<String, Value>, at: At) {
        self.masks.remove(&sid);
        if let Some((conn, dir, ws)) = self.subs.remove(&sid) {
            if let (false, Some(c)) = (ws, self.conns.get_mut(&conn)) {
                let wire = tail.get("wire").and_then(Value::as_u64).unwrap_or(seen);
                c.acc[dir] = c.acc[dir].saturating_add(wire);
            }
            return;
        }
        let Some(c) = self.conns.remove(&sid) else {
            return;
        };
        if c.tunnel {
            self.tunnel_end(&c, at);
        } else {
            if let Some(fp) = &c.fp {
                self.trusted.insert((c.host.clone(), fp.clone()));
            }
            self.plain_end(&c, tail, at);
        }
    }

    /// A listed tunnel is judged at its END: a sign-in is no loss, a named GAP is `named`, none is
    /// a silent loss.
    fn tunnel_end(&mut self, c: &Conn, at: At) {
        if c.after_trust && !c.quiet {
            self.lh.raise(Res::Warn, "", Some(at));
            self.refused
                .insert((c.host.clone(), c.fp.clone().unwrap_or_default()));
        }
        if !c.listed || c.reason == "sign_in" {
            return;
        }
        self.hosts.insert(c.host.clone());
        if c.named {
            *self.named.entry(c.reason.clone()).or_default() += 1;
            self.lh.raise(Res::Named, "", Some(at));
        } else {
            self.add("tunnels_unnamed", 1);
            self.lh.raise(Res::Fail, "", Some(at));
        }
    }

    /// A decrypted connection's plain bytes against what its streams account for, per direction.
    fn plain_end(&mut self, c: &Conn, tail: &Map<String, Value>, at: At) {
        let mut worst = None;
        for (d, key) in ["plain_up", "plain_down"].into_iter().enumerate() {
            let Some(plain) = tail.get(key).and_then(Value::as_u64) else {
                continue;
            };
            let un = plain.saturating_sub(c.acc[d].saturating_add(c.lost));
            self.add("unaccounted_bytes", un);
            let (big, un_x) = (un > MIB, u128::from(un));
            let res = if big && un_x * 2 > u128::from(plain) && c.gaps == 0 {
                Res::Fail
            } else if big && un_x * 10 > u128::from(plain) {
                Res::Warn
            } else {
                Res::Pass
            };
            worst = worst.max(Some(res));
            self.cb.raise(res, "", Some(at));
        }
        if let Some(w) = worst {
            self.add("checked", 1);
            match w {
                Res::Fail => self.add("fail", 1),
                Res::Warn => self.add("warn", 1),
                _ => {}
            }
        }
    }

    pub fn finish(mut self) -> Vec<Check> {
        let n = std::mem::take(&mut self.n);
        let g = |k: &str| n.get(k).copied().unwrap_or(0);
        let long: Vec<&String> = (self.long.iter())
            .filter(|h| !self.listed.contains(*h))
            .collect();
        if !long.is_empty() {
            self.lh.raise(Res::Warn, "", None);
        }
        let refused: Vec<Value> = (self.refused.iter())
            .map(|(host, fingerprint)| json!({"host": host, "fingerprint": fingerprint}))
            .collect();
        let hosts = (self.hosts.iter().chain(long.iter().copied())).cloned();
        let hosts = hosts.collect::<Vec<_>>().join(", ");
        let blind = self.named.values().sum::<u64>() + g("tunnels_unnamed");
        let whys = [
            format!(
                "{} listed connections: {} decrypted, {} sign-in, {blind} blind ({} with no GAP), \
                 {} conn GAPs, {} long tunnels to unlisted hosts, {} refusals after trust; \
                 hosts: {hosts}",
                g("listed_conns"),
                g("decrypted"),
                g("sign_in"),
                g("tunnels_unnamed"),
                g("conn_gaps"),
                long.len(),
                refused.len()
            ),
            format!(
                "{} of {} decrypted connections have bytes no stream accounts for ({} bytes)",
                g("warn") + g("fail"),
                g("checked"),
                g("unaccounted_bytes")
            ),
            format!(
                "{} streams have over 1 MiB of fail-closed masking ({} bytes in all)",
                g("streams"),
                g("bytes")
            ),
        ];
        let facts = [
            json!({"listed_conns": g("listed_conns"), "decrypted": g("decrypted"),
                "sign_in": g("sign_in"), "tunnels_named": self.named,
                "tunnels_unnamed": g("tunnels_unnamed"), "conn_gaps": g("conn_gaps"),
                "long_tunnel_unlisted": long, "refusal_after_trust": refused}),
            json!({"checked": g("checked"), "warn": g("warn"), "fail": g("fail"),
                "unaccounted_bytes": g("unaccounted_bytes")}),
            json!({"streams": g("streams"), "bytes": g("bytes"), "largest": g("largest")}),
        ];
        let ids = ["listed_hosts_decrypted", "conn_bytes", "mask_failclosed"];
        let mut out = vec![self.lh, self.cb, self.mf];
        for (((c, id), why), f) in out.iter_mut().zip(ids).zip(whys).zip(facts) {
            c.id = id;
            c.facts = f.as_object().cloned().unwrap_or_default();
            if c.result > Res::Pass {
                c.why = why;
            }
        }
        out
    }
}
