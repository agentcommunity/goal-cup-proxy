//! Counts at write and entry totals over boots (point 6, PRD-01 section 9, PRD-02 section 6).
//!
//! Boot 0 is written with `PartWriter` and left with an open frame, as a killed recorder leaves
//! it; recovery takes its COUNT from `counts.jsonl` and writes a `recorder_down` gap; boot 1 adds
//! one more request. The entry totals must be boot 0 plus boot 1 plus that one off span, and
//! must stay so once boot 0's part file is pruned after upload (ledger U1).

use goalcup::pipeline::counts::{Counter, CountsLog, count_at};
use goalcup::pipeline::{Counts, OpenFlags, entry_totals};
use goalcup_format::{
    EndReason, GapReason, Now, PartOptions, PartWriter, Record, Recover, Recovered, StreamKind,
};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

const ENTRY: &str = "e0123456789abcdef";
const BOOT_ID: [u8; 16] = [7; 16];
const HOST: &str = "api.openai.com";
const SEC: u64 = 1_000_000_000;
const NONE: OpenFlags = OpenFlags::NONE;

/// A clock for the test: wall and record clock move together.
struct Clock(u64);

impl Clock {
    fn tick(&mut self) -> u64 {
        self.0 += 1_000_000;
        self.0
    }
    fn now(&self) -> Now {
        Now {
            wall_ns: 1_800_000_000 * SEC + self.0,
            mono_ns: self.0,
        }
    }
}

/// An OPEN on connection 1 (a `conn` stream is on none).
fn open(t: u64, sid: u64, skind: StreamKind, meta: Value) -> Record {
    let conn = u64::from(skind != StreamKind::Conn);
    let meta = meta.to_string().into_bytes();
    Record::Open {
        sid,
        t,
        skind,
        conn,
        req: 0,
        meta,
    }
}

fn data(t: u64, sid: u64, off: u64, flags: u8, bytes: &[u8]) -> Record {
    let bytes = bytes.to_vec();
    Record::Data {
        sid,
        t,
        off,
        flags,
        bytes,
    }
}

fn end(t: u64, sid: u64, seen: u64) -> Record {
    let (reason, tail) = (EndReason::Complete, Vec::new());
    Record::End {
        sid,
        t,
        reason,
        seen,
        tail,
    }
}

fn post_open(t: u64, sid: u64) -> Record {
    open(t, sid, StreamKind::HttpReq, json!({"method": "POST"}))
}

fn model() -> OpenFlags {
    OpenFlags::POST | OpenFlags::DECRYPTED
}

fn opts(rotate: bool) -> PartOptions {
    let mut o = PartOptions {
        window_log: 20,
        ..PartOptions::default()
    };
    if rotate {
        o.rotate_after_ns = 60 * SEC;
    }
    o
}

/// One boot: its part writer, its counter and `counts.jsonl`.
struct W {
    w: PartWriter,
    c: Counter,
    log: CountsLog,
}

impl W {
    fn open(rec: &Path, opts: PartOptions, k: &Clock, rules: &[String]) -> W {
        let w = PartWriter::open(rec, opts, ENTRY, BOOT_ID, k.now()).unwrap();
        W::new(w, rec, rules)
    }

    fn new(w: PartWriter, rec: &Path, rules: &[String]) -> W {
        let log = CountsLog::open(rec.parent().unwrap());
        let c = Counter::new(rules);
        W { w, c, log }
    }

    /// Writes a record, then counts it, as the writer does: the host is the one the mask worker
    /// took before masking the OPEN meta.
    fn put(&mut self, r: Record, flags: OpenFlags) {
        self.w.write(&r).unwrap();
        let conn = matches!(r, Record::Open { skind, .. } if skind == StreamKind::Conn);
        self.c.written(&r, flags, conn.then_some(HOST));
    }

    fn conn(&mut self, k: &mut Clock) {
        let meta = json!({"host": HOST, "port": 443, "mode": "decrypt"});
        self.put(
            open(k.tick(), 1, StreamKind::Conn, meta),
            OpenFlags::DECRYPTED,
        );
    }

    /// One decrypted POST with its body and END.
    fn post(&mut self, k: &mut Clock, sid: u64) {
        self.put(post_open(k.tick(), sid), model());
        self.put(data(k.tick(), sid, 0, 0, b"{\"a\":1}"), NONE);
        self.put(end(k.tick(), sid, 7), NONE);
    }

    /// The COUNT event, then a LINK, with the `counts.jsonl` line at that LINK.
    fn count_and_link(&mut self, k: &mut Clock, boot: u64) -> Counts {
        let ev = self.c.count_event(boot, 60_000, 0, 0);
        let json = ev.to_string().into_bytes();
        self.w.write(&Record::Event { t: k.tick(), json }).unwrap();
        k.tick();
        self.line(k, &ev, true);
        self.c.counts()
    }

    /// A LINK 3 s on and its `counts.jsonl` line, with no COUNT event (one comes every 60 s).
    fn link_and_line(&mut self, k: &mut Clock) {
        k.0 += 3 * SEC;
        let ev = self.c.count_event(self.w.boot(), 0, 0, 0);
        self.line(k, &ev, false);
    }

    /// A LINK, its `counts.jsonl` line (with the open-stream table when `table`), then a sync.
    fn line(&mut self, k: &Clock, ev: &Value, table: bool) {
        let seq = self.w.link(k.now()).unwrap().seq;
        let open = table.then(|| self.c.open_table());
        let (spans, rules) = (self.c.off_spans(), self.c.fail_closed());
        self.log.append(seq, ev, spans, open, rules).unwrap();
        self.w.sync().unwrap();
    }

    /// The writer dies: its frame stays open, nothing after the last index line.
    fn die(self) -> CountsLog {
        std::mem::forget(self.w);
        self.log
    }
}

/// A temp entry folder, its `rec` and a clock.
fn entry() -> (tempfile::TempDir, PathBuf, Clock) {
    let dir = tempfile::tempdir().unwrap();
    let rec = dir.path().join("rec");
    (dir, rec, Clock(1000 * SEC))
}

/// A part file's index.
fn idx_of(part: &Path) -> PathBuf {
    let name = part.file_name().unwrap().to_str().unwrap();
    part.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")))
}

fn parts(rec: &Path) -> Vec<PathBuf> {
    goalcup_format::read::part_files(rec).unwrap()
}

/// The first part file and its index are uploaded and pruned.
fn prune_first(rec: &Path) {
    let part = parts(rec).remove(0);
    std::fs::remove_file(idx_of(&part)).unwrap();
    std::fs::remove_file(part).unwrap();
}

/// Recovers the dead boot as the next boot does, its COUNT from `counts.jsonl`; `dead_start`
/// when the entry left no index line.
fn recover_at(rec: &Path, opts: PartOptions, now: Now, dead_start: Option<Now>) -> Recovered {
    let entry_dir = rec.parent().unwrap();
    let at = |seq| count_at(entry_dir, seq).expect("a counts.jsonl line at the kept LINK");
    let rc = Recover {
        entry: ENTRY,
        boot_id: BOOT_ID,
        now,
        durable: None,
        count_at: &at,
        tail: Map::new(),
        dead_start: dead_start.map(|d| (d, BOOT_ID)),
        seal: None,
    };
    PartWriter::recover(rec, opts, rc).unwrap()
}

fn recover(rec: &Path, opts: PartOptions, now: Now) -> W {
    W::new(recover_at(rec, opts, now, None).writer.unwrap(), rec, &[])
}

fn boot0(rec: &Path, k: &mut Clock) -> Counts {
    let rules = ["bearer".to_string(), "fail_closed".to_string()];
    let mut b = W::open(rec, opts(false), k, &rules);
    b.conn(k);
    // A whole POST whose body has a fail-closed mask.
    b.post(k, 2);
    let mask = Record::Mask {
        sid: 2,
        off: 0,
        len: 7,
        rule: 1,
    };
    b.put(mask, NONE);
    // A POST with a hole in its body.
    b.put(post_open(k.tick(), 3), model());
    b.put(data(k.tick(), 3, 0, 0, b"abc"), NONE);
    let start = k.tick();
    let gap = Record::Gap {
        sid: 3,
        reason: GapReason::MemoryGuard,
        start_wall: start,
        end_wall: start + 5,
        start_mono: start,
        end_mono: k.tick(),
        off: 3,
        lost: 11,
        tail: br#"{"guard":"held_bytes"}"#.to_vec(),
    };
    b.put(gap, NONE);
    b.put(data(k.tick(), 3, 13, 0, b"xy"), NONE);
    b.put(end(k.tick(), 3, 15), NONE);
    // One WebSocket text message from the client in two pieces, then a ping: not a request.
    let ws = open(k.tick(), 4, StreamKind::WsUp, json!({}));
    b.put(ws, OpenFlags::DECRYPTED);
    b.put(data(k.tick(), 4, 0, 0x10, b"hel"), NONE);
    b.put(data(k.tick(), 4, 3, 0x11, b"lo"), NONE);
    b.put(data(k.tick(), 4, 5, 0x91, b"p"), NONE);
    // A POST whose END is never written.
    b.put(post_open(k.tick(), 5), model());
    b.put(data(k.tick(), 5, 0, 0, b"{\"b\""), NONE);

    let counts = b.count_and_link(k, 0);
    assert_eq!((counts.http_req, counts.ws_up_msgs), (3, 1));
    assert_eq!(
        counts.http_post, 2,
        "a request counts once its END is written"
    );
    assert_eq!(b.c.count_event(0, 0, 0, 0)["open_streams"], 3);
    b.die();
    counts
}

#[test]
fn entry_totals_are_each_boot_plus_off_spans() {
    let (dir, rec, mut k) = entry();
    let entry_dir = dir.path();
    let b0 = boot0(&rec, &mut k);
    let link_at = k.0;

    // A live entry with an unclosed tail reads up to where the reader stops.
    let live = entry_totals(entry_dir);
    assert_eq!((&live.counts, live.off_spans), (&b0, 0));

    // A torn last line of counts.jsonl is skipped, never an error.
    let log_path = entry_dir.join("counts.jsonl");
    let mut text = std::fs::read(&log_path).unwrap();
    text.extend_from_slice(b"{\"seq\":99,\"cou");
    std::fs::write(&log_path, &text).unwrap();

    // The next boot recovers the dead one, 5 s later.
    k.0 = link_at + 5 * SEC;
    let mut b = recover(&rec, opts(false), k.now());
    // The recovery frame is on disk: the next boot syncs counts.jsonl from it.
    let before = b.log.sync(entry_dir).unwrap();
    assert_eq!((before.off_spans, before.counts.off_ms), (1, 5000));

    b.conn(&mut k);
    b.post(&mut k, 2);
    let b1 = b.count_and_link(&mut k, 1);
    b.log.compact().unwrap();

    let mut want = b0;
    want.add(&b1);
    want.gaps += 1;
    want.lost_unknown += 1;
    want.off_ms += 5000;
    let got = entry_totals(entry_dir);
    assert_eq!((&got.counts, got.off_spans), (&want, 1));
    let c = &got.counts;
    assert_eq!(
        (c.off_ms, c.http_req, c.http_post, c.model_requests()),
        (5000, 4, 3, 4)
    );
    assert_eq!(
        (c.req_whole, c.req_holes, c.hosts.get(HOST)),
        (3, 1, Some(&4))
    );
    assert_eq!((c.mask_failclosed_bytes, c.gaps, c.lost_bytes), (7, 2, 10));

    // Boot 0's part file is pruned after upload: the totals and the off span stand.
    prune_first(&rec);
    assert_eq!(entry_totals(entry_dir), got);
}

/// Boot 0 of an entry that left no index line starts with its own `recorder_down` gap (PRD-01
/// section 12). Its off span stands once that first part file is pruned, while boot 0 is live
/// and after it dies and is recovered by a boot whose sync runs only after a crash.
#[test]
fn boot_zero_start_gap_keeps_its_off_span_once_pruned() {
    let (dir, rec, mut k) = entry();
    let entry_dir = dir.path();
    std::fs::create_dir_all(&rec).unwrap();
    let dead = Now {
        wall_ns: k.now().wall_ns - 5 * SEC,
        mono_ns: k.0 - 5 * SEC,
    };
    let r = recover_at(&rec, opts(true), k.now(), Some(dead));
    let mut b = W::new(r.writer.unwrap(), &rec, &[]);
    b.put(r.gap_after_start.unwrap(), NONE);
    b.post(&mut k, 1);
    b.count_and_link(&mut k, 0);
    k.0 += 61 * SEC;
    assert!(b.w.rotate_if_due(k.now()).unwrap().is_some());
    b.post(&mut k, 2);
    b.count_and_link(&mut k, 0);
    let live = entry_totals(entry_dir);
    assert_eq!((live.off_spans, live.counts.gaps), (1, 1));

    // Part 0 is uploaded and pruned while boot 0 is live.
    prune_first(&rec);
    assert_eq!(entry_totals(entry_dir), live);

    // Boot 0 dies. Boot 1 recovers it 5 s later and dies before its first LINK and its sync;
    // the boot after it recovers again 2 s later. Its sync reads what is on disk, so boot 0's
    // line holds its own span and both recovery gaps.
    b.die();
    k.0 += 5 * SEC;
    recover(&rec, opts(true), k.now()).die();
    k.0 += 2 * SEC;
    let mut b = recover(&rec, opts(true), k.now());
    let synced = b.log.sync(entry_dir).unwrap();
    let s = (synced.off_spans, synced.counts.gaps, synced.counts.off_ms);
    assert_eq!(s, (3, 3, 12_000));

    // Boot 1 writes; boot 0's last part file is pruned: its line keeps its totals.
    b.post(&mut k, 1);
    let b1 = b.count_and_link(&mut k, 1);
    prune_first(&rec);
    let mut want = synced.counts;
    want.add(&b1);
    let pruned = entry_totals(entry_dir);
    assert_eq!((pruned.counts, pruned.off_spans), (want, 3));
}

/// Point 6: a boot that dies 10 s in has a `counts.jsonl` line at each LINK but no COUNT event
/// yet (one is written every 60 s). Before any recovery (the null recorder, `goalcup upload`)
/// its totals are its line at its last whole index line, never 0, and recovery keeps them. It
/// died after its last LINK and line were written but before that LINK's index line: recovery
/// cuts that LINK, so the totals never count it (S4: never ahead of the record).
#[test]
fn a_dead_boot_counts_its_last_indexed_line_before_recovery() {
    let (dir, rec, mut k) = entry();
    let mut b = W::open(&rec, opts(false), &k, &[]);
    b.conn(&mut k);
    for sid in 2..4 {
        b.post(&mut k, sid);
        b.link_and_line(&mut k);
    }
    let (kept, kept_at) = (b.c.counts(), k.0);
    b.post(&mut k, 4);
    b.link_and_line(&mut k);
    b.die();
    // The last index line never reached the file.
    let idx = idx_of(&parts(&rec)[0]);
    let text = std::fs::read(&idx).unwrap();
    let cut = text[..text.len() - 1].iter().rposition(|&b| b == b'\n');
    std::fs::write(&idx, &text[..cut.unwrap() + 1]).unwrap();

    assert_eq!(kept.model_requests(), 2);
    assert_eq!(entry_totals(dir.path()).counts, kept);

    // Recovery writes that line as the boot's COUNT: the totals stand, plus the off span from
    // the kept LINK.
    k.0 += 5 * SEC;
    let mut log = recover(&rec, opts(false), k.now()).log;
    let got = log.sync(dir.path()).unwrap();
    let mut want = kept;
    want.gaps += 1;
    want.lost_unknown += 1;
    want.off_ms += (k.0 - kept_at) / 1_000_000;
    assert_eq!((got.counts, got.off_spans), (want, 1));
}

/// A boot read from `counts.jsonl` alone (no part file can be read) is its latest line, not
/// its first.
#[test]
fn a_boot_not_read_from_part_files_is_its_latest_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = CountsLog::open(dir.path());
    let one = json!({"ev": "count", "boot": 0, "http_post": 1});
    log.append(5, &one, 0, None, &[]).unwrap();
    let five = json!({"ev": "count", "boot": 0, "http_post": 5});
    log.append(6, &five, 0, None, &[]).unwrap();
    assert_eq!(entry_totals(dir.path()).counts.http_post, 5);
}

/// Boot 0 with a COUNT and LINK in its first part file, then, after a rotation, one request per
/// sid in `posts` with a line at each LINK. Returns the boot alive and its counts.
fn two_parts(rec: &Path, k: &mut Clock, posts: std::ops::Range<u64>) -> (W, Counts) {
    let mut b = W::open(rec, opts(true), k, &[]);
    b.conn(k);
    b.post(k, 2);
    b.count_and_link(k, 0);
    k.0 += 61 * SEC;
    assert!(b.w.rotate_if_due(k.now()).unwrap().is_some());
    for sid in posts {
        b.post(k, sid);
        b.link_and_line(k);
    }
    let latest = b.c.counts();
    (b, latest)
}

/// A read that fails partway through a boot (here the index of its second part file) never
/// takes that boot below its latest line, and the sync keeps that line.
#[test]
fn a_failed_read_keeps_the_latest_line_of_its_boot() {
    let (dir, rec, mut k) = entry();
    let entry_dir = dir.path();
    let (b, latest) = two_parts(&rec, &mut k, 3..5);
    let mut log = b.die();
    assert_eq!(latest.model_requests(), 3);
    assert_eq!(entry_totals(entry_dir).counts, latest);

    std::fs::write(idx_of(&parts(&rec)[1]), b"not an index line\n").unwrap();
    assert_eq!(entry_totals(entry_dir).counts, latest);
    assert_eq!(log.sync(entry_dir).unwrap().counts, latest);
    assert_eq!(entry_totals(entry_dir).counts, latest);
}

/// A read that fails inside a boot already recovered and synced, before its recovery frame,
/// keeps that boot's recovery gap from its line, and the sync keeps it there for when the boot's
/// part files are pruned after upload (point 6).
#[test]
fn a_failed_read_keeps_a_recovered_boot_gap() {
    let (dir, rec, mut k) = entry();
    let entry_dir = dir.path();
    two_parts(&rec, &mut k, 3..4).0.die();
    let boot0_parts = parts(&rec);
    assert_eq!(boot0_parts.len(), 2);

    k.0 += 5 * SEC;
    let mut b = recover(&rec, opts(true), k.now());
    let synced = b.log.sync(entry_dir).unwrap();
    assert_eq!((synced.counts.gaps, synced.off_spans), (1, 1));
    b.post(&mut k, 1);
    b.count_and_link(&mut k, 1);
    let whole = entry_totals(entry_dir);
    assert_eq!((whole.counts.gaps, whole.off_spans), (1, 1));
    assert_eq!(whole.counts.off_ms, synced.counts.off_ms);

    // The index of boot 0's second part file cannot be read.
    let idx = idx_of(&boot0_parts[1]);
    let good = std::fs::read(&idx).unwrap();
    std::fs::write(&idx, b"not an index line\n").unwrap();
    assert_eq!(entry_totals(entry_dir), whole);
    assert_eq!(b.log.sync(entry_dir).unwrap(), whole);

    // Readable again, then uploaded and pruned: the line holds what the part files held.
    std::fs::write(&idx, good).unwrap();
    for part in &boot0_parts {
        std::fs::remove_file(idx_of(part)).unwrap();
        std::fs::remove_file(part).unwrap();
    }
    assert_eq!(entry_totals(entry_dir), whole);
}
