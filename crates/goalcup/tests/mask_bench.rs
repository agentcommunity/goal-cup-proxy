//! A5, by hand (PRD-05 section 11, PRD-14): masking speed and start-up cost, printed for the
//! night log. One `#[ignore]` test in no gate tier, with no floor; it never fails. Every speed
//! comes with the `fail_closed` bytes of the run and whether its output equals `mask_whole`: a
//! stream that fails closed skips work, so its speed alone says nothing. Run it in a release
//! build:
//!
//! `cargo test --release -p goalcup --test mask_bench -- --ignored --nocapture by_hand_mask_bench`

#[allow(dead_code)]
#[path = "mask_corpus/common.rs"]
mod common;
#[allow(dead_code)]
#[path = "mask_corpus/synthetic.rs"]
mod synthetic;

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
