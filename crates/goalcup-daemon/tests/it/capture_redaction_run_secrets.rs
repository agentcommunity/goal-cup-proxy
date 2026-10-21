//! A run's own exact secret value on the streaming redactor (D60).
//!
//! The launcher hands the daemon the run's proxy credential, the one secret goalcup gives a
//! harness. Every form of it (`goalcup_protocol::run_secret::encoded_forms`), with the line breaks
//! a wrapped or escaped copy carries, is removed from every recorded body, wherever the stream is
//! cut into pushes, and equal to the whole-record oracle `Redactor::for_run`. Every value here is
//! synthetic.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use goalcup_daemon::redact::stream::StreamRedactor;
use goalcup_daemon::redact::{Redactor, RunSecrets};
use goalcup_protocol::capture::*;
use goalcup_protocol::redaction::RedactionAuditSpan;
use goalcup_protocol::run_secret::{RunSecret, encoded_forms};

use crate::common::redaction::*;

/// A synthetic proxy credential: 32 lowercase hex digits.
const PROXY: &str = "0123456789abcdef0123456789abcdef";

/// The run's values, compiled once as a run's are, and shared by every redactor below.
fn secrets() -> RunSecrets {
    static SECRETS: std::sync::OnceLock<RunSecrets> = std::sync::OnceLock::new();
    SECRETS
        .get_or_init(|| RunSecrets::new(&[RunSecret::new(PROXY.as_bytes()).unwrap()]).unwrap())
        .clone()
}

fn oracle(input: &[u8]) -> (Vec<u8>, Vec<RedactionAuditSpan>) {
    let r = Redactor::for_run("run", &[], &secrets())
        .unwrap()
        .apply(input);
    (r.bytes, r.spans)
}

fn redactor() -> StreamRedactor {
    StreamRedactor::for_run(&[], &secrets()).unwrap()
}

fn retained(records: &[CaptureRecord]) -> (Vec<u8>, Vec<RedactionAuditSpan>) {
    let mut bytes = Vec::new();
    let mut spans = Vec::new();
    for r in records {
        if let CaptureRecord::Segment {
            data_b64, audit, ..
        } = r
        {
            bytes.extend(STANDARD.decode(data_b64).unwrap());
            spans.extend(audit.spans.iter().cloned());
        }
    }
    (bytes, spans)
}

/// Whether `bytes` still hold any form of the value, with every line break taken out first
/// (LF, CR, and either escaped with one or two backslashes), so a wrapped form counts as the
/// form.
fn leaks(bytes: &[u8]) -> bool {
    let mut joined = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'\n' | b'\r', ..) => i += 1,
            (b'\\', Some(b'n' | b'r'), _) => i += 2,
            (b'\\', Some(b'\\'), Some(b'n' | b'r')) => i += 3,
            (b, ..) => {
                joined.push(b);
                i += 1;
            }
        }
    }
    encoded_forms(PROXY.as_bytes())
        .iter()
        .any(|form| joined.windows(form.len()).any(|w| w == form.as_slice()))
}

/// The proxy credential where a run puts it: a request body, a model reply, an agent's `env`
/// output in a tool result, base64 at every alignment (`env | base64`), every byte
/// percent-encoded, every byte `\u`-escaped, behind an escaped colon, and glued to text an
/// earlier rule claims (the H8 residuals).
fn corpus() -> Vec<u8> {
    let env_line = format!("HOME=/home/me\nHTTPS_PROXY=http://goalcup:{PROXY}@127.0.0.1:47123\n");
    let mut c = Vec::new();
    c.extend(format!("{{\"model\":\"m\",\"messages\":[{{\"role\":\"user\",\"content\":\"use {PROXY} please\"}}]}}\n").as_bytes());
    c.extend(
        format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"it is {PROXY}.\"}}}}]}}\n\n")
            .as_bytes(),
    );
    c.extend(
        format!(
            "{{\"role\":\"tool\",\"content\":{}}}\n",
            serde_json::to_string(&env_line).unwrap()
        )
        .as_bytes(),
    );
    c.extend(STANDARD.encode(env_line.as_bytes()).as_bytes());
    c.push(b'\n');
    for lead in ["", "a", "ab"] {
        let text = format!("{lead}{PROXY}!");
        c.extend(STANDARD.encode(text.as_bytes()).as_bytes());
        c.push(b' ');
        c.extend(URL_SAFE_NO_PAD.encode(text.as_bytes()).as_bytes());
        c.push(b'\n');
    }
    let percent: String = PROXY.bytes().map(|b| format!("%{b:02X}")).collect();
    c.extend(format!("q={percent}&x=1\n").as_bytes());
    let escaped: String = PROXY.bytes().map(|b| format!("\\u{b:04x}")).collect();
    c.extend(format!("{{\"v\":\"{escaped}\"}}\n").as_bytes());
    c.extend(format!("{{\"p\":\"goalcup\\u003a{PROXY}\"}}\n").as_bytes());
    c.extend(format!("sk-ant-api03-AAAAAAAAAAAAAAAAAAAAgoalcup:{PROXY}\n").as_bytes());
    c
}

/// Every form leaves the chain, at every split of the stream, and the stream equals the oracle.
#[test]
fn a_run_secret_split_at_any_byte_is_never_persisted() {
    let input = corpus();
    assert!(leaks(&input), "the corpus holds the value");
    let (want_bytes, want_spans) = oracle(&input);
    assert!(!leaks(&want_bytes), "the oracle redacts every form");
    assert!(
        want_spans.iter().any(|s| s.kind == "run_secret"),
        "{want_spans:?}"
    );
    for split in 0..=input.len() {
        let mut p = Producer::with_redactor(P1, redactor());
        let mut records = p.open(1, CaptureSource::WireRequest);
        records.extend(p.bytes(1, &input[..split]));
        records.extend(p.redactor.flush(daemon_time(1)).unwrap());
        records.extend(p.bytes(1, &input[split..]));
        records.extend(p.end(1, EndReason::Complete));
        let (bytes, spans) = retained(&records);
        assert!(!leaks(&bytes), "split at {split}");
        assert_eq!(
            bytes, want_bytes,
            "retained bytes differ when split at {split}"
        );
        assert_eq!(spans, want_spans, "audit differs when split at {split}");
    }
    let (bytes, spans) = retained(&stream_through(redactor(), &input, &[1], 5));
    assert_eq!(
        (bytes, spans),
        (want_bytes, want_spans),
        "one byte per push"
    );
}

/// A stream cut while a value is held (the producer lost, the stream interrupted, a gap)
/// releases none of the value's delivered bytes.
#[test]
fn a_cut_inside_a_run_secret_releases_none_of_it() {
    {
        let value = PROXY;
        let input = format!("{{\"text\":\"{value}\"}}\n").into_bytes();
        let at = input
            .windows(value.len())
            .position(|w| w == value.as_bytes())
            .unwrap();
        for k in at + 1..at + value.len() {
            let delivered = &value.as_bytes()[..k - at];
            for mode in ["interrupt", "end-interrupted", "gap"] {
                let mut p = Producer::with_redactor(P1, redactor());
                let mut records = p.open(1, CaptureSource::WireRequest);
                records.extend(p.bytes(1, &input[..k]));
                records.extend(p.redactor.flush(daemon_time(1)).unwrap());
                match mode {
                    "interrupt" => records.extend(
                        p.redactor
                            .interrupt(daemon_time(2), GapReason::ProducerLost),
                    ),
                    "end-interrupted" => records.extend(p.end(1, EndReason::Interrupted)),
                    _ => {
                        records.extend(p.gap(1, GapReason::QueueFull, None));
                        records.extend(p.end(1, EndReason::Complete));
                    }
                }
                let (bytes, _) = retained(&records);
                // A marker's own letters are not the value's: only what is outside one counts.
                let bytes = without_markers(&bytes);
                let held = bytes.windows(delivered.len()).any(|w| w == delivered);
                assert!(
                    !held,
                    "{mode}: {} delivered bytes were released",
                    delivered.len()
                );
            }
        }
    }
}

/// `bytes` with every `<redacted:KIND>` marker taken out.
fn without_markers(bytes: &[u8]) -> Vec<u8> {
    let marker = regex::bytes::Regex::new(r"<redacted:[a-z_]+>").unwrap();
    marker.replace_all(bytes, &b""[..]).into_owned()
}

/// Without run secrets the redactor is the one it was: a value no shared rule knows stays.
#[test]
fn a_run_without_secrets_redacts_as_before() {
    let input = format!("{{\"text\":\"{PROXY}\"}}\n").into_bytes();
    let plain = StreamRedactor::for_run(&[], &RunSecrets::default()).unwrap();
    let (bytes, spans) = retained(&stream_through(plain, &input, &[7], 1));
    assert_eq!(bytes, input);
    assert!(spans.is_empty());
    assert!(!format!("{:?}", secrets()).contains(PROXY));
}

/// `text` cut into lines of `width` characters, each ended by `end`: what GNU `base64` (76
/// columns, the default) and `fold -w N` write.
fn wrap(text: &[u8], width: usize, end: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in text.chunks(width) {
        out.extend_from_slice(line);
        out.extend_from_slice(end.as_bytes());
    }
    out
}

/// `bytes` as one JSON string inside a tool result: each line break becomes the text `\n`.
fn in_tool_result(bytes: &[u8]) -> Vec<u8> {
    let content = serde_json::to_string(&String::from_utf8_lossy(bytes)).unwrap();
    format!("{{\"role\":\"tool\",\"content\":{content}}}\n").into_bytes()
}

/// `bytes` the way Hermes records a tool's output (C1b): `json.dumps({"output": …})` becomes
/// the message `content`, which is itself a JSON string, so each line break is the text `\\n`.
fn in_hermes_tool_result(bytes: &[u8]) -> Vec<u8> {
    let inner = serde_json::json!({ "output": String::from_utf8_lossy(bytes), "exit_code": 0 });
    let content = serde_json::to_string(&inner.to_string()).unwrap();
    format!("{{\"role\":\"tool\",\"content\":{content}}}\n").into_bytes()
}

/// The wrapped forms a run really produces (C1, C1b): `env | base64` as GNU `base64` prints
/// it, wrapped at 76 columns, with the value at every alignment and every place a break can fall
/// in it; the same in a JSON tool result and in Hermes's doubly escaped one, with LF and CR LF;
/// URL-safe; `fold -w 20` of the raw value; and the value percent-encoded, folded.
fn wrapped_cases() -> Vec<(String, Vec<u8>)> {
    use base64::engine::general_purpose::URL_SAFE;
    let mut cases = Vec::new();
    for lead in 0..=57 {
        let env = format!(
            "{}HOME=/home/me\nHTTPS_PROXY=http://goalcup:{PROXY}@127.0.0.1:47123\n",
            "x".repeat(lead)
        );
        let gnu = wrap(STANDARD.encode(env.as_bytes()).as_bytes(), 76, "\n");
        let crlf = wrap(STANDARD.encode(env.as_bytes()).as_bytes(), 76, "\r\n");
        cases.push((format!("hermes lead {lead}"), in_hermes_tool_result(&gnu)));
        if lead % 4 == 0 || lead < 3 {
            cases.push((format!("tool result lead {lead}"), in_tool_result(&gnu)));
            cases.push((
                format!("hermes crlf lead {lead}"),
                in_hermes_tool_result(&crlf),
            ));
            let url = wrap(URL_SAFE.encode(env.as_bytes()).as_bytes(), 76, "\n");
            cases.push((format!("url-safe lead {lead}"), url));
            cases.push((format!("crlf lead {lead}"), crlf));
        }
        cases.push((format!("env | base64 lead {lead}"), gnu));
    }
    for lead in 0..20 {
        let text = format!("{}{PROXY}\n", "x".repeat(lead));
        let folded = wrap(text.as_bytes(), 20, "\n");
        if lead % 7 == 0 {
            cases.push((
                format!("fold in a tool result lead {lead}"),
                in_tool_result(&folded),
            ));
            cases.push((
                format!("fold in hermes lead {lead}"),
                in_hermes_tool_result(&folded),
            ));
        }
        cases.push((format!("fold -w 20 lead {lead}"), folded));
    }
    let percent: String = PROXY.bytes().map(|b| format!("%{b:02X}")).collect();
    for width in [20, 76] {
        cases.push((
            format!("percent folded at {width}"),
            wrap(percent.as_bytes(), width, "\n"),
        ));
    }
    cases
}

/// A wrapped form leaves the chain too: in one push and split at every byte, each equal to the
/// oracle, and none holds a form once its breaks are taken out (C1, C1b).
#[test]
fn a_wrapped_run_secret_is_never_persisted() {
    for (name, input) in wrapped_cases() {
        assert!(leaks(&input), "{name}: the case holds the value");
        let (want_bytes, want_spans) = oracle(&input);
        assert!(!leaks(&want_bytes), "{name}: the oracle redacts it");
        for split in 0..=input.len() {
            let mut p = Producer::with_redactor(P1, redactor());
            let mut records = p.open(1, CaptureSource::WireRequest);
            records.extend(p.bytes(1, &input[..split]));
            records.extend(p.redactor.flush(daemon_time(1)).unwrap());
            records.extend(p.bytes(1, &input[split..]));
            records.extend(p.end(1, EndReason::Complete));
            let (bytes, spans) = retained(&records);
            assert!(!leaks(&bytes), "{name}: split at {split}");
            assert_eq!(bytes, want_bytes, "{name}: bytes differ, split at {split}");
            assert_eq!(spans, want_spans, "{name}: audit differs, split at {split}");
        }
        let (bytes, spans) = retained(&stream_through(redactor(), &input, &[1], 3));
        assert_eq!(
            (bytes, spans),
            (want_bytes, want_spans),
            "{name}: a byte per push"
        );
    }
}

/// The longest wrapping of the longest form a run secret can have, `\u`-escaped with a doubly
/// escaped CR LF in every gap, is held whole and redacted, below the hold bound.
#[test]
fn the_longest_wrapped_form_fits_the_hold() {
    use goalcup_daemon::redact::stream::MAX_HELD_BYTES;
    use goalcup_protocol::run_secret::{MAX_RUN_SECRET_BYTES, MAX_WRAPPED_FORM_BYTES};
    const { assert!(MAX_WRAPPED_FORM_BYTES <= MAX_HELD_BYTES) };
    // Not periodic, so that few partial matches of it are ever live at once.
    const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let value: String = (0..MAX_RUN_SECRET_BYTES)
        .map(|i| char::from(ALNUM[(i * 7 + i / 62) % ALNUM.len()]))
        .collect();
    let secrets = RunSecrets::new(&[RunSecret::new(value.as_bytes()).unwrap()]).unwrap();
    // Every gap gets the longest break it may: a doubly escaped CR LF, or a raw CR LF before a
    // backslash of the form (an escaped break is not taken there).
    let form: String = value.bytes().map(|b| format!("\\u{b:04x}")).collect();
    let mut text = String::from("x");
    for (i, c) in form.chars().enumerate() {
        if i > 0 {
            text.push_str(if c == '\\' { "\r\n" } else { "\\\\r\\\\n" });
        }
        text.push(c);
    }
    text.push_str("y\n");
    assert!(text.len() - 3 <= MAX_WRAPPED_FORM_BYTES, "{}", text.len());
    assert!(text.len() > 8 * 1024, "{}", text.len());
    let (bytes, spans) = retained(&stream_through(
        StreamRedactor::for_run(&[], &secrets).unwrap(),
        text.as_bytes(),
        &[97],
        1,
    ));
    assert_eq!(
        bytes,
        b"x<redacted:run_secret>y\n",
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(spans.len(), 1);
}

/// A stream's metadata strings (its label and path) lose the value too.
#[test]
fn a_run_secret_in_metadata_is_redacted() {
    let mut p = Producer::with_redactor(P1, redactor());
    let mut metadata = http_meta(CaptureSource::WireRequest);
    metadata.label = format!("CONNECT goalcup:{PROXY}@127.0.0.1");
    metadata.path = Some(format!("/v1/{PROXY}/responses?k=1"));
    let records = p.push(
        1,
        FrameEvent::Open {
            source: CaptureSource::WireRequest,
            metadata,
        },
    );
    let text = serde_json::to_string(&records).unwrap();
    assert!(!text.contains(PROXY), "{text}");
    assert!(text.contains("<redacted:run_secret>"), "{text}");
}
