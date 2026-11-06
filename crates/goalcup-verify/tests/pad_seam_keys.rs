//! The test pad (`scripts/pad/check.py`) reads the verifier's JSON through a table of key paths. A
//! renamed key would make the pad read nothing and say nothing, so every S6 and S7 path in that
//! table must exist in real output: `check --json` and `usage --json` on the committed K2 record.

mod common;

use serde_json::Value;
use std::path::Path;

/// (key, why) pairs that are legitimately absent on this record. Empty: K2 shows every one.
const ABSENT_OK: &[(&str, &str)] = &[];

fn has(doc: &Value, path: &str) -> bool {
    path.split('.').try_fold(doc, |d, k| d.get(k)).is_some()
}

#[test]
fn pad_seam_keys_exist_in_real_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let src = std::fs::read_to_string(root.join("scripts/pad/check.py")).unwrap();
    let seam = &src[src.find("SEAM = {").unwrap()..];
    let seam = &seam[seam.find("in {").unwrap()..];
    let seam = &seam[..seam.find(".items()").unwrap()];
    // Pairs look like `"s6_checks": "checks"`: after the first quote, every fourth piece starts one.
    let parts: Vec<&str> = seam.split('"').collect();
    let pairs: Vec<(&str, &str)> = parts[1..]
        .chunks(4)
        .filter(|c| c.len() == 4 && (c[0].starts_with("s6_") || c[0].starts_with("s7_")))
        .map(|c| (c[0], c[2]))
        .collect();
    let want = seam.matches("\"s6_").count() + seam.matches("\"s7_").count();
    assert_eq!(pairs.len(), want, "the SEAM scan dropped pairs");
    assert!(pairs.len() >= 20, "only {} seam pairs found", pairs.len());

    let entry = root.join("scripts/pad/fixtures/k2-part");
    let (_, check) = common::check(&entry, &[]);
    let usage = common::run(&["usage", entry.to_str().unwrap(), "--json"])
        .1
        .expect("usage printed no JSON");
    let entries = check["entries"].as_array().unwrap();
    assert!(!entries.is_empty(), "no entries in check output");

    let mut missing = vec![];
    for (key, path) in pairs {
        if ABSENT_OK.iter().any(|(k, _)| *k == key) {
            continue;
        }
        let ok = match key {
            "s6_exit" | "s6_entries" => has(&check, path),
            _ if key.starts_with("s6_") => entries.iter().all(|e| has(e, path)),
            _ => has(&usage, path),
        };
        if !ok {
            missing.push(format!("{key} = {path}"));
        }
    }
    assert!(
        missing.is_empty(),
        "pad seam keys missing from verifier output: {missing:#?}"
    );
}
