//! Where the artifact root may sit relative to the workspace, that the workspace is a new
//! directory outside any Git worktree with zero entries, that the workspace walk applies its
//! exclusions the way the seal reads them, and the connector path templates. The V1
//! `hook_install[]` rendering these tests also covered was removed by OS-31.

use goalcup_daemon::paths::{PathVars, claude_slug, expand_globs, resolve_template};
use goalcup_daemon::workspace::*;
use goalcup_protocol::manifest::ConnectorManifest;
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

// ---------------------------------------------------------------- the workspace

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
    };
    assert!(
        check_workspace_available(&roots).is_err(),
        "R3: outside any Git worktree"
    );

    let ws = t.path().join("fresh");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("leftover.txt"), b"x").unwrap();
    let roots = Roots {
        workspace: ws,
        artifact_root: t.path().join("art"),
    };
    assert!(
        check_workspace_available(&roots).is_err(),
        "R3: zero entries"
    );

    let roots = Roots {
        workspace: t.path().join("not-yet"),
        artifact_root: t.path().join("art"),
    };
    check_workspace_available(&roots).expect("a workspace that does not exist yet is empty");
}

#[test]
fn a_clean_directory_outside_every_worktree_is_not_a_worktree() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("clean/ws");
    std::fs::create_dir_all(&outside).unwrap();
    assert_eq!(git_worktree_of(&outside), None);
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
