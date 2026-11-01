//! The `timeline` report (PRD-08 section 3, "Reports that never fail"): buckets of the built
//! entry add up to the checks' facts and the recount; an entry with nothing in it has none.

mod common;

use common::run1::{Shape, run1};
use common::{Entry, check, item, mutate};
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
