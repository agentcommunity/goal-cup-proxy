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
fn harness_help_names_the_built_in_ids() {
    let homes = Homes::new();
    for cmd in ["setup", "start"] {
        let out = homes.run(&[cmd, "--help"]);
        let help = String::from_utf8(out.stdout).unwrap();
        for (id, _) in goalcup::connector::builtin::CONNECTORS {
            assert!(help.contains(id), "{cmd} --help lacks {id}: {help}");
        }
    }
}

#[test]
fn internal_flags_are_accepted_but_not_in_the_help() {
    let homes = Homes::new();
    for cmd in ["start", "continue"] {
        let help = String::from_utf8(homes.run(&[cmd, "--help"]).stdout).unwrap();
        for flag in ["--test", "--connector", "--hosts"] {
            assert!(!help.contains(flag), "{cmd} --help shows {flag}: {help}");
        }
    }
}

#[test]
fn a_typo_d_flag_is_one_line_with_the_did_you_mean() {
    let out = Homes::new().run(&["start", "--hrness", "x"]);
    assert_eq!(out.status.code(), Some(1));
    one_error_line(&out);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--harness"), "stderr: {err}");
}

#[test]
fn cut_commands_and_unknown_flags_exit_1_never_2() {
    let homes = Homes::new();
    let cases: [&[&str]; 9] = [
        &["login"],
        &["mcp"],
        &["doctor"],
        &["entries"],
        &["prep"],
        &["--bogus"],
        &["status", "--bogus"],
        &["setup", "--json"],
        &["start", "--dev"],
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
    for flag in ["--connector", "--hosts"] {
        let extras: [&[&str]; 2] = [&[], &["--repo", "someone/somewhere"]];
        for extra in extras {
            let homes = Homes::new();
            // A file that does not exist: refused before anything is read.
            let none = homes.goalcup.path().join("none.toml");
            let mut args = vec!["start", flag, none.to_str().unwrap()];
            args.extend_from_slice(extra);
            let out = homes.run(&args);
            assert_eq!(out.status.code(), Some(1), "goalcup {args:?}");
            one_error_line(&out);
            assert!(is_empty(homes.goalcup.path()), "goalcup {args:?}");
        }
    }
}

#[test]
fn a_connector_file_given_to_harness_is_refused_in_a_competition_run() {
    let homes = Homes::new();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mine.toml");
    std::fs::write(&file, "").unwrap();
    let args = [
        "start",
        "--harness",
        file.to_str().unwrap(),
        "--repo",
        "someone/somewhere",
    ];
    let out = homes.run(&args);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--harness"));
    assert!(is_empty(homes.goalcup.path()));
}

#[test]
fn no_command_prints_the_help_and_exits_1() {
    let out = Homes::new().run(&[]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(COMMANDS.iter().all(|c| err.contains(c)), "stderr: {err}");
}

/// PRD-17 HF4: a second start on a harness home that a run already holds refuses with exit 1
/// before anything is asked of the server (the address is dead: a call would end with 3) and
/// before anything is written.
#[test]
fn second_start_on_one_home_refuses() {
    let homes = Homes::new();
    let project = homes.user.path().join("p");
    std::fs::create_dir_all(&project).unwrap();
    let connector = homes.user.path().join("tester.toml");
    std::fs::write(
        &connector,
        "id = \"tester\"\nname = \"Tester\"\nprogram = \"true\"\nskills_dir = \"{home}/skills\"\n",
    )
    .unwrap();
    let held = homes.goalcup.path().join("homes");
    std::fs::create_dir_all(&held).unwrap();
    let lock = std::fs::File::create(held.join("tester.goalcup.lock")).unwrap();
    lock.try_lock().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_goalcup"))
        .args(["start", "--practice", "--connector"])
        .arg(&connector)
        .current_dir(&project)
        .env("GOALCUP_SERVER", "http://127.0.0.1:1")
        .env("GOALCUP_HOME", homes.goalcup.path())
        .env("HOME", homes.user.path())
        .env("XDG_CONFIG_HOME", homes.user.path())
        .env_remove("https_proxy") // a build box's proxy refuses every start
        .env_remove("HTTPS_PROXY")
        .env_remove("all_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("GOALCUP_TIME_LEFT_FILE")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    one_error_line(&out);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains(&held.join("tester").display().to_string()),
        "{err}"
    );
    assert!(!homes.goalcup.path().join("runs").exists());
    drop(lock);
}
