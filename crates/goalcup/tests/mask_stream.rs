//! The streaming masker against the whole-record masker (PRD-05 section 6, PRD-17 K1, K3, K4,
//! K8, K9, K15): however a stream is split, its bytes and spans equal `mask_whole` while no hold
//! overflows; past the hold it masks more, never less; a gap never releases a secret prefix raw;
//! a fail-closed window ends at the message. Every planted key is fake and built from pieces.

use goalcup::connector::{self, FieldValue, MaskPattern, MaskRule};
use goalcup::mask::{Masked, Masker, StreamMasker};
use goalcup_format::StreamKind;

type Res = Result<(), Box<dyn std::error::Error>>;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

fn run(set: &[u8], n: usize, seed: usize) -> String {
    (0..n)
        .map(|i| set[(i * 7 + seed * 3 + 3) % set.len()] as char)
        .collect()
}

/// Fake keys with the length of their fixed prefix.
fn keys() -> Vec<(String, usize)> {
    let upper = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    vec![
        (["sk-", "ant-", "api03-", &run(ALNUM, 30, 1)].concat(), 3),
        (["sk-", "proj-", &run(ALNUM, 30, 2)].concat(), 3),
        (["github", "_pat_", &run(ALNUM, 30, 3)].concat(), 11),
        (["gh", "p_", &run(ALNUM, 30, 4)].concat(), 4),
        (["AK", "IA", &run(upper, 16, 5)].concat(), 4),
        (
            [
                "ey",
                "J",
                &run(ALNUM, 12, 6),
                ".",
                &run(ALNUM, 12, 7),
                ".",
                &run(ALNUM, 12, 8),
            ]
            .concat(),
            3,
        ),
        (["xo", "xb-", &run(ALNUM, 20, 9)].concat(), 5),
        (["AI", "za", &run(ALNUM, 35, 10)].concat(), 4),
        (["h", "f_", &run(ALNUM, 30, 11)].concat(), 3),
        (["np", "m_", &run(ALNUM, 30, 12)].concat(), 4),
        (["xa", "i-", &run(ALNUM, 30, 13)].concat(), 4),
        (["sk", "_live_", &run(ALNUM, 24, 14)].concat(), 8),
        (["gl", "pat-", &run(ALNUM, 24, 15)].concat(), 6),
        (
            ["do", "p_v1_", &run(b"0123456789abcdef", 40, 16)].concat(),
            7,
        ),
    ]
}

/// The masker with a rule of every connector form: the shipped Codex and Claude Code rows and
/// a JSON string field.
fn masker() -> Result<Masker, Box<dyn std::error::Error>> {
    let mut rules: Vec<MaskRule> = connector::load("codex-cli", None)?.mask;
    rules.extend(connector::load("claude-code-cli", None)?.mask);
    rules.push(MaskRule {
        kind: "amp_ws_token".into(),
        pattern: MaskPattern::JsonField {
            field: "wsToken".into(),
            value: FieldValue::String,
        },
    });
    Ok(Masker::new(&rules, Vec::new())?)
}

/// Inputs that reach every rule, PEM blocks, escapes up to seven backslashes and invalid UTF-8.
fn planted() -> Vec<Vec<u8>> {
    let v = run(ALNUM, 24, 30);
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut line = Vec::new();
    for (key, _) in keys() {
        line.extend(format!("{{\"k\":\"{key}\"}}\n").bytes());
    }
    out.push(line);
    for k in 0..=7 {
        let b = "\\".repeat(k);
        out.push(format!("{{{b}\"api_key{b}\": {b}\"{v}{b}\"}} {b}\"x\":1\n").into_bytes());
    }
    out.push(
        format!("Authorization: Bearer {v}\nexport API_KEY={v}\ntoken = '{v}'\n").into_bytes(),
    );
    out.push(
        format!("grant_type=refresh_token&refresh_token=1%2F%2F0g{v}&x=0.AX{v}\n").into_bytes(),
    );
    out.push(
        format!("git remote -v\nhttps://u:{v}@h.example and {{\"u\":\"https:\\/\\/a:{v}@b\"}}\n")
            .into_bytes(),
    );
    out.push(b"see sk-abcde...wxyz and sk-0\xe2\x80\xa6cdef and \"sk-ab\\u2026wxyz\"\n".to_vec());
    out.push(
        format!(
            "-----BEGIN RSA PRIVATE KEY-----\n{v}\n{v}\n-----END RSA PRIVATE KEY-----\n{{\"c\":\"-----BEGIN PRIVATE KEY-----\\n{v}\\\"{v}\\n-----END PRIVATE KEY-----\"}}\n"
        )
        .into_bytes(),
    );
    out.push(
        format!(
            "{{\"d\":\"-----BEGIN EC PRIVATE KEY-----\\n{v}\"}}\n{{\"d\":\"{v}\\n-----END\"}}\n"
        )
        .into_bytes(),
    );
    out.push(
        format!(
            "{{\"wsToken\":\"{v}\",\"x-codex-turn-state\": \"{v}\",\"safety_identifier\":17}}\nx-anthropic-billing-header: cc_version=2.1; {v}\n"
        )
        .into_bytes(),
    );
    let gh = keys()[3].0.clone();
    let mut bad = format!("{{\"k\":\"{}", &gh[..10]).into_bytes();
    bad.push(0xff);
    bad.extend(format!("{}\"}}\n", &gh[10..]).bytes());
    bad.extend(b"\xff\xfe");
    bad.extend(format!(" x \\\\\\n{gh} \u{e9}").bytes());
    bad.extend(b"\xe2\x80");
    out.push(bad);
    out.push(format!("\x1b[32m{gh}\x1b[0m \\\\u001b[1m{gh} xx{gh} %3D{gh}").into_bytes());
    out
}

/// The output of one stream fed in `pieces`, ended by `finish`, with spans made absolute.
fn stream_in(m: &Masker, input: &[u8], cuts: &[usize]) -> Masked {
    let mut s = m.stream(StreamKind::Native);
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

fn append(all: &mut Masked, piece: Masked) {
    let base = all.bytes.len();
    all.bytes.extend(piece.bytes);
    all.spans.extend(piece.spans.into_iter().map(|mut s| {
        s.start += base;
        s
    }));
}

fn random_cuts(rng: &mut Rng, len: usize, max: usize) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut at = 0;
    while at < len {
        at += 1 + rng.below(max);
        cuts.push(at.min(len));
    }
    cuts
}

fn starred_set(out: &Masked) -> Vec<bool> {
    let mut set = vec![false; out.bytes.len()];
    for s in &out.spans {
        for b in &mut set[s.start..s.start + s.len] {
            *b = true;
        }
    }
    set
}

/// K1: at every split point and over seeded random chunkings, the stream equals `mask_whole`,
/// and its length is the input's.
#[test]
fn every_split_equals_the_whole_record_masker() -> Res {
    let m = masker()?;
    let mut rng = Rng(0xC0FF_EE11);
    for input in planted() {
        let whole = m.mask_whole(&input);
        assert_eq!(whole.bytes.len(), input.len());
        assert!(
            !whole.spans.is_empty(),
            "{:?}",
            String::from_utf8_lossy(&input)
        );
        for split in 0..=input.len() {
            let got = stream_in(&m, &input, &[split]);
            assert_eq!(
                got,
                whole,
                "split {split} of {:?}",
                String::from_utf8_lossy(&input)
            );
        }
        for max in [1, 3, 17, 64] {
            for _ in 0..5 {
                let cuts = random_cuts(&mut rng, input.len(), max);
                assert_eq!(stream_in(&m, &input, &cuts), whole, "cuts {cuts:?}");
            }
        }
    }
    Ok(())
}

/// Past the hold the stream masks more than the whole-record masker, never less, and keeps
/// every length.
#[test]
fn never_less_past_the_hold() -> Res {
    let m = masker()?;
    let gh = keys()[3].0.clone();
    let long = |unit: &str| unit.repeat(20 * 1024 / unit.len() + 1);
    let cases = [
        format!("{{\"api_key\":\"{}\"}}\n{gh}\n", long("a")),
        format!(
            "-----BEGIN RSA PRIVATE KEY-----\n{} {gh}\nok {gh}\n",
            long("MIIEowIB")
        ),
        format!("Authorization: Bearer {}\n{gh}\n", long("Ab9_")),
        format!("{{\"x\":\"{}\"}}\n{gh}\n", long("sk-")),
        format!("{}{gh}\n", long("\\")),
        format!("https://u:{}@h\n{gh}\n", long("p")),
    ];
    let mut rng = Rng(0x5EC2E7);
    for input in &cases {
        let input = input.as_bytes();
        let want = starred_set(&m.mask_whole(input));
        let max = rng.below(3000) + 1;
        for cuts in [
            vec![16 * 1024],
            vec![1, 8, 64],
            random_cuts(&mut rng, input.len(), max),
        ] {
            let got = stream_in(&m, input, &cuts);
            assert_eq!(got.bytes.len(), input.len());
            let have = starred_set(&got);
            let missing = want.iter().zip(&have).filter(|(w, h)| **w && !**h).count();
            assert_eq!(missing, 0, "{missing} whole-record bytes released raw");
            let tail = &got.bytes[got.bytes.len() - gh.len() - 1..got.bytes.len() - 1];
            assert!(
                tail.iter().all(|&b| b == b'*'),
                "the key after the overflow is masked"
            );
        }
    }
    Ok(())
}

/// K9: a gap never releases a secret prefix raw once its fixed prefix is complete, and a
/// message end decides the held bytes exactly as `mask_whole` of the message.
#[test]
fn a_gap_or_boundary_never_releases_a_secret_prefix_raw() -> Res {
    let m = masker()?;
    for (key, prefix) in keys() {
        let input = format!("{{\"k\":\"{key}\"}}");
        let at = input.find(&key).unwrap_or(0);
        for k in at..=at + key.len() {
            let mut s = m.stream(StreamKind::Native);
            let mut out = s.push(&input.as_bytes()[..k]);
            append(&mut out, s.gap());
            assert_eq!(out.bytes.len(), k);
            if k >= at + prefix {
                assert!(
                    out.bytes[at..k].iter().all(|&b| b == b'*'),
                    "{key} cut at {k}"
                );
            }
            let mut s = m.stream(StreamKind::WsUp);
            let mut out = s.push(&input.as_bytes()[..k]);
            append(&mut out, s.boundary());
            assert_eq!(
                out,
                m.mask_whole(&input.as_bytes()[..k]),
                "{key} message cut at {k}"
            );
        }
    }
    Ok(())
}

fn messages(s: &mut StreamMasker, input: &[u8]) -> Masked {
    let mut out = Masked::default();
    for piece in input.chunks(4096) {
        append(&mut out, s.push(piece));
    }
    append(&mut out, s.boundary());
    out
}

/// K15: a 20 KiB undecided candidate in message 1 of a `ws_up` stream fails closed, and the
/// window ends at the message: messages 2 to 11 come through byte for byte.
#[test]
fn a_fail_closed_window_ends_at_the_message() -> Res {
    let m = masker()?;
    let fail_closed = m.mask_rules().len() - 1;
    let mut s = m.stream(StreamKind::WsUp);
    let first = format!(
        r#"{{"type":"response.create","input":"sk-{}"}}"#,
        "A9".repeat(10 * 1024)
    );
    let out = messages(&mut s, first.as_bytes());
    assert_eq!(out.bytes.len(), first.len());
    assert!(
        out.spans.iter().any(|s| s.rule == fail_closed),
        "the overflow is fail_closed"
    );
    for i in 2..=11 {
        let plain = format!(r#"{{"type":"response.create","n":{i},"input":"hello there"}}"#);
        let out = messages(&mut s, plain.as_bytes());
        assert_eq!(out.bytes, plain.as_bytes(), "message {i}");
        assert!(out.spans.is_empty(), "message {i}");
    }
    Ok(())
}

/// The guard ring is cleared at a message end: a key that starts message 2 is masked even when
/// message 1 ended in a letter.
#[test]
fn a_key_right_after_a_boundary_is_masked() -> Res {
    let m = masker()?;
    let gh = keys()[3].0.clone();
    let mut s = m.stream(StreamKind::WsUp);
    let out = messages(&mut s, b"previous message ends in a letter x");
    assert!(out.spans.is_empty());
    let out = messages(&mut s, format!("{gh} rest").as_bytes());
    assert!(out.bytes[..gh.len()].iter().all(|&b| b == b'*'));
    Ok(())
}

/// K4: Claude Code's Read tool prints a key with `N→` before every line, JSON-escaped. It is
/// one masked block at every split point, and the record after it is untouched.
#[test]
fn a_key_printed_by_the_read_tool_is_one_block() -> Res {
    let m = masker()?;
    let body = run(ALNUM, 64, 60);
    let lines = [
        "-----BEGIN RSA PRIVATE KEY-----".to_string(),
        body.clone(),
        body.clone(),
        body,
        "-----END RSA PRIVATE KEY-----".to_string(),
    ];
    let numbered: Vec<String> = (1..)
        .zip(&lines)
        .map(|(n, line)| format!("{n:>6}\u{2192}{line}"))
        .collect();
    let input = format!(
        "{{\"type\":\"tool_result\",\"content\":\"{}\"}}\n{{\"next\":1}}\n",
        numbered.join("\\n")
    );
    let input = input.as_bytes();
    let whole = m.mask_whole(input);
    let text = String::from_utf8(whole.bytes.clone())?;
    let lead = "{\"type\":\"tool_result\",\"content\":\"     1\u{2192}";
    assert!(text.starts_with(lead), "{text}");
    assert!(text.ends_with("\"}\n{\"next\":1}\n"), "{text}");
    let inner = &text[lead.len()..text.len() - "\"}\n{\"next\":1}\n".len()];
    assert!(inner.bytes().all(|b| b == b'*'), "{text}");
    assert_eq!(whole.spans.len(), 1);
    for split in 0..=input.len() {
        assert_eq!(stream_in(&m, input, &[split]), whole, "split at {split}");
    }
    Ok(())
}

/// A long Codex reasoning item (its largest `encrypted_content`, repeated to 18 KiB) has no
/// secret: it streams through byte for byte at every chunking, and each line stays JSON.
#[test]
fn a_long_codex_reasoning_item_streams_through_intact() -> Res {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/mask_corpus/codex-app/rollout-main-01a06657.jsonl");
    let rollout = std::fs::read_to_string(path)?;
    let (line, token) = rollout
        .lines()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let token = v["payload"]["encrypted_content"].as_str()?.to_string();
            Some((line, token))
        })
        .max_by_key(|(_, token)| token.len())
        .ok_or("a reasoning item")?;
    let long = format!("{}=", token.trim_end_matches('=').repeat(8));
    assert!(long.len() > 16 * 1024);
    let input = format!(
        "{}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\"}}}}\n",
        line.replace(&token, &long)
    );
    let input = input.as_bytes();
    let m = masker()?;
    let whole = m.mask_whole(input);
    assert_eq!(whole.bytes, input);
    let mut rng = Rng(0xC0DE_C0DE);
    for cuts in [
        vec![16 * 1024],
        vec![4096, 8192, 12288],
        random_cuts(&mut rng, input.len(), 3000),
    ] {
        let got = stream_in(&m, input, &cuts);
        assert!(got.spans.is_empty());
        assert!(got.bytes == input, "the record changed");
        for record in got.bytes.split(|&b| b == b'\n').filter(|r| !r.is_empty()) {
            serde_json::from_slice::<serde_json::Value>(record)?;
        }
    }
    Ok(())
}

/// A Codex `rate_limits` object is data, not a secret, with the Codex connector's rules loaded.
#[test]
fn a_codex_rate_limit_object_streams_through_with_its_rules() -> Res {
    let m = Masker::new(&connector::load("codex-cli", None)?.mask, Vec::new())?;
    let input = concat!(
        r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":"#,
        r#"{"used_percent":12.0,"window_minutes":300,"resets_in_seconds":4211}},"#,
        r#""plan_type":"plus","credits":{"has_credits":true,"balance":"17.5"}}}"#,
        "\n"
    )
    .as_bytes();
    for split in 0..=input.len() {
        let got = stream_in(&m, input, &[split]);
        assert_eq!(got.bytes, input);
        assert!(got.spans.is_empty());
    }
    Ok(())
}
