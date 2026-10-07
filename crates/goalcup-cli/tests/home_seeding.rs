//! A per-run harness home that is not a first-run one, and a `--print-launch` that really
//! does run nothing.
//!
//! # The home seed
//!
//! `connectors/claude-code-cli/manifest.toml` pins `CLAUDE_CONFIG_DIR = {harness_home}` and
//! the daemon mints that directory per entry, so Claude Code would meet a brand-new user on
//! every start: a theme picker, then "Select login method". The connector declares
//! `[home_seed]`, and `goalcup start` copies exactly those keys out of the participant's own
//! configuration before the spawn — a READ of an app-wide file, never a write, which is why
//! D21's list of permitted app-wide WRITES is untouched by it.
//!
//! The key list covers the two prompts a first full loop meets: the fullscreen-renderer
//! upsell and "Claude in Chrome extension detected". The second takes the keystrokes of a
//! typed turn, so a run would deliver NO message and exit 0 with the transcript unchanged.
//!
//! # `--print-launch`
//!
//! `--print-launch` is documented "run nothing". It still creates what the rendering needs —
//! a launch line rendered against invented values would be a line that does not work — and
//! then abandons the entry it rendered from. `goalcup resume --print-launch` holds the same
//! contract: it prints the line and runs nothing after it — no seed, no spawn, no
//! registration.
//!
//! # How the seed is written
//!
//! * The seeded document is written through a temp file created `0600` and renamed over the
//!   target — never in place — so it is never on disk under any other mode and a process
//!   killed mid-write never truncates the harness's own live document.
//! * The source read is bounded at 64 MiB; a candidate over that is a refusal naming the
//!   size, not a load of whatever is there.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_cli::commands::start::start_with;
use serde_json::Value;

/// Read a JSON object off disk, or fail naming the path.
fn json_at(path: &std::path::Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The keys the manifest names are copied in, the keys it does not name are not, whatever
/// was already in the per-run home survives, and the participant's own file is left exactly
/// as it was found.
///
/// The survival clause is the resume case, asserted here because it is the same function:
/// the second launch of a run writes into a home the harness has been living in, and a seed
/// that replaced the document would take its project entry and its history with it.
#[test]
fn the_per_run_home_is_seeded_from_the_participants_own_file_and_nothing_is_written_back() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let home = support::home_with_a_signed_in_harness(tmp.path());
    let source = home.join(".claude.json");
    let before = std::fs::read(&source).unwrap();

    // Something the harness itself would have written, from an earlier half of the run.
    let seeded_path = std::path::Path::new(&m.info().harness_home).join(".claude.json");
    std::fs::write(&seeded_path, r#"{"history":["an earlier turn"]}"#).unwrap();

    let mut o = support::start_options("claude-code-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_exits("claude-code-cli");
    let mut out = Vec::new();
    start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_home(&home),
        &o,
        &mut out,
    )
    .unwrap();

    let seeded = json_at(&seeded_path);
    assert_eq!(
        seeded["hasCompletedOnboarding"],
        Value::Bool(true),
        "without it the first screen is the onboarding flow"
    );
    assert_eq!(
        seeded["oauthAccount"]["emailAddress"], "participant@example.test",
        "without it the second screen is the login wall"
    );
    assert_eq!(
        seeded["theme"], "dark",
        "and the theme picker never appears"
    );
    // The two prompts that come after those three. The fullscreen upsell
    // restarts the UI in-process mid-run; the Claude in Chrome one takes a typed turn's
    // keystrokes and the run delivers no message at all.
    assert_eq!(
        seeded["fullscreenUpsellSeenCount"],
        serde_json::json!(3),
        "the count carries across, so \"Try the new fullscreen renderer?\" is not asked again"
    );
    for key in [
        "hasCompletedClaudeInChromeOnboarding",
        "claudeInChromeDefaultEnabled",
        "cachedChromeExtensionInstalled",
    ] {
        assert_eq!(
            seeded[key],
            Value::Bool(true),
            "`{key}` is the participant's own answer to \"Claude in Chrome extension \
             detected\", and a fresh home without it is asked again"
        );
    }
    assert!(
        seeded["projects"].get("/somewhere/else").is_none(),
        "`projects` is in the source file and NOT in `[home_seed] keys`: the seed copies the \
         manifest's list and not everything it finds, and the participant's own directories \
         are never carried across"
    );
    assert_eq!(
        seeded["history"],
        serde_json::json!(["an earlier turn"]),
        "the document already in the per-run home is merged into, never replaced — which is \
         what a resume writes into"
    );

    // The one thing written that is not a copy: THIS run's own workspace, marked trusted.
    // Claude Code asks its safety question per project, about a path that did not exist when
    // the participant's file was written, and its default answer is `No, exit`.
    let projects = seeded["projects"]
        .as_object()
        .expect("the trust table is an object");
    let ws = m.info().workspace;
    let canonical = std::fs::canonicalize(&ws)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ws.clone());
    assert_eq!(
        projects[&canonical]["hasTrustDialogAccepted"],
        Value::Bool(true),
        "the tracker's own empty workspace for this run is trusted: {projects:?}"
    );
    assert!(
        projects.keys().all(|k| *k == ws || *k == canonical),
        "and NO other directory is named there — the tracker vouches for the one folder it \
         created itself: {projects:?}"
    );

    assert_eq!(
        std::fs::read(&source).unwrap(),
        before,
        "the participant's own configuration was READ; D21 lists the app-wide WRITES and \
         this is not one of them"
    );
    let printed = String::from_utf8(out).unwrap();
    assert!(
        printed.contains(&source.display().to_string())
            && printed.contains(&seeded_path.display().to_string())
            && printed.contains("READ"),
        "the participant is told which file was read and where it went: {printed}"
    );
    assert!(
        printed.contains("trusted"),
        "…and that the run's own workspace was marked trusted: {printed}"
    );
}

/// A machine that has never signed the harness in is refused, and refused BEFORE a harness
/// is spawned — the entry it needed is abandoned and nothing is left behind.
///
/// The alternative is to launch, and sit at a login screen that no unattended run can answer,
/// holding an armed entry until its deadline.
#[test]
fn a_machine_that_has_never_signed_in_is_refused_and_leaves_no_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    // Onboarded, and signed out: the one key that matters is missing.
    let home = tmp.path().join("signed-out-home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join(".claude.json"),
        r#"{"hasCompletedOnboarding":true,"theme":"dark"}"#,
    )
    .unwrap();

    let mut o = support::start_options("claude-code-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_exits("claude-code-cli");
    let err = start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_home(&home),
        &o,
        &mut Vec::new(),
    )
    .expect_err("a run that would meet a login wall is refused");

    assert_eq!(err.exit_code(), 1, "the participant can fix it");
    let said = format!("{err}");
    assert!(
        said.contains("oauthAccount") && said.contains("log in"),
        "the refusal says which key is missing and what to do: {said}"
    );
    assert!(
        !m.calls().iter().any(|c| c == "register_process"),
        "nothing was spawned and nothing was handed over: {:?}",
        m.calls()
    );
    assert_eq!(m.abandoned().len(), 1, "the entry it needed is unmade");
    assert!(
        !goalcup_cli::entries::entry_path(tmp.path(), &m.info().entry_id).exists(),
        "and this machine's note of it goes with it"
    );
}

/// `--print-launch` prints the line and leaves nothing behind.
///
/// It cannot print without an entry — the line names the workspace, the artifact root, the
/// receiver token and the session the daemon minted — so it makes one, prints from it, and
/// abandons it. `goalcup start --help` says the flag runs nothing, and this is what that has
/// to mean.
#[test]
fn print_launch_abandons_the_entry_it_rendered_from_and_seeds_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let home = support::home_with_a_signed_in_harness(tmp.path());
    let seeded_path = std::path::Path::new(&m.info().harness_home).join(".claude.json");

    let mut o = support::start_options("claude-code-cli");
    o.print_launch = true;
    let mut out = Vec::new();
    let outcome = start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_home(&home),
        &o,
        &mut out,
    )
    .unwrap();

    let printed = String::from_utf8(out).unwrap();
    assert!(
        printed.contains("--session-id"),
        "the launch line was printed: {printed}"
    );
    assert!(
        printed.contains("abandoned"),
        "and the participant is told the entry went: {printed}"
    );
    assert_eq!(
        m.abandoned(),
        vec![(
            outcome.entry_id.clone(),
            "`goalcup start --print-launch` printed the launch line and ran nothing".to_string()
        )],
        "one abandon, naming why"
    );
    assert!(
        !goalcup_cli::entries::entry_path(tmp.path(), &outcome.entry_id).exists(),
        "no cache file is left for a run nobody started"
    );
    assert!(
        !m.calls().iter().any(|c| c == "register_process"),
        "and nothing was spawned: {:?}",
        m.calls()
    );
    assert!(
        !seeded_path.exists(),
        "a flag that runs nothing seeds nothing: the home it would have written into is \
         about to be abandoned"
    );
}

/// The version the preflight MEASURED is what the daemon is given.
///
/// The daemon runs no version command of its own — `detection.version_command` is the CLI's
/// to run, and the CLI has just run it to decide whether the machine is inside D12's tested
/// range at all. Without it the entry's signed status would say `harness: unknown` for a
/// binary `goalcup doctor` had just measured.
///
/// `--print-launch`, because that reaches `create_workspace` — which is where the number
/// travels — without launching a harness that would take this test's terminal.
#[test]
fn the_version_the_preflight_measured_is_what_the_daemon_is_given() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let home = support::home_with_a_signed_in_harness(tmp.path());

    // A `claude` on PATH that answers `--version` with 2.1.259, inside the shipped range.
    let bin = support::fake_claude_on_path(tmp.path());
    let mut env = support::clean_env_with_home(&home);
    env.insert("PATH".to_string(), bin.display().to_string());

    let mut o = support::start_options("claude-code-cli");
    o.print_launch = true;
    start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &env,
        &o,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        m.last_workspace_opts().harness_version.as_deref(),
        Some("2.1.259"),
        "the number the preflight printed is the number the daemon is told"
    );

    // …and a machine with no harness installed sends `None`, not an empty string. An empty
    // string is a measurement of nothing dressed as one.
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let home = support::home_with_a_signed_in_harness(tmp.path());
    let mut o = support::start_options("claude-code-cli");
    o.print_launch = true;
    start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_home(&home),
        &o,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        m.last_workspace_opts().harness_version,
        None,
        "`claude` is not on this PATH, so nothing was measured and nothing is claimed"
    );
}

/// `codex-cli` seeds the per-run `CODEX_HOME` with the participant's own
/// `auth.json`, whole, `0600`, and never writes anything back.
///
/// `CODEX_HOME` is the per-run home the daemon mints, and Codex keeps its credential in
/// `$CODEX_HOME/auth.json`, so without the copy a run stops at the sign-in screen. No Codex
/// process is started here — this is the copy, asserted on its own.
#[test]
fn the_codex_credential_is_copied_whole_never_overwritten_and_never_written_back() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let harness_home = tmp.path().join("run-home");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        home.join(".codex/auth.json"),
        b"{\"tokens\":\"the participant's\"}",
    )
    .unwrap();
    // A file the seed must not touch, in the same directory it reads from.
    std::fs::write(home.join(".codex/config.toml"), b"model = \"gpt-5\"\n").unwrap();

    let manifest = support::connector("codex-cli").manifest;
    let spec = manifest
        .home_seed
        .as_ref()
        .expect("codex-cli seeds its home");
    assert!(spec.keys.is_empty(), "this connector merges no keys");
    assert_eq!(spec.files, vec!["auth.json".to_string()]);

    let seeded = goalcup_cli::home_seed::seed("codex-cli", spec, &home, &harness_home, &workspace)
        .expect("it seeds");
    let written = harness_home.join("auth.json");
    assert_eq!(
        std::fs::read(&written).unwrap(),
        std::fs::read(home.join(".codex/auth.json")).unwrap(),
        "the whole file, verbatim"
    );
    assert_eq!(seeded.files.len(), 1);
    assert!(seeded.missing.is_empty() && seeded.kept.is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&written).unwrap().permissions().mode() & 0o777,
            0o600,
            "it is a credential"
        );
    }
    assert!(
        !harness_home.join("config.toml").exists(),
        "only the declared file is copied"
    );
    let line = seeded.line();
    assert!(
        line.contains("READ"),
        "the line says which way the bytes went: {line}"
    );
    assert!(line.contains("0600"), "{line}");

    // A resume finds it already there — the harness may have refreshed it — and leaves it.
    std::fs::write(&written, b"{\"tokens\":\"refreshed by the harness\"}").unwrap();
    let again = goalcup_cli::home_seed::seed("codex-cli", spec, &home, &harness_home, &workspace)
        .expect("it re-seeds");
    assert_eq!(
        std::fs::read_to_string(&written).unwrap(),
        "{\"tokens\":\"refreshed by the harness\"}",
        "a newer credential the harness wrote is not replaced by an older one"
    );
    assert_eq!(again.kept.len(), 1);
    assert!(again.files.is_empty());

    // And a machine with no credential is a LINE, not a refusal: the sign-in screen is one
    // a person can still answer, and `CODEX_HOME=<entry home> codex login` is the other way.
    let bare = tmp.path().join("bare-home");
    std::fs::create_dir_all(bare.join(".codex")).unwrap();
    let missing = goalcup_cli::home_seed::seed("codex-cli", spec, &bare, &harness_home, &workspace)
        .expect("a missing credential is not a refusal");
    assert_eq!(missing.missing.len(), 1);
    assert!(
        missing.line().contains("Not on this machine"),
        "{}",
        missing.line()
    );

    // Nothing was written under the participant's own home, by either call.
    let mut left: Vec<String> = std::fs::read_dir(home.join(".codex"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec!["auth.json".to_string(), "config.toml".to_string()]
    );
}

/// A declared credential is an account file, not an arbitrary path the launcher may be
/// tricked into reading.  In particular, a symlink at the declared file name must not turn
/// `[home_seed] files = ["auth.json"]` into a copy of a file elsewhere on disk.
#[cfg(unix)]
#[test]
fn a_symlinked_declared_credential_is_refused_without_copying_its_target() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let harness_home = tmp.path().join("run-home");
    let workspace = tmp.path().join("ws");
    let outside = tmp.path().join("outside-auth.json");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(&outside, b"outside-the-declared-codex-config").unwrap();
    std::os::unix::fs::symlink(&outside, home.join(".codex/auth.json")).unwrap();

    let manifest = support::connector("codex-cli").manifest;
    let spec = manifest
        .home_seed
        .as_ref()
        .expect("codex-cli seeds its home");
    let err = goalcup_cli::home_seed::seed("codex-cli", spec, &home, &harness_home, &workspace)
        .expect_err("a source symlink must not be followed");

    assert!(
        format!("{err}").contains("symbolic link"),
        "the refusal identifies the unsafe source shape: {err}"
    );
    assert!(
        !harness_home.join("auth.json").exists(),
        "the symlink target was not copied into the run home"
    );
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"outside-the-declared-codex-config",
        "the unrelated target was never read into a tracker output or changed"
    );
}

/// The source directory is part of the connector declaration too.  A symlinked
/// `{home}/.codex` must not make a normal `auth.json` in another directory eligible for a
/// seed copy.
#[cfg(unix)]
#[test]
fn a_symlinked_declared_config_directory_is_refused_without_copying_its_file() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let harness_home = tmp.path().join("run-home");
    let workspace = tmp.path().join("ws");
    let outside = tmp.path().join("outside-config");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let outside_auth = outside.join("auth.json");
    std::fs::write(&outside_auth, b"outside-the-declared-codex-directory").unwrap();
    std::os::unix::fs::symlink(&outside, home.join(".codex")).unwrap();

    let manifest = support::connector("codex-cli").manifest;
    let spec = manifest
        .home_seed
        .as_ref()
        .expect("codex-cli seeds its home");
    let err = goalcup_cli::home_seed::seed("codex-cli", spec, &home, &harness_home, &workspace)
        .expect_err("a source directory symlink must not be followed");

    assert!(
        format!("{err}").contains("symbolic link"),
        "the refusal identifies the unsafe source directory: {err}"
    );
    assert!(
        !harness_home.join("auth.json").exists(),
        "the file in the linked directory was not copied into the run home"
    );
    assert_eq!(
        std::fs::read(&outside_auth).unwrap(),
        b"outside-the-declared-codex-directory",
        "the unrelated config file was not copied or changed"
    );
}

/// The pathname selected by `[home_seed]` is not a capability. Once its directory has been
/// acquired, replacing that pathname with a link must not retarget the credential read.
#[cfg(unix)]
#[test]
fn an_intermediate_source_directory_swap_cannot_retarget_the_auth_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let harness_home = tmp.path().join("run-home");
    let workspace = tmp.path().join("ws");
    let outside = tmp.path().join("outside-config");
    let detached = tmp.path().join("detached-codex");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(home.join(".codex/auth.json"), b"inside-the-declared-config").unwrap();
    std::fs::write(outside.join("auth.json"), b"outside-the-declared-config").unwrap();

    let manifest = support::connector("codex-cli").manifest;
    let spec = manifest
        .home_seed
        .as_ref()
        .expect("codex-cli seeds its home");
    let source_dir = home.join(".codex");
    let replacement_source_dir = source_dir.clone();
    let replacement_outside = outside.clone();
    let _hook = goalcup_cli::home_seed::replace_declared_source_directory_after_acquire_for_tests(
        source_dir.clone(),
        move || {
            std::fs::rename(&replacement_source_dir, &detached)
                .expect("detach the acquired source directory");
            std::os::unix::fs::symlink(&replacement_outside, &replacement_source_dir)
                .expect("replace its pathname with an outside directory link");
        },
    );

    let seeded = goalcup_cli::home_seed::seed("codex-cli", spec, &home, &harness_home, &workspace)
        .expect("the retained source directory remains readable");
    assert_eq!(
        std::fs::read(harness_home.join("auth.json")).unwrap(),
        b"inside-the-declared-config",
        "the post-acquisition path replacement cannot redirect the copied credential"
    );
    assert_eq!(seeded.files.len(), 1);
    assert_eq!(
        std::fs::read(outside.join("auth.json")).unwrap(),
        b"outside-the-declared-config",
        "the outside file remains only an attempted redirection target"
    );
}

/// A resume receives the harness-home spelling from the daemon, but the spelling is not
/// write authority. If that directory is exchanged after the CLI acquires it, neither a
/// whole credential copy nor the JSON merge may follow the replacement outside the entry.
#[cfg(unix)]
#[test]
fn a_swapped_destination_home_is_refused_without_writing_outside_the_entry() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let harness_home = tmp.path().join("run-home");
    let detached = tmp.path().join("detached-run-home");
    let outside = tmp.path().join("outside-home");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(home.join(".codex/auth.json"), b"inside credential").unwrap();
    std::fs::write(outside.join("sentinel.txt"), b"outside").unwrap();

    let manifest = support::connector("codex-cli").manifest;
    let spec = manifest
        .home_seed
        .as_ref()
        .expect("codex-cli seeds its home");
    let swapped_home = harness_home.clone();
    let outside_target = outside.clone();
    let _hook = goalcup_cli::home_seed::replace_destination_home_after_acquire_for_tests(
        harness_home.clone(),
        move || {
            std::fs::rename(&swapped_home, &detached).expect("detach the acquired run home");
            std::os::unix::fs::symlink(&outside_target, &swapped_home)
                .expect("replace the run-home spelling with an outside link");
        },
    );

    let err = goalcup_cli::home_seed::seed("codex-cli", spec, &home, &harness_home, &workspace)
        .expect_err("a replaced destination authority must be refused");
    assert!(
        format!("{err}").contains("destination"),
        "the refusal identifies the unsafe destination: {err}"
    );
    assert_eq!(
        std::fs::read(outside.join("sentinel.txt")).unwrap(),
        b"outside"
    );
    assert!(
        !outside.join("auth.json").exists(),
        "the replacement home never receives the credential"
    );
}

/// The selected home can keep its own name while an earlier path component is exchanged.
/// Revalidating the selected directory identity catches that ancestor swap as well.
#[cfg(unix)]
#[test]
fn a_swapped_destination_ancestor_is_refused_without_writing_outside_the_entry() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let container = tmp.path().join("entry-artifacts");
    let harness_home = container.join("run-home");
    let detached = tmp.path().join("detached-entry-artifacts");
    let outside_container = tmp.path().join("outside-artifacts");
    let outside_home = outside_container.join("run-home");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&outside_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(home.join(".codex/auth.json"), b"inside credential").unwrap();
    std::fs::write(outside_home.join("sentinel.txt"), b"outside").unwrap();

    let manifest = support::connector("codex-cli").manifest;
    let spec = manifest
        .home_seed
        .as_ref()
        .expect("codex-cli seeds its home");
    let swapped_container = container.clone();
    let replacement_container = outside_container.clone();
    let _hook = goalcup_cli::home_seed::replace_destination_home_after_acquire_for_tests(
        harness_home.clone(),
        move || {
            std::fs::rename(&swapped_container, &detached)
                .expect("detach the acquired home through its ancestor");
            std::os::unix::fs::symlink(&replacement_container, &swapped_container)
                .expect("replace the destination ancestor with an outside link");
        },
    );

    goalcup_cli::home_seed::seed("codex-cli", spec, &home, &harness_home, &workspace)
        .expect_err("a replaced destination ancestor must be refused");
    assert_eq!(
        std::fs::read(outside_home.join("sentinel.txt")).unwrap(),
        b"outside"
    );
    assert!(!outside_home.join("auth.json").exists());
}

/// The merged document is written through a temp file created `0600`
/// and renamed over the target, never in place.
///
/// Two things follow from that and are both asserted here. First, the file is `0600` from
/// its own creation — there is no separate `chmod` call left in the path for a window to
/// open around. Second, `rename(2)` retargets the directory entry itself rather than
/// following it, so a symlink planted at the destination — by a participant's home, or by
/// an earlier half of the same run — is REPLACED, and whatever it pointed at is left alone.
#[cfg(unix)]
#[test]
fn the_seeded_document_is_0600_from_creation_and_a_planted_symlink_is_replaced_not_followed() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let home = support::home_with_a_signed_in_harness(tmp.path());
    let harness_home = tmp.path().join("run-home");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();

    // Somewhere else entirely: the file a symlink AT the destination would point through if
    // this write followed it instead of replacing it.
    let elsewhere = tmp.path().join("elsewhere.json");
    std::fs::write(&elsewhere, b"{\"not\":\"the tracker's to touch\"}").unwrap();
    let to = harness_home.join(".claude.json");
    std::os::unix::fs::symlink(&elsewhere, &to).unwrap();

    let spec = support::connector("claude-code-cli")
        .manifest
        .home_seed
        .expect("claude-code-cli seeds its home");
    goalcup_cli::home_seed::seed("claude-code-cli", &spec, &home, &harness_home, &workspace)
        .expect("it seeds");

    let meta = std::fs::symlink_metadata(&to).unwrap();
    assert!(
        !meta.file_type().is_symlink(),
        "the symlink at the destination was replaced, not written through"
    );
    assert_eq!(
        meta.permissions().mode() & 0o777,
        0o600,
        "0600 from the file's own creation, never a `chmod` after the fact"
    );
    assert_eq!(
        std::fs::read(&elsewhere).unwrap(),
        b"{\"not\":\"the tracker's to touch\"}",
        "and the file the symlink pointed at is untouched"
    );
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&to).unwrap()).unwrap();
    assert_eq!(
        written["oauthAccount"]["emailAddress"], "participant@example.test",
        "and the seed itself still ran: {written}"
    );
}

/// A candidate over 64 MiB is refused by size, naming the path, and
/// is never read whole first.
///
/// `set_len` makes the file sparse — no bytes are actually written to disk for it — so this
/// stays fast while still being a file the filesystem reports as larger than the cap.
#[test]
fn a_source_file_over_the_cap_is_refused_by_size_and_never_read_whole() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let harness_home = tmp.path().join("run-home");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();

    let oversized = home.join(".claude.json");
    let file = std::fs::File::create(&oversized).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(file);

    let spec = support::connector("claude-code-cli")
        .manifest
        .home_seed
        .expect("claude-code-cli seeds its home");
    let err =
        goalcup_cli::home_seed::seed("claude-code-cli", &spec, &home, &harness_home, &workspace)
            .expect_err("a source over the cap is refused rather than read");
    let msg = format!("{err}");
    assert!(
        msg.contains(&oversized.display().to_string()),
        "names the oversized file: {msg}"
    );
    assert!(
        msg.contains("64") && msg.contains("larger"),
        "and says why: {msg}"
    );
}

/// The verbatim credential copy has the same source boundary as the JSON-object merge. An
/// oversized source must be rejected before a destination credential is replaced.
#[test]
fn a_raw_home_seed_file_at_the_cap_copies_but_cap_plus_one_preserves_the_destination() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let harness_home = tmp.path().join("run-home");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let source = home.join(".codex/auth.json");
    let destination = harness_home.join("auth.json");
    let spec = support::connector("codex-cli")
        .manifest
        .home_seed
        .expect("codex copies its declared raw credential");

    std::fs::write(&source, b"{}").unwrap();
    goalcup_cli::home_seed::seed("codex-cli", &spec, &home, &harness_home, &workspace)
        .expect("a regular small raw source copies");
    assert_eq!(std::fs::read(&destination).unwrap(), b"{}");

    std::fs::remove_file(&destination).unwrap();
    let source_file = std::fs::File::create(&source).unwrap();
    source_file.set_len(64 * 1024 * 1024).unwrap();
    drop(source_file);
    goalcup_cli::home_seed::seed("codex-cli", &spec, &home, &harness_home, &workspace)
        .expect("a raw source exactly at the cap remains accepted");
    assert_eq!(
        std::fs::metadata(&destination).unwrap().len(),
        64 * 1024 * 1024
    );

    std::fs::remove_file(&destination).unwrap();
    let source_file = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap();
    source_file.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(source_file);
    let error = goalcup_cli::home_seed::seed("codex-cli", &spec, &home, &harness_home, &workspace)
        .expect_err("a raw source beyond the cap is refused before it is allocated or copied");
    assert!(format!("{error}").contains("larger"));
    assert!(
        !destination.exists(),
        "the oversized source never allocates a replacement destination"
    );
}

/// `goalcup resume --print-launch` asks `resume_preview`, then only prints the line.
///
/// It must not catch capture up or clear the registered process handle, and it leaves both
/// credentials and the local cache unchanged. Like `goalcup start --print-launch`, it never
/// seeds, launches, or registers a harness — but unlike start's preview, it retains the
/// existing entry exactly as it was.
#[test]
fn print_launch_on_resume_seeds_nothing_spawns_nothing_and_registers_nothing() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_resumable(&tmp, "claude-code-cli");
    let entry_id = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    support::set_cached_connector(&cfg, &entry_id, "claude-code-cli");
    let seeded_path = std::path::Path::new(&m.resume_info().harness_home).join(".claude.json");
    let entry_path = goalcup_cli::entries::entry_path(&cfg, &entry_id);
    let cached_before = std::fs::read(&entry_path).expect("the existing entry cache");

    let mut o = support::resume_options();
    o.print_launch = true;
    let mut out = Vec::new();
    goalcup_cli::commands::resume::resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("claude-code-cli"),
        &cfg,
        &entry_id,
        &o,
        &mut out,
    )
    .unwrap();

    let printed = String::from_utf8(out).unwrap();
    assert!(
        printed.contains("--resume"),
        "the launch line was printed: {printed}"
    );
    assert!(
        !m.calls().iter().any(|c| c == "resume"),
        "print-launch must not call the mutating resume endpoint: {:?}",
        m.calls()
    );
    assert!(
        m.calls().iter().any(|c| c == "resume_preview"),
        "print-launch reads only the dedicated preview pins: {:?}",
        m.calls()
    );
    assert!(
        !printed.contains("caught up"),
        "a preview never claims it caught capture up: {printed}"
    );
    assert!(
        !m.calls().iter().any(|c| c == "register_process"),
        "print-launch runs nothing after the print: {:?}",
        m.calls()
    );
    assert!(
        !seeded_path.exists(),
        "and it seeds nothing: a flag documented as printing a line does not write a \
         participant's credentials into the run home"
    );
    assert_eq!(
        std::fs::read(&entry_path).expect("the entry cache remains"),
        cached_before,
        "a preview must not rewrite the entry cache"
    );
    assert_eq!(
        m.calls(),
        vec!["resume_preview"],
        "preview calls no mutating daemon method and does not reach status/browser work"
    );
}

/// The seeding builds the record the chain carries, and that record holds the
/// participant's DECISIONS and never their documents.
///
/// The value rule lives in `goalcup_protocol::events::SeededKey::new` and is asserted there;
/// what this asserts is that the seeding applies it to the real connector's real key list —
/// `claudeInChromeDefaultEnabled` (the browser-tools switch, a boolean) carries its value,
/// `oauthAccount` is an object holding an account and is named only — and that `from` is the
/// manifest's spelling rather than a path naming this machine.
#[test]
fn the_seeding_records_the_decisions_it_carried_and_never_the_account() {
    let tmp = tempfile::tempdir().unwrap();
    let home = support::home_with_a_signed_in_harness(tmp.path());
    let harness_home = tmp.path().join("run-home");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();

    let spec = support::connector("claude-code-cli")
        .manifest
        .home_seed
        .expect("claude-code-cli seeds its home");
    let seeded =
        goalcup_cli::home_seed::seed("claude-code-cli", &spec, &home, &harness_home, &workspace)
            .expect("it seeds");
    let r = &seeded.record;

    assert_eq!(r.connector, "claude-code-cli");
    assert_eq!(
        r.from, "{home}/.claude.json",
        "the manifest's spelling, never the resolved path"
    );
    assert_eq!(
        r.keys.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(),
        seeded.keys.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
        "the record names exactly the keys the seeding copied"
    );

    let value = |name: &str| {
        r.keys
            .iter()
            .find(|k| k.name == name)
            .unwrap_or_else(|| panic!("{name} was copied"))
            .value
            .clone()
    };
    assert_eq!(
        value("claudeInChromeDefaultEnabled"),
        Some(Value::Bool(true)),
        "the feature switch is a boolean and is the point of the record"
    );
    assert_eq!(value("theme"), Some(Value::String("dark".into())));
    assert_eq!(value("numStartups"), Some(serde_json::json!(7)));
    assert_eq!(
        value("oauthAccount"),
        None,
        "an object holding an account is NAMED and never quoted"
    );

    // …and the serialised record carries no part of that account, which is the property the
    // rule exists for rather than the rule itself.
    let text = serde_json::to_string(r).unwrap();
    assert!(
        !text.contains("participant@example.test"),
        "the record carries an address out of the participant's file: {text}"
    );
    assert!(
        !text.contains("/somewhere/else"),
        "and nothing the manifest did not name: {text}"
    );

    assert!(
        r.keys_unchanged.is_empty(),
        "a first seeding held nothing already"
    );
    assert!(r.files.is_empty() && r.files_kept.is_empty() && r.files_missing.is_empty());

    // The one entry the tracker DECIDED rather than copied, on a list of its own so a reader
    // of somebody else's trace can tell the two apart — and without the workspace path,
    // which names a directory on this machine.
    assert_eq!(
        r.written
            .iter()
            .map(|k| (k.name.clone(), k.value.clone()))
            .collect::<Vec<_>>(),
        spec.trust_keys
            .iter()
            .map(|k| (k.clone(), Some(Value::Bool(true))))
            .collect::<Vec<_>>(),
        "the workspace trust write is recorded as the tracker's own"
    );
    assert!(
        !seeded.trusted.is_empty(),
        "the seeding did make that write, which is what the record above claims"
    );
    let written_text = serde_json::to_string(&r.written).unwrap();
    assert!(
        !written_text.contains(&workspace.display().to_string()),
        "the workspace path is not in the record: {written_text}"
    );

    // The resume seeds again into a home that already holds exactly what was written, so the
    // record says so: every key already held, and the merge changed nothing.
    let again =
        goalcup_cli::home_seed::seed("claude-code-cli", &spec, &home, &harness_home, &workspace)
            .expect("it re-seeds");
    assert_eq!(
        again.record.keys_unchanged, seeded.keys,
        "on a resume every declared key is already held at the value being written, so the \
         merge changed nothing — each key was written again, identically"
    );
}

/// A seeded credential is recorded by name, length and
/// digest — never by its bytes — and a resume that leaves one in place records it as kept.
#[test]
fn a_seeded_credential_is_recorded_by_digest_and_never_by_its_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    let harness_home = tmp.path().join("run-home");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&harness_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let secret = b"{\"tokens\":\"the participant's\"}";
    std::fs::write(home.join(".codex/auth.json"), secret).unwrap();

    let manifest = support::connector("codex-cli").manifest;
    let spec = manifest
        .home_seed
        .as_ref()
        .expect("codex-cli seeds its home");
    let seeded = goalcup_cli::home_seed::seed("codex-cli", spec, &home, &harness_home, &workspace)
        .expect("it seeds");
    let r = &seeded.record;

    assert_eq!(r.connector, "codex-cli");
    assert_eq!(
        r.from, "{home}/.codex",
        "the file clause's source, as the manifest spells it"
    );
    assert!(r.keys.is_empty(), "this connector merges no keys");
    assert_eq!(r.files.len(), 1);
    assert_eq!(r.files[0].name, "auth.json");
    assert_eq!(r.files[0].bytes, secret.len() as u64);
    let text = serde_json::to_string(r).unwrap();
    assert!(
        !text.contains("the participant's"),
        "a credential's CONTENTS never reach the record: {text}"
    );
    assert!(
        !text.contains(&goalcup_verifier::legs::sha256_hex(secret)),
        "nor a digest of them, which would identify the same credential across public \
         traces (fix 8 round 1): {text}"
    );
    assert!(
        r.written.is_empty(),
        "this connector declares no trust field, so the tracker decided nothing"
    );

    // A resume finds it there and leaves it. Named, and not re-digested: the launch that
    // copied it chained its length and digest earlier in the same chain.
    let again = goalcup_cli::home_seed::seed("codex-cli", spec, &home, &harness_home, &workspace)
        .expect("it re-seeds");
    assert!(again.record.files.is_empty());
    assert_eq!(again.record.files_kept, vec!["auth.json".to_string()]);

    // And a machine with no credential names it as missing rather than refusing.
    let bare = tmp.path().join("bare-home");
    std::fs::create_dir_all(bare.join(".codex")).unwrap();
    let none = goalcup_cli::home_seed::seed("codex-cli", spec, &bare, &harness_home, &workspace)
        .expect("a missing credential is not a refusal");
    assert_eq!(none.record.files_missing, vec!["auth.json".to_string()]);
}
