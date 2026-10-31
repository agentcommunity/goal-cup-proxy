//! PRD-02 P13 (`pipeline_mask_panic`), points 1, 3 and 4: a masker panic drops only that piece
//! and what the masker held, behind one `mask_failed` GAP, writes no raw byte, and the stream goes
//! on with DATA and GAP exactly contiguous. A `fin` piece ends its message, and an EVENT body is
//! masked. The panic is the test-hooks marker (`pipeline::PANIC_MARKER`).

mod common;

use bytes::Bytes;
use common::{HANG, within};
use goalcup::mask::Masker;
use goalcup::pipeline::{
    EndReason, GapReason, OpenFlags, PANIC_MARKER, Recorder, StreamHandle, StreamKind,
};
use goalcup_format::Record;
use serde_json::json;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

fn open(rec: &Recorder, kind: StreamKind) -> StreamHandle {
    let flags = OpenFlags::POST | OpenFlags::DECRYPTED;
    rec.open(kind, 0, 0, 0, flags, json!({}))
}

/// Ends one more request stream, then waits for a durable point that counts `posts` requests
/// ended, with nothing held: everything offered before it is written and synced.
fn settle(rec: &Recorder, posts: u64) {
    open(rec, StreamKind::HttpReq).end(rec.now(), EndReason::Complete, None);
    let rec = rec.clone();
    within(HANG, move || {
        while rec.durable().borrow().totals.counts.http_post < posts || rec.live().held_bytes > 0 {
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

/// The index file of a part file.
fn index_of(part: &Path) -> std::path::PathBuf {
    let name = part.file_name().unwrap().to_str().unwrap();
    part.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")))
}

/// Every record of the entry up to its last index line.
fn records(dir: &Path) -> Vec<Record> {
    let mut out = Vec::new();
    for part in goalcup_format::read::part_files(&dir.join("rec")).unwrap() {
        let Some(last) = goalcup_format::read::read_index(&index_of(&part))
            .unwrap()
            .pop()
        else {
            continue;
        };
        let f = std::fs::File::open(&part).unwrap().take(last.c_end);
        out.extend(
            goalcup_format::read::decode(f)
                .unwrap()
                .map(|i| i.unwrap().rec),
        );
    }
    out
}

/// Every byte of the part files as stored and as decompressed (to the end of what is flushed),
/// and of their index files.
fn all_bytes(dir: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    for part in goalcup_format::read::part_files(&dir.join("rec")).unwrap() {
        out.extend(std::fs::read(&part).unwrap());
        out.extend(std::fs::read(index_of(&part)).unwrap());
        let mut z = zstd::stream::read::Decoder::new(std::fs::File::open(&part).unwrap()).unwrap();
        let mut buf = [0u8; 1 << 16];
        while let Ok(n @ 1..) = z.read(&mut buf) {
            out.extend_from_slice(&buf[..n]);
        }
    }
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// One stream's DATA, MASK and GAP records in order, checked contiguous from 0 to its END's
/// `seen` (PRD-01 invariants 4, 5 and 11). Returns (DATA off, bytes, flags) and the GAPs as
/// (reason, off, lost + 1).
#[allow(clippy::type_complexity)]
fn stream(recs: &[Record], want: u64) -> (Vec<(u64, Vec<u8>, u8)>, Vec<(GapReason, u64, u64)>) {
    let (mut data, mut gaps, mut at) = (Vec::new(), Vec::new(), 0);
    for r in recs {
        match r {
            Record::Data {
                sid,
                off,
                flags,
                bytes,
                ..
            } if *sid == want => {
                assert_eq!(*off, at, "DATA where the stream stands");
                at += bytes.len() as u64;
                data.push((*off, bytes.clone(), *flags));
            }
            Record::Mask { sid, off, len, .. } if *sid == want => {
                let (d_off, d, _) = data.last().expect("MASK after its DATA");
                assert!(*off >= *d_off && off + len <= d_off + d.len() as u64);
                let s = (off - d_off) as usize;
                assert!(d[s..s + *len as usize].iter().all(|&b| b == b'*'));
            }
            Record::Gap {
                sid,
                reason,
                start_mono,
                end_mono,
                off,
                lost,
                ..
            } if *sid == want => {
                assert_eq!(*off, at, "the GAP starts at the hole");
                assert!(start_mono < end_mono);
                at += lost - 1;
                gaps.push((*reason, *off, *lost));
            }
            Record::End { sid, seen, .. } if *sid == want => {
                assert_eq!(*seen, at, "DATA and GAP fill the stream");
                return (data, gaps);
            }
            _ => {}
        }
    }
    panic!("no END for stream {want}");
}

#[test]
fn a_marker_piece_is_one_mask_failed_gap_and_the_stream_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::real_for_tests(dir.path(), Masker::passthrough_for_tests());
    let piece = [b"ab".as_slice(), PANIC_MARKER, b"cd"].concat();
    let mut h = open(&rec, StreamKind::HttpReq);
    let sid = h.sid();
    h.data(rec.now(), Bytes::from_static(b"hello "));
    h.data(rec.now(), Bytes::from(piece.clone()));
    h.data(rec.now(), Bytes::from_static(b"world"));
    h.end(rec.now(), EndReason::Complete, None);
    settle(&rec, 2);
    let (data, gaps) = stream(&records(dir.path()), sid);
    let n = piece.len() as u64;
    assert_eq!(gaps, vec![(GapReason::MaskFailed, 6, n + 1)]);
    let bytes: Vec<&[u8]> = data.iter().map(|d| d.1.as_slice()).collect();
    assert_eq!(bytes, vec![b"hello ".as_slice(), b"world"]);
    assert_eq!(data[1].0, 6 + n);
    assert!(!contains(&all_bytes(dir.path()), PANIC_MARKER));
}

/// The masker in force (the S2 stub until PRD-05's core masker merges) with a candidate it may
/// hold back at the end of the first piece; the panic drops it with the piece, and DATA, MASK and
/// GAP stay contiguous either way. The stub holds nothing, so with it only the empty hold runs.
#[test]
fn a_panic_with_bytes_held_leaves_data_and_gap_contiguous() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::real_for_tests(dir.path(), Masker::new::<()>(&[], Vec::new()).unwrap());
    let mut h = open(&rec, StreamKind::HttpResp);
    let sid = h.sid();
    let pieces: [&[u8]; 3] = [b"data: {\"auth\": \"Bearer sk-", PANIC_MARKER, b"\"}\n\n"];
    for p in pieces {
        h.data(rec.now(), Bytes::from_static(p));
    }
    h.end(rec.now(), EndReason::Complete, None);
    settle(&rec, 1);
    let (data, gaps) = stream(&records(dir.path()), sid);
    assert_eq!(gaps.len(), 1);
    let (reason, off, lost) = gaps[0];
    assert_eq!(reason, GapReason::MaskFailed);
    let held = (pieces[0].len() as u64).saturating_sub(off);
    assert_eq!(lost, held + PANIC_MARKER.len() as u64 + 1);
    assert!(
        data.iter().any(|d| d.0 == off + lost - 1),
        "the stream goes on"
    );
}

#[test]
fn a_fin_piece_ends_its_message() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::real_for_tests(dir.path(), Masker::passthrough_for_tests());
    let mut h = rec.open(StreamKind::WsUp, 0, 0, 0, OpenFlags::NONE, json!({}));
    let sid = h.sid();
    let msgs: [(u8, bool, &[u8]); 5] = [
        (0x1, false, b"abc"),
        (0x1, true, b"def"),
        (0x2, true, b""),
        (0x1, true, b"x"),
        (0x9, true, b"p"),
    ];
    for (op, fin, b) in msgs {
        h.ws(rec.now(), op, fin, Bytes::from_static(b));
    }
    h.end(rec.now(), EndReason::Complete, None);
    settle(&rec, 1);
    let (data, gaps) = stream(&records(dir.path()), sid);
    assert!(gaps.is_empty());
    let got: Vec<(u64, &[u8], u8)> = data.iter().map(|d| (d.0, d.1.as_slice(), d.2)).collect();
    let want: Vec<(u64, &[u8], u8)> = vec![
        (0, b"abc", 0x10),
        (3, b"def", 0x11),
        (6, b"", 0x21),
        (6, b"x", 0x11),
        (7, b"p", 0x91),
    ];
    assert_eq!(got, want);
}

#[test]
fn an_event_body_is_masked() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::real_for_tests(dir.path(), Masker::new::<()>(&[], Vec::new()).unwrap());
    let key = format!("sk-{}{}", "x".repeat(20), "7".repeat(20));
    rec.event(json!({"ev": "note", "what": "connect_refused", "detail": format!("Bearer {key}")}));
    settle(&rec, 1);
    let events = records(dir.path())
        .into_iter()
        .filter(|r| matches!(r, Record::Event { .. }))
        .count();
    assert!(events >= 2, "the start event and the note are written");
    assert!(!contains(&all_bytes(dir.path()), key.as_bytes()));
}
