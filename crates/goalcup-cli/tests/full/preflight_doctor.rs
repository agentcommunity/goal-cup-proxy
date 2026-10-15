//! `goalcup doctor` over the shipped `harness.toml` connectors, and the one door `--dev`
//! comes through. The V1 tier, version-range, banned-environment, required-path and free-port
//! gates read the V1 `manifest.toml` and went with it (OS-29).

use crate::common::support;

use goalcup_cli::commands::doctor::{DoctorOptions, run_with};

/// An environment with no `PATH`: no harness executable is found, so every connector's
/// `harness_executable` check fails and the other checks are read on their own.
fn no_path() -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::new()
}

fn opts() -> DoctorOptions {
    DoctorOptions {
        json: false,
        harness: None,
    }
}

/// With no daemon running the report still says what it found about the daemon, and the
/// daemon check fails only when `goalcup start` could not start one: no `goalcup-daemon` beside
/// this executable or on `PATH` (the run-scoped daemon).
#[test]
fn doctor_reports_on_a_machine_with_no_daemon_and_fails_only_without_a_daemon_binary() {
    let mut out = Vec::new();
    let code = run_with(
        None,
        &support::connectors_dir(),
        &support::tmp_cfg(),
        &no_path(),
        &opts(),
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    let daemon_line = text
        .lines()
        .find(|line| line.contains(" daemon "))
        .unwrap_or_else(|| panic!("a daemon line: {text}"));
    for id in ["claude-code-cli", "codex-cli", "hermes-cli"] {
        assert!(text.contains(id), "every shipped connector is reported on");
    }
    // The same lookup `goalcup start` makes, from this same executable.
    if goalcup_cli::daemon::spawn::daemon_binary().is_ok() {
        assert!(daemon_line.contains("PASS"), "{daemon_line}");
    } else {
        assert!(daemon_line.contains("FAIL"), "{daemon_line}");
        assert_eq!(code, 1, "a daemon that cannot be started fails the report");
    }
    assert_ne!(code, 2, "2 is a verdict's alone");
}

/// A `.goalcup-preflight-<pid>` was the V1 `required_paths` check asking whether it could
/// write, removed in the next statement — unless the process was killed in between. `goalcup
/// doctor` is the command that looks at this directory, so it is the one that tidies it.
#[test]
fn doctor_sweeps_the_probe_files_a_killed_run_left_in_the_configuration_directory() {
    let cfg = support::tmp_cfg();
    let stale = cfg.join(".goalcup-preflight-999999");
    std::fs::write(&stale, b"").expect("a probe file from a run that was killed");
    let keep = cfg.join("control.json.bak");
    std::fs::write(&keep, b"").expect("a file that is none of this command's business");

    let mut out = Vec::new();
    run_with(
        None,
        &support::connectors_dir(),
        &cfg,
        &no_path(),
        &opts(),
        &mut out,
    )
    .unwrap();
    assert!(!stale.exists(), "the stale probe file is swept");
    assert!(keep.exists(), "and nothing else in the directory is");
}

// ------------------------------------------------------------ the roster and the one door

/// A `control.json` naming a daemon this run has no client for: the daemon check is a `Warn`,
/// so the exit code under test is the connectors' and not a missing daemon's.
fn cfg_with_a_daemon() -> std::path::PathBuf {
    let cfg = support::tmp_cfg();
    std::fs::write(
        goalcup_cli::config::control_path(&cfg),
        format!(
            r#"{{"control_api_version":{},"daemon_version":"0.1.0","address":"127.0.0.1:1",
                "port":1,"receiver_address":"127.0.0.1:2","token":"t"}}"#,
            goalcup_protocol::control::CONTROL_API_VERSION
        ),
    )
    .unwrap();
    cfg
}

fn doctor_at(
    connectors: &std::path::Path,
    cfg: &std::path::Path,
    o: &DoctorOptions,
) -> Result<(u8, String), goalcup_cli::CliError> {
    let mut out = Vec::new();
    let code = run_with(None, connectors, cfg, &no_path(), o, &mut out)?;
    Ok((code, String::from_utf8(out).unwrap()))
}

/// `--harness <id>` narrows the report AND the number to one connector.
///
/// "Is this machine healthy" and "can I start THIS harness" are two questions, and a
/// participant about to start one harness is asking the second: a machine can start Codex
/// perfectly well while another connector's check fails and `goalcup doctor` leaves under 1.
/// The failure planted here is an edited `claude-code-cli/harness.toml`, which no longer
/// matches its pin; `codex-cli` is untouched.
#[test]
fn harness_scopes_the_doctor_report_and_its_exit_code_to_one_connector() {
    let cfg = cfg_with_a_daemon();
    let dir = support::empty_cfg();
    support::copy_connectors_into(&dir);
    let claude = dir.join("claude-code-cli/harness.toml");
    let text = std::fs::read_to_string(&claude).unwrap();
    std::fs::write(
        &claude,
        format!("{text}\n# edited: no longer the pinned bytes\n"),
    )
    .unwrap();

    let (code, text) = doctor_at(&dir, &cfg, &opts()).unwrap();
    assert_eq!(
        code, 1,
        "unscoped, one connector's failure fails the lot: {text}"
    );
    for id in ["claude-code-cli", "codex-cli", "hermes-cli"] {
        assert!(
            text.contains(id) || text.contains(&dir.join(id).display().to_string()),
            "and every connector is reported: {text}"
        );
    }

    // No `PATH` here, so codex-cli's own `harness_executable` fails; what is pinned is that
    // claude-code-cli's failure is not in the scoped report, and so not in its number.
    let mut scoped = opts();
    scoped.harness = Some("codex-cli".to_string());
    let (_, text) = doctor_at(&dir, &cfg, &scoped).unwrap();
    assert!(text.contains("codex-cli"));
    assert!(
        !text.contains("claude-code-cli"),
        "a scoped report is about the one connector: {text}"
    );
    scoped.json = true;
    let (code, text) = doctor_at(&dir, &cfg, &scoped).unwrap();
    let document: serde_json::Value = serde_json::from_str(&text).unwrap();
    let reports = document["connectors"].as_array().unwrap();
    assert_eq!(reports.len(), 1, "{document:#}");
    let failed: Vec<&str> = reports[0]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["level"] == "fail")
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        failed,
        ["harness_executable"],
        "only the asked-after connector's own failure: {document:#}"
    );
    assert_eq!(code, 1, "and the number is that report's");

    let mut absent = opts();
    absent.harness = Some("not-a-connector".to_string());
    let err = doctor_at(&dir, &cfg, &absent)
        .expect_err("a harness this build does not ship is a refusal");
    assert_eq!(err.exit_code(), 1);
    assert!(
        format!("{err}").contains("codex-cli"),
        "and it names the ids that are there: {err}"
    );
}

/// An id roster drops exactly the connector whose `harness.toml` does not parse, because it
/// declares no id. Doctor enumerates directories instead, so the broken one is a named failure
/// and the command cannot leave under 0.
#[test]
fn a_connector_whose_harness_toml_does_not_parse_is_a_named_failure_rather_than_a_gap() {
    let dir = support::empty_cfg();
    support::copy_connectors_into(&dir);
    let broken = dir.join("broken");
    std::fs::create_dir_all(&broken).expect("the broken connector's directory");
    std::fs::write(broken.join("harness.toml"), "this is not = = toml\n").expect("a broken file");

    let (code, text) = doctor_at(&dir, &support::tmp_cfg(), &opts()).unwrap();
    assert!(
        text.contains(&broken.display().to_string()),
        "the report names the directory it could not read: {text:?}"
    );
    assert!(
        text.contains("FAIL"),
        "and it is a failure, not a gap: {text:?}"
    );
    assert_eq!(code, 1, "so the command cannot leave under 0");
    for id in ["claude-code-cli", "codex-cli", "hermes-cli"] {
        assert!(text.contains(id), "and the others are still reported on");
    }
}

/// `--dev` is the ONE door, and doctor never types it: a connector this build does not pin
/// is a `manifest_pin` failure whose fix names the door, and the shipped connectors pass the
/// same check without the report mentioning `--dev` at all.
#[test]
fn an_unpinned_rulebook_is_a_doctor_failure_whose_fix_names_the_one_door() {
    let dir = support::empty_cfg();
    support::copy_connectors_into(&dir);
    let forked = dir.join("codex-synthetic");
    std::fs::create_dir(&forked).unwrap();
    let text = std::fs::read_to_string(dir.join("codex-cli/harness.toml"))
        .unwrap()
        .replacen("id = \"codex-cli\"", "id = \"codex-synthetic\"", 1);
    std::fs::write(forked.join("harness.toml"), text).unwrap();

    let mut json = opts();
    json.json = true;
    let (code, out) = doctor_at(&dir, &cfg_with_a_daemon(), &json).unwrap();
    assert_eq!(code, 1, "{out}");
    let document: serde_json::Value = serde_json::from_str(&out).unwrap();
    let gate = |id: &str| -> serde_json::Value {
        document["connectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["connector_id"] == id)
            .and_then(|r| {
                r["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["id"] == "manifest_pin")
                    .cloned()
            })
            .unwrap_or_else(|| panic!("{id} carries the pin gate: {document:#}"))
    };
    let unpinned = gate("codex-synthetic");
    assert_eq!(unpinned["level"], "fail", "{unpinned:#}");
    assert!(
        unpinned["fix"].as_str().unwrap().contains("--dev"),
        "the fix names the one door: {unpinned:#}"
    );
    for id in ["claude-code-cli", "codex-cli", "hermes-cli"] {
        let shipped = gate(id);
        assert_eq!(
            shipped["level"], "pass",
            "{id} is the rulebook this build pins"
        );
        assert!(
            !shipped.to_string().contains("--dev"),
            "{id} did not come through the door, so nothing says it did"
        );
    }
}

/// `goalcup doctor` fails when the harness `goalcup start` would run is not there, and passes
/// any build that is (D53: recorded, not pinned): the same lookup `start` makes, without
/// starting anything or hashing the file; the report names the path only.
#[test]
fn doctor_fails_naming_a_missing_harness_and_passes_any_build_it_finds() {
    let cfg = cfg_with_a_daemon();
    let mut scoped = opts();
    scoped.harness = Some("codex-cli".to_string());
    let bin = support::empty_cfg();
    let env = std::collections::BTreeMap::from([("PATH".to_string(), bin.display().to_string())]);
    let check = |text: &str| -> String {
        text.lines()
            .find(|l| l.contains("harness_executable"))
            .unwrap_or_else(|| panic!("a harness_executable line: {text}"))
            .to_string()
    };

    let mut out = Vec::new();
    let code = run_with(
        None,
        &support::connectors_dir(),
        &cfg,
        &env,
        &scoped,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(code, 1, "no harness installed fails the report: {text}");
    let line = check(&text);
    assert!(line.contains("FAIL"), "{line}");
    assert!(
        line.contains("`codex` is not on PATH") && line.contains("codex-cli"),
        "naming the program and the harness: {line}"
    );

    // Something called `codex` that is not any measured build: it passes, named by its path.
    let impostor = bin.join("codex");
    let marker = bin.join("ran");
    std::fs::write(
        &impostor,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&impostor, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut out = Vec::new();
    let code = run_with(
        None,
        &support::connectors_dir(),
        &cfg,
        &env,
        &scoped,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "any build passes the report: {text}");
    let line = check(&text);
    let canonical = std::fs::canonicalize(&impostor).unwrap();
    let digest = goalcup_daemon::workspace::sha256_hex(&std::fs::read(&impostor).unwrap());
    assert!(
        line.contains("PASS")
            && line.contains(&canonical.display().to_string())
            && !line.contains(&digest)
            && line.contains("recorded, not pinned"),
        "naming the path and no digest: {line}"
    );
    assert!(!marker.exists(), "and it was never run");
}

// ------------------------------------------------------ the two ways the daemon check fails

/// A `control.json` as the running daemon writes it, with the version and the port the
/// caller wants to meet. Written by the test and never by the CLI — this is the one file in
/// the configuration directory that the daemon owns.
fn write_control_file(cfg: &std::path::Path, control_api_version: u32, port: u16) {
    let document = serde_json::json!({
        "control_api_version": control_api_version,
        "daemon_version": "0.0.0-test",
        "address": format!("127.0.0.1:{port}"),
        "port": port,
        "receiver_address": format!("127.0.0.1:{}", port + 1),
        "token": "a-test-control-token",
    });
    std::fs::write(
        cfg.join("control.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .expect("the control file the daemon would have written");
}

/// A loopback port nothing is listening on: bound to learn a free number, then dropped. The
/// operating system refuses a connection to it, which is exactly what a daemon that has died
/// leaving its `control.json` behind looks like from here.
fn a_closed_loopback_port() -> u16 {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port to learn a number from");
    let port = listener.local_addr().expect("the bound address").port();
    drop(listener);
    port
}

/// The version arm of `daemon_check`. A `control.json` this build
/// cannot make a client from is a daemon speaking another API version, and the fix names
/// both numbers because it is "upgrade whichever of the two is older".
#[test]
fn doctor_fails_naming_both_versions_when_the_control_file_speaks_another_api() {
    let cfg = support::tmp_cfg();
    let theirs = goalcup_protocol::control::CONTROL_API_VERSION + 1;
    write_control_file(&cfg, theirs, 1);

    let mut out = Vec::new();
    let code = run_with(
        None,
        &support::connectors_dir(),
        &cfg,
        &no_path(),
        &opts(),
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    let line = text
        .lines()
        .find(|l| l.contains("daemon") && l.contains("FAIL"))
        .unwrap_or_else(|| panic!("a FAIL line for the daemon check: {text}"));
    assert!(
        line.contains(&theirs.to_string()),
        "the daemon's version: {line}"
    );
    assert!(
        line.contains(&goalcup_protocol::control::CONTROL_API_VERSION.to_string()),
        "and this build's: {line}"
    );
    assert!(
        text.contains("upgrade whichever"),
        "with the fix that ends it: {text}"
    );
    assert_eq!(code, 1, "and the command cannot leave under 0");
}

/// The other arm of `daemon_check`. A `control.json` naming a port nothing answers
/// on is a daemon that died without taking its file with it, and the check must say so
/// (naming the port) rather than report the file's existence as health.
#[test]
fn doctor_reports_a_control_file_whose_port_nothing_is_listening_on_as_no_daemon() {
    let cfg = support::tmp_cfg();
    let port = a_closed_loopback_port();
    write_control_file(&cfg, goalcup_protocol::control::CONTROL_API_VERSION, port);
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)
        .expect("a client for a version this build speaks, whatever is on the far end");

    let mut out = Vec::new();
    let code = run_with(
        Some(&client),
        &support::connectors_dir(),
        &cfg,
        &no_path(),
        &opts(),
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    let line = text
        .lines()
        .find(|l| l.contains(" daemon "))
        .unwrap_or_else(|| panic!("a line for the daemon check: {text}"));
    assert!(
        line.contains(&port.to_string()),
        "the port it could not reach: {line}"
    );
    // A daemon that is gone is no daemon: the run-scoped one is started by `goalcup start`, so
    // the check fails only when this executable could not start one.
    if goalcup_cli::daemon::spawn::daemon_binary().is_ok() {
        assert!(line.contains("PASS"), "{line}");
    } else {
        assert!(line.contains("FAIL"), "{line}");
        assert_eq!(code, 1);
    }
    assert!(
        !text.contains("a-test-control-token"),
        "and the bearer is in no diagnostic: {text}"
    );
}
