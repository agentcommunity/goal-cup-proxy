//! A5, by hand (PRD-05 section 11, PRD-14): masking speed and start-up cost, printed for the
//! night log. One `#[ignore]` test in no gate tier, with no floor; it never fails. Run it in a
//! release build:
//!
//! `cargo test --release -p goalcup --test mask_bench -- --ignored --nocapture by_hand_mask_bench`

#[path = "mask_corpus/synthetic.rs"]
mod synthetic;

use goalcup::mask::{Masker, SecretValue, ValueSource};
use goalcup_format::StreamKind;
use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

/// MB/s of one stream fed `input` in `piece`-byte pieces, best of passes over at least a second.
fn speed(m: &Masker, kind: StreamKind, input: &[u8], piece: usize) -> f64 {
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
    input.len() as f64 / 1e6 / best.as_secs_f64()
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
        "mask_bench: generated Codex request {} bytes in 16 KiB pieces: {:.1} MB/s",
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
    match largest.and_then(|(_, p)| Some((std::fs::read(&p).ok()?, p))) {
        Some((bytes, p)) => println!(
            "mask_bench: {} ({} bytes) in 16 KiB pieces: {:.1} MB/s",
            p.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            bytes.len(),
            speed(&m, StreamKind::Native, &bytes, 16 * 1024)
        ),
        None => println!("mask_bench: no transcript under {}", dir.display()),
    }

    let mut rng = synthetic::Rng(0xBE7C);
    let mut values = || -> Vec<SecretValue> {
        (0..32)
            .map(|_| {
                SecretValue::new(
                    ValueSource::Env,
                    rng.chars(synthetic::ALNUM, 256).into_bytes(),
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
        println!("mask_bench: generated request with those 32 values: {req_speed:.1} MB/s");
    }
    println!(
        "mask_bench: RSS before {rss_before:?} kB, after {rss_after:?} kB, peak (VmHWM) {:?} kB",
        status_kb("VmHWM:")
    );
}
