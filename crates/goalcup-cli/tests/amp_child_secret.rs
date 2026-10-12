//! `AMP_API_KEY` goes through the same child-secret seam as `DEEPSEEK_API_KEY`.
//!
//! The token here is invented. Amp's real token grammar is UNKNOWN (m2-amp-readiness), so this
//! test pins only what Goal Cup does with a token of the admitted grammar: 20 to 256 bytes of
//! `[A-Za-z0-9_-]`, with no `sk-` prefix, so the key scan is shown to work for a shape the
//! DeepSeek tests do not cover. No real token is read, and nothing reads the real `HOME`.

mod m2_profile_support;

use goalcup_cli::CliError;
use goalcup_cli::key_scan::{FragmentMatcher, ScanOutcome, scan_tree};
use goalcup_cli::launch::{build_capture_launch, child_secret_for_harness};
use goalcup_protocol::harness::Origin;
use m2_profile_support as s;
use std::collections::BTreeMap;

/// An invented token of the admitted grammar. Never a real credential.
const AMP_TOKEN: &str = "amp-synthetic_7Qx2Lm9Rt4Vw8Yz1Bc5Df3Gh";

/// A shipped keyed profile (Hermes: per-run home, `stderr_line` identity) that names
/// `AMP_API_KEY` instead of its own key, and still validates.
fn amp_keyed(exe: Option<&std::path::Path>) -> goalcup_cli::connector::HarnessConnector {
    let mut c = match exe {
        Some(exe) => s::repinned("hermes-cli", exe),
        None => s::shipped("hermes-cli"),
    };
    c.loaded
        .manifest
        .capture
        .credential
        .as_mut()
        .expect("hermes names a credential")
        .env = "AMP_API_KEY".to_string();
    c.loaded
        .manifest
        .validate(Origin::Pinned)
        .expect("AMP_API_KEY is an admitted credential name");
    c
}

fn env_with(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("AMP_API_KEY".to_string(), value.to_string())])
}

fn forms() -> Vec<String> {
    use base64::Engine as _;
    vec![
        AMP_TOKEN.to_string(),
        AMP_TOKEN[..8].to_string(),
        AMP_TOKEN[AMP_TOKEN.len() - 8..].to_string(),
        base64::engine::general_purpose::STANDARD.encode(AMP_TOKEN),
    ]
}

#[test]
fn amp_child_secret_is_not_in_launch_debug_or_parent_env() {
    assert!(goalcup_protocol::capture_policy::CREDENTIAL_ENV.contains(&"AMP_API_KEY"));
    let l = s::Layout::new();
    let exe = s::fake_harness(&l.bin);
    let c = amp_keyed(Some(&exe));
    let profile = &c.loaded.manifest.capture;

    let mut parent = s::parent_env();
    parent.insert("AMP_API_KEY".to_string(), AMP_TOKEN.to_string());
    let original_parent_env = parent.clone();

    let secret = child_secret_for_harness(profile, &parent)
        .expect("a token of the admitted grammar")
        .expect("the profile names a credential");
    assert_eq!(secret.name(), "AMP_API_KEY");

    let pinned = s::pinned(&c, &exe);
    let proxy = s::proxy_url();
    let plan = build_capture_launch(
        &c,
        &s::vars(&l),
        &l.workspace,
        &s::launch(&l, b"hello", &pinned, &proxy, &parent),
    )
    .expect("the plan builds");

    for (surface, text) in [
        ("ChildSecret Debug", format!("{secret:?}")),
        ("ChildSecret pretty Debug", format!("{secret:#?}")),
        ("plan Debug", format!("{plan:?}")),
    ] {
        for form in forms() {
            assert!(!text.contains(&form), "{surface} shows a form of the token");
        }
    }
    assert!(!plan.env_set.contains_key("AMP_API_KEY"));
    assert!(plan.env_set.values().all(|v| v != AMP_TOKEN));
    assert_eq!(parent, original_parent_env, "the parent env is untouched");
}

#[test]
fn an_unusable_amp_token_is_refused_by_name_without_echo() {
    let c = amp_keyed(None);
    let profile = &c.loaded.manifest.capture;
    let long = "a".repeat(257);
    let cases: [(&str, BTreeMap<String, String>); 5] = [
        ("missing", BTreeMap::new()),
        ("short", env_with("amp-short-0123456")),
        ("dot", env_with(&format!("{AMP_TOKEN}.sig"))),
        ("space", env_with(&format!("{AMP_TOKEN} x"))),
        ("long", env_with(&long)),
    ];
    for (what, env) in cases {
        let err = match child_secret_for_harness(profile, &env) {
            Err(e) => e,
            Ok(_) => panic!("{what}: an unusable token was accepted"),
        };
        assert!(matches!(err, CliError::Refused(_)), "{what}: {err:?}");
        let said = err.to_string();
        assert!(said.contains("AMP_API_KEY"), "{what}: {said}");
        for value in env.values().filter(|v| v.len() > 8) {
            assert!(!said.contains(value.as_str()), "{what}: echoed the value");
        }
    }
    // The bounds themselves are admitted.
    for len in [20, 256] {
        let token = "A".repeat(len);
        assert!(
            child_secret_for_harness(profile, &env_with(&token))
                .unwrap()
                .is_some(),
            "{len} bytes"
        );
    }
}

#[test]
fn the_key_scan_finds_fragments_of_a_token_without_an_sk_prefix() {
    let matcher = FragmentMatcher::new(AMP_TOKEN.as_bytes()).expect("a usable token");
    let n = AMP_TOKEN.len();
    let planted = [
        ("full.txt", AMP_TOKEN.to_string()),
        ("prefix.txt", format!("x {} y", &AMP_TOKEN[..8])),
        ("suffix.txt", format!("x {} y", &AMP_TOKEN[n - 8..])),
        (
            "mask.txt",
            format!("Bearer {}...{}", &AMP_TOKEN[..6], &AMP_TOKEN[n - 4..]),
        ),
    ];
    for (name, body) in planted {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(name), body).unwrap();
        match scan_tree(&matcher, &[dir.path()], 1 << 20) {
            ScanOutcome::Hit { .. } => {}
            other => panic!("{name}: {other:?}"),
        }
    }
    let clean = tempfile::tempdir().unwrap();
    std::fs::write(
        clean.path().join("log.txt"),
        "Couldn't connect to the Amp server",
    )
    .unwrap();
    assert!(matches!(
        scan_tree(&matcher, &[clean.path()], 1 << 20),
        ScanOutcome::Clean
    ));
}
