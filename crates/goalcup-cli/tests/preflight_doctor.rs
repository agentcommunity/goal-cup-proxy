//! D12 and D23 as preflight checks, `goalcup doctor`, and the one door `--dev` comes
//! through.
//!
//! Nothing here mutates the process environment. Every check that reads one is handed a
//! `BTreeMap` built in the test, which is the whole reason `connector_checks` and
//! `sample_harness_version` take an environment rather than reading the participant's
//! (`tests/env_resolution.rs` says why that distinction is load-bearing).

mod support;

#[cfg(unix)]
#[path = "../../goalcup-daemon/tests/support/table_lock.rs"]
mod table_lock;

// The tests below that use `clean_env()` (`PATH = "/usr/bin:/bin"`) are left unguarded:
// they spawn nothing on a machine with no `codex` on that PATH. If that ever changes — a
// `codex` installed at `/usr/bin` or `/bin`, or `clean_env()` widened — they become
// process-table producers too and need `table_lock::TableLock::acquire()` like the
// tests below that already carry it.

use goalcup_cli::commands::doctor::{DoctorOptions, run_with};
use goalcup_cli::preflight::{
    Level, connector_checks, connector_checks_with_dev, required_paths_gate,
    sample_harness_version_within, server_port_gate, tier_gate,
};
use std::collections::BTreeMap;

fn clean_env() -> BTreeMap<String, String> {
    BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())])
}

fn opts() -> DoctorOptions {
    DoctorOptions {
        json: false,
        harness: None,
    }
}

#[test]
fn a_banned_variable_is_a_failure_that_says_how_to_fix_it() {
    let c = support::connector("claude-code-cli");
    let mut env = clean_env();
    env.insert("CLAUDE_CODE_SKIP_PROMPT_HISTORY".into(), "1".into());
    let r = connector_checks(&c, &env, &support::tmp_cfg());
    assert!(r.failed());
    let f = r
        .checks
        .iter()
        .find(|k| k.level == Level::Fail && k.detail.contains("CLAUDE_CODE_SKIP_PROMPT_HISTORY"))
        .expect("named in the detail");
    assert!(
        !f.fix.is_empty(),
        "a refusal a participant cannot act on is one they route around"
    );
    assert!(f.fix.contains("unset"), "say the command");
}

#[test]
fn the_tier_gate_is_d23s_measured_table_and_refuses_anything_but_full() {
    for id in ["claude-code-cli", "codex-cli"] {
        assert_eq!(
            tier_gate(&support::connector(id)).level,
            Level::Pass,
            "{id} is full in D23"
        );
    }
    let mut c = support::connector("codex-cli");
    c.manifest.tier = goalcup_protocol::manifest::Tier::ElevatedReview;
    let elevated = tier_gate(&c);
    assert_eq!(
        elevated.level,
        Level::Fail,
        "M1 runs no review, so the tier is refused here"
    );
    assert!(elevated.detail.to_lowercase().contains("review"));
    assert!(
        elevated.detail.contains("M1"),
        "say WHY it is refused now and not forever"
    );
    c.manifest.tier = goalcup_protocol::manifest::Tier::Unsupported;
    let unsupported = tier_gate(&c);
    assert_eq!(unsupported.level, Level::Fail);
    assert!(unsupported.detail.to_lowercase().contains("not supported"));
    assert_ne!(
        elevated.detail, unsupported.detail,
        "two different answers must not read as one refusal"
    );
}

#[test]
fn the_version_gate_is_d12_and_the_ports_gate_is_the_one_the_pinned_port_needs() {
    let mut c = support::connector("codex-cli");
    // No shipped connector declares `free_ports`: the launch binds the daemon's reserved port,
    // which `server_port_gate` checks. The GATE still has to work, so this test gives the
    // connector a port it holds itself — a test that bound a fixed port would assert nothing
    // on a machine where something already had it, and a machine where something already has
    // it is precisely the machine this check is for.
    assert!(
        c.manifest.preflight.free_ports.is_empty(),
        "no shipped connector pins a fixed port: {:?}",
        c.manifest.preflight.free_ports
    );
    let held = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port to hold");
    let port = held.local_addr().expect("the port it landed on").port();
    c.manifest.preflight.free_ports = vec![port];
    let r = connector_checks(&c, &clean_env(), &support::tmp_cfg());
    let gate = r
        .checks
        .iter()
        .find(|k| k.id == "free_ports")
        .expect("the ports gate is asked");
    assert_eq!(
        gate.level,
        Level::Fail,
        "a held port is a Fail now, not a surprise after the workspace exists"
    );
    assert!(
        gate.detail.contains(&port.to_string()),
        "and the refusal names it: {:?}",
        gate.detail
    );
    drop(held);
    // The version check reports Warn (not Fail) when the binary is simply absent: a
    // machine without the harness installed is a doctor report, not a broken tracker.
    let r = connector_checks(
        &support::connector("codex-cli"),
        &clean_env(),
        &support::tmp_cfg(),
    );
    let v = r.checks.iter().find(|k| k.id == "harness_version").unwrap();
    assert!(matches!(v.level, Level::Warn | Level::Pass | Level::Fail));
    assert!(
        v.detail.contains("=0.153.0"),
        "the tested range is quoted at the participant"
    );
}

#[test]
fn doctor_reports_on_a_machine_with_no_daemon_and_exits_one() {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_with(
        None,
        &support::connectors_dir(),
        &support::tmp_cfg(),
        &clean_env(),
        &opts(),
        &mut out,
        &mut err,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("daemon"),
        "it says the daemon is not reachable"
    );
    for id in ["claude-code-cli", "codex-cli"] {
        assert!(text.contains(id), "every shipped connector is reported on");
    }
    assert_eq!(
        code, 1,
        "M1 acceptance: doctor reports, and exits 1, with no daemon running"
    );
    assert_ne!(code, 2, "2 is a verdict's alone");
}

/// `--harness <id>` narrows the report AND the number to one connector.
///
/// "Is this machine healthy" and "can I start THIS harness" are two questions, and a
/// participant about to start one harness is asking the second: a machine can start Claude
/// Code perfectly well while another connector's check fails and `goalcup doctor` leaves
/// under 1.
///
/// The failure planted here is `ANTHROPIC_BASE_URL`, which is on `claude-code-cli`'s
/// `banned_env` and not on `codex-cli`'s — so one connector fails and the other does not.
/// The daemon check is answered by a `control.json` this test writes, so the number under
/// test is the connectors' and not a missing daemon's.
#[test]
fn harness_scopes_the_doctor_report_and_its_exit_code_to_one_connector() {
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
    let mut env = clean_env();
    env.insert("ANTHROPIC_BASE_URL".into(), "http://127.0.0.1:9".into());

    let run = |o: &DoctorOptions| -> Result<(u8, String), goalcup_cli::CliError> {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            None,
            &support::connectors_dir(),
            &cfg,
            &env,
            o,
            &mut out,
            &mut err,
        )?;
        Ok((code, String::from_utf8(out).unwrap()))
    };

    let (code, text) = run(&opts()).unwrap();
    assert_eq!(code, 1, "unscoped, one connector's failure fails the lot");
    for id in ["claude-code-cli", "codex-cli"] {
        assert!(text.contains(id), "and every connector is reported: {text}");
    }

    let mut scoped = opts();
    scoped.harness = Some("codex-cli".to_string());
    let (code, text) = run(&scoped).unwrap();
    assert_eq!(
        code, 0,
        "the connector asked after is fine, and that is the answer: {text}"
    );
    assert!(text.contains("codex-cli"));
    assert!(
        !text.contains("claude-code-cli") && !text.contains("opencode"),
        "a scoped report is about the one connector: {text}"
    );

    let mut scoped = opts();
    scoped.harness = Some("claude-code-cli".to_string());
    let (code, _) = run(&scoped).unwrap();
    assert_eq!(
        code, 1,
        "and scoping never hides the failure of the connector it scoped to"
    );

    let mut absent = opts();
    absent.harness = Some("not-a-connector".to_string());
    let err = run(&absent).expect_err("a harness this build does not ship is a refusal");
    assert_eq!(err.exit_code(), 1);
    assert!(
        format!("{err}").contains("claude-code-cli"),
        "and it names the ids that are there: {err}"
    );
}

/// `connector::list` answers with the ids manifests DECLARE, and a manifest that does not
/// parse declares none — so a doctor built on that roster would report on every connector
/// except the broken one and leave under 0. It enumerates directories instead.
#[test]
fn a_connector_whose_manifest_does_not_parse_is_a_named_failure_rather_than_a_gap() {
    let dir = support::empty_cfg();
    support::copy_connectors_into(&dir);
    let broken = dir.join("broken");
    std::fs::create_dir_all(&broken).expect("the broken connector's directory");
    std::fs::write(broken.join("manifest.toml"), "this is not = = toml\n").expect("a broken file");

    assert_eq!(
        goalcup_cli::connector::list(&dir)
            .expect("the roster still loads")
            .len(),
        2,
        "the id roster cannot see it, which is the whole reason doctor no longer walks it"
    );

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_with(
        None,
        &dir,
        &support::tmp_cfg(),
        &clean_env(),
        &opts(),
        &mut out,
        &mut err,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
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

#[test]
fn doctor_joins_v1_and_harness_reports_only_when_their_directories_are_identical() {
    let root = support::empty_cfg();

    let v1_shared = root.join("v1-shared");
    std::fs::create_dir(&v1_shared).unwrap();
    let v1 = std::fs::read_to_string(support::connectors_dir().join("codex-cli/manifest.toml"))
        .unwrap()
        .replacen("id = \"codex-cli\"", "id = \"shared-dev\"", 1);
    std::fs::write(v1_shared.join("manifest.toml"), v1).unwrap();

    let harness_shared = root.join("harness-shared");
    std::fs::create_dir(&harness_shared).unwrap();
    let harness =
        std::fs::read_to_string(support::connectors_dir().join("claude-code-cli/harness.toml"))
            .unwrap()
            .replacen("id = \"claude-code-cli\"", "id = \"shared-dev\"", 1);
    std::fs::write(harness_shared.join("harness.toml"), harness).unwrap();

    let basename_v1 = root.join("v1-basename-source");
    std::fs::create_dir(&basename_v1).unwrap();
    let v1 = std::fs::read_to_string(support::connectors_dir().join("codex-cli/manifest.toml"))
        .unwrap()
        .replacen("id = \"codex-cli\"", "id = \"basename-target\"", 1);
    std::fs::write(basename_v1.join("manifest.toml"), v1).unwrap();

    let invalid_harness = root.join("basename-target");
    std::fs::create_dir(&invalid_harness).unwrap();
    std::fs::write(invalid_harness.join("harness.toml"), "not = valid = toml\n").unwrap();

    let mut options = opts();
    options.json = true;
    let mut out = Vec::new();
    let mut err = Vec::new();
    run_with(
        None,
        &root,
        &support::tmp_cfg(),
        &clean_env(),
        &options,
        &mut out,
        &mut err,
    )
    .unwrap();
    let document: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let reports = document["connectors"].as_array().unwrap();
    assert_eq!(
        reports.len(),
        4,
        "two cross-directory id/basename coincidences remain four reports: {document:#}"
    );
    assert_eq!(
        reports
            .iter()
            .filter(|report| report["connector_id"] == "shared-dev")
            .count(),
        2,
        "a declared id is not a directory join key: {document:#}"
    );
    assert!(
        reports.iter().any(|report| {
            report["connector_id"] == invalid_harness.display().to_string()
                && report["checks"].as_array().is_some_and(|checks| {
                    checks.iter().any(|check| check["id"] == "harness_manifest")
                })
        }),
        "an invalid harness remains attached to its own exact directory: {document:#}"
    );
}

/// A `.goalcup-preflight-<pid>` is `required_paths_gate` asking whether it can write, and it
/// is removed in the next statement — unless the process was killed in between. `goalcup
/// doctor` is the command that looks at this directory, so it is the one that tidies it.
#[test]
fn doctor_sweeps_the_probe_files_a_killed_run_left_in_the_configuration_directory() {
    let cfg = support::tmp_cfg();
    let stale = cfg.join(".goalcup-preflight-999999");
    std::fs::write(&stale, b"").expect("a probe file from a run that was killed");
    let keep = cfg.join("control.json.bak");
    std::fs::write(&keep, b"").expect("a file that is none of this command's business");

    let mut out = Vec::new();
    let mut err = Vec::new();
    run_with(
        None,
        &support::connectors_dir(),
        &cfg,
        &clean_env(),
        &opts(),
        &mut out,
        &mut err,
    )
    .unwrap();
    assert!(!stale.exists(), "the stale probe file is swept");
    assert!(keep.exists(), "and nothing else in the directory is");
}

// ------------------------------------------------------------ the one door, and the probe

/// `--dev` is the ONE door: it is the only thing that admits a rulebook this build does not
/// pin, and where it does, the report says so.
#[test]
fn an_unpinned_rulebook_is_refused_unless_the_one_door_is_open_and_then_it_says_so() {
    let dir = support::connectors_dir_with_edited_manifest("codex-cli");
    let edited = goalcup_cli::connector::load(&dir, "codex-cli").expect("the edited copy loads");
    // The gate compares the DIRECTORY digest (manifest plus every template under it), the
    // stronger of the two pins this build carries.
    let pinned = goalcup_protocol::pins::pinned_connector_dir_sha256("codex-cli")
        .expect("codex-cli's directory is pinned");

    let strict = connector_checks(&edited, &clean_env(), &support::tmp_cfg());
    let gate = strict
        .checks
        .iter()
        .find(|k| k.id == "manifest_pin")
        .expect("the release-hash allowlist is a preflight check");
    assert_eq!(
        gate.level,
        Level::Fail,
        "a rulebook this build does not pin is refused"
    );
    assert!(
        gate.detail.contains(pinned) && gate.detail.contains(&edited.dir_sha256),
        "and the refusal names both digests: {:?}",
        gate.detail
    );
    assert!(
        gate.fix.contains("--dev"),
        "the fix names the one door: {:?}",
        gate.fix
    );

    let dev = connector_checks_with_dev(&edited, &clean_env(), &support::tmp_cfg(), true);
    let gate = dev
        .checks
        .iter()
        .find(|k| k.id == "manifest_pin")
        .expect("the check is still made under --dev");
    assert_ne!(gate.level, Level::Fail, "--dev admits it");
    assert!(
        gate.detail.contains("--dev"),
        "and every place it matters says so: {:?}",
        gate.detail
    );
    assert!(
        dev.render().contains("--dev"),
        "including on the page the participant reads"
    );

    // The shipped rulebooks are pinned, so the door changes nothing for them.
    for id in ["claude-code-cli", "codex-cli"] {
        let c = support::connector(id);
        let r = connector_checks(&c, &clean_env(), &support::tmp_cfg());
        let gate = r
            .checks
            .iter()
            .find(|k| k.id == "manifest_pin")
            .expect("every report carries the gate");
        assert_eq!(
            gate.level,
            Level::Pass,
            "{id} is the rulebook this build pins"
        );
        assert!(
            !r.render().contains("--dev"),
            "{id} did not come through the door, so nothing says it did"
        );
    }
}

/// The daemon REMEMBERS a reserved `server_port` and never re-reserves it, so the check that
/// it is still free belongs here, before a resume relaunches into a port somebody else took
/// while the run was stopped.
#[test]
fn a_remembered_server_port_is_checked_before_a_resume_and_a_held_one_is_refused() {
    assert!(
        server_port_gate(None).is_none(),
        "a connector that reserves no port has nothing to check"
    );
    let free = server_port_gate(Some(0)).expect("port 0 is still a port the caller named");
    assert_eq!(free.level, Level::Pass, "port 0 binds anywhere");

    let held = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port to hold");
    let port = held.local_addr().expect("the port it landed on").port();
    let gate = server_port_gate(Some(port)).expect("a remembered port is checked");
    assert_eq!(gate.level, Level::Fail, "somebody else has it now");
    assert!(
        gate.detail.contains(&port.to_string()),
        "and the refusal names it: {:?}",
        gate.detail
    );
    assert!(!gate.fix.is_empty(), "and says what to do about it");
    drop(held);
}

/// D12's probe runs the harness, and it runs it in an environment with no base URL and no
/// proxy in it: a probe that inherited `ANTHROPIC_BASE_URL` would be asking a different
/// server which version the participant is on.
#[cfg(unix)]
#[test]
fn the_version_probe_runs_the_harness_with_no_base_url_and_no_proxy_in_its_environment() {
    let _lock = table_lock::TableLock::acquire();
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let bin = support::empty_cfg();
    let recorded = bin.join("seen-env.txt");
    let shim = bin.join("codex");
    let mut f = std::fs::File::create(&shim).expect("the shim");
    write!(
        f,
        "#!/bin/sh\nenv > {}\necho 'codex-cli 0.153.0 (rust)'\n",
        recorded.display()
    )
    .expect("the shim's body");
    drop(f);
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("executable");

    let mut env = BTreeMap::new();
    // The shim's directory first, then the real one: `resolve_on_path` walks the PATH it is
    // given in order, and the shim itself needs a `/usr/bin` to find `env` in.
    env.insert(
        "PATH".to_string(),
        format!("{}:/usr/bin:/bin", bin.display()),
    );
    env.insert("CODEX_BASE_URL".to_string(), "http://elsewhere".to_string());
    env.insert(
        "OPENAI_BASE_URL".to_string(),
        "http://elsewhere".to_string(),
    );
    env.insert("HTTPS_PROXY".to_string(), "http://elsewhere".to_string());
    env.insert("http_proxy".to_string(), "http://elsewhere".to_string());

    let c = support::connector("codex-cli");
    let sampled = sample_harness_version_within(&c, &env, std::time::Duration::from_secs(10));
    assert_eq!(
        sampled.map_err(|e| e.to_string()),
        Ok("0.153.0".to_string()),
        "the probe reads the version out of whatever the harness printed"
    );

    let seen = std::fs::read_to_string(&recorded).expect("the shim recorded its environment");
    assert!(
        seen.lines().count() > 1,
        "the shim recorded nothing, so the loop below would assert over an empty file:          {seen:?}"
    );
    for line in seen.lines() {
        let name = line.split('=').next().unwrap_or(line).to_ascii_uppercase();
        assert!(
            !name.ends_with("_BASE_URL"),
            "the probe carried {line:?} into the harness"
        );
        assert!(
            !name.ends_with("_PROXY") && name != "PROXY",
            "the probe carried {line:?} into the harness"
        );
    }
    assert!(
        seen.contains("PATH="),
        "and it is not an empty environment: the harness still needs a PATH"
    );
}

/// A `codex` on a `PATH` of its own. `body` is the whole of a `/bin/sh` script, and what
/// comes back is the directory to put FIRST on the `PATH` a probe is handed.
///
/// The test above builds its own, because its body has to name a file inside this directory
/// and so needs the directory before the script exists.
#[cfg(unix)]
fn fake_harness(body: &str) -> std::path::PathBuf {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let bin = support::empty_cfg();
    let shim = bin.join("codex");
    let mut f = std::fs::File::create(&shim).expect("the shim");
    write!(f, "#!/bin/sh\n{body}\n").expect("the shim's body");
    drop(f);
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("executable");
    bin
}

/// The probe is bounded. A harness that never answers is a doctor report, not a `goalcup`
/// that hangs.
#[cfg(unix)]
#[test]
fn the_version_probe_is_bounded_and_a_harness_that_never_answers_is_refused() {
    let _lock = table_lock::TableLock::acquire();
    let bin = fake_harness("sleep 30");
    let env = BTreeMap::from([("PATH".to_string(), bin.display().to_string())]);
    let c = support::connector("codex-cli");
    let started = std::time::Instant::now();
    let said = match sample_harness_version_within(&c, &env, std::time::Duration::from_millis(300))
    {
        Ok(v) => panic!("a harness that printed nothing answered {v:?}"),
        Err(e) => e.to_string(),
    };
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "the probe waited for the harness rather than for its own deadline"
    );
    assert!(
        said.contains("codex"),
        "and the refusal names the harness it gave up on: {said:?}"
    );
}

/// The deadline covers the PIPES and not only the child.
///
/// A harness that backgrounds a helper leaves that helper holding this probe's stdout, and a
/// read to EOF then waits on the helper rather than on the harness: the probe returned when
/// `sleep 30` did, long after the timeout it was given, on a harness that had already exited.
#[cfg(unix)]
#[test]
fn a_backgrounded_process_holding_stdout_cannot_take_the_probe_past_its_deadline() {
    let _lock = table_lock::TableLock::acquire();
    let bin = fake_harness("sleep 30 &\necho 'codex-cli 0.153.0 (rust)'");
    let env = BTreeMap::from([(
        "PATH".to_string(),
        format!("{}:/usr/bin:/bin", bin.display()),
    )]);
    let c = support::connector("codex-cli");
    let timeout = std::time::Duration::from_millis(300);
    let started = std::time::Instant::now();
    let said = match sample_harness_version_within(&c, &env, timeout) {
        Ok(v) => panic!("a probe whose stdout is still held answered {v:?}"),
        Err(e) => e.to_string(),
    };
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the probe returned on its own deadline and not on `sleep`'s: {:?}",
        started.elapsed()
    );
    assert!(
        said.contains(&format!("{timeout:?}")),
        "and the refusal names the timeout: {said:?}"
    );
    assert!(
        said.contains("codex"),
        "and the harness it gave up on: {said:?}"
    );
}

/// The probe returning on time is not the same thing as the helper being dead: a `sleep 30 &`
/// left signalled alone, at the pid the `Child` gave, outlives the probe that refused inside
/// its deadline. The group `process_group(0)` created for this child must be killed too, not
/// only the direct child.
///
/// # It waits for a condition, not for a duration
///
/// A fixed duration standing in for a condition makes this test flaky.
/// `sample_harness_version_within` `killpg`s the whole group at its deadline, and a
/// freshly-`process_group(0)`'d `/bin/sh` can take long to reach its second line on a loaded
/// machine: a shim that wrote its helper's pid there would be killed before the write, and
/// the test would then wait for a file nobody was ever going to create. Three things close
/// that:
///
/// 1. **The fixture records the pid FIRST.** Its first two words start the helper and its
///    next statement writes the pid, through a temporary file and a rename, so what the test
///    reads is either absent or complete and never half a number.
/// 2. **The probe runs on a thread of its own**, so the test can watch for that file WHILE
///    the probe is still inside its deadline. The condition — "the helper exists and this
///    test knows its pid" — is waited for; the deadline is only interpreted after it holds.
/// 3. **The deadline is generous** ([`KILL_DEADLINE`], five seconds). This test's subject is
///    the KILL and not the bound; the bound is what
///    `the_version_probe_is_bounded_and_a_harness_that_never_answers_is_refused` and
///    `a_probe_whose_output_is_held_by_a_grandchild_still_returns_on_time` are for, and both
///    of those measure it in milliseconds.
#[cfg(unix)]
#[test]
fn the_deadline_kills_the_backgrounded_helper_and_not_only_the_direct_child() {
    // Guard on the test's main thread ONLY, as the first statement of the body. The probe
    // below runs on `std::thread::spawn`; the depth counter is thread-local, so acquiring
    // inside that closure would block on the mutex this thread already holds and deadlock.
    let _lock = table_lock::TableLock::acquire();
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    /// What the probe gets. Generous on purpose: see the third point above.
    const KILL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);
    /// The ceiling on every condition this test waits for. Only ever reached when the
    /// condition is not going to come true at all, so it is long enough to say so.
    const PATIENCE: std::time::Duration = std::time::Duration::from_secs(30);

    let bin = support::empty_cfg();
    let pidfile = bin.join("helper.pid");
    let shim = bin.join("codex");
    let mut f = std::fs::File::create(&shim).expect("the shim");
    // The helper is started and its pid is recorded before anything that takes time: the
    // direct child never answers (it sleeps in the foreground too), so the probe runs it to
    // the deadline exactly like
    // `the_version_probe_is_bounded_and_a_harness_that_never_answers_is_refused`.
    //
    // The write is `> tmp` then `mv`, because this file is READ from another process while
    // this one is writing it: a rename within one directory is atomic and a redirect is not,
    // so the reader sees no file or a whole pid and never an empty one it would parse as a
    // failure to record.
    write!(
        f,
        "#!/bin/sh\nsleep 30 &\necho $! > {pid}.tmp\nmv {pid}.tmp {pid}\nsleep 30\n",
        pid = pidfile.display()
    )
    .expect("the shim's body");
    drop(f);
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("executable");

    let env = BTreeMap::from([(
        "PATH".to_string(),
        format!("{}:/usr/bin:/bin", bin.display()),
    )]);
    let c = support::connector("codex-cli");

    // The probe, on its own thread, so the condition below can be waited for while the
    // deadline it is a precondition of is still running.
    let probe = std::thread::spawn(move || sample_harness_version_within(&c, &env, KILL_DEADLINE));

    // THE CONDITION: the fixture has started its helper and recorded which pid it is. Until
    // this holds there is nothing for the kill to prove, and a test that read the file once
    // and gave up would be measuring the machine's load.
    let started = std::time::Instant::now();
    let pid = loop {
        if let Ok(s) = std::fs::read_to_string(&pidfile)
            && let Ok(p) = s.trim().parse::<u32>()
        {
            break p;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "the fixture never recorded its helper's pid at {} — it is written before the \
             shim sleeps, so this is the shim never running rather than the deadline \
             arriving early",
            pidfile.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };

    // Only now is the deadline the subject. The probe answers a refusal, inside its own
    // bound, and the helper the fixture backgrounded is gone with it.
    let result = probe.join().expect("the probe thread");
    assert!(
        result.is_err(),
        "a harness that never answers is refused: {result:?}"
    );
    assert!(
        started.elapsed() < PATIENCE,
        "the probe waited for its own deadline and not for the helper: {:?}",
        started.elapsed()
    );

    let gone_by = std::time::Instant::now() + PATIENCE;
    let gone = loop {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            // `kill -0` on a pid that is gone prints on stderr, and this loop asks the
            // question until the answer is exactly that: the noise is the expected case.
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !alive {
            break true;
        }
        if std::time::Instant::now() >= gone_by {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(
        gone,
        "the backgrounded helper (pid {pid}) survived the probe's deadline"
    );
}

/// D12, the other way round: the harness is installed, it answers, and the number is below
/// the floor of the range the connector was tested at. That is the one branch that is a
/// `Fail` — "this build could not measure it" and "newer than tested" are `Warn`s, and the
/// answers must not read as one.
#[cfg(unix)]
#[test]
fn a_harness_below_d12s_floor_is_a_failure_naming_both_numbers() {
    let _lock = table_lock::TableLock::acquire();
    let bin = fake_harness("echo 'codex-cli 0.1.0 (rust)'");
    let env = BTreeMap::from([(
        "PATH".to_string(),
        format!("{}:/usr/bin:/bin", bin.display()),
    )]);
    let r = connector_checks(&support::connector("codex-cli"), &env, &support::tmp_cfg());
    let gate = r
        .checks
        .iter()
        .find(|k| k.id == "harness_version")
        .expect("D12 is asked of every connector");
    assert_eq!(
        gate.level,
        Level::Fail,
        "an installed harness at the wrong version is refused: {:?}",
        gate.detail
    );
    assert!(
        gate.detail.contains("0.1.0"),
        "the version found is named: {:?}",
        gate.detail
    );
    assert!(
        gate.detail.contains("=0.153.0"),
        "and the range it is outside: {:?}",
        gate.detail
    );
    assert!(
        gate.fix.contains("0.153.0"),
        "and the fix is the version to install: {:?}",
        gate.fix
    );
    assert!(r.failed(), "so the report refuses the run");
}

/// D12: a Codex newer than the tested `=0.153.0` is allowed. The gate is a `Warn` that names
/// it untested, and the report does not refuse the run.
#[cfg(unix)]
#[test]
fn a_harness_newer_than_tested_passes_preflight_with_a_warn() {
    let _lock = table_lock::TableLock::acquire();
    let bin = fake_harness("echo 'codex-cli 0.160.2 (rust)'");
    let env = BTreeMap::from([(
        "PATH".to_string(),
        format!("{}:/usr/bin:/bin", bin.display()),
    )]);
    let r = connector_checks(&support::connector("codex-cli"), &env, &support::tmp_cfg());
    let gate = r
        .checks
        .iter()
        .find(|k| k.id == "harness_version")
        .expect("D12 is asked of every connector");
    assert_eq!(
        gate.level,
        Level::Warn,
        "newer is not refused: {:?}",
        gate.detail
    );
    assert!(gate.detail.contains("0.160.2"), "{:?}", gate.detail);
    assert!(gate.detail.contains("untested"), "{:?}", gate.detail);
    assert!(
        gate.detail.contains("=0.153.0"),
        "the tested version is named: {:?}",
        gate.detail
    );
    assert!(
        r.harness_version
            .as_deref()
            .is_some_and(|v| v.contains("0.160.2")),
        "the measured version is carried to the daemon: {:?}",
        r.harness_version
    );
    assert!(
        !r.failed(),
        "so the report does not refuse the run: {:?}",
        r.checks
    );
}

/// D23's `required_paths`, as far as they can be known before a run: a literal path that is
/// not there, and a configuration directory this process cannot write — a machine that
/// cannot write there cannot hold an entry either.
#[test]
fn required_paths_refuses_a_missing_literal_and_a_configuration_directory_it_cannot_write() {
    let mut c = support::connector("codex-cli");
    let missing = "/goalcup-no-such-root/required-by-a-test";
    c.manifest.preflight.required_paths =
        vec![missing.to_string(), "{harness_home}/logs".to_string()];
    let gate = required_paths_gate(&c, &support::tmp_cfg());
    assert_eq!(gate.level, Level::Fail, "{:?}", gate.detail);
    assert!(
        gate.detail.contains(missing),
        "the path that is not there is named: {:?}",
        gate.detail
    );
    assert!(
        gate.detail.contains("{harness_home}/logs"),
        "and the templated one is printed as deferred rather than silently skipped: {:?}",
        gate.detail
    );
    assert!(
        !gate.fix.is_empty(),
        "and there is something to do about it"
    );

    // Nothing required of the machine, so the only thing left to fail on is where `goalcup`
    // keeps its own files.
    let mut nothing_required = support::connector("codex-cli");
    nothing_required.manifest.preflight.required_paths = Vec::new();
    let root = support::empty_cfg();
    let a_file = root.join("config-is-a-file");
    std::fs::write(&a_file, b"").expect("a file where a directory should be");
    let gate = required_paths_gate(&nothing_required, &a_file);
    assert_eq!(gate.level, Level::Fail, "{:?}", gate.detail);
    assert!(
        gate.detail.contains("not a directory"),
        "and says which way it is wrong: {:?}",
        gate.detail
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let read_only = root.join("read-only");
        std::fs::create_dir_all(&read_only).expect("a read-only configuration directory");
        std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o555))
            .expect("its mode");
        // Asserted only where the mode binds THIS process: root writes into 0o555 regardless,
        // and a failure for that reason would be a test about the account it ran under rather
        // than about the gate.
        if std::fs::File::create(read_only.join(".writable")).is_err() {
            let gate = required_paths_gate(&nothing_required, &read_only);
            assert_eq!(gate.level, Level::Fail, "{:?}", gate.detail);
            assert!(
                gate.detail.contains(&read_only.display().to_string()),
                "and names the directory: {:?}",
                gate.detail
            );
        }
    }
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
    let mut err = Vec::new();
    let code = run_with(
        None,
        &support::connectors_dir(),
        &cfg,
        &clean_env(),
        &opts(),
        &mut out,
        &mut err,
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
/// rather than report the file's existence as health.
#[test]
fn doctor_fails_when_the_control_file_names_a_port_nothing_is_listening_on() {
    let cfg = support::tmp_cfg();
    let port = a_closed_loopback_port();
    write_control_file(&cfg, goalcup_protocol::control::CONTROL_API_VERSION, port);
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)
        .expect("a client for a version this build speaks, whatever is on the far end");

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_with(
        Some(&client),
        &support::connectors_dir(),
        &cfg,
        &clean_env(),
        &opts(),
        &mut out,
        &mut err,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    let line = text
        .lines()
        .find(|l| l.contains("daemon") && l.contains("FAIL"))
        .unwrap_or_else(|| panic!("a FAIL line for the daemon check: {text}"));
    assert!(
        line.contains(&port.to_string()),
        "the port it could not reach: {line}"
    );
    assert!(
        text.contains("start `goalcup-daemon`"),
        "and the fix a participant acts on: {text}"
    );
    assert!(
        !text.contains("a-test-control-token"),
        "and the bearer is in no diagnostic: {text}"
    );
    assert_eq!(code, 1);
}
