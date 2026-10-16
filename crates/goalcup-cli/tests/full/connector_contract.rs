//! The connector contract, dynamic half: one participant-shaped run of every connector folder
//! under `connectors/` ([`shipped_connector_ids`]) through the real `goalcup` and daemon
//! binaries, with one stand-in (`fixtures/connector-stand-in.sh`) installed as its program.
//!
//! The stand-in knows no harness. The runner derives everything it needs from the connector's
//! own `harness.toml`: chat mode or headless, its version probe, the session file its first
//! native source finds, its goal-cup skill folder, its CA variables and its identity line. So a
//! new connector folder is run here with no edit. The connector directory is copied byte for
//! byte, so it loads as the pinned one without `--dev`; the provider-host probe is skipped
//! (`GOALCUP_TEST_OFFLINE_UPSTREAMS`), so no provider is reached and nothing goes over the wire.
//! The static half is `tests/it/connector_contract.rs`.

use crate::common::m2_cli_support::OnPty;
use crate::common::m2_vertical_support::{self, GOAL, RESERVE, Vertical};
use crate::common::real_home_guard::with_real_home_guard;
use crate::common::secret_support::KEY;
use crate::common::support;

use goalcup_protocol::capture::{CaptureRecord, CaptureSource};
use goalcup_protocol::capture_policy::SHARED_CA_ENV;
use goalcup_protocol::harness::{CaptureSpec, IdGrammar, IdentitySource, Origin};
use goalcup_protocol::pins::shipped_connector_ids;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

/// What a chat-mode run's participant types.
const PROMPT: &str = "hello";

/// `text` as one single-quoted shell word.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The session id the stand-in reports, in the connector's identity grammar.
fn session_id(c: &CaptureSpec) -> &'static str {
    match c.identity.grammar {
        Some(IdGrammar::TimestampHex) => "20261016_120000_abcdef",
        _ => "kit-session-0001",
    }
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

/// The stand-in's variables, as shell assignments, derived from the connector alone.
fn stand_in_header(c: &CaptureSpec, out: &Path) -> String {
    let config_home = format!("${{{}}}", c.home.env.as_deref().unwrap_or("HOME"));
    let sid = session_id(c);
    let native = c
        .native
        .first()
        .expect("a connector reads at least one native source");
    let root = native
        .root
        .replace("{config_home}", &config_home)
        .replace("{run_home}", "${HOME}");
    let pattern = native
        .pattern
        .replace('*', "kit")
        .replace('?', "k")
        .replace("{id}", sid);
    let skills = c
        .home
        .skills_dir
        .as_ref()
        .map(|dir| format!("{config_home}/{dir}/goal-cup"))
        .unwrap_or_default();
    let identity = &c.identity;
    let (stream, line) = match identity.source {
        IdentitySource::None => ("", String::new()),
        IdentitySource::StderrLine => (
            "stderr",
            format!("{}{sid}", identity.prefix.as_deref().unwrap_or("")),
        ),
        IdentitySource::StdoutJson => {
            let mut event = serde_json::Value::Null;
            for (path, value) in [
                (
                    identity.match_field.as_deref(),
                    identity.match_value.as_deref(),
                ),
                (identity.id_field.as_deref(), Some(sid)),
            ] {
                let mut at = &mut event;
                for part in path
                    .expect("a stdout_json identity names its fields")
                    .split('.')
                {
                    if !at.is_object() {
                        *at = serde_json::json!({});
                    }
                    at = at
                        .as_object_mut()
                        .unwrap()
                        .entry(part)
                        .or_insert(serde_json::Value::Null);
                }
                *at = serde_json::json!(value.expect("and the value it matches"));
            }
            ("stdout", event.to_string())
        }
        other => panic!(
            "the connector stand-in does not speak `{other:?}` identity yet: add it to \
             fixtures/connector-stand-in.sh and here"
        ),
    };
    let mode = if c.interactive { "chat" } else { "headless" };
    format!(
        "KIT_OUT={out}\nKIT_MODE={mode}\nKIT_VERSION_ARGV={version}\n\
         KIT_NATIVE_FILE=\"{root}/{pattern}\"\nKIT_SKILLS_DIR=\"{skills}\"\nKIT_CA_VARS={ca}\n\
         KIT_ID_STREAM={stream}\nKIT_ID_LINE={line}\nKIT_CONFIGURE={configure}\n\
         export KIT_OUT KIT_MODE KIT_VERSION_ARGV KIT_NATIVE_FILE KIT_SKILLS_DIR KIT_CA_VARS \
         KIT_ID_STREAM KIT_ID_LINE KIT_CONFIGURE\n",
        out = sh_quote(&out.display().to_string()),
        version = sh_quote(&c.executable.version_args.join(" ")),
        ca = sh_quote(&c.ca.env.join(" ")),
        line = sh_quote(&line),
        configure = sh_quote(&configure_lines(c)),
    )
}

/// One connector, run the way a participant runs it, and judged.
fn run_connector(id: &str) {
    let world = Vertical::new("plain", serde_json::json!({}));
    // The world's development workload goes: without `--dev` every connector in the directory
    // is held to the pinned rules, and that one is a development connector.
    std::fs::remove_dir_all(world.connectors.join(m2_vertical_support::CONNECTOR)).unwrap();
    let connector_dir = world.connectors.join(id);
    support::copy_tree(&support::connectors_dir().join(id), &connector_dir);
    let pinned = goalcup_protocol::pins::pinned_connector_dir_sha256(id)
        .expect("a shipped connector is pinned");
    assert_eq!(
        goalcup_protocol::connector_dir::connector_dir_sha256(&connector_dir).unwrap(),
        pinned,
        "{id}: the copy is the pinned directory"
    );
    let manifest = goalcup_protocol::harness::load_dir(&connector_dir, Origin::Pinned)
        .expect("the shipped connector loads")
        .manifest;
    let c = &manifest.capture;

    // The stand-in, under the connector's program name. A chat-mode harness gets the
    // participant's PATH, so it starts as an npm harness does, `#!/usr/bin/env node` with the
    // `node` beside it; a headless one gets the launch's fixed PATH, so it is a plain script.
    let out = world.root.join("kit-out");
    let bin = world.root.join("participant-bin");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    let body = std::fs::read_to_string(support::fixture("connector-stand-in.sh")).unwrap();
    let (_, body) = body.split_once('\n').unwrap();
    let shebang = if c.interactive {
        "#!/usr/bin/env node"
    } else {
        "#!/bin/sh"
    };
    let program = bin.join(&c.executable.program);
    std::fs::write(
        &program,
        format!("{shebang}\n{}{body}", stand_in_header(c, &out)),
    )
    .unwrap();
    std::fs::write(bin.join("node"), "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
    for file in [program.clone(), bin.join("node")] {
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let runs = world.root.join("runs");
    let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup"));
    command
        .args(["start", "--harness", id])
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
    if c.interactive {
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
    } else {
        use std::os::unix::process::CommandExt;
        let output = command
            .arg("--goal-file")
            .arg(&world.goal)
            .stdin(Stdio::null())
            .process_group(0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{id}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

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
    let tty = if c.interactive { "tty" } else { "not-a-tty" };
    assert_eq!(report("stdin").trim(), tty, "{id}");
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
    let chat_only = if c.interactive {
        Some("xterm-256color")
    } else {
        None
    };
    assert_eq!(env.get("TERM").map(String::as_str), chat_only, "{id}");
    let argv = report("argv");
    assert!(
        env.values().all(|v| !v.contains(KEY)),
        "{id}: the child was handed a provider key"
    );
    assert!(!argv.contains(KEY), "{id}: the key is in no argument");
    // `goalcup start` hands each CA variable the file its row names (D41): a harness's own
    // variable the proxy's CA alone, a shared row the proxy's CA and the system's roots.
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

    // The argv is the manifest's `args`, rendered for this run's workspace; a headless run
    // places its goal after them, as one element.
    let entry = world.entry_id();
    let workspace = goalcup_cli::entries::read_entry(&world.cfg, &entry)
        .expect("the run's entry record")
        .workspace;
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
    let goal = std::fs::read_to_string(&world.goal).unwrap();
    if c.interactive {
        assert_eq!(got.len(), want.len(), "{id}: the args alone");
    } else {
        let rest = &got[want.len()..];
        assert_eq!(
            rest.iter().filter(|a| a.contains(goal.trim())).count(),
            1,
            "{id}: one element holds the goal: {rest:?}"
        );
    }

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
        Some((id.to_string(), Some(pinned.to_string()))),
        "{id}: Launch binds the pinned connector"
    );
    let streams = chain.streams();
    let goals: Vec<_> = streams
        .values()
        .filter(|s| s.source == CaptureSource::Goal)
        .collect();
    assert_eq!(goals.len(), 1, "{id}");
    let (want_goal, prompt) = if c.interactive {
        (
            goalcup_cli::commands::start::CHAT_MODE_GOAL.to_string(),
            PROMPT,
        )
    } else {
        (goal.clone(), GOAL)
    };
    assert_eq!(goals[0].data, want_goal.as_bytes(), "{id}: the goal stream");
    let native: Vec<u8> = streams
        .values()
        .filter(|s| s.source == CaptureSource::Native)
        .flat_map(|s| s.data.clone())
        .collect();
    assert!(
        String::from_utf8_lossy(&native).contains(prompt),
        "{id}: the prompt is in the recorded session file"
    );
    let (code, integrity) = world.verify(&entry);
    assert_eq!(integrity["sealed"], true, "{id}: {integrity}");
    assert_eq!(integrity["integrity"], "valid", "{id}: {integrity}");
    assert_eq!(code, 0, "{id}: {integrity}");
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
            run_connector(id);
        }
    });
}
