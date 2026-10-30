//! The streaming masker against the whole-record masker (PRD-05 section 6, PRD-17 K1, K3, K4,
//! K8, K9, K15): however a stream is split, its bytes and spans equal `mask_whole` while no hold
//! overflows; past the hold it masks more, never less; a gap never releases a secret prefix raw;
//! a fail-closed window ends at the message. Every planted key is fake and built from pieces.

#[allow(dead_code)]
#[path = "mask_corpus/common.rs"]
mod common;

use common::{ALNUM, Rng, append, keys, random_cuts, run, starred};
use goalcup::connector::{self, FieldValue, MaskPattern, MaskRule};
use goalcup::mask::{Masked, Masker, SecretValue, StreamMasker, ValueSource};
use goalcup_format::StreamKind;

type Res = Result<(), Box<dyn std::error::Error>>;

fn stream_in(m: &Masker, input: &[u8], cuts: &[usize]) -> Masked {
    common::stream_in(m, StreamKind::Native, input, cuts)
}

/// The bytes `mask_whole` stars that `got` leaves raw (input stars aside).
fn released_raw(input: &[u8], whole: &Masked, got: &Masked) -> usize {
    (0..input.len())
        .filter(|&i| whole.bytes[i] == b'*' && input[i] != b'*' && got.bytes[i] != b'*')
        .count()
}

/// The masker with a rule of every connector form: two shipped connectors' rows and
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
    for (_, key, _) in keys() {
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
    let gh = keys()[3].1.clone();
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
    let gh = keys()[3].1.clone();
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
        let whole = m.mask_whole(input);
        let max = rng.below(3000) + 1;
        for cuts in [
            vec![16 * 1024],
            vec![1, 8, 64],
            random_cuts(&mut rng, input.len(), max),
        ] {
            let got = stream_in(&m, input, &cuts);
            assert_eq!(got.bytes.len(), input.len());
            let missing = released_raw(input, &whole, &got);
            assert_eq!(missing, 0, "{missing} whole-record bytes released raw");
            let end = input.len() - 1;
            assert!(
                starred(&got, end - gh.len()..end),
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
    for (_, key, prefix) in keys() {
        let input = format!("{{\"k\":\"{key}\"}}");
        let at = input.find(&key).unwrap_or(0);
        for k in at..=at + key.len() {
            let mut s = m.stream(StreamKind::Native);
            let mut out = s.push(&input.as_bytes()[..k]);
            append(&mut out, s.gap());
            assert_eq!(out.bytes.len(), k);
            if k >= at + prefix {
                assert!(starred(&out, at..k), "{key} cut at {k}");
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
    let gh = keys()[3].1.clone();
    let mut s = m.stream(StreamKind::WsUp);
    let out = messages(&mut s, b"previous message ends in a letter x");
    assert!(out.spans.is_empty());
    let out = messages(&mut s, format!("{gh} rest").as_bytes());
    assert!(starred(&out, 0..gh.len()));
    Ok(())
}

/// K4: a file-reading tool prints a key with `N→` before every line, JSON-escaped. It is one
/// masked block at every split point, and the record after it is untouched.
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

/// An email and a Codex `rate_limits` object are data, not secrets, with the Codex connector's
/// rules loaded.
#[test]
fn an_email_and_a_codex_rate_limit_object_stream_through() -> Res {
    let m = Masker::new(&connector::load("codex-cli", None)?.mask, Vec::new())?;
    let input = concat!(
        r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":"#,
        r#"{"used_percent":12.0,"window_minutes":300,"resets_in_seconds":4211}},"#,
        r#""plan_type":"plus","credits":{"has_credits":true,"balance":"17.5"}}}"#,
        "\nAuthor: Ada Lovelace <ada.lovelace@example.com>\n"
    )
    .as_bytes();
    for split in 0..=input.len() {
        let got = stream_in(&m, input, &[split]);
        assert_eq!(got.bytes, input);
        assert!(got.spans.is_empty());
    }
    Ok(())
}

/// Breaker case B1: a PEM block meets an exact value and a connector header or a shell value
/// whose match runs past the block's END line (or past the hold's cut). Exact values are blocks
/// too, so the blocks nest; the claim that crosses the block's edge is still absorbed, whole and
/// streamed. At every split point a message end equals `mask_whole`, and a gap never releases a
/// byte `mask_whole` stars.
#[test]
fn a_pem_block_with_an_exact_value_absorbs_a_claim_past_its_end() -> Res {
    let value = ["gh", "p_", &Rng(77).chars(ALNUM, 36)].concat();
    let scalar = |kind: &str, field: &str| MaskRule {
        kind: kind.into(),
        pattern: MaskPattern::JsonField {
            field: field.into(),
            value: FieldValue::Scalar,
        },
    };
    let rules = [
        MaskRule {
            kind: "billing_header".into(),
            pattern: MaskPattern::HeaderPrefix("x-vendor-billing-".into()),
        },
        scalar("installation_id", "x-codex-installation-id"),
        scalar("turn_state", "x-codex-turn-state"),
    ];
    let secret = SecretValue::new(ValueSource::Workspace, value.clone().into_bytes());
    let m = Masker::new(&rules, vec![secret])?;
    let (begin, end) = ("-----BEGIN PRIVATE KEY-----", "-----END PRIVATE KEY-----");
    let header = "x-vendor-billing-x:";
    let v = &value;
    // Each input with the text of the claim that runs past a block's END line.
    let cases = [
        (format!("{begin}{v}\"{v}\".{v}{header}{end}e\"\x1b{v}-{v}"), format!("{end}e")),
        (format!("{begin}\"n{v}{header}{end}\\\n{begin}"), format!("{end}\\")),
        (format!("{begin}\"_{v}{header}{end}@\"{begin}"), format!("{end}@")),
        (format!("{begin}{end}{begin}{v}apikey=\"----BAKI{end}"), "----BAKI-----END".into()),
        (format!("{begin}{end}{begin}{v}{header}{end}:\""), format!("{end}:")),
        (format!("{v}{begin}{v}{header}{end}-\""), format!("{end}-")),
    ];
    for (input, claim) in &cases {
        let input = input.as_bytes();
        let whole = m.mask_whole(input);
        let at = input
            .windows(claim.len())
            .position(|w| w == claim.as_bytes())
            .ok_or("the claim")?;
        assert!(starred(&whole, at..at + claim.len()), "{claim:?} is masked");
        for split in 0..=input.len() {
            let mut s = m.stream(StreamKind::WsUp);
            let mut got = s.push(&input[..split]);
            append(&mut got, s.push(&input[split..]));
            append(&mut got, s.boundary());
            assert_eq!(got, whole, "message end, split {split}");
            let mut s = m.stream(StreamKind::WsUp);
            let mut got = s.push(&input[..split]);
            append(&mut got, s.push(&input[split..]));
            append(&mut got, s.gap());
            assert_eq!(released_raw(input, &whole, &got), 0, "gap, split {split}");
        }
    }
    Ok(())
}
