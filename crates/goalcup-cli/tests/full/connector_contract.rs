//! The connector contract, dynamic half: one participant-shaped run of every connector folder
//! under `connectors/` ([`shipped_connector_ids`]) through the real `goalcup` and daemon
//! binaries, with one stand-in (`fixtures/connector-stand-in.sh`) installed as its program.
//!
//! The stand-in knows no harness. The runner derives everything it needs from the connector's
//! own `harness.toml`: its version probe, the session file its first native source finds, its
//! goal-cup skill folder and its CA variables. Every connector runs in chat mode (D57) on a
//! terminal of its own. So a new connector folder is run here with no edit. The connector directory is copied byte for
//! byte, so it loads as the pinned one without `--dev`. The frozen schema-1 fixtures
//! (`tests/fixtures/schema-1-connectors/`, harnesses nobody ships) take the same run through
//! `--dev`, which pins the connector contract; the provider-host probe is skipped
//! (`GOALCUP_TEST_OFFLINE_UPSTREAMS`), so no provider is reached and nothing goes over the wire.
//! The static half is `tests/it/connector_contract.rs`.

use crate::common::m2_cli_support::OnPty;
use crate::common::m2_vertical_support::{self, RESERVE, Vertical};
use crate::common::real_home_guard::with_real_home_guard;
use crate::common::secret_support::KEY;
use crate::common::support;

use goalcup_protocol::capture::{CaptureRecord, CaptureSource};
use goalcup_protocol::capture_policy::SHARED_CA_ENV;
use goalcup_protocol::harness::{CaptureSpec, HarnessManifest, Origin};
use goalcup_protocol::pins::shipped_connector_ids;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// What a chat-mode run's participant types.
const PROMPT: &str = "hello";

/// `text` as one single-quoted shell word.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The connector's `capture.configure` commands, one per line, each argument list joined by
/// spaces: how the stand-in recognises one.
fn configure_lines(c: &CaptureSpec) -> String {
    c.configure
        .iter()
        .map(|command| command.join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The connector's `[launch_prep] after_exit` commands (D72), one per line, each argument list
/// joined by spaces: how the stand-in recognises one.
fn after_exit_lines(m: &HarnessManifest) -> String {
    m.launch_prep
        .iter()
        .flat_map(|p| p.after_exit.iter())
        .map(|command| command.join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The stand-in's variables, as shell assignments, derived from the connector alone.
fn stand_in_header(m: &HarnessManifest, out: &Path) -> String {
    let c = &m.capture;
    let config_home = format!("${{{}}}", c.home.env.as_deref().unwrap_or("HOME"));
    let native = c
        .native
        .first()
        .expect("a connector reads at least one native source");
    let root = native
        .root
        .replace("{config_home}", &config_home)
        .replace("{run_home}", "${HOME}");
    let pattern = native.pattern.replace('*', "kit").replace('?', "k");
    let skills = c
        .home
        .skills_dir
        .as_ref()
        .map(|dir| format!("{config_home}/{dir}/goal-cup"))
        .unwrap_or_default();
    format!(
        "KIT_OUT={out}\nKIT_VERSION_ARGV={version}\n\
         KIT_NATIVE_FILE=\"{root}/{pattern}\"\nKIT_SKILLS_DIR=\"{skills}\"\nKIT_CA_VARS={ca}\n\
         KIT_CONFIGURE={configure}\nKIT_AFTER_EXIT={after_exit}\n\
         export KIT_OUT KIT_VERSION_ARGV KIT_NATIVE_FILE KIT_SKILLS_DIR KIT_CA_VARS \
         KIT_CONFIGURE KIT_AFTER_EXIT\n",
        out = sh_quote(&out.display().to_string()),
        version = sh_quote(&c.executable.version_args.join(" ")),
        ca = sh_quote(&c.ca.env.join(" ")),
        configure = sh_quote(&configure_lines(c)),
        after_exit = sh_quote(&after_exit_lines(m)),
    )
}

/// Where a connector run comes from: a shipped connector, loaded as the pinned directory, or a
/// frozen schema-1 fixture, which no build pins, run through `--dev`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Folder {
    Shipped,
    Schema1Fixture,
}

/// One connector, run the way a participant runs it, and judged.
fn run_connector(id: &str, from: Folder) {
    let world = Vertical::new("plain", serde_json::json!({}));
    // The world's development workload goes: without `--dev` every connector in the directory
    // is held to the pinned rules, and that one is a development connector.
    std::fs::remove_dir_all(world.connectors.join(m2_vertical_support::CONNECTOR)).unwrap();
    let connector_dir = world.connectors.join(id);
    let source = match from {
        Folder::Shipped => support::connectors_dir(),
        Folder::Schema1Fixture => support::schema_1_fixtures_dir(),
    };
    support::copy_tree(&source.join(id), &connector_dir);
    let digest = goalcup_protocol::connector_dir::connector_dir_sha256(&connector_dir).unwrap();
    let origin = match from {
        Folder::Shipped => {
            let pinned = goalcup_protocol::pins::pinned_connector_dir_sha256(id)
                .expect("a shipped connector is pinned");
            assert_eq!(digest, pinned, "{id}: the copy is the pinned directory");
            Origin::Pinned
        }
        Folder::Schema1Fixture => Origin::Dev,
    };
    let manifest = goalcup_protocol::harness::load_dir(&connector_dir, origin)
        .unwrap_or_else(|e| panic!("{id}: the connector recipe no longer validates: {e}"))
        .manifest;
    let c = &manifest.capture;

    // The stand-in, under the connector's program name. A chat-mode harness gets the
    // participant's PATH, so it starts as an npm harness does, `#!/usr/bin/env node` with the
    // `node` beside it.
    let out = world.root.join("kit-out");
    let bin = world.root.join("participant-bin");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    let body = std::fs::read_to_string(support::fixture("connector-stand-in.sh")).unwrap();
    let (_, body) = body.split_once('\n').unwrap();
    let shebang = "#!/usr/bin/env node";
    let program = bin.join(&c.executable.program);
    std::fs::write(
        &program,
        format!("{shebang}\n{}{body}", stand_in_header(&manifest, &out)),
    )
    .unwrap();
    std::fs::write(bin.join("node"), "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
    for file in [program.clone(), bin.join("node")] {
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // The model service the participant's own setup saved, for a connector that reads it (D62).
    if let Some(from) = &c.model_host_from {
        let home = world
            .cfg
            .join(goalcup_protocol::harness::CAPTURE_HOMES_DIR)
            .join(id);
        support::write_model_host_config(&home, from, "https://models.example.test/v1");
    }

    let runs = world.root.join("runs");
    let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup"));
    command.args(["start", "--harness", id]);
    if from == Folder::Schema1Fixture {
        command.arg("--dev");
    }
    command
        .current_dir(&runs)
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
        )
        .env("HOME", world.root.join("home"))
        .env(goalcup_cli::config::CONFIG_DIR_ENV, &world.cfg)
        .env(goalcup_cli::config::CONNECTORS_DIR_ENV, &world.connectors)
        .env(goalcup_cli::commands::start::TEST_OFFLINE_UPSTREAMS, "1")
        .env("GOALCUP_TEST_FINISH_BUDGET_MS", "10000")
        .env("GOALCUP_TEST_LOCAL_WRAP_UP", "2000,100")
        .env("GOALCUP_TEST_DISK", format!("{RESERVE},0,50"))
        .env("TERM", "xterm-256color")
        .env("LANG", "en_US.UTF-8");
    for key in ["USER", "LOGNAME", "TMPDIR"] {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    // A provider key exported in the participant's shell never reaches the harness (D61).
    for name in ["DEEPSEEK_API_KEY", "AMP_API_KEY", "OPENAI_API_KEY"] {
        command.env(name, KEY);
    }
    let mut run = OnPty::spawn(command, None);
    run.wait_shown("kit: ready");
    run.type_line(PROMPT);
    run.wait_shown(&format!("kit: noted {PROMPT}"));
    run.type_line("/exit");
    let status = run.wait();
    assert!(status.success(), "{id}: {status:?}\n{}", run.shown());
    assert!(
        !run.shown().contains("kit: configured"),
        "{id}: a configure command printed on the participant's terminal\n{}",
        run.shown()
    );

    // What the harness was started with.
    let report = |name: &str| std::fs::read_to_string(out.join(name)).unwrap_or_default();
    // Each `capture.configure` command ran once, before the launch (D61): no terminal for its
    // input, the harness's own home, no proxy, and nothing it printed on the screen.
    let configured: Vec<String> = report("configure").lines().map(str::to_string).collect();
    let want_configured: Vec<String> = configure_lines(c)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| format!("{l} not-a-tty"))
        .collect();
    assert_eq!(configured, want_configured, "{id}: the configure commands");
    // The `[launch_prep] after_exit` commands ran, in order, last, with no terminal for their
    // input (D72). A launch prep command of the same words (Hermes's `gateway stop -y`) also ran
    // before the launch, so only the tail is theirs.
    let after: Vec<String> = report("after_exit").lines().map(str::to_string).collect();
    let want_after: Vec<String> = after_exit_lines(&manifest)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| format!("{l} not-a-tty"))
        .collect();
    assert!(
        after.ends_with(&want_after),
        "{id}: the after-exit commands: {after:?}, want {want_after:?} last"
    );
    if !c.configure.is_empty() {
        let configure_env = report("configure.env");
        assert!(
            !configure_env
                .lines()
                .any(|l| l.to_ascii_uppercase().starts_with("HTTPS_PROXY=")),
            "{id}: a configure command records nothing, so it has no proxy"
        );
        if let Some(home) = &c.home.env {
            let launched = report("env");
            let line = |text: &str| {
                text.lines()
                    .find(|l| l.starts_with(&format!("{home}=")))
                    .map(str::to_string)
            };
            assert_eq!(
                line(&configure_env),
                line(&launched),
                "{id}: a configure command runs in the harness's own home"
            );
        }
    }
    assert_eq!(report("stdin").trim(), "tty", "{id}");
    assert_eq!(
        report("skills").trim(),
        if c.home.skills_dir.is_some() {
            "SKILL.md"
        } else {
            "absent"
        },
        "{id}: the goal-cup skill"
    );
    let env: BTreeMap<String, String> = report("env")
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let inside = |key: &str| {
        let value = env.get(key).unwrap_or_else(|| panic!("{id}: no `{key}`"));
        assert!(
            Path::new(value).starts_with(&world.root),
            "{id}: `{key}` is outside the run's world: {value}"
        );
    };
    inside("HOME");
    if let Some(home) = &c.home.env {
        inside(home);
    }
    assert!(
        !env.keys()
            .any(|k| k.to_ascii_uppercase().ends_with("_BASE_URL")),
        "{id}: D1, no model base URL: {env:?}"
    );
    assert_eq!(
        env.get("TERM").map(String::as_str),
        Some("xterm-256color"),
        "{id}"
    );
    let argv = report("argv");
    assert!(
        env.values().all(|v| !v.contains(KEY)),
        "{id}: the child was handed a provider key"
    );
    assert!(!argv.contains(KEY), "{id}: the key is in no argument");
    // `goalcup start` hands each CA variable the file its row names (D41): a harness's own
    // variable the recording proxy's CA alone, a shared row the recording proxy's CA and the
    // system's roots.
    for name in &c.ca.env {
        let file = std::fs::read(out.join(format!("ca-{name}")))
            .unwrap_or_else(|e| panic!("{id}: the file `{name}` names: {e}"));
        let certs = goalcup_cli::ca_bundle::certificates(&file);
        if SHARED_CA_ENV.contains(&name.as_str()) {
            assert!(certs.len() > 1, "{id}: `{name}` keeps the system's roots");
        } else {
            assert_eq!(certs.len(), 1, "{id}: `{name}` trusts the proxy CA alone");
        }
    }

    // The argv is the manifest's `args`, rendered for this run's workspace, and nothing more.
    let entry = world.entry_id();
    let entry_record =
        goalcup_cli::entries::read_entry(&world.cfg, &entry).expect("the run's entry record");
    let workspace = entry_record.workspace;
    let mut vars = goalcup_protocol::template::TemplateVars::new();
    vars.set("workspace", workspace.clone());
    let want: Vec<String> = c
        .args
        .iter()
        .map(|a| goalcup_protocol::template::render(a, &vars).unwrap())
        .collect();
    let got: Vec<&str> = argv.lines().collect();
    assert_eq!(
        &got[..want.len().min(got.len())],
        want.as_slice(),
        "{id}: the args"
    );
    assert_eq!(got.len(), want.len(), "{id}: the args alone");

    // The chain: the pinned connector bound in `Launch`, one goal stream, the session file.
    let chain = world.chain(&entry);
    let launch = chain.records().find_map(|record| match record {
        CaptureRecord::Launch {
            profile_id,
            connector_dir_sha256,
            ..
        } => Some((profile_id.clone(), connector_dir_sha256.clone())),
        _ => None,
    });
    assert_eq!(
        launch,
        Some((id.to_string(), Some(digest))),
        "{id}: Launch binds the connector directory that ran"
    );
    let streams = chain.streams();
    let goals: Vec<_> = streams
        .values()
        .filter(|s| s.source == CaptureSource::Goal)
        .collect();
    assert_eq!(goals.len(), 1, "{id}");
    assert_eq!(
        goals[0].data,
        goalcup_cli::commands::start::CHAT_MODE_GOAL.as_bytes(),
        "{id}: the goal stream"
    );
    let native: Vec<u8> = streams
        .values()
        .filter(|s| s.source == CaptureSource::Native)
        .flat_map(|s| s.data.clone())
        .collect();
    assert!(
        String::from_utf8_lossy(&native).contains(PROMPT),
        "{id}: the prompt is in the recorded session file"
    );
    let (code, integrity) = world.verify(&entry);
    assert_eq!(integrity["sealed"], true, "{id}: {integrity}");
    assert_eq!(integrity["integrity"], "valid", "{id}: {integrity}");
    assert_eq!(code, 0, "{id}: {integrity}");

    // The session file is recorded under the hint the connector declared, as declared (D43). A
    // hint no decoder accepts is capture-only: the research report lists it under
    // `analysis.undecoded`, and the integrity verdict above is unchanged (D45).
    let declared = c.native[0].protocol_hint.as_deref();
    let registry = goalcup_verifier::capture_analysis::registry::DecoderRegistry::builtin();
    let (_, summary) = world.research(&entry);
    let undecoded = summary["analysis"]["undecoded"]
        .as_array()
        .unwrap_or_else(|| panic!("{id}: no analysis.undecoded: {summary}"));
    let mut native_streams = 0;
    for (key, stream) in streams
        .iter()
        .filter(|(_, s)| s.source == CaptureSource::Native)
    {
        native_streams += 1;
        assert_eq!(
            stream.meta.protocol_hint.as_deref(),
            declared,
            "{id}: the native stream keeps its declared hint"
        );
        let open = goalcup_verifier::capture_analysis::api::StreamOpen::new(
            key,
            CaptureSource::Native,
            declared,
            stream.meta.content_type.as_deref(),
        );
        let listed = undecoded
            .iter()
            .any(|g| g["source"] == "native" && g["hint"].as_str() == declared);
        assert_eq!(
            listed,
            registry.accepting(&open).is_empty(),
            "{id}: capture-only exactly when no decoder accepts `{declared:?}`: {summary}"
        );
    }
    assert!(native_streams > 0, "{id}: a native stream was recorded");
}

/// Every connector folder passes one participant-shaped run with no case of its own: it loads
/// as the pinned directory, runs the one stand-in with the environment its manifest asks for,
/// gets exactly its argv, records its prompt from the session file its native source reads,
/// binds the pinned digest in `Launch`, and seals a trace the verifier reads as valid. Nothing
/// under the real `HOME`'s app-wide harness homes changes.
#[test]
fn every_connector_runs_once_as_a_participant_runs_it() {
    let ids = shipped_connector_ids();
    assert!(!ids.is_empty(), "connectors/ holds the shipped connectors");
    with_real_home_guard(|| {
        for id in &ids {
            run_connector(id, Folder::Shipped);
        }
    });
}

/// The connector contract's launch promise: a connector written against schema 1 today, for a
/// harness nobody ships (the frozen fixtures, docs/CONNECTOR-SCHEMA.md "The connector contract"),
/// keeps launching with no Rust change. Through `--dev` it passes the same participant-shaped run
/// as a shipped connector: its configure commands, its own environment and CA file, its argv, its
/// session file recorded with a hint no decoder knows, and a sealed trace the verifier reads as
/// valid.
#[test]
fn a_schema_1_connector_written_today_keeps_launching() {
    with_real_home_guard(|| {
        for id in support::schema_1_fixture_ids() {
            run_connector(&id, Folder::Schema1Fixture);
        }
    });
}
