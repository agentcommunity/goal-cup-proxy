//! The shipped connectors, as this CLI depends on them: each loads from its `harness.toml`
//! alone, is the directory this build pins, and is found by the id it declares.
//!
//! The recipe assertions below duplicate the protocol crate's own schema tests deliberately.
//! It is the CLI that breaks when a capture recipe changes — a renamed program, a termination
//! strategy that moved, a base URL that crept into the environment — so the CLI asserts the
//! values it depends on rather than trusting that somebody else's test still covers them.
//!
//! The V1 `manifest.toml` recipe, version-range and hook-template tests went with the V1 files
//! (OS-29); the pin, discovery and duplicate-id promises they carried are asserted here over
//! `harness.toml`.

#[path = "common/support/mod.rs"]
mod support;

use goalcup_cli::connector;
use goalcup_cli::preflight::{Level, harness_pin_gate};
use goalcup_protocol::harness::Strategy;
use std::path::PathBuf;

fn load_harness(id: &str) -> goalcup_cli::connector::HarnessConnector {
    connector::load_harness(&support::connectors_dir(), id, false)
        .expect("the shipped schema-1 harness loads")
}

#[test]
fn claude_capture_recipe_is_the_pinned_headless_launch() {
    let m = load_harness("claude-code-cli").loaded.manifest;
    let c = &m.capture;
    assert_eq!(c.executable.program, "claude");
    assert_eq!(
        c.executable.version_args,
        ["--version"],
        "docs/harnesses/claude-code-cli.md"
    );
    let session = c
        .args
        .iter()
        .position(|a| a == "--session-id")
        .expect("the session id is preassigned on the command line");
    assert_eq!(c.args[session + 1], "{session_id}");
    assert!(
        !c.args.iter().any(|a| a == "--no-session-persistence"),
        "the native transcript is what the capture reads: {:?}",
        c.args
    );
    assert!(
        !c.env.keys().any(|k| k.ends_with("_BASE_URL")),
        "D1: the tracker never sets a model base URL"
    );
    assert_eq!(c.home.env.as_deref(), Some("CLAUDE_CONFIG_DIR"));
    let t = m.termination();
    assert_eq!(
        (t.strategy(), t.grace_seconds()),
        (Strategy::ProcessGroup, 10)
    );
}

#[test]
fn the_codex_capture_recipe_kills_the_native_binary_not_the_wrapper() {
    let m = load_harness("codex-cli").loaded.manifest;
    let c = &m.capture;
    assert_eq!(c.executable.program, "codex");
    assert_eq!(
        c.executable.version_args,
        ["--version"],
        "docs/harnesses/codex-cli.md"
    );
    assert!(
        !c.args.iter().any(|a| a == "--ephemeral"),
        "the rollout is what the capture reads: {:?}",
        c.args
    );
    assert!(
        !c.env.keys().any(|k| k.ends_with("_BASE_URL")),
        "D1: the tracker never sets a model base URL"
    );
    assert_eq!(c.home.env.as_deref(), Some("CODEX_HOME"));
    let t = m.termination();
    assert_eq!(t.strategy(), Strategy::NativePid);
    assert_eq!(
        t.native_binary.as_deref(),
        Some("codex"),
        "D22: the npm wrapper is not the session"
    );
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
    // PRD 5.4: the digest the release-hash allowlist compares against is the one this build
    // pins. Over the whole DIRECTORY, so a seed under `files/` is covered as well as the
    // `harness.toml` beside it.
    for id in ["claude-code-cli", "codex-cli", "hermes-cli"] {
        let c = load_harness(id);
        let digest = &c.loaded.connector_dir_sha256;
        // Lowercase hex, and the length alone does not say that: the pins are compared
        // case-insensitively, so a digest that started arriving uppercase from here would
        // keep passing while every printed digest changed shape.
        assert_eq!(digest.len(), 64);
        assert!(
            digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{id}'s digest is lowercase hex: {digest}"
        );
        assert_eq!(
            Some(digest.as_str()),
            goalcup_protocol::pins::pinned_connector_dir_sha256(id),
            "{id} must match goalcup_protocol::pins::pinned_connector_dir_sha256"
        );
        let gate = harness_pin_gate(&c, false);
        assert_eq!(gate.level, Level::Pass, "{id}: {}", gate.detail);
    }
    // An edited harness.toml under a pinned id is refused at load, with or without --dev, and
    // the refusal names the digest it found.
    let dir = support::connectors_dir_with_edited_harness("codex-cli");
    assert_altered_is_refused(&dir, "codex-cli");
}

/// `dir/<id>` is a pinned id holding bytes that are not its pin: `load_harness` refuses it as
/// rung 1 under both doors, naming the digest the directory actually has.
fn assert_altered_is_refused(dir: &std::path::Path, id: &str) {
    let found = goalcup_protocol::connector_dir::connector_dir_sha256(&dir.join(id)).unwrap();
    assert_ne!(
        Some(found.as_str()),
        goalcup_protocol::pins::pinned_connector_dir_sha256(id)
    );
    for dev in [false, true] {
        let error = match connector::load_harness(dir, id, dev) {
            Ok(_) => panic!("{id}: an altered pinned connector must not load (dev {dev})"),
            Err(error) => error,
        };
        assert_eq!(error.exit_code(), 1, "{error}");
        let said = error.to_string();
        assert!(
            said.contains("exact pinned bytes") && said.contains(&found),
            "dev {dev}: {said}"
        );
    }
}

/// The gap the directory pin exists to close: `harness.toml` is the shipped one, byte for
/// byte, and a seed beside it is not. The seed is copied into the run's home, so an edit there
/// is an edit to what the run does, made under a manifest that still passes every check taken
/// over the file.
#[test]
fn an_edited_seed_is_altered_even_though_the_harness_toml_is_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    support::copy_connector(&tmp, "hermes-cli");
    let seed = tmp.path().join("hermes-cli/files/config.yaml");
    let mut body = std::fs::read(&seed).unwrap();
    body.push(b'\n');
    std::fs::write(&seed, &body).unwrap();

    assert_eq!(
        std::fs::read(tmp.path().join("hermes-cli/harness.toml")).unwrap(),
        support::connector("hermes-cli").loaded.harness_toml,
        "harness.toml is untouched, and a digest over it alone sees nothing"
    );
    assert_altered_is_refused(tmp.path(), "hermes-cli");
}

/// A connector no build pins is refused without `--dev` and admitted with it, and the report
/// says which door it came through. The fixture is a copy of a shipped connector under a new
/// id and a new folder, which is exactly how a participant would produce one: discovery uses
/// the id the manifest declares, never an enumerated registry.
#[test]
fn a_connector_this_build_does_not_pin_is_refused_without_dev_and_warned_with_it() {
    let tmp = tempfile::tempdir().unwrap();
    for id in ["claude-code-cli", "codex-cli"] {
        support::copy_connector(&tmp, id);
    }
    let forked = tmp.path().join("claude-code-synthetic");
    std::fs::create_dir(&forked).unwrap();
    let text = std::fs::read_to_string(tmp.path().join("claude-code-cli/harness.toml"))
        .unwrap()
        .replacen(
            "id = \"claude-code-cli\"",
            "id = \"claude-code-synthetic\"",
            1,
        );
    std::fs::write(forked.join("harness.toml"), text).unwrap();

    let c = connector::load_harness(tmp.path(), "claude-code-synthetic", true)
        .expect("an unpinned connector loads through --dev");
    assert_eq!(
        goalcup_protocol::pins::pinned_connector_dir_sha256("claude-code-synthetic"),
        None,
        "no build pins a connector it does not ship"
    );
    let strict = harness_pin_gate(&c, false);
    assert_eq!(strict.level, Level::Fail, "{}", strict.detail);
    assert!(strict.fix.contains("--dev"), "{}", strict.fix);
    let dev = harness_pin_gate(&c, true);
    assert_eq!(dev.level, Level::Warn, "{}", dev.detail);
    assert!(dev.detail.contains("--dev"), "{}", dev.detail);
}

/// Discovery over the shipped directory, and the two refusals it can produce.
#[test]
fn discovery_lists_what_ships_and_refuses_a_connector_that_does_not() {
    let dir = support::connectors_dir();
    let ids: Vec<String> = connector::harness_dirs(&dir)
        .unwrap()
        .iter()
        .map(|d| d.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(ids, ["claude-code-cli", "codex-cli", "hermes-cli"]);
    for id in &ids {
        let c = load_harness(id);
        assert_eq!(&c.loaded.manifest.id, id);
        assert!(
            c.dir.join("harness.toml").is_file(),
            "`dir` is {id}'s own directory, which is where its files/ live"
        );
        assert!(
            !c.dir.join("manifest.toml").exists(),
            "{id}: no V1 manifest ships (OS-29)"
        );
    }
    // "pick a connector this build ships" is rung 1, the rung `PreflightFailed` gets.
    let (code, said) = match connector::load_harness(&dir, "cursor-cli", false) {
        Ok(c) => (0, c.loaded.manifest.id),
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
    let (code, said) = match connector::load_harness(&dir, traversal, false) {
        Ok(c) => (0, c.loaded.manifest.id),
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
/// `goalcup doctor` would report one twin while `goalcup start` ran whichever the sort
/// happened to reach first. [`connector::load_harness`] refuses instead.
#[test]
fn an_id_two_directories_declare_is_refused_by_name_and_by_both_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let text = std::fs::read_to_string(support::connectors_dir().join("codex-cli/harness.toml"))
        .unwrap()
        .replacen("id = \"codex-cli\"", "id = \"codex-dup\"", 1);
    // Two directories whose manifests declare the same id, which is exactly what a
    // participant produces by copying a connector directory and editing nothing.
    for folder in ["codex-dup", "codex-copy"] {
        std::fs::create_dir(tmp.path().join(folder)).unwrap();
        std::fs::write(tmp.path().join(folder).join("harness.toml"), &text).unwrap();
    }

    let (code, said) = match connector::load_harness(tmp.path(), "codex-dup", true) {
        Ok(c) => (0, format!("loaded {}", c.dir.display())),
        Err(e) => (e.exit_code(), e.to_string()),
    };
    assert_eq!(
        code, 1,
        "an ambiguous id is a refusal and not a silent choice: {said}"
    );
    for named in ["codex-dup", "codex-copy"] {
        assert!(
            said.contains(&tmp.path().join(named).display().to_string()),
            "the refusal names {named}'s path so the twin can be found: {said}"
        );
    }
}

/// `pin-manifests.sh` copied beside a copy of the shipped connectors, run with `args`.
#[cfg(unix)]
fn run_pinner(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    let scripts = root.join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/pin-manifests.sh"),
        scripts.join("pin-manifests.sh"),
    )
    .unwrap();
    std::process::Command::new("sh")
        .arg(scripts.join("pin-manifests.sh"))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
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
    std::fs::copy(&harness, twin.join("harness.toml")).unwrap();

    let output = run_pinner(tmp.path(), &["--print"]);
    assert!(!output.status.success(), "duplicate ids must refuse");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("duplicate declared connector id"),
        "{stderr}"
    );
    assert!(stderr.contains(hyphen_id), "{stderr}");
}

/// A connector folder without a `harness.toml` — a leftover V1 folder holding only a
/// `manifest.toml`, say — is refused by the pinner in both modes, and the rewrite mode leaves
/// the committed table untouched rather than writing one that silently lacks that connector.
#[cfg(unix)]
#[test]
fn the_pinner_refuses_a_folder_without_a_harness_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let connectors = tmp.path().join("connectors");
    support::copy_connectors_into(&connectors);
    let v1_only = connectors.join("v1-only");
    std::fs::create_dir(&v1_only).unwrap();
    std::fs::write(v1_only.join("manifest.toml"), "id = \"v1-only\"\n").unwrap();
    let table = tmp
        .path()
        .join("crates/goalcup-protocol/pins/connector-dirs.txt");
    std::fs::create_dir_all(table.parent().unwrap()).unwrap();
    std::fs::write(&table, "sentinel\n").unwrap();

    for args in [&["--print"][..], &[][..]] {
        let output = run_pinner(tmp.path(), args);
        assert!(
            !output.status.success(),
            "{args:?}: a folder without harness.toml must refuse"
        );
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("holds no harness.toml"),
            "{args:?}: {stderr}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&table).unwrap(),
        "sentinel\n",
        "a refused rewrite writes no table"
    );
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
fn connectors_dir_finds_the_shipped_connectors_beside_the_binary_it_was_run_from() {
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
#[ignore = "run by connectors_dir_finds_the_shipped_connectors_beside_the_binary_it_was_run_from"]
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
    for id in ["claude-code-cli", "codex-cli", "hermes-cli"] {
        goalcup_cli::connector::load_harness(&resolved, id, false)
            .unwrap_or_else(|e| panic!("and what it found is the shipped rulebook: {id}: {e}"));
    }
    println!("{CHILD_AGREED}");
}
