//! The contract this crate is built against: the exit ladder, the nine-command tree, the
//! tracker's own configuration directory, and the `control.json` the daemon writes.
//!
//! `assert_cmd` is unavailable offline, so the binary is the one cargo built for this test
//! target, as `crates/goalcup-daemon/tests/it/binary_surface.rs` does it.
//!
//! **Nothing in this file touches the process environment.** Most
//! of the tests here spawn `goalcup` with `Command::output()`, which reads the environment
//! map, and `std::env::set_var` is `unsafe` in edition 2024 for exactly that reason: a
//! `set_var` on one test thread while another is building a child's environment is
//! undefined behaviour, not a flake. The environment-dependent cases live in
//! `tests/env_resolution.rs`, which spawns nothing and runs them one after another.

#[path = "common/support/mod.rs"]
mod support;

use goalcup_cli::CliError;
use std::process::Command;

fn goalcup() -> Command {
    Command::new(env!("CARGO_BIN_EXE_goalcup"))
}

#[test]
fn the_exit_ladder_reserves_two_for_a_verdict_and_leaves_four_unused() {
    assert_eq!(CliError::Refused("x".into()).exit_code(), 1);
    assert_eq!(CliError::Internal("x".into()).exit_code(), 3);
    for e in [
        CliError::Refused("a".into()),
        CliError::Internal("c".into()),
    ] {
        assert_ne!(
            e.exit_code(),
            2,
            "round 2 seams A7: 2 belongs to a verdict alone"
        );
    }
}

/// Every secret this CLI writes is readable by its owner and by nobody else.
///
/// The assertion is on the group and other bits alone. `write_secret_file` opens with
/// `mode(0o600)`, and an open's mode is masked by the process umask — a participant running
/// under `umask 077` gets `0o600` and one under a stricter umask gets less. Both keep the
/// promise; an `== 0o600` would have failed the second machine for keeping it harder.
///
/// The `$GOALCUP_CONFIG_DIR` half lives in `tests/env_resolution.rs`: the mode has nothing to
/// do with the environment, and this file spawns children.
#[test]
fn every_secret_this_cli_writes_is_unreadable_by_anyone_but_its_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let secret = tmp.path().join("secret.json");
    goalcup_cli::config::write_secret_file(&secret, b"{}").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&secret).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o077,
            0,
            "a secret this CLI wrote is at mode {:o}, which is readable off the owner",
            mode & 0o777
        );
        assert_eq!(
            mode & 0o400,
            0o400,
            "and its owner can still read it: {:o}",
            mode & 0o777
        );
    }
}

/// A missing `control.json` is "there is no daemon", which is a refusal the participant can
/// fix — rung 1, the rung `ControlError::Unreachable` already gets — and the message names
/// the path the daemon's file would have been at.
///
/// This is the library seam. The end-to-end half is
/// `doctor_with_no_daemon_is_rung_one_through_the_binary_and_names_control_json` below,
/// which runs the shipped binary.
#[test]
fn no_control_file_is_no_daemon_and_that_is_rung_one_naming_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let expected = goalcup_cli::config::control_path(tmp.path());
    let (code, said) = match goalcup_cli::config::read_control_file(tmp.path()) {
        Ok(f) => (0, format!("{f:?}")),
        Err(e) => (e.exit_code(), e.to_string()),
    };
    assert_eq!(
        code, 1,
        "no daemon is a refusal the participant can fix, not the CLI failing: {said:?}"
    );
    assert!(
        said.contains(&expected.display().to_string()),
        "and it names where the daemon's file would have been: {said:?}"
    );
}

/// The read is bounded. `control.json` sits under a directory the participant owns, so its
/// size is not this CLI's to trust; a file over the cap is refused naming the cap rather
/// than pulled into memory whole and then rejected by serde.
#[test]
fn a_control_file_over_the_cap_is_refused_naming_the_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let path = goalcup_cli::config::control_path(tmp.path());
    let cap = goalcup_cli::config::CONTROL_FILE_MAX_BYTES;
    let oversized = vec![b'a'; usize::try_from(cap).unwrap() + 1];
    std::fs::write(&path, &oversized).unwrap();
    let (code, said) = match goalcup_cli::config::read_control_file(tmp.path()) {
        Ok(f) => (0, format!("{f:?}")),
        Err(e) => (e.exit_code(), e.to_string()),
    };
    assert_eq!(code, 1, "an oversized control file is a refusal: {said:?}");
    assert!(
        said.contains(&cap.to_string()) && said.contains(&path.display().to_string()),
        "and it names the cap and the file: {said:?}"
    );

    // A document exactly at the cap is still read. The bound is a bound, not a tighter
    // limit on the daemon's own file by one byte.
    let real = r#"{"control_api_version":7,"daemon_version":"0.1.0","address":"127.0.0.1:53310",
            "port":53310,"token":"t"}"#;
    let padding = usize::try_from(cap).unwrap() - real.len();
    std::fs::write(&path, format!("{real}{}", " ".repeat(padding))).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        cap,
        "the fixture is exactly at the cap"
    );
    let f = goalcup_cli::config::read_control_file(tmp.path()).unwrap();
    assert_eq!(f.port, 53310, "a file exactly at the cap still reads");
}

/// Every rung of the ladder, from every variant, and the one place `2` is allowed to come
/// from.
///
/// The ladder test pins the two variants a command raises by hand. This one walks
/// the whole enum — including the four wrapped errors, whose numbers no test named — so a
/// variant added later without a rung is a failure here rather than a participant reading a
/// verdict's number off a CLI that could not open a file.
#[test]
fn every_variant_lands_on_a_rung_and_no_variant_lands_on_two() {
    assert_eq!(
        CliError::Control(goalcup_protocol::control::ControlError::PreflightFailed(
            "the daemon ships a different manifest".to_string()
        ))
        .exit_code(),
        1,
        "a preflight refusal is raised before an entry exists and is fixed by installing a \
         matching build, so it is a rung-1 refusal the participant can act on"
    );
    let variants = [
        CliError::Io {
            path: "/nowhere".to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
        },
        CliError::Json(serde_json::from_str::<u8>("{").unwrap_err()),
        CliError::Control(goalcup_protocol::control::ControlError::Unreachable(
            "x".into(),
        )),
        CliError::Protocol(goalcup_protocol::ProtocolError::Signature("x".into())),
        CliError::Refused("a".into()),
        CliError::Internal("c".into()),
    ];
    for e in variants {
        let code = e.exit_code();
        assert!(
            [1, 3].contains(&code),
            "{e:?} left under {code}, which is not a rung of the ladder"
        );
    }
}

/// clap exits `2` on a usage error, and `2` is a verdict here — so this binary parses with
/// `try_parse` and chooses its own number, the way `goalcup-verify`'s `main` does.
/// A mistyped sub-command that exited 2 would read to a participant as an entry that failed
/// verification.
#[test]
fn a_usage_error_never_leaves_under_a_verdicts_number() {
    for args in [vec!["verify"], vec!["--no-such-flag"], vec!["stort"]] {
        let out = goalcup().args(&args).output().unwrap();
        assert!(!out.status.success(), "`goalcup {args:?}` must not succeed");
        assert_eq!(
            out.status.code(),
            Some(1),
            "`goalcup {args:?}` is a refusal the participant can fix, not a verdict"
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).is_empty(),
            "a usage error says nothing on stdout"
        );
    }
}

/// `--version` and `--help` are not failures: they print on stdout and exit 0.
#[test]
fn version_and_help_print_on_stdout_and_exit_zero() {
    for flag in ["--version", "--help"] {
        let out = goalcup().arg(flag).output().unwrap();
        assert!(out.status.success(), "{flag} exits 0");
        assert!(
            String::from_utf8_lossy(&out.stderr).is_empty(),
            "{flag} says nothing on stderr"
        );
    }
    let out = goalcup().arg("--version").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        text.contains(goalcup_cli::CLI_VERSION),
        "--version prints CLI_VERSION ({}), and it printed {text:?}",
        goalcup_cli::CLI_VERSION
    );
}

/// `goalcup seal` is wired to a real command, and the first thing it needs is a daemon.
///
/// Rung 1, and not the `Internal` rung an unwritten command would leave under: "there is no
/// daemon running" is a thing the participant fixes by starting one, and "this build never
/// wrote the command" is not.
///
/// Everything past it is driven against the in-process mock in `tests/it/seal_scan.rs` and
/// against the real daemon in `tests/m2_cli_lifecycle.rs`.
#[test]
fn seal_is_no_longer_a_stub_and_no_daemon_is_rung_one() {
    let cfg = support::empty_cfg();
    let expected = goalcup_cli::config::control_path(&cfg)
        .display()
        .to_string();
    for args in [vec!["seal"], vec!["seal", "entry-0000"]] {
        let out = goalcup()
            .args(&args)
            .env_clear()
            .envs(support::clean_env())
            .env("GOALCUP_CONFIG_DIR", &cfg)
            .env("GOALCUP_CONNECTORS_DIR", support::connectors_dir())
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "`goalcup {args:?}` with no daemon is a refusal the participant can fix, not a \
             stub's rung 3"
        );
        let said = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            said.starts_with("goalcup: ") && said.contains(&expected),
            "and it names where the daemon's file would have been: {said:?}"
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).is_empty(),
            "`goalcup {args:?}` printed nothing before it refused"
        );
    }
}

/// The three participant surfaces are wired to real commands, and each leaves under rung 1
/// when it cannot answer, printing nothing on stdout.
///
/// - `status` and `status --json` with no run on record refuse before any daemon is started:
///   the answer needs none, so nothing is written and `daemon.log` never appears.
/// - `entries` with a `daemon.toml` the daemon refuses starts that daemon (as `start` and
///   `seal` do) and refuses naming `daemon.log`, where the daemon's own words are.
///
/// Rung 1 and not the rung of an unwritten command: both are things the participant fixes.
/// Everything past the first refusal is driven against the in-process mock in
/// `tests/it/status_views.rs`, and the daemon that does start is `tests/full/run_scoped_daemon.rs`.
#[test]
fn the_three_participant_surfaces_refuse_at_rung_one_and_status_starts_nothing_for_no_run() {
    let cfg = support::empty_cfg();
    std::fs::write(
        cfg.join(goalcup_cli::daemon::spawn::DAEMON_CONFIG_NAME),
        "this is not = = toml\n",
    )
    .unwrap();
    let log = cfg.join(goalcup_cli::daemon::spawn::DAEMON_LOG_NAME);
    let run = |args: &[&str]| {
        goalcup()
            .args(args)
            .env_clear()
            .envs(support::clean_env())
            .env("GOALCUP_CONFIG_DIR", &cfg)
            .env("GOALCUP_CONNECTORS_DIR", support::connectors_dir())
            .output()
            .unwrap()
    };
    for args in [vec!["status"], vec!["status", "--json"]] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(1), "`goalcup {args:?}`");
        let said = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            said.starts_with("goalcup: ") && said.contains("no run has been started"),
            "`goalcup {args:?}` says there is no run: {said:?}"
        );
        assert!(String::from_utf8_lossy(&out.stdout).is_empty());
        assert!(!log.exists(), "`goalcup {args:?}` started a daemon");
    }
    let out = run(&["entries"]);
    assert_eq!(out.status.code(), Some(1), "a daemon that will not start");
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        said.starts_with("goalcup: ")
            && said.contains("no daemon is running")
            && said.contains(&log.display().to_string()),
        "it started one, and names where that daemon's words are: {said:?}"
    );
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
}

/// No daemon running is the normal state between runs, and `goalcup doctor` reports it through
/// the binary: the report names where the daemon's file would be and still covers every shipped
/// connector. That it does not fail on it is the `--json` test below.
///
/// The environment is cleared to `PATH` alone plus the two variables that point this run at
/// a directory it owns: a developer with a daemon running would otherwise have this test
/// answer about their machine. `Command::env` is not `set_var` — this file spawns children,
/// and `tests/env_resolution.rs` says why that distinction is load-bearing.
#[test]
fn doctor_with_no_daemon_names_control_json_through_the_binary() {
    let cfg = support::empty_cfg();
    let out = goalcup()
        .arg("doctor")
        .env_clear()
        .envs(support::clean_env())
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .env("GOALCUP_CONNECTORS_DIR", support::connectors_dir())
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(2), "2 is a verdict's alone");
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        said.contains(
            &goalcup_cli::config::control_path(&cfg)
                .display()
                .to_string()
        ),
        "it names where the daemon's file would have been: {said:?}"
    );
    for id in goalcup_protocol::pins::shipped_connector_ids() {
        assert!(
            said.contains(&id),
            "the report covers every shipped connector: {said:?}"
        );
    }
    assert!(
        !said.contains("--dev"),
        "no door was opened, so nothing says one was: {said:?}"
    );
}

#[cfg(unix)]
#[test]
fn start_and_doctor_skip_a_non_utf8_environment_pair_through_the_real_binary() {
    use std::os::unix::ffi::OsStringExt;

    let cfg = support::empty_cfg();
    let invalid_key = std::ffi::OsString::from_vec(b"GOALCUP_INVALID_\xff".to_vec());
    let invalid_value = std::ffi::OsString::from_vec(b"value-\xff".to_vec());
    for command in ["doctor", "start"] {
        let mut child = goalcup();
        child
            .arg(command)
            .env_clear()
            .envs(support::clean_env())
            .env("GOALCUP_CONFIG_DIR", &cfg)
            .env("GOALCUP_CONNECTORS_DIR", support::connectors_dir())
            .env(&invalid_key, &invalid_value);
        if command == "start" {
            let id = goalcup_protocol::pins::shipped_connector_ids()[0].clone();
            child.args(["--harness", &id, "--attach"]);
        }
        let out = child.output().unwrap();
        assert_ne!(
            out.status.code(),
            Some(101),
            "{command} must not panic on a non-UTF-8 environment: {out:?}"
        );
    }
}

/// `goalcup doctor` has no verify mode (D48): its help offers no `--verify` and the flag is
/// refused as a usage error, on rung 1. The top-level footer says that 2 and 4 are never this
/// binary's exit codes.
#[test]
fn doctor_has_no_verify_mode_and_the_help_footers_say_so() {
    let out = goalcup().args(["doctor", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.contains("--verify"), "no verify flag (D48): {text}");
    let refused = goalcup()
        .args(["doctor", "--verify", "somewhere"])
        .output()
        .unwrap();
    assert_eq!(
        refused.status.code(),
        Some(1),
        "an unknown flag is a usage error"
    );
    let top = goalcup().arg("--help").output().unwrap();
    let top = String::from_utf8(top.stdout).unwrap();
    assert!(
        top.contains("This command never exits with 2 or 4"),
        "the footer says 2 and 4 are never this binary's: {top}"
    );
}

/// No command prints a secret, and the two documents that hold one say `<redacted>` when
/// they are formatted.
#[test]
fn nothing_this_crate_formats_can_print_the_daemons_bearer() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        goalcup_cli::config::control_path(tmp.path()),
        r#"{"control_api_version":3,"daemon_version":"0.1.0","address":"127.0.0.1:1",
            "port":1,"receiver_address":"127.0.0.1:2","token":"sup3rs3cr3t"}"#,
    )
    .unwrap();
    let f = goalcup_cli::config::read_control_file(tmp.path()).unwrap();
    let printed = format!("{f:?}");
    assert!(
        !printed.contains("sup3rs3cr3t"),
        "a `{{:?}}` of the control file handed over the bearer: {printed}"
    );
    assert!(printed.contains("<redacted>"), "and it says so: {printed}");
}

/// `--json` is a promise about STDOUT, and it is kept through the shipped binary: one
/// document, nothing in front of it.
///
/// The library test beside this one (`preflight_doctor.rs`) asserts where the development
/// banner goes; this asserts that `main.rs` hands the command a stderr to put it on, which is
/// the half a library test cannot see.
#[test]
fn doctor_json_is_one_parseable_document_on_stdout_through_the_binary() {
    let cfg = support::empty_cfg();
    let out = goalcup()
        .args(["doctor", "--json"])
        .env_clear()
        .envs(support::clean_env())
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .env("GOALCUP_CONNECTORS_DIR", support::connectors_dir())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let document: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("stdout must parse as one document ({e}): {text:?}"));
    assert!(
        document.get("daemon").is_some() && document.get("connectors").is_some(),
        "and it is the report: {text:?}"
    );
    assert_eq!(
        document["daemon"]["level"], "pass",
        "no daemon running is not a failure under --json either: {text:?}"
    );
    let failed = document["connectors"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|report| report["checks"].as_array().unwrap())
        .any(|check| check["level"] == "fail");
    assert_eq!(
        out.status.code(),
        Some(u8::from(failed).into()),
        "the number follows the connectors alone: {text:?}"
    );
}

// ------------------------------------------------------------------ the capture CLI (D48)
//
// The binary is the M2 capture CLI: nine commands, and the removed V1 ones refuse by name
// instead of vanishing into a clap usage error.

/// The commands `goalcup --help` lists, in order, and nothing else.
#[test]
fn goalcup_lists_the_nine_commands_and_nothing_else() {
    for flag in ["-h", "--help"] {
        let out = goalcup().arg(flag).output().unwrap();
        assert!(out.status.success());
        let text = String::from_utf8(out.stdout).unwrap();
        let listed: Vec<String> = text
            .lines()
            .skip_while(|l| l.trim() != "Commands:")
            .skip(1)
            .take_while(|l| !l.trim().is_empty())
            .filter_map(|l| l.split_whitespace().next().map(str::to_string))
            .collect();
        assert_eq!(
            listed,
            [
                "start", "continue", "status", "entries", "seal", "doctor", "login", "prep",
                "upload"
            ],
            "`goalcup {flag}` lists the capture commands and nothing else"
        );
        assert!(
            text.contains("goalcup-verify") && text.contains("Exit codes:"),
            "`goalcup {flag}` says where verification lives and carries the ladder: {text}"
        );
    }
    let bad = goalcup().arg("verify").output().unwrap();
    assert!(!bad.status.success(), "`goalcup verify` must not exist");
}

/// `goalcup start` names its goal: the V1 path's flags are not in `--help`, and leaving
/// `--goal-file` out is a usage error that says so.
#[test]
fn goalcup_start_requires_a_goal_file_for_a_headless_harness_and_offers_no_v1_flags() {
    let out = goalcup().args(["start", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    for flag in ["--harness", "--dev", "--goal-file"] {
        assert!(
            text.contains(flag),
            "`goalcup start --help` must list {flag}"
        );
    }
    for flag in [
        "--attach",
        "--print-launch",
        "--no-open",
        "--arming-timeout",
        "--hours",
        "--capture-reserve-bytes",
    ] {
        assert!(
            !text.contains(flag),
            "`goalcup start --help` must not offer {flag}: {text}"
        );
    }
    // A headless harness without its goal file, and a chat-mode one (D57) with one, are refused
    // before a daemon is started or anything is made (a configuration directory of the test's
    // own all the same).
    let cfg = tempfile::tempdir().unwrap();
    // The two shapes are the fixture connectors (`tests/fixtures/connectors`), through `--dev`.
    let out = goalcup()
        .args(["start", "--harness", "keyed-per-run", "--dev"])
        .env("GOALCUP_CONFIG_DIR", cfg.path())
        .env("GOALCUP_CONNECTORS_DIR", support::fixture_connectors_dir())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "a refusal is rung 1");
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        said.contains("--goal-file"),
        "and names the missing flag: {said}"
    );
    let out = goalcup()
        .args([
            "start",
            "--harness",
            "chat-shared-home",
            "--dev",
            "--goal-file",
            "g.txt",
        ])
        .env("GOALCUP_CONFIG_DIR", cfg.path())
        .env("GOALCUP_CONNECTORS_DIR", support::fixture_connectors_dir())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "a refusal is rung 1");
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(said.contains("chat mode"), "{said}");
    assert_eq!(
        std::fs::read_dir(cfg.path()).unwrap().count(),
        0,
        "no daemon was started and nothing was written"
    );
    let refused = goalcup()
        .args([
            "start",
            "--harness",
            "any-harness",
            "--goal-file",
            "g.txt",
            "--attach",
        ])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1), "a V1 flag is a usage error");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("--attach"),
        "and clap names it"
    );
}
