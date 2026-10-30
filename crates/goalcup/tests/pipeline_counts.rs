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
    log.append(line.seq, &ev).unwrap();
    w.sync().unwrap();
    c.counts()
}

/// The first part file and its index.
fn first_part(rec: &Path) -> (PathBuf, PathBuf) {
    let part = goalcup_format::read::part_files(rec).unwrap()[0].clone();
    let name = part.file_name().unwrap().to_str().unwrap();
    let idx = part.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")));
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

fn conn(w: &mut PartWriter, c: &mut Counter, k: &mut Clock) {
    let meta = json!({"host": HOST, "port": 443, "mode": "decrypt", "listed": "shared",
                      "kind": "provider", "reason": null});
    let r = open(k.tick(), 1, StreamKind::Conn, 0, meta);
    put(w, c, r, OpenFlags::DECRYPTED);
}

fn boot0(rec: &Path, entry_dir: &Path, k: &mut Clock, opts: PartOptions) -> Counts {
    let rules = ["bearer".to_string(), "fail_closed".to_string()];
    let mut c = Counter::new(&rules);
    let mut log = CountsLog::open(entry_dir).unwrap();
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
    let mut log = CountsLog::open(entry_dir).unwrap();

    // The next boot recovers the dead one, 5 s later.
    k.0 = link_at + 5 * SEC;
    let at = |seq| count_at(entry_dir, seq).expect("a counts.jsonl line at the kept LINK");
    let mut tail = serde_json::Map::new();
    tail.insert("cause".into(), json!({"code": 9}));
    let rc = Recover {
        entry: ENTRY,
        boot_id: BOOT_ID,
        now: k.now(),
        durable: None,
        count_at: &at,
        tail,
        dead_start: None,
        seal: None,
    };
    let mut w = PartWriter::recover(&rec, opts, rc).unwrap().writer.unwrap();
    assert_eq!(w.boot(), 1);
    let frame_count: serde_json::Value = serde_json::from_slice(&at(kept)).unwrap();
    log.recovered(0, Some((kept, &frame_count)), 5000).unwrap();

    let mut c = Counter::new(&[]);
    conn(&mut w, &mut c, &mut k);
    post(&mut w, &mut c, &mut k, 2);
    let b1 = count_and_link(&mut w, &c, &mut k, &mut log, 1);
    log.compact().unwrap();
    // Compacted: one line per earlier boot plus this boot's latest, each still found by seq.
    assert!(count_at(entry_dir, 0).is_some());
    assert!(count_at(entry_dir, 2).is_some());

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
