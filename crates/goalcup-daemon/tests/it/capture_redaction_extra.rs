//! Connector `[redaction].extra_patterns` on the streaming redactor (D39).
//!
//! The independent oracle is the V1 whole-record redactor with the same extra patterns,
//! `Redactor::for_patterns(id, extra).apply(whole stream)`. Below the hold bound the
//! stream must equal it at every chunking; past the bound the stream may redact more,
//! never less; at a cut (interrupt, `End{Interrupted}`, `Gap`) it may not release a byte
//! the oracle redacts in the delivered prefix, nor a held connector value. The patterns
//! are the ones the repository's schema-1 harnesses ship, read from `connectors/*/harness.toml`,
//! and the inputs follow the shapes those patterns were written for: Codex `token_count`
//! events and Claude request bodies and headers. Every value below is synthetic.

use base64::{Engine, engine::general_purpose::STANDARD};
use goalcup_daemon::redact::stream::{MAX_HELD_BYTES, StreamRedactor};
use goalcup_daemon::redact::{Redactor, extra_patterns_of};
use goalcup_protocol::capture::*;
use goalcup_protocol::harness::{ExtraPattern, FieldValue, HarnessManifest, RedactionRule};
use goalcup_protocol::redaction::RedactionAuditSpan;

use crate::common::redaction;
use redaction::*;

// ── the shipped rules and the oracle ─────────────────────────────────────────────────

fn harness_patterns(connector: &str) -> Vec<ExtraPattern> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(connector)
        .join("harness.toml");
    let harness = HarnessManifest::from_toml_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    extra_patterns_of(&harness)
}

/// Codex's three rules, then Claude's billing header: one list, one order, used by both
/// the stream and the oracle.
fn shipped() -> Vec<ExtraPattern> {
    let mut all = harness_patterns("codex-cli");
    all.extend(harness_patterns("claude-code-cli"));
    all
}

const EXTRA_KINDS: [&str; 4] = [
    "codex_rate_limits",
    "codex_plan_type",
    "codex_credits_remaining",
    "anthropic_billing_header",
];

/// The whole-record redactor over exactly [`shipped`]: the independent oracle.
fn oracle() -> Redactor {
    Redactor::for_patterns("shipped", &shipped()).unwrap()
}

fn legacy(input: &[u8]) -> (Vec<u8>, Vec<RedactionAuditSpan>) {
    let r = oracle().apply(input);
    (r.bytes, r.spans)
}

fn redactor() -> StreamRedactor {
    StreamRedactor::with_extra_patterns(&shipped()).expect("the shipped patterns stream")
}

// ── one producer ─────────────────────────────────────────────────────────────────────

fn meta(label: &str) -> CaptureMeta {
    CaptureMeta {
        wire: Some(WireIdentity {
            connection: CONN.into(),
            exchange: 1,
            message_index: None,
        }),
        label: label.into(),
        protocol_hint: Some("anthropic-messages".into()),
        method: Some("POST".into()),
        host: Some("api.example.test".into()),
        path: Some("/v1/messages".into()),
        status: None,
        content_type: Some("application/json".into()),
    }
}

/// The retained bytes and audit spans, after checking that the segments are contiguous in
/// source offsets, numbered from zero, and that every span lies in its segment's range.
fn retained(records: &[CaptureRecord]) -> (Vec<u8>, Vec<RedactionAuditSpan>) {
    let mut bytes = Vec::new();
    let mut spans = Vec::new();
    let mut part = 0;
    let mut next: Option<u64> = None;
    for r in records {
        if let CaptureRecord::Segment {
            part: p,
            source_offset,
            source_len,
            data_b64,
            audit,
            ..
        } = r
        {
            assert_eq!(*p, part, "parts are numbered from zero without gaps");
            part += 1;
            if let Some(o) = next {
                assert!(*source_offset >= o, "segments never overlap");
            }
            next = Some(source_offset + source_len);
            for s in &audit.spans {
                assert!(
                    s.start >= *source_offset
                        && s.end <= source_offset + source_len
                        && s.start < s.end,
                    "audit span {s:?} lies inside its segment's source range"
                );
            }
            bytes.extend(STANDARD.decode(data_b64).unwrap());
            spans.extend(audit.spans.iter().cloned());
        }
    }
    (bytes, spans)
}

fn run(rng: &mut Rng, out: &mut Vec<u8>, class: &[u8], n: usize) {
    for _ in 0..n {
        out.push(class[rng.below(class.len())]);
    }
}

fn long(unit: &[u8], n: usize) -> Vec<u8> {
    unit.iter().copied().cycle().take(n).collect()
}

/// Whole copies of `unit`, at least `n` bytes: a run that stays balanced.
fn units(unit: &[u8], n: usize) -> Vec<u8> {
    unit.repeat(n.div_ceil(unit.len()))
}

/// The directional oracle past the hold: at three chunkings, every byte the oracle redacts
/// lies inside a stream audit span. The first failure, if any.
fn never_less(name: &str, input: &[u8], rng: &mut Rng) -> Option<String> {
    let (_, want) = legacy(input);
    for chunks in [
        vec![MAX_SEGMENT_BYTES],
        random_chunks(rng, 3000),
        vec![1, 7, 64],
    ] {
        let (_, got) = retained(&stream_through(redactor(), input, &chunks, 2));
        let missing = uncovered(&want, &got);
        if let Some((kind, s, e)) = missing.first() {
            return Some(format!(
                "{name}: {} oracle span(s) retained raw, first {kind} {s}..{e} {:?}",
                missing.len(),
                String::from_utf8_lossy(&input[*s as usize..(*e as usize).min(*s as usize + 40)])
            ));
        }
    }
    None
}

fn kinds_of(spans: &[RedactionAuditSpan]) -> std::collections::BTreeSet<String> {
    spans.iter().map(|s| s.kind.clone()).collect()
}

// ── realistic shapes ────────────────────────────────────────────────────────────────

const TOKEN_COUNT: &[u8] = b"{\"timestamp\":\"2026-09-28T10:00:00.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":1234,\"output_tokens\":56}},\"rate_limits\":{\"primary\":{\"used_percent\":12.5,\"window_minutes\":300,\"resets_in_seconds\":4821},\"secondary\":{\"used_percent\":3.0,\"window_minutes\":10080,\"resets_in_seconds\":512345},\"credits\":{\"has_credits\":true,\"unlimited\":false,\"balance\":\"412\"},\"plan_type\":\"plus\"}}}\n";
const SSE_EVENT: &[u8] = b"event: codex.rate_limits\ndata: {\"type\":\"codex.rate_limits\",\"plan_type\":\"pro\",\"rate_limits\":{\"primary\":{\"used_percent\":40.0}},\"credits_remaining\":17.5}\n\n";
const CREDITS_QUOTED: &[u8] = b"{\"credits_remaining\": \"412.00\", \"plan_type\": \"team\"}\n";
const HTTP_HEADERS: &[u8] = b"POST /v1/messages HTTP/1.1\r\nhost: api.example.test\r\nx-anthropic-billing-header: cc_version=2.1.283.4f2; cc_entrypoint=cli\r\ncontent-type: application/json\r\n\r\n";
const CLAUDE_BODY: &[u8] = b"{\"model\":\"claude-haiku\",\"system\":[{\"type\":\"text\",\"text\":\"x-anthropic-billing-header: cc_version=2.1.283.4f2; cc_entrypoint=cli; cch=00000; cc_prompt_id=58740b59-3dfe-46fb-8704-7d049d21a7ed;\"},{\"type\":\"text\",\"text\":\"You are a coding agent.\"}],\"messages\":[]}\n";
const PRETTY: &[u8] = b"{\n  \"rate_limits\": {\n    \"primary\": {\n      \"used_percent\": 12.5\n    },\n    \"note\": \"line one\nline two\"\n  },\n  \"plan_type\" :\n    \"enterprise\nplan\",\n  \"credits_remaining\":\n    0\n}\nx-anthropic-billing-header:\r\n  cc_version=2.1.283.4f2\r\n";

/// Every shipped extra rule in the shapes it was written for, their near misses, and the
/// overlaps with the shared rules the whole-record policy resolves by priority.
fn corpus() -> Vec<u8> {
    let mut c = Vec::new();
    for part in [
        TOKEN_COUNT,
        SSE_EVENT,
        CREDITS_QUOTED,
        HTTP_HEADERS,
        CLAUDE_BODY,
        PRETTY,
    ] {
        c.extend_from_slice(part);
    }
    // Near misses: no match, or a shorter one.
    c.extend_from_slice(
        b"{\"rate_limits\": null, \"plan_type\": \"\", \"credits_remaining\": ,}\n",
    );
    c.extend_from_slice(b"{\"plan_type\": \"a\\\"b\", \"credits_remaining\": \"4\\\"12\"}\n");
    c.extend_from_slice(b"{\"rate_limits\":{\"a\":{\"b\":{\"c\":{\"d\":1}}}}}\n");
    c.extend_from_slice(b"{\"rate_limits\":{\"a\":{\"b\":{\"c\":1},\"e\":{}}},\"f\":2}\n");
    c.extend_from_slice(b"x-anthropic-billing-: none X-Anthropic-Billing-Header: caps\n");
    c.extend_from_slice(b"x-anthropic-billing-header:\"quoted\" x-anthropic-billing-a:  \n\"\n");
    // Overlaps with the shared rules.
    c.extend_from_slice(b"{\"rate_limits\":{\"note\":\"ops@example.com\"},\"plan_type\":\"sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWX\"}\n");
    c.extend_from_slice(b"x-anthropic-billing-header: token=ABCDEFGHIJKLMNOPQRST\n");
    // Invalid UTF-8 and multi-byte text inside values.
    c.extend_from_slice(b"{\"plan_type\":\"pl\xffus\",\"credits_remaining\":\xe2\x82\xac5,\"rate_limits\":{\"x\":\"\xc3\xa9\"}}\n");
    c
}

// ── below the hold: exactly the oracle ───────────────────────────────────────────────

#[test]
fn every_split_point_matches_the_oracle_with_extra_patterns() {
    let input = corpus();
    let (want_bytes, want_spans) = legacy(&input);
    let kinds = kinds_of(&want_spans);
    for kind in EXTRA_KINDS {
        assert!(
            kinds.contains(kind),
            "the corpus exercises {kind}: {kinds:?}"
        );
    }
    assert!(
        input.len() < MAX_HELD_BYTES,
        "the corpus stays below the hold"
    );
    for split in 0..=input.len() {
        let mut p = Producer::with_redactor(P1, redactor());
        let mut records = p.open(1, CaptureSource::WireRequest);
        records.extend(p.bytes(1, &input[..split]));
        records.extend(p.redactor.flush(daemon_time(1)).unwrap());
        records.extend(p.bytes(1, &input[split..]));
        records.extend(p.end(1, EndReason::Complete));
        let (bytes, spans) = retained(&records);
        assert_eq!(spans, want_spans, "audit differs when split at {split}");
        assert_eq!(
            bytes, want_bytes,
            "retained bytes differ when split at {split}"
        );
    }
    let (bytes, spans) = retained(&stream_through(redactor(), &input, &[1], 5));
    assert_eq!(
        (bytes, spans),
        (want_bytes, want_spans),
        "one byte per push"
    );
}

/// An instance of one connector rule's shape near its edges: pretty-printed, nested,
/// escaped, empty, cut short, or glued to the next rule.
fn extra_template(rng: &mut Rng) -> Vec<u8> {
    let spaces: &[&[u8]] = &[b"", b" ", b"\n", b"\t ", b"\r\n", "\u{a0}".as_bytes()];
    let mut t = Vec::new();
    match rng.below(4) {
        0 => {
            t.extend_from_slice(b"\"rate_limits\"");
            t.extend_from_slice(pick(rng, spaces));
            t.extend_from_slice(pick(rng, &[b":", b"=", b": "]));
            t.extend_from_slice(pick(rng, spaces));
            let mut depth = 0usize;
            for _ in 0..rng.below(24) {
                match rng.below(6) {
                    0 => {
                        t.push(b'{');
                        depth += 1;
                    }
                    1 if depth > 0 => {
                        t.push(b'}');
                        depth -= 1;
                    }
                    _ => t.extend_from_slice(pick(
                        rng,
                        &[
                            b"\"used_percent\":1.5,",
                            b"\n",
                            b"\"k\":\"v\"",
                            b"\xff",
                            b"\"",
                        ],
                    )),
                }
            }
            let close = depth.min(rng.below(5));
            t.extend(std::iter::repeat_n(b'}', close));
        }
        1 => {
            t.extend_from_slice(b"\"plan_type\"");
            t.extend_from_slice(pick(rng, spaces));
            t.push(b':');
            t.extend_from_slice(pick(rng, spaces));
            t.push(b'"');
            let n = rng.below(8);
            run(rng, &mut t, b"plusproteam \n\\\xff", n);
            if rng.below(4) != 0 {
                t.push(b'"');
            }
        }
        2 => {
            t.extend_from_slice(b"\"credits_remaining\"");
            t.extend_from_slice(pick(rng, spaces));
            t.push(b':');
            t.extend_from_slice(pick(rng, spaces));
            t.extend_from_slice(pick(rng, &[b"", b"\"", b"-"]));
            let n = rng.below(8);
            run(rng, &mut t, b"0123456789. \"\\{,}", n);
            t.extend_from_slice(pick(rng, &[b"", b"\"", b",", b"}", b" "]));
        }
        _ => {
            t.extend_from_slice(pick(
                rng,
                &[
                    b"x-anthropic-billing-",
                    b"x-anthropic-billing",
                    b"X-anthropic-billing-",
                ],
            ));
            let n = rng.below(8);
            run(rng, &mut t, b"header-cc0", n);
            t.extend_from_slice(pick(rng, &[b":", b": ", b"=", b":\r\n "]));
            t.extend_from_slice(pick(rng, spaces));
            let n = rng.below(10);
            run(rng, &mut t, b"cc_version=2.1; \"\r\n", n);
        }
    }
    if rng.below(5) == 0 {
        let keep = rng.below(t.len() + 1);
        t.truncate(keep);
    }
    t
}

const FRAGMENTS: &[&[u8]] = &[
    b"{",
    b"}",
    b"\"",
    b",",
    b":",
    b" ",
    b"\n",
    b"\r\n",
    b"\\",
    b"\xff",
    "\u{2003}".as_bytes(),
    b"\"api_key\": \"0123456789abcdefXYZ\"",
    b"sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV",
    b"Bearer abcdefghijklmnop",
    b"ops@example.com",
    b"token=ABCDEFGHIJKLMNOPQRST",
    b"-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----",
];

fn random_input(rng: &mut Rng, parts: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..1 + rng.below(parts) {
        match rng.below(6) {
            0..=2 => out.extend_from_slice(&extra_template(rng)),
            3 => out.extend_from_slice(pick(
                rng,
                &[
                    TOKEN_COUNT,
                    SSE_EVENT,
                    CREDITS_QUOTED,
                    HTTP_HEADERS,
                    CLAUDE_BODY,
                    PRETTY,
                ],
            )),
            _ => out.extend_from_slice(FRAGMENTS[rng.below(FRAGMENTS.len())]),
        }
    }
    out
}

#[test]
fn seeded_random_chunking_matches_the_oracle_with_extra_patterns() {
    let seeds: u64 = std::env::var("T04B_FUZZ_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    let mut seen = std::collections::BTreeSet::new();
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let input = random_input(&mut rng, 24);
        let (want_bytes, want_spans) = legacy(&input);
        seen.extend(kinds_of(&want_spans));
        for chunking in 0..3 {
            let chunks = match chunking {
                0 => vec![1],
                1 => random_chunks(&mut rng, 9),
                _ => random_chunks(&mut rng, 64),
            };
            let (bytes, spans) =
                retained(&stream_through(redactor(), &input, &chunks, rng.below(4)));
            assert_eq!(
                (bytes.clone(), spans.clone()),
                (want_bytes.clone(), want_spans.clone()),
                "seed {seed} chunking {chunking}: input {:?} streamed {:?}",
                String::from_utf8_lossy(&input),
                String::from_utf8_lossy(&bytes)
            );
        }
    }
    for kind in EXTRA_KINDS {
        assert!(seen.contains(kind), "the generator reaches {kind}");
    }
}

// ── past the hold: never less than the oracle ────────────────────────────────────────

/// Every way a connector rule's candidate can reach the hold bound, and every way a
/// fail-closed redaction started by another rule can meet one.
fn fail_closed_cases() -> Vec<(String, Vec<u8>)> {
    let h = MAX_HELD_BYTES;
    let cat = |parts: &[&[u8]]| parts.concat();
    let object_body = units(b"\"primary\":{\"used_percent\":12.5},\n", h + 4096);
    let lorem = long(b"lorem ipsum dolor sit amet ", h + 4096);
    let local = long(b"abcdefghijklmnopqrstuvwxyz0123456789.", h + 4096);
    let mut cases: Vec<(String, Vec<u8>)> = vec![
        (
            "rate_limits object longer than the hold".into(),
            cat(&[
                b"{\"rate_limits\": {",
                &object_body,
                b"\"plan\":\"QZrl\"}}\n{\"plan_type\":\"QZplus\"}\n",
            ]),
        ),
        (
            "whitespace before the rate_limits colon".into(),
            cat(&[
                b"{\"rate_limits\"",
                &long(b" \n\t", h + 50),
                b": {\"primary\":{\"used_percent\":99}}} tail\n",
            ]),
        ),
        (
            "whitespace after the rate_limits colon".into(),
            cat(&[
                b"{\"rate_limits\":",
                &long(b"\r\n ", h + 50),
                b"{\"primary\":{\"used_percent\":99}}} tail\n",
            ]),
        ),
        (
            "plan_type value longer than the hold".into(),
            cat(&[
                b"{\"plan_type\": \"",
                &long(b"plus pro\nteam ", h + 4096),
                b"QZplan\"} tail\n",
            ]),
        ),
        (
            "bare credits value longer than the hold".into(),
            cat(&[
                b"{\"credits_remaining\": ",
                &long(b"0123456789.", h + 4096),
                b"} tail\n",
            ]),
        ),
        (
            "quoted credits value longer than the hold".into(),
            cat(&[
                b"{\"credits_remaining\": \"",
                &long(b"41 2\n0", h + 4096),
                b"\"} tail\n",
            ]),
        ),
        (
            "billing header name longer than the hold".into(),
            cat(&[
                b"x-anthropic-billing-",
                &long(b"header-", h + 4096),
                b": cc_version=QZ2.1.283\r\n",
            ]),
        ),
        (
            "whitespace after the billing colon".into(),
            cat(&[
                b"x-anthropic-billing-header:",
                &long(b" \r\n", h + 50),
                b"cc_version=QZ2.1.283; cc_entrypoint=cli\r\n",
            ]),
        ),
        (
            "billing value longer than the hold".into(),
            cat(&[
                b"{\"text\":\"x-anthropic-billing-header: ",
                &long(b"cc_prompt_id=58740b59-3dfe; ", h + 4096),
                b"QZend\"}\n",
            ]),
        ),
        // A redaction another rule starts must follow a connector candidate that begins
        // inside it across the `\n`s the connector rule can match.
        (
            "multi-line rate_limits glued to a long local-part run".into(),
            cat(&[
                &local,
                b"\"rate_limits\": {\n\"primary\": {\"used_percent\": 99}\n,\"QZrl\":1}\nafter\n",
            ]),
        ),
        (
            "billing colon then newline inside a long line".into(),
            cat(&[
                &lorem,
                b" x-anthropic-billing-header:\n  cc_version=QZ2.1.283\r\nafter\n",
            ]),
        ),
        (
            "plan_type across newlines after a long JSON value".into(),
            cat(&[
                b"{\"password\": \"",
                &lorem,
                b"\", \"plan_type\"\n:\n\"QZ\nplan\"}\nafter\n",
            ]),
        ),
        (
            "quoted credits across a newline after a long JSON value".into(),
            cat(&[
                b"{\"password\": \"",
                &lorem,
                b"\", \"credits_remaining\": \"QZ\n412\"}\nafter\n",
            ]),
        ),
        // The bytes before a fail-closed start leave with every match over them replaced,
        // the connector rules' included.
        (
            "plan_type decided just before a long local-part run".into(),
            cat(&[b"{\"plan_type\":\"QZplus\",", &local, b"@example.com\n"]),
        ),
        (
            "rate_limits inside a line that outgrows the hold".into(),
            cat(&[
                b"{\"rate_limits\":{\"primary\":{\"used_percent\":12.5}},\"x\":\"",
                &long(b"abcdefghij", h + 4096),
                b"\"}\n",
            ]),
        ),
        // A connector candidate is the earliest undecided byte and a shared secret sits
        // inside its value.
        (
            "shared secret inside a long rate_limits object".into(),
            cat(&[
                b"{\"rate_limits\":{\"a\":\"",
                &units(b"abcdefghij ", h / 2),
                b"sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWX ",
                &units(b"klmnopqrst\n", h / 2 + 4096),
                b"\"}}\n",
            ]),
        ),
    ];
    cases.push((
        "chained connector matches each near the hold".into(),
        cat(&[
            &long(b"x", h - 30),
            b"\"plan_type\":\"QZplus\",\"credits_remaining\":412,\"rate_limits\":{\"p\":{\"u\":1}}\n",
            &long(b"y", h - 30),
            b"x-anthropic-billing-header: cc_version=QZ2.1.283\r\n",
        ]),
    ));
    cases
}

#[test]
fn fail_closed_reproducers_never_expose_an_oracle_span() {
    let mut rng = Rng(0x704B_FA11);
    let mut failures = Vec::new();
    let mut connector_cases = 0usize;
    let cases = fail_closed_cases();
    let total = cases.len();
    for (name, input) in cases {
        let (_, want) = legacy(&input);
        assert!(!want.is_empty(), "{name}: the oracle redacts something");
        connector_cases += usize::from(want.iter().any(|s| EXTRA_KINDS.contains(&s.kind.as_str())));
        failures.extend(never_less(&name, &input, &mut rng));
        // Distinctive connector values never survive, at one more chunking.
        let (bytes, _) = retained(&stream_through(redactor(), &input, &[4093], 3));
        if bytes.windows(2).any(|w| w == b"QZ") {
            failures.push(format!("{name}: a QZ value was retained raw"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // Every case but the one where a shared secret suppresses the object's claim.
    assert_eq!(
        connector_cases,
        total - 1,
        "cases whose oracle redacts a connector span"
    );
}

/// Seeded inputs well past the hold: long runs of every class a connector candidate can
/// grow through, whitespace of the hold's size around colons, connector matches glued to
/// long runs at both ends, multi-line objects, and the shared rules mixed in.
fn adversarial_input(rng: &mut Rng) -> Vec<u8> {
    let h = MAX_HELD_BYTES;
    let mut input = Vec::new();
    while input.len() < 48_000 {
        match rng.below(10) {
            0 | 1 => input.extend_from_slice(&random_input(rng, 8)),
            2 => {
                // A connector key, then whitespace of about the hold's size, then the rest.
                let (head, rest): (&[u8], &[u8]) = match rng.below(4) {
                    0 => (b"\"rate_limits\"", b": {\"primary\":{\"used_percent\":7}}"),
                    1 => (b"\"plan_type\":", b"\"QZplus\""),
                    2 => (b"\"credits_remaining\"", b":412"),
                    _ => (b"x-anthropic-billing-header:", b"cc_version=QZ2.1"),
                };
                input.extend_from_slice(head);
                let n = h - 64 + rng.below(4000);
                let mut ws = Vec::new();
                while ws.len() < n {
                    ws.extend_from_slice(pick(
                        rng,
                        &[b" ", b"\t", b"\n", b"\r\n", "\u{2003}".as_bytes()],
                    ));
                }
                input.extend_from_slice(&ws);
                input.extend_from_slice(rest);
                input.extend_from_slice(pick(rng, &[b"", b"\n", b",", b"}", b"\""]));
            }
            3 => {
                // A connector value longer than the hold.
                let n = h + rng.below(4000);
                match rng.below(4) {
                    0 => {
                        input.extend_from_slice(b"\"rate_limits\":{");
                        run(rng, &mut input, b"abc012:,\"\n {}", n);
                        input.extend_from_slice(b"}}}");
                    }
                    1 => {
                        input.extend_from_slice(b"\"plan_type\":\"");
                        run(rng, &mut input, b"plus pro\nteam", n);
                        input.push(b'"');
                    }
                    2 => {
                        input.extend_from_slice(b"\"credits_remaining\":");
                        input.extend_from_slice(pick(rng, &[b"", b"\""]));
                        run(rng, &mut input, b"0123456789.\n ", n);
                        input.extend_from_slice(pick(rng, &[b"", b"\"", b","]));
                    }
                    _ => {
                        input.extend_from_slice(b"x-anthropic-billing-");
                        let name = rng.below(3) * n / 2;
                        run(rng, &mut input, b"header-", name);
                        input.extend_from_slice(b": ");
                        run(rng, &mut input, b"cc_version=2.1; ", n);
                    }
                }
            }
            4 | 5 => {
                // A long run, glued to a connector match or not.
                input.extend_from_slice(pick(rng, &[b"", b",", b"\n", b" "]));
                let class = pick(
                    rng,
                    &[
                        b"abcdefghijklmnopqrstuvwxyz0123456789.".as_slice(),
                        b"lorem ipsum dolor ",
                        b"0123456789abcdef",
                        b" \t",
                        b"abc012+/=\n",
                    ],
                );
                let n = 1_000 + rng.below(21_000);
                run(rng, &mut input, class, n);
                input.extend_from_slice(&extra_template(rng));
            }
            6 => {
                // A long JSON secret value whose line carries connector matches across
                // newlines.
                input.extend_from_slice(b"{\"password\": \"");
                let n = h / 2 + rng.below(h);
                run(rng, &mut input, b"lorem ipsum ", n);
                input.extend_from_slice(b"\", ");
                for _ in 0..1 + rng.below(3) {
                    input.extend_from_slice(&extra_template(rng));
                }
                input.push(b'\n');
            }
            _ => input.extend_from_slice(&extra_template(rng)),
        }
    }
    input
}

#[test]
fn adversarial_long_inputs_never_expose_an_oracle_span() {
    let seeds: u64 = std::env::var("T04B_ADVERSARIAL_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let mut failures = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0xA076_1D64_78BD_642F) | 1);
        let input = adversarial_input(&mut rng);
        seen.extend(kinds_of(&legacy(&input).1));
        failures.extend(never_less(&format!("seed {seed}"), &input, &mut rng));
    }
    for kind in EXTRA_KINDS {
        assert!(seen.contains(kind), "the generator reaches {kind}");
    }
    assert!(
        failures.is_empty(),
        "{} of {seeds} seeds:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ── cuts ─────────────────────────────────────────────────────────────────────────────

/// Each shipped rule in context, with the byte range of the value it redacts.
fn cut_cases() -> Vec<(&'static str, Vec<u8>, std::ops::Range<usize>)> {
    // The value is the part a cut must never release once any of it is delivered: the
    // rule's span for the three Codex rules. The billing rule's span is its whole match,
    // but its header name is not the secret, and a cut inside the name (before the colon)
    // is released raw by V1 on the same prefix too, so here its value is what follows the
    // colon. A bare credits value may begin with a multi-byte character, so a cut can fall
    // inside the first character's encoding.
    let table: [(&str, &[u8], &[u8]); 9] = [
        (
            "rate_limits",
            b"{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":12.5}},\"x\":1}}\n",
            b"{\"primary\":{\"used_percent\":12.5}}",
        ),
        (
            "plan_type",
            b"{\"plan_type\" : \"enterprise\"}\n",
            b"enterprise",
        ),
        (
            "credits bare",
            b"{\"credits_remaining\": 412.75}\n",
            b"412.75",
        ),
        (
            "credits quoted",
            b"{\"credits_remaining\":\"41 2\"}\n",
            b"\"41 2\"",
        ),
        (
            "credits bare, 2-byte start",
            "{\"credits_remaining\": \u{a3}5}\n".as_bytes(),
            "\u{a3}5".as_bytes(),
        ),
        (
            "credits bare, 3-byte start",
            "{\"credits_remaining\": \u{20ac}5}\n".as_bytes(),
            "\u{20ac}5".as_bytes(),
        ),
        (
            "credits bare, 4-byte start",
            "{\"credits_remaining\":\u{1f600}5}\n".as_bytes(),
            "\u{1f600}5".as_bytes(),
        ),
        (
            "billing header",
            b"host: a\r\nx-anthropic-billing-header: cc_version=2.1.283\r\n\r\n",
            b"cc_version=2.1.283",
        ),
        (
            "billing text",
            b"{\"text\":\"x-anthropic-billing-header: cch=00000;\"}\n",
            b"cch=00000;",
        ),
    ];
    let mut cases = Vec::new();
    for (name, input, value) in table {
        let at = input
            .windows(value.len())
            .position(|w| w == value)
            .expect("value in input");
        cases.push((name, input.to_vec(), at..at + value.len()));
    }
    cases
}

/// Every byte of `range` lies inside some span.
fn covers(spans: &[RedactionAuditSpan], range: std::ops::Range<u64>) -> bool {
    range
        .clone()
        .all(|b| spans.iter().any(|s| s.start <= b && b < s.end))
}

#[test]
fn cuts_at_every_offset_never_release_a_held_connector_value() {
    for (name, input, value) in cut_cases() {
        let (complete_bytes, complete_spans) =
            retained(&stream_through(redactor(), &input, &[3], 1));
        assert_eq!(
            (complete_bytes.clone(), complete_spans.clone()),
            legacy(&input),
            "{name}: complete run equals the oracle"
        );
        assert!(
            covers(&complete_spans, value.start as u64..value.end as u64),
            "{name}: the oracle redacts the value"
        );
        for k in 0..=input.len() {
            let (_, prefix_spans) = legacy(&input[..k]);
            // The delivered part of the value, when the cut falls inside or after it.
            let delivered = value.start as u64..(value.end.min(k)) as u64;
            for mode in ["interrupt", "end-interrupted", "gap-none", "gap-some"] {
                let mut p = Producer::with_redactor(P1, redactor());
                let mut records = p.open(1, CaptureSource::WireRequest);
                records.extend(p.bytes(1, &input[..k.min(1)]));
                records.extend(p.bytes(1, &input[k.min(1)..k]));
                records.extend(p.redactor.flush(daemon_time(1)).unwrap());
                match mode {
                    "interrupt" => records.extend(
                        p.redactor
                            .interrupt(daemon_time(2), GapReason::ProducerLost),
                    ),
                    "end-interrupted" => records.extend(p.end(1, EndReason::Interrupted)),
                    "gap-none" => {
                        records.extend(p.gap(1, GapReason::QueueFull, None));
                        records.extend(p.end(1, EndReason::Complete));
                    }
                    _ => {
                        records.extend(p.gap(1, GapReason::QueueFull, Some(5)));
                        let rest = &input[k..];
                        records.extend(p.bytes(1, rest));
                        records.extend(p.end(1, EndReason::Complete));
                    }
                }
                let (bytes, spans) = retained(&records);
                let before: Vec<RedactionAuditSpan> = spans
                    .iter()
                    .filter(|s| s.end <= k as u64)
                    .cloned()
                    .collect();
                if mode == "interrupt" {
                    // Nothing undecided leaves: the output is a prefix of the complete run.
                    assert!(
                        complete_bytes.starts_with(&bytes),
                        "{name} {mode} at {k}: retained {:?} is not a prefix of {:?}",
                        String::from_utf8_lossy(&bytes),
                        String::from_utf8_lossy(&complete_bytes)
                    );
                    assert!(
                        spans.iter().all(|s| complete_spans.contains(s)),
                        "{name} {mode} at {k}: a span the complete run does not have"
                    );
                    continue;
                }
                let missing = uncovered(&prefix_spans, &before);
                assert!(
                    missing.is_empty(),
                    "{name} {mode} at {k}: oracle(prefix) span retained raw {missing:?}"
                );
                if !delivered.is_empty() {
                    assert!(
                        covers(&before, delivered.clone()),
                        "{name} {mode} at {k}: delivered value bytes {delivered:?} retained \
                         raw in {:?}",
                        String::from_utf8_lossy(&bytes)
                    );
                }
                if mode == "gap-some" {
                    // After the gap the rest is scanned on its own, from offset k + 5.
                    let (_, rest_spans) = legacy(&input[k..]);
                    let shifted: Vec<RedactionAuditSpan> = rest_spans
                        .into_iter()
                        .map(|s| RedactionAuditSpan {
                            kind: s.kind,
                            start: s.start + k as u64 + 5,
                            end: s.end + k as u64 + 5,
                        })
                        .collect();
                    let after: Vec<RedactionAuditSpan> = spans
                        .iter()
                        .filter(|s| s.start >= k as u64 + 5)
                        .cloned()
                        .collect();
                    assert!(
                        uncovered(&shifted, &after).is_empty(),
                        "{name} {mode} at {k}: the part after the gap retained a span raw"
                    );
                }
            }
        }
    }
}

// ── construction ─────────────────────────────────────────────────────────────────────

/// Every rule form streams under names of the connector's own choosing, with no core change,
/// and equals the whole-record oracle with the same rules at every split point. A rule that
/// cannot be built never reaches the redactor ([`ExtraPattern::new`] refuses it).
#[test]
fn every_rule_form_streams_under_any_valid_name() {
    let field = |kind: &str, field: &str, value| {
        ExtraPattern::new(
            kind,
            RedactionRule::JsonField {
                field: field.into(),
                value,
            },
        )
        .unwrap()
    };
    let rules = vec![
        field("acme_usage", "usage-2", FieldValue::Object),
        field("acme_tier", "Tier", FieldValue::String),
        field("acme_left", "left", FieldValue::Scalar),
        ExtraPattern::new("acme_quota", RedactionRule::HeaderPrefix("x-acme-".into())).unwrap(),
    ];
    let input: &[u8] = b"{\"usage-2\": {\"in\": {\"n\": 3}}, \"Tier\": \"gold\", \"left\": 12.5}\n\
POST /v1 HTTP/1.1\r\nx-acme-quota: 900; window=1h\r\n\r\n\
{\"note\":\"x-acme-seen: yes\", \"left\": \"9\", \"Tier\": \"\", \"usage-2\": null}\n";
    let whole = Redactor::for_patterns("acme", &rules).unwrap().apply(input);
    let kinds = kinds_of(&whole.spans);
    for kind in ["acme_usage", "acme_tier", "acme_left", "acme_quota"] {
        assert!(
            kinds.contains(kind),
            "the input exercises {kind}: {kinds:?}"
        );
    }
    for split in 0..=input.len() {
        let mut p =
            Producer::with_redactor(P1, StreamRedactor::with_extra_patterns(&rules).unwrap());
        let mut records = p.open(1, CaptureSource::WireRequest);
        records.extend(p.bytes(1, &input[..split]));
        records.extend(p.redactor.flush(daemon_time(1)).unwrap());
        records.extend(p.bytes(1, &input[split..]));
        records.extend(p.end(1, EndReason::Complete));
        assert_eq!(
            retained(&records),
            (whole.bytes.clone(), whole.spans.clone()),
            "split at {split}"
        );
    }
    // The kind is checked when the rule is built: the markers and audit spans carry it.
    let too_long = "k".repeat(49);
    for kind in [
        "",
        "plan\ntype",
        "plan\"type",
        "plan type",
        "my-plan",
        too_long.as_str(),
    ] {
        let err = ExtraPattern::new(kind, RedactionRule::HeaderPrefix("x-a".into())).unwrap_err();
        assert!(err.contains("kind"), "{kind:?}: {err}");
    }
    // The shipped rules, under any valid kind, and none at all, are accepted.
    let renamed: Vec<ExtraPattern> = shipped()
        .into_iter()
        .enumerate()
        .map(|(i, p)| ExtraPattern::new(format!("my_plan_{i}"), p.rule).unwrap())
        .collect();
    StreamRedactor::with_extra_patterns(&renamed).unwrap();
    StreamRedactor::with_extra_patterns(&[]).unwrap();
    StreamRedactor::with_extra_patterns(&shipped()).unwrap();
}

#[test]
fn no_extra_patterns_is_the_shared_policy() {
    let input = corpus();
    let run = |redactor: StreamRedactor| {
        let mut p = Producer::with_redactor(P1, redactor);
        let mut records = p.open(1, CaptureSource::WireRequest);
        records.extend(p.bytes(1, &input));
        records.extend(p.end(1, EndReason::Complete));
        retained(&records)
    };
    let shared = Redactor::new().apply(&input);
    assert_eq!(
        run(StreamRedactor::with_extra_patterns(&[]).unwrap()),
        (shared.bytes.clone(), shared.spans.clone())
    );
    assert_eq!(run(StreamRedactor::new()), (shared.bytes, shared.spans));
    // With the connector's rules the same input loses more, and exactly what V1 removes.
    assert_eq!(run(redactor()), legacy(&input));
}

#[test]
fn metadata_strings_go_through_the_connector_rules() {
    let mut r = redactor();
    let mut metadata = meta("x-anthropic-billing-header: cc_version=2.1.283.4f2");
    metadata.protocol_hint = Some("{\"plan_type\":\"plus\"}".into());
    let records = r
        .push(CaptureFrame {
            stream: StreamKey {
                producer: P1.into(),
                stream: 1,
            },
            seq: 0,
            observed: producer_time(P1, 1),
            event: FrameEvent::Open {
                source: CaptureSource::WireRequest,
                metadata,
            },
        })
        .unwrap();
    let Some(CaptureRecord::Open { metadata, .. }) = records.first() else {
        panic!("an Open record: {records:?}");
    };
    assert_eq!(metadata.label, "<redacted:anthropic_billing_header>");
    assert_eq!(
        metadata.protocol_hint.as_deref(),
        Some("{\"plan_type\":\"<redacted:codex_plan_type>\"}")
    );
}
