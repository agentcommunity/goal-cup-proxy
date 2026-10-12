//! `goalcup seal` with no capture status: the daemon is asked once, a `busy` is ridden out,
//! a torn entry is refused with what happened, and nothing else this command could print.
//! The capture finish itself is driven against the real daemon in `m2_cli_lifecycle`.

use crate::common::support;

use goalcup_cli::commands::seal::seal_with;
use goalcup_protocol::control::ControlError;

/// The dev-dependency above is a TEST dependency, and the binary a participant installs
/// links none of it.
///
/// Two assertions, because either one alone is weak. `src/` naming the daemon at all is the
/// thing that would put it in the shipped graph, and the manifest placing it under
/// `[dev-dependencies]` is what keeps `cargo build --offline` — run with no features and no
/// dev-dependencies — from building it.
#[test]
fn the_shipped_binary_links_no_daemon() {
    for entry in support::walk("crates/goalcup-cli/src") {
        let src = std::fs::read_to_string(&entry).unwrap();
        // Comments name it all over this crate — the seal's whole job is to mirror
        // `goalcup_daemon::workspace`, and saying so is the point. CODE naming it is what
        // would put it in the shipped graph, so the comment lines are dropped first.
        for line in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
            assert!(
                !line.contains("goalcup_daemon"),
                "{entry:?} names the daemon crate in code: {line}"
            );
        }
    }
    let toml = std::fs::read_to_string("Cargo.toml").unwrap();
    let dev = toml.find("[dev-dependencies]").expect("a dev section");
    let named = toml
        .find("\ngoalcup-daemon = ")
        .expect("the dev-dependency itself");
    assert!(
        named > dev,
        "goalcup-daemon is declared before [dev-dependencies], which ships it"
    );
}

/// `busy` is the daemon holding the entry for one asynchronous call. The daemon never answers
/// the bare seal `busy` (the route reads only the registry row), so the bare seal rides the
/// generic retry like any other ordinary call.
///
/// Both ends of the rule are here: one `busy` is absorbed and the seal is asked again (the
/// mock's second answer is a pre-v5 seal report, which this command refuses at rung 3), and a
/// daemon that is STILL busy when the retry is spent is rung 1 with the daemon's own words
/// in it.
#[test]
fn one_busy_seal_is_asked_again_and_a_standing_busy_is_rung_one_after_the_bounded_asks() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());

    m.busy_once("seal");
    let err = seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(
        m.calls().iter().filter(|c| *c == "seal").count(),
        2,
        "the transient was ridden out rather than reported"
    );
    // The second ask is answered with the mock's pre-v5 seal report, which no daemon of this
    // build sends to a bare seal and which this command refuses rather than prints.
    assert!(
        matches!(
            err,
            goalcup_cli::CliError::Control(ControlError::Malformed(_))
        ),
        "{err:?}"
    );
    assert_eq!(err.exit_code(), 3);

    let m = support::mock_sealable(&tmp, "claude-code-cli");
    m.fail_always(
        "seal",
        ControlError::Busy("the pump is mid-cycle on this entry".to_string()),
    );
    let err = seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "actionable: wait a moment and run it again"
    );
    let said = err.to_string();
    assert!(
        said.contains("the pump is mid-cycle on this entry"),
        "the daemon's own words, not a sentence the CLI invented: {said}"
    );
    assert!(
        said.contains("no seal manifest was returned"),
        "state only what this command observed: {said}"
    );
    assert!(
        said.contains("may still be completing a seal"),
        "a Busy response does not prove that another asynchronous seal did not commit: {said}"
    );
    assert_eq!(
        m.calls().iter().filter(|c| *c == "seal").count(),
        goalcup_cli::retry::BUSY_ATTEMPTS,
        "the real command reaches every bounded seal ask"
    );
}

/// H-15: `goalcup seal` on an entry whose trace is torn. The tracker holds the row as a capture
/// but has no status and no open capture, and answers the CLI's legacy seal `malformed`. That was
/// reported as "malformed control response" at rung 3; it is a safe refusal at rung 1 that says
/// what happened, that nothing was changed, and what to do next.
#[test]
fn a_torn_entry_is_refused_with_what_happened_and_nothing_is_sealed() {
    use goalcup_cli::CliError;

    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    m.fail_always(
        "status",
        ControlError::NotFound("no status for that entry".into()),
    );
    m.fail_always(
        "seal",
        ControlError::Malformed("a v5 capture entry is sealed with a capture seal request".into()),
    );
    let before = support::artifact_snapshot(std::path::Path::new(&m.info().artifact_root));

    let mut out = Vec::new();
    let error = seal_with(&m, &support::connectors_dir(), &cfg, &entry, &mut out)
        .expect_err("a torn entry cannot be sealed");

    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    assert_eq!(
        error.exit_code(),
        1,
        "a refusal, not a CLI that could not do its job"
    );
    let said = error.to_string();
    assert!(
        said.contains(&format!("Entry {entry} cannot be sealed")),
        "{said}"
    );
    assert!(said.contains("torn or unreadable"), "{said}");
    assert!(
        said.contains("Nothing was sealed, written or repaired"),
        "{said}"
    );
    assert!(said.contains("preserved"), "{said}");
    assert!(
        said.contains("goalcup-verify --capture-integrity <the entry's trace directory>"),
        "the one command to read what survives: {said}"
    );
    assert!(said.contains(&format!("a folder named {entry}")), "{said}");
    assert!(said.contains("start a new run"), "{said}");
    assert!(
        !said.contains("malformed control response"),
        "the misleading wording is gone: {said}"
    );
    assert_eq!(
        support::artifact_snapshot(std::path::Path::new(&m.info().artifact_root)),
        before,
        "nothing under the artifact root was touched"
    );

    // A `malformed` the status read gives no reason to read as a torn entry stays what it was.
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    m.fail_always("seal", ControlError::Malformed("something else".into()));
    let error = seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        &mut Vec::new(),
    )
    .expect_err("malformed");
    assert!(
        matches!(error, CliError::Control(ControlError::Malformed(_))),
        "{error:?}"
    );
    assert_eq!(error.exit_code(), 3);
}
