//! What `goalcup-daemon` answers before it opens anything.
//!
//! The question a `goalcup doctor` asks of a daemon that is not up, which is exactly when
//! it is worth asking: which binary is this.
//!
//! `CARGO_BIN_EXE_goalcup-daemon` is the binary the test harness built, so this runs against
//! the shipped argument parser rather than against `Args` as a Rust type.

/// `DAEMON_VERSION` is served on `GET /v0/health`, so the one process that knows which binary is running could only be
/// asked over an authenticated socket while it was up. `--version` is the answer when it is
/// not, and it is the same string, from the same constant, rather than a second spelling.
#[test]
fn the_binary_says_which_version_it_is_without_starting_anything() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-daemon"))
        .arg("--version")
        .output()
        .expect("the binary runs");
    assert!(out.status.success(), "--version exits 0");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(goalcup_daemon::DAEMON_VERSION),
        "--version must print DAEMON_VERSION ({}), and it printed {text:?}",
        goalcup_daemon::DAEMON_VERSION
    );
    // Nothing was opened: no registry lock, no listener, no `control.json`. A `--version`
    // that started the daemon would be refused by a daemon already holding the lock, which
    // is the one moment it is worth running.
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "--version says nothing on stderr"
    );
}
