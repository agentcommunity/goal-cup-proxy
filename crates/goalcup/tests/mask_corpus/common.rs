//! Support shared by the masking tests (PRD-05), included with `#[path]`: a seeded generator,
//! fake keys built from pieces, and one stream fed in pieces. Every key here is fake.

use goalcup::mask::{Masked, Masker};
use goalcup_format::StreamKind;
use std::ops::Range;

/// A small xorshift generator: deterministic, no dependency.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn chars(&mut self, set: &[u8], n: usize) -> String {
        (0..n).map(|_| set[self.below(set.len())] as char).collect()
    }
}

pub const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// `n` characters of `set`, deterministic, never a real key.
pub fn run(set: &[u8], n: usize, seed: usize) -> String {
    (0..n)
        .map(|i| set[(i * 7 + seed * 3 + 3) % set.len()] as char)
        .collect()
}

/// One fake key per guarded literal: its kind, the key, and the length of its fixed prefix.
pub fn keys() -> Vec<(&'static str, String, usize)> {
    let upper = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let jwt = [
        "ey",
        "J",
        &run(ALNUM, 12, 6),
        ".",
        &run(ALNUM, 12, 7),
        ".",
        &run(ALNUM, 12, 8),
    ];
    vec![
        (
            "anthropic_key",
            ["sk-", "ant-", "api03-", &run(ALNUM, 30, 1)].concat(),
            3,
        ),
        ("openai_key", ["sk-", "proj-", &run(ALNUM, 30, 2)].concat(), 3),
        (
            "github_token",
            ["github", "_pat_", &run(ALNUM, 30, 3)].concat(),
            11,
        ),
        ("github_token", ["gh", "p_", &run(ALNUM, 30, 4)].concat(), 4),
        ("aws_access_key", ["AK", "IA", &run(upper, 16, 5)].concat(), 4),
        ("jwt", jwt.concat(), 3),
        ("slack_token", ["xo", "xb-", &run(ALNUM, 20, 9)].concat(), 5),
        ("google_key", ["AI", "za", &run(ALNUM, 35, 10)].concat(), 4),
        (
            "huggingface_token",
            ["h", "f_", &run(ALNUM, 30, 11)].concat(),
            3,
        ),
        ("npm_token", ["np", "m_", &run(ALNUM, 30, 12)].concat(), 4),
        ("xai_key", ["xa", "i-", &run(ALNUM, 30, 13)].concat(), 4),
        ("stripe_key", ["sk", "_live_", &run(ALNUM, 24, 14)].concat(), 8),
        ("gitlab_token", ["gl", "pat-", &run(ALNUM, 24, 15)].concat(), 6),
        (
            "digitalocean_token",
            ["do", "p_v1_", &run(b"0123456789abcdef", 40, 16)].concat(),
            7,
        ),
    ]
}

/// One stream of `kind` fed `input` cut at `cuts`, ended by `finish`, spans made absolute.
pub fn stream_in(m: &Masker, kind: StreamKind, input: &[u8], cuts: &[usize]) -> Masked {
    let mut s = m.stream(kind);
    let mut all = Masked::default();
    let mut at = 0;
    for &cut in cuts.iter().chain([input.len()].iter()) {
        let cut = cut.clamp(at, input.len());
        append(&mut all, s.push(&input[at..cut]));
        at = cut;
    }
    append(&mut all, s.finish());
    all
}

/// `piece` after `all`, its spans made relative to `all`.
pub fn append(all: &mut Masked, piece: Masked) {
    let base = all.bytes.len();
    all.bytes.extend(piece.bytes);
    all.spans.extend(piece.spans.into_iter().map(|mut s| {
        s.start += base;
        s
    }));
}

pub fn random_cuts(rng: &mut Rng, len: usize, max: usize) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut at = 0;
    while at < len {
        at += 1 + rng.below(max);
        cuts.push(at.min(len));
    }
    cuts
}

/// The length is kept, every span is non-empty and inside the bytes, bytes inside spans are
/// `*` and bytes outside are the input's.
pub fn only_spans_changed(input: &[u8], out: &Masked) -> bool {
    let mut set = vec![false; input.len()];
    for s in &out.spans {
        if s.len == 0 || s.start + s.len > input.len() {
            return false;
        }
        set[s.start..s.start + s.len].fill(true);
    }
    out.bytes.len() == input.len()
        && (0..input.len()).all(|i| out.bytes[i] == if set[i] { b'*' } else { input[i] })
}

/// Every byte of `r` in `out` is `*`.
pub fn starred(out: &Masked, r: Range<usize>) -> bool {
    out.bytes.get(r).is_some_and(|b| b.iter().all(|&b| b == b'*'))
}
