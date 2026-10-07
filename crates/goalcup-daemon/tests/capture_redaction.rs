//! Bounded streaming redaction of protocol-v5 capture frames.
//!
//! The independent oracle for retained bytes is the legacy whole-record
//! `Redactor::new().apply` over the complete stream: while no held suffix reaches the
//! bound, streaming output must equal it at every chunking. Every credential below is
//! synthetic.

use base64::{Engine, engine::general_purpose::STANDARD};
use goalcup_daemon::redact::Redactor;
use goalcup_daemon::redact::stream::{MAX_HELD_BYTES, StreamRedactor};
use goalcup_protocol::capture::*;
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::redaction::RedactionAuditSpan;

#[path = "support/redaction.rs"]
mod redaction;
use redaction::*;

fn frame(seq: u64, event: FrameEvent) -> CaptureFrame {
    CaptureFrame {
        stream: StreamKey {
            producer: "11".repeat(16),
            stream: 1,
        },
        seq,
        observed: CaptureTime {
            wall: "2026-09-25T00:00:00Z".into(),
            mono_ns: seq,
            clock_id: "11".repeat(16),
        },
        event,
    }
}
#[test]
fn token_split_across_pushes_is_never_persisted() {
    let mut redactor = StreamRedactor::new();
    let mut records = redactor
        .push(frame(
            0,
            FrameEvent::Open {
                source: CaptureSource::WireResponse,
                metadata: CaptureMeta {
                    label: "exchange-0".into(),
                    wire: Some(WireIdentity {
                        connection: "22".repeat(16),
                        exchange: 0,
                        message_index: None,
                    }),
                    protocol_hint: None,
                    method: None,
                    host: None,
                    path: None,
                    status: None,
                    content_type: None,
                },
            },
        ))
        .unwrap();
    for (seq, offset, bytes) in [
        (1, 0, b"sk-ant-".as_slice()),
        (2, 7, b"abcdefghijklmnopqrstuv\n".as_slice()),
    ] {
        records.extend(
            redactor
                .push(frame(
                    seq,
                    FrameEvent::Bytes {
                        offset,
                        data: bytes.to_vec(),
                    },
                ))
                .unwrap(),
        );
    }
    records.extend(
        redactor
            .push(frame(
                3,
                FrameEvent::End {
                    reason: EndReason::Complete,
                },
            ))
            .unwrap(),
    );
    let mut retained = Vec::new();
    for record in records {
        if let CaptureRecord::Segment { data_b64, .. } = record {
            retained.extend(STANDARD.decode(data_b64).unwrap());
        }
    }
    assert!(!retained.windows(7).any(|bytes| bytes == b"sk-ant-"));
    assert!(retained.windows(9).any(|bytes| bytes == b"<redacted"));

    // The interrupted-prefix cases: a secret prefix whose remainder never arrives. The
    // prefix is held (a flush releases only what precedes it), then either the owner
    // interrupts the producer or the producer ends the stream as interrupted. Neither
    // path may retain the prefix.
    for ending in ["owner-interrupt", "end-interrupted", "gap"] {
        let mut p = Producer::new(P1);
        let mut records = p.open(1, CaptureSource::WireResponse);
        records.extend(p.bytes(1, b"reply: sk-ant-"));
        records.extend(p.bytes(1, b"abcdefghij"));
        records.extend(p.redactor.flush(daemon_time(5)).unwrap());
        assert_eq!(
            retained_of(&records, 1).0,
            b"reply: ",
            "{ending}: only the safe prefix is released"
        );
        match ending {
            "owner-interrupt" => records.extend(
                p.redactor
                    .interrupt(daemon_time(6), GapReason::ProducerLost),
            ),
            "end-interrupted" => records.extend(p.end(1, EndReason::Interrupted)),
            _ => records.extend(p.gap(1, GapReason::QueueFull, None)),
        }
        records.extend(p.redactor.flush(daemon_time(7)).unwrap());
        let all = all_segment_bytes(&records);
        assert!(
            !contains(&all, b"sk-ant-"),
            "{ending}: the held prefix was persisted"
        );
        assert!(
            !contains(&all, b"abcdefghij"),
            "{ending}: the held suffix was persisted"
        );
        assert!(
            records
                .iter()
                .any(|r| matches!(r, CaptureRecord::Gap { .. } | CaptureRecord::End { .. })),
            "{ending}: the cut is recorded"
        );
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────────────

const P2: &str = "c0ffee00d15ea5e0badc0de5a1b2c3d4";

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn all_segment_bytes(records: &[CaptureRecord]) -> Vec<u8> {
    let mut out = Vec::new();
    for r in records {
        if let CaptureRecord::Segment { data_b64, .. } = r {
            out.extend(STANDARD.decode(data_b64).unwrap());
        }
    }
    out
}

/// The retained bytes and audit spans of one stream, after checking that its segments
/// are contiguous in source coordinates, numbered from zero, and that every span lies in
/// its own segment's source range.
fn retained_of(records: &[CaptureRecord], stream: u64) -> (Vec<u8>, Vec<RedactionAuditSpan>) {
    let mut bytes = Vec::new();
    let mut spans = Vec::new();
    let mut expect_part = 0;
    let mut expect_offset: Option<u64> = None;
    for r in records {
        if let CaptureRecord::Segment {
            stream: key,
            part,
            source_offset,
            source_len,
            data_b64,
            audit,
            ..
        } = r
        {
            if key.stream != stream {
                continue;
            }
            assert_eq!(
                *part, expect_part,
                "parts are numbered from zero without gaps"
            );
            expect_part += 1;
            if let Some(o) = expect_offset {
                assert_eq!(
                    *source_offset, o,
                    "segments are contiguous in source offsets"
                );
            }
            expect_offset = Some(source_offset + source_len);
            let data = STANDARD.decode(data_b64).unwrap();
            assert!(
                data.len() <= MAX_SEGMENT_BYTES,
                "sanitized segment within C1 bound"
            );
            for s in &audit.spans {
                assert!(
                    s.start >= *source_offset
                        && s.end <= source_offset + source_len
                        && s.start < s.end,
                    "audit span {s:?} lies inside its segment's source range"
                );
            }
            bytes.extend(data);
            spans.extend(audit.spans.iter().cloned());
        }
    }
    (bytes, spans)
}

fn legacy(input: &[u8]) -> (Vec<u8>, Vec<RedactionAuditSpan>) {
    let r = Redactor::new().apply(input);
    (r.bytes, r.spans)
}

/// Fragments that exercise every shared rule, its thresholds and its near misses.
const FRAGMENTS: &[&[u8]] = &[
    b"sk-",
    b"ant-",
    b"api03-",
    b"proj-",
    b"sk_live_",
    b"rk_live_",
    b"ghp_",
    b"gho_",
    b"ghx_",
    b"AKIA",
    b"Bearer",
    b"bearer",
    b"BEARER",
    b"eyJ",
    b"hbGciOiJIUzI1NiJ9",
    b".",
    b"..",
    b"-----BEGIN ",
    b"RSA ",
    b"PRIVATE KEY",
    b"-----",
    b"-----END ",
    b"\n",
    b"\"",
    b"\":",
    b":",
    b"=",
    b" = ",
    b"api_key",
    b"api-key",
    b"apikey",
    b"x-api-key",
    b"token",
    b"Token",
    b"secret",
    b"password",
    "\u{17f}ecret".as_bytes(),
    "api_\u{212a}ey".as_bytes(),
    "to\u{212a}en".as_bytes(),
    b"tokenizer",
    b"@",
    b"user",
    b"example",
    b".com",
    b".io",
    b"xoxb-",
    b"AIza",
    b"hf_",
    b"npm_",
    b"xai-",
    b"glpat-",
    b"dop_v1_",
    // Masked provider keys (S9): the mask, its separators and their near misses.
    b"***",
    b"...",
    b"********",
    "\u{2026}".as_bytes(),
    b"\\u2026",
    b"sk-ab",
    b"wxyz",
    b"task-",
    b"-sk-",
    b"_",
    b"**********************************",
    b"sk-0",
    b"sk-or-v1",
    b"0123456789abcdef",
    b"ABCDEFGHIJKLMNOP",
    b"abcdefghij",
    b"QWERTYUIOPASDFGH",
    b"deadbeef",
    b"\\",
    b"'",
    "\u{e9}".as_bytes(),
    "\u{2003}".as_bytes(),
    "\u{a0}".as_bytes(),
    b"\xff",
    b"\xe2\x84",
    b"\xc5",
    b" ",
    b"  ",
    b"\t",
    b"{",
    b"}",
    b",",
    b"_",
    b"-",
    b"%",
    b"+",
    b"/",
    b"x",
    b"Z9",
    b"q",
];

fn run(rng: &mut Rng, out: &mut Vec<u8>, class: &[u8], lo: usize, hi: usize) {
    let n = lo + rng.below(hi - lo + 1);
    for _ in 0..n {
        out.push(class[rng.below(class.len())]);
    }
}

const C1: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
const C2: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const SPACES: &[&[u8]] = &[
    b"",
    b" ",
    b"  ",
    b"\t",
    b"\n",
    "\u{2003}".as_bytes(),
    "\u{a0}".as_bytes(),
];

/// An instance of one shared rule's shape, near its thresholds, sometimes broken or cut.
fn template(rng: &mut Rng) -> Vec<u8> {
    let mut t = Vec::new();
    match rng.below(19) {
        0 => {
            t.extend_from_slice(b"sk-ant-");
            run(rng, &mut t, C1, 15, 25);
        }
        1 => {
            t.extend_from_slice(pick(rng, &[b"sk-", b"sk-proj-", b"sk_"]));
            run(rng, &mut t, C1, 15, 25);
        }
        2 => {
            t.extend_from_slice(b"gh");
            t.push(b"pousrx"[rng.below(6)]);
            t.push(b'_');
            run(rng, &mut t, C2, 15, 25);
        }
        3 => {
            t.extend_from_slice(b"AKIA");
            run(rng, &mut t, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789", 13, 19);
        }
        4 => {
            t.extend_from_slice(pick(rng, &[b"Bearer", b"bearer", b"BeArEr", b"bearers"]));
            for _ in 0..rng.below(3) {
                t.extend_from_slice(pick(rng, SPACES));
            }
            run(rng, &mut t, b"abcXYZ019._-", 12, 20);
            if rng.below(3) == 0 {
                t.extend_from_slice("\u{17f}\u{212a}".as_bytes());
                run(rng, &mut t, b"abc", 0, 4);
            }
        }
        5 => {
            t.extend_from_slice(b"eyJ");
            run(rng, &mut t, C1, 8, 12);
            t.push(b'.');
            run(rng, &mut t, C1, 8, 12);
            if rng.below(2) == 0 {
                t.push(b'.');
                run(rng, &mut t, C1, 8, 12);
            }
        }
        6 => {
            t.extend_from_slice(b"-----BEGIN ");
            t.extend_from_slice(pick(
                rng,
                &[b"RSA ", b"", b"EC ", b"OPENSSH ", b"RSA  ", b"rsa "],
            ));
            t.extend_from_slice(b"PRIVATE KEY-----");
            t.extend_from_slice(pick(rng, &[b"", b"-", b"\n"]));
            run(rng, &mut t, b"ABCabc012+/=\n", 0, 30);
            if rng.below(3) != 0 {
                t.extend_from_slice(pick(
                    rng,
                    &[b"-----END ", b"------END ", b"-----EN", b"----END "],
                ));
                t.extend_from_slice(pick(rng, &[b"RSA ", b"", b"EC "]));
                t.extend_from_slice(pick(
                    rng,
                    &[b"PRIVATE KEY-----", b"PRIVATE KEY----", b"PRIVATE KEY"],
                ));
            }
        }
        7 => {
            t.extend_from_slice(b"xox");
            t.push(b"bapxs"[rng.below(5)]);
            t.push(b'-');
            run(rng, &mut t, b"abc012-", 8, 12);
        }
        8 => {
            t.push(b'"');
            t.extend_from_slice(pick(
                rng,
                &[
                    b"api_key",
                    b"x-api-key",
                    b"apikey",
                    b"api-Key",
                    b"token",
                    b"refresh_token",
                    b"Secret",
                    b"password",
                    b"tokenizer",
                    b"key",
                    b"my_token_id",
                    b"_token",
                    b"token_",
                    b"a__token",
                    "\u{17f}ecret".as_bytes(),
                    "api_\u{212a}ey".as_bytes(),
                ],
            ));
            t.push(b'"');
            t.extend_from_slice(pick(rng, SPACES));
            t.extend_from_slice(pick(rng, &[b":", b"=", b": "]));
            t.extend_from_slice(pick(rng, SPACES));
            t.push(b'"');
            run(rng, &mut t, b"abcABC012 +/=.\\\xff\xc3\xa9", 12, 20);
            if rng.below(4) != 0 {
                t.push(b'"');
            }
        }
        9 => {
            t.extend_from_slice(pick(
                rng,
                &[
                    b"api_key",
                    b"API-KEY",
                    b"apikey",
                    b"token",
                    b"TOKEN",
                    b"secret",
                    b"password",
                    "\u{17f}ecret".as_bytes(),
                    "to\u{212a}en".as_bytes(),
                    b"tokens",
                ],
            ));
            t.extend_from_slice(pick(rng, SPACES));
            t.extend_from_slice(pick(rng, &[b"=", b":", b"-"]));
            t.extend_from_slice(pick(rng, SPACES));
            t.extend_from_slice(pick(rng, &[b"", b"'", b"\""]));
            run(rng, &mut t, C1, 12, 20);
        }
        10 => {
            run(rng, &mut t, b"abcXYZ019._%+-", 1, 8);
            t.push(b'@');
            run(rng, &mut t, b"abc019.-", 0, 6);
            if rng.below(3) != 0 {
                t.push(b'.');
                run(rng, &mut t, b"abcXYZ", 1, 3);
            }
        }
        11 => {
            t.extend_from_slice(b"AIza");
            run(rng, &mut t, C1, 26, 34);
        }
        12..=14 => {
            t.extend_from_slice(pick(rng, &[b"hf_", b"npm_", b"xai-"]));
            run(rng, &mut t, C2, 16, 24);
        }
        15 => {
            t.extend_from_slice(pick(rng, &[b"sk", b"rk"]));
            t.extend_from_slice(b"_live_");
            run(rng, &mut t, C2, 12, 20);
        }
        16 => {
            t.extend_from_slice(b"glpat-");
            run(rng, &mut t, C1, 16, 24);
        }
        // A masked key: widths and separators on both sides of every threshold.
        18 => {
            // What may sit before the key: nothing, a boundary, a letter or a mark. None of
            // them changes what is a mask (the rule has no left boundary).
            t.extend_from_slice(pick(
                rng,
                &[
                    b"",
                    b" ",
                    b"\"",
                    b"-",
                    b"a",
                    b"_",
                    "\u{e9}".as_bytes(),
                    b"\xff",
                ],
            ));
            t.extend_from_slice(pick(rng, &[b"sk-", b"SK-", b"sK-", "\u{17f}k-".as_bytes()]));
            run(rng, &mut t, b"abcXYZ019-", 0, 10);
            t.extend_from_slice(pick(
                rng,
                &[
                    b"...",
                    b"***",
                    b"*****",
                    b"********",
                    b"*********",
                    b"..",
                    b"....",
                    "\u{2026}".as_bytes(),
                    b"\\u2026",
                    b"\\u202",
                ],
            ));
            // The long OpenAI echo, with the stars either side of the 64 the rule allows.
            if rng.below(6) == 0 {
                t.extend(std::iter::repeat_n(b'*', rng.below(75)));
            }
            run(rng, &mut t, C2, 2, 6);
        }
        _ => {
            t.extend_from_slice(b"dop_v1_");
            run(rng, &mut t, b"0123456789abcdef", 28, 36);
        }
    }
    if rng.below(4) == 0 {
        let keep = rng.below(t.len() + 1);
        t.truncate(keep);
    }
    t
}

fn random_input(rng: &mut Rng, max_parts: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let parts = 1 + rng.below(max_parts);
    for _ in 0..parts {
        match rng.below(10) {
            1..=3 => out.extend_from_slice(&template(rng)),
            // Runs that cross the class thresholds (10, 16, 20, 30, 32).
            0 => {
                let class: &[u8] = match rng.below(4) {
                    0 => b"abcdefghijklmnopqrstuvwxyz0123456789",
                    1 => b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
                    2 => b"0123456789abcdef",
                    _ => b"aZ09_-",
                };
                let n = rng.below(40);
                for _ in 0..n {
                    out.push(class[rng.below(class.len())]);
                }
            }
            _ => out.extend_from_slice(FRAGMENTS[rng.below(FRAGMENTS.len())]),
        }
    }
    out
}

// ── chunking invariance against the whole-record policy ─────────────────────────────

/// A corpus with one of every supported secret shape, near misses, JSON and shell
/// assignments, a PEM block, multi-byte and invalid UTF-8.
fn corpus() -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(
        b"POST /v1/messages\n{\"model\":\"m\",\"api_key\":\"0123456789abcdefXYZ\",",
    );
    c.extend_from_slice(b"\"x-api-key\" : \"sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZ012345\",");
    c.extend_from_slice(b"\"tokenizer\":\"not-a-secret-value-at-all\",\"note\":\"short\"}\n");
    c.extend_from_slice(b"Authorization: Bearer abcdefghijklmnop.qrstuvwxyz-0123\n");
    c.extend_from_slice(b"export OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz0123\n");
    c.extend_from_slice(b"token = 'ABCDEFGHIJKLMNOP1234' and password: hunter2hunter2hunter2\n");
    c.extend_from_slice(b"ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 AKIAABCDEFGHIJKLMNOP ");
    c.extend_from_slice(b"AKIAABCDEFGHIJKLMNO (short) xoxb-1234567890-abcdef ");
    c.extend_from_slice(
        b"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.c2lnbmF0dXJlLXN5bnRoZXRpYw\n",
    );
    c.extend_from_slice(b"mail someone.else+tag@example.co.uk or a@b.c now\n");
    c.extend_from_slice(b"-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Qu\nKUpRKfFLfRYC9AIKjbJTWit+CqvjWYzvQwECAwEAAQ==\n-----END RSA PRIVATE KEY-----\n");
    c.extend_from_slice(b"AIzaSyA-abcdefghijklmnopqrstuvwxyz0123 hf_abcdefghijklmnopqrstuvwxyz ");
    c.extend_from_slice(b"npm_abcdefghijklmnopqrstuvwxyz0123 xai-abcdefghijklmnopqrstuvwxyz ");
    c.extend_from_slice(b"sk_live_abcdefghijklmnop12 glpat-abcdefghijklmnopqrst12 ");
    c.extend_from_slice(
        b"dop_v1_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n",
    );
    c.extend_from_slice(
        "caf\u{e9} \u{17f}ecret=ABCDEFGHIJKLMNOPQRST api_\u{212a}ey: abcdefghijklmnopqrst\n"
            .as_bytes(),
    );
    // Masked provider keys, a 401 echo and a request-dump header, with near misses.
    c.extend_from_slice(
        b"{\"error\":\"Incorrect API key provided: sk-ab***wxyz.\"} sk-abcde...wxyz ",
    );
    c.extend_from_slice(
        b"\"Authorization\": \"Bearer sk-abcde\\u2026wxyz\" sk-a***wxyz sk-ab**wxyz ",
    );
    c.extend_from_slice("sk-abcdefgh\u{2026}0123 SK-ab********WXYZ sk-ab***wxy\n".as_bytes());
    c.extend_from_slice(
        b"Incorrect API key provided: sk-R7aMV***************************************oKYo. \
          sk-0...cdef Bearer sk-or-v1...cdef task-ab***wxyz api_key=SECRETSECRETSECRETsk-ab***wxyz\n",
    );
    c.extend_from_slice(
        b"binary \xff\xfe\x00\x01 sk-ant-\xffABCDEFGHIJKLMNOPQRSTUVWXYZ \xe2\x82\xac end",
    );
    c
}

#[test]
fn every_split_point_matches_whole_record_policy() {
    let input = corpus();
    let (want_bytes, want_spans) = legacy(&input);
    assert!(
        want_spans.len() >= 20,
        "the corpus exercises the rule set: {}",
        want_spans.len()
    );
    for split in 0..=input.len() {
        let mut p = Producer::new(P1);
        let mut records = p.open(1, CaptureSource::WireRequest);
        records.extend(p.bytes(1, &input[..split]));
        records.extend(p.redactor.flush(daemon_time(1)).unwrap());
        records.extend(p.bytes(1, &input[split..]));
        records.extend(p.end(1, EndReason::Complete));
        let (bytes, spans) = retained_of(&records, 1);
        assert_eq!(spans, want_spans, "audit differs when split at {split}");
        assert_eq!(
            bytes, want_bytes,
            "retained bytes differ when split at {split}"
        );
    }
}

#[test]
fn seeded_random_chunking_matches_whole_record_policy() {
    let seeds: u64 = std::env::var("T04_FUZZ_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let input = random_input(&mut rng, 60);
        let (want_bytes, want_spans) = legacy(&input);
        for chunking in 0..3 {
            let chunks = match chunking {
                0 => vec![1],
                1 => random_chunks(&mut rng, 9),
                _ => random_chunks(&mut rng, 64),
            };
            let flush_every = rng.below(4);
            let records = stream_through(StreamRedactor::new(), &input, &chunks, flush_every);
            let (bytes, spans) = retained_of(&records, 1);
            assert_eq!(
                (bytes.clone(), spans.clone()),
                (want_bytes.clone(), want_spans.clone()),
                "seed {seed} chunking {chunking}: input {:?} streamed {:?}",
                String::from_utf8_lossy(&input),
                String::from_utf8_lossy(&bytes)
            );
        }
    }
}

/// The directional oracle past the hold bound: the stream may redact more than the
/// whole-record policy, never less. Checked at several chunkings; the failure, if any.
fn never_less(name: &str, input: &[u8], rng: &mut Rng) -> Option<String> {
    let (_, want) = legacy(input);
    for chunks in [
        vec![MAX_SEGMENT_BYTES],
        random_chunks(rng, 3000),
        vec![1, 7, 64],
    ] {
        let (_, got) = retained_of(&stream_through(StreamRedactor::new(), input, &chunks, 2), 1);
        let missing = uncovered(&want, &got);
        if let Some((kind, s, e)) = missing.first() {
            return Some(format!(
                "{name}: {} whole-record span(s) retained raw, first {kind} {s}..{e} {:?}",
                missing.len(),
                String::from_utf8_lossy(&input[*s as usize..(*e as usize).min(*s as usize + 40)])
            ));
        }
    }
    None
}

/// Forced decisions at the hold bound must never retain a secret the whole-record policy
/// redacts.
#[test]
fn forced_decisions_never_expose_a_whole_record_secret() {
    let mut rng = Rng(0x5EC2E7);
    let secret = b"QZqz0QZqz1QZqz2QZqz3";
    let long = |unit: &[u8]| {
        unit.iter()
            .copied()
            .cycle()
            .take(MAX_HELD_BYTES + 4096)
            .collect::<Vec<u8>>()
    };
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();

    // (A) a decided higher-priority match that begins a local-part run of the hold's size.
    let mut a1 = b"cfg api_key=".to_vec();
    a1.extend_from_slice(secret);
    a1.push(b'.');
    a1.extend(long(b"abcdefghij."));
    cases.push(("A shell value then a local-part run", a1));
    let mut a2 = b"Authorization: Bearer ".to_vec();
    a2.extend_from_slice(secret);
    a2.push(b'+');
    a2.extend(long(b"abcdefghijklmnopqrstuvwxyz"));
    cases.push(("A bearer value then a local-part run", a2));

    // (B) a JSON value whose prefix precedes an unterminated PEM block.
    let mut b = b"{\"password\":\"".to_vec();
    b.extend_from_slice(secret);
    b.extend_from_slice(b" -----BEGIN RSA PRIVATE KEY-----\n");
    b.extend(long(
        b"MIIEowIBAAKCAQEA0Z3VS5JJcds3xfn/ygWyF8PbnGy0AHB7MhgHcTz6sE2I2yPB\n",
    ));
    b.extend_from_slice(b"\"}");
    cases.push(("B JSON value prefix before a PEM", b));

    // (C) a decided JSON claim split by a forced cut at a PEM that a later key blocks.
    let mut c = b"{\"api_key\":\"-----BEGIN RSA PRIVATE KEY-----\n".to_vec();
    c.extend_from_slice(secret);
    c.extend_from_slice(b"\"} sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA ");
    c.extend(long(b"lorem ipsum dolor sit amet "));
    cases.push(("C JSON claim split by a forced cut", c));

    // (D) a JWT inside a shell value replaced conservatively: the JWT outlives the value.
    let mut d = b"token=eyJ".to_vec();
    d.extend(long(b"abcdefghijklmnopqrstuvwxyz0123456789"));
    d.extend_from_slice(b".cGF5bG9hZF9zeW50aGV0aWM.c2lnbmF0dXJlX3N5bnRoZXRpYw end");
    cases.push(("D JWT inside a replaced shell value", d));

    // (E) a PEM block glued to an email domain replaced conservatively.
    let mut e = b"mail x@".to_vec();
    e.extend(long(b"example.com."));
    e.extend_from_slice(b"com-----BEGIN RSA PRIVATE KEY-----\n");
    e.extend_from_slice(secret);
    e.extend_from_slice(b"\n-----END RSA PRIVATE KEY----- done");
    cases.push(("E PEM glued to a replaced email domain", e));

    // (F) a higher-priority candidate inside a decided lower-priority match fails later.
    let mut f = vec![b'a'; MAX_HELD_BYTES - "x@y.bearer ".len() - 8];
    f.extend_from_slice(b"x@y.bearer ");
    f.extend_from_slice(b"QZqz0QZq");
    f.extend_from_slice(b" end");
    cases.push(("F outranked candidate fails", f));

    let mut failures = Vec::new();
    for (name, input) in &cases {
        assert!(
            !legacy(input).1.is_empty(),
            "{name}: the whole-record policy redacts something"
        );
        failures.extend(never_less(name, input, &mut rng));
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// Mechanisms past the hold bound, and the class whose own unproven prefix outgrows the
/// hold. The stream must fail closed.
fn fail_closed_cases() -> Vec<(String, Vec<u8>)> {
    let h = MAX_HELD_BYTES;
    let secret: &[u8] = b"QZqz0QZqz1QZqz2QZqz3";
    let long = |unit: &[u8], n: usize| unit.iter().copied().cycle().take(n).collect::<Vec<u8>>();
    let cat = |parts: &[&[u8]]| parts.concat();
    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    let pem_lines = long(
        b"MIIEowIBAAKCAQEA0Z3VS5JJcds3xfn/ygWyF8PbnGy0AHB7MhgHcTz6sE2I2yPB\n",
        h + 4096,
    );
    let lorem = long(b"lorem ipsum dolor sit amet ", h + 4096);
    let jwt: &[u8] = b"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.c2lnbmF0dXJlc3ludGg";
    let anthropic: &[u8] = b"sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAA";

    // A JSON value crossing a PEM END line, the PEM claim suppressed by a key.
    for suppressor in [anthropic, jwt] {
        cases.push((
            format!(
                "N1a PEM END inside a JSON value ({})",
                String::from_utf8_lossy(&suppressor[..6])
            ),
            cat(&[
                b"-----BEGIN RSA PRIVATE KEY-----\n",
                &pem_lines,
                b" ",
                suppressor,
                b" \"api_key\": \"xxxx\n-----END RSA PRIVATE KEY----- ",
                secret,
                b"\" tail",
            ]),
        ));
    }
    // A shell assignment after a JSON value whose claim a higher-priority key
    // inside it suppresses.
    for suppressor in [anthropic, jwt, b"xoxb-1234567890-abcdefghij".as_slice()] {
        for assign in [b"token=\"".as_slice(), b"api_key: \""] {
            cases.push((
                format!(
                    "N1b {} after a suppressed JSON value ({})",
                    String::from_utf8_lossy(assign),
                    String::from_utf8_lossy(&suppressor[..4])
                ),
                cat(&[
                    b"{\"password\": \"",
                    &lorem,
                    b" ",
                    suppressor,
                    b" ",
                    assign,
                    secret,
                    b" tail",
                ]),
            ));
        }
    }
    // A vendor token glued to an email domain that outgrew the hold.
    for token in [
        b"dop_v1_0123456789abcdef0123456789abcdef".as_slice(),
        b"hf_ABCDEFGHIJKLMNOPQRSTUVWX",
        b"npm_ABCDEFGHIJKLMNOPQRSTUVWX",
        b"sk_live_ABCDEFGHIJKLMNOPQR",
    ] {
        cases.push((
            format!(
                "N1c {} glued to a long email domain",
                String::from_utf8_lossy(&token[..4])
            ),
            // The run ends with `ghi-`: the email match ends at `.ghi`, before the token.
            cat(&[
                b"mail x@",
                &long(b"abc-def.ghi-", (h + 4096) / 12 * 12),
                token,
                b" tail",
            ]),
        ));
    }
    // Chained JWTs.
    cases.push((
        "N1d chained JWTs".into(),
        cat(&[
            b"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJRWnF6MFFaIn0.",
            &long(b"abcdefghij", h + 4096),
            b".eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJRWnF6MFFaIn0.",
            secret,
            b" tail",
        ]),
    ));
    // The next window starts at a JSON value's closing quote.
    cases.push((
        "N2 JSON closing quote".into(),
        cat(&[
            b"{\"password\":\"",
            &long(b"lorem ipsum ", h + 4096),
            b"\"token\": \"a b c d e f g h i j k l m n o\"api_key\": \"",
            secret,
            b"\"}",
        ]),
    ));
    // Whitespace of the hold's size between a keyword and its value.
    let spaces: [(&str, Vec<u8>); 5] = [
        ("spaces", long(b" ", h + 50)),
        ("newlines", long(b"\n", h + 50)),
        ("mixed", long(b" \t\n ", h + 50)),
        ("em-space", long("\u{2003}".as_bytes(), (h + 50) / 3 * 3)),
        ("hold-3 spaces", long(b" ", h - 3)),
    ];
    for (form, ws) in &spaces {
        cases.push((
            format!("WS bearer, {form}"),
            cat(&[b"Authorization: Bearer", ws, secret, b" tail"]),
        ));
    }
    let ws = &spaces[0].1;
    for (name, head, sep, close) in [
        (
            "password=",
            b"password=".as_slice(),
            b"".as_slice(),
            b" tail".as_slice(),
        ),
        ("api_key =", b"api_key", b"= ", b" tail"),
        ("JSON key then colon", b"{\"api_key\"", b": \"", b"\"}"),
        ("JSON colon then value", b"{\"api_key\":", b"\"", b"\"}"),
    ] {
        cases.push((format!("WS {name}"), cat(&[head, ws, sep, secret, close])));
    }
    // A match whose own unproven prefix outgrows the hold, for every kind that has one.
    cases.push((
        "PREFIX JWT first segment".into(),
        cat(&[
            b"eyJ",
            &long(b"abcdefghijklmnopqrstuvwxyz0123456789", h + 4096),
            b".eyJzdWIiOiJRWnF6MFFaIn0.",
            secret,
            b" tail",
        ]),
    ));
    cases.push((
        "PREFIX PEM header".into(),
        cat(&[
            b"-----BEGIN ",
            &long(b"RSA ", h + 4096),
            b"PRIVATE KEY-----\n",
            secret,
            b"\n-----END RSA PRIVATE KEY----- tail",
        ]),
    ));
    cases.push((
        "PREFIX JSON key".into(),
        cat(&[
            b"{\"",
            &long(b"abcdefghijklmnopqrstuvwxyz", h + 4096),
            b"_token\": \"",
            secret,
            b"\"}",
        ]),
    ));
    cases.push((
        "PREFIX JSON key with a vendor key inside".into(),
        cat(&[
            b"{\"",
            &long(b"abcdefghij", h / 2),
            b"ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            &long(b"klmnopqrst", h / 2 + 4096),
            b"_token\": \"",
            secret,
            b"\"}",
        ]),
    ));
    cases.push((
        "PREFIX email local part".into(),
        cat(&[
            &long(b"abcdefghijklmnopqrstuvwxyz0123456789.", h + 4096),
            b"QZqz0QZqz1@example.com tail",
        ]),
    ));
    cases.push((
        "PREFIX email domain without TLD".into(),
        cat(&[
            b"x@",
            &long(b"abcdefghij0123456789-", h + 4096),
            b"QZqz0QZqz1.com tail",
        ]),
    ));
    cases
}

#[test]
fn fail_closed_reproducers_never_expose_a_whole_record_secret() {
    let mut rng = Rng(0xFA11_C105);
    let cases = fail_closed_cases();
    let mut failures = Vec::new();
    for (name, input) in &cases {
        assert!(
            !legacy(input).1.is_empty(),
            "{name}: the whole-record policy redacts something"
        );
        failures.extend(never_less(name, input, &mut rng));
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// Seeded adversarial inputs for the directional oracle: long whitespace between keyword
/// and value, chained secrets, lower-priority matches spanning a replacement's end, JSON
/// closing-quote pairings, prefixes longer than the hold for every kind, and long runs
/// glued to whatever follows them.
fn adversarial_input(rng: &mut Rng) -> Vec<u8> {
    let h = MAX_HELD_BYTES;
    let value =
        |rng: &mut Rng, out: &mut Vec<u8>| run(rng, out, b"QZqz0123456789abcdefABCDEF", 16, 40);
    let ws_chars: &[&[u8]] = &[
        b" ",
        b"\t",
        b"\n",
        "\u{2003}".as_bytes(),
        "\u{a0}".as_bytes(),
    ];
    let mut input = Vec::new();
    while input.len() < 48_000 {
        match rng.below(12) {
            0 => input.extend_from_slice(&template(rng)),
            1 => input.extend_from_slice(&random_input(rng, 8)),
            2 => {
                input.extend_from_slice(pick(
                    rng,
                    &[
                        b"Authorization: Bearer",
                        b"password=",
                        b"api_key",
                        b"{\"api_key\"",
                        b"{\"api_key\":",
                        b"token:",
                        b"secret ",
                    ],
                ));
                let n = h - 64 + rng.below(4_000);
                let mut ws = Vec::new();
                while ws.len() < n {
                    ws.extend_from_slice(ws_chars[rng.below(ws_chars.len())]);
                }
                input.extend_from_slice(&ws);
                input.extend_from_slice(pick(rng, &[b"", b"= ", b": \"", b":\"", b"\"", b"="]));
                value(rng, &mut input);
                input.extend_from_slice(pick(rng, &[b" ", b"\"}", b"\n", b".", b""]));
            }
            3 => {
                // Chained secrets, glued.
                for _ in 0..2 + rng.below(3) {
                    input.extend_from_slice(pick(
                        rng,
                        &[
                            b"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJRWnF6MFFaIn0.".as_slice(),
                            b"dop_v1_0123456789abcdef0123456789abcdef",
                            b"hf_",
                            b"sk-ant-api03-",
                            b"ghp_",
                            b"x@y.",
                            b"-",
                            b".",
                        ],
                    ));
                    let n = if rng.below(3) == 0 {
                        h + rng.below(4000)
                    } else {
                        10 + rng.below(30)
                    };
                    run(rng, &mut input, b"abcdefghij0123456789ABCDEF", n, n);
                }
            }
            4 => {
                // Lower-priority matches spanning where a replacement would end.
                if rng.below(2) == 0 {
                    input.extend_from_slice(b"-----BEGIN RSA PRIVATE KEY-----\n");
                    let n = h + rng.below(4000);
                    run(rng, &mut input, b"ABCabc012+/=\n", n, n);
                    input.extend_from_slice(pick(
                        rng,
                        &[b" sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAA ".as_slice(), b" "],
                    ));
                    input.extend_from_slice(b"\"api_key\": \"xx\n-----END RSA PRIVATE KEY----- ");
                    value(rng, &mut input);
                    input.extend_from_slice(b"\" ");
                } else {
                    input.extend_from_slice(b"{\"password\": \"");
                    let n = h + rng.below(4000);
                    run(rng, &mut input, b"lorem ipsum dolor ", n, n);
                    input.extend_from_slice(pick(
                        rng,
                        &[b" hf_ABCDEFGHIJKLMNOPQRSTUVWX ".as_slice(), b" "],
                    ));
                    input.extend_from_slice(pick(
                        rng,
                        &[b"token=\"", b"api_key: \"", b"\"token\": \""],
                    ));
                    value(rng, &mut input);
                    input.extend_from_slice(b"\" ");
                }
            }
            5 => {
                // JSON closing-quote pairings after a long value.
                input.extend_from_slice(b"{\"password\":\"");
                let n = h / 2 + rng.below(h);
                run(rng, &mut input, b"lorem ipsum ", n, n);
                input.extend_from_slice(
                    b"\"token\": \"a b c d e f g h i j k l m n o\"api_key\": \"",
                );
                value(rng, &mut input);
                input.extend_from_slice(b"\"}");
            }
            6 => {
                // A prefix longer than the hold, for a random kind.
                let n = h + rng.below(4000);
                match rng.below(7) {
                    0 => {
                        input.extend_from_slice(b"eyJ");
                        run(rng, &mut input, b"abcdefghij0123456789", n, n);
                        input.extend_from_slice(b".eyJzdWIiOiJRWnF6MFFaIn0.");
                        value(rng, &mut input);
                    }
                    1 => {
                        input.extend_from_slice(b"-----BEGIN ");
                        run(rng, &mut input, b"RSA ", n, n);
                        input.extend_from_slice(b"PRIVATE KEY-----\n");
                        value(rng, &mut input);
                        input.extend_from_slice(b"\n-----END RSA PRIVATE KEY-----");
                    }
                    2 => {
                        input.extend_from_slice(b"{\"");
                        run(rng, &mut input, b"abcdefghij", n, n);
                        input.extend_from_slice(b"_token\": \"");
                        value(rng, &mut input);
                        input.extend_from_slice(b"\"}");
                    }
                    3 => {
                        run(rng, &mut input, b"abcdefghij0123456789.", n, n);
                        input.push(b'@');
                        value(rng, &mut input);
                        input.extend_from_slice(b".com");
                    }
                    4 => {
                        input.extend_from_slice(b"x@");
                        run(rng, &mut input, b"abcdefghij-", n, n);
                        value(rng, &mut input);
                        input.extend_from_slice(b".com");
                    }
                    5 => {
                        input.extend_from_slice(b"sk-ant-");
                        run(rng, &mut input, b"abcdefghij0123456789_-", n, n);
                    }
                    _ => {
                        input.extend_from_slice(b"Bearer");
                        let m = n;
                        run(rng, &mut input, b" \t\n", m, m);
                        value(rng, &mut input);
                    }
                }
                input.extend_from_slice(pick(rng, &[b" ", b"\n", b"", b"\"", b","]));
            }
            7 | 8 => {
                // A long run, glued or not, closed by anything or nothing.
                input.extend_from_slice(pick(rng, &[b"", b",", b"\n", b" "]));
                let class = pick(
                    rng,
                    &[
                        b"abcdefghijklmnopqrstuvwxyz0123456789.".as_slice(),
                        b"0123456789abcdef",
                        b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-",
                        b"abc012+/=\n",
                        b"lorem ipsum dolor ",
                        b"abcdefghij.+%",
                        b" \t",
                        b"RSA PRIVATE KEY ",
                    ],
                );
                let n = 1_000 + rng.below(21_000);
                run(rng, &mut input, class, n, n);
                input.extend_from_slice(pick(
                    rng,
                    &[
                        b"",
                        b",",
                        b"\n",
                        b"@example.com",
                        b"\"",
                        b".",
                        b" ",
                        b"\":\"",
                        b"-----END RSA PRIVATE KEY-----",
                    ],
                ));
            }
            _ => input.extend_from_slice(FRAGMENTS[rng.below(FRAGMENTS.len())]),
        }
    }
    input
}

#[test]
fn adversarial_long_inputs_never_expose_a_whole_record_secret() {
    let seeds: u64 = std::env::var("T04_ADVERSARIAL_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let mut failures = Vec::new();
    let mut spans = 0usize;
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0xA076_1D64_78BD_642F) | 1);
        let input = adversarial_input(&mut rng);
        spans += legacy(&input).1.len();
        failures.extend(never_less(&format!("seed {seed}"), &input, &mut rng));
    }
    assert!(spans > 0);
    assert!(
        failures.is_empty(),
        "{} of {seeds} seeds:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Seeded inputs past the hold bound with the same directional oracle. Every long run is
/// closed by `,`, so no whole-record match can begin inside one and run past the hold
/// (the documented unproven-prefix limits); runs may be glued to what precedes them.
#[test]
fn seeded_long_inputs_never_expose_a_whole_record_secret() {
    // 60 by default.
    let seeds: u64 = std::env::var("T04_DIRECTIONAL_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let classes: &[&[u8]] = &[
        b"abcdefghijklmnopqrstuvwxyz0123456789.",
        b"0123456789abcdef",
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-",
        b"abc012+/=\n",
        b"lorem ipsum dolor ",
        b"abcdefghij.+%",
    ];
    let heads: &[&[u8]] = &[
        b"api_key=",
        b"Bearer ",
        b"token: '",
        b"{\"password\":\"",
        b"\"x-api-key\" : \"",
        b"{\"api_key\":\"-----BEGIN RSA PRIVATE KEY-----\n",
        b"-----BEGIN PRIVATE KEY-----\n",
        b"eyJhbGciOiJIUzI1NiJ9.",
        b"x@y.",
        b"sk-ant-api03-",
        b"ghp_",
        b"dop_v1_",
    ];
    let mut failures = Vec::new();
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        let mut input = Vec::new();
        while input.len() < 60_000 {
            match rng.below(8) {
                0..=2 => input.extend_from_slice(&template(&mut rng)),
                3 => input.extend_from_slice(&random_input(&mut rng, 6)),
                4 | 5 => {
                    if rng.below(2) == 0 {
                        input.push(b',');
                    }
                    let class = classes[rng.below(classes.len())];
                    let n = 1_000 + rng.below(21_000);
                    run(&mut rng, &mut input, class, n, n);
                    input.push(b',');
                }
                6 => {
                    input.extend_from_slice(heads[rng.below(heads.len())]);
                    run(&mut rng, &mut input, b"QZqz0123456789abcdefABCDEF", 16, 40);
                    input.extend_from_slice(pick(
                        &mut rng,
                        &[b".", b"+", b" ", b"\"}", b"\n", b"%"],
                    ));
                }
                _ => {
                    input.extend_from_slice(b"-----BEGIN RSA PRIVATE KEY-----\n");
                    let n = 100 + rng.below(20_000);
                    run(&mut rng, &mut input, b"ABCabc012+/=\n", n, n);
                    if rng.below(2) == 0 {
                        input.extend_from_slice(b"\n-----END RSA PRIVATE KEY-----\n");
                    }
                }
            }
        }
        failures.extend(never_less(&format!("seed {seed}"), &input, &mut rng));
    }
    assert!(
        failures.is_empty(),
        "{} of {seeds} seeds:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Seeded inputs far longer than the hold bound, with long runs of every class a
/// candidate can grow through, streamed at very different chunkings: forced decisions
/// must be the same wherever the frame boundaries fall, and so must a discontinuity.
#[test]
fn seeded_long_inputs_are_chunking_invariant() {
    let seeds: u64 = std::env::var("T04_LONG_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) | 1);
        let mut input = Vec::new();
        while input.len() < 70_000 {
            match rng.below(6) {
                0 => {
                    let class: &[u8] = pick(
                        &mut rng,
                        &[
                            b"abcdefghijklmnopqrstuvwxyz0123456789.",
                            b"0123456789abcdef",
                            b" \t",
                            b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-",
                            b"abc012+/=\n",
                            b"RSA PRIVATE KEY ",
                        ],
                    );
                    let n = 2_000 + rng.below(22_000);
                    run(&mut rng, &mut input, class, n, n);
                }
                1 => {
                    let prefix = pick(
                        &mut rng,
                        &[
                            b"-----BEGIN RSA PRIVATE KEY-----\n".as_slice(),
                            b"{\"api_key\": \"",
                            b"Bearer ",
                            b"eyJhbGciOiJIUzI1NiJ9.",
                            b"token=",
                            b"sk-ant-",
                            b"x@",
                            b"\"",
                            b"-----BEGIN ",
                            b"-----END RSA PRIVATE KEY-----",
                        ],
                    );
                    input.extend_from_slice(prefix);
                }
                _ => input.extend_from_slice(&random_input(&mut rng, 30)),
            }
        }
        let whole = |ending: EndReason, chunks: &[usize]| {
            let mut p = Producer::new(P1);
            let mut records = p.open(1, CaptureSource::WireResponse);
            let mut at = 0;
            let mut i = 0;
            while at < input.len() {
                let n = chunks[i % chunks.len()].min(input.len() - at);
                records.extend(p.bytes(1, &input[at..at + n]));
                at += n;
                i += 1;
                if i % 3 == 0 {
                    records.extend(p.redactor.flush(daemon_time(i as u64)).unwrap());
                }
            }
            records.extend(p.end(1, ending));
            retained_of(&records, 1)
        };
        for ending in [EndReason::Complete, EndReason::Interrupted] {
            let reference = whole(ending, &[MAX_SEGMENT_BYTES]);
            for chunks in [
                random_chunks(&mut rng, 40),
                random_chunks(&mut rng, 5000),
                vec![1, 2, 3, 5, 8, 13],
            ] {
                assert_eq!(
                    whole(ending, &chunks),
                    reference,
                    "seed {seed} {ending:?}: output depends on chunking"
                );
            }
        }
    }
}

/// Inputs whose candidates outgrow the hold bound. Output must not depend on chunking;
/// where the whole-record rule would also match, the conservative replacement reaches
/// the same span.
#[test]
fn long_secrets_are_chunking_invariant() {
    let mut rng = Rng(0xDEC0DE);
    let lower = |rng: &mut Rng, n: usize, set: &[u8]| -> Vec<u8> {
        (0..n).map(|_| set[rng.below(set.len())]).collect()
    };
    let b64ish = b"abcdefghijklmnopqrstuvwxyz0123456789/";
    let mut cases: Vec<(&str, Vec<u8>, bool, &[u8])> = Vec::new();

    let mut pem = b"key follows: -----BEGIN RSA PRIVATE KEY-----\n".to_vec();
    let body = lower(&mut rng, 40_000, b64ish);
    pem.extend_from_slice(&body);
    pem.extend_from_slice(b"SYNTHETICBODYMARK\n-----END RSA PRIVATE KEY-----\ntrailer");
    cases.push(("pem", pem, true, b"SYNTHETICBODYMARK"));

    let mut jwt = b"cookie eyJhbGciOiJIUzI1NiJ9.eyJ".to_vec();
    jwt.extend_from_slice(&lower(
        &mut rng,
        30_000,
        b"abcdefghijklmnopqrstuvwxyz0123456789",
    ));
    jwt.extend_from_slice(b"SYNTHETICJWTMARK.c2lnbmF0dXJlLXN5bnRoZXRpYw; path=/");
    cases.push(("jwt", jwt, false, b"SYNTHETICJWTMARK"));

    let mut json = b"{\"model\":\"m\",\"api_key\":\"".to_vec();
    json.extend_from_slice(&lower(&mut rng, 30_000, b64ish));
    json.extend_from_slice(b"SYNTHETICJSONMARK\"}");
    cases.push(("json", json, false, b"SYNTHETICJSONMARK"));

    let mut bearer = b"Authorization: Bearer ".to_vec();
    bearer.extend_from_slice(&lower(
        &mut rng,
        30_000,
        b"abcdefghijklmnopqrstuvwxyz0123456789.",
    ));
    bearer.extend_from_slice(b"SYNTHETICBEARERMARK\r\nnext: header");
    cases.push(("bearer", bearer, false, b"SYNTHETICBEARERMARK"));

    let mut dop = b"do token dop_v1_".to_vec();
    let hex = lower(&mut rng, 40_000, b"0123456789abcdef");
    let hex_probe = hex[20_000..20_032].to_vec();
    dop.extend_from_slice(&hex);
    dop.extend_from_slice(b" done");
    cases.push(("digitalocean", dop, false, &hex_probe));

    // Long unproven prefixes: released, so only chunking invariance applies.
    let mut email = lower(&mut rng, 40_000, b"abcdefghijklmnopqrstuvwxyz0123456789.");
    email.extend_from_slice(b"@example.com and more");
    cases.push(("email-run", email, false, b""));

    let mut spaced = b"bearer".to_vec();
    spaced.extend(std::iter::repeat_n(b' ', 20_000));
    spaced.extend_from_slice(b"abcdefghijklmnopqrstuvwxyz end");
    cases.push(("bearer-spaces", spaced, false, b""));

    let mut key = b"{\"".to_vec();
    key.extend_from_slice(&lower(&mut rng, 20_000, b"abcdefghijklmnopqrstuvwxyz"));
    key.extend_from_slice(b"_token\": \"abcdefghijklmnopqrstuvwxyz\"}");
    cases.push(("json-long-key", key, false, b""));

    // A PEM block with no END line: replaced to the end of the stream.
    let mut open_pem = b"-----BEGIN PRIVATE KEY-----\n".to_vec();
    open_pem.extend_from_slice(&lower(&mut rng, 30_000, b64ish));
    open_pem.extend_from_slice(b"SYNTHETICOPENPEMMARK");
    cases.push(("pem-unterminated", open_pem, false, b"SYNTHETICOPENPEMMARK"));

    for (name, input, legacy_equal, secret) in &cases {
        let reference = retained_of(
            &stream_through(StreamRedactor::new(), input, &[MAX_SEGMENT_BYTES], 0),
            1,
        );
        // Past the hold the stream fails closed: it may replace more than the whole-record
        // policy, never less, and it still replaces exactly that span when a record
        // terminator follows the secret (the PEM block here).
        assert!(
            uncovered(&legacy(input).1, &reference.1).is_empty(),
            "{name}: whole-record bytes retained raw"
        );
        if *legacy_equal {
            assert_eq!(
                reference,
                legacy(input),
                "{name}: the whole-record span, to its terminator"
            );
        }
        if !secret.is_empty() {
            assert!(
                !contains(&reference.0, secret),
                "{name}: secret material retained"
            );
        }
        for chunks in [
            vec![1],
            vec![4096],
            random_chunks(&mut rng, 3000),
            vec![7, 1, 13, 64],
        ] {
            let flush_every = 1 + rng.below(3);
            let got = retained_of(
                &stream_through(StreamRedactor::new(), input, &chunks, flush_every),
                1,
            );
            assert_eq!(
                got,
                reference,
                "{name}: output depends on chunking {:?}",
                &chunks[..4.min(chunks.len())]
            );
        }
    }
}

type PemCase = (&'static str, Vec<u8>, Vec<u8>, (u64, u64));

/// A PEM block is never shadowed by a claim inside it or across its edge, in a stream too:
/// `(name, input, redacted bytes, the span's source range)`. Every expectation is written out,
/// so the test does not borrow the whole-record policy's answer to check the stream against.
fn pem_block_cases() -> Vec<PemCase> {
    const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
    const END: &str = "-----END PRIVATE KEY-----";
    let mut cases = Vec::new();
    for (name, inner) in [
        ("github_token", "ghp_AAAAAAAAAAAAAAAAAAAA"),
        ("aws_access_key", "AKIAIOSFODNN7EXAMPLE"),
        ("anthropic_key", "sk-ant-api03-AAAABBBBCCCCDDDDEEEE"),
        ("bearer", "Bearer AAAAAAAAAAAAAAAA"),
        ("email", "x@y.com"),
        (
            "several",
            "ghp_AAAAAAAAAAAAAAAAAAAA AKIAIOSFODNN7EXAMPLE x@y.com",
        ),
    ] {
        let block = format!("{BEGIN}\nMIIBsecretbody\n{inner}\nQQ==\n{END}");
        let text = format!("before\n{block}\nafter");
        cases.push((
            name,
            text.clone().into_bytes(),
            b"before\n<redacted:private_key>\nafter".to_vec(),
            (7, 7 + block.len() as u64),
        ));
    }
    // A BEGIN with no END runs to the next key's END; a claim inside leaves both raw.
    let run = format!(
        "{BEGIN}\nMIIBfirstsecret\nghp_AAAAAAAAAAAAAAAAAAAA\nnotes\n\
         -----BEGIN RSA PRIVATE KEY-----\nMIIBsecondsecret\n-----END RSA PRIVATE KEY-----"
    );
    cases.push((
        "unterminated begin, later end",
        format!("log\n{run}\ntail").into_bytes(),
        b"log\n<redacted:private_key>\ntail".to_vec(),
        (4, 4 + run.len() as u64),
    ));
    // Across the BEGIN edge (`bearer` takes `-----BEGIN`) and the END edge (an email takes
    // `KEY-----`).
    let lead = format!("Authorization: Bearer {}", "A".repeat(16));
    let block = format!("{BEGIN}\nMIIBsecretbody\n{END}");
    cases.push((
        "across the begin edge",
        format!("{lead}{block}\nafter").into_bytes(),
        b"Authorization: <redacted:private_key>\nafter".to_vec(),
        (15, (lead.len() + block.len()) as u64),
    ));
    let text = format!("{block}x@y.com tail");
    cases.push((
        "across the end edge",
        text.clone().into_bytes(),
        b"<redacted:private_key> tail".to_vec(),
        (0, (text.len() - " tail".len()) as u64),
    ));
    // A key cut off before its END line is a block to the end of the stream, as in a record.
    let open = format!("{BEGIN}\nMIIBsecretbody\n(truncated");
    let text = format!("log\n{open}");
    cases.push((
        "truncated key",
        text.clone().into_bytes(),
        b"log\n<redacted:private_key>".to_vec(),
        (4, text.len() as u64),
    ));
    let text = format!("log\n{BEGIN}\nMIIBsecretbody\nghp_AAAAAAAAAAAAAAAAAAAA\n(truncated");
    cases.push((
        "truncated key with a claim inside",
        text.clone().into_bytes(),
        b"log\n<redacted:private_key>".to_vec(),
        (4, text.len() as u64),
    ));
    let text = format!("Bearer abcdefgh{open}");
    cases.push((
        "truncated key behind a short claim",
        text.clone().into_bytes(),
        b"<redacted:private_key>".to_vec(),
        (0, text.len() as u64),
    ));
    // Where taking the block out of first-wins let a lower claim shadow a token.
    let key = format!("{BEGIN}\nMIIBsecretbody\n{END}");
    let text = format!("{key}x@a.glpat-ABCDEFGHIJKLMNOPQRSTUVWXYZ tail");
    cases.push((
        "an email and a token across the end edge",
        text.clone().into_bytes(),
        b"<redacted:private_key> tail".to_vec(),
        (0, (text.len() - " tail".len()) as u64),
    ));
    let text = format!("glpat-ABCDEFGHIJKLMNOPQRSTUVtoken=ABCDEFGHIJKLMNOPQR{key}");
    cases.push((
        "an assignment and a token across the begin edge",
        text.clone().into_bytes(),
        b"<redacted:private_key>".to_vec(),
        (0, text.len() as u64),
    ));
    cases
}

/// Each case, split in two at every position (with a flush between, so the first half is
/// decided as far as it can be), then pushed a byte at a time with a flush after each byte:
/// the same bytes and the same one span as the whole record.
#[test]
fn a_pem_block_is_never_shadowed_at_any_split_point() {
    for (name, input, want, (start, end)) in pem_block_cases() {
        let (want_bytes, want_spans) = legacy(&input);
        assert_eq!(want_bytes, want, "{name}: the whole record");
        assert_eq!(want_spans.len(), 1, "{name}: {want_spans:?}");
        assert_eq!(want_spans[0].kind, "private_key", "{name}");
        assert_eq!(
            (want_spans[0].start, want_spans[0].end),
            (start, end),
            "{name}"
        );
        for split in 0..=input.len() {
            let mut p = Producer::new(P1);
            let mut records = p.open(1, CaptureSource::WireRequest);
            records.extend(p.bytes(1, &input[..split]));
            records.extend(p.redactor.flush(daemon_time(1)).unwrap());
            records.extend(p.bytes(1, &input[split..]));
            records.extend(p.end(1, EndReason::Complete));
            let (bytes, spans) = retained_of(&records, 1);
            assert_eq!(spans, want_spans, "{name}: audit differs split at {split}");
            assert_eq!(bytes, want, "{name}: bytes differ split at {split}");
        }
        let records = stream_through(StreamRedactor::new(), &input, &[1], 1);
        assert_eq!(
            retained_of(&records, 1),
            (want_bytes, want_spans),
            "{name}: byte at a time"
        );
    }
}

/// The same at the hold bound: a block with a claim inside it that fits in the hold, one that
/// outgrows it with the claim before, at and after the bound, one with the claim last, and a
/// `BEGIN` that never ends before a later key. Past the bound the stream fails closed and
/// replaces from the block's start to the record terminator, which here is the block.
#[test]
fn a_pem_block_with_a_claim_inside_is_one_redaction_at_the_hold_bound() {
    const BEGIN: &str = "-----BEGIN RSA PRIVATE KEY-----\n";
    const END: &str = "-----END RSA PRIVATE KEY-----";
    let mut rng = Rng(0xB10C);
    let filler = |rng: &mut Rng, n: usize| -> String {
        let set = b"abcdefghijklmnopqrstuvwxyz0123456789/";
        (0..n).map(|_| set[rng.below(set.len())] as char).collect()
    };
    let token = "ghp_AAAAAAAAAAAAAAAAAAAA";
    // `(name, body before the claim, claim, body after it)`.
    let h = MAX_HELD_BYTES;
    let mut cases: Vec<(String, String, &str, String)> = Vec::new();
    for (name, before, after) in [
        ("fits the hold", h / 2 - 300, h / 2 - 300),
        ("claim just inside the bound", h - 64, 4096),
        ("claim at the bound", h - BEGIN.len(), 4096),
        ("claim just past the bound", h + 64, 4096),
        ("claim first", 0, h + 4096),
        ("claim last", h + 4096, 0),
    ] {
        cases.push((
            name.to_string(),
            filler(&mut rng, before),
            token,
            filler(&mut rng, after),
        ));
    }
    let mut inputs: Vec<(String, Vec<u8>, Vec<u8>)> = Vec::new();
    let mut truncated: Vec<(String, Vec<u8>, Vec<u8>)> = Vec::new();
    for (name, before, claim, after) in &cases {
        let block = format!("{BEGIN}{before}\n{claim}\n{after}\n{END}");
        inputs.push((
            name.clone(),
            format!("log {block}\ntrailer").into_bytes(),
            claim.as_bytes().to_vec(),
        ));
    }
    // A BEGIN with no END, a claim, more than the hold of text, then a whole second key.
    let run = format!(
        "{BEGIN}{}\n{token}\n{}\n{BEGIN}{}\n{END}",
        filler(&mut rng, 100),
        filler(&mut rng, h + 4096),
        filler(&mut rng, 200)
    );
    inputs.push((
        "unterminated begin, claim, then a later key".to_string(),
        format!("log {run}\ntrailer").into_bytes(),
        token.as_bytes().to_vec(),
    ));

    // Keys with no END at all, at and past the bound: a block to the end of the stream.
    for (name, before, after) in [
        ("truncated, fits the hold", h / 2 - 300, h / 2 - 300),
        ("truncated, claim at the bound", h - BEGIN.len(), 4096),
        ("truncated, claim past the bound", h + 64, 4096),
        ("truncated, claim first", 0, h + 4096),
    ] {
        let (before, after) = (filler(&mut rng, before), filler(&mut rng, after));
        truncated.push((
            name.to_string(),
            format!("log {BEGIN}{before}\n{token}\n{after}").into_bytes(),
            token.as_bytes().to_vec(),
        ));
    }

    let expected_whole: &[u8] = b"log <redacted:private_key>\ntrailer";
    let expected_open: &[u8] = b"log <redacted:private_key>";
    #[allow(clippy::type_complexity)]
    let all: Vec<(&String, &Vec<u8>, &Vec<u8>, &[u8])> = inputs
        .iter()
        .map(|(n, i, c)| (n, i, c, expected_whole))
        .chain(truncated.iter().map(|(n, i, c)| (n, i, c, expected_open)))
        .collect();
    for (name, input, claim, expected) in all {
        let want = legacy(input);
        assert_eq!(want.1.len(), 1, "{name}: the whole record is one block");
        assert_eq!(want.1[0].kind, "private_key", "{name}");
        assert_eq!(want.0, expected, "{name}: the whole record");
        for chunks in [
            vec![MAX_SEGMENT_BYTES],
            vec![1],
            vec![4096],
            random_chunks(&mut rng, 3000),
            vec![7, 1, 13, 64],
        ] {
            let flush_every = rng.below(4);
            let got = retained_of(
                &stream_through(StreamRedactor::new(), input, &chunks, flush_every),
                1,
            );
            assert_eq!(
                got,
                want,
                "{name}: chunking {:?}",
                &chunks[..4.min(chunks.len())]
            );
            assert!(!contains(&got.0, claim), "{name}: the claim is retained");
            assert!(!contains(&got.0, b"MIIB"), "{name}: key material retained");
        }
    }
}

/// A forced decision must not release a higher-priority candidate that is still short
/// because a decided lower-priority match began before it and holds the bytes: here a
/// local part long enough to fill the hold ends in `x@y.bearer`, and the bearer token
/// after it has not reached sixteen characters when the hold is full.
#[test]
fn forced_release_keeps_a_candidate_inside_a_decided_match() {
    let fill = MAX_HELD_BYTES - "x@y.bearer ".len() - 8;
    let mut input = vec![b'a'; fill];
    input.extend_from_slice(b"x@y.bearer SYNTHETICbearerTOKEN0123 end");
    let reference = retained_of(
        &stream_through(StreamRedactor::new(), &input, &[MAX_SEGMENT_BYTES], 0),
        1,
    );
    assert!(
        !contains(&reference.0, b"SYNTHETIC"),
        "bearer token released: {:?}",
        String::from_utf8_lossy(&reference.0[reference.0.len().saturating_sub(80)..])
    );
    assert!(reference.1.iter().any(|s| s.kind == "bearer"));
    for chunks in [vec![1], vec![999], vec![4096, 1]] {
        assert_eq!(
            retained_of(
                &stream_through(StreamRedactor::new(), &input, &chunks, 2),
                1
            ),
            reference
        );
    }

    // The same with a JWT inside a shell assignment that has already stopped at `.`.
    let mut rng = Rng(5);
    let mut input = Vec::new();
    run(
        &mut rng,
        &mut input,
        b"abcdefghijklmnopqrstuvwxyz",
        12_000,
        12_000,
    );
    input.extend_from_slice(b" token=eyJhbGciOiJIUzI1NiJ9.eyJ");
    run(
        &mut rng,
        &mut input,
        b"abcdefghijklmnopqrstuvwxyz0123456789",
        9_000,
        9_000,
    );
    input.extend_from_slice(b"SYNTHETICPAYLOAD.c2lnbmF0dXJlLXN5bnRoZXRpYw end");
    let (bytes, spans) = retained_of(
        &stream_through(StreamRedactor::new(), &input, &[MAX_SEGMENT_BYTES], 0),
        1,
    );
    assert_eq!(
        (bytes.clone(), spans.clone()),
        legacy(&input),
        "the JWT wins its range, as whole-record"
    );
    assert!(!contains(&bytes, b"SYNTHETICPAYLOAD") && !contains(&bytes, b"c2lnbmF0"));
    for chunks in [vec![1], vec![999], vec![4096, 1]] {
        assert_eq!(
            retained_of(
                &stream_through(StreamRedactor::new(), &input, &chunks, 2),
                1
            ),
            (bytes.clone(), spans.clone())
        );
    }
}

// ── named behaviors ──────────────────────────────────────────────────────────────────

#[test]
fn unknown_stream_rejected() {
    for event in [
        FrameEvent::Bytes {
            offset: 0,
            data: b"hello".to_vec(),
        },
        FrameEvent::End {
            reason: EndReason::Complete,
        },
        FrameEvent::Gap {
            reason: GapReason::QueueFull,
            lost_bytes: None,
        },
    ] {
        let mut p = Producer::new(P1);
        p.open(1, CaptureSource::WireRequest);
        let err = p.try_push(5, event).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("never opened"), "{err}");
        // A refused frame refuses the rest of the producer's frames.
        assert!(
            p.try_push(
                1,
                FrameEvent::Bytes {
                    offset: 0,
                    data: b"x".to_vec()
                }
            )
            .is_err()
        );
    }

    // Producer binding and sequence order are checked before any stream state.
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireRequest);
    let mut other = p.next_frame(
        1,
        FrameEvent::Bytes {
            offset: 0,
            data: b"x".to_vec(),
        },
    );
    other.stream.producer = P2.into();
    assert!(p.redactor.push(other).is_err());

    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireRequest);
    p.seq += 1;
    assert!(
        p.try_push(
            1,
            FrameEvent::Bytes {
                offset: 0,
                data: b"x".to_vec()
            }
        )
        .is_err()
    );

    let mut p = Producer::new(P1);
    p.seq = 1;
    assert!(
        p.try_push(
            1,
            FrameEvent::Open {
                source: CaptureSource::WireRequest,
                metadata: http_meta(CaptureSource::WireRequest),
            }
        )
        .is_err(),
        "a producer's frames start at sequence zero"
    );
}

#[test]
fn ended_stream_rejects_bytes() {
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireResponse);
    p.bytes(1, b"complete body");
    p.end(1, EndReason::Complete);
    let err = p
        .try_push(
            1,
            FrameEvent::Bytes {
                offset: 13,
                data: b"more".to_vec(),
            },
        )
        .unwrap_err();
    assert!(err.to_string().contains("already ended"), "{err}");

    // An ended stream ID can never be opened again, and IDs only grow.
    let mut p = Producer::new(P1);
    p.open(2, CaptureSource::WireResponse);
    p.end(2, EndReason::Complete);
    assert!(
        p.try_push(
            2,
            FrameEvent::Open {
                source: CaptureSource::WireResponse,
                metadata: http_meta(CaptureSource::WireResponse),
            }
        )
        .is_err()
    );
    let mut p = Producer::new(P1);
    p.open(3, CaptureSource::WireResponse);
    assert!(
        p.try_push(
            2,
            FrameEvent::Open {
                source: CaptureSource::WireResponse,
                metadata: http_meta(CaptureSource::WireResponse),
            }
        )
        .is_err()
    );

    // Offsets are contiguous after the first bytes.
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireResponse);
    p.bytes(1, b"abc");
    assert!(
        p.try_push(
            1,
            FrameEvent::Bytes {
                offset: 4,
                data: b"d".to_vec()
            }
        )
        .is_err()
    );
}

#[test]
fn stream_limit_bounds_memory() {
    let mut p = Producer::new(P1);
    for s in 1..=MAX_OPEN_STREAMS as u64 {
        p.open(s, CaptureSource::WireResponse);
        // Every stream holds an undecided secret prefix.
        assert!(p.bytes(s, b"sk-ant-abcdef").is_empty());
    }
    let err = p
        .try_push(
            MAX_OPEN_STREAMS as u64 + 1,
            FrameEvent::Open {
                source: CaptureSource::WireResponse,
                metadata: http_meta(CaptureSource::WireResponse),
            },
        )
        .unwrap_err();
    assert!(err.to_string().contains("too many open"), "{err}");

    // Held state per stream is bounded: an undecided run far longer than the bound fails
    // closed instead of being held, and each record terminator hands the stream back, so
    // pushed minus accounted source never exceeds the hold bound.
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireResponse);
    let mut pushed = 0usize;
    let mut records = Vec::new();
    let mut line = vec![b'a'; MAX_SEGMENT_BYTES - 1];
    line.push(b'\n');
    for i in 0..8 {
        records.extend(p.bytes(1, &line));
        pushed += line.len();
        records.extend(p.redactor.flush(daemon_time(i)).unwrap());
        retained_of(&records, 1);
        let accounted: u64 = records
            .iter()
            .filter_map(|r| match r {
                CaptureRecord::Segment { source_len, .. } => Some(*source_len),
                _ => None,
            })
            .sum();
        assert!(
            pushed - accounted as usize <= MAX_HELD_BYTES,
            "held {} bytes after {} pushed",
            pushed - accounted as usize,
            pushed
        );
    }
    let (bytes, _) = retained_of(&records, 1);
    assert!(
        !contains(&bytes, &[b'a'; 64]),
        "each over-long run is redacted, never released"
    );
}

#[test]
fn metadata_is_redacted_without_retaining_query() {
    let key = "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZ012345";
    let mut p = Producer::new(P1);
    let records = p.push(
        1,
        FrameEvent::Open {
            source: CaptureSource::WireRequest,
            metadata: CaptureMeta {
                wire: Some(WireIdentity {
                    connection: CONN.into(),
                    exchange: 3,
                    message_index: None,
                }),
                label: format!("GET /v1/keys/{key}"),
                protocol_hint: Some("openai-responses".into()),
                method: Some("GET".into()),
                host: Some("api.example.test".into()),
                path: Some(format!(
                    "/v1/keys/{key}?api_key=AIzaSyA-abcdefghijklmnopqrstuvwxyz0123&x=1#frag"
                )),
                status: None,
                content_type: Some("application/json".into()),
            },
        },
    );
    let [
        CaptureRecord::Open {
            stream, metadata, ..
        },
    ] = records.as_slice()
    else {
        panic!("one Open record: {records:?}");
    };
    // Identity is structural: never taken from, or rewritten like, a label.
    assert_eq!(
        stream,
        &StreamKey {
            producer: P1.into(),
            stream: 1
        }
    );
    assert_eq!(metadata.wire.as_ref().unwrap().connection, CONN);
    assert_eq!(
        metadata.path.as_deref(),
        Some("/v1/keys/<redacted:anthropic_key>")
    );
    assert_eq!(metadata.label, "GET /v1/keys/<redacted:anthropic_key>");
    let json = serde_json::to_string(&records).unwrap();
    for leaked in [key, "AIzaSyA", "api_key", "x=1", "frag", "?"] {
        assert!(!json.contains(leaked), "{leaked} retained in {json}");
    }

    // Redaction can lengthen a string; it stays within the string bound and stays redacted.
    let mut p = Producer::new(P1);
    let label = "AKIAABCDEFGHIJKLMNOP ".repeat(48);
    assert!(label.len() <= MAX_METADATA_STRING_BYTES);
    let records = p.push(
        1,
        FrameEvent::Open {
            source: CaptureSource::Native,
            metadata: CaptureMeta {
                wire: None,
                label,
                protocol_hint: None,
                method: None,
                host: None,
                path: None,
                status: None,
                content_type: None,
            },
        },
    );
    let CaptureRecord::Open { metadata, .. } = &records[0] else {
        panic!()
    };
    assert!(metadata.label.len() <= MAX_METADATA_STRING_BYTES);
    assert!(!metadata.label.contains("AKIAABCD"));
    assert!(metadata.validate().is_ok());
}

#[test]
fn long_private_key_does_not_grow_pending_buffer() {
    let mut p = Producer::new(P1);
    let mut records = p.open(1, CaptureSource::WireResponse);
    let lead = b"{\"content\":\"here is the key ";
    let begin = b"-----BEGIN OPENSSH PRIVATE KEY-----\n";
    records.extend(p.bytes(1, lead));
    records.extend(p.bytes(1, begin));
    let line = b"b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAABlwAAAAdzc2gtcn\n";
    let mut body_len = 0usize;
    // One MiB of key body, pushed in bounded frames. Nothing of it is ever returned.
    for i in 0..16u64 {
        let chunk: Vec<u8> = line
            .iter()
            .copied()
            .cycle()
            .take(MAX_SEGMENT_BYTES)
            .collect();
        body_len += chunk.len();
        let out = p.bytes(1, &chunk);
        let mut flushed = p.redactor.flush(daemon_time(i)).unwrap();
        flushed.extend(out);
        assert!(!contains(&all_segment_bytes(&flushed), b"b3BlbnNz"));
        records.extend(flushed);
    }
    let key_start = (lead.len()) as u64;
    // The bytes before the key were released while the key streamed past: the key's
    // prefix is not what held them back.
    assert_eq!(retained_of(&records, 1).0, lead);

    // Interrupted mid-key, the replacement in progress is dropped with the undecided
    // bytes: the retained source range ends where the key begins, so a resume from the
    // durable end re-reads the key from its start, and the gap accounts for the rest.
    let mut q = Producer::new(P1);
    q.open(1, CaptureSource::WireResponse);
    q.bytes(1, lead);
    q.bytes(1, begin);
    for _ in 0..16 {
        let chunk: Vec<u8> = line
            .iter()
            .copied()
            .cycle()
            .take(MAX_SEGMENT_BYTES)
            .collect();
        q.bytes(1, &chunk);
    }
    let cut = q
        .redactor
        .interrupt(daemon_time(99), GapReason::ProducerLost);
    let (kept, spans) = retained_of(&cut, 1);
    assert_eq!((kept.as_slice(), spans.len()), (lead.as_slice(), 0));
    let covered: u64 = cut
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Segment { source_len, .. } => Some(*source_len),
            _ => None,
        })
        .sum();
    assert_eq!(covered, key_start);
    assert!(matches!(
        cut.last(),
        Some(CaptureRecord::Gap {
            lost_bytes: None,
            ..
        })
    ));

    let tail = b"-----END OPENSSH PRIVATE KEY-----\n\"}";
    records.extend(p.bytes(1, tail));
    records.extend(p.end(1, EndReason::Complete));
    let (bytes, spans) = retained_of(&records, 1);
    let end_of_key = key_start + (begin.len() + body_len + tail.len() - 3) as u64;
    assert_eq!(
        spans,
        vec![RedactionAuditSpan {
            kind: "private_key".into(),
            start: key_start,
            end: end_of_key
        }]
    );
    let mut want = lead.to_vec();
    want.extend_from_slice(b"<redacted:private_key>\n\"}");
    assert_eq!(bytes, want);
}

#[test]
fn many_short_matches_fit_envelope_budget() {
    let key = b"AKIAABCDEFGHIJKLMNOP ";
    let mut input = Vec::new();
    while input.len() + key.len() <= MAX_SEGMENT_BYTES {
        input.extend_from_slice(key);
    }
    let matches = input.len() / key.len();
    for chunks in [vec![MAX_SEGMENT_BYTES], vec![333]] {
        let records = stream_through(StreamRedactor::new(), &input, &chunks, 0);
        let (bytes, spans) = retained_of(&records, 1);
        assert_eq!(spans.len(), matches, "no audit entry is dropped");
        assert_eq!((bytes, spans), legacy(&input));
        let segments = records
            .iter()
            .filter(|r| matches!(r, CaptureRecord::Segment { .. }))
            .count();
        assert!(segments > 1, "the audit forces the output to split");
        for record in records {
            envelope_fits(record);
        }
    }

    // The same with the largest clock identifier the bound admits, all of it JSON-escaped.
    let clock = "\"".repeat(MAX_METADATA_STRING_BYTES);
    let mut r = StreamRedactor::new();
    let at = |seq: u64| CaptureTime {
        wall: "2026-09-27T10:00:00.000000001Z".into(),
        mono_ns: u64::MAX - seq,
        clock_id: clock.clone(),
    };
    let key = StreamKey {
        producer: P1.into(),
        stream: 1,
    };
    let mut records = r
        .push(CaptureFrame {
            stream: key.clone(),
            seq: 0,
            observed: at(0),
            event: FrameEvent::Open {
                source: CaptureSource::Native,
                metadata: CaptureMeta {
                    wire: None,
                    label: "x".into(),
                    protocol_hint: None,
                    method: None,
                    host: None,
                    path: None,
                    status: None,
                    content_type: None,
                },
            },
        })
        .unwrap();
    let mut offset = 0;
    for (i, chunk) in input.chunks(4_000).enumerate() {
        records.extend(
            r.push(CaptureFrame {
                stream: key.clone(),
                seq: i as u64 + 1,
                observed: at(i as u64 + 1),
                event: FrameEvent::Bytes {
                    offset,
                    data: chunk.to_vec(),
                },
            })
            .unwrap(),
        );
        offset += chunk.len() as u64;
    }
    records.extend(r.flush(daemon_time(1)).unwrap());
    assert_eq!(retained_of(&records, 1), legacy(&input));
    for record in records {
        envelope_fits(record);
    }
}

/// The record fits the largest envelope the writer could wrap it in, newline included.
fn envelope_fits(record: CaptureRecord) {
    let envelope = Envelope {
        protocol_version: M2_PROTOCOL_VERSION,
        entry_id: "e".repeat(64),
        source: "capture".into(),
        recorded_at: "2026-09-27T10:00:00.123456789Z".into(),
        link: Link {
            seq: u64::MAX,
            prev: "0".repeat(64),
            hash: "f".repeat(64),
        },
        body: EventBody::Capture { record },
        redaction: None,
    };
    let line = serde_json::to_vec(&envelope).unwrap().len() + 1;
    assert!(line <= MAX_ENVELOPE_BYTES, "envelope of {line} bytes");
}

#[test]
fn audit_offsets_are_source_absolute() {
    let mut input = vec![b','; 40_000];
    let aws_at = 55_000;
    input.extend(vec![b','; 30_000]);
    input[aws_at..aws_at + 20].copy_from_slice(b"AKIAABCDEFGHIJKLMNOP");
    // A token that straddles the second push boundary (70_000).
    let ghp_at = 69_990;
    input[ghp_at - 1] = b' ';
    let ghp = b"ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    input.truncate(ghp_at);
    input.extend_from_slice(ghp);
    input.extend(vec![b';'; 10_000]);
    let mut p = Producer::new(P1);
    let mut records = p.open(1, CaptureSource::WireRequest);
    records.extend(p.bytes(1, &input[..40_000]));
    records.extend(p.bytes(1, &input[40_000..70_000]));
    records.extend(p.bytes(1, &input[70_000..]));
    records.extend(p.end(1, EndReason::Complete));
    let (bytes, spans) = retained_of(&records, 1);
    assert_eq!(
        spans,
        vec![
            RedactionAuditSpan {
                kind: "aws_access_key".into(),
                start: aws_at as u64,
                end: aws_at as u64 + 20
            },
            RedactionAuditSpan {
                kind: "github_token".into(),
                start: ghp_at as u64,
                end: (ghp_at + ghp.len()) as u64
            },
        ]
    );
    let covered: u64 = records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Segment { source_len, .. } => Some(*source_len),
            _ => None,
        })
        .sum();
    assert_eq!(
        covered,
        input.len() as u64,
        "segments cover every source byte once"
    );
    assert_eq!(bytes, legacy(&input).0);
}

#[test]
fn safe_bytes_preserve_exact_order() {
    let mut rng = Rng(77);
    // No '@', quotes, dashes, underscores or letters: nothing here can begin a rule.
    let alphabet = b"0123456789 ,;:{}[]()<>!#$&*/|~^\n\t";
    let input: Vec<u8> = (0..150_000)
        .map(|_| alphabet[rng.below(alphabet.len())])
        .collect();
    assert!(legacy(&input).1.is_empty());
    for chunks in [
        random_chunks(&mut rng, 5000),
        vec![1, 2, 3],
        vec![MAX_SEGMENT_BYTES],
    ] {
        let records = stream_through(StreamRedactor::new(), &input, &chunks, 2);
        let (bytes, spans) = retained_of(&records, 1);
        assert!(spans.is_empty());
        assert_eq!(bytes, input);
    }
}

#[test]
fn binary_non_utf8_input_is_supported() {
    let mut input: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    input.extend_from_slice(b"\xff\xfeghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\xff\x80");
    input.extend_from_slice("\u{e9}t\u{e9} \u{17f}ecret=ABCDEFGHIJKLMNOPQRST \u{1F600}".as_bytes());
    input.extend_from_slice(b"\xf0\x9f\x98 truncated emoji, lone \xc3 and \xed\xa0\x80 surrogate");
    let (want_bytes, want_spans) = legacy(&input);
    assert_eq!(want_spans.len(), 2);
    for chunks in [vec![1], vec![2, 3], vec![777], vec![MAX_SEGMENT_BYTES]] {
        let (bytes, spans) = retained_of(
            &stream_through(StreamRedactor::new(), &input, &chunks, 1),
            1,
        );
        assert_eq!(spans, want_spans);
        assert_eq!(bytes, want_bytes);
        assert!(!contains(&bytes, b"ghp_ABCD"));
        assert!(
            contains(&bytes, &(0..=255u8).collect::<Vec<_>>()),
            "binary bytes kept verbatim"
        );
    }
}

#[test]
fn merged_observations_keep_time_interval() {
    let mut p = Producer::new(P1);
    let mut records = p.open(1, CaptureSource::WireResponse);
    records.extend(p.bytes(1, b"data: one "));
    let t1 = producer_time(P1, p.mono);
    records.extend(p.bytes(1, b"two "));
    records.extend(p.bytes(1, b"three\n"));
    let t3 = producer_time(P1, p.mono);
    assert!(
        records
            .iter()
            .all(|r| !matches!(r, CaptureRecord::Segment { .. })),
        "coalesced until a tick"
    );
    let tick = daemon_time(1_000_000);
    let flushed = p.redactor.flush(tick.clone()).unwrap();
    let [
        CaptureRecord::Segment {
            first,
            last,
            part,
            source_offset,
            source_len,
            ..
        },
    ] = flushed.as_slice()
    else {
        panic!("one merged segment: {flushed:?}");
    };
    assert_eq!((*part, *source_offset, *source_len), (0, 0, 20));
    assert_eq!(first, &t1, "first input observation, not the tick");
    assert_eq!(last, &t3, "last input observation, not the tick");
    assert_eq!(first.clock_id, P1, "segment times keep the producer clock");

    // Bytes held undecided at t4 and released by the observation at t5 carry [t4, t5].
    let mut records = p.bytes(1, b"key sk-");
    let t4 = producer_time(P1, p.mono);
    records.extend(p.bytes(1, b"x!"));
    let t5 = producer_time(P1, p.mono);
    records.extend(p.redactor.flush(daemon_time(1_000_001)).unwrap());
    let [
        CaptureRecord::Segment {
            first,
            last,
            source_offset,
            ..
        },
    ] = records.as_slice()
    else {
        panic!("{records:?}");
    };
    assert_eq!((first, last, *source_offset), (&t4, &t5, 20));
    assert_eq!(
        p.redactor.flush(daemon_time(1_000_002)).unwrap(),
        vec![],
        "a tick never invents output"
    );

    // A candidate held across many more frames than the redactor remembers still gets
    // the observation of the frame that delivered its first byte.
    let mut records = Vec::new();
    let mut first_time = None;
    for b in b"see sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWX " {
        records.extend(p.bytes(1, &[*b]));
        if *b == b's' && first_time.is_none() {
            first_time = Some(producer_time(P1, p.mono));
        }
    }
    let decided = producer_time(P1, p.mono);
    records.extend(p.redactor.flush(daemon_time(1_000_003)).unwrap());
    let segs: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Segment {
                first, last, audit, ..
            } => Some((first.clone(), last.clone(), audit.spans.len())),
            _ => None,
        })
        .collect();
    let (first, last, _) = segs
        .iter()
        .find(|(_, _, spans)| *spans == 1)
        .expect("the key's segment");
    assert!(
        first.mono_ns <= first_time.unwrap().mono_ns,
        "interval starts no later than the key's first byte"
    );
    assert_eq!(last, &decided);
}

#[test]
fn recovery_discards_old_secret_suffix() {
    // Old producer: a response stream whose last bytes are an undecided secret prefix.
    let mut old = Producer::new(P1);
    let mut chain = old.open(1, CaptureSource::WireResponse);
    chain.extend(old.bytes(1, b"event: delta\ndata: {\"text\":\"use sk-ant-api03-"));
    chain.extend(old.bytes(1, b"ABCDEFGHIJ"));
    // New producer after the emitter rotated: Recovery first, then re-observed state.
    let mut new = Producer::new(P2);
    let recovery = new.next_frame(
        0,
        FrameEvent::Recovery {
            previous_producer: P1.into(),
            reason: GapReason::QueueFull,
        },
    );
    // The entry owner admits the Recovery: it interrupts the named producer first, with
    // its own clock, then appends the new producer's records.
    chain.extend(
        old.redactor
            .interrupt(daemon_time(500), GapReason::QueueFull),
    );
    chain.extend(new.redactor.push(recovery).unwrap());
    chain.extend(new.push(
        0,
        FrameEvent::Connection {
            connection: CONN.into(),
            event: ConnectionEvent::Open {
                alpn: Some("http/1.1".into()),
                already_open: true,
            },
        },
    ));
    // The new producer's stream carries what the old suffix would have completed.
    chain.extend(new.open(1, CaptureSource::WireResponse));
    chain.extend(new.bytes(1, b"KLMNOPQRSTUVWXYZ012345\"}\n"));
    chain.extend(new.end(1, EndReason::Complete));

    let all = all_segment_bytes(&chain);
    assert!(
        !contains(&all, b"sk-ant-"),
        "old secret prefix persisted: {:?}",
        String::from_utf8_lossy(&all)
    );
    assert!(!contains(&all, b"ABCDEFGHIJ"));
    assert!(
        contains(&all, b"event: delta\ndata: {\"text\":\"use "),
        "decided bytes survive"
    );
    let gap_at = chain
        .iter()
        .position(|r| matches!(r, CaptureRecord::Gap { .. }))
        .unwrap();
    let recovery_at = chain
        .iter()
        .position(|r| matches!(r, CaptureRecord::Recovery { .. }))
        .unwrap();
    assert!(
        gap_at < recovery_at,
        "the old epoch's gap precedes the recovery"
    );
    let CaptureRecord::Gap {
        stream,
        observed,
        reason,
        lost_bytes,
    } = &chain[gap_at]
    else {
        panic!()
    };
    assert_eq!(
        stream.as_ref().unwrap(),
        &StreamKey {
            producer: P1.into(),
            stream: 1
        }
    );
    assert_eq!(
        (observed.clock_id.as_str(), *reason, *lost_bytes),
        (DAEMON_CLOCK, GapReason::QueueFull, None)
    );
    let CaptureRecord::Recovery {
        previous_producer,
        producer,
        observed,
        ..
    } = &chain[recovery_at]
    else {
        panic!()
    };
    assert_eq!((previous_producer.as_str(), producer.as_str()), (P1, P2));
    assert_eq!(
        observed.clock_id, P2,
        "a producer's own record keeps its clock"
    );
    // Every segment time is a producer observation.
    for r in &chain {
        if let CaptureRecord::Segment {
            first,
            last,
            stream,
            ..
        } = r
        {
            assert_eq!(
                (first.clock_id.as_str(), last.clock_id.as_str()),
                (stream.producer.as_str(), stream.producer.as_str())
            );
        }
    }
}

#[test]
fn sparse_tick_flushes_safe_prefix() {
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireResponse);
    assert!(p.bytes(1, b"data: {\"delta\":\"hi\"}\n\n").is_empty());
    let observed = producer_time(P1, p.mono);
    let tick = p.redactor.flush(daemon_time(250)).unwrap();
    let (bytes, _) = retained_of(&tick, 1);
    assert_eq!(
        bytes, b"data: {\"delta\":\"hi\"}\n\n",
        "a sparse stream is flushed by the tick"
    );
    let CaptureRecord::Segment { first, last, .. } = &tick[0] else {
        panic!()
    };
    assert_eq!((first, last), (&observed, &observed));

    assert!(p.bytes(1, b"data: partial sk-").is_empty());
    let tick = p.redactor.flush(daemon_time(500)).unwrap();
    let CaptureRecord::Segment {
        data_b64,
        source_offset,
        ..
    } = &tick[0]
    else {
        panic!("{tick:?}")
    };
    assert_eq!(STANDARD.decode(data_b64).unwrap(), b"data: partial ");
    assert_eq!(*source_offset, 22);
    assert_eq!(
        p.redactor.flush(daemon_time(750)).unwrap(),
        vec![],
        "the undecided suffix stays held"
    );
}

#[test]
fn interrupt_never_flushes_secret_prefix() {
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WsServer);
    p.open(2, CaptureSource::WsClient);
    assert!(p.bytes(1, b"hello sk-ant-abcdefghij").is_empty());
    assert!(p.bytes(2, b"{\"password\": \"hunter2").is_empty());
    let tick = p.redactor.flush(daemon_time(1)).unwrap();
    assert_eq!(retained_of(&tick, 1).0, b"hello ");
    assert_eq!(retained_of(&tick, 2).0, b"{");
    let cut = p
        .redactor
        .interrupt(daemon_time(2), GapReason::ProducerLost);
    let gaps: Vec<_> = cut
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Gap {
                stream,
                observed,
                lost_bytes,
                ..
            } => Some((stream.clone(), observed.clone(), *lost_bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(
        gaps.len(),
        2,
        "one unknown-loss gap per open stream: {cut:?}"
    );
    for (stream, observed, lost) in gaps {
        assert!(stream.is_some());
        assert_eq!((observed.clock_id.as_str(), lost), (DAEMON_CLOCK, None));
    }
    assert!(
        all_segment_bytes(&cut).is_empty(),
        "nothing undecided is released: {cut:?}"
    );
    // The instance is finished: no second report, no more frames, nothing to flush.
    assert!(
        p.redactor
            .interrupt(daemon_time(3), GapReason::ProducerLost)
            .is_empty()
    );
    assert_eq!(p.redactor.flush(daemon_time(4)).unwrap(), vec![]);
    assert!(
        p.try_push(
            1,
            FrameEvent::Bytes {
                offset: 23,
                data: b"klmnop".to_vec()
            }
        )
        .is_err()
    );

    // A producer with nothing open still reports its loss once.
    let mut p = Producer::new(P2);
    let cut = p.redactor.interrupt(daemon_time(5), GapReason::IngestLost);
    assert_eq!(
        cut,
        vec![CaptureRecord::Gap {
            stream: None,
            observed: daemon_time(5),
            reason: GapReason::IngestLost,
            lost_bytes: None,
        }]
    );
}

#[test]
fn one_streams_pending_suffix_does_not_hold_another_stream() {
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireRequest);
    p.open(2, CaptureSource::WireResponse);
    assert!(p.bytes(1, b"auth: Bearer abc").is_empty());
    assert!(p.bytes(2, b"ok\n").is_empty());
    assert!(p.bytes(1, b"def").is_empty());
    let mut records = p.redactor.flush(daemon_time(1)).unwrap();
    assert_eq!(retained_of(&records, 2).0, b"ok\n");
    assert_eq!(retained_of(&records, 1).0, b"auth: ");
    records.extend(p.bytes(1, b"ghijklmnopq\n"));
    records.extend(p.end(1, EndReason::Complete));
    assert_eq!(retained_of(&records, 1).0, b"auth: <redacted:bearer>\n");
}

#[test]
fn complete_does_not_mean_provider_operation_succeeded() {
    let mut p = Producer::new(P1);
    let mut records = p.push(
        1,
        FrameEvent::Open {
            source: CaptureSource::WireResponse,
            metadata: CaptureMeta {
                status: Some(529),
                ..http_meta(CaptureSource::WireResponse)
            },
        },
    );
    let body = b"{\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}";
    records.extend(p.bytes(1, body));
    records.extend(p.end(1, EndReason::Complete));
    let kinds: Vec<&str> = records
        .iter()
        .map(|r| match r {
            CaptureRecord::Open { .. } => "open",
            CaptureRecord::Segment { .. } => "segment",
            CaptureRecord::End { .. } => "end",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["open", "segment", "end"]);
    let CaptureRecord::Open { metadata, .. } = &records[0] else {
        panic!()
    };
    assert_eq!(metadata.status, Some(529));
    // Complete is transport framing only: the error body is kept verbatim and nothing
    // asserts that the provider operation succeeded.
    assert_eq!(retained_of(&records, 1).0, body);
    let CaptureRecord::End { reason, .. } = &records[2] else {
        panic!()
    };
    assert_eq!(*reason, EndReason::Complete);
}

#[test]
fn connection_controls_never_create_byte_parts() {
    let mut p = Producer::new(P1);
    let mut records = Vec::new();
    for event in [
        ConnectionEvent::Open {
            alpn: None,
            already_open: false,
        },
        ConnectionEvent::UpgradedWebSocket,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Interrupted,
        },
    ] {
        let f = p.next_frame(
            0,
            FrameEvent::Connection {
                connection: CONN.into(),
                event: event.clone(),
            },
        );
        let observed = f.observed.clone();
        let out = p.redactor.push(f).unwrap();
        assert_eq!(
            out,
            vec![CaptureRecord::Connection {
                producer: P1.into(),
                connection: CONN.into(),
                observed,
                event,
            }]
        );
        records.extend(out);
    }
    assert!(
        records
            .iter()
            .all(|r| matches!(r, CaptureRecord::Connection { .. }))
    );
    // Stream zero is never a byte stream, and controls never use a payload stream.
    assert!(
        p.try_push(
            0,
            FrameEvent::Bytes {
                offset: 0,
                data: b"x".to_vec()
            }
        )
        .is_err()
    );
    let mut p = Producer::new(P1);
    assert!(
        p.try_push(
            0,
            FrameEvent::Open {
                source: CaptureSource::WireRequest,
                metadata: http_meta(CaptureSource::WireRequest),
            }
        )
        .is_err()
    );
    let mut p = Producer::new(P1);
    assert!(
        p.try_push(
            1,
            FrameEvent::Connection {
                connection: CONN.into(),
                event: ConnectionEvent::UpgradedWebSocket,
            }
        )
        .is_err()
    );

    // A control frame first returns decided payload segments, so retained order follows
    // observation order; it adds none of its own.
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireResponse);
    assert!(p.bytes(1, b"body\n").is_empty());
    let out = p.push(
        0,
        FrameEvent::Connection {
            connection: CONN.into(),
            event: ConnectionEvent::Close {
                outcome: ConnectionOutcome::Complete,
            },
        },
    );
    assert!(matches!(
        out.as_slice(),
        [
            CaptureRecord::Segment { .. },
            CaptureRecord::Connection { .. }
        ]
    ));
}

#[test]
fn recovery_cannot_clear_previous_gap() {
    let mut old = Producer::new(P1);
    let mut chain = old.open(1, CaptureSource::WireResponse);
    chain.extend(old.bytes(1, b"partial token=abcdefgh"));
    // The producer itself reports a loss inside the stream; the gap is sticky.
    chain.extend(old.gap(1, GapReason::QueueFull, None));
    chain.extend(
        old.redactor
            .interrupt(daemon_time(10), GapReason::QueueFull),
    );
    let gaps_before = chain
        .iter()
        .filter(|r| matches!(r, CaptureRecord::Gap { .. }))
        .count();
    assert_eq!(gaps_before, 2);

    let mut new = Producer::new(P2);
    chain.extend(new.push(
        0,
        FrameEvent::Recovery {
            previous_producer: P1.into(),
            reason: GapReason::QueueFull,
        },
    ));
    chain.extend(new.open(1, CaptureSource::WireResponse));
    chain.extend(new.bytes(1, b"fresh\n"));
    chain.extend(new.end(1, EndReason::Complete));
    assert_eq!(
        chain
            .iter()
            .filter(|r| matches!(r, CaptureRecord::Gap { .. }))
            .count(),
        gaps_before,
        "recovery adds no record that retracts or replaces a gap"
    );
    // The old epoch's streams cannot be resumed: not through the old instance, and not
    // through the new one, whose stream keys belong to the new producer only.
    assert!(
        old.try_push(
            1,
            FrameEvent::Bytes {
                offset: 22,
                data: b"ijklmnop".to_vec()
            }
        )
        .is_err()
    );
    let mut stale = new.next_frame(
        1,
        FrameEvent::Bytes {
            offset: 6,
            data: b"x".to_vec(),
        },
    );
    stale.stream.producer = P1.into();
    assert!(new.redactor.push(stale).is_err());
    // Recovery is a producer's first frame, never a mid-epoch reset.
    let mut late = Producer::new(P2);
    late.open(1, CaptureSource::WireResponse);
    assert!(
        late.try_push(
            0,
            FrameEvent::Recovery {
                previous_producer: P1.into(),
                reason: GapReason::QueueFull
            }
        )
        .is_err()
    );
    let all = all_segment_bytes(&chain);
    assert!(
        !contains(&all, b"abcdefgh"),
        "the held assignment value is not persisted"
    );
}

#[test]
fn gap_frames_keep_source_coordinates() {
    let mut p = Producer::new(P1);
    let mut records = p.open(1, CaptureSource::Native);
    records.extend(p.bytes(1, b"line one\nkey sk-ant-xyz"));
    records.extend(p.gap(1, GapReason::QueueFull, Some(5)));
    records.extend(p.bytes(1, b"after the gap\n"));
    records.extend(p.end(1, EndReason::Complete));
    let segments: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Segment {
                source_offset,
                source_len,
                data_b64,
                audit,
                ..
            } => Some((
                *source_offset,
                *source_len,
                STANDARD.decode(data_b64).unwrap(),
                audit.spans.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].0, 0);
    assert_eq!(segments[0].1, 23);
    assert_eq!(segments[0].2, b"line one\nkey <redacted:anthropic_key>");
    assert_eq!(
        segments[0].3,
        vec![RedactionAuditSpan {
            kind: "anthropic_key".into(),
            start: 13,
            end: 23
        }]
    );
    assert_eq!(
        (segments[1].0, segments[1].2.as_slice()),
        (28, b"after the gap\n".as_slice())
    );
    let gap = records
        .iter()
        .position(|r| matches!(r, CaptureRecord::Gap { .. }))
        .unwrap();
    assert!(
        matches!(&records[gap], CaptureRecord::Gap { lost_bytes: Some(5), observed, .. } if observed.clock_id == P1)
    );

    // An unknown-size gap lets the next bytes start later, never earlier.
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::Native);
    p.bytes(1, b"abc");
    p.gap(1, GapReason::SourceChanged, None);
    assert!(
        p.try_push(
            1,
            FrameEvent::Bytes {
                offset: 2,
                data: b"x".to_vec()
            }
        )
        .is_err()
    );
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::Native);
    p.bytes(1, b"abc");
    p.gap(1, GapReason::SourceChanged, None);
    let out = p.push(
        1,
        FrameEvent::Bytes {
            offset: 100,
            data: b"later\n".to_vec(),
        },
    );
    let mut out2 = out;
    out2.extend(p.end(1, EndReason::Complete));
    assert!(out2.iter().any(|r| matches!(
        r,
        CaptureRecord::Segment {
            source_offset: 100,
            ..
        }
    )));
}

#[test]
fn flush_takes_a_valid_tick_and_consumes_no_sequence() {
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireRequest);
    let bad = CaptureTime {
        wall: "yesterday".into(),
        mono_ns: 0,
        clock_id: DAEMON_CLOCK.into(),
    };
    assert_eq!(
        p.redactor.flush(bad).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    for i in 0..3 {
        p.redactor.flush(daemon_time(i)).unwrap();
    }
    // The producer's next sequence number is still the one after its Open.
    assert!(
        p.try_push(
            1,
            FrameEvent::Bytes {
                offset: 0,
                data: b"x".to_vec()
            }
        )
        .is_ok()
    );
}

// ── Hermes lane A, S9: masked keys at the hold bound ──────────────────────────────────

const MASKED_FORMS: &[&str] = &[
    "sk-ab***wxyz",
    "sk-abcdefgh********wxyz",
    "sk-abcde...wxyz",
    "sk-abcde\u{2026}wxyz",
    "sk-abcde\\u2026wxyz",
    "SK-ABCDE...WXYZ",
    "sk-0...cdef",
    "sk-or-v1...cdef",
    "sk-proj******************************************Ldy",
    "sk-R7aMV***************************************oKYo",
    "sk-R7aMV****************************************************************oKYo",
];

/// The same split-and-chunk oracle as the corpus, for each masked form on its own, in
/// the middle of ordinary bytes and at both ends of a stream: a masked key split across any
/// two pushes, or any three, is one redaction identical to the whole-record one.
#[test]
fn a_masked_key_split_across_any_chunk_boundary_is_one_redaction() {
    for form in MASKED_FORMS {
        // There is no left boundary: a mask is a mask after anything.
        for (input, masked) in [
            (form.as_bytes().to_vec(), true),
            (format!("a {form} b").into_bytes(), true),
            (format!("{form}{form}").into_bytes(), true),
            (format!("sk-{form} {form}x\n").into_bytes(), true),
            (format!("a-{form}").into_bytes(), true),
            (format!("ta{form}").into_bytes(), true),
            (format!("_{form}").into_bytes(), true),
            (format!("\n{form}").into_bytes(), true),
            (format!("\\n{form}").into_bytes(), true),
            (format!("\u{1b}[33m{form}").into_bytes(), true),
            (format!("\u{e9}{form}").into_bytes(), true),
            (format!("\u{300}{form}").into_bytes(), true),
        ] {
            let (want_bytes, want_spans) = legacy(&input);
            assert_eq!(
                want_spans.iter().any(|s| s.kind == "masked_key"),
                masked,
                "{form}: the oracle on {:?}",
                String::from_utf8_lossy(&input)
            );
            for first in 0..=input.len() {
                // Every pair of cuts for a short input; for a long one, every first cut and
                // the second at the places a decision can change.
                let seconds: Vec<usize> = if input.len() <= 64 {
                    (first..=input.len()).collect()
                } else {
                    let mut v: Vec<usize> = [0, 1, 2, 3, 9, 33, 70]
                        .iter()
                        .map(|d| (first + d).min(input.len()))
                        .chain([input.len() / 2, input.len()])
                        .filter(|c| *c >= first)
                        .collect();
                    v.sort_unstable();
                    v.dedup();
                    v
                };
                for second in seconds {
                    let mut p = Producer::new(P1);
                    let mut records = p.open(1, CaptureSource::WireResponse);
                    for part in [&input[..first], &input[first..second], &input[second..]] {
                        records.extend(p.bytes(1, part));
                        records.extend(p.redactor.flush(daemon_time(1)).unwrap());
                    }
                    records.extend(p.end(1, EndReason::Complete));
                    let (bytes, spans) = retained_of(&records, 1);
                    assert_eq!(
                        (bytes, spans),
                        (want_bytes.clone(), want_spans.clone()),
                        "{form:?} in {:?} split at {first} and {second}",
                        String::from_utf8_lossy(&input)
                    );
                }
            }
        }
    }
}

/// The hold bound. A masked key that begins before, at or after the point where a long
/// undecided run (an email local part, which every byte keeps live) forces the stream to
/// fail closed is never retained: the stream may redact more than the whole-record policy
/// (here, the whole rest of the line), never less. Swept across the bound, at several
/// chunkings, with the key before the run, inside it and after it.
#[test]
fn a_masked_key_around_the_hold_bound_is_never_retained() {
    let h = MAX_HELD_BYTES;
    let mut rng = Rng(0x5EC2E7);
    let mut failures = Vec::new();
    for form in MASKED_FORMS {
        for pad in [0, 1, 2, 3, 5, 8, 13, 21, 34, 47, h - 40, h - 30] {
            for shape in 0..3 {
                // `abcdefghij.` repeated is one live local-part run that never decides.
                let run = |n: usize| {
                    b"abcdefghij."
                        .iter()
                        .copied()
                        .cycle()
                        .take(n)
                        .collect::<Vec<u8>>()
                };
                let mut input = Vec::new();
                match shape {
                    // The key first, then a run that reaches the bound.
                    0 => {
                        input.extend_from_slice(form.as_bytes());
                        input.extend(run(h - 40 + pad));
                    }
                    // The run reaches the bound and the key starts inside the pad.
                    1 => {
                        input.extend(run(h - 24 + pad));
                        input.push(b'.');
                        input.extend_from_slice(form.as_bytes());
                    }
                    // The key glued to a run on both sides (a `.` is a boundary and a
                    // local-part unit: the run stays live through it).
                    _ => {
                        input.extend(run(h - 30 + pad));
                        input.push(b'.');
                        input.extend_from_slice(form.as_bytes());
                        input.extend(run(64));
                    }
                }
                input.push(b'\n');
                let (_, want) = legacy(&input);
                assert!(
                    want.iter().any(|s| s.kind == "masked_key"),
                    "{form} pad {pad} shape {shape}: the oracle sees the key"
                );
                for chunks in [
                    vec![MAX_SEGMENT_BYTES],
                    random_chunks(&mut rng, 3000),
                    vec![1, 7, 64],
                ] {
                    let (bytes, got) = retained_of(
                        &stream_through(StreamRedactor::new(), &input, &chunks, 2),
                        1,
                    );
                    let missing = uncovered(&want, &got);
                    if let Some((kind, s, e)) = missing.first() {
                        failures.push(format!(
                            "{form} pad {pad} shape {shape}: {kind} {s}..{e} retained raw"
                        ));
                    }
                    let needle = form.as_bytes();
                    if bytes.windows(needle.len()).any(|w| w == needle) {
                        failures.push(format!(
                            "{form} pad {pad} shape {shape}: the key is in the retained bytes"
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A masked key whose tail never arrives is held and never released: a discontinuity
/// after the separator replaces what was held, an interrupt discards it.
#[test]
fn an_incomplete_masked_key_is_never_released_on_a_discontinuity() {
    for ending in ["owner-interrupt", "end-interrupted", "gap"] {
        let mut p = Producer::new(P1);
        let mut records = p.open(1, CaptureSource::WireResponse);
        records.extend(p.bytes(1, b"401: sk-abcde***wx"));
        records.extend(p.redactor.flush(daemon_time(5)).unwrap());
        assert_eq!(
            retained_of(&records, 1).0,
            b"401: ",
            "{ending}: only the safe prefix is released"
        );
        match ending {
            "owner-interrupt" => records.extend(
                p.redactor
                    .interrupt(daemon_time(6), GapReason::ProducerLost),
            ),
            "end-interrupted" => records.extend(p.end(1, EndReason::Interrupted)),
            _ => records.extend(p.gap(1, GapReason::QueueFull, None)),
        }
        records.extend(p.redactor.flush(daemon_time(7)).unwrap());
        let all = all_segment_bytes(&records);
        assert!(!contains(&all, b"sk-abcde"), "{ending}: {all:?}");
        assert!(!contains(&all, b"***wx"), "{ending}: {all:?}");
    }
}
