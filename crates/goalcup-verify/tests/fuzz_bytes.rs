//! `goalcup-verify check` on bytes no recorder wrote (PRD-17 VR3, L-VR3: a crafted record must
//! not panic): 200 seeds each of random bytes, a clean part file with one compressed byte
//! flipped, and one with a raw (decompressed) byte flipped, each written as the entry's only
//! part file. Every case exits 2 or 3 (3 never from a panic), or 1 when it reads as a crashed
//! entry that is not sealed; never 0 or a signal, and within 10 s.

mod common;

use common::*;
use goalcup_format::part_name;
use goalcup_format::read::part_files;
use std::fs::{self, File};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SEEDS: u64 = 200;
const LIMIT: Duration = Duration::from_secs(10);

/// xorshift64, as recovery.rs's truncation test uses.
fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut s = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    }
}

/// The bytes of a clean, sealed entry's single part file.
fn clean_part(name: &str) -> Vec<u8> {
    let parts = part_files(&clean(name).join("rec")).unwrap();
    assert_eq!(parts.len(), 1);
    fs::read(&parts[0]).unwrap()
}

/// Writes `bytes` as an entry's only part file and runs `check` on it: the exit must be 2, or 3
/// without a panic, inside [`LIMIT`].
fn judge(what: &str, seed: u64, bytes: &[u8]) {
    let root = tmp(&format!("fuzz-{what}-{seed}"));
    let rec = root.join("e0").join("rec");
    fs::create_dir_all(&rec).unwrap();
    fs::write(rec.join(part_name(0)), bytes).unwrap();
    let err = root.join("stderr");
    let mut child = Command::new(env!("CARGO_BIN_EXE_goalcup-verify"))
        .arg("check")
        .arg(root.join("e0"))
        .stdout(Stdio::null())
        .stderr(File::create(&err).unwrap())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if started.elapsed() > LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{what} seed {seed}: check ran over {LIMIT:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let text = fs::read_to_string(&err).unwrap_or_default();
    // A flip that ends the stream in an open frame after a LINK reads as a crashed, unsealed
    // entry (the index is not read): exit 1, the missing seal named.
    let crashed = status.code() == Some(1) && text.contains("The entry is not sealed.");
    assert!(
        matches!(status.code(), Some(2 | 3)) || crashed,
        "{what} seed {seed}: exit {status}\n{text}"
    );
    assert!(
        !text.contains("internal error"),
        "{what} seed {seed}: panicked\n{text}"
    );
    // A failing case's folder stays for a look.
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn random_bytes_are_refused() {
    for seed in 0..SEEDS {
        let mut r = rng(seed);
        let len = (r() % 4096) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| r() as u8).collect();
        judge("random", seed, &bytes);
    }
}

#[test]
fn a_flipped_compressed_byte_is_refused() {
    let part = clean_part("fuzz-clean-z");
    for seed in 0..SEEDS {
        let mut r = rng(seed);
        let mut b = part.clone();
        let i = (r() % b.len() as u64) as usize;
        b[i] ^= (r() % 255 + 1) as u8;
        judge("zst", seed, &b);
    }
}

#[test]
fn a_flipped_raw_byte_is_refused() {
    let raw = zstd::decode_all(&clean_part("fuzz-clean-raw")[..]).unwrap();
    for seed in 0..SEEDS {
        let mut r = rng(seed);
        let mut b = raw.clone();
        let i = (r() % b.len() as u64) as usize;
        b[i] ^= (r() % 255 + 1) as u8;
        judge("raw", seed, &zstd::encode_all(&b[..], 3).unwrap());
    }
}
