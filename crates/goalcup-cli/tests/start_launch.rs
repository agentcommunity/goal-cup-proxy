//! `goalcup start`'s launch half: the argv each connector's recipe renders to, the pins that
//! reach the child, and the two refusals that happen before anything is spawned.
//!
//! The second half of the file drives the command that uses the plan.

mod support;

#[cfg(feature = "v1-compat")]
use goalcup_cli::launch::{LaunchKind, plan};
#[cfg(feature = "v1-compat")]
use goalcup_cli::launch::{scrubbed_env, shell_line, spawn};
#[cfg(feature = "v1-compat")]
use goalcup_protocol::manifest::LaunchMode;

#[cfg(feature = "v1-compat")]
#[test]
fn each_connector_plans_the_argv_and_the_pins_its_spike_measured() {
    let info = support::workspace_info();
    let v = goalcup_cli::vars::from_workspace(&info);
    let ws = std::path::Path::new(&info.workspace);

    let p = plan(
        &support::connector("claude-code-cli"),
        &v,
        ws,
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap();
    assert_eq!(p.mode, LaunchMode::Tui);
    assert_eq!(
        p.argv,
        vec![
            "--settings".to_string(),
            format!("{}/settings-launch.json", info.artifact_root),
            "--session-id".to_string(),
            info.session_id.clone()
        ]
    );
    assert_eq!(
        p.env_set.get("CLAUDE_CONFIG_DIR").unwrap(),
        &info.harness_home
    );
    assert!(
        p.env_set
            .get("OTEL_LOG_RAW_API_BODIES")
            .unwrap()
            .starts_with("file:")
    );
    assert!(p.env_remove.iter().any(|k| k == "ANTHROPIC_BASE_URL"));
    assert!(
        !p.env_set.keys().any(|k| k.ends_with("_BASE_URL")),
        "D1: no proxy, ever"
    );
    assert_eq!(p.cwd, ws);

    let r = plan(
        &support::connector("claude-code-cli"),
        &v,
        ws,
        LaunchKind::Resume,
        &support::launch_env(),
    )
    .unwrap();
    assert!(r.argv.iter().any(|a| a == "--resume"));
    assert!(
        !r.argv.iter().any(|a| a == "--session-id"),
        "spike 4: --session-id creates, --resume selects; never both"
    );

    let p = plan(
        &support::connector("codex-cli"),
        &v,
        ws,
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap();
    assert!(
        !p.argv.iter().any(|a| a.contains("hook")),
        "the hook flags went with the hook leg: {:?}",
        p.argv
    );
    assert_eq!(p.env_set.get("CODEX_HOME").unwrap(), &info.harness_home);
    let r = plan(
        &support::connector("codex-cli"),
        &v,
        ws,
        LaunchKind::Resume,
        &support::launch_env(),
    )
    .unwrap();
    assert_eq!(r.argv[0], "resume");
    assert_eq!(
        r.argv[1], info.session_id,
        "positional, per SessionArgStyle::Positional"
    );
}

#[cfg(feature = "v1-compat")]
#[test]
fn a_banned_argument_is_refused_before_anything_is_spawned() {
    let info = support::workspace_info();
    let v = goalcup_cli::vars::from_workspace(&info);
    let mut c = support::connector("codex-cli");
    c.manifest.launch.args.push("--ephemeral".into());
    let err = plan(
        &c,
        &v,
        std::path::Path::new(&info.workspace),
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);
    assert!(
        format!("{err}").contains("--ephemeral"),
        "name the banned token"
    );
    // Whole arguments, never substrings.
    let mut c = support::connector("codex-cli");
    c.manifest.launch.args.push("--ephemerally".into());
    assert!(
        plan(
            &c,
            &v,
            std::path::Path::new(&info.workspace),
            LaunchKind::Start,
            &support::launch_env()
        )
        .is_ok()
    );
}

#[cfg(feature = "v1-compat")]
#[test]
fn a_recipe_naming_a_port_it_did_not_get_is_refused_rather_than_started_on_nothing() {
    let mut info = support::workspace_info();
    info.server_port = None;
    let v = goalcup_cli::vars::from_workspace(&info);
    // No shipped connector names `{server_port}`, so the recipe is given one.
    let mut c = support::connector("codex-cli");
    c.manifest
        .launch
        .args
        .extend(["--port".to_string(), "{server_port}".to_string()]);
    let err = plan(
        &c,
        &v,
        std::path::Path::new(&info.workspace),
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "ledger P4: a port that was never reserved is a refusal the participant is told \
         about, not this CLI falling over"
    );
    assert!(format!("{err}").contains("port"));
}

#[cfg(feature = "v1-compat")]
#[test]
fn the_printed_line_is_the_line_that_runs_and_the_child_gets_the_cwd_and_the_environment() {
    let tmp = tempfile::tempdir().unwrap();
    let mut info = support::workspace_info();
    info.workspace = tmp.path().display().to_string();
    let v = goalcup_cli::vars::from_workspace(&info);
    let mut c = support::connector("claude-code-cli");
    c.manifest.launch.program = "sh".into();
    c.manifest.launch.args = vec![
        "-c".into(),
        "printf '%s' \"$CLAUDE_CONFIG_DIR\" > proof.txt".into(),
    ];
    let p = plan(
        &c,
        &v,
        tmp.path(),
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap();
    let mut s = spawn(&p).unwrap();
    assert!(s.child.wait().unwrap().success());
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("proof.txt")).unwrap(),
        info.harness_home,
        "cwd is the workspace and the pins reached the child"
    );
}

/// The one value in a launch environment that is a secret, and the one surface that prints
/// a launch. A manifest may pin a `*_TOKEN`, and `--print-launch` renders it.
///
/// The redaction has to live
/// on `shell_line` and on `LaunchPlan`'s own `Debug` — a `{:?}` in an error path reaches a
/// terminal exactly the way `--print-launch` does, and a redaction applied at one call site
/// is a guarantee that lasts until the second one.
#[cfg(feature = "v1-compat")]
#[test]
fn neither_the_printed_line_nor_a_debug_of_the_plan_carries_the_receiver_token() {
    let info = support::workspace_info();
    let v = goalcup_cli::vars::from_workspace(&info);
    let ws = std::path::Path::new(&info.workspace);
    let id = "codex-cli";
    // No shipped connector pins a token, and a V1 `[launch.env]` pin of one is refused outright
    // (`launch_env_pin_denied`), so the token is planted on the plan itself: the tracker's own
    // launch supplies one this way, and the printers must withhold it.
    let secret = "a-bearer-token-that-must-not-print";
    let c = support::connector(id);
    let mut p = plan(&c, &v, ws, LaunchKind::Start, &support::launch_env()).unwrap();
    p.env_set
        .insert("GOALCUP_TOKEN".to_string(), secret.to_string());
    assert_eq!(
        p.env_set.get("GOALCUP_TOKEN").unwrap(),
        secret,
        "{id} carries the token, so there is something to leak"
    );
    let line = shell_line(&p);
    assert!(
        !line.contains(secret),
        "{id}: --print-launch handed over the bearer: {line}"
    );
    assert!(line.contains("GOALCUP_TOKEN"), "{id}: and it names the pin");
    assert!(
        line.contains("withheld") && line.contains("will not record"),
        "{id}: a withheld secret says it is withheld and says what pasting the line \
         would get you, rather than standing a placeholder where a value goes: {line}"
    );
    assert!(
        !line.contains("GOALCUP_TOKEN="),
        "{id}: and it is not rendered as an assignment at all: {line}"
    );
    let printed = format!("{p:?}");
    assert!(
        !printed.contains(secret),
        "{id}: a `{{:?}}` of the plan handed over the bearer: {printed}"
    );
}

/// `LaunchMode::AttachOnly` is the desktop case: the folder is armed and the participant
/// opens a session in it themselves. There is no argv to build, so there is no plan.
#[cfg(feature = "v1-compat")]
#[test]
fn an_attach_only_connector_builds_no_plan_at_all() {
    let info = support::workspace_info();
    let v = goalcup_cli::vars::from_workspace(&info);
    let mut c = support::connector("claude-code-cli");
    c.manifest.launch.mode = LaunchMode::AttachOnly;
    let err = plan(
        &c,
        &v,
        std::path::Path::new(&info.workspace),
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap_err();
    assert_eq!(
        err.exit_code(),
        3,
        "the caller checks the mode first; reaching here is this CLI's own bug"
    );
    assert!(format!("{err}").contains("attach"));
}

/// No shell is involved in a launch, and that is a claim about the child and not only about
/// the printed line: each element of `argv` becomes one `Command::arg`, so a rendered value
/// carrying a space is one argument and a rendered value carrying a brace is one argument
/// with a brace in it. A workspace path with a space in it is the ordinary case this is
/// about — `~/My Projects/repo` — and a value that reached a harness as two arguments is
/// a run that did something other than what the recipe says.
#[cfg(feature = "v1-compat")]
#[test]
fn a_rendered_space_or_brace_is_one_argument_and_the_printed_line_quotes_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut info = support::workspace_info();
    info.workspace = tmp.path().display().to_string();
    let v = goalcup_cli::vars::from_workspace(&info);
    let mut c = support::connector("claude-code-cli");
    c.manifest.launch.program = "sh".into();
    c.manifest.launch.args = vec![
        "-c".into(),
        r#"printf '%s\n' "$@" > argv.txt"#.into(),
        "dump".into(),
        "one two".into(),
        // `{{` and `}}` are the template's own escapes, so this renders to `{brace} x`.
        "{{brace}} x".into(),
    ];
    let p = plan(
        &c,
        &v,
        tmp.path(),
        LaunchKind::Start,
        &support::launch_env(),
    )
    .unwrap();
    assert_eq!(
        p.argv[3..],
        ["one two".to_string(), "{brace} x".to_string()],
        "one element each, rendered"
    );

    let mut s = spawn(&p).unwrap();
    assert!(s.child.wait().unwrap().success());
    let dumped = std::fs::read_to_string(tmp.path().join("argv.txt")).unwrap();
    assert_eq!(
        dumped.lines().collect::<Vec<_>>(),
        vec!["one two", "{brace} x"],
        "the child received two arguments, not four: {dumped:?}"
    );

    let line = shell_line(&p);
    assert!(
        line.contains("'one two'") && line.contains("'{brace} x'"),
        "the printed line is the shell-bound one, so it quotes what a shell would split: \
         {line}"
    );
}

/// D23's denylist, on the launch side. The preflight refuses a start under one of these and
/// `probe_env` strips it from the version probe; the launch strips it from the harness, so a
/// participant whose shell exports `HTTPS_PROXY` does not run 168 hours through it.
///
/// The parent environment is built here rather than read off this process: what is asserted
/// is which variables survive the scrub, and a test that mutated this process's environment
/// to ask that would be a test the rest of the binary races.
#[cfg(feature = "v1-compat")]
#[test]
fn the_launch_environment_is_scrubbed_of_the_denylist_and_the_child_never_sees_one() {
    let tmp = tempfile::tempdir().unwrap();
    let mut info = support::workspace_info();
    info.workspace = tmp.path().display().to_string();
    let v = goalcup_cli::vars::from_workspace(&info);

    let banned = [
        "ANTHROPIC_BASE_URL",
        "OPENAI_BASE_URL",
        "HTTPS_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "PROXY",
    ];
    let mut parent = support::clean_env();
    for k in banned {
        parent.insert(k.to_string(), "http://elsewhere.invalid".to_string());
    }
    parent.insert("EDITOR".to_string(), "vi".to_string());
    parent.insert(
        "A_PARENT_SECRET".to_string(),
        "sk-not-in-a-debug".to_string(),
    );
    let env = scrubbed_env(&parent);

    let mut c = support::connector("claude-code-cli");
    c.manifest.launch.program = "sh".into();
    c.manifest.launch.args = vec!["-c".into(), "env > dump.txt".into()];
    let p = plan(&c, &v, tmp.path(), LaunchKind::Start, &env).unwrap();
    for k in banned {
        assert!(!p.env_base.contains_key(k), "{k} survived the scrub");
    }
    assert!(
        !format!("{p:?}").contains("sk-not-in-a-debug"),
        "a `{{:?}}` of the plan printed the participant's own environment back"
    );

    let mut s = spawn(&p).unwrap();
    assert!(s.child.wait().unwrap().success());
    let dumped = std::fs::read_to_string(tmp.path().join("dump.txt")).unwrap();
    let names: Vec<&str> = dumped.lines().filter_map(|l| l.split('=').next()).collect();
    for k in banned {
        assert!(!names.contains(&k), "{k} reached the harness: {names:?}");
    }
    assert!(
        names.contains(&"EDITOR") && names.contains(&"PATH"),
        "the rest of the participant's environment is theirs and is carried across: {names:?}"
    );
    assert!(
        names.contains(&"CLAUDE_CONFIG_DIR"),
        "and the connector's own pins are on top of it: {names:?}"
    );
}

// ------------------------------------------------------------- the command

#[cfg(feature = "v1-compat")]
use goalcup_cli::commands::start::start_with;

/// The order is the whole of D22: the daemon can kill from the moment it can, so it learns
/// about the process only once the folder is armed — and the workspace, afterwards, holds
/// exactly what the daemon put in it.
///
/// The connectors directory is `support::connectors_dir_with_a_harness_that_exits`, not the
/// shipped one, and the header of `tests/support/mod.rs` says why: launch mode spawns the
/// recipe and waits for it, and the shipped recipe is `claude` — a TUI that would take this
/// test's terminal on a machine where it is installed and fail the launch on one where it is
/// not. The edit moves the connector off this build's pin, so the run goes through `--dev`,
/// which is the one door and says so on stdout.
#[cfg(feature = "v1-compat")]
#[test]
fn start_creates_arms_registers_and_writes_nothing_into_the_workspace_itself() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let mut out = Vec::new();
    let mut o = support::start_options("claude-code-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_exits("claude-code-cli");
    let outcome = start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &o,
        &mut out,
    )
    .unwrap();

    let calls = m.calls();
    let i = |name: &str| {
        calls
            .iter()
            .position(|c| c == name)
            .unwrap_or_else(|| panic!("{name} was never called: {calls:?}"))
    };
    assert!(i("create_workspace") < i("arm"));
    assert!(
        i("arm") < i("register_process"),
        "D22: the daemon can kill from the moment it can"
    );
    assert!(!outcome.entry_id.is_empty());
    assert!(
        m.abandoned().is_empty(),
        "nothing failed, so nothing was abandoned"
    );
    assert_eq!(
        outcome.exit_status.map(|s| s.success()),
        Some(true),
        "the command waited for the harness it launched"
    );

    // The workspace holds exactly what the daemon put there, and nothing this CLI wrote.
    let info = m.info();
    let mut present = support::relative_files(std::path::Path::new(&info.workspace));
    assert!(
        present.iter().any(|p| p.ends_with("proof.txt")),
        "the launched harness ran in the workspace: {present:?}"
    );
    // And it ran the launch the plan built: the fixture writes back the `[launch.env]` pin it
    // was given and the argv it was called with, so what is asserted is that the rendering
    // reached the child rather than only that a process started.
    let proof =
        std::fs::read_to_string(std::path::Path::new(&info.workspace).join("proof.txt")).unwrap();
    let mut proven = proof.lines();
    assert_eq!(
        proven.next(),
        Some(info.harness_home.as_str()),
        "`[launch.env] CLAUDE_CONFIG_DIR = {{harness_home}}` reached the harness: {proof:?}"
    );
    let argv = proven.next().unwrap_or_default();
    assert!(
        argv.contains(&format!("{}/settings-launch.json", info.artifact_root))
            && argv.contains(&info.session_id),
        "the rendered argv reached the harness: {argv:?}"
    );
    present.retain(|p| !p.ends_with("proof.txt"));
    assert_eq!(
        present, info.workspace_relative,
        "RULES.md: the workspace is empty except for the tracker's own config"
    );

    // The entry cache: the one file this command writes, and the source of the pinned run
    // key for every later process.
    let rec = goalcup_cli::entries::read_entry(tmp.path(), &outcome.entry_id).unwrap();
    assert_eq!(rec.run_public_key, info.run_public_key);
    assert_eq!(rec.workspace_relative, info.workspace_relative);
    assert_eq!(
        (rec.status_url.as_str(), rec.connector_id.as_str()),
        (info.status_url.as_str(), "claude-code-cli")
    );
    assert_eq!(
        (rec.harness_home.as_str(), rec.session_id.as_str()),
        (info.harness_home.as_str(), info.session_id.as_str()),
        "Task 12 resolves its lock paths from this record, before it posts a resume"
    );
    assert_eq!(
        rec.last_pid,
        m.last_registered_pid(),
        "the pid the daemon was handed is the pid the record carries"
    );
    assert!(
        rec.dev,
        "this run was started with `--dev` (`o.dev = true` above), and the record is the only \
         place a later process can learn that — `goalcup seal` prints a `goalcup-verify` \
         line, and without the flag the line it prints cannot verify this trace"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = goalcup_cli::entries::entry_path(tmp.path(), &outcome.entry_id);
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o077,
            0,
            "the participant's own bookkeeping is readable by nobody else"
        );
    }
    let serialised = std::fs::read_to_string(goalcup_cli::entries::entry_path(
        tmp.path(),
        &outcome.entry_id,
    ))
    .unwrap();
    assert!(
        !serialised.contains(support::GRANT_TOKEN),
        "no token in the participant's cache"
    );

    let printed = String::from_utf8(out).unwrap();
    assert!(
        printed.contains(&info.status_url),
        "R18: the URL is printed even headless"
    );
    assert!(
        printed.contains("--dev"),
        "a run that came through the door says so: {printed}"
    );
}

/// `--dev` admits a machine that has never run `goalcup auth`: the fourth admission of the
/// one door.
///
/// Nothing in this tree serves a device code, so without it the only way past the grant
/// refusal is a hand-written `grant.json`, which would make the README's development
/// walkthrough executable by nobody. This test is the walkthrough's first
/// command, without a grant on disk: the run goes through, the banner names what was
/// admitted, and the token that travels is empty rather than invented.
#[cfg(feature = "v1-compat")]
#[test]
fn dev_admits_a_machine_with_no_grant_and_the_banner_names_it() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    // `mock_armed` leaves the grant `goalcup auth` would have written. This run is the case
    // where nobody ever ran it.
    std::fs::remove_file(goalcup_cli::config::grant_path(tmp.path())).unwrap();

    let mut out = Vec::new();
    let mut o = support::start_options("claude-code-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_exits("claude-code-cli");
    let outcome = start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &o,
        &mut out,
    )
    .unwrap();
    assert!(!outcome.entry_id.is_empty(), "the run started");

    let printed = String::from_utf8(out).unwrap();
    assert!(
        printed.contains("--dev") && printed.contains("goalcup auth"),
        "the banner names the admission the run actually used: {printed}"
    );
    assert_eq!(
        m.last_workspace_opts().grant_token,
        "",
        "the field still travels, and what an empty token is worth is the server's call"
    );
}

/// And the same command WITHOUT the door is the refusal — the check is kept, never
/// dropped — with the flag that opens it named in the refusal.
#[cfg(feature = "v1-compat")]
#[test]
fn without_the_door_a_machine_with_no_grant_is_still_refused_before_anything_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let mut o = support::start_options("claude-code-cli");
    o.dev = false;
    let err = start_with(
        &m,
        &support::connectors_dir(),
        &support::empty_cfg(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);
    let said = format!("{err}");
    assert!(
        said.contains("goalcup auth") && said.contains("--dev"),
        "the refusal names both ways out of it: {said:?}"
    );
    assert!(m.calls().is_empty(), "nothing was created to abandon");
}

/// A developer who HAS a grant and typed `--dev` for the rulebook is not told their run went
/// out without one: the second banner line is printed only where the second admission was
/// used.
#[cfg(feature = "v1-compat")]
#[test]
fn the_grant_banner_is_printed_only_where_the_grant_was_actually_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let mut out = Vec::new();
    let mut o = support::start_options("claude-code-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_exits("claude-code-cli");
    start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &o,
        &mut out,
    )
    .unwrap();
    let printed = String::from_utf8(out).unwrap();
    assert!(
        printed.contains("--dev"),
        "the rulebook banner is still printed: {printed}"
    );
    assert!(
        !printed.contains("never run `goalcup auth`"),
        "and the grant banner is not, because this machine has one: {printed}"
    );
    assert_eq!(
        m.last_workspace_opts().grant_token,
        support::GRANT_TOKEN,
        "the grant on disk is the one that travelled"
    );
}

#[cfg(feature = "v1-compat")]
#[test]
fn a_missing_grant_an_altered_manifest_and_a_failed_preflight_each_refuse_before_anything_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let o = support::start_options("claude-code-cli");

    // No grant.
    let err = start_with(
        &m,
        &support::connectors_dir(),
        &support::empty_cfg(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);
    assert!(format!("{err}").contains("goalcup auth"));
    assert!(m.calls().is_empty(), "nothing was created to abandon");

    // An altered rulebook (PRD 5.4).
    let edited = support::connectors_dir_with_edited_manifest("claude-code-cli");
    let err = start_with(
        &m,
        &edited,
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);
    assert!(format!("{err}").to_lowercase().contains("manifest"));
    assert!(m.calls().is_empty(), "nothing was created to abandon");

    // A banned variable (RULES.md, D12).
    let mut env = support::clean_env_with_no_harness_installed();
    env.insert("ANTHROPIC_BASE_URL".into(), "http://elsewhere".into());
    let err = start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &env,
        &o,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);
    assert!(format!("{err}").contains("ANTHROPIC_BASE_URL"));
    assert!(m.calls().is_empty(), "nothing was created to abandon");
}

/// SYSTEM-PLAN §4.4 step 6: a failure before the prompt is submitted leaves no entry, and
/// that promise is only true if something removes it. The local record goes with it — a
/// cached entry the daemon has deleted is a record every later command would refuse on.
#[cfg(feature = "v1-compat")]
#[test]
fn a_failure_after_the_workspace_exists_abandons_the_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.fail_next(
        "arm",
        goalcup_protocol::control::ControlError::Conflict(
            "a tracked session already exists in this harness home".into(),
        ),
    );
    let o = support::start_options("codex-cli");
    let err = start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_ne!(err.exit_code(), 0);
    let abandoned = m.abandoned();
    assert_eq!(
        abandoned.len(),
        1,
        "SYSTEM-PLAN 4.4 step 6: no entry survives a failed start"
    );
    assert!(!abandoned[0].1.is_empty(), "the reason is recorded");
    assert_eq!(abandoned[0].0, m.info().entry_id);
    assert!(
        !goalcup_cli::entries::entry_path(tmp.path(), &m.info().entry_id).exists(),
        "and the local record goes with it"
    );
    assert!(
        !m.calls().contains(&"register_process".to_string()),
        "nothing was spawned"
    );
}

/// A `busy` is the daemon holding the entry for one asynchronous call and it is retried by
/// the helper; a `conflict` (above) is a refusal. The two share `409` and nothing else.
#[cfg(feature = "v1-compat")]
#[test]
fn a_busy_arm_is_retried_rather_than_abandoned() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.busy_once("arm");
    let mut o = support::start_options("claude-code-cli");
    o.print_launch = true;
    start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        m.calls().iter().filter(|c| *c == "arm").count(),
        2,
        "the helper asked again"
    );
    // The one abandon this run performs is `--print-launch`'s own, and its
    // reason says so. A `busy` contributes nothing to that list: it was ridden out.
    let abandoned = m.abandoned();
    assert_eq!(abandoned.len(), 1, "only the print-launch abandon");
    assert!(
        abandoned[0].1.contains("--print-launch"),
        "a busy is not a reason to abandon: {abandoned:?}"
    );
}

#[cfg(feature = "v1-compat")]
#[test]
fn start_wires_arm_to_the_entry_retry_and_does_not_retry_unreachable_or_spawn() {
    let source = include_str!("../src/commands/start.rs");
    let (_, after_arm_step) = source
        .split_once("// 6. Arm the folder.")
        .expect("the start arm step");
    let (arm_step, _) = after_arm_step
        .split_once("// From the arm on")
        .expect("the end of the arm step");
    assert!(arm_step.contains("retry_busy_entry(|| client.arm"));

    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.fail_always(
        "arm",
        goalcup_protocol::control::ControlError::Unreachable("daemon absent".into()),
    );
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;
    let workspace = std::path::PathBuf::from(&m.info().workspace);

    let error = start_with(
        &m,
        &support::connectors_dir_with_a_harness_that_exits("claude-code-cli"),
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect_err("an unreachable arm is returned without launching the harness");

    assert_eq!(error.exit_code(), 1);
    assert_eq!(
        m.calls().iter().filter(|call| *call == "arm").count(),
        1,
        "Unreachable is not safe to replay for this mutation"
    );
    assert!(
        !workspace.join("proof.txt").exists(),
        "no child was spawned"
    );
    assert!(!m.calls().contains(&"register_process".to_string()));
}

#[test]
fn the_shared_entry_retry_is_wired_only_to_the_nine_safe_runtime_routes() {
    let mut inventory = std::collections::BTreeMap::<String, usize>::new();
    let source_root = support::repo_root().join("crates/goalcup-cli/src");
    for path in support::walk("crates/goalcup-cli/src") {
        let source = std::fs::read_to_string(&path).unwrap();
        let file = path
            .strip_prefix(&source_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for line in source
            .lines()
            .filter(|line| line.contains("retry_busy_entry(|| client."))
        {
            for route in [
                "arm(",
                "abandon(",
                "resume_preview(",
                "resume(",
                "cancel_process_handoff(",
            ] {
                if line.contains(route) {
                    *inventory.entry(format!("{file}:{route}")).or_default() += 1;
                }
            }
        }
    }

    assert_eq!(
        inventory,
        std::collections::BTreeMap::from([
            ("commands/resume.rs:cancel_process_handoff(".to_string(), 3),
            ("commands/resume.rs:resume(".to_string(), 1),
            ("commands/resume.rs:resume_preview(".to_string(), 3),
            ("commands/start.rs:abandon(".to_string(), 1),
            ("commands/start.rs:arm(".to_string(), 1),
        ]),
        "the helper must not spread to process, seal, create, status, health, entries, current, or start-signal routes"
    );
}

#[cfg(feature = "v1-compat")]
#[test]
fn a_busy_process_handoff_reuses_the_one_handle_before_the_daemon_takes_it() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.busy_once("register_process");
    let watched = support::watched(&m);
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;

    start_with(
        &watched,
        &support::connectors_dir_with_a_harness_that_exits("claude-code-cli"),
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect("a transient process Busy is retried before the launch is abandoned");

    let handed = watched.registered_handles();
    assert_eq!(handed.len(), 2, "one Busy gets one retry of /process");
    assert_eq!(
        handed[0], handed[1],
        "the retry submits the exact immutable handle built for the spawned child"
    );
    assert_eq!(
        m.calls()
            .iter()
            .filter(|call| *call == "register_process")
            .count(),
        2,
        "the daemon takes the second and only successful handoff"
    );
    assert!(m.abandoned().is_empty(), "the handoff ultimately succeeded");
}

/// `/process` is the one idempotent mutation whose exact request can safely cross an
/// ambiguous transport boundary.  The first call commits in the daemon and loses its reply;
/// the CLI must replay the same typed registration, receive the durable acknowledgement, and
/// leave its child/entry intact.
#[cfg(feature = "v1-compat")]
#[test]
fn a_lost_process_reply_replays_the_exact_registration_without_abandoning() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let watched = support::watched(&m);
    watched.lose_next_accepted_process_reply();
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;

    start_with(
        &watched,
        &support::connectors_dir_with_a_harness_that_exits("claude-code-cli"),
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect("the exact process replay is acknowledged after the lost response");

    let handed = watched.registered_handles();
    assert_eq!(handed.len(), 2, "the CLI made one exact replay");
    assert_eq!(
        handed[0], handed[1],
        "the same immutable handle was replayed"
    );
    assert_eq!(
        m.calls()
            .iter()
            .filter(|call| *call == "register_process")
            .count(),
        1,
        "the daemon committed only the first handoff; the second request was its exact replay"
    );
    assert!(
        m.abandoned().is_empty(),
        "the CLI never kills or abandons an accepted child"
    );
}

/// A daemon that stays absent consumes the bounded process envelope, then keeps the exact
/// pending registration and child for a later idempotent replay. No cleanup may run after
/// the first transport error while a lost accepted response is still recoverable.
#[cfg(feature = "v1-compat")]
#[test]
fn an_absent_process_daemon_exhausts_the_handoff_envelope_without_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.fail_always(
        "register_process",
        goalcup_protocol::control::ControlError::Unreachable("daemon absent".to_string()),
    );
    let watched = support::watched(&m);
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;

    let error = start_with(
        &watched,
        &support::connectors_dir_with_a_harness_that_exits("claude-code-cli"),
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect_err("an absent daemon eventually exhausts the bounded handoff replay");
    assert_eq!(
        error.exit_code(),
        1,
        "unreachable remains the daemon-absent rung"
    );
    assert_eq!(
        watched.registered_handles().len(),
        goalcup_cli::retry::PROCESS_BUSY_ATTEMPTS,
        "every bounded process attempt happened before cleanup"
    );
    let calls = m.calls();
    let last_process = calls
        .iter()
        .rposition(|call| call == "register_process")
        .expect("the bounded process calls were recorded");
    assert_eq!(
        last_process + 1,
        calls.len(),
        "the exhausted process retry is the final daemon action; no unsafe cleanup follows"
    );
    assert!(m.abandoned().is_empty());
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id)
            .unwrap()
            .pending_process_registration
            .is_some(),
        "recovery keeps the exact immutable request body"
    );
}

#[cfg(feature = "v1-compat")]
#[test]
fn attach_mode_builds_no_plan_and_names_the_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut o = support::start_options("codex-cli");
    o.attach = true;
    let mut out = Vec::new();
    start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains(&m.info().workspace));
    assert!(
        !m.calls().contains(&"register_process".to_string()),
        "nothing was spawned"
    );
    assert!(
        m.last_workspace_opts().attach,
        "and the daemon was told which mode this is"
    );
}

#[cfg(feature = "v1-compat")]
#[test]
fn print_launch_prints_the_line_and_registers_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let mut o = support::start_options("claude-code-cli");
    o.print_launch = true;
    let mut out = Vec::new();
    start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("--settings") && text.contains("--session-id"));
    assert!(!m.calls().contains(&"register_process".to_string()));
}

/// `--print-launch` and not a launch, for the reason `support`'s header gives: the
/// shipped `codex-cli` recipe spawns `codex`, and a test that spawned it would be about the
/// machine it ran on. The flag under test reaches the daemon before either mode branches.
#[cfg(feature = "v1-compat")]
#[test]
fn the_arming_timeout_flag_is_step_three_of_the_precedence_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let mut o = support::start_options("codex-cli");
    o.arming_timeout = Some(45);
    o.print_launch = true;
    start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        m.last_workspace_opts().arming_timeout_seconds,
        Some(45),
        "connector default, then DaemonConfig, then this — stated once on manifest::Arming"
    );
    assert_eq!(
        m.last_workspace_opts().deadline_hours,
        goalcup_cli::commands::start::DEFAULT_DEADLINE_HOURS,
        "and the deadline the run is judged against travels with it"
    );
}

/// Every daemon call this crate makes goes through the one `busy` retry.
///
/// A grep and not a type, because the rule is about call sites: `retry::retry_busy` is a
/// helper a command can simply forget to wrap a call in, and the failure mode of forgetting
/// is a command that dies on a transient the daemon expects every client to ride out.
///
/// **The sweep is over the whole of `src/`**, not `src/commands/` alone: a command is where
/// a daemon call is usually made, but `src/arming.rs` calls `wait_start_signal` and `status`,
/// both wrapped, and a future unwrapped call beside either one would pass a guard whose name
/// says "every". The wider walk costs nothing: every `client.` in this crate that is not in a
/// comment is a call site, wherever the file sits.
///
/// **It reads one line at a time, so the convention it enforces is one line per call site**:
/// a `client.` and the `retry_busy` wrapping it are written on the same line, `rustfmt`
/// permitting, and a call split across lines would pass this sweep while wrapped in nothing
/// at all. That is a real constraint on how these commands are written, and it is cheap —
/// every daemon call in this crate is a closure short enough to fit.
#[test]
fn no_daemon_call_in_this_crate_is_made_outside_the_busy_retry() {
    let mut seen = 0;
    for entry in support::walk("crates/goalcup-cli/src") {
        let source = std::fs::read_to_string(&entry).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for (n, line) in lines.iter().copied().enumerate() {
            if !line.contains("client.") || line.trim_start().starts_with("//") {
                continue;
            }
            seen += 1;
            // The capture seal's own retry (`retry_capture_seal`, H-10 4a) has a closure that
            // builds its request from what remains of the budget before it calls, so its one
            // `client.seal_capture(` call sits inside a block that `retry_capture_seal(` opens
            // a few lines above: still exactly one call site, still inside the retry, and
            // nothing else may use this form.
            let capture_seal = line.trim_start().starts_with("client.seal_capture(")
                && lines[n.saturating_sub(12)..n]
                    .iter()
                    .any(|above| above.contains("retry_capture_seal("));
            assert!(
                line.contains("retry_busy") || capture_seal,
                "{}:{} calls the daemon outside the retry: {}",
                entry.display(),
                n + 1,
                line.trim()
            );
        }
    }
    assert!(seen > 0, "the sweep found no daemon call at all to check");
    // And the widened walk reaches call sites `src/commands/` does not hold — `src/arming.rs`
    // has two — so this test would still be doing its job if every command moved. A count and
    // not a path list: naming the file would go stale the next time one moved.
    let outside: usize = support::walk("crates/goalcup-cli/src")
        .iter()
        .filter(|e| !e.starts_with(support::repo_root().join("crates/goalcup-cli/src/commands")))
        .filter_map(|e| std::fs::read_to_string(e).ok())
        .map(|source| {
            source
                .lines()
                .filter(|l| l.contains("client.") && !l.trim_start().starts_with("//"))
                .count()
        })
        .sum();
    assert!(
        outside > 0,
        "the widened walk reaches daemon calls `src/commands` does not hold, and it found none"
    );
}

/// The other half of the window: a failure **after** the harness is spawned and before the
/// daemon has been told about it. The entry goes, the record goes, and the child does not
/// outlive the workspace it was running in — nothing would ever have measured it, and
/// nothing would ever have sealed what it wrote.
///
/// The fixture SLEEPS. A conflict can identify another durable process authority, so this
/// invocation stops only its newly spawned child and preserves the entry for recovery.
#[cfg(feature = "v1-compat")]
#[test]
fn a_conflicting_process_handoff_stops_only_its_local_child() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.fail_next(
        "register_process",
        goalcup_protocol::control::ControlError::Conflict("this entry is already sealed".into()),
    );
    let w = support::watched(&m);
    let mut o = support::start_options("claude-code-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_sleeps("claude-code-cli");
    let err = start_with(
        &w,
        &connectors,
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &o,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "a conflict is the daemon refusing, and the rung is its own"
    );
    let handed = w
        .registered_handle()
        .expect("the daemon was handed a process before it refused");
    support::wait_until_gone(handed.pid);
    assert!(m.abandoned().is_empty());
    let record = goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).unwrap();
    assert!(
        record.pending_process_registration.is_none(),
        "a first-attempt definite conflict leaves no stale body for a later resume to replay"
    );
    assert_eq!(record.last_pid, None, "the rejected child is no authority");

    let before = m.calls().len();
    goalcup_cli::commands::resume::resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("claude-code-cli"),
        tmp.path(),
        &m.info().entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("the preserved entry can later take a normal resume path");
    let calls = &m.calls()[before..];
    let resume = calls.iter().position(|call| call == "resume").unwrap();
    let handoff = calls
        .iter()
        .position(|call| call == "register_process")
        .unwrap();
    assert!(
        resume < handoff,
        "the stale initial body was cleared instead of being replayed before the new CAS"
    );
}

#[cfg(feature = "v1-compat")]
#[test]
fn a_cleanup_cache_write_failure_still_stops_the_retained_initial_child() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.fail_always(
        "register_process",
        goalcup_protocol::control::ControlError::Conflict(
            "the process was definitely refused".into(),
        ),
    );
    let watched = support::watched(&m);
    watched.break_entry_cache_after_register(tmp.path());
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;

    start_with(
        &watched,
        &support::connectors_dir_with_a_harness_that_sleeps("claude-code-cli"),
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect_err("the durable cleanup transition could not be written");
    let handed = watched.registered_handle().expect("the child was offered");
    let alive = goalcup_protocol::process::is_alive(handed.pid);
    if alive {
        std::process::Command::new("kill")
            .args(["-KILL", &handed.pid.to_string()])
            .status()
            .unwrap();
        support::wait_until_gone(handed.pid);
    }
    assert!(!alive, "the retained Child is still terminated and waited");

    std::fs::remove_file(tmp.path().join("entries")).unwrap();
    std::fs::rename(
        tmp.path().join("entries-held-by-test"),
        tmp.path().join("entries"),
    )
    .unwrap();
    let record = goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).unwrap();
    assert!(
        record.pending_process_registration.is_some(),
        "the prior replay row remains fail-closed when cleanup could not become durable"
    );
    assert_eq!(
        record.pending_process_registration_origin,
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Initial)
    );
}

#[cfg(feature = "v1-compat")]
#[test]
fn unresolved_native_cleanup_write_failure_recovers_to_a_non_replayable_blocker() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.fail_always(
        "register_process",
        goalcup_protocol::control::ControlError::Conflict(
            "the process was definitely refused".into(),
        ),
    );
    let watched = support::watched(&m);
    let pidfile = std::path::Path::new(&m.info().workspace).join("pid.txt");
    watched.answer_register_only_after(&pidfile);
    watched.break_entry_cache_after_register(tmp.path());
    let mut options = support::start_options("codex-cli");
    options.dev = true;

    start_with(
        &watched,
        &support::connectors_dir_with_an_unresolved_tui_native_child("codex-cli"),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &options,
        &mut Vec::new(),
    )
    .expect_err("the cleanup transition write is unavailable");
    let native_pid: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(goalcup_protocol::process::is_alive(native_pid));

    std::fs::remove_file(tmp.path().join("entries")).unwrap();
    std::fs::rename(
        tmp.path().join("entries-held-by-test"),
        tmp.path().join("entries"),
    )
    .unwrap();
    let pending = goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).unwrap();
    assert!(pending.pending_process_registration.is_some());
    assert_eq!(
        pending
            .pending_unresolved_native_cleanup
            .as_ref()
            .map(|state| state.origin),
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Initial),
        "the failed cleanup write cannot erase the earlier unresolved-native latch"
    );

    goalcup_cli::commands::resume::resume_with(
        &watched,
        &support::connectors_dir(),
        tmp.path(),
        &m.info().entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("recovery retires replay but cannot clear unresolved native cleanup");
    let path = goalcup_cli::entries::entry_path(tmp.path(), &m.info().entry_id);
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(raw.get("pending_process_registration").is_none());
    assert!(raw.get("pending_process_cleanup").is_some());
    assert_eq!(
        raw.get("pending_unresolved_native_cleanup")
            .and_then(|value| value.get("origin")),
        Some(&serde_json::Value::String("initial".to_string()))
    );
    assert!(m.resume_requests().is_empty());

    std::process::Command::new("kill")
        .args(["-KILL", &native_pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(native_pid);
}

/// A daemon-side `400 malformed` is a validation refusal before process authority changes.
/// Its `ControlError` variant matches an unparseable reply, but its response origin does not:
/// this child must be stopped and its pending replay body cleared.
#[cfg(feature = "v1-compat")]
#[test]
fn a_framed_malformed_process_refusal_stops_the_child_and_clears_the_pending_handoff() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.fail_next(
        "register_process",
        goalcup_protocol::control::ControlError::Malformed("native pid validation failed".into()),
    );
    let w = support::watched(&m);
    w.frame_next_process_malformed_refusal();
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;

    let error = start_with(
        &w,
        &support::connectors_dir_with_a_harness_that_sleeps("claude-code-cli"),
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect_err("a framed malformed refusal is definitely pre-admission");

    assert_eq!(
        error.exit_code(),
        3,
        "a daemon-side malformed validation refusal remains a malformed-control-response rung"
    );
    let handed = w
        .registered_handle()
        .expect("the rejected child was offered");
    support::wait_until_gone(handed.pid);
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).is_err(),
        "a definite daemon validation refusal abandons the unstarted entry, leaving no pending body to replay"
    );
    assert_eq!(
        m.abandoned().len(),
        1,
        "the initial start's definitely unadmitted entry is safely abandoned"
    );
}

/// The raw-client contract establishes that a framed 3xx is not a daemon refusal.  The
/// lifecycle must therefore retain the offered child and exact body even when the local test
/// daemon did not accept it: an intermediary could have hidden a durable admission.
#[cfg(feature = "v1-compat")]
#[test]
fn a_framed_redirect_process_reply_keeps_the_child_and_exact_pending_handoff() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.fail_next(
        "register_process",
        goalcup_protocol::control::ControlError::Malformed("redirected process route".into()),
    );
    let w = support::watched(&m);
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;

    let error = start_with(
        &w,
        &support::connectors_dir_with_a_harness_that_sleeps("claude-code-cli"),
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect_err("a redirect cannot prove process admission was refused");
    assert_eq!(error.exit_code(), 3);

    let handed = w.registered_handle().expect("the child was offered");
    assert!(
        goalcup_protocol::process::is_alive(handed.pid),
        "the potentially admitted child remains alive for exact replay"
    );
    let pending = goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).unwrap();
    assert_eq!(pending.last_pid, Some(handed.pid));
    assert_eq!(
        pending.pending_process_registration,
        Some(goalcup_protocol::process::ProcessRegistration {
            generation: m.info().process_generation,
            handle: handed.clone(),
        }),
        "the full immutable request remains available for replay"
    );
    assert!(m.abandoned().is_empty());

    std::process::Command::new("kill")
        .args(["-TERM", &handed.pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(handed.pid);
}

/// A structured authenticated 4xx has not handed the new child to the daemon.  The initial
/// entry itself may still have a different lifecycle, so these refusals retain its cache but
/// clear the rejected child's pending body rather than leaving it for an unsafe replay.
#[cfg(feature = "v1-compat")]
#[test]
fn framed_401_404_and_422_process_refusals_stop_the_child_and_clear_its_pending_body() {
    for (name, refusal) in [
        (
            "401",
            goalcup_protocol::control::ControlError::Unauthorized("control token refused".into()),
        ),
        (
            "404",
            goalcup_protocol::control::ControlError::NotFound("entry is no longer live".into()),
        ),
        (
            "422",
            goalcup_protocol::control::ControlError::PreflightFailed(
                "process handoff policy refused".into(),
            ),
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let m = support::mock_armed(&tmp, "claude-code-cli");
        m.fail_next("register_process", refusal);
        let mut options = support::start_options("claude-code-cli");
        options.dev = true;

        let error = start_with(
            &m,
            &support::connectors_dir_with_a_harness_that_sleeps("claude-code-cli"),
            tmp.path(),
            &support::clean_env_with_a_signed_in_home(tmp.path()),
            &options,
            &mut Vec::new(),
        )
        .expect_err("a framed {name} refusal stops this local launch");
        let offered = m.process_registrations();
        assert_eq!(offered.len(), 1, "the {name} child was offered once");
        let handed = offered[0].handle.clone();
        let alive_after_refusal = goalcup_protocol::process::is_alive(handed.pid);
        if alive_after_refusal {
            std::process::Command::new("kill")
                .args(["-TERM", &handed.pid.to_string()])
                .status()
                .unwrap();
            support::wait_until_gone(handed.pid);
        }
        assert!(
            !alive_after_refusal,
            "the {name} refusal must not orphan the unregistered child"
        );
        assert!(
            m.abandoned().is_empty(),
            "the {name} response did not confirm this entry can be abandoned"
        );
        let repaired = goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id).unwrap();
        assert!(
            repaired.pending_process_registration.is_none(),
            "the {name} refusal leaves no rejected request to replay"
        );
        assert_eq!(repaired.last_pid, None, "the {name} child is no authority");
        assert!(
            !format!("{error}").is_empty(),
            "the {name} refusal is reported"
        );
    }
}

/// A malformed `/process` response may have followed a durable admission. The initial child
/// and its full immutable replay body must survive until a later resume confirms the replay.
#[cfg(feature = "v1-compat")]
#[test]
fn an_ambiguous_process_response_keeps_the_initial_child_and_replays_before_a_resume_cas() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let w = support::watched(&m);
    w.malform_accepted_process_replies_until_released();
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;

    let error = start_with(
        &w,
        &support::connectors_dir_with_a_harness_that_sleeps("claude-code-cli"),
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect_err("a malformed response cannot prove the daemon rejected the child");
    assert_eq!(error.exit_code(), 3);
    assert_eq!(
        m.process_registrations().len(),
        1,
        "the malformed response followed a durable process admission"
    );
    assert!(
        m.abandoned().is_empty(),
        "the entry remains available for replay"
    );

    let handed = w.registered_handle().unwrap();
    assert!(
        goalcup_protocol::process::is_alive(handed.pid),
        "an ambiguously admitted child is not killed locally"
    );
    let entry_id = m.info().entry_id;
    let pending = goalcup_cli::entries::read_entry(tmp.path(), &entry_id).unwrap();
    assert_eq!(pending.last_pid, Some(handed.pid));
    assert_eq!(
        pending.pending_process_registration,
        Some(goalcup_protocol::process::ProcessRegistration {
            generation: m.info().process_generation,
            handle: handed.clone(),
        }),
        "the complete registration, not a reconstructed subset, remains replayable"
    );

    w.release_accepted_process_reply();
    goalcup_cli::commands::resume::resume_with(
        &w,
        &support::connectors_dir_with_a_resume_that_exits("claude-code-cli"),
        tmp.path(),
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("the later resume replays before spending another resume generation");
    assert_eq!(
        w.registered_handles().last(),
        Some(&handed),
        "the later acknowledgement was for the original durable registration"
    );
    assert_eq!(m.process_registrations().len(), 1);
    assert!(
        m.resume_requests().is_empty(),
        "replay precedes every resume CAS"
    );
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &entry_id)
            .unwrap()
            .pending_process_registration
            .is_none(),
        "only a confirmed replay clears the pending body"
    );

    std::process::Command::new("kill")
        .args(["-TERM", &handed.pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(handed.pid);
}

/// Recovery must remember that this cached body came from the initial opening. A later
/// definite refusal makes it cleanup-only, but it must not invent and cancel a resume permit.
#[cfg(feature = "v1-compat")]
#[test]
fn an_initial_pending_replay_keeps_initial_origin_when_later_definitely_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.fail_always(
        "register_process",
        goalcup_protocol::control::ControlError::Unreachable(
            "the first response did not arrive".into(),
        ),
    );
    let watched = support::watched(&m);
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_sleeps("claude-code-cli");

    start_with(
        &watched,
        &connectors,
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect_err("the initial handoff remains replayable after an ambiguous response");
    let entry_id = m.info().entry_id;
    let pending = goalcup_cli::entries::read_entry(tmp.path(), &entry_id).unwrap();
    let registration = pending
        .pending_process_registration
        .clone()
        .expect("the initial exact body is durable");
    assert_eq!(
        pending.pending_process_registration_origin,
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Initial)
    );

    m.fail_always(
        "register_process",
        goalcup_protocol::control::ControlError::Conflict(
            "the initial request was absent before mutation".into(),
        ),
    );
    goalcup_cli::commands::resume::resume_with(
        &watched,
        &support::connectors_dir(),
        tmp.path(),
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the exact-live rejected initial child remains a cleanup blocker");

    assert!(
        m.process_handoff_cancellations().is_empty(),
        "an initial opening never owns a resume permit"
    );
    assert!(
        m.resume_requests().is_empty(),
        "cleanup precedes any new CAS"
    );
    let cleaning = goalcup_cli::entries::read_entry(tmp.path(), &entry_id).unwrap();
    assert!(cleaning.pending_process_registration.is_none());
    assert!(cleaning.pending_process_registration_origin.is_none());
    assert_eq!(cleaning.pending_process_cleanup, Some(registration.clone()));
    assert_eq!(
        cleaning.pending_process_cleanup_origin,
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Initial)
    );

    std::process::Command::new("kill")
        .args(["-KILL", &registration.handle.pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(registration.handle.pid);
}

/// The first `/process` request was durably admitted, but each bounded in-command replay
/// loses its response. Recovery therefore has to leave the child and full body intact for a
/// later invocation rather than confusing exhaustion with non-admission.
#[cfg(feature = "v1-compat")]
#[test]
fn an_accepted_process_reply_lost_through_retry_exhaustion_replays_on_later_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let w = support::watched(&m);
    w.lose_accepted_process_replies_until_released();
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;

    let error = start_with(
        &w,
        &support::connectors_dir_with_a_harness_that_sleeps("claude-code-cli"),
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &options,
        &mut Vec::new(),
    )
    .expect_err("lost replies exhaust the bounded in-command replay envelope");
    assert_eq!(error.exit_code(), 1);
    let offered = w.registered_handles();
    assert_eq!(
        offered.len(),
        goalcup_cli::retry::PROCESS_BUSY_ATTEMPTS,
        "every in-command attempt submitted the same admitted registration"
    );
    assert!(
        offered.windows(2).all(|pair| pair[0] == pair[1]),
        "each bounded retry retained the exact child authority"
    );
    assert_eq!(
        m.process_registrations().len(),
        1,
        "only the first request was admitted; all later ones were exact replays"
    );
    assert!(m.abandoned().is_empty());

    let handed = offered.last().cloned().unwrap();
    assert!(goalcup_protocol::process::is_alive(handed.pid));
    let entry_id = m.info().entry_id;
    let pending = goalcup_cli::entries::read_entry(tmp.path(), &entry_id).unwrap();
    assert_eq!(pending.last_pid, Some(handed.pid));
    assert_eq!(
        pending.pending_process_registration,
        Some(goalcup_protocol::process::ProcessRegistration {
            generation: m.info().process_generation,
            handle: handed.clone(),
        })
    );

    w.release_accepted_process_reply();
    goalcup_cli::commands::resume::resume_with(
        &w,
        &support::connectors_dir_with_a_resume_that_exits("claude-code-cli"),
        tmp.path(),
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("a later resume gets the durable acknowledgement before any CAS");
    assert_eq!(
        w.registered_handles().last(),
        Some(&handed),
        "the later acknowledgement was for the same exhausted replay body"
    );
    assert_eq!(m.process_registrations().len(), 1);
    assert!(m.resume_requests().is_empty());
    assert!(
        goalcup_cli::entries::read_entry(tmp.path(), &entry_id)
            .unwrap()
            .pending_process_registration
            .is_none()
    );

    std::process::Command::new("kill")
        .args(["-TERM", &handed.pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(handed.pid);
}

/// The kill on that path signals the process GROUP, where the launch owns one.
///
/// The harnesses this CLI spawns are wrappers — `npm` execs a native binary, a server forks a
/// worker — so a `Child::kill` on the leader alone leaves the work still running in a
/// workspace the very next call abandons. The fixture is the smallest thing with that shape:
/// a `LaunchMode::Server` recipe, which is the one mode `spawn` puts in a group of its own,
/// backgrounding a `sleep` and naming its pid.
#[cfg(feature = "v1-compat")]
#[test]
fn the_kill_reaches_the_whole_group_and_not_only_the_process_that_was_spawned() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    m.fail_next(
        "register_process",
        goalcup_protocol::control::ControlError::Conflict("this entry is already sealed".into()),
    );
    let w = support::watched(&m);
    // The fork is a fact and not a race: the daemon does not answer until the pid file the
    // recipe writes is there, which is a moment a real one over a socket takes anyway.
    let pidfile = std::path::Path::new(&m.info().workspace).join("pid.txt");
    w.answer_register_only_after(&pidfile);
    let mut o = support::start_options("claude-code-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_group_owning_harness("claude-code-cli");
    let err = start_with(
        &w,
        &connectors,
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &o,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);

    let handed = w
        .registered_handle()
        .expect("the daemon was handed a process before it refused");
    assert!(
        handed.pgid.is_some(),
        "the fixture launches in server mode, so there is a group to signal"
    );
    let forked: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    support::wait_until_gone(handed.pid);
    support::wait_until_gone(forked);
    assert!(m.abandoned().is_empty());
}

/// A TUI cannot own a private process group. When its wrapper has already exposed the
/// pinned native descendant, definite-refusal cleanup must verify and stop that exact native
/// instance before killing/waiting the retained wrapper, then clear cleanup only after both
/// authorities are absent.
#[cfg(feature = "v1-compat")]
#[test]
fn a_tui_definite_refusal_does_not_leave_its_native_child_alive() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.fail_next(
        "register_process",
        goalcup_protocol::control::ControlError::Conflict(
            "the process was definitely refused".into(),
        ),
    );
    let watched = support::watched(&m);
    let pidfile = std::path::Path::new(&m.info().workspace).join("pid.txt");
    watched.answer_register_only_after(&pidfile);
    let mut options = support::start_options("codex-cli");
    options.dev = true;

    start_with(
        &watched,
        &support::connectors_dir_with_a_tui_native_child("codex-cli"),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &options,
        &mut Vec::new(),
    )
    .expect_err("the handoff is definitely refused");

    let handle = watched
        .registered_handle()
        .expect("the wrapper was offered");
    let native_pid: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(handle.pgid, None, "a TUI owns no process group");
    assert_eq!(handle.native_pid, Some(native_pid));
    assert!(handle.native_process_start.is_some());
    support::wait_until_gone(native_pid);
    support::wait_until_gone(handle.pid);
}

/// If the native process exists but has not yet exec'd into the manifest-pinned binary,
/// bounded launch discovery cannot name it. Killing only the retained TUI wrapper is then
/// not proof that the native process is gone. A definite refusal must leave a durable,
/// non-replayable manual-cleanup latch rather than clearing or abandoning the entry.
#[cfg(feature = "v1-compat")]
#[test]
fn an_unresolved_tui_native_refusal_never_clears_or_relaunches_automatically() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.fail_always(
        "register_process",
        goalcup_protocol::control::ControlError::Conflict(
            "the process was definitely refused".into(),
        ),
    );
    let watched = support::watched(&m);
    let pidfile = std::path::Path::new(&m.info().workspace).join("pid.txt");
    watched.answer_register_only_after(&pidfile);
    let mut options = support::start_options("codex-cli");
    options.dev = true;

    let refusal = start_with(
        &watched,
        &support::connectors_dir_with_an_unresolved_tui_native_child("codex-cli"),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &options,
        &mut Vec::new(),
    )
    .expect_err("the handoff is definitely refused");
    assert!(format!("{refusal}").contains("definitely refused"));

    let handle = watched
        .registered_handle()
        .expect("the wrapper was offered");
    let native_pid: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(handle.pgid, None);
    assert_eq!(
        handle.native_pid, None,
        "bounded discovery intentionally missed"
    );
    support::wait_until_gone(handle.pid);
    assert!(
        goalcup_protocol::process::is_alive(native_pid),
        "the test needs the undiscovered native process to survive its wrapper"
    );
    assert!(
        m.abandoned().is_empty(),
        "unresolved containment is not abandonment proof"
    );

    let path = goalcup_cli::entries::entry_path(tmp.path(), &m.info().entry_id);
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        raw.get("pending_unresolved_native_cleanup")
            .and_then(|value| value.get("native_binary")),
        Some(&serde_json::Value::String("sleep".to_string())),
        "the unresolved native authority remains an explicit cleanup blocker"
    );
    assert!(raw.get("pending_process_registration").is_none());
    assert!(raw.get("pending_process_cleanup").is_some());

    let registrations_before = m.process_registrations().len();
    let resumes_before = m.resume_requests().len();
    let error = goalcup_cli::commands::resume::resume_with(
        &watched,
        &support::connectors_dir(),
        tmp.path(),
        &m.info().entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("persisted unresolved-native cleanup blocks automatic recovery");
    assert!(format!("{error}").contains("native"));
    assert_eq!(m.process_registrations().len(), registrations_before);
    assert_eq!(m.resume_requests().len(), resumes_before);

    std::process::Command::new("kill")
        .args(["-KILL", &native_pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(native_pid);
}

/// A failed spawn syscall created no process, so the conservative marker written before
/// it must not survive merely because the daemon could not confirm entry abandonment.
#[cfg(feature = "v1-compat")]
#[test]
fn a_definitive_start_spawn_error_clears_the_speculative_native_guard() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    m.fail_always(
        "abandon",
        goalcup_protocol::control::ControlError::Unreachable(
            "the abandon response was unavailable".into(),
        ),
    );
    let mut options = support::start_options("codex-cli");
    options.dev = true;

    start_with(
        &m,
        &support::connectors_dir_with_an_unspawnable_tui_native("codex-cli"),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &options,
        &mut Vec::new(),
    )
    .expect_err("the missing executable cannot spawn");

    assert!(m.process_registrations().is_empty());
    assert_eq!(
        m.abandoned().len(),
        0,
        "the mocked abandon was not confirmed"
    );
    assert!(
        m.calls().contains(&"abandon".to_string()),
        "the ordinary start cleanup still attempted daemon abandonment"
    );
    let retained = goalcup_cli::entries::read_entry(tmp.path(), &m.info().entry_id)
        .expect("the unconfirmed abandon keeps the local entry inspectable");
    assert!(retained.pending_process_registration.is_none());
    assert!(retained.pending_process_cleanup.is_none());
    assert!(
        retained.pending_unresolved_native_cleanup.is_none(),
        "a spawn error is proof that the speculative native child never existed"
    );
}

/// D22's pid-age guard: the daemon refuses to signal a pid whose process is younger than its
/// handle claims, because that is how a recycled number is told from the launch's own
/// process. A `launched_at` copied off the workspace's `created_at` is therefore not a
/// harmless approximation — every second between the daemon creating the workspace and this
/// command spawning the harness is a second of slack spent, and a start that took more than
/// five would hand over a handle the terminator can never act on. The harness would outlive
/// its own deadline, silently.
#[cfg(feature = "v1-compat")]
#[test]
fn the_handle_is_stamped_when_the_harness_started_and_not_when_the_workspace_was_made() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let w = support::watched(&m);
    let mut o = support::start_options("claude-code-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_exits("claude-code-cli");
    start_with(
        &w,
        &connectors,
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &o,
        &mut Vec::new(),
    )
    .unwrap();

    let handed = w.registered_handle().expect("a process was registered");
    let stamped = goalcup_protocol::control::parse_z(&handed.launched_at)
        .expect("launched_at is an RFC-3339 instant the daemon can parse");
    // Ordering and not equality: the two are seconds apart on a fast machine and the same
    // second on a slow one, and what the guard cares about is that the stamp is not older
    // than the process it describes.
    assert!(
        stamped >= w.workspace_created_at(),
        "launched_at {} predates the workspace it was launched into",
        handed.launched_at
    );
    assert_ne!(
        handed.launched_at,
        m.info().created_at,
        "the workspace's own stamp is taken before the arm, the cache and the plan"
    );
}

/// A terminal that goes away after the arm is not a reason to unmake the run.
///
/// Every line this command prints after the arm is inside the abandon window, and a `?` on
/// one of them would delete a folder the daemon has armed because the participant closed a
/// pager. The write error is still reported — the run is what stands, not the silence.
#[cfg(feature = "v1-compat")]
#[test]
fn a_terminal_that_went_away_after_the_arm_is_reported_and_the_run_stands() {
    /// Fails on the line carrying the status URL, which is printed after the arm and after
    /// the entry cache — and succeeds on everything before it, so the failure under test is
    /// the one this test is named for and not a preflight warning.
    struct GoesAwayAtTheStatusUrl;

    impl std::io::Write for GoesAwayAtTheStatusUrl {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if String::from_utf8_lossy(buf).contains("status:") {
                return Err(std::io::Error::other("the terminal went away"));
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let mut o = support::start_options("claude-code-cli");
    o.print_launch = true;
    let err = start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut GoesAwayAtTheStatusUrl,
    )
    .unwrap_err();
    assert!(
        m.calls().contains(&"arm".to_string()),
        "the failure under test is after the arm: {:?}",
        m.calls()
    );
    assert!(format!("{err}").contains("stdout"), "{err}");
    assert!(
        m.abandoned().is_empty(),
        "a write that failed is not a reason to delete an armed workspace"
    );
    assert!(
        goalcup_cli::entries::entry_path(tmp.path(), &m.info().entry_id).exists(),
        "and the local record stands with it"
    );
}

/// The entry cache on a machine that has never started a run: `entries.rs` is shared with the
/// capture `status` and `seal`, which choose their entry through it.
#[test]
fn the_entry_cache_says_so_when_there_is_none() {
    let cfg = support::empty_cfg();
    assert!(
        goalcup_cli::entries::latest_entry(&cfg).unwrap().is_none(),
        "a machine that has never started a run is not an error"
    );
    let err = goalcup_cli::entries::read_entry(&cfg, "entry-nope").unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "naming an entry this machine never started is a refusal, not the CLI failing"
    );
}

/// The record `start` wrote, by id and as "the last one".
#[cfg(feature = "v1-compat")]
#[test]
fn the_entry_cache_reads_back_by_id_and_by_recency_and_says_so_when_there_is_none() {
    let cfg = support::empty_cfg();
    assert!(
        goalcup_cli::entries::latest_entry(&cfg).unwrap().is_none(),
        "a machine that has never started a run is not an error"
    );
    let err = goalcup_cli::entries::read_entry(&cfg, "entry-nope").unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "naming an entry this machine never started is a refusal, not the CLI failing"
    );

    // A real launch, and NOT `--print-launch`: that flag abandons the entry it rendered
    // from and takes this very record with it, so a cache
    // test written on top of it would be asserting against a file the command deletes on
    // purpose. The fake harness exits immediately, which is what makes a launch usable here.
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let mut o = support::start_options("claude-code-cli");
    o.dev = true;
    let connectors = support::connectors_dir_with_a_harness_that_exits("claude-code-cli");
    let outcome = start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_a_signed_in_home(tmp.path()),
        &o,
        &mut Vec::new(),
    )
    .unwrap();
    let latest = goalcup_cli::entries::latest_entry(tmp.path())
        .unwrap()
        .unwrap();
    assert_eq!(latest.entry_id, outcome.entry_id);
    assert_eq!(
        latest.created_at,
        m.info().created_at,
        "the daemon's stamp, carried across: this CLI reads no clock of its own"
    );
    assert_eq!(
        latest.last_pid,
        m.last_registered_pid(),
        "the record names the process the daemon was handed"
    );
}

/// The capture launch's deny policy reaches a V1 `[launch.env]` pin too: a telemetry exporter
/// or an injection variable in a manifest would move a run's prompts off the machine from an
/// otherwise ordinary-looking diff, while the shipped telemetry switches stay valid.
#[cfg(feature = "v1-compat")]
#[test]
fn a_manifest_that_pins_an_exporter_or_an_injection_variable_in_launch_env_is_refused() {
    let info = support::workspace_info();
    let v = goalcup_cli::vars::from_workspace(&info);
    let ws = std::path::Path::new(&info.workspace);

    for pinned in [
        "OTEL_LOGS_EXPORTER",
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "NODE_OPTIONS",
        "DYLD_INSERT_LIBRARIES",
    ] {
        let mut c = support::connector("claude-code-cli");
        c.manifest
            .launch
            .env
            .insert(pinned.to_string(), "x".to_string());
        let err = plan(&c, &v, ws, LaunchKind::Start, &support::launch_env())
            .expect_err("the capture deny list applies to a V1 launch pin");
        assert_eq!(err.exit_code(), 1, "{pinned}");
        assert!(
            format!("{err}").contains(pinned),
            "the refusal names {pinned}"
        );
    }
    // The shipped Claude manifest pins its own telemetry switches, and still plans.
    let c = support::connector("claude-code-cli");
    plan(&c, &v, ws, LaunchKind::Start, &support::launch_env()).expect("the shipped pins pass");
}

/// D23's denylist on the SET side: a manifest that pins a base URL in `[launch.env]` is
/// refused by `plan`, and `--dev` does not open that door.
///
/// `scrubbed_env` takes these names out of the INHERITED environment and `spawn` applies
/// `env_set` AFTER `env_remove`, so a pinned one would beat both the scrub and the removal
/// list. `--dev` exists precisely to admit a manifest this build does not pin, which is the
/// run where it matters most.
#[cfg(feature = "v1-compat")]
#[test]
fn a_manifest_that_pins_a_base_url_in_launch_env_is_refused_even_under_dev() {
    let info = support::workspace_info();
    let v = goalcup_cli::vars::from_workspace(&info);
    let ws = std::path::Path::new(&info.workspace);

    for pinned in ["ANTHROPIC_BASE_URL", "HTTPS_PROXY", "PROXY"] {
        let mut c = support::connector("claude-code-cli");
        c.manifest
            .launch
            .env
            .insert(pinned.to_string(), "http://elsewhere.example".to_string());
        let err = plan(&c, &v, ws, LaunchKind::Start, &support::launch_env())
            .expect_err("a pinned base URL or proxy is refused, whatever else the recipe says");
        assert_eq!(err.exit_code(), 1, "{pinned}");
        let said = format!("{err}");
        assert!(said.contains(pinned), "the refusal names it: {said}");
        assert!(
            said.contains("claude-code-cli"),
            "and the connector that pinned it: {said}"
        );
    }

    // Through the command, under `--dev`, with a manifest on disk. **A proxy and not a base
    // URL**: `ConnectorManifest::validate` already refuses a `*_BASE_URL` in `[launch.env]`
    // (`manifest.rs`, "D1: the tracker never sets a model base URL"), so a manifest pinning
    // one cannot be loaded at all — and it says nothing about `*_PROXY` or a bare `PROXY`,
    // which is the half of D23's denylist that reached the child. `is_denied` is the whole
    // rule and `plan` is where it is asked.
    let mut o = support::start_options("codex-cli");
    o.dev = true;
    o.print_launch = true;
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "codex-cli");
    let connectors =
        support::connectors_dir_with_launch_env("codex-cli", "HTTPS_PROXY", "http://elsewhere");
    let err = goalcup_cli::commands::start::start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .expect_err("--dev admits an unpinned rulebook and never a base URL");
    assert_eq!(err.exit_code(), 1, "{err}");
    assert!(format!("{err}").contains("HTTPS_PROXY"), "{err}");
    assert_eq!(
        m.abandoned().len(),
        1,
        "and the entry that existed by then is unmade"
    );
}

/// The page `create_workspace` answered with is checked BEFORE it is written into the entry
/// cache, and a start whose daemon named anything else leaves nothing behind.
///
/// `goalcup ui` refuses a cached URL that is not a loopback page, so `goalcup start` must
/// not write such a string into the record: the refusal would arrive one command later, on a
/// machine that had already spent a workspace on it.
#[cfg(feature = "v1-compat")]
#[test]
fn a_status_url_that_is_not_a_loopback_page_refuses_the_start_and_caches_nothing() {
    for bad in [
        "https://evil.example/v0/ui?entry=entry-mock-0001",
        "http://127.0.0.1/v0/ui?entry=entry-mock-0001",
        "http://127.0.0.1@evil.example:8080/v0/ui",
        "file:///etc/passwd",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let m = support::mock_armed(&tmp, "claude-code-cli");
        m.set_workspace_answer(Some(bad), None);
        let err = goalcup_cli::commands::start::start_with(
            &m,
            &support::connectors_dir(),
            tmp.path(),
            &support::clean_env_with_no_harness_installed(),
            &support::start_options("claude-code-cli"),
            &mut Vec::new(),
        )
        .expect_err("a page this CLI will not open is not one it records");
        assert_eq!(err.exit_code(), 1, "{bad}");
        assert!(
            format!("{err}").contains(bad),
            "the refusal names it: {err}"
        );
        assert!(
            !goalcup_cli::entries::entry_path(tmp.path(), &m.info().entry_id).exists(),
            "and nothing was cached: {bad}"
        );
        assert_eq!(m.abandoned().len(), 1, "and the entry is unmade: {bad}");
        assert!(
            !m.calls().contains(&"arm".to_string()),
            "before the folder was ever armed: {bad}"
        );
    }
}

/// The second spelling of the workspace is on the screen where it IS a second spelling, and
/// absent where it is not.
///
/// `WorkspaceInfo.workspace_requested` is a display-only field: a field the daemon fills in
/// is a field nobody can see unless a command prints it.
#[cfg(feature = "v1-compat")]
#[test]
fn the_requested_workspace_spelling_is_printed_only_where_it_differs() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let mut o = support::start_options("claude-code-cli");
    o.print_launch = true;
    let mut out = Vec::new();
    goalcup_cli::commands::start::start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut out,
    )
    .unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(
        !said.contains("asked for"),
        "one spelling is not two: {said}"
    );

    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "claude-code-cli");
    let requested = format!("{}/../{}", m.info().workspace, "elsewhere");
    m.set_workspace_answer(None, Some(&requested));
    let mut out = Vec::new();
    goalcup_cli::commands::start::start_with(
        &m,
        &support::connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut out,
    )
    .unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(
        said.contains(&requested) && said.contains("asked for"),
        "the spelling the participant asked for is named beside the one it resolves to: \
         {said}"
    );
}
