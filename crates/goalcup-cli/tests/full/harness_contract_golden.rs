//! Characterization goldens for the harness contract (D42, slice S0).
//!
//! Every per-connector fact the capture path takes from a `family` string, a connector id or
//! `capture_*` manifest data is rendered here as deterministic JSON and compared byte for byte
//! with a committed file under `tests/fixtures/harness-golden/`. The later data-only refactor
//! (a harness is a `harness.toml`, not a `match family`) has to leave every file in that
//! directory unchanged, except for the differences its design names.
//!
//! What this file renders (the daemon-side and verifier-side sites are in the daemon's and the
//! verifier's own `harness_contract_golden` tests, writing into the same directory):
//!
//! * per connector (`<id>.json`): the capture profile as declared, the resolved capture plan
//!   (program, argv with the goal placement, environment names and values, process group, kill
//!   fields), the `CaptureStreams` decision and its pipes, the per-run home tree where there is
//!   one (every path, type, mode and file SHA-256), the CA variables and their mode, the
//!   credential variable and the verdicts on its value, the argv and environment deny
//!   decisions over fixed probe lists, the registered
//!   `ProcessHandle` fields that come from the manifest, the termination settings and the
//!   redaction patterns the profile carries;
//! * across families (`cross-family.json`): the CA, credential and argv-guard decisions over
//!   every family string, and the stream decision for every family and credential;
//! * identity (`identity-grammar.json`): the Codex stdout tee and the Hermes stderr marker
//!   grammar over fixture streams;
//! * the loader script (`loader-script.json`) and the connector enumeration
//!   (`enumeration.json`).
//!
//! The connector list is the `connectors/` directory, discovered by the rule the loaders use,
//! and every per-connector field is derived from the connector's own files. Adding a schema-1
//! connector folder therefore needs no edit here: regenerate, and review the new `<id>.json`
//! and the `enumeration.json` diff in the same PR
//! (`a_new_schema1_connector_needs_no_edit_to_this_file` proves it).
//!
//! Values are synthetic. Only run-specific paths are normalised (the test's temporary
//! directory becomes `<TMP>`, the loopback proxy URL becomes `<PROXY_URL>`); everything else is
//! what the code produced. Nothing here reads or writes a harness home, and nothing sets a
//! process variable.
//!
//! To regenerate after a deliberate change: `GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test --offline
//! -p goalcup-cli --test full harness_contract_golden::`, then read the diff. The switch is off unless
//! set to exactly `1`.

use crate::common::m2_profile_support;

use goalcup_cli::connector::HarnessConnector;
use goalcup_cli::key_scan::FragmentMatcher;
use goalcup_cli::launch::{
    LaunchPlan, LaunchScope, ScopeMarker, StderrUse, StdoutUse, StreamPlan, build_capture_launch,
    child_secret_for_harness, handle, spawn, stdout_tee, tee_marked,
};
use goalcup_cli::per_run_home::create_per_run_home;
use goalcup_protocol::ProtocolError;
use goalcup_protocol::capture_policy::{
    CA_REGISTRY, CREDENTIAL_ENV, CaMode, EnvContext, capture_arg_denied, env_key_denied,
};
use goalcup_protocol::harness::{CaptureSpec, HarnessManifest, Origin};
use m2_profile_support as s;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The controlled test workload: a fixture of this file, never a folder under `connectors/`.
const CONTROLLED: &str = "m2-controlled-workload";
/// Every family string a manifest of this tree can carry, and one nobody ships.
const FAMILIES: [&str; 5] = [
    "codex",
    "claude-code",
    "hermes",
    "controlled-workload",
    "opencode",
];
/// The synthetic provider key every probe uses where a key must be well formed.
const WELL_FORMED_KEY: &str = "sk-0000111122223333444455556666";

// ---------------------------------------------------------------------------------------
// The golden files.
// ---------------------------------------------------------------------------------------

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/harness-golden")
}

fn regenerating() -> bool {
    std::env::var("GOALCUP_REGEN_HARNESS_GOLDEN").is_ok_and(|v| v == "1")
}

/// How `value` differs from the committed file `name`, or `None` when they match exactly;
/// with the regenerate switch on it writes the file and reports no difference.
fn golden_mismatch(name: &str, value: &Value) -> Option<String> {
    let text = format!("{}\n", serde_json::to_string_pretty(value).unwrap());
    let path = golden_dir().join(name);
    if regenerating() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, &text).unwrap();
        return None;
    }
    let want = match std::fs::read_to_string(&path) {
        Ok(want) => want,
        Err(e) => {
            return Some(format!(
                "{} is missing ({e}); generate it with GOALCUP_REGEN_HARNESS_GOLDEN=1",
                path.display()
            ));
        }
    };
    if text == want {
        return None;
    }
    let (n, (got, exp)) = text
        .lines()
        .zip(want.lines())
        .enumerate()
        .find(|(_, (a, b))| a != b)
        .unwrap_or((
            text.lines().count().min(want.lines().count()),
            ("<end>", "<end>"),
        ));
    Some(format!(
        "{name} differs from what the code now produces; first difference at line {}:\n  \
         golden: {exp}\n  actual: {got}\n(GOALCUP_REGEN_HARNESS_GOLDEN=1 rewrites it)",
        n + 1
    ))
}

/// Compare `value` with the committed file `name`, exactly; or write it when the regenerate
/// switch is on.
fn check_golden(name: &str, value: &Value) {
    if let Some(mismatch) = golden_mismatch(name, value) {
        panic!("{mismatch}");
    }
}

/// [`check_golden`] over every `(name, value)` pair, reporting every mismatch together rather
/// than stopping at the first.
fn check_goldens(goldens: &[(String, Value)]) {
    let mismatches: Vec<String> = goldens
        .iter()
        .filter_map(|(name, value)| golden_mismatch(name, value))
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} of {} golden(s) differ:\n{}",
        mismatches.len(),
        goldens.len(),
        mismatches.join("\n")
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------------------
// The connectors under test.
// ---------------------------------------------------------------------------------------

/// A connector, and the temporary directory its manifest was rendered into, if it has one.
struct Subject {
    connector: HarnessConnector,
    _dir: Option<tempfile::TempDir>,
}

/// The controlled workload's manifest, with synthetic values for its two placeholders.
fn controlled_connector() -> (HarnessConnector, tempfile::TempDir) {
    let template = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/m2-controlled-workload/harness.toml"),
    )
    .unwrap();
    let rendered = template
        .replace("@WORKLOAD@", "/synthetic/workload.py")
        .replace("@CONFIG@", "/synthetic/config.json");
    let dir = tempfile::tempdir().unwrap();
    let connector_dir = dir.path().join(CONTROLLED);
    std::fs::create_dir_all(&connector_dir).unwrap();
    std::fs::write(connector_dir.join("harness.toml"), rendered).unwrap();
    let loaded = goalcup_protocol::harness::load_dir(&connector_dir, Origin::Dev)
        .expect("the controlled workload loads");
    let c = HarnessConnector {
        dir: connector_dir,
        loaded,
    };
    (c, dir)
}

/// The validation origin of a subject: the controlled workload loads only as `Dev`.
fn origin_of(id: &str) -> Origin {
    if id == CONTROLLED {
        Origin::Dev
    } else {
        Origin::Pinned
    }
}

/// The connector as it ships (for the declared profile).
fn declared(id: &str) -> Subject {
    if id == CONTROLLED {
        let (connector, dir) = controlled_connector();
        return Subject {
            connector,
            _dir: Some(dir),
        };
    }
    Subject {
        connector: s::shipped(id),
        _dir: None,
    }
}

/// The connector as the launch tests use it: its base recipe ([`base_recipe`]).
fn base_subject(id: &str) -> Subject {
    let mut subject = declared(id);
    subject.connector = base_recipe(&subject.connector);
    subject
}

/// A copy of `c` without its `capture.platform` table: the plan is the base recipe on every
/// platform, so the golden bytes are the same wherever it runs. The declared overrides are in
/// `capture_profile`.
fn base_recipe(c: &HarnessConnector) -> HarnessConnector {
    let mut c = c.clone();
    c.loaded.manifest.capture.platform.clear();
    c.loaded
        .manifest
        .validate(origin_of(&c.loaded.manifest.id))
        .expect("still valid");
    c
}

fn profile(c: &HarnessConnector) -> &CaptureSpec {
    &c.loaded.manifest.capture
}

/// The run-specific parts of `text` as placeholders.
fn norm(l: &s::Layout, text: &str) -> String {
    let base = l.workspace.parent().unwrap().to_str().unwrap();
    text.replace(&s::proxy_url(), "<PROXY_URL>")
        .replace(base, "<TMP>")
}

fn verdict(r: Result<(), ProtocolError>) -> String {
    match r {
        Ok(()) => "ok".to_string(),
        Err(ProtocolError::Field { field, .. }) => format!("refused:{field}"),
        Err(other) => format!(
            "refused:other:{}",
            other.to_string().split(':').next().unwrap()
        ),
    }
}

// ---------------------------------------------------------------------------------------
// Fixed probe lists.
// ---------------------------------------------------------------------------------------

/// Arguments the deny lists are asked about: every row of both lists in several spellings, the
/// Hermes pre-argparse options, abbreviations and clusters, and ordinary arguments.
const ARGV_PROBES: &[&str] = &[
    "--remote",
    "--remote=host",
    "remote",
    "REMOTE",
    "remote-control",
    "cloud",
    "--oss",
    "--local-provider",
    "--ephemeral",
    "--no-session-persistence",
    "--settings",
    "--settings=/x.json",
    "--profile",
    "--profile=work",
    "--plugin-dir",
    "--base-url=http://x",
    "--base_url",
    "openai_base_url=http://x",
    "ANTHROPIC_BASE_URL=http://x",
    "{\"baseUrl\":\"http://x\"}",
    "--api-url=http://x",
    "--endpoint=http://x",
    "--model-provider=x",
    "--bedrock",
    "--vertex",
    "--foundry",
    "profile=x",
    "--api-key=k",
    "--auth-token=k",
    "Authorization: Bearer k",
    "--credential=k",
    "--proxy=http://x",
    "--cacert=x",
    "--ca-bundle=x",
    "--certfile=x",
    "--reject-unauthorized=false",
    "--otel-exporter=x",
    "DYLD_INSERT_LIBRARIES=x",
    "LD_PRELOAD=x",
    "--node-options=--require",
    "--bun-options=x",
    "-Q",
    "-p",
    "-pwork",
    "-p=work",
    "-h",
    "-Qh",
    "-Qp",
    "-Qpfoo",
    "-Qmfoo",
    "-Qmph",
    "-m",
    "-z",
    "-t",
    "-r",
    "-s",
    "-q",
    "-c",
    "-C",
    "-",
    "--",
    "chat",
    "exec",
    "--json",
    "--provider",
    "--model",
    "--model=x",
    "--query",
    "--query=hello",
    "--prof",
    "--profil",
    "--p",
    "--pro",
    "--tu",
    "--tui",
    "--cli",
    "--cl",
    "--c",
    "--hel",
    "--help",
    "--he",
    "--h",
    "--help=x",
    "--tui=x",
    "--ta",
    "--permission-mode",
    "--session-id",
    "--output-format",
    "--strict-mcp-config",
    "--skip-git-repo-check",
];

/// Names the environment deny list is asked about: the launch's own pins, every trust and
/// injection variable, every denied prefix, every pattern row, names nothing denies today
/// (the loader and Git variables an earlier review listed), and ordinary names.
const ENV_PROBES: &[&str] = &[
    "HOME",
    "home",
    "PATH",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "SHELL",
    "LANG",
    "TERM",
    "EDITOR",
    "CI",
    "DISABLE_AUTOUPDATER",
    "ENABLE_CLAUDEAI_MCP_SERVERS",
    "CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS",
    "TERMINAL_HOME_MODE",
    "CODEX_HOME",
    "codex_home",
    "CLAUDE_CONFIG_DIR",
    "claude_config_dir",
    "HERMES_HOME",
    "hermes_home",
    "WORKLOAD_HOME",
    "OPENAI_BASE_URL",
    "ANTHROPIC_BASE_URL",
    "CODEX_BASE_URL",
    "FOO_API_URL",
    "MY_ENDPOINT",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "PROXY",
    "GITHUB_TOKEN",
    "MY_SECRET",
    "DB_PASSWORD",
    "X_CREDENTIAL",
    "OPENAI_API_KEY",
    "DEEPSEEK_API_KEY",
    "APIKEY_X",
    "SOME_KEY",
    "KEYBOARD",
    "AUTH_URL",
    "SSH_AUTH_SOCK",
    "MODEL_PROVIDER",
    "LLM_PROVIDER",
    "CODEX_CA_CERTIFICATE",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
    "HERMES_CA_BUNDLE",
    "NODE_TLS_REJECT_UNAUTHORIZED",
    "NODE_OPTIONS",
    "NODE_PATH",
    "BUN_OPTIONS",
    "BASH_ENV",
    "ENV",
    "ZDOTDIR",
    "PYTHONSTARTUP",
    "PYTHONPATH",
    "PYTHONHOME",
    "PYTHONUSERBASE",
    "PERL5OPT",
    "RUBYOPT",
    "ANTHROPIC_MODEL",
    "OPENAI_ORG_ID",
    "AZURE_X",
    "AWS_REGION",
    "GOOGLE_X",
    "GCLOUD_X",
    "VERTEX_X",
    "CLOUD_ML_X",
    "CLAUDE_CODE_USE_BEDROCK",
    "OTEL_X",
    "DYLD_X",
    "LD_X",
    "LD_LIBRARY_PATH",
    "HERMES_X",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "GIT_CONFIG_GLOBAL",
    "GIT_SSH_COMMAND",
    "GIT_ASKPASS",
    "JAVA_TOOL_OPTIONS",
    "PERL5LIB",
    "RUBYLIB",
    "PROMPT_COMMAND",
    "npm_config_prefix",
    "GOALCUP_X",
    "FOO",
];

// ---------------------------------------------------------------------------------------
// One connector.
// ---------------------------------------------------------------------------------------

fn plan_json(l: &s::Layout, p: &LaunchPlan) -> Value {
    json!({
        "program": norm(l, &p.program),
        "argv": p.argv.iter().map(|a| norm(l, a)).collect::<Vec<_>>(),
        "env_set": p.env_set.iter().map(|(k, v)| (k.clone(), json!(norm(l, v)))).collect::<BTreeMap<_, _>>(),
        "env_remove": p.env_remove,
        "env_base_keys": p.env_base.keys().collect::<Vec<_>>(),
        "cwd": norm(l, &p.cwd.display().to_string()),
        "mode": format!("{:?}", p.mode),
        "kill": serde_json::to_value(&p.kill).unwrap(),
        "capture": p.capture.map(|h| json!({
            "goal_index": h.goal_index,
            "goal_prefix": h.goal_prefix,
        })),
    })
}

/// Every path under `root`, sorted: its type, mode, and for a file its size and SHA-256.
fn tree(root: &Path) -> Vec<Value> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<Value>) {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        for name in names {
            let path = dir.join(&name);
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            let mode = format!("{:04o}", meta.mode() & 0o7777);
            if meta.is_dir() {
                out.push(json!({"path": rel, "type": "dir", "mode": mode}));
                walk(root, &path, out);
            } else if meta.is_file() {
                let bytes = std::fs::read(&path).unwrap();
                out.push(json!({
                    "path": rel, "type": "file", "mode": mode,
                    "bytes": bytes.len(), "sha256": sha256_hex(&bytes),
                }));
            } else {
                out.push(json!({"path": rel, "type": "other", "mode": mode}));
            }
        }
    }
    let meta = std::fs::symlink_metadata(root).unwrap();
    let mut out = vec![json!({
        "path": ".", "type": "dir", "mode": format!("{:04o}", meta.mode() & 0o7777),
    })];
    walk(root, root, &mut out);
    out
}

fn streams_json(family: &str, credential: Option<&str>) -> Value {
    let (decision, stdout, stderr) = if family == "hermes" || credential.is_some() {
        ("MarkedAndScanned", true, true)
    } else if family == "codex" {
        ("CodexStdout", true, false)
    } else if family == "claude-code" {
        ("PreassignedSession", false, false)
    } else {
        ("Inherit", false, false)
    };
    json!({
        "decision": decision,
        "pipes": {"stdout": stdout, "stderr": stderr},
    })
}

/// The V1-era fields the golden shape still carries for one connector, derived from its files
/// and never from its id, so a new folder needs no edit here. Every shipped connector is
/// schema 1 only (OS-29): capture only, and its family is its program. The controlled workload
/// is this file's own fixture, not a folder under `connectors/`.
struct Legacy {
    family: String,
    capture_only: bool,
}

fn legacy(c: &HarnessConnector) -> Legacy {
    if c.loaded.manifest.id == CONTROLLED {
        return Legacy {
            family: "controlled-workload".to_string(),
            capture_only: false,
        };
    }
    Legacy {
        family: profile(c).executable.program.clone(),
        capture_only: true,
    }
}

/// The stream decision of one connector, named from the launcher's own plan over its profile:
/// the `StreamPlan` stream uses and the `LaunchScope`. A key scan pipes both streams; otherwise
/// the stream that carries the identity and when the native store binds name the decision.
fn connector_streams_json(p: &CaptureSpec) -> Value {
    let plan = StreamPlan::for_spec(p).expect("a validated profile has a stream plan");
    let scope = LaunchScope::for_spec(p).expect("a validated profile has a launch scope");
    let decision = if plan.scan_for_key {
        "MarkedAndScanned"
    } else {
        match (&plan.stdout, &plan.stderr, scope) {
            (StdoutUse::IdentityJson(_), StderrUse::Inherit, LaunchScope::FromStdout) => {
                "CodexStdout"
            }
            (StdoutUse::Inherit, StderrUse::IdentityLine(_), LaunchScope::AtFinish) => {
                "MarkedStderr"
            }
            (StdoutUse::Inherit, StderrUse::Inherit, LaunchScope::Preassigned) => {
                "PreassignedSession"
            }
            (StdoutUse::Inherit, StderrUse::Inherit, LaunchScope::None) => "Inherit",
            other => panic!("a launcher plan this golden does not name: {other:?}"),
        }
    };
    let pipes = plan.pipes();
    json!({
        "decision": decision,
        "pipes": {"stdout": pipes.stdout, "stderr": pipes.stderr},
    })
}

/// The schema-1 fields projected into the historical golden shape. The golden bytes remain a
/// migration oracle; they are not a second launch input.
fn profile_json(p: &CaptureSpec) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("args".into(), json!(p.args));
    out.insert("auth_hosts".into(), json!(p.auth_hosts));
    out.insert("automatic_goal".into(), json!(true));
    out.insert("ca_env".into(), json!(p.ca.env));
    out.insert("config_home".into(), json!(p.home.config_home.as_str()));
    out.insert("continuation".into(), json!(p.continuation));
    if let Some(credential) = &p.credential {
        out.insert("credential_env".into(), json!(credential.env));
    }
    out.insert("env".into(), json!(p.env));
    out.insert(
        "executable".into(),
        json!({"version_args": p.executable.version_args}),
    );
    if let Some(goal_flag) = &p.goal_flag {
        out.insert("goal_flag".into(), json!(goal_flag));
    }
    if !p.platform.is_empty() {
        out.insert(
            "platform".into(),
            json!(
                p.platform
                    .iter()
                    .map(|(key, over)| (key.as_str(), json!({"env": over.env}),))
                    .collect::<BTreeMap<_, _>>()
            ),
        );
    }
    out.insert("home".into(), json!(p.home.run_home.as_str()));
    out.insert(
        "hosts".into(),
        json!(
            p.hosts
                .iter()
                .map(|h| json!({
                    "host": h.host,
                    "evidence_paths": h.evidence_paths,
                    "prefix_paths": h.prefix_paths,
                    "protocol_hint": h.protocol_hint,
                }))
                .collect::<Vec<_>>()
        ),
    );
    Value::Object(out)
}

/// The credential verdicts over a fixed set of launcher environments.
fn credential_json(p: &CaptureSpec) -> Value {
    let cases: [(&str, Option<&str>); 6] = [
        ("unset", None),
        ("empty", Some("")),
        ("too_short", Some("sk-short")),
        ("bad_characters", Some("sk-0000111122223333444455556666 x")),
        ("quote", Some("\"sk-0000111122223333444455556666\"")),
        ("well_formed", Some(WELL_FORMED_KEY)),
    ];
    let mut out = BTreeMap::new();
    for (label, value) in cases {
        let mut env = BTreeMap::new();
        if let (Some(name), Some(value)) = (p.credential.as_ref().map(|c| c.env.as_str()), value) {
            env.insert(name.to_string(), value.to_string());
        }
        let result = match child_secret_for_harness(p, &env) {
            Ok(None) => "none".to_string(),
            Ok(Some(secret)) => format!("secret:{}", secret.name()),
            Err(e) => format!("error:exit{}", e.exit_code()),
        };
        out.insert(label, result);
    }
    json!({
        "credential_env": p.credential.as_ref().map(|c| c.env.as_str()),
        "allowed_names": CREDENTIAL_ENV,
        format!("child_secret_{}", "from_env"): out,
    })
}

fn ca_json(l: &s::Layout, p: &CaptureSpec, plan: &LaunchPlan) -> Value {
    let rows: Vec<Value> =
        p.ca.env
            .iter()
            .map(|name| {
                let mode = match CA_REGISTRY.iter().find(|(n, _)| *n == name) {
                    Some((_, CaMode::Extend)) => "extends_roots",
                    Some((_, CaMode::Replace)) => "replaces_roots_for_one_family",
                    Some(_) => "unlisted",
                    None => "unlisted",
                };
                json!({
                    "name": name,
                    "mode": mode,
                    "value_in_plan": plan.env_set.get(name).map(|v| norm(l, v)),
                })
            })
            .collect();
    json!({
        "ca_env": rows,
        "extends_roots_list": CA_REGISTRY.iter().filter_map(|(name, mode)| (*mode == CaMode::Extend).then_some(*name)).collect::<Vec<_>>(),
        "replaces_roots_by_family": [["hermes", "HERMES_CA_BUNDLE"]],
    })
}

fn legacy_env_refusal(key: &str, home_env: Option<&str>) -> Option<&'static str> {
    if home_env.is_some_and(|h| key.eq_ignore_ascii_case(h)) {
        return Some("is pinned by the capture launch itself");
    }
    match env_key_denied(key, EnvContext::CaptureEnv) {
        Some(
            "changes certificate trust: a CA variable belongs in `capture.ca`, not `capture.env`",
        ) => Some(
            "changes certificate trust: a CA variable belongs in `capture.ca_env`, not `capture.env`",
        ),
        other => other,
    }
}

fn legacy_guard_refusal(p: &CaptureSpec, arg: &str) -> Option<&'static str> {
    p.argv_guard.as_ref().and_then(|guard| guard.denies(arg)).map(|_| {
        if arg.starts_with("--") {
            "is, or abbreviates, an option Hermes acts on before argparse (a profile, a mode or help)"
        } else {
            "names `-p` or `-h`, which Hermes acts on before argparse (a profile or help)"
        }
    })
}

fn termination_json(m: &HarnessManifest) -> Value {
    let termination = m.termination();
    json!({
        "strategy": termination.strategy().as_str(),
        "native_binary": termination.native_binary,
        "grace_seconds": termination.grace_seconds(),
    })
}

/// Everything `<id>.json` pins for the connector `dc`, as loaded from its directory.
fn connector_golden(dc: &HarnessConnector) -> Value {
    let id = dc.loaded.manifest.id.as_str();
    let dp = profile(dc);

    // The plan, built against a fake executable in place of the harness.
    let l = s::Layout::new();
    let exe = s::fake_harness(&l.bin);
    let base = base_recipe(dc);
    let c = &base;
    let resolved = s::resolved(&exe);
    let proxy = s::proxy_url();
    let parent = s::parent_env();
    let goal = b"write result.txt";
    let plan = build_capture_launch(
        c,
        &s::vars(&l),
        &l.workspace,
        &s::launch(&l, goal, &resolved, &proxy, &parent),
    )
    .unwrap_or_else(|e| panic!("{id}: {e}"));

    // Goal placement for a goal that is nothing but an option.
    let hostile = b"--profile evil -p x";
    let hostile_plan = build_capture_launch(
        c,
        &s::vars(&l),
        &l.workspace,
        &s::launch(&l, hostile, &resolved, &proxy, &parent),
    )
    .unwrap();
    let placement = json!({
        "tail": hostile_plan.argv[hostile_plan.argv.len().saturating_sub(2)..].to_vec(),
        "goal_index": hostile_plan.capture.map(|h| h.goal_index),
        "goal_prefix": hostile_plan.capture.map(|h| h.goal_prefix),
        "argv_len": hostile_plan.argv.len(),
    });

    // The handle the manifest's kill fields end up in, from a live fake child.
    let mut live = plan.clone();
    live.env_set.insert("FAKE_HOLD".into(), "60".into());
    let spawned = spawn(&live).expect("the fake harness starts");
    let registered = handle(&live, &spawned, "2026-10-05T00:00:00Z");
    let mut child = spawned.child;
    let program = spawned.program.clone();
    let _ = child.kill();
    let _ = child.wait();
    let h = registered.expect("a live child registers");
    let handle_json = json!({
        "strategy": serde_json::to_value(h.strategy).unwrap(),
        "native_binary": h.native_binary,
        "grace_seconds": h.grace_seconds,
        "pgid_is_pid": h.pgid == Some(h.pid),
        "native_pid_found": h.native_pid.is_some(),
        "has_process_start": h.process_start.is_some(),
        "program": norm(&l, &program),
        "argv": h.argv.iter().map(|a| norm(&l, a)).collect::<Vec<_>>(),
        "env_keys": h.env_keys.iter().filter(|k| *k != "FAKE_HOLD").collect::<Vec<_>>(),
    });

    // The per-run home, when the profile asks for one.
    let home = match dp.home.config_home.as_str() {
        "per_run" => {
            let run = tempfile::tempdir().unwrap();
            let run_dir = run.path().join("run");
            std::fs::create_dir(&run_dir).unwrap();
            let made =
                create_per_run_home(&run_dir, &dc.loaded.manifest.capture.home, &dc.loaded.seeds)
                    .expect("the per-run home is made");
            json!({
                "kind": "per_run",
                "dir_name": made.file_name().unwrap().to_str().unwrap(),
                "returned_under_run_dir": made.starts_with(std::fs::canonicalize(&run_dir).unwrap()),
                "tree": tree(&made),
            })
        }
        _ => json!({"kind": "shared", "per_run_tree": null}),
    };

    let files: Vec<Value> = {
        let dir = dc.dir.join("files");
        match std::fs::read_dir(&dir) {
            Ok(rd) => {
                let mut names: Vec<_> = rd.map(|e| e.unwrap().file_name()).collect();
                names.sort();
                names
                    .into_iter()
                    .map(|n| {
                        let bytes = std::fs::read(dir.join(&n)).unwrap();
                        json!({"name": n.to_str().unwrap(), "bytes": bytes.len(), "sha256": sha256_hex(&bytes)})
                    })
                    .collect()
            }
            Err(_) => Vec::new(),
        }
    };

    let home_env = dp.home.env.as_deref();
    let argv_deny: Vec<Value> = ARGV_PROBES
        .iter()
        .map(|arg| {
            json!({
                "arg": arg,
                "generic": capture_arg_denied(arg),
                "for_family": legacy_guard_refusal(dp, arg),
            })
        })
        .collect();
    let env_deny: Vec<Value> = ENV_PROBES
        .iter()
        .map(|name| json!({"name": name, "denied": legacy_env_refusal(name, home_env)}))
        .collect();

    let legacy = legacy(dc);
    let origin = origin_of(id);

    json!({
        "connector": {
            "id": dc.loaded.manifest.id,
            "family": legacy.family,
            "capture_only": legacy.capture_only,
            "launch_program": dp.executable.program,
            "launch_home_env": dp.home.env,
            "validates": verdict(dc.loaded.manifest.validate(origin)),
            format!("validate_{}", "capture"): verdict(dc.loaded.manifest.validate(origin)),
            "files_dir": files,
        },
        "capture_profile": profile_json(dp),
        "plan": plan_json(&l, &plan),
        "goal_placement_for_an_option_shaped_goal": placement,
        "streams": connector_streams_json(dp),
        "home": home,
        "ca": ca_json(&l, dp, &plan),
        "credential": credential_json(dp),
        "termination": {
            "kill": termination_json(&dc.loaded.manifest),
            "registered_handle": handle_json,
        },
        "argv_deny": argv_deny,
        "env_deny": env_deny,
        "redaction": {
            "extra_patterns": dc.loaded.manifest.redaction.as_ref()
                .map(|r| r.extra_patterns.iter().map(|p| json!({"kind": p.kind, "regex": p.regex})).collect::<Vec<_>>())
                .unwrap_or_default(),
        },
    })
}

/// Every per-connector golden for the connectors directory `root`: one `(<id>.json, value)` per
/// connector it holds, in discovery order.
fn goldens(root: &Path) -> Vec<(String, Value)> {
    connector_ids(root)
        .into_iter()
        .map(|id| {
            let c = goalcup_cli::connector::load_harness(root, &id, false)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            (format!("{id}.json"), connector_golden(&c))
        })
        .collect()
}

/// One `<id>.json` per connector the directory holds: a new folder under `connectors/` gets its
/// golden from the regenerate switch, with no edit here. Every differing connector is reported.
#[test]
fn every_shipped_connector_capture_behaviour_is_pinned() {
    check_goldens(&goldens(&s::connectors_dir()));
}

#[test]
fn the_controlled_workload_capture_behaviour_is_pinned() {
    let subject = declared(CONTROLLED);
    check_golden(
        "m2-controlled-workload.json",
        &connector_golden(&subject.connector),
    );
}

// ---------------------------------------------------------------------------------------
// Across families.
// ---------------------------------------------------------------------------------------

/// A valid base manifest whose family, CA variables and credential the matrices vary. The
/// shipped Codex manifest accepts every family's argv guard, so a refusal in a row is the
/// field the row varies.
fn base() -> HarnessManifest {
    s::shipped("codex-cli").loaded.manifest
}

fn legacy_ca_verdict(family: &str, names: &[&str]) -> String {
    let duplicate = names
        .iter()
        .enumerate()
        .any(|(i, name)| names[..i].contains(name));
    let listed = names.iter().all(|name| {
        matches!(
            *name,
            "CODEX_CA_CERTIFICATE" | "NODE_EXTRA_CA_CERTS" | "HERMES_CA_BUNDLE"
        )
    });
    let replacing_for_other = family != "hermes" && names.contains(&"HERMES_CA_BUNDLE");
    if names.is_empty() || duplicate || !listed || replacing_for_other {
        "refused:capture.ca_env".to_string()
    } else {
        "ok".to_string()
    }
}

fn legacy_credential_verdict(family: &str, name: Option<&str>) -> String {
    match name {
        None => "ok",
        Some("DEEPSEEK_API_KEY") if family != "codex" => "ok",
        _ => "refused:capture.credential_env",
    }
    .to_string()
}

fn legacy_family_guard(family: &str, arg: &str) -> Option<&'static str> {
    if family != "hermes" {
        return None;
    }
    let hermes = s::shipped("hermes-cli");
    legacy_guard_refusal(&hermes.loaded.manifest.capture, arg)
}

#[test]
fn the_family_keyed_decisions_over_every_family_are_pinned() {
    // The CA variable a family may name.
    let ca_sets: [(&str, Vec<&str>); 10] = [
        ("codex_ca", vec!["CODEX_CA_CERTIFICATE"]),
        ("node_extra_ca", vec!["NODE_EXTRA_CA_CERTS"]),
        ("hermes_ca", vec!["HERMES_CA_BUNDLE"]),
        ("ssl_cert_file", vec!["SSL_CERT_FILE"]),
        ("requests_ca_bundle", vec!["REQUESTS_CA_BUNDLE"]),
        ("empty_name", vec![""]),
        ("none", vec![]),
        (
            "duplicate",
            vec!["NODE_EXTRA_CA_CERTS", "NODE_EXTRA_CA_CERTS"],
        ),
        ("two", vec!["CODEX_CA_CERTIFICATE", "NODE_EXTRA_CA_CERTS"]),
        (
            "measured_and_replacing",
            vec!["NODE_EXTRA_CA_CERTS", "HERMES_CA_BUNDLE"],
        ),
    ];
    let mut ca = BTreeMap::new();
    for family in FAMILIES {
        let mut row = BTreeMap::new();
        for (label, names) in &ca_sets {
            row.insert(*label, legacy_ca_verdict(family, names));
        }
        ca.insert(family, row);
    }

    // The credential variable a family may name.
    let credentials: [(&str, Option<&str>); 4] = [
        ("none", None),
        ("deepseek", Some("DEEPSEEK_API_KEY")),
        ("openai", Some("OPENAI_API_KEY")),
        ("empty", Some("")),
    ];
    let mut credential = BTreeMap::new();
    for family in FAMILIES {
        let mut row = BTreeMap::new();
        for (label, name) in credentials {
            row.insert(label, legacy_credential_verdict(family, name));
        }
        credential.insert(family, row);
    }

    // The argv guard of every family over the probe list.
    let mut argv = BTreeMap::new();
    for family in FAMILIES {
        let row: Vec<Value> = ARGV_PROBES
            .iter()
            .map(|arg| json!([arg, legacy_family_guard(family, arg)]))
            .collect();
        argv.insert(family, row);
    }

    // The same argument as one more element of the recipe: whether validation refuses it.
    let mut argv_in_recipe = BTreeMap::new();
    for family in FAMILIES {
        let mut row = BTreeMap::new();
        for arg in [
            "--profile=x",
            "--tui",
            "-p",
            "-Qh",
            "--prof",
            "--query=x",
            "-Q",
        ] {
            let refused =
                capture_arg_denied(arg).is_some() || legacy_family_guard(family, arg).is_some();
            row.insert(
                arg,
                if refused {
                    "refused:capture.args".to_string()
                } else {
                    "ok".to_string()
                },
            );
        }
        argv_in_recipe.insert(family, row);
    }

    // The stream decision for every family and credential.
    let mut streams = BTreeMap::new();
    for family in FAMILIES {
        let mut row = BTreeMap::new();
        for (label, name) in [
            ("no_credential", None),
            ("credential", Some("DEEPSEEK_API_KEY")),
        ] {
            row.insert(label, streams_json(family, name));
        }
        streams.insert(family, row);
    }

    check_golden(
        "cross-family.json",
        &json!({
            "families": FAMILIES,
            "base_manifest": {"id": base().id, "family": "codex", "baseline": "ok"},
            "ca_env_by_family": ca,
            "credential_by_family": credential,
            "argv_guard_by_family": argv,
            "argv_in_recipe_by_family": argv_in_recipe,
            "capture_streams_by_family": streams,
        }),
    );
}

// ---------------------------------------------------------------------------------------
// Identity grammars.
// ---------------------------------------------------------------------------------------

/// A reader that hands out its chunks one `read` at a time, then EOF.
struct Chunks(std::collections::VecDeque<Vec<u8>>);

impl Chunks {
    fn new(chunks: Vec<Vec<u8>>) -> Chunks {
        Chunks(chunks.into_iter().filter(|c| !c.is_empty()).collect())
    }
}

impl Read for Chunks {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.0.pop_front() {
            None => Ok(0),
            Some(mut chunk) => {
                let n = chunk.len().min(buf.len());
                buf[..n].copy_from_slice(&chunk[..n]);
                if n < chunk.len() {
                    self.0.push_front(chunk.split_off(n));
                }
                Ok(n)
            }
        }
    }
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Closed;

impl Write for Closed {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
}

fn started(id: &str) -> Vec<u8> {
    format!("{{\"type\":\"thread.started\",\"thread_id\":\"{id}\"}}\n").into_bytes()
}

/// What the Codex stdout tee announced for these chunks, and what it forwarded.
fn codex_tee(chunks: Vec<Vec<u8>>, closed: bool) -> Value {
    let input: Vec<u8> = chunks.iter().flatten().copied().collect();
    let (tx, rx) = std::sync::mpsc::channel();
    let sink = Sink::default();
    let result = if closed {
        stdout_tee(Chunks::new(chunks), Closed, tx)
    } else {
        stdout_tee(Chunks::new(chunks), sink.clone(), tx)
    };
    let announced: Vec<Value> = rx.try_iter().map(|m| json!(m)).collect();
    let forwarded = sink.0.lock().unwrap().clone();
    json!({
        "result": if result.is_ok() { "ok" } else { "error" },
        "announced": announced,
        "forwarded_equals_input": !closed && forwarded == input,
        "forwarded_bytes": forwarded.len(),
    })
}

/// What the Hermes stderr tee read as the session marker.
fn hermes_marker(chunks: Vec<Vec<u8>>) -> Value {
    let tee = tee_marked(Chunks::new(chunks), Sink::default(), None).unwrap();
    let marker = tee.marker_until(Instant::now() + Duration::from_secs(20));
    match marker {
        ScopeMarker::One(id) => json!({"marker": "one", "id": id}),
        ScopeMarker::Absent => json!({"marker": "absent"}),
        ScopeMarker::Conflicting => json!({"marker": "conflicting"}),
    }
}

fn line(text: &str) -> Vec<u8> {
    format!("{text}\n").into_bytes()
}

#[test]
fn the_identity_grammars_are_pinned() {
    let big = |n: usize| vec![b'x'; n];
    let limit = 1024 * 1024;
    let mut overlong_started = b"{\"type\":\"thread.started\",\"thread_id\":\"".to_vec();
    overlong_started.extend(big(limit + 8));
    overlong_started.extend(b"\"}\n");
    let mut overlong_noise = big(limit + 1);
    overlong_noise.push(b'\n');
    let mut at_limit = big(limit);
    at_limit.push(b'\n');

    let codex_cases: Vec<(&str, Value)> = vec![
        ("thread_started", codex_tee(vec![started("t-1")], false)),
        (
            "after_noise",
            codex_tee(
                vec![
                    line("not json"),
                    line("{\"type\":\"turn.started\"}"),
                    started("t-2"),
                ],
                false,
            ),
        ),
        (
            "one_byte_chunks",
            codex_tee(
                started("t-3").chunks(1).map(<[u8]>::to_vec).collect(),
                false,
            ),
        ),
        (
            "no_thread_id",
            codex_tee(vec![line("{\"type\":\"thread.started\"}")], false),
        ),
        (
            "numeric_thread_id",
            codex_tee(
                vec![line("{\"type\":\"thread.started\",\"thread_id\":7}")],
                false,
            ),
        ),
        (
            "never_started",
            codex_tee(vec![line("{\"type\":\"turn.started\"}")], false),
        ),
        (
            "unterminated_last_line",
            codex_tee(
                vec![started("t-4").strip_suffix(b"\n").unwrap().to_vec()],
                false,
            ),
        ),
        (
            "first_of_two_wins",
            codex_tee(vec![started("first"), started("second")], false),
        ),
        (
            "type_must_be_exact",
            codex_tee(
                vec![line("{\"type\":\"thread.started.x\",\"thread_id\":\"t\"}")],
                false,
            ),
        ),
        (
            "crlf",
            codex_tee(
                vec![b"{\"type\":\"thread.started\",\"thread_id\":\"t-5\"}\r\n".to_vec()],
                false,
            ),
        ),
        (
            "long_id_is_not_judged_here",
            codex_tee(vec![started(&"a".repeat(200))], false),
        ),
        (
            "overlong_noise_then_started",
            codex_tee(vec![overlong_noise.clone(), started("late")], false),
        ),
        (
            "overlong_started_line_is_skipped",
            codex_tee(vec![overlong_started, started("after")], false),
        ),
        (
            "line_exactly_at_the_limit_then_started",
            codex_tee(vec![at_limit, started("ok")], false),
        ),
        (
            "closed_destination_still_announces",
            codex_tee(vec![started("t-6")], true),
        ),
        ("empty_stream", codex_tee(vec![], false)),
    ];

    let id = "20260930_141500_a1b2c3";
    let other = "20260930_141501_ffeedd";
    let marker = |text: String| hermes_marker(vec![line(&text)]);
    let hermes_cases: Vec<(&str, Value)> = vec![
        ("valid", marker(format!("session_id: {id}"))),
        (
            "valid_repeated",
            hermes_marker(vec![
                line(&format!("session_id: {id}")),
                line(&format!("session_id: {id}")),
            ]),
        ),
        (
            "conflicting",
            hermes_marker(vec![
                line(&format!("session_id: {id}")),
                line(&format!("session_id: {other}")),
            ]),
        ),
        ("absent", hermes_marker(vec![line("hello")])),
        ("empty_stream", hermes_marker(vec![])),
        ("no_space_after_colon", marker(format!("session_id:{id}"))),
        ("two_spaces", marker(format!("session_id:  {id}"))),
        ("leading_space", marker(format!(" session_id: {id}"))),
        ("embedded_in_a_line", marker(format!("x session_id: {id}"))),
        ("trailing_text", marker(format!("session_id: {id} x"))),
        (
            "trailing_cr",
            hermes_marker(vec![format!("session_id: {id}\r\n").into_bytes()]),
        ),
        (
            "uppercase_hex",
            marker("session_id: 20260930_141500_A1B2C3".to_string()),
        ),
        (
            "hex_above_f",
            marker("session_id: 20260930_141500_a1b2g3".to_string()),
        ),
        (
            "short_hex",
            marker("session_id: 20260930_141500_a1b2c".to_string()),
        ),
        (
            "long_hex",
            marker("session_id: 20260930_141500_a1b2c3d".to_string()),
        ),
        (
            "letters_in_date",
            marker("session_id: 2026093x_141500_a1b2c3".to_string()),
        ),
        (
            "wrong_separator",
            marker("session_id: 20260930-141500-a1b2c3".to_string()),
        ),
        (
            "a_uuid",
            marker("session_id: 5f0c2a8e-8a0e-4c1f-9d6b-2b7d2b0e9a11".to_string()),
        ),
        (
            "a_codex_style_token",
            marker("session_id: rollout-abc".to_string()),
        ),
        ("prefix_case", marker(format!("Session_id: {id}"))),
        (
            "split_across_chunks",
            hermes_marker(vec![
                b"session_".to_vec(),
                b"id: 2026".to_vec(),
                b"0930_141500_a1b2c3\n".to_vec(),
            ]),
        ),
        (
            "unterminated_last_line",
            hermes_marker(vec![format!("session_id: {id}").into_bytes()]),
        ),
        (
            "overlong_line_before_marker",
            hermes_marker(vec![
                big(5000),
                b"\n".to_vec(),
                line(&format!("session_id: {id}")),
            ]),
        ),
        (
            "overlong_marker_line",
            hermes_marker(vec![
                format!("session_id: {id}").into_bytes(),
                big(5000),
                b"\n".to_vec(),
            ]),
        ),
    ];

    // Claude Code is told its identity rather than reading it: the plan carries the id.
    let l = s::Layout::new();
    let exe = s::fake_harness(&l.bin);
    let claude = base_subject("claude-code-cli");
    let resolved = s::resolved(&exe);
    let proxy = s::proxy_url();
    let parent = s::parent_env();
    let plan = build_capture_launch(
        &claude.connector,
        &s::vars(&l),
        &l.workspace,
        &s::launch(&l, b"g", &resolved, &proxy, &parent),
    )
    .unwrap();
    let at = plan.argv.iter().position(|a| a == "--session-id").unwrap();

    check_golden(
        "identity-grammar.json",
        &json!({
            "codex_stdout_tee": codex_cases.into_iter().collect::<BTreeMap<_, _>>(),
            "hermes_stderr_marker": hermes_cases.into_iter().collect::<BTreeMap<_, _>>(),
            "claude_preassigned": {
                "flag": plan.argv[at],
                "value_is_the_launch_session_id": plan.argv[at + 1] == s::SESSION,
            },
            "stdout_line_limit_bytes": limit,
            "stderr_line_limit_bytes": goalcup_cli::capture_tee::LINE_LIMIT,
        }),
    );
    let _ = FragmentMatcher::new(WELL_FORMED_KEY.as_bytes());
}

// ---------------------------------------------------------------------------------------
// The loader script.
// ---------------------------------------------------------------------------------------

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/with-deepseek-key.sh")
}

/// Run the loader against a fake `goalcup` that records its argv and whether
/// `DEEPSEEK_API_KEY` reached it (never the value).
fn loader(args: &dyn Fn(&Path) -> Vec<String>, env_text: &str, mode: u32) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let fake = bin.join("goalcup");
    std::fs::write(
        &fake,
        "#!/bin/sh -p\nprintf '%s\\n' \"$@\" > \"$OUT/argv\"\n\
         if [ -n \"${DEEPSEEK_API_KEY+x}\" ]; then printf '%s' \"${#DEEPSEEK_API_KEY}\" > \"$OUT/key-length\"; fi\n\
         env | /usr/bin/sed -n 's/^\\(OTHER_[A-Z_]*\\)=.*/\\1/p' > \"$OUT/leaked\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let file = dir.path().join("key.env");
    std::fs::write(&file, env_text).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
    let out = std::process::Command::new("/bin/sh")
        .arg("-p")
        .arg(script())
        .args(args(&file))
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("OUT", dir.path())
        .output()
        .unwrap();
    let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).ok();
    json!({
        "exit": out.status.code(),
        "ran_goalcup": dir.path().join("argv").exists(),
        "goalcup_argv": read("argv").map(|a| a.lines().map(|l| l.replace(dir.path().to_str().unwrap(), "<TMP>")).collect::<Vec<_>>()),
        "key_length_in_goalcup_env": read("key-length"),
        "other_variables_leaked": read("leaked").map(|t| t.lines().map(str::to_string).collect::<Vec<_>>()),
        "stderr_has_key": String::from_utf8_lossy(&out.stderr).contains(WELL_FORMED_KEY),
        "stdout_has_key": String::from_utf8_lossy(&out.stdout).contains(WELL_FORMED_KEY),
        "refusal_prefix": String::from_utf8_lossy(&out.stderr).starts_with("refused: "),
    })
}

#[test]
fn the_deepseek_loader_script_behaviour_is_pinned() {
    let good = format!("OTHER_SECRET=nope\nDEEPSEEK_API_KEY={WELL_FORMED_KEY}\n");
    let start = |harness: &'static str, tail: Vec<&'static str>| {
        move |file: &Path| -> Vec<String> {
            let mut v = vec![
                file.display().to_string(),
                "start".into(),
                "--harness".into(),
                harness.into(),
            ];
            v.extend(tail.iter().map(|t| t.to_string()));
            v
        }
    };
    let cases: Vec<(&str, Value)> = vec![
        (
            "hermes_with_goal_file",
            loader(
                &start("hermes-cli", vec!["--goal-file", "/g.txt"]),
                &good,
                0o600,
            ),
        ),
        (
            "hermes_with_equals_options",
            loader(
                &start("hermes-cli", vec!["--goal-file=/g.txt"]),
                &good,
                0o600,
            ),
        ),
        (
            "codex_is_refused",
            loader(
                &start("codex-cli", vec!["--goal-file", "/g.txt"]),
                &good,
                0o600,
            ),
        ),
        (
            "claude_is_refused",
            loader(
                &start("claude-code-cli", vec!["--goal-file", "/g.txt"]),
                &good,
                0o600,
            ),
        ),
        (
            "another_option_is_refused",
            loader(
                &start("hermes-cli", vec!["--goal-file", "/g.txt", "--dev"]),
                &good,
                0o600,
            ),
        ),
        (
            "another_subcommand_is_refused",
            loader(
                &|file: &Path| {
                    vec![
                        file.display().to_string(),
                        "status".into(),
                        "--harness".into(),
                        "hermes-cli".into(),
                        "--goal-file".into(),
                        "/g".into(),
                    ]
                },
                &good,
                0o600,
            ),
        ),
        (
            "mode_0644_is_refused",
            loader(
                &start("hermes-cli", vec!["--goal-file", "/g.txt"]),
                &good,
                0o644,
            ),
        ),
        (
            "no_key_line_is_refused",
            loader(
                &start("hermes-cli", vec!["--goal-file", "/g.txt"]),
                "OTHER_SECRET=x\n",
                0o600,
            ),
        ),
        (
            "two_key_lines_are_refused",
            loader(
                &start("hermes-cli", vec!["--goal-file", "/g.txt"]),
                &format!(
                    "DEEPSEEK_API_KEY={WELL_FORMED_KEY}\nDEEPSEEK_API_KEY={WELL_FORMED_KEY}\n"
                ),
                0o600,
            ),
        ),
        (
            "short_key_is_refused",
            loader(
                &start("hermes-cli", vec!["--goal-file", "/g.txt"]),
                "DEEPSEEK_API_KEY=sk-short\n",
                0o600,
            ),
        ),
        (
            "quoted_key_is_accepted",
            loader(
                &start("hermes-cli", vec!["--goal-file", "/g.txt"]),
                &format!("DEEPSEEK_API_KEY=\"{WELL_FORMED_KEY}\"\n"),
                0o600,
            ),
        ),
        (
            "key_with_a_space_is_refused",
            loader(
                &start("hermes-cli", vec!["--goal-file", "/g.txt"]),
                "DEEPSEEK_API_KEY=sk-0000111122223333 44445555\n",
                0o600,
            ),
        ),
    ];
    check_golden(
        "loader-script.json",
        &json!({
            "script": "scripts/with-deepseek-key.sh",
            "cases": cases.into_iter().collect::<BTreeMap<_, _>>(),
        }),
    );
}

// ---------------------------------------------------------------------------------------
// The connectors this tree ships.
// ---------------------------------------------------------------------------------------

/// The schema-1 connector folders under `root`, by the rule `goalcup start` and the daemon
/// discover them with: real directories holding a `harness.toml`, sorted.
fn connector_ids(root: &Path) -> Vec<String> {
    goalcup_protocol::harness::discover_connector_dirs(root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .iter()
        .map(|d| d.file_name().unwrap().to_str().unwrap().to_string())
        .collect()
}

/// `enumeration.json` for the connectors directory `root`.
fn enumeration(root: &Path) -> Value {
    let ids = connector_ids(root);
    let mut rows = Vec::new();
    for id in &ids {
        let c = goalcup_cli::connector::load_harness(root, id, false)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        let legacy = legacy(&c);
        rows.push(json!({
            "dir": id,
            "id": c.loaded.manifest.id,
            "family": legacy.family,
            "has_capture_profile": true,
            "capture_only": legacy.capture_only,
            "has_pinned_dir_digest": goalcup_protocol::pins::pinned_connector_dir_sha256(id).is_some(),
            "dir_digest_matches_its_pin": goalcup_protocol::pins::connector_dir_is_pinned(id, &c.loaded.connector_dir_sha256),
        }));
    }
    json!({
        "shipped": ids,
        "controlled_workload_fixture": CONTROLLED,
        "rows": rows,
    })
}

#[test]
fn the_shipped_connector_enumeration_is_pinned() {
    check_golden("enumeration.json", &enumeration(&s::connectors_dir()));
}

/// The id of the synthetic connector the data-only proof adds.
const EXTRA: &str = "synthetic-extra-cli";

/// A folder with no `harness.toml` the data-only proof adds. The golden derivation skips it, as
/// `goalcup start` does; shipping one under `connectors/` still fails `harness_conformance`.
const STRAY: &str = "notes-not-a-connector";

/// A connectors directory holding a copy of every shipped connector, one synthetic schema-1
/// connector in the shape of the kit's stderr-token fixture (with a program that is not an
/// interpreter, so it validates as `Origin::Pinned`), and one folder with no `harness.toml`.
fn connectors_plus_one() -> tempfile::TempDir {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let dest = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &dest);
            } else {
                std::fs::copy(e.path(), &dest).unwrap();
            }
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let real = s::connectors_dir();
    for e in std::fs::read_dir(&real).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &tmp.path().join(e.file_name()));
        }
    }
    let kit = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/harness-kit/synthetic-stderr-token/harness.toml");
    let mut text = std::fs::read_to_string(kit).unwrap();
    for (from, to) in [
        (
            "id = \"kit-synthetic-stderr-token\"",
            format!("id = \"{EXTRA}\""),
        ),
        (
            "args = [\"-I\", \"-S\", \"@STAND_IN@\", \"@PORT@\", \"@TEST_ID@\", \"@MARKER@\", \"--\"]",
            "args = [\"run\", \"--\"]".to_string(),
        ),
        (
            "program = \"python3\"",
            "program = \"synthtool\"".to_string(),
        ),
        // A pinned connector names no loopback host; the kit's stand-in host is Dev-only.
        (
            "host = \"api.e00.localhost\"",
            "host = \"api.synthetic-extra.example\"".to_string(),
        ),
    ] {
        assert!(text.contains(from), "{from} is not in the kit fixture");
        text = text.replace(from, &to);
    }
    let extra = tmp.path().join(EXTRA);
    std::fs::create_dir(&extra).unwrap();
    std::fs::write(extra.join("harness.toml"), text).unwrap();
    std::fs::create_dir(tmp.path().join(STRAY)).unwrap();
    tmp
}

/// Adding a schema-1 connector is data only. The same bodies the real tests pin the repo with,
/// [`goldens`] and [`enumeration`], run over a copy of `connectors/` plus one synthetic folder
/// and one stray folder: the synthetic connector gets its `<id>.json` with the derived fields,
/// its enumeration row, and the stray folder gets neither, while the shipped rows do not move.
/// Only the regenerated JSON changes in such a PR, never this file.
#[test]
fn a_new_schema1_connector_needs_no_edit_to_this_file() {
    let tmp = connectors_plus_one();
    let root = tmp.path();
    assert!(
        root.join(STRAY).is_dir(),
        "the temp tree holds the stray folder"
    );
    let mut want = connector_ids(&s::connectors_dir());
    want.push(EXTRA.to_string());
    want.sort();

    // Every gap at once, so a failure names all of them.
    let mut gaps: Vec<String> = Vec::new();
    let mut expect = |what: &str, got: Value, want: Value| {
        if got != want {
            gaps.push(format!("{what}: got {got}, want {want}"));
        }
    };

    // The per-connector goldens, from the body that pins the repo.
    let plus_goldens = goldens(root);
    expect(
        "the per-connector golden files",
        json!(plus_goldens.iter().map(|(n, _)| n).collect::<Vec<_>>()),
        json!(
            want.iter()
                .map(|id| format!("{id}.json"))
                .collect::<Vec<_>>()
        ),
    );
    let extra_file = format!("{EXTRA}.json");
    let extra = plus_goldens
        .iter()
        .find(|(n, _)| *n == extra_file)
        .map(|(_, v)| v.clone())
        .unwrap_or(Value::Null);
    expect(
        "the new connector's id",
        extra["connector"]["id"].clone(),
        json!(EXTRA),
    );
    expect(
        "family (the program)",
        extra["connector"]["family"].clone(),
        json!("synthtool"),
    );
    expect(
        "capture_only",
        extra["connector"]["capture_only"].clone(),
        json!(true),
    );
    expect(
        "streams (from the stderr_line identity)",
        extra["streams"].clone(),
        json!({"decision": "MarkedStderr", "pipes": {"stdout": false, "stderr": true}}),
    );

    // The enumeration, from the body that pins the repo.
    let real = enumeration(&s::connectors_dir());
    let plus = enumeration(root);
    expect("the enumeration", plus["shipped"].clone(), json!(want));
    let rows = plus["rows"].as_array().unwrap();
    for row in real["rows"].as_array().unwrap() {
        expect(
            &format!("shipped row {}", row["id"]),
            json!(rows.contains(row)),
            json!(true),
        );
    }
    let row = rows.iter().find(|r| r["dir"] == EXTRA);
    expect(
        "the new connector's row",
        row.map(|r| {
            json!([
                r["id"],
                r["family"],
                r["capture_only"],
                r["has_pinned_dir_digest"]
            ])
        })
        .unwrap_or(Value::Null),
        json!([EXTRA, "synthtool", true, false]),
    );
    expect(
        "the stray folder in the enumeration",
        json!(
            plus["shipped"].as_array().unwrap().contains(&json!(STRAY))
                || rows.iter().any(|r| r["dir"] == STRAY)
        ),
        json!(false),
    );
    assert!(
        gaps.is_empty(),
        "{} gap(s):\n{}",
        gaps.len(),
        gaps.join("\n")
    );
}
