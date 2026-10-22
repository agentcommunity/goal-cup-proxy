//! The signed status document (D25).
//!
//! One signed `control::SignedStatus`, verified with the run public key the caller pinned and
//! never with the key the document names, and carrying no secret at all: the status route is
//! unauthenticated, so a token on it would be a token on a route with no door. Every document
//! is a v5 local capture's, so `detail` is the capture's sentence.

// Each test binary uses a different subset of the shared support.
#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;
use common::support;

use goalcup_daemon::registry::EntryState;
use goalcup_daemon::status::{build_payload, sign_payload};
use goalcup_protocol::capture_status::{CaptureState, RunState};
use goalcup_protocol::control::{ArmingState, SignedStatus};
use goalcup_protocol::events::HealthState;

#[test]
fn the_signed_status_verifies_against_the_run_key_and_carries_no_secret() {
    let e = support::started_entry();
    let key = e.keys.signing_key().unwrap();
    let signed = sign_payload(
        build_payload(&support::status_inputs(&e)),
        "2026-09-02T00:00:00Z".to_string(),
        &key,
    )
    .unwrap();
    assert!(
        signed
            .verify_with(&e.keys.public_key_hex().unwrap())
            .is_ok()
    );
    assert!(
        signed.verify_with(&"aa".repeat(32)).is_err(),
        "protocol C1: a document that names the key that checks it checks nothing"
    );

    let json = serde_json::to_string(&signed).unwrap();
    assert!(
        !json.contains(e.keys.signing_seed_hex.as_str()),
        "the status route is unauthenticated: no secrets on it"
    );
}

/// A sealed row reports `sealed`, read off the row and not off the health the caller passed.
#[test]
fn a_sealed_entry_reports_sealed() {
    let mut sealed = support::started_entry();
    sealed.state = EntryState::Sealed;
    sealed.sealed_at = Some("2026-09-03T09:12:44Z".to_string());
    let payload = build_payload(&support::status_inputs(&sealed));
    assert_eq!(payload.health, HealthState::Sealed);
    assert_eq!(payload.sealed_at, sealed.sealed_at);
    assert_eq!(payload.seconds_remaining, None);
}

#[test]
fn an_unreconciled_sealing_row_is_not_reported_as_live_or_sealed() {
    let mut sealing = support::started_entry();
    sealing.state = EntryState::Sealing;
    sealing.sealed_at = None;
    let payload = build_payload(&support::status_inputs(&sealing));
    assert_eq!(payload.health, HealthState::SealingRecoveryNeeded);
    assert_eq!(payload.sealed_at, None);
}

/// A version nothing measured is absent from the document, never an empty
/// string sitting inside the `Some`.
#[test]
fn an_unprobed_harness_version_is_absent_from_the_document_rather_than_empty() {
    let probed = support::started_entry();
    assert_eq!(
        build_payload(&support::status_inputs(&probed))
            .harness_version
            .as_deref(),
        Some("2.1.259"),
        "a measured version still travels"
    );

    let mut unprobed = support::started_entry();
    unprobed.harness_version = String::new();
    assert_eq!(
        build_payload(&support::status_inputs(&unprobed)).harness_version,
        None,
        "nothing measured it, and the document says so rather than saying an empty string"
    );
}

/// `detail` is the capture's sentence, the capture projection travels, and the fields the
/// wire shape keeps for uploads and deadlines are empty: a local capture has neither.
#[test]
fn the_payload_is_the_capture_projection() {
    let e = support::started_entry();
    let mut inputs = support::status_inputs(&e);
    inputs.arming = ArmingState::Armed;
    let payload = build_payload(&inputs);
    assert_eq!(payload.arming, ArmingState::Armed);
    assert_eq!(payload.health, HealthState::Online);
    assert_eq!(payload.capture, Some(support::recording_capture()));
    assert!(
        payload.detail.starts_with("Local capture is recording"),
        "{}",
        payload.detail
    );
    assert_eq!(payload.seconds_remaining, None);
    assert_eq!(payload.started_at, None);
    assert_eq!(payload.deadline_at, None);
    assert_eq!(payload.events_uploaded, 0);
    assert_eq!(payload.events_pending, 0);

    // A failure is named before anything else.
    inputs.capture.run = RunState::Sealed;
    inputs.capture.capture = CaptureState::Failed;
    assert!(
        build_payload(&inputs)
            .detail
            .starts_with("Local capture failed")
    );
}

/// The document survives the trip a reader makes (JSON out of the daemon, back into a
/// `SignedStatus`) and still verifies against the pinned run key.
#[test]
fn the_signed_document_round_trips() {
    let e = support::started_entry();
    let key = e.keys.signing_key().unwrap();
    let public = e.keys.public_key_hex().unwrap();
    let signed = sign_payload(
        build_payload(&support::status_inputs(&e)),
        "2026-09-04T09:00:00Z".to_string(),
        &key,
    )
    .unwrap();
    let back: SignedStatus =
        serde_json::from_str(&serde_json::to_string_pretty(&signed).unwrap()).unwrap();
    back.verify_with(&public).unwrap();
    assert_eq!(back, signed);
}

#[test]
fn the_summary_is_the_entry_a_reader_sees_in_a_list() {
    let e = support::started_entry();
    let row = goalcup_daemon::status::summary(&e, ArmingState::Started, HealthState::Online);
    assert_eq!(row.entry_id, "entry-0001");
    assert_eq!(row.arming, ArmingState::Started);
    assert_eq!(row.health, HealthState::Online);
    assert_eq!(row.started_at, None);
    assert_eq!(row.sealed_at, None);
}
