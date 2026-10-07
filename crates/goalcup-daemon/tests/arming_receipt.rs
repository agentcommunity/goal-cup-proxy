//! What it takes to start a run's clock, and what it takes to stop one starting.
//!
//! Three claims, and none of them is a rule the daemon interprets:
//!
//! - Nothing but a transcript record starts a run (D16). There is no hook leg,
//!   so the first leg-1 record is the start signal and its corroboration; a gate that has
//!   seen no record cannot stamp a start, whatever a server signed.
//! - A harness home that already holds a tracked session is not armed: the daemon
//!   can read the occupant's own `session_meta` before it arms, so it does.
//! - A receipt is what the server signed. `accept_receipt` checks the
//!   signature against the key this build pins and then checks every claim against what
//!   the daemon asserted, so a server that quietly moves `deadline_at` is caught at the
//!   moment of issue rather than by the verifier at the end of the week.
#![cfg(feature = "v1-compat")]

mod support;

use ed25519_dalek::Signer;
use goalcup_daemon::arming::{ArmingGate, GateState};
use goalcup_daemon::receipt::accept_receipt;
use goalcup_daemon::registry::StartPathIdentity;
use goalcup_protocol::events::{ForeignScanOutcome, ForeignScanUnavailableReason};

fn trusted_plan(
    active: &goalcup_protocol::manifest::ConnectorManifest,
) -> goalcup_daemon::foreign::ForeignScanPlan {
    let connectors = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    goalcup_daemon::foreign::TrustedManifests::load(&connectors)
        .expect("the shipped scan union is trusted")
        .plan_for(active)
}

fn coverage(
    result: &goalcup_daemon::foreign::ForeignScanResult,
    connector: &str,
    scan_index: u64,
) -> ForeignScanOutcome {
    result
        .coverage
        .iter()
        .find(|coverage| {
            coverage.declaring_connector == connector && coverage.scan_index == scan_index
        })
        .unwrap_or_else(|| panic!("missing {connector} foreign_scan[{scan_index}] coverage"))
        .outcome
        .clone()
}

#[test]
fn arming_a_harness_home_that_already_holds_a_tracked_session_is_refused() {
    // Splitting a Codex run across two homes costs only a Flag at the verifier; the daemon can
    // see the occupant before it arms, so it does.
    let t = tempfile::tempdir().expect("a temporary directory");
    let ws = t.path().join("ws");
    let (_dir, registry, entry) = support::registry_with_entry("e1", &ws.display().to_string());
    let home = t.path().join("home/codex");
    support::plant_rollout(&home, &entry.workspace, "thread-existing");

    let vars = support::path_vars(&entry.workspace, &home);
    let e = goalcup_daemon::foreign::pre_arm_scan(
        &support::manifest("codex-cli"),
        &vars,
        &registry,
        &home,
        "candidate",
    )
    .expect_err("an occupied home is refused")
    .to_string();
    assert!(
        e.contains("thread-existing") && e.contains(&home.display().to_string()),
        "{e}"
    );

    let clean = t.path().join("home/fresh");
    std::fs::create_dir_all(&clean).expect("the clean home");
    goalcup_daemon::foreign::pre_arm_scan(
        &support::manifest("codex-cli"),
        &support::path_vars(&entry.workspace, &clean),
        &registry,
        &clean,
        "candidate",
    )
    .expect("a home holding nothing arms");

    // And a home whose occupant is working somewhere this daemon does not track is not a
    // reason to refuse: the refusal is about a run split across two homes, not about the
    // participant having used the harness before.
    let elsewhere = t.path().join("home/elsewhere");
    support::plant_rollout(
        &elsewhere,
        &t.path().join("some-other-project"),
        "thread-other",
    );
    goalcup_daemon::foreign::pre_arm_scan(
        &support::manifest("codex-cli"),
        &support::path_vars(&entry.workspace, &elsewhere),
        &registry,
        &elsewhere,
        "candidate",
    )
    .expect("a session in an untracked directory is not this daemon's business");
}

/// The other half: an entry the registry already placed in this home occupies
/// it, whatever is or is not on disk.
#[test]
fn a_home_another_entry_is_already_placed_in_is_refused_before_a_byte_is_read() {
    let t = tempfile::tempdir().expect("a temporary directory");
    let home = t.path().join("home/codex");
    std::fs::create_dir_all(&home).expect("the home");
    let (_dir, registry, entry) =
        support::registry_with_entry("e1", &t.path().join("ws").display().to_string());
    registry
        .update(&entry.entry_id, |e| e.harness_home = Some(home.clone()))
        .expect("the entry is placed");

    let e = goalcup_daemon::foreign::pre_arm_scan(
        &support::manifest("codex-cli"),
        &support::path_vars(&entry.workspace, &home),
        &registry,
        &home,
        "candidate",
    )
    .expect_err("an occupied home is refused")
    .to_string();
    assert!(
        e.contains("e1") && e.contains(&home.display().to_string()),
        "{e}"
    );
}

#[test]
fn a_scan_publishes_identifiers_and_paths_and_no_byte_of_anyone_elses_transcript() {
    let t = tempfile::tempdir().expect("a temporary directory");
    let ws = t.path().join("ws");
    let secret = "a prompt nobody outside that session agreed to hand over";
    support::plant_claude_slug(t.path(), &ws, "sess-9", secret);

    let manifest = support::manifest("claude-code-cli");
    let s = goalcup_daemon::foreign::scan_once(
        &trusted_plan(&manifest),
        &support::path_vars(&ws, t.path()),
        std::slice::from_ref(&ws),
    )
    .expect("the scan reads what is there");
    assert_eq!(s.sightings.len(), 1);
    assert_eq!(s.sightings[0].session_id, "sess-9");
    let event = serde_json::to_string(&s.sightings[0].to_event()).expect("the event serialises");
    assert!(!event.contains(secret), "D20: no content, ever");
    assert!(
        !event.contains(&s.sightings[0].directory),
        "the matched directory is not published"
    );
    assert!(event.contains("sess-9") && event.contains("foreign_session"));

    // A session working somewhere the daemon tracks nothing is not a sighting.
    let elsewhere = goalcup_daemon::foreign::scan_once(
        &trusted_plan(&manifest),
        &support::path_vars(&ws, t.path()),
        &[t.path().join("somewhere-else")],
    )
    .expect("the scan reads what is there");
    assert!(elsewhere.sightings.is_empty());
}

#[tokio::test]
async fn the_receipt_is_verified_against_the_pinned_key_and_against_the_daemons_own_claims() {
    let (server, url) = support::devserver().await;
    let claims = support::claims("e1", &support::pinned_start(), &support::pinned_deadline());
    let pinned = goalcup_devserver::dev_server_public_key_hex();

    let r = goalcup_daemon::receipt::ReceiptClient::new(url.clone())
        .request(&claims, &support::run_public_key())
        .await
        .expect("the dev server signs");
    accept_receipt(&r, &claims, &pinned).expect("its own claims, its own key");

    // A server that quietly extends the deadline is caught here, not a week later.
    let mut moved = claims.clone();
    moved.deadline_at = "2036-10-01T00:00:00Z".to_string();
    let e = accept_receipt(&r, &moved, &pinned)
        .expect_err("a moved deadline is refused")
        .to_string();
    assert!(e.contains("deadline_at"), "{e}");
    // And a receipt signed by anyone else is refused before the entry is touched.
    let self_signed = goalcup_protocol::receipt::StartReceipt::sign(
        claims.clone(),
        &ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]),
    )
    .expect("a receipt signs");
    assert!(accept_receipt(&self_signed, &claims, &pinned).is_err());
    // A signature that is the right shape under the right key and still does not stand.
    let mut tampered = r.clone();
    tampered.signature = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
            .sign(b"something else")
            .to_bytes(),
    );
    assert!(accept_receipt(&tampered, &claims, &pinned).is_err());

    // Existing default callers retain the one-shot 409 behavior. The configurable
    // qualification-window constructor has the idempotent retry seam tested in dev_key.rs.
    let again = goalcup_daemon::receipt::ReceiptClient::new(url)
        .request(&claims, &support::run_public_key())
        .await
        .expect_err("a default-mode re-issue is refused");
    assert!(again.to_string().contains("409"), "{again}");
    let _ = server;
}

#[test]
fn a_valid_server_signature_cannot_replace_the_expected_receipt_bindings() {
    let pinned = goalcup_devserver::dev_server_public_key_hex();
    let mut expected = support::claims("e1", &support::pinned_start(), &support::pinned_deadline());
    expected.run_public_key = Some(hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
            .verifying_key()
            .to_bytes(),
    ));
    expected.questionnaire_version = Some("2026.09-v1".to_string());

    let mut substituted = expected.clone();
    substituted.run_public_key = Some(hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[10u8; 32])
            .verifying_key()
            .to_bytes(),
    ));
    substituted.questionnaire_version = Some("2026.09-v2".to_string());
    let receipt = goalcup_protocol::receipt::StartReceipt::sign(
        substituted,
        &goalcup_devserver::dev_server_signing_key(),
    )
    .expect("the pinned server can sign the substituted claims");

    accept_receipt(&receipt, &expected, &pinned)
        .expect_err("a valid signature over different binding claims must be refused");

    let mut uppercase = expected.clone();
    uppercase.run_public_key = expected
        .run_public_key
        .as_ref()
        .map(|key| key.to_uppercase());
    let receipt = goalcup_protocol::receipt::StartReceipt::sign(
        uppercase,
        &goalcup_devserver::dev_server_signing_key(),
    )
    .expect("the pinned server signs the equivalent spelling");
    accept_receipt(&receipt, &expected, &pinned)
        .expect("hex case does not change which run key the receipt names");

    let mut half = expected.clone();
    half.questionnaire_version = None;
    let receipt = goalcup_protocol::receipt::StartReceipt::sign(
        half,
        &goalcup_devserver::dev_server_signing_key(),
    )
    .unwrap();
    accept_receipt(&receipt, &expected, &pinned)
        .expect_err("a valid signature over half a binding is refused");

    let mut malformed = expected.clone();
    malformed.run_public_key = Some("not-a-key".to_string());
    let receipt = goalcup_protocol::receipt::StartReceipt::sign(
        malformed,
        &goalcup_devserver::dev_server_signing_key(),
    )
    .unwrap();
    accept_receipt(&receipt, &expected, &pinned)
        .expect_err("a valid signature cannot make a malformed run key usable");
}

#[test]
fn a_signed_receipt_rejects_a_deadline_that_is_not_strictly_after_its_start() {
    let pinned = goalcup_devserver::dev_server_public_key_hex();
    let mut claims = support::claims("e1", "2026-09-01T00:00:02Z", "2026-09-01T00:00:02Z");
    let receipt = goalcup_protocol::receipt::StartReceipt::sign(
        claims.clone(),
        &goalcup_devserver::dev_server_signing_key(),
    )
    .expect("the development signer signs the malformed ordering");
    let error = accept_receipt(&receipt, &claims, &pinned)
        .expect_err("a signed deadline at the start is not an admissible interval")
        .to_string();
    assert!(error.contains("deadline_at"), "{error}");
    assert!(error.contains("after"), "{error}");

    claims.deadline_at = "2026-09-01T00:00:01Z".to_string();
    let receipt = goalcup_protocol::receipt::StartReceipt::sign(
        claims.clone(),
        &goalcup_devserver::dev_server_signing_key(),
    )
    .expect("the development signer signs the reversed ordering");
    let error = accept_receipt(&receipt, &claims, &pinned)
        .expect_err("a signed deadline before the start is not admissible")
        .to_string();
    assert!(error.contains("deadline_at"), "{error}");
    assert!(error.contains("after"), "{error}");
}

#[tokio::test]
async fn signed_short_deadline_is_authoritative_and_participant_deadline_input_is_ignored() {
    let server = std::sync::Arc::new(
        goalcup_devserver::DevServer::with_qualification_window(2)
            .expect("the positive qualification window builds"),
    );
    server.set_clock("2026-09-01T00:00:00Z");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the loopback listener binds");
    let address = listener.local_addr().expect("the listener address reads");
    let routed = server.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, goalcup_devserver::dev_router(routed)).await;
    });

    for (entry_id, participant_started_at, participant_deadline_at) in [
        (
            "short-deadline-shorten",
            "2026-09-01T00:00:00Z",
            "2026-09-01T00:00:01Z",
        ),
        (
            "short-deadline-extend",
            "1999-01-01T00:00:00Z",
            "2036-10-01T00:00:00Z",
        ),
    ] {
        let claims = support::claims(entry_id, participant_started_at, participant_deadline_at);
        let receipt = goalcup_daemon::receipt::ReceiptClient::new(format!("http://{address}"))
            .request(&claims, &support::run_public_key())
            .await
            .expect("the development server signs the short receipt");

        assert_eq!(receipt.claims.started_at, "2026-09-01T00:00:00Z");
        assert_eq!(receipt.claims.deadline_at, "2026-09-01T00:00:02Z");
        assert_ne!(
            receipt.claims.deadline_at,
            goalcup_protocol::control::format_z(
                goalcup_protocol::control::parse_z(&receipt.claims.started_at)
                    .expect("the signed start parses")
                    .saturating_add(goalcup_daemon::receipt::ENTRY_WINDOW_SECONDS)
            )
        );

        let participant_error = accept_receipt(
            &receipt,
            &claims,
            &goalcup_devserver::dev_server_public_key_hex(),
        )
        .expect_err("participant timing input cannot become the daemon's authority")
        .to_string();
        if entry_id == "short-deadline-shorten" {
            assert!(
                participant_error.contains("deadline_at"),
                "the refusal names the participant-controlled deadline mismatch: {participant_error}"
            );
            assert!(
                !participant_error.contains("started_at"),
                "the shorten case keeps the participant start equal to the signed start: {participant_error}"
            );
        } else {
            assert!(
                participant_error.contains("started_at"),
                "the extension fixture's first mismatch is its participant start: {participant_error}"
            );
        }

        let mut daemon_claims = claims;
        daemon_claims.started_at = receipt.claims.started_at.clone();
        daemon_claims.deadline_at = receipt.claims.deadline_at.clone();
        accept_receipt(
            &receipt,
            &daemon_claims,
            &goalcup_devserver::dev_server_public_key_hex(),
        )
        .expect("the daemon accepts the verified organizer-signed deadline");
    }
}

#[tokio::test]
async fn record_start_stores_the_organizer_signed_deadline_for_participant_timing_attempts() {
    let server = std::sync::Arc::new(
        goalcup_devserver::DevServer::with_qualification_window(2)
            .expect("the positive qualification window builds"),
    );
    server.set_clock("2026-09-01T00:00:00Z");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the loopback listener binds");
    let address = listener.local_addr().expect("the listener address reads");
    let routed = server.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, goalcup_devserver::dev_router(routed)).await;
    });

    let signed_started_at = "2026-09-01T00:00:00Z";
    let signed_deadline_at = "2026-09-01T00:00:02Z";
    let signed_deadline_unix = goalcup_protocol::control::parse_z(signed_deadline_at)
        .expect("the organizer-signed deadline parses") as f64;
    let legacy_deadline_unix = (goalcup_protocol::control::parse_z(signed_started_at)
        .expect("the organizer-signed start parses")
        + goalcup_daemon::receipt::ENTRY_WINDOW_SECONDS) as f64;

    for (entry_id, participant_started_at, participant_deadline_at) in [
        (
            "record-start-shorten",
            signed_started_at,
            "2026-09-01T00:00:01Z",
        ),
        (
            "record-start-extend",
            "1999-01-01T00:00:00Z",
            "2036-10-01T00:00:00Z",
        ),
    ] {
        let workspace = format!("/ws/{entry_id}");
        let (_dir, registry, entry) = support::registry_with_entry(entry_id, &workspace);
        let expected_paths = StartPathIdentity::from_entry(&entry);
        arm(&registry, entry_id);
        let mut claims = support::claims(entry_id, participant_started_at, participant_deadline_at);
        claims.connector_id = entry.connector_id.clone();
        claims.connector_dir_sha256 = entry.connector_dir_sha256.clone();
        let receipt = goalcup_daemon::receipt::ReceiptClient::new(format!("http://{address}"))
            .request(&claims, &support::run_public_key())
            .await
            .expect("the development server signs the organizer deadline");

        let manifest = support::manifest("codex-cli");
        let mut gate = ArmingGate::new(&manifest, 0.0);
        gate.observe_leg1("rollout", 12, &claims.session_id, 2.0);
        let stored = goalcup_daemon::receipt::record_start(
            &registry,
            entry_id,
            &mut gate,
            &receipt,
            &claims,
            &expected_paths,
            &goalcup_devserver::dev_server_public_key_hex(),
        )
        .expect("record_start stores the organizer-signed deadline");

        assert_eq!(stored.receipt(), Some(&receipt));
        assert_eq!(stored.started_at.as_deref(), Some(signed_started_at));
        assert_eq!(stored.deadline_at.as_deref(), Some(signed_deadline_at));
        assert_eq!(stored.deadline_unix, Some(signed_deadline_unix));
        assert_ne!(stored.deadline_unix, Some(legacy_deadline_unix));

        let reloaded = registry
            .get(entry_id)
            .expect("the stored entry reads")
            .expect("the stored entry remains present");
        assert_eq!(reloaded.receipt(), Some(&receipt));
        assert_eq!(reloaded.started_at.as_deref(), Some(signed_started_at));
        assert_eq!(reloaded.deadline_at.as_deref(), Some(signed_deadline_at));
        assert_eq!(reloaded.deadline_unix, Some(signed_deadline_unix));
    }
}

/// The receipt the server signed is the one the registry stores, `started_at` is the
/// server's and not this machine's clock, and the state moves with it — the length
/// gate keys on `Armed | StartSignalSeen`, so a stamped `started_at` under a pre-goal state
/// would leave the goal limit applied to every later prompt.
#[tokio::test]
async fn a_verified_receipt_is_stored_and_started_at_is_the_servers_own_clock() {
    let (server, url) = support::devserver().await;
    let (_dir, registry, entry) = support::registry_with_entry("e1", "/ws/e1");
    let expected_paths = StartPathIdentity::from_entry(&entry);
    arm(&registry, "e1");
    let pinned = goalcup_devserver::dev_server_public_key_hex();
    let mut claims = support::claims("e1", "1999-01-01T00:00:00Z", "1999-01-02T00:00:00Z");
    claims.connector_id = entry.connector_id.clone();
    claims.connector_dir_sha256 = entry.connector_dir_sha256.clone();

    let receipt = goalcup_daemon::receipt::ReceiptClient::new(url)
        .request(&claims, &support::run_public_key())
        .await
        .expect("the dev server signs");

    // This is structural, not a caller's discipline: a gate that has seen no transcript
    // record cannot stamp a start, whatever the server signed.
    let m = support::manifest("codex-cli");
    let mut uncorroborated = ArmingGate::new(&m, 0.0);
    assert!(
        goalcup_daemon::receipt::record_start(
            &registry,
            "e1",
            &mut uncorroborated,
            &receipt,
            &claims,
            &expected_paths,
            &pinned,
        )
        .is_err(),
        "a gate no transcript record reached must not become a started_at"
    );
    assert_eq!(
        registry
            .get("e1")
            .expect("the entry reads")
            .expect("it is there")
            .started_at,
        None
    );

    // The session the receipt names is the one the gate observed, and a receipt
    // naming any other session is not this run's.
    let mut wrong_session = ArmingGate::new(&m, 0.0);
    wrong_session.observe_leg1("rollout", 7, "ses-somebody-else", 2.0);
    let e = goalcup_daemon::receipt::record_start(
        &registry,
        "e1",
        &mut wrong_session,
        &receipt,
        &claims,
        &expected_paths,
        &pinned,
    )
    .expect_err("a receipt naming a session the gate never saw is refused")
    .to_string();
    assert!(e.contains("session_id"), "{e}");

    let mut gate = ArmingGate::new(&m, 0.0);
    gate.observe_leg1("rollout", 12, &claims.session_id, 2.0);
    assert_eq!(gate.session_id(), Some(claims.session_id.as_str()));
    let stored = goalcup_daemon::receipt::record_start(
        &registry,
        "e1",
        &mut gate,
        &receipt,
        &claims,
        &expected_paths,
        &pinned,
    )
    .expect("a verified receipt is stored");
    assert_eq!(gate.state(), GateState::Started);

    assert_eq!(stored.receipt(), Some(&receipt));
    assert_eq!(
        stored.started_at.as_deref(),
        Some(support::pinned_start().as_str())
    );
    assert_eq!(
        stored.deadline_at.as_deref(),
        Some(support::pinned_deadline().as_str())
    );
    assert_eq!(stored.state, goalcup_daemon::registry::EntryState::Started);
    assert_eq!(
        stored.session_id.as_deref(),
        Some(claims.session_id.as_str()),
        "the session on the entry is the corroborating record's"
    );

    // Finding 3: the registry is the authority on whether this entry may start, not the
    // in-memory gate. A second start against an entry the registry already says is
    // `Started` writes nothing, however corroborated the gate handing it in.
    let mut second = ArmingGate::new(&m, 0.0);
    second.observe_leg1("rollout", 13, &claims.session_id, 2.0);
    let e = goalcup_daemon::receipt::record_start(
        &registry,
        "e1",
        &mut second,
        &receipt,
        &claims,
        &expected_paths,
        &pinned,
    )
    .expect_err("an entry the registry says has started does not start again")
    .to_string();
    assert!(e.contains("started"), "the refusal names the state: {e}");
    let after = registry
        .get("e1")
        .expect("the entry reads")
        .expect("it is there");
    assert_eq!(
        after.started_at.as_deref(),
        Some(support::pinned_start().as_str()),
        "the held receipt's start is untouched"
    );

    // A receipt under anybody else's key is not stored at all.
    let (_dir2, registry2, entry2) = support::registry_with_entry("e1", "/ws/e1");
    let expected_paths2 = StartPathIdentity::from_entry(&entry2);
    arm(&registry2, "e1");
    let forged = goalcup_protocol::receipt::StartReceipt::sign(
        receipt.claims.clone(),
        &ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]),
    )
    .expect("a receipt signs");
    let mut gate2 = ArmingGate::new(&m, 0.0);
    gate2.observe_leg1("rollout", 12, &claims.session_id, 2.0);
    assert!(
        goalcup_daemon::receipt::record_start(
            &registry2,
            "e1",
            &mut gate2,
            &forged,
            &claims,
            &expected_paths2,
            &pinned,
        )
        .is_err()
    );
    assert_eq!(
        gate2.state(),
        GateState::StartSignalSeen,
        "a refused receipt leaves the gate where it was"
    );
    let after = registry2
        .get("e1")
        .expect("the entry reads")
        .expect("it is there");
    assert_eq!(after.receipt(), None);
    assert_eq!(after.started_at, None);
    assert_eq!(after.state, goalcup_daemon::registry::EntryState::Armed);
    let _ = server;
}

/// Move an entry to `Armed`, which is the state the arming gate runs in and the only one
/// (with `StartSignalSeen`) a start may be recorded from.
fn arm(registry: &goalcup_daemon::registry::Registry, entry_id: &str) {
    registry
        .update(entry_id, |e| {
            e.state = goalcup_daemon::registry::EntryState::Armed
        })
        .expect("the entry is armed");
}

#[test]
fn foreign_line_scan_reports_discovery_open_malformed_and_overlong_sources() {
    let t = tempfile::tempdir().expect("a temporary directory");
    let ws = t.path().join("ws");
    let manifest = support::manifest("codex-cli");
    let plan = trusted_plan(&manifest);
    let vars = support::path_vars(&ws, t.path());
    let projects = t.path().join(".claude/projects");

    std::fs::create_dir_all(projects.parent().unwrap()).unwrap();
    std::fs::write(&projects, b"not a directory").unwrap();
    let discovery = goalcup_daemon::foreign::scan_once(&plan, &vars, std::slice::from_ref(&ws))
        .expect("discovery refusal is evidence");
    assert_eq!(
        coverage(&discovery, "codex-cli", 1),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Discovery)
    );

    std::fs::remove_file(&projects).unwrap();
    let slug = projects.join("slug");
    std::fs::create_dir_all(&slug).unwrap();
    let record = slug.join("record.jsonl");
    std::fs::write(&record, b"not json\n").unwrap();
    let malformed = goalcup_daemon::foreign::scan_once(&plan, &vars, std::slice::from_ref(&ws))
        .expect("malformed record is evidence");
    assert_eq!(
        coverage(&malformed, "codex-cli", 1),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Row)
    );

    std::fs::write(
        &record,
        vec![b'x'; goalcup_daemon::MAX_LINE_BYTES as usize + 1],
    )
    .unwrap();
    let overlong = goalcup_daemon::foreign::scan_once(&plan, &vars, std::slice::from_ref(&ws))
        .expect("record cap is evidence");
    assert_eq!(
        coverage(&overlong, "codex-cli", 1),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::RecordLimit)
    );

    std::fs::remove_file(&record).unwrap();
    std::fs::create_dir(&record).unwrap();
    let open = goalcup_daemon::foreign::scan_once(&plan, &vars, std::slice::from_ref(&ws))
        .expect("an unreadable matched source is evidence");
    assert_eq!(
        coverage(&open, "codex-cli", 1),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Open)
    );
}

#[cfg(unix)]
#[test]
fn foreign_special_source_child() {
    use std::os::unix::ffi::OsStrExt;

    let Ok(kind) = std::env::var("GOALCUP_FOREIGN_SPECIAL_SOURCE_CHILD") else {
        return;
    };
    let t = tempfile::tempdir().expect("a temporary directory");
    let ws = t.path().join("ws");
    let (source, connector, scan_index) = match kind.as_str() {
        "line" => (
            t.path().join(".claude/projects/slug/record.jsonl"),
            "codex-cli",
            1,
        ),
        unexpected => panic!("unknown child case {unexpected}"),
    };
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    let source_c = std::ffi::CString::new(source.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(source_c.as_ptr(), 0o600) }, 0);

    let manifest = support::manifest("codex-cli");
    let result = goalcup_daemon::foreign::scan_once(
        &trusted_plan(&manifest),
        &support::path_vars(&ws, t.path()),
        std::slice::from_ref(&ws),
    )
    .expect("an unsupported source is explicit evidence");
    assert_eq!(
        coverage(&result, connector, scan_index),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Open)
    );
}

#[cfg(unix)]
#[test]
fn line_special_sources_are_refused_without_blocking() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    {
        let kind = "line";
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "foreign_special_source_child", "--nocapture"])
            .env("GOALCUP_FOREIGN_SPECIAL_SOURCE_CHILD", kind)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "{kind} foreign scan blocked on a special source; stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "{kind} child exited with {status}");
    }
}

#[test]
fn foreign_glob_path_cap_is_an_explicit_outcome() {
    let t = tempfile::tempdir().expect("a temporary directory");
    let ws = t.path().join("ws");
    let slug = t.path().join(".claude/projects/slug");
    std::fs::create_dir_all(&slug).unwrap();
    for index in 0..=goalcup_daemon::foreign::MAX_FOREIGN_SCAN_PATHS {
        std::fs::write(slug.join(format!("{index:04}.jsonl")), b"{}\n").unwrap();
    }
    let manifest = support::manifest("codex-cli");
    let result = goalcup_daemon::foreign::scan_once(
        &trusted_plan(&manifest),
        &support::path_vars(&ws, t.path()),
        std::slice::from_ref(&ws),
    )
    .expect("path cap is evidence");
    assert_eq!(
        coverage(&result, "codex-cli", 1),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::PathLimit)
    );
}

#[cfg(unix)]
#[test]
fn a_discovered_symbolic_foreign_record_is_refused_not_followed() {
    let t = tempfile::tempdir().expect("a temporary directory");
    let ws = t.path().join("ws");
    let slug = t.path().join(".claude/projects/slug");
    std::fs::create_dir_all(&slug).unwrap();
    std::os::unix::fs::symlink(slug.join("gone"), slug.join("record.jsonl")).unwrap();
    let manifest = support::manifest("codex-cli");
    let result = goalcup_daemon::foreign::scan_once(
        &trusted_plan(&manifest),
        &support::path_vars(&ws, t.path()),
        std::slice::from_ref(&ws),
    )
    .expect("unsupported-source evidence is explicit");
    assert_eq!(
        coverage(&result, "codex-cli", 1),
        ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Open)
    );
}

#[test]
fn own_config_scans_apply_only_to_their_exact_active_connector() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("workspace");
    let real_home = t.path().join("real-home");
    let claude_config = t.path().join("scoped-claude");
    let slug = claude_config.join("projects/other");
    std::fs::create_dir_all(&slug).unwrap();
    std::fs::write(
        slug.join("second.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({
                "cwd": ws.display().to_string(),
                "sessionId": "second-claude-session"
            })
        ),
    )
    .unwrap();
    let vars = goalcup_daemon::paths::PathVars::new(
        ws.clone(),
        real_home,
        claude_config,
        t.path().join("artifacts"),
        None,
        ("2026".into(), "09".into(), "04".into()),
    );

    let claude = support::manifest("claude-code-cli");
    let found = goalcup_daemon::foreign::scan_once(
        &trusted_plan(&claude),
        &vars,
        std::slice::from_ref(&ws),
    )
    .unwrap();
    assert!(found.sightings.iter().any(|sighting| {
        sighting.harness == "claude-code-cli" && sighting.session_id == "second-claude-session"
    }));
    assert!(found.coverage.iter().any(|coverage| {
        coverage.declaring_connector == "claude-code-cli" && coverage.scan_index == 1
    }));

    let codex = support::manifest("codex-cli");
    let inapplicable =
        goalcup_daemon::foreign::scan_once(&trusted_plan(&codex), &vars, std::slice::from_ref(&ws))
            .unwrap();
    assert!(inapplicable.coverage.iter().all(|coverage| {
        !(coverage.declaring_connector == "claude-code-cli" && coverage.scan_index == 1)
    }));
}

/// The contract `partition.rs` states and this module inherits: the compare is
/// byte-exact, so the seeding site owes a canonical workspace. On macOS a tempdir is handed
/// out as `/var/folders/…` while the harness writes `/private/var/folders/…` into its own
/// `cwd`, and an entry seeded with the first spelling makes the daemon blind to a session
/// sitting in its own workspace.
#[test]
fn an_uncanonicalised_registry_workspace_makes_a_private_var_occupant_invisible() {
    let t = tempfile::tempdir().expect("a temporary directory");
    let home = t.path().join("home/codex");
    let canonical = std::path::Path::new("/private/var/folders/xy/T/ws-e1");
    let uncanonical = std::path::Path::new("/var/folders/xy/T/ws-e1");
    support::plant_rollout(&home, canonical, "thread-existing");

    // Seeded the way the contract forbids: the occupant is there, and the scan cannot see
    // it. This is the defect being pinned, not the behaviour being asked for.
    let (_dir, blind, _e) = support::registry_with_entry("e1", &uncanonical.display().to_string());
    goalcup_daemon::foreign::pre_arm_scan(
        &support::manifest("codex-cli"),
        &support::path_vars(uncanonical, &home),
        &blind,
        &home,
        "candidate",
    )
    .expect("an uncanonicalised workspace does not match the harness's own cwd");

    // Seeded the way the contract requires, the same home is refused.
    let (_dir2, seeing, _e2) = support::registry_with_entry("e1", &canonical.display().to_string());
    assert!(
        goalcup_daemon::foreign::pre_arm_scan(
            &support::manifest("codex-cli"),
            &support::path_vars(canonical, &home),
            &seeing,
            &home,
            "candidate",
        )
        .is_err(),
        "the canonical spelling is the one the occupant wrote"
    );
}

/// A server that accepts the connection and then says nothing must not hold the
/// blocking task for ever, on a daemon whose whole run is one request.
#[tokio::test]
async fn a_receipt_server_that_accepts_and_never_answers_is_a_timeout_and_not_a_hang() {
    assert_eq!(
        goalcup_daemon::receipt::NETWORK_TIMEOUT,
        std::time::Duration::from_secs(10),
        "connect, read and write are bounded by the same ten seconds"
    );

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    // Accepted and held open: the connect succeeds, the request is written, and nothing
    // ever comes back.
    let held = std::thread::spawn(move || listener.accept().map(|(stream, _)| stream));

    let began = std::time::Instant::now();
    let e = goalcup_daemon::receipt::ReceiptClient::with_timeout(
        format!("http://{addr}"),
        std::time::Duration::from_millis(250),
    )
    .request(
        &support::claims("e1", &support::pinned_start(), &support::pinned_deadline()),
        &support::run_public_key(),
    )
    .await
    // The `await` returning at all is the second half of the claim: the `spawn_blocking`
    // finished, so its worker thread is back in the pool rather than parked on a socket
    // for the rest of the run.
    .expect_err("a server that never answers is an error, not a hang");
    assert!(
        began.elapsed() < std::time::Duration::from_secs(5),
        "the request gave up after its own timeout, not after the OS's: {:?}",
        began.elapsed()
    );
    assert!(!e.to_string().is_empty());
    drop(held.join().expect("the listener thread finishes"));
}

/// Cleartext to anywhere but loopback would carry a receipt request — and, with the same rule in
/// `DaemonConfig::validate`, a whole run's chunks — over a network in the clear.
#[tokio::test]
async fn cleartext_is_loopback_only_and_https_names_where_tls_lands() {
    let claims = support::claims("e1", &support::pinned_start(), &support::pinned_deadline());
    for loopback in [
        "http://127.0.0.1:9",
        "http://127.9.9.9:9",
        "http://localhost:9",
        "http://[::1]:9",
    ] {
        // Nothing listens on port 9, so the refusal these must not produce is the url one:
        // a connection error means the url itself was accepted.
        let e = goalcup_daemon::receipt::ReceiptClient::with_timeout(
            loopback,
            std::time::Duration::from_millis(250),
        )
        .request(&claims, &support::run_public_key())
        .await
        .expect_err("nothing is listening there")
        .to_string();
        assert!(
            !e.contains("loopback"),
            "{loopback} is loopback and must be accepted: {e}"
        );
    }

    let e = goalcup_daemon::receipt::ReceiptClient::new("http://events.example.invalid")
        .request(&claims, &support::run_public_key())
        .await
        .expect_err("cleartext off this machine is refused")
        .to_string();
    assert!(e.contains("loopback"), "{e}");

    let e = goalcup_daemon::receipt::ReceiptClient::new("https://events.example.invalid")
        .request(&claims, &support::run_public_key())
        .await
        .expect_err("this build has no TLS")
        .to_string();
    assert!(e.contains("M4"), "the refusal names where TLS lands: {e}");
}

/// Finding 7: `ENTRY_WINDOW_SECONDS` is restated in the daemon rather than imported from a
/// dev dependency that holds a server key (D3). Restated, so asserted.
#[test]
fn the_daemons_entry_window_is_the_servers_own() {
    assert_eq!(
        goalcup_daemon::receipt::ENTRY_WINDOW_SECONDS,
        goalcup_devserver::ENTRY_WINDOW_SECONDS,
        "the deadline the daemon re-derives is the one the server computes"
    );
}
