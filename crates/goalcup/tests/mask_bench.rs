//! A5, by hand (PRD-05 section 11, PRD-14): masking speed and start-up cost, printed for the
//! night log. One `#[ignore]` test in no gate tier, with no floor; it never fails. Every speed
//! comes with the `fail_closed` bytes of the run and whether its output equals `mask_whole`: a
//! stream that fails closed skips work, so its speed alone says nothing. Run it in a release
//! build:
//!
//! `cargo test --release -p goalcup --test mask_bench -- --ignored --nocapture by_hand_mask_bench`
//!
//! One test runs in every tier, `exact_values_mask_above_a_floor`: in a release build it
//! checks that 32 exact values of 256 bytes are masked in every planted form, that the stream
//! equals `mask_whole`, and that start-up and one stream stay above a conservative floor (the
//! single-pass matcher of the owner decision of 2026-11-01). A debug build skips it and says
//! so.

#[allow(dead_code)]
#[path = "mask_corpus/common.rs"]
mod common;
#[allow(dead_code)]
#[path = "mask_corpus/synthetic.rs"]
mod synthetic;

use base64::Engine as _;
use goalcup::mask::{Masker, SecretValue, ValueSource};
use goalcup_format::StreamKind;
use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

/// One stream fed `input` in `piece`-byte pieces: MB/s (best of passes over at least a
/// second), then its `fail_closed` bytes and whether its output equals `mask_whole`, as a line.
fn speed(m: &Masker, kind: StreamKind, input: &[u8], piece: usize) -> String {
    let mut best = Duration::MAX;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(1) || best == Duration::MAX {
        let t = Instant::now();
        let mut s = m.stream(kind);
        for p in input.chunks(piece) {
            black_box(s.push(p));
        }
        black_box(s.finish());
        best = best.min(t.elapsed());
    }
    let cuts: Vec<usize> = (1..input.len().div_ceil(piece))
        .map(|i| i * piece)
        .collect();
    let out = common::stream_in(m, kind, input, &cuts);
    let fail_closed = m.mask_rules().len() - 1;
    let starred: usize = out
        .spans
        .iter()
        .filter(|s| s.rule == fail_closed)
        .map(|s| s.len)
        .sum();
    format!(
        "{:.1} MB/s, fail_closed {starred} bytes, equals mask_whole: {}",
        input.len() as f64 / 1e6 / best.as_secs_f64(),
        out == m.mask_whole(input)
    )
}

/// A line of `/proc/self/status` in kB, when the file is there.
fn status_kb(key: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with(key))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[test]
#[ignore = "by hand: release build, see the module comment"]
fn by_hand_mask_bench() {
    let Ok(m) = Masker::new(&[], Vec::new()) else {
        println!("mask_bench: the masker did not build");
        return;
    };
    let req = synthetic::codex_request();
    println!(
        "mask_bench: generated Codex request {} bytes in 16 KiB pieces: {}",
        req.bytes.len(),
        speed(&m, StreamKind::HttpReq, &req.bytes, 16 * 1024)
    );

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mask_corpus/claude-code-cli");
    let largest = std::fs::read_dir(&dir).ok().and_then(|entries| {
        entries
            .filter_map(Result::ok)
            .filter_map(|e| Some((e.metadata().ok()?.len(), e.path())))
            .max()
    });
    let transcript = largest.and_then(|(_, p)| Some((std::fs::read(&p).ok()?, p)));
    match &transcript {
        Some((bytes, p)) => println!(
            "mask_bench: {} ({} bytes) in 16 KiB pieces: {}",
            p.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            bytes.len(),
            speed(&m, StreamKind::Native, bytes, 16 * 1024)
        ),
        None => println!("mask_bench: no transcript under {}", dir.display()),
    }

    // `BEGIN` lines with no `END` keep a candidate live for most of the hold (review R3-F1).
    for gap in [300, 512, 1024, 2048, 4096, 15_000] {
        let line = format!("{}\n", "-----BEGIN PRIVATE KEY-----");
        let mut text = String::new();
        while text.len() < 1 << 20 {
            text.push_str(&line);
            text.push_str(&"x".repeat(gap - line.len()));
        }
        println!(
            "mask_bench: 1 MiB of BEGIN lines {gap} B apart, no END, in 16 KiB pieces: {}",
            speed(&m, StreamKind::Native, text.as_bytes(), 16 * 1024)
        );
    }

    let mut rng = common::Rng(0xBE7C);
    let mut values = || -> Vec<SecretValue> {
        (0..32)
            .map(|_| {
                SecretValue::new(
                    ValueSource::Workspace,
                    rng.chars(common::ALNUM, 256).into_bytes(),
                )
            })
            .collect()
    };
    let rss_before = status_kb("VmRSS:");
    let mut best = Duration::MAX;
    let mut built = None;
    for _ in 0..3 {
        let v = values();
        let t = Instant::now();
        let m = Masker::new(&[], v);
        best = best.min(t.elapsed());
        built = m.ok();
    }
    let rss_after = status_kb("VmRSS:");
    let counts = built.as_ref().map(Masker::value_counts);
    println!(
        "mask_bench: Masker::new with 32 values of 256 bytes: {:.1} ms (best of 3), counts {counts:?}",
        best.as_secs_f64() * 1e3
    );
    if let Some(m) = &built {
        let req_speed = speed(m, StreamKind::HttpReq, &req.bytes, 16 * 1024);
        println!("mask_bench: generated request with those 32 values: {req_speed}");
        if let Some((bytes, _)) = &transcript {
            let native = speed(m, StreamKind::Native, bytes, 16 * 1024);
            println!("mask_bench: the transcript with those 32 values: {native}");
        }
    }
    println!(
        "mask_bench: RSS before {rss_before:?} kB, after {rss_after:?} kB, peak (VmHWM) {:?} kB",
        status_kb("VmHWM:")
    );
}

/// Every byte of `value` percent-encoded (upper hex) or `\u`-escaped (lower hex).
fn encoded(value: &[u8], percent: bool) -> Vec<u8> {
    value
        .iter()
        .flat_map(|b| {
            if percent {
                format!("%{b:02X}").into_bytes()
            } else {
                format!("\\u{b:04x}").into_bytes()
            }
        })
        .collect()
}

/// About 4 MB of run-1-like input (the generated Codex request, then corpus files) with
/// planted lines between its parts, each holding a value in one form. Returns the input and,
/// per planted line, the range that must leave starred.
fn planted_input(values: &[Vec<u8>]) -> (Vec<u8>, Vec<std::ops::Range<usize>>) {
    let mut input = synthetic::codex_request().bytes;
    input.push(b'\n');
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mask_corpus");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .flat_map(|dir| {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
        })
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    let mut must = Vec::new();
    let mut plant = |input: &mut Vec<u8>, i: usize| {
        let v = &values[i % values.len()];
        let mut line = |input: &mut Vec<u8>, form: &[u8], keep: std::ops::Range<usize>| {
            input.extend_from_slice(b"{\"type\":\"tool_output\",\"text\":\"cat .env: ");
            let at = input.len();
            input.extend_from_slice(form);
            must.push(at + keep.start..at + keep.end);
            input.extend_from_slice(b" done\"}\n");
        };
        line(input, v, 0..v.len());
        let percent = encoded(v, true);
        line(input, &percent, 0..percent.len());
        let escaped = encoded(v, false);
        line(input, &escaped, 0..escaped.len());
        // An escaped break is never taken right before a backslash of the form.
        let mut wrapped = v[..100].to_vec();
        wrapped.extend_from_slice(if v[100] == b'\\' { b"\r\n" } else { b"\\n" });
        wrapped.extend_from_slice(&v[100..]);
        line(input, &wrapped, 0..wrapped.len());
        let mut inside = b"prefix-".to_vec();
        inside.extend_from_slice(v);
        inside.extend_from_slice(b"-suffix");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&inside);
        line(input, b64.as_bytes(), 20..330);
    };
    let mut i = 0;
    for path in files {
        if input.len() >= 4_000_000 {
            break;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            input.extend_from_slice(&bytes);
            if !input.ends_with(b"\n") {
                input.push(b'\n');
            }
            plant(&mut input, i);
            i += 1;
        }
    }
    (input, must)
}

#[test]
fn exact_values_mask_above_a_floor() {
    if cfg!(debug_assertions) {
        println!("exact_values_mask_above_a_floor: skipped: debug build");
        return;
    }
    // 32 distinct usable values of 256 bytes, generated here: never a real secret.
    let mut rng = common::Rng(0xF100_0256);
    let values: Vec<Vec<u8>> = (0..32)
        .map(|i| {
            let set: &[u8] = if i % 4 == 0 {
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789\"\\<&"
            } else {
                common::ALNUM
            };
            rng.chars(set, 256).into_bytes()
        })
        .collect();
    let given: Vec<SecretValue> = values
        .iter()
        .map(|v| SecretValue::new(ValueSource::Workspace, v.clone()))
        .collect();
    let t = Instant::now();
    let m = Masker::new(&[], given).expect("the masker builds");
    let built = t.elapsed();
    assert_eq!(m.value_counts().workspace, 32, "every value is used");
    println!(
        "exact_values_mask_above_a_floor: Masker::new with 32 values of 256 bytes: {:.1} ms",
        built.as_secs_f64() * 1e3
    );
    assert!(
        built < Duration::from_secs(1),
        "Masker::new took 1 s or more"
    );

    let (input, must) = planted_input(&values);
    assert!(input.len() > 3_000_000, "the input is about 4 MB");
    let piece = 16 * 1024;
    let cuts: Vec<usize> = (1..input.len().div_ceil(piece))
        .map(|i| i * piece)
        .collect();
    let out = common::stream_in(&m, StreamKind::HttpReq, &input, &cuts);
    for v in &values {
        assert!(
            !out.bytes.windows(v.len()).any(|w| w == v.as_slice()),
            "a planted value left raw"
        );
    }
    for r in &must {
        assert!(
            common::starred(&out, r.clone()),
            "a planted form left raw at {r:?}"
        );
    }
    assert!(
        out == m.mask_whole(&input),
        "the stream differs from mask_whole"
    );

    let mut best = Duration::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        let mut s = m.stream(StreamKind::HttpReq);
        for p in input.chunks(piece) {
            black_box(s.push(p));
        }
        black_box(s.finish());
        best = best.min(t.elapsed());
    }
    let rate = input.len() as f64 / 1e6 / best.as_secs_f64();
    println!(
        "exact_values_mask_above_a_floor: {} bytes, {} planted forms, one stream in 16 KiB pieces: {rate:.1} MB/s (best of 3), peak (VmHWM) {:?} kB",
        input.len(),
        must.len(),
        status_kb("VmHWM:")
    );
    assert!(rate > 20.0, "one stream masked below 20 MB/s");
}
