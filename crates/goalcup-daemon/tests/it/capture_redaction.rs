//! Bounded streaming redaction of protocol-v5 capture frames.
//!
//! The independent oracle for retained bytes is the legacy whole-record
//! `Redactor::new().apply` over the complete stream: while no held suffix reaches the
//! bound, streaming output must equal it at every chunking. Every credential below is
//! synthetic.

use base64::{Engine, engine::general_purpose::STANDARD};
use goalcup_daemon::redact::stream::{MAX_HELD_BYTES, StreamRedactor};
use goalcup_daemon::redact::{PEM_BODY_MAX, PEM_SLICE, Redactor};
use goalcup_protocol::capture::*;
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::redaction::RedactionAuditSpan;

use crate::common::redaction;
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

/// The run's proxy credential (H8) is a shared rule, so every connector's streams drop it. An
/// `env` dump in a tool result, a JSON environment snapshot, the URL percent-encoded and the
/// Basic token the harness derives from it: split at every byte, with a flush between the two
/// pushes, the secret never reaches a segment, the stream equals the whole-record oracle, and
/// the URL keeps its `@host`. A cut while the secret is held releases none of it.
#[test]
fn a_proxy_credential_split_at_any_byte_is_never_persisted() {
    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";
    // `base64("goalcup:" + SECRET)` without its `==`, and its secret-bearing tail.
    const BASIC: &[u8] = b"Z29hbGN1cDowMTIzNDU2Nzg5YWJjZGVmMDEyMzQ1Njc4OWFiY2RlZg";
    let forms: [(&[u8], &[u8]); 4] = [
        (
            b"$ env\nHOME=/tmp/run\nHTTPS_PROXY=http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123\nPATH=/usr/bin\n",
            b"@127.0.0.1:47123",
        ),
        (
            b"{\"type\":\"executor_environment_snapshot\",\"env\":{\"https_proxy\":\"http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123\"}}\n",
            b"@127.0.0.1:47123\"}}",
        ),
        (
            b"GET /x?proxy=http%3A%2F%2Fgoalcup%3A0123456789abcdef0123456789abcdef%40127.0.0.1%3A47123 HTTP/1.1\n",
            b"%40127.0.0.1",
        ),
        (
            b"proxy-authorization: Basic Z29hbGN1cDowMTIzNDU2Nzg5YWJjZGVmMDEyMzQ1Njc4OWFiY2RlZg==\n",
            b"Basic <redacted:proxy_credential>==",
        ),
    ];
    for (input, kept) in forms {
        let (want_bytes, want_spans) = legacy(input);
        assert!(
            want_spans.iter().any(|s| s.kind == "proxy_credential"),
            "the oracle names the rule on {:?}",
            String::from_utf8_lossy(input)
        );
        for split in 0..=input.len() {
            let mut p = Producer::new(P1);
            let mut records = p.open(1, CaptureSource::WireRequest);
            records.extend(p.bytes(1, &input[..split]));
            records.extend(p.redactor.flush(daemon_time(1)).unwrap());
            records.extend(p.bytes(1, &input[split..]));
            records.extend(p.end(1, EndReason::Complete));
            let (bytes, spans) = retained_of(&records, 1);
            assert_eq!(
                (&bytes, &spans),
                (&want_bytes, &want_spans),
                "split at {split} of {:?}",
                String::from_utf8_lossy(input)
            );
            for needle in [SECRET, &SECRET[..16], &SECRET[16..], BASIC, &BASIC[24..]] {
                assert!(
                    !contains(&bytes, needle),
                    "split at {split}: secret persisted"
                );
            }
            assert!(
                contains(&bytes, kept),
                "split at {split}: the rest of the URL stays"
            );
        }
    }

    // A cut while the credential is held: the owner interrupts, the producer ends the stream
    // interrupted, or a gap. The held prefix is past `goalcup:`, so none of it is released.
    for ending in ["owner-interrupt", "end-interrupted", "gap"] {
        let mut p = Producer::new(P1);
        let mut records = p.open(1, CaptureSource::WireResponse);
        records.extend(p.bytes(1, b"HTTPS_PROXY=http://goalcup:0123456789"));
        records.extend(p.bytes(1, b"abcdef0123"));
        records.extend(p.redactor.flush(daemon_time(5)).unwrap());
        assert_eq!(
            retained_of(&records, 1).0,
            b"HTTPS_PROXY=http://",
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
            !contains(&all, b"0123456789abcdef0123"),
            "{ending}: the held secret was persisted"
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
    b"github_pat_",
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
    // The proxy credential (H8): each form's fixed prefix, a near miss, and a URL's host.
    b"goalcup:",
    b"goalcup%3A",
    b"goalcup%3a",
    b"goalcup",
    b"Z29hbGN1cDo",
    b"Z29hbGN1cDp",
    b"@127.0.0.1:47123",
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
    b"\\\"",
    b"\\\"client_secret\\\":\\\"",
    b"token=\\'",
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
    match rng.below(20) {
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
        // The proxy credential (H8): each form at and either side of its fixed length.
        17 => {
            if rng.below(3) == 0 {
                t.extend_from_slice(pick(rng, &[b"Z29hbGN1cD", b"Z29hbGN1cE"]));
                t.push(b"opq"[rng.below(3)]);
                run(rng, &mut t, b"ABCMYZabcz0189+/", 40, 46);
                t.extend_from_slice(pick(rng, &[b"", b"==", b"\\/", b"="]));
            } else {
                t.extend_from_slice(pick(rng, &[b"http://", b"", b"\"", b"%2F%2F"]));
                t.extend_from_slice(pick(
                    rng,
                    &[
                        b"goalcup:",
                        b"goalcup%3A",
                        b"goalcup%3a",
                        b"goalcup%3",
                        b"Goalcup:",
                    ],
                ));
                run(rng, &mut t, b"0123456789abcdef", 29, 35);
                if rng.below(4) == 0 {
                    t.push(b"AFx-"[rng.below(4)]);
                }
                t.extend_from_slice(pick(rng, &[b"@127.0.0.1:47123", b"%40", b"@", b""]));
            }
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
    // The proxy credential (H8): an `env` dump, a JSON environment snapshot, the URL
    // percent-encoded, the Basic token, and near misses (31 hex, upper case, another user).
    c.extend_from_slice(
        b"HTTPS_PROXY=http://goalcup:0123456789abcdef0123456789abcdef@127.0.0.1:47123\n",
    );
    c.extend_from_slice(
        b"{\"env\":{\"https_proxy\":\"http://goalcup:fedcba9876543210fedcba9876543210@[::1]:9\"}}\n",
    );
    c.extend_from_slice(
        b"proxy=http%3A%2F%2Fgoalcup%3a00112233445566778899aabbccddeeff%40localhost%3A1 \
          Proxy-Authorization: Basic Z29hbGN1cDowMTIzNDU2Nzg5YWJjZGVmMDEyMzQ1Njc4OWFiY2RlZg==\n",
    );
    c.extend_from_slice(
        b"goalcup:0123456789abcdef0123456789abcde@x goalcup:0123456789ABCDEF0123456789ABCDEF \
          other:0123456789abcdef0123456789abcdef@127.0.0.1\n",
    );
    c.extend_from_slice(
        b"binary \xff\xfe\x00\x01 sk-ant-\xffABCDEFGHIJKLMNOPQRSTUVWXYZ \xe2\x82\xac end",
    );
    c
}

shards!(every_split_point_matches_whole_record_policy, 2);
fn every_split_point_matches_whole_record_policy(shard: &Shard) {
    let input = corpus();
    let (want_bytes, want_spans) = legacy(&input);
    assert!(
        want_spans.len() >= 20,
        "the corpus exercises the rule set: {}",
        want_spans.len()
    );
    for split in 0..=input.len() {
        if !shard.mine() {
            continue;
        }
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

shards!(seeded_random_chunking_matches_whole_record_policy, 2);
fn seeded_random_chunking_matches_whole_record_policy(shard: &Shard) {
    let seeds: u64 = std::env::var("T04_FUZZ_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    // Each seed draws its input and chunkings from its own generator.
    for seed in 1..=seeds {
        if !shard.mine() {
            continue;
        }
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
    // A JSON key is at most 2,008 units, so a key longer than the hold is no key: only a match
    // inside the run is left to redact. A key at the cap is one, and its value past the hold
    // fails closed.
    cases.push((
        "PREFIX JSON key at the cap".into(),
        cat(&[
            b"{\"",
            &long(b"abcdefghijklmnopqrstuvwxyz", 999),
            b"_api_key\": \"",
            &long(b"QZqz0123456789", h + 4096),
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
                            b"github_pat_",
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

shards!(
    adversarial_long_inputs_never_expose_a_whole_record_secret,
    2
);
fn adversarial_long_inputs_never_expose_a_whole_record_secret(shard: &Shard) {
    let seeds: u64 = std::env::var("T04_ADVERSARIAL_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let mut failures = Vec::new();
    let mut spans = 0usize;
    for seed in 1..=seeds {
        if !shard.mine() {
            continue;
        }
        let mut rng = Rng(seed.wrapping_mul(0xA076_1D64_78BD_642F) | 1);
        let input = adversarial_input(&mut rng);
        spans += legacy(&input).1.len();
        failures.extend(never_less(&format!("seed {seed}"), &input, &mut rng));
    }
    // A part that took no seed (fewer seeds than parts) has nothing to have found.
    let taken = shard.taken();
    assert!(
        taken == 0 || spans > 0,
        "{}: {taken} seeds drew no secret span",
        shard.label()
    );
    assert!(
        failures.is_empty(),
        "{}: {} of its {taken} seeds:\n{}",
        shard.label(),
        failures.len(),
        failures.join("\n")
    );
}

shards!(seeded_long_inputs_never_expose_a_whole_record_secret, 6);
/// Seeded inputs past the hold bound with the same directional oracle. Every long run is
/// closed by `,`, so no whole-record match can begin inside one and run past the hold
/// (the documented unproven-prefix limits); runs may be glued to what precedes them.
fn seeded_long_inputs_never_expose_a_whole_record_secret(shard: &Shard) {
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
        b"github_pat_",
        b"dop_v1_",
    ];
    let mut failures = Vec::new();
    for seed in 1..=seeds {
        if !shard.mine() {
            continue;
        }
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
        "{}: {} of its {} seeds:\n{}",
        shard.label(),
        failures.len(),
        shard.taken(),
        failures.join("\n")
    );
}

shards!(seeded_long_inputs_are_chunking_invariant, 6);
/// Seeded inputs far longer than the hold bound, with long runs of every class a
/// candidate can grow through, streamed at very different chunkings: forced decisions
/// must be the same wherever the frame boundaries fall, and so must a discontinuity.
fn seeded_long_inputs_are_chunking_invariant(shard: &Shard) {
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
            let chunkings = [
                random_chunks(&mut rng, 40),
                random_chunks(&mut rng, 5000),
                vec![1, 2, 3, 5, 8, 13],
            ];
            if !shard.mine() {
                continue;
            }
            let reference = whole(ending, &[MAX_SEGMENT_BYTES]);
            for chunks in chunkings {
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

    // A PEM block is at most `PEM_BODY_MAX` bytes after its BEGIN line, so a longer body is the
    // bounded block and then text: the stream equals the whole-record policy.
    let mut pem = b"key follows: -----BEGIN RSA PRIVATE KEY-----\nSYNTHETICBODYMARK".to_vec();
    let body = lower(&mut rng, 40_000, b64ish);
    pem.extend_from_slice(&body);
    pem.extend_from_slice(b"\n-----END RSA PRIVATE KEY-----\ntrailer");
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

    // A local part and a JSON key are bounded (1,024 and 2,008 units), so a longer run is never
    // held past the bound: the stream equals the whole-record policy.
    let mut email = lower(&mut rng, 40_000, b"abcdefghijklmnopqrstuvwxyz0123456789.");
    email.extend_from_slice(b"@example.com and more");
    cases.push(("email-run", email, true, b""));

    let mut key = b"{\"".to_vec();
    key.extend_from_slice(&lower(&mut rng, 20_000, b"abcdefghijklmnopqrstuvwxyz"));
    key.extend_from_slice(b"_token\": \"abcdefghijklmnopqrstuvwxyz\"}");
    cases.push(("json-long-key", key, true, b""));

    // A long unproven prefix: released, so only chunking invariance applies.
    let mut spaced = b"bearer".to_vec();
    spaced.extend(std::iter::repeat_n(b' ', 20_000));
    spaced.extend_from_slice(b"abcdefghijklmnopqrstuvwxyz end");
    cases.push(("bearer-spaces", spaced, false, b""));

    // A PEM block with no END line and no quote: replaced for `PEM_BODY_MAX` bytes, not to the
    // end of the stream.
    let mut open_pem = b"-----BEGIN PRIVATE KEY-----\nSYNTHETICOPENPEMMARK".to_vec();
    open_pem.extend_from_slice(&lower(&mut rng, 30_000, b64ish));
    cases.push(("pem-unterminated", open_pem, true, b"SYNTHETICOPENPEMMARK"));

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

/// The same at the PEM bound: an END line must start within `PEM_BODY_MAX` bytes of the BEGIN line,
/// and without one the block is those units (I-1), which fit the hold. A block with a claim
/// inside it is one redaction; a block whose END comes later, and a key with no END, are the
/// bounded block with the claim inside it, and the text after the bound stays. The stream
/// equals the whole-record policy at every chunking.
#[test]
fn a_pem_block_with_a_claim_inside_is_one_redaction_at_the_pem_bound() {
    const BEGIN: &str = "-----BEGIN RSA PRIVATE KEY-----\n";
    const END: &str = "-----END RSA PRIVATE KEY-----";
    const BOUND: usize = PEM_BODY_MAX;
    let mut rng = Rng(0xB10C);
    let filler = |rng: &mut Rng, n: usize| -> String {
        let set = b"abcdefghijklmnopqrstuvwxyz0123456789/";
        (0..n).map(|_| set[rng.below(set.len())] as char).collect()
    };
    let token = "ghp_AAAAAAAAAAAAAAAAAAAA";
    let h = MAX_HELD_BYTES;
    // The block starts after `log `; its body starts at the BEGIN line's newline.
    let body_at = "log ".len() + BEGIN.len() - 1;
    let bounded = body_at + BOUND;
    // `(name, input, the end of the private_key span that starts the record)`.
    let mut cases: Vec<(String, Vec<u8>, usize)> = Vec::new();
    // The END starts at body unit `before + token + after + 4` (four newlines precede it).
    let at_bound = BOUND - token.len() - 4 - 100;
    for (name, before, after, whole) in [
        ("fits the hold", BOUND / 2 - 300, BOUND / 2 - 300, true),
        ("END starts at the bound", at_bound, 100, true),
        ("END starts past the bound", at_bound + 1, 100, false),
        ("claim first, END past the hold", 0, h + 4096, false),
    ] {
        let block = format!(
            "{BEGIN}{}\n{token}\n{}\n{END}",
            filler(&mut rng, before),
            filler(&mut rng, after)
        );
        let end = if whole { 4 + block.len() } else { bounded };
        cases.push((
            name.to_string(),
            format!("log {block}\ntrailer").into_bytes(),
            end,
        ));
    }
    // A BEGIN with no END, a claim, more than the hold of text, then a whole second key.
    let run = format!(
        "{BEGIN}{}\n{token}\n{}\n{BEGIN}{}\n{END}",
        filler(&mut rng, 100),
        filler(&mut rng, h + 4096),
        filler(&mut rng, 200)
    );
    cases.push((
        "unterminated begin, claim, then a later key".to_string(),
        format!("log {run}\ntrailer").into_bytes(),
        bounded,
    ));
    // Keys with no END at all: to the end of the stream within the bound, else the bound.
    for (name, before, after) in [
        (
            "truncated, fits the bound",
            BOUND / 2 - 300,
            BOUND / 2 - 300,
        ),
        ("truncated, claim first", 0, h + 4096),
    ] {
        let input = format!(
            "log {BEGIN}{}\n{token}\n{}",
            filler(&mut rng, before),
            filler(&mut rng, after)
        );
        let end = input.len().min(bounded);
        cases.push((name.to_string(), input.into_bytes(), end));
    }

    for (name, input, end) in &cases {
        let want = legacy(input);
        let (kind, start, block_end, parts) = block_run(&want.1);
        assert_eq!(
            (kind, start, block_end),
            ("private_key", 4, *end as u64),
            "{name}: the block"
        );
        // A block with no END is cut once, at `PEM_SLICE` past its BEGIN.
        assert!(parts <= 2, "{name}: {parts} parts");
        assert!(
            !want.1[parts..].iter().any(|s| s.start < *end as u64),
            "{name}: one redaction for the block"
        );
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
            assert!(
                !contains(&got.0, token.as_bytes()),
                "{name}: the claim is retained"
            );
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

    // Held state per stream is bounded: an undecided run far longer than the bound (a key
    // body after its prefix) fails closed instead of being held, and each record terminator
    // hands the stream back, so pushed minus accounted source never exceeds the hold bound.
    let mut p = Producer::new(P1);
    p.open(1, CaptureSource::WireResponse);
    let mut pushed = 0usize;
    let mut records = Vec::new();
    let mut line = b"sk-ant-".to_vec();
    line.resize(MAX_SEGMENT_BYTES - 1, b'a');
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

/// A refresh token in the body of a request to a token endpoint on an opened host never reaches a
/// segment in the clear. The proxy records every request on an opened host and hands the body over
/// unchanged, so this redactor is the only thing between the token and the chain. Checked on the
/// decoded segment bytes (`all_segment_bytes`), not on the base64 text, and not vacuously: the
/// body's other key is retained.
#[test]
fn a_token_in_a_body_on_an_opened_host_is_masked_in_the_chain() {
    const TOKEN: &[u8] = b"rt-0123456789abcdefghij";
    let body = br#"{"grant_type":"refresh_token","refresh_token":"rt-0123456789abcdefghij"}"#;
    let mut p = Producer::new(P1);
    let mut records = p.open(1, CaptureSource::WireRequest);
    records.extend(p.bytes(1, body));
    records.extend(p.end(1, EndReason::Complete));
    let retained = all_segment_bytes(&records);
    assert!(
        contains(&retained, b"grant_type"),
        "the body was retained: {}",
        String::from_utf8_lossy(&retained)
    );
    assert!(
        !contains(&retained, TOKEN),
        "the refresh token reached a segment in the clear: {}",
        String::from_utf8_lossy(&retained)
    );
}

/// A PEM body far past the bound (one MiB) never grows the hold: its first `PEM_BODY_MAX` bytes are
/// the block, and the rest leaves as it streams past, exactly as the whole-record policy
/// redacts it (I-1).
#[test]
fn long_private_key_does_not_grow_pending_buffer() {
    let mut p = Producer::new(P1);
    let mut records = p.open(1, CaptureSource::WireResponse);
    let lead = b"{\"content\":\"here is the key ";
    let begin = b"-----BEGIN OPENSSH PRIVATE KEY-----\n";
    let line = b"b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAABlwAAAAdzc2gtcn\n";
    let tail = b"-----END OPENSSH PRIVATE KEY-----\n\"}";
    let mut input = Vec::new();
    let mut pushed = 0usize;
    let accounted = |records: &[CaptureRecord]| -> usize {
        records
            .iter()
            .filter_map(|r| match r {
                CaptureRecord::Segment { source_len, .. } => Some(*source_len as usize),
                _ => None,
            })
            .sum()
    };
    for (i, chunk) in [lead.to_vec(), begin.to_vec()]
        .into_iter()
        .chain((0..16).map(|_| {
            line.iter()
                .copied()
                .cycle()
                .take(MAX_SEGMENT_BYTES)
                .collect()
        }))
        .chain([tail.to_vec()])
        .enumerate()
    {
        records.extend(p.bytes(1, &chunk));
        records.extend(p.redactor.flush(daemon_time(i as u64)).unwrap());
        pushed += chunk.len();
        input.extend_from_slice(&chunk);
        assert!(
            pushed - accounted(&records) <= MAX_HELD_BYTES,
            "held {} bytes after {pushed} pushed",
            pushed - accounted(&records)
        );
    }
    records.extend(p.end(1, EndReason::Complete));
    let (bytes, spans) = retained_of(&records, 1);
    let key_start = lead.len() as u64;
    assert_eq!(
        block_run(&spans),
        (
            "private_key",
            key_start,
            key_start + (begin.len() - 1 + PEM_BODY_MAX) as u64,
            2
        )
    );
    assert_eq!(spans[1].start, key_start + PEM_SLICE as u64);
    assert_eq!((bytes, spans), legacy(&input));

    // Interrupted mid-key, the undecided key is dropped: the retained source range ends where
    // the key begins, so a resume from the durable end re-reads the key from its start, and
    // the gap accounts for the rest.
    let mut q = Producer::new(P1);
    q.open(1, CaptureSource::WireResponse);
    q.bytes(1, lead);
    q.bytes(1, begin);
    let body: Vec<u8> = line.iter().copied().cycle().take(4096).collect();
    q.bytes(1, &body);
    let cut = q
        .redactor
        .interrupt(daemon_time(99), GapReason::ProducerLost);
    let (kept, spans) = retained_of(&cut, 1);
    assert_eq!((kept.as_slice(), spans.len()), (lead.as_slice(), 0));
    assert_eq!(accounted(&cut) as u64, key_start);
    assert!(matches!(
        cut.last(),
        Some(CaptureRecord::Gap {
            lost_bytes: None,
            ..
        })
    ));
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

shards!(
    a_masked_key_split_across_any_chunk_boundary_is_one_redaction,
    3
);
/// The same split-and-chunk oracle as the corpus, for each masked form on its own, in
/// the middle of ordinary bytes and at both ends of a stream: a masked key split across any
/// two pushes, or any three, is one redaction identical to the whole-record one.
fn a_masked_key_split_across_any_chunk_boundary_is_one_redaction(shard: &Shard) {
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
            if !shard.mine() {
                continue;
            }
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

shards!(a_masked_key_around_the_hold_bound_is_never_retained, 8);
/// The hold bound. A masked key that begins before, at or after the point where a long
/// undecided run (an email local part, which every byte keeps live) forces the stream to
/// fail closed is never retained: the stream may redact more than the whole-record policy
/// (here, the whole rest of the line), never less. Swept across the bound, at several
/// chunkings, with the key before the run, inside it and after it.
fn a_masked_key_around_the_hold_bound_is_never_retained(shard: &Shard) {
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
                // Drawn for every case, so each part sees the chunkings the unsplit loop saw.
                let chunkings = [
                    vec![MAX_SEGMENT_BYTES],
                    random_chunks(&mut rng, 3000),
                    vec![1, 7, 64],
                ];
                if !shard.mine() {
                    continue;
                }
                let (_, want) = legacy(&input);
                assert!(
                    want.iter().any(|s| s.kind == "masked_key"),
                    "{form} pad {pad} shape {shape}: the oracle sees the key"
                );
                for chunks in chunkings {
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

/// I-2: a Codex native rollout carries `encrypted_content` on every reasoning item, a
/// base64url run with no quote or `@` in it. Past 16 KiB that run used to hold a candidate
/// (a quoted JSON key, an email local part) to the hold bound and fail closed, which replaced
/// the rest of the line with a marker and left a record that was no longer JSON. The measured
/// shape: the largest reasoning line of a committed Codex rollout, its token repeated to
/// 18 KiB. It contains no secret, so it streams through byte for byte at every chunking.
#[test]
fn a_long_codex_reasoning_item_streams_through_intact() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../spikes/codex-app/fixtures/rollout-main-01a06657.jsonl");
    let rollout = std::fs::read_to_string(path).unwrap();
    let (line, token) = rollout
        .lines()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let token = v["payload"]["encrypted_content"].as_str()?.to_string();
            Some((line, token))
        })
        .max_by_key(|(_, token)| token.len())
        .expect("a reasoning item with encrypted_content");
    assert_eq!(token.len(), 2276, "the measured largest token");
    let long = format!("{}=", token.trim_end_matches('=').repeat(8));
    assert!(long.len() > MAX_HELD_BYTES);
    let input = format!(
        "{}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\"}}}}\n",
        line.replace(&token, &long)
    );
    let input = input.as_bytes();
    assert_eq!(legacy(input), (input.to_vec(), Vec::new()));
    let mut rng = Rng(0xC0DE_C0DE);
    for chunks in [
        vec![MAX_SEGMENT_BYTES],
        vec![1],
        vec![4096],
        random_chunks(&mut rng, 3000),
    ] {
        let (bytes, spans) =
            retained_of(&stream_through(StreamRedactor::new(), input, &chunks, 2), 1);
        assert!(spans.is_empty(), "chunks {:?}: {spans:?}", &chunks[..1]);
        assert!(
            bytes == input,
            "chunks {:?}: the record changed",
            &chunks[..1]
        );
        for record in bytes.split(|&b| b == b'\n').filter(|r| !r.is_empty()) {
            serde_json::from_slice::<serde_json::Value>(record).expect("each record is JSON");
        }
    }
}

/// I-1, then H-20: a lone PEM `BEGIN` (an agent writing a PEM parser, `grep` output, a key cut
/// off by an output limit) with no END used to redact the rest of the stream, a whole native
/// session, because a capture stream is one text and the block ran to its end; then the next
/// 12,288 bytes as one span, which broke the JSON of every record it reached. The measured
/// shape: one JSONL record mentioning `BEGIN`, then 2,000 ordinary records. The block is now at
/// most `PEM_BODY_MAX` bytes, and only the string interiors inside it go: every record parses,
/// the JSON between the strings stays, and every byte past the bound is kept. The stream equals
/// the whole-record policy.
#[test]
fn a_lone_pem_begin_redacts_at_most_the_pem_bound() {
    let mut input =
        br#"{"type":"msg","text":"if line.starts_with(\"-----BEGIN PRIVATE KEY-----\") {"}"#
            .to_vec();
    input.push(b'\n');
    let mut later = Vec::new();
    for i in 0..2000 {
        later.extend_from_slice(
            format!(
                "{{\"type\":\"msg\",\"n\":{i},\"text\":\"ordinary transcript line number {i}\"}}\n"
            )
            .as_bytes(),
        );
    }
    input.extend_from_slice(&later);
    let begin_at = 44;
    let block_end = begin_at + "-----BEGIN PRIVATE KEY-----".len() + PEM_BODY_MAX;
    let want = legacy(&input);
    assert_eq!(want.1[0].start, begin_at as u64);
    assert!(want.1.len() > 100, "one claim per string interior");
    assert!(
        want.1
            .iter()
            .all(|s| s.kind == "private_key" && s.end as usize <= block_end),
        "every claim is a piece of the block"
    );
    for span in &want.1 {
        let piece = &input[span.start as usize..span.end as usize];
        assert!(!piece.contains(&b'"') || piece.windows(2).any(|w| w == b"\\\""));
    }
    let mut rng = Rng(0x0B_E61A);
    for chunks in [
        vec![MAX_SEGMENT_BYTES],
        vec![1],
        random_chunks(&mut rng, 3000),
    ] {
        let got = retained_of(
            &stream_through(StreamRedactor::new(), &input, &chunks, 2),
            1,
        );
        assert_eq!(got, want, "chunks {:?}", &chunks[..1]);
        // Everything from the first record boundary after the block is kept as it was, and
        // every record parses.
        let kept_from =
            block_end + input[block_end..].iter().position(|&b| b == b'\n').unwrap() + 1;
        assert!(
            got.0.ends_with(&input[kept_from..]),
            "chunks {:?}: the records after the bound were lost ({} of {} bytes kept)",
            &chunks[..1],
            got.0.len(),
            input.len()
        );
        for record in got.0.split(|&b| b == b'\n').filter(|r| !r.is_empty()) {
            serde_json::from_slice::<serde_json::Value>(record).expect("each record is JSON");
        }
        assert!(input.len() - kept_from > 100_000);
    }
}

/// I-4 in the stream: a generic secret in JSON-escaped text (tool output inside a native
/// record) is redacted at every split point exactly as the whole-record policy redacts it.
#[test]
fn an_escaped_json_secret_is_redacted_at_every_split_point() {
    let input: &[u8] = br#"{"type":"function_call_output","output":"{\n  \"api_key\": \"q8Zr2LmN0pX4vB7cT1yW\",\n  \"client_secret\": \"Zx9Yw8Vu7Ts6Rq5Po4Nm\"\n}"}
{"type":"function_call_output","output":"db:\n  password: \"CorrectHorseBattery9x\"\n"}
{"type":"function_call","arguments":"{\"cmd\":[\"curl\",\"-d\",\"{\\\"api_key\\\":\\\"Q2w3E4r5T6y7U8i9O0pA\\\"}\"]}"}
"#;
    let (want_bytes, want_spans) = legacy(input);
    assert_eq!(want_spans.len(), 4, "{want_spans:?}");
    for secret in [
        &b"q8Zr2LmN0pX4vB7cT1yW"[..],
        b"Zx9Yw8Vu7Ts6Rq5Po4Nm",
        b"CorrectHorseBattery9x",
        b"Q2w3E4r5T6y7U8i9O0pA",
    ] {
        assert!(!contains(&want_bytes, secret));
    }
    for split in 0..=input.len() {
        let mut p = Producer::new(P1);
        let mut records = p.open(1, CaptureSource::Native);
        records.extend(p.bytes(1, &input[..split]));
        records.extend(p.redactor.flush(daemon_time(1)).unwrap());
        records.extend(p.bytes(1, &input[split..]));
        records.extend(p.end(1, EndReason::Complete));
        let (bytes, spans) = retained_of(&records, 1);
        assert_eq!(spans, want_spans, "audit differs when split at {split}");
        assert_eq!(bytes, want_bytes, "bytes differ when split at {split}");
    }
    for line in want_bytes.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
        serde_json::from_slice::<serde_json::Value>(line).expect("each record is JSON");
    }
}

/// I-2's bounds are long enough for real shapes: a JSON key with a long compound prefix, and a
/// URL credential, which the `email` rule sees as a local part (`git remote -v` prints
/// Bitbucket `x-token-auth` tokens of about 190 characters, Azure DevOps tokens of 84). Each
/// is redacted whole, and the stream equals the whole-record policy at every chunking.
#[test]
fn long_secret_keys_and_url_credentials_are_redacted_whole() {
    let token = |n: usize| -> String {
        let set = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        (0..n)
            .map(|i| set[(i * 7 + 3) % set.len()] as char)
            .collect()
    };
    let value = "abcdefghijklmnopqrstuvwxyz012345";
    let cases: Vec<(String, String)> = vec![
        (
            format!("{{\"{}_api_key\": \"{value}\"}}\n", "a".repeat(61)),
            value.to_string(),
        ),
        (
            format!("{{\"{}_api_key\": \"{value}\"}}\n", "a".repeat(999)),
            value.to_string(),
        ),
        (
            format!("curl https://user:{}@artifacts.example.com/x\n", token(81)),
            token(81),
        ),
        (
            format!(
                "origin\thttps://x-token-auth:{}@bitbucket.org/team/repo.git (fetch)\n",
                token(190)
            ),
            token(190),
        ),
    ];
    let mut rng = Rng(0x0C_A9);
    for (input, secret) in &cases {
        let input = input.as_bytes();
        let want = legacy(input);
        // No 16 characters of the secret survive.
        for w in secret.as_bytes().windows(16) {
            assert!(
                !contains(&want.0, w),
                "{input:?}: {:?}",
                String::from_utf8_lossy(&want.0)
            );
        }
        for chunks in [
            vec![MAX_SEGMENT_BYTES],
            vec![1],
            random_chunks(&mut rng, 64),
        ] {
            let got = retained_of(&stream_through(StreamRedactor::new(), input, &chunks, 2), 1);
            assert_eq!(got, want, "{:?}", String::from_utf8_lossy(input));
        }
    }
}

/// I-1 review: Claude Code's Read tool prints a file with `N→` (U+2192) before every line, so a
/// key read from disk reaches the native record as non-ASCII text between its BEGIN and END
/// lines, JSON-escaped. The block is bounded in bytes, not ASCII characters, so it still runs
/// to its END line: one redaction, at every split point, as the whole-record policy has it.
#[test]
fn a_key_printed_by_the_read_tool_is_one_redaction() {
    let lines = [
        "-----BEGIN RSA PRIVATE KEY-----",
        "MIIEowIBAAKCAQEAu1SU1LfVLPHCozMxH2Mo4lgOEePzNm0tRgeLezV6ffAt0gunVTLw",
        "7onLRnrq0/IzW7yWR7QkrmBL7jTKEn5u+qKhbwKfBstIs+bMY2Zkp18gnTxKLxoS2tFc",
        "zGkPLPgizskuemMghRniWaoLcyehkd3qqGElvW/VDL5AaWTg0nLVkjRo9z+40RQzuVaE",
        "8AkAFmxZzow3x+VJYKdjykkJ0iT9wCS0DRTXu269V264Vf/3jvredZiKRkgwlL9xNAwx",
        "-----END RSA PRIVATE KEY-----",
    ];
    let numbered: Vec<String> = (1..)
        .zip(lines)
        .map(|(n, line)| format!("{n:>6}\u{2192}{line}"))
        .collect();
    let input = format!(
        "{{\"type\":\"tool_result\",\"content\":\"{}\"}}\n{{\"next\":1}}\n",
        numbered.join("\\n")
    );
    let input = input.as_bytes();
    let (want_bytes, want_spans) = legacy(input);
    assert_eq!(
        String::from_utf8(want_bytes.clone()).unwrap(),
        "{\"type\":\"tool_result\",\"content\":\"     1\u{2192}<redacted:private_key>\"}\n{\"next\":1}\n"
    );
    assert_eq!(want_spans.len(), 1);
    for split in 0..=input.len() {
        let mut p = Producer::new(P1);
        let mut records = p.open(1, CaptureSource::Native);
        records.extend(p.bytes(1, &input[..split]));
        records.extend(p.redactor.flush(daemon_time(1)).unwrap());
        records.extend(p.bytes(1, &input[split..]));
        records.extend(p.end(1, EndReason::Complete));
        let got = retained_of(&records, 1);
        assert_eq!(
            got,
            (want_bytes.clone(), want_spans.clone()),
            "split at {split}"
        );
    }
}

/// A bounded block that ends inside a multi-byte character takes the whole character: the
/// marker never leaves half of one behind, and the stream agrees at every chunking.
#[test]
fn a_pem_bound_inside_a_character_takes_the_character() {
    let begin = "-----BEGIN PRIVATE KEY-----";
    for pad in 0..4 {
        let mut input = format!("x {begin}");
        input.push_str(&"a".repeat(PEM_BODY_MAX - 1 - pad));
        input.push_str(&"\u{1F511}".repeat(4));
        input.push_str(" after\n");
        let input = input.as_bytes();
        let (bytes, spans) = legacy(input);
        let text = String::from_utf8(bytes.clone()).expect("the marker cut no character");
        assert!(text.starts_with("x <redacted:private_key>"), "pad {pad}");
        assert!(text.ends_with(" after\n"), "pad {pad}");
        let mut rng = Rng(0x1F511 + pad as u64);
        for chunks in [
            vec![MAX_SEGMENT_BYTES],
            vec![1],
            random_chunks(&mut rng, 64),
        ] {
            let got = retained_of(&stream_through(StreamRedactor::new(), input, &chunks, 2), 1);
            assert_eq!(got, (bytes.clone(), spans.clone()), "pad {pad}");
        }
    }
}

// ── H-20: invalid UTF-8 inside a key body, and a lone BEGIN ─────────────────────────────

/// Run `spikes/redact.py` over `input`, bytes in and bytes out, with its per-match digest
/// stripped (`<redacted:KIND:0123abcd>` becomes `<redacted:KIND>`); `None` without `python3`.
fn redact_py(input: &[u8]) -> Option<Vec<u8>> {
    let script =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spikes/redact.py");
    let dir = tempfile::tempdir().unwrap();
    let (src, dst) = (dir.path().join("in"), dir.path().join("out"));
    std::fs::write(&src, input).unwrap();
    match std::process::Command::new("python3")
        .arg("-B")
        .arg(&script)
        .arg(&src)
        .arg(&dst)
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) => assert!(status.success(), "redact.py failed"),
        Err(e) => {
            eprintln!("python3 is not available ({e}); the script half is skipped");
            return None;
        }
    }
    let out = std::fs::read(&dst).unwrap();
    let digest = regex::bytes::Regex::new(r"<redacted:([A-Za-z0-9_\-]+):[0-9a-f]{8}>").unwrap();
    Some(digest.replace_all(&out, &b"<redacted:$1>"[..]).into_owned())
}

/// The stream at three chunkings, the whole-record `Redactor` and `spikes/redact.py` all give
/// the same bytes for `input`; the whole-record result. A shape the script does not carry (the
/// vendor prefixes it predates) is checked against the first two only.
fn all_three_agree(name: &str, input: &[u8]) -> (Vec<u8>, Vec<RedactionAuditSpan>) {
    agree(name, input, true)
}

fn agree(name: &str, input: &[u8], script_has_it: bool) -> (Vec<u8>, Vec<RedactionAuditSpan>) {
    let want = legacy(input);
    let mut rng = Rng(0x4_20 + input.len() as u64);
    for chunks in [
        vec![MAX_SEGMENT_BYTES],
        vec![1],
        random_chunks(&mut rng, 97),
    ] {
        let got = retained_of(&stream_through(StreamRedactor::new(), input, &chunks, 2), 1);
        assert_eq!(
            got,
            want,
            "{name}: the stream differs at {:?}",
            &chunks[..1]
        );
    }
    if let Some(script) = redact_py(input).filter(|_| script_has_it) {
        assert_eq!(
            String::from_utf8_lossy(&script),
            String::from_utf8_lossy(&want.0),
            "{name}: spikes/redact.py differs"
        );
        assert_eq!(script, want.0, "{name}: spikes/redact.py differs in bytes");
    }
    want
}

/// No eight-byte run of `secret` is in `out`.
fn no_run_of(out: &[u8], secret: &[u8]) -> Option<String> {
    secret
        .windows(8)
        .find(|w| contains(out, w))
        .map(|w| String::from_utf8_lossy(w).into_owned())
}

/// H-20: a byte that is not valid UTF-8 inside a key's body used to end the key there, so the
/// part after it, or the whole key when the part before it was too short for the rule, stayed
/// raw. The rules now read an invalid byte as `0`, a character every key body class takes:
/// the key is one match. For each known key shape, an invalid byte (a lone `0xFF`, a lone lead
/// byte, a surrogate's three bytes, a cut-off sequence) early in the body: no eight-byte run of
/// the key is retained, and the stream, the whole-record policy and `spikes/redact.py` agree.
#[test]
fn a_key_with_invalid_utf8_in_its_body_is_still_redacted() {
    let alnum = |n: usize| -> Vec<u8> {
        (0..n)
            .map(|i| b"Ab3dEf7hJk9mNp2qRs5tUv8wXy"[i % 26])
            .collect()
    };
    let mut keys: Vec<(&str, Vec<u8>)> = Vec::new();
    let mut key = |name: &'static str, parts: &[&[u8]]| keys.push((name, parts.concat()));
    // Shapes `spikes/redact.py` predates: the daemon's own rules.
    let daemon_only = ["google_key", "gitlab_token"];
    key(
        "anthropic_key",
        &[b"sk-ant-api03-Ab3dE", b"\xff", &alnum(35)],
    );
    key("openai_key", &[b"sk-proj-Ab3", b"\xff", &alnum(40)]);
    key("github_token", &[b"ghp_Ab3", b"\xff", &alnum(33)]);
    key("github_pat", &[b"github_pat_Ab3dE", b"\xff", &alnum(70)]);
    key("aws_access_key", &[b"AKIAIOSF", b"\xff", b"DNN7EXAMPLE"]);
    key(
        "slack_token",
        &[b"xoxb-12", b"\xff", b"34567890-ABCDEFGHIJ"],
    );
    key("google_key", &[b"AIzaSyAb3", b"\xff", &alnum(30)]);
    key("gitlab_token", &[b"glpat-Ab3", b"\xff", &alnum(20)]);
    key(
        "jwt",
        &[
            b"eyJhbGciOiJIUzI1",
            b"\xff",
            b"NiJ9.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4",
        ],
    );
    key(
        "proxy_credential",
        &[b"goalcup:0123a", b"\xff", b"56789abcdef0123456789abcdef"],
    );
    key(
        "lone lead byte",
        &[b"sk-ant-api03-Ab3dE", b"\xc3", &alnum(35)],
    );
    key(
        "surrogate",
        &[b"sk-ant-api03-Ab3dE", b"\xed\xa0\x80", &alnum(35)],
    );
    key(
        "cut-off sequence",
        &[b"sk-ant-api03-Ab3dE", b"\xe2\x82", &alnum(35)],
    );
    key(
        "pem body",
        &[
            b"-----BEGIN PRIVATE KEY-----\\nMIIEvQIBADANBgkq",
            b"\xff",
            b"hkiG9w0BAQEFAASC\\n-----END PRIVATE KEY-----",
        ],
    );
    for (name, secret) in &keys {
        let input = [
            &br#"{"type":"tool_result","content":"found "#[..],
            secret,
            &b" in the file\"}\n{\"type\":\"next\"}\n"[..],
        ]
        .concat();
        let (out, spans) = agree(name, &input, !daemon_only.contains(name));
        assert_eq!(spans.len(), 1, "{name}: one redaction");
        if let Some(run) = no_run_of(&out, secret) {
            panic!("{name}: {run:?} of the key is retained");
        }
        assert!(
            contains(&out, b" in the file\"}\n{\"type\":\"next\"}\n"),
            "{name}"
        );
    }

    // A JSON secret value: `[^"\\]` did not take an invalid byte either.
    let value = [&b"q8Zr2Lm"[..], b"\xff", b"N0pX4vB7cT1yW9"].concat();
    let input = [&br#"{"api_key":""#[..], &value, b"\"}\n"].concat();
    let (out, _) = all_three_agree("json value", &input);
    assert_eq!(
        out,
        b"{\"api_key\":\"<redacted:generic_secret_assignment>\"}\n"
    );

    // The invalid byte at every position of one body, every line its own record.
    let mut every = Vec::new();
    for at in 0..40 {
        let mut body = alnum(40);
        body[at] = 0xFF;
        every.extend_from_slice(b"{\"k\":\"sk-ant-api03-");
        every.extend_from_slice(&body);
        every.extend_from_slice(b"\"}\n");
    }
    let (out, spans) = all_three_agree("every position", &every);
    assert_eq!(spans.len(), 40);
    assert!(!contains(&out, b"Ab3dEf7h") && !contains(&out, b"\xff"));
    // An invalid byte where no key is stays as it was.
    let (out, spans) = all_three_agree("no key", b"plain \xff\xfe bytes \xc3( kept\n");
    assert!(spans.is_empty());
    assert_eq!(out, b"plain \xff\xfe bytes \xc3( kept\n");
}

/// H-20: a lone `-----BEGIN PRIVATE KEY-----` with no END redacted the next 12,288 bytes as
/// one span, across every record they reached, and broke their JSON. Now only the string
/// interiors in the block's `PEM_BODY_MAX` bytes go: a key cut off by an output limit is still
/// taken whole, the JSON between strings (numbers, `false`, the braces) stays, every record
/// parses, and the 4 KiB of ordinary JSON past the bound is kept byte for byte. Raw text with no
/// quote loses exactly `PEM_BODY_MAX` bytes after its BEGIN line. All three redactors agree.
#[test]
fn a_lone_begin_does_not_redact_to_the_end_of_the_record() {
    let body = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7VJTUt9Us8cKj\\n\
                MzEfYyjiWA4R4/M2bS1GB4t7NXp98C3SC6dVMvDuictGeurT8jNbvJZHtCSuYEvu\\n\
                [output truncated]";
    let lead = r#"{"type":"tool_result","content":"$ head -c 200 key.pem\n"#;
    let begin = "-----BEGIN PRIVATE KEY-----\\n";
    let mut rest = String::from(r#"","is_error":false,"files":["#);
    let mut i = 0;
    while rest.len() < PEM_BODY_MAX + 4096 {
        rest.push_str(&format!(
            r#"{{"path":"src/module_{i}.rs","lines":{},"note":"an ordinary entry"}},"#,
            40 + i
        ));
        i += 1;
    }
    rest.push_str(r#"{"path":"README.md"}]}"#);
    let record = format!("{lead}{begin}{body}{rest}\n");
    let input = format!("{record}{{\"type\":\"next\",\"n\":1}}\n");
    let (out, spans) = all_three_agree("lone begin in JSON", input.as_bytes());
    let start = lead.len();
    let bound = start + "-----BEGIN PRIVATE KEY-----".len() + PEM_BODY_MAX;
    assert_eq!(
        (spans[0].start as usize, spans[0].end as usize),
        (start, start + begin.len() + body.len()),
        "the BEGIN and the cut-off body to its closing quote"
    );
    assert!(
        spans
            .iter()
            .all(|s| s.kind == "private_key" && s.end as usize <= bound)
    );
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("MIIEvQIBADAN") && !text.contains("truncated"));
    assert!(text.contains(r#"<redacted:private_key>","<redacted:private_key>":false,"#));
    // From the first entry wholly past the bound, the record is kept as it was.
    let tail_from = bound + input[bound..].find("{\"path\"").unwrap();
    assert!(input.len() - tail_from > 4096);
    assert!(
        text.ends_with(&input[tail_from..]),
        "the JSON past the bound is kept"
    );
    for line in text.lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("each record still parses");
    }

    // Raw text with no quote and no END loses exactly `PEM_BODY_MAX` bytes after its BEGIN
    // line, and keeps the rest.
    let begin = "-----BEGIN PRIVATE KEY-----";
    let after: String = (0..PEM_BODY_MAX + 4096)
        .map(|i| char::from(b"abcdefghij \n"[i % 12]))
        .collect();
    let input = format!("log {begin}{after}");
    let (out, spans) = all_three_agree("lone begin in raw text", input.as_bytes());
    let end = 4 + begin.len() + PEM_BODY_MAX;
    assert_eq!(
        (spans.len(), block_run(&spans)),
        (2, ("private_key", 4, end as u64, 2)),
        "the BEGIN line and the bounded block, cut once at PEM_SLICE"
    );
    assert_eq!(spans[1].start, (4 + PEM_SLICE) as u64);
    assert!(out.ends_with(&input.as_bytes()[end..]));
}

/// The bound both arms of the regex literal spell is [`PEM_BODY_MAX`], and a block whose bound
/// falls on the backslash of an escaped quote leaves the escape whole, so the record parses.
#[test]
fn the_pem_block_keeps_an_escaped_quote_and_spells_the_bound() {
    let source = goalcup_daemon::redact::patterns()
        .iter()
        .find(|p| p.kind == "private_key")
        .expect("the private_key rule")
        .re
        .as_str();
    let body = format!("(?-u:[\\x00-\\xFF]){{0,{PEM_BODY_MAX}}}");
    assert_eq!(source.matches(&body).count(), 2, "{source}");

    let begin = "-----BEGIN PRIVATE KEY-----";
    for before in [PEM_BODY_MAX - 1, PEM_BODY_MAX - 2, PEM_BODY_MAX] {
        let input = format!(
            "{{\"t\":\"{begin}{}\\\"quoted\\\" and on\"}}\n",
            "a".repeat(before)
        );
        serde_json::from_str::<serde_json::Value>(&input).expect("the input is JSON");
        let (out, spans) = all_three_agree("escaped quote at the bound", input.as_bytes());
        assert_eq!(block_run(&spans).3, spans.len());
        assert_eq!(spans.len(), 2, "one string, cut once at PEM_SLICE");
        let text = String::from_utf8(out).unwrap();
        serde_json::from_str::<serde_json::Value>(&text).expect("the record parses");
        assert!(!text.contains("aaaaaaaa"), "{before}");
    }
}

/// One SSE shape a model's text deltas arrive in: the event a delta is framed in.
fn sse_event(shape: &str, delta: &str) -> String {
    let d = serde_json::to_string(delta).unwrap();
    match shape {
        "anthropic" => format!(
            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{d}}}}}\n\n"
        ),
        "chat" => format!(
            "data: {{\"id\":\"chatcmpl-9xYz\",\"object\":\"chat.completion.chunk\",\"created\":1760000000,\"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{d}}},\"finish_reason\":null}}]}}\n\n"
        ),
        "responses" => format!(
            "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_0a1b2c\",\"output_index\":0,\"content_index\":0,\"delta\":{d}}}\n\n"
        ),
        _ => unreachable!(),
    }
}

/// The text deltas of an SSE stream, parsed back and joined: what a reader of the trace sees.
fn sse_text(shape: &str, stream: &[u8]) -> String {
    let mut text = String::new();
    for line in String::from_utf8_lossy(stream).lines() {
        let Some(json) = line.strip_prefix("data: ") else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_str(json).expect("each data line parses");
        let delta = match shape {
            "anthropic" => &v["delta"]["text"],
            "chat" => &v["choices"][0]["delta"]["content"],
            "responses" => match v["type"].as_str() {
                Some("response.output_text.delta") => &v["delta"],
                _ => continue,
            },
            _ => unreachable!(),
        };
        // A delta whose keys were redacted (inside a block) has no path; `sse_strings` reads it.
        if let Some(delta) = delta.as_str() {
            text.push_str(delta);
        }
    }
    text
}

/// Every string of every data line of an SSE stream, keys included, decoded and joined.
fn sse_strings(stream: &[u8]) -> String {
    fn walk(v: &serde_json::Value, out: &mut String) {
        match v {
            serde_json::Value::String(s) => out.push_str(s),
            serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            serde_json::Value::Object(o) => o.iter().for_each(|(k, v)| {
                out.push_str(k);
                walk(v, out);
            }),
            _ => {}
        }
    }
    let mut out = String::new();
    for line in String::from_utf8_lossy(stream).lines() {
        if let Some(json) = line.strip_prefix("data: ") {
            let v: serde_json::Value = serde_json::from_str(json).expect("each data line parses");
            walk(&v, &mut out);
        }
    }
    out
}

shards!(a_key_streamed_as_sse_deltas_is_redacted_in_every_delta, 8);
/// H-20 review: a key the model prints arrives as many SSE text deltas. Spread over 20 or more
/// deltas in the Anthropic, chat-completions and Responses shapes, complete, cut off before its
/// END, and with its END line split across two deltas (the BEGIN line whole in one delta), no
/// 16-character run of the body survives, in the raw bytes or in the text rebuilt from the
/// deltas (every string of every data line, decoded). Every data line still parses, the text past the bound is kept, and the stream at
/// three chunkings, the whole-record policy and `spikes/redact.py` agree.
fn a_key_streamed_as_sse_deltas_is_redacted_in_every_delta(shard: &Shard) {
    const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
    const END: &str = "-----END PRIVATE KEY-----";
    let b64 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut checked = 0;
    for seed in 1..=3u64 {
        let mut rng = Rng(0x55E_0000 + seed);
        let lines: Vec<String> = (0..26)
            .map(|_| (0..64).map(|_| char::from(b64[rng.below(64)])).collect())
            .collect();
        let after: String = (0..3000)
            .map(|i| ["keep ", "it ", "safe ", "and ", "rotate ", "it. "][i % 6])
            .collect();
        for variant in ["complete", "truncated", "split end"] {
            let body = match variant {
                "truncated" => lines.join("\n")[..1200].to_string(),
                _ => lines.join("\n"),
            };
            let key = match variant {
                "truncated" => format!("{BEGIN}\n{body}"),
                _ => format!("{BEGIN}\n{body}\n{END}"),
            };
            let lead = "Here is the key you asked for:\n";
            let text = format!("{lead}{key}\n{after}");
            let begin_at = lead.len();
            let key_end = begin_at + key.len();
            let end_at = key_end - END.len();
            // 24 cuts in the key after its BEGIN line. A whole END line, or one cut inside it.
            let mut cuts: Vec<usize> = (0..24)
                .map(|_| begin_at + BEGIN.len() + rng.below(key.len() - BEGIN.len()))
                .collect();
            if variant == "split end" {
                cuts.retain(|&c| c <= end_at || c >= key_end);
                cuts.push(end_at + 7);
            } else if variant == "complete" {
                cuts.retain(|&c| c <= end_at || c >= key_end);
            }
            cuts.push(begin_at);
            cuts.sort_unstable();
            cuts.dedup();
            let key_deltas = cuts.len();
            // The text after the key in 80-byte deltas.
            cuts.extend((key_end..text.len()).step_by(80));
            let mut deltas = Vec::new();
            let mut at = 0;
            for c in cuts.into_iter().chain([text.len()]) {
                if c > at {
                    deltas.push(&text[at..c]);
                    at = c;
                }
            }
            assert!(
                key_deltas >= 20,
                "{variant} seed {seed}: {key_deltas} deltas"
            );
            for shape in ["anthropic", "chat", "responses"] {
                if !shard.mine() {
                    continue;
                }
                let name = format!("{shape} {variant} seed {seed}");
                let stream: String = deltas.iter().map(|d| sse_event(shape, d)).collect();
                assert_eq!(
                    sse_text(shape, stream.as_bytes()),
                    text,
                    "{name}: the generator"
                );
                let (out, _) = all_three_agree(&name, stream.as_bytes());
                let rebuilt = sse_strings(&out);
                for line in &lines {
                    for run in line.as_bytes().windows(16) {
                        assert!(
                            !contains(&out, run)
                                && !rebuilt.contains(std::str::from_utf8(run).unwrap()),
                            "{name}: {:?} of the body survives",
                            std::str::from_utf8(run).unwrap()
                        );
                        checked += 1;
                    }
                }
                assert!(
                    !rebuilt.contains("BEGIN") && !rebuilt.contains(" KEY-"),
                    "{name}"
                );
                assert!(
                    sse_text(shape, &out).ends_with(&after[after.len() - 2000..]),
                    "{name}: the text past the bound is kept"
                );
            }
        }
    }
    // Over 10,000 runs checked in all; each part checks its share.
    assert!(checked * shard.parts() > 10_000, "{checked}");
}

shards!(
    a_second_begin_inside_a_block_keeps_the_stream_equal_to_the_record,
    4
);
/// H-20 re-review: a second BEGIN inside a lone BEGIN's block is no match to the whole-record
/// regex (it lies inside the first match), but the stream held it to its own bound, so the first
/// block outlived the hold and the stream failed closed: a one-line request body lost every
/// later turn. The stream no longer starts a BEGIN inside an open block. Two BEGIN mentions
/// 0.5, 4, 8 and 15.9 KB apart, in a one-line JSON body and in JSONL: the stream equals the
/// whole-record policy at three chunkings (and `spikes/redact.py` agrees), every record parses,
/// and every later turn's marker past the blocks is kept.
fn a_second_begin_inside_a_block_keeps_the_stream_equal_to_the_record(shard: &Shard) {
    const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
    for gap in [500usize, 4000, 8000, 15_900] {
        for one_line in [true, false] {
            if !shard.mine() {
                continue;
            }
            let name = format!("gap {gap}, one line {one_line}");
            let (open, sep, close) = if one_line {
                (r#"{"model":"m","messages":["#, ",", "]}\n")
            } else {
                ("", "\n", "\n")
            };
            let turn = |text: &str| format!(r#"{{"role":"user","content":"{text}"}}"#);
            let mut turns = vec![turn(&format!("parse lines that start with {BEGIN} please"))];
            let mut filler = 0;
            loop {
                let so_far: usize = turns.iter().map(|t| t.len() + 1).sum();
                if so_far + 40 >= gap {
                    break;
                }
                let n = (gap - so_far - 40).min(200);
                turns.push(turn(&"x".repeat(n.saturating_sub(30).max(1))));
                filler += 1;
            }
            turns.push(turn(&format!("and also {BEGIN} here")));
            let first_later = turns.len();
            for i in 0..1000 {
                turns.push(turn(&format!("later turn MARK-{i:04} ordinary text")));
            }
            let input = format!("{open}{}{close}", turns.join(sep));
            assert!(filler > 0 || gap < 600, "{name}");
            let (out, spans) = all_three_agree(&name, input.as_bytes());
            let text = String::from_utf8(out).unwrap();
            for line in text.lines() {
                serde_json::from_str::<serde_json::Value>(line).expect("each record parses");
            }
            // Every marker that starts past the last block's bound is kept.
            let last = spans.iter().map(|s| s.end).max().unwrap() as usize;
            let mut kept = 0;
            for t in &turns[first_later..] {
                let at = input.find(t.as_str()).unwrap();
                if at > last {
                    assert!(text.contains(t.as_str()), "{name}: {t} lost");
                    kept += 1;
                }
            }
            assert!(kept > 600, "{name}: {kept} later turns kept");
        }
    }
}

/// H-20 re-review: in raw text the quotes need not pair as JSON. A note mentions a BEGIN, quotes
/// once, then prints a whole key: the key's body fell outside the pieces the block cut at quotes
/// and was kept. A piece outside the strings that holds a key-shaped run is redacted too, so no
/// 16-character run of the key survives, and the stream, the whole-record policy and
/// `spikes/redact.py` agree. With the quotes paired, ordinary JSON framing is still kept.
#[test]
fn a_key_after_an_odd_quote_in_raw_text_is_still_redacted() {
    let b64 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut rng = Rng(0x0DD_9007E);
    let lines: Vec<String> = (0..26)
        .map(|_| (0..64).map(|_| char::from(b64[rng.below(64)])).collect())
        .collect();
    for quotes in [1, 3] {
        let note = format!(
            "# Notes\n\nKeys start with a `-----BEGIN PRIVATE KEY-----` line.\n{}He said \"use this one:\n\
             -----BEGIN RSA PRIVATE KEY-----\n{}\n-----END RSA PRIVATE KEY-----\nThanks.\n",
            "a \"quoted\" word, ".repeat((quotes - 1) / 2),
            lines.join("\n")
        );
        let name = format!("{quotes} quotes");
        let (out, _) = all_three_agree(&name, note.as_bytes());
        for line in &lines {
            if let Some(run) = no_run_of_len(&out, line.as_bytes(), 16) {
                panic!("{name}: {run:?} of the key survives");
            }
        }
        assert!(out.starts_with(b"# Notes\n\nKeys start with a `"), "{name}");
    }
}

/// No `n`-byte run of `secret` is in `out`.
fn no_run_of_len(out: &[u8], secret: &[u8], n: usize) -> Option<String> {
    secret
        .windows(n)
        .find(|w| contains(out, w))
        .map(|w| String::from_utf8_lossy(w).into_owned())
}

/// H-20 re-review (Minor): the 512 bytes the bound leaves under the hold are room for a real
/// BEGIN line. A label far longer than any real one holds its block past the hold, and the
/// stream fails closed there: it may redact more than the whole-record policy, but never less,
/// and no run of the key survives in either.
#[test]
fn a_long_begin_label_fails_safe() {
    let b64 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut rng = Rng(0x1AB_E15);
    let lines: Vec<String> = (0..26)
        .map(|_| (0..64).map(|_| char::from(b64[rng.below(64)])).collect())
        .collect();
    for label in [450usize, 500, 1100, 4000] {
        let begin = format!("-----BEGIN {} PRIVATE KEY-----", "A".repeat(label));
        for (shape, input) in [
            (
                "json",
                format!(
                    "{{\"text\":\"{begin}\\n{}\"}}\n{{\"next\":1}}\n",
                    lines.join("\\n")
                ),
            ),
            ("raw", format!("{begin}\n{}\n", lines.join("\n"))),
        ] {
            let name = format!("label {label} {shape}");
            let input = input.as_bytes();
            let want = legacy(input);
            let mut rng = Rng(label as u64);
            for chunks in [
                vec![MAX_SEGMENT_BYTES],
                vec![1],
                random_chunks(&mut rng, 97),
            ] {
                let got = retained_of(&stream_through(StreamRedactor::new(), input, &chunks, 2), 1);
                assert!(
                    uncovered(&want.1, &got.1).is_empty(),
                    "{name}: the stream redacts less than the whole-record policy"
                );
                for (what, out) in [("stream", &got.0), ("record", &want.0)] {
                    for line in &lines {
                        if let Some(run) = no_run_of_len(out, line.as_bytes(), 16) {
                            panic!("{name}: the {what} keeps {run:?} of the key");
                        }
                    }
                }
            }
        }
    }
}

/// Words for filler: no quote, no backslash, no base64 run of 16.
fn words(n: usize) -> String {
    "lorem ipsum dolor sit amet consectetur adipiscing elit "
        .chars()
        .cycle()
        .take(n)
        .collect()
}

/// `lines` lines of 64 base64 characters, a key body.
fn key_lines(rng: &mut Rng, lines: usize) -> Vec<String> {
    let b64 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    (0..lines)
        .map(|_| (0..64).map(|_| char::from(b64[rng.below(64)])).collect())
        .collect()
}

shards!(a_real_key_late_in_a_lone_begins_window_is_redacted_whole, 4);
/// H-20 re-review 2: a lone BEGIN mention opens a window; a real key whose BEGIN falls late in
/// it, with its END past the window, lost the tail past the bound. A window with no END now
/// ends at the first BEGIN inside it, and that BEGIN opens its own. The real key's BEGIN at
/// 12 to 16 KB after the mention, for RSA-2048 and RSA-4096 bodies, in JSONL and raw text: no
/// 16-character run survives, and the stream, the whole-record policy and `spikes/redact.py`
/// agree.
fn a_real_key_late_in_a_lone_begins_window_is_redacted_whole(shard: &Shard) {
    const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
    const END: &str = "-----END PRIVATE KEY-----";
    let mut rng = Rng(0x1A7E_0B16);
    let mut offsets: Vec<usize> = (12_000..=16_000).step_by(500).collect();
    offsets.extend([15_870, 15_871, 15_872, 15_873]);
    for (size, n) in [("rsa2048", 26), ("rsa4096", 51)] {
        let lines = key_lines(&mut rng, n);
        for &t in &offsets {
            // JSONL: the mention, one filler record, the key read by a tool, more records.
            let rec1 = format!(r#"{{"type":"tool","output":"grep: {BEGIN} in parser.rs"}}"#);
            let body1 = rec1.find(BEGIN).unwrap() + BEGIN.len();
            let (fill, key_at) = (r#"{"type":"msg","text":""#, r#"{"type":"tool","output":""#);
            let used = (rec1.len() + 1 - body1) + fill.len() + 3 + key_at.len();
            let jsonl = format!(
                "{rec1}\n{fill}{}\"}}\n{key_at}{BEGIN}\\n{}\\n{END}\\n\"}}\n{}",
                words(t - used),
                lines.join("\\n"),
                "{\"type\":\"msg\",\"text\":\"after\"}\n".repeat(50)
            );
            assert_eq!(jsonl.rfind(BEGIN).unwrap() - body1, t);
            // Raw text: the mention, filler, the key.
            let head = format!("grep hit: {BEGIN} in parser.rs\n");
            let raw = format!(
                "{head}{}\n{BEGIN}\n{}\n{END}\nafter\n",
                words(t - (head.len() - head.find(BEGIN).unwrap() - BEGIN.len()) - 1),
                lines.join("\n")
            );
            for (shape, input) in [("jsonl", jsonl), ("raw", raw)] {
                if !shard.mine() {
                    continue;
                }
                let name = format!("{size} {shape} at {t}");
                let (out, _) = all_three_agree(&name, input.as_bytes());
                for line in &lines {
                    if let Some(run) = no_run_of_len(&out, line.as_bytes(), 16) {
                        panic!("{name}: {run:?} of the key survives");
                    }
                }
                if shape == "jsonl" {
                    for record in String::from_utf8(out).unwrap().lines() {
                        serde_json::from_str::<serde_json::Value>(record)
                            .unwrap_or_else(|e| panic!("{name}: {e}: {record}"));
                    }
                }
            }
        }
    }
}

shards!(
    a_run_across_the_end_of_a_lone_begins_window_keeps_the_stream_equal,
    4
);
/// H-20 re-review 2: a base64url run (no `/`: a Codex `encrypted_content` value) that starts
/// inside a lone BEGIN's window and crosses its end kept the email recognizer live from inside
/// the window, so the whole window was held past the hold and the stream failed closed: a
/// one-line body lost every later record. A decided window now leaves in parts, at its pieces
/// (and its one cut at `PEM_SLICE`), so the stream equals the whole-record policy. The
/// reviewer's runs, as `(start in the window, length)`, in JSONL and in a one-line body.
fn a_run_across_the_end_of_a_lone_begins_window_keeps_the_stream_equal(shard: &Shard) {
    const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
    let b64url = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let b64 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut rng = Rng(0xE0C_0DE);
    let cases: &[(&str, usize, usize)] = &[
        ("base64url", 100, 16_000),
        ("base64url", 100, 16_500),
        ("base64url", 100, 20_000),
        ("base64url", 15_000, 2_000),
        ("base64url", 15_000, 5_000),
        ("base64url", 15_500, 1_000),
        ("base64url", 15_800, 500),
        ("base64url", 15_860, 300),
        ("base64url", 17_000, 3_000),
        ("base64", 100, 20_000),
        ("base64", 15_000, 5_000),
        ("letters", 15_000, 2_000),
    ];
    for &(class, s, len) in cases {
        let run: String = (0..len)
            .map(|_| match class {
                "base64url" => char::from(b64url[rng.below(64)]),
                "base64" => char::from(b64[rng.below(64)]),
                _ => 'w',
            })
            .collect();
        for one_line in [false, true] {
            if !shard.mine() {
                continue;
            }
            let name = format!("{class} {s}/{len} one line {one_line}");
            let (sep, open, close) = if one_line {
                (",", r#"{"input":["#, "]}\n")
            } else {
                ("\n", "", "\n")
            };
            let rec1 = format!(r#"{{"type":"message","text":"look for {BEGIN} lines"}}"#);
            let body1 = open.len() + rec1.find(BEGIN).unwrap() + BEGIN.len();
            let fill = r#"{"type":"message","text":""#;
            let blob = r#"{"type":"reasoning","encrypted_content":""#;
            let used = (open.len() + rec1.len() + sep.len() - body1) + fill.len() + 2 + sep.len();
            let mut records = vec![rec1];
            if s > used + blob.len() {
                records.push(format!("{fill}{}\"}}", words(s - used - blob.len())));
            }
            records.push(format!("{blob}{run}\"}}"));
            for i in 0..300 {
                records.push(format!(r#"{{"type":"message","text":"turn MARK-{i:04}"}}"#));
            }
            let input = format!("{open}{}{close}", records.join(sep));
            if s > used + blob.len() {
                assert_eq!(input.find(&run).unwrap() - body1, s, "{name}");
            }
            let (out, spans) = all_three_agree(&name, input.as_bytes());
            let text = String::from_utf8(out).unwrap();
            for line in text.lines() {
                serde_json::from_str::<serde_json::Value>(line)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
            }
            let last = spans.iter().map(|s| s.end).max().unwrap() as usize;
            for i in 0..300 {
                let mark = format!("turn MARK-{i:04}");
                if input.find(&mark).unwrap() > last {
                    assert!(text.contains(&mark), "{name}: {mark} lost");
                }
            }
            assert!(text.contains("turn MARK-0299"), "{name}");
        }
    }
}

shards!(
    a_cut_right_after_a_backslash_leaves_nothing_between_the_two_markers,
    6
);
/// H-20 re-review 3 (Important 1): a bounded window's cut at `PEM_SLICE` that fell right after
/// a backslash left that backslash raw between the two markers. The stream decides the first
/// part on the bytes up to the cut alone, where a piece loses the backslashes it ends on, so its
/// first marker ended early: `<redacted:private_key>\<redacted:private_key>`, which is not
/// JSON and differs from the whole-record policy. The cut now moves past a backslash run and
/// the byte it escapes, in all three redactors. One string holds the cut and crosses the
/// window's end with a 10,000-character base64url run, so the stream must release at the cut.
/// The bytes before the cut are swept over each escape shape (`\n`; `\"` and `\\\"`;
/// `C:\\path`; `\u2192` and `\n`; a raw `→`; a key body): for one run of each kind in the
/// shape, every position with a backslash before the cut and the position after the run, and
/// one ordinary position; in JSONL for every shape, and in a one-line body and SSE for the
/// runs of one, two and three backslashes (`quote`, `path`). Each: the stream at three
/// chunkings equals the whole-record policy and `spikes/redact.py` (the raw `→` excepted: the
/// script counts the bound in characters, so its block ends later in bytes); the two markers
/// touch; every record parses; and no 16-character run of the key body survives.
fn a_cut_right_after_a_backslash_leaves_nothing_between_the_two_markers(shard: &Shard) {
    const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
    let b64url = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut rng = Rng(0x0BAC_51A5);
    let run: String = (0..10_000)
        .map(|_| char::from(b64url[rng.below(64)]))
        .collect();
    let body = key_lines(&mut rng, 26);
    // A string's text as the tool or model wrote it; the JSON escaping puts the backslashes
    // in. `ascii` writes `→` as `\u2192`, as `ensure_ascii` would.
    let escape = |text: &str, ascii: bool| -> String {
        let json = serde_json::to_string(text).unwrap();
        let inner = json[1..json.len() - 1].to_string();
        if ascii {
            inner.replace('\u{2192}', "\\u2192")
        } else {
            inner
        }
    };
    let arrows = "     1\u{2192}line\n     2\u{2192}line\n";
    // (name, one period of the text, repeats, ascii, the script counts the same)
    let shapes: Vec<(&str, String, usize, bool, bool)> = vec![
        (
            "newline",
            "line one\nline two\nline three\n".into(),
            40,
            true,
            true,
        ),
        (
            "quote",
            "he said \\\"no\\\" then \"yes\" again ".into(),
            40,
            true,
            true,
        ),
        ("path", "C:\\Users\\a\\b\\c path ".into(), 40, true, true),
        ("u2192", arrows.into(), 40, true, true),
        ("raw arrow", arrows.into(), 40, false, false),
        ("key body", format!("{}\n", body[0]), 26, true, true),
    ];
    let later: Vec<String> = (0..100).map(|i| format!("MARK-{i:04} later")).collect();
    let build = |form: &str, text: &str| -> String {
        match form {
            "jsonl" => {
                let mut s = format!("{{\"type\":\"tool_result\",\"content\":\"grep: {BEGIN}\"}}\n");
                s.push_str(&format!(
                    "{{\"type\":\"reasoning\",\"content\":\"{text}\"}}\n"
                ));
                for mark in &later {
                    s.push_str(&format!("{{\"type\":\"note\",\"text\":\"{mark}\"}}\n"));
                }
                s
            }
            "body" => {
                let mut turns = vec![
                    format!("{{\"role\":\"user\",\"content\":\"grep {BEGIN}\"}}"),
                    format!("{{\"role\":\"assistant\",\"content\":\"{text}\"}}"),
                ];
                turns.extend(
                    later
                        .iter()
                        .map(|m| format!("{{\"role\":\"user\",\"content\":\"{m}\"}}")),
                );
                format!("{{\"input\":[{}]}}\n", turns.join(","))
            }
            _ => {
                let ev = |t: &str| {
                    format!(
                        "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{t}\"}}}}\n\n"
                    )
                };
                let mut s = ev(&format!("The parser checks {BEGIN} first."));
                s.push_str(&ev(text));
                for mark in &later {
                    s.push_str(&ev(&format!(" {mark}")));
                }
                s
            }
        }
    };
    let parses = |form: &str, text: &str, name: &str| {
        let lines: Vec<&str> = match form {
            "jsonl" | "body" => text.lines().collect(),
            _ => text
                .lines()
                .filter_map(|l| l.strip_prefix("data: "))
                .collect(),
        };
        assert!(!lines.is_empty(), "{name}");
        for line in lines {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|e| panic!("{name}: a record does not parse: {e}\n{line}"));
        }
    };
    let mut cases = 0;
    for (shape, text, repeats, ascii, script_counts_the_same) in &shapes {
        let period = escape(text, *ascii);
        let piece = period.repeat(*repeats);
        let e = period.as_bytes();
        // Where the cut lands in the period: for the first run of each length, after each of
        // its backslashes and after the byte it escapes; and one ordinary position.
        let mut at: std::collections::BTreeSet<usize> = [0].into();
        let mut seen = std::collections::BTreeSet::new();
        let mut s = 0;
        while s < e.len() {
            if e[s] != b'\\' {
                s += 1;
                continue;
            }
            let len = e[s..].iter().take_while(|&&b| b == b'\\').count();
            if seen.insert(len) {
                at.extend(s + 1..=s + len + 1);
            }
            s += len;
        }
        let forms: &[&str] = if matches!(*shape, "quote" | "path") {
            &["jsonl", "body", "sse"]
        } else {
            &["jsonl"]
        };
        for &form in forms {
            for &k in &at {
                cases += 1;
                if !shard.mine() {
                    continue;
                }
                let name = format!("{shape} {form} cut at {k}");
                let probe = build(form, &format!("{piece}{run}"));
                let begin = probe.find(BEGIN).unwrap();
                let piece_at = probe.find(&piece).unwrap();
                let pad = begin + PEM_SLICE - piece_at - k;
                let input = build(form, &format!("{}{piece}{run}", words(pad)));
                assert_eq!(input.find(BEGIN).unwrap(), begin, "{name}");
                let piece_at = input.find(&piece).unwrap();
                assert_eq!(
                    piece_at + k,
                    begin + PEM_SLICE,
                    "{name}: the cut lands at {k}"
                );
                let (out, spans) = agree(&name, input.as_bytes(), *script_counts_the_same);
                let text = String::from_utf8(out).unwrap();
                assert!(
                    text.contains("<redacted:private_key><redacted:private_key>"),
                    "{name}: the two markers do not touch"
                );
                assert!(
                    !text.contains(">\\<"),
                    "{name}: a backslash between markers"
                );
                parses(form, &text, &name);
                let last = spans.iter().map(|s| s.end).max().unwrap() as usize;
                assert!(
                    last > begin + PEM_SLICE,
                    "{name}: the block ends past the cut"
                );
                if *shape == "key body" {
                    for line in &body {
                        if let Some(w) = line
                            .as_bytes()
                            .windows(16)
                            .find(|w| text.as_bytes().windows(16).any(|t| t == *w))
                        {
                            panic!(
                                "{name}: {:?} of the key survives",
                                String::from_utf8_lossy(w)
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(cases >= 40, "{cases} cases swept, every part's included");
}

/// H-20 re-review 3 (Minor 2): `spikes/redact.py` had only the shell/TOML form of
/// `generic_secret_assignment`. On a logprobs `"token"` whose value holds a second `BEGIN`
/// mid-string it wrote two touching block markers where the daemon, whose JSON-form claim spans
/// both pieces, writes one. The script now carries the JSON form with the value alone as its
/// span: the three agree, the record stays JSON, a quoted key the shell form cannot reach is
/// redacted, and a value an earlier rule already claimed is not redacted twice.
#[test]
fn the_script_carries_the_json_form_of_a_generic_secret() {
    let chunk = |content: &str, logprob: &str| {
        format!(
            "data: {{\"object\": \"chat.completion.chunk\", \"choices\": [{{\"index\": 0, \"delta\": {{\"content\": \"{content}\"}}, \"logprobs\": {{\"content\": [{{\"token\": \"{content}\", \"logprob\": {logprob}}}]}}}}]}}\n\n"
        )
    };
    let mut input = chunk(
        "The parser checks -----BEGIN PRIVATE KEY-----",
        "-1.2345678901234568e-05",
    );
    for i in 0..40 {
        input.push_str(&chunk(&format!(" word{i}"), "-2.4691357802469136e-05"));
    }
    let (out, _) = all_three_agree("logprob token", input.as_bytes());
    let text = String::from_utf8(out).unwrap();
    for line in text.lines().filter_map(|l| l.strip_prefix("data: ")) {
        serde_json::from_str::<serde_json::Value>(line).expect("each event parses");
    }
    assert!(
        text.contains(r#""<redacted:private_key>": "<redacted:private_key>", "<redacted:private_key>": -1.2345678901234568e-05"#),
        "one marker for the token value:\n{text}"
    );

    let input = concat!(
        r#"{"api_key":"q8Zr2LmN0pX4vB7cT1yW9"}"#,
        "\n",
        r#"{"x-api-key":"sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFF"}"#,
        "\n",
        r#"{"refresh_token": "abcdefghijklmnopqrstuvwxyz tail"}"#,
        "\n",
        r#"{"tokenizer":"abcdefghijklmnopqrstuvwxyz"}"#,
        "\n",
        r#"{"note":"password: \"abcdefghijklmnopqrstuvwxyz\""}"#,
        "\n",
    );
    let (out, spans) = all_three_agree("quoted keys", input.as_bytes());
    assert_eq!(
        String::from_utf8(out).unwrap(),
        concat!(
            r#"{"api_key":"<redacted:generic_secret_assignment>"}"#,
            "\n",
            r#"{"x-api-key":"<redacted:anthropic_key>"}"#,
            "\n",
            r#"{"refresh_token": "<redacted:generic_secret_assignment>"}"#,
            "\n",
            r#"{"tokenizer":"abcdefghijklmnopqrstuvwxyz"}"#,
            "\n",
            // The shell form, as before: its span takes the opening `\"` and leaves the closing one.
            r#"{"note":"<redacted:generic_secret_assignment>\""}"#,
            "\n",
        )
    );
    assert_eq!(spans.len(), 4);
}

/// H-20 re-review 2 (Minor): an exponent float holds a run of digits and an `e`
/// (`2.4691357802469136e-05`), which is number-shaped and not a key line, so metric records
/// inside a lone BEGIN's window keep their numbers and parse; the three redactors agree.
#[test]
fn exponent_floats_inside_a_block_keep_their_records_parsing() {
    let mut input = String::from(r#"{"text":"match -----BEGIN PRIVATE KEY----- here"}"#);
    input.push('\n');
    for i in 0..200 {
        input.push_str(&format!(
            "{{\"token\":{i},\"logprob\":-2.4691357802469136e-05,\"p\":9.99975309247135E+01}}\n"
        ));
    }
    let (out, _) = all_three_agree("exponent floats", input.as_bytes());
    let text = String::from_utf8(out).unwrap();
    for line in text.lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("each record parses");
    }
    assert!(text.contains("2.4691357802469136e-05"));
}

/// The first span's kind, its start, the end of the run of same-kind spans that touch it, and
/// how many spans that run is: a PEM block cut at `PEM_SLICE` is two touching spans.
fn block_run(spans: &[RedactionAuditSpan]) -> (&str, u64, u64, usize) {
    let first = &spans[0];
    let mut end = first.end;
    let mut parts = 1;
    for s in &spans[1..] {
        if s.kind != first.kind || s.start != end {
            break;
        }
        end = s.end;
        parts += 1;
    }
    (first.kind.as_str(), first.start, end, parts)
}
