//! The command tree (PRD-10 section 1 and section 11): the eight commands, the cut ones gone,
//! exit codes never 2, the version, and file connectors refused in a competition run before
//! anything is written. Facts only; no wording.

use std::path::Path;
use std::process::{Command, Output};

const COMMANDS: [&str; 8] = [
    "setup",
    "start",
    "continue",
    "recover",
    "status",
    "seal",
    "upload",
    "time-left",
];

/// A fresh, empty home for one spawn. `GOALCUP_SERVER=off`: unset would mean production.
struct Homes {
    goalcup: tempfile::TempDir,
    user: tempfile::TempDir,
}

impl Homes {
    fn new() -> Homes {
        Homes {
            goalcup: tempfile::tempdir().unwrap(),
            user: tempfile::tempdir().unwrap(),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_goalcup"))
            .args(args)
            .env("GOALCUP_SERVER", "off")
            .env("GOALCUP_HOME", self.goalcup.path())
            .env("HOME", self.user.path())
            .env("XDG_CONFIG_HOME", self.user.path())
            .output()
            .unwrap()
    }
}

fn is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().next().is_none()
}

/// An error is one line `goalcup: <text>` on stderr.
fn one_error_line(out: &Output) {
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(err.lines().count(), 1, "stderr: {err}");
    assert!(err.starts_with("goalcup: "), "stderr: {err}");
}

#[test]
fn help_lists_exactly_the_eight_commands() {
    let out = Homes::new().run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let help = String::from_utf8(out.stdout).unwrap();
    let names: Vec<&str> = help
        .lines()
        .skip_while(|l| l.trim() != "Commands:")
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .filter(|l| l.starts_with("  ") && !l.starts_with("   "))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(names, COMMANDS);
}

#[test]
fn cut_commands_and_unknown_flags_exit_1_never_2() {
    let homes = Homes::new();
    let cases: [&[&str]; 10] = [
        &["login"],
        &["mcp"],
        &["doctor"],
        &["entries"],
        &["prep"],
        &["--bogus"],
        &["status", "--bogus"],
        &["setup", "--json"],
        &["start", "--dev"],
        &[],
    ];
    for args in cases {
        let out = homes.run(args);
        assert_eq!(out.status.code(), Some(1), "goalcup {args:?}");
        one_error_line(&out);
    }
    assert!(is_empty(homes.goalcup.path()));
}

#[test]
fn version_is_1_0_0() {
    let out = Homes::new().run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        "goalcup 1.0.0"
    );
}

#[test]
fn file_flags_in_a_competition_run_are_refused_before_anything_is_written() {
    let files = tempfile::tempdir().unwrap();
    let file = files.path().join("given.toml");
    std::fs::write(&file, "").unwrap();
    let file = file.to_str().unwrap();
    for flag in ["--connector", "--hosts"] {
        let extras: [&[&str]; 2] = [&[], &["--repo", "someone/somewhere"]];
        for extra in extras {
            let homes = Homes::new();
            let mut args = vec!["start", flag, file];
            args.extend_from_slice(extra);
            let out = homes.run(&args);
            assert_eq!(out.status.code(), Some(1), "goalcup {args:?}");
            one_error_line(&out);
            assert!(is_empty(homes.goalcup.path()), "goalcup {args:?}");
        }
    }
}

#[test]
fn hidden_recorder_parses() {
    let out = Homes::new().run(&["recorder", "--help"]);
    assert_eq!(out.status.code(), Some(0));
}
