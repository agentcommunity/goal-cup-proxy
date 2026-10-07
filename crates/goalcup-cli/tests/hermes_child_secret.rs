//! The provider key reaches the harness child and nothing else.
//!
//! Every key here is the synthetic one in `secret_support`. The real key file is never read by
//! any test, and nothing reads the real `HOME`.

mod m2_profile_support;
mod secret_support;

use goalcup_cli::CliError;
use goalcup_cli::launch::{
    ChildSecret, GOAL_ARG_MARKER, build_capture_launch, child_secret_for_harness, shell_line,
};
use goalcup_protocol::harness::CaptureSpec;
use m2_profile_support as s;
use secret_support::KEY;
use std::collections::BTreeMap;

fn env_with(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("DEEPSEEK_API_KEY".to_string(), value.to_string())])
}

fn hermes_profile() -> CaptureSpec {
    s::shipped("hermes-cli").loaded.manifest.capture
}

fn secret() -> ChildSecret {
    child_secret_for_harness(&hermes_profile(), &env_with(KEY))
        .expect("a valid key")
        .expect("the profile names a credential")
}

/// The forms a leak of the key could take on a printed surface.
fn forms() -> Vec<String> {
    let mut forms = vec![
        KEY.to_string(),
        KEY[..8].to_string(),
        KEY[KEY.len() - 8..].to_string(),
        KEY["sk-".len()..].to_string(),
    ];
    use base64::Engine as _;
    forms.push(base64::engine::general_purpose::STANDARD.encode(KEY));
    forms.push(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(KEY));
    forms
}

fn assert_none_of_the_forms(surface: &str, text: &str) {
    for form in forms() {
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
    let secret = secret();
    let shown = format!("{secret:?}");
    assert!(shown.contains("DEEPSEEK_API_KEY"), "{shown}");
    assert!(shown.contains("<redacted>"), "{shown}");
    assert_none_of_the_forms("ChildSecret Debug", &shown);
    let pretty = format!("{secret:#?}");
    assert_none_of_the_forms("ChildSecret pretty Debug", &pretty);
}

/// A profile that names no credential hands over nothing, even when the variable is set.
#[test]
fn a_profile_without_a_credential_hands_over_nothing() {
    let mut profile = hermes_profile();
    profile.credential = None;
    assert!(
        child_secret_for_harness(&profile, &env_with(KEY))
            .unwrap()
            .is_none()
    );
    let codex = s::shipped("codex-cli").loaded.manifest.capture;
    assert!(
        child_secret_for_harness(&codex, &env_with(KEY))
            .unwrap()
            .is_none()
    );
}

/// A missing or unusable key is a refusal that does not echo it.
#[test]
fn a_missing_or_unusable_key_is_refused_without_being_echoed() {
    let profile = hermes_profile();
    let long = format!("sk-{}", "a".repeat(400));
    let cases: [(&str, BTreeMap<String, String>); 8] = [
        ("missing", BTreeMap::new()),
        ("empty", env_with("")),
        ("short", env_with("sk-short")),
        ("space", env_with(&format!("{} {}", &KEY[..15], &KEY[16..]))),
        ("quote", env_with(&format!("\"{KEY}\""))),
        ("equals", env_with(&format!("{KEY}="))),
        ("newline", env_with(&format!("{KEY}\n"))),
        ("long", env_with(&long)),
    ];
    for (what, env) in cases {
        let err = match child_secret_for_harness(&profile, &env) {
            Err(e) => e,
            Ok(_) => panic!("{what}: a key that is not usable was accepted"),
        };
        assert!(matches!(err, CliError::Refused(_)), "{what}: {err:?}");
        assert_eq!(err.exit_code(), 1, "{what}");
        let said = err.to_string();
        assert!(said.contains("DEEPSEEK_API_KEY"), "{what}: {said}");
        for value in env.values().filter(|v| v.len() > 8) {
            assert!(!said.contains(value.as_str()), "{what}: echoed the value");
        }
        assert!(
            said.contains("Nothing was created") || said.contains("Nothing is launched"),
            "{what}: {said}"
        );
    }
}

/// A hostile goal is one `--query=<goal>` element wherever it would otherwise be read by
/// Hermes' raw argv scan or by argparse, and the plan, its previews and its handle never show it.
#[test]
fn hostile_goals_stay_one_goal_flag_element_and_the_key_is_in_no_plan_surface() {
    let l = s::Layout::new();
    let exe = s::fake_harness(&l.bin);
    let c = s::repinned("hermes-cli", &exe);
    let pinned = s::pinned(&c, &exe);
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
            &s::launch(&l, goal.as_bytes(), &pinned, &proxy, &parent),
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
            ("shell_line", shell_line(&plan)),
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
        let line = shell_line(&plan);
        assert!(line.contains("--query"), "{line}");
        assert!(
            !line.contains(GOAL_ARG_MARKER) || line.contains("withheld"),
            "{line}"
        );
    }
}

/// The hermes plan is a plain capture plan in every other respect: the goal flag changes only
/// where the goal sits, and a profile with no flag is unchanged (Codex and Claude end in `--`).
#[test]
fn the_other_shipped_profiles_still_place_the_goal_after_a_separator() {
    for id in ["codex-cli", "claude-code-cli"] {
        let l = s::Layout::new();
        let exe = s::fake_harness(&l.bin);
        let c = s::repinned(id, &exe);
        let pinned = s::pinned(&c, &exe);
        let proxy = s::proxy_url();
        let parent = s::parent_env();
        let plan = build_capture_launch(
            &c,
            &s::vars(&l),
            &l.workspace,
            &s::launch(&l, b"hello", &pinned, &proxy, &parent),
        )
        .unwrap();
        assert_eq!(plan.argv[plan.argv.len() - 2], "--", "{id}");
        assert_eq!(plan.argv.last().unwrap(), "hello", "{id}");
        assert_eq!(plan.capture.unwrap().goal_prefix, 0, "{id}");
    }
}
