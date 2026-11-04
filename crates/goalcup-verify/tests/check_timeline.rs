//! The `timeline` report (PRD-08 section 3, "Reports that never fail"): buckets of the built
//! entry add up to the checks' facts and the recount; an entry with nothing in it has none.

mod common;

use common::run1::{Shape, run1};
use common::{Entry, check, find, item, mutate, result};
use goalcup_format::{EndReason, GapReason, Record};
use serde_json::Value;

fn rows(v: &Value) -> &Vec<Value> {
    v["entries"][0]["timeline"].as_array().unwrap()
}

fn sum(v: &Value, key: &str) -> u64 {
    rows(v).iter().map(|b| b[key].as_u64().unwrap()).sum()
}

#[test]
fn buckets_add_up_to_the_check_facts_and_counts() {
    let (_, v) = check(&run1("timeline-sums", Shape::Killed), &[]);
    let f = &item(&v, "wire_keeps_up")["facts"];
    assert_eq!(sum(&v, "harness_calls"), f["harness"].as_u64().unwrap());
    let ok = sum(&v, "wire_calls") - sum(&v, "wire_errors");
    assert_eq!(ok, f["wire_ok"].as_u64().unwrap());
    let c = &v["entries"][0]["counts"];
    assert_eq!(sum(&v, "off_ms"), c["off_ms"].as_u64().unwrap());
    let gaps: u64 = c["gaps"]
        .as_object()
        .unwrap()
        .values()
        .map(|n| n.as_u64().unwrap())
        .sum();
    assert_eq!(sum(&v, "gaps"), gaps);
    assert!(sum(&v, "native_bytes") > 0 && rows(&v).len() > 6);
}

#[test]
fn a_recorder_down_for_days_gets_a_row_per_bucket_it_covers() {
    let mut e = Entry::new("timeline-days");
    e.standard();
    let ms = e.crash("killed", 3 * 24 * 3_600_000);
    e.exchange(EndReason::Complete);
    e.count();
    let (_, v) = check(&e.seal(), &[]);
    assert_eq!(sum(&v, "off_ms"), ms);
    let n = rows(&v).len() as u64;
    assert!((ms / 600_000..=ms / 600_000 + 3).contains(&n), "{n} rows");
}

/// PRD-01 section 12: an entry with no index line restarts boot 0 after the dead boot, whose
/// off span starts before the first PART. The timeline still holds the whole span.
#[test]
fn an_off_span_before_the_first_part_is_in_the_timeline() {
    let dir = Entry::recovered_before_first_link("timeline-head", "killed", 30_000);
    let (_, v) = check(&dir, &[]);
    let off = v["entries"][0]["counts"]["off_ms"].as_u64().unwrap();
    assert!(off >= 30_000);
    assert_eq!(sum(&v, "off_ms"), off);
}

#[test]
fn an_entry_with_nothing_in_it_has_an_empty_timeline() {
    let (_, v) = check(&Entry::new("timeline-empty").seal(), &[]);
    assert!(rows(&v).is_empty());
}

/// Moves the entry's `recorder_down` span to times that are not ms-aligned; returns its ns.
fn skew_down_span(dir: &std::path::Path, start_by: i64, end_by: i64) -> u64 {
    let mut span = 0;
    mutate(dir, None, |recs| {
        for r in recs.iter_mut() {
            if let Record::Gap {
                reason: GapReason::RecorderDown,
                start_wall,
                end_wall,
                ..
            } = r
            {
                *start_wall = start_wall.wrapping_add_signed(start_by);
                *end_wall = end_wall.wrapping_add_signed(end_by);
                span = *end_wall - *start_wall;
            }
        }
    });
    span
}

/// Each row divides its ns once: the rows' off_ms is the single-division total of the span,
/// not a floor per piece. Here the head and a piece in bucket 0 share one row.
#[test]
fn a_head_piece_and_the_rest_of_its_row_are_floored_once() {
    let dir = Entry::recovered_before_first_link("timeline-head-ns", "killed", 30_000);
    let span = skew_down_span(&dir, 123_457, 700_300);
    let (_, v) = check(&dir, &[]);
    assert_eq!(sum(&v, "off_ms"), span / 1_000_000);
}

/// A span with ragged ends that crosses a bucket boundary loses under 1 ms per row.
#[test]
fn a_ragged_span_across_buckets_loses_under_a_ms_per_row() {
    let mut e = Entry::new("timeline-ragged");
    e.standard();
    e.crash("killed", 25 * 60_000);
    e.exchange(EndReason::Complete);
    e.count();
    let dir = e.seal();
    let span = skew_down_span(&dir, 123_457, -234_567);
    let (_, v) = check(&dir, &[]);
    let (got, n) = (sum(&v, "off_ms"), rows(&v).len() as u64);
    assert!(n > 2);
    assert!(
        got <= span / 1_000_000 && span / 1_000_000 - got < n,
        "{got} of {span} in {n}"
    );
}

/// A `standard` entry with a crash; the `recorder_down` GAP's end wall and the first LINK after
/// it are moved by `f`.
fn link_wall_after_down(name: &str, f: impl Fn(u64) -> u64) -> std::path::PathBuf {
    let mut e = Entry::new(name);
    e.standard();
    e.crash("killed", 60_000);
    e.exchange(EndReason::Complete);
    e.count();
    let dir = e.seal();
    mutate(&dir, None, |recs| {
        let down = find(recs, is_down);
        if let Record::Gap { end_wall, .. } = &mut recs[down] {
            *end_wall = f(*end_wall);
        }
        let i = down + find(&recs[down..], |r| matches!(r, Record::Link { .. }));
        if let Record::Link { wall_ns, .. } = &mut recs[i] {
            *wall_ns = f(*wall_ns);
        }
    })
}

fn is_down(r: &Record) -> bool {
    matches!(
        r,
        Record::Gap {
            reason: GapReason::RecorderDown,
            ..
        }
    )
}

/// Debug build, a subprocess and up to 100k JSON rows take about 3 s; uncapped takes minutes.
/// The row bound is the real proof; the time bound only catches a run-away.
const SLOW: std::time::Duration = std::time::Duration::from_secs(30);

/// cr-format-verify-4: a forged LINK wall time cannot make the timeline allocate a row per
/// bucket up to u64::MAX (here with the off span forged to reach it); the wall/mono mismatch is
/// named.
#[test]
fn forged_link_wall_does_not_blow_up_the_timeline() {
    let dir = link_wall_after_down("tl-forge", |_| u64::MAX);
    let t = std::time::Instant::now();
    let (_, v) = check(&dir, &[]);
    assert!(t.elapsed() < SLOW);
    assert!(rows(&v).len() <= 100_000, "{} rows", rows(&v).len());
    assert_eq!(result(&v, "wall_sane"), "named");
}

/// Many forged off spans share one bucket budget: 1000 `recorder_down` GAPs from the first PART
/// to u64::MAX cost no more than one, and the off total is kept.
#[test]
fn a_thousand_forged_off_spans_share_one_bucket_budget() {
    let dir = link_wall_after_down("tl-forge-many", |_| u64::MAX);
    let mut n = 0;
    mutate(&dir, None, |recs| {
        let Record::Part { wall_ns: first, .. } = recs[0] else {
            panic!("no PART first")
        };
        let down = find(recs, is_down);
        let mut g = recs[down].clone();
        if let Record::Gap { start_wall, .. } = &mut g {
            *start_wall = first;
        }
        recs.splice(down + 1..down + 1, std::iter::repeat_n(g, 1000));
        n = recs.len();
    });
    let t = std::time::Instant::now();
    let (_, v) = check(&dir, &[]);
    assert!(t.elapsed() < SLOW, "{:?}", t.elapsed());
    assert!(rows(&v).len() <= 100_000 + n, "{} rows", rows(&v).len());
}

/// A clock step after boot is legal: a LINK a day off its PART's wall plus mono passes.
#[test]
fn a_link_wall_a_day_off_the_mono_clock_is_fine() {
    let dir = link_wall_after_down("tl-day", |w| w + 86_400_000_000_000);
    let (_, v) = check(&dir, &[]);
    assert_eq!(result(&v, "wall_sane"), "pass");
}

/// A recovery LINK after a reboot is stamped with the new boot's mono clock, which restarted:
/// it is below the dead boot's PART mono and is not compared with it. Here boot 0 ran on a
/// machine up 500 days; its PART and LINKs carry that mono, the recovery LINK does not.
#[test]
fn a_recovery_link_after_a_reboot_is_not_judged_on_the_dead_boots_clock() {
    let dir = link_wall_after_down("tl-reboot", |w| w);
    const UP: u64 = 500 * 86_400 * 1_000_000_000;
    mutate(&dir, None, |recs| {
        let down = find(recs, is_down);
        for r in &mut recs[..down] {
            match r {
                Record::Part { mono_ns, .. } | Record::Link { mono_ns, .. } => *mono_ns += UP,
                _ => {}
            }
        }
    });
    let (_, v) = check(&dir, &[]);
    assert_eq!(result(&v, "wall_sane"), "pass");
}
