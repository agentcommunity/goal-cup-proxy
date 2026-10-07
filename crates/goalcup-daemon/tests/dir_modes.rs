//! Unix permission contracts for the daemon's two configured roots.
//!
//! These are startup behaviours, not generic directory policy: the per-run home still uses
//! the runtime's ordinary directory helper.  The subprocess cases exercise the participant's
//! stderr contract and set a restrictive umask only in the forked child, never process-wide in
//! this test runner.

#![cfg(unix)]

mod support;

use std::io::{BufRead as _, Read as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const PRIVATE_ROOT_MODE: u32 = 0o700;
const PUBLIC_ROOT_MODE: u32 = 0o755;
const WARNING_MARKER: &str = "permission warning:";

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("{} has no metadata: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

#[tokio::test]
async fn fresh_artifact_and_config_roots_are_0700() {
    let temp = tempfile::tempdir().expect("a disposable daemon prefix");
    let config = support::config_at(temp.path(), "http://127.0.0.1:1");
    assert!(!config.artifact_root.exists());
    assert!(!config.config_dir.exists());

    let daemon = goalcup_daemon::runtime::Daemon::start(config.clone())
        .await
        .expect("the daemon starts over absent configured roots");

    assert_eq!(mode(&config.artifact_root), PRIVATE_ROOT_MODE);
    assert_eq!(mode(&config.config_dir), PRIVATE_ROOT_MODE);
    daemon.stop().await;
}

#[test]
fn a_fresh_config_root_is_normalized_after_a_restrictive_umask() {
    let temp = tempfile::tempdir().expect("a disposable daemon prefix");
    let artifact_root = temp.path().join("holder");
    let config_root = temp.path().join("config");
    make_private(&artifact_root);
    let config_path = write_nonplaceholder_config(temp.path());

    // 0100 strips owner execute from mkdir's requested 0700 while leaving enough access to
    // retain a no-follow directory descriptor.  The descriptor fchmod must restore exact 0700.
    let (status, stderr) = run_foreground(&config_path, Some(0o100));

    assert_eq!(status.code(), Some(0), "stderr:\n{stderr}");
    assert_eq!(
        mode(&artifact_root),
        PRIVATE_ROOT_MODE,
        "existing other root"
    );
    assert_eq!(mode(&config_root), PRIVATE_ROOT_MODE, "fresh config root");
}

#[test]
fn a_fresh_artifact_root_is_normalized_after_a_restrictive_umask() {
    let temp = tempfile::tempdir().expect("a disposable daemon prefix");
    let artifact_root = temp.path().join("holder");
    let config_root = temp.path().join("config");
    make_private(&config_root);
    let config_path = write_nonplaceholder_config(temp.path());

    // Keep the artifact role independent: skipping only its post-create normalization must
    // leave 0600 here and fail this assertion even if config_dir's implementation is correct.
    let (status, stderr) = run_foreground(&config_path, Some(0o100));

    assert_eq!(status.code(), Some(0), "stderr:\n{stderr}");
    assert_eq!(
        mode(&artifact_root),
        PRIVATE_ROOT_MODE,
        "fresh artifact root"
    );
    assert_eq!(mode(&config_root), PRIVATE_ROOT_MODE, "existing other root");
}

#[test]
fn an_owner_denying_umask_fails_safely_without_path_normalization() {
    let temp = tempfile::tempdir().expect("a disposable daemon prefix");
    let artifact_root = temp.path().join("holder");
    let config_root = temp.path().join("config");
    let config_path = write_nonplaceholder_config(temp.path());

    // 0777 creates the leaf with mode 000, so macOS cannot acquire even the no-follow
    // directory descriptor needed by fchmod.  Startup must fail instead of falling back to a
    // pathname chmod that could follow a replacement link.
    let (status, stderr) = run_until_exit(&config_path, 0o777);

    assert_eq!(status.code(), Some(1), "stderr:\n{stderr}");
    assert_eq!(
        mode(&artifact_root),
        0,
        "the inaccessible created inode is unchanged"
    );
    assert!(
        !config_root.exists(),
        "startup stopped before the second root"
    );
    assert!(
        stderr.contains(&artifact_root.display().to_string()),
        "the configured path is named: {stderr}"
    );
}

#[tokio::test]
async fn a_created_root_replaced_by_a_symlink_cannot_chmod_its_outside_target() {
    let temp = tempfile::tempdir().expect("a disposable daemon prefix");
    let outside = temp.path().join("outside");
    make_public(&outside);
    let config = support::config_at(temp.path(), "http://127.0.0.1:1");
    goalcup_daemon::runtime::Daemon::replace_new_configured_root_with_symlink_for_test(
        &config.artifact_root,
        &outside,
    );

    let error = match goalcup_daemon::runtime::Daemon::start(config.clone()).await {
        Ok(daemon) => {
            daemon.stop().await;
            panic!("a replaced fresh artifact root must not start")
        }
        Err(error) => error,
    };

    assert!(
        std::fs::symlink_metadata(&config.artifact_root)
            .expect("the replacement link remains")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        mode(&outside),
        PUBLIC_ROOT_MODE,
        "normalization must never follow the replacement link"
    );
    assert!(
        error
            .to_string()
            .contains(&config.artifact_root.display().to_string()),
        "the configured path is named: {error}"
    );
}

#[test]
fn an_existing_public_artifact_root_warns_once_and_still_starts() {
    let temp = tempfile::tempdir().expect("a disposable daemon prefix");
    let config_path = write_nonplaceholder_config(temp.path());
    let artifact_root = temp.path().join("holder");
    let config_root = temp.path().join("config");
    make_public(&artifact_root);

    let (status, stderr) = run_foreground(&config_path, None);

    assert_eq!(status.code(), Some(0), "stderr:\n{stderr}");
    assert_one_warning(&stderr, "artifact_root", &artifact_root, "0755");
    assert_eq!(mode(&artifact_root), PUBLIC_ROOT_MODE, "existing root");
    assert_eq!(mode(&config_root), PRIVATE_ROOT_MODE, "fresh other root");
}

#[test]
fn an_existing_public_config_root_warns_once_and_still_starts() {
    let temp = tempfile::tempdir().expect("a disposable daemon prefix");
    let config_path = write_nonplaceholder_config(temp.path());
    let artifact_root = temp.path().join("holder");
    let config_root = temp.path().join("config");
    make_public(&config_root);

    let (status, stderr) = run_foreground(&config_path, None);

    assert_eq!(status.code(), Some(0), "stderr:\n{stderr}");
    assert_one_warning(&stderr, "config_dir", &config_root, "0755");
    assert_eq!(mode(&config_root), PUBLIC_ROOT_MODE, "existing root");
    assert_eq!(mode(&artifact_root), PRIVATE_ROOT_MODE, "fresh other root");
}

fn make_public(path: &Path) {
    std::fs::create_dir(path).unwrap_or_else(|error| panic!("create {}: {error}", path.display()));
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(PUBLIC_ROOT_MODE))
        .unwrap_or_else(|error| panic!("chmod {}: {error}", path.display()));
}

fn make_private(path: &Path) {
    std::fs::create_dir(path).unwrap_or_else(|error| panic!("create {}: {error}", path.display()));
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(PRIVATE_ROOT_MODE))
        .unwrap_or_else(|error| panic!("chmod {}: {error}", path.display()));
}

fn write_nonplaceholder_config(root: &Path) -> std::path::PathBuf {
    support::write_config_with(root, |text| {
        text.lines()
            .map(|line| {
                if line.starts_with("server_public_key =") {
                    format!("server_public_key = \"{}\"", "ab".repeat(32))
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn assert_one_warning(stderr: &str, role: &str, path: &Path, observed_mode: &str) {
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|line| line.contains(WARNING_MARKER))
        .collect();
    assert_eq!(warnings.len(), 1, "stderr was:\n{stderr}");
    let warning = warnings[0];
    assert!(warning.contains(role), "warning was: {warning}");
    assert!(
        warning.contains(&path.display().to_string()),
        "warning was: {warning}"
    );
    assert!(warning.contains(observed_mode), "warning was: {warning}");
}

/// Start the foreground daemon, wait on its ordinary two-line readiness signal, then stop
/// and reap it within a fixed bound.  The umask is changed after `fork` and before `exec`, so
/// parallel tests in this process never inherit it.
fn run_foreground(config_path: &Path, child_umask: Option<libc::mode_t>) -> (ExitStatus, String) {
    let mut command = foreground_command(config_path, child_umask);
    let mut child = OwnedChild::new(command.spawn().expect("the foreground daemon starts"));
    let stdout = child
        .child
        .stdout
        .take()
        .expect("the daemon stdout is piped");
    let mut stdout = std::io::BufReader::new(stdout);
    let mut readiness = Vec::new();
    for _ in 0..2 {
        let mut line = String::new();
        let bytes = stdout
            .read_line(&mut line)
            .expect("the readiness line reads");
        if bytes == 0 {
            let status = child.wait_bounded(Duration::from_secs(5));
            let stderr = read_stderr(&mut child);
            panic!("daemon exited before readiness with {status:?}; stderr:\n{stderr}");
        }
        readiness.push(line.trim().to_string());
    }
    assert!(readiness[0].starts_with("127.0.0.1:"), "{readiness:?}");
    assert!(readiness[1].ends_with("control.json"), "{readiness:?}");

    child.terminate();
    let status = child.wait_bounded(Duration::from_secs(5));
    let stderr = read_stderr(&mut child);
    (status, stderr)
}

fn run_until_exit(config_path: &Path, child_umask: libc::mode_t) -> (ExitStatus, String) {
    let mut command = foreground_command(config_path, Some(child_umask));
    let mut child = OwnedChild::new(command.spawn().expect("the foreground daemon starts"));
    let status = child.wait_bounded(Duration::from_secs(10));
    let stderr = read_stderr(&mut child);
    (status, stderr)
}

fn foreground_command(config_path: &Path, child_umask: Option<libc::mode_t>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup-daemon"));
    command
        .arg("--config")
        .arg(config_path)
        .arg("--foreground")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(child_umask) = child_umask {
        // SAFETY: this closure runs in the child after `fork` and calls only the async-signal-safe
        // `umask(2)` before `exec`. It mutates no state in the parent test process.
        unsafe {
            command.pre_exec(move || {
                libc::umask(child_umask);
                Ok(())
            });
        }
    }
    command
}

fn read_stderr(child: &mut OwnedChild) -> String {
    let mut stderr = String::new();
    child
        .child
        .stderr
        .take()
        .expect("the daemon stderr is piped")
        .read_to_string(&mut stderr)
        .expect("the daemon stderr reads");
    stderr
}

struct OwnedChild {
    child: Child,
    reaped: bool,
}

impl OwnedChild {
    fn new(child: Child) -> Self {
        Self {
            child,
            reaped: false,
        }
    }

    fn terminate(&mut self) {
        // SAFETY: the pid belongs to this unreaped child and the signal is a constant.
        assert_eq!(
            unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) },
            0
        );
    }

    fn wait_bounded(&mut self, within: Duration) -> ExitStatus {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().expect("query the daemon child") {
                self.reaped = true;
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the daemon did not stop within {within:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
