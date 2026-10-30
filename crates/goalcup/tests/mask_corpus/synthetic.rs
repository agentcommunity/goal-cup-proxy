//! The synthetic masking inputs (PRD-05 "Files it owns", the corpus row). Three are committed as
//! templates under `synthetic/` with double-brace placeholders; the 1.5 MB Codex-shaped request
//! is generated here from a fixed seed. Every value is fake and assembled from pieces in code,
//! so no committed file holds a contiguous shaped key. Included by `tests/mask_corpus.rs` and
//! `tests/mask_bench.rs` with `#[path]`; nothing else reads these inputs.

#![allow(dead_code)]

use std::ops::Range;

pub const PLANTED_ENV: &str = include_str!("synthetic/planted.env");
pub const PEM_SSE: &str = include_str!("synthetic/pem-sse.txt");
pub const HARNESS_SETTINGS: &str = include_str!("synthetic/harness-settings.json");

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
pub const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
pub const HEX: &[u8] = b"0123456789abcdef";
pub const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub const B64URL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// A template with every placeholder replaced, and where each replacement landed.
pub struct Filled {
    pub bytes: Vec<u8>,
    pub at: Vec<(String, Range<usize>)>,
}

impl Filled {
    /// The ranges every placeholder `name` was filled at.
    pub fn ranges(&self, name: &str) -> Vec<Range<usize>> {
        self.at
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, r)| r.clone())
            .collect()
    }
}

/// Replace every `{{name}}` in `template` by `value(name)`, called once per occurrence.
pub fn fill(template: &str, mut value: impl FnMut(&str) -> String) -> Filled {
    let mut bytes = Vec::with_capacity(template.len() * 2);
    let mut at = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find("{{") {
        bytes.extend_from_slice(&rest.as_bytes()[..i]);
        let after = &rest[i + 2..];
        let j = after.find("}}").expect("every placeholder is closed");
        let name = &after[..j];
        let v = value(name);
        at.push((name.to_string(), bytes.len()..bytes.len() + v.len()));
        bytes.extend_from_slice(v.as_bytes());
        rest = &after[j + 2..];
    }
    bytes.extend_from_slice(rest.as_bytes());
    Filled { bytes, at }
}

/// One PEM private key: the BEGIN line, `lines` body lines of 64 base64 bytes, the END line.
pub fn pem_parts(rng: &mut Rng, lines: usize) -> (String, Vec<String>, String) {
    let begin = ["-----BEG", "IN RSA PRIV", "ATE KEY-----"].concat();
    let end = ["-----EN", "D RSA PRIV", "ATE KEY-----"].concat();
    let body = (0..lines).map(|_| rng.chars(B64, 64)).collect();
    (begin, body, end)
}

/// The planted values of `planted.env` by placeholder name, each fake and built from pieces.
/// `aws_secret` is the one value with no shape.
pub fn planted_values() -> Vec<(&'static str, String)> {
    let mut r = Rng(0x0E57_AB1E);
    let (begin, body, end) = pem_parts(&mut r, 8);
    let pem = [begin, body.join("\n"), end].join("\n");
    let jwt = [
        "ey".to_string(),
        "J".into(),
        r.chars(ALNUM, 20),
        ".".into(),
        r.chars(ALNUM, 30),
        ".".into(),
        r.chars(ALNUM, 24),
    ]
    .concat();
    vec![
        ("openai", ["sk-", "proj-", &r.chars(ALNUM, 48)].concat()),
        (
            "anthropic",
            ["sk-", "ant-", "api03-", &r.chars(ALNUM, 48)].concat(),
        ),
        ("ghp", ["gh", "p_", &r.chars(ALNUM, 36)].concat()),
        (
            "github_pat",
            [
                "github",
                "_pat_",
                &r.chars(ALNUM, 22),
                "_",
                &r.chars(ALNUM, 59),
            ]
            .concat(),
        ),
        ("akia", ["AK", "IA", &r.chars(UPPER, 16)].concat()),
        ("aws_secret", r.chars(B64, 40)),
        ("aiza", ["AI", "za", &r.chars(ALNUM, 35)].concat()),
        ("hf", ["h", "f_", &r.chars(ALNUM, 34)].concat()),
        ("xai", ["xa", "i-", &r.chars(ALNUM, 48)].concat()),
        ("glpat", ["gl", "pat-", &r.chars(ALNUM, 20)].concat()),
        ("jwt", jwt),
        ("db_password", r.chars(ALNUM, 20)),
        ("api_key", r.chars(ALNUM, 32)),
        ("token", r.chars(ALNUM, 32)),
        ("gateway", r.chars(ALNUM, 40)),
        (
            "openrouter",
            ["sk-", "or-", "v1-", &r.chars(HEX, 64)].concat(),
        ),
        ("pem", pem),
        ("plain_key", r.chars(ALNUM, 32)),
    ]
}

/// The value planted under `name`.
pub fn planted(name: &str) -> String {
    planted_values()
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v)
        .expect("a planted value of that name")
}

/// `planted.env` filled.
pub fn planted_env() -> Filled {
    fill(PLANTED_ENV, planted)
}

/// `harness-settings.json` filled.
pub fn harness_settings() -> Filled {
    fill(HARNESS_SETTINGS, planted)
}

/// `pem-sse.txt` filled: one key, its text spread over many SSE string deltas.
pub fn pem_sse() -> Filled {
    let mut r = Rng(0x00C0_FFEE);
    let (begin, body, end) = pem_parts(&mut r, 16);
    let mut body = body.into_iter();
    fill(PEM_SSE, |name| match name {
        "pem_begin" => begin.clone(),
        "pem_end" => end.clone(),
        "pem_body" => body.next().expect("a body line per placeholder"),
        _ => panic!("unknown placeholder"),
    })
}

/// The generated Codex-shaped request.
pub struct CodexRequest {
    pub bytes: Vec<u8>,
    /// Every `encrypted_content` string interior.
    pub blobs: Vec<Range<usize>>,
    /// Planted guarded prefixes right after a `-`, of a rule that completes there.
    pub dashed: usize,
    /// Planted prefixes glued after a letter.
    pub glued: usize,
}

/// The seed of the generated request; a change makes a new input.
pub const CODEX_SEED: u64 = 0x5EED_C0DE_0001;

/// A 1.5 MB Codex-shaped request (one JSON line): user messages, reasoning items whose
/// `encrypted_content` is a base64url blob of 512 to 14,511 bytes, tool calls and their output.
/// Every third blob holds `eyJ`, `sk-` or `hf_` glued after a letter; every 40th, from the 8th,
/// holds one right after a `-`, near the blob's end. Blobs stay under 16 KiB, so no hold can
/// overflow.
pub fn codex_request() -> CodexRequest {
    let mut r = Rng(CODEX_SEED);
    let mut out = String::with_capacity(1_600_000);
    let mut blobs = Vec::new();
    let (mut dashed, mut glued) = (0, 0);
    out.push_str(
        r#"{"model":"example-coder-large","instructions":"You are a coding agent. Keep answers short.","input":["#,
    );
    let tokens = ["ey", "sk", "hf"];
    let (mut n, mut k) = (0usize, 0usize);
    while out.len() < 1_500_000 {
        if n > 0 {
            out.push(',');
        }
        match n % 4 {
            0 => out.push_str(&format!(
                r#"{{"type":"message","role":"user","content":[{{"type":"input_text","text":"Please run the tests in part {n} and fix what fails."}}]}}"#
            )),
            1 => {
                let len = 512 + r.below(14_000);
                let mut blob: Vec<u8> = b"gAAAAAB".to_vec();
                blob.extend(r.chars(B64URL, len).bytes());
                let tok = |i: usize| -> Vec<u8> {
                    match tokens[i % 3] {
                        "ey" => [b"ey".as_slice(), b"J"].concat(),
                        "sk" => [b"s".as_slice(), b"k-"].concat(),
                        _ => [b"h".as_slice(), b"f_"].concat(),
                    }
                };
                if k % 3 == 0 {
                    let p = 16 + r.below(len - 64);
                    blob[p - 1] = b'Q';
                    let t = tok(k / 3);
                    blob[p..p + 3].copy_from_slice(&t);
                    for b in &mut blob[p + 3..p + 27] {
                        *b = ALNUM[r.below(ALNUM.len())];
                    }
                    glued += 1;
                }
                if k % 40 == 7 {
                    let p = blob.len() - 100 - r.below(300);
                    blob[p - 1] = b'-';
                    let t = tok(k / 40);
                    blob[p..p + 3].copy_from_slice(&t);
                    for b in &mut blob[p + 3..p + 27] {
                        *b = ALNUM[r.below(ALNUM.len())];
                    }
                    if t != b"eyJ" {
                        dashed += 1;
                    }
                }
                blob.push(b'=');
                out.push_str(&format!(
                    r#"{{"type":"reasoning","id":"rs_{k:06}","summary":[{{"type":"summary_text","text":"Reading the failing test."}}],"content":null,"encrypted_content":""#
                ));
                let at = out.len();
                out.push_str(std::str::from_utf8(&blob).expect("ascii"));
                blobs.push(at..out.len());
                out.push_str(r#""}"#);
                k += 1;
            }
            2 => out.push_str(&format!(
                r#"{{"type":"function_call","name":"shell","arguments":"{{\"command\":[\"cargo\",\"test\",\"-p\",\"part{n}\"],\"workdir\":\"/work\"}}","call_id":"call_{n:06}"}}"#
            )),
            _ => out.push_str(&format!(
                r#"{{"type":"function_call_output","call_id":"call_{:06}","output":"running 12 tests\ntest result: ok. 12 passed; 0 failed\n"}}"#,
                n - 1
            )),
        }
        n += 1;
    }
    out.push_str(
        r#"],"tools":[],"tool_choice":"auto","parallel_tool_calls":false,"reasoning":{"effort":"medium","summary":"auto"},"store":false,"stream":true,"include":["reasoning.encrypted_content"]}"#,
    );
    CodexRequest {
        bytes: out.into_bytes(),
        blobs,
        dashed,
        glued,
    }
}
