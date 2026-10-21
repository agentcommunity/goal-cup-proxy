//! Characterization goldens for the harness contract (D42).
//!
//! Every per-connector fact the capture path takes from a connector's files is rendered here as
//! deterministic JSON and compared byte for byte with a committed file under
//! `tests/fixtures/harness-golden/`. The daemon and the verifier render their own halves in their
//! `harness_contract_golden` tests, into the same directory.
//!
//! * One file per connector, `<id>.json`: whether the folder's digest matches its pin, the
//!   resolved capture plan (program, argv with the goal placement, environment, process group,
//!   kill fields), the stream decision and its pipes, the per-run home tree where there is one,
//!   the CA variables, the redaction patterns, and the argv and environment probes whose verdict
//!   for this connector differs from the shared policy. The declared manifest is not echoed: the
//!   pin covers it. The registered process handle is checked against the plan, not pinned.
//! * Shared files, which no connector changes: `policy.json` (the generic argv and environment
//!   deny verdicts over fixed probe lists and the CA-variable rules).
//!
//! The connector list is `shipped_connector_ids()`, and every per-connector field is derived
//! from the connector's own files. Adding a connector folder therefore needs no edit here and
//! changes no shared file: regenerate, and review the new `<id>.json` in the same PR
//! (`a_new_connector_needs_no_edit_to_this_file` proves it).
//!
//! Values are synthetic. Only run-specific paths are normalised (the test's temporary
//! directory becomes `<TMP>`, the loopback proxy URL becomes `<PROXY_URL>`); everything else is
//! what the code produced. Nothing here reads or writes a harness home, and nothing sets a
//! process variable.
//!
//! To regenerate after a deliberate change: `GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test --offline
//! -p goalcup-cli --test it harness_contract_golden::`, then read the diff. The switch is off
//! unless set to exactly `1`.

use crate::common::m2_profile_support;

use goalcup_cli::ca_bundle::{HandedFile, handed_file};
use goalcup_cli::connector::HarnessConnector;
use goalcup_cli::launch::{LaunchPlan, build_capture_launch, handle, spawn};
use goalcup_cli::per_run_home::create_per_run_home;
use goalcup_protocol::ProtocolError;
use goalcup_protocol::capture_policy::{
    CA_ENV_SUFFIXES, CaMode, EnvContext, SHARED_CA_ENV, TRUST_ONLY, capture_arg_denied,
    env_key_denied,
};
use goalcup_protocol::harness::{CaptureSpec, HarnessManifest, Origin};
use goalcup_protocol::pins::shipped_connector_ids;
use m2_profile_support as s;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The controlled test workload: a fixture of this file, never a folder under `connectors/`.
const CONTROLLED: &str = "m2-controlled-workload";

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

/// A copy of `c` without its `capture.platform` table: the plan is the base recipe on every
/// platform, so the golden bytes are the same wherever it runs. The declared overrides, like
/// every other declared field, are pinned by the folder's digest.
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

/// Arguments the deny lists are asked about: every row of the generic list in several spellings,
/// options an argv guard may name before the harness's own parser, abbreviations and clusters,
/// and ordinary arguments.
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
        // Headless mode's goal slot, gone with it (D57): no plan has one.
        "capture": Value::Null,
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

/// The stream decision of one connector. Every connector is chat mode (D57): the harness
/// inherits all its streams, and no identity is read from one.
fn connector_streams_json() -> Value {
    json!({
        "decision": "Inherit",
        "pipes": {"stdout": false, "stderr": false},
    })
}

/// Each CA variable the profile names, the profile's mode, the file the launch hands it, and the
/// value the plan gives it (the proxy CA's path for a harness's own variable).
fn ca_json(l: &s::Layout, p: &CaptureSpec, plan: &LaunchPlan) -> Value {
    let mode = match p.ca.mode {
        CaMode::Extend => "extend",
        CaMode::Replace => "replace",
        _ => "unknown",
    };
    let rows: Vec<Value> =
        p.ca.env
            .iter()
            .map(|name| {
                json!({
                    "name": name,
                    "mode": mode,
                    "file": match handed_file(name) {
                        HandedFile::ProxyCa => "proxy_ca",
                        HandedFile::SystemRootsAndProxyCa => "system_roots_and_proxy_ca",
                    },
                    "value_in_plan": plan.env_set.get(name).map(|v| norm(l, v)),
                })
            })
            .collect();
    json!(rows)
}

/// What validation says when `edit` is applied to the connector's manifest.
fn validated_with(
    m: &HarnessManifest,
    origin: Origin,
    edit: impl FnOnce(&mut CaptureSpec),
) -> String {
    let mut m = m.clone();
    edit(&mut m.capture);
    verdict(m.validate(origin))
}

/// The verdict the shared policy alone gives a probe placed in `field`.
fn policy_verdict(field: &str, denied: Option<&str>) -> String {
    match denied {
        Some(_) => format!("refused:{field}"),
        None => "ok".to_string(),
    }
}

/// The fixed probes, then the probes this connector's own manifest calls for, each once.
fn probes(fixed: &[&str], own: Vec<String>) -> Vec<String> {
    let mut all: Vec<String> = fixed.iter().map(|p| p.to_string()).collect();
    for probe in own {
        if !all.contains(&probe) {
            all.push(probe);
        }
    }
    all
}

/// The arguments this connector's own `argv_guard` names, as a harness's parser reads them, so
/// a new connector's guard is probed with no edit to [`ARGV_PROBES`].
fn own_argv_probes(m: &HarnessManifest) -> Vec<String> {
    let Some(g) = m.capture.argv_guard.as_ref() else {
        return Vec::new();
    };
    let mut own = Vec::new();
    for long in &g.long {
        own.push(format!("--{long}"));
        own.push(format!("--{long}=x"));
        if g.long_abbreviations && long.len() > 1 {
            own.push(format!("--{}", &long[..long.len() - 1]));
        }
    }
    own.extend(g.short.iter().map(|s| format!("-{s}")));
    own.extend(g.short_with_value.iter().map(|s| format!("-{s}x")));
    own
}

/// The names this connector's own manifest calls for: its home variable, one name under each
/// reserved prefix, and its CA variables, so a new connector's own names are probed with no
/// edit to [`ENV_PROBES`].
fn own_env_probes(m: &HarnessManifest) -> Vec<String> {
    let c = &m.capture;
    let mut own: Vec<String> = c.home.env.iter().cloned().collect();
    own.extend(c.reserved_env_prefixes.iter().map(|p| format!("{p}X")));
    own.extend(c.ca.env.iter().cloned());
    own
}

/// The argv probes whose verdict, as the first element of this connector's `capture.args`,
/// differs from the shared policy's (`policy.json`): what this connector's own rules add.
fn argv_differences(m: &HarnessManifest, origin: Origin) -> Vec<Value> {
    probes(ARGV_PROBES, own_argv_probes(m))
        .iter()
        .map(String::as_str)
        .filter_map(|arg| {
            let got = validated_with(m, origin, |p| p.args.insert(0, arg.to_string()));
            (got != policy_verdict("capture.args", capture_arg_denied(arg))).then(|| {
                json!({
                    "arg": arg,
                    "in_capture_args": got,
                    "argv_guard": m.capture.argv_guard.as_ref().and_then(|g| g.denies(arg)),
                })
            })
        })
        .collect()
}

/// The environment probes whose verdict, as one more `capture.env` key of this connector,
/// differs from the shared policy's (`policy.json`).
fn env_differences(m: &HarnessManifest, origin: Origin) -> Vec<Value> {
    probes(ENV_PROBES, own_env_probes(m))
        .iter()
        .map(String::as_str)
        .filter_map(|name| {
            // A value every value rule admits, so the verdict is about the name.
            let value = if name.eq_ignore_ascii_case("SHELL") {
                "/bin/sh"
            } else {
                "1"
            };
            let got = validated_with(m, origin, |p| {
                p.env.insert(name.to_string(), value.to_string());
            });
            let policy = env_key_denied(name, EnvContext::CaptureEnv);
            (got != policy_verdict("capture.env", policy))
                .then(|| json!({"name": name, "in_capture_env": got}))
        })
        .collect()
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
    // Chat mode (D57) gives the harness no goal: the plan is the connector's args alone.
    let plan = build_capture_launch(
        c,
        &s::vars(&l),
        &l.workspace,
        &s::chat_launch(&l, &resolved, &proxy, &parent),
    )
    .unwrap_or_else(|e| panic!("{id}: {e}"));

    // What headless mode called goal placement: the argv's tail, and no goal slot.
    let placement = json!({
        "tail": plan.argv[plan.argv.len().saturating_sub(2)..]
            .iter()
            .map(|a| norm(&l, a))
            .collect::<Vec<_>>(),
        "goal_index": Value::Null,
        "goal_prefix": Value::Null,
        "argv_len": plan.argv.len(),
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
    assert_eq!(
        json!({
            "strategy": h.strategy,
            "native_binary": h.native_binary,
            "grace_seconds": h.grace_seconds,
        }),
        serde_json::to_value(&plan.kill).unwrap(),
        "{id}: the registered handle carries the plan's kill fields"
    );
    assert_eq!(norm(&l, &program), norm(&l, &plan.program), "{id}");
    assert!(
        h.process_start.is_some(),
        "{id}: the handle records the start"
    );

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

    let origin = origin_of(id);
    let m = &dc.loaded.manifest;

    json!({
        "connector": {
            "id": m.id,
            "launch_program": dp.executable.program,
            "launch_home_env": dp.home.env,
            "validates": verdict(m.validate(origin)),
            "dir_digest_matches_its_pin": goalcup_protocol::pins::connector_dir_is_pinned(id, &dc.loaded.connector_dir_sha256),
            "files_dir": files,
        },
        "plan": plan_json(&l, &plan),
        "goal_placement_for_an_option_shaped_goal": placement,
        "streams": connector_streams_json(),
        "home": home,
        "ca": ca_json(&l, dp, &plan),
        "argv_differs_from_policy": argv_differences(m, origin),
        "env_differs_from_policy": env_differences(m, origin),
        "redaction": {
            "extra_patterns": m.redaction.as_ref()
                .map(|r| r.extra_patterns.iter().map(|p| json!({"kind": p.kind(), "regex": p.regex()})).collect::<Vec<_>>())
                .unwrap_or_default(),
        },
    })
}

/// One `(<id>.json, value)` per connector of `ids`, each loaded from the connectors directory
/// `root`.
fn goldens(root: &Path, ids: &[String]) -> Vec<(String, Value)> {
    ids.iter()
        .map(|id| {
            let c = goalcup_cli::connector::load_harness(root, id, false)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            (format!("{id}.json"), connector_golden(&c))
        })
        .collect()
}

/// One `<id>.json` per shipped connector: a new folder under `connectors/` gets its golden from
/// the regenerate switch, with no edit here. Every differing connector is reported.
#[test]
fn every_shipped_connector_capture_behaviour_is_pinned() {
    check_goldens(&goldens(&s::connectors_dir(), &shipped_connector_ids()));
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
// The shared policy.
// ---------------------------------------------------------------------------------------

/// The capture policy every connector shares, over the fixed probe lists: the generic argv and
/// environment deny verdicts, the CA-variable rules and the admitted credential names. A connector's
/// own file lists only the probes whose verdict differs for it, so a policy change rewrites this
/// one file, not one per connector.
#[test]
fn the_shared_capture_policy_is_pinned() {
    check_golden(
        "policy.json",
        &json!({
            "argv": ARGV_PROBES
                .iter()
                .map(|arg| json!({"arg": arg, "denied": capture_arg_denied(arg)}))
                .collect::<Vec<_>>(),
            "env": ENV_PROBES
                .iter()
                .map(|name| json!({"name": name, "denied": env_key_denied(name, EnvContext::CaptureEnv)}))
                .collect::<Vec<_>>(),
            "ca": {
                "shared_extend_only": SHARED_CA_ENV,
                "own_name_suffixes": CA_ENV_SUFFIXES,
                "refused": TRUST_ONLY,
            },
        }),
    );
}

// ---------------------------------------------------------------------------------------
// The connectors this tree ships.
// ---------------------------------------------------------------------------------------

/// The connector folders under `root`, by the rule `goalcup start` and the daemon discover them
/// with: real directories holding a `harness.toml`, sorted.
fn connector_ids(root: &Path) -> Vec<String> {
    goalcup_protocol::harness::discover_connector_dirs(root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .iter()
        .map(|d| d.file_name().unwrap().to_str().unwrap().to_string())
        .collect()
}

/// The golden files no connector owns, written by this crate, the daemon and the verifier. None
/// of them is keyed by a connector, so adding one changes none of them.
const SHARED_GOLDENS: &[&str] = &[
    "decoder-selection.json",
    "launch-allowance-reader.json",
    "policy.json",
    "redaction-extra-patterns.json",
];

/// Every file under `harness-golden/` is a shared golden or belongs to a shipped connector (or to
/// the controlled workload fixture): its `<id>.json`, or the daemon's `<id>.daemon.json`. A
/// removed connector's goldens fail here until they go too.
#[test]
fn every_golden_file_belongs_to_a_shipped_connector_or_is_shared() {
    let mut owners = shipped_connector_ids();
    owners.push(CONTROLLED.to_string());
    let mut strays: Vec<String> = std::fs::read_dir(golden_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| {
            let id = name
                .strip_suffix(".daemon.json")
                .or_else(|| name.strip_suffix(".json"));
            !SHARED_GOLDENS.contains(&name.as_str())
                && !id.is_some_and(|id| owners.iter().any(|o| o == id))
        })
        .collect();
    strays.sort();
    assert!(
        strays.is_empty(),
        "no shipped connector or shared table owns {strays:?}: delete them"
    );
}

/// The id of the synthetic connector the data-only proof adds.
const EXTRA: &str = "synthetic-extra-cli";

/// A folder with no `harness.toml` the data-only proof adds. The golden derivation skips it, as
/// `goalcup start` does; shipping one under `connectors/` still fails `harness_conformance`.
const STRAY: &str = "notes-not-a-connector";

/// A connectors directory holding a copy of every shipped connector, one synthetic schema-1
/// connector in the shape of the kit's controlled-workload fixture (with a program that is not an
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
        .join("tests/fixtures/harness-kit/controlled-workload/harness.toml");
    let mut text = std::fs::read_to_string(kit).unwrap();
    for (from, to) in [
        (
            "id = \"kit-controlled-workload\"",
            format!("id = \"{EXTRA}\""),
        ),
        (
            "args = [\"-I\", \"-S\", \"@STAND_IN@\", \"@PORT@\", \"@TEST_ID@\", \"@MARKER@\", \"@GO@\"]",
            "args = [\"run\"]".to_string(),
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

/// Adding a connector is data only. The body that pins the repo, [`goldens`], runs over a copy
/// of `connectors/` plus one synthetic folder and one stray folder: the synthetic connector gets
/// its `<id>.json` with the derived fields, the stray folder gets none, and every shipped
/// connector's golden is what it is without them. The shared goldens take no connector input.
/// Only the new `<id>.json` is added in such a PR, never an edit to this file.
#[test]
fn a_new_connector_needs_no_edit_to_this_file() {
    let tmp = connectors_plus_one();
    let root = tmp.path();
    assert!(
        root.join(STRAY).is_dir(),
        "the temp tree holds the stray folder"
    );
    let mut want = shipped_connector_ids();
    want.push(EXTRA.to_string());
    want.sort();

    // Every gap at once, so a failure names all of them.
    let mut gaps: Vec<String> = Vec::new();
    let mut expect = |what: &str, got: Value, want: Value| {
        if got != want {
            gaps.push(format!("{what}: got {got}, want {want}"));
        }
    };

    let plus = goldens(root, &connector_ids(root));
    expect(
        "the per-connector golden files",
        json!(plus.iter().map(|(n, _)| n).collect::<Vec<_>>()),
        json!(
            want.iter()
                .map(|id| format!("{id}.json"))
                .collect::<Vec<_>>()
        ),
    );
    for (name, value) in goldens(&s::connectors_dir(), &shipped_connector_ids()) {
        expect(
            &format!("{name} beside the new connector"),
            json!(plus.iter().any(|(n, v)| *n == name && *v == value)),
            json!(true),
        );
    }
    let extra_file = format!("{EXTRA}.json");
    let extra = plus
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
        "an unpinned folder's digest",
        extra["connector"]["dir_digest_matches_its_pin"].clone(),
        json!(false),
    );
    expect(
        "streams (chat mode inherits them all)",
        extra["streams"].clone(),
        json!({"decision": "Inherit", "pipes": {"stdout": false, "stderr": false}}),
    );
    assert!(
        gaps.is_empty(),
        "{} gap(s):\n{}",
        gaps.len(),
        gaps.join("\n")
    );
}
