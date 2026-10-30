//! Written counts (seam S4): the COUNT keys of PRD-01 section 9, the durable point, entry totals.

use super::OpenFlags;
use super::clock::Mono;
use super::status::write_private;
use goalcup_format::{EndReason, GapReason, IndexLine, Record, StreamKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, Read, Write};
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
    /// summed per key, `held_peak` the largest. It saturates, since its input is read from disk.
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
    /// `recorder_down` spans this boot wrote (its own start gap), for `counts.jsonl`.
    spans: u64,
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
            spans: 0,
            open: HashMap::new(),
        }
    }

    /// Counts one written record. `flags` and `host` matter only for OPEN; no meta is read back,
    /// since it is masked by then (PRD-05 amendment). `host` is a `conn` stream's host, which the
    /// mask worker takes from the OPEN meta before `mask_meta` and runs through the masker on its
    /// own, so COUNT `hosts` equals a recount from the masked OPEN meta and a host holding a
    /// secret shape never reaches disk. A tunnel is known by `OpenFlags::TUNNEL` alone.
    pub fn written(&mut self, r: &Record, flags: OpenFlags, host: Option<&str>) {
        let c = &mut self.counts;
        match r {
            Record::Open {
                sid, skind, conn, ..
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
                        let host: Arc<str> = host.unwrap_or_default().into();
                        c.conns += 1;
                        o.tunnel = flags.contains(OpenFlags::TUNNEL);
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
                    let (up, down) = (
                        t["up"].as_u64().unwrap_or(0),
                        t["down"].as_u64().unwrap_or(0),
                    );
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
                    self.spans += 1;
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
            Record::Mask { len, rule, .. } if self.fail_closed.contains(rule) => {
                c.mask_failclosed_bytes += len;
            }
            _ => {}
        }
    }

    /// This boot's counts so far.
    pub fn counts(&self) -> Counts {
        self.counts.clone()
    }

    /// The `recorder_down` spans written in this boot so far: what [`CountsLog::append`] takes,
    /// never a COUNT key.
    pub fn off_spans(&self) -> u64 {
        self.spans
    }

    /// Streams opened and not yet ended in this boot.
    pub fn open_streams(&self) -> u64 {
        self.open.len() as u64
    }

    /// The COUNT event (PRD-01 section 9): every key, over the records written before it in this
    /// boot. `held_bytes` is the gauge now; `held_peak` the boot's peak, from the recorder.
    pub fn count_event(&self, boot: u64, up_ms: u64, held_bytes: u64, held_peak: u64) -> Value {
        let mut v = serde_json::to_value(&self.counts).unwrap_or(Value::Null);
        if let Value::Object(m) = &mut v {
            m.insert("ev".into(), "count".into());
            m.insert("boot".into(), boot.into());
            m.insert("up_ms".into(), up_ms.into());
            m.insert("open_streams".into(), self.open_streams().into());
            m.insert("held_bytes".into(), held_bytes.into());
            m.insert("held_peak".into(), held_peak.into());
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

/// One boot as `counts.jsonl` keeps it, and as [`entry_totals`] reads it from the part files.
#[derive(Default, Serialize, Deserialize)]
struct Boot {
    /// The LINK of the line: the COUNT's LINK when appended, the boot's last LINK when synced
    /// (as read: its last LINK up to the last whole index line).
    seq: u64,
    /// The boot's last COUNT event JSON.
    count: Option<Value>,
    /// Its `recorder_down` spans on `sid 0`: boot 0's start gap and its recovery frames' gaps.
    #[serde(default)]
    spans: u64,
    /// The lengths in ms of its `recorder_down` gaps after its last COUNT (a recovery frame's
    /// gap), or of all of them while it has none: no COUNT covers them.
    #[serde(default)]
    down_ms: Vec<u64>,
    /// Scan only: the last LINK read before `count` was taken, so a line with a later `seq`
    /// holds a later count.
    #[serde(skip)]
    count_after: Option<u64>,
}

impl Boot {
    fn boot(&self) -> Option<u64> {
        self.count.as_ref()?["boot"].as_u64()
    }
}

/// `counts.jsonl` (PRD-02 section 5 step 3), local only, one compact line per LINK:
/// `{"seq":<link seq>,"count":<the COUNT event JSON at that LINK>,"spans":<n>}`, where `spans`
/// is the boot's own `recorder_down` spans so far ([`Counter::off_spans`]). A later boot takes
/// a dead boot's COUNT from it without decoding part files, and [`entry_totals`] takes a boot's
/// totals from it once the boot's part files are pruned after upload (ledger U1).
pub struct CountsLog {
    path: PathBuf,
    /// The last line of each boot, by boot.
    by_boot: BTreeMap<u64, String>,
}

impl CountsLog {
    const NAME: &'static str = "counts.jsonl";

    /// The log of the entry in `entry_dir`. It reads nothing: [`CountsLog::sync`] rewrites the
    /// file from disk at boot, before the first [`CountsLog::append`].
    pub fn open(entry_dir: &Path) -> CountsLog {
        CountsLog {
            path: entry_dir.join(Self::NAME),
            by_boot: BTreeMap::new(),
        }
    }

    /// Appends one line at a LINK: this boot's COUNT event JSON at the LINK with that `seq`, and
    /// the boot's own `recorder_down` spans so far.
    pub fn append(&mut self, seq: u64, count: &Value, spans: u64) -> io::Result<()> {
        let line = serde_json::json!({ "seq": seq, "count": count, "spans": spans }).to_string();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&self.path)?;
        f.write_all(format!("{line}\n").as_bytes())?;
        self.by_boot
            .insert(count["boot"].as_u64().unwrap_or(0), line);
        Ok(())
    }

    /// Rewrites the file by atomic rename, written and synced before the rename, to one line per
    /// boot.
    pub fn compact(&mut self) -> io::Result<()> {
        let mut out = String::new();
        for l in self.by_boot.values() {
            out.push_str(l);
            out.push('\n');
        }
        write_private(&self.path, out.as_bytes(), true)
    }

    /// At boot, after recovery and before this boot's first LINK (PRD-02 section 9.2): rewrites
    /// the file to one line per earlier boot as [`entry_totals`] reads it (its last COUNT on
    /// disk, its `recorder_down` spans and the lengths of those after that COUNT), drops
    /// lines of boots that left nothing durable, and returns the entry totals over those boots.
    /// Every line comes from what is on disk, so a crash at any point of a recovery is repaired
    /// by the next boot's call: only a live recorder prunes part files, after this call. It runs
    /// before this boot writes any record to disk (its start event and boot 0's start gap stay
    /// in the encoder until the first LINK), so the totals hold no record of this boot and its
    /// `Counter` adds them once.
    pub fn sync(&mut self, entry_dir: &Path) -> io::Result<EntryTotals> {
        let boots = boots(entry_dir);
        self.by_boot = boots
            .iter()
            .filter(|(_, b)| b.count.is_some())
            .filter_map(|(n, b)| Some((*n, serde_json::to_string(b).ok()?)))
            .collect();
        self.compact()?;
        Ok(totals(&boots))
    }
}

/// The whole, parseable lines of a `counts.jsonl` text. A last line without a newline is torn and
/// skipped; so is any line that does not parse.
fn lines(text: &[u8]) -> impl Iterator<Item = Boot> {
    let whole = text
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(&text[..0], |i| &text[..i]);
    whole
        .split(|&b| b == b'\n')
        .filter_map(|l| serde_json::from_slice(l).ok())
}

/// The COUNT event JSON of the `counts.jsonl` line whose `seq` is `seq`: what
/// `goalcup_format::Recover::count_at` needs. None when no whole line has it.
pub fn count_at(entry_dir: &Path, seq: u64) -> Option<Vec<u8>> {
    let text = std::fs::read(entry_dir.join(CountsLog::NAME)).ok()?;
    lines(&text)
        .filter(|b| b.seq == seq)
        .last()
        .and_then(|b| serde_json::to_vec(&b.count?).ok())
}

/// Entry totals over every boot of the entry in `entry_dir` (PRD-01 section 9): one function for
/// boot recovery, `close_dead_entry`, the null recorder and `goalcup upload`.
///
/// Per boot (the PART before it) the last COUNT event, summed with [`Counts::add`]; then every
/// `sid 0` `recorder_down` GAP adds one off span, and, when no later COUNT of its boot covers it
/// (a recovery frame's GAP follows that frame's COUNT), 1 to `gaps` and `lost_unknown` and its
/// length to `off_ms`. Reads each part file up to its last whole index line, which recovery keeps,
/// so a boot's totals never go down when it is recovered; never panics and never fails: no part
/// file gives the default.
///
/// It decodes every part file left on disk: the writer calls it once at boot (through
/// [`CountsLog::sync`]) and adds this boot's counter after, never at each durable point.
pub fn entry_totals(entry_dir: &Path) -> EntryTotals {
    totals(&boots(entry_dir))
}

/// Every boot of the entry. Each part file is read only up to its last whole index line, what
/// recovery keeps (PRD-01 section 12 step 1), so no count here is one a recovery takes back.
///
/// A boot read from part files counts from its last COUNT event. While it has no recovery frame
/// (a COUNT, then a `recorder_down` gap), its latest `counts.jsonl` line up to its last LINK read
/// wins when later: a dead boot not yet recovered has a line at each LINK but a COUNT event only
/// every 60 s, and its recovery frame will take that line. Its own spans are the larger of what
/// its part files and its line hold, since its first part file (boot 0's start gap) may be
/// pruned.
///
/// A boot not read (its part files pruned, or a read failed in it or before it) is its latest
/// line up to the last whole index line on disk; the boot a read failed in takes that line too
/// when it is later than what was read. Lines of a later boot after a whole read are left out:
/// that boot left nothing recovery keeps (its part file is deleted and its number reused).
fn boots(entry_dir: &Path) -> BTreeMap<u64, Boot> {
    let (mut boots, stopped, kept) = scan(&entry_dir.join("rec"));
    let first = boots.keys().next().copied().unwrap_or(u64::MAX);
    // Each boot's latest line within its bound; on a tie the later one in the file.
    let mut latest: BTreeMap<u64, Boot> = BTreeMap::new();
    if let Ok(text) = std::fs::read(entry_dir.join(CountsLog::NAME)) {
        for line in lines(&text) {
            let Some(n) = line.boot() else { continue };
            let bound = match boots.get(&n) {
                Some(b) if stopped != Some(n) => Some(b.seq),
                _ => kept,
            };
            if bound.is_some_and(|s| line.seq > s) {
                continue;
            }
            match latest.entry(n) {
                Entry::Occupied(e) if e.get().seq > line.seq => {}
                Entry::Occupied(mut e) => {
                    let spans = e.get().spans.max(line.spans);
                    *e.get_mut() = Boot { spans, ..line };
                }
                Entry::Vacant(e) => {
                    e.insert(line);
                }
            }
        }
    }
    for (n, line) in latest {
        match boots.get_mut(&n) {
            Some(b) => {
                let recovered = b.count.is_some() && !b.down_ms.is_empty();
                let (seq, spans) = (b.seq.max(line.seq), b.spans.max(line.spans));
                if !recovered && b.count_after.is_none_or(|s| line.seq > s) {
                    // The line whole: a synced line keeps the recovery gaps a partial read
                    // stopped before, and an appended one has none (its start gap is in
                    // `spans`).
                    *b = Boot { seq, spans, ..line };
                } else {
                    b.spans = spans;
                }
            }
            None if n < first || stopped.is_some_and(|s| n >= s) => {
                boots.insert(n, line);
            }
            None => {}
        }
    }
    boots
}

/// Reads every part file of `rec` up to its last whole index line. Returns the boots read, the
/// boot a failed read stopped in (0 before any PART), and the `seq` of the last whole index line
/// on disk (None when there is none, or the index that would hold it could not be read).
fn scan(rec: &Path) -> (BTreeMap<u64, Boot>, Option<u64>, Option<u64>) {
    let mut boots: BTreeMap<u64, Boot> = BTreeMap::new();
    let files = match goalcup_format::read::part_files(rec) {
        Ok(files) => files,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return (boots, Some(0), None),
    };
    let ends: Vec<io::Result<Option<IndexLine>>> = files.iter().map(|p| last_line(p)).collect();
    let mut kept = None;
    for end in ends.iter().rev() {
        match end {
            Ok(None) => continue,
            Ok(Some(l)) => kept = Some(l.seq),
            Err(_) => {}
        }
        break;
    }
    let mut boot = None;
    for (p, end) in files.iter().zip(ends) {
        let end = match end {
            Ok(Some(l)) => l.c_end,
            // Recovery deletes a part file with no whole index line.
            Ok(None) => continue,
            Err(_) => return (boots, Some(boot.unwrap_or(0)), kept),
        };
        let reader = match File::open(p).and_then(|f| goalcup_format::read::decode(f.take(end))) {
            Ok(r) => r,
            // Pruned after upload since it was listed.
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return (boots, Some(boot.unwrap_or(0)), kept),
        };
        let mut link = None;
        for item in reader {
            let Ok(item) = item else {
                return (boots, Some(boot.unwrap_or(0)), kept);
            };
            let n = boot.unwrap_or(0);
            match &item.rec {
                Record::Part {
                    boot: b, next_link, ..
                } => (boot, link) = (Some(*b), next_link.checked_sub(1)),
                Record::Link { seq, .. } => {
                    boots.entry(n).or_default().seq = *seq;
                    link = Some(*seq);
                }
                Record::Event { json, .. } => {
                    let Ok(v) = serde_json::from_slice::<Value>(json) else {
                        continue;
                    };
                    if v["ev"] == "count" {
                        let e = boots.entry(n).or_default();
                        (e.count, e.count_after) = (Some(v), link);
                        e.down_ms.clear();
                    }
                }
                r @ Record::Gap {
                    sid: 0,
                    reason: GapReason::RecorderDown,
                    ..
                } => {
                    let b = boots.entry(n).or_default();
                    b.down_ms.push(off_ms(r));
                    b.spans += 1;
                }
                _ => {}
            }
        }
    }
    (boots, None, kept)
}

/// A part file's last whole index line: one whose `c_end` is within the file, as recovery reads
/// it. None when it has none, or when the files are gone (pruned since they were listed).
fn last_line(part: &Path) -> io::Result<Option<IndexLine>> {
    let gone = |e: io::Error| match e.kind() {
        io::ErrorKind::NotFound => Ok(None),
        _ => Err(e),
    };
    let len = match std::fs::metadata(part) {
        Ok(m) => m.len(),
        Err(e) => return gone(e),
    };
    let name = part
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let idx = part.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")));
    match goalcup_format::read::read_index(&idx) {
        Ok(lines) => Ok(lines.into_iter().take_while(|l| l.c_end <= len).last()),
        Err(e) => gone(e),
    }
}

fn totals(boots: &BTreeMap<u64, Boot>) -> EntryTotals {
    let mut t = EntryTotals::default();
    for b in boots.values() {
        let mut c: Counts = b
            .count
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let n = b.down_ms.len() as u64;
        (c.gaps, c.lost_unknown) = (c.gaps.saturating_add(n), c.lost_unknown.saturating_add(n));
        c.off_ms = b
            .down_ms
            .iter()
            .fold(c.off_ms, |a, ms| a.saturating_add(*ms));
        t.counts.add(&c);
        t.off_spans = t.off_spans.saturating_add(b.spans);
    }
    t
}
