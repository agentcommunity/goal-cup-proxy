//! Schema 1 owns capture behavior, and doctor describes a connector from its `harness.toml`
//! without spawning it.

use crate::common::support;

use goalcup_cli::commands::doctor::{DoctorOptions, run_with};
use goalcup_cli::commands::start::start_with;
use goalcup_protocol::pins::shipped_connector_ids;

fn opts(harness: Option<&str>) -> DoctorOptions {
    DoctorOptions {
        json: false,
        harness: harness.map(str::to_string),
    }
}

/// `start` of a keyed connector (the `keyed-per-run` fixture): a `--goal-file` start reaches the
/// launch's own checks. With no provider key in the environment it refuses, naming the variable
/// and never a value, before the daemon is asked for anything, so no capture workspace or
/// reserved chain is left behind.
#[test]
fn a_goal_file_start_of_a_keyed_connector_refuses_without_a_key_before_the_daemon() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_armed(&tmp, "keyed-per-run");
    let goal = tmp.path().join("goal.txt");
    std::fs::write(&goal, "write result.txt").unwrap();
    let mut o = support::start_options("keyed-per-run");
    o.goal_file = Some(goal);
    o.dev = true;
    let err = start_with(
        &m,
        &support::fixture_connectors_dir(),
        tmp.path(),
        &support::clean_env_with_no_harness_installed(),
        &o,
        &mut Vec::new(),
    )
    .expect_err("no provider key");
    assert_eq!(err.exit_code(), 1, "{err}");
    assert!(err.to_string().contains("DEEPSEEK_API_KEY"), "{err}");
    assert!(
        !err.to_string()
            .contains("cannot be launched for capture by this build yet"),
        "the family refusal is gone: {err}"
    );
    assert!(
        m.calls().is_empty(),
        "the daemon was asked: {:?}",
        m.calls()
    );
}

/// A hint no built-in decoder accepts is reported as capture-only, from `harness.toml` alone.
#[test]
fn doctor_lists_a_harness_only_unknown_hint_as_capture_only() {
    let tmp = tempfile::tempdir().unwrap();
    let only = support::copy_fixture_connector(
        "chat-shared-home",
        tmp.path(),
        "harness-only",
        "example-unknown",
    );
    let path = only.join("harness.toml");
    let hint = "protocol_hint = \"anthropic-messages\"";
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(source.contains(hint));
    std::fs::write(
        &path,
        source.replacen(hint, "protocol_hint = \"example-unknown\"", 1),
    )
    .unwrap();

    let cfg = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    run_with(
        None,
        tmp.path(),
        cfg.path(),
        &std::collections::BTreeMap::new(),
        &opts(None),
        &mut out,
    )
    .expect("doctor reports even with no daemon");
    let said = String::from_utf8(out).unwrap();
    assert!(said.contains("example-unknown"), "{said}");
    assert!(said.contains("no decoder accepts"), "{said}");
    assert!(said.contains("capture-only"), "{said}");
}

/// With no `--harness`, doctor checks the harness `goalcup prep` settled, so a harness the
/// participant never installed cannot fail the report; `--harness all` still checks them all.
#[test]
fn doctor_defaults_to_the_harness_in_prep_json_and_all_checks_every_one() {
    let ids = shipped_connector_ids();
    assert!(ids.len() >= 2, "two connectors to tell apart: {ids:?}");
    let (prepped, explicit) = (ids[ids.len() - 1].as_str(), ids[0].as_str());
    let cfg = tempfile::tempdir().unwrap();
    goalcup_cli::prep_file::write_prep(
        cfg.path(),
        &goalcup_cli::prep_file::PrepFile {
            harness: prepped.to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    let report = |harness: Option<&str>| {
        let mut out = Vec::new();
        run_with(
            None,
            &support::connectors_dir(),
            cfg.path(),
            &std::collections::BTreeMap::new(),
            &opts(harness),
            &mut out,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    };

    let said = report(None);
    assert!(
        said.contains(&format!("goalcup doctor — {prepped} (from goalcup prep)")),
        "{said}"
    );
    for id in ids.iter().filter(|id| *id != prepped) {
        assert!(!said.contains(id.as_str()), "{id}: {said}");
    }

    let said = report(Some("all"));
    for id in &ids {
        assert!(
            said.contains(id.as_str()),
            "`--harness all` reports {id}: {said}"
        );
    }
    assert!(!said.contains("(from goalcup prep)"), "{said}");

    let said = report(Some(explicit));
    assert!(
        said.contains(&format!("{explicit} only")),
        "an explicit id wins: {said}"
    );
    assert!(!said.contains(prepped), "{said}");
}

/// A `prep.json` that names a harness no connector here declares (stale after an upgrade) does
/// not make bare `goalcup doctor` refuse: it checks every connector, as before prep existed.
/// An explicit `--harness <id>` that nothing declares is still a refusal.
#[test]
fn doctor_falls_back_to_every_connector_when_prep_names_an_unknown_harness() {
    let cfg = tempfile::tempdir().unwrap();
    goalcup_cli::prep_file::write_prep(
        cfg.path(),
        &goalcup_cli::prep_file::PrepFile {
            harness: "no-such-harness".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    let run = |harness: Option<&str>, out: &mut Vec<u8>| {
        run_with(
            None,
            &support::connectors_dir(),
            cfg.path(),
            &std::collections::BTreeMap::new(),
            &opts(harness),
            out,
        )
    };
    let mut out = Vec::new();
    run(None, &mut out).expect("bare doctor still reports");
    let said = String::from_utf8(out).unwrap();
    assert!(said.starts_with("goalcup doctor\n"), "{said}");
    assert!(!said.contains("from goalcup prep"), "{said}");
    for id in shipped_connector_ids() {
        assert!(said.contains(&id), "every connector is reported: {said}");
    }
    let refused = run(Some("no-such-harness"), &mut Vec::new());
    assert!(refused.is_err(), "an explicit unknown id is refused");
}
