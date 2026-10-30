//! Integrity (PRD-08 section 3, PRD-01 section 13): the 14 invariants and the record checks, in
//! one streaming pass per entry. State is a few words per open stream (dropped at END) and per
//! boot. A finding is a code and counts, never a byte of the record.

use crate::input::{self, Entry};
use goalcup_format::dump::wall;
use goalcup_format::{EndReason, FLAG_FIN, GapReason, Item, Record, StreamKind};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

/// 180 s: the longest COUNT spacing (invariant 13).
const CADENCE_NS: u64 = 180_000_000_000;

/// A check result. Ordered from least to most severe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Res {
    Pass,
    Skip,
    Warn,
    Named,
    Fail,
}

impl Res {
    pub fn name(self) -> &'static str {
        ["pass", "skip", "warn", "named", "fail"][self as usize]
    }
}

/// One check of S6: id, invariant number, result, one plain line, the first finding, facts.
#[derive(Clone, Debug)]
pub struct Check {
    pub id: &'static str,
    pub inv: Option<u8>,
    pub result: Res,
    pub why: String,
    pub first: Option<Value>,
    pub facts: Map<String, Value>,
}

impl Check {
    pub fn new(id: &'static str, inv: Option<u8>, result: Res, why: &str) -> Self {
        Check {
            id,
            inv,
            result,
            why: why.into(),
            first: None,
            facts: Map::new(),
        }
    }
}

/// The integrity checks, in report order.
pub const IDS: &[(&str, Option<u8>)] = &[
    ("inv1", Some(1)),
    ("inv2", Some(2)),
    ("inv3", Some(3)),
    ("inv4", Some(4)),
    ("inv5", Some(5)),
    ("inv6", Some(6)),
    ("inv7", Some(7)),
    ("inv8", Some(8)),
    ("inv9", Some(9)),
    ("streams_closed", Some(10)),
    ("seen_matches_written", Some(11)),
    ("gap_has_end", Some(12)),
    ("recorder_alive", Some(13)),
    ("inv14", Some(14)),
    ("torn_tail", None),
    ("native_path_gen", None),
    ("counts_match_seal", None),
    ("gaps_named", None),
    ("record_decode", None),
    ("parts_pruned", None),
    ("record_kind_unknown", None),
    ("gap_reason_unknown", None),
];

/// COUNT counters (PRD-01 section 9). Gauges (`open_streams`, `held_bytes`, `held_peak`, `up_ms`)
/// are not recounted.
const COUNTERS: &[&str] = &[
    "http_req",
    "http_post",
    "ws_up_msgs",
    "ws_down_msgs",
    "req_whole",
    "req_holes",
    "conns",
    "tunnels",
    "tunnel_up",
    "tunnel_down",
    "wire_bytes",
    "native_files",
    "native_bytes",
    "snapshots",
    "gaps",
    "lost_bytes",
    "lost_unknown",
    "off_ms",
    "mask_failclosed_bytes",
];

/// A recount of COUNT keys, plus what S6 shows beside them.
#[derive(Clone, Default)]
pub struct Recount {
    pub c: BTreeMap<&'static str, u64>,
    pub hosts: BTreeMap<String, u64>,
    /// host: conns, up, down, listed.
    pub tunnel: BTreeMap<String, (u64, u64, u64, Option<String>)>,
    pub by_reason: BTreeMap<String, (u64, u64, u64)>,
    pub after_stop: u64,
    pub off_spans: u64,
}

impl Recount {
    fn add(&mut self, k: &'static str, v: u64) {
        *self.c.entry(k).or_default() += v;
    }
    pub fn get(&self, k: &str) -> u64 {
        self.c.get(k).copied().unwrap_or(0)
    }
    fn merge(&mut self, o: &Recount) {
        o.c.iter().for_each(|(k, v)| self.add(k, *v));
        o.hosts
            .iter()
            .for_each(|(k, v)| *self.hosts.entry(k.clone()).or_default() += v);
        for (h, (n, up, down, l)) in &o.tunnel {
            let t = self.tunnel.entry(h.clone()).or_default();
            (t.0, t.1, t.2) = (t.0 + n, t.1 + up, t.2 + down);
            if l.is_some() {
                t.3.clone_from(l);
            }
        }
        for (r, (n, b, u)) in &o.by_reason {
            let x = self.by_reason.entry(r.clone()).or_default();
            (x.0, x.1, x.2) = (x.0 + n, x.1 + b, x.2 + u);
        }
        self.after_stop += o.after_stop;
        self.off_spans += o.off_spans;
    }
    /// The COUNT keys as JSON, to compare with a COUNT event or the seal's `totals`.
    fn json(&self) -> Map<String, Value> {
        let mut m: Map<String, Value> = COUNTERS
            .iter()
            .map(|k| (k.to_string(), self.get(k).into()))
            .collect();
        m.insert("hosts".into(), json!(self.hosts));
        let th: Map<String, Value> = self
            .tunnel
            .iter()
            .map(|(h, (n, u, d, l))| {
                (
                    h.clone(),
                    json!({"conns": n, "up": u, "down": d, "listed": l}),
                )
            })
            .collect();
        m.insert("tunnel_hosts".into(), th.into());
        m
    }
}

/// The recount's keys that a COUNT (or `totals`) leaves out or writes differently. Gauges and
/// other keys the recount does not know are left alone.
fn differ(written: &Value, mine: &Recount) -> Vec<String> {
    mine.json()
        .into_iter()
        .filter_map(|(k, m)| match written.get(&k) {
            Some(v) if *v == m => None,
            Some(v) => Some(format!("{k} {v} against {m}")),
            None => Some(format!("{k} missing against {m}")),
        })
        .collect()
}

/// Per open stream: a few words.
#[derive(Default)]
struct St {
    kind: Option<StreamKind>,
    from: u64,
    /// The end of written bytes or of a known gap.
    next: u64,
    /// An unknown-loss gap starts here and its hole is not closed yet.
    unknown: Option<u64>,
    hole: u64,
    holed: bool,
    gapped: bool,
    msg_gap: bool,
    /// An `http_req` POST on a decrypted connection.
    post: bool,
    /// The host of a decrypted connection (its own for `conn`), for `hosts`.
    host: Option<String>,
    /// For a tunnel `conn`: its host.
    tunnel: Option<String>,
    native: Option<(String, u64)>,
    /// Runs of `*` in written bytes, for MASK (invariant 5).
    stars: Vec<(u64, u64)>,
}

/// Where a finding is: raw offset, stream, wall time.
#[derive(Clone, Copy)]
struct At(u64, u64, u64);

/// What one entry gave.
pub struct Outcome {
    pub checks: Vec<Check>,
    pub sealed: bool,
    pub seal_reason: Option<String>,
    pub boots: u64,
    pub start: Map<String, Value>,
    pub counts: Recount,
    pub entry: Option<String>,
    pub raw: u64,
    pub compressed: u64,
    pub data_bytes: u64,
    pub links: u64,
    pub pruned: bool,
}

/// Checks one entry. `now_ns` is set for `--live`.
pub fn check(e: &Entry, now_ns: Option<u64>) -> Outcome {
    let checks = IDS
        .iter()
        .map(|(id, inv)| Check::new(id, *inv, Res::Pass, ""))
        .collect();
    let mut p = Pass {
        checks,
        ..Default::default()
    };
    let (mut err, mut truncated) = (None, None);
    let mut meter = std::rc::Rc::new(input::Meter::default());
    match input::open(e) {
        Ok((mut rd, m)) => {
            for r in rd.by_ref() {
                match r {
                    Ok(it) => p.item(&it),
                    Err(x) => err = Some(x.to_string()),
                }
            }
            (truncated, meter) = (rd.tail().truncated, m);
        }
        Err(x) => err = Some(x.to_string()),
    }
    let at = At(p.end_off, 0, p.wall_of(p.t));
    if let Some(why) = err {
        p.fail(
            "record_decode",
            &format!("the record stops decoding: {why}"),
            at,
        );
    } else if let Some(o) = truncated {
        if p.seal.is_some() {
            p.fail(
                "torn_tail",
                "a sealed entry ends inside a record",
                At(o, 0, at.2),
            );
        } else {
            p.fact("torn_tail", "open_end", o.into());
        }
    }
    p.finish(meter.open_frame.get(), now_ns, e);
    let seal_reason = p
        .seal
        .as_ref()
        .and_then(|s| s.get("reason")?.as_str().map(String::from));
    Outcome {
        sealed: p.seal.is_some(),
        seal_reason,
        boots: p.boots,
        start: p.start.clone().unwrap_or_default(),
        counts: p.all.clone(),
        entry: p.entry.clone(),
        raw: meter.raw.get(),
        compressed: input::compressed(e),
        data_bytes: p.data_bytes,
        links: p.links,
        pruned: p.pruned,
        checks: p.checks,
    }
}

#[derive(Default)]
struct Pass {
    checks: Vec<Check>,
    entry: Option<String>,
    start: Option<Map<String, Value>>,
    mask_rules: Vec<String>,
    // Time base: the PART's wall and record clock, the running record time.
    part_wall: u64,
    part_mono: u64,
    t: u64,
    end_off: u64,
    boot: Option<u64>,
    part: u64,
    boots: u64,
    pruned: bool,
    /// The first boot of a pruned entry: streams and counts began in part files that are gone.
    partial: bool,
    next_seq: u64,
    last_hash: [u8; 32],
    last_link: Option<(u64, u64)>,
    links: u64,
    data_bytes: u64,
    // Per boot.
    streams: HashMap<u64, St>,
    used: HashSet<u64>,
    reqs: HashMap<u64, u64>,
    natives_boot: HashSet<(String, u64)>,
    cur: Recount,
    at_count: Option<Recount>,
    down_after: (u64, u64),
    boot_head: bool,
    boot0_gap: bool,
    pending_down: usize,
    cadence_t: u64,
    suspend: u64,
    regular_link_t: Option<u64>,
    last_count_wall: u64,
    // Per entry.
    natives: HashMap<(String, u64), (u64, bool)>,
    totals: Recount,
    all: Recount,
    down_gaps: Vec<Value>,
    seal: Option<Map<String, Value>>,
    seal_link: bool,
}

impl Pass {
    fn idx(&self, id: &str) -> usize {
        self.checks
            .iter()
            .position(|c| c.id == id)
            .expect("a known check id")
    }

    /// Raises a check to `res` when that is worse; the first finding at the worst level is kept.
    fn set(&mut self, id: &str, res: Res, why: &str, at: Option<At>) {
        let i = self.idx(id);
        let c = &mut self.checks[i];
        if res > c.result {
            c.result = res;
            c.why = why.into();
            c.first = at.map(|At(o, sid, w)| json!({"o": o, "sid": sid, "wall": wall(w)}));
        }
    }

    fn fail(&mut self, id: &str, why: &str, at: At) {
        let gone = matches!(id, "inv3" | "inv7" | "inv9") && self.partial;
        if gone || (id == "counts_match_seal" && self.pruned) {
            return;
        }
        self.set(id, Res::Fail, why, Some(at));
    }

    fn fact(&mut self, id: &str, k: &str, v: Value) {
        let i = self.idx(id);
        self.checks[i].facts.insert(k.into(), v);
    }

    fn bump(&mut self, id: &str, k: &str, by: u64) {
        let i = self.idx(id);
        let f = self.checks[i].facts.entry(k).or_insert(0.into());
        *f = (f.as_u64().unwrap_or(0) + by).into();
    }

    fn wall_of(&self, t: u64) -> u64 {
        (i128::from(self.part_wall) + i128::from(t) - i128::from(self.part_mono))
            .clamp(0, u64::MAX.into()) as u64
    }

    fn item(&mut self, it: &Item) {
        let (sid, t) = match &it.rec {
            Record::Open { sid, t, .. }
            | Record::Data { sid, t, .. }
            | Record::End { sid, t, .. } => (*sid, *t),
            Record::Gap { sid, end_mono, .. } => (*sid, *end_mono),
            Record::Mask { sid, .. } => (*sid, self.t),
            Record::Event { t, .. } => (0, *t),
            Record::Link { mono_ns, .. } | Record::Part { mono_ns, .. } => (0, *mono_ns),
            Record::Unknown { .. } => (0, self.t),
        };
        self.t = t;
        self.end_off = it.off;
        let at = match it.rec {
            Record::Part { wall_ns, .. } => At(it.off, 0, wall_ns),
            _ => At(it.off, sid, self.wall_of(t)),
        };
        let is_down = matches!(
            it.rec,
            Record::Gap {
                sid: 0,
                reason: GapReason::RecorderDown,
                ..
            }
        );
        let is_link = matches!(it.rec, Record::Link { .. });
        let ev = match &it.rec {
            Record::Event { json, .. } => {
                serde_json::from_slice::<Map<String, Value>>(json).unwrap_or_default()
            }
            _ => Map::new(),
        };
        let ev_name = ev
            .get("ev")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if self.seal.is_some() {
            if is_link && !self.seal_link {
                self.seal_link = true;
            } else {
                self.fail("inv8", "a record follows the seal and its link", at);
            }
        }
        if self.boot.is_none() && !matches!(it.rec, Record::Part { .. }) {
            self.fail("inv1", "the record does not start with PART", at);
            self.boot = Some(0);
            self.boots = 1;
        }
        // A recorder_down gap sits in a recovery frame: only links, more such gaps, the seal or
        // the next boot's PART follow it.
        let recovery =
            is_link || is_down || ev_name == "seal" || matches!(it.rec, Record::Part { .. });
        if self.pending_down > 0 && !recovery {
            self.fail("inv6", "a recorder_down gap inside a boot", at);
            self.pending_down = 0;
        }
        if !matches!(it.rec, Record::Part { .. } | Record::Link { .. })
            && !matches!(ev_name.as_str(), "start" | "prev_boot")
            && !is_down
        {
            self.boot_head = false;
        }
        match &it.rec {
            Record::Part {
                entry,
                boot,
                part,
                raw_offset,
                wall_ns,
                mono_ns,
                next_link,
                prev,
                ..
            } => {
                match self.boot {
                    None => {
                        (self.pruned, self.partial) = (*raw_offset != 0, *raw_offset != 0);
                        if self.pruned {
                            self.set(
                                "parts_pruned",
                                Res::Named,
                                "local part files were pruned; check the server copy",
                                Some(at),
                            );
                        } else {
                            if *part != 0 || *boot != 0 || *prev != [0; 32] {
                                self.fail("inv1", "the first PART is not part 0 of boot 0", at);
                            }
                            if *next_link != 0 {
                                self.fail("inv2", "the link chain does not start at seq 0", at);
                            }
                        }
                        self.next_seq = *next_link;
                        self.last_hash = *prev;
                        self.entry = Some(entry.clone());
                        self.boots = 1;
                        self.boot_head = true;
                    }
                    Some(b) => {
                        if Some(entry) != self.entry.as_ref()
                            || *raw_offset != it.off
                            || *part != self.part + 1
                        {
                            self.fail("inv1", "a PART breaks the entry, offset or part order", at);
                        }
                        if *boot < b || *boot > b + 1 {
                            self.fail("inv1", "boot numbers have a hole or go back", at);
                        }
                        if *prev != self.last_hash || *next_link != self.next_seq {
                            self.fail("inv2", "a PART does not continue the link chain", at);
                        }
                        if *boot != b {
                            if self.pending_down == 0 {
                                self.fail(
                                    "inv6",
                                    &format!("boot {boot} has no recorder_down gap before it"),
                                    at,
                                );
                            }
                            self.end_boot();
                            (self.boots, self.partial) = (self.boots + 1, false);
                            self.boot_head = true;
                        } else if self.pending_down > 0 {
                            self.fail("inv6", "a recorder_down gap and no new boot", at);
                        }
                    }
                }
                self.pending_down = 0;
                self.boot = Some(*boot);
                self.part = *part;
                (self.part_wall, self.part_mono, self.t) = (*wall_ns, *mono_ns, *mono_ns);
                if self.boot_head {
                    self.cadence_t = *mono_ns;
                    self.last_count_wall = *wall_ns;
                }
            }
            Record::Open {
                sid,
                skind,
                conn,
                req,
                meta,
                ..
            } => self.open(*sid, *skind, *conn, *req, meta, at),
            Record::Data {
                sid,
                off,
                flags,
                bytes,
                ..
            } => self.data(*sid, *off, *flags, bytes, at),
            Record::End {
                sid,
                reason,
                seen,
                tail,
                ..
            } => self.end(*sid, *reason, *seen, tail, at),
            Record::Gap {
                sid,
                reason,
                start_wall,
                end_wall,
                start_mono,
                end_mono,
                off,
                lost,
                tail,
            } => {
                let tail: Map<String, Value> = serde_json::from_slice(tail).unwrap_or_default();
                let other_clock = tail.contains_key("boot_id");
                let (s, e) = if other_clock {
                    (*start_wall, *end_wall)
                } else {
                    (*start_mono, *end_mono)
                };
                if (e <= s && *lost <= 1) || end_wall < start_wall {
                    self.fail("gap_has_end", "a GAP with no extent (run 1's shape)", at);
                }
                let name = reason.name();
                if let GapReason::Other(_) = reason {
                    self.set(
                        "gap_reason_unknown",
                        Res::Warn,
                        &format!("a GAP has reason {name}"),
                        Some(at),
                    );
                }
                let lost_b = lost.saturating_sub(1);
                self.cur.add("gaps", 1);
                if *lost == 0 {
                    self.cur.add("lost_unknown", 1);
                } else {
                    self.cur.add("lost_bytes", lost_b);
                }
                let r = self.cur.by_reason.entry(name.clone()).or_default();
                (r.0, r.1, r.2) = (r.0 + 1, r.1 + lost_b, r.2 + u64::from(*lost == 0));
                if *reason == GapReason::AfterStop {
                    self.cur.after_stop += lost_b;
                }
                if is_down {
                    let ms = e.saturating_sub(s) / 1_000_000;
                    self.cur.add("off_ms", ms);
                    self.cur.off_spans += 1;
                    let cause = tail.get("cause").cloned().unwrap_or(Value::Null);
                    self.down_gaps.push(json!({"cause": cause, "start": wall(*start_wall), "end": wall(*end_wall), "ms": ms}));
                    self.down_after = (self.down_after.0 + 1, self.down_after.1 + ms);
                    if self.boot_head && self.boot == Some(0) && !self.boot0_gap {
                        self.boot0_gap = true;
                    } else {
                        let anchored = self
                            .last_link
                            .is_some_and(|(w, m)| *start_wall == w || *start_mono == m);
                        if !anchored {
                            self.fail("inv6", "a recorder_down gap not at the last link", at);
                        }
                        self.pending_down += 1;
                    }
                } else if *sid != 0 {
                    self.gap(*sid, *off, *lost, name, at);
                }
            }
            Record::Mask {
                sid,
                off,
                len,
                rule,
            } => {
                let Some(st) = self.streams.get(sid) else {
                    return self.fail("inv3", "a MASK of a stream not open", at);
                };
                let end = off + len;
                // The runs are in offset order: the one that starts at or before `off`.
                let i = st.stars.partition_point(|(a, _)| a <= off);
                if i == 0 || st.stars[i - 1].1 < end {
                    self.fail("inv5", "a MASK covers bytes that are not written as *", at);
                }
                if usize::try_from(*rule)
                    .ok()
                    .and_then(|i| self.mask_rules.get(i))
                    .is_some_and(|r| r == "fail_closed")
                {
                    self.cur.add("mask_failclosed_bytes", *len);
                }
            }
            Record::Event { .. } => self.event(&ev_name, ev, at),
            Record::Link {
                seq,
                wall_ns,
                mono_ns,
                hash,
            } => {
                self.links += 1;
                if it.link_ok == Some(false) {
                    self.fail("inv2", "a link hash does not recompute", at);
                }
                if *seq != self.next_seq {
                    self.fail(
                        "inv2",
                        &format!("link seq {seq} where {} was due", self.next_seq),
                        at,
                    );
                }
                self.next_seq = seq + 1;
                self.last_hash = *hash;
                self.last_link = Some((*wall_ns, *mono_ns));
                if self.down_after.0 == 0 {
                    self.regular_link_t = Some(*mono_ns);
                }
            }
            Record::Unknown { .. } => {
                self.set(
                    "record_kind_unknown",
                    Res::Warn,
                    "records of an unknown kind were skipped",
                    Some(at),
                );
                self.bump("record_kind_unknown", "records", 1);
            }
        }
    }

    fn open(&mut self, sid: u64, kind: StreamKind, conn: u64, req: u64, meta: &[u8], at: At) {
        if sid == 0 || !self.used.insert(sid) {
            return self.fail("inv3", "a stream id is 0 or used twice in one boot", at);
        }
        let meta: Map<String, Value> = serde_json::from_slice(meta).unwrap_or_default();
        let s = |k: &str| meta.get(k).and_then(Value::as_str).map(String::from);
        let from = meta.get("from").and_then(Value::as_u64).unwrap_or(0);
        let parent = self.streams.get(&conn).and_then(|c| c.host.clone());
        let mut st = St {
            kind: Some(kind),
            from,
            next: from,
            ..Default::default()
        };
        match kind {
            StreamKind::Conn => {
                self.cur.add("conns", 1);
                let host = s("host").unwrap_or_default();
                if s("mode").as_deref() == Some("tunnel") {
                    self.cur.add("tunnels", 1);
                    self.cur.tunnel.entry(host.clone()).or_default().0 += 1;
                    st.tunnel = Some(host);
                } else {
                    st.host = Some(host);
                }
            }
            StreamKind::HttpReq => {
                self.cur.add("http_req", 1);
                self.reqs.insert(sid, conn);
                st.post = parent.is_some() && s("method").as_deref() == Some("POST");
                st.host = parent;
            }
            StreamKind::HttpResp => {
                if req != 0 && self.reqs.get(&req) != Some(&conn) {
                    self.fail("inv9", "a response names no request on its connection", at);
                }
            }
            StreamKind::WsUp | StreamKind::WsDown => st.host = parent,
            StreamKind::Native => {
                self.cur.add("native_files", 1);
                let key = (
                    s("path").unwrap_or_default(),
                    meta.get("gen").and_then(Value::as_u64).unwrap_or(0),
                );
                if !self.natives_boot.insert(key.clone()) {
                    self.fail(
                        "native_path_gen",
                        "two native streams of one boot share path and gen",
                        at,
                    );
                }
                if let Some((end, unknown)) = self.natives.get(&key)
                    && from > *end
                    && !unknown
                {
                    self.fail("inv14", "a native stream resumes past the earlier end", at);
                }
                st.native = Some(key);
            }
            StreamKind::Snapshot => self.cur.add("snapshots", 1),
            StreamKind::Other(_) => {}
        }
        self.streams.insert(sid, st);
    }

    fn data(&mut self, sid: u64, off: u64, flags: u8, bytes: &[u8], at: At) {
        let len = bytes.len() as u64;
        self.data_bytes += len;
        let Some(st) = self.streams.get_mut(&sid) else {
            return self.fail("inv3", "a DATA of a stream not open", at);
        };
        let mut bad = st.kind == Some(StreamKind::Conn);
        match st.unknown.take() {
            Some(u) if off >= u => st.hole += off - u,
            Some(_) => bad = true,
            None => bad |= off != st.next,
        }
        st.next = off + len;
        let mut i = 0;
        while let Some(a) = bytes[i..].iter().position(|&b| b == b'*').map(|a| a + i) {
            let b = bytes[a..]
                .iter()
                .position(|&b| b != b'*')
                .map_or(bytes.len(), |b| a + b);
            let (s, e) = (off + a as u64, off + b as u64);
            match st.stars.last_mut() {
                Some(last) if last.1 == s => last.1 = e,
                _ => st.stars.push((s, e)),
            }
            i = b;
        }
        let (kind, host, msg_gap) = (
            st.kind.unwrap_or(StreamKind::Other(0)),
            st.host.clone(),
            st.msg_gap,
        );
        // A message end with a text or binary opcode; only that clears a gap before it.
        let msg = flags & FLAG_FIN != 0 && matches!(flags >> 4, 1 | 2);
        if msg {
            st.msg_gap = false;
        }
        let snap = matches!(kind, StreamKind::Native | StreamKind::Snapshot);
        self.cur
            .add(if snap { "native_bytes" } else { "wire_bytes" }, len);
        if msg && kind == StreamKind::WsDown {
            self.cur.add("ws_down_msgs", 1);
        }
        if msg && kind == StreamKind::WsUp {
            self.cur.add("ws_up_msgs", 1);
            self.request(host, !msg_gap);
        }
        if bad {
            self.fail("inv4", "a DATA hole that no GAP fills", at);
        }
    }

    /// A model request written (`http_post` or `ws_up_msgs` is counted by the caller).
    fn request(&mut self, host: Option<String>, whole: bool) {
        if let Some(h) = host {
            *self.cur.hosts.entry(h).or_default() += 1;
        }
        self.cur
            .add(if whole { "req_whole" } else { "req_holes" }, 1);
    }

    fn end(&mut self, sid: u64, reason: EndReason, seen: u64, tail: &[u8], at: At) {
        let Some(mut st) = self.streams.remove(&sid) else {
            return self.fail("inv3", "an END of a stream not open", at);
        };
        let tail: Map<String, Value> = serde_json::from_slice(tail).unwrap_or_default();
        if let Some(u) = st.unknown.take() {
            st.hole += seen.saturating_sub(u);
            st.next = seen;
        }
        if st.kind != Some(StreamKind::Conn) {
            if st.holed {
                self.set(
                    "seen_matches_written",
                    Res::Named,
                    "unknown-loss holes are reported, not checked",
                    Some(at),
                );
                self.bump("seen_matches_written", "holes", 1);
                self.bump("seen_matches_written", "hole_bytes", st.hole);
            } else if seen != st.next || seen < st.from {
                self.fail(
                    "seen_matches_written",
                    "an END's seen differs from what was written",
                    at,
                );
            }
        }
        if let Some(host) = &st.tunnel {
            let n = |k: &str| tail.get(k).and_then(Value::as_u64).unwrap_or(0);
            let (up, down) = (n("up"), n("down"));
            self.cur.add("tunnel_up", up);
            self.cur.add("tunnel_down", down);
            let t = self.cur.tunnel.entry(host.clone()).or_default();
            (t.1, t.2) = (t.1 + up, t.2 + down);
        }
        if st.post {
            self.cur.add("http_post", 1);
            self.request(st.host.clone(), reason == EndReason::Complete && !st.gapped);
        }
        if let Some(k) = st.native.take() {
            self.natives.insert(k, (st.next, st.holed));
        }
    }

    fn gap(&mut self, sid: u64, off: u64, lost: u64, reason: String, at: At) {
        let Some(st) = self.streams.get_mut(&sid) else {
            return self.fail("inv3", "a GAP of a stream not open", at);
        };
        (st.gapped, st.msg_gap) = (true, true);
        if let Some(host) = &st.tunnel {
            self.cur.tunnel.entry(host.clone()).or_default().3 = Some(reason);
        }
        if st.kind == Some(StreamKind::Conn) {
            return;
        }
        let bad = st.unknown.is_none() && off != st.next;
        if lost == 0 {
            st.unknown = Some(st.unknown.unwrap_or(off));
            st.holed = true;
        } else if st.unknown.is_none() {
            st.next = off + lost - 1;
        }
        if bad {
            self.fail("inv4", "a GAP not at the end of the written bytes", at);
        }
    }

    fn event(&mut self, ev: &str, m: Map<String, Value>, at: At) {
        match ev {
            "start" => {
                if let Some(Value::Array(r)) = m.get("mask_rules") {
                    self.mask_rules = r
                        .iter()
                        .map(|v| v.as_str().unwrap_or_default().to_string())
                        .collect();
                }
                self.start.get_or_insert(m);
            }
            "suspend" => self.suspend += m.get("ns").and_then(Value::as_u64).unwrap_or(0),
            "count" => {
                let d = differ(&Value::Object(m), &self.cur);
                if !d.is_empty() {
                    self.fail(
                        "inv7",
                        &format!("a COUNT does not equal the recount: {}", d.join(", ")),
                        at,
                    );
                }
                let gap = self
                    .t
                    .saturating_sub(self.cadence_t)
                    .saturating_sub(self.suspend);
                // A recovery frame's COUNT stands at the dead boot's last kept LINK, so a late one is
                // a real silence too.
                if gap > CADENCE_NS {
                    let why = format!("no COUNT for {} s within a boot", gap / 1_000_000_000);
                    self.fail("recorder_alive", &why, at);
                }
                (self.cadence_t, self.suspend, self.last_count_wall) = (self.t, 0, at.2);
                self.at_count = Some(self.cur.clone());
                self.down_after = (0, 0);
            }
            "seal" => {
                let mut total = self.totals.clone();
                total.merge(&self.boot_total());
                let d = differ(m.get("totals").unwrap_or(&Value::Null), &total);
                if !d.is_empty() {
                    self.fail(
                        "counts_match_seal",
                        &format!(
                            "the seal's totals differ from the recount: {}",
                            d.join(", ")
                        ),
                        at,
                    );
                }
                let recovered = m.get("reason").and_then(Value::as_str) == Some("recovered");
                if recovered && self.pending_down != 1 {
                    self.fail(
                        "inv6",
                        "a recovered seal without exactly one recorder_down gap",
                        at,
                    );
                }
                if self.pending_down == 0 && !self.streams.is_empty() {
                    self.fail(
                        "streams_closed",
                        &format!("{} streams have no END at the seal", self.streams.len()),
                        at,
                    );
                }
                self.pending_down = 0;
                self.seal = Some(m);
            }
            _ => {}
        }
    }

    /// This boot's part of the entry total (PRD-01 section 9): its last COUNT, plus the
    /// recorder_down gaps after it.
    fn boot_total(&self) -> Recount {
        let Some(mut r) = self.at_count.clone() else {
            return self.cur.clone();
        };
        let (n, ms) = self.down_after;
        r.add("gaps", n);
        r.add("lost_unknown", n);
        r.add("off_ms", ms);
        r
    }

    /// Closes the boot: cadence until its last regular link, native ends, totals.
    fn end_boot(&mut self) {
        if let Some(t) = self.regular_link_t.take() {
            let gap = t
                .saturating_sub(self.cadence_t)
                .saturating_sub(self.suspend);
            if gap > CADENCE_NS {
                let at = At(self.end_off, 0, self.wall_of(t));
                self.fail(
                    "recorder_alive",
                    &format!(
                        "no COUNT for {} s before the boot's last link",
                        gap / 1_000_000_000
                    ),
                    at,
                );
            }
        }
        for st in self.streams.values() {
            if let Some(k) = &st.native {
                self.natives
                    .insert(k.clone(), (st.next, st.holed || st.unknown.is_some()));
            }
        }
        let bt = self.boot_total();
        self.totals.merge(&bt);
        self.all.merge(&self.cur);
        self.streams.clear();
        self.used.clear();
        self.reqs.clear();
        self.natives_boot.clear();
        (self.cur, self.at_count, self.down_after, self.suspend) =
            (Recount::default(), None, (0, 0), 0);
    }

    fn finish(&mut self, open_frame: bool, now_ns: Option<u64>, e: &Entry) {
        if self.boot.is_none() {
            if now_ns.is_some() {
                self.set("inv1", Res::Skip, "nothing of the live entry is durable yet", None);
            } else {
                self.fail("inv1", "the record holds no PART", At(0, 0, 0));
            }
        }
        let open: Vec<Value> = self
            .streams
            .keys()
            .map(|s| format!("{}.{s}", self.boot.unwrap_or(0)).into())
            .collect();
        let sealed = self.seal.is_some();
        let last_count = self.last_count_wall;
        self.end_boot();
        if sealed && (!self.seal_link || open_frame) {
            let at = At(self.end_off, 0, self.wall_of(self.t));
            self.fail("inv8", "the seal lacks its link or frame end", at);
        }
        if !sealed {
            self.set("counts_match_seal", Res::Skip, "not sealed", None);
        } else if self.pruned {
            self.set(
                "counts_match_seal",
                Res::Skip,
                "local part files were pruned",
                None,
            );
        }
        if !sealed && !open.is_empty() {
            let why = format!(
                "the entry is not sealed: {} streams are still open",
                open.len()
            );
            self.set("streams_closed", Res::Named, &why, None);
            self.fact("streams_closed", "open", open.into());
        }
        if let Some(now) = now_ns
            && !sealed
            && self.boot.is_some()
            && now.saturating_sub(last_count) > CADENCE_NS
        {
            let status: Map<String, Value> = std::fs::read(e.dir.join("status.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            let since = status.get("off").and_then(|o| o.get("since"));
            if status.get("mode").and_then(Value::as_str) == Some("null") {
                let since = since.and_then(Value::as_str).unwrap_or("an unknown time");
                self.set(
                    "recorder_alive",
                    Res::Named,
                    &format!("null mode since {since}"),
                    None,
                );
            } else {
                let why = format!(
                    "no COUNT for {} s: the recorder and its watchdog may both be gone",
                    (now - last_count) / 1_000_000_000
                );
                self.set("recorder_alive", Res::Fail, &why, None);
            }
        }
        let n = self.down_gaps.len();
        if n > 0 {
            let causes: Vec<String> = self
                .down_gaps
                .iter()
                .map(|g| {
                    format!(
                        "{} {} s",
                        g["cause"].as_str().unwrap_or("unknown"),
                        g["ms"].as_u64().unwrap_or(0) / 1000
                    )
                })
                .collect();
            self.set(
                "inv6",
                Res::Named,
                &format!("recording was off {n} times: {}", causes.join(", ")),
                None,
            );
        }
        let gaps = std::mem::take(&mut self.down_gaps);
        self.fact("inv6", "gaps", gaps.into());
        let by: Map<String, Value> = self
            .all
            .by_reason
            .iter()
            .map(|(r, (n, b, u))| {
                (
                    r.clone(),
                    json!({"count": n, "lost_bytes": b, "lost_unknown": u}),
                )
            })
            .collect();
        let total = self.all.get("gaps");
        if total > 0 {
            self.set(
                "gaps_named",
                Res::Named,
                &format!("{total} GAPs record a loss"),
                None,
            );
        }
        let i = self.idx("gaps_named");
        self.checks[i].facts = by;
    }
}
