//! The shared secret shapes (seam S12, PRD-05 section 2) and the rules they make.
//!
//! This part reads `src/mask/shapes.tsv` on its own, through `include_str!`, the way the
//! verifier does, so it pins the data file without any `mask::` API.

#[allow(dead_code)]
#[path = "mask_corpus/common.rs"]
mod common;

use common::{ALNUM, Rng, keys, only_spans_changed, run, shape_rows, starred};
use regex::bytes::RegexBuilder;

struct Shape {
    kind: String,
    guard: bool,
}

/// Every row compiled with the builder settings `compile` in `goalcup-daemon` uses today.
fn shapes() -> Vec<Shape> {
    shape_rows()
        .into_iter()
        .map(|row| {
            assert_eq!(row.len(), 3, "a row is kind, guard, regex: {row:?}");
            if let Err(e) = RegexBuilder::new(row[2]).build() {
                panic!("{} does not compile: {e}", row[0]);
            }
            Shape {
                kind: row[0].to_string(),
                guard: row[1] == "1",
            }
        })
        .collect()
}

/// The literal kinds in `shapes.tsv` order. The JSON literal (10) and the shell literal (11)
/// are fixed indexes the stream relies on; `url_password` (12) took the slot `email` left. The
/// pad's C3 reads these names and this order (PRD-05 amendments).
const KINDS: [&str; 20] = [
    "anthropic_key",
    "openai_key",
    "masked_key",
    "github_token",
    "github_token",
    "aws_access_key",
    "bearer",
    "jwt",
    "private_key",
    "slack_token",
    "generic_secret_assignment",
    "generic_secret_assignment",
    "url_password",
    "google_key",
    "huggingface_token",
    "npm_token",
    "xai_key",
    "stripe_key",
    "gitlab_token",
    "digitalocean_token",
];

#[test]
fn shapes_tsv_is_the_shared_list() {
    let shapes = shapes();
    let kinds: Vec<&str> = shapes.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(kinds, KINDS);

    let guarded: Vec<usize> = shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| s.guard)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(guarded, [0, 1, 3, 4, 5, 7, 9, 13, 14, 15, 16, 17, 18, 19]);
}

// ── the masker over whole inputs (PRD-05 sections 2 to 5) ─────────────────────────────────

use goalcup::connector::{self, FieldValue, MaskPattern, MaskRule};
use goalcup::mask::{FAIL_CLOSED, Masked, Masker};

type Res = Result<(), Box<dyn std::error::Error>>;

fn masker() -> Result<Masker, goalcup::mask::MaskError> {
    Masker::new(&[], Vec::new())
}

/// The kind name of each span of `out`.
fn kinds(m: &Masker, out: &Masked) -> Vec<String> {
    let rules = m.mask_rules();
    out.spans
        .iter()
        .map(|s| rules.get(s.rule).cloned().unwrap_or_default())
        .collect()
}

fn assert_masks(m: &Masker, input: &str, secret: &str) {
    let out = m.mask_whole(input.as_bytes());
    assert!(only_spans_changed(input.as_bytes(), &out));
    let at = input.find(secret).unwrap_or(usize::MAX);
    assert!(
        at != usize::MAX && starred(&out, at..at + secret.len()),
        "{input:?} not masked"
    );
}

fn assert_clear(m: &Masker, input: &str) {
    let out = m.mask_whole(input.as_bytes());
    assert_eq!(out.bytes, input.as_bytes(), "{input:?} changed");
    assert!(out.spans.is_empty(), "{input:?}: {:?}", kinds(m, &out));
}

#[test]
fn mask_rules_lists_kinds_then_connectors_then_run_secret_then_fail_closed() -> Res {
    let mut want: Vec<String> = Vec::new();
    for kind in KINDS {
        if !want.iter().any(|k| k == kind) {
            want.push(kind.to_string());
        }
    }
    assert_eq!(want.len(), 18);
    let plain: Vec<String> = [want.clone(), vec!["run_secret".into(), FAIL_CLOSED.into()]].concat();
    assert_eq!(masker()?.mask_rules(), plain);

    let codex = connector::load("codex-cli", None)?.mask;
    let mut rules = codex.clone();
    rules.push(codex[0].clone());
    let m = Masker::new(&rules, Vec::new())?;
    let mut with: Vec<String> = want.clone();
    for r in &codex {
        if !with.contains(&r.kind) {
            with.push(r.kind.clone());
        }
    }
    with.extend(["run_secret".to_string(), FAIL_CLOSED.to_string()]);
    assert_eq!(m.mask_rules(), with);
    assert_eq!(m.mask_rules().last().map(String::as_str), Some(FAIL_CLOSED));
    Ok(())
}

#[test]
fn url_password_cases() -> Res {
    let m = masker()?;
    let pass = ["hun", "ter2"].concat();
    let out = m.mask_whole(format!("postgres://app:{pass}@localhost:5432").as_bytes());
    assert_eq!(out.bytes, b"postgres://app:*******@localhost:5432");
    assert_eq!(kinds(&m, &out), ["url_password"]);
    assert_masks(&m, &format!(r#"{{"u":"https:\/\/u:{pass}@h"}}"#), &pass);
    assert_masks(
        &m,
        &format!("git remote -v\norigin\thttps://bob:{pass}@git.example.test/r"),
        &pass,
    );
    for left_alone in [
        "ssh://git@github.com",
        "https://host:443/path",
        "npm:react@18",
        "mail alice@example.com",
    ] {
        assert_clear(&m, left_alone);
    }
    // Over-masked, accepted (section 5).
    let out = m.mask_whole(b"//127.0.0.1:8080@x");
    assert_eq!(out.bytes, b"//127.0.0.1:****@x");
    let out = m.mask_whole(b"// a note //a:b@c");
    assert_eq!(out.bytes, b"// a note //a:*@c");
    Ok(())
}

/// K12: a URL credential of 81 or 190 characters stays masked (`url_password` took over from
/// the dropped `email` rule).
#[test]
fn long_url_credentials_stay_masked() -> Res {
    let m = masker()?;
    let t81 = run(ALNUM, 81, 20);
    let t190 = run(ALNUM, 190, 21);
    assert_masks(
        &m,
        &format!("curl https://user:{t81}@artifacts.example.com/x\n"),
        &t81,
    );
    assert_masks(
        &m,
        &format!("origin\thttps://x-token-auth:{t190}@bitbucket.org/team/repo.git (fetch)\n"),
        &t190,
    );
    Ok(())
}

/// Section 4: each allowed left context masks a guarded key, each glue byte does not.
#[test]
fn the_left_guard_keeps_escapes_and_drops_glue() -> Res {
    let m = masker()?;
    let contexts = [
        "",
        "\n",
        "\\n",
        "\\\\n",
        "\\\\\\\\n",
        "\t",
        "\\t",
        "\\u0041",
        "\\x41",
        "%3D",
        "%20",
        "\x1b[32m",
        "\\u001b[32m",
        "\\\\u001b[1;32m",
        "\\x1b[0m",
        "-",
        "token-",
        " ",
        "=",
        "\"",
    ];
    for (kind, key, _) in keys() {
        for ctx in contexts {
            let input = format!("{ctx}{key} end");
            let out = m.mask_whole(input.as_bytes());
            assert!(only_spans_changed(input.as_bytes(), &out));
            assert!(
                starred(&out, ctx.len()..ctx.len() + key.len()),
                "{kind} after {ctx:?} not masked"
            );
        }
        for glue in ["x", "Z", "7", "_", "KEY_", "\\\\"] {
            let input = format!("{glue}{key} end");
            let out = m.mask_whole(input.as_bytes());
            let raw = &out.bytes[glue.len()..glue.len() + key.len()];
            if glue == "\\\\" {
                // Two backslashes are an escaped backslash, not glue.
                assert!(raw.iter().all(|&b| b == b'*'), "{kind} after {glue:?}");
            } else {
                assert_eq!(raw, key.as_bytes(), "{kind} after {glue:?} was masked");
            }
        }
    }
    Ok(())
}

/// A long base64url run (Codex's `encrypted_content`) with key prefixes inside, none right
/// after a `-`, comes through byte for byte, whole and streamed, and the stream never holds
/// it. A `-sk-` inside shows the hit the default class `[A-Za-z0-9_]` lets through (PRD-00 owner
/// question 10).
#[test]
fn a_base64url_blob_with_key_prefixes_inside_is_left_alone() -> Res {
    let m = masker()?;
    let mut blob = String::new();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_";
    let mut rng = Rng(0x5EED);
    let planted = ["sk-", "hf_", "ghp_", "AIza", "eyJ"];
    while blob.len() < 20 * 1024 {
        let x = rng.next();
        match x % 97 {
            0 => blob.push_str("-9"),
            1 => blob.push_str(planted[(x / 97 % 5) as usize]),
            _ => blob.push(alphabet[(x / 97 % 63) as usize] as char),
        }
    }
    let input = format!(r#"{{"type":"reasoning","encrypted_content":"Q{blob}"}}"#);
    assert_clear(&m, &input);
    let mut s = m.stream(goalcup_format::StreamKind::WsUp);
    let (mut pushed, mut back) = (0usize, 0usize);
    let mut all = Vec::new();
    for piece in input.as_bytes().chunks(1000) {
        pushed += piece.len();
        let out = s.push(piece);
        back += out.bytes.len();
        assert!(out.spans.is_empty());
        all.extend(out.bytes);
        assert!(pushed - back <= 2100, "{} bytes held", pushed - back);
    }
    let out = s.boundary();
    assert!(out.spans.is_empty());
    all.extend(out.bytes);
    assert_eq!(all, input.as_bytes());

    let key = ["sk-", &run(ALNUM, 30, 30)].concat();
    let input = format!(r#"{{"encrypted_content":"Qab-{key}-rest_of_blob"}}"#);
    let out = m.mask_whole(input.as_bytes());
    assert_eq!(kinds(&m, &out), ["openai_key"]);
    let at = input.find(&key).unwrap_or(0);
    assert!(starred(&out, at..at + key.len()));
    Ok(())
}

/// K2: form-encoded refresh tokens are masked; a path after `--token=` is not.
#[test]
fn form_encoded_refresh_tokens_are_masked() -> Res {
    let m = masker()?;
    assert_masks(
        &m,
        r#"{"refresh_token":"1\/\/0gAbCdEfGhIjKlMnOpQrStUv"}"#,
        "0gAbCdEfGhIjKlMnOpQrStUv",
    );
    assert_masks(
        &m,
        "grant_type=refresh_token&refresh_token=1%2F%2F0gAbCdEfGhIjKlMnOpQrStUv",
        "0gAbCdEfGhIjKlMnOpQrStUv",
    );
    assert_masks(
        &m,
        "grant_type=refresh_token&refresh_token=0.AXoA-bcdefghijklmnopqrstuvwxyz_0123",
        "AXoA-bcdefghijklmnopqrstuvwxyz_0123",
    );
    for probe in [
        "git clone --token=src/components/some-long-file-name.tsx",
        "let token = entry.keys.signing_key",
        "token=0123456789abcd%2F",
    ] {
        assert_clear(&m, probe);
    }
    Ok(())
}

/// K7: a provider key echoed masked is masked again, as `masked_key`.
#[test]
fn masked_key_forms_are_masked() -> Res {
    let m = masker()?;
    let stars = "*".repeat(39);
    for (text, mask) in [
        (
            "Incorrect API key provided: sk-abcde...wxyz.",
            "sk-abcde...wxyz",
        ),
        ("key sk-0...cdef here", "sk-0...cdef"),
        ("'sk-ab\u{2026}wxyz'", "sk-ab\u{2026}wxyz"),
        (r#""sk-ab…wxyz""#, r"sk-ab…wxyz"),
        (
            &format!("sk-R7aMV{stars}oKYo"),
            &format!("sk-R7aMV{stars}oKYo"),
        ),
    ] {
        let out = m.mask_whole(text.as_bytes());
        assert!(only_spans_changed(text.as_bytes(), &out));
        assert_eq!(kinds(&m, &out), ["masked_key"], "{text}");
        let at = text.find(mask).unwrap_or(0);
        assert!(starred(&out, at..at + mask.len()), "{text}");
    }
    Ok(())
}

/// Section 3: no rule span holds an unescaped quote or splits an escape, and a span over part
/// of a character covers the character.
#[test]
fn no_rule_span_holds_a_quote_or_splits_an_escape() -> Res {
    let m = masker()?;
    let key = ["gh", "p_", &run(ALNUM, 30, 40)].concat();
    let value = run(ALNUM, 24, 41);
    let inputs = [
        format!("password: \"{value}\""),
        format!("password: \\\"{value}\\\""),
        format!("{{\\\"api_key\\\": \\\"{value}\\\"}}"),
        format!("{{\"text\":\"line\\n{key}\\nnext\"}}"),
        format!("{{\"text\":\"\\\\\\n{key}\\\"\"}}"),
        format!("token = '{value}'"),
        format!("Bearer {value}\\\"x"),
        format!("{{\"k\":\"\u{e9}{key}\u{e9}\"}}"),
        format!(
            "{{\"c\":\"-----BEGIN RSA PRIVATE KEY-----\\n{value}\\\"{value}\\n-----END RSA PRIVATE KEY-----\"}}"
        ),
    ];
    let fail_closed = m.mask_rules().len() - 1;
    for input in &inputs {
        let b = input.as_bytes();
        let out = m.mask_whole(b);
        assert!(only_spans_changed(b, &out), "{input}");
        assert!(!out.spans.is_empty(), "{input}");
        assert!(std::str::from_utf8(&out.bytes).is_ok(), "{input}");
        for s in out.spans.iter().filter(|s| s.rule != fail_closed) {
            let before = |at: usize| b[..at].iter().rev().take_while(|&&c| c == b'\\').count();
            let quote = (s.start..s.start + s.len).any(|at| b[at] == b'"' && before(at) % 2 == 0);
            assert!(!quote, "{input}: a span holds a quote");
            assert_eq!(
                before(s.start) % 2,
                0,
                "{input}: a span starts inside an escape"
            );
            let end = s.start + s.len;
            assert!(
                end == b.len() || before(end) % 2 == 0,
                "{input}: a span ends on an escape"
            );
        }
    }
    Ok(())
}

/// K11: a connector JSON field rule masks the value and keeps the quotes.
#[test]
fn a_connector_field_rule_masks_its_value() -> Res {
    let rule = MaskRule {
        kind: "amp_ws_token".into(),
        pattern: MaskPattern::JsonField {
            field: "wsToken".into(),
            value: FieldValue::String,
        },
    };
    let m = Masker::new(&[rule], Vec::new())?;
    let token = run(ALNUM, 12, 50);
    let input = format!(r#"{{"wsToken":"{token}","x":1}}"#);
    let out = m.mask_whole(input.as_bytes());
    assert_eq!(kinds(&m, &out), ["amp_ws_token"]);
    assert_eq!(
        String::from_utf8(out.bytes)?,
        format!(r#"{{"wsToken":"{}","x":1}}"#, "*".repeat(token.len()))
    );

    let codex = connector::load("codex-cli", None)?.mask;
    let m = Masker::new(&codex, Vec::new())?;
    let state = run(ALNUM, 10, 51);
    let input = format!(r#"{{"x-codex-turn-state": "{state}"}}"#);
    let out = m.mask_whole(input.as_bytes());
    assert_eq!(kinds(&m, &out), ["codex_turn_state"]);
    assert_eq!(
        String::from_utf8(out.bytes)?,
        format!(r#"{{"x-codex-turn-state": "{}"}}"#, "*".repeat(state.len()))
    );
    Ok(())
}

/// Review X F1: a long run of glued prefixes (an agent writing kebab-case, or `…sk-…` runs) is
/// one pass, whole and streamed in 16 KiB pieces, and nothing in it is masked. The time is
/// printed, not asserted; the search it replaced took about 35 s for 256 KiB.
#[test]
fn a_long_run_of_glued_prefixes_is_one_pass() -> Res {
    let m = masker()?;
    for (unit, size) in [("task-", 1 << 20), ("Xsk-", 1 << 18), ("xAIza", 1 << 18)] {
        let input = unit.repeat(size / unit.len()).into_bytes();
        let started = std::time::Instant::now();
        assert_eq!(m.mask_whole(&input).bytes, input, "{unit}");
        let mut s = m.stream(goalcup_format::StreamKind::Native);
        let mut back = Vec::with_capacity(input.len());
        for piece in input.chunks(16 * 1024) {
            back.extend(s.push(piece).bytes);
        }
        back.extend(s.finish().bytes);
        assert_eq!(back, input, "{unit} streamed");
        println!(
            "{unit} x {}: {:.2} s whole and streamed",
            input.len() / unit.len(),
            started.elapsed().as_secs_f64()
        );
    }
    Ok(())
}
