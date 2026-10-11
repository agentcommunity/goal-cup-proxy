//! The shipped connectors, as this CLI depends on them.
//!
//! The launch, resume, kill and hook-install assertions below duplicate the protocol crate's own
//! `manifest_load.rs` deliberately. It is the CLI that breaks when a launch recipe changes
//! — a renamed flag, a `mode` that moved from `tui` to `server`, a resume argument that
//! stopped being positional — so the CLI asserts the values it depends on rather than
//! trusting that somebody else's test still covers them.
//!
//! Nothing here mutates the process environment: the two cases that need one build a child's
//! environment map and hand it to `Command`, which is a different thing from `set_var`
//! (`tests/env_resolution.rs` says why that distinction is load-bearing).

mod support;

use goalcup_cli::connector::{self, PinState};
use goalcup_cli::version::VersionRange;
use goalcup_protocol::manifest::{InstallScope, KillStrategy, LaunchMode, SessionArgStyle};
use std::path::PathBuf;

fn load(id: &str) -> goalcup_cli::connector::Connector {
    connector::load(&support::connectors_dir(), id).expect("the shipped connector loads")
}

fn load_harness(id: &str) -> goalcup_cli::connector::HarnessConnector {
    connector::load_harness(&support::connectors_dir(), id, false)
        .expect("the shipped schema-1 harness loads")
}

#[test]
fn claude_launch_recipe_matches_spike_4_run_sh() {
    let c = load("claude-code-cli");
    let m = &c.manifest;
    assert_eq!(m.launch.mode, LaunchMode::Tui);
    assert_eq!(m.launch.program, "claude");
    assert_eq!(
        m.launch.args,
        vec![
            "--settings".to_string(),
            "{artifact_root}/settings-launch.json".to_string(),
            "--session-id".to_string(),
            "{session_id}".to_string()
        ]
    );
    assert_eq!(m.launch.home_env.as_deref(), Some("CLAUDE_CONFIG_DIR"));
    assert_eq!(
        m.launch.env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
        Some("{harness_home}")
    );
    assert_eq!(
        m.launch
            .env
            .get("OTEL_LOG_RAW_API_BODIES")
            .map(String::as_str),
        Some("file:{artifact_root}/raw-bodies")
    );
    assert!(
        !m.launch.env.keys().any(|k| k.ends_with("_BASE_URL")),
        "D1: the tracker never sets a model base URL"
    );
    assert!(
        m.launch
            .env_remove
            .iter()
            .any(|k| k == "ANTHROPIC_BASE_URL")
    );
    assert!(
        m.launch
            .banned_args
            .iter()
            .any(|a| a == "--no-session-persistence")
    );
    assert_eq!(
        m.resume.args,
        vec![
            "--settings".to_string(),
            "{artifact_root}/settings-launch.json".to_string(),
            "--resume".to_string(),
            "{session_id}".to_string()
        ]
    );
    assert_eq!(
        m.resume.session_arg_style,
        SessionArgStyle::Flag,
        "it is an enum at HEAD, not the string the old plan asserted"
    );
    let sl = m
        .launch
        .status_line
        .as_ref()
        .expect("R18: Claude gets a status line");
    assert_eq!(
        (sl.settings_key.as_str(), sl.command_template.as_str()),
        ("statusLine", "cat {status_line_path}")
    );
    assert_eq!(m.hook_install[0].path, "settings-launch.json");
    assert_eq!(m.hook_install[0].scope, InstallScope::ArtifactRoot);
    assert_eq!(
        m.hook_install.len(),
        1,
        "the workspace settings shim went with the hook leg"
    );
    assert_eq!(
        (m.kill.strategy, m.kill.grace_seconds),
        (KillStrategy::ProcessGroup, 10)
    );
}

#[test]
fn the_codex_recipe_matches_what_the_spike_actually_ran() {
    let m = load("codex-cli").manifest;
    assert_eq!(m.launch.program, "codex");
    assert!(
        !m.launch.args.iter().any(|a| a.contains("hook")),
        "the hook flags went with the hook leg (m2/wire deletion pass): {:?}",
        m.launch.args
    );
    assert_eq!(m.launch.home_env.as_deref(), Some("CODEX_HOME"));
    assert_eq!(m.resume.args[0], "resume");
    assert_eq!(m.resume.args[1], "{session_id}", "positional, at index 1");
    assert_eq!(m.resume.session_arg_style, SessionArgStyle::Positional);
    assert_eq!(m.kill.strategy, KillStrategy::NativePid);
    assert_eq!(
        m.kill.native_binary.as_deref(),
        Some("codex"),
        "D22: the npm wrapper is not the session"
    );
    for banned in ["--ephemeral", "exec"] {
        assert!(
            m.launch.banned_args.iter().any(|a| a == banned),
            "missing {banned}"
        );
    }
}

#[test]
fn the_version_range_grammar_is_d12s_two_operators_and_compares_numerically() {
    let r = VersionRange::parse(">=2.1.259").unwrap();
    assert!(r.matches("2.1.259") && r.matches("2.2.0") && r.matches("2.1.300"));
    assert!(
        !r.matches("2.1.9"),
        "numeric per component: 9 is not above 259"
    );
    assert!(!r.matches("2.0.999"));
    let e = VersionRange::parse("=0.153.0").unwrap();
    assert!(e.matches("0.153.0") && !e.matches("0.153.1"));
    // D12: `=` records what was tested, it does not pin. Newer is
    // untested and allowed; the bound is the floor.
    use goalcup_cli::version::Standing;
    assert_eq!(e.standing("0.153.1"), Standing::NewerThanTested);
    assert_eq!(e.standing("0.152.9"), Standing::BelowFloor);
    assert_eq!(r.standing("2.1.300"), Standing::Tested);
    assert!(
        !e.matches("0.153.0-beta"),
        "a pre-release is a build made before the release D12 tested, and no range here can \
         name a tag"
    );
    assert!(
        VersionRange::parse("^1.2").is_err(),
        "two operators, no more"
    );
    assert_eq!(
        goalcup_cli::version::extract_version("codex-cli 0.153.0 (rust)").as_deref(),
        Some("0.153.0")
    );
    assert_eq!(
        goalcup_cli::version::extract_version("no numbers here"),
        None
    );
    for id in ["claude-code-cli", "codex-cli"] {
        VersionRange::parse(&load(id).manifest.tested_version_range)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// Each shipped schema-1 harness pins the exact version its recipe was measured at,
/// and that version is one the unchanged V1 range already covers — the capture pin does not
/// move `tested_version_range`, whose floor semantics are V1's (D12).
#[test]
fn each_harness_pins_an_exact_version_the_unchanged_v1_range_covers() {
    use goalcup_cli::version::Standing;
    for (id, range, version) in [
        ("claude-code-cli", ">=2.1.259", "2.1.283"),
        ("codex-cli", "=0.153.0", "0.153.0"),
    ] {
        let m = load(id).manifest;
        assert_eq!(
            m.tested_version_range, range,
            "{id}: V1's range is untouched"
        );
        let h = load_harness(id);
        let exe = &h.loaded.manifest.capture.executable;
        assert_eq!(exe.version, version, "{id}: launch-recipes §2");
        assert_eq!(
            VersionRange::parse(range).unwrap().standing(&exe.version),
            Standing::Tested,
            "{id}"
        );
    }
}

/// Every host a shipped schema-1 harness names is one the recording proxy accepts — its
/// own host rule and the per-launch CA it mints — and the protocol's rule, which validates
/// the manifest at load, agrees with the proxy's on every shape either one refuses.
#[test]
fn every_capture_host_is_one_the_proxy_accepts_and_the_two_host_rules_agree() {
    use goalcup_cli::proxy::tls::{ProxyCa, is_config_host};
    use goalcup_protocol::capture_policy::is_capture_host;
    for id in ["claude-code-cli", "codex-cli"] {
        let c = load_harness(id).loaded.manifest.capture;
        let approved: Vec<String> = c.hosts.iter().map(|h| h.host.clone()).collect();
        for h in approved.iter().chain(c.auth_hosts.iter()) {
            assert!(is_config_host(h), "{id}: `{h}`");
        }
        ProxyCa::generate(&approved).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let long_label = "a".repeat(64);
    let long_name = format!("{}.com", "a.".repeat(126));
    for sample in [
        "chatgpt.com",
        "api.anthropic.com",
        "localhost",
        "a-b.c",
        "xn--bcher-kva.example",
        "1.2.3",
        "123",
        "",
        "-a.com",
        "a-.com",
        "A.com",
        "a..com",
        "a.com.",
        "*.a.com",
        "a.com:443",
        "https://a.com",
        "u@a.com",
        "127.0.0.1",
        "::1",
        "[::1]",
        "a b.com",
        long_label.as_str(),
        long_name.as_str(),
    ] {
        assert_eq!(
            is_capture_host(sample),
            is_config_host(sample),
            "`{sample}`: the load-time rule and the proxy's must agree"
        );
    }
}

#[test]
fn a_shipped_connector_is_pinned_by_the_verifier_this_build_links() {
    // PRD 5.4: the digest the release-hash allowlist compares against is the one the verifier
    // binary pins. Over the whole DIRECTORY, because PRD 5.4 says "manifests and shims" and
    // the shims are the hook bodies under `files/`.
    for id in ["claude-code-cli", "codex-cli"] {
        let c = load(id);
        // Lowercase hex, and the length alone does not say that: `ManifestPinning::resolve`
        // compares case-insensitively, so a digest that started arriving uppercase from here
        // would keep passing while every printed digest changed shape.
        assert_eq!(c.manifest_sha256.len(), 64);
        assert_eq!(c.dir_sha256.len(), 64);
        assert_ne!(
            c.manifest_sha256, c.dir_sha256,
            "{id}: the directory digest covers files/ and the manifest digest does not"
        );
        assert!(
            c.manifest_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{id}'s digest is lowercase hex: {}",
            c.manifest_sha256
        );
        assert!(
            matches!(connector::pin_state(&c), PinState::Pinned),
            "{id} must match goalcup_protocol::pins::pinned_connector_dir_sha256"
        );
    }
    // An edited manifest is Altered, and says which digest it should have been.
    let tmp = tempfile::tempdir().unwrap();
    support::copy_connector(&tmp, "claude-code-cli");
    let path = tmp.path().join("claude-code-cli/manifest.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("free_ports = []", "free_ports = [1]")).unwrap();
    let edited = connector::load(tmp.path(), "claude-code-cli").unwrap();
    match connector::pin_state(&edited) {
        PinState::Altered { pinned } => assert_eq!(
            Some(pinned.as_str()),
            goalcup_protocol::pins::pinned_connector_dir_sha256("claude-code-cli")
        ),
        other => panic!("an edited rulebook must not read as pinned: {other:?}"),
    }
}

/// The gap the directory pin exists to close: the manifest is the shipped one, byte for
/// byte, and a hook template beside it is not.
///
/// The daemon renders that template into the workspace, records its rendered digest as the
/// config the entry will guard, and then guards it — so an edit here is an edit to what the
/// run captures, made under a manifest that still passes every check taken over the file.
/// `pin_state` reads `Altered`, and the manifest digest on the same `Connector` is still
/// the pinned one, which is what makes the point rather than merely asserting it.
#[test]
fn an_edited_hook_template_is_altered_even_though_the_manifest_is_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    support::copy_connector(&tmp, "codex-cli");
    let tracker = tmp.path().join("codex-cli/files/config.toml");
    let mut body = std::fs::read(&tracker).unwrap();
    body.push(b'\n');
    std::fs::write(&tracker, &body).unwrap();

    let tampered = connector::load(tmp.path(), "codex-cli").unwrap();
    assert_eq!(
        Some(tampered.manifest_sha256.as_str()),
        goalcup_verifier::pinned_manifest_sha256("codex-cli"),
        "the manifest is untouched, and a pin over it sees nothing"
    );
    match connector::pin_state(&tampered) {
        PinState::Altered { pinned } => assert_eq!(
            Some(pinned.as_str()),
            goalcup_protocol::pins::pinned_connector_dir_sha256("codex-cli")
        ),
        other => panic!("an edited hook template must not read as pinned: {other:?}"),
    }
}

/// The rest of the allowlist's vocabulary: a connector no build pins is `Unpinned`, and the
/// roster's own edited-manifest fixture reads `Altered`.
///
/// `Unpinned` is the reading a hand-written connector gets, and it has to be reachable from
/// a test or the third arm of the enum is a claim nothing checks. The fixture is a copy of a
/// shipped connector under a new id, which is exactly how a participant would produce one.
#[test]
fn a_connector_this_build_does_not_pin_reads_as_unpinned_and_an_edited_one_as_altered() {
    let tmp = tempfile::tempdir().unwrap();
    // A third surface is declared from the Claude Code family beside both shipping
    // surfaces. Discovery must use the id declared by each manifest, never an enumerated
    // registry or the directory basename.
    support::copy_connector(&tmp, "claude-code-cli");
    std::fs::rename(
        tmp.path().join("claude-code-cli"),
        tmp.path().join("claude-code-synthetic"),
    )
    .unwrap();
    let path = tmp.path().join("claude-code-synthetic/manifest.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text.replacen(
        "id = \"claude-code-cli\"",
        "id = \"claude-code-synthetic\"",
        1,
    );
    let text = text.replacen(
        "harness = \"claude-code-cli\"\npath_glob = \"{config_dir}/projects/*/*.jsonl\"",
        "harness = \"claude-code-synthetic\"\npath_glob = \"{config_dir}/projects/*/*.jsonl\"",
        1,
    );
    std::fs::write(&path, text).unwrap();
    for id in ["claude-code-cli", "codex-cli"] {
        support::copy_connector(&tmp, id);
    }
    let forked = connector::load(tmp.path(), "claude-code-synthetic").unwrap();
    assert_eq!(
        connector::pin_state(&forked),
        PinState::Unpinned,
        "no build pins a connector it does not ship"
    );
    assert_eq!(forked.manifest.family, "claude-code");
    assert_eq!(
        goalcup_cli::connector::list(tmp.path()).unwrap(),
        vec![
            "claude-code-cli".to_string(),
            "claude-code-synthetic".to_string(),
            "codex-cli".to_string(),
        ],
        "discovery accepts a third declared surface without a registry edit"
    );

    let dir = support::connectors_dir_with_edited_manifest("codex-cli");
    let edited = connector::load(&dir, "codex-cli").unwrap();
    assert!(
        matches!(connector::pin_state(&edited), PinState::Altered { .. }),
        "the roster's edited-manifest fixture is what Task 9 refuses a start on"
    );
}

/// Discovery over the shipped directory, and the two refusals it can produce.
#[test]
fn discovery_lists_what_ships_and_refuses_a_connector_that_does_not() {
    let dir = support::connectors_dir();
    assert_eq!(
        connector::list(&dir).unwrap(),
        vec!["claude-code-cli".to_string(), "codex-cli".to_string()]
    );
    for id in ["claude-code-cli", "codex-cli"] {
        assert!(
            load(id).dir.join("manifest.toml").is_file(),
            "`dir` is {id}'s own directory, which is where its files/ live"
        );
    }
    // "pick a connector this build ships" is rung 1, the rung `UnsupportedConnector` gets.
    let (code, said) = match connector::load(&dir, "cursor-cli") {
        Ok(c) => (0, c.manifest.id),
        Err(e) => (e.exit_code(), e.to_string()),
    };
    assert_eq!(
        code, 1,
        "a connector that does not ship is a refusal: {said}"
    );
    assert!(
        said.contains("cursor-cli") && said.contains("claude-code-cli"),
        "and it names what was asked for and what ships: {said}"
    );
    // A traversal is not a connector id: nothing joins the caller's string onto the path.
    let traversal = "../connectors/codex-cli";
    let (code, said) = match connector::load(&dir, traversal) {
        Ok(c) => (0, c.manifest.id),
        Err(e) => (e.exit_code(), e.to_string()),
    };
    assert_eq!(
        code, 1,
        "a traversal is a connector that does not ship: {said}"
    );
    assert!(
        said.contains(traversal),
        "and the refusal names what was asked for, verbatim: {said}"
    );
    assert!(
        !said.contains(&dir.join(traversal).display().to_string()),
        "no path under the connectors directory was joined out of it: {said}"
    );
}

/// One id, two directories, and nothing picks between them.
///
/// Discovery keys on the id a manifest DECLARES, so two directories can declare one id — and
/// the one that sorted second would vanish. That is where an unpinned rulebook would sit:
/// `goalcup doctor` would report the pinned twin while `goalcup start` ran whichever the sort
/// happened to reach first. Both [`connector::load`] and [`connector::list`] refuse instead.
#[test]
fn an_id_two_directories_declare_is_refused_by_name_and_by_both_paths() {
    let tmp = tempfile::tempdir().unwrap();
    support::copy_connector(&tmp, "codex-cli");
    let text = std::fs::read_to_string(tmp.path().join("codex-cli/manifest.toml")).unwrap();
    // A second directory whose manifest declares the same id, which is exactly what a
    // participant produces by copying a connector directory and editing nothing.
    let twin = tmp.path().join("codex-copy");
    std::fs::create_dir_all(&twin).unwrap();
    std::fs::write(twin.join("manifest.toml"), &text).unwrap();

    let (code, said) = match connector::load(tmp.path(), "codex-cli") {
        Ok(c) => (0, format!("loaded {}", c.dir.display())),
        Err(e) => (e.exit_code(), e.to_string()),
    };
    assert_eq!(
        code, 1,
        "an ambiguous id is a refusal and not a silent choice: {said}"
    );
    for named in ["codex-cli", "codex-copy"] {
        assert!(
            said.contains(&tmp.path().join(named).display().to_string()),
            "the refusal names {named}'s path so the twin can be found: {said}"
        );
    }
    let listed = match connector::list(tmp.path()) {
        Ok(ids) => format!("{ids:?}"),
        Err(e) => {
            assert_eq!(e.exit_code(), 1);
            String::new()
        }
    };
    assert!(
        listed.is_empty(),
        "and the roster refuses too rather than listing the id once: {listed}"
    );
}

/// The shell pinner keys table rows by manifest-declared ids too, so it must reject the
/// same ambiguous shipping set that the CLI refuses before it can generate duplicate rows.
#[cfg(unix)]
#[test]
fn the_pinner_refuses_duplicate_declared_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let connectors = tmp.path().join("connectors");
    support::copy_connectors_into(&connectors);
    let hyphen_id = "-n";
    let original = connectors.join("claude-code-cli/manifest.toml");
    let original_text = std::fs::read_to_string(&original).unwrap();
    std::fs::write(
        &original,
        original_text.replacen(
            "id = \"claude-code-cli\"",
            &format!("id = \"{hyphen_id}\""),
            1,
        ),
    )
    .unwrap();
    // The connector's harness.toml declares the id too, and the pinner refuses a folder whose
    // two files disagree (`folder_id`, another refusal, covered by
    // `the_pinner_refuses_a_harness_toml_whose_id_differs_from_the_manifests`); rewrite it so only the duplicate is left.
    let harness = connectors.join("claude-code-cli/harness.toml");
    let harness_text = std::fs::read_to_string(&harness).unwrap();
    std::fs::write(
        &harness,
        harness_text.replacen(
            "id = \"claude-code-cli\"",
            &format!("id = \"{hyphen_id}\""),
            1,
        ),
    )
    .unwrap();
    let twin = connectors.join("claude-code-cli-copy");
    std::fs::create_dir(&twin).unwrap();
    std::fs::copy(&original, twin.join("manifest.toml")).unwrap();
    std::fs::copy(&harness, twin.join("harness.toml")).unwrap();
    let scripts = tmp.path().join("scripts");
    std::fs::create_dir(&scripts).unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/pin-manifests.sh"),
        scripts.join("pin-manifests.sh"),
    )
    .unwrap();

    let output = std::process::Command::new("sh")
        .arg(scripts.join("pin-manifests.sh"))
        .arg("--print")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(!output.status.success(), "duplicate ids must refuse");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("duplicate declared connector id"),
        "{stderr}"
    );
    assert!(stderr.contains(hyphen_id), "{stderr}");
}

/// `pin-manifests.sh`'s `folder_id` refuses a folder whose `harness.toml` and `manifest.toml`
/// declare different ids, because one pin table would be keyed by an id the other does not carry.
#[cfg(unix)]
#[test]
fn the_pinner_refuses_a_harness_toml_whose_id_differs_from_the_manifests() {
    let tmp = tempfile::tempdir().unwrap();
    let connectors = tmp.path().join("connectors");
    support::copy_connectors_into(&connectors);
    let harness = connectors.join("codex-cli/harness.toml");
    let text = std::fs::read_to_string(&harness).unwrap();
    assert!(text.contains("id = \"codex-cli\""));
    std::fs::write(
        &harness,
        text.replacen("id = \"codex-cli\"", "id = \"codex-cli-other\"", 1),
    )
    .unwrap();
    let scripts = tmp.path().join("scripts");
    std::fs::create_dir(&scripts).unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/pin-manifests.sh"),
        scripts.join("pin-manifests.sh"),
    )
    .unwrap();

    let output = std::process::Command::new("sh")
        .arg(scripts.join("pin-manifests.sh"))
        .arg("--print")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(!output.status.success(), "disagreeing ids must refuse");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("harness.toml declares 'codex-cli-other' and manifest.toml 'codex-cli'"),
        "{stderr}"
    );
}

/// The D12 range, run against a harness binary's real output rather than a string literal.
///
/// The fake `claude` is resolved **on `PATH`** by a child shell, because that is how the
/// launch this range predicts will resolve it. The child's environment is built here rather
/// than inherited: it carries `PATH` and nothing else, so no model base URL and no proxy is
/// set for the probe, and the assertion below checks that rather than trusting it.
///
/// Running the version command itself is `preflight::sample_harness_version`. What is under
/// test here is the pair — `extract_version` over what a harness actually printed, and
/// `VersionRange::matches` over what came out.
#[cfg(unix)]
#[test]
fn the_range_holds_against_a_fake_harness_on_path_in_and_out_of_range() {
    let range = VersionRange::parse(">=2.1.259").expect("Claude's shipped range");
    for (printed, in_range) in [
        ("2.1.259 (Claude Code)", true),
        ("2.1.9 (Claude Code)", false),
    ] {
        let tmp = tempfile::tempdir().expect("a directory to put a fake harness in");
        let fake = tmp.path().join("claude");
        std::fs::write(&fake, format!("#!/bin/sh\necho '{printed}'\n")).expect("the fake harness");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
                .expect("the fake harness is executable");
        }

        let mut env = support::clean_env();
        env.insert("PATH".to_string(), tmp.path().display().to_string());
        assert!(
            !env.keys()
                .any(|k| k.ends_with("_BASE_URL") || k.ends_with("_PROXY")),
            "a version probe never runs with a model base URL or a proxy set: {env:?}"
        );
        // `/bin/sh` by absolute path, so the only thing `PATH` above resolves is the fake.
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args(["-c", "claude --version"])
            .env_clear()
            .envs(&env);
        let out = output_within(command, goalcup_cli::preflight::PROBE_TIMEOUT);
        assert!(
            out.status.success(),
            "the fake harness exited {:?}",
            out.status
        );

        let said = String::from_utf8_lossy(&out.stdout).to_string();
        let found = goalcup_cli::version::extract_version(&said)
            .unwrap_or_else(|| panic!("no version in {said:?}"));
        assert_eq!(
            range.matches(&found),
            in_range,
            "{found:?} against >=2.1.259, from a harness that printed {said:?}"
        );
    }
}

/// `Command::output()` with the bound `preflight` puts on its own probe.
///
/// The same shape as `sample_harness_version_within`'s wait — poll `try_wait`, kill at the
/// deadline, and only then read the pipes, so a child that filled one cannot hold this
/// process while this process holds it. That function is not reusable here (it takes a
/// `Connector` and runs the manifest's own version command), so what is shared is the bound
/// itself: [`goalcup_cli::preflight::PROBE_TIMEOUT`].
#[cfg(unix)]
fn output_within(
    mut command: std::process::Command,
    timeout: std::time::Duration,
) -> std::process::Output {
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the child runs");
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait().expect("waiting on the child") {
            Some(_) => break,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the child did not exit within {timeout:?}");
            }
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    child.wait_with_output().expect("what the child printed")
}

/// `config::connectors_dir`'s exe-relative fallback, driven through a child process.
///
/// It cannot be checked in this process: an integration test's own executable lives in
/// `target/debug/deps/`, so both candidates resolve to directories that do not exist and
/// the rule is never exercised. So the test binary is COPIED into each of the two layouts
/// the function knows about — an installed `bin/` beside `share/goalcup/connectors`, and a
/// `target/debug/` build beside a checkout's `connectors/` — and re-run there, one ignored
/// case at a time, with the expected directory handed over in the child's environment.
///
/// The third layout is both at once, which pins the ORDER: an installed build must not fall
/// back to a checkout that happens to be two levels above it.
#[cfg(unix)]
#[test]
fn connectors_dir_finds_the_shipped_manifests_beside_the_binary_it_was_run_from() {
    let me = std::env::current_exe().expect("this test binary's own path");
    let name = me
        .file_name()
        .map(std::ffi::OsString::from)
        .expect("and it is a file");

    for (exe_dir, ship_into, expected) in [
        (
            "opt/goalcup/bin",
            vec!["opt/goalcup/share/goalcup/connectors"],
            "opt/goalcup/share/goalcup/connectors",
        ),
        ("target/debug", vec!["connectors"], "connectors"),
        (
            "opt/goalcup/bin",
            vec!["opt/goalcup/share/goalcup/connectors", "connectors"],
            "opt/goalcup/share/goalcup/connectors",
        ),
    ] {
        let root = tempfile::tempdir().expect("a layout to run the binary from");
        let bin = root.path().join(exe_dir);
        std::fs::create_dir_all(&bin).expect("the directory the binary goes in");
        let copied = bin.join(&name);
        std::fs::copy(&me, &copied).expect("the test binary, copied into the layout");
        for rel in &ship_into {
            support::copy_connectors_into(&root.path().join(rel));
        }

        let mut env = support::clean_env();
        env.insert(
            EXPECTED_CONNECTORS_ENV.to_string(),
            root.path().join(expected).display().to_string(),
        );
        let out = std::process::Command::new(&copied)
            .args([
                "--exact",
                "--ignored",
                "--nocapture",
                "the_child_half_of_the_connectors_dir_fallback",
            ])
            .env_clear()
            .envs(&env)
            .output()
            .expect("the copied test binary runs");
        let printed = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            out.status.success(),
            "the child run from {} did not agree that connectors_dir() resolves to {expected}:\n{printed}{}",
            copied.display(),
            String::from_utf8_lossy(&out.stderr),
        );
        // A green child is not enough on its own: the child returns early when its variable
        // is unset, so an exit code of 0 could mean "agreed" or "did nothing". The sentinel
        // is printed only after the assertions, so this is what says the coverage was real.
        assert!(
            printed.contains(CHILD_AGREED),
            "the child exited 0 without saying it had checked anything:\n{printed}{}",
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// Where the child of the test above says what it found. Set by that test and by nothing
/// else; a run of this binary from cargo never sets it, which is the belt to the `#[ignore]`
/// brace below.
const EXPECTED_CONNECTORS_ENV: &str = "GOALCUP_TEST_EXPECTED_CONNECTORS_DIR";

/// What the child prints once it has agreed, and the parent looks for.
///
/// The gate below makes an unset variable a no-op, so an exit code of 0 does not prove the
/// child checked anything. This line does. It is printed last, after both assertions.
const CHILD_AGREED: &str = "goalcup-test: connectors_dir() resolved what the layout put there";

/// The other half of the test above, run only as its child.
///
/// `#[ignore]` keeps it out of the ordinary suite — where its `current_exe()` is in
/// `target/debug/deps/` and there is nothing to find — and the child passes `--ignored` to
/// select exactly it.
///
/// An unset [`EXPECTED_CONNECTORS_ENV`] returns rather than failing, the way the verifier's
/// own `GOALCUP_REGEN` cases gate: `cargo test -- --include-ignored` is a command the README
/// names as safe, and it runs this case with no parent to have set the variable. What keeps
/// the gate honest is [`CHILD_AGREED`], which the parent asserts on.
#[cfg(unix)]
#[test]
#[ignore = "run by connectors_dir_finds_the_shipped_manifests_beside_the_binary_it_was_run_from"]
fn the_child_half_of_the_connectors_dir_fallback() {
    let Ok(expected) = std::env::var(EXPECTED_CONNECTORS_ENV) else {
        println!("${EXPECTED_CONNECTORS_ENV} is unset: this case is run by its parent, not alone.");
        return;
    };
    let resolved = goalcup_cli::config::connectors_dir().expect("the fallback resolves");
    // Canonicalised on both sides: the resolved path is `<exe dir>/../share/...` verbatim —
    // `config_dir` and `connectors_dir` deliberately return what they were given — and on
    // macOS the temporary root is itself reached through a symlink.
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).map(|c| c.display().to_string());
    assert_eq!(
        canon(&resolved).ok(),
        canon(std::path::Path::new(&expected)).ok(),
        "connectors_dir() resolved {} and the layout put them in {expected}",
        resolved.display()
    );
    assert_eq!(
        goalcup_cli::connector::list(&resolved).unwrap(),
        vec!["claude-code-cli".to_string(), "codex-cli".to_string()],
        "and what it found is the shipped rulebook"
    );
    println!("{CHILD_AGREED}");
}
