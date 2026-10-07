//! The contract this crate is built against: the exit ladder, the eight-command tree, the
//! tracker's own configuration directory, and the `control.json` the daemon writes.
//!
//! `assert_cmd` is unavailable offline, so the binary is the one cargo built for this test
//! target, as `crates/goalcup-daemon/tests/binary_surface.rs` does it.
//!
//! **Nothing in this file touches the process environment.** Most
//! of the tests here spawn `goalcup` with `Command::output()`, which reads the environment
//! map, and `std::env::set_var` is `unsafe` in edition 2024 for exactly that reason: a
//! `set_var` on one test thread while another is building a child's environment is
//! undefined behaviour, not a flake. The environment-dependent cases live in
//! `tests/env_resolution.rs`, which spawns nothing and runs them one after another.

mod support;

use goalcup_cli::CliError;
use std::process::Command;

fn goalcup() -> Command {
    Command::new(env!("CARGO_BIN_EXE_goalcup"))
}

#[test]
fn the_exit_ladder_reserves_two_for_a_verdict_and_gives_a_gate_four() {
    assert_eq!(CliError::Refused("x".into()).exit_code(), 1);
    assert_eq!(CliError::Gate("x".into()).exit_code(), 4);
    assert_eq!(CliError::Internal("x".into()).exit_code(), 3);
    for e in [
        CliError::Refused("a".into()),
        CliError::Gate("b".into()),
        CliError::Internal("c".into()),
    ] {
        assert_ne!(
            e.exit_code(),
            2,
            "round 2 seams A7: 2 belongs to a verdict alone"
        );
    }
}

#[cfg(feature = "v1-compat")]
#[test]
fn help_lists_eight_subcommands_and_points_verification_at_the_one_binary() {
    let out = goalcup().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    for cmd in [
        "auth", "start", "status", "entries", "ui", "resume", "seal", "doctor",
    ] {
        assert!(text.contains(cmd), "--help must list {cmd}");
    }
    // One verifier binary. `goalcup verify` is not a sub-command.
    let bad = goalcup().arg("verify").output().unwrap();
    assert!(!bad.status.success(), "`goalcup verify` must not exist");
    assert!(
        text.contains("goalcup-verify"),
        "--help says where verification lives"
    );
}

/// **Exactly** eight, in both helps.
///
/// The count matters as much as the membership: clap ships a built-in `help` sub-command
/// that appears in this list looking exactly like one this tree decided to ship, and a
/// ninth line here is either that or a `verify` somebody put back.
#[cfg(feature = "v1-compat")]
#[test]
fn the_commands_block_holds_the_eight_and_nothing_else_under_either_help() {
    for flag in ["-h", "--help"] {
        let out = goalcup().arg(flag).output().unwrap();
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
                "auth", "start", "status", "entries", "ui", "resume", "seal", "doctor"
            ],
            "`goalcup {flag}` lists the eight and nothing else"
        );
        // The line pointing at the one verifier binary survives the SHORT help too, which
        // is the one a reader who is lost actually runs.
        assert!(
            text.contains("goalcup-verify"),
            "`goalcup {flag}` says where verification lives"
        );
    }
}

/// The exit ladder is under BOTH helps, and rung 4 names every gate.
///
/// Rung 4 is this tree's own invention and the one a participant actually meets, so `--help`
/// documents it. Every gate is named here, so a help text that drops one fails rather than
/// misleading a participant whose resume left under a number `--help` says belongs to a
/// harness that never armed.
#[cfg(feature = "v1-compat")]
#[test]
fn the_exit_ladder_is_under_both_helps_and_names_every_gate() {
    for flag in ["-h", "--help"] {
        let out = goalcup().arg(flag).output().unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(
            text.contains("Exit codes:"),
            "`goalcup {flag}` carries the ladder"
        );
        for rung in ["0", "1", "3", "4"] {
            assert!(
                text.contains(rung),
                "`goalcup {flag}` names rung {rung}: {text}"
            );
        }
        assert!(
            text.contains("ARMING GATE"),
            "and says what 4 is, which is the one a participant meets: {text}"
        );
        assert!(
            text.contains("lock gate") && text.contains("goalcup resume"),
            "and names the harness-lock gate on that rung: {text}"
        );
        assert!(
            text.contains("local-resume gate"),
            "and names the local serialization gate on that rung: {text}"
        );
        assert!(
            text.contains("2 is never this binary's"),
            "and that 2 is the verifier's and not this one's: {text}"
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
    let secret = goalcup_cli::config::grant_path(tmp.path());
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
    let real = r#"{"control_api_version":3,"daemon_version":"0.1.0","address":"127.0.0.1:53310",
            "port":53310,"receiver_address":"127.0.0.1:53311","token":"t"}"#;
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

#[test]
fn the_control_file_this_cli_reads_is_the_five_field_document_the_daemon_writes() {
    let tmp = tempfile::tempdir().unwrap();
    // Byte for byte the shape of `goalcup_daemon::control::ControlFile`.
    std::fs::write(
        goalcup_cli::config::control_path(tmp.path()),
        r#"{"control_api_version":3,"daemon_version":"0.1.0","address":"127.0.0.1:53310",
            "port":53310,"token":"t"}"#,
    )
    .unwrap();
    let f = goalcup_cli::config::read_control_file(tmp.path()).unwrap();
    assert_eq!(
        (f.control_api_version, f.port, f.token.as_str()),
        (3, 53310, "t")
    );
    // The Unix transport is cut on both sides; a `socket` key is ignored, never honoured.
    let source = include_str!("../src/config.rs");
    assert!(
        !source.contains("socket"),
        "no Unix transport, on either side"
    );
}

/// Every rung of the ladder, from every variant, and the one place `2` is allowed to come
/// from.
///
/// The test above pins the three variants a command raises by hand. This one walks
/// the whole enum — including the four wrapped errors, whose numbers no test named — so a
/// variant added later without a rung is a failure here rather than a participant reading a
/// verdict's number off a CLI that could not open a file.
#[test]
fn every_variant_lands_on_a_rung_and_no_variant_lands_on_two() {
    let control_gate = CliError::Control(goalcup_protocol::control::ControlError::ArmingTimeout {
        detail: "the folder was never opened".to_string(),
    });
    assert_eq!(
        control_gate.exit_code(),
        4,
        "an arming timeout is the gate saying no, and it leaves under 4"
    );
    assert_eq!(
        CliError::Control(goalcup_protocol::control::ControlError::PreflightFailed(
            "the daemon ships a different manifest".to_string()
        ))
        .exit_code(),
        1,
        "2026-09-06, M1c Task 10: 4 is D16's arming gate and nothing else. A preflight \
         refusal is raised before an entry exists and is fixed by installing a matching \
         build, so it is a rung-1 refusal the participant can act on"
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
        control_gate,
        CliError::Protocol(goalcup_protocol::ProtocolError::Signature("x".into())),
        CliError::Refused("a".into()),
        CliError::Gate("b".into()),
        CliError::Internal("c".into()),
    ];
    for e in variants {
        let code = e.exit_code();
        assert!(
            [1, 3, 4].contains(&code),
            "{e:?} left under {code}, which is not a rung of the ladder"
        );
    }
}

/// Rung 4 is a GATE's, and this enumerates every site in `src/` that raises one.
///
/// `goalcup resume`'s lock gate raises `CliError::Gate` too, from two places, so a sweep of
/// only the refusals `goalcup start` can raise would miss them, and `--help`, `lib.rs` and
/// README would come to say three different things about one number.
///
/// So the sweep is over the whole of `src/`, in the shape of
/// `start_launch.rs`'s `retry_busy` walk: every `CliError::Gate(` construction, wherever it
/// sits, has to be one of the three gates. A sixth site is not forbidden — it is a decision,
/// and this test is where the decision has to be written down before it ships, beside the
/// ladder in `lib.rs` and the sentence in `--help`.
///
/// The end-to-end halves are `tests/start_arming.rs`, where the arming gate is driven
/// against a mock that can withhold a signal, and `tests/resume_locks.rs`, where the lock
/// gate is driven against real files on disk.
#[test]
fn four_is_a_gates_rung_and_every_site_that_raises_it_is_a_gate() {
    assert_eq!(
        CliError::Gate("the arming gate closed on entry e1".into()).exit_code(),
        4
    );
    assert_eq!(
        CliError::Control(goalcup_protocol::control::ControlError::ArmingTimeout {
            detail: "the folder was never opened".to_string()
        })
        .exit_code(),
        4
    );
    // Everything else `goalcup start` can refuse with, and not one of them is a 4.
    for (name, e) in [
        (
            "no grant",
            CliError::Refused("run `goalcup auth` first".into()),
        ),
        (
            "no daemon",
            CliError::Control(goalcup_protocol::control::ControlError::Unreachable(
                "no control.json".into(),
            )),
        ),
        (
            "a preflight the daemon refused",
            CliError::Control(goalcup_protocol::control::ControlError::PreflightFailed(
                "the daemon ships a different manifest".into(),
            )),
        ),
        (
            "an entry that is already sealed",
            CliError::Control(goalcup_protocol::control::ControlError::Conflict(
                "already sealed".into(),
            )),
        ),
        (
            "a document this CLI could not parse",
            CliError::Internal("not written yet".into()),
        ),
        // The same fact `goalcup seal` puts on rung 1, so `goalcup resume` puts it
        // there too. A daemon still holding an entry after the retry is spent has decided
        // nothing about this run, and one fact on two rungs is a number no script can
        // branch on.
        (
            "a tracker still busy when the retry is spent",
            CliError::Refused("still busy after 3 attempts".into()),
        ),
    ] {
        assert_ne!(
            e.exit_code(),
            4,
            "{name} is not a gate, and 4 is a gate's number"
        );
    }

    // Every construction of the rung, enumerated. One line per site, which is how this crate
    // already writes them — `CliError::Gate(format!(` opens the call and the sentence
    // follows underneath.
    let mut sites: Vec<(String, usize, String)> = Vec::new();
    for entry in support::walk("crates/goalcup-cli/src") {
        let Ok(source) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for (n, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            // The match arm in `lib.rs` reads the rung; it does not raise it.
            if !trimmed.contains("CliError::Gate(")
                || trimmed.contains("CliError::Gate(_)")
                || trimmed.starts_with("//")
            {
                continue;
            }
            let file = entry
                .strip_prefix(support::repo_root())
                .unwrap_or(&entry)
                .display()
                .to_string();
            sites.push((file, n + 1, trimmed.to_string()));
        }
    }
    let by_file = |name: &str| sites.iter().filter(|(f, _, _)| f.ends_with(name)).count();
    assert_eq!(
        by_file("src/commands/start.rs"),
        1,
        "the arming gate: a gate the daemon settled. {sites:#?}"
    );
    assert_eq!(
        by_file("src/commands/resume.rs"),
        3,
        "the two harness-lock sites plus the local-resume serialization gate. \
         {sites:#?}"
    );
    assert_eq!(
        sites.len(),
        4,
        "a fifth site is a fourth gate, and rung 4 has three — say which it is in `lib.rs`'s \
         ladder, in `AFTER_HELP`, and here: {sites:#?}"
    );
}

/// A verdict `goalcup-verify` produced, relayed with its own number unchanged — including
/// the `2` this CLI never chooses for itself.
///
/// That no variant of `CliError` lands on 2 is asserted by
/// `every_variant_lands_on_a_rung_and_no_variant_lands_on_two` above, which walks the enum.
/// This one checks the other half — that the relay does not re-derive the number.
#[test]
fn a_relayed_verdict_keeps_the_verifiers_own_number() {
    use goalcup_verifier::checks::Severity;
    let mut verdict = a_verdict();
    verdict.overall = Severity::Fail;
    assert_eq!(goalcup_cli::relay_verdict(&verdict), 2);
    verdict.overall = Severity::Flag;
    assert_eq!(goalcup_cli::relay_verdict(&verdict), 1);
    verdict.overall = Severity::Pass;
    assert_eq!(goalcup_cli::relay_verdict(&verdict), 0);
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
/// **`--dry-run` is in the list on purpose.** It makes no call — `tests/seal_scan.rs` counts
/// the mock's calls and finds none. A real seal resolves the client before the entry, the
/// same order `goalcup resume` takes, because its first need is a daemon; a `--dry-run`
/// resolves neither a client nor anything else the tracker owns, and the second half of this
/// test is that difference. Everything past it — the scan, the self-check, the trace
/// directory and the printed verifier line — is driven against the in-process mock in
/// `tests/seal_scan.rs`.
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

    // `--dry-run` is the one that does NOT go this way. It asks
    // the tracker nothing, so it builds no client and never reads `control.json`: on this
    // machine it gets as far as the entry record, and the refusal is about the record. Still
    // rung 1, still actionable, and about the thing that is actually missing.
    let out = goalcup()
        .args(["seal", "--dry-run"])
        .env_clear()
        .envs(support::clean_env())
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .env("GOALCUP_CONNECTORS_DIR", support::connectors_dir())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        said.starts_with("goalcup: ") && said.contains("no run has been started"),
        "a dry run on a machine with no daemon is refused for the record it cannot find, \
         never for a tracker it was never going to call: {said:?}"
    );
    assert!(
        !said.contains(&expected),
        "and it did not go looking for the daemon's own file: {said:?}"
    );
}

/// The three participant surfaces are wired to real commands, and the first thing each of
/// them needs is a daemon — so on a machine with none they all leave under rung 1 naming
/// `control.json`, which is the same answer `goalcup doctor` and `goalcup start` give from
/// the same file.
///
/// Rung 1 and not the rung of an unwritten command: "there is no daemon running" is a thing
/// the participant fixes by starting one, and "this build never wrote the command" is not.
/// Everything past that first refusal
/// — the pinned key, the render, the URL that is opened — is driven against the in-process
/// mock in `tests/status_views.rs`, where a test can assert what was printed without a
/// daemon on the machine that runs it.
#[test]
fn the_three_participant_surfaces_are_no_longer_stubs_and_no_daemon_is_rung_one() {
    let cfg = support::empty_cfg();
    let expected = goalcup_cli::config::control_path(&cfg)
        .display()
        .to_string();
    // `ui` is the V1 path's (D42): it is a command only in a `v1-compat` build.
    let commands = {
        #[allow(unused_mut)]
        let mut commands = vec![vec!["status"], vec!["status", "--json"], vec!["entries"]];
        #[cfg(feature = "v1-compat")]
        commands.extend([vec!["ui"], vec!["ui", "--print"]]);
        commands
    };
    for args in commands {
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

/// `goalcup resume` is wired to a real command, and the first thing it needs is a daemon.
///
/// Rung 1, naming `control.json` — the same answer the four commands above give from the
/// same file, and deliberately **not** the rung a held lock leaves under. Everything past
/// this first refusal — the pre-POST lock check, the ordered five steps, the relaunch and
/// the hand-off — is driven against the in-process mock in `tests/resume_locks.rs`, where a
/// test can assert what was spawned without a daemon or a harness on the machine that runs
/// it.
///
/// The bare form and the flagged form both, because `--print-launch` here does not stop
/// before the daemon the way `goalcup start --print-launch` does: the entry it resumes
/// already exists, so there is nothing to preview without asking the tracker.
#[cfg(feature = "v1-compat")]
#[test]
fn resume_is_no_longer_a_stub_and_no_daemon_is_rung_one() {
    let cfg = support::empty_cfg();
    let expected = goalcup_cli::config::control_path(&cfg)
        .display()
        .to_string();
    for args in [
        vec!["resume"],
        vec!["resume", "entry-0000"],
        vec!["resume", "--print-launch", "--no-open"],
    ] {
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

/// `goalcup auth` is wired to a real command, and a run that was told nothing leaves under
/// rung 1 saying what to set.
///
/// The environment is cleared to `PATH` alone (`support::clean_env`): a developer with
/// `$GOALCUP_SERVER` exported would otherwise run this test against whatever that names,
/// which is a network call from a contract test and an answer about their machine.
#[cfg(feature = "v1-compat")]
#[test]
fn auth_is_no_longer_a_stub_and_a_missing_server_is_a_refusal_the_participant_can_fix() {
    let cfg = support::empty_cfg();
    let out = goalcup()
        .arg("auth")
        .env_clear()
        .envs(support::clean_env())
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "a command that was not told where the server is is rung 1, not a stub's rung 3"
    );
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        said.starts_with("goalcup: ") && said.contains(goalcup_cli::commands::auth::SERVER_ENV),
        "`goalcup auth` names the variable that fixes it: {said:?}"
    );
    assert!(
        goalcup_cli::commands::auth::load_grant(&cfg)
            .unwrap()
            .is_none(),
        "a refused `goalcup auth` stores nothing"
    );
}

/// `goalcup start` is wired to a real command, and neither of the two ways it refuses before
/// it can do anything leaves under a verdict's number.
///
/// A machine with no daemon is rung 1 naming `control.json` — the same answer `goalcup
/// doctor` gives, from the same file — and a command line that named no harness is a clap
/// usage error, which this binary lands on 1 rather than on clap's own 2 (see
/// `a_usage_error_never_leaves_under_a_verdicts_number`). Nothing here reaches the daemon:
/// the whole of `goalcup start` past that point is driven against the in-process mock in
/// `tests/start_launch.rs`, where a test can assert what was created without a daemon on the
/// machine that runs it.
#[cfg(feature = "v1-compat")]
#[test]
fn start_with_no_daemon_is_rung_one_and_a_start_without_a_harness_is_a_usage_error() {
    let cfg = support::empty_cfg();
    let out = goalcup()
        .args(["start", "--harness", "claude-code-cli"])
        .env_clear()
        .envs(support::clean_env())
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .env("GOALCUP_CONNECTORS_DIR", support::connectors_dir())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "no daemon is a refusal the participant can fix, not a stub's rung 3"
    );
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        said.starts_with("goalcup: ")
            && said.contains(
                &goalcup_cli::config::control_path(&cfg)
                    .display()
                    .to_string()
            ),
        "and it names where the daemon's file would have been: {said:?}"
    );

    let out = goalcup()
        .arg("start")
        .env_clear()
        .envs(support::clean_env())
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "`goalcup start` with no --harness is a usage error, and a usage error is never a 2"
    );
}

/// "No daemon is rung 1" **through the binary**.
///
/// `read_control_file` returning `Refused` is the library's answer. This runs the shipped
/// binary against a configuration directory with no `control.json` in it and asserts the
/// number a script would read, and the path a participant would act on.
///
/// The environment is cleared to `PATH` alone plus the two variables that point this run at
/// a directory it owns: a developer with a daemon running would otherwise have this test
/// answer about their machine. `Command::env` is not `set_var` — this file spawns children,
/// and `tests/env_resolution.rs` says why that distinction is load-bearing.
#[test]
fn doctor_with_no_daemon_is_rung_one_through_the_binary_and_names_control_json() {
    let cfg = support::empty_cfg();
    let out = goalcup()
        .arg("doctor")
        .env_clear()
        .envs(support::clean_env())
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .env("GOALCUP_CONNECTORS_DIR", support::connectors_dir())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "M1 acceptance: `goalcup doctor` on a machine with no daemon reports and exits 1"
    );
    assert_ne!(out.status.code(), Some(2), "2 is a verdict's alone");
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        said.contains(
            &goalcup_cli::config::control_path(&cfg)
                .display()
                .to_string()
        ),
        "and it names where the daemon's file would have been: {said:?}"
    );
    for id in ["claude-code-cli", "codex-cli"] {
        assert!(
            said.contains(id),
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
            child.args(["--harness", "claude-code-cli", "--attach"]);
        }
        let out = child.output().unwrap();
        assert_ne!(
            out.status.code(),
            Some(101),
            "{command} must not panic on a non-UTF-8 environment: {out:?}"
        );
    }
}

/// `goalcup doctor --help` states the exit codes of BOTH its modes.
///
/// The two ladders overlap at 1 — a verdict's `Flag` and this CLI's `Refused` — and they do
/// not collide only because `--verify` runs no check of its own. A reader who cannot see
/// that in `--help` will reverse-engineer it from one run and get it wrong.
#[cfg(feature = "v1-compat")]
#[test]
fn doctor_help_says_which_ladder_each_of_its_two_modes_leaves_under() {
    let out = goalcup().args(["doctor", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    for said in ["--verify", "exit code", "verdict"] {
        assert!(
            text.to_lowercase().contains(&said.to_lowercase()),
            "`goalcup doctor --help` must say {said:?}: {text}"
        );
    }
    assert!(
        text.contains("--verify-with-dev") && text.contains("--verify-dev-key"),
        "and it names the two development flags: {text}"
    );
}

/// The default build has no verify mode (D42): `goalcup doctor` offers neither `--verify` nor
/// the development flags, and its footer names only the exit codes a default build can leave under.
/// The top-level footer no longer claims 2 is unreachable for a reason that is false.
#[cfg(not(feature = "v1-compat"))]
#[test]
fn the_default_doctor_has_no_verify_mode_and_the_help_footers_say_so() {
    let out = goalcup().args(["doctor", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        !text.contains("--verify"),
        "no verify flag in a default build: {text}"
    );
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
        top.contains("2 is never this binary's, and neither is 4"),
        "the default footer names 2 and 4 as V1-only: {top}"
    );
    assert!(
        top.contains("goalcup-verify --capture-integrity"),
        "and says where verification is: {top}"
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

/// A `Verdict` with every field set, so the relay above has something real to relay.
fn a_verdict() -> goalcup_verifier::Verdict {
    use goalcup_verifier::{KeySource, ManifestPin, Verdict};
    Verdict {
        verifier_version: "0".to_string(),
        schema_version: goalcup_verifier::atif::SCHEMA_VERSION.to_string(),
        connector_id: "claude-code-cli".to_string(),
        connector_manifest_sha256: "00".repeat(32),
        manifest_pin: ManifestPin::Pinned,
        entry_id: "e".to_string(),
        harness_version: "0".to_string(),
        key_source: KeySource::None,
        dev: false,
        overall: goalcup_verifier::checks::Severity::Pass,
        human_messages: Vec::new(),
        checks: Vec::new(),
        review_queue: Vec::new(),
        gaps: Vec::new(),
        seal_attribution: Vec::new(),
        tokens: goalcup_verifier::verdict::TokenSummary {
            by_model: Vec::new(),
            stripped: Vec::new(),
        },
        parser_census: std::collections::BTreeMap::new(),
        carried_settings: Vec::new(),
    }
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
    assert_eq!(
        out.status.code(),
        Some(1),
        "no daemon is still rung 1 under --json"
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let document: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("stdout must parse as one document ({e}): {text:?}"));
    assert!(
        document.get("daemon").is_some() && document.get("connectors").is_some(),
        "and it is the report: {text:?}"
    );
}

/// `goalcup start` has no `--hours`, and `--help` does not offer one.
///
/// No code under `goalcup-daemon/src/` reads `WorkspaceOpts.deadline_hours`, and the deadline
/// a run is stamped with is the daemon's own `ENTRY_WINDOW_SECONDS` — 604 800, the same week
/// `DEFAULT_DEADLINE_HOURS` names. A flag whose number is silently replaced is worse than no
/// flag, and the 168 hours is a rule rather than a preference.
#[cfg(feature = "v1-compat")]
#[test]
fn start_offers_no_deadline_flag_because_the_window_is_a_rule() {
    let out = goalcup().args(["start", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        !text.contains("--hours"),
        "`goalcup start --help` must not offer a deadline: {text}"
    );
    // The flags that ARE the participant's, so this test fails on a help that lost the lot
    // rather than passing because the block moved.
    for flag in [
        "--harness",
        "--attach",
        "--print-launch",
        "--no-open",
        "--dev",
        "--goal-file",
        "--capture-reserve-bytes",
    ] {
        assert!(
            text.contains(flag),
            "`goalcup start --help` must list {flag}"
        );
    }
    let refused = goalcup()
        .args(["start", "--harness", "codex-cli", "--hours", "24"])
        .output()
        .unwrap();
    assert!(
        !refused.status.success(),
        "a flag this build does not have is a usage error and never a silently ignored one"
    );
    assert!(
        String::from_utf8(refused.stderr)
            .unwrap()
            .contains("--hours"),
        "and clap names it"
    );
}

// ------------------------------------------------------------------ the default build (D42)
//
// Without `v1-compat` the binary is the M2 capture CLI: five commands, and the V1 ones refuse by
// name instead of vanishing into a clap usage error.

/// The commands a default build lists, in order, and nothing else.
#[cfg(not(feature = "v1-compat"))]
#[test]
fn a_default_build_lists_the_five_capture_commands_and_nothing_else() {
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
            ["start", "status", "entries", "seal", "doctor"],
            "`goalcup {flag}` lists the capture commands and nothing else"
        );
        assert!(
            text.contains("goalcup-verify") && text.contains("Exit codes:"),
            "`goalcup {flag}` says where verification lives and carries the ladder: {text}"
        );
        assert!(
            text.contains("v1-compat"),
            "and says that the other rungs belong to the V1 path: {text}"
        );
    }
    let bad = goalcup().arg("verify").output().unwrap();
    assert!(!bad.status.success(), "`goalcup verify` must not exist");
}

/// `auth`, `resume` and `ui` are hidden stubs in a default build: a refusal, rung 1, naming
/// the feature, written before any daemon or configuration is looked at.
#[cfg(not(feature = "v1-compat"))]
#[test]
fn a_default_build_refuses_each_v1_command_by_name_and_never_panics() {
    let cfg = support::empty_cfg();
    for args in [
        vec!["auth"],
        vec!["auth", "--server", "https://example.invalid", "--no-open"],
        vec!["resume"],
        vec!["resume", "entry-0000", "--print-launch"],
        vec!["ui"],
        vec!["ui", "--print"],
    ] {
        let out = goalcup()
            .args(&args)
            .env_clear()
            .envs(support::clean_env())
            .env("GOALCUP_CONFIG_DIR", &cfg)
            .env("GOALCUP_CONNECTORS_DIR", support::connectors_dir())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "`goalcup {args:?}`");
        let said = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            said.starts_with("goalcup: ")
                && said.contains(&format!("`goalcup {}`", args[0]))
                && said.contains("v1-compat")
                && said.contains("--goal-file"),
            "`goalcup {args:?}` says what it is and how to get a capture run: {said:?}"
        );
        assert!(
            !said.contains("panicked"),
            "a refusal and not a panic: {said:?}"
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).is_empty(),
            "`goalcup {args:?}` printed nothing before it refused"
        );
    }
}

/// A default `goalcup start` names its goal: the V1 path's flags are not in `--help`, and
/// leaving `--goal-file` out is a usage error that says so.
#[cfg(not(feature = "v1-compat"))]
#[test]
fn a_default_start_requires_a_goal_file_and_offers_no_v1_flags() {
    let out = goalcup().args(["start", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    for flag in [
        "--harness",
        "--dev",
        "--goal-file",
        "--capture-reserve-bytes",
    ] {
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
    ] {
        assert!(
            !text.contains(flag),
            "`goalcup start --help` must not offer {flag}: {text}"
        );
    }
    let out = goalcup()
        .args(["start", "--harness", "codex-cli"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "a usage error is rung 1");
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        said.contains("--goal-file"),
        "and names the missing flag: {said}"
    );
    let refused = goalcup()
        .args([
            "start",
            "--harness",
            "codex-cli",
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
