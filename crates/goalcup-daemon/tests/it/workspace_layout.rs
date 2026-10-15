//! Where the artifact root may sit relative to the workspace, that the workspace is a new
//! directory outside any Git worktree with zero entries, and that the workspace walk applies its
//! exclusions the way the seal reads them.

use goalcup_daemon::workspace::*;
use goalcup_protocol::seal::GitState;
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

// ---------------------------------------------------------------- a clone of the participant's repository (D56)

/// Run `git` in `ws` with the seal's own sanitized environment and a test identity; `home` is
/// the test's empty `HOME`, so no user or system configuration reaches the repository.
fn git(ws: &Path, home: &Path, args: &[&str]) {
    let status = goalcup_daemon::workspace_git::command(ws, home)
        .expect("git on PATH")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

/// A repository at `<t>/ws` with one commit holding `a.txt`, a link to it, a file whose name
/// ends in a carriage return and a `.gitignore` for `target`.
fn committed_clone(t: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let ws = t.join("ws");
    let home = t.join("home");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    git(&ws, &home, &["init", "-q"]);
    std::fs::write(ws.join("a.txt"), b"one\n").unwrap();
    std::fs::write(ws.join(".gitignore"), b"target\n").unwrap();
    std::os::unix::fs::symlink("a.txt", ws.join("link.txt")).unwrap();
    // A name a line read would cut short: `hash-object` must still hash this file.
    std::fs::write(ws.join("cr\r"), b"carriage\n").unwrap();
    git(
        &ws,
        &home,
        &["add", "a.txt", ".gitignore", "link.txt", "cr\r"],
    );
    git(&ws, &home, &["commit", "-q", "-m", "first"]);
    (ws, home)
}

fn head(ws: &Path, home: &Path) -> String {
    let out = goalcup_daemon::workspace_git::command(ws, home)
        .expect("git on PATH")
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

const BUDGET: std::time::Duration = std::time::Duration::from_secs(30);

#[test]
fn a_fresh_clone_seals_no_commit_and_the_empty_digest() {
    use goalcup_daemon::workspace_git::state;
    let empty = sha256_hex(b"diff\0untracked\0");

    // An unborn HEAD: the clone of an empty repository.
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("ws");
    let home = t.path().join("home");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    git(&ws, &home, &["init", "-q"]);
    let got = state(&ws, BUDGET).expect("a Git work folder has a state");
    assert_eq!(got.commit, None, "no commit yet");
    assert_eq!(got.dirty_sha256, empty);

    // A clean clone with a commit: the commit, and still nothing uncommitted. An ignored file
    // is not uncommitted work.
    let t = tempfile::tempdir().unwrap();
    let (ws, home) = committed_clone(t.path());
    std::fs::create_dir_all(ws.join("target")).unwrap();
    std::fs::write(ws.join("target/build.o"), b"ignored").unwrap();
    let got = state(&ws, BUDGET).unwrap();
    assert_eq!(got.commit.as_deref(), Some(head(&ws, &home).as_str()));
    assert_eq!(got.commit.as_ref().unwrap().len(), 40);
    assert_eq!(got.dirty_sha256, empty);

    // Not a Git work folder: no state, and the walk takes over.
    let plain = t.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert!(state(&plain, BUDGET).is_err());
}

#[test]
fn the_digest_moves_with_an_untracked_file_and_a_tracked_edit() {
    use goalcup_daemon::workspace_git::state;
    let t = tempfile::tempdir().unwrap();
    let (ws, _home) = committed_clone(t.path());
    let framed = |diff: &[u8], untracked: &[u8]| {
        let mut bytes = b"diff\0".to_vec();
        bytes.extend_from_slice(diff);
        bytes.extend_from_slice(b"untracked\0");
        bytes.extend_from_slice(untracked);
        sha256_hex(&bytes)
    };
    let line = |path: &str, value: &str| format!("{path}\0{value}\n").into_bytes();

    std::fs::write(ws.join("new.txt"), b"fresh\n").unwrap();
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::write(ws.join("src/lib.rs"), b"fn f() {}\n").unwrap();
    let untracked = [
        line("new.txt", &sha256_hex(b"fresh\n")),
        line("src/lib.rs", &sha256_hex(b"fn f() {}\n")),
    ]
    .concat();
    assert_eq!(
        state(&ws, BUDGET).unwrap().dirty_sha256,
        framed(b"", &untracked),
        "each untracked file, sorted, with its content digest"
    );

    std::fs::write(ws.join("a.txt"), b"two\n").unwrap();
    assert_eq!(
        state(&ws, BUDGET).unwrap().dirty_sha256,
        framed(&line("a.txt", &sha256_hex(b"two\n")), &untracked),
        "a tracked edit is listed with its new content"
    );

    std::fs::remove_file(ws.join("a.txt")).unwrap();
    assert_eq!(
        state(&ws, BUDGET).unwrap().dirty_sha256,
        framed(&line("a.txt", "deleted"), &untracked),
        "a tracked file removed is listed as deleted"
    );

    // A link pointed elsewhere is listed, never followed: the walk's absent digest.
    std::fs::remove_file(ws.join("link.txt")).unwrap();
    std::os::unix::fs::symlink("/dev/zero", ws.join("link.txt")).unwrap();
    let absent = "0".repeat(64);
    assert_eq!(
        state(&ws, BUDGET).unwrap().dirty_sha256,
        framed(
            &[line("a.txt", "deleted"), line("link.txt", &absent)].concat(),
            &untracked
        ),
    );
}

/// The agent writes `.git/config`: an fsmonitor hook there would run inside the daemon at the
/// seal. It must not.
#[test]
fn an_agent_written_fsmonitor_is_never_run() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let (ws, _home) = committed_clone(t.path());
    let marker = t.path().join("fsmonitor-ran");
    let hook = t.path().join("fsmonitor.sh");
    std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut config = std::fs::OpenOptions::new()
        .append(true)
        .open(ws.join(".git/config"))
        .unwrap();
    use std::io::Write;
    writeln!(
        config,
        "[core]\n\tfsmonitor = {}\n\tuntrackedCache = true",
        hook.display()
    )
    .unwrap();
    drop(config);
    std::fs::write(ws.join("a.txt"), b"edited\n").unwrap();
    std::fs::write(ws.join("b.txt"), b"new\n").unwrap();

    assert!(goalcup_daemon::workspace_git::state(&ws, BUDGET).is_ok());
    assert!(
        !marker.exists(),
        "the agent's fsmonitor ran inside the seal"
    );
}

/// Ruling A: `git diff` against the work tree runs a repository-configured clean filter (or a
/// long-running process filter) on every file `.gitattributes` assigns it to. The agent writes
/// both files, so the seal must never run one.
#[test]
fn an_agent_written_clean_filter_is_never_run() {
    let t = tempfile::tempdir().unwrap();
    let (ws, _home) = committed_clone(t.path());
    let clean = t.path().join("clean-ran");
    let process = t.path().join("process-ran");
    let mut config = std::fs::OpenOptions::new()
        .append(true)
        .open(ws.join(".git/config"))
        .unwrap();
    use std::io::Write;
    writeln!(
        config,
        "[filter \"evil\"]\n\tclean = \"touch '{}'; cat\"\n\tprocess = \"sh -c 'touch {}'\"",
        clean.display(),
        process.display()
    )
    .unwrap();
    drop(config);
    std::fs::write(ws.join(".gitattributes"), b"* filter=evil\n").unwrap();
    std::fs::write(ws.join(".git/info/attributes"), b"* filter=evil\n").unwrap_or(());
    std::fs::write(ws.join("a.txt"), b"edited\n").unwrap();

    let got = goalcup_daemon::workspace_git::state(&ws, BUDGET);
    assert!(
        !process.exists(),
        "the agent's process filter ran inside the seal"
    );
    assert!(
        !clean.exists(),
        "the agent's clean filter ran inside the seal"
    );
    let got = got.expect("a filter the seal does not run cannot fail it either");
    assert_ne!(
        got.dirty_sha256,
        sha256_hex(b"diff\0untracked\0"),
        "the edit is seen"
    );
}

#[test]
fn a_workspace_inside_another_work_tree_is_refused() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("outer/.git")).unwrap();
    let art = t.path().join("tracker/art");

    // A clone inside another work tree: its own `.git` does not make it a root.
    let nested = t.path().join("outer/ws");
    std::fs::create_dir_all(nested.join(".git")).unwrap();
    std::fs::write(nested.join("a.txt"), b"x").unwrap();
    let err = check_workspace_available(&Roots {
        workspace: nested,
        artifact_root: art.clone(),
    })
    .expect_err("R3: a work tree inside another is refused");
    assert!(format!("{err}").contains("Git worktree"), "got {err}");

    // A linked worktree (`.git` is a file) is not the root of its own clone.
    let linked = t.path().join("linked");
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(linked.join(".git"), b"gitdir: /elsewhere\n").unwrap();
    let err = check_workspace_available(&Roots {
        workspace: linked,
        artifact_root: art.clone(),
    })
    .expect_err("only a `.git` directory makes a clone root");
    assert!(format!("{err}").contains("Git worktree"), "got {err}");

    // The root of its own clone is admitted with its files.
    let clone = t.path().join("clone");
    std::fs::create_dir_all(clone.join(".git")).unwrap();
    std::fs::write(clone.join("a.txt"), b"x").unwrap();
    check_workspace_available(&Roots {
        workspace: clone,
        artifact_root: art,
    })
    .expect("a clone root is admitted non-empty");
}

#[test]
fn a_non_git_workspace_still_walks() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("main.py"), b"print(1)").unwrap();
    let m = seal_workspace(&ws, "e1", "2026-10-15T00:00:00Z", BUDGET).unwrap();
    assert_eq!(m.git, None);
    assert_eq!(m.home, None);
    assert_eq!(m.files.len(), 1);
    assert_eq!(m.files[0].path, "main.py");
    assert_eq!(m.files[0].sha256, sha256_hex(b"print(1)"));

    // A clone seals its Git state and walks nothing.
    let t = tempfile::tempdir().unwrap();
    let (ws, _home) = committed_clone(t.path());
    let m = seal_workspace(&ws, "e1", "2026-10-15T00:00:00Z", BUDGET).unwrap();
    assert!(matches!(m.git, Some(GitState::Read(_))), "{:?}", m.git);
    assert!(m.files.is_empty(), "{:?}", m.files);
}

/// The reason an unreadable clone's manifest records, which must not name where it is.
fn unreadable(m: &goalcup_protocol::seal::SealManifest, ws: &Path) -> String {
    assert!(
        m.files.is_empty(),
        "an unreadable clone is not walked: {:?}",
        m.files
    );
    let Some(GitState::Unreadable { unreadable }) = &m.git else {
        panic!("not marked unreadable: {:?}", m.git);
    };
    assert!(!unreadable.is_empty());
    assert!(!unreadable.contains(&*ws.to_string_lossy()), "{unreadable}");
    unreadable.clone()
}

/// D56 amended: a clone whose Git state cannot be read still seals, never walked, with the reason
/// in place of the state; the verifier marks it incomplete. A broken `.git` is one way.
#[test]
fn a_broken_clone_seals_unreadable_never_walked() {
    let t = tempfile::tempdir().unwrap();
    let (ws, _home) = committed_clone(t.path());
    std::fs::write(ws.join(".git/HEAD"), b"not a ref\n").unwrap();
    let m = seal_workspace(&ws, "e1", "2026-10-15T00:00:00Z", BUDGET).unwrap();
    unreadable(&m, &ws);
}

/// Running out of time is another: a clone Git is given no time to read seals unreadable.
#[test]
fn a_clone_out_of_time_seals_unreadable() {
    let t = tempfile::tempdir().unwrap();
    let (ws, _home) = committed_clone(t.path());
    let m = seal_workspace(&ws, "e1", "2026-10-15T00:00:00Z", std::time::Duration::ZERO).unwrap();
    assert_eq!(unreadable(&m, &ws), "the time to read it ran out");
}

/// A tracked file the seal may not look at is `unreadable` in the digest, not `deleted`.
#[test]
fn a_tracked_file_behind_a_closed_directory_is_unreadable_not_deleted() {
    use goalcup_daemon::workspace_git::state;
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let (ws, home) = committed_clone(t.path());
    std::fs::create_dir_all(ws.join("d")).unwrap();
    std::fs::write(ws.join("d/f"), b"x\n").unwrap();
    git(&ws, &home, &["add", "d/f"]);
    git(&ws, &home, &["commit", "-q", "-m", "d"]);
    std::fs::set_permissions(ws.join("d"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let got = state(&ws, BUDGET);
    std::fs::set_permissions(ws.join("d"), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        got.unwrap().dirty_sha256,
        sha256_hex(b"diff\0d/f\0unreadable\nuntracked\0")
    );
}

/// Review C1: Git before 2.44 ignores `GIT_NO_LAZY_FETCH`. A promisor remote the agent writes,
/// with an `ext::` URL it allows, and one tree object it deletes would make `ls-tree -r HEAD`
/// run the agent's command. No transport is allowed, so it never runs.
#[test]
fn an_agent_written_promisor_remote_is_never_fetched() {
    let t = tempfile::tempdir().unwrap();
    let (ws, home) = committed_clone(t.path());
    std::fs::create_dir_all(ws.join("sub")).unwrap();
    std::fs::write(ws.join("sub/f"), b"x\n").unwrap();
    git(&ws, &home, &["add", "sub/f"]);
    git(&ws, &home, &["commit", "-q", "-m", "sub"]);
    let out = goalcup_daemon::workspace_git::command(&ws, &home)
        .expect("git on PATH")
        .args(["rev-parse", "HEAD:sub"])
        .output()
        .unwrap();
    let tree = String::from_utf8(out.stdout).unwrap().trim().to_string();
    std::fs::remove_file(ws.join(".git/objects").join(&tree[..2]).join(&tree[2..])).unwrap();
    let marker = t.path().join("fetched");
    let mut config = std::fs::OpenOptions::new()
        .append(true)
        .open(ws.join(".git/config"))
        .unwrap();
    use std::io::Write;
    writeln!(
        config,
        "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tpartialClone = origin\n\
         [remote \"origin\"]\n\turl = ext::sh -c touch% {}\n\tpromisor = true\n\
         [protocol \"ext\"]\n\tallow = always",
        marker.display()
    )
    .unwrap();
    drop(config);

    // The transport guard alone, as on a Git that ignores `GIT_NO_LAZY_FETCH`.
    let _ = goalcup_daemon::workspace_git::command(&ws, &home)
        .expect("git on PATH")
        .env_remove("GIT_NO_LAZY_FETCH")
        .args(["ls-tree", "-r", "HEAD"])
        .output()
        .unwrap();
    assert!(
        !marker.exists(),
        "the agent's promisor remote ran a command"
    );
    assert!(goalcup_daemon::workspace_git::state(&ws, BUDGET).is_err());
    assert!(
        !marker.exists(),
        "the seal fetched from the agent's promisor remote"
    );
}

/// Review I1: a nested repository `ls-files` collapses to `nested/` is hashed file by file.
#[test]
fn a_nested_repository_is_hashed_file_by_file() {
    use goalcup_daemon::workspace_git::state;
    let t = tempfile::tempdir().unwrap();
    let (ws, home) = committed_clone(t.path());
    let nested = ws.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    git(&nested, &home, &["init", "-q"]);
    std::fs::write(nested.join("work.rs"), b"one").unwrap();
    let before = state(&ws, BUDGET).unwrap().dirty_sha256;
    std::fs::write(nested.join("work.rs"), b"two").unwrap();
    let after = state(&ws, BUDGET).unwrap().dirty_sha256;
    assert_ne!(
        before, after,
        "work inside a nested repository moves the digest"
    );
}

/// Review I1: a gitlink staged in the index, with no commit of it, is recorded by its id.
#[test]
fn an_index_only_gitlink_is_recorded() {
    use goalcup_daemon::workspace_git::state;
    let t = tempfile::tempdir().unwrap();
    let (ws, home) = committed_clone(t.path());
    let clean = state(&ws, BUDGET).unwrap().dirty_sha256;
    let id = "1".repeat(40);
    git(
        &ws,
        &home,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{id},vendored"),
        ],
    );
    let mut bytes = b"diff\0vendored\0gitlink:".to_vec();
    bytes.extend_from_slice(id.as_bytes());
    bytes.extend_from_slice(b"\nuntracked\0");
    let got = state(&ws, BUDGET).unwrap().dirty_sha256;
    assert_ne!(got, clean);
    assert_eq!(got, sha256_hex(&bytes));
}

/// Review I1: only `.gitignore` files hide an untracked file; `.git/info/exclude`, which no
/// digest covers, does not.
#[test]
fn an_agent_written_exclude_file_hides_nothing() {
    use goalcup_daemon::workspace_git::state;
    let t = tempfile::tempdir().unwrap();
    let (ws, _home) = committed_clone(t.path());
    let clean = state(&ws, BUDGET).unwrap().dirty_sha256;
    std::fs::write(ws.join(".git/info/exclude"), b"hidden.txt\n").unwrap();
    std::fs::write(ws.join("hidden.txt"), b"work").unwrap();
    assert_ne!(state(&ws, BUDGET).unwrap().dirty_sha256, clean);
}
