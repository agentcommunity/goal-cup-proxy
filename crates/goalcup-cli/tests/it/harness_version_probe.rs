//! D53: the harness build is recorded, not pinned. Any regular file resolves; the release
//! refuses one swapped since it was resolved; and the version the harness reports for its
//! `version_args` is kept, or `unreported`.

use goalcup_cli::harness_version::{
    MAX_REPORTED_VERSION_BYTES, UNREPORTED, reported_line, reported_version,
    reported_version_in_scratch_home, reported_version_within,
};
use goalcup_cli::launch::{recheck_resolved_executable, resolve_harness_executable};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A script `name` in `dir` with `body` after its shebang, executable, at its canonical path.
fn stub(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::canonicalize(path).unwrap()
}

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|a| (*a).to_string()).collect()
}

/// A temporary directory at its canonical path, with a `home` inside it.
fn scratch() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let home = root.join("home");
    std::fs::create_dir(&home).unwrap();
    (tmp, root, home)
}

#[test]
fn the_version_a_stub_prints_is_reported() {
    let (_tmp, root, home) = scratch();
    let exe = stub(
        &root,
        "toy",
        r#"[ "$1" = "--version" ] || exit 3
printf '  toy 1.2.3  \nsecond line\n'
/usr/bin/env > "$HOME/probe-env""#,
    );
    let resolved = resolve_harness_executable(&exe).expect("any regular file resolves");
    assert_eq!(
        reported_version(&resolved, &args(&["--version"]), &home),
        "toy 1.2.3"
    );
    // The probe's environment is built from nothing: `PATH` and the run's private `HOME`, and
    // what the shell adds itself. Nothing from this test process reaches it.
    let env = std::fs::read_to_string(home.join("probe-env")).unwrap();
    let mut names: Vec<&str> = env
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        .filter(|name| !matches!(*name, "PWD" | "OLDPWD" | "SHLVL" | "_"))
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["HOME", "PATH"], "{env}");
    assert!(env.contains(&format!("HOME={}\n", home.display())), "{env}");
    assert!(
        env.contains(&format!("PATH={}\n", goalcup_cli::launch::CAPTURE_PATH)),
        "{env}"
    );
}

/// The launch probes in a throwaway home beside the run's: whatever the probe writes there (Codex
/// creates `~/.codex/tmp/arg0`) is removed with it, and the run's own `HOME` never sees it.
#[test]
fn the_launch_probe_writes_only_into_a_throwaway_home() {
    let (_tmp, root, run_home) = scratch();
    let exe = stub(
        &root,
        "toy",
        r#"mkdir -p "$HOME/.toy/tmp" && echo state > "$HOME/.toy/tmp/lock"; echo "toy 1.2.3""#,
    );
    let resolved = resolve_harness_executable(&exe).unwrap();
    let probe_home = root.join("version-probe-home");
    assert_eq!(
        reported_version_in_scratch_home(&resolved, &args(&["--version"]), &probe_home),
        "toy 1.2.3"
    );
    assert!(!probe_home.exists(), "the throwaway home is removed");
    assert_eq!(std::fs::read_dir(&run_home).unwrap().count(), 0);
}

#[test]
fn without_version_args_the_harness_is_not_asked() {
    let (_tmp, root, home) = scratch();
    let exe = stub(&root, "toy", r#"echo ran > "$HOME/ran"; echo "toy 1.2.3""#);
    let resolved = resolve_harness_executable(&exe).unwrap();
    assert_eq!(reported_version(&resolved, &[], &home), UNREPORTED);
    assert!(!home.join("ran").exists(), "nothing was started");
}

#[test]
fn a_probe_that_outlives_its_timeout_is_killed_and_unreported() {
    let (_tmp, root, home) = scratch();
    let exe = stub(
        &root,
        "toy",
        r#"echo $$ > "$HOME/pid"
echo "toy 1.2.3"
exec /bin/sleep 30"#,
    );
    let started = Instant::now();
    let version =
        reported_version_within(&exe, &args(&["--version"]), &home, Duration::from_secs(2));
    assert_eq!(version, UNREPORTED);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let pid: i32 = std::fs::read_to_string(home.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: signal 0 only asks whether the pid exists.
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    assert!(!alive, "the timed-out probe was killed and reaped");
}

#[test]
fn a_failed_or_unusable_probe_is_unreported() {
    let (_tmp, root, home) = scratch();
    for (name, body) in [
        ("fails", "echo 'toy 1.2.3'; exit 1"),
        ("silent", "exit 0"),
        ("blank", "echo; echo 'toy 1.2.3'"),
        ("control", "printf 'toy\\0011.2.3\\n'"),
    ] {
        let exe = stub(&root, name, body);
        assert_eq!(
            reported_version_within(&exe, &args(&["--version"]), &home, Duration::from_secs(5)),
            UNREPORTED,
            "{name}"
        );
    }
    let missing = root.join("not-there");
    assert_eq!(
        reported_version_within(
            &missing,
            &args(&["--version"]),
            &home,
            Duration::from_secs(5)
        ),
        UNREPORTED
    );
}

#[test]
fn only_a_bounded_printable_first_line_is_kept() {
    assert_eq!(reported_line(b"codex-cli 0.153.0\n"), "codex-cli 0.153.0");
    assert_eq!(
        reported_line(b"\t2.1.283 (Claude Code)\r\nmore"),
        "2.1.283 (Claude Code)"
    );
    assert_eq!(reported_line(b""), UNREPORTED);
    assert_eq!(reported_line(b"   \n1.0"), UNREPORTED);
    assert_eq!(reported_line(b"caf\xc3\xa9 1.0\n"), UNREPORTED);
    assert_eq!(reported_line(b"\xff\xfe\n"), UNREPORTED);
    assert_eq!(reported_line(b"v1\x1b[0m\n"), UNREPORTED);
    let longest = "9".repeat(MAX_REPORTED_VERSION_BYTES);
    assert_eq!(reported_line(longest.as_bytes()), longest);
    let longer = "9".repeat(MAX_REPORTED_VERSION_BYTES + 1);
    assert_eq!(reported_line(longer.as_bytes()), UNREPORTED);
}

#[test]
fn the_release_refuses_a_file_swapped_since_it_was_resolved() {
    let (_tmp, root, _home) = scratch();
    let exe = stub(&root, "toy", "echo 'toy 1.2.3'");
    let resolved = resolve_harness_executable(&exe).unwrap();
    recheck_resolved_executable(&resolved).expect("the same bytes pass");

    // Another build put in its place, atomically, as an updater would.
    let other = stub(&root, "toy-next", "echo 'toy 1.2.4'");
    std::fs::rename(&other, &exe).unwrap();
    let refusal = recheck_resolved_executable(&resolved)
        .expect_err("a swap mid-launch is refused")
        .to_string();
    assert!(
        refusal.contains("was replaced after it was resolved")
            && refusal.contains(resolved.sha256()),
        "{refusal}"
    );
    // Resolved again, the new build is as acceptable as the old one was.
    let again = resolve_harness_executable(&exe).unwrap();
    assert_ne!(again.sha256(), resolved.sha256());
    recheck_resolved_executable(&again).unwrap();

    // A symlink is refused, not followed.
    let link = root.join("toy-link");
    std::os::unix::fs::symlink(&exe, &link).unwrap();
    let refusal = resolve_harness_executable(&link)
        .expect_err("a link")
        .to_string();
    assert!(refusal.contains("is a symlink"), "{refusal}");
}
