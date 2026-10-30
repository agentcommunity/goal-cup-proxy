//! PRD-06 "Automated tests", `upload_chunks.rs`: chunks cut from real `PartWriter` output start at
//! each of section 3's four cases (and at a fallback part that ends at an `L` line), end on index
//! lines, stay within 32 MiB, never cross a part, never pass the durable point, never have raw
//! length 0; the `R` recovery chunk is sent whole; the first `prev` is 64 zeros; every body put
//! together equals the part files.

mod common;

use common::records::{BOOT_ID, ENTRY, at};
use goalcup::upload::chunk::{self, Bound, Chunk, Cut, MAX_CHUNK};
use goalcup::upload::client::RunPosition;
use goalcup::upload::state::Confirmed;
use goalcup_format::read::read_index;
use goalcup_format::{
    Durable, IndexKind, IndexLine, PartOptions, PartWriter, Record, Recover, hex, index_name,
    part_name,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;

const ZERO: [u8; 32] = [0; 32];

fn opts(rotate_at: u64) -> PartOptions {
    PartOptions {
        window_log: 20,
        rotate_at,
        rotate_after_ns: u64::MAX,
    }
}

/// Bytes that do not compress, so compressed lengths grow with the payload.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// `links` flushes, each after one EVENT of about `size` bytes, rotating when due. Returns the
/// durable point after a sync.
fn work(w: &mut PartWriter, n: &mut u64, links: usize, size: usize) -> Durable {
    for _ in 0..links {
        *n += 1;
        let json = format!(r#"{{"ev":"x","n":"{}"}}"#, hex(&noise(*n, size / 2)));
        let t = at(*n).mono_ns;
        w.write(&Record::Event { t, json: json.into_bytes() }).unwrap();
        w.link(at(*n)).unwrap();
        w.rotate_if_due(at(*n)).unwrap();
    }
    w.sync().unwrap().unwrap()
}

/// Every index line of the folder, with its part's first raw offset, in order.
fn lines(dir: &Path) -> Vec<(u64, IndexLine)> {
    let mut parts: Vec<u64> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".idx")?.strip_prefix("rec-")?.parse().ok()
        })
        .collect();
    parts.sort_unstable();
    parts
        .into_iter()
        .flat_map(|p| read_index(&dir.join(index_name(p))).unwrap().into_iter().map(move |l| (p, l)))
        .collect()
}

fn durable(part: u64, line: IndexLine) -> Bound {
    Bound::Durable(Durable { part: part_name(part), index_len: 0, line })
}

fn pos(offset: u64, next_seq: u64, head: [u8; 32]) -> RunPosition {
    RunPosition {
        offset,
        next_seq,
        head: hex(&head),
        sealed: false,
        stop: false,
        committed: None,
    }
}

/// The server's position after it stored `c`.
fn after(c: &Chunk) -> RunPosition {
    pos(c.raw_end, c.last_seq + 1, c.head)
}

fn cut(dir: &Path, p: &RunPosition, b: &Bound) -> Cut {
    chunk::cut(dir, p, b, None).unwrap()
}

fn one(dir: &Path, p: &RunPosition, b: &Bound) -> Chunk {
    match cut(dir, p, b) {
        Cut::Chunk(c) => c,
        other => panic!("expected a chunk at {}, got {other:?}", p.offset),
    }
}

fn is_stuck(c: Cut) -> bool {
    matches!(c, Cut::Stuck { .. })
}

fn body(c: &Chunk) -> Vec<u8> {
    let mut b = Vec::new();
    chunk::body(c).unwrap().read_to_end(&mut b).unwrap();
    b
}

fn cat(dir: &Path, parts: &[u64]) -> Vec<u8> {
    parts.iter().flat_map(|&p| fs::read(dir.join(part_name(p))).unwrap()).collect()
}

/// A sealed entry over several parts, walked from S = 0 as a live uploader sees it: the durable
/// point moves one index line at a time and each chunk's end is fed back as the next position.
#[test]
fn walk_from_zero_sends_every_part_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = 0;
    let mut w = PartWriter::open(dir.path(), opts(48 << 10), ENTRY, BOOT_ID, at(0)).unwrap();
    work(&mut w, &mut n, 30, 6 << 10);
    w.seal(at(n + 1)).unwrap();
    let all = lines(dir.path());
    let mut parts: Vec<u64> = all.iter().map(|(p, _)| *p).collect();
    parts.dedup();
    assert!(parts.len() >= 3, "the test needs several parts");

    let (mut p, mut sent, mut chunks) = (pos(0, 0, [0; 32]), Vec::new(), Vec::new());
    for (part, line) in &all {
        let bound = durable(*part, *line);
        while let Cut::Chunk(c) = cut(dir.path(), &p, &bound) {
            assert!(c.raw_end > c.raw_start && c.raw_end <= line.raw_end);
            assert!(c.c_end > c.c_start && c.c_end - c.c_start <= MAX_CHUNK);
            // It ends on an index line of its own part, and starts at that part's byte 0 or at an
            // index line of it.
            let own: Vec<&IndexLine> = all.iter().filter(|(q, _)| c.part.ends_with(part_name(*q))).map(|(_, l)| l).collect();
            assert!(own.iter().any(|l| l.c_end == c.c_end && l.raw_end == c.raw_end && l.seq == c.last_seq));
            assert!(c.c_start == 0 || own.iter().any(|l| l.c_end == c.c_start && l.raw_end == c.raw_start));
            let b = body(&c);
            assert_eq!(chunk::sha256(&c).unwrap(), hex(&Sha256::digest(&b)));
            let h = chunk::headers(ENTRY, &c, chunk::sha256(&c).unwrap());
            assert_eq!((h.offset, h.raw_len, h.first_seq), (p.offset, c.raw_end - p.offset, p.next_seq));
            assert_eq!(h.prev, p.head);
            sent.extend_from_slice(&b);
            p = after(&c);
            chunks.push(c);
        }
    }
    assert_eq!(chunks[0].prev, ZERO);
    assert_eq!(chunk::headers(ENTRY, &chunks[0], String::new()).prev, "0".repeat(64));
    assert!(chunks.iter().any(|c| c.c_start > 0), "case 2 was walked");
    assert_eq!(chunks.iter().filter(|c| c.c_start == 0).count(), parts.len(), "case 1 per part");
    assert_eq!(sent, cat(dir.path(), &parts));
    let last = all.last().unwrap().1;
    assert_eq!((p.offset, p.next_seq), (last.raw_end, last.seq + 1));
    // Case 3: S at the seal's E line and no next part: nothing to send, not stuck.
    assert_eq!(cut(dir.path(), &p, &Bound::OnDisk), Cut::Nothing);
}

#[test]
fn the_four_start_cases() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = 0;
    let mut w = PartWriter::open(dir.path(), opts(48 << 10), ENTRY, BOOT_ID, at(0)).unwrap();
    let d = work(&mut w, &mut n, 20, 6 << 10);
    let all = lines(dir.path());
    let bound = Bound::Durable(d);
    let (p0, l) = all.iter().find(|(_, l)| l.kind == IndexKind::L).copied().unwrap();
    let (_, e) = all.iter().find(|(_, l)| l.kind == IndexKind::E).copied().unwrap();

    // 1: S names a part: byte 0 of it; `prev` is the E line before it.
    let c = one(dir.path(), &pos(e.raw_end, e.seq + 1, e.hash), &bound);
    assert!(c.part.ends_with(part_name(e.raw_end)));
    assert_eq!((c.c_start, c.prev, c.first_seq), (0, e.hash, e.seq + 1));
    // 2: S is an L line's end: the same part at that line's c_end.
    let c = one(dir.path(), &pos(l.raw_end, l.seq + 1, l.hash), &bound);
    assert!(c.part.ends_with(part_name(p0)));
    assert_eq!(c.c_start, l.c_end);
    // 3: S is an E line's end and the next part is not on disk yet: nothing to send.
    for f in fs::read_dir(dir.path()).unwrap() {
        let name = f.unwrap().file_name().into_string().unwrap();
        let first: Option<u64> = name.strip_prefix("rec-").and_then(|n| n.strip_suffix(".gcr.zst")?.parse().ok());
        if first.is_some_and(|q| q >= e.raw_end) {
            fs::remove_file(dir.path().join(name)).unwrap();
        }
    }
    assert_eq!(cut(dir.path(), &pos(e.raw_end, e.seq + 1, e.hash), &Bound::OnDisk), Cut::Nothing);
    // 4: S is nowhere in the local index.
    assert!(is_stuck(cut(dir.path(), &pos(l.raw_end + 1, l.seq + 1, l.hash), &bound)));
}

/// PRD-01's recovery fallback: a crashed part ends at an `L` line and the next part starts at that
/// line's `raw_end`. Case 1 wins over case 2.
#[test]
fn a_fallback_part_after_an_l_line_starts_at_its_byte_0() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = 0;
    let mut w = PartWriter::open(dir.path(), opts(u64::MAX), ENTRY, BOOT_ID, at(0)).unwrap();
    let d = work(&mut w, &mut n, 3, 2 << 10);
    std::mem::forget(w);
    let l = d.line;
    assert_eq!(l.kind, IndexKind::L);
    // The next part, as the fallback would start it: its own bytes and one index line.
    let next = l.raw_end;
    fs::write(dir.path().join(part_name(next)), noise(9, 300)).unwrap();
    let line = IndexLine { kind: IndexKind::L, seq: l.seq + 1, raw_end: next + 500, c_end: 300, hash: [5; 32] };
    fs::write(dir.path().join(index_name(next)), format!("{line}\n")).unwrap();

    let c = one(dir.path(), &pos(next, l.seq + 1, l.hash), &Bound::OnDisk);
    assert!(c.part.ends_with(part_name(next)));
    assert_eq!((c.c_start, c.c_end, c.raw_end, c.prev), (0, 300, next + 500, l.hash));
}

#[test]
fn never_passes_the_durable_point() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = 0;
    let mut w = PartWriter::open(dir.path(), opts(u64::MAX), ENTRY, BOOT_ID, at(0)).unwrap();
    work(&mut w, &mut n, 10, 2 << 10);
    let all = lines(dir.path());
    let (p0, mid) = all[4];
    let bound = durable(p0, mid);
    let c = one(dir.path(), &pos(0, 0, ZERO), &bound);
    assert_eq!((c.raw_end, c.c_end, c.last_seq), (mid.raw_end, mid.c_end, mid.seq));
    assert_eq!(cut(dir.path(), &after(&c), &bound), Cut::Nothing);
    let c2 = one(dir.path(), &after(&c), &durable(p0, all[9].1));
    assert_eq!((c2.c_start, c2.raw_end), (mid.c_end, all[9].1.raw_end));
}

#[test]
fn stays_within_32_mib_and_never_has_raw_length_0() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = PartWriter::open(dir.path(), opts(u64::MAX), ENTRY, BOOT_ID, at(0)).unwrap();
    for i in 1..=40u64 {
        let bytes = noise(i, 1 << 20);
        w.write(&Record::Data { sid: 1, t: at(i).mono_ns, off: 0, flags: 0, bytes }).unwrap();
        w.link(at(i)).unwrap();
    }
    let d = w.seal(at(41)).unwrap();
    assert!(d.line.c_end > MAX_CHUNK, "the part needs more than one chunk");
    let (mut p, mut sizes) = (pos(0, 0, ZERO), Vec::new());
    while let Cut::Chunk(c) = cut(dir.path(), &p, &Bound::Durable(d.clone())) {
        assert!(c.c_end - c.c_start <= MAX_CHUNK);
        sizes.push(c.c_end - c.c_start);
        p = after(&c);
    }
    assert!(sizes.len() >= 2);
    assert_eq!(sizes.iter().sum::<u64>(), d.line.c_end);

    // A writer bug: the first line past S lies more than 32 MiB on. Stuck, never a chunk.
    let bad = tempfile::tempdir().unwrap();
    let big = IndexLine { kind: IndexKind::L, seq: 0, raw_end: 100, c_end: MAX_CHUNK + 1, hash: [1; 32] };
    fs::write(bad.path().join(part_name(0)), b"").unwrap();
    fs::write(bad.path().join(index_name(0)), format!("{big}\n")).unwrap();
    assert!(is_stuck(cut(bad.path(), &pos(0, 0, ZERO), &durable(0, big))));
    // A writer bug: a line that adds no raw bytes. Stuck, never a chunk of raw length 0.
    let a = IndexLine { kind: IndexKind::L, seq: 0, raw_end: 50, c_end: 10, hash: [1; 32] };
    let b = IndexLine { seq: 1, c_end: 20, hash: [2; 32], ..a };
    fs::write(bad.path().join(index_name(0)), format!("{a}\n{b}\n")).unwrap();
    assert!(is_stuck(cut(bad.path(), &pos(50, 1, [1; 32]), &durable(0, b))));
}

/// A dead entry closed by recovery: the frame after the last `L` line, the recovery frame and
/// its `R` line go in one chunk.
#[test]
fn the_recovery_chunk_is_sent_whole() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = 0;
    let mut w = PartWriter::open(dir.path(), opts(u64::MAX), ENTRY, BOOT_ID, at(0)).unwrap();
    let d = work(&mut w, &mut n, 4, 2 << 10);
    std::mem::forget(w);
    let count_at = |_: u64| br#"{"ev":"count"}"#.to_vec();
    let serde_json::Value::Object(tail) = serde_json::json!({"cause": "killed"}) else { unreachable!() };
    let rc = Recover {
        entry: ENTRY,
        boot_id: BOOT_ID,
        now: at(n + 10),
        durable: None,
        count_at: &count_at,
        tail,
        dead_start: None,
        seal: Some(br#"{"ev":"seal","reason":"recovered"}"#.to_vec()),
    };
    let r = PartWriter::recover(dir.path(), opts(u64::MAX), rc).unwrap().line.unwrap();
    assert_eq!(r.kind, IndexKind::R);

    let l = d.line;
    let c = one(dir.path(), &pos(l.raw_end, l.seq + 1, l.hash), &Bound::OnDisk);
    assert_eq!((c.c_start, c.c_end, c.raw_end, c.last_seq, c.head), (l.c_end, r.c_end, r.raw_end, r.seq, r.hash));
    assert_eq!(cut(dir.path(), &after(&c), &Bound::OnDisk), Cut::Nothing);
}

#[test]
fn a_head_or_seq_the_index_does_not_hold_is_stuck() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = 0;
    let mut w = PartWriter::open(dir.path(), opts(32 << 10), ENTRY, BOOT_ID, at(0)).unwrap();
    let d = work(&mut w, &mut n, 12, 6 << 10);
    let bound = Bound::Durable(d);
    assert!(is_stuck(cut(dir.path(), &pos(0, 0, [0xff; 32]), &bound)));
    assert!(is_stuck(cut(dir.path(), &pos(0, 3, ZERO), &bound)));

    // A pruned boundary: the part before S is gone; only `confirmed` holds the link at S.
    let (_, e) = lines(dir.path()).into_iter().find(|(_, l)| l.kind == IndexKind::E).unwrap();
    fs::remove_file(dir.path().join(part_name(0))).unwrap();
    fs::remove_file(dir.path().join(index_name(0))).unwrap();
    let p = pos(e.raw_end, e.seq + 1, e.hash);
    assert!(is_stuck(cut(dir.path(), &p, &bound)));
    let ok = Confirmed { raw: e.raw_end, seq: e.seq, hash: hex(&e.hash), at: String::new() };
    let c = match chunk::cut(dir.path(), &p, &bound, Some(&ok)).unwrap() {
        Cut::Chunk(c) => c,
        other => panic!("expected a chunk, got {other:?}"),
    };
    assert_eq!((c.c_start, c.prev), (0, e.hash));
    let wrong = Confirmed { hash: hex(&[3; 32]), ..ok };
    assert!(is_stuck(chunk::cut(dir.path(), &p, &bound, Some(&wrong)).unwrap()));
}

/// With no live writer, a part whose last `L` line lies past the file's length is sent only up to
/// the last whole line.
#[test]
fn on_disk_never_sends_past_the_last_whole_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = 0;
    let mut w = PartWriter::open(dir.path(), opts(u64::MAX), ENTRY, BOOT_ID, at(0)).unwrap();
    work(&mut w, &mut n, 6, 2 << 10);
    std::mem::forget(w);
    let all = lines(dir.path());
    let (last, prev) = (all[5].1, all[4].1);
    let f = fs::OpenOptions::new().write(true).open(dir.path().join(part_name(0))).unwrap();
    f.set_len(last.c_end - 1).unwrap();
    let c = one(dir.path(), &pos(0, 0, ZERO), &Bound::OnDisk);
    assert_eq!((c.c_end, c.raw_end), (prev.c_end, prev.raw_end));
    assert_eq!(cut(dir.path(), &after(&c), &Bound::OnDisk), Cut::Nothing);
}
