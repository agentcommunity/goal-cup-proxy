//! Record builders for the `goalcup-verify check` tests.
//!
//! [`Entry`] writes one entry folder (`<root>/<entry>/rec`) through `PartWriter` with window
//! 2^20 and tallies its own COUNT values by PRD-01 section 9, so a test never leans on the
//! verifier's recount. [`read_records`] and [`write_raw`] rewrite a record stream as plain
//! records for the mutations `PartWriter` refuses (PART and LINK). [`fetched`] makes the fetched
//! layout, [`run`] runs the binary. Every value here is an obvious fake.
#![allow(dead_code)]

use goalcup_format::codec::{Codec, Record};
use goalcup_format::link::{Chain, ZERO};
use goalcup_format::read::{open_dir, part_files};
use goalcup_format::{EndReason, FLAG_FIN, GapReason, Now, PartOptions, PartWriter, Recover};
use goalcup_format::{StreamKind, index_name, part_name};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

pub const ENTRY: &str = "e0123456789abcdef";
pub const RUN: &str = "r0123456789abcdef";
pub const HOST: &str = "model.example.test";
pub const BOOT_ID: [u8; 16] = [0x11; 16];
pub const MS: u64 = 1_000_000;
/// 2026-11-07T00:00:00Z.
pub const WALL0: u64 = 1_794_009_600_000_000_000;
pub const MONO0: u64 = 5_000_000_000;
pub const MASK_RULES: [&str; 3] = ["bearer", "openai_key", "fail_closed"];
const COUNTERS: [&str; 19] = [
    "http_req", "http_post", "ws_up_msgs", "ws_down_msgs", "req_whole", "req_holes", "conns",
    "tunnels", "tunnel_up", "tunnel_down", "wire_bytes", "native_files", "native_bytes",
    "snapshots", "gaps", "lost_bytes", "lost_unknown", "off_ms", "mask_failclosed_bytes",
];

pub fn opts() -> PartOptions {
    PartOptions { window_log: 20, ..PartOptions::default() }
}

/// A fresh, empty folder under the test target dir.
pub fn tmp(name: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// The record clock and wall clock, moved by hand.
#[derive(Clone, Copy)]
pub struct Clock {
    pub wall0: u64,
    pub ns: u64,
}

impl Clock {
    pub fn now(&self) -> Now {
        Now { wall_ns: self.wall0 + self.ns, mono_ns: MONO0 + self.ns }
    }
    /// Moves 1 ms and returns the record clock.
    pub fn tick(&mut self) -> u64 {
        self.ns += MS;
        self.now().mono_ns
    }
}

#[derive(Clone, Default)]
struct Stream {
    skind: Option<StreamKind>,
    host: String,
    decrypt: bool,
    tunnel: bool,
    post: bool,
    gapped: bool,
}

/// One boot's written counts, by PRD-01 section 9, kept as each record is written.
#[derive(Clone, Default)]
pub struct Tally {
    boot: u64,
    part_mono: u64,
    n: BTreeMap<&'static str, u64>,
    hosts: BTreeMap<String, u64>,
    streams: HashMap<u64, Stream>,
    open: u64,
}

impl Tally {
    fn new(boot: u64, part_mono: u64) -> Self {
        Self { boot, part_mono, ..Self::default() }
    }

    fn add(&mut self, k: &'static str, v: u64) {
        *self.n.entry(k).or_default() += v;
    }

    fn model_request(&mut self, host: String, whole: bool) {
        *self.hosts.entry(host).or_default() += 1;
        self.add(if whole { "req_whole" } else { "req_holes" }, 1);
    }

    pub fn observe(&mut self, r: &Record) {
        match r {
            Record::Open { sid, skind, conn, meta, .. } => {
                let m: Value = serde_json::from_slice(meta).unwrap_or(Value::Null);
                let c = self.streams.get(conn).cloned().unwrap_or_default();
                let mut s = Stream { skind: Some(*skind), host: c.host, decrypt: c.decrypt, ..Stream::default() };
                match skind {
                    StreamKind::Conn => {
                        s.host = m["host"].as_str().unwrap_or_default().to_string();
                        s.decrypt = m["mode"] == "decrypt";
                        s.tunnel = m["mode"] == "tunnel";
                        self.add("conns", 1);
                        self.add("tunnels", u64::from(s.tunnel));
                    }
                    StreamKind::HttpReq => {
                        s.post = m["method"] == "POST";
                        self.add("http_req", 1);
                    }
                    StreamKind::Native => self.add("native_files", 1),
                    StreamKind::Snapshot => self.add("snapshots", 1),
                    _ => {}
                }
                self.open += 1;
                self.streams.insert(*sid, s);
            }
            Record::Data { sid, flags, bytes, .. } => {
                let s = self.streams.get(sid).cloned().unwrap_or_default();
                let len = bytes.len() as u64;
                match s.skind {
                    Some(StreamKind::Native) => self.add("native_bytes", len),
                    Some(StreamKind::HttpReq | StreamKind::HttpResp | StreamKind::WsUp | StreamKind::WsDown) => {
                        self.add("wire_bytes", len)
                    }
                    _ => {}
                }
                let msg = flags & FLAG_FIN != 0 && matches!(flags >> 4, 1 | 2);
                if msg && s.skind == Some(StreamKind::WsUp) && s.decrypt {
                    self.add("ws_up_msgs", 1);
                    self.model_request(s.host, !s.gapped);
                } else if msg && s.skind == Some(StreamKind::WsDown) {
                    self.add("ws_down_msgs", 1);
                }
            }
            Record::End { sid, reason, tail, .. } => {
                let s = self.streams.get(sid).cloned().unwrap_or_default();
                if s.skind == Some(StreamKind::HttpReq) && s.post && s.decrypt {
                    self.add("http_post", 1);
                    self.model_request(s.host, *reason == EndReason::Complete && !s.gapped);
                }
                if s.tunnel {
                    let t: Value = serde_json::from_slice(tail).unwrap_or(Value::Null);
                    self.add("tunnel_up", t["up"].as_u64().unwrap_or(0));
                    self.add("tunnel_down", t["down"].as_u64().unwrap_or(0));
                }
                self.open = self.open.saturating_sub(1);
            }
            Record::Gap { sid, reason, start_mono, end_mono, lost, .. } => {
                self.add("gaps", 1);
                match lost {
                    0 => self.add("lost_unknown", 1),
                    n => self.add("lost_bytes", n - 1),
                }
                if *reason == GapReason::RecorderDown {
                    self.add("off_ms", (end_mono - start_mono) / MS);
                }
                if let Some(s) = self.streams.get_mut(sid) {
                    s.gapped = true;
                }
            }
            Record::Mask { len, rule, .. } if *rule == 2 => self.add("mask_failclosed_bytes", *len),
            _ => {}
        }
    }

    /// The COUNT event at record time `t`.
    pub fn count(&self, t: u64) -> Value {
        let mut v = Map::new();
        v.insert("ev".into(), "count".into());
        v.insert("boot".into(), self.boot.into());
        v.insert("up_ms".into(), ((t - self.part_mono) / MS).into());
        for k in COUNTERS {
            v.insert(k.into(), self.n.get(k).copied().unwrap_or(0).into());
        }
        v.insert("hosts".into(), json!(self.hosts));
        v.insert("tunnel_hosts".into(), json!({}));
        v.insert("open_streams".into(), self.open.into());
        v.insert("held_bytes".into(), 0.into());
        v.insert("held_peak".into(), 0.into());
        Value::Object(v)
    }
}

/// The entry total (section 9): each boot's last COUNT summed, then every recovery frame's
/// `recorder_down` gap added to `gaps`, `lost_unknown` and `off_ms`.
pub fn totals(boots: &[Value], down_ms: &[u64]) -> Value {
    let mut t = Map::new();
    let mut hosts: BTreeMap<String, u64> = BTreeMap::new();
    for k in COUNTERS {
        t.insert(k.into(), boots.iter().map(|b| b[k].as_u64().unwrap()).sum::<u64>().into());
    }
    for b in boots {
        for (h, n) in b["hosts"].as_object().unwrap() {
            *hosts.entry(h.clone()).or_default() += n.as_u64().unwrap();
        }
    }
    let add = |t: &mut Map<String, Value>, k: &str, v: u64| {
        let n = t[k].as_u64().unwrap() + v;
        t.insert(k.into(), n.into());
    };
    add(&mut t, "gaps", down_ms.len() as u64);
    add(&mut t, "lost_unknown", down_ms.len() as u64);
    add(&mut t, "off_ms", down_ms.iter().sum());
    t.insert("hosts".into(), json!(hosts));
    t.insert("tunnel_hosts".into(), json!({}));
    t.insert("held_peak".into(), 0.into());
    Value::Object(t)
}

/// One entry folder being written.
pub struct Entry {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub rec: PathBuf,
    pub w: Option<PartWriter>,
    pub clock: Clock,
    pub tally: Tally,
    /// The last COUNT of every finished boot.
    pub boots: Vec<Value>,
    /// The length of every recovery frame's `recorder_down` gap, in ms.
    pub down_ms: Vec<u64>,
    sid: u64,
    counts_at: HashMap<u64, Vec<u8>>,
}

impl Entry {
    /// PART and the start event, on the fixed test clock.
    pub fn new(name: &str) -> Self {
        Self::at(name, WALL0)
    }

    /// The same, with wall time starting at `wall0` (a `--live` test passes a time an hour ago).
    pub fn at(name: &str, wall0: u64) -> Self {
        let root = tmp(name);
        let dir = root.join(ENTRY);
        let rec = dir.join("rec");
        let clock = Clock { wall0, ns: 0 };
        let w = PartWriter::open(&rec, opts(), ENTRY, BOOT_ID, clock.now()).unwrap();
        let mut e = Self {
            root,
            dir,
            rec,
            w: Some(w),
            clock,
            tally: Tally::new(0, MONO0),
            boots: vec![],
            down_ms: vec![],
            sid: 0,
            counts_at: HashMap::new(),
        };
        e.event(start(false));
        e
    }

    /// Writes one record, after tallying it.
    pub fn put(&mut self, r: Record) {
        self.tally.observe(&r);
        self.w.as_mut().unwrap().write(&r).unwrap();
    }

    pub fn event(&mut self, v: Value) {
        let t = self.clock.tick();
        self.put(Record::Event { t, json: v.to_string().into_bytes() });
    }

    pub fn open(&mut self, skind: StreamKind, conn: u64, req: u64, meta: Value) -> u64 {
        self.sid += 1;
        let (sid, t) = (self.sid, self.clock.tick());
        self.put(Record::Open { sid, t, skind, conn, req, meta: meta.to_string().into_bytes() });
        sid
    }

    pub fn data(&mut self, sid: u64, off: u64, flags: u8, bytes: &[u8]) {
        let t = self.clock.tick();
        self.put(Record::Data { sid, t, off, flags, bytes: bytes.to_vec() });
    }

    pub fn end(&mut self, sid: u64, reason: EndReason, seen: u64, tail: Value) {
        let t = self.clock.tick();
        let tail = if tail.is_null() { vec![] } else { tail.to_string().into_bytes() };
        self.put(Record::End { sid, t, reason, seen, tail });
    }

    /// A GAP lasting `ms` that ends now. `start == end` when `ms` is 0.
    pub fn gap(&mut self, sid: u64, reason: GapReason, ms: u64, off: u64, lost: u64, tail: Value) {
        self.clock.ns += ms * MS + MS;
        let end = self.clock.now();
        let tail = if tail.is_null() { vec![] } else { tail.to_string().into_bytes() };
        self.put(Record::Gap {
            sid,
            reason,
            start_wall: end.wall_ns - ms * MS,
            end_wall: end.wall_ns,
            start_mono: end.mono_ns - ms * MS,
            end_mono: end.mono_ns,
            off,
            lost,
            tail,
        });
    }

    pub fn conn(&mut self) -> u64 {
        let meta = json!({"host": HOST, "port": 443, "mode": "decrypt", "listed": "shared",
            "kind": "provider", "reason": null, "alpn": "http/1.1", "tls": "1.3",
            "fingerprint": "0123456789abcdef"});
        self.open(StreamKind::Conn, 0, 0, meta)
    }

    /// One decrypted connection with a POST and its response, which ends with `resp_end`. The
    /// request body holds one masked span (rule `bearer`).
    pub fn exchange(&mut self, resp_end: EndReason) {
        let c = self.conn();
        let head = json!({"method": "POST", "path": "/v1/responses", "ver": "1.1",
            "headers": [["content-type", "application/json"]], "masked": [], "coding": null});
        let q = self.open(StreamKind::HttpReq, c, 0, head);
        let body = br#"{"model":"m1","input":"hi","key":"****"}"#;
        self.data(q, 0, 0, body);
        self.put(Record::Mask { sid: q, off: body.len() as u64 - 6, len: 4, rule: 0 });
        self.end(q, EndReason::Complete, body.len() as u64, json!({"wire": body.len()}));
        let head = json!({"status": 200, "ver": "1.1", "headers": [["content-type", "text/event-stream"]],
            "masked": [], "coding": null, "decoded": true});
        let p = self.open(StreamKind::HttpResp, c, q, head);
        let body = b"data: {\"type\":\"response.completed\"}\n\n";
        self.data(p, 0, 0, body);
        self.end(p, resp_end, body.len() as u64, json!({"wire": body.len()}));
        let tail = json!({"up": 900, "down": 700, "plain_up": 300, "plain_down": 200});
        self.end(c, EndReason::Complete, 0, tail);
    }

    /// One decrypted connection upgraded to a WebSocket: one client message in two pieces, one
    /// server message.
    pub fn websocket(&mut self) {
        let c = self.conn();
        let head = json!({"method": "GET", "path": "/v1/responses", "ver": "1.1",
            "headers": [["upgrade", "websocket"]], "masked": [], "coding": null});
        let q = self.open(StreamKind::HttpReq, c, 0, head);
        self.end(q, EndReason::Complete, 0, Value::Null);
        let up = self.open(StreamKind::WsUp, c, q, json!({"deflate": false}));
        let down = self.open(StreamKind::WsDown, c, q, json!({"deflate": false}));
        self.data(up, 0, 0x10, br#"{"type":"response.create","#);
        self.data(up, 26, 0x10 | FLAG_FIN, br#""input":"hi"}"#);
        self.data(down, 0, 0x10 | FLAG_FIN, br#"{"type":"response.completed"}"#);
        self.end(up, EndReason::Complete, 39, json!({"close_code": 1000}));
        self.end(down, EndReason::Complete, 29, json!({"close_code": 1000}));
        let tail = json!({"up": 400, "down": 300, "plain_up": 200, "plain_down": 100});
        self.end(c, EndReason::Complete, 0, tail);
    }

    /// One native stream of `path` and `gen` from `from`, two DATA pieces, ended `complete`.
    pub fn native(&mut self, path: &str, gen_: u64, from: u64) -> u64 {
        let meta = json!({"path": path, "gen": gen_, "size_at_open": from, "from": from, "fid": "1a2b"});
        let s = self.open(StreamKind::Native, 0, 0, meta);
        let (a, b) = (b"{\"line\":1}\n", b"{\"line\":2}\n");
        self.data(s, from, 0, a);
        self.data(s, from + a.len() as u64, 0, b);
        self.end(s, EndReason::Complete, from + 22, Value::Null);
        s
    }

    /// The clean content: a launch, an exchange, a WebSocket, a session file, a COUNT.
    pub fn standard(&mut self) {
        self.event(json!({"ev": "launch", "pid": 4242, "program": "fake-harness"}));
        self.exchange(EndReason::Complete);
        self.websocket();
        self.native("fake:sessions/s1.jsonl", 0, 0);
        self.count();
    }

    /// A LINK; keeps the COUNT as it stands there for crash recovery. Returns its time.
    pub fn link(&mut self) -> Now {
        self.clock.tick();
        let now = self.clock.now();
        let seq = self.w.as_mut().unwrap().link(now).unwrap().seq;
        self.counts_at.insert(seq, self.tally.count(now.mono_ns).to_string().into_bytes());
        now
    }

    /// A COUNT event and the LINK after it.
    pub fn count(&mut self) {
        self.count_with(|_| {});
    }

    pub fn count_with(&mut self, f: impl FnOnce(&mut Value)) {
        let t = self.clock.tick();
        let mut v = self.tally.count(t);
        f(&mut v);
        self.put(Record::Event { t, json: v.to_string().into_bytes() });
        self.link();
    }

    /// Time passes with nothing written.
    pub fn idle(&mut self, ms: u64) {
        self.clock.ns += ms * MS;
    }

    /// The machine slept `ms`: the record clock moves, then a `suspend` event and a LINK.
    pub fn sleep(&mut self, ms: u64) {
        self.idle(ms);
        self.event(json!({"ev": "suspend", "ns": ms * MS}));
        self.link();
    }

    /// The recorder dies after a LINK (records after it are lost) and comes back `down_ms`
    /// later through `PartWriter::recover`: the recovery frame holds the dead boot's COUNT and a
    /// `recorder_down` gap with `cause`; the next boot writes `start` and `prev_boot`. Returns the
    /// gap's length in ms.
    pub fn crash(&mut self, cause: &str, down_ms: u64) -> u64 {
        let at = self.link();
        let mut w = self.w.take().unwrap();
        w.sync().unwrap();
        let lost = Record::Event { t: self.clock.tick(), json: br#"{"ev":"note","what":"writer_slow"}"#.to_vec() };
        w.write(&lost).unwrap();
        drop(w);
        self.idle(down_ms);
        let now = self.clock.now();
        let counts = self.counts_at.clone();
        let count_at = move |seq: u64| counts[&seq].clone();
        let mut tail = Map::new();
        tail.insert("cause".into(), cause.into());
        tail.insert("prev".into(), json!({"signal": 9}));
        let rc = Recover { entry: ENTRY, boot_id: BOOT_ID, now, durable: None, count_at: &count_at,
            tail, dead_start: None, seal: None };
        let r = PartWriter::recover(&self.rec, opts(), rc).unwrap();
        let ms = (now.mono_ns - at.mono_ns) / MS;
        self.boots.push(self.tally.count(at.mono_ns));
        self.down_ms.push(ms);
        self.w = r.writer;
        self.tally = Tally::new(self.tally.boot + 1, now.mono_ns);
        self.event(start(true));
        self.event(json!({"ev": "prev_boot", "signal": 9}));
        ms
    }

    /// An `after_stop` GAP: bytes relayed after the seal began.
    pub fn after_stop(&mut self) {
        self.gap(0, GapReason::AfterStop, 2_000, 0, 4097, json!({"conns": 1, "hosts": [HOST]}));
    }

    /// The final COUNT, the seal with its totals, the seal's LINK. Returns the entry folder.
    pub fn seal(self) -> PathBuf {
        self.seal_with(|_| {})
    }

    /// The same, with the seal's totals changed by `f`.
    pub fn seal_with(mut self, f: impl FnOnce(&mut Value)) -> PathBuf {
        self.event(json!({"ev": "exit", "code": 0}));
        let t = self.clock.tick();
        let last = self.tally.count(t);
        self.put(Record::Event { t, json: last.to_string().into_bytes() });
        let mut boots = self.boots.clone();
        boots.push(last);
        let mut tot = totals(&boots, &self.down_ms);
        f(&mut tot);
        self.event(json!({"ev": "seal", "reason": "harness_exit", "exit": {"code": 0},
            "harness": "exited", "workspace_head": null, "dirty": null,
            "boots": boots.len(), "totals": tot}));
        self.clock.tick();
        self.w.take().unwrap().seal(self.clock.now()).unwrap();
        self.dir
    }

    /// Leaves the entry unsealed, as a crash would: a COUNT, a LINK, both files synced, so the
    /// last frame is open and the index has lines.
    pub fn leave(mut self) -> PathBuf {
        self.count();
        self.w.take().unwrap().sync().unwrap();
        self.dir
    }
}

/// A sealed clean entry: launch, exchange, WebSocket, session file, COUNTs.
pub fn clean(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.seal()
}

pub fn start(resume: bool) -> Value {
    let zeros = "0".repeat(64);
    json!({"ev": "start", "run": RUN, "entry": ENTRY, "segment": 0, "goalcup": "1.0.0",
        "build": null, "os": "linux", "arch": "x86_64", "connector": "fake",
        "connector_source": "file", "connector_sha256": zeros, "harness_version": null,
        "hosts": {"shared": [{"host": HOST, "kind": "provider", "pinned": false}],
                  "participant": [], "config": []},
        "hosts_sha256": zeros, "mask_rules": MASK_RULES, "mask_values": {"skipped": 0},
        "fd_limit": 1024, "keep": true, "deadline": null, "server_time": null,
        "local_wall_at_server_time": null, "workspace_head": null, "resume": resume})
}

/// Every record of an entry's `rec` folder, in order.
pub fn read_records(rec: &Path) -> Vec<Record> {
    open_dir(rec).unwrap().map(|i| i.unwrap().rec).collect()
}

/// Rewrites a `rec` folder from plain records: one part file per PART (named by its raw
/// offset; PART `raw_offset` and `prev` are set to fit), LINK hashes computed, except the LINK
/// with seq `bad_link`, which gets a wrong hash. Each part file is one zstd frame up to its last
/// LINK, with one `E` index line there, and a second frame for anything after it.
pub fn write_raw(rec: &Path, recs: &[Record], bad_link: Option<u64>) {
    for p in part_files(rec).unwrap() {
        fs::remove_file(&p).unwrap();
        let _ = fs::remove_file(p.with_extension("").with_extension("idx"));
    }
    let (mut codec, mut chain, mut prev) = (Codec::default(), Chain::new(ZERO), ZERO);
    let (mut buf, mut start, mut off) = (Vec::new(), 0u64, 0u64);
    let mut last_link: Option<(u64, usize, [u8; 32])> = None;
    for (i, r) in recs.iter().enumerate() {
        let mut r = r.clone();
        if let Record::Part { raw_offset, prev: p, .. } = &mut r {
            if i > 0 {
                flush(rec, start, &buf, last_link.take());
                buf.clear();
                start = off;
            }
            (*raw_offset, *p) = (off, prev);
        }
        let mut b = Vec::new();
        codec.encode(&r, &mut b).unwrap();
        if let Record::Link { seq, .. } = r {
            let n = b.len() - 32;
            let h = chain.link(&b[..n]);
            prev = h;
            let stored = if bad_link == Some(seq) { [0xee; 32] } else { h };
            b[n..].copy_from_slice(&stored);
            last_link = Some((seq, buf.len() + b.len(), stored));
        } else {
            chain.feed(&b);
        }
        off += b.len() as u64;
        buf.extend_from_slice(&b);
    }
    flush(rec, start, &buf, last_link);
}

fn flush(rec: &Path, start: u64, raw: &[u8], link: Option<(u64, usize, [u8; 32])>) {
    let cut = link.map_or(raw.len(), |l| l.1);
    let mut z = zstd::encode_all(&raw[..cut], 3).unwrap();
    let c_end = z.len();
    if cut < raw.len() {
        z.extend(zstd::encode_all(&raw[cut..], 3).unwrap());
    }
    fs::write(rec.join(part_name(start)), z).unwrap();
    if let Some((seq, end, hash)) = link {
        let line = format!("E {seq} {} {c_end} {}\n", start + end as u64, goalcup_format::hex(&hash));
        fs::write(rec.join(index_name(start)), line).unwrap();
    }
}

/// Decodes an entry, lets `f` change its records, and writes it back (see [`write_raw`]).
pub fn mutate(dir: &Path, bad_link: Option<u64>, f: impl FnOnce(&mut Vec<Record>)) -> PathBuf {
    let rec = dir.join("rec");
    let mut recs = read_records(&rec);
    f(&mut recs);
    write_raw(&rec, &recs, bad_link);
    dir.to_path_buf()
}

/// Appends raw record bytes to the last part file as one more zstd frame.
pub fn append_raw(dir: &Path, raw: &[u8]) {
    let last = part_files(&dir.join("rec")).unwrap().pop().unwrap();
    let mut b = fs::read(&last).unwrap();
    b.extend(zstd::encode_all(raw, 3).unwrap());
    fs::write(last, b).unwrap();
}

/// Changes the JSON of the `n`th event whose `ev` is `ev`.
pub fn edit_event(recs: &mut [Record], ev: &str, n: usize, f: impl FnOnce(&mut Value)) {
    let json = recs
        .iter_mut()
        .filter_map(|r| match r {
            Record::Event { json, .. } => Some(json),
            _ => None,
        })
        .filter(|j| serde_json::from_slice::<Value>(j).unwrap()["ev"] == ev)
        .nth(n)
        .unwrap();
    let mut v: Value = serde_json::from_slice(json).unwrap();
    f(&mut v);
    *json = v.to_string().into_bytes();
}

/// The position of the first record matching `f`.
pub fn find(recs: &[Record], f: impl Fn(&Record) -> bool) -> usize {
    recs.iter().position(f).unwrap()
}

/// The fetched layout: an entry's part files concatenated into `<out>/<entry>/rec.gcr.zst`.
pub fn fetched(dir: &Path, out: &Path) -> PathBuf {
    let mut all = Vec::new();
    for p in part_files(&dir.join("rec")).unwrap() {
        all.extend(fs::read(p).unwrap());
    }
    fs::create_dir_all(out.join(ENTRY)).unwrap();
    fs::write(out.join(ENTRY).join("rec.gcr.zst"), all).unwrap();
    out.to_path_buf()
}

/// Runs `goalcup-verify` with `args`: the exit code and stdout parsed as JSON, if it is.
pub fn run(args: &[&str]) -> (i32, Option<Value>) {
    let out = Command::new(env!("CARGO_BIN_EXE_goalcup-verify")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), serde_json::from_slice(&out.stdout).ok())
}

/// `check <dir> --json [extra]`.
pub fn check(dir: &Path, extra: &[&str]) -> (i32, Value) {
    let mut args = vec!["check", dir.to_str().unwrap(), "--json"];
    args.extend_from_slice(extra);
    let (code, v) = run(&args);
    (code, v.unwrap_or_else(|| panic!("no S6 JSON for {dir:?}, exit {code}")))
}

/// The first entry's check item with this id.
pub fn item<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["entries"][0]["checks"]
        .as_array()
        .and_then(|c| c.iter().find(|c| c["id"] == id))
        .unwrap_or_else(|| panic!("no check {id} in the report"))
}

/// The first entry's result for check `id`.
pub fn result<'a>(v: &'a Value, id: &str) -> &'a str {
    item(v, id)["result"].as_str().unwrap()
}

/// The ids of the first entry's checks with this result.
pub fn ids_with(v: &Value, res: &str) -> Vec<String> {
    v["entries"][0]["checks"]
        .as_array()
        .map(|c| c.iter().filter(|c| c["result"] == res).map(|c| c["id"].as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}
