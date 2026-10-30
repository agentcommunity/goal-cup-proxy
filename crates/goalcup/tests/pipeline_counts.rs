//! Counts at write and entry totals over boots (point 6, PRD-01 section 9, PRD-02 section 6).
//!
//! Boot 0 is written with `PartWriter` and left with an open frame, as a killed recorder leaves
//! it; recovery takes its COUNT from `counts.jsonl` and writes a `recorder_down` gap; boot 1 adds
//! one more request. The entry totals must be boot 0 plus boot 1 plus that one off span, and
//! must stay so once boot 0's part file is pruned after upload (ledger U1).

use goalcup::pipeline::counts::{Counter, CountsLog, count_at};
use goalcup::pipeline::{Counts, OpenFlags, entry_totals};
use goalcup_format::{
    EndReason, GapReason, Now, PartOptions, PartWriter, Record, Recover, StreamKind,
};
use serde_json::json;
use std::path::{Path, PathBuf};

const ENTRY: &str = "e0123456789abcdef";
const BOOT_ID: [u8; 16] = [7; 16];
const HOST: &str = "api.openai.com";
const SEC: u64 = 1_000_000_000;

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

/// Writes a record, then counts it, as the writer does: the host is the one the mask worker
/// took before masking the OPEN meta.
fn put(w: &mut PartWriter, c: &mut Counter, r: Record, flags: OpenFlags) {
    w.write(&r).unwrap();
    let host = matches!(
        r,
        Record::Open {
            skind: StreamKind::Conn,
            ..
        }
    )
    .then_some(HOST);
    c.written(&r, flags, host);
}

fn open(t: u64, sid: u64, skind: StreamKind, conn: u64, meta: serde_json::Value) -> Record {
    Record::Open {
        sid,
        t,
        skind,
        conn,
        req: 0,
        meta: meta.to_string().into_bytes(),
    }
}

fn data(t: u64, sid: u64, off: u64, flags: u8, bytes: &[u8]) -> Record {
    Record::Data {
        sid,
        t,
        off,
        flags,
        bytes: bytes.to_vec(),
    }
}

fn end(t: u64, sid: u64, seen: u64) -> Record {
    Record::End {
        sid,
        t,
        reason: EndReason::Complete,
        seen,
        tail: Vec::new(),
    }
}

/// One decrypted POST with its body and END.
fn post(w: &mut PartWriter, c: &mut Counter, k: &mut Clock, sid: u64) {
    let model = OpenFlags::POST | OpenFlags::DECRYPTED;
    let head = json!({"method": "POST", "path": "/v1/responses"});
    put(
        w,
        c,
        open(k.tick(), sid, StreamKind::HttpReq, 1, head),
        model,
    );
    put(
        w,
        c,
        data(k.tick(), sid, 0, 0, b"{\"a\":1}"),
        OpenFlags::NONE,
    );
    put(w, c, end(k.tick(), sid, 7), OpenFlags::NONE);
}

/// The COUNT event, then a LINK, with the `counts.jsonl` line at that LINK.
fn count_and_link(
    w: &mut PartWriter,
    c: &Counter,
    k: &mut Clock,
    log: &mut CountsLog,
    boot: u64,
) -> Counts {
    let ev = c.count_event(boot, 60_000, 0, 0);
    w.write(&Record::Event {
        t: k.tick(),
        json: ev.to_string().into_bytes(),
    })
    .unwrap();
    k.tick();
    let line = w.link(k.now()).unwrap();
    log.append(line.seq, &ev, c.off_spans()).unwrap();
    w.sync().unwrap();
    c.counts()
}

/// A part file's index.
fn idx_of(part: &Path) -> PathBuf {
    let name = part.file_name().unwrap().to_str().unwrap();
    part.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")))
}

/// The first part file and its index.
fn first_part(rec: &Path) -> (PathBuf, PathBuf) {
    let part = goalcup_format::read::part_files(rec).unwrap()[0].clone();
    let idx = idx_of(&part);
    (part, idx)
}

/// The `seq` of the last index line in the first part file.
fn last_seq(rec: &Path) -> u64 {
    let idx = first_part(rec).1;
    goalcup_format::read::read_index(&idx)
        .unwrap()
        .last()
        .unwrap()
        .seq
}

/// Recovers the dead boot as the next boot does, its COUNT from `counts.jsonl`.
fn recover(
    rec: &Path,
    entry_dir: &Path,
    opts: PartOptions,
    now: Now,
    tail: serde_json::Map<String, serde_json::Value>,
) -> PartWriter {
    let at = |seq| count_at(entry_dir, seq).expect("a counts.jsonl line at the kept LINK");
    let rc = Recover {
        entry: ENTRY,
        boot_id: BOOT_ID,
        now,
        durable: None,
        count_at: &at,
        tail,
        dead_start: None,
        seal: None,
    };
    PartWriter::recover(rec, opts, rc).unwrap().writer.unwrap()
}

/// A LINK and its `counts.jsonl` line, then a sync, with no COUNT event (one comes every 60 s).
fn link_and_line(w: &mut PartWriter, c: &Counter, k: &mut Clock, log: &mut CountsLog) {
    k.0 += 3 * SEC;
    let line = w.link(k.now()).unwrap();
    log.append(line.seq, &c.count_event(w.boot(), 0, 0, 0), c.off_spans())
        .unwrap();
    w.sync().unwrap();
}

fn conn(w: &mut PartWriter, c: &mut Counter, k: &mut Clock) {
    let meta = json!({"host": HOST, "port": 443, "mode": "decrypt", "listed": "shared",
                      "kind": "provider", "reason": null});
    let r = open(k.tick(), 1, StreamKind::Conn, 0, meta);
    put(w, c, r, OpenFlags::DECRYPTED);
}

fn boot0(rec: &Path, entry_dir: &Path, k: &mut Clock, opts: PartOptions) -> Counts {
    let rules = ["bearer".to_string(), "fail_closed".to_string()];
    let mut c = Counter::new(&rules);
    let mut log = CountsLog::open(entry_dir);
    let mut w = PartWriter::open(rec, opts, ENTRY, BOOT_ID, k.now()).unwrap();

    conn(&mut w, &mut c, k);
    // A whole POST whose body has a fail-closed mask.
    post(&mut w, &mut c, k, 2);
    let mask = Record::Mask {
        sid: 2,
        off: 0,
        len: 7,
        rule: 1,
    };
    put(&mut w, &mut c, mask, OpenFlags::NONE);
    // A POST with a hole in its body.
    let model = OpenFlags::POST | OpenFlags::DECRYPTED;
    let head = json!({"method": "POST", "path": "/v1/responses"});
    put(
        &mut w,
        &mut c,
        open(k.tick(), 3, StreamKind::HttpReq, 1, head.clone()),
        model,
    );
    put(
        &mut w,
        &mut c,
        data(k.tick(), 3, 0, 0, b"abc"),
        OpenFlags::NONE,
    );
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
        tail: br#"{"guard":"held_bytes","limit":536870912}"#.to_vec(),
    };
    put(&mut w, &mut c, gap, OpenFlags::NONE);
    put(
        &mut w,
        &mut c,
        data(k.tick(), 3, 13, 0, b"xy"),
        OpenFlags::NONE,
    );
    put(&mut w, &mut c, end(k.tick(), 3, 15), OpenFlags::NONE);
    // One WebSocket text message from the client, in two pieces.
    put(
        &mut w,
        &mut c,
        open(k.tick(), 4, StreamKind::WsUp, 1, json!({"deflate": false})),
        OpenFlags::DECRYPTED,
    );
    put(
        &mut w,
        &mut c,
        data(k.tick(), 4, 0, 0x10, b"hel"),
        OpenFlags::NONE,
    );
    put(
        &mut w,
        &mut c,
        data(k.tick(), 4, 3, 0x11, b"lo"),
        OpenFlags::NONE,
    );
    // A ping from the client is not a request.
    put(
        &mut w,
        &mut c,
        data(k.tick(), 4, 5, 0x91, b"p"),
        OpenFlags::NONE,
    );
    // A POST whose END is never written.
    put(
        &mut w,
        &mut c,
        open(k.tick(), 5, StreamKind::HttpReq, 1, head),
        model,
    );
    put(
        &mut w,
        &mut c,
        data(k.tick(), 5, 0, 0, b"{\"b\""),
        OpenFlags::NONE,
    );

    let counts = count_and_link(&mut w, &c, k, &mut log, 0);
    assert_eq!(counts.http_req, 3);
    assert_eq!(
        counts.http_post, 2,
        "a request counts once its END is written"
    );
    assert_eq!(counts.ws_up_msgs, 1);
    assert_eq!(c.open_streams(), 3);
    // The writer dies here: the frame stays open, nothing after the last index line.
    std::mem::forget(w);
    counts
}

#[test]
fn entry_totals_are_each_boot_plus_off_spans() {
    let dir = tempfile::tempdir().unwrap();
    let entry_dir = dir.path();
    let rec = entry_dir.join("rec");
    let opts = PartOptions {
        window_log: 20,
        ..PartOptions::default()
    };
    let mut k = Clock(1000 * SEC);

    let b0 = boot0(&rec, entry_dir, &mut k, opts);
    let link_at = k.0;
    let kept = last_seq(&rec);

    // A live entry with an unclosed tail reads up to where the reader stops.
    let live = entry_totals(entry_dir);
    assert_eq!(live.counts, b0);
    assert_eq!(live.off_spans, 0);

    // A torn last line of counts.jsonl is skipped, never an error.
    let log_path = entry_dir.join("counts.jsonl");
    let mut text = std::fs::read(&log_path).unwrap();
    text.extend_from_slice(b"{\"seq\":99,\"cou");
    std::fs::write(&log_path, &text).unwrap();
    let mut log = CountsLog::open(entry_dir);

    // The next boot recovers the dead one, 5 s later.
    k.0 = link_at + 5 * SEC;
    let mut tail = serde_json::Map::new();
    tail.insert("cause".into(), json!({"code": 9}));
    let mut w = recover(&rec, entry_dir, opts, k.now(), tail);
    assert_eq!(w.boot(), 1);
    // The recovery frame is on disk: the next boot syncs counts.jsonl from it.
    let before = log.sync(entry_dir).unwrap();
    assert_eq!(before.off_spans, 1);
    assert_eq!(before.counts.off_ms, 5000);

    let mut c = Counter::new(&[]);
    conn(&mut w, &mut c, &mut k);
    post(&mut w, &mut c, &mut k, 2);
    let b1 = count_and_link(&mut w, &c, &mut k, &mut log, 1);
    log.compact().unwrap();
    // Compacted: one line per earlier boot plus this boot's latest.
    assert!(count_at(entry_dir, kept + 1).is_some());
    assert!(count_at(entry_dir, kept + 2).is_some());

    let mut want = b0.clone();
    want.add(&b1);
    want.gaps += 1;
    want.lost_unknown += 1;
    want.off_ms += 5000;
    let got = entry_totals(entry_dir);
    assert_eq!(got.counts, want);
    assert_eq!(got.off_spans, 1);
    assert_eq!(got.counts.off_ms, 5000);
    assert_eq!(got.counts.http_req, 4);
    assert_eq!(got.counts.http_post, 3);
    assert_eq!(got.counts.model_requests(), 4);
    assert_eq!((got.counts.req_whole, got.counts.req_holes), (3, 1));
    assert_eq!(got.counts.hosts.get(HOST), Some(&4));
    assert_eq!(got.counts.mask_failclosed_bytes, 7);
    assert_eq!(got.counts.gaps, 2);
    assert_eq!(got.counts.lost_bytes, 10);

    // Boot 0's part file is pruned after upload: the totals and the off span stand.
    let (part, idx) = first_part(&rec);
    std::fs::remove_file(idx).unwrap();
    std::fs::remove_file(part).unwrap();
    let pruned = entry_totals(entry_dir);
    assert_eq!(pruned.counts, want);
    assert_eq!(pruned.off_spans, 1);
}

/// Boot 0 of an entry that left no index line starts with its own `recorder_down` gap (PRD-01
/// section 12). Its off span stands once that first part file is pruned, while boot 0 is live
/// and after it dies and is recovered by a boot whose sync runs only after a crash.
#[test]
fn boot_zero_start_gap_keeps_its_off_span_once_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let entry_dir = dir.path();
    let rec = entry_dir.join("rec");
    std::fs::create_dir_all(&rec).unwrap();
    let opts = PartOptions {
        window_log: 20,
        rotate_after_ns: 60 * SEC,
        ..PartOptions::default()
    };
    let mut k = Clock(1000 * SEC);
    let none = |_: u64| Vec::new();
    let dead = Now {
        wall_ns: k.now().wall_ns - 5 * SEC,
        mono_ns: k.0 - 5 * SEC,
    };
    let rc = Recover {
        entry: ENTRY,
        boot_id: BOOT_ID,
        now: k.now(),
        durable: None,
        count_at: &none,
        tail: serde_json::Map::new(),
        dead_start: Some((dead, BOOT_ID)),
        seal: None,
    };
    let r = PartWriter::recover(&rec, opts, rc).unwrap();
    let mut w = r.writer.unwrap();
    let mut c = Counter::new(&[]);
    let mut log = CountsLog::open(entry_dir);
    put(&mut w, &mut c, r.gap_after_start.unwrap(), OpenFlags::NONE);
    post(&mut w, &mut c, &mut k, 1);
    count_and_link(&mut w, &c, &mut k, &mut log, 0);
    k.0 += 61 * SEC;
    assert!(w.rotate_if_due(k.now()).unwrap().is_some());
    post(&mut w, &mut c, &mut k, 2);
    count_and_link(&mut w, &c, &mut k, &mut log, 0);
    let live = entry_totals(entry_dir);
    assert_eq!((live.off_spans, live.counts.gaps), (1, 1));

    // Part 0 is uploaded and pruned while boot 0 is live.
    let (part, idx) = first_part(&rec);
    std::fs::remove_file(idx).unwrap();
    std::fs::remove_file(part).unwrap();
    assert_eq!(entry_totals(entry_dir), live);

    // Boot 0 dies. Boot 1 recovers it 5 s later and dies before its first LINK and its sync;
    // the boot after it recovers again 2 s later. Its sync reads what is on disk, so boot 0's
    // line holds its own span and both recovery gaps.
    std::mem::forget(w);
    let next = |now: Now| recover(&rec, entry_dir, opts, now, serde_json::Map::new());
    k.0 += 5 * SEC;
    std::mem::forget(next(k.now()));
    k.0 += 2 * SEC;
    let mut w = next(k.now());
    assert_eq!(w.boot(), 1);
    let mut log = CountsLog::open(entry_dir);
    let synced = log.sync(entry_dir).unwrap();
    assert_eq!(synced.off_spans, 3);
    assert_eq!(synced.counts.gaps, 3);
    assert_eq!(synced.counts.off_ms, 12_000);

    // Boot 1 writes; boot 0's last part file is pruned: its line keeps its totals.
    let mut c = Counter::new(&[]);
    post(&mut w, &mut c, &mut k, 1);
    let b1 = count_and_link(&mut w, &c, &mut k, &mut log, 1);
    let (part, idx) = first_part(&rec);
    std::fs::remove_file(idx).unwrap();
    std::fs::remove_file(part).unwrap();
    let mut want = synced.counts.clone();
    want.add(&b1);
    let pruned = entry_totals(entry_dir);
    assert_eq!(pruned.counts, want);
    assert_eq!(pruned.off_spans, 3);
}

/// Point 6: a boot that dies 10 s in has a `counts.jsonl` line at each LINK but no COUNT event
/// yet (one is written every 60 s). Before any recovery (the null recorder, `goalcup upload`)
/// its totals are its line at its last whole index line, never 0, and recovery keeps them. It
/// died after its last LINK and line were written but before that LINK's index line: recovery
/// cuts that LINK, so the totals never count it (S4: never ahead of the record).
#[test]
fn a_dead_boot_counts_its_last_indexed_line_before_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let entry_dir = dir.path();
    let rec = entry_dir.join("rec");
    let opts = PartOptions {
        window_log: 20,
        ..PartOptions::default()
    };
    let mut k = Clock(1000 * SEC);
    let mut c = Counter::new(&[]);
    let mut log = CountsLog::open(entry_dir);
    let mut w = PartWriter::open(&rec, opts, ENTRY, BOOT_ID, k.now()).unwrap();
    conn(&mut w, &mut c, &mut k);
    for sid in 2..4 {
        post(&mut w, &mut c, &mut k, sid);
        link_and_line(&mut w, &c, &mut k, &mut log);
    }
    let (kept, kept_at) = (c.counts(), k.0);
    post(&mut w, &mut c, &mut k, 4);
    link_and_line(&mut w, &c, &mut k, &mut log);
    std::mem::forget(w);
    // The last index line never reached the file.
    let idx = first_part(&rec).1;
    let text = std::fs::read(&idx).unwrap();
    let cut = text[..text.len() - 1]
        .iter()
        .rposition(|&b| b == b'\n')
        .unwrap()
        + 1;
    std::fs::write(&idx, &text[..cut]).unwrap();

    assert_eq!(kept.model_requests(), 2);
    assert_eq!(entry_totals(entry_dir).counts, kept);

    // Recovery writes that line as the boot's COUNT: the totals stand, plus the off span from
    // the kept LINK.
    k.0 += 5 * SEC;
    recover(&rec, entry_dir, opts, k.now(), serde_json::Map::new());
    let got = log.sync(entry_dir).unwrap();
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
    log.append(5, &json!({"ev": "count", "boot": 0, "http_post": 1}), 0)
        .unwrap();
    log.append(6, &json!({"ev": "count", "boot": 0, "http_post": 5}), 0)
        .unwrap();
    assert_eq!(entry_totals(dir.path()).counts.http_post, 5);
}

/// A read that fails partway through a boot (here the index of its second part file) never
/// takes that boot below its latest line, and the sync keeps that line.
#[test]
fn a_failed_read_keeps_the_latest_line_of_its_boot() {
    let dir = tempfile::tempdir().unwrap();
    let entry_dir = dir.path();
    let rec = entry_dir.join("rec");
    let opts = PartOptions {
        window_log: 20,
        rotate_after_ns: 60 * SEC,
        ..PartOptions::default()
    };
    let mut k = Clock(1000 * SEC);
    let mut c = Counter::new(&[]);
    let mut log = CountsLog::open(entry_dir);
    let mut w = PartWriter::open(&rec, opts, ENTRY, BOOT_ID, k.now()).unwrap();
    conn(&mut w, &mut c, &mut k);
    post(&mut w, &mut c, &mut k, 2);
    count_and_link(&mut w, &c, &mut k, &mut log, 0);
    k.0 += 61 * SEC;
    assert!(w.rotate_if_due(k.now()).unwrap().is_some());
    for sid in 3..5 {
        post(&mut w, &mut c, &mut k, sid);
        link_and_line(&mut w, &c, &mut k, &mut log);
    }
    std::mem::forget(w);
    let latest = c.counts();
    assert_eq!(latest.model_requests(), 3);
    assert_eq!(entry_totals(entry_dir).counts, latest);

    let idx = idx_of(&goalcup_format::read::part_files(&rec).unwrap()[1]);
    std::fs::write(&idx, b"not an index line\n").unwrap();
    assert_eq!(entry_totals(entry_dir).counts, latest);
    assert_eq!(log.sync(entry_dir).unwrap().counts, latest);
    assert_eq!(entry_totals(entry_dir).counts, latest);
}

/// A read that fails inside a boot already recovered and synced, before its recovery frame,
/// keeps that boot's recovery gap from its line, and the sync keeps it there for when the boot's
/// part files are pruned after upload (point 6).
#[test]
fn a_failed_read_keeps_a_recovered_boot_gap() {
    let dir = tempfile::tempdir().unwrap();
    let entry_dir = dir.path();
    let rec = entry_dir.join("rec");
    let opts = PartOptions {
        window_log: 20,
        rotate_after_ns: 60 * SEC,
        ..PartOptions::default()
    };
    let mut k = Clock(1000 * SEC);
    let mut c = Counter::new(&[]);
    let mut log = CountsLog::open(entry_dir);
    let mut w = PartWriter::open(&rec, opts, ENTRY, BOOT_ID, k.now()).unwrap();
    conn(&mut w, &mut c, &mut k);
    post(&mut w, &mut c, &mut k, 2);
    count_and_link(&mut w, &c, &mut k, &mut log, 0);
    k.0 += 61 * SEC;
    assert!(w.rotate_if_due(k.now()).unwrap().is_some());
    post(&mut w, &mut c, &mut k, 3);
    link_and_line(&mut w, &c, &mut k, &mut log);
    std::mem::forget(w);
    let boot0_parts = goalcup_format::read::part_files(&rec).unwrap();
    assert_eq!(boot0_parts.len(), 2);

    k.0 += 5 * SEC;
    let mut w = recover(&rec, entry_dir, opts, k.now(), serde_json::Map::new());
    let mut log = CountsLog::open(entry_dir);
    let synced = log.sync(entry_dir).unwrap();
    assert_eq!((synced.counts.gaps, synced.off_spans), (1, 1));
    let mut c = Counter::new(&[]);
    post(&mut w, &mut c, &mut k, 1);
    count_and_link(&mut w, &c, &mut k, &mut log, 1);
    let whole = entry_totals(entry_dir);
    assert_eq!((whole.counts.gaps, whole.off_spans), (1, 1));
    assert_eq!(whole.counts.off_ms, synced.counts.off_ms);

    // The index of boot 0's second part file cannot be read.
    let idx = idx_of(&boot0_parts[1]);
    let good = std::fs::read(&idx).unwrap();
    std::fs::write(&idx, b"not an index line\n").unwrap();
    assert_eq!(entry_totals(entry_dir), whole);
    assert_eq!(log.sync(entry_dir).unwrap(), whole);

    // Readable again, then uploaded and pruned: the line holds what the part files held.
    std::fs::write(&idx, good).unwrap();
    for part in &boot0_parts {
        std::fs::remove_file(idx_of(part)).unwrap();
        std::fs::remove_file(part).unwrap();
    }
    assert_eq!(entry_totals(entry_dir), whole);
}
