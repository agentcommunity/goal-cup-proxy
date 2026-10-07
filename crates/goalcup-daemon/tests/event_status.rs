//! The round's dates in the signed status document are ones somebody supplied.
//!
//! `EventStatus` is the one part of the document an agent may trust — it asks
//! the MCP server when the start window closes and is answered with a number signed by the
//! run key.
//!
//! Both readings are asserted here, and a reader can
//! tell them apart: a configured document says `event_status_source: "config"`, and one from
//! a machine that was told nothing still carries the entry's timeline and says `"fallback"`.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_daemon::config::EventConfig;
use goalcup_protocol::control::{EventStatusSource, ROUTE_WORKSPACES, SignedStatus, WorkspaceInfo};

/// The round these tests configure. Well away from any clock a test runs on, so a field
/// that came from the entry rather than from the configuration is visibly different rather
/// than accidentally equal.
const T0: &str = "2026-10-01T00:00:00Z";
const CLOSES: &str = "2026-10-08T00:00:00Z";
const RULES: &str = "rules-2026-10-01";

fn round() -> EventConfig {
    EventConfig {
        t0: T0.to_string(),
        start_window_closes_at: CLOSES.to_string(),
        rules_version: RULES.to_string(),
    }
}

/// The document as a reader gets it: off the cache file, signature checked against the key
/// the CLI was handed, never against the `public_key` label on the document itself.
fn signed(config_dir: &std::path::Path, info: &WorkspaceInfo) -> SignedStatus {
    let path = config_dir
        .join("cache")
        .join(format!("status-{}.json", info.entry_id));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let signed: SignedStatus = serde_json::from_str(&text).expect("the cache holds a document");
    assert!(
        signed.verify_with(&info.run_public_key).is_ok(),
        "the status document verifies under the run key the CLI pinned"
    );
    signed
}

#[tokio::test]
async fn a_configured_round_reaches_the_signed_status_document() {
    let h = support::control::empty_control_harness_with(|c| c.event = Some(round())).await;
    let info: WorkspaceInfo = h
        .post_json(
            ROUTE_WORKSPACES,
            &support::control::workspace_opts("claude-code-cli"),
        )
        .await;

    let payload = signed(&h.config_dir, &info).payload;
    assert_eq!(payload.event.t0, T0);
    assert_eq!(payload.event.start_window_closes_at, CLOSES);
    assert_eq!(payload.event.rules_version, RULES);
    assert_eq!(payload.event_status_source, EventStatusSource::Config);
    // None of the three is the entry's own row, which is what the fallback puts there.
    assert_ne!(payload.event.t0, info.created_at);
    assert_ne!(payload.event.rules_version, goalcup_daemon::DAEMON_VERSION);

    // `publication_closes_at` is deliberately not configurable: it is the deadline the start
    // receipt fixes, and the receipt is immutable. Before a receipt there is none, so the
    // entry's creation stands in — the same value the fallback carries, and the reason
    // `event_status_source` is decided on the other three fields alone.
    assert_eq!(payload.event.publication_closes_at, info.created_at);
}

#[tokio::test]
async fn a_machine_told_no_round_says_so_rather_than_inventing_one() {
    let h = support::control::empty_control_harness().await;
    let info: WorkspaceInfo = h
        .post_json(
            ROUTE_WORKSPACES,
            &support::control::workspace_opts("claude-code-cli"),
        )
        .await;

    let payload = signed(&h.config_dir, &info).payload;
    assert_eq!(payload.event_status_source, EventStatusSource::Fallback);
    // The fallback: the entry's own timeline.
    assert_eq!(payload.event.t0, info.created_at);
    assert_eq!(payload.event.start_window_closes_at, info.created_at);
    assert_eq!(payload.event.publication_closes_at, info.created_at);
    assert_eq!(payload.event.rules_version, goalcup_daemon::DAEMON_VERSION);
}

/// `Daemon::rebuild`, the rehydration path, sets the configured round too.
///
/// `control::build_entry` calls `set_event_status` unconditionally on a fresh entry, and
/// `rebuild` does too: a restart must not change what a signed document says the round's
/// dates are.
#[tokio::test]
async fn a_restart_does_not_drop_the_round_the_machine_was_told() {
    let h = support::control::empty_control_harness_with(|c| c.event = Some(round())).await;
    let info: WorkspaceInfo = h
        .post_json(
            ROUTE_WORKSPACES,
            &support::control::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_eq!(
        signed(&h.config_dir, &info).payload.event_status_source,
        EventStatusSource::Config,
        "the case only exists once the first daemon has a configured round to lose"
    );

    // The same tree, a second daemon. Every open row comes back through `rebuild`.
    h.daemon.stop().await;
    let second = goalcup_daemon::runtime::Daemon::start(h.config.clone())
        .await
        .expect("a second daemon starts over the same tree");
    second
        .refresh_statuses()
        .expect("the rehydrated entry publishes");

    let payload = signed(&h.config_dir, &info).payload;
    assert_eq!(
        payload.event_status_source,
        EventStatusSource::Config,
        "the rehydrated entry still carries the round this machine was told about"
    );
    assert_eq!(payload.event.t0, T0);
    assert_eq!(payload.event.start_window_closes_at, CLOSES);
    assert_eq!(payload.event.rules_version, RULES);
    second.stop().await;
}

/// The round must survive the seal. `compose_capture_closed` is a second payload builder for
/// capture-closed rows, and it carries the configured round, not the fallback, from the
/// seal on and across every daemon start after it.
#[tokio::test]
async fn a_sealed_entry_keeps_the_round_the_machine_was_told() {
    let h = support::control::empty_control_harness_with(|c| c.event = Some(round())).await;
    let info: WorkspaceInfo = h
        .post_json(
            ROUTE_WORKSPACES,
            &support::control::workspace_opts("claude-code-cli"),
        )
        .await;

    assert_eq!(
        signed(&h.config_dir, &info).payload.event_status_source,
        EventStatusSource::Config,
        "the case only exists once the live document carries a configured round"
    );

    h.post_json::<_, goalcup_protocol::seal::SealManifest>(
        &goalcup_protocol::control::entry_route(
            &info.entry_id,
            goalcup_protocol::control::SUFFIX_SEAL,
        ),
        &(),
    )
    .await;

    // The restart is what routes the row through `compose_capture_closed`: `Daemon::start`
    // republishes and re-signs every capture-closed row it finds.
    h.daemon.stop().await;
    let second = goalcup_daemon::runtime::Daemon::start(h.config.clone())
        .await
        .expect("a second daemon starts over the same tree");
    second
        .refresh_statuses()
        .expect("the sealed row republishes");

    let payload = signed(&h.config_dir, &info).payload;
    assert_eq!(
        payload.event_status_source,
        EventStatusSource::Config,
        "a sealed row still states the round this machine was told about"
    );
    assert_eq!(payload.event.t0, T0);
    assert_eq!(payload.event.start_window_closes_at, CLOSES);
    assert_eq!(payload.event.rules_version, RULES);
    second.stop().await;
}

/// The property that justifies holding the round on the row: a sealed document states the
/// round the run was created under, not whatever `[event]` the next daemon happens to read.
/// A fix that threaded the live config into the startup republication would pass the test
/// above and fail this one — the second daemon here was told no round at all.
#[tokio::test]
async fn a_sealed_document_does_not_change_when_the_round_is_removed() {
    let h = support::control::empty_control_harness_with(|c| c.event = Some(round())).await;
    let info: WorkspaceInfo = h
        .post_json(
            ROUTE_WORKSPACES,
            &support::control::workspace_opts("claude-code-cli"),
        )
        .await;

    h.post_json::<_, goalcup_protocol::seal::SealManifest>(
        &goalcup_protocol::control::entry_route(
            &info.entry_id,
            goalcup_protocol::control::SUFFIX_SEAL,
        ),
        &(),
    )
    .await;

    // The seal-time publish, before any restart: the first composer to see the row closed.
    let sealed = signed(&h.config_dir, &info).payload;
    assert_eq!(
        sealed.event_status_source,
        EventStatusSource::Config,
        "the document the seal publishes still states the configured round"
    );
    assert_eq!(sealed.event.t0, T0);
    assert_eq!(sealed.event.start_window_closes_at, CLOSES);
    assert_eq!(sealed.event.rules_version, RULES);

    h.daemon.stop().await;
    let mut untold = h.config.clone();
    untold.event = None;
    let second = goalcup_daemon::runtime::Daemon::start(untold)
        .await
        .expect("a second daemon starts over the same tree with no [event] section");
    second
        .refresh_statuses()
        .expect("the sealed row republishes");

    let payload = signed(&h.config_dir, &info).payload;
    assert_eq!(
        payload.event_status_source,
        EventStatusSource::Config,
        "a sealed row states the round it was created under, not the next daemon's config"
    );
    assert_eq!(payload.event.t0, T0);
    assert_eq!(payload.event.start_window_closes_at, CLOSES);
    assert_eq!(payload.event.rules_version, RULES);
    second.stop().await;
}
