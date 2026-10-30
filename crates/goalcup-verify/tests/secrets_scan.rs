//! The secret scan (PRD-08 section 3, seam S12): exit codes, hit lists and MASK counts only.

mod common;

use base64::{Engine, engine::general_purpose::STANDARD};
use common::*;
use goalcup_format::StreamKind::{HttpReq, HttpResp, Native};
use goalcup_format::{EndReason, GapReason, Record};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A planted value of no secret shape; `/` and `+` give it a percent form.
fn known() -> String {
    format!("zq{}/{}+{}", "Vx".repeat(6), "Wy".repeat(6), "Uz".repeat(6))
}

/// An entry with one stream (sid 1, an SSE response or a request) of `pieces`, then MASKs of
/// (off, len, rule); rule 0 is `bearer`, 2 `fail_closed`.
fn entry(name: &str, sse: bool, pieces: &[Vec<u8>], masks: &[[u64; 3]]) -> PathBuf {
    let mut e = Entry::new(name);
    let meta = json!({"status": 200, "headers": [["content-type", "text/event-stream"]]});
    let (sid, mut at) = (e.open([HttpReq, HttpResp][usize::from(sse)], 0, 0, meta), 0);
    for p in pieces {
        e.data(sid, at, 0, p);
        at += p.len() as u64;
    }
    for &[off, len, rule] in masks {
        e.put(Record::Mask {
            sid,
            off,
            len,
            rule,
        });
    }
    e.end(sid, EndReason::Complete, at, Value::Null);
    e.seal()
}

/// `secrets <paths> --known <file>` fed `stdin`, never with `--json`: stdout is still the report.
fn scan(paths: &[&Path], stdin: &[u8]) -> (i32, Value) {
    let k = tmp("known").join("known");
    std::fs::write(&k, known() + "\r\n\n").unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_goalcup-verify"));
    let c = c.arg("secrets").args(paths).arg("--known").arg(&k);
    let (i, o) = (Stdio::piped(), Stdio::piped());
    let mut c = c.stdin(i).stdout(o).spawn().unwrap();
    let _ = std::io::Write::write_all(&mut c.stdin.take().unwrap(), stdin);
    let out = c.wait_with_output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).expect("the report on stdout");
    assert!(v["masks"].is_object() && v["known_values"] == 1, "{v}");
    (out.status.code().unwrap_or(-1), v)
}

/// The exit, the lengths of `known_hits`, `shape_hits`, `glued_shape` and `big_masks`, the report.
fn lens(name: &str, pieces: &[Vec<u8>], masks: &[[u64; 3]]) -> (i32, [Option<usize>; 4], Value) {
    let (code, v) = scan(&[&entry(name, false, pieces, masks)], b"");
    let lists = ["known_hits", "shape_hits", "glued_shape", "big_masks"];
    (code, lists.map(|l| v[l].as_array().map(Vec::len)), v)
}

/// One known hit of `form`, counted once, in the stream (or its joined deltas).
fn found(name: &str, sse: bool, pieces: &[Vec<u8>], form: &str) {
    let (code, v) = scan(&[&entry(name, sse, pieces, &[])], b"");
    let h = &v["known_hits"];
    let got = (code, h.as_array().map(Vec::len), &h[0]["form"], &h[0]["n"]);
    assert_eq!(got, (2, Some(1), &json!(form), &json!(1)), "{name}: {v}");
    let end = [".1", ".1/deltas"][usize::from(sse)];
    let at = h[0]["in"].as_str().unwrap_or("");
    assert!(at.ends_with(end), "{name}: {v}");
}

#[test]
fn secrets_known_value_found_split_base64_percent_and_in_sse_deltas() {
    let k = known();
    let b64 = |pad: usize| STANDARD.encode([&b"ab"[..pad], k.as_bytes(), b"!"].concat());
    let d = |p: &str| format!("data: {}\n\n", json!({"type": "x.delta", "delta": p}));
    let cc = |p: &str| {
        format!(
            "data: {}\n\n",
            json!({"choices": [{"delta": {"content": p}}]})
        )
    };
    let (one, two) = k.as_bytes().split_at(9);
    found("split", false, &[one.to_vec(), two.to_vec()], "raw");
    for (pad, form) in ["base64_0", "base64_1", "base64_2"].into_iter().enumerate() {
        found(form, false, &[b64(pad).into_bytes()], form);
    }
    let pct = k.replace('/', "%2F").replace('+', "%2B");
    found("percent", false, &[pct.into_bytes()], "percent");
    let (a, b, c) = (&k[..8], &k[8..20], &k[20..]);
    found("sse", true, &[d(a).into(), d(b).into(), d(c).into()], "raw");
    found(
        "chat",
        true,
        &[cc(a).into(), cc(b).into(), cc(c).into()],
        "raw",
    );
}

#[test]
fn secrets_masked_glued_spaced_and_big_spans() {
    let (n, shape) = (known().len() as u64, format!("sk-{}", "Ab1".repeat(8)));
    let stars = |n: u64| "*".repeat(n as usize).into_bytes();
    let (code, got, v) = lens("starred", &[b"k=".to_vec(), stars(n)], &[[2, n, 0]]);
    assert_eq!(
        (code, got, &v["masks"]),
        (0, [Some(0); 4], &json!({"bearer": 1}))
    );
    let (code, got, v) = lens("glued", &[format!("x{shape}").into()], &[]);
    assert_eq!(
        (code, got, &v["result"]),
        (0, [0, 0, 1, 0].map(Some), &json!("warn"))
    );
    assert_eq!(v["glued_shape"][0]["kind"], "openai_key");
    let (code, got, _) = lens("spaced", &[format!("x {shape}").into()], &[]);
    assert_eq!((code, got), (2, [0, 1, 0, 0].map(Some)));
    let halves = [[0, 10240, 2], [10240, 10240, 2]];
    let (code, got, v) = lens("closed", &[stars(10240), stars(10240)], &halves);
    assert_eq!((code, got), (0, [Some(0); 4]));
    assert_eq!(v["masks"], json!({"fail_closed": 1}));
    assert_eq!(v["mask_bytes"]["fail_closed"], 20480);
    let (code, got, v) = lens("big", &[stars(20480)], &[[0, 20480, 0]]);
    assert_eq!((code, got), (2, [0, 0, 0, 1].map(Some)));
    assert_eq!(v["masks"], json!({"bearer": 1}));
}

#[test]
fn secrets_known_value_found_in_a_raw_file_and_on_stdin() {
    let (log, k) = (tmp("raw").join("harness.log"), known());
    std::fs::write(&log, format!("one\ntwo {k} three\n")).unwrap();
    for (path, stdin) in [(log.as_path(), &b""[..]), (Path::new("-"), k.as_bytes())] {
        let (code, v) = scan(&[path], stdin);
        let hits = v["known_hits"].as_array().map(Vec::len);
        assert_eq!((code, hits), (2, Some(1)));
        assert_eq!(v["inputs"][0]["as"], "raw");
    }
    // A run folder: the entry's record, its loose files and the run's are all read, not the
    // recorder's own `rec/` (the run CA key); a symlink loop leaves the input partial.
    let e = entry("run", false, &[b"clean".to_vec()], &[]);
    let run = e.parent().unwrap();
    std::fs::write(e.join("rec/run-ca.key"), &k).unwrap();
    assert_eq!(scan(&[&e.join("rec")], b"").0, 0);
    std::fs::write(e.join("goalcup.log"), &k).unwrap();
    std::fs::write(run.join("notes.txt"), &k).unwrap();
    std::os::unix::fs::symlink(".", run.join("self")).unwrap();
    let (code, v) = scan(&[run], b"");
    let hits = v["known_hits"].as_array().map(Vec::len);
    assert_eq!(
        (code, hits, &v["inputs"][0]["as"]),
        (2, Some(2), &json!("record")),
        "{v}"
    );
    assert_eq!(v["inputs"][0]["read"], "partial");
}

#[test]
fn secrets_check_known_adds_the_secrets_check() {
    let (path, k) = (tmp("check-known").join("known"), known());
    std::fs::write(&path, &k).unwrap();
    let mut e = Entry::new("check-dirty");
    e.standard();
    let meta = json!({"path": "fake:notes.txt", "gen": 0, "size_at_open": 0, "from": 0,
        "fid": "1a2c"});
    let s = e.open(Native, 0, 0, meta);
    e.data(s, 0, 0, k.as_bytes());
    e.end(s, EndReason::Complete, k.len() as u64, Value::Null);
    let args = ["--known", path.to_str().unwrap()];
    let (code, v) = check(&e.seal(), &args);
    assert_eq!((code, ids_with(&v, "fail")), (2, vec!["secrets".into()]));
    let (code, v) = check(&clean("check-clean"), &args);
    assert_eq!((code, result(&v, "secrets")), (0, "pass"));
    let (_, v) = check(&clean("check-none"), &[]);
    assert!(!v.to_string().contains(r#""id":"secrets""#));
}

#[test]
fn secrets_nothing_is_joined_across_a_gap() {
    let (k, shape) = (known(), format!("sk-{}", "Ab1".repeat(8)));
    let (a, b) = k.split_at(9);
    for (one, two, want, shapes) in [(a, b, 0, 0), ("abc", shape.as_str(), 2, 1)] {
        let mut e = Entry::new(&format!("gap-{want}"));
        let (sid, at) = (e.open(HttpReq, 0, 0, json!({})), one.len() as u64);
        e.data(sid, 0, 0, one.as_bytes());
        e.gap(sid, GapReason::MemoryGuard, 0, at, 102, Value::Null);
        e.data(sid, at + 101, 0, two.as_bytes());
        e.end(
            sid,
            EndReason::Complete,
            at + 101 + two.len() as u64,
            Value::Null,
        );
        let (code, v) = scan(&[&e.seal()], b"");
        let got = [&v["known_hits"], &v["shape_hits"]].map(|l| l.as_array().map(Vec::len));
        assert_eq!((code, got), (want, [Some(0), Some(shapes)]), "{v}");
    }
}
