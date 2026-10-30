//! Written counts (seam S4): the COUNT keys of PRD-01 section 9, the durable point, entry totals.

use super::clock::Mono;
use super::OpenFlags;
use super::status::write_private;
use goalcup_format::{EndReason, GapReason, Record, StreamKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The COUNT keys of PRD-01 section 9, counted at write. As entry totals (over boots): counters
/// summed, maps summed per key, `held_peak` the largest, gauges left out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Counts {
    pub http_req: u64,
    pub http_post: u64,
    pub ws_up_msgs: u64,
    pub ws_down_msgs: u64,
    pub req_whole: u64,
    pub req_holes: u64,
    pub conns: u64,
    pub tunnels: u64,
    pub tunnel_up: u64,
    pub tunnel_down: u64,
    pub wire_bytes: u64,
    pub native_files: u64,
    pub native_bytes: u64,
    pub snapshots: u64,
    pub gaps: u64,
    pub lost_bytes: u64,
    pub lost_unknown: u64,
    pub off_ms: u64,
    pub mask_failclosed_bytes: u64,
    /// Model requests written per decrypted host.
    pub hosts: BTreeMap<String, u64>,
    pub tunnel_hosts: BTreeMap<String, TunnelHost>,
    pub held_peak: u64,
}

impl Counts {
    /// "Model requests written": `http_post + ws_up_msgs`. A request count, never a call count.
    pub fn model_requests(&self) -> u64 {
        self.http_post + self.ws_up_msgs
    }

    /// Adds another boot's counts as PRD-01 section 9's entry total does: counters summed, maps
    /// summed per key, `held_peak` the largest.
    pub fn add(&mut self, o: &Counts) {
        for (a, b) in [
            (&mut self.http_req, o.http_req),
            (&mut self.http_post, o.http_post),
            (&mut self.ws_up_msgs, o.ws_up_msgs),
            (&mut self.ws_down_msgs, o.ws_down_msgs),
            (&mut self.req_whole, o.req_whole),
            (&mut self.req_holes, o.req_holes),
            (&mut self.conns, o.conns),
            (&mut self.tunnels, o.tunnels),
            (&mut self.tunnel_up, o.tunnel_up),
            (&mut self.tunnel_down, o.tunnel_down),
            (&mut self.wire_bytes, o.wire_bytes),
            (&mut self.native_files, o.native_files),
            (&mut self.native_bytes, o.native_bytes),
            (&mut self.snapshots, o.snapshots),
            (&mut self.gaps, o.gaps),
            (&mut self.lost_bytes, o.lost_bytes),
            (&mut self.lost_unknown, o.lost_unknown),
            (&mut self.off_ms, o.off_ms),
            (&mut self.mask_failclosed_bytes, o.mask_failclosed_bytes),
        ] {
            *a = a.saturating_add(b);
        }
        for (host, n) in &o.hosts {
            let a = self.hosts.entry(host.clone()).or_default();
            *a = a.saturating_add(*n);
        }
        for (host, t) in &o.tunnel_hosts {
            let a = self.tunnel_hosts.entry(host.clone()).or_default();
            a.conns = a.conns.saturating_add(t.conns);
            a.up = a.up.saturating_add(t.up);
            a.down = a.down.saturating_add(t.down);
            if t.listed.is_some() {
                a.listed.clone_from(&t.listed);
            }
        }
        self.held_peak = self.held_peak.max(o.held_peak);
    }
}

/// One tunnelled host in COUNT `tunnel_hosts`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelHost {
    pub conns: u64,
    pub up: u64,
    pub down: u64,
    /// For a listed host, the GAP reason name of its tunnels; null for an unlisted host.
    pub listed: Option<String>,
}

/// Entry totals (PRD-01 section 9, PRD-00 R3-2): the COUNT keys over every boot, plus
/// `off_spans`, the number of `recorder_down` spans on `sid 0`. `off_spans` is never a COUNT key
/// and is never written to a COUNT event; the heartbeat `off_spans` and the sealed `off.spans`
/// take it from here, `off_ms` from `counts`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryTotals {
    pub counts: Counts,
    pub off_spans: u64,
}

/// The durable point, published on a `watch` channel after each `fdatasync` (PRD-02 section 5
/// step 4). The uploader sends up to it and never past it, cutting chunks by index lines.
/// `totals` are entry totals as they stood at this LINK, so a count that leaves the machine never
/// runs ahead of the record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Durable {
    /// The part file, its index length and the index line, as `PartWriter::sync`, `rotate_if_due`
    /// and `seal` return it. None before the first durable point.
    pub point: Option<goalcup_format::Durable>,
    pub totals: EntryTotals,
    /// When this point became durable, for its age.
    pub at: Mono,
}

/// One open stream in the writer's per-sid table. Dropped at END, so the table holds only what is
/// open.
struct Open {
    kind: StreamKind,
    /// An `http_req` with `POST` on a decrypted connection: a model request once its END is
    /// written.
    post: bool,
    /// A GAP was written on this stream (for `ws_up`: since its last message end).
    gap: bool,
    /// A `conn` stream's own host; a request's or message's connection host.
    host: Option<Arc<str>>,
    /// A `conn` stream that is a tunnel.
    tunnel: bool,
}

/// The writer's count at write (PRD-02 section 6, PRD-01 section 9). The writer calls
/// [`Counter::written`] once per record, after the record is written, so every count is a count
/// of what is in the record.
pub struct Counter {
    /// The rule numbers of the start event's `mask_rules` that name `fail_closed`.
    fail_closed: Vec<u64>,
    counts: Counts,
    open: HashMap<u64, Open>,
}

impl Counter {
    /// `mask_rules` is the start event's list, which MASK `rule` indexes (PRD-01 section 8).
    pub fn new(mask_rules: &[String]) -> Counter {
        Counter {
            fail_closed: (0u64..)
                .zip(mask_rules)
                .filter(|(_, r)| *r == "fail_closed")
                .map(|(i, _)| i)
                .collect(),
            counts: Counts::default(),
            open: HashMap::new(),
        }
    }

    /// Counts one written record. `flags` matter only for OPEN; the only OPEN meta read is a
    /// `conn` stream's `host` and `mode`.
    pub fn written(&mut self, r: &Record, flags: OpenFlags) {
        let c = &mut self.counts;
        match r {
            Record::Open {
                sid,
                skind,
                conn,
                meta,
                ..
            } => {
                let mut o = Open {
                    kind: *skind,
                    post: false,
                    gap: false,
                    host: None,
                    tunnel: false,
                };
                match skind {
                    StreamKind::Conn => {
                        let m: Value = serde_json::from_slice(meta).unwrap_or(Value::Null);
                        let host: Arc<str> = m["host"].as_str().unwrap_or_default().into();
                        c.conns += 1;
                        o.tunnel = m["mode"] == "tunnel" || flags.contains(OpenFlags::TUNNEL);
                        if o.tunnel {
                            c.tunnels += 1;
                            c.tunnel_hosts.entry(host.to_string()).or_default().conns += 1;
                        }
                        o.host = Some(host);
                    }
                    StreamKind::HttpReq => {
                        c.http_req += 1;
                        o.post = flags.contains(OpenFlags::POST | OpenFlags::DECRYPTED);
                        o.host = self.open.get(conn).and_then(|x| x.host.clone());
                    }
                    StreamKind::WsUp => {
                        o.host = self.open.get(conn).and_then(|x| x.host.clone());
                    }
                    StreamKind::Native => c.native_files += 1,
                    StreamKind::Snapshot => c.snapshots += 1,
                    _ => {}
                }
                self.open.insert(*sid, o);
            }
            Record::Data {
                sid,
                flags: f,
                bytes,
                ..
            } => {
                let Some(o) = self.open.get_mut(sid) else {
                    return;
                };
                let n = bytes.len() as u64;
                match o.kind {
                    StreamKind::Native | StreamKind::Snapshot => c.native_bytes += n,
                    _ => c.wire_bytes += n,
                }
                // A message end with a text or binary opcode (PRD-01 section 3 DATA flags).
                let msg = f & goalcup_format::FLAG_FIN != 0 && matches!(f >> 4, 0x1 | 0x2);
                match o.kind {
                    StreamKind::WsUp if msg => {
                        c.ws_up_msgs += 1;
                        whole_or_holes(c, !o.gap, o.host.as_deref());
                        o.gap = false;
                    }
                    StreamKind::WsDown if msg => c.ws_down_msgs += 1,
                    _ => {}
                }
            }
            Record::End {
                sid, reason, tail, ..
            } => {
                let Some(o) = self.open.remove(sid) else {
                    return;
                };
                if o.post {
                    c.http_post += 1;
                    let whole = *reason == EndReason::Complete && !o.gap;
                    whole_or_holes(c, whole, o.host.as_deref());
                }
                if o.tunnel {
                    let t: Value = serde_json::from_slice(tail).unwrap_or(Value::Null);
                    let (up, down) = (t["up"].as_u64().unwrap_or(0), t["down"].as_u64().unwrap_or(0));
                    c.tunnel_up += up;
                    c.tunnel_down += down;
                    let h = c
                        .tunnel_hosts
                        .entry(o.host.as_deref().unwrap_or_default().to_string())
                        .or_default();
                    h.up += up;
                    h.down += down;
                }
            }
            Record::Gap {
                sid, reason, lost, ..
            } => {
                c.gaps += 1;
                match lost {
                    0 => c.lost_unknown += 1,
                    n => c.lost_bytes += n - 1,
                }
                if *sid == 0 && *reason == GapReason::RecorderDown {
                    c.off_ms += off_ms(r);
                }
                if let Some(o) = self.open.get_mut(sid) {
                    o.gap = true;
                    if o.tunnel {
                        c.tunnel_hosts
                            .entry(o.host.as_deref().unwrap_or_default().to_string())
                            .or_default()
                            .listed = Some(reason.name());
                    }
                }
            }
            Record::Mask { len, rule, .. } => {
                if self.fail_closed.contains(rule) {
                    c.mask_failclosed_bytes += len;
                }
            }
            _ => {}
        }
    }

    /// Raises this boot's `held_peak` to `held_bytes` when it is above it.
    pub fn held(&mut self, held_bytes: u64) {
        self.counts.held_peak = self.counts.held_peak.max(held_bytes);
    }

    /// This boot's counts so far.
    pub fn counts(&self) -> Counts {
        self.counts.clone()
    }

    /// Streams opened and not yet ended in this boot.
    pub fn open_streams(&self) -> u64 {
        self.open.len() as u64
    }

    /// The COUNT event (PRD-01 section 9): every key, over the records written before it in this
    /// boot. `held_bytes` is the gauge now; `held_peak` the boot's peak.
    pub fn count_event(&self, boot: u64, up_ms: u64, held_bytes: u64, held_peak: u64) -> Value {
        let mut v = serde_json::to_value(&self.counts).unwrap_or(Value::Null);
        if let Value::Object(m) = &mut v {
            m.insert("ev".into(), "count".into());
            m.insert("boot".into(), boot.into());
            m.insert("up_ms".into(), up_ms.into());
            m.insert("open_streams".into(), self.open_streams().into());
            m.insert("held_bytes".into(), held_bytes.into());
            m.insert(
                "held_peak".into(),
                held_peak.max(self.counts.held_peak).into(),
            );
        }
        v
    }
}

/// One model request written: whole (ended complete with no gap) or with holes, and its host.
fn whole_or_holes(c: &mut Counts, whole: bool, host: Option<&str>) {
    if whole {
        c.req_whole += 1;
    } else {
        c.req_holes += 1;
    }
    if let Some(h) = host {
        *c.hosts.entry(h.to_string()).or_default() += 1;
    }
}

/// The length of a GAP in whole ms: on the record clock, or on wall time when its tail has
/// `boot_id` (its end was taken under another clock, PRD-01 section 6).
fn off_ms(r: &Record) -> u64 {
    let Record::Gap {
        start_wall,
        end_wall,
        start_mono,
        end_mono,
        tail,
        ..
    } = r
    else {
        return 0;
    };
    let wall = serde_json::from_slice::<Value>(tail).is_ok_and(|t| t.get("boot_id").is_some());
    let ns = if wall {
        end_wall.saturating_sub(*start_wall)
    } else {
        end_mono.saturating_sub(*start_mono)
    };
    ns / 1_000_000
}

/// `counts.jsonl` (PRD-02 section 5 step 3), local only: one compact line per LINK,
/// `{"seq":<link seq>,"count":<the COUNT event JSON at that LINK>}`. A later boot takes a dead
/// boot's COUNT from it without decoding part files.
pub struct CountsLog {
    path: PathBuf,
    /// The last line of each earlier boot, by boot.
    earlier: BTreeMap<u64, String>,
    /// This boot's latest line and its boot.
    latest: Option<(u64, String)>,
}

impl CountsLog {
    const NAME: &'static str = "counts.jsonl";

    /// Reads what the file holds. A torn last line is skipped and the file rewritten without it.
    pub fn open(entry_dir: &Path) -> io::Result<CountsLog> {
        let path = entry_dir.join(Self::NAME);
        let text = match std::fs::read(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let mut log = CountsLog {
            path,
            earlier: BTreeMap::new(),
            latest: None,
        };
        for (line, v) in lines(&text) {
            let boot = v["count"]["boot"].as_u64().unwrap_or(0);
            log.push(boot, line.to_string());
        }
        if !text.is_empty() && !text.ends_with(b"\n") {
            log.compact()?;
        }
        Ok(log)
    }

    fn push(&mut self, boot: u64, line: String) {
        if let Some((b, l)) = self.latest.take()
            && b != boot
        {
            self.earlier.insert(b, l);
        }
        self.latest = Some((boot, line));
    }

    /// Appends one line at a LINK: this boot's COUNT event JSON at the LINK with that `seq`.
    pub fn append(&mut self, seq: u64, count: &Value) -> io::Result<()> {
        let line = serde_json::json!({ "seq": seq, "count": count }).to_string();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&self.path)?;
        f.write_all(format!("{line}\n").as_bytes())?;
        self.push(count["boot"].as_u64().unwrap_or(0), line);
        Ok(())
    }

    /// Rewrites the file by atomic rename, written and synced before the rename, to one line per
    /// earlier boot plus this boot's latest.
    pub fn compact(&mut self) -> io::Result<()> {
        let mut out = String::new();
        for l in self.earlier.values().chain(self.latest.as_ref().map(|(_, l)| l)) {
            out.push_str(l);
            out.push('\n');
        }
        write_private(&self.path, out.as_bytes(), true)
    }
}

/// The whole, parseable lines of a `counts.jsonl` text. A last line without a newline is torn and
/// skipped; so is any line that does not parse.
fn lines(text: &[u8]) -> impl Iterator<Item = (&str, Value)> {
    let whole = text
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(&text[..0], |i| &text[..i]);
    whole.split(|&b| b == b'\n').filter_map(|l| {
        let s = std::str::from_utf8(l).ok()?;
        let v: Value = serde_json::from_str(s).ok()?;
        v.get("seq")?.as_u64()?;
        Some((s, v))
    })
}

/// The COUNT event JSON of the `counts.jsonl` line whose `seq` is `seq`: what
/// `goalcup_format::Recover::count_at` needs. None when no whole line has it.
pub fn count_at(entry_dir: &Path, seq: u64) -> Option<Vec<u8>> {
    let text = std::fs::read(entry_dir.join(CountsLog::NAME)).ok()?;
    lines(&text)
        .filter(|(_, v)| v["seq"].as_u64() == Some(seq))
        .last()
        .and_then(|(_, v)| serde_json::to_vec(&v["count"]).ok())
}

/// Entry totals over every boot of the entry in `entry_dir` (PRD-01 section 9): one function for
/// boot recovery, `close_dead_entry`, the null recorder and `goalcup upload`.
///
/// Per boot (the PART before it) the last COUNT event, summed with [`Counts::add`]; then every
/// `sid 0` `recorder_down` GAP adds one off span, and, when no later COUNT of its boot covers it
/// (a recovery frame's GAP follows that frame's COUNT), 1 to `gaps` and `lost_unknown` and its
/// length to `off_ms`. A boot whose part files were pruned takes its last `counts.jsonl` line.
/// Reads up to where the reader stops (a torn or unclosed tail of a live entry), never panics and
/// never fails: no part file gives the default.
pub fn entry_totals(entry_dir: &Path) -> EntryTotals {
    #[derive(Default)]
    struct Boot {
        last: Option<Counts>,
        /// `recorder_down` lengths in ms after the last COUNT.
        down: Vec<u64>,
    }
    let mut boots: BTreeMap<u64, Boot> = BTreeMap::new();
    let mut off_spans = 0;
    if let Ok(reader) = goalcup_format::read::open_dir(&entry_dir.join("rec")) {
        let mut boot = None;
        for item in reader {
            let Ok(item) = item else { break };
            match &item.rec {
                Record::Part { boot: b, .. } => boot = Some(*b),
                Record::Event { json, .. } => {
                    let Ok(v) = serde_json::from_slice::<Value>(json) else {
                        continue;
                    };
                    if v["ev"] == "count"
                        && let Some(b) = boot
                    {
                        let e = boots.entry(b).or_default();
                        e.last = Some(serde_json::from_value(v).unwrap_or_default());
                        e.down.clear();
                    }
                }
                r @ Record::Gap {
                    sid: 0,
                    reason: GapReason::RecorderDown,
                    ..
                } => {
                    off_spans += 1;
                    boots.entry(boot.unwrap_or(0)).or_default().down.push(off_ms(r));
                }
                _ => {}
            }
        }
    }
    // Boots before the first part file left on disk (pruned after upload): their last line.
    let first = boots.keys().next().copied().unwrap_or(u64::MAX);
    if let Ok(text) = std::fs::read(entry_dir.join(CountsLog::NAME)) {
        for (_, v) in lines(&text) {
            if let Some(b) = v["count"]["boot"].as_u64().filter(|b| *b < first) {
                boots.entry(b).or_default().last =
                    serde_json::from_value(v["count"].clone()).ok();
            }
        }
    }
    let mut t = EntryTotals::default();
    for b in boots.values() {
        if let Some(c) = &b.last {
            t.counts.add(c);
        }
        for ms in &b.down {
            t.counts.gaps += 1;
            t.counts.lost_unknown += 1;
            t.counts.off_ms += ms;
        }
    }
    t.off_spans = off_spans;
    t
}
