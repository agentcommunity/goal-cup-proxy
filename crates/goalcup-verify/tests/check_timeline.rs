//! The `timeline` report (PRD-08 section 3, "Reports that never fail"): buckets of the built
//! entry add up to the checks' facts and the recount; an entry with nothing in it has none.

mod common;

use common::run1::{Shape, run1};
use common::{Entry, check, item};
use goalcup_format::EndReason;
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
