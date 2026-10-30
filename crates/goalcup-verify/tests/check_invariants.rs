//! `goalcup-verify check` on PRD-01's 14 invariants (PRD-08 section 3, "Integrity"): a clean
//! entry passes, as part files and as a fetched copy; one mutation per invariant, plus wrong or
//! missing COUNT and seal keys, a torn tail and a repeated native `path` and `gen`, fails its own
//! check with exit 2 (PRD-17 VR3: never 3); an unknown record kind and GAP reason 8 only warn.

mod common;

use common::*;
use goalcup_format::codec::Record;
use goalcup_format::{GapReason, StreamKind};
use serde_json::{Value, json};
use std::path::PathBuf;

/// A sid of the first OPEN of this kind.
fn sid_of(recs: &[Record], kind: StreamKind) -> u64 {
    match &recs[find(
        recs,
        |r| matches!(r, Record::Open { skind, .. } if *skind == kind),
    )] {
        Record::Open { sid, .. } => *sid,
        _ => unreachable!(),
    }
}

fn bump(v: &mut Value, k: &str, by: u64) {
    v[k] = json!(v[k].as_u64().unwrap() + by);
}

fn m_no_part_first(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        r.remove(0);
    })
}

fn m_link_seq_hole(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        for rec in r.iter_mut() {
            if let Record::Link { seq, .. } = rec
                && *seq > 0
            {
                *seq += 1;
            }
        }
    })
}

fn m_link_wrong_hash(name: &str) -> PathBuf {
    mutate(&clean(name), Some(0), |_| {})
}

fn m_data_before_open(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        let i = find(r, |x| {
            matches!(
                x,
                Record::Open {
                    skind: StreamKind::HttpReq,
                    ..
                }
            )
        });
        r.swap(i, i + 1);
    })
}

fn m_offset_hole(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        let s = sid_of(r, StreamKind::Native);
        let i = find(
            r,
            |x| matches!(x, Record::Data { sid, off, .. } if *sid == s && *off > 0),
        );
        if let Record::Data { off, .. } = &mut r[i] {
            *off += 10;
        }
        let j = find(r, |x| matches!(x, Record::End { sid, .. } if *sid == s));
        if let Record::End { seen, .. } = &mut r[j] {
            *seen += 10;
        }
    })
}

fn m_mask_over_text(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        let i = find(r, |x| matches!(x, Record::Mask { .. }));
        if let Record::Mask { off, .. } = &mut r[i] {
            *off = 0;
        }
    })
}

fn m_boot_without_gap(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.crash("killed", 5_000);
    e.count();
    mutate(&e.seal(), None, |r| {
        r.retain(|x| {
            !matches!(
                x,
                Record::Gap {
                    reason: GapReason::RecorderDown,
                    ..
                }
            )
        });
    })
}

fn m_count_off_by_one(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        edit_event(r, "count", 0, |v| bump(v, "http_post", 1))
    })
}

fn m_count_key_missing(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        edit_event(r, "count", 0, |v| {
            v.as_object_mut().unwrap().remove("http_post");
        })
    })
}

fn m_record_after_seal(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        let json = br#"{"ev":"note","what":"outlived","count":1,"programs":["fake"]}"#.to_vec();
        r.push(Record::Event {
            t: MONO0 + 3_600_000 * MS,
            json,
        });
    })
}

fn m_resp_names_missing_req(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        let i = find(r, |x| {
            matches!(
                x,
                Record::Open {
                    skind: StreamKind::HttpResp,
                    ..
                }
            )
        });
        if let Record::Open { req, .. } = &mut r[i] {
            *req = 99;
        }
    })
}

fn m_stream_without_end(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        let s = sid_of(r, StreamKind::WsDown);
        r.retain(|x| !matches!(x, Record::End { sid, .. } if *sid == s));
    })
}

fn m_seen_off(name: &str) -> PathBuf {
    mutate(&clean(name), None, |r| {
        let s = sid_of(r, StreamKind::HttpReq);
        let i = find(r, |x| matches!(x, Record::End { sid, .. } if *sid == s));
        if let Record::End { seen, .. } = &mut r[i] {
            *seen += 5;
        }
    })
}

fn m_gap_without_extent(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    let tail = json!({"guard": "held_bytes", "limit": 1});
    e.gap(0, GapReason::MemoryGuard, 0, 0, 0, tail);
    e.seal()
}

fn m_count_silence(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.idle(200_000);
    e.count();
    e.seal()
}

/// A COUNT more than 180 s late, written by recovery at the dead boot's last kept LINK.
fn m_count_silence_then_crash(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.idle(200_000);
    e.crash("killed", 1_000);
    e.seal()
}

fn m_native_resume_hole(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.crash("killed", 1_000);
    e.native("fake:sessions/s1.jsonl", 0, 50);
    e.seal()
}

fn m_seal_totals_wrong(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.seal_with(|t| bump(t, "http_post", 1))
}

fn m_seal_totals_key_missing(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.seal_with(|t| {
        t.as_object_mut().unwrap().remove("hosts");
    })
}

fn m_torn_tail(name: &str) -> PathBuf {
    let d = clean(name);
    // An EVENT that says 32 body bytes and holds 1.
    append_raw(&d, &[0x07, 0x20, 0x00]);
    d
}

fn m_native_path_gen_twice(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.native("fake:sessions/s1.jsonl", 0, 22);
    e.seal()
}

fn m_gap_reason_8(name: &str) -> PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.gap(0, GapReason::from_u8(8), 1_000, 0, 1, Value::Null);
    e.seal()
}

type Build = fn(&str) -> PathBuf;

/// PRD-17 VR3: every mutation fails its own check with exit 2, never 3. (check id, builder).
const FAILS: &[(&str, Build)] = &[
    ("inv1", m_no_part_first),
    ("inv2", m_link_seq_hole),
    ("inv2", m_link_wrong_hash),
    ("inv3", m_data_before_open),
    ("inv4", m_offset_hole),
    ("inv5", m_mask_over_text),
    ("inv6", m_boot_without_gap),
    ("inv7", m_count_off_by_one),
    ("inv7", m_count_key_missing),
    ("inv8", m_record_after_seal),
    ("inv9", m_resp_names_missing_req),
    ("streams_closed", m_stream_without_end),
    ("seen_matches_written", m_seen_off),
    ("gap_has_end", m_gap_without_extent),
    ("recorder_alive", m_count_silence),
    ("recorder_alive", m_count_silence_then_crash),
    ("inv14", m_native_resume_hole),
    ("counts_match_seal", m_seal_totals_wrong),
    ("counts_match_seal", m_seal_totals_key_missing),
    ("torn_tail", m_torn_tail),
    ("native_path_gen", m_native_path_gen_twice),
];

#[test]
fn vr3_every_mutation_fails_its_check_with_exit_2() {
    for (i, (id, build)) in FAILS.iter().enumerate() {
        let (code, v) = check(&build(&format!("vr3-{i}")), &[]);
        assert_eq!(result(&v, id), "fail", "mutation {i} ({id})");
        assert_eq!(code, 2, "mutation {i} ({id})");
    }
}

#[test]
fn clean_entry_passes_as_parts_and_as_fetched_copy() {
    let d = clean("clean");
    let (code, v) = check(&d, &[]);
    assert_eq!(code, 0, "{:?}", ids_with(&v, "fail"));
    assert_eq!(v["result"], "pass");
    for n in 1..=9 {
        assert_eq!(result(&v, &format!("inv{n}")), "pass", "inv{n}");
    }
    for id in [
        "streams_closed",
        "seen_matches_written",
        "gap_has_end",
        "recorder_alive",
        "inv14",
        "counts_match_seal",
        "torn_tail",
        "native_path_gen",
        "gaps_named",
        "record_decode",
    ] {
        assert_eq!(result(&v, id), "pass", "{id}");
    }
    let e = &v["entries"][0];
    assert_eq!(e["sealed"], true);
    assert_eq!(e["boots"], 1);
    assert_eq!(e["label"], "wire_and_native");
    // One POST and one WebSocket client message: the participant's "model requests written".
    assert_eq!(e["counts"]["model_requests"], 2);

    let out = fetched(&d, &tmp("clean-fetched"));
    let (code, v) = check(&out, &[]);
    assert_eq!(code, 0, "{:?}", ids_with(&v, "fail"));
}

#[test]
fn counts_apart_by_a_suspend_pass() {
    let mut e = Entry::new("suspend");
    e.standard();
    e.sleep(200_000);
    e.count();
    let (code, v) = check(&e.seal(), &[]);
    assert_eq!(result(&v, "recorder_alive"), "pass");
    assert_eq!(code, 0, "{:?}", ids_with(&v, "fail"));
}

#[test]
fn unknown_record_kind_warns_and_keeps_the_exit() {
    let mut e = Entry::new("unknown-kind");
    e.standard();
    e.put(Record::Unknown {
        kind: 0x20,
        body: b"future".to_vec(),
    });
    e.count();
    let (code, v) = check(&e.seal(), &[]);
    assert_eq!(result(&v, "record_kind_unknown"), "warn");
    assert_eq!(code, 0, "{:?}", ids_with(&v, "fail"));
}

#[test]
fn gap_reason_8_warns_and_does_not_crash() {
    let (code, v) = check(&m_gap_reason_8("gap-reason-8"), &[]);
    assert_eq!(result(&v, "gap_reason_unknown"), "warn");
    // A GAP is a named loss: exit 1, from `gaps_named`, never a crash.
    assert_eq!(result(&v, "gaps_named"), "named");
    assert_eq!(code, 1, "{:?}", ids_with(&v, "fail"));
}

#[test]
fn missing_input_exits_3() {
    let gone = tmp("missing").join("no-such-entry");
    let (code, _) = run(&["check", gone.to_str().unwrap(), "--json"]);
    assert_eq!(code, 3);
}
