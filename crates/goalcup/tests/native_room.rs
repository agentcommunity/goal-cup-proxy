//! PRD-04 acceptance `native_room.rs` (point 3, the file is the buffer): with no room nothing is
//! read and nothing is lost, a read never passes `room()`, and one huge file cannot starve another.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use goalcup::native::fsx::Dir;
use goalcup::native::tail::{Stop, Tail};
use goalcup::pipeline::{Call, Cursors, Recorder};

fn walk(rec: &Recorder, tail: &mut Tail, dir: &Path, names: &[&str]) {
    let root = Dir::open_root(dir).unwrap();
    for name in names {
        let st = root.stat_at(OsStr::new(name)).unwrap();
        tail.settle(rec, format!("row:{name}"), dir, Path::new(name), st, false);
    }
    tail.sweep(rec);
}

/// Every DATA piece of the stream for `id`, as (offset, length), and the bytes joined.
fn pieces(c: &[Call], id: &str) -> (Vec<(u64, usize)>, Vec<u8>) {
    let sid = c.iter().find_map(|c| match c {
        Call::Open(sid, .., meta) if meta["path"] == id => Some(*sid),
        _ => None,
    });
    let (mut at, mut bytes) = (Vec::new(), Vec::new());
    for call in c {
        if let Call::Data(s, _, off, b) = call
            && Some(*s) == sid
        {
            at.push((*off, b.len()));
            bytes.extend_from_slice(b);
        }
    }
    (at, bytes)
}

fn contiguous(at: &[(u64, usize)]) -> bool {
    at.windows(2).all(|w| w[0].0 + w[0].1 as u64 == w[1].0) && at.first().is_none_or(|a| a.0 == 0)
}

#[test]
fn no_room_reads_nothing_and_every_byte_arrives_in_order_once_room_returns() {
    let dir = tempfile::tempdir().unwrap();
    let body: Vec<u8> = (0..(1u32 << 20) + 7).map(|i| (i % 251) as u8).collect();
    fs::write(dir.path().join("big"), &body).unwrap();
    let (rec, calls) = Recorder::for_tests(0);
    let mut tail = Tail::new(UNIX_EPOCH, Cursors::default());
    walk(&rec, &mut tail, dir.path(), &["big"]);
    assert!(matches!(
        tail.read(&rec, 64 << 20, Duration::ZERO),
        Stop::Room
    ));
    assert!(!calls.all().iter().any(|c| matches!(c, Call::Data(..))));
    calls.set_room(100_000);
    while !matches!(tail.read(&rec, 64 << 20, Duration::ZERO), Stop::Done) {}
    let (at, bytes) = pieces(&calls.all(), "row:big");
    assert!(contiguous(&at));
    assert!(
        at.iter().all(|a| a.1 <= 100_000),
        "no piece is longer than room()"
    );
    assert_eq!(bytes, body);
}

#[test]
fn a_huge_file_does_not_starve_a_small_one() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a-huge"), vec![b'a'; 3 << 20]).unwrap();
    fs::write(dir.path().join("b-small"), b"small\n").unwrap();
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let mut tail = Tail::new(UNIX_EPOCH, Cursors::default());
    walk(&rec, &mut tail, dir.path(), &["a-huge", "b-small"]);
    assert!(matches!(
        tail.read(&rec, 1 << 20, Duration::ZERO),
        Stop::Slice
    ));
    assert!(pieces(&calls.all(), "row:b-small").1.is_empty());
    tail.read(&rec, 1 << 20, Duration::ZERO);
    assert_eq!(pieces(&calls.all(), "row:b-small").1, b"small\n");
    while !matches!(tail.read(&rec, 1 << 20, Duration::ZERO), Stop::Done) {}
    let (at, bytes) = pieces(&calls.all(), "row:a-huge");
    assert!(contiguous(&at));
    assert_eq!(bytes.len(), 3 << 20);
}
