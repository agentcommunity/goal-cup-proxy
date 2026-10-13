//! The control API's state, its one authentication rule, `control.json`, and the
//! router the CLI and the bundled page both reach.
//!
//! The handler cases live here too. What this file gates is the surface
//! around them: which routes carry a token, what an un-normalised path does to that
//! decision, what the file the CLI reads to find the daemon contains, and the one table
//! that turns a `ControlError` into an HTTP status.

use crate::common::support;

use goalcup_daemon::control;
use goalcup_protocol::control as protocol;

fn tree_inventory(
    root: &std::path::Path,
) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(
        root: &std::path::Path,
        current: &std::path::Path,
        inventory: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        for child in std::fs::read_dir(current).expect("the snapshot root reads") {
            let child = child.expect("a snapshot directory entry reads");
            let path = child.path();
            let relative = path
                .strip_prefix(root)
                .expect("the walk stays below its root")
                .to_path_buf();
            let metadata = std::fs::symlink_metadata(&path).expect("snapshot metadata reads");
            if metadata.is_dir() {
                inventory.insert(relative, b"directory".to_vec());
                visit(root, &path, inventory);
            } else if metadata.is_file() {
                let mut value = b"file\0".to_vec();
                value.extend(std::fs::read(&path).expect("snapshot file reads"));
                inventory.insert(relative, value);
            } else {
                panic!("the capture fixture contains an unexpected object: {path:?}");
            }
        }
    }

    let mut inventory = std::collections::BTreeMap::new();
    visit(root, root, &mut inventory);
    inventory
}

#[derive(Debug, PartialEq)]
struct ReopenedCaptureSnapshot {
    row: goalcup_daemon::registry::Entry,
    chain: Vec<u8>,
    artifacts: std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    workspace: std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
}

fn reopened_capture_snapshot(
    h: &support::EmptyControlHarness,
    entry_id: &str,
) -> ReopenedCaptureSnapshot {
    let row = h
        .daemon
        .registry()
        .get(entry_id)
        .expect("the registry reads")
        .expect("the capture row exists");
    ReopenedCaptureSnapshot {
        chain: std::fs::read(row.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the chain reads"),
        artifacts: tree_inventory(&row.artifact_dir),
        workspace: tree_inventory(&row.workspace),
        row,
    }
}

fn capture_request(
    h: &support::EmptyControlHarness,
    name: &str,
) -> protocol::CaptureWorkspaceRequest {
    protocol::CaptureWorkspaceRequest {
        workspace: h.root.join("workspaces").join(name).display().to_string(),
        profile_id: "codex-cli".to_string(),
        connector_dir_sha256: goalcup_protocol::pins::pinned_connector_dir_sha256("codex-cli")
            .expect("the shipped connector is pinned")
            .to_string(),
        reserve_bytes: 4 * 1024 * 1024,
        run_home: h.root.join("run-home").display().to_string(),
        config_home: h.root.join("config-home").display().to_string(),
    }
}

async fn create_capture(
    h: &support::EmptyControlHarness,
    name: &str,
) -> protocol::CaptureWorkspaceInfo {
    h.post_json(
        protocol::ROUTE_CAPTURE_WORKSPACES,
        &capture_request(h, name),
    )
    .await
}

/// A volume the chain cannot be reserved on is refused with the sentence naming what does
/// qualify, and only that refusal carries it: a reservation that fails for another reason must
/// not send the participant to move their trace folder.
///
/// The fault is thread-local and one-shot; the control listener runs on this test's
/// current-thread runtime, so the handler that consumes it is on the thread that set it.
#[cfg(debug_assertions)]
#[tokio::test]
async fn an_unqualified_filesystem_refusal_names_what_qualifies() {
    use goalcup_platform::tracecap::{ReservationFault, inject_reservation_fault};
    const QUALIFIED: &str = "the trace folder must be on APFS (macOS) or ext4 or XFS (Linux); move artifact_root in daemon.toml";
    let h = support::empty_control_harness().await;
    for (fault, names_qualified) in [
        (ReservationFault::UnqualifiedFileSystem, true),
        (ReservationFault::AllocatorGrantsNothing, false),
    ] {
        let request = capture_request(&h, &format!("refused-{fault:?}"));
        inject_reservation_fault(fault);
        let response = h
            .post_body(protocol::ROUTE_CAPTURE_WORKSPACES, &request)
            .await;
        assert_eq!(response.status(), 422, "{}", response.body());
        let body: protocol::ErrorBody = response.json().unwrap();
        assert_eq!(body.error.code, "preflight_failed");
        assert!(
            body.error.message.contains("cannot reserve"),
            "{fault:?}: {}",
            body.error.message
        );
        assert_eq!(
            body.error.message.contains(QUALIFIED),
            names_qualified,
            "{fault:?}: {}",
            body.error.message
        );
    }
}

/// The authentication rule, asserted from both sides of the same function: the CLI's
/// `is_public_route` and the daemon's answer have to agree route for route, because the
/// failure this test exists to prevent is the two drifting apart and making the status
/// route a guaranteed `401`.
#[tokio::test]
async fn exactly_the_public_routes_answer_without_a_token_and_nothing_else_does() {
    let h = support::capture_control_harness().await;

    let status = protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS);
    assert!(protocol::is_public_route(&status), "{status}");
    assert_eq!(
        h.get_no_token(&status).await.status(),
        200,
        "{status} must answer without a token"
    );

    for path in [
        // The removed V1 page is not public: nothing is served there.
        "/v0/ui".to_string(),
        protocol::ROUTE_ENTRIES.to_string(),
        protocol::ROUTE_ENTRIES_CURRENT.to_string(),
        protocol::ROUTE_HEALTH.to_string(),
        protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL),
    ] {
        assert!(!protocol::is_public_route(&path), "{path}");
        let r = h.get_no_token(&path).await;
        assert_eq!(
            r.status(),
            401,
            "{path} is a directory of the participant's workspaces"
        );
        let body: protocol::ErrorBody = r.json().expect("the contract's error body");
        assert_eq!(body.error.code, "unauthorized");
        assert!(!body.error.message.is_empty());
        assert!(!body.error.detail.is_null(), "all three fields are filled");
    }

    assert_eq!(h.get(protocol::ROUTE_HEALTH).await.status(), 200);
}

/// The same rule over the whole route table rather than over four hand-picked paths, so
/// a route added to `control::TOP_LEVEL_ROUTES` or `control::ENTRY_SUFFIXES` without a
/// thought about its bearer fails here.
#[tokio::test]
async fn every_route_the_table_names_is_public_or_answers_401() {
    let h = support::capture_control_harness().await;
    let mut paths: Vec<String> = control::TOP_LEVEL_ROUTES
        .iter()
        .map(|r| r.to_string())
        .collect();
    paths.extend(
        control::ENTRY_SUFFIXES
            .iter()
            .map(|s| protocol::entry_route(&h.entry_id, s)),
    );
    assert!(paths.len() >= 10, "the table is the whole surface");

    for path in paths {
        let status = h.get_no_token(&path).await.status();
        match protocol::is_public_route(&path) {
            // A public route answers whatever it answers; what it never does is 401.
            true => assert_ne!(status, 401, "{path} is public"),
            // Everything else meets the bearer before it meets a router, which is why an
            // unregistered path answers 401 rather than 404.
            false => assert_eq!(status, 401, "{path} answered without a token"),
        }
    }
}

/// `is_public_route` does one percent decode per
/// segment; asking it about an already-decoded path makes that decode the second one, and
/// `%252e%252e` then reads as literal text, is admitted, and resolves to `..` at the next
/// normalising hop. So the middleware hands it `req.uri().path()` verbatim.
///
/// The cases are written against the public route that exists (`/v0/entries/{id}/status`);
/// they were once written against `/v0/assets/*` and `/v0/ui`, which nothing serves and
/// which are no longer public.
#[tokio::test]
async fn the_auth_layer_asks_about_the_raw_path_and_a_traversal_is_never_public() {
    let h = support::capture_control_harness().await;
    for path in [
        "/v0/entries/../status",
        "/v0/entries/%2e%2e/status",
        "/v0/entries/%2E%2E/status",
        "/v0/entries/%2e/status",
        "/v0/entries/e1/status/../../health",
        "/v0/entries/%zz/status",
    ] {
        assert!(!protocol::is_public_route(path), "{path}");
        assert_eq!(
            h.get_raw_no_token(path).await.status(),
            401,
            "{path} reached a handler without a token"
        );
    }

    // `is_public_route` admits `/v0/entries/%252e%252e/status`. It does one decode per
    // segment: `%252e%252e` decodes once to the six literal characters `%2e%2e`, which is
    // neither `.` nor `..`, and the segment is admitted as the id of the public status route.
    // So this asserts what the tree does — and then asserts the two facts that make
    // admitting it harmless:
    assert!(
        protocol::is_public_route(DOUBLE_ENCODED),
        "the contract's own function admits a segment whose SINGLE decode is not a dot"
    );
    let r = h.get_raw_no_token(DOUBLE_ENCODED).await;
    // (1) It reaches no entry. axum matches on the raw path too, so `%252e%252e` is one
    //     literal id that names no entry: the answer is 404, never a 200.
    assert_eq!(r.status(), 404, "{DOUBLE_ENCODED} must reach no entry");
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "not_found");
    // (2) The hop that would turn it into a traversal is itself refused. A proxy that
    //     decodes once sends `/v0/entries/%2e%2e/status`, and that path is not public and
    //     answers 401 — asserted in the loop above.

    // `%2f` inside a segment is not a traversal and is not refused here; the handlers
    // behind the layer are what decode it, after auth has already answered. So
    // `/v0/entries/a%2fb/status` is public — one status route with an id spelled oddly —
    // because the segment holds no literal `/`, so it is one id, and
    // the id it decodes to is simply an entry this daemon does not hold.
    let encoded_slash = "/v0/entries/a%2fb/status";
    assert!(protocol::is_public_route(encoded_slash));
    assert_eq!(h.get_raw_no_token(encoded_slash).await.status(), 404);
}

/// The one traversal spelling `is_public_route` admits. Named so the two
/// assertions about it read as one decision rather than as a stray string.
const DOUBLE_ENCODED: &str = "/v0/entries/%252e%252e/status";

/// The file the CLI reads to find the daemon: mode, contents, and the one comparison that
/// says the two tokens in this process are different secrets.
#[tokio::test]
async fn the_control_file_is_0600_carries_no_socket_and_its_token_is_not_the_receivers() {
    let h = support::capture_control_harness().await;
    let path = h.config_dir.join("control.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path)
                .expect("control.json is there")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("control.json reads"))
            .expect("control.json is JSON");
    assert!(
        v.get("socket").is_none(),
        "the Unix transport is cut; M1c must not look for one"
    );
    assert_eq!(
        v["port"].as_u64().expect("a port") as u16,
        h.client.addr.port()
    );
    assert_eq!(
        v["control_api_version"].as_u64().expect("a version") as u32,
        protocol::CONTROL_API_VERSION
    );
    assert_ne!(
        v["token"].as_str().expect("a token"),
        h.ingest_secret,
        "D24's token is in the workspace where the agent can read it; this one is not"
    );
    // The file is the only place the CLI's bearer lives, and it is not the daemon's to
    // print: a `Debug` that carried it would put it in every interpolated error.
    let parsed: control::ControlFile =
        serde_json::from_value(v).expect("control.json is the daemon's own shape");
    assert!(!format!("{parsed:?}").contains(&parsed.token));
}

/// The error table, enumerated from the contract's own variants
/// rather than from a list of code strings a new variant would never join.
///
/// `control::every_control_error()` and `control::CONTROL_ERROR_CODES` are generated from
/// the same rows `control::http_status_for`'s match is, and that match names every
/// `ControlError` variant with no `_` arm. So a new variant does not compile until
/// it has a row; the row gives it a code, a status and a sample; and the length assertion
/// below then fails until this test's own expectations name it too. A new refusal cannot
/// become a silent 500.
#[test]
fn every_control_error_variant_maps_to_exactly_one_http_status() {
    // The hand-held half. Its length is the generated table's, so a row added to the
    // module fails to compile here until this list grows with it.
    let expected: [(&str, u16); control::CONTROL_ERROR_CODES.len()] = [
        ("unreachable", 503),
        ("not_found", 404),
        ("conflict", 409),
        ("unauthorized", 401),
        ("preflight_failed", 422),
        ("internal", 500),
        ("malformed", 400),
        // `unsupported` is 426 and nothing else is. The version gate is the only
        // thing that raises it, and a client reading the code alone and a client reading
        // the status alone must reach the same conclusion.
        ("unsupported", 426),
        // Prepared is already durable; only exact replay or cancellation can settle it.
        ("process_admission_uncertain", 500),
        // Prepared may also be waiting transiently for its runtime; same retry status,
        // distinct post-mutation classification.
        ("process_admission_busy", 409),
        // 409, the same status `conflict` carries. Told apart by the code and only by the
        // code: `conflict` is the entry refusing and `busy` is the entry held for one
        // pump, and a client that retried on the status alone spun against sealed entries.
        ("busy", 409),
        // 409 as well, and never `busy`: the v5 local finish ran out of its budget, wrote no
        // terminal and is not retried. Its message is the refused stage's token.
        ("finish_deadline", 409),
    ];

    let all = control::every_control_error();
    assert_eq!(
        all.len(),
        control::CONTROL_ERROR_CODES.len(),
        "the generated sample list and the generated code list are one table"
    );
    for (error, code) in all.iter().zip(control::CONTROL_ERROR_CODES) {
        assert_eq!(error.code(), *code, "the table's rows are in one order");
        let (_, status) = expected
            .iter()
            .find(|(c, _)| c == code)
            .unwrap_or_else(|| panic!("{code} is a new variant with no expected status"));
        assert_eq!(
            control::http_status_for(error).as_u16(),
            *status,
            "{code} moved"
        );
    }

    // Exactly one code answers 426, which is what makes the version gate's body readable
    // by a client that looks only at the code.
    let upgrades: Vec<&str> = all
        .iter()
        .filter(|e| control::http_status_for(e) == 426)
        .map(|e| e.code())
        .collect();
    assert_eq!(upgrades, vec!["unsupported"]);

    // Four codes share 409: permanent conflict, definite pre-admission busy, ambiguous
    // post-Prepare busy, and an expired local finish. Their explicit codes, not the shared
    // status, drive client safety.
    let conflicts: Vec<&str> = all
        .iter()
        .filter(|e| control::http_status_for(e) == 409)
        .map(|e| e.code())
        .collect();
    assert_eq!(
        conflicts,
        vec![
            "conflict",
            "process_admission_busy",
            "busy",
            "finish_deadline"
        ]
    );
}

/// The daemon's own refusals reach the wire as the contract's codes — enumerated over
/// every `DaemonError` variant, not over the five a handler happens to raise today.
///
/// `control::every_daemon_error()` is generated from the same rows `control::control_error`
/// matches on, and that match has no `_` arm. A seventeenth variant therefore does not
/// compile until it is mapped, and the mapping hands this test a sample of it.
///
/// `ControlError::Busy` stays on the wire (the CLI retries on it) but no `DaemonError`
/// maps to it any more; `Refused` is the entry's own state saying no and stays `conflict`.
#[test]
fn the_daemon_s_own_errors_become_the_contract_s_codes() {
    use goalcup_daemon::DaemonError;
    for (error, code) in [
        // A permanent refusal — already sealed, a lock on disk — is `conflict`.
        (DaemonError::Refused("already sealed".into()), "conflict"),
        // The v5 finish's expiry is neither: its own code, carrying the stage token alone.
        (
            DaemonError::FinishDeadline {
                stage: "owner_drain",
            },
            "finish_deadline",
        ),
        (DaemonError::Guard("no".into()), "conflict"),
        (DaemonError::Missing("gone".into()), "not_found"),
        (DaemonError::Config("bad".into()), "preflight_failed"),
        (DaemonError::Db("the registry".into()), "internal"),
    ] {
        assert_eq!(control::control_error(error).code(), code);
    }
    assert_eq!(
        control::control_error(DaemonError::FinishDeadline {
            stage: "owner_drain"
        })
        .to_string(),
        "owner_drain",
        "the wire message of a finish deadline is the bare stage token"
    );

    // Every variant, so a new one cannot arrive as an unnoticed 500. Each is mapped, and
    // each mapping's code is one the table above gives a status to.
    for error in control::every_daemon_error() {
        let mapped = control::control_error(error);
        assert!(
            control::CONTROL_ERROR_CODES.contains(&mapped.code()),
            "{} is not a code this daemon can give a status",
            mapped.code()
        );
    }

    // And the codes the CLI can rebuild round-trip through `from_code`. `unreachable`,
    // `internal`, `malformed` and `unsupported` are not in that set — the protocol's
    // `from_code` collapses them to `Internal` by design, "never a silent success" — so
    // the round trip is asserted over the five it does name.
    for code in [
        "not_found",
        "conflict",
        "unauthorized",
        "preflight_failed",
        "busy",
    ] {
        assert_eq!(protocol::ControlError::from_code(code, "m").code(), code);
    }

    // And the two that share 409 are not each other. A `from_code` that collapsed `busy`
    // into `Conflict` would leave every client back where it started, retrying a refusal
    // that will never change its mind.
    assert!(matches!(
        protocol::ControlError::from_code("busy", "held"),
        protocol::ControlError::Busy(_)
    ));
    assert!(matches!(
        protocol::ControlError::from_code("conflict", "sealed"),
        protocol::ControlError::Conflict(_)
    ));
}

/// The two statuses this module derives from the request line and
/// from the version header rather than from a `ControlError`, and what a client that reads
/// only the body code makes of each.
///
/// `405` is the method's, and its body code is `malformed` — the request line named a
/// method the route does not answer, which is the only one of the contract's ten codes
/// that is true of it: `not_found` would tell a client "no such entry" about a
/// route that exists.
///
/// `426` is the version's, and its body code is `unsupported`, which is the same status
/// [`control::http_status_for`] gives that code — asserted in the table test above — so
/// the code and the status cannot disagree.
///
/// Both codes round-trip through `ControlError::from_code`, so a client rebuilding the
/// error from its own wire code does not get `Internal` for two codes the protocol itself
/// defines and this very handler raises.
#[tokio::test]
async fn the_two_statuses_the_code_does_not_decide_carry_the_codes_that_are_true_of_them() {
    let h = support::capture_control_harness().await;

    let wrong_method = h
        .request(
            "POST",
            protocol::ROUTE_HEALTH,
            Some(h.control_token.clone()),
            vec![(
                control::VERSION_HEADER.to_string(),
                protocol::CONTROL_API_VERSION.to_string(),
            )],
            String::new(),
        )
        .await;
    assert_eq!(wrong_method.status(), 405);
    let body: protocol::ErrorBody = wrong_method.json().expect("the contract's error body");
    assert_eq!(body.error.code, "malformed");
    // A client that rebuilds the error from its code gets `Malformed` back, not the
    // `Internal` catch-all — `from_code` names this code. Pinned here because
    // the CLI's error handling reads this body.
    assert!(matches!(
        protocol::ControlError::from_code(&body.error.code, "m"),
        protocol::ControlError::Malformed(_)
    ));
    // The code alone does not say 405 — the status does. `malformed` is a 400 when a
    // handler raises it, and both readings ("your request was wrong") are true.
    assert_eq!(
        control::http_status_for(&protocol::ControlError::Malformed("m".into())).as_u16(),
        400
    );

    let refused = h
        .request(
            "GET",
            protocol::ROUTE_HEALTH,
            Some(h.control_token.clone()),
            vec![(control::VERSION_HEADER.to_string(), "99".to_string())],
            String::new(),
        )
        .await;
    assert_eq!(refused.status(), 426);
    let body: protocol::ErrorBody = refused.json().expect("the contract's error body");
    assert_eq!(body.error.code, "unsupported");
    assert!(matches!(
        protocol::ControlError::from_code(&body.error.code, "m"),
        protocol::ControlError::Unsupported(_)
    ));
    // Unlike 405, this one is the status its code maps to: one code, one status.
    assert_eq!(
        control::http_status_for(&protocol::ControlError::Unsupported("m".into())).as_u16(),
        426
    );
}

/// The scheme is a token in a header and HTTP spells it however it likes; the
/// secret after it is a secret and is compared byte for byte.
#[tokio::test]
async fn the_bearer_scheme_is_case_insensitive_and_the_token_is_not() {
    let h = support::capture_control_harness().await;
    let authorization = |value: String| vec![("Authorization".to_string(), value)];

    for scheme in ["Bearer", "bearer", "BEARER", "bEaReR"] {
        assert_eq!(
            h.request(
                "GET",
                protocol::ROUTE_HEALTH,
                None,
                authorization(format!("{scheme} {}", h.control_token)),
                String::new(),
            )
            .await
            .status(),
            200,
            "{scheme} is the same scheme"
        );
    }

    // The token is not a scheme. A hex bearer upper-cased is a different secret.
    assert_eq!(
        h.request(
            "GET",
            protocol::ROUTE_HEALTH,
            None,
            authorization(format!("Bearer {}", h.control_token.to_uppercase())),
            String::new(),
        )
        .await
        .status(),
        401
    );
    // And a scheme this API does not speak is not a bearer at all.
    assert_eq!(
        h.request(
            "GET",
            protocol::ROUTE_HEALTH,
            None,
            authorization(format!("Basic {}", h.control_token)),
            String::new(),
        )
        .await
        .status(),
        401
    );
}

/// `control.json` holds the credential every authenticated route trusts, and the
/// daemon writes it into a directory a participant can reach, so a planted symlink there
/// must not be followed: a write that opened the path with `O_TRUNC` would hand the bearer to
/// whatever the link named.
///
/// The write refuses a target that is not a regular file, and it never opens the
/// target at all — it creates `control.json.<pid>.tmp` with `O_EXCL` and renames over it.
#[cfg(unix)]
#[test]
fn a_planted_symlink_at_control_json_is_refused_and_its_target_is_untouched() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let config_dir = dir.path();
    let target = config_dir.join("elsewhere.json");
    std::fs::write(&target, b"not a control file\n").expect("the planted target");
    std::os::unix::fs::symlink(&target, config_dir.join(control::CONTROL_FILE_NAME))
        .expect("the planted link");

    let secret = "the-cli-bearer-nobody-else-may-read";
    let err = control::write_control_file(config_dir, 7788, secret)
        .expect_err("a control.json that is not a regular file is refused");
    assert!(
        format!("{err}").contains(control::CONTROL_FILE_NAME),
        "the refusal names the file it would not write: {err}"
    );
    assert!(!format!("{err}").contains(secret), "and never the bearer");

    assert_eq!(
        std::fs::read_to_string(&target).expect("the link target survives"),
        "not a control file\n",
        "the bearer must not be written through the link"
    );
    assert!(
        std::fs::symlink_metadata(config_dir.join(control::CONTROL_FILE_NAME))
            .expect("the link is still there")
            .file_type()
            .is_symlink(),
        "the write refuses; it does not quietly replace what it found"
    );
    // Nothing was left behind either: the temp file is created only after the refusal.
    let strays: Vec<_> = std::fs::read_dir(config_dir)
        .expect("the directory reads")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(
        strays.is_empty(),
        "no temp file survives a refusal: {strays:?}"
    );

    // A link naming a path that does not exist is refused as well, rather than creating it.
    std::fs::remove_file(config_dir.join(control::CONTROL_FILE_NAME)).expect("the link goes");
    let absent = config_dir.join("not-here.json");
    std::os::unix::fs::symlink(&absent, config_dir.join(control::CONTROL_FILE_NAME))
        .expect("a dangling link");
    assert!(control::write_control_file(config_dir, 7788, secret).is_err());
    assert!(!absent.exists(), "a dangling link is not a path to create");
}

/// The write is atomic, so a CLI reading `control.json` while the
/// daemon writes it never reads half a document: a write that truncated in place would
/// give a reader a window on an empty file and on a half-written one.
#[cfg(unix)]
#[test]
fn a_concurrent_reader_never_sees_a_partial_control_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let config_dir = dir.path().to_path_buf();
    let path = config_dir.join(control::CONTROL_FILE_NAME);
    let tokens = ["a".repeat(64), "b".repeat(64)];

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writers: Vec<_> = tokens
        .iter()
        .enumerate()
        .map(|(i, token)| {
            let (dir, token) = (config_dir.clone(), token.clone());
            std::thread::spawn(move || {
                for round in 0..40u16 {
                    // Both writers are in one process, so both want the same
                    // `control.json.<pid>.tmp`, and the second is refused rather than
                    // joining a file another writer holds — which is the `O_EXCL` half of
                    // the rule, exercised here rather than only argued. Retrying is what
                    // a caller does with that refusal; the deadline is so a genuine
                    // failure is a failure and not a hang.
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    loop {
                        match control::write_control_file(&dir, 7000 + round + i as u16, &token) {
                            Ok(_) => break,
                            Err(e) if std::time::Instant::now() < deadline => {
                                let _ = e;
                                std::thread::sleep(std::time::Duration::from_micros(100));
                            }
                            Err(e) => panic!("the write never succeeded: {e}"),
                        }
                    }
                }
            })
        })
        .collect();

    let reader = {
        let (path, stop, tokens) = (path.clone(), stop.clone(), tokens.clone());
        std::thread::spawn(move || {
            let mut reads = 0u32;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    // Before the first rename there is no file at all, which is a state a
                    // reader can tell apart from a truncated one.
                    continue;
                };
                let parsed: control::ControlFile =
                    serde_json::from_str(&text).unwrap_or_else(|e| {
                        panic!("a reader saw a partial control.json: {e}: {text:?}")
                    });
                assert!(
                    tokens.contains(&parsed.token),
                    "a reader saw a token neither writer wrote"
                );
                reads += 1;
            }
            reads
        })
    };

    for w in writers {
        w.join().expect("a writer finishes");
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let reads = reader.join().expect("the reader finishes");
    assert!(reads > 0, "the reader read the file at least once");
}

/// The version is on every response, including the refusals, and a client that
/// speaks a different one is refused rather than answered in JSON it will mis-read.
#[tokio::test]
async fn every_response_carries_the_version_and_a_stranger_major_is_refused() {
    let h = support::capture_control_harness().await;
    let expected = protocol::CONTROL_API_VERSION.to_string();
    for r in [
        // the status, public
        h.get_no_token(&protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS))
            .await,
        // a refusal, unauthenticated
        h.get_no_token(protocol::ROUTE_HEALTH).await,
        // an answer, authenticated
        h.get(protocol::ROUTE_HEALTH).await,
        // a route this build serves nothing on
        h.get("/v0/nothing-here").await,
    ] {
        assert_eq!(
            r.header(control::VERSION_HEADER),
            Some(expected.as_str()),
            "{} carried no version",
            r.status()
        );
    }

    let refused = h
        .request(
            "GET",
            protocol::ROUTE_HEALTH,
            Some(h.control_token.clone()),
            vec![(control::VERSION_HEADER.to_string(), "99".to_string())],
            String::new(),
        )
        .await;
    assert_eq!(refused.status(), 426);
    let body: protocol::ErrorBody = refused.json().expect("the contract's error body");
    assert_eq!(body.error.code, "unsupported");
    assert_eq!(
        body.error.detail["daemon"]
            .as_u64()
            .expect("the daemon's version") as u32,
        protocol::CONTROL_API_VERSION
    );

    // An unreadable version is refused too, and the detail says the daemon could not read
    // the client's: a client whose version this daemon cannot parse is one it cannot
    // promise anything to.
    let unreadable = h
        .request(
            "GET",
            protocol::ROUTE_HEALTH,
            Some(h.control_token.clone()),
            vec![(control::VERSION_HEADER.to_string(), "v-one".to_string())],
            String::new(),
        )
        .await;
    assert_eq!(unreadable.status(), 426);
    let body: protocol::ErrorBody = unreadable.json().expect("the contract's error body");
    assert!(body.error.detail["client"].is_null());

    // A route that exists under another method answers the contract's shape, not axum's
    // empty 405: a client that parses every failure as an `ErrorBody` must not read "the
    // daemon said nothing".
    let wrong_method = h
        .request(
            "POST",
            protocol::ROUTE_HEALTH,
            Some(h.control_token.clone()),
            vec![(control::VERSION_HEADER.to_string(), expected.clone())],
            String::new(),
        )
        .await;
    assert_eq!(wrong_method.status(), 405);
    let body: protocol::ErrorBody = wrong_method.json().expect("the contract's error body");
    // The code it carries is asserted in its own test above; what this one is
    // about is that the 405 is stamped with the version like every other response.
    assert!(!body.error.message.is_empty());
    assert_eq!(
        wrong_method.header(control::VERSION_HEADER),
        Some(expected.as_str())
    );

    // A version header the daemon does speak is not a refusal.
    assert_eq!(
        h.request(
            "GET",
            protocol::ROUTE_HEALTH,
            Some(h.control_token.clone()),
            vec![(control::VERSION_HEADER.to_string(), expected.clone())],
            String::new(),
        )
        .await
        .status(),
        200
    );
}

/// One fixed refusal, whatever the token was. A body that differed between "no
/// header" and "a token nobody holds" is the difference an attacker is looking for, and a
/// body that quoted the token back would put the secret in the reply.
#[tokio::test]
async fn a_wrong_token_is_answered_exactly_as_no_token_is() {
    let h = support::capture_control_harness().await;
    let none = h.get_no_token(protocol::ROUTE_HEALTH).await;
    let wrong = h
        .request(
            "GET",
            protocol::ROUTE_HEALTH,
            Some("f".repeat(h.control_token.len())),
            Vec::new(),
            String::new(),
        )
        .await;
    let truncated = h
        .request(
            "GET",
            protocol::ROUTE_HEALTH,
            Some(h.control_token[..8].to_string()),
            Vec::new(),
            String::new(),
        )
        .await;
    assert_eq!(none.status(), 401);
    assert_eq!(wrong.status(), 401);
    assert_eq!(truncated.status(), 401);
    assert_eq!(none.body(), wrong.body());
    assert_eq!(none.body(), truncated.body());
    for r in [&none, &wrong, &truncated] {
        assert!(!r.body().contains(&h.control_token));
        assert!(!r.body().contains(&h.ingest_secret));
    }
}

/// The health document, which is the one route `goalcup doctor` needs and the one place
/// the control API states its own version on the wire as well as in the header.
#[tokio::test]
async fn health_reports_the_version_the_control_file_promised() {
    let h = support::capture_control_harness().await;
    let health: protocol::DaemonHealth = h
        .get(protocol::ROUTE_HEALTH)
        .await
        .json()
        .expect("the contract's health document");
    assert_eq!(health.control_api_version, protocol::CONTROL_API_VERSION);
    assert_eq!(health.daemon_version, goalcup_daemon::DAEMON_VERSION);
    assert_eq!(health.entries_open, 1);
    assert!(health.uptime_seconds < 60);
}

/// The public status route is the one signed document, and being public is exactly why it
/// may not carry either token.
#[tokio::test]
async fn the_public_status_route_answers_a_signed_document_carrying_no_secret() {
    let h = support::capture_control_harness().await;
    let path = protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS);
    let r = h.get_no_token(&path).await;
    assert_eq!(r.status(), 200);
    for secret in [h.ingest_secret.as_str(), h.control_token.as_str()] {
        assert!(!r.body().contains(secret));
    }
    let signed: protocol::SignedStatus = r.json().expect("the contract's signed status");
    assert!(signed.verify_with(&h.run_public_key).is_ok());
    assert_eq!(signed.payload.entry_id, h.entry_id);

    // An id this daemon does not hold is a 404 in the contract's shape, not an empty body
    // a client would parse as a status.
    let missing = h
        .get_no_token(&protocol::entry_route(
            "no-such-entry",
            control::SUFFIX_STATUS,
        ))
        .await;
    assert_eq!(missing.status(), 404);
    let body: protocol::ErrorBody = missing.json().expect("the contract's error body");
    assert_eq!(body.error.code, "not_found");
}

/// The listener is bound before `control.json` is written, because the file must
/// never name a port nothing is listening on. A write that is refused must then not
/// leave an authenticated listener up
/// with a bearer nobody on the machine holds — a port open to nothing, for the life of the
/// process, that the CLI cannot reach and the operator cannot see.
///
/// So the write's failure aborts the listener before it returns, and the port is closed by
/// the time `Daemon::start` answers.
#[cfg(unix)]
#[tokio::test]
async fn a_refused_control_file_write_leaves_the_control_port_closed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = std::fs::canonicalize(dir.path()).expect("the tempdir canonicalises");
    let mut config = support::config_at(&root, "http://127.0.0.1:1");

    // A port of this test's own choosing, so the assertion afterwards can name it: with
    // `127.0.0.1:0` the OS picks one and a failed `start` never reports which.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = probe.local_addr().expect("the probe's address").port();
    drop(probe);
    config.control_addr = format!("127.0.0.1:{port}")
        .parse()
        .expect("a loopback address");

    // The refusal itself: a `control.json` that is not a regular file.
    std::fs::create_dir_all(&config.config_dir).expect("the config dir");
    std::fs::create_dir_all(config.config_dir.join(control::CONTROL_FILE_NAME))
        .expect("a directory where the file goes");

    let err = goalcup_daemon::runtime::Daemon::start(config)
        .await
        .err()
        .expect("a control.json that cannot be written is a daemon that does not start");
    assert!(
        format!("{err}").contains(control::CONTROL_FILE_NAME),
        "{err}"
    );

    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "the control listener outlived the start that failed: an authenticated port whose \
         bearer nobody holds"
    );
}

// =========================================================================================
// The process handle and the abandon.
// =========================================================================================

/// Restarted v5 captures retain their durable Registered fact for sealing, but it is no longer
/// live process-admission authority. The exact historical request therefore refuses before the
/// process route's ordinary idempotent-204 shortcut and changes no retained evidence.
#[tokio::test]
async fn a_reopened_capture_refuses_registered_process_replay() {
    let h = support::empty_control_harness().await;
    let info = create_capture(&h, "registered-replay").await;
    let mut child = support::spawn_harness();
    let mut handle = support::handle_for_now(child.id());
    handle.native_binary = Some("codex".to_string());
    let registration = support::process_registration(info.process_generation, handle);
    let process = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    let accepted = h.post_body(&process, &registration).await;
    assert_eq!(accepted.status(), 204, "{}", accepted.body());
    let registered = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the registry reads before restart")
        .expect("the registered capture row exists before restart");
    assert_eq!(
        registered.process_handoff_phase(),
        goalcup_daemon::registry::ProcessHandoffPhase::Registered,
        "the accepted route durably reached Registered before the replay fixture restarts"
    );
    assert_eq!(
        registered.last_process.as_ref(),
        Some(&registration.handle),
        "the durable row retains the exact accepted termination handle"
    );
    assert_eq!(
        registered.process_handoff_generation(),
        registration.generation,
        "the durable Registered fact names the replayed generation"
    );

    let h = support::restart_empty_control(h).await;
    let before = reopened_capture_snapshot(&h, &info.entry_id);
    let replay = h.post_body(&process, &registration).await;
    assert_eq!(replay.status(), 409, "{}", replay.body());
    let error: protocol::ErrorBody = replay.json().expect("the conflict response shape");
    assert_eq!(error.error.code, "conflict");
    assert!(
        error.error.message.contains("recovered capture")
            && error.error.message.contains("status and seal"),
        "the refusal names the retained authority: {}",
        error.error.message
    );
    assert_eq!(
        reopened_capture_snapshot(&h, &info.entry_id),
        before,
        "the exact replay mutates no retained capture state"
    );

    let _ = child.kill();
    let _ = child.wait();
}

/// A `codex-cli` capture create under the harness root, shaped like the one
/// `support::capture_control_harness` sends.
#[cfg(unix)]
fn capture_create_request(
    h: &support::EmptyControlHarness,
    name: &str,
) -> protocol::CaptureWorkspaceRequest {
    protocol::CaptureWorkspaceRequest {
        workspace: h.root.join("workspaces").join(name).display().to_string(),
        profile_id: "codex-cli".to_string(),
        connector_dir_sha256: goalcup_protocol::pins::pinned_connector_dir_sha256("codex-cli")
            .unwrap()
            .to_string(),
        reserve_bytes: 4 * 1024 * 1024,
        run_home: h
            .root
            .join(format!("{name}-run-home"))
            .display()
            .to_string(),
        config_home: h
            .root
            .join(format!("{name}-config-home"))
            .display()
            .to_string(),
    }
}

/// What a refused capture create must leave: a `409`, the outside directory holding only its
/// sentinel, and no entry registered.
#[cfg(unix)]
fn assert_refused_and_outside_untouched(
    h: &support::EmptyControlHarness,
    response: &support::Response,
    outside: &std::path::Path,
) {
    assert_eq!(response.status(), 409, "{}", response.body());
    let names: Vec<_> = std::fs::read_dir(outside)
        .expect("the outside directory remains readable")
        .map(|entry| entry.expect("an outside entry reads").file_name())
        .collect();
    assert_eq!(
        names,
        vec![std::ffi::OsString::from("sentinel.txt")],
        "the create wrote below the outside directory"
    );
    assert_eq!(
        std::fs::read(outside.join("sentinel.txt")).unwrap(),
        b"outside"
    );
    assert!(
        h.daemon.entries().unwrap().is_empty(),
        "a refused create registered an entry"
    );
}

/// A capture create acquires every component beneath the configured artifact root through
/// the artifact-root authority, never through a path: an `artifacts` component swapped for a
/// symlink to an outside directory is refused, and nothing is written through it.
#[cfg(unix)]
#[tokio::test]
async fn a_capture_create_refuses_a_symlinked_artifacts_component() {
    let h = support::empty_control_harness().await;
    let outside = h.root.join("outside-artifacts");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("sentinel.txt"), b"outside").unwrap();
    let artifacts = h.config.artifact_root.join("artifacts");
    let _ = std::fs::remove_dir_all(&artifacts);
    std::os::unix::fs::symlink(&outside, &artifacts).unwrap();

    let response = h
        .post_body(
            protocol::ROUTE_CAPTURE_WORKSPACES,
            &capture_create_request(&h, "swapped-artifacts"),
        )
        .await;
    assert_refused_and_outside_untouched(&h, &response, &outside);
}

/// The same, with the whole configured artifact root moved away and replaced by a symlink.
#[cfg(unix)]
#[tokio::test]
async fn a_capture_create_refuses_a_symlinked_artifact_root() {
    let h = support::empty_control_harness().await;
    let outside = h.root.join("outside-root");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("sentinel.txt"), b"outside").unwrap();
    std::fs::rename(&h.config.artifact_root, h.root.join("detached-root")).unwrap();
    std::os::unix::fs::symlink(&outside, &h.config.artifact_root).unwrap();

    let response = h
        .post_body(
            protocol::ROUTE_CAPTURE_WORKSPACES,
            &capture_create_request(&h, "swapped-root"),
        )
        .await;
    assert_refused_and_outside_untouched(&h, &response, &outside);
}

/// Every one of these routes is behind the bearer, and an id this daemon does not hold
/// is a `404` in the contract's shape on each of them.
#[tokio::test]
async fn task_21_s_routes_need_the_bearer_and_answer_404_for_an_entry_that_is_not_here() {
    let h = support::empty_control_harness().await;
    for path in [
        protocol::entry_route("no-such-entry", control::SUFFIX_PROCESS),
        protocol::entry_route("no-such-entry", control::SUFFIX_ABANDON),
    ] {
        assert!(!protocol::is_public_route(&path), "{path}");
        assert_eq!(
            h.request("POST", &path, None, Vec::new(), String::new())
                .await
                .status(),
            401,
            "{path}"
        );
    }

    // Each route with the body it actually takes: `abandon` takes none, and `process` takes
    // the handle. An id this daemon does not hold is the same 404 on both, and the refusal
    // names no path — the caller knows what it asked for.
    for r in [
        h.post(&protocol::entry_route(
            "no-such-entry",
            control::SUFFIX_ABANDON,
        ))
        .await,
        h.post_body(
            &protocol::entry_route("no-such-entry", control::SUFFIX_PROCESS),
            &support::process_registration(1, support::process_handle()),
        )
        .await,
    ] {
        assert_eq!(r.status(), 404, "{}", r.body());
        let body: protocol::ErrorBody = r.json().expect("the contract's error body");
        assert_eq!(body.error.code, "not_found");
    }

    // The one body this module defines is `deny_unknown_fields` like every other document
    // this daemon parses: a field `abandon` does not take is a 400, not a field ignored.
    let strange = h
        .post_body(
            &protocol::entry_route("no-such-entry", control::SUFFIX_ABANDON),
            &serde_json::json!({ "reason": "changed my mind", "force": true }),
        )
        .await;
    assert_eq!(strange.status(), 400);
    let body: protocol::ErrorBody = strange.json().expect("the contract's error body");
    assert_eq!(body.error.code, "malformed");
}

// =========================================================================================
// The reads and the two calls that end a run.
// =========================================================================================

/// A daemon killed mid-write leaves `control.json.<pid>.tmp` behind, and the temp
/// name carries the pid — so the next daemon that happens to be handed that pid would be
/// refused by its predecessor's leavings, with `O_EXCL` reporting "another writer holds it"
/// about a writer that has been dead for a week.
///
/// The sweep is at startup and it removes exactly that one name, so the `O_EXCL` create
/// itself keeps its whole meaning for the concurrent case above.
#[cfg(unix)]
#[tokio::test]
async fn a_stale_control_temp_from_a_dead_daemon_does_not_block_the_next_start() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = std::fs::canonicalize(dir.path()).expect("the tempdir canonicalises");
    let config = support::config_at(&root, "http://127.0.0.1:1");
    std::fs::create_dir_all(&config.config_dir).expect("the config dir");
    let stale = config.config_dir.join(format!(
        "{}.{}.tmp",
        control::CONTROL_FILE_NAME,
        std::process::id()
    ));
    std::fs::write(&stale, b"what a killed daemon left behind\n").expect("the stale temp");

    let daemon = goalcup_daemon::runtime::Daemon::start(config.clone())
        .await
        .expect("a stale temp file is not a reason to refuse to start");
    assert!(
        config.config_dir.join(control::CONTROL_FILE_NAME).is_file(),
        "control.json was written"
    );
    assert!(!stale.exists(), "the stale temp is gone");
    drop(daemon);
}

/// The control listener comes down when `control.json` cannot be written, rather
/// than serving a bearer that exists nowhere for the life of the process.
#[cfg(unix)]
#[tokio::test]
async fn a_refused_control_file_write_leaves_the_control_listener_closed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = std::fs::canonicalize(dir.path()).expect("the tempdir canonicalises");
    let mut config = support::config_at(&root, "http://127.0.0.1:1");

    // A port of this test's own choosing, so the assertion below can name it: with
    // `127.0.0.1:0` the OS picks and a failed `start` never reports which.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let control_port = probe.local_addr().expect("the probe's address").port();
    drop(probe);
    config.control_addr = format!("127.0.0.1:{control_port}")
        .parse()
        .expect("a loopback address");

    // The refusal itself: a `control.json` that is not a regular file.
    std::fs::create_dir_all(&config.config_dir).expect("the config dir");
    std::fs::create_dir_all(config.config_dir.join(control::CONTROL_FILE_NAME))
        .expect("a directory where the file goes");

    goalcup_daemon::runtime::Daemon::start(config)
        .await
        .err()
        .expect("a control.json that cannot be written is a daemon that does not start");

    assert!(
        std::net::TcpStream::connect(("127.0.0.1", control_port)).is_err(),
        "the control listener outlived the start that failed"
    );
}

// ------------------------------------------------------------------ the Host check

/// One request with the `Host` lines the caller names, byte for byte, and nothing else
/// added. `support::ControlClient` always writes `Host: <the listener's address>`, which is
/// what every in-tree client sends; the cases below are the ones it cannot write.
///
/// Returns the status and the body's `error.code` (empty when the body is not an error).
async fn request_with_host_lines(
    client: &support::ControlClient,
    method: &'static str,
    path: &str,
    host_lines: &[String],
    bearer: bool,
) -> (u16, String) {
    let addr = client.addr.to_string();
    let mut head = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    for line in host_lines {
        head.push_str(&format!("Host: {line}\r\n"));
    }
    if bearer {
        head.push_str(&format!("Authorization: Bearer {}\r\n", client.token));
        head.push_str(&format!(
            "x-goalcup-control-version: {}\r\n",
            protocol::CONTROL_API_VERSION
        ));
    }
    head.push_str("Content-Length: 0\r\n\r\n");
    tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(&addr).expect("connect");
        stream.write_all(head.as_bytes()).expect("write");
        let mut raw = Vec::new();
        let _ = stream.read_to_end(&mut raw);
        let text = String::from_utf8_lossy(&raw).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let code = text
            .split_once("\r\n\r\n")
            .and_then(|(_, body)| serde_json::from_str::<protocol::ErrorBody>(body).ok())
            .map(|b| b.error.code)
            .unwrap_or_default();
        (status, code)
    })
    .await
    .expect("the blocking request finishes")
}

/// A browser at `evil.example`, whose DNS now answers `127.0.0.1`, sends
/// `Host: evil.example:<port>` and reads the answer same-origin. So a `Host` that is not one
/// of the control listener's own three spellings is refused before the public/bearer split,
/// on a public route (the one a browser can read without a token) and on a bearer route
/// (where the refusal precedes even a correct token).
#[tokio::test]
async fn a_foreign_host_is_refused_on_a_public_route_and_on_a_bearer_route() {
    let h = support::capture_control_harness().await;
    let port = h.client.addr.port();
    let status = protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS);
    let other_port = if port == 1 { 2 } else { port - 1 };

    let foreign = [
        format!("evil.example:{port}"),
        format!("127.0.0.1.evil.example:{port}"),
        format!("localhost.evil.example:{port}"),
        format!("127.0.0.1:{other_port}"),
        format!("localhost:{other_port}"),
        format!("[::1]:{other_port}"),
        // No port: only a listener on 80 may be reached that way.
        "127.0.0.1".to_string(),
        "localhost".to_string(),
        "[::1]".to_string(),
        // Not an authority at all.
        String::new(),
        format!("127.0.0.1:{port}@evil.example"),
        format!("127.0.0.1:0{port}"),
        format!("127.0.0.1:+{port}"),
        format!("127.0.0.1:{port}:{port}"),
        format!("::1:{port}"),
        format!("localhost.:{port}"),
        format!("127.0.0.2:{port}"),
        format!("0.0.0.0:{port}"),
    ];
    for host in &foreign {
        for (path, bearer) in [
            (status.clone(), false),
            (protocol::ROUTE_HEALTH.to_string(), true),
            (status.clone(), true),
        ] {
            let (code, error) = request_with_host_lines(
                &h.client,
                "GET",
                &path,
                std::slice::from_ref(host),
                bearer,
            )
            .await;
            assert_eq!(code, 401, "Host `{host}` on {path} (bearer {bearer})");
            assert_eq!(error, "unauthorized", "Host `{host}` on {path}");
        }
    }
    // A POST with a correct bearer and version, from a foreign host, mutates nothing.
    let (code, _) = request_with_host_lines(
        &h.client,
        "POST",
        &protocol::entry_route(&h.entry_id, control::SUFFIX_ABANDON),
        &[format!("evil.example:{port}")],
        true,
    )
    .await;
    assert_eq!(code, 401, "a foreign Host never reaches a mutation");

    // No Host line at all, and two of them (one good, one foreign), are both refused.
    for lines in [
        Vec::new(),
        vec![format!("127.0.0.1:{port}"), format!("evil.example:{port}")],
        vec![format!("127.0.0.1:{port}"), format!("127.0.0.1:{port}")],
    ] {
        let (code, _) = request_with_host_lines(&h.client, "GET", &status, &lines, false).await;
        assert_eq!(code, 401, "Host lines {lines:?}");
    }
}

/// A proxy may send an absolute-form request target. Its authority is another answer to
/// "who is this for", so a valid `Host` line cannot make a foreign absolute authority local.
/// The matching loopback form remains an ordinary request for the public status route.
#[tokio::test]
async fn an_absolute_form_target_must_name_the_bound_listener() {
    let h = support::capture_control_harness().await;
    let port = h.client.addr.port();
    let host = format!("127.0.0.1:{port}");
    let status = protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS);

    let foreign_target = format!("http://evil.example:{port}{status}");
    let (code, error) = request_with_host_lines(
        &h.client,
        "GET",
        &foreign_target,
        std::slice::from_ref(&host),
        false,
    )
    .await;
    assert_eq!(
        code, 401,
        "a valid Host cannot override a foreign authority"
    );
    assert_eq!(error, "unauthorized");

    let loopback_target = format!("http://127.0.0.1:{port}{status}");
    let (code, error) = request_with_host_lines(
        &h.client,
        "GET",
        &loopback_target,
        std::slice::from_ref(&host),
        false,
    )
    .await;
    assert_eq!(code, 200, "the listener's absolute authority is accepted");
    assert!(
        error.is_empty(),
        "a successful response is not an error body"
    );
}

/// The three loopback spellings of the listener's own address pass, on a public route and on
/// a bearer route. A hostname is case-insensitive.
#[tokio::test]
async fn the_three_loopback_spellings_of_the_listener_pass() {
    let h = support::capture_control_harness().await;
    let port = h.client.addr.port();
    let status = protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS);
    for host in [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("LocalHost:{port}"),
        format!("[::1]:{port}"),
    ] {
        let (code, _) = request_with_host_lines(
            &h.client,
            "GET",
            &status,
            std::slice::from_ref(&host),
            false,
        )
        .await;
        assert_eq!(code, 200, "Host `{host}` on the public status route");
        let (code, _) = request_with_host_lines(
            &h.client,
            "GET",
            protocol::ROUTE_HEALTH,
            std::slice::from_ref(&host),
            true,
        )
        .await;
        assert_eq!(code, 200, "Host `{host}` on a bearer route");
    }
}

/// The refusal is stamped with the version like every other response, and says nothing
/// about the presented Host (nor the token).
#[tokio::test]
async fn the_host_refusal_carries_the_version_header_and_quotes_nothing_it_was_sent() {
    let h = support::capture_control_harness().await;
    let port = h.client.addr.port();
    let r = h
        .client
        .request(
            "GET",
            protocol::ROUTE_HEALTH,
            Some(h.client.token.clone()),
            Vec::new(),
            String::new(),
        )
        .await;
    assert_eq!(
        r.status(),
        200,
        "the control case: the support client's own Host"
    );
    assert_eq!(
        r.header("x-goalcup-control-version"),
        Some(protocol::CONTROL_API_VERSION.to_string().as_str())
    );

    // The public status route, so the bearer check cannot be what refuses it: only the Host
    // check can.
    let status_path = protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS);
    let addr = h.client.addr.to_string();
    let body = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(&addr).expect("connect");
        write!(
            s,
            "GET {status_path} HTTP/1.1\r\nHost: evil-marker.example:{port}\r\nConnection: close\r\n\r\n",
        )
        .expect("write");
        let mut raw = String::new();
        let _ = s.read_to_string(&mut raw);
        raw
    })
    .await
    .expect("request");
    assert!(body.starts_with("HTTP/1.1 401"), "{body}");
    assert!(
        body.to_ascii_lowercase().contains(&format!(
            "x-goalcup-control-version: {}",
            protocol::CONTROL_API_VERSION
        )),
        "{body}"
    );
    assert!(
        !body.contains("evil-marker"),
        "the refusal quotes the Host it was sent"
    );
}

// ------------------------------------------------------------------ /v0/assets

/// Nothing serves `/v0/assets/*`, so nothing under it is public: the prefix was dead public
/// surface, and an unauthenticated read answers like any other protected path.
#[tokio::test]
async fn nothing_under_v0_assets_is_public() {
    assert!(!protocol::is_public_route("/v0/assets/x"));
    assert!(!protocol::is_public_route("/v0/assets/sponsors.json"));
    let h = support::capture_control_harness().await;
    for path in ["/v0/assets/x", "/v0/assets/sponsors.json", "/v0/assets/"] {
        assert_eq!(
            h.get_no_token(path).await.status(),
            401,
            "{path} answered a stranger"
        );
        assert_eq!(h.get(path).await.status(), 404, "{path} is not a route");
    }
}
