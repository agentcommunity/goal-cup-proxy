//! Where the artifact root may sit relative to the workspace, that the workspace is a new
//! directory outside any Git worktree with zero entries, and that the workspace walk applies its
//! exclusions the way the seal reads them.

use goalcup_daemon::workspace::*;
use std::path::Path;

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
        artifact_root: t.path().join("tracker/art"),
    };
    let err = check_workspace_available(&roots).expect_err("R3: outside any Git worktree");
    assert!(format!("{err}").contains("Git worktree"), "got {err}");

    let ws = t.path().join("fresh");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("leftover.txt"), b"x").unwrap();
    let roots = Roots {
        workspace: ws,
        artifact_root: t.path().join("tracker/art"),
    };
    let err = check_workspace_available(&roots).expect_err("R3: zero entries");
    assert!(format!("{err}").contains("not empty"), "got {err}");

    let roots = Roots {
        workspace: t.path().join("not-yet"),
        artifact_root: t.path().join("tracker/art"),
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

/// Review R5 I-3: the agent can leave a FIFO, a link to an endless device or an unreadable
/// directory in its workspace. The seal walk must still finish, list each one with the absent
/// digest, and hash the ordinary files. On the old walk the FIFO blocked `open` forever.
#[test]
fn the_seal_walk_finishes_over_a_fifo_a_device_link_and_an_unreadable_directory() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: reading the effective uid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return; // `chmod 000` does not lock root out.
    }
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("ws");
    std::fs::create_dir_all(ws.join("locked")).unwrap();
    std::fs::write(ws.join("locked/inside.txt"), b"hidden").unwrap();
    std::fs::write(ws.join("main.py"), b"print(1)").unwrap();
    let fifo = std::ffi::CString::new(ws.join("pipe").to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path and a plain mode.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    std::os::unix::fs::symlink("/dev/zero", ws.join("zero")).unwrap();
    std::os::unix::fs::symlink(ws.join("main.py"), ws.join("link.py")).unwrap();
    std::fs::set_permissions(ws.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let walked = ws.clone();
    std::thread::spawn(move || {
        let _ = tx.send(list_workspace(&walked, "e1", "2026-10-13T00:00:00Z", &[]));
    });
    let listed = rx.recv_timeout(std::time::Duration::from_secs(10));
    std::fs::set_permissions(ws.join("locked"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let m = listed
        .expect("the walk finished")
        .expect("an unreadable directory is listed, not an error");

    let digest = |p: &str| {
        m.files
            .iter()
            .find(|f| f.path == p)
            .unwrap_or_else(|| panic!("{p} is listed: {:?}", m.files))
            .sha256
            .clone()
    };
    let absent = "0".repeat(64);
    assert_eq!(digest("main.py"), sha256_hex(b"print(1)"));
    for p in ["pipe", "zero", "link.py", "locked"] {
        assert_eq!(digest(p), absent, "{p}");
    }
}
