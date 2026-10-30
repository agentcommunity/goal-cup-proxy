//! The shared secret shapes (seam S12, PRD-05 section 2) and the rules they make.
//!
//! This part reads `src/mask/shapes.tsv` on its own, through `include_str!`, the way the
//! verifier does, so it pins the data file without any `mask::` API.

use regex::bytes::{Regex, RegexBuilder};

const SHAPES: &str = include_str!("../src/mask/shapes.tsv");

struct Shape {
    kind: String,
    guard: bool,
    re: Regex,
}

/// Every data row as its three raw fields, comments and blank lines skipped.
fn rows() -> Vec<Vec<&'static str>> {
    SHAPES
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').collect())
        .collect()
}

/// Every row compiled with the builder settings `compile` in `goalcup-daemon` uses today.
fn shapes() -> Vec<Shape> {
    rows()
        .into_iter()
        .map(|row| {
            assert_eq!(row.len(), 3, "a row is kind, guard, regex: {row:?}");
            let re = RegexBuilder::new(row[2])
                .build()
                .unwrap_or_else(|e| panic!("{} does not compile: {e}", row[0]));
            Shape {
                kind: row[0].to_string(),
                guard: row[1] == "1",
                re,
            }
        })
        .collect()
}

/// The span a row claims: group 1 when the regex has a group, else the whole match.
fn span<'h>(shape: &Shape, hay: &'h [u8]) -> Option<&'h [u8]> {
    let caps = shape.re.captures(hay)?;
    let m = if shape.re.captures_len() > 1 {
        caps.get(1)?
    } else {
        caps.get(0)?
    };
    Some(m.as_bytes())
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

#[test]
fn url_password_masks_only_the_password() {
    let shapes = shapes();
    let url = &shapes[12];
    assert_eq!(url.kind, "url_password");

    let password = ["hun", "ter2"].concat();
    let plain = format!("postgres://app:{password}@localhost:5432");
    assert_eq!(span(url, plain.as_bytes()), Some(password.as_bytes()));

    let pass = ["pa", "ss"].concat();
    let escaped = format!(r"https:\/\/u:{pass}@h");
    assert_eq!(span(url, escaped.as_bytes()), Some(pass.as_bytes()));

    for left_alone in [
        "ssh://git@github.com",
        "https://host:443/path",
        "npm:react@18",
    ] {
        assert_eq!(span(url, left_alone.as_bytes()), None, "{left_alone}");
    }
}

#[test]
fn an_email_is_no_longer_a_secret() {
    let shapes = shapes();
    let local = ["ali", "ce"].concat();
    for text in [
        format!("{local}@example.com"),
        "Author: A B <a.b@example.org>".to_string(),
    ] {
        for shape in &shapes {
            assert!(
                !shape.re.is_match(text.as_bytes()),
                "{} matches {text}",
                shape.kind
            );
        }
    }
}

// ── the masker over whole inputs (PRD-05 sections 2 to 5) ─────────────────────────────────

use goalcup::connector::{self, FieldValue, MaskPattern, MaskRule};
use goalcup::mask::{FAIL_CLOSED, Masked, Masker};

type Res = Result<(), Box<dyn std::error::Error>>;

fn masker() -> Result<Masker, goalcup::mask::MaskError> {
    Masker::new(&[], Vec::new())
}

/// `n` characters of `set`, deterministic, never a real key.
fn run(set: &[u8], n: usize, seed: usize) -> String {
    (0..n)
        .map(|i| set[(i * 7 + seed * 3 + 3) % set.len()] as char)
        .collect()
}

const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// One fake key per guarded literal, assembled from pieces.
fn guarded_keys() -> Vec<(&'static str, String)> {
    let hex = b"0123456789abcdef";
    let upper = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    vec![
        ("anthropic_key", ["sk-", "ant-", "api03-", &run(ALNUM, 30, 1)].concat()),
        ("openai_key", ["sk-", "proj-", &run(ALNUM, 30, 2)].concat()),
        ("github_token", ["github", "_pat_", &run(ALNUM, 30, 3)].concat()),
        ("github_token", ["gh", "p_", &run(ALNUM, 30, 4)].concat()),
        ("aws_access_key", ["AK", "IA", &run(upper, 16, 5)].concat()),
        (
            "jwt",
            ["ey", "J", &run(ALNUM, 12, 6), ".", &run(ALNUM, 12, 7), ".", &run(ALNUM, 12, 8)]
                .concat(),
        ),
        ("slack_token", ["xo", "xb-", &run(ALNUM, 20, 9)].concat()),
        ("google_key", ["AI", "za", &run(ALNUM, 35, 10)].concat()),
        ("huggingface_token", ["h", "f_", &run(ALNUM, 30, 11)].concat()),
        ("npm_token", ["np", "m_", &run(ALNUM, 30, 12)].concat()),
        ("xai_key", ["xa", "i-", &run(ALNUM, 30, 13)].concat()),
        ("stripe_key", ["sk", "_live_", &run(ALNUM, 24, 14)].concat()),
        ("gitlab_token", ["gl", "pat-", &run(ALNUM, 24, 15)].concat()),
        ("digitalocean_token", ["do", "p_v1_", &run(hex, 40, 16)].concat()),
    ]
}

/// The kind name of each span of `out`.
fn kinds(m: &Masker, out: &Masked) -> Vec<String> {
    let rules = m.mask_rules();
    out.spans
        .iter()
        .map(|s| rules.get(s.rule).cloned().unwrap_or_default())
        .collect()
}

/// Every masked byte is `*`, every span is inside the bytes, and the length is the input's.
fn check_shape(input: &[u8], out: &Masked) {
    assert_eq!(out.bytes.len(), input.len());
    let mut starred = vec![false; input.len()];
    for s in &out.spans {
        assert!(s.len > 0 && s.start + s.len <= input.len());
        for b in &mut starred[s.start..s.start + s.len] {
            *b = true;
        }
    }
    for (i, (&a, &b)) in input.iter().zip(&out.bytes).enumerate() {
        if starred[i] {
            assert_eq!(b, b'*', "byte {i} of a span is not a star");
        } else {
            assert_eq!(a, b, "byte {i} outside every span changed");
        }
    }
}

/// `secret` sits at `at` in `input`: is every byte of it starred?
fn starred(out: &Masked, at: usize, secret: &str) -> bool {
    out.bytes[at..at + secret.len()].iter().all(|&b| b == b'*')
}

fn assert_masks(m: &Masker, input: &str, secret: &str) {
    let out = m.mask_whole(input.as_bytes());
    check_shape(input.as_bytes(), &out);
    let at = input.find(secret).unwrap_or(usize::MAX);
    assert!(at != usize::MAX && starred(&out, at, secret), "{input:?} not masked");
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
    assert_masks(&m, &format!("git remote -v\norigin\thttps://bob:{pass}@git.example.test/r"), &pass);
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
    assert_masks(&m, &format!("curl https://user:{t81}@artifacts.example.com/x\n"), &t81);
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
        "", "\n", "\\n", "\\\\n", "\\\\\\\\n", "\t", "\\t", "\\u0041", "\\x41", "%3D", "%20",
        "\x1b[32m", "\\u001b[32m", "\\\\u001b[1;32m", "\\x1b[0m", "-", "token-", " ", "=", "\"",
    ];
    for (kind, key) in guarded_keys() {
        for ctx in contexts {
            let input = format!("{ctx}{key} end");
            let out = m.mask_whole(input.as_bytes());
            check_shape(input.as_bytes(), &out);
            assert!(starred(&out, ctx.len(), &key), "{kind} after {ctx:?} not masked");
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
    let mut x: u64 = 0x5EED;
    let planted = ["sk-", "hf_", "ghp_", "AIza", "eyJ"];
    while blob.len() < 20 * 1024 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
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
    assert!(starred(&out, input.find(&key).unwrap_or(0), &key));
    Ok(())
}

#[test]
fn an_email_and_a_codex_rate_limit_object_come_through() -> Res {
    let codex = connector::load("codex-cli", None)?.mask;
    for m in [masker()?, Masker::new(&codex, Vec::new())?] {
        assert_clear(&m, "Author: Ada Lovelace <ada.lovelace@example.com>\n");
        assert_clear(
            &m,
            concat!(
                r#"{"type":"token_count","rate_limits":{"primary":{"used_percent":12.0,"#,
                r#""window_minutes":300},"plan_type":"plus","credits":{"balance":"17.5"}}}"#
            ),
        );
    }
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
        ("Incorrect API key provided: sk-abcde...wxyz.", "sk-abcde...wxyz"),
        ("key sk-0...cdef here", "sk-0...cdef"),
        ("'sk-ab\u{2026}wxyz'", "sk-ab\u{2026}wxyz"),
        (r#""sk-ab…wxyz""#, r"sk-ab…wxyz"),
        (&format!("sk-R7aMV{stars}oKYo"), &format!("sk-R7aMV{stars}oKYo")),
    ] {
        let out = m.mask_whole(text.as_bytes());
        check_shape(text.as_bytes(), &out);
        assert_eq!(kinds(&m, &out), ["masked_key"], "{text}");
        let at = text.find(mask).unwrap_or(0);
        assert!(starred(&out, at, mask), "{text}");
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
        format!("{{\"c\":\"-----BEGIN RSA PRIVATE KEY-----\\n{value}\\\"{value}\\n-----END RSA PRIVATE KEY-----\"}}"),
    ];
    let fail_closed = m.mask_rules().len() - 1;
    for input in &inputs {
        let b = input.as_bytes();
        let out = m.mask_whole(b);
        check_shape(b, &out);
        assert!(!out.spans.is_empty(), "{input}");
        assert!(std::str::from_utf8(&out.bytes).is_ok(), "{input}");
        for s in out.spans.iter().filter(|s| s.rule != fail_closed) {
            let before = |at: usize| b[..at].iter().rev().take_while(|&&c| c == b'\\').count();
            for at in s.start..s.start + s.len {
                assert!(
                    !(b[at] == b'"' && before(at) % 2 == 0),
                    "{input}: a span holds a quote"
                );
            }
            assert_eq!(before(s.start) % 2, 0, "{input}: a span starts inside an escape");
            let end = s.start + s.len;
            assert!(end == b.len() || before(end) % 2 == 0, "{input}: a span ends on an escape");
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
