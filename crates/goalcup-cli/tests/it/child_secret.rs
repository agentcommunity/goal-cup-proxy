//! The provider key reaches the harness child and nothing else, for every credential name a
//! connector may declare (`capture_policy::CREDENTIAL_ENV`).
//!
//! The subject is the keyed fixture connector (`tests/fixtures/connectors/keyed-per-run`: a goal
//! flag, a per-run home, `stderr_line` identity), with its credential renamed to each admitted
//! name in turn. Every key here is synthetic: `secret_support::KEY` (the `sk-` shape) and
//! [`PLAIN_TOKEN`] (no provider prefix). The real key file is never read by any test, and
//! nothing reads the real `HOME`.

use crate::common::m2_profile_support;
use crate::common::secret_support;
use crate::common::support;

use goalcup_cli::CliError;
use goalcup_cli::connector::HarnessConnector;
use goalcup_cli::key_scan::{FragmentMatcher, ScanOutcome, scan_tree};
use goalcup_cli::launch::{ChildSecret, build_capture_launch, child_secret_for_harness};
use goalcup_protocol::capture_policy::CREDENTIAL_ENV;
use goalcup_protocol::harness::Origin;
use m2_profile_support as s;
use secret_support::KEY;
use std::collections::BTreeMap;

/// An invented token of the admitted grammar (20 to 256 bytes of `[A-Za-z0-9_-]`) with no `sk-`
/// prefix, so the key scan is shown to work for a shape `KEY` does not cover. Never a real
/// credential.
const PLAIN_TOKEN: &str = "tok-synthetic_7Qx2Lm9Rt4Vw8Yz1Bc5Df3Gh";

/// The keyed fixture connector, its credential named `name`; it still validates.
fn keyed(name: &str) -> HarnessConnector {
    let mut c = support::fixture_connector("keyed-per-run");
    c.loaded
        .manifest
        .capture
        .credential
        .as_mut()
        .expect("the keyed fixture names a credential")
        .env = name.to_string();
    c.loaded
        .manifest
        .validate(Origin::Pinned)
        .unwrap_or_else(|e| panic!("{name} is an admitted credential name: {e}"));
    c
}

fn env_with(name: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(name.to_string(), value.to_string())])
}

fn secret(name: &str, value: &str) -> ChildSecret {
    child_secret_for_harness(&keyed(name).loaded.manifest.capture, &env_with(name, value))
        .expect("a valid key")
        .expect("the profile names a credential")
}

/// The forms a leak of `key` could take on a printed surface.
fn forms(key: &str) -> Vec<String> {
    let mut forms = vec![
        key.to_string(),
        key[..8].to_string(),
        key[key.len() - 8..].to_string(),
    ];
    if let Some(body) = key.strip_prefix("sk-") {
        forms.push(body.to_string());
    }
    use base64::Engine as _;
    forms.push(base64::engine::general_purpose::STANDARD.encode(key));
    forms.push(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key));
    forms
}

fn assert_none_of_the_forms(surface: &str, text: &str) {
    for form in forms(KEY).into_iter().chain(forms(PLAIN_TOKEN)) {
        assert!(
            !text.contains(&form),
            "{surface} shows a form of the key: {form}"
        );
    }
}

/// `ChildSecret` prints its name and never its value, and is neither `Clone` nor
/// `Serialize`.
#[test]
fn a_child_secret_prints_its_name_and_never_its_value() {
    for name in CREDENTIAL_ENV {
        for value in [KEY, PLAIN_TOKEN] {
            let secret = secret(name, value);
            assert_eq!(secret.name(), *name);
            let shown = format!("{secret:?}");
            assert!(shown.contains(name), "{shown}");
            assert!(shown.contains("<redacted>"), "{shown}");
            assert_none_of_the_forms("ChildSecret Debug", &shown);
            let pretty = format!("{secret:#?}");
            assert_none_of_the_forms("ChildSecret pretty Debug", &pretty);
        }
    }
}

/// A profile that names no credential hands over nothing, even when the variable is set.
#[test]
fn a_profile_without_a_credential_hands_over_nothing() {
    for name in CREDENTIAL_ENV {
        let mut profile = keyed(name).loaded.manifest.capture;
        profile.credential = None;
        assert!(
            child_secret_for_harness(&profile, &env_with(name, KEY))
                .unwrap()
                .is_none()
        );
        let chat = support::fixture_connector("chat-shared-home")
            .loaded
            .manifest
            .capture;
        assert!(
            child_secret_for_harness(&chat, &env_with(name, KEY))
                .unwrap()
                .is_none()
        );
    }
}

/// A missing or unusable key is a refusal that names the variable and does not echo the value;
/// the grammar's bounds themselves are admitted.
#[test]
fn a_missing_or_unusable_key_is_refused_without_being_echoed() {
    for name in CREDENTIAL_ENV {
        let profile = keyed(name).loaded.manifest.capture;
        let long = format!("sk-{}", "a".repeat(400));
        let cases: [(&str, BTreeMap<String, String>); 11] = [
            ("missing", BTreeMap::new()),
            ("empty", env_with(name, "")),
            ("short", env_with(name, "sk-short")),
            (
                "space",
                env_with(name, &format!("{} {}", &KEY[..15], &KEY[16..])),
            ),
            ("quote", env_with(name, &format!("\"{KEY}\""))),
            ("equals", env_with(name, &format!("{KEY}="))),
            ("newline", env_with(name, &format!("{KEY}\n"))),
            ("long", env_with(name, &long)),
            ("one past the bound", env_with(name, &"a".repeat(257))),
            ("dot", env_with(name, &format!("{PLAIN_TOKEN}.sig"))),
            ("19 bytes", env_with(name, &"A".repeat(19))),
        ];
        for (what, env) in cases {
            let err = match child_secret_for_harness(&profile, &env) {
                Err(e) => e,
                Ok(_) => panic!("{name} {what}: a key that is not usable was accepted"),
            };
            assert!(matches!(err, CliError::Refused(_)), "{what}: {err:?}");
            assert_eq!(err.exit_code(), 1, "{what}");
            let said = err.to_string();
            assert!(said.contains(name), "{what}: {said}");
            for value in env.values().filter(|v| v.len() > 8) {
                assert!(!said.contains(value.as_str()), "{what}: echoed the value");
            }
            assert!(
                said.contains("Nothing was created") || said.contains("Nothing is launched"),
                "{what}: {said}"
            );
        }
        for len in [20, 256] {
            let token = "A".repeat(len);
            assert!(
                child_secret_for_harness(&profile, &env_with(name, &token))
                    .unwrap()
                    .is_some(),
                "{name}: {len} bytes"
            );
        }
    }
}

/// The key, in the tracker's own environment, is in no surface of the plan, and the parent
/// environment is left exactly as it was.
#[test]
fn the_key_is_in_no_plan_surface_and_the_parent_env_is_untouched() {
    for name in CREDENTIAL_ENV {
        for value in [KEY, PLAIN_TOKEN] {
            let l = s::Layout::new();
            let exe = s::fake_harness(&l.bin);
            let c = keyed(name);
            let mut parent = s::parent_env();
            parent.insert(name.to_string(), value.to_string());
            let original = parent.clone();
            let resolved = s::resolved(&exe);
            let proxy = s::proxy_url();
            let plan = build_capture_launch(
                &c,
                &s::vars(&l),
                &l.workspace,
                &s::launch(&l, b"hello", &resolved, &proxy, &parent),
            )
            .expect("the plan builds");
            for (surface, text) in [
                ("plan Debug", format!("{plan:?}")),
                ("plan pretty Debug", format!("{plan:#?}")),
            ] {
                assert_none_of_the_forms(surface, &text);
            }
            assert!(!plan.env_set.contains_key(*name), "{name}");
            assert!(plan.env_set.values().all(|v| v != value));
            assert_eq!(parent, original, "the parent env is untouched");
        }
    }
}

/// The key scan finds the fragments of a token with no `sk-` prefix, and nothing in clean text.
#[test]
fn the_key_scan_finds_fragments_of_a_token_without_an_sk_prefix() {
    let matcher = FragmentMatcher::new(PLAIN_TOKEN.as_bytes()).expect("a usable token");
    let n = PLAIN_TOKEN.len();
    let planted = [
        ("full.txt", PLAIN_TOKEN.to_string()),
        ("prefix.txt", format!("x {} y", &PLAIN_TOKEN[..8])),
        ("suffix.txt", format!("x {} y", &PLAIN_TOKEN[n - 8..])),
        (
            "mask.txt",
            format!("Bearer {}...{}", &PLAIN_TOKEN[..6], &PLAIN_TOKEN[n - 4..]),
        ),
    ];
    for (file, body) in planted {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(file), body).unwrap();
        match scan_tree(&matcher, &[dir.path()], 1 << 20) {
            ScanOutcome::Hit { .. } => {}
            other => panic!("{file}: {other:?}"),
        }
    }
    let clean = tempfile::tempdir().unwrap();
    std::fs::write(
        clean.path().join("log.txt"),
        "Couldn't connect to the server",
    )
    .unwrap();
    assert!(matches!(
        scan_tree(&matcher, &[clean.path()], 1 << 20),
        ScanOutcome::Clean
    ));
}

/// A hostile goal is one `--query=<goal>` element wherever it would otherwise be read by the
/// harness's raw argv scan (its `argv_guard`) or by its argument parser, and the plan, its
/// previews and its handle never show it.
#[test]
fn hostile_goals_stay_one_goal_flag_element_and_the_key_is_in_no_plan_surface() {
    let l = s::Layout::new();
    let exe = s::fake_harness(&l.bin);
    let c = keyed("DEEPSEEK_API_KEY");
    let resolved = s::resolved(&exe);
    let proxy = s::proxy_url();
    // The key is in the tracker's own environment, as the loader leaves it. The capture plan
    // reads only the identity names from there.
    let mut parent = s::parent_env();
    parent.insert("DEEPSEEK_API_KEY".to_string(), KEY.to_string());
    let profile = &c.loaded.manifest.capture;
    for goal in [
        "--profile=x",
        "-p x",
        "--tui",
        "-h",
        "a=b",
        "-leading dash",
        "line one\n--tui\nline three",
        "--query=twice",
        "--",
    ] {
        let plan = build_capture_launch(
            &c,
            &s::vars(&l),
            &l.workspace,
            &s::launch(&l, goal.as_bytes(), &resolved, &proxy, &parent),
        )
        .unwrap_or_else(|e| panic!("{goal:?}: {e}"));
        assert_eq!(plan.argv.len(), profile.args.len() + 1, "{goal:?}");
        assert_eq!(
            &plan.argv[..profile.args.len()],
            profile.args.as_slice(),
            "{goal:?}: the profile's args come first and unchanged"
        );
        assert_eq!(
            plan.argv.last().unwrap(),
            &format!("--query={goal}"),
            "{goal:?}: the goal is the value of the one flag element"
        );
        assert!(
            !plan.argv.iter().any(|a| a == "--"),
            "{goal:?}: this recipe has no `--`"
        );
        let capture = plan.capture.unwrap();
        assert_eq!(capture.goal_index, plan.argv.len() - 1);
        assert_eq!(capture.goal_prefix, "--query=".len());

        for (surface, text) in [
            ("Debug", format!("{plan:?}")),
            ("pretty Debug", format!("{plan:#?}")),
        ] {
            // A two-byte goal such as `-h` occurs inside ordinary path text (`run-home`), so the
            // whole goal is searched for only when it is long enough to mean something, and the
            // element the goal rides in is searched for always.
            assert!(
                !text.contains(&format!("--query={goal}")),
                "{goal:?}: {surface} shows the goal element"
            );
            if goal.len() >= 6 {
                assert!(!text.contains(goal), "{goal:?}: {surface} shows the goal");
            }
            assert_none_of_the_forms(surface, &text);
        }
        assert!(
            !plan.env_set.contains_key("DEEPSEEK_API_KEY"),
            "the key is never part of the plan's environment"
        );
        assert!(plan.env_set.values().all(|v| v != KEY));
        let debugged = format!("{plan:?}");
        assert!(
            debugged.contains(&format!("--query=<goal: {} bytes>", goal.len())),
            "{debugged}"
        );
    }
}
