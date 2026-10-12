//! Redaction at capture: the shared pattern set, the connector's own patterns, and the inline
//! audit the altered envelope carries beside the redacted bytes.

use goalcup_daemon::redact::Redactor;

#[test]
fn a_key_is_removed_with_kind_only_marker_and_original_offsets() {
    let mut r = Redactor::new();
    let line = br#"{"authorization":"Bearer sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFF"}"#;
    let out = r.apply(line);
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    assert!(
        !text.contains("sk-ant-api03"),
        "the key does not reach the chain"
    );
    assert!(
        text.contains("<redacted:anthropic_key>"),
        "the v2 replacement carries the public kind and no secret-derived digest"
    );
    assert_eq!(out.spans.len(), 1);
    let span = &out.spans[0];
    assert_eq!(
        &line[span.start as usize..span.end as usize],
        b"sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFF",
        "offsets index the PRE-redaction bytes, as redact.py records them"
    );

    let audit = serde_json::to_string(&out.spans).unwrap();
    assert!(!audit.contains("salt"));
    assert!(!audit.contains("digest"));
    assert!(!text.contains("AAAABBBB"));
}

#[test]
fn the_same_kind_has_the_same_public_marker_without_an_oracle() {
    let mut r = Redactor::new();
    let line = br#"{"authorization":"Bearer sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFF"}"#;
    let first = r.apply(line);
    let second = r.apply(line);
    assert_eq!(first.bytes, second.bytes);
    assert_eq!(first.spans, second.spans);
    assert_eq!(
        String::from_utf8(first.bytes).unwrap(),
        r#"{"authorization":"Bearer <redacted:anthropic_key>"}"#
    );
}

#[test]
fn the_widened_set_and_the_connectors_own_patterns_both_fire() {
    let mut base = Redactor::new();
    for (probe, kind) in [
        ("AIzaSyA1234567890abcdefghijklmnopqrstu", "google_key"),
        ("hf_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345", "huggingface_token"),
        ("glpat-ABCDEFGHIJKLMNOPQRST", "gitlab_token"),
        // A base64 refresh token, in the shape the daemon actually sees one: a quoted
        // JSON key. The script's `key = value` form cannot reach that at all, and
        // widening that form's value class instead would reach into
        // ordinary dotted code without ever catching this line.
        (
            r#""refresh_token": "aGVsbG8rd29ybGQvdGhpcz0=""#,
            "generic_secret_assignment",
        ),
    ] {
        let out = base.apply(probe.as_bytes());
        assert_eq!(out.spans.len(), 1, "{probe} was not caught");
        assert_eq!(out.spans[0].kind, kind);
    }

    let m = connector("codex-cli");
    let mut r = Redactor::for_patterns("codex-cli", &m).unwrap();
    assert!(
        r.kinds().len() > goalcup_daemon::redact::patterns().len(),
        "the connector's extra_patterns[] are appended, not ignored"
    );
    let out = r.apply(br#"{"plan_type":"pro","credits_remaining":412}"#);

    // Asserted against the document, not against an encoded wrapper around it.
    let doc: serde_json::Value = serde_json::from_slice(&out.bytes)
        .expect("a redacted record is still the JSON every reader downstream parses");
    for (field, published) in [("plan_type", "pro"), ("credits_remaining", "412")] {
        let value = doc
            .get(field)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{field} was replaced by a marker: {doc}"));
        assert_ne!(
            value, published,
            "D26 strips billing before publication; R10 strips it before the upload"
        );
        assert!(
            value.starts_with("<redacted:") && value.ends_with('>'),
            "the brief names credits_remaining beside plan_type, and harness.toml must \
             too: {field} is {value}"
        );
    }
    assert_eq!(
        out.spans.len(),
        2,
        "one span per field, at the offsets in the bytes this stage was handed"
    );
}

// ---------------------------------------------------------------------------------------
// Beyond the three shapes above.
// ---------------------------------------------------------------------------------------

/// Acceptance line 1: the kinds of the shared set, in order. The script's ten, `masked_key`
/// (listed after `openai_key` here, last in the script), the JSON form of
/// `generic_secret_assignment` ahead of the script's own form, then seven vendor prefixes.
/// The script's rules themselves are compared in `spikes_redact_py_patterns_match_the_shared_set`.
#[test]
fn the_shared_set_kinds_are_in_the_documented_order() {
    let kinds: Vec<&str> = goalcup_daemon::redact::patterns()
        .iter()
        .map(|p| p.kind.as_str())
        .collect();
    assert_eq!(
        kinds,
        vec![
            // spikes/redact.py's original ten, in its order, with `masked_key` after
            // `openai_key` (the script lists it last; it is soft, so the place is only a place).
            "anthropic_key",
            "openai_key",
            // A masked provider key (`sk-ab***wxyz`).
            "masked_key",
            "github_token",
            "aws_access_key",
            "bearer",
            "jwt",
            "private_key",
            "slack_token",
            // One kind, two literals: the JSON form the daemon's only input shape uses,
            // then the script's own shell/TOML `key = value` form.
            "generic_secret_assignment",
            "generic_secret_assignment",
            "email",
            // then the seven fixed vendor prefixes.
            "google_key",
            "huggingface_token",
            "npm_token",
            "xai_key",
            "stripe_key",
            "gitlab_token",
            "digitalocean_token",
        ]
    );
    assert_eq!(goalcup_daemon::redact::patterns().len(), 19);
}

/// Every one of the script's ten still fires, on the shapes it was written for. The spike
/// fixtures were redacted with that script and the verifier's parsers read those bytes.
#[test]
fn every_pattern_the_script_carries_still_fires() {
    let mut r = Redactor::new();
    for (probe, kind) in [
        ("sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFF", "anthropic_key"),
        ("sk-proj-AAAABBBBCCCCDDDDEEEEFFFF", "openai_key"),
        ("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123", "github_token"),
        ("AKIAIOSFODNN7EXAMPLE", "aws_access_key"),
        ("Bearer abcdefghijklmnopqrst", "bearer"),
        (
            "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4",
            "jwt",
        ),
        (
            "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----",
            "private_key",
        ),
        ("xoxb-1234567890-ABCDEFGHIJ", "slack_token"),
        (
            "api_key = \"ABCDEFGHIJKLMNOP\"",
            "generic_secret_assignment",
        ),
        ("someone@example.com", "email"),
        // and the seven vendor prefixes.
        ("AIzaSyA1234567890abcdefghijklmnopqrstu", "google_key"),
        ("hf_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345", "huggingface_token"),
        ("npm_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345", "npm_token"),
        ("xai-ABCDEFGHIJKLMNOPQRSTUVWXYZ012345", "xai_key"),
        ("sk_live_ABCDEFGHIJKLMNOPQRSTUVWX", "stripe_key"),
        ("rk_live_ABCDEFGHIJKLMNOPQRSTUVWX", "stripe_key"),
        ("glpat-ABCDEFGHIJKLMNOPQRST", "gitlab_token"),
        (
            "dop_v1_abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "digitalocean_token",
        ),
    ] {
        let out = r.apply(probe.as_bytes());
        assert!(!out.spans.is_empty(), "{probe} was not caught at all");
        assert_eq!(
            out.spans[0].kind, kind,
            "{probe} was caught by the wrong rule"
        );
        assert!(
            !String::from_utf8_lossy(&out.bytes).contains(probe),
            "{probe} survived into the redacted bytes"
        );
    }
}

/// A match overlapping an earlier claimed span is
/// skipped. `bearer` and `openai_key` both cover this line; `anthropic_key` claimed it
/// first, so exactly one span is logged.
#[test]
fn a_match_overlapping_an_earlier_claimed_span_is_skipped() {
    let mut r = Redactor::new();
    let out = r.apply(br#"Authorization: Bearer sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFF"#);
    assert_eq!(out.spans.len(), 1);
    assert_eq!(out.spans[0].kind, "anthropic_key");
}

/// A record with nothing to remove passes through byte-identical and earns no audit.
#[test]
fn a_clean_record_passes_through_byte_identical_and_logs_nothing() {
    let mut r = Redactor::new();
    let line = br#"{"type":"user","cwd":"/w","text":"run the tests please"}"#;
    let out = r.apply(line);
    assert_eq!(out.bytes, line.to_vec(), "no match, no rewrite");
    assert!(out.spans.is_empty());
}

/// The offsets index the pre-redaction bytes even when several spans are removed, and the
/// spans come back in the order they sit in those bytes.
#[test]
fn several_spans_index_the_original_bytes_in_positional_order() {
    let mut r = Redactor::new();
    let line =
        b"to someone@example.com with AKIAIOSFODNN7EXAMPLE and ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123";
    let out = r.apply(line);
    assert_eq!(out.spans.len(), 3);
    assert!(out.spans.windows(2).all(|w| w[0].end <= w[1].start));
    assert_eq!(
        &line[out.spans[0].start as usize..out.spans[0].end as usize],
        b"someone@example.com"
    );
    assert_eq!(
        &line[out.spans[1].start as usize..out.spans[1].end as usize],
        b"AKIAIOSFODNN7EXAMPLE"
    );
    assert_eq!(
        &line[out.spans[2].start as usize..out.spans[2].end as usize],
        b"ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123"
    );
}

#[test]
fn reverse_ordered_high_match_input_keeps_first_pattern_wins_and_source_order() {
    let mut r = Redactor::new();
    let mut line = Vec::new();
    for i in 0..2_000 {
        line.extend_from_slice(format!("person{i}@example.test ").as_bytes());
    }
    for i in 0..2_000 {
        line.extend_from_slice(format!("sk-ant-api03-{i:020} ").as_bytes());
    }

    let out = r.apply(&line);
    assert_eq!(out.spans.len(), 4_000);
    assert!(
        out.spans
            .windows(2)
            .all(|pair| pair[0].end <= pair[1].start)
    );
    assert!(
        out.spans[..2_000].iter().all(|span| span.kind == "email")
            && out.spans[2_000..]
                .iter()
                .all(|span| span.kind == "anthropic_key"),
        "the priority pass cannot reorder the source-position audit"
    );
}

/// Acceptance line 4, the other half: `for_patterns` catches Claude's billing header
/// block, exactly as it catches Codex's `plan_type`.
#[test]
fn for_patterns_catches_claudes_billing_header_block() {
    let m = connector("claude-code-cli");
    let mut r = Redactor::for_patterns("claude-code-cli", &m).unwrap();
    let out = r.apply(b"x-anthropic-billing-tier: enterprise-2026\n");
    assert_eq!(out.spans.len(), 1);
    assert_eq!(out.spans[0].kind, "anthropic_billing_header");
    assert!(!String::from_utf8_lossy(&out.bytes).contains("enterprise-2026"));
}

/// Both legs go through the same `Redactor`: there is one `apply`, it takes bytes, and it
/// does not know or care which leg produced them. A leg-2 body redacts the same way.
#[test]
fn a_leg_two_body_redacts_through_the_same_apply() {
    let mut r = Redactor::new();
    let leg2 = br#"{"event":"UserPromptSubmit","prompt":"my key is sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFF"}"#;
    let out = r.apply(leg2);
    assert_eq!(out.spans.len(), 1);
    assert!(!String::from_utf8_lossy(&out.bytes).contains("sk-ant-api03"));
}

/// A record whose bytes are not valid UTF-8 is still redacted, and the bytes around the
/// removed span survive untouched. Redaction runs on bytes, not on a lossy transcription.
#[test]
fn invalid_utf8_around_a_secret_is_preserved_and_the_secret_is_still_removed() {
    let mut r = Redactor::new();
    let mut line: Vec<u8> = b"prefix ".to_vec();
    line.push(0xff);
    line.extend_from_slice(b" sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFF ");
    line.push(0xfe);
    let out = r.apply(&line);
    assert_eq!(out.spans.len(), 1);
    assert!(out.bytes.contains(&0xff), "the byte before survives");
    assert!(out.bytes.contains(&0xfe), "the byte after survives");
    assert!(!out.bytes.windows(12).any(|w| w == b"sk-ant-api03"));
}

/// A connector whose `extra_patterns[]` will not compile is refused at build time, not
/// silently dropped: a pattern the participant can read in `harness.toml` but that never
/// fires is a privacy hole with a paper trail saying otherwise.
#[test]
fn an_uncompilable_extra_pattern_is_an_error_not_a_silent_drop() {
    let mut m = connector("codex-cli");
    m.push(goalcup_protocol::manifest::RedactionPattern {
        kind: "broken".to_string(),
        regex: "([unclosed".to_string(),
    });
    let err = Redactor::for_patterns("codex-cli", &m).unwrap_err();
    assert!(format!("{err}").contains("broken"), "the kind names itself");
}

/// The marker carries only the public pattern kind. The original byte offsets are carried
/// in the hash-covered inline audit rather than encoded into the payload.
#[test]
fn the_marker_carries_only_the_public_kind() {
    let mut r = Redactor::new();
    let out = r.apply(b"AKIAIOSFODNN7EXAMPLE");
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    assert_eq!(text, "<redacted:aws_access_key>");
}

// ---------------------------------------------------------------------------------------
// The daemon's only input shape is JSON.
// ---------------------------------------------------------------------------------------

/// The script's `generic_secret_assignment` requires `[=:]` immediately after the key
/// word, so a quoted JSON key never fires it — `{"api_key":"…"}` would produce zero spans, and
/// JSON is the only shape a harness store or a leg-2 body ever hands this stage. The JSON
/// literal covers it; it is bounded by the value's own closing quote, so it cannot reach
/// past the field the way a widened value class would.
///
/// The span is the value, not the field. The literal declares a capture group, and a
/// pattern that declares one has that group for its span. A whole-match span takes the
/// key, the colon and both quotes with it — `{<redacted:…>}` — and JSON is not only this
/// stage's input shape but its output shape: the chain and the verifier's parsers both
/// read redacted bytes as JSON. The key, the colon and the quotes stay; the bytes between
/// the quotes become the marker.
#[test]
fn a_quoted_json_key_is_the_shape_the_daemon_actually_sees() {
    for (probe, key, gone) in [
        (
            r#"{"api_key":"AAAABBBBCCCCDDDDEEEE"}"#,
            "api_key",
            "AAAABBBBCCCCDDDDEEEE",
        ),
        (
            r#"{"api_key": "AAAABBBBCCCCDDDDEEEE"}"#,
            "api_key",
            "AAAABBBBCCCCDDDDEEEE",
        ),
        (
            r#"{"refresh_token": "aGVsbG8rd29ybGQvdGhpcz0="}"#,
            "refresh_token",
            "aGVsbG8rd29ybGQvdGhpcz0=",
        ),
        (
            r#"{"password":"hunter2hunter2hunter2"}"#,
            "password",
            "hunter2hunter2hunter2",
        ),
    ] {
        let mut r = Redactor::new();
        let out = r.apply(probe.as_bytes());
        assert_eq!(out.spans.len(), 1, "{probe} yielded the wrong span count");
        assert_eq!(out.spans[0].kind, "generic_secret_assignment", "{probe}");
        assert!(
            !String::from_utf8_lossy(&out.bytes).contains(gone),
            "{probe} left its value in the chain"
        );

        // The offsets name the value alone.
        assert_eq!(
            &probe.as_bytes()[out.spans[0].start as usize..out.spans[0].end as usize],
            gone.as_bytes(),
            "{probe}: the span must cover the value and nothing around it"
        );

        // And what the chain hashes is still a JSON record, carrying the key it always
        // carried, whose value is the marker the log can be lined up against.
        let v: serde_json::Value = serde_json::from_slice(&out.bytes)
            .unwrap_or_else(|e| panic!("{probe} left the record unparseable: {e}"));
        assert_eq!(
            v[key],
            serde_json::Value::String("<redacted:generic_secret_assignment>".to_string()),
            "{probe}: the key survives and its value is the marker"
        );
    }
}

/// The shell/TOML literal keeps `redact.py`'s original narrow value class. Widening it to
/// `[A-Za-z0-9_\-+/=.]{16,}` to catch base64 would put every dotted expression after a
/// `token =` inside a span — a source line, not a secret, removed from the chain
/// irreversibly.
#[test]
fn the_narrow_value_class_leaves_ordinary_dotted_code_alone() {
    let mut r = Redactor::new();
    let out = r.apply(b"let token = entry.keys.receiver_token.clone();");
    assert_eq!(
        out.spans.len(),
        0,
        "a dotted code expression is not a secret assignment"
    );

    let mut r = Redactor::new();
    let line = br#"api_key = "AAAABBBBCCCCDDDDEEEE""#;
    let out = r.apply(line);
    assert_eq!(out.spans.len(), 1, "the script's own shape still fires");
    assert_eq!(out.spans[0].kind, "generic_secret_assignment");
    assert!(!String::from_utf8_lossy(&out.bytes).contains("AAAABBBBCCCCDDDDEEEE"));
    // Deliberately not the JSON form's value-only span. This literal declares no capture
    // group, so its span is the whole match — which is what `redact.py` replaces
    // (`m.group(0)`), and the committed spike corpus carries the markers that produced. The
    // shape is a shell export or a TOML line, not a JSON field, so nothing downstream
    // parses it and there is no structure to keep intact.
    assert_eq!(
        &line[out.spans[0].start as usize..out.spans[0].end as usize],
        br#"api_key = "AAAABBBBCCCCDDDDEEEE"#,
        "the shell/TOML form mirrors the script it was copied from"
    );
}

/// Over-redaction, measured rather than argued. The shared set runs over every committed
/// spike fixture and must find nothing the corpus does not already carry as a
/// `<redacted:…>` marker — except the catches named below, each of which has to be hit.
///
/// Read-only: this test opens `spikes/` and writes nothing.
#[test]
fn the_shared_set_finds_nothing_new_in_the_committed_m0_fixtures() {
    /// `(path suffix, exact matched text)`. Every entry is a real credential-shaped field
    /// that `spikes/redact.py` could not reach because its key is a quoted JSON key —
    /// the bug the JSON literal exists to fix — and is what `opencode`'s own
    /// `opencode_provider_apikey` pattern was written for. The list is not a blanket
    /// exemption: each entry must fire, or the test fails.
    ///
    /// The text is the value and not the whole field, because the JSON literal's span is
    /// its capture group.
    const KNOWN_NEW_CATCHES: &[(&str, &str)] =
        &[("opencode/fixtures/hooks.jsonl", "opencode-oauth-dummy-key")];

    let spikes = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spikes");
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for spike in std::fs::read_dir(&spikes).unwrap() {
        let fixtures = spike.unwrap().path().join("fixtures");
        let Ok(entries) = std::fs::read_dir(&fixtures) else {
            continue; // a spike with no fixtures/ directory
        };
        for e in entries {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "jsonl") {
                files.push(p); // skip README.md, *.json, *.sh, *.diff
            }
        }
    }
    files.sort();
    assert!(
        files.len() > 50,
        "the corpus went missing: only {} fixtures found under {}",
        files.len(),
        spikes.display()
    );

    let mut hits = vec![0usize; KNOWN_NEW_CATCHES.len()];
    let mut unexplained: Vec<String> = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        // Where the corpus already carries a marker `spikes/redact.py` left behind.
        let mut markers: Vec<(usize, usize)> = Vec::new();
        let mut i = 0;
        while let Some(off) = bytes[i..]
            .windows(10)
            .position(|w| w == b"<redacted:")
            .map(|o| o + i)
        {
            let end = bytes[off..]
                .iter()
                .position(|b| *b == b'>')
                .map(|o| off + o + 1)
                .unwrap_or(bytes.len());
            markers.push((off, end));
            i = end;
        }

        let mut r = Redactor::new();
        let out = r.apply(&bytes);
        for span in &out.spans {
            let start = span.start as usize;
            let end = span.end as usize;
            if markers.iter().any(|(s, e)| start < *e && *s < end) {
                continue; // the shared set re-matching a marker the script already wrote
            }
            let text = String::from_utf8_lossy(&bytes[start..end]).into_owned();
            let name = path.to_string_lossy().replace('\\', "/");
            match KNOWN_NEW_CATCHES
                .iter()
                .position(|(suffix, want)| name.ends_with(suffix) && *want == text)
            {
                Some(k) => {
                    hits[k] += 1;
                    // The record this catch sits in is a real captured JSONL line, and it
                    // has to survive redaction as JSON — the property a whole-field span
                    // broke. Redact that one line on its own and parse the result.
                    let ls = bytes[..start]
                        .iter()
                        .rposition(|b| *b == b'\n')
                        .map_or(0, |i| i + 1);
                    let le = bytes[start..]
                        .iter()
                        .position(|b| *b == b'\n')
                        .map_or(bytes.len(), |i| start + i);
                    let line = &bytes[ls..le];
                    serde_json::from_slice::<serde_json::Value>(line)
                        .unwrap_or_else(|e| panic!("{name}: the fixture line is not JSON: {e}"));
                    let mut one = Redactor::new();
                    let red = one.apply(line);
                    serde_json::from_slice::<serde_json::Value>(&red.bytes).unwrap_or_else(|e| {
                        panic!("{name}: redaction left the record unparseable: {e}")
                    });
                }
                None => unexplained.push(format!("{name}: {} {text:?}", span.kind)),
            }
        }
    }

    assert!(
        unexplained.is_empty(),
        "the shared set over-redacts the real corpus:\n{}",
        unexplained.join("\n")
    );
    for (k, (suffix, want)) in KNOWN_NEW_CATCHES.iter().enumerate() {
        assert_eq!(
            hits[k], 1,
            "the known catch {suffix} {want:?} no longer fires — remove it from the list"
        );
    }
}

/// `(?-u)` turns Unicode mode off, and a `regex::bytes` pattern in that mode may match
/// bytes that are not valid UTF-8. That would widen a text-pattern contract to arbitrary
/// binary bytes and could cut a span boundary through the middle of a codepoint. A
/// connector's `harness.toml` may not ask for that.
#[test]
fn an_extra_pattern_that_turns_unicode_mode_off_is_refused() {
    let base = connector("codex-cli");
    for regex in [r"(?-u)\xff+", r"(?i-u)secret", r"(?-ui:secret)"] {
        let mut m = base.clone();
        m.push(goalcup_protocol::manifest::RedactionPattern {
            kind: "byte_mode".to_string(),
            regex: regex.to_string(),
        });
        let err = Redactor::for_patterns("codex-cli", &m).unwrap_err();
        let text = format!("{err}");
        assert!(text.contains("byte_mode"), "{regex}: {text}");
        assert!(text.contains("Unicode"), "{regex}: {text}");
    }
}

/// The codex `harness.toml` strips the credit balance as well as `plan_type` and `rate_limits`:
/// it is the one number the spike measured fifteen times in ten minutes.
#[test]
fn the_codex_harness_strips_the_credit_balance_it_names() {
    let m = connector("codex-cli");
    assert!(
        m.iter().any(|p| p.kind == "codex_credits_remaining"),
        "harness.toml declares the rule a participant can read"
    );
    let mut r = Redactor::for_patterns("codex-cli", &m).unwrap();
    let out = r.apply(br#"{"credits_remaining": 17.5, "plan_type": "pro"}"#);
    let kinds: Vec<&str> = out.spans.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(kinds, vec!["codex_credits_remaining", "codex_plan_type"]);
    assert!(!String::from_utf8_lossy(&out.bytes).contains("17.5"));
}

// ---------------------------------------------------------------------------------------
// The connectors' own patterns, and the record they leave behind.
// ---------------------------------------------------------------------------------------

/// One shipped connector's `[redaction].extra_patterns`, read from its `harness.toml`.
fn connector(id: &str) -> Vec<goalcup_protocol::manifest::RedactionPattern> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id)
        .join("harness.toml");
    let harness = goalcup_protocol::harness::HarnessManifest::from_toml_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .expect("harness.toml parses");
    goalcup_daemon::redact::extra_patterns_of(&harness)
}

/// A connector's own `extra_patterns[]` must leave the record it fires on parseable, the
/// same rule the shared JSON literal follows.
///
/// A pattern that matched a whole JSON field — key, colon, quotes and
/// value — would turn `{"plan_type":"pro"}` into a bare marker in place of the field. That is
/// not a cosmetic loss. Redacted bytes are what the chain carries and what the verifier's
/// parsers read, so a record a connector rule fired on would be a record no reader could parse.
///
/// Each case is shaped like the field the spike actually measured; the value is asserted
/// gone, the key is asserted still there, and `serde_json` is asserted to accept what
/// comes out.
#[test]
fn a_connector_pattern_leaves_the_record_valid_json_with_its_key_intact() {
    struct Case {
        connector: &'static str,
        /// The kind that must claim the span. Not always the connector's rule: the shared
        /// set runs first, and a span it claims makes the connector's overlapping match a
        /// skip.
        kind: &'static str,
        record: &'static str,
        /// A JSON pointer that must still resolve after redaction.
        key: &'static str,
        /// Text that must not survive.
        gone: &'static [&'static str],
    }

    for c in [
        // Codex: `rate_limits` rides in every `token_count` event, and the object
        // is nested — `primary` is an object of its own. Anything that stops at the first
        // `}` cuts the record in half.
        Case {
            connector: "codex-cli",
            kind: "codex_rate_limits",
            record: r#"{"type":"token_count","payload":{"info":{"total_token_usage":{"input_tokens":412213}},"rate_limits":{"primary":{"used_percent":12.5,"resets_in_seconds":1800},"plan_type":"pro","credits_remaining":17.5}}}"#,
            key: "/payload/rate_limits",
            gone: &["12.5", "1800", "pro", "17.5"],
        },
        // The same two fields on their own.
        Case {
            connector: "codex-cli",
            kind: "codex_plan_type",
            record: r#"{"plan_type":"pro","credits_remaining":17.5}"#,
            key: "/plan_type",
            gone: &["pro"],
        },
        // The credit balance as a bare JSON number: there are no quotes around it in the
        // record, so the marker has to bring its own or the field stops being a value.
        Case {
            connector: "codex-cli",
            kind: "codex_credits_remaining",
            record: r#"{"plan_type":"pro","credits_remaining":17.5}"#,
            key: "/credits_remaining",
            gone: &["17.5"],
        },
        // OpenCode's `hooks.jsonl` shape, a 24-byte key in a provider block: the
        // shared literal claims it whatever the connector, so it is asserted on one still
        // shipped. The record must survive.
        Case {
            connector: "codex-cli",
            kind: "generic_secret_assignment",
            record: r#"{"provider":{"openai":{"options":{"apiKey":"opencode-oauth-dummy-key","headerTimeout":300000}}}}"#,
            key: "/provider/openai/options/apiKey",
            gone: &["opencode-oauth-dummy-key"],
        },
        // Claude's `rawbody-goal-turn.request.json`: the billing header is a line
        // inside a system block's `text`, not a JSON field of its own. Its match is bounded
        // by the string's own closing quote, so the marker lands inside the quotes that are
        // already there.
        Case {
            connector: "claude-code-cli",
            kind: "anthropic_billing_header",
            record: r#"{"system":[{"type":"text","text":"x-anthropic-billing-header: cc_version=2.1.259.2b1; cc_entrypoint=cli; cc_prompt_id=58740b59-3dfe-46fb-8704-7d049d21a7ed;"}]}"#,
            key: "/system/0/text",
            gone: &["cc_version", "58740b59"],
        },
    ] {
        let m = connector(c.connector);
        let mut r = Redactor::for_patterns(c.connector, &m).unwrap();
        let out = r.apply(c.record.as_bytes());

        let kinds: Vec<&str> = out.spans.iter().map(|s| s.kind.as_str()).collect();
        assert!(
            kinds.contains(&c.kind),
            "{}: {} did not fire on {}, spans {kinds:?}",
            c.connector,
            c.kind,
            c.record
        );

        let text = String::from_utf8_lossy(&out.bytes).to_string();
        for g in c.gone {
            assert!(
                !text.contains(g),
                "{}: `{g}` survived {}: {text}",
                c.connector,
                c.kind
            );
        }

        let v: serde_json::Value = serde_json::from_slice(&out.bytes).unwrap_or_else(|e| {
            panic!(
                "{}: {} left bytes that are no longer JSON ({e}): {text}",
                c.connector, c.kind
            )
        });
        assert!(
            v.pointer(c.key).is_some(),
            "{}: {} took `{}` with it: {text}",
            c.connector,
            c.kind,
            c.key
        );
    }
}

/// V11's token totals come out of a Codex `token_count` event, and `rate_limits` is a
/// sibling field in the very same record. A rule that unparses the record to strip the
/// operator's plan loses the totals with it — which is why the object's replacement has to
/// be a JSON value and not a bare marker.
#[test]
fn stripping_rate_limits_leaves_the_token_count_record_readable() {
    let m = connector("codex-cli");
    let mut r = Redactor::for_patterns("codex-cli", &m).unwrap();
    let record = br#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":412213,"output_tokens":1737}},"rate_limits":{"primary":{"used_percent":12.5,"resets_in_seconds":1800}}}}"#;
    let out = r.apply(record);
    let v: serde_json::Value = serde_json::from_slice(&out.bytes)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.bytes)));
    assert_eq!(
        v.pointer("/payload/info/last_token_usage/input_tokens")
            .and_then(|x| x.as_u64()),
        Some(412213),
        "the totals V11 reports are in the record redaction just rewrote"
    );
    assert!(
        v.pointer("/payload/rate_limits").and_then(|x| x.as_str())
            == Some("<redacted:codex_rate_limits>"),
        "the field stays, carrying the marker as its value"
    );
}

// ── Hermes lane A, S9: masked provider keys ───────────────────────────────────────────
//
// A provider that rejects a key echoes it masked: `key[:8]…key[-4:]` in a request dump,
// `sk-R7aMV***…***oKYo` in an OpenAI 401 body. None of the full-key rules sees it (it is
// shorter than any of their value classes), so a masked key reached the chain readable (D41).
// Every key below is synthetic.
//
// The rule is `(?i)sk-[A-Za-z0-9-]{1,8}(?:\.{3}|…|\\u2026|\*{3,64})[A-Za-z0-9]{3,4}`:
// a run of one to eight letters, digits and hyphens (Hermes masks `key[:4]`, `key[:6]` and
// `key[:8]`, and an OpenRouter or Anthropic prefix has hyphens in it), a separator, and a
// three or four character tail, with no boundary at either edge. It is *soft*: it never
// shadows another rule (see `select`), so adding it can only add redaction, and an anchor
// would only cost coverage.

fn spans_of(line: &[u8]) -> (String, Vec<(String, String)>) {
    let out = Redactor::new().apply(line);
    let spans = out
        .spans
        .iter()
        .map(|s| {
            (
                s.kind.clone(),
                String::from_utf8_lossy(&line[s.start as usize..s.end as usize]).into_owned(),
            )
        })
        .collect();
    (String::from_utf8(out.bytes).unwrap(), spans)
}

/// The bytes of `line` a redaction replaced.
fn covered(line: &[u8], spans: &[goalcup_protocol::redaction::RedactionAuditSpan]) -> Vec<bool> {
    let mut covered = vec![false; line.len()];
    for s in spans {
        for flag in &mut covered[s.start as usize..s.end as usize] {
            *flag = true;
        }
    }
    covered
}

/// With the REAL OpenAI 401 echo: eight characters of key, about thirty-nine stars
/// and four characters of tail.
#[test]
fn a_masked_key_in_a_401_body_is_redacted() {
    let (text, spans) = spans_of(
        br#"{"error":{"message":"Incorrect API key provided: sk-R7aMV***************************************oKYo."}}"#,
    );
    assert_eq!(
        text,
        r#"{"error":{"message":"Incorrect API key provided: <redacted:masked_key>."}}"#
    );
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].0, "masked_key");
    let (text, spans) = spans_of(b"Incorrect API key provided: sk-ab***wxyz");
    assert_eq!(text, "Incorrect API key provided: <redacted:masked_key>");
    assert_eq!(spans[0].1, "sk-ab***wxyz");
}

/// A JSON `Authorization` header value of 15 bytes, one below `bearer`'s 16, so
/// `bearer` cannot fire and the masked rule must.
#[test]
fn a_masked_bearer_value_below_the_bearer_threshold_is_redacted() {
    let (text, spans) = spans_of(br#"{"Authorization": "Bearer sk-abcde...wxyz"}"#);
    assert_eq!(text, r#"{"Authorization": "Bearer <redacted:masked_key>"}"#);
    assert_eq!(
        spans,
        [("masked_key".to_string(), "sk-abcde...wxyz".to_string())]
    );
}

/// The masks real tools emit, each redacted whole.
#[test]
fn the_masks_real_tools_emit_are_redacted() {
    for (line, want) in [
        // OpenAI's 401 echo, and its project-key form with a three character tail.
        (
            "Incorrect API key provided: sk-R7aMV***************************************oKYo.",
            "Incorrect API key provided: <redacted:masked_key>.",
        ),
        (
            "Incorrect API key provided: 'sk-proj**********************************************Ldy'.",
            "Incorrect API key provided: '<redacted:masked_key>'.",
        ),
        // Hermes `mask_secret`: key[:4] + ... + key[-4:], a one character run after `sk-`.
        ("sk-0...cdef", "<redacted:masked_key>"),
        // Hermes `_mask_token` (6/4) and `_mask_api_key_for_logs` (8/4).
        ("sk-012...cdef", "<redacted:masked_key>"),
        ("Bearer sk-0123a...cdef", "Bearer <redacted:masked_key>"),
        // key[:8] of an OpenRouter and an Anthropic key: the run has hyphens in it.
        ("Bearer sk-or-v1...cdef", "Bearer <redacted:masked_key>"),
        ("Bearer sk-ant-a...cdef", "Bearer <redacted:masked_key>"),
    ] {
        let (text, _) = spans_of(line.as_bytes());
        assert_eq!(text, want, "{line}");
    }
    // DeepSeek's own 401 names only the last four characters and no `sk-`. That is not a
    // fragment of the key under D41 (a bare four-character suffix is not), and it is not
    // this rule's to catch.
    let line = "Authentication Fails, Your api key: ****cdef is invalid";
    assert_eq!(spans_of(line.as_bytes()).0, line);
}

/// Every separator and width the rule names, each caught whole; the text around it intact.
#[test]
fn every_masked_form_is_redacted_whole() {
    let long_stars = format!("sk-R7aMV{}oKYo", "*".repeat(64));
    for key in [
        "sk-ab***wxyz",
        "sk-a***wxyz",
        "sk-abcdefgh***wxyz",
        "sk-ab********wxyz",
        "sk-abc...wxyz",
        "sk-abcdefgh...wxyz",
        "sk-abcde\\u2026wxyz",
        "sk-abcde\\U2026wxyz",
        "sk-abcde\u{2026}wxyz",
        "sk-ab***wxy",
        "sk-ab...xyz",
        "sk-ab-cd-ef***wxyz",
        "SK-ABCDE...WXYZ",
        "sK-abcde***wxyz",
        "s\u{212a}-abcde***wxyz",
        "sk-abcde***\u{17f}\u{212a}yz",
        "sk-ab***0123",
        long_stars.as_str(),
    ] {
        let line = format!("before {key} after");
        let (text, spans) = spans_of(line.as_bytes());
        assert_eq!(text, "before <redacted:masked_key> after", "{key}");
        assert_eq!(
            spans,
            [("masked_key".to_string(), key.to_string())],
            "{key}"
        );
    }
}

/// What the rule does not claim. Each is a plausible ordinary string.
#[test]
fn near_misses_of_a_masked_key_are_left_alone() {
    let too_many_stars = format!("sk-ab{}wxyz", "*".repeat(65));
    for text in [
        "sk-abcdefghi***wxyz", // nine characters before the mask
        "sk-ab**wxyz",         // two stars
        too_many_stars.as_str(),
        "sk-ab..wxyz",   // two dots
        "sk-ab....wxyz", // four dots
        "sk-ab***wx",    // a two-character tail
        "sk-ab***wx_z",  // a non-alphanumeric in the tail
        "sk-ab***",      // no tail
        "sk-***wxyz",    // nothing before the mask
        "sk_ab***wxyz",  // not `sk-`
        "ask ab***wxyz",
        "sk-ab\\u202wxyz",
    ] {
        let (out, spans) = spans_of(text.as_bytes());
        assert!(
            !spans.iter().any(|(kind, _)| kind == "masked_key"),
            "{text:?} was claimed: {spans:?}"
        );
        assert_eq!(out, text, "{text:?}");
    }
    // There is no left boundary: a mask is a mask after a newline, a tab, a `_`, a letter
    // (JSON's own `\n` is a backslash and an `n`), an ANSI colour code or anything else.
    for text in [
        "-sk-ab***wxyz",
        "\"sk-ab***wxyz\"",
        "(sk-ab***wxyz)",
        "dump:\nsk-abcde...wxyz",
        "\tsk-abcde...wxyz",
        "KEY_sk-abcde...wxyz",
        "\u{1b}[33msk-abcde...wxyz",
        r#"{"content":"dump:\nsk-abcde...wxyz"}"#,
        "task-ab***wxyz",
        "ssk-ab***wxyz",
        "my_sk-ab***wxyz",
        "\u{e9}sk-ab***wxyz",
        "\u{300}sk-ab***wxyz",
        "\u{17f}k-ab***wxyz",
    ] {
        let (out, _) = spans_of(text.as_bytes());
        assert!(out.contains("<redacted:masked_key>"), "{text:?}: {out}");
        assert!(
            !out.contains("wxyz") && !out.contains("sk-ab"),
            "{text:?}: {out}"
        );
    }
    // The tail is not anchored: a mask followed by more word characters still loses the
    // four it names, and the rest stays.
    let (out, _) = spans_of(b"sk-ab***wxyz12");
    assert_eq!(out, "<redacted:masked_key>12");
}

/// A full key is still `openai_key`, alone, and the masked rule does not take over
/// a line that holds both.
#[test]
fn full_keys_are_still_classified_openai_key() {
    let full = format!("sk-{}", "0123456789abcdef".repeat(2));
    let (_, spans) = spans_of(full.as_bytes());
    assert_eq!(spans, [("openai_key".to_string(), full.clone())]);
    let (text, spans) = spans_of(format!("{full} then sk-ab***wxyz").as_bytes());
    assert_eq!(text, "<redacted:openai_key> then <redacted:masked_key>");
    assert_eq!(
        spans.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        ["openai_key", "masked_key"]
    );
}

/// These six inputs each hold a secret that a rule redacted before the
/// masked rule existed, glued to something mask-shaped. The masked rule used to win the
/// overlap and the longer secret leaked. It is soft now: it can only add redaction, and what
/// it adds on top of an overlap is the part of the mask nothing else covered.
#[test]
fn a_mask_shaped_substring_never_weakens_an_existing_redaction() {
    let cases: [(&str, &[&str]); 6] = [
        // The mask sits mid-word here, so it is no mask: what is left of it is what the
        // policy left before the rule existed. The longer secret must still go.
        ("api_key=SECRETSECRETSECRETsk-ab***wxyz", &["SECRET"]),
        ("password: \"SECRETSECRETSECRETask-ab***wxyz\"", &["SECRET"]),
        ("xoxb-SECRETSECRETsk-ab***wxyz", &["SECRET"]),
        (
            "Bearer sk-ab...wxyzSECRETTAILSECRETTAIL",
            &["SECRET", "wxyz"],
        ),
        (
            "Bearer abcdefghij.sk-ab...wxyz.SECRETTAILSECRETTAIL",
            &["SECRET", "wxyz"],
        ),
        (
            "privatelocal.part.sk-ab...wxyz@example.com",
            &["privatelocal", "wxyz", "example"],
        ),
    ];
    for (line, forbidden) in cases {
        let (text, _) = spans_of(line.as_bytes());
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "{line:?} -> {text:?} keeps {needle}"
            );
        }
        // And never less than the policy without the masked rule.
        let new = Redactor::new().apply(line.as_bytes());
        let base = Redactor::new().apply_without_kind(line.as_bytes(), "masked_key");
        let (new, base) = (
            covered(line.as_bytes(), &new.spans),
            covered(line.as_bytes(), &base.spans),
        );
        for (i, was) in base.iter().enumerate() {
            assert!(
                !was || new[i],
                "{line:?}: byte {i} was redacted without the rule"
            );
        }
    }
    // The reviewer's counter-example to "lowest priority": email claims `wxyz@ex.com`, and
    // the part of the mask it does not cover must still go.
    let (text, _) = spans_of(b"sk-abcde***wxyz@ex.com");
    assert!(!text.contains("sk-abcde"), "{text}");
}

struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

/// For any input, the policy with the masked rule redacts at least every byte the policy
/// without it did: glue masks to every other rule's shape, in every order.
#[test]
fn the_masked_rule_only_ever_adds_redaction() {
    let pieces: &[&str] = &[
        "sk-ab***wxyz",
        "sk-abcde...wxyz",
        "sk-0...cdef",
        "sk-or-v1...cdef",
        "sk-R7aMV*******************oKYo",
        "sk-ab\u{2026}wxyz",
        "sk-ab\\u2026wxyz",
        "api_key=",
        "token: \"",
        "\"api_key\":\"",
        "password=",
        "Bearer ",
        "bearer  ",
        "xoxb-",
        "ghp_",
        "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4",
        "AKIA",
        "sk-ant-",
        "sk-proj-",
        "SECRETSECRETSECRET",
        "abcdefghij",
        "user.name",
        "@example.com",
        "x@",
        ".",
        "-",
        "_",
        " ",
        "\n",
        "\"",
        "ab",
        "wxyz",
        "\u{e9}",
        "-----BEGIN RSA PRIVATE KEY-----\n",
        "-----END RSA PRIVATE KEY-----",
        "MIIB",
    ];
    let mut rng = Lcg(0xC0FFEE);
    let mut reaches = 0usize;
    for _ in 0..20_000 {
        let mut line = String::new();
        for _ in 0..1 + rng.below(8) {
            line.push_str(pieces[rng.below(pieces.len())]);
        }
        let bytes = line.as_bytes();
        let new = Redactor::new().apply(bytes);
        let base = Redactor::new().apply_without_kind(bytes, "masked_key");
        let (new_cover, base_cover) = (covered(bytes, &new.spans), covered(bytes, &base.spans));
        for (i, was) in base_cover.iter().enumerate() {
            assert!(
                !was || new_cover[i],
                "{line:?}: byte {i} was redacted without the masked rule and is not with it"
            );
        }
        reaches += usize::from(new.spans.iter().any(|s| s.kind == "masked_key"));
    }
    assert!(reaches > 2_000, "the generator reaches the rule: {reaches}");
}

/// The script's rule list carries the same masked rule, last, and redacts the same
/// shapes. The behavioural half is skipped, loudly, without `python3`.
///
/// The known, accepted differences between the two engines, in the script's favour (it
/// over-redacts), are `CLASS_DELTAS` and `output_deltas` below, pinned exactly: Python's
/// `re.IGNORECASE` also folds `İ` and `ı` (U+0130, U+0131) into `[A-Za-z]`, which Rust's
/// case folding does not, and Python's `\s` includes U+001C..U+001F. This test pins the
/// masked rule on its own, with the `masked_key` offsets. The script applies its rules one
/// after another, so after its pass
/// it also redacts whatever is still raw of a mask the daemon's `select` would have kept the
/// remainder of; `spikes_redact_py_is_never_weaker_than_the_daemon`
/// holds it to that.
#[test]
fn spikes_redact_py_carries_the_same_masked_rule() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = root.join("spikes/redact.py");
    let source = std::fs::read_to_string(&script).unwrap();
    let rule = r"(?i)sk-[A-Za-z0-9-]{1,8}(?:\.{3}|…|\\u2026|\*{3,64})[A-Za-z0-9]{3,4}";
    assert!(source.contains(rule), "redact.py does not carry the rule");
    let rust = goalcup_daemon::redact::patterns()
        .iter()
        .find(|p| p.kind == "masked_key")
        .expect("the shared set carries masked_key");
    assert_eq!(rust.re.as_str(), rule, "Rust and Python carry one rule");
    // The script applies its rules in sequence, so a soft rule is one listed after all the
    // others: it then only sees text no other rule replaced.
    let masked = source.find("\"masked_key\"").expect("a masked_key entry");
    for other in [
        "anthropic_key",
        "openai_key",
        "bearer",
        "generic_secret_assignment",
        "email",
    ] {
        assert!(
            source.find(&format!("\"{other}\"")).unwrap() < masked,
            "the script lists masked_key after {other}"
        );
    }

    // A deterministic ASCII corpus of mask-shaped and near-mask text, with nothing another
    // rule would match (so the script's sequential substitution leaves offsets alone).
    let pieces: &[&str] = &[
        "sk-ab***wxyz",
        "sk-a***wxy",
        "sk-abcde...wxyz",
        "sk-0...cdef",
        "sk-or-v1...cdef",
        "sk-R7aMV*****************oKYo",
        "sk-proj*****************Ldy",
        "sk-abcdefghi***wxyz",
        "sk-ab**wxyz",
        "sk-ab***wx",
        "task-ab***wxyz",
        "ssk-ab***wxyz",
        "my_sk-ab***wxyz",
        "-sk-ab***wxyz",
        "sk-ab***wxyz12",
        "sk-ab\\u2026wxyz",
        "sk-ab\\U2026wxyz",
        "sk-ab\\u202wxyz",
        "SK-AB***WXYZ",
        " ",
        "\n",
        "\"",
        ":",
        ".",
        "word",
        "and",
    ];
    let mut rng = Lcg(0xABCDEF);
    let mut corpus = String::new();
    for _ in 0..600 {
        for _ in 0..1 + rng.below(5) {
            corpus.push_str(pieces[rng.below(pieces.len())]);
        }
        corpus.push('\n');
    }
    let want: Vec<(u64, u64)> = rust
        .re
        .find_iter(corpus.as_bytes())
        .map(|m| (m.start() as u64, m.end() as u64))
        .collect();
    assert!(
        want.len() > 300,
        "the corpus reaches the rule: {}",
        want.len()
    );

    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.txt");
    let output = dir.path().join("out.txt");
    std::fs::write(&input, &corpus).unwrap();
    match std::process::Command::new("python3")
        .arg(&script)
        .arg(&input)
        .arg(&output)
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) => assert!(status.success()),
        Err(e) => {
            eprintln!("python3 is not available ({e}); the behavioural half is skipped");
            return;
        }
    }
    let log = std::fs::read_to_string(dir.path().join("out.txt.redactions.jsonl")).unwrap();
    let got: Vec<(u64, u64)> = log
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["kind"] == "masked_key")
        .map(|v| (v["start"].as_u64().unwrap(), v["end"].as_u64().unwrap()))
        .collect();
    // The script's offsets are in the text as the earlier rules left it. Nothing in the
    // corpus matches an earlier rule, so they are the original offsets.
    assert_eq!(got, want, "Rust and Python find the same masks");
    let redacted = std::fs::read_to_string(&output).unwrap();
    assert_eq!(
        redacted.matches("<redacted:masked_key:").count(),
        want.len()
    );
}

// ---------------------------------------------------------------------------------------
// `spikes/redact.py` against the shared set: one regression gate for drift.
//
// The script is a third copy of the rules (after the batch redactor and the stream one). These
// tests read it, run it, and compare it with `patterns()` for every rule the two share, so a
// rule edited on one side fails a test. Every difference between the two is written out in a
// table below (`SCRIPT_RULE_SPELLING_DELTAS`, `RUST_ONLY_KINDS`, `CLASS_DELTAS`,
// `OUTPUT_DELTAS`); a difference that appears, or goes away, fails too. All of it is skipped,
// loudly, without `python3`.
// ---------------------------------------------------------------------------------------

/// Run a Python snippet against `spikes/redact.py` (its path is `sys.argv[1]`) and parse the
/// JSON it prints; `None` without `python3`. `-B`: importing the script must not leave a
/// `__pycache__` in the repository.
fn python_json(snippet: &str) -> Option<serde_json::Value> {
    let script =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spikes/redact.py");
    match std::process::Command::new("python3")
        .arg("-B")
        .arg("-c")
        .arg(snippet)
        .arg(&script)
        .output()
    {
        Ok(out) => {
            assert!(
                out.status.success(),
                "python3 failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            Some(serde_json::from_slice(&out.stdout).expect("the snippet prints JSON"))
        }
        Err(e) => {
            eprintln!("python3 is not available ({e}); the python parity checks are skipped");
            None
        }
    }
}

/// A snippet that imports the script as a module (it has a `__main__` guard), binds it to
/// `m`, and then runs `body`.
fn with_script(body: &str) -> String {
    format!(
        "import importlib.util, json, sys\n\
         spec = importlib.util.spec_from_file_location('redact', sys.argv[1])\n\
         m = importlib.util.module_from_spec(spec)\n\
         spec.loader.exec_module(m)\n\
         {body}\n"
    )
}

/// The script's rules where the shared set spells the regex differently, as
/// `(kind, python regex, rust regex)`. One literal differs in spelling: in the shell/TOML literal the
/// script's groups capture, which does not matter to it (it replaces `m.group(0)`); the shared
/// literal's are non-capturing, because in Rust a capture group
/// *is* the span (`Pattern::span_group`) and this rule's span is the whole match. And the
/// script writes the optional quote as `['\"]` inside a Python raw string, which is an escaped
/// quote that means the same as Rust's `['"]`.
///
/// Two differ in a bound, because the daemon's stream holds an undecided run and the script
/// holds nothing. The daemon's `email` local part is at most 1,024 units, so a bare token run does
/// not hold to the hold bound (I-2); the script keeps `+` and redacts a longer local part whole.
/// The daemon's PEM block ends at an END line that starts within 12,288 bytes, or is those
/// bytes, so a lone BEGIN no longer redacts the rest of the stream (I-1); the script
/// runs the block to its END line or the end of the text. In both the script redacts more,
/// never less.
const SCRIPT_RULE_SPELLING_DELTAS: &[(&str, &str, &str)] = &[
    (
        "generic_secret_assignment",
        r#"(?i)(api[_-]?key|secret|token|password)\s*[=:]\s*(?:\\?['\"])?([A-Za-z0-9_\-]{16,})"#,
        r#"(?i)(?:api[_-]?key|secret|token|password)\s*[=:]\s*(?:\\?['"])?(?:[A-Za-z0-9_\-]{16,})"#,
    ),
    (
        "email",
        r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}",
        r"[A-Za-z0-9._%+\-]{1,1024}@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}",
    ),
    (
        "private_key",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|\Z)",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----(?:(?-u:[\x00-\xFF]){0,12288}?-----END [A-Z ]*PRIVATE KEY-----|(?-u:[\x00-\xFF]){0,12288})",
    ),
];

/// The shared literals the script has no counterpart for, in `patterns()` order: the JSON form
/// of `generic_secret_assignment` (the script needs `[=:]` right after the key word, so it
/// never fires on a quoted JSON key) and the seven vendor prefixes it predates.
const RUST_ONLY_KINDS: &[&str] = &[
    "generic_secret_assignment",
    "google_key",
    "huggingface_token",
    "npm_token",
    "xai_key",
    "stripe_key",
    "gitlab_token",
    "digitalocean_token",
];

/// Every regex of `spikes/redact.py` is a literal of the shared set, under the same kind: the
/// same string, or the one spelling delta above. What the shared set carries beyond that is
/// exactly [`RUST_ONLY_KINDS`], and the rules the two share are in the same order (`masked_key`
/// aside: the script lists it last and the shared set after `openai_key`, and being soft it
/// wins no overlap, so neither place matters).
#[test]
fn spikes_redact_py_patterns_match_the_shared_set() {
    let Some(script) = python_json(&with_script(
        "print(json.dumps([[k, rx.pattern] for k, rx in m.PATTERNS]))",
    )) else {
        return;
    };
    let script: Vec<(String, String)> = script
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r[0].as_str().unwrap().to_string(),
                r[1].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(script.len(), 11, "the script's ten rules and masked_key");
    let shared = goalcup_daemon::redact::patterns();

    let mut paired = vec![false; shared.len()];
    let mut deltas_used = vec![false; SCRIPT_RULE_SPELLING_DELTAS.len()];
    let mut positions = Vec::new();
    for (kind, py) in &script {
        let rust_spelling = SCRIPT_RULE_SPELLING_DELTAS
            .iter()
            .position(|(k, p, _)| k == kind && p == py)
            .map(|i| {
                deltas_used[i] = true;
                SCRIPT_RULE_SPELLING_DELTAS[i].2
            })
            .unwrap_or(py.as_str());
        let at = (0..shared.len())
            .find(|&i| {
                !paired[i] && shared[i].kind == *kind && shared[i].re.as_str() == rust_spelling
            })
            .unwrap_or_else(|| {
                panic!(
                    "redact.py's {kind} rule has no literal in the shared set:\n  script: {py}\n\
                     Change both, or write the difference into SCRIPT_RULE_SPELLING_DELTAS."
                )
            });
        paired[at] = true;
        positions.push((at, kind.as_str()));
    }
    assert!(
        deltas_used.iter().all(|used| *used),
        "SCRIPT_RULE_SPELLING_DELTAS has a row the script no longer carries"
    );

    let unpaired: Vec<&str> = shared
        .iter()
        .zip(&paired)
        .filter(|(_, paired)| !**paired)
        .map(|(p, _)| p.kind.as_str())
        .collect();
    assert_eq!(
        unpaired, RUST_ONLY_KINDS,
        "the shared set carries a literal the script does not, or the reverse"
    );

    // Order: the shared set lists the script's rules in the script's order, `masked_key`
    // excepted.
    let script_order: Vec<&str> = script
        .iter()
        .map(|(k, _)| k.as_str())
        .filter(|k| *k != "masked_key")
        .collect();
    positions.sort();
    let shared_order: Vec<&str> = positions
        .iter()
        .map(|(_, k)| *k)
        .filter(|k| *k != "masked_key")
        .collect();
    assert_eq!(
        shared_order, script_order,
        "the two lists order the shared rules alike"
    );
}

/// The kinds of the script's rules.
const SCRIPT_KINDS: &[&str] = &[
    "anthropic_key",
    "openai_key",
    "github_token",
    "aws_access_key",
    "bearer",
    "jwt",
    "private_key",
    "slack_token",
    "generic_secret_assignment",
    "email",
    "masked_key",
];

/// Strip the script's per-match digest: `<redacted:KIND:0123abcd>` becomes the daemon's
/// `<redacted:KIND>`.
fn without_script_digests(text: &str) -> String {
    regex::Regex::new(r"<redacted:([A-Za-z0-9_\-]+):[0-9a-f]{8}>")
        .unwrap()
        .replace_all(text, "<redacted:$1>")
        .into_owned()
}

/// Run the probes through the script (once, joined by a separator that no rule can cross)
/// and through the daemon (one by one). Each row is `(probe, script output, daemon output)`;
/// `None` without `python3`.
fn run_both(probes: &[String]) -> Option<Vec<(String, String, String)>> {
    const SEP: &str = "\n~~~~\n";
    let out = run_redact_py(&probes.join(SEP))?;
    let out = without_script_digests(&out);
    let outs: Vec<&str> = out.split(SEP).collect();
    assert_eq!(outs.len(), probes.len(), "the separators survive redaction");
    Some(
        probes
            .iter()
            .zip(outs)
            .map(|(probe, py)| {
                let rust = Redactor::new().apply(probe.as_bytes());
                (
                    probe.clone(),
                    py.to_string(),
                    String::from_utf8(rust.bytes).unwrap(),
                )
            })
            .collect(),
    )
}

/// For each of the script's rules, shapes it redacts and near misses it leaves alone, and a
/// few rules overlapping. No probe is a shape only the daemon has (a quoted JSON key, a vendor
/// prefix), none holds a character in [`CLASS_DELTAS`], and none is one where the script's
/// rule-after-rule substitution and the daemon's first-wins claiming draw a boundary
/// differently: `spikes_redact_py_is_never_weaker_than_the_daemon` holds those.
fn shared_rule_probes() -> Vec<String> {
    let a = |n: usize| "A".repeat(n);
    let mut p: Vec<String> = Vec::new();
    let mut add = |s: String| p.push(s);
    // anthropic_key, openai_key
    add("x sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFF y".into());
    add(format!("sk-ant-{}", a(20)));
    add(format!("sk-ant-{}", a(19)));
    add(format!("sk-proj-{}", a(24)));
    add(format!("sk-{}", a(20)));
    add(format!("sk-{}", a(19)));
    add(format!("dash sk-{}_-{} end", a(10), a(10)));
    // github_token: all five prefixes, and near misses
    for k in ["p", "o", "u", "s", "r"] {
        add(format!("gh{k}_{}", a(20)));
    }
    add(format!("ghp_{}", a(19)));
    add(format!("ghx_{}", a(20)));
    // aws_access_key
    add("AKIAIOSFODNN7EXAMPLE and AKIA0123456789ABCDEF".into());
    add("AKIAIOSFODNN7EXAMPL".into());
    add("akiaiosfodnn7example".into());
    // bearer: case, whitespace, the dotted class
    add(format!("Authorization: Bearer {}", a(16)));
    add(format!("bearer\t{}", a(16)));
    add(format!("BEARER {}", a(16)));
    add(format!("Bearer abc.def-ghi_jkl.{}", a(4)));
    add(format!("Bearer {}", a(15)));
    add(format!("Bearer{}", a(16)));
    // jwt
    add("eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4".into());
    add("eyJhbGciOiJIUzI.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4".into());
    add("eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJS".into());
    // private_key: labelled, unlabelled, two in a row (one that never ends is last, below)
    add("-----BEGIN RSA PRIVATE KEY-----\nMIIB\nQQ==\n-----END RSA PRIVATE KEY-----".into());
    add("-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----".into());
    add(
        "-----BEGIN EC PRIVATE KEY-----\na\n-----END EC PRIVATE KEY----- and \
         -----BEGIN PRIVATE KEY-----\nb\n-----END PRIVATE KEY-----"
            .into(),
    );
    // slack_token
    for k in ["b", "a", "p", "r", "s"] {
        add(format!("xox{k}-1234567890-ABCDEFGHIJ"));
    }
    add("xoxb-123456789".into());
    add("xoxz-1234567890-ABCDEFGHIJ".into());
    // generic_secret_assignment: the script's shell/TOML form
    for key in [
        "api_key", "API-KEY", "apikey", "secret", "token", "Password",
    ] {
        for sep in ["=", ": ", " = ", ":"] {
            add(format!("{key}{sep}{}", a(16)));
        }
    }
    add(format!("api_key = \"{}\"", a(16)));
    // The quote escaped inside a JSON string (I-4): the span takes the backslash with it.
    add(format!("db:\\n  password: \\\"{}\\\"", a(20)));
    add(format!("token=\\'{}", a(16)));
    add(format!("token=\\{}", a(16)));
    add(format!("token='{}", a(20)));
    add(format!("export MY_SECRET={}", a(24)));
    add(format!("api_key={}", a(15)));
    add(format!("tokenizer={}", a(20)));
    add(format!("token {}", a(20)));
    // email
    add("write to a@b.co or user.name+tag@ex-ample.com.".into());
    add("a@b".into());
    add("@example.com".into());
    add("a@b.c".into());
    // masked_key, alone
    add("sk-ab***wxyz and sk-abcde...wxyz and sk-0...cdef".into());
    add("sk-R7aMV*****************oKYo".into());
    add("sk-ab**wxyz sk-ab***wx".into());
    add("sk-ab\\u2026wxyz".into());
    // rules overlapping, where the earlier rule wins whole in both
    add(format!("Bearer sk-ant-api03-{}", a(24)));
    add(format!("token=sk-ant-api03-{}", a(24)));
    add(format!("api_key={}@example.com", a(20)));
    add(format!("Bearer {}.{}@ex.com", a(10), a(10)));
    add(format!(
        "eyJ{}.eyJ{}.eyJ{}@host.example.com",
        a(10),
        a(10),
        a(10)
    ));
    add(format!("{} sk-ab***wxyz {}", a(3), a(3)));
    // private_key with an earlier rule's match inside it: the block is one redaction in both
    for inner in [
        "ghp_AAAAAAAAAAAAAAAAAAAA",
        "AKIAIOSFODNN7EXAMPLE",
        "x@y.com",
        "Bearer AAAAAAAAAAAAAAAA",
        "sk-ant-api03-AAAABBBBCCCCDDDDEEEE",
        "xoxb-1234567890-ABCDEFGHIJ",
    ] {
        add(format!(
            "k:\n-----BEGIN PRIVATE KEY-----\nMIIB\n{inner}\nQQ==\n-----END PRIVATE KEY-----\nz"
        ));
    }
    // private_key with a claim across its edge: one redaction in both, the outside part of the
    // claim with it (`bearer` takes `-----BEGIN` and needs sixteen characters, `secret=` the
    // same; an email's local part takes `KEY-----`).
    let block = "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----";
    add(format!("Authorization: Bearer {}{block}\nz", a(16)));
    add(format!("Authorization: Bearer abcdefgh{block}\nz"));
    add(format!("secret=ABCDEFGHIJ{block}\nz"));
    add(format!("{block}x@y.com tail"));
    add(format!("{block}x@y.com{block}"));
    // A BEGIN with no END runs to the end of the text. It is last because the script runs over
    // every probe as one text, where its end is the end of the last probe.
    add("-----BEGIN PRIVATE KEY-----\nMIIB\nno end line".into());
    p
}

/// Every rule the two share: the script and the daemon produce the same redacted text.
#[test]
fn spikes_redact_py_and_the_daemon_redact_the_shared_rules_alike() {
    let probes = shared_rule_probes();
    let Some(rows) = run_both(&probes) else {
        return;
    };
    let mut reached = std::collections::BTreeSet::new();
    for (probe, py, rust) in &rows {
        assert_eq!(py, rust, "script and daemon differ on {probe:?}");
        for kind in SCRIPT_KINDS {
            if rust.contains(&format!("<redacted:{kind}>")) {
                reached.insert(*kind);
            }
        }
    }
    // The probes reach every rule the script has (a set that missed one would pass for the
    // wrong reason), and leave some text alone (so "same output" is not "both redact all").
    for kind in SCRIPT_KINDS {
        assert!(reached.contains(kind), "no probe reaches {kind}");
    }
    assert!(
        rows.iter().filter(|(probe, _, rust)| probe == rust).count() >= 8,
        "the near misses stay raw in both"
    );
}

/// The code points whose membership in a regex class the script and the daemon disagree on,
/// both ways: `(key in the script's dump, class, script only, daemon only)`. These are the whole of the two
/// engines' difference for the classes the shared rules use, and they are why `OUTPUT_DELTAS`
/// has the rows it has.
///
/// - `\s`: Python's `str` patterns count the four ASCII information separators U+001C..U+001F
///   as whitespace (`str.isspace`); Rust's `\s` is Unicode `White_Space`, which does not.
/// - case-insensitive `[A-Za-z0-9]`: Python's `IGNORECASE` also folds U+0130 `İ` and U+0131
///   `ı` onto `I` and `i`; Rust's simple case folding does not. Both engines fold U+017F `ſ`
///   (onto `s`) and U+212A `K` (onto `k`), so those are not here.
///
/// In both cases the script redacts more (it over-redacts), never less.
const CLASS_DELTAS: &[(&str, &str, &[u32], &[u32])] = &[
    ("space", r"\s", &[0x1c, 0x1d, 0x1e, 0x1f], &[]),
    ("fold", "(?i)[A-Za-z0-9]", &[0x130, 0x131], &[]),
];

#[test]
fn spikes_redact_py_character_classes_differ_from_the_daemons_only_as_documented() {
    let Some(python) = python_json(&with_script(
        r#"import re
space = re.compile(r'\s')
word = re.compile(r'(?i)[A-Za-z0-9]')
sets = {'space': [], 'fold': []}
for cp in range(0x110000):
    if 0xD800 <= cp <= 0xDFFF:
        continue
    if space.fullmatch(chr(cp)):
        sets['space'].append(cp)
    if word.fullmatch(chr(cp)):
        sets['fold'].append(cp)
print(json.dumps(sets))"#,
    )) else {
        return;
    };
    let python_set = |name: &str| -> std::collections::BTreeSet<u32> {
        python[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect()
    };
    for &(name, class, python_only, rust_only) in CLASS_DELTAS {
        let rust_re = regex::Regex::new(&format!("^{class}$")).unwrap();
        let rust: std::collections::BTreeSet<u32> = (0..0x110000u32)
            .filter_map(char::from_u32)
            .filter(|c| rust_re.is_match(c.encode_utf8(&mut [0; 4])))
            .map(u32::from)
            .collect();
        let python = python_set(name);
        let only_python: Vec<u32> = python.difference(&rust).copied().collect();
        let only_rust: Vec<u32> = rust.difference(&python).copied().collect();
        assert_eq!(
            only_python, python_only,
            "{class}: code points only the script's class admits"
        );
        assert_eq!(
            only_rust, rust_only,
            "{class}: code points only the daemon's class admits"
        );
    }
}

/// The known differences, end to end: `(why, probe, script output, daemon output)`. A probe
/// here is one the script redacts and the daemon does not, or redacts shorter. Each row
/// follows from a row of [`CLASS_DELTAS`]. (The one row the engines' different overlap rules
/// used to add, an earlier rule's match inside a PEM block, is gone: the daemon's block rule
/// absorbs it, see `a_pem_block_is_never_shadowed_by_a_claim_inside_it`.)
fn output_deltas() -> Vec<(&'static str, String, &'static str, String)> {
    vec![
        (
            "U+0131 folds onto `i` in the script's `[A-Za-z0-9-]` (the key part of a mask)",
            "sk-ab\u{131}***wxyz".to_string(),
            "<redacted:masked_key>",
            "sk-ab\u{131}***wxyz".to_string(),
        ),
        (
            "U+0130 folds onto `I` in the script's `[A-Za-z0-9]` (the tail of a mask)",
            "sk-ab***wxy\u{130}".to_string(),
            "<redacted:masked_key>",
            "<redacted:masked_key>\u{130}".to_string(),
        ),
        (
            "U+001F is whitespace to the script's `\\s`",
            format!("Bearer\u{1f}{}", "A".repeat(16)),
            "<redacted:bearer>",
            format!("Bearer\u{1f}{}", "A".repeat(16)),
        ),
    ]
}

#[test]
fn spikes_redact_py_and_the_daemon_differ_on_the_documented_probes_only_as_documented() {
    let deltas = output_deltas();
    let probes: Vec<String> = deltas.iter().map(|d| d.1.clone()).collect();
    let Some(rows) = run_both(&probes) else {
        return;
    };
    for ((why, probe, script, daemon), (_, py, rust)) in deltas.iter().zip(rows) {
        assert_eq!(&py, script, "{why}: the script's output for {probe:?}");
        assert_eq!(&rust, daemon, "{why}: the daemon's output for {probe:?}");
        assert_ne!(py, rust, "{why}: the difference is gone; remove the row");
    }
}

/// Run `spikes/redact.py` over `text`; the redacted text, or `None` without `python3`.
fn run_redact_py(text: &str) -> Option<String> {
    let script =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spikes/redact.py");
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.txt");
    let output = dir.path().join("out.txt");
    std::fs::write(&input, text).unwrap();
    match std::process::Command::new("python3")
        .arg(&script)
        .arg(&input)
        .arg(&output)
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) => assert!(status.success()),
        Err(e) => {
            eprintln!("python3 is not available ({e}); the behavioural half is skipped");
            return None;
        }
    }
    Some(std::fs::read_to_string(&output).unwrap())
}

/// The fixtures script is never weaker than the daemon. Its sequential
/// pass left `key[:8]` of `sk-abcde***wxyz@ex.com` raw (the email replaced `wxyz@ex.com`, and
/// what was left no longer matched), where the daemon redacts all of it.
#[test]
fn spikes_redact_py_is_never_weaker_than_the_daemon() {
    // The named inputs: M-10, and the six that shadowed before the rule was soft.
    let named = [
        ("sk-abcde***wxyz@ex.com", vec!["sk-abcde", "wxyz"]),
        ("api_key=SECRETSECRETSECRETsk-ab***wxyz", vec!["SECRET"]),
        (
            "password: \"SECRETSECRETSECRETask-ab***wxyz\"",
            vec!["SECRET"],
        ),
        ("xoxb-SECRETSECRETsk-ab***wxyz", vec!["SECRET"]),
        (
            "Bearer sk-ab...wxyzSECRETTAILSECRETTAIL",
            vec!["SECRET", "wxyz"],
        ),
        (
            "Bearer abcdefghij.sk-ab...wxyz.SECRETTAILSECRETTAIL",
            vec!["SECRET", "wxyz"],
        ),
        (
            "privatelocal.part.sk-ab...wxyz@example.com",
            vec!["privatelocal", "wxyz", "example"],
        ),
        ("dump:\nsk-abcde...wxyz", vec!["sk-abcde"]),
        // A PEM block with an earlier rule's match across its BEGIN edge: the sequential pass
        // used to let `bearer` take `-----BEGIN` and leave the body raw.
        (
            "Bearer AAAAAAAAAAAAAAAA-----BEGIN PRIVATE KEY-----\nSECRETBODY\n-----END PRIVATE KEY-----",
            vec!["SECRETBODY", "PRIVATE KEY"],
        ),
        (
            "-----BEGIN PRIVATE KEY-----\nSECRETBODY\nghp_AAAAAAAAAAAAAAAAAAAA\n-----END PRIVATE KEY-----x@y.com",
            vec!["SECRETBODY", "ghp_", "PRIVATE KEY"],
        ),
        // The outside part of a claim across the BEGIN edge is below the rule's minimum once the
        // block is gone from it: it is redacted with the block, not left behind.
        (
            "Authorization: Bearer abcdefgh-----BEGIN PRIVATE KEY-----\nSECRETBODY\n-----END PRIVATE KEY-----",
            vec!["abcdefgh", "SECRETBODY"],
        ),
        (
            "secret=ABCDEFGHIJ-----BEGIN PRIVATE KEY-----\nSECRETBODY\n-----END PRIVATE KEY-----",
            vec!["ABCDEFGHIJ", "SECRETBODY"],
        ),
        // A key cut off before its END line.
        (
            "key follows\n-----BEGIN RSA PRIVATE KEY-----\nSECRETBODY\nsecond SECRETLINE",
            vec!["SECRETBODY", "SECRETLINE", "PRIVATE KEY"],
        ),
    ];
    let joined: String = named.iter().map(|(line, _)| format!("{line}\n")).collect();
    let Some(out) = run_redact_py(&joined) else {
        return;
    };
    for (line, forbidden) in &named {
        // Each line is redacted on its own in the daemon; here the whole text goes through
        // once, so look at the lines of the output that correspond (no rule spans a newline
        // in these inputs).
        for needle in forbidden {
            assert!(
                !out.contains(needle),
                "{line:?}: the script left {needle:?} raw:\n{out}"
            );
        }
    }

    // A differential over glued shapes: every span the daemon redacts is absent, whole, from
    // the script's output. (A leak shows as a raw span; a different boundary does not.)
    let pieces: &[&str] = &[
        "sk-ab***wxyz",
        "sk-abcde...wxyz",
        "sk-0...cdef",
        "sk-or-v1...cdef",
        "api_key=",
        "token: ",
        "password=",
        "Bearer ",
        "xoxb-",
        "ghp_",
        "SECRETSECRETSECRET",
        "abcdefghij",
        "user.name",
        "@example.com",
        "x@",
        ".",
        "-",
        "_",
        " ",
        "ab",
        "wxyz",
    ];
    let mut rng = Lcg(0xFEEDF00D);
    let mut text = String::new();
    let mut lines = Vec::new();
    for _ in 0..1500 {
        let mut line = String::new();
        for _ in 0..1 + rng.below(7) {
            line.push_str(pieces[rng.below(pieces.len())]);
        }
        text.push_str(&line);
        text.push('\n');
        lines.push(line);
    }
    let Some(out) = run_redact_py(&text) else {
        return;
    };
    let mut checked = 0;
    for line in &lines {
        for span in Redactor::new().apply(line.as_bytes()).spans {
            let raw = &line[span.start as usize..span.end as usize];
            // A span too short to be told from ordinary text proves nothing by being absent.
            if raw.len() < 8 {
                continue;
            }
            checked += 1;
            assert!(
                !out.contains(raw),
                "{line:?}: the daemon redacts {raw:?} ({}) and the script left it raw",
                span.kind
            );
        }
    }
    assert!(
        checked > 1000,
        "the differential reaches the rules: {checked}"
    );
}

/// Softness belongs to the shared rule's slot, not to a kind name. A
/// connector's own pattern that happens to be called `masked_key` is an ordinary hard rule,
/// and two rules of that name overlapping cannot make `apply` panic.
#[test]
fn a_connector_pattern_named_masked_key_is_an_ordinary_rule() {
    let mut m = connector("codex-cli");
    m.push(goalcup_protocol::manifest::RedactionPattern {
        kind: "masked_key".to_string(),
        regex: r"k-ab\.\.\.wx".to_string(),
    });
    let mut r = Redactor::for_patterns("codex-cli", &m).expect("the connector compiles");
    for line in [
        &b"see sk-ab...wxyz now"[..],
        b"sk-ab...wxyz sk-ab...wxyz",
        b"k-ab...wx sk-ab***wxyz",
    ] {
        let out = r.apply(line);
        let text = String::from_utf8(out.bytes).unwrap();
        assert!(!text.contains("wxyz") && !text.contains("sk-ab"), "{text}");
        assert!(
            out.spans.windows(2).all(|w| w[0].end <= w[1].start),
            "never overlapping"
        );
    }
}

/// The block every test below wraps something in.
const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
const END: &str = "-----END PRIVATE KEY-----";

/// One span, of kind `private_key`, from `start` to `end` of `text`; nothing else.
fn assert_one_block(text: &str, start: usize, end: usize, out: &goalcup_daemon::redact::Redacted) {
    assert_eq!(
        out.spans.len(),
        1,
        "one claim, the block: {:?} in {text:?}",
        out.spans
    );
    let span = &out.spans[0];
    assert_eq!(span.kind, "private_key", "{text:?}");
    assert_eq!(
        (span.start as usize, span.end as usize),
        (start, end),
        "{text:?}"
    );
}

/// A PEM block is a whole-block claim that no claim inside it shadows. Before, first-wins by
/// list position gave an inner `github_token` or `aws_access_key` (listed before
/// `private_key`) the claim and dropped the block whole: the match was redacted and the
/// BEGIN/END lines and the rest of the key were kept raw.
#[test]
fn a_pem_block_is_never_shadowed_by_a_claim_inside_it() {
    for inner in [
        "ghp_AAAAAAAAAAAAAAAAAAAA",
        "AKIAIOSFODNN7EXAMPLE",
        "sk-ant-api03-AAAABBBBCCCCDDDDEEEE",
        "sk-proj-AAAABBBBCCCCDDDDEEEE",
        "Bearer AAAAAAAAAAAAAAAA",
        "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4",
        "x@y.com",
        "api_key=AAAAAAAAAAAAAAAA",
        "xoxb-1234567890-ABCDEFGHIJ",
        "sk-ab***wxyz",
        // several, and one that is only part of a line
        "ghp_AAAAAAAAAAAAAAAAAAAA AKIAIOSFODNN7EXAMPLE x@y.com",
    ] {
        let block = format!("{BEGIN}\nMIIBsecretbody\n{inner}\nQQ==\n{END}");
        let text = format!("before\n{block}\nafter");
        let out = Redactor::new().apply(text.as_bytes());
        assert_eq!(
            String::from_utf8(out.bytes.clone()).unwrap(),
            "before\n<redacted:private_key>\nafter",
            "inner {inner:?}"
        );
        assert_one_block(
            &text,
            "before\n".len(),
            "before\n".len() + block.len(),
            &out,
        );
    }
}

/// The real trigger: a `BEGIN` whose `END` never comes runs, lazily, to the next key's `END`.
/// One claim inside the run used to leave both keys raw.
#[test]
fn an_unterminated_begin_runs_to_the_next_ends_block_and_one_claim_inside_leaves_nothing_raw() {
    let run = format!(
        "{BEGIN}\nMIIBfirstsecret\nghp_AAAAAAAAAAAAAAAAAAAA\nnotes\n\
         -----BEGIN RSA PRIVATE KEY-----\nMIIBsecondsecret\n-----END RSA PRIVATE KEY-----"
    );
    let text = format!("log\n{run}\ntail");
    let out = Redactor::new().apply(text.as_bytes());
    let redacted = String::from_utf8(out.bytes.clone()).unwrap();
    assert_eq!(redacted, "log\n<redacted:private_key>\ntail");
    assert!(!redacted.contains("secret"));
    assert_one_block(&text, "log\n".len(), "log\n".len() + run.len(), &out);

    // The same, with the claim in the later key and with an AWS key.
    let run = format!(
        "{BEGIN}\nMIIBfirstsecret\nnotes\n-----BEGIN RSA PRIVATE KEY-----\nAKIAIOSFODNN7EXAMPLE\n\
         MIIBsecondsecret\n-----END RSA PRIVATE KEY-----"
    );
    let out = Redactor::new().apply(run.as_bytes());
    assert_eq!(out.bytes, b"<redacted:private_key>");
    assert_one_block(&run, 0, run.len(), &out);
}

/// A claim across the block's edge is merged with it: one span from the earlier start to the
/// later end, named for the block. Before, a claim that won (`bearer`) dropped the block whole
/// and the key's body stayed raw; one that lost (`email`) was dropped whole and its tail
/// stayed raw.
#[test]
fn a_claim_across_a_pem_edge_is_merged_into_the_block() {
    // Across the BEGIN edge: `bearer`'s value class takes `-----BEGIN`.
    let lead = format!("Authorization: Bearer {}", "A".repeat(16));
    let block = format!("{BEGIN}\nMIIBsecretbody\n{END}");
    let text = format!("{lead}{block}\nafter");
    let out = Redactor::new().apply(text.as_bytes());
    assert_eq!(
        String::from_utf8(out.bytes.clone()).unwrap(),
        "Authorization: <redacted:private_key>\nafter"
    );
    assert_one_block(
        &text,
        "Authorization: ".len(),
        lead.len() + block.len(),
        &out,
    );

    // Across the END edge: an email's local part takes `KEY-----`.
    let text = format!("{BEGIN}\nMIIBsecretbody\n{END}x@y.com tail");
    let out = Redactor::new().apply(text.as_bytes());
    assert_eq!(
        String::from_utf8(out.bytes.clone()).unwrap(),
        "<redacted:private_key> tail"
    );
    assert_one_block(&text, 0, text.len() - " tail".len(), &out);

    // Near the block but not touching it: still two claims, nothing merged.
    let text = format!("ghp_AAAAAAAAAAAAAAAAAAAA\n{BEGIN}\nMIIB\n{END}\nx@y.com");
    let out = Redactor::new().apply(text.as_bytes());
    assert_eq!(
        String::from_utf8(out.bytes.clone()).unwrap(),
        "<redacted:github_token>\n<redacted:private_key>\n<redacted:email>"
    );
    assert_eq!(out.spans.len(), 3);
}

/// Only the shared block rule absorbs. A connector pattern that reuses the name `private_key`
/// is an ordinary rule, last in the list, and loses an overlap as any extra pattern does.
#[test]
fn a_connector_pattern_named_private_key_is_an_ordinary_rule() {
    let mut m = connector("codex-cli");
    m.push(goalcup_protocol::manifest::RedactionPattern {
        kind: "private_key".to_string(),
        regex: r"A{10}-ZZZZ".to_string(),
    });
    let mut r = Redactor::for_patterns("codex-cli", &m).expect("the connector compiles");
    let text = format!("ghp_{}-ZZZZ", "A".repeat(20));
    let out = r.apply(text.as_bytes());
    assert_eq!(
        String::from_utf8(out.bytes).unwrap(),
        "<redacted:github_token>-ZZZZ",
        "the connector's rule lost the overlap whole, as an ordinary rule does"
    );
}

/// What a PEM block with no claim near it, or with `masked_key` inside, does.
#[test]
fn a_pem_block_without_competing_claims_and_masked_keys_inside_it() {
    let text = format!("{BEGIN}\nMIIB\n{END}");
    let out = Redactor::new().apply(text.as_bytes());
    assert_one_block(&text, 0, text.len(), &out);
    // A mask inside the block is soft: the block covers all of it, and the block alone is the
    // audit.
    let text = format!("{BEGIN}\nsk-ab***wxyz\n{END}");
    let out = Redactor::new().apply(text.as_bytes());
    assert_one_block(&text, 0, text.len(), &out);
    // A mask across the block's edge keeps its outside part, as before.
    let text = format!("{BEGIN}\nMIIB\n{END} sk-ab***wxyz");
    let out = Redactor::new().apply(text.as_bytes());
    assert_eq!(
        String::from_utf8(out.bytes).unwrap(),
        "<redacted:private_key> <redacted:masked_key>"
    );
}

/// A synthetic RSA block.
const KEY: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Qu\n-----END RSA PRIVATE KEY-----";

/// Which bytes of `input` a redaction replaced.
fn byte_cover(input: &[u8], out: &goalcup_daemon::redact::Redacted) -> Vec<bool> {
    let mut covered = vec![false; input.len()];
    for s in &out.spans {
        for c in &mut covered[s.start as usize..s.end as usize] {
            *c = true;
        }
    }
    covered
}

/// Taking the block out of first-wins let the lower-priority claims it used to drop win, and
/// they shadowed other claims that first-wins redacts (the reviewer's 31 inputs). The block
/// merges with what it overlaps after first-wins, so those tokens stay redacted.
#[test]
fn a_block_does_not_free_a_lower_claim_to_shadow_a_token() {
    for (input, want) in [
        (
            // An email across the END edge no longer beats the `glpat-` token that starts in
            // its top-level domain.
            format!("{KEY}x@a.glpat-ABCDEFGHIJKLMNOPQRSTUVWXYZ tail"),
            "<redacted:private_key> tail",
        ),
        (
            // The shell form across the BEGIN edge no longer beats the `glpat-` token that
            // ends in its key word.
            format!("glpat-ABCDEFGHIJKLMNOPQRSTUVtoken=ABCDEFGHIJKLMNOPQR{KEY}"),
            "<redacted:private_key>",
        ),
    ] {
        let out = Redactor::new().apply(input.as_bytes());
        let text = String::from_utf8(out.bytes.clone()).unwrap();
        assert_eq!(text, want, "{input:?}");
        let base = Redactor::new().apply_first_wins_only(input.as_bytes());
        let (now, was) = (
            byte_cover(input.as_bytes(), &out),
            byte_cover(input.as_bytes(), &base),
        );
        assert!(
            was.iter().zip(&now).all(|(was, now)| !was || *now),
            "{input:?}: first-wins redacts bytes the merge left raw"
        );
    }
}

/// Pieces that build PEM-heavy inputs: headers and END lines (labelled, bare, partial),
/// separators, body fragments, and the tokens, emails and assignments that overlap them.
const PEM_PIECES: &[&str] = &[
    "-----BEGIN RSA PRIVATE KEY-----",
    "-----BEGIN PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "-----END RSA PRIVATE KEY-----",
    "-----END PRIVATE KEY-----",
    "-----END OPENSSH PRIVATE KEY-----",
    "-----BEGIN ",
    "-----END ",
    "PRIVATE KEY-----",
    "-----",
    "\n",
    "\r\n",
    "\\n",
    "\\r\\n",
    "MIIBOgIBAAJBAKj34GkxFhD9",
    "0vcNLYLInFEX6Ppy1tPf9Cnz+/=",
    "ghp_ABCDEFGHIJKLMNOPQRSTUVWX",
    "AKIAABCDEFGHIJKLMNOP",
    "Bearer ",
    "abcdefgh",
    "KEY-----",
    "x@a.glpat",
    "glpat-ABCDEFGHIJKLMNOPQRSTUV",
    "xai-ABCDEFGHIJKLMNOPQRSTUV",
    "x@ex.com",
    "@",
    ".",
    "-",
    "token=",
    "\"api_key\": \"",
    "\"",
    "ABCDEFGHIJKLMNOPQR",
    "sk-ab***wxyz",
    "sk-0123456789abcdef0123456789abcdef",
    "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2Q",
    "xoxb-ABCDEFGHIJK",
    " ",
    "\t",
    "\u{e9}",
];

/// The differential: for every input of a PEM-heavy corpus, what a block leaves redacted is a
/// superset of what first-wins alone redacts (`apply_first_wins_only`, the real selection with
/// the blocks not merged), every byte of every block match is redacted, and the spans are
/// sorted and never overlap. Includes the reviewer's two examples, a key with no END, and
/// blocks meeting a token on either side.
#[test]
fn a_block_only_ever_adds_redaction_to_first_wins() {
    let block_re = goalcup_daemon::redact::patterns()
        .iter()
        .find(|p| p.kind == "private_key")
        .unwrap()
        .re
        .clone();
    let mut rng = Lcg(0x9E37_7991_2345_6789);
    let mut corpus: Vec<String> = vec![
        format!("{KEY}x@a.glpat-ABCDEFGHIJKLMNOPQRSTUVWXYZ tail"),
        format!("glpat-ABCDEFGHIJKLMNOPQRSTUVtoken=ABCDEFGHIJKLMNOPQR{KEY}"),
        format!("Authorization: Bearer abcdefgh{KEY}"),
        format!("{KEY}KEY-----x@ex.comtoken=MIIBOgIBAAJBAKj34GkxFhD9{KEY}"),
        "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj34GkxFhD9\n(truncated)".to_string(),
    ];
    for _ in 0..40_000 {
        let mut text = String::new();
        for _ in 0..1 + rng.below(26) {
            text.push_str(PEM_PIECES[rng.below(PEM_PIECES.len())]);
        }
        corpus.push(text);
    }
    let (mut with_block, mut strictly_more) = (0, 0);
    for text in &corpus {
        let input = text.as_bytes();
        let out = Redactor::new().apply(input);
        let base = Redactor::new().apply_first_wins_only(input);
        let (now, was) = (byte_cover(input, &out), byte_cover(input, &base));
        for (i, (was, now)) in was.iter().zip(&now).enumerate() {
            assert!(
                !was || *now,
                "{text:?}: first-wins redacts byte {i}, the merge leaves it raw"
            );
        }
        assert!(
            out.spans.windows(2).all(|w| w[0].end <= w[1].start),
            "{text:?}: spans are sorted and disjoint"
        );
        let mut has_block = false;
        for m in block_re.find_iter(input) {
            has_block = true;
            assert!(
                now[m.start()..m.end()].iter().all(|c| *c),
                "{text:?}: a block match is not wholly redacted"
            );
        }
        with_block += usize::from(has_block);
        strictly_more += usize::from(now != was);
    }
    eprintln!(
        "block differential: {} inputs, {with_block} with a block, {strictly_more} redacted beyond first-wins, 0 weaker",
        corpus.len()
    );
    assert!(
        with_block > 10_000,
        "the corpus reaches blocks: {with_block}"
    );
    assert!(strictly_more > 1_000, "and the merge adds: {strictly_more}");
}

/// A key cut off before its END line (printed with `head`, truncated by an output limit, split
/// across records) is a block to the end of the record. The regex needed an END, and a
/// truncated key was returned raw.
#[test]
fn a_pem_block_with_no_end_runs_to_the_end_of_the_record() {
    let open = "-----BEGIN RSA PRIVATE KEY-----\nMIIBsecretbody\nmore secretbody\n(truncated)";
    let text = format!("log\n{open}");
    let out = Redactor::new().apply(text.as_bytes());
    assert_eq!(
        String::from_utf8(out.bytes.clone()).unwrap(),
        "log\n<redacted:private_key>"
    );
    assert_one_block(&text, "log\n".len(), text.len(), &out);

    // With a claim inside it, and a claim across its BEGIN edge.
    let text = format!("Bearer abcdefgh{open}\nghp_AAAAAAAAAAAAAAAAAAAA\ntail");
    let out = Redactor::new().apply(text.as_bytes());
    assert_eq!(
        String::from_utf8(out.bytes.clone()).unwrap(),
        "<redacted:private_key>"
    );
    assert_one_block(&text, 0, text.len(), &out);

    // An END line still ends it, and what follows stays.
    let text = format!("{open}\n{END}\nkept");
    let out = Redactor::new().apply(text.as_bytes());
    assert_eq!(
        String::from_utf8(out.bytes).unwrap(),
        "<redacted:private_key>\nkept"
    );
}

/// I-4: tool output and file contents reach every native transcript and request body as JSON
/// escaped inside a string, so a generic secret there is `\"api_key\": \"…\"`. Neither form of
/// `generic_secret_assignment` fired on it. Both now take a backslash before a quote, and the
/// redacted record is still JSON whose escaped string is still JSON: the JSON form's span is
/// the value's interior, with a bare marker between the escaped quotes, and the shell form's
/// span takes the escaped opening quote whole, so no backslash is left dangling.
#[test]
fn a_secret_in_json_escaped_text_is_redacted_and_the_record_stays_json() {
    let m = "<redacted:generic_secret_assignment>";
    let cases: [(&str, String); 2] = [
        (
            r#"{"type":"tool_result","content":"{\n  \"api_key\": \"q8Zr2LmN0pX4vB7cT1yW\",\n  \"client_secret\": \"Zx9Yw8Vu7Ts6Rq5Po4Nm\"\n}"}"#,
            format!(
                r#"{{"type":"tool_result","content":"{{\n  \"api_key\": \"{m}\",\n  \"client_secret\": \"{m}\"\n}}"}}"#
            ),
        ),
        (
            r#"{"content":"db:\n  password: \"CorrectHorseBattery9x\"\n"}"#,
            format!(r#"{{"content":"db:\n  {m}\"\n"}}"#),
        ),
    ];
    for (record, want) in &cases {
        let out = Redactor::new().apply(record.as_bytes());
        let got = String::from_utf8(out.bytes).unwrap();
        assert_eq!(&got, want, "{record}");
        assert!(
            out.spans
                .iter()
                .all(|s| s.kind == "generic_secret_assignment")
        );
        let outer: serde_json::Value = serde_json::from_str(&got).expect("the record is JSON");
        let inner = outer["content"]
            .as_str()
            .or(outer["output"].as_str())
            .unwrap();
        if inner.starts_with('{') {
            serde_json::from_str::<serde_json::Value>(inner).expect("the escaped text is JSON");
        }
    }
}
