//! The wire leg (PRD-09 section 3): one streaming pass over F1 records that turns every wire
//! call of an entry into a [`CallRow`]. Keys are (boot, sid); record times of different boots
//! are never compared.
//!
//! WebSocket pairing: a `response.create` (or warm-up) waits for the next `response.created` on
//! its connection, and `previous_response_id` wins over order when both carry it.
//! Attaching rule (run 1 sent many rate limits as both a stream `error` and a `response.failed`):
//! a terminal or `error` goes to the open call; once that call has failed, the other kind still
//! goes to it when the ids agree, and anything else starts the next waiting create. An id-less
//! `error` after a failure goes to the next waiting create instead when that create was sent after
//! the failure.
//!
//! A WebSocket message still open when its boot ends (a PART of a new boot, or the end of the
//! records) is read as cut (PRD-01 section 7).

use std::collections::{HashMap, VecDeque};

use goalcup_format::{EndReason, FLAG_FIN, Record, StreamKind};
use serde_json::Value;

use super::shapes::{self, Client, End, Reply, Server};
use super::sse::Sse;
use super::{CallRow, Ctx, HostKind, Kind, Leg, Outcome, Shape, Via, host_kind};

type Key = (u64, u64);

/// The wire state of one entry: open streams, open WebSockets and the rows so far. No `Debug`:
/// it holds body bytes, which no panic or log may print.
#[derive(Default)]
pub struct Wire {
    rows: Vec<Row>,
    conns: HashMap<Key, Conn>,
    calls: HashMap<Key, usize>,
    reqs: HashMap<Key, Req>,
    resps: HashMap<Key, Resp>,
    ws: HashMap<Key, Ws>,
    socks: HashMap<Key, Sock>,
    gaps: Vec<Gap>,
}

#[derive(Default)]
struct Row {
    row: CallRow,
    reply: Reply,
    conn: u64,
    status: Option<u16>,
    t0: u64,
    t1: Option<u64>,
    /// The time of the first terminal or `error` message (WebSocket).
    end_t: Option<u64>,
    req_ok: bool,
    resp_ok: bool,
    /// (sid, first offset, one past the last) of the request and the reply; a whole HTTP stream
    /// is (sid, 0, `u64::MAX`).
    up: Option<(u64, u64, u64)>,
    down: Option<(u64, u64, u64)>,
    down_bad: bool,
    seen_terminal: bool,
    seen_error: bool,
    cancelled: bool,
}

#[derive(Clone, Default)]
struct Conn {
    host: Option<String>,
    kind: Option<HostKind>,
    listed: bool,
}

struct Req {
    row: usize,
    path: String,
    body: Buf,
}

struct Resp {
    row: usize,
    sse: Option<Sse>,
    body: Buf,
}

struct Ws {
    sock: Key,
    up: bool,
    msgs: u64,
    /// The open message's bytes; `next` runs on across messages.
    buf: Buf,
    /// The open message's first offset and time.
    start: Option<(u64, u64)>,
    last_t: u64,
    op: u8,
}

/// One WebSocket message: number, socket, sid and offsets, FIN and no hole, first and last time.
struct Msg {
    n: u64,
    sock: Key,
    range: (u64, u64, u64),
    whole: bool,
    t0: u64,
    t1: u64,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct Sock {
    conn: u64,
    pending: VecDeque<(usize, Option<String>)>,
    cur: Option<usize>,
}

struct Gap {
    boot: u64,
    sid: u64,
    off: u64,
    lost: u64,
    mono: (u64, u64),
    wall: (u64, u64),
}

/// A body buffered by offset; `next` is one past the last byte seen, and a hole stops it.
#[derive(Default)]
struct Buf {
    bytes: Vec<u8>,
    next: u64,
    hole: bool,
}

impl Buf {
    /// Moves past `len` bytes at `off`; false, and a hole, when they do not follow on.
    fn step(&mut self, off: u64, len: usize) -> bool {
        let on = off == self.next;
        self.hole |= !on;
        self.next = off.saturating_add(len as u64);
        on
    }

    fn push(&mut self, off: u64, b: &[u8]) {
        self.step(off, b.len());
        if !self.hole {
            self.bytes.extend_from_slice(b);
        }
    }
}

fn meta(m: &[u8]) -> Value {
    serde_json::from_slice(m).unwrap_or(Value::Null)
}

impl Wire {
    /// Takes one record of the boot `ctx` names (PART, LINK and EVENT are read by the sink).
    pub fn push(&mut self, ctx: &Ctx, rec: &Record) {
        let b = ctx.boot;
        match rec {
            Record::Open {
                sid,
                t,
                skind,
                conn,
                req,
                meta: m,
            } => self.open(ctx, (b, *sid), *t, *skind, (*conn, *req), &meta(m)),
            Record::Data {
                sid,
                t,
                off,
                flags,
                bytes,
            } => self.data(ctx, (b, *sid), *t, *off, *flags, bytes),
            Record::End { sid, reason, .. } => {
                self.end(ctx, (b, *sid), *reason == EndReason::Complete)
            }
            Record::Gap {
                sid,
                start_wall,
                end_wall,
                start_mono,
                end_mono,
                off,
                lost,
                ..
            } => self.gaps.push(Gap {
                boot: b,
                sid: *sid,
                off: *off,
                lost: *lost,
                mono: (*start_mono, *end_mono),
                wall: (*start_wall, *end_wall),
            }),
            _ => {}
        }
    }

    fn open(&mut self, ctx: &Ctx, k: Key, t: u64, sk: StreamKind, cr: (u64, u64), m: &Value) {
        let (conn, req) = cr;
        let s = |k: &str| m.get(k).and_then(Value::as_str);
        match sk {
            StreamKind::Conn => {
                let host = s("host").map(str::to_string);
                let found = host.as_deref().and_then(|h| ctx.listed(h));
                let kind = host_kind(s("kind")).or(found.flatten());
                let listed = !m.get("listed").is_none_or(Value::is_null)
                    || s("kind").is_some()
                    || found.is_some();
                self.conns.insert(k, Conn { host, kind, listed });
            }
            StreamKind::HttpReq if s("method") == Some("POST") => {
                let path = s("path").unwrap_or("").split('?').next().unwrap_or("");
                let c = self.conns.get(&(k.0, conn)).cloned().unwrap_or_default();
                let shape = shapes::shape_of(path);
                let kind = match shape {
                    Some(_) => Kind::Call,
                    None if c.listed => Kind::Other,
                    None => return,
                };
                let i = self.row(ctx, (k.1, None), (conn, &c), kind, Via::Http, t);
                let r = &mut self.rows[i];
                (r.row.shape, r.up) = (shape, Some((k.1, 0, u64::MAX)));
                self.calls.insert(k, i);
                let (path, body) = (path.to_string(), Buf::default());
                self.reqs.insert(k, Req { row: i, path, body });
            }
            StreamKind::HttpResp => {
                let Some(&i) = self.calls.get(&(k.0, req)) else {
                    return;
                };
                let r = &mut self.rows[i];
                r.down = Some((k.1, 0, u64::MAX));
                (r.t1, r.row.wall_end) = (Some(t), ctx.wall(t));
                r.status = m
                    .get("status")
                    .and_then(Value::as_u64)
                    .and_then(|n| n.try_into().ok());
                let sse = m
                    .get("headers")
                    .and_then(Value::as_array)
                    .is_some_and(|hs| {
                        hs.iter().any(|h| {
                            let n = h.get(0).and_then(Value::as_str).unwrap_or("");
                            let v = h.get(1).and_then(Value::as_str).unwrap_or("");
                            n.eq_ignore_ascii_case("content-type")
                                && v.trim_start()
                                    .to_ascii_lowercase()
                                    .starts_with("text/event-stream")
                        })
                    });
                let sse = sse.then(Sse::default);
                self.resps.insert(
                    k,
                    Resp {
                        row: i,
                        sse,
                        body: Buf::default(),
                    },
                );
            }
            StreamKind::WsUp | StreamKind::WsDown => {
                let sock = (k.0, req);
                self.socks.entry(sock).or_default().conn = conn;
                let up = sk == StreamKind::WsUp;
                let w = Ws {
                    sock,
                    up,
                    msgs: 0,
                    buf: Buf::default(),
                    start: None,
                    last_t: t,
                    op: 0,
                };
                self.ws.insert(k, w);
            }
            _ => {}
        }
    }

    /// A new row at (sid, msg) on connection `conn`.
    fn row(
        &mut self,
        ctx: &Ctx,
        at: (u64, Option<u64>),
        conn: (u64, &Conn),
        kind: Kind,
        via: Via,
        t: u64,
    ) -> usize {
        let ((sid, msg), (conn, c)) = (at, conn);
        let mut row = CallRow::new(Leg::Wire, ctx.entry.clone(), ctx.boot, kind);
        row.sid = Some(sid);
        row.msg = msg;
        row.host = c.host.clone();
        row.host_kind = c.kind;
        row.via = Some(via);
        row.wall_start = ctx.wall(t);
        self.rows.push(Row {
            row,
            conn,
            t0: t,
            ..Row::default()
        });
        self.rows.len() - 1
    }

    fn data(&mut self, ctx: &Ctx, k: Key, t: u64, off: u64, flags: u8, bytes: &[u8]) {
        if let Some(r) = self.reqs.get_mut(&k) {
            r.body.push(off, bytes);
        } else if let Some(r) = self.resps.get_mut(&k) {
            let row = &mut self.rows[r.row];
            row.t1 = Some(t);
            row.row.wall_end = ctx.wall(t);
            match &mut r.sse {
                Some(sse) => {
                    if !r.body.step(off, bytes.len()) {
                        sse.reset();
                    }
                    if let Some(shape) = row.row.shape {
                        sse.feed(bytes, &mut |p| sse_event(&mut row.reply, shape, p));
                    }
                }
                None => r.body.push(off, bytes),
            }
        } else if let Some(w) = self.ws.get_mut(&k) {
            if w.start.is_none() {
                // A message starts where its first byte is; a hole before it is not its own.
                (w.start, w.buf.next) = (Some((off, t)), off);
            }
            w.buf.push(off, bytes);
            w.op = flags >> 4;
            w.last_t = t;
            if flags & FLAG_FIN != 0 {
                self.message(ctx, k, true);
            }
        }
    }

    fn end(&mut self, ctx: &Ctx, k: Key, complete: bool) {
        if let Some(r) = self.reqs.remove(&k) {
            self.req_done(r, complete);
        } else if let Some(r) = self.resps.remove(&k) {
            self.resp_done(r, complete);
        } else {
            self.message(ctx, k, false);
            self.ws.remove(&k);
        }
    }

    /// Every WebSocket message still open is cut: its boot ended (a PART of a new boot, or the
    /// end of the records). `ctx` is still the dead boot's.
    pub fn cut(&mut self, ctx: &Ctx) {
        let mut open: Vec<(u64, Key)> = self
            .ws
            .iter()
            .filter_map(|(k, w)| Some((w.start?.1, *k)))
            .collect();
        open.sort_unstable();
        for (_, k) in open {
            self.message(ctx, k, false);
        }
    }

    fn req_done(&mut self, r: Req, complete: bool) {
        let row = &mut self.rows[r.row];
        row.req_ok = complete && !r.body.hole;
        if let Some(shape) = row.row.shape {
            let v = serde_json::from_slice::<Value>(&r.body.bytes).ok();
            let q = shapes::request(shape, &r.path, v.as_ref());
            (row.row.model_requested, row.row.effort) = (q.model, q.effort);
        }
    }

    fn resp_done(&mut self, mut r: Resp, complete: bool) {
        let row = &mut self.rows[r.row];
        row.resp_ok = complete && !r.body.hole;
        if let Some(shape) = row.row.shape {
            match &mut r.sse {
                Some(sse) => sse.finish(&mut |p| sse_event(&mut row.reply, shape, p)),
                None => row.reply.body(shape, &r.body.bytes),
            }
        }
    }

    /// The open WebSocket message of stream `k`, whole or cut (`fin` false); none open, nothing.
    fn message(&mut self, ctx: &Ctx, k: Key, fin: bool) {
        let Some(w) = self.ws.get_mut(&k) else {
            return;
        };
        let Some((start, t0)) = w.start.take() else {
            return;
        };
        let hole = std::mem::take(&mut w.buf.hole);
        let m = Msg {
            n: w.msgs,
            sock: w.sock,
            range: (k.1, start, w.buf.next),
            whole: fin && !hole,
            t0,
            t1: w.last_t,
            bytes: std::mem::take(&mut w.buf.bytes),
        };
        w.msgs += 1;
        match (w.op, w.up) {
            (1, true) => self.client_msg(ctx, m),
            (1, false) => self.server_msg(ctx, m),
            _ => {}
        }
    }

    fn client_msg(&mut self, ctx: &Ctx, m: Msg) {
        let (msg, v) = shapes::client(&m.bytes);
        let kind = match msg {
            Client::Create { warmup: true, .. } => Kind::Warmup,
            Client::Create { .. } => Kind::Call,
            Client::Interrupt => Kind::Interrupt,
            Client::Other => return,
        };
        let conn = self.socks.get(&m.sock).map_or(0, |s| s.conn);
        let c = self.conns.get(&(ctx.boot, conn)).cloned().unwrap_or_default();
        let i = self.row(ctx, (m.range.0, Some(m.n)), (conn, &c), kind, Via::Ws, m.t0);
        let (rows, s) = (&mut self.rows, self.socks.entry(m.sock).or_default());
        let r = &mut rows[i];
        (r.up, r.req_ok) = (Some(m.range), m.whole);
        if let Client::Create { prev, .. } = msg {
            r.row.shape = Some(Shape::Responses);
            let q = shapes::request(Shape::Responses, "", v.as_ref());
            (r.row.model_requested, r.row.effort) = (q.model, q.effort);
            s.pending.push_back((i, prev));
            return;
        }
        let open = s
            .cur
            .filter(|&c| !rows[c].seen_terminal && !rows[c].seen_error);
        if let Some(c) = open.or_else(|| s.pending.front().map(|p| p.0)) {
            let r = &mut rows[c];
            r.cancelled |= r.reply.end.is_none();
        }
    }

    fn server_msg(&mut self, ctx: &Ctx, m: Msg) {
        let Some(s) = self.socks.get_mut(&m.sock) else {
            return;
        };
        let v = serde_json::from_slice::<Value>(&m.bytes).unwrap_or(Value::Null);
        let kind = shapes::server(&v);
        let rows = &self.rows;
        let open = |c: usize| !rows[c].seen_terminal && !rows[c].seen_error;
        let target = match &kind {
            Server::Created { prev } => {
                let at = prev
                    .as_deref()
                    .and_then(|p| s.pending.iter().position(|q| q.1.as_deref() == Some(p)));
                s.pending.remove(at.unwrap_or(0)).map(|q| q.0)
            }
            Server::Terminal { .. } | Server::Error => {
                let (err, id) = match &kind {
                    Server::Terminal { id } => (false, id.as_ref()),
                    _ => (true, None),
                };
                let fits = |c: usize| {
                    let r = &rows[c];
                    let ids = id.is_none() || r.reply.id.is_none() || r.reply.id.as_ref() == id;
                    let failed = matches!(r.reply.end, Some(End::Failed { .. }));
                    let unseen = if err { !r.seen_error } else { !r.seen_terminal };
                    // An id-less error belongs to a create sent after this call failed.
                    let newer = err && s.pending.iter().any(|q| Some(rows[q.0].t0) > r.end_t);
                    ids && (open(c) || (failed && unseen && !newer))
                };
                match s.cur.filter(|&c| fits(c)) {
                    Some(c) => Some(c),
                    None => s.pending.pop_front().map(|q| q.0),
                }
            }
            Server::Other => s.cur.filter(|&c| open(c)),
        };
        let Some(c) = target else {
            return;
        };
        s.cur = Some(c);
        let r = &mut self.rows[c];
        match kind {
            Server::Terminal { .. } => r.seen_terminal = true,
            Server::Error => r.seen_error = true,
            _ => {}
        }
        if r.seen_terminal || r.seen_error {
            r.end_t.get_or_insert(m.t0);
        }
        r.reply.event(Shape::Responses, &v);
        r.down_bad |= !m.whole;
        r.down = Some(match r.down {
            Some((sid, lo, _)) => (sid, lo, m.range.2),
            None => m.range,
        });
        r.t1 = Some(m.t1);
        if let Some(w) = ctx.wall(m.t1) {
            r.row.wall_end = Some(w);
        }
    }

    /// The rows in a stable order (boot, sid, msg), outcomes and `whole` decided; `ctx` is the
    /// last boot's.
    pub fn finish(mut self, ctx: &Ctx) -> Vec<CallRow> {
        self.cut(ctx);
        for (_, r) in std::mem::take(&mut self.reqs) {
            self.req_done(r, false);
        }
        for (_, r) in std::mem::take(&mut self.resps) {
            self.resp_done(r, false);
        }
        let gaps = std::mem::take(&mut self.gaps);
        let mut out: Vec<CallRow> = self.rows.into_iter().map(|r| decide(r, &gaps)).collect();
        out.sort_by_key(|r| (r.boot, r.sid, r.msg));
        out
    }
}

fn sse_event(reply: &mut Reply, shape: Shape, p: &[u8]) {
    if p.trim_ascii() == b"[DONE]" {
        reply.finish(End::Ok);
    } else if let Ok(v) = serde_json::from_slice::<Value>(p) {
        reply.event(shape, &v);
    }
}

/// Outcome, detail, tokens and `whole` of one row.
fn decide(mut r: Row, gaps: &[Gap]) -> CallRow {
    let http = r.status.filter(|&s| s >= 400);
    let end = match (http, r.reply.end.take()) {
        (Some(s), Some(End::Failed { rate, code, .. })) => Some(End::Failed {
            rate: rate || s == 429,
            status: Some(s),
            code,
        }),
        (Some(s), _) => Some(End::Failed {
            rate: s == 429,
            status: Some(s),
            code: None,
        }),
        (None, e) if r.row.kind == Kind::Other => {
            e.or((r.resp_ok && r.status.is_some()).then_some(End::Ok))
        }
        (None, e) => e,
    };
    let row = &mut r.row;
    row.outcome = match end {
        _ if r.cancelled => Outcome::Cancelled,
        Some(End::Ok) => Outcome::Ok,
        Some(End::Failed { rate, status, code }) => {
            let d: Vec<String> = status
                .map(|s| s.to_string())
                .into_iter()
                .chain(code.clone())
                .collect();
            row.detail = (!d.is_empty()).then(|| d.join(" "));
            if rate {
                Outcome::RateLimited { status, code }
            } else {
                Outcome::Error { status, code }
            }
        }
        None => Outcome::Unknown,
    };
    if row.outcome == Outcome::Ok {
        row.detail = r.reply.detail.take();
    }
    row.model = r.reply.model.take().or_else(|| row.model_requested.clone());
    let t = r.reply.tokens;
    row.tokens = t;
    row.extra = std::mem::take(&mut r.reply.extra);
    let said = [t.input, t.cached, t.cache_write, t.output, t.reasoning];
    row.usage_from = said.iter().any(Option::is_some).then_some(Leg::Wire);
    let (b, t1) = (row.boot, r.t1.unwrap_or(u64::MAX));
    let wall = (row.wall_start, row.wall_end.unwrap_or(u64::MAX));
    // A loss hits [lo, hi) when it starts before hi and ends at or after lo: a loss ending at a
    // message's first byte may hold its head, a loss starting where a message ended does not.
    // Unknown loss (`lost` 0) runs on; a no-loss GAP (`lost` 1) hits when its offset is in range.
    let hit = |g: &Gap, range: Option<(u64, u64, u64)>| match range {
        Some((sid, lo, hi)) => {
            g.sid == sid && g.off < hi && (g.lost == 0 || g.off.saturating_add(g.lost - 1) >= lo)
        }
        None => false,
    };
    let lost = gaps.iter().any(|g| {
        let mine = g.boot == b
            && (hit(g, r.up)
                || hit(g, r.down)
                || (r.conn != 0 && g.sid == r.conn && g.mono.0 <= t1 && g.mono.1 >= r.t0));
        let all = g.sid == 0
            && wall
                .0
                .is_some_and(|w0| g.wall.0 <= wall.1 && g.wall.1 >= w0);
        mine || all
    });
    row.whole = !lost
        && r.req_ok
        && match row.via {
            Some(Via::Ws) if row.kind == Kind::Interrupt => true,
            Some(Via::Ws) => (r.seen_terminal || r.seen_error) && !r.down_bad,
            _ => r.resp_ok,
        };
    r.row
}
