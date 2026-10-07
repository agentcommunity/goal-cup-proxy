//! What `goalcup-daemon` answers before it opens anything.
//!
//! Two questions a `goalcup doctor` asks of a daemon that is not up, which is exactly when
//! they are worth asking: which binary is this, and what does it do.
//!
//! `CARGO_BIN_EXE_goalcup-daemon` is the binary the test harness built, so these run against
//! the shipped argument parser rather than against `Args` as a Rust type.

/// `DAEMON_VERSION` is chained as `EventBody::VersionChange` and served on
/// `GET /v0/health`, so the one process that knows which binary is running could only be
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

/// Clap uses a struct's doc comment as `long_about`, so `--help` opens with it: the opening
/// says what the program is, and the paragraph describing `--print-addresses` sits on its
/// flag.
#[test]
fn help_opens_by_describing_the_program_and_not_one_of_its_flags() {
    let long = help(&["--help"]);
    let short = help(&["-h"]);

    for (name, text) in [("--help", &long), ("-h", &short)] {
        assert!(
            text.contains("capture daemon"),
            "{name} must open by saying what the program is: {text}"
        );
        // The paragraph about `--print-addresses` is still in the output, on the flag it
        // describes, but it must not be the first thing a reader meets.
        let opening = text.split("Usage:").next().unwrap_or_default();
        assert!(
            !opening.contains("where a client connects"),
            "{name} opens with `--print-addresses`'s own paragraph: {opening}"
        );
    }
    // The flag's paragraph is still documented, on the flag.
    assert!(
        long.contains("where a client connects"),
        "--print-addresses keeps its explanation: {long}"
    );
    // And every flag this build has is named, so a reader of `--help` can configure it.
    for flag in ["--config", "--foreground", "--print-addresses", "--version"] {
        assert!(long.contains(flag), "--help does not name {flag}");
    }
}

fn help(args: &[&str]) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-daemon"))
        .args(args)
        .output()
        .expect("the binary runs");
    assert!(out.status.success(), "{args:?} exits 0");
    String::from_utf8_lossy(&out.stdout).to_string()
}
