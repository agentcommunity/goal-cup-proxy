//! Four functions: where the artifact root may sit relative to the workspace, that
//! the workspace is a new directory outside any Git worktree with zero entries, that the
//! tracker's own folder-scoped files are written and their hashes recorded, and that the
//! baseline is the `SealManifest` the verifier compares a sealed file against.

use goalcup_daemon::paths::{PathVars, claude_slug, expand_globs, resolve_template};
use goalcup_daemon::workspace::*;
use goalcup_platform::tracecap::EntryArtifactRoot;
use goalcup_protocol::manifest::{ConnectorManifest, InstallScope};
use goalcup_protocol::template::TemplateVars;
use std::path::{Path, PathBuf};

fn connector_dir(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id)
}

fn manifest(id: &str) -> ConnectorManifest {
    let path = connector_dir(id).join("manifest.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    ConnectorManifest::from_toml_str(&text).expect("manifest parses")
}

/// The per-run bearer these fixtures render with. A constant so a test can look for it
/// in the bytes a template produced.
const RECEIVER_TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn vars(roots: &Roots) -> TemplateVars {
    let mut v = TemplateVars::new();
    v.set("workspace", roots.workspace.to_string_lossy().to_string());
    v.set(
        "artifact_root",
        roots.artifact_root.to_string_lossy().to_string(),
    );
    v.set(
        "harness_home",
        roots.harness_home.to_string_lossy().to_string(),
    );
    v.set("receiver_http", "http://127.0.0.1:53311");
    v.set("receiver_token", RECEIVER_TOKEN);
    v.set("entry_id", "entry-0001");
    v.set("run_id", "entry-0001");
    v.set("session_id", "95addb64-9dac-4046-8ef6-8aa53fa0c7f1");
    v.set("server_port", "7788");
    v.set(
        "status_line_path",
        roots
            .artifact_root
            .join("status-line.txt")
            .to_string_lossy()
            .to_string(),
    );
    // Declared and empty; the second install pass fills it with the peer's hash.
    v
}

/// Both roots under one `tempfile::tempdir()`, because the nesting check resolves symlinks
/// and on macOS a `/tmp` path resolves through `/private`.
fn roots(dir: &Path) -> Roots {
    let r = Roots {
        workspace: dir.join("holder/ws-one"),
        artifact_root: dir.join("tracker-artifacts/entry-0001"),
        harness_home: dir.join("tracker-artifacts/homes/entry-0001"),
    };
    std::fs::create_dir_all(&r.artifact_root).unwrap();
    std::fs::create_dir_all(&r.harness_home).unwrap();
    r
}

/// Every file under `root`, as sorted relative strings. A test that has to say "nothing
/// was written" needs the whole subtree: a top-level listing would miss a file the tracker
/// put one directory down.
fn tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stack.push(path);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
    }
    out.sort();
    out
}

/// The nine path tokens, pointed at the same temp tree, so `config_allowlist[]` resolves
/// under the test's own directory rather than the developer's real `~/.claude`.
fn path_vars(r: &Roots) -> PathVars {
    PathVars::new(
        r.workspace.clone(),
        r.harness_home.clone(),
        r.harness_home.join(".config"),
        r.artifact_root.clone(),
        None,
        ("2026".into(), "09".into(), "04".into()),
    )
}

// ---------------------------------------------------------------- the four demands

#[test]
fn the_artifact_root_may_not_be_identical_nested_or_adjacent_to_the_workspace() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("run/ws");
    for bad in [
        ws.clone(),
        ws.join("art"),
        t.path().join("run"),
        t.path().join("run/art"),
    ] {
        assert!(
            check_artifact_placement(&bad, &ws).is_err(),
            "{} must be refused: `ls ..` from the workspace shows a sibling (M0 §2.4)",
            bad.display()
        );
    }
    assert!(check_artifact_placement(&t.path().join("artifacts"), &ws).is_ok());
}

#[test]
fn each_refused_relationship_says_which_one_it_was() {
    let ws = Path::new("/Users/team/goalcup-spikes/ws-one");
    let err = check_artifact_placement(Path::new("/Users/team/goalcup-spikes"), ws)
        .expect_err("an ancestor is enumerable with `ls ..`");
    assert!(format!("{err}").contains("ancestor"), "got {err}");

    let err = check_artifact_placement(Path::new("/Users/team/goalcup-spikes/ws-one/.goalcup"), ws)
        .expect_err("inside the workspace is the worst possible place");
    assert!(format!("{err}").contains("inside"), "got {err}");

    let err = check_artifact_placement(Path::new("/Users/team/goalcup-spikes/artifacts"), ws)
        .expect_err("M0 §2.4: the agent ran `ls -la ..` on its first tool call");
    assert!(format!("{err}").contains("sibling"), "got {err}");

    let err = check_artifact_placement(ws, ws).expect_err("identical paths");
    assert!(format!("{err}").contains("identical"), "got {err}");

    check_artifact_placement(
        Path::new("/Users/team/Library/Application Support/goalcup"),
        ws,
    )
    .expect("an unrelated tracker-owned directory is the correct placement");
}

#[test]
fn a_workspace_inside_a_git_worktree_or_holding_files_is_refused() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("repo/.git")).unwrap();
    let inside = t.path().join("repo/ws");
    std::fs::create_dir_all(&inside).unwrap();
    assert_eq!(git_worktree_of(&inside), Some(t.path().join("repo")));

    let roots = Roots {
        workspace: inside,
        artifact_root: t.path().join("art"),
        harness_home: t.path().join("home"),
    };
    let empty = Installation {
        files: Vec::new(),
        workspace_relative: Vec::new(),
    };
    assert!(
        create_workspace(&roots, "e1", &empty, &[]).is_err(),
        "R3: outside any Git worktree"
    );

    let ws = t.path().join("fresh");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("leftover.txt"), b"x").unwrap();
    let roots = Roots {
        workspace: ws,
        artifact_root: t.path().join("art"),
        harness_home: t.path().join("home"),
    };
    assert!(
        create_workspace(&roots, "e1", &empty, &[]).is_err(),
        "R3: zero entries"
    );
}

#[test]
fn a_clean_directory_outside_every_worktree_is_not_a_worktree() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("clean/ws");
    std::fs::create_dir_all(&outside).unwrap();
    assert_eq!(git_worktree_of(&outside), None);
}

#[test]
fn the_baseline_is_a_seal_manifest_and_a_fresh_workspace_proves_it_was_empty() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("ws/e1");
    let roots = Roots {
        workspace: ws.clone(),
        artifact_root: t.path().join("art"),
        harness_home: t.path().join("home"),
    };
    let installed = Installation {
        files: vec![TrackerFile {
            relative_path: ".claude/settings.json".to_string(),
            absolute_path: ws.join(".claude/settings.json"),
            scope: InstallScope::Workspace,
            contents: "{}".to_string(),
            sha256: sha256_hex(b"{}"),
            mode: goalcup_daemon::workspace::DEFAULT_MODE,
            restore_invalidates_trust: false,
            harness_owned: false,
        }],
        workspace_relative: vec![".claude/settings.json".to_string()],
    };
    // The union the seal will apply, handed in rather than rediscovered here.
    let excluded = vec![".claude/settings.json".to_string()];
    let w = create_workspace(&roots, "e1", &installed, &excluded).unwrap();

    assert_eq!(w.trace_dir, w.artifact_dir.join("trace"));
    assert!(
        w.baseline.files.is_empty(),
        "the zero-entry proof is an empty file list"
    );
    assert_eq!(w.baseline.entry_id, "e1");
    // The tracker's own file is excluded, so the seal never asks the agent to explain it.
    assert!(
        w.baseline
            .excluded
            .contains(&".claude/settings.json".to_string())
    );
    assert_eq!(
        w.expected_config[".claude/settings.json"],
        sha256_hex(b"{}")
    );
    assert!(ws.join(".claude/settings.json").exists());
}

/// `create_workspace` is still a public boundary for fixtures and sibling crates.  A caller
/// may therefore forge its own `Installation`; a file declared workspace-scoped must not
/// turn that raw absolute pathname into authority to write the entry artifact directory.
#[test]
fn create_workspace_refuses_a_forged_workspace_file_outside_its_declared_root_before_any_write() {
    let temp = tempfile::tempdir().expect("the fixture root exists");
    let roots = roots(temp.path());
    std::fs::write(roots.artifact_root.join("keep.txt"), b"ARTIFACT_SENTINEL")
        .expect("the artifact sentinel is written");
    let forged = Installation::from_files(vec![TrackerFile {
        relative_path: ".claude/settings.json".to_string(),
        absolute_path: roots.artifact_root.join("settings-launch.json"),
        scope: InstallScope::Workspace,
        contents: "receiver-token-that-must-not-escape".to_string(),
        sha256: sha256_hex(b"receiver-token-that-must-not-escape"),
        mode: goalcup_daemon::workspace::DEFAULT_MODE,
        restore_invalidates_trust: false,
        harness_owned: false,
    }]);

    let error = create_workspace(&roots, "e1", &forged, &[".claude/settings.json".into()])
        .expect_err("a declared workspace file cannot carry an artifact-root write pathname");

    assert!(
        error
            .to_string()
            .contains("does not match its Workspace root"),
        "the refusal names the forged scope/path mismatch: {error}"
    );
    assert!(
        !roots.workspace.exists(),
        "the path check runs before workspace creation or any other setup write"
    );
    assert_eq!(
        tree(&roots.artifact_root),
        vec!["keep.txt"],
        "the forged receiver-token settings file never reaches the artifact directory"
    );
}

#[test]
fn the_seal_walk_and_the_baseline_walk_apply_the_same_exclusion_union() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("ws");
    std::fs::create_dir_all(ws.join(".opencode/plugin")).unwrap();
    std::fs::create_dir_all(ws.join("src/.git")).unwrap();
    std::fs::write(ws.join(".opencode/plugin/goalcup-tracker.ts"), b"tracker").unwrap();
    std::fs::write(ws.join("src/.git/HEAD"), b"planted").unwrap();
    std::fs::write(ws.join("main.py"), b"print(1)").unwrap();

    let excluded = vec![
        ".git".to_string(),
        ".opencode/plugin/goalcup-tracker.ts".to_string(),
    ];
    let m = list_workspace(&ws, "e1", "2026-09-08T00:00:00Z", &excluded).unwrap();
    let paths: Vec<&str> = m.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"main.py"));
    assert!(
        !paths.contains(&".opencode/plugin/goalcup-tracker.ts"),
        "the tracker's own file"
    );
    // Anchored `is_excluded`: a nested .git is the agent's work.
    assert!(
        paths.contains(&"src/.git/HEAD"),
        "`mkdir src/.git` must not hide a subtree from the seal"
    );
    assert_eq!(
        m.excluded,
        {
            let mut e = excluded.clone();
            e.sort();
            e
        },
        "SealManifest.excluded carries the union that was applied"
    );
    assert_eq!(
        m.files,
        m.clone().sorted().files,
        "deterministic order, always"
    );
    // Digests, not names: `attribute_seal` compares path and digest, so a baselined file
    // rewritten under a new digest is not `Baseline`.
    let main = m.files.iter().find(|f| f.path == "main.py").unwrap();
    assert_eq!(main.sha256, sha256_hex(b"print(1)"));
    assert_eq!(main.bytes, 8);
}

// ---------------------------------------------------------------- path templates

#[test]
fn claude_slug_matches_the_spike_4_project_directory() {
    assert_eq!(
        claude_slug(Path::new(
            "/Users/team/goalcup-spikes/ws-claude-code-cli-20260903T073054Z"
        )),
        "-Users-team-goalcup-spikes-ws-claude-code-cli-20260903T073054Z"
    );
}

#[test]
fn templates_resolve_every_variable_and_glob_the_unknown_session_id() {
    let vars = PathVars::new(
        PathBuf::from("/Users/team/goalcup-spikes/ws-one"),
        PathBuf::from("/Users/team"),
        PathBuf::from("/home/h/.claude"),
        PathBuf::from("/var/goalcup"),
        None,
        ("2026".into(), "09".into(), "04".into()),
    );
    assert_eq!(
        resolve_template("{config_dir}/projects/{slug}/{session_id}.jsonl", &vars).unwrap(),
        "/home/h/.claude/projects/-Users-team-goalcup-spikes-ws-one/*.jsonl",
        "an unknown session id becomes a glob, never an empty segment"
    );
    // The canonical workspace is resolved once, at construction, so a bound variant is
    // built by cloning and setting the one field that moves rather than by a struct literal.
    let mut bound = vars.clone();
    bound.session_id = Some("95addb64".into());
    assert_eq!(
        resolve_template("{config_dir}/projects/{slug}/{session_id}.jsonl", &bound).unwrap(),
        "/home/h/.claude/projects/-Users-team-goalcup-spikes-ws-one/95addb64.jsonl"
    );
    assert_eq!(
        resolve_template("{artifact_root}/raw-bodies/", &bound).unwrap(),
        "/var/goalcup/raw-bodies/"
    );
    // Codex's date-partitioned tree, which is why `yyyy`/`mm`/`dd` are tokens at all.
    assert_eq!(
        resolve_template(
            "{config_dir}/sessions/{yyyy}/{mm}/{dd}/rollout-*.jsonl",
            &bound
        )
        .unwrap(),
        "/home/h/.claude/sessions/2026/09/04/rollout-*.jsonl"
    );
    // One strict renderer, so a typo is an error rather than a glob that silently never
    // matches.
    let err = resolve_template("{config-dir}/projects", &bound)
        .expect_err("`config-dir` is not one of the nine tokens; `config_dir` is");
    assert!(format!("{err}").contains("config-dir"), "got {err}");
}

#[test]
fn every_path_template_in_every_manifest_uses_only_the_nine_tokens() {
    let vars = PathVars::new(
        PathBuf::from("/tmp/ws-one"),
        PathBuf::from("/Users/team"),
        PathBuf::from("/tmp/home/.config"),
        PathBuf::from("/var/goalcup/entry-0001"),
        Some("95addb64".into()),
        ("2026".into(), "09".into(), "04".into()),
    );
    for id in ["claude-code-cli", "codex-cli"] {
        let m = manifest(id);
        for store in &m.store {
            for template in [
                Some(store.path_template.clone()),
                store.subagent_path_template.clone(),
                store.spill_path_template.clone(),
            ]
            .into_iter()
            .flatten()
            {
                resolve_template(&template, &vars)
                    .unwrap_or_else(|e| panic!("{id}/{}: {template}: {e}", store.name));
            }
        }
        for lock in &m.lock_paths {
            resolve_template(lock, &vars).unwrap_or_else(|e| panic!("{id}: {lock}: {e}"));
        }
        for allowed in &m.config_allowlist {
            resolve_template(allowed, &vars).unwrap_or_else(|e| panic!("{id}: {allowed}: {e}"));
        }
        for scan in &m.foreign_scan {
            resolve_template(&scan.path_glob, &vars)
                .unwrap_or_else(|e| panic!("{id}: {}: {e}", scan.path_glob));
        }
    }
}

#[test]
fn globs_expand_within_a_component_and_a_missing_tree_yields_nothing() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("sessions/2026/09/04");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["rollout-b.jsonl", "rollout-a.jsonl", "notes.txt"] {
        std::fs::write(root.join(name), b"x").unwrap();
    }
    let hits = expand_globs(&format!("{}/rollout-*.jsonl", root.display()));
    assert_eq!(
        hits,
        vec![root.join("rollout-a.jsonl"), root.join("rollout-b.jsonl")],
        "sorted, so a poll order never depends on the filesystem's iteration order"
    );
    assert_eq!(
        expand_globs(&format!(
            "{}/sessions/*/*/*/rollout-*.jsonl",
            t.path().display()
        ))
        .len(),
        2,
        "a `*` in the middle of a path walks that level"
    );
    assert!(
        expand_globs(&format!("{}/nothing/here/*.jsonl", t.path().display())).is_empty(),
        "a store whose directory does not exist yet is normal before the harness starts"
    );
    let plain = root.join("notes.txt");
    assert_eq!(expand_globs(&plain.to_string_lossy()), vec![plain]);
}

// ---------------------------------------------------------------- the install

#[test]
fn every_connector_renders_files_that_parse_and_hold_no_placeholder() {
    for id in ["claude-code-cli", "codex-cli"] {
        let dir = tempfile::tempdir().unwrap();
        let r = roots(dir.path());
        let m = manifest(id);
        let done = render_tracker_files(&m, &connector_dir(id), &vars(&r), &r, false)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!done.files.is_empty(), "{id} rendered nothing");
        for f in &done.files {
            assert!(
                !f.contents.contains("{receiver_token}"),
                "{id}: {}",
                f.relative_path
            );
            assert!(
                !f.contents.contains("{workspace}"),
                "{id}: {}",
                f.relative_path
            );
            assert_eq!(f.sha256, sha256_hex(f.contents.as_bytes()));
            if f.relative_path.ends_with(".json") {
                serde_json::from_str::<serde_json::Value>(&f.contents)
                    .unwrap_or_else(|e| panic!("{id}: {} is not JSON: {e}", f.relative_path));
            }
        }
    }
}

#[test]
fn rendering_writes_outside_the_workspace_and_defers_the_workspace_scoped_files() {
    // The ordering: the workspace must be proved empty before anything lands in it, so
    // `render_tracker_files` writes the artifact-root and harness-home scopes only and
    // `create_workspace` writes the rest once it has counted zero entries.
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    let m = manifest("claude-code-cli");
    let done =
        render_tracker_files(&m, &connector_dir("claude-code-cli"), &vars(&r), &r, false).unwrap();
    assert!(r.artifact_root.join("settings-launch.json").is_file());
    assert!(
        !r.workspace.exists(),
        "nothing may touch the workspace before it is proved empty"
    );
    assert_eq!(
        done.workspace_relative,
        Vec::<String>::new(),
        "Claude installs nothing into the workspace since its settings shim went"
    );
}

/// Fresh workspace setup receives the entry capability before it renders Claude's launch
/// settings. Replacing the display spelling afterwards cannot redirect
/// that write to a directory outside the configured artifact root.
#[cfg(unix)]
#[test]
fn prepared_workspace_writes_claude_launch_settings_through_the_retained_entry() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("temporary root");
    let configured_root = dir.path().join("tracker-artifacts");
    std::fs::create_dir(&configured_root).expect("configured root");
    let r = Roots {
        workspace: dir.path().join("holder/ws-one"),
        artifact_root: configured_root.join("artifacts/entry-0001"),
        harness_home: configured_root.join("homes/entry-0001"),
    };
    std::fs::create_dir_all(&r.harness_home).expect("harness home");
    let artifact = EntryArtifactRoot::create(&configured_root, "entry-0001")
        .expect("the fresh entry capability is acquired");
    let outside = dir.path().join("outside-artifact-sentinel");
    std::fs::create_dir(&outside).expect("outside directory");
    std::fs::write(
        outside.join("settings-launch.json"),
        b"OUTSIDE_LAUNCH_SETTINGS_SENTINEL",
    )
    .expect("outside sentinel");
    let detached = dir.path().join("detached-entry-artifact");
    std::fs::rename(&r.artifact_root, &detached).expect("detach retained entry");
    symlink(&outside, &r.artifact_root).expect("swap mutable entry spelling");

    prepare_with_artifact(
        &manifest("claude-code-cli"),
        &connector_dir("claude-code-cli"),
        &vars(&r),
        &path_vars(&r),
        &r,
        "entry-0001",
        &artifact,
    )
    .expect("workspace setup retains its entry authority");

    assert!(
        String::from_utf8(
            std::fs::read(detached.join("settings-launch.json"))
                .expect("detached retained entry receives the launch settings")
        )
        .unwrap()
        .contains("\"statusLine\""),
        "the retained launch settings are the rendered ones"
    );
    assert_eq!(
        std::fs::read(outside.join("settings-launch.json")).expect("outside sentinel remains"),
        b"OUTSIDE_LAUNCH_SETTINGS_SENTINEL"
    );
}

#[test]
fn claude_installs_one_file_in_the_artifact_root_and_the_status_line_reads_a_plain_file() {
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    let m = manifest("claude-code-cli");
    let ws = prepare(
        &m,
        &connector_dir("claude-code-cli"),
        &vars(&r),
        &path_vars(&r),
        &r,
        "entry-0001",
    )
    .unwrap();

    assert_eq!(
        ws.installed.workspace_relative,
        Vec::<String>::new(),
        "no tracker file lands in the workspace on Claude since its settings shim went"
    );
    let launch = ws
        .installed
        .files
        .iter()
        .find(|f| f.scope == InstallScope::ArtifactRoot)
        .expect("the --settings file goes to the artifact root");
    assert_eq!(
        launch.absolute_path,
        r.artifact_root.join("settings-launch.json")
    );
    assert!(launch.absolute_path.is_file());

    let v: serde_json::Value = serde_json::from_str(&launch.contents).unwrap();
    assert!(
        v.get("hooks").is_none(),
        "the hook object went with the hook leg"
    );
    assert_eq!(v["statusLine"]["type"], "command");
    let cmd = v["statusLine"]["command"].as_str().unwrap();
    assert!(
        cmd.starts_with("cat "),
        "R18: the status line reads a file, it holds no secret"
    );
    assert!(!cmd.contains("Bearer") && !cmd.contains("0123456789abcdef"));

    // The baseline excludes the tracker's own files, so a fresh workspace is empty.
    assert!(ws.baseline.files.is_empty(), "got {:?}", ws.baseline);
    assert_eq!(ws.baseline.entry_id, "entry-0001");
    assert_eq!(ws.artifact_dir, r.artifact_root);
    assert_eq!(ws.trace_dir, r.artifact_root.join("trace"));
    assert!(!ws.artifact_dir.starts_with(&ws.root));
}

#[test]
fn codex_installs_only_its_harness_config() {
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    let m = manifest("codex-cli");
    let ws = prepare(
        &m,
        &connector_dir("codex-cli"),
        &vars(&r),
        &path_vars(&r),
        &r,
        "entry-0001",
    )
    .unwrap();

    assert!(
        ws.installed.workspace_relative.is_empty(),
        "the hooks.json and hook-post.sh shims went with the hook leg: {:?}",
        ws.installed.workspace_relative
    );
    assert!(!r.workspace.join(".codex").exists());
    let config = ws
        .installed
        .files
        .iter()
        .find(|f| f.relative_path == "config.toml")
        .expect("codex writes its own config.toml into the harness home");
    assert_eq!(config.scope, InstallScope::HarnessHome);
    assert_eq!(
        config.sha256,
        sha256_hex(&std::fs::read(&config.absolute_path).unwrap()),
        "the recorded hash is the bytes on disk"
    );
}

#[test]
fn the_exclusion_union_is_the_connector_list_plus_the_trackers_own_files() {
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    // On Claude the tracker installs nothing into the workspace, so the union is
    // the connector's own list.
    let m = manifest("claude-code-cli");
    let installed =
        render_tracker_files(&m, &connector_dir("claude-code-cli"), &vars(&r), &r, false).unwrap();
    let union = exclusion_union(&m, &installed);
    assert!(union.contains(&".claude".to_string()));
    // Sorted and deduplicated. The case that exercised the dedup, OpenCode naming its own
    // plugin in `workspace_exclusions[]` and installing it too, left with that connector in
    // the m2/wire deletion pass.
    let mut sorted = union.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(union, sorted);
}

#[test]
fn preparing_over_a_used_workspace_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    let m = manifest("claude-code-cli");
    prepare(
        &m,
        &connector_dir("claude-code-cli"),
        &vars(&r),
        &path_vars(&r),
        &r,
        "entry-0001",
    )
    .unwrap();
    // The first entry's work. Claude installs nothing into the workspace, so the
    // run's own file is what makes the workspace a used one.
    std::fs::write(r.workspace.join("hello.py"), "print(1)\n").unwrap();
    let before = tree(&r.workspace);
    assert!(!before.is_empty(), "the first entry left something");

    // A second entry with its own artifact root and harness home, pointed at the workspace
    // the first one already used.
    let second = Roots {
        workspace: r.workspace.clone(),
        artifact_root: dir.path().join("tracker-artifacts/entry-0002"),
        harness_home: dir.path().join("tracker-artifacts/homes/entry-0002"),
    };
    std::fs::create_dir_all(&second.artifact_root).unwrap();
    std::fs::create_dir_all(&second.harness_home).unwrap();

    let err = prepare(
        &m,
        &connector_dir("claude-code-cli"),
        &vars(&second),
        &path_vars(&second),
        &second,
        "entry-0002",
    )
    .expect_err("a workspace with entries is not a fresh workspace");
    assert!(format!("{err}").contains("not empty"), "got {err}");

    // A refusal leaves nothing on disk. `settings-launch.json` is artifact-root-scoped, so
    // rendering it before the workspace checks have run would leave a file behind for a run
    // that never started and that nothing will ever clean up.
    assert_eq!(
        tree(&second.artifact_root),
        Vec::<String>::new(),
        "a refused prepare left files in the artifact root"
    );
    assert_eq!(
        tree(&second.harness_home),
        Vec::<String>::new(),
        "a refused prepare left files in the harness home"
    );
    assert_eq!(
        tree(&r.workspace),
        before,
        "a refused prepare touched the workspace it refused"
    );
}

#[test]
fn the_app_wide_allowlist_is_hashed_at_launch_and_never_restored() {
    // D21. These are the participant's own files, outside every
    // tracker-created folder: hashed so a mid-run change is reportable, and never written
    // to (AGENTS.md "never touch app-wide harness config").
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    let pv = path_vars(&r);
    let m = manifest("claude-code-cli");
    assert!(
        !m.config_allowlist.is_empty(),
        "claude-code-cli names the app-wide settings files it wants watched"
    );

    // One of them exists at launch; the rest do not.
    let settings = pv.home.join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, "{\"model\":\"opus\"}\n").unwrap();

    let base = baseline_allowlist(&m, &pv).unwrap();
    assert!(
        !base.is_empty(),
        "every allowlisted path gets a row, present or not"
    );
    let present = base
        .iter()
        .find(|a| a.path == settings)
        .expect("the existing file is baselined");
    assert!(present.existed);
    assert_eq!(present.sha256, sha256_hex(b"{\"model\":\"opus\"}\n"));
    assert!(
        present.mtime_unix.is_some(),
        "D21 asks for the mtime as well as the hash"
    );

    assert!(
        base.iter().any(|a| !a.existed
            && a.sha256 == "0000000000000000000000000000000000000000000000000000000000000000"),
        "a path that does not exist yet is recorded as absent, because its later \
         appearance is exactly the event R9's third clause is for"
    );

    // Sorted and unique, so two globs that overlap do not produce two rows.
    let mut sorted = base.clone();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(base, sorted);
    let unique: std::collections::BTreeSet<_> = base.iter().map(|a| &a.path).collect();
    assert_eq!(unique.len(), base.len());

    // The tracker did not write any of them.
    let ws = prepare(
        &m,
        &connector_dir("claude-code-cli"),
        &vars(&r),
        &pv,
        &r,
        "entry-0001",
    )
    .unwrap();
    assert_eq!(ws.allowlist.len(), base.len());
    // That none of them is a file the tracker wrote is asserted for every shipped
    // connector by `no_connector_allowlists_a_file_it_installs`, below.
}

#[test]
fn no_connector_allowlists_a_file_it_installs() {
    // The two lists mean opposite things. `config_allowlist[]` is
    // D21: app-wide files the tracker did not write, baselined so a change is reportable
    // and never restored. `hook_install[]` is the tracker's own config, restored on
    // deviation. A path on both asks the daemon to restore a file and to leave it alone,
    // and whichever half wins, the manifest says the other. So the intersection is empty
    // for every shipped connector.
    for id in ["claude-code-cli", "codex-cli"] {
        let dir = tempfile::tempdir().unwrap();
        let r = roots(dir.path());
        let m = manifest(id);
        let ws = prepare(
            &m,
            &connector_dir(id),
            &vars(&r),
            &path_vars(&r),
            &r,
            "entry-0001",
        )
        .unwrap_or_else(|e| panic!("{id}: {e}"));
        // Optional installs count too: a file the daemon writes only sometimes is still a
        // file the daemon writes.
        let optional = render_tracker_files(&m, &connector_dir(id), &vars(&r), &r, true)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        let installed: Vec<PathBuf> = ws
            .installed
            .files
            .iter()
            .chain(optional.files.iter())
            .map(|f| f.absolute_path.clone())
            .collect();
        for a in &ws.allowlist {
            assert!(
                !installed.contains(&a.path),
                "{id}: {} is on config_allowlist[] and is also a hook_install[] file; \
                 it cannot be both never-restored and restore-on-deviation",
                a.path.display()
            );
        }
    }
}

#[test]
fn the_protocol_view_carries_what_the_cli_needs_and_no_more() {
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    let m = manifest("claude-code-cli");
    let ws = prepare(
        &m,
        &connector_dir("claude-code-cli"),
        &vars(&r),
        &path_vars(&r),
        &r,
        "entry-0001",
    )
    .unwrap();
    let view = ws.installed.as_protocol();
    assert_eq!(view.len(), 1);
    // The CLI builds `--settings <path>` from this and renders nothing itself.
    let launch = view
        .iter()
        .find(|f| f.relative_path == "settings-launch.json")
        .unwrap();
    assert_eq!(launch.scope, InstallScope::ArtifactRoot);
    assert_eq!(launch.sha256.len(), 64);
    assert!(launch.bytes > 0);
}

#[cfg(unix)]
#[test]
fn a_symlinked_artifact_root_is_refused_by_task_1s_resolving_check() {
    // The four clauses above compare the paths as written. `..` after a symlink does not
    // mean what it reads as: with `link -> art/inner`, the kernel resolves `link/../ws` to
    // `art/ws`, inside the artifact root, while the lexical answer is `<t>/ws`.
    // `nests` is what sees that, and `check_artifact_placement` calls it.
    let t = tempfile::tempdir().unwrap();
    let art = t.path().join("art");
    std::fs::create_dir_all(art.join("inner")).unwrap();
    std::os::unix::fs::symlink(art.join("inner"), t.path().join("link")).unwrap();
    let ws = t.path().join("link/../ws");
    let err = check_artifact_placement(&art, &ws)
        .expect_err("the workspace resolves to a directory inside the artifact root");
    assert!(
        format!("{err}").contains("symlinks are resolved"),
        "got {err}"
    );
}

/// `harness_owned`: a file the per-run home already holds is left byte for byte, and one
/// it does not is written exactly as any other `hook_install[]` entry.
///
/// Measured on the shipped codex connector rather than on a fixture manifest, because the
/// point of the flag is which file it is on. The seeded body is Codex's own config, so the
/// "already there" case here is the case the flag exists for: a participant who ran
/// `CODEX_HOME=<home> codex login` before the start, or a resume into a home a previous leg
/// left behind, keeps the trust table and the tip counter Codex wrote into it.
#[test]
fn a_harness_owned_file_the_home_already_holds_is_left_exactly_as_it_is() {
    let already =
        b"# mine\ncheck_for_update_on_startup = true\n\n[tui.model_availability_nux]\ngpt = 3\n";

    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    let m = manifest("codex-cli");
    std::fs::write(r.harness_home.join("config.toml"), already).unwrap();
    let ws = prepare(
        &m,
        &connector_dir("codex-cli"),
        &vars(&r),
        &path_vars(&r),
        &r,
        "entry-0001",
    )
    .unwrap();
    assert_eq!(
        std::fs::read(r.harness_home.join("config.toml")).unwrap(),
        already,
        "the harness owns this file; a home that already held one keeps it"
    );
    // It is still disclosed — the tracker declares every file it would install — and the
    // digest it is disclosed under is what was rendered. Nothing compares that digest to
    // the bytes on disk, which is exactly what `ConfigWatch::new`'s filter guarantees.
    let listed = ws
        .installed
        .files
        .iter()
        .find(|f| f.relative_path == "config.toml")
        .expect("the entry is still declared");
    assert!(listed.harness_owned);
    assert_eq!(listed.sha256, sha256_hex(listed.contents.as_bytes()));

    // …and the ordinary case, in the same test, so "left as it is" cannot pass by never
    // writing at all.
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    let ws = prepare(
        &m,
        &connector_dir("codex-cli"),
        &vars(&r),
        &path_vars(&r),
        &r,
        "entry-0001",
    )
    .unwrap();
    let written = std::fs::read(r.harness_home.join("config.toml")).unwrap();
    assert_ne!(written, already);
    assert_eq!(
        sha256_hex(&written),
        ws.installed
            .files
            .iter()
            .find(|f| f.relative_path == "config.toml")
            .unwrap()
            .sha256,
        "an absent file is written, and it is the bytes the tracker recorded"
    );
}

#[test]
fn every_installed_file_hashes_to_the_bytes_that_are_actually_on_disk() {
    // The recorded digest is what the config watch compares against on every poll, so a
    // digest of anything but the bytes as written would report a deviation that never
    // happened — or, worse, miss one that did.
    for id in ["claude-code-cli", "codex-cli"] {
        let dir = tempfile::tempdir().unwrap();
        let r = roots(dir.path());
        let m = manifest(id);
        let ws = prepare(
            &m,
            &connector_dir(id),
            &vars(&r),
            &path_vars(&r),
            &r,
            "entry-0001",
        )
        .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!ws.installed.files.is_empty(), "{id} installed nothing");
        for f in &ws.installed.files {
            let on_disk = std::fs::read(&f.absolute_path)
                .unwrap_or_else(|e| panic!("{id}: {}: {e}", f.absolute_path.display()));
            assert_eq!(f.sha256, sha256_hex(&on_disk), "{id}: {}", f.relative_path);
            assert_eq!(
                f.contents.as_bytes(),
                on_disk.as_slice(),
                "{id}: {}",
                f.relative_path
            );
            // Nothing lands outside the three roots this crate's
            // writes are bounded to. The harness home is one of them: codex-cli renders
            // this run's `config.toml` there, a
            // `scope = "harness_home"` entry.
            assert!(
                f.absolute_path.starts_with(&r.workspace)
                    || f.absolute_path.starts_with(&r.artifact_root)
                    || f.absolute_path.starts_with(&r.harness_home),
                "{id}: {} escaped all three roots",
                f.absolute_path.display()
            );
        }
        // The baseline is the zero-entry proof, and its exclusion list is the union the
        // seal will apply again.
        assert!(
            ws.baseline.files.is_empty(),
            "{id}: {:?}",
            ws.baseline.files
        );
        assert_eq!(
            ws.baseline.excluded,
            exclusion_union(&m, &ws.installed),
            "{id}: SealManifest.excluded must carry the union that was applied"
        );
        assert_eq!(ws.baseline.workspace, r.workspace.to_string_lossy());
    }
}

/// The per-run bearer never lands in a file a second local account can read.
///
/// Every shipped connector, every scope, every rendered file: a file whose template
/// interpolates `{receiver_token}` is written owner-only, and one that does not keeps the
/// mode its manifest declares. Asserted over every shipped connector rather than over a list of
/// paths, so a fourth connector — or a fifth template on an existing one — is covered by
/// this test the day it is added rather than the day somebody remembers to extend it.
///
/// The mode is read off disk, not off `TrackerFile.mode`: the field is what the renderer
/// decided and the bits are what a reader of the participant's home would find.
#[cfg(unix)]
#[test]
fn every_rendered_file_carrying_the_receiver_token_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let mut token_files = 0;
    for id in ["claude-code-cli", "codex-cli"] {
        let dir = tempfile::tempdir().unwrap();
        let r = roots(dir.path());
        let m = manifest(id);
        // No shipped template interpolates the token, so a copy of Codex's `config.toml`
        // is given a line that does: the owner-only rule stays covered on a real render.
        let planted = tempfile::tempdir().unwrap();
        let source = if id == "codex-cli" {
            let copy = planted.path().join(id);
            std::fs::create_dir_all(copy.join("files")).unwrap();
            std::fs::copy(
                connector_dir(id).join("manifest.toml"),
                copy.join("manifest.toml"),
            )
            .unwrap();
            let body =
                std::fs::read_to_string(connector_dir(id).join("files/config.toml")).unwrap();
            std::fs::write(
                copy.join("files/config.toml"),
                format!("{body}\n# token = \"{{receiver_token}}\"\n"),
            )
            .unwrap();
            copy
        } else {
            connector_dir(id)
        };
        let ws = prepare(&m, &source, &vars(&r), &path_vars(&r), &r, "entry-0001").unwrap();

        for file in &ws.installed.files {
            let on_disk = std::fs::metadata(&file.absolute_path)
                .unwrap_or_else(|e| panic!("{}: {e}", file.absolute_path.display()))
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(
                on_disk, file.mode,
                "{id} {}: the bits on disk are the ones the renderer decided",
                file.relative_path
            );
            // The token this run was given, in the bytes as installed. `vars()` is what
            // supplied it, so this finds the same string the templates interpolated.
            if file.contents.contains(RECEIVER_TOKEN) {
                token_files += 1;
                assert_eq!(
                    on_disk & 0o077,
                    0,
                    "{id} {} carries the run's bearer at mode {on_disk:o}: a second local \
                     account reads it and posts forged leg-2 records",
                    file.relative_path
                );
            }
        }
    }
    // Only the planted Codex line interpolates the token (see above). Counted so a template that stopped interpolating it — and therefore stopped being
    // covered — cannot make this test pass by asserting nothing.
    assert_eq!(
        token_files, 1,
        "the planted template interpolates the receiver token"
    );
}
