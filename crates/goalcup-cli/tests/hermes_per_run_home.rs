//! The per-run private Hermes home.
//!
//! The home is made inside the run directory, from the connector's own seed files, and nothing
//! outside the run is read or written through it. Every directory here is a private temporary
//! one: nothing resolves a real `HOME`, and no test reads or writes any `.hermes`.
#![cfg(unix)]

use goalcup_cli::CliError;
use goalcup_cli::per_run_home::{
    create_per_run_home, icloud_folder_synced, icloud_folder_synced_with,
    refuse_inside_git_work_tree, refuse_synced_folder, refuse_synced_folder_probing,
    refuse_synced_folder_under,
};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The corrected D41 seed, spelled out here so a change to the shipped file is a change to
/// this test as well.
const AUTH_SEED: &str = r#"{"version": 1, "providers": {}, "suppressed_sources": {"deepseek": ["env:DEEPSEEK_API_KEY"]}}"#;

fn connector_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors/hermes-cli")
}

fn loaded_hermes() -> goalcup_protocol::harness::LoadedConnector {
    goalcup_protocol::harness::load_dir(&connector_dir(), goalcup_protocol::harness::Origin::Pinned)
        .unwrap()
}

fn create_home(run: &Path) -> Result<PathBuf, CliError> {
    let loaded = loaded_hermes();
    create_per_run_home(run, &loaded.manifest.capture.home, &loaded.seeds)
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

fn run_dir() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let run = std::fs::canonicalize(tmp.path())
        .unwrap()
        .join("goalcup-run");
    std::fs::create_dir(&run).unwrap();
    (tmp, run)
}

fn loaded_temp_home_connector(
    dirs: &str,
    seeds: &str,
    files: &[(&str, &[u8])],
) -> (
    tempfile::TempDir,
    goalcup_protocol::harness::LoadedConnector,
) {
    let tmp = tempfile::tempdir().unwrap();
    let connector = tmp.path().join("per-run-home-test");
    std::fs::create_dir_all(connector.join("files")).unwrap();
    let original = std::fs::read_to_string(connector_dir().join("harness.toml")).unwrap();
    let old_home = r#"dirs = ["home"]
real_dirs = ["{user_home}/.hermes"]
seed = [
  { file = "config.yaml" },
  { file = "auth.json" },
  { file = ".update_check" },
]"#;
    let new_home =
        format!("dirs = {dirs}\nreal_dirs = [\"{{user_home}}/.hermes\"]\nseed = {seeds}");
    let manifest = original
        .replace("id = \"hermes-cli\"", "id = \"per-run-home-test\"")
        .replace(old_home, &new_home);
    assert_ne!(manifest, original, "the schema-1 fixture was not rewritten");
    std::fs::write(connector.join("harness.toml"), manifest).unwrap();
    for (name, bytes) in files {
        let path = connector.join("files").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let loaded =
        goalcup_protocol::harness::load_dir(&connector, goalcup_protocol::harness::Origin::Pinned)
            .unwrap();
    (tmp, loaded)
}

#[test]
fn declared_child_before_parent_is_created_in_parent_first_order() {
    let (_connector, loaded) = loaded_temp_home_connector(
        r#"["declared/leaf", "declared"]"#,
        r#"[{ file = "payload.bin" }]"#,
        &[("payload.bin", b"declared tree")],
    );
    let (_tmp, run) = run_dir();
    let home = create_per_run_home(&run, &loaded.manifest.capture.home, &loaded.seeds).unwrap();
    assert_eq!(mode(&home.join("declared")), 0o700);
    assert_eq!(mode(&home.join("declared/leaf")), 0o700);
    assert_eq!(
        std::fs::read(home.join("payload.bin")).unwrap(),
        b"declared tree"
    );
}

#[test]
fn duplicate_declared_directories_are_idempotent() {
    let (_connector, loaded) = loaded_temp_home_connector(
        r#"["cache", "cache"]"#,
        r#"[{ file = "payload.bin" }]"#,
        &[("payload.bin", b"one directory")],
    );
    let (_tmp, run) = run_dir();
    let home = create_per_run_home(&run, &loaded.manifest.capture.home, &loaded.seeds).unwrap();
    assert_eq!(mode(&home.join("cache")), 0o700);
    assert_eq!(std::fs::read_dir(home.join("cache")).unwrap().count(), 0);
}

#[test]
fn case_alias_prefixes_created_by_this_plan_work_on_case_insensitive_filesystems() {
    let (_connector, loaded) = loaded_temp_home_connector(
        r#"["Case", "case/child"]"#,
        r#"[{ file = "payload.bin" }]"#,
        &[("payload.bin", b"case alias")],
    );
    let (_tmp, run) = run_dir();
    let home = create_per_run_home(&run, &loaded.manifest.capture.home, &loaded.seeds).unwrap();
    assert_eq!(mode(&home.join("Case")), 0o700);
    assert_eq!(mode(&home.join("case/child")), 0o700);
    assert_eq!(
        std::fs::read(home.join("payload.bin")).unwrap(),
        b"case alias"
    );
}

#[test]
fn nested_seed_parents_are_synthesized_once_with_exact_modes_and_bytes() {
    let (_connector, loaded) = loaded_temp_home_connector(
        "[]",
        r#"[
  { file = "implicit/shared/readonly.bin", mode = "0400" },
  { file = "implicit/shared/deeper/writable.bin", mode = "0600" },
]"#,
        &[
            ("implicit/shared/readonly.bin", b"read only"),
            ("implicit/shared/deeper/writable.bin", b"read write"),
        ],
    );
    let (_tmp, run) = run_dir();
    let home = create_per_run_home(&run, &loaded.manifest.capture.home, &loaded.seeds).unwrap();
    for dir in ["implicit", "implicit/shared", "implicit/shared/deeper"] {
        assert_eq!(mode(&home.join(dir)), 0o700, "{dir}");
    }
    assert_eq!(mode(&home.join("implicit/shared/readonly.bin")), 0o400);
    assert_eq!(
        mode(&home.join("implicit/shared/deeper/writable.bin")),
        0o600
    );
    assert_eq!(
        std::fs::read(home.join("implicit/shared/readonly.bin")).unwrap(),
        b"read only"
    );
    assert_eq!(
        std::fs::read(home.join("implicit/shared/deeper/writable.bin")).unwrap(),
        b"read write"
    );
}

/// The tree, the modes and the bytes.
#[test]
fn the_home_is_a_private_tree_of_exactly_the_connectors_seed_files() {
    let (_tmp, run) = run_dir();
    let home = create_home(&run).expect("a per-run home");
    assert_eq!(home, run.join("hermes"));
    assert_eq!(mode(&home), 0o700);
    assert_eq!(
        mode(&home.join("home")),
        0o700,
        "TERMINAL_HOME_MODE=profile's home"
    );
    assert!(home.join("home").is_dir());
    for name in ["config.yaml", "auth.json", ".update_check"] {
        let written = home.join(name);
        assert_eq!(mode(&written), 0o600, "{name}");
        assert_eq!(
            std::fs::read(&written).unwrap(),
            std::fs::read(connector_dir().join("files").join(name)).unwrap(),
            "{name}: byte for byte the connector's file"
        );
    }
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        format!("{AUTH_SEED}\n"),
        "the corrected D41 seed: only `suppressed_sources` stops the pool seeding from the env"
    );
    // Exactly these entries, and nothing else: no sessions directory (the daemon makes that one),
    // no .env, no state.
    let mut names: Vec<String> = std::fs::read_dir(&home)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, [".update_check", "auth.json", "config.yaml", "home"]);
    assert_eq!(std::fs::read_dir(home.join("home")).unwrap().count(), 0);
}

/// The two shared-home launch profiles carry none of the per-run layout that Hermes declares.
/// This keeps the legacy CLI anchor's cross-connector assertion beside the behavior it protects.
#[test]
fn shared_homes_declare_no_per_run_layout_or_seeds() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    for id in ["codex-cli", "claude-code-cli"] {
        let loaded = goalcup_protocol::harness::load_dir(
            &root.join(id),
            goalcup_protocol::harness::Origin::Pinned,
        )
        .unwrap_or_else(|error| panic!("{id}: {error}"));
        let home = &loaded.manifest.capture.home;
        assert!(home.per_run_dir.is_none(), "{id}");
        assert!(home.dirs.is_empty(), "{id}");
        assert!(home.seed.is_empty(), "{id}");
    }
}

/// An existing `hermes` entry of any kind is refused and left exactly as it was: the home must
/// be new, so nothing already in it can be read as this run's.
#[test]
fn an_existing_home_is_refused_and_left_alone() {
    let (_tmp, run) = run_dir();
    let existing = run.join("hermes");
    std::fs::create_dir(&existing).unwrap();
    std::fs::write(existing.join("sentinel"), b"mine").unwrap();
    let err = create_home(&run).expect_err("it already exists");
    assert!(
        matches!(err, CliError::Io { .. } | CliError::Refused(_)),
        "{err:?}"
    );
    assert_eq!(std::fs::read(existing.join("sentinel")).unwrap(), b"mine");
    assert!(!existing.join("config.yaml").exists());
    assert!(!existing.join("auth.json").exists());

    // A symlink there is refused too, and what it names is not written through.
    let (_tmp2, run2) = run_dir();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), run2.join("hermes")).unwrap();
    assert!(create_home(&run2).is_err());
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

/// The home writer uses the bytes captured by the protocol loader, never a second connector read.
#[test]
fn connector_mutation_after_load_cannot_change_written_seed_bytes() {
    let connector = tempfile::tempdir().unwrap();
    let copied = connector.path().join("hermes-copy");
    std::fs::create_dir_all(copied.join("files")).unwrap();
    std::fs::copy(
        connector_dir().join("harness.toml"),
        copied.join("harness.toml"),
    )
    .unwrap();
    for name in ["config.yaml", "auth.json", ".update_check"] {
        std::fs::copy(
            connector_dir().join("files").join(name),
            copied.join("files").join(name),
        )
        .unwrap();
    }
    let mut text = std::fs::read_to_string(copied.join("harness.toml")).unwrap();
    text = text.replace(
        "{ file = \".update_check\" }",
        "{ file = \".update_check\", mode = \"0400\" }",
    );
    std::fs::write(copied.join("harness.toml"), text).unwrap();
    let loaded =
        goalcup_protocol::harness::load_dir(&copied, goalcup_protocol::harness::Origin::Pinned)
            .unwrap();
    let captured = loaded
        .seeds
        .iter()
        .find(|seed| seed.file == "config.yaml")
        .unwrap()
        .bytes
        .clone();
    std::fs::write(copied.join("files/config.yaml"), b"mutated after load\n").unwrap();

    let (_tmp, run) = run_dir();
    let home = create_per_run_home(&run, &loaded.manifest.capture.home, &loaded.seeds).unwrap();
    assert_eq!(std::fs::read(home.join("config.yaml")).unwrap(), captured);
    assert_eq!(mode(&home.join("config.yaml")), 0o600);
    assert_eq!(mode(&home.join(".update_check")), 0o400);
}

/// The written configuration holds the settings the recipe depends on, and none that
/// names a provider, host, key or base URL.
#[test]
fn the_written_config_has_snapshots_no_thinking_and_no_compression() {
    let (_tmp, run) = run_dir();
    let home = create_home(&run).unwrap();
    let config = std::fs::read_to_string(home.join("config.yaml")).unwrap();
    let lines: Vec<&str> = config
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    for wanted in [
        "write_json_snapshots: true",
        r#"reasoning_effort: "none""#,
        "compression:",
        "title_generation:",
        // No scanner binary is fetched from GitHub and run with the key in
        // its environment, and no remote model catalog is read.
        "tirith_enabled: false",
        "model_catalog:",
    ] {
        assert!(lines.contains(&wanted), "missing `{wanted}`:\n{config}");
    }
    // The `enabled: false` lines: title generation, compression and the model catalog.
    assert_eq!(
        lines.iter().filter(|l| **l == "enabled: false").count(),
        3,
        "{config}"
    );
    for forbidden in ["base_url", "provider", "api_key", "fallback", "http"] {
        assert!(
            !lines.iter().any(|l| l.contains(forbidden)),
            "the config names `{forbidden}`:\n{config}"
        );
    }
}

/// The update check is answered from a seed that is always fresh, so Hermes
/// never asks PyPI. The seed is a static, pinnable file: a timestamp far in the future, the pinned
/// version, and no revision.
#[test]
fn the_update_check_seed_is_a_static_always_fresh_cache_of_the_pinned_version() {
    let (_tmp, run) = run_dir();
    let home = create_home(&run).unwrap();
    let seed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join(".update_check")).unwrap()).unwrap();
    assert_eq!(seed["ver"], "0.19.0");
    assert_eq!(seed["behind"], 0);
    assert!(seed["rev"].is_null());
    // Fresh for as long as `now - ts < 6 h` can hold: far past any clock this tree will see.
    assert!(seed["ts"].as_u64().unwrap() > 4_000_000_000);
}

/// A goal directory inside a Git work tree is refused: a run directory made beside the
/// goal would hold request dumps and session files, and a repository is where they get committed.
#[test]
fn a_goal_directory_inside_a_git_work_tree_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let plain = root.join("plain/deeper");
    std::fs::create_dir_all(&plain).unwrap();
    refuse_inside_git_work_tree(&plain).expect("no repository above it");

    // A `.git` directory above, at any depth.
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let inside = repo.join("a/b/c");
    std::fs::create_dir_all(&inside).unwrap();
    for dir in [&repo, &repo.join("a"), &inside] {
        let err = refuse_inside_git_work_tree(dir).expect_err("inside a work tree");
        assert!(matches!(err, CliError::Refused(_)), "{err:?}");
        assert_eq!(err.exit_code(), 1);
        assert!(err.to_string().contains("Git work tree"), "{err}");
        assert!(err.to_string().contains("Nothing was created"), "{err}");
    }

    // A linked work tree has a `.git` FILE, which is as much a work tree.
    let linked = root.join("linked");
    std::fs::create_dir_all(linked.join("sub")).unwrap();
    std::fs::write(linked.join(".git"), "gitdir: /elsewhere/.git/worktrees/x\n").unwrap();
    assert!(refuse_inside_git_work_tree(&linked.join("sub")).is_err());

    // A sibling of the repository is not inside it.
    refuse_inside_git_work_tree(&root.join("plain")).expect("a sibling");
}

/// "Stays local" is not only a Git matter. A goal file under a folder a client
/// syncs off the machine would have `request_dump_*.json` uploaded, so it is refused too.
#[test]
fn a_goal_directory_in_a_synced_folder_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let plain = root.join("work/goals");
    std::fs::create_dir_all(&plain).unwrap();
    refuse_synced_folder(&plain).expect("an ordinary directory");
    for synced in [
        "Library/Mobile Documents/com~apple~CloudDocs/goals",
        "Library/CloudStorage/Dropbox/goals",
        "Library/CloudStorage/GoogleDrive-me@example.com/My Drive/goals",
        "Dropbox/goals",
        "Dropbox (Personal)/goals",
        "OneDrive/goals",
        "OneDrive - Contoso/goals",
        "iCloud Drive/goals",
        "Google Drive/goals",
    ] {
        let dir = root.join("home").join(synced);
        std::fs::create_dir_all(&dir).unwrap();
        let err = refuse_synced_folder(&dir).expect_err(synced);
        assert!(matches!(err, CliError::Refused(_)), "{synced}: {err:?}");
        assert_eq!(err.exit_code(), 1);
        assert!(err.to_string().contains("synced"), "{synced}: {err}");
        assert!(err.to_string().contains("Nothing was created"), "{err}");
    }
    // A directory that merely has a similar name is not one.
    let near = root.join("home/Dropboxes-notes/goals");
    std::fs::create_dir_all(&near).unwrap();
    refuse_synced_folder(&near).expect("a different name");
}

/// The attribute File Provider uses cannot be set on a fake folder (macOS refuses the
/// `com.apple.` name), so these tests mark folders with one that can be, and the mechanism is the
/// same: ask the folder for the attribute, by name.
#[cfg(target_os = "macos")]
const TEST_ATTRIBUTE: &str = "dev.goalcup.test-file-provider";

#[cfg(target_os = "macos")]
fn mark(dir: &Path) {
    let status = std::process::Command::new("/usr/bin/xattr")
        .args(["-w", TEST_ATTRIBUTE, "x"])
        .arg(dir)
        .status()
        .unwrap();
    assert!(status.success());
}

/// H-18b: iCloud "Desktop & Documents" makes `~/Desktop` and `~/Documents` synced folders without
/// any name saying so. The local signal is the File Provider attribute on the folder (or the
/// folder being the very directory iCloud Drive holds under that name). With the signal, a goal
/// file there is refused with the sync-folder refusal; without it, never, and a leftover stub
/// folder in iCloud Drive is not a signal. Fake HOME, no network.
#[cfg(target_os = "macos")]
#[test]
fn icloud_desktop_and_documents_are_refused_only_when_the_signal_is_present() {
    let tmp = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(tmp.path()).unwrap().join("home");
    let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    for dir in [
        "Desktop/goals",
        "Documents/work/goals",
        "Downloads/goals",
        "Library/Mobile Documents/com~apple~CloudDocs/Desktop",
        "Library/Mobile Documents/com~apple~CloudDocs/Documents",
    ] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    let desktop = home.join("Desktop/goals");
    let documents = home.join("Documents/work/goals");
    let downloads = home.join("Downloads/goals");
    assert!(icloud.join("Desktop").is_dir());
    let probe = |h: &Path, name: &str| icloud_folder_synced_with(h, name, TEST_ATTRIBUTE);

    // No signal: an iCloud Drive folder of the same name that is not these folders (a stub
    // left behind, or a Documents folder an app made) refuses nothing.
    for dir in [&desktop, &documents, &downloads] {
        refuse_synced_folder_probing(dir, Some(&home), &probe).expect("no signal, no refusal");
        refuse_synced_folder_under(dir, Some(&home)).expect("the real signal is absent too");
    }
    assert!(!icloud_folder_synced(&home, "Desktop"));
    assert!(!icloud_folder_synced(&home, "Documents"));

    // Desktop is marked: refused, wherever below it, and Documents is not.
    mark(&home.join("Desktop"));
    assert!(probe(&home, "Desktop") && !probe(&home, "Documents"));
    for dir in [&home.join("Desktop"), &desktop] {
        let err =
            refuse_synced_folder_probing(dir, Some(&home), &probe).expect_err("Desktop is synced");
        assert!(matches!(err, CliError::Refused(_)), "{err:?}");
        assert_eq!(err.exit_code(), 1);
        assert!(err.to_string().contains("sync client uploads"), "{err}");
        assert!(err.to_string().contains("Nothing was created"), "{err}");
    }
    refuse_synced_folder_probing(&documents, Some(&home), &probe).expect("Documents has no signal");
    refuse_synced_folder_probing(&downloads, Some(&home), &probe)
        .expect("Downloads is never judged");

    mark(&home.join("Documents"));
    assert!(refuse_synced_folder_probing(&documents, Some(&home), &probe).is_err());
    refuse_synced_folder_probing(&downloads, Some(&home), &probe)
        .expect("Downloads is never judged");

    // A signal with no HOME to anchor it is not a signal, and an empty HOME is no HOME.
    refuse_synced_folder_probing(&desktop, None, &probe).expect("no HOME, no iCloud judgement");
    refuse_synced_folder_probing(&desktop, Some(Path::new("")), &probe).expect("empty HOME");
    // By name only, the old entry point has no HOME and judges no iCloud folder.
    refuse_synced_folder(&desktop).expect("by name only");
}

/// The other half of the signal: the folder is the same directory iCloud Drive holds under the
/// same name (how Desktop & Documents were mirrored before File Provider). Two paths to one
/// directory are made with a symlink, which is as much one inode as a firm link is.
#[cfg(unix)]
#[test]
fn a_folder_that_is_the_icloud_drive_folder_is_synced() {
    let tmp = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(tmp.path()).unwrap().join("home");
    let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    std::fs::create_dir_all(icloud.join("Documents")).unwrap();
    std::fs::create_dir_all(home.join("Desktop")).unwrap();
    assert!(!icloud_folder_synced(&home, "Documents"), "folder absent");
    std::os::unix::fs::symlink(icloud.join("Documents"), home.join("Documents")).unwrap();
    assert!(icloud_folder_synced(&home, "Documents"));
    assert!(
        !icloud_folder_synced(&home, "Desktop"),
        "a different directory"
    );
}
