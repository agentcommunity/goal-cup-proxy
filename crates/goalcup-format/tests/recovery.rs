//! Crash recovery (section 12) at window 2^27: a part file cut at any byte recovers to its last
//! index line, is closed with `01 00 00` and a recovery frame, and the entry decodes as one stream
//! with one `recorder_down` gap; every part file decodes alone. A boot that left no index line is
//! deleted and its number reused; a part file already closed by `R` takes a second recovery frame;
//! the recovery frame's COUNT equals a recount of its boot; a sealed entry takes nothing more.

use goalcup_format::read::{Item, decode, open_dir, part_files, read_index};
use goalcup_format::{
    GapReason, IndexKind, Now, PartOptions, PartWriter, Record, Recover, StreamKind,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const ENTRY: &str = "e00000000000000aa";
const BOOT_A: [u8; 16] = [0xaa; 16];

struct Clock(u64);

impl Clock {
    fn now(&mut self) -> Now {
        self.0 += 1_000_000;
        Now {
            wall_ns: 1_794_009_600_000_000_000 + self.0,
            mono_ns: 7_000_000_000 + self.0,
        }
    }
}

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// The recorder's side of one boot: a start event, then `n` exchanges, a LINK every 4. Records
/// the COUNT (`http_req` OPENs written in this boot) at each LINK, as `counts.jsonl` would.
fn work(w: &mut PartWriter, c: &mut Clock, n: u64, counts: &mut HashMap<u64, Vec<u8>>) {
    let boot = w.boot();
    let ev = format!(r#"{{"ev":"start","resume":{}}}"#, boot > 0);
    w.write(&Record::Event {
        t: c.now().mono_ns,
        json: ev.into_bytes(),
    })
    .unwrap();
    for i in 0..n {
        let sid = 1 + i;
        let open = Record::Open {
            sid,
            t: c.now().mono_ns,
            skind: StreamKind::HttpReq,
            conn: 0,
            req: 0,
            meta: b"{}".to_vec(),
        };
        w.write(&open).unwrap();
        let body = format!(
            r#"{{"model":"m1","input":"{}"}}"#,
            "hello world ".repeat(200 + i as usize * 37)
        );
        let len = body.len() as u64;
        w.write(&Record::Data {
            sid,
            t: c.now().mono_ns,
            off: 0,
            flags: 0,
            bytes: body.into_bytes(),
        })
        .unwrap();
        let end = Record::End {
            sid,
            t: c.now().mono_ns,
            reason: goalcup_format::EndReason::Complete,
            seen: len,
            tail: vec![],
        };
        w.write(&end).unwrap();
        if i % 4 == 3 {
            let seq = w.link(c.now()).unwrap().seq;
            counts.insert(
                seq,
                format!(r#"{{"ev":"count","boot":{boot},"http_req":{}}}"#, i + 1).into_bytes(),
            );
        }
    }
    w.sync().unwrap();
}

fn idx_of(part: &Path) -> PathBuf {
    let name = part.file_name().unwrap().to_str().unwrap();
    part.with_file_name(name.replace(".gcr.zst", ".idx"))
}

/// A recorder killed with `kill -9`: no destructor runs, nothing is finished.
fn kill(w: PartWriter) {
    std::mem::forget(w);
}

fn rc<'a>(now: Now, count_at: &'a dyn Fn(u64) -> Vec<u8>) -> Recover<'a> {
    let serde_json::Value::Object(tail) = serde_json::json!({"cause": {"signal": 9}, "exits": 1})
    else {
        unreachable!()
    };
    Recover {
        entry: ENTRY,
        boot_id: BOOT_A,
        now,
        durable: None,
        count_at,
        tail,
        dead_start: None,
        seal: None,
    }
}

fn recover(d: &Path, c: &mut Clock, counts: &HashMap<u64, Vec<u8>>) -> goalcup_format::Recovered {
    let count_at = |seq: u64| counts[&seq].clone();
    PartWriter::recover(d, PartOptions::default(), rc(c.now(), &count_at)).unwrap()
}

fn tail_of(g: &Record) -> serde_json::Value {
    let Record::Gap { tail, .. } = g else {
        panic!("not a gap")
    };
    serde_json::from_slice(tail).unwrap()
}

/// Reads the whole entry and checks what holds for any recovered entry: no error, closed frames,
/// no cut record, every link recomputes, link seqs run without a hole, boots have no hole.
fn read_entry(d: &Path) -> Vec<Item> {
    let mut rd = open_dir(d).unwrap();
    let items: Vec<Item> = rd.by_ref().collect::<Result<_, _>>().unwrap();
    assert_eq!(rd.tail(), Default::default());
    let seqs: Vec<u64> = items
        .iter()
        .filter_map(|i| match i.rec {
            Record::Link { seq, .. } => {
                assert_eq!(i.link_ok, Some(true));
                Some(seq)
            }
            _ => None,
        })
        .collect();
    assert_eq!(seqs, (0..seqs.len() as u64).collect::<Vec<_>>());
    for p in part_files(d).unwrap() {
        let mut rd = decode(fs::File::open(&p).unwrap()).unwrap();
        assert!(rd.by_ref().all(|i| i.unwrap().link_ok != Some(false)));
        assert_eq!(rd.tail(), Default::default(), "{p:?} decodes alone");
    }
    items
}

fn boots(items: &[Item]) -> Vec<u64> {
    items
        .iter()
        .filter_map(|i| match i.rec {
            Record::Part { boot, .. } => Some(boot),
            _ => None,
        })
        .collect()
}

fn down_gaps(items: &[Item]) -> Vec<usize> {
    (0..items.len())
        .filter(|&i| {
            matches!(
                items[i].rec,
                Record::Gap {
                    reason: GapReason::RecorderDown,
                    ..
                }
            )
        })
        .collect()
}

#[test]
fn a_part_file_cut_at_any_byte_recovers_to_its_last_index_line() {
    let src = dir("recovery-src");
    let mut c = Clock(0);
    let mut counts = HashMap::new();
    let mut w = PartWriter::open(&src, PartOptions::default(), ENTRY, BOOT_A, c.now()).unwrap();
    work(&mut w, &mut c, 22, &mut counts);
    kill(w);
    let part = part_files(&src).unwrap().remove(0);
    let idx = idx_of(&part);
    let lines = read_index(&idx).unwrap();
    assert_eq!(lines.len(), 5);
    let full = fs::read(&part).unwrap();

    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for round in 0..3 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let from = lines[0].c_end;
        let cut = from + seed % (full.len() as u64 - from + 1);
        let d = dir(&format!("recovery-cut-{round}"));
        fs::write(d.join(part.file_name().unwrap()), &full[..cut as usize]).unwrap();
        fs::copy(&idx, d.join(idx.file_name().unwrap())).unwrap();
        let kept = *lines.iter().rev().find(|l| l.c_end <= cut).unwrap();

        let mut c2 = Clock(c.0 + 5_000_000_000);
        let r = recover(&d, &mut c2, &counts);
        assert_eq!(r.line.unwrap().kind, IndexKind::R);
        let mut w = r.writer.unwrap();
        assert_eq!(w.boot(), 1);
        work(&mut w, &mut c2, 5, &mut HashMap::new());
        w.seal(c2.now()).unwrap();

        let items = read_entry(&d);
        assert_eq!(boots(&items), [0, 1]);
        let gaps = down_gaps(&items);
        assert_eq!(gaps.len(), 1, "cut at {cut}");
        let g = gaps[0];
        // COUNT, GAP, LINK: the recovery frame, right after the kept LINK.
        let Record::Link { seq, mono_ns, .. } = items[g - 2].rec else {
            panic!("no kept LINK before the frame")
        };
        assert_eq!(seq, kept.seq);
        let Record::Gap {
            start_mono,
            end_mono,
            lost,
            ..
        } = items[g].rec
        else {
            unreachable!()
        };
        assert_eq!((start_mono, lost), (mono_ns, 0));
        assert!(end_mono > start_mono);
        let Record::Event { json, .. } = &items[g - 1].rec else {
            panic!("no COUNT in the recovery frame")
        };
        let count: serde_json::Value = serde_json::from_slice(json).unwrap();
        let recount = items[..g - 1]
            .iter()
            .filter(|i| {
                matches!(
                    i.rec,
                    Record::Open {
                        skind: StreamKind::HttpReq,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(count["http_req"], recount as u64);
        assert!(matches!(items[g + 1].rec, Record::Link { .. }));
    }
}

#[test]
fn a_boot_with_no_index_line_is_deleted_and_an_r_closed_part_file_takes_a_second_frame() {
    let d = dir("recovery-twice");
    let mut c = Clock(0);
    let mut counts = HashMap::new();
    let mut w = PartWriter::open(&d, PartOptions::default(), ENTRY, BOOT_A, c.now()).unwrap();
    work(&mut w, &mut c, 8, &mut counts);
    kill(w);

    // Boot 1 dies before its first LINK.
    let mut w = recover(&d, &mut c, &counts).writer.unwrap();
    assert_eq!(w.boot(), 1);
    work(&mut w, &mut c, 3, &mut HashMap::new());
    kill(w);
    assert_eq!(part_files(&d).unwrap().len(), 2);

    // Its part file is deleted and boot 1 is reused; part file 0 takes a second recovery frame.
    let r = recover(&d, &mut c, &counts);
    let mut w = r.writer.unwrap();
    assert_eq!(w.boot(), 1);
    work(&mut w, &mut c, 4, &mut HashMap::new());
    w.seal(c.now()).unwrap();

    let first = part_files(&d).unwrap().remove(0);
    let idx = idx_of(&first);
    let kinds: Vec<IndexKind> = read_index(&idx).unwrap().iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        [IndexKind::L, IndexKind::L, IndexKind::R, IndexKind::R]
    );

    let items = read_entry(&d);
    assert_eq!(boots(&items), [0, 1]);
    let gaps = down_gaps(&items);
    assert_eq!(gaps.len(), 2);
    // The second frame holds only the GAP and a LINK: boot 0's COUNT is already in the first.
    assert!(matches!(items[gaps[1] - 1].rec, Record::Link { .. }));
    assert!(matches!(items[gaps[1] + 1].rec, Record::Link { .. }));
    let Record::Gap { end_mono, .. } = items[gaps[0]].rec else {
        unreachable!()
    };
    let Record::Gap { start_mono, .. } = items[gaps[1]].rec else {
        unreachable!()
    };
    assert_eq!(
        start_mono, end_mono,
        "the second gap starts where the first ended, at its LINK"
    );
}

#[test]
fn an_entry_with_no_index_line_starts_boot_0_with_a_gap() {
    let d = dir("recovery-none");
    let mut c = Clock(0);
    let started = c.now();
    let mut w = PartWriter::open(&d, PartOptions::default(), ENTRY, BOOT_A, started).unwrap();
    work(&mut w, &mut c, 3, &mut HashMap::new());
    kill(w);

    let none = |_: u64| unreachable!("no LINK, no COUNT");
    let mut rc = rc(c.now(), &none);
    rc.dead_start = Some((started, BOOT_A));
    let r = PartWriter::recover(&d, PartOptions::default(), rc).unwrap();
    let mut w = r.writer.unwrap();
    assert_eq!(w.boot(), 0);
    w.write(&Record::Event {
        t: c.now().mono_ns,
        json: br#"{"ev":"start"}"#.to_vec(),
    })
    .unwrap();
    let g = r.gap_after_start.unwrap();
    let Record::Gap {
        start_mono,
        end_mono,
        ref tail,
        ..
    } = g
    else {
        panic!("not a gap")
    };
    assert_eq!(start_mono, started.mono_ns);
    assert!(end_mono > start_mono);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(tail).unwrap()["exits"],
        1
    );
    w.write(&g).unwrap();
    w.seal(c.now()).unwrap();

    let items = read_entry(&d);
    assert_eq!(boots(&items), [0]);
    assert_eq!(down_gaps(&items), [2]);
}

#[test]
fn a_no_index_line_gap_has_an_extent_and_names_a_new_clock() {
    let none = |_: u64| unreachable!("no LINK, no COUNT");
    let now = Clock(0).now();
    // No part file and no start time from the watchdog: the gap still has an extent.
    let d = dir("recovery-empty");
    let g = PartWriter::recover(&d, PartOptions::default(), rc(now, &none))
        .unwrap()
        .gap_after_start
        .unwrap();
    let Record::Gap {
        start_mono,
        end_mono,
        ..
    } = g
    else {
        unreachable!()
    };
    assert!(end_mono > start_mono);
    assert_eq!(tail_of(&g).get("boot_id"), None);
    // A start taken before a reboot: the tail names the new clock, wall times give the extent.
    let d = dir("recovery-reboot");
    let mut r = rc(now, &none);
    r.dead_start = Some((now, [0xbb; 16]));
    let g = PartWriter::recover(&d, PartOptions::default(), r)
        .unwrap()
        .gap_after_start
        .unwrap();
    let Record::Gap {
        start_wall,
        end_wall,
        ..
    } = g
    else {
        unreachable!()
    };
    assert!(end_wall > start_wall);
    assert_eq!(tail_of(&g)["boot_id"], "aa".repeat(16));
    assert_eq!(tail_of(&g)["cause"]["signal"], 9);
}

#[test]
fn a_sealed_entry_takes_nothing_more() {
    let seal = br#"{"ev":"seal","reason":"recovered"}"#;
    for by_recovery in [false, true] {
        let d = dir(&format!("recovery-sealed-{by_recovery}"));
        let mut c = Clock(0);
        let mut counts = HashMap::new();
        let mut w = PartWriter::open(&d, PartOptions::default(), ENTRY, BOOT_A, c.now()).unwrap();
        work(&mut w, &mut c, 4, &mut counts);
        let ev = Record::Event {
            t: c.now().mono_ns,
            json: seal.to_vec(),
        };
        let line = if by_recovery {
            kill(w);
            let count_at = |seq: u64| counts[&seq].clone();
            let mut r = rc(c.now(), &count_at);
            r.seal = Some(seal.to_vec());
            PartWriter::recover(&d, PartOptions::default(), r)
                .unwrap()
                .line
        } else {
            w.write(&ev).unwrap();
            Some(w.seal(c.now()).unwrap().line)
        };
        let part = part_files(&d).unwrap().remove(0);
        let files = || (fs::read(&part).unwrap(), fs::read(idx_of(&part)).unwrap());
        let before = files();
        // Killed after the seal was synced, before `sealed` was sent: recovery adds nothing.
        let r = recover(&d, &mut c, &counts);
        assert!(r.writer.is_none() && r.gap_after_start.is_none());
        assert_eq!((r.line, files()), (line, before));
    }
}
