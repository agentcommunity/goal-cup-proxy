//! The masker over the copied corpus and the synthetic inputs (PRD-05 A2, section 10; PRD-17
//! K4, K5, K10). The corpus is every capture under `tests/mask_corpus/` (the `synthetic/`
//! templates excepted); the copied fixtures' `<redacted:...>` markers are data and stay.

#[allow(dead_code)]
#[path = "mask_corpus/common.rs"]
mod common;
#[path = "mask_corpus/synthetic.rs"]
mod synthetic;

use common::{Rng, guarded_kinds, only_spans_changed, random_cuts, starred, stream_in};
use goalcup::connector::MaskRule;
use goalcup::mask::{Masked, Masker, Span};
use goalcup_format::StreamKind;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

type Res = Result<(), Box<dyn std::error::Error>>;
type Tally = BTreeMap<(String, String), usize>;

/// Spans the shared set adds to the corpus outside the old markers and outside Codex blobs:
/// (file, kind, count). Every entry must fire exactly so often, and nothing else may. The one
/// entry is today's known catch: a quoted JSON `apiKey` field the old script could not reach.
const KNOWN_CATCHES: &[(&str, &str, usize)] =
    &[("opencode/hooks.jsonl", "generic_secret_assignment", 1)];

/// Guarded-rule spans inside the generated Codex request's blobs right after a `-`, which the
/// default guard class `[A-Za-z0-9_]` lets through (PRD-00 owner question 10): (kind, count).
/// Counted and named, not a fail (A2).
const GENERATED_AFTER_DASH: &[(&str, usize)] = &[("huggingface_token", 1), ("openai_key", 2)];

/// The one masker these tests use: the shared rules, no connector rules, no exact values.
fn masker() -> Masker {
    Masker::new(&[], Vec::new()).expect("the built-in shapes compile")
}

fn corpus() -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).expect("the corpus directory") {
            let p = e.expect("a corpus entry").path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "synthetic") {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "jsonl") {
                out.push(p);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mask_corpus");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    assert_eq!(files.len(), 36, "the 36 copied captures");
    files
        .into_iter()
        .map(|p| {
            let name = p.strip_prefix(&root).expect("under the root");
            let name = name.to_string_lossy().replace('\\', "/");
            (name, std::fs::read(&p).expect("a readable capture"))
        })
        .collect()
}

/// Where the old script's `<redacted:...>` markers sit.
fn markers(b: &[u8]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = find(&b[i..], b"<redacted:").map(|o| o + i) {
        let end = b[off..]
            .iter()
            .position(|&c| c == b'>')
            .map_or(b.len(), |o| off + o + 1);
        out.push(off..end);
        i = end;
    }
    out
}

/// Every `encrypted_content` string interior, in plain or escaped JSON.
fn blobs(b: &[u8]) -> Vec<Range<usize>> {
    let key = b"encrypted_content";
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = find(&b[i..], key).map(|o| o + i) {
        let mut s = off + key.len();
        while s < b.len() && b"\\\": ".contains(&b[s]) {
            s += 1;
        }
        let mut e = s;
        while e < b.len() && (b[e].is_ascii_alphanumeric() || b"-_=".contains(&b[e])) {
            e += 1;
        }
        if e > s {
            out.push(s..e);
        }
        i = e.max(off + 1);
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn overlaps(s: &Span, r: &Range<usize>) -> bool {
    s.start < r.end && r.start < s.start + s.len
}

fn tally(list: &[(&str, &str, usize)]) -> Tally {
    list.iter()
        .map(|(f, k, n)| ((f.to_string(), k.to_string()), *n))
        .collect()
}

fn listing(t: &Tally) -> String {
    t.iter()
        .map(|((f, k), n)| format!("    ({f:?}, {k:?}, {n}),"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs `f` on every item, spread over the machine's threads; a panic in one fails the test.
fn par_each<T: Sync>(items: &[T], f: impl Fn(&T) + Sync) {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads.min(items.len()) {
            scope.spawn(|| {
                while let Some(item) =
                    items.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                {
                    f(item);
                }
            });
        }
    });
}

/// Known catches and K5: over every capture, `mask_whole` adds spans only where the list says;
/// every Codex blob comes through byte for byte.
#[test]
fn the_corpus_gets_spans_only_where_listed() {
    let m = masker();
    let names = m.mask_rules();
    let mut outside = Tally::new();
    let mut blob_count = 0;
    for (name, bytes) in corpus() {
        let whole = m.mask_whole(&bytes);
        assert!(only_spans_changed(&bytes, &whole), "{name}");
        let (marks, blobs) = (markers(&bytes), blobs(&bytes));
        blob_count += blobs.len();
        for s in &whole.spans {
            let kind = names[s.rule].clone();
            assert!(
                !blobs.iter().any(|b| overlaps(s, b)),
                "{name}: a {kind} span inside a Codex blob"
            );
            if !marks.iter().any(|r| overlaps(s, r)) {
                *outside.entry((name.clone(), kind)).or_default() += 1;
            }
        }
    }
    assert!(
        blob_count >= 37,
        "the Codex blobs went missing: {blob_count}"
    );
    assert!(
        outside == tally(KNOWN_CATCHES),
        "spans outside the listed catches; the corpus now gives:\n{}",
        listing(&outside)
    );
}

/// K1 over the corpus: seeded random chunkings, every split point within 64 bytes of each span
/// edge and, for files up to 16 KiB, one byte at a time give the bytes and spans of
/// `mask_whole`, with no `fail_closed` span. The files run on all threads, largest first.
#[test]
fn the_corpus_streams_as_the_whole_record_masker() {
    let m = masker();
    let fail_closed = m.mask_rules().len() - 1;
    let mut files = corpus();
    files.sort_by_key(|(_, b)| std::cmp::Reverse(b.len()));
    par_each(&files, |(name, bytes)| {
        let whole = m.mask_whole(bytes);
        assert!(whole.spans.iter().all(|s| s.rule != fail_closed), "{name}");
        for seed in [1u64, 2, 3] {
            let mut rng = Rng(0x9E37_79B9 ^ seed);
            let cuts = random_cuts(&mut rng, bytes.len(), 4096);
            assert!(
                stream_in(&m, StreamKind::Native, bytes, &cuts) == whole,
                "{name}: seed {seed}"
            );
        }
        if bytes.len() <= 16 * 1024 {
            let cuts: Vec<usize> = (1..bytes.len()).collect();
            assert!(
                stream_in(&m, StreamKind::Native, bytes, &cuts) == whole,
                "{name}: one byte at a time"
            );
        }
        edge_splits(&m, name, bytes, &whole);
    });
}

/// Each span's line (or lines) on its own, split at every point within 64 bytes of a span edge,
/// streams as `mask_whole` of that line.
fn edge_splits(m: &Masker, name: &str, bytes: &[u8], whole: &Masked) {
    let mut lines: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for s in &whole.spans {
        let ls = bytes[..s.start]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        let le = bytes[s.start + s.len..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |i| s.start + s.len + i + 1);
        lines
            .entry((ls, le))
            .or_default()
            .extend([s.start, s.start + s.len]);
    }
    for ((ls, le), edges) in lines {
        let line = &bytes[ls..le];
        let lw = m.mask_whole(line);
        assert_eq!(lw.bytes, &whole.bytes[ls..le], "{name}: the line alone");
        let mut points: Vec<usize> = edges
            .iter()
            .flat_map(|&e| e.saturating_sub(64).max(ls)..=(e + 64).min(le))
            .map(|p| p - ls)
            .collect();
        points.sort_unstable();
        points.dedup();
        for p in points {
            assert!(
                stream_in(m, StreamKind::Native, line, &[p]) == lw,
                "{name}: split at line offset {p}"
            );
        }
    }
}

/// K10: a harness's billing header text inside the system prompt body is masked by the
/// connector's header-prefix rule; the rest of the body stays readable and parses.
#[test]
fn the_billing_header_in_a_system_prompt_is_masked() -> Res {
    let rule: MaskRule = serde_json::from_value(serde_json::json!({
        "kind": "anthropic_billing_header",
        "header_prefix": "x-anthropic-billing-",
    }))?;
    let m = Masker::new(&[rule], Vec::new())?;
    let header = "x-anthropic-billing-header: cc_version=2.0.31.a1b; cc_entrypoint=cli;";
    let body = serde_json::json!({
        "model": "example-coder-large",
        "max_tokens": 32000,
        "system": [
            {"type": "text", "text": header},
            {"type": "text", "text": "You are a coding agent. Keep answers short."},
        ],
        "messages": [{"role": "user", "content": "run the tests"}],
        "stream": true,
    })
    .to_string();
    let input = body.as_bytes();
    let whole = m.mask_whole(input);
    let at = body.find(header).ok_or("the header in the body")?;
    let value = at + header.find(':').ok_or("a colon")? + 2;
    assert!(
        whole.bytes[value..at + header.len()]
            .iter()
            .all(|&b| b == b'*')
    );
    assert!(
        whole
            .spans
            .iter()
            .any(|s| m.mask_rules()[s.rule] == "anthropic_billing_header")
    );
    assert!(only_spans_changed(input, &whole));
    let out: serde_json::Value = serde_json::from_slice(&whole.bytes)?;
    assert_eq!(
        out["system"][1]["text"],
        "You are a coding agent. Keep answers short."
    );
    assert_eq!(out["model"], "example-coder-large");
    let mut rng = Rng(0x0B11);
    let cuts = random_cuts(&mut rng, input.len(), 17);
    assert!(stream_in(&m, StreamKind::HttpReq, input, &cuts) == whole);
    Ok(())
}

/// Section 10: every shaped planted value is masked, raw and inside a JSON tool result; the
/// AWS-style value with no shape is not (a stated loss of the default build, owner question 3).
#[test]
fn every_shaped_planted_value_is_masked() -> Res {
    let m = masker();
    let env = synthetic::planted_env();
    let escaped = serde_json::json!({
        "type": "tool_result",
        "tool_use_id": "toolu_synthetic",
        "content": std::str::from_utf8(&env.bytes)?,
    })
    .to_string();
    let mut rng = Rng(0x0E57);
    for (form, input) in [("raw", env.bytes.clone()), ("json", escaped.into_bytes())] {
        let whole = m.mask_whole(&input);
        assert!(only_spans_changed(&input, &whole), "{form}");
        let cuts = random_cuts(&mut rng, input.len(), 64);
        assert!(
            stream_in(&m, StreamKind::Native, &input, &cuts) == whole,
            "{form}"
        );
        for (name, value) in synthetic::planted_values() {
            if name == "plain_key" {
                continue; // not in the .env
            }
            let text = serde_json::to_string(&value)?;
            let text = if form == "raw" {
                value.clone()
            } else {
                text[1..text.len() - 1].to_string()
            };
            let at = find(&input, text.as_bytes()).ok_or(name)?;
            let r = at..at + text.len();
            if name == "aws_secret" {
                assert!(whole.spans.iter().all(|s| !overlaps(s, &r)), "{form}");
                assert_eq!(&whole.bytes[r], text.as_bytes(), "{form}: no shape, kept");
            } else {
                assert!(starred(&whole, r), "{form}: {name} is masked");
            }
        }
    }
    Ok(())
}

/// A2 and section 6 on the generated Codex request in 16 KiB pieces: byte for byte except the
/// listed guarded spans right after a `-` inside a blob, and no `fail_closed` span.
#[test]
fn the_generated_codex_request_keeps_its_blobs() {
    let m = masker();
    let names = m.mask_rules();
    let guarded = guarded_kinds();
    let req = synthetic::codex_request();
    assert!(req.bytes.len() >= 1_500_000 && req.dashed >= 3 && req.glued >= 50);
    let cuts: Vec<usize> = (1..=req.bytes.len() / 16384).map(|i| i * 16384).collect();
    let got = stream_in(&m, StreamKind::HttpReq, &req.bytes, &cuts);
    assert!(
        got == m.mask_whole(&req.bytes),
        "the stream equals the whole"
    );
    assert!(only_spans_changed(&req.bytes, &got));
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for s in &got.spans {
        let kind = &names[s.rule];
        assert!(
            guarded.contains(kind)
                && req.bytes[s.start - 1] == b'-'
                && req
                    .blobs
                    .iter()
                    .any(|b| b.start <= s.start && s.start + s.len <= b.end),
            "a {kind} span outside a blob or not right after a '-'"
        );
        *kinds.entry(kind.clone()).or_default() += 1;
    }
    let want: BTreeMap<String, usize> = GENERATED_AFTER_DASH
        .iter()
        .map(|(k, n)| (k.to_string(), *n))
        .collect();
    assert!(
        kinds == want,
        "the generated request now gives {kinds:?} ({} planted)",
        req.dashed
    );
}

/// The positions of `line` that are inside a JSON string (between its quotes, escapes counted).
fn string_interiors(line: &[u8]) -> Vec<bool> {
    let (mut inside, mut escaped) = (false, false);
    line.iter()
        .map(|&b| {
            let quote = b == b'"' && !escaped;
            escaped = b == b'\\' && !escaped;
            if quote {
                inside = !inside;
                false
            } else {
                inside
            }
        })
        .collect()
}

/// K4: a PEM key spread over SSE string deltas loses every byte of its text. Inside the block
/// every string interior is starred, keys included; the JSON between the deltas keeps its quotes,
/// braces, colons, commas and numbers and still parses; every line outside the block, and every
/// `event:` line, comes through byte for byte.
#[test]
fn a_pem_key_over_sse_deltas_loses_every_body_byte() -> Res {
    let m = masker();
    let sse = synthetic::pem_sse();
    let whole = m.mask_whole(&sse.bytes);
    assert!(only_spans_changed(&sse.bytes, &whole));
    let mut rng = Rng(0x55E);
    let cuts = random_cuts(&mut rng, sse.bytes.len(), 48);
    assert!(stream_in(&m, StreamKind::HttpResp, &sse.bytes, &cuts) == whole);
    let key: Vec<Range<usize>> = sse.at.iter().map(|(_, r)| r.clone()).collect();
    assert_eq!(key.len(), 18);
    for r in &key {
        assert!(starred(&whole, r.clone()), "a PEM piece survived");
    }
    let block = key[0].start..key[key.len() - 1].end;
    let (ins, outs) = (
        sse.bytes.split(|&b| b == b'\n'),
        whole.bytes.split(|&b| b == b'\n'),
    );
    let (mut at, mut inside_block) = (0, 0);
    for (a, b) in ins.zip(outs) {
        let line = at..at + a.len();
        at += a.len() + 1;
        let touched = block.start < line.end && line.start < block.end;
        if !touched || !a.starts_with(b"data: ") {
            assert_eq!(a, b, "a line outside the key or an event line changed");
            continue;
        }
        inside_block += 1;
        serde_json::from_slice::<serde_json::Value>(&b[6..])?;
        for (i, interior) in string_interiors(a).into_iter().enumerate() {
            assert!(interior || a[i] == b[i], "the JSON between deltas changed");
        }
    }
    assert_eq!(inside_block, 18, "every delta of the key was checked");
    Ok(())
}

/// A harness settings file: its keys are masked, its model name and base URLs stay readable.
#[test]
fn a_settings_file_keeps_its_model_and_base_url() -> Res {
    let m = masker();
    let settings = synthetic::harness_settings();
    let whole = m.mask_whole(&settings.bytes);
    for (_, r) in &settings.at {
        assert!(
            starred(&whole, r.clone()),
            "a key in the settings file survived"
        );
    }
    let out: serde_json::Value = serde_json::from_slice(&whole.bytes)?;
    assert_eq!(out["model"], "example-coder-large");
    assert_eq!(
        out["providers"]["gateway"]["models"][0],
        "example-coder-large"
    );
    assert_eq!(
        out["providers"]["gateway"]["baseURL"],
        "https://gateway.example.test/v1"
    );
    assert_eq!(
        out["env"]["OPENAI_BASE_URL"],
        "https://gateway.example.test/v1"
    );
    assert_eq!(
        out["providers"]["openai"]["base_url"],
        "https://api.example.test/v1"
    );
    Ok(())
}
