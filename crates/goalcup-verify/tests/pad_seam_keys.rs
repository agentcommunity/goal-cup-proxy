//! The test pad (`scripts/pad/check.py`) reads the verifier's JSON through a table of key paths. A
//! renamed key would make the pad read nothing and say nothing, so every verifier path in that
//! table (S6 `check`, S7 `usage`, its threads and its call rows) must exist in real output on the
//! committed K2 record, read the way the pad reads it.

mod common;

use serde_json::Value;
use std::path::Path;

/// The verifier's keys in the table: `check` (s6_), `usage` (s7_), a usage thread (t_), a call row (c_).
const PREFIXES: [&str; 4] = ["s6_", "s7_", "t_", "c_"];

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
        .filter(|c| c.len() == 4 && PREFIXES.iter().any(|p| c[0].starts_with(p)))
        .map(|c| (c[0], c[2]))
        .collect();
    let want: usize = PREFIXES
        .iter()
        .map(|p| seam.matches(&format!("\"{p}")).count())
        .sum();
    assert_eq!(pairs.len(), want, "the SEAM scan dropped pairs");
    assert!(pairs.len() >= 20, "only {} seam pairs found", pairs.len());

    // As check.py runs them: `check --json`, and `usage --json --out <dir>`, whose files it reads.
    let entry = root.join("scripts/pad/fixtures/k2-part");
    let (_, check) = common::check(&entry, &[]);
    let out = tempfile::tempdir().unwrap();
    let dir = out.path().to_str().unwrap();
    common::run(&["usage", entry.to_str().unwrap(), "--json", "--out", dir]);
    let read = |ext: &str| {
        let files = std::fs::read_dir(out.path()).unwrap();
        let p = files
            .map(|f| f.unwrap().path())
            .find(|p| p.to_str().unwrap().ends_with(ext));
        std::fs::read_to_string(p.unwrap_or_else(|| panic!("usage --out wrote no *{ext}"))).unwrap()
    };
    let usage: Value = serde_json::from_str(&read(".json")).unwrap();
    let calls: Vec<Value> = read(".calls.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let entries = check["entries"].as_array().unwrap();
    let threads = usage["threads"].as_array().unwrap();
    assert!(!entries.is_empty() && !threads.is_empty() && !calls.is_empty());

    let mut missing = vec![];
    for (key, path) in pairs {
        if ABSENT_OK.iter().any(|(k, _)| *k == key) {
            continue;
        }
        let ok = match &key[..2] {
            _ if key == "s6_exit" || key == "s6_entries" => has(&check, path),
            "s6" => entries.iter().all(|e| has(e, path)),
            "s7" => has(&usage, path),
            "t_" => threads.iter().all(|t| has(t, path)),
            _ => calls.iter().all(|c| has(c, path)),
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
