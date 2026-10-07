//! The control API's state, its one authentication rule, `control.json`, and the
//! router the CLI and the bundled page both reach.
//!
//! The handler cases live here too. What this file gates is the surface
//! around them: which routes carry a token, what an un-normalised path does to that
//! decision, what the file the CLI reads to find the daemon contains, and the one table
//! that turns a `ControlError` into an HTTP status.

mod support;

use goalcup_daemon::control;
#[cfg(feature = "v1-compat")]
use goalcup_daemon::registry::EntryState;
use goalcup_protocol::control as protocol;
#[cfg(feature = "v1-compat")]
use goalcup_protocol::envelope::EventBody;

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

async fn create_capture(
    h: &support::EmptyControlHarness,
    name: &str,
) -> protocol::CaptureWorkspaceInfo {
    let request = protocol::CaptureWorkspaceRequest {
        workspace: h.root.join("workspaces").join(name).display().to_string(),
        profile_id: "codex-cli".to_string(),
        connector_dir_sha256: goalcup_protocol::pins::pinned_connector_dir_sha256("codex-cli")
            .expect("the shipped connector is pinned")
            .to_string(),
        reserve_bytes: 4 * 1024 * 1024,
        run_home: h.root.join("run-home").display().to_string(),
        config_home: h.root.join("config-home").display().to_string(),
    };
    h.post_json(protocol::ROUTE_CAPTURE_WORKSPACES, &request)
        .await
}

/// The authentication rule, asserted from both sides of the same function: the CLI's
/// `is_public_route` and the daemon's answer have to agree route for route, because the
/// failure this test exists to prevent is the two drifting apart and making the status
/// page a guaranteed `401`.
#[tokio::test]
async fn exactly_the_public_routes_answer_without_a_token_and_nothing_else_does() {
    let h = support::capture_control_harness().await;

    for path in [
        protocol::ROUTE_UI.to_string(),
        protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS),
    ] {
        assert!(protocol::is_public_route(&path), "{path}");
        // The page itself is V1: a default build mounts the stub that says so, still public.
        let served = if path == protocol::ROUTE_UI && cfg!(not(feature = "v1-compat")) {
            404
        } else {
            200
        };
        assert_eq!(
            h.get_no_token(&path).await.status(),
            served,
            "{path} must serve a browser"
        );
    }

    for path in [
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
    assert!(paths.len() >= 12, "the table is the whole surface");

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
/// The cases are written against the public routes that exist (`/v0/ui` and
/// `/v0/entries/{id}/status`); they were once written against `/v0/assets/*`, which nothing
/// served and which is no longer public.
#[tokio::test]
async fn the_auth_layer_asks_about_the_raw_path_and_a_traversal_is_never_public() {
    let h = support::capture_control_harness().await;
    for path in [
        "/v0/entries/../status",
        "/v0/entries/%2e%2e/status",
        "/v0/entries/%2E%2E/status",
        "/v0/entries/%2e/status",
        "/v0/ui/../entries",
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
        ("arming_timeout", 408),
        ("preflight_failed", 422),
        ("unsupported_connector", 422),
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
/// The pair that matters is `Refused` and `Busy`. Both are `409` and they mean opposite
/// things: `Refused` is the entry's own state saying no, and `Busy` is the entry held for
/// the length of one pump. A client retries `busy` and reports `conflict`, which it can
/// only do because the two carry different codes in the body of a status they share.
#[test]
fn the_daemon_s_own_errors_become_the_contract_s_codes() {
    use goalcup_daemon::DaemonError;
    for (error, code) in [
        // The pair that matters. A permanent refusal — already sealed, a lock on
        // disk — stays `conflict`; the transient hold is `busy`, and `busy` is the only
        // code the contract lets a client retry on.
        (DaemonError::Refused("already sealed".into()), "conflict"),
        (DaemonError::Busy("mid-upload".into()), "busy"),
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
    // the round trip is asserted over the six it does name.
    for code in [
        "not_found",
        "conflict",
        "unauthorized",
        "arming_timeout",
        "preflight_failed",
        "unsupported_connector",
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
        // the page, public
        h.get_no_token(protocol::ROUTE_UI).await,
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

/// The version header is an exact protocol token, not an integer a parser may normalize.
/// Otherwise a client which serialized its request under a different version spelling can
/// reach a current mutation route merely because both spellings happen to parse as `2`.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_noncanonical_or_duplicate_version_header_cannot_consume_a_resume_generation() {
    let expected = protocol::CONTROL_API_VERSION.to_string();
    for (name, headers) in [
        (
            "leading zero",
            vec![(control::VERSION_HEADER.to_string(), format!("0{expected}"))],
        ),
        (
            "leading plus",
            vec![(control::VERSION_HEADER.to_string(), format!("+{expected}"))],
        ),
        (
            "duplicate",
            vec![
                (control::VERSION_HEADER.to_string(), expected.clone()),
                (control::VERSION_HEADER.to_string(), expected.clone()),
            ],
        ),
        (
            "mixed-case duplicate",
            vec![
                (control::VERSION_HEADER.to_string(), expected.clone()),
                ("X-GoAlCuP-CoNtRoL-VeRsIoN".to_string(), expected.clone()),
            ],
        ),
    ] {
        let h = support::control_harness().await;
        let before = h.entry();
        let chain_before =
            std::fs::read(before.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
                .expect("the started entry has its baseline chain");
        let response = h
            .request(
                "POST",
                &protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME),
                Some(h.control_token.clone()),
                headers,
                serde_json::to_string(&protocol::ResumeRequest {
                    expected_generation: before.process_handoff_generation(),
                })
                .expect("the request serialises"),
            )
            .await;

        assert_eq!(response.status(), 426, "{name}: {}", response.body());
        assert_eq!(
            response.headers_named(control::VERSION_HEADER),
            vec![expected.as_str()],
            "{name} refusal carries the daemon's canonical version"
        );
        let body: protocol::ErrorBody = response.json().expect("the version refusal shape");
        assert_eq!(body.error.code, "unsupported", "{name}");
        assert!(
            body.error.detail["client"].is_null(),
            "{name} is not a canonical version token"
        );
        assert_eq!(h.entry(), before, "{name} did not consume authority");
        assert_eq!(
            std::fs::read(before.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
            chain_before,
            "{name} reached no capture side effect"
        );
    }

    let h = support::control_harness().await;
    let before = h.entry();
    let chain_before = std::fs::read(before.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
        .expect("the started entry has its baseline chain");
    let response = h
        .request_raw_header_values(
            "POST",
            &protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME),
            Some(h.control_token.clone()),
            vec![(control::VERSION_HEADER.to_string(), vec![0x80])],
            serde_json::to_string(&protocol::ResumeRequest {
                expected_generation: before.process_handoff_generation(),
            })
            .expect("the request serialises"),
        )
        .await;
    assert_eq!(response.status(), 426, "binary header: {}", response.body());
    assert_eq!(
        response.headers_named(control::VERSION_HEADER),
        vec![expected.as_str()],
        "the binary-header refusal carries one canonical response version"
    );
    let body: protocol::ErrorBody = response.json().expect("the version refusal shape");
    assert_eq!(body.error.code, "unsupported");
    assert!(body.error.detail["client"].is_null());
    assert_eq!(h.entry(), before, "the binary header consumed no authority");
    assert_eq!(
        std::fs::read(before.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
        chain_before,
        "the binary header reached no capture side effect"
    );
}

/// Both directions of a generation mismatch are refusals before the control handler can
/// catch up, clear a registered process, or accept an unsolicited replacement identity.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn stale_and_future_resume_or_process_generations_are_no_mutation_refusals() {
    for (name, generation) in [("stale", 0), ("future", 2)] {
        let h = support::control_harness().await;
        let before = h.entry();
        assert_eq!(
            before.process_handoff_generation(),
            1,
            "the fixture supplies one currently registered launch generation"
        );
        let chain_path = before.trace_dir.join(goalcup_protocol::export::CHAIN_FILE);
        let chain_before = std::fs::read(&chain_path).expect("the baseline chain exists");

        for (route, body) in [
            (
                protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME),
                serde_json::to_string(&protocol::ResumeRequest {
                    expected_generation: generation,
                })
                .expect("the resume request serialises"),
            ),
            (
                protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS),
                serde_json::to_string(&support::process_registration(
                    generation,
                    support::process_handle(),
                ))
                .expect("the process registration serialises"),
            ),
        ] {
            let response = h
                .request(
                    "POST",
                    &route,
                    Some(h.control_token.clone()),
                    vec![(
                        control::VERSION_HEADER.to_string(),
                        protocol::CONTROL_API_VERSION.to_string(),
                    )],
                    body,
                )
                .await;
            assert_eq!(
                response.status(),
                409,
                "{name} {route}: {}",
                response.body()
            );
            assert_eq!(
                response.headers_named(control::VERSION_HEADER),
                vec![protocol::CONTROL_API_VERSION.to_string().as_str()],
                "{name} {route} still identifies the response protocol"
            );
            let error: protocol::ErrorBody = response.json().expect("the conflict shape");
            assert_eq!(error.error.code, "conflict", "{name} {route}");
            assert_eq!(h.entry(), before, "{name} {route} changed no durable state");
            assert_eq!(
                std::fs::read(&chain_path).unwrap(),
                chain_before,
                "{name} {route} appended no event"
            );
        }
    }
}

/// A cancellation is a real authenticated recovery transition, not an optimistic local
/// cleanup.  The handler must accept only the current issued OpenResume generation and must
/// leave both durable authority and the trace alone for every malformed or mismatched body.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn process_cancel_accepts_only_the_current_open_resume_generation() {
    let h = support::control_harness().await;
    let resume = protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME);
    let cancel = protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS_CANCEL);
    let opened: protocol::ResumeInfo = h
        .post_json(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: h.entry().process_handoff_generation(),
            },
        )
        .await;
    let before_refusals = h.entry();
    let chain_before = support::read_chain(&h.trace_dir);
    assert_eq!(
        before_refusals.process_handoff_phase(),
        goalcup_daemon::registry::ProcessHandoffPhase::OpenResume,
        "the real resume route issued the permit this handler may cancel"
    );

    for (name, body, status, code) in [
        ("missing", String::new(), 400, "malformed"),
        (
            "unknown",
            serde_json::json!({
                "expected_generation": opened.process_generation,
                "unexpected": true,
            })
            .to_string(),
            400,
            "malformed",
        ),
        (
            "malformed",
            serde_json::json!({ "expected_generation": "not-a-generation" }).to_string(),
            400,
            "malformed",
        ),
        (
            "stale",
            serde_json::to_string(&protocol::ProcessHandoffCancelRequest {
                expected_generation: opened.process_generation - 1,
                request_digest: None,
            })
            .expect("the stale cancellation serialises"),
            409,
            "conflict",
        ),
        (
            "future",
            serde_json::to_string(&protocol::ProcessHandoffCancelRequest {
                expected_generation: opened.process_generation + 1,
                request_digest: None,
            })
            .expect("the future cancellation serialises"),
            409,
            "conflict",
        ),
    ] {
        let response = h
            .request(
                "POST",
                &cancel,
                Some(h.control_token.clone()),
                vec![(
                    control::VERSION_HEADER.to_string(),
                    protocol::CONTROL_API_VERSION.to_string(),
                )],
                body,
            )
            .await;
        assert_eq!(response.status(), status, "{name}: {}", response.body());
        assert_eq!(
            response.headers_named(control::VERSION_HEADER),
            vec![protocol::CONTROL_API_VERSION.to_string().as_str()],
            "{name} refusal carries exactly one protocol version"
        );
        let error: protocol::ErrorBody = response.json().expect("the error response shape");
        assert_eq!(error.error.code, code, "{name}");
        assert_eq!(
            h.entry(),
            before_refusals,
            "{name} changed no registry state"
        );
        assert_eq!(
            support::read_chain(&h.trace_dir),
            chain_before,
            "{name} appended no capture event"
        );
    }

    let response = h
        .post_body(
            &cancel,
            &protocol::ProcessHandoffCancelRequest {
                expected_generation: opened.process_generation,
                request_digest: None,
            },
        )
        .await;
    assert_eq!(response.status(), 200, "{}", response.body());
    assert_eq!(
        response.headers_named(control::VERSION_HEADER),
        vec![protocol::CONTROL_API_VERSION.to_string().as_str()],
        "a successful cancellation carries exactly one protocol version"
    );
    let cancelled: protocol::ProcessHandoffCancelInfo =
        response.json().expect("the cancellation response shape");
    assert_eq!(cancelled.process_generation, opened.process_generation);
    let after = h.entry();
    assert_eq!(
        after.process_handoff_phase(),
        goalcup_daemon::registry::ProcessHandoffPhase::ResumeCancelled
    );
    assert_eq!(after.last_process, before_refusals.last_process);
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
// The workspace, the arming, the process handle and the abandon.
// =========================================================================================

/// The workspace through the front door. `POST /v0/workspaces` is `workspace::prepare` plus
/// `Daemon::register`, and what comes back is everything the CLI needs to launch: the
/// status URL the participant opens, the run key its signed status verifies against, the
/// per-run receiver token, and every file the daemon rendered — so the CLI renders none
/// and the seal excludes exactly what the tracker wrote.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn creating_a_workspace_returns_everything_the_cli_needs_and_writes_nothing_twice() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;

    assert!(
        info.status_url
            .contains(&format!("?entry={}", info.entry_id)),
        "the page has no token and must not need /v0/entries/current: {}",
        info.status_url
    );
    assert!(!info.run_public_key.is_empty());
    // The daemon runs no version probe of its own, so the number the CLI's
    // preflight measured is the number on the row — and the signed status document reads it
    // from there.
    assert_eq!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the entry reads")
            .expect("it is there")
            .harness_version,
        "2.1.259",
        "the entry carries what `WorkspaceOpts.harness_version` sent"
    );
    assert!(
        info.installed_files
            .iter()
            .any(|f| f.relative_path == "settings-launch.json")
    );
    assert!(
        info.workspace_relative.is_empty(),
        "R15: the seal excludes exactly what the tracker wrote, and on Claude that is \
         nothing since the workspace settings shim went"
    );
    for f in &info.installed_files {
        let bytes = std::fs::read(&f.path).expect("the daemon wrote the file it named");
        assert_eq!(goalcup_daemon::workspace::sha256_hex(&bytes), f.sha256);
        assert_eq!(bytes.len() as u64, f.bytes);
    }
    assert!(
        std::path::Path::new(&info.artifact_root)
            .join("trace")
            .exists(),
        "`open_trace` is called at registration, so seq 0 is the baseline (C6)"
    );

    // The other clauses, as the daemon actually laid them out: a workspace holding only
    // what the tracker put there, and an artifact root that is neither inside it nor its
    // sibling.
    let workspace = std::path::Path::new(&info.workspace);
    assert!(workspace.is_dir());
    assert_eq!(
        std::fs::read_dir(workspace)
            .expect("the workspace reads")
            .count(),
        0,
        "the tracker puts nothing in a Claude workspace since its settings shim went"
    );
    let artifact_root = std::path::Path::new(&info.artifact_root);
    assert!(!artifact_root.starts_with(workspace));
    assert!(!workspace.starts_with(artifact_root));
    assert_ne!(artifact_root.parent(), workspace.parent());

    // The entry is in the daemon, bound to that workspace, and its signed status is
    // already published — `Daemon::register` publishes one so the page has something to
    // read before the first tick.
    let entries = h.daemon.entries().expect("the entry list reads");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].entry_id, info.entry_id);
    assert_eq!(entries[0].workspace, info.workspace);
    assert_eq!(entries[0].connector_id, "claude-code-cli");
    assert!(std::path::Path::new(&info.status_cache_path).is_file());
    assert!(std::path::Path::new(&info.harness_home).is_dir());

    // The one secret this body may carry is the one the agent is meant to read; the CLI's
    // bearer is not in it.
    let raw = h
        .post_body(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("codex-cli"),
        )
        .await;
    assert_eq!(raw.status(), 200);
    assert!(!raw.body().contains(&h.control_token));
}

/// Fresh registration must acquire every component beneath the configured artifact root
/// before it writes Claude's launch settings.  An attacker replacing the not-yet-created
/// `artifacts` component cannot turn the control request into a token-bearing write outside
/// that configured root.
#[cfg(unix)]
#[tokio::test]
async fn fresh_workspace_registration_refuses_a_swapped_artifacts_parent() {
    use std::os::unix::fs::symlink;

    let h = support::empty_control_harness().await;
    let outside = h.root.join("outside-artifact-parent");
    std::fs::create_dir(&outside).expect("outside directory");
    std::fs::write(outside.join("keep.txt"), b"OUTSIDE_ARTIFACT_SENTINEL")
        .expect("outside sentinel");
    symlink(&outside, h.config.artifact_root.join("artifacts"))
        .expect("the mutable artifacts component is replaced");

    let response = h
        .post_body(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;

    assert_ne!(
        response.status(),
        200,
        "a fresh entry must not follow a swapped artifacts component: {}",
        response.body()
    );
    assert_eq!(
        std::fs::read(outside.join("keep.txt")).expect("outside sentinel remains"),
        b"OUTSIDE_ARTIFACT_SENTINEL"
    );
    let outside_entries: Vec<_> = std::fs::read_dir(&outside)
        .expect("outside directory remains readable")
        .map(|entry| entry.expect("outside entry reads").file_name())
        .collect();
    assert_eq!(
        outside_entries,
        vec![std::ffi::OsString::from("keep.txt")],
        "a lexical launch-settings write would have created an entry child below outside"
    );
    assert!(
        !outside.join("settings-launch.json").exists(),
        "the outside root itself must never receive the token-bearing launch settings"
    );
    assert!(
        h.daemon.entries().expect("the live map reads").is_empty(),
        "a refused fresh authority acquisition registers no runtime"
    );
}

/// The answer is the contract's shape and nothing wider: a daemon that added a field would
/// be extending a wire type `goalcup-protocol` owns, which this crate does not do.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn the_workspace_answer_carries_exactly_the_fields_the_contract_declares() {
    let h = support::empty_control_harness().await;
    let raw = h
        .post_body(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_eq!(raw.status(), 200, "{}", raw.body());

    let on_the_wire: serde_json::Value = raw.json().expect("the answer is JSON");
    let info: protocol::WorkspaceInfo = raw.json().expect("the contract's workspace info");
    let round_tripped = serde_json::to_value(&info).expect("the contract's type serialises");
    assert_eq!(
        on_the_wire, round_tripped,
        "a field on the wire that `WorkspaceInfo` lacks is dropped in silence by a client"
    );
}

/// The two preflight refusals a caller can provoke before `create_workspace` writes
/// anything: a connector this build does not ship, and a manifest that is not the one in
/// the release. Both are `422`, and neither leaves a directory behind.
///
/// The third refusal — a connector directory that is not the pinned one — is not here,
/// because no caller can provoke it. `WorkspaceOpts` carries no directory digest: the daemon
/// hashes its own `connectors_dir` and compares that against
/// `goalcup_protocol::pins::CONNECTOR_DIR_SHA256S`, so the only way to move that comparison is to move the
/// daemon's connectors directory. `a_daemon_whose_hook_template_was_edited_refuses_to_start`
/// is that test.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_connector_this_daemon_does_not_ship_and_a_manifest_that_moved_are_both_refused() {
    let h = support::empty_control_harness().await;

    let mut unknown = support::workspace_opts("claude-code-cli");
    unknown.connector_id = "cursor-cli".to_string();
    let r = h.post_body(protocol::ROUTE_WORKSPACES, &unknown).await;
    assert_eq!(r.status(), 422);
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "unsupported_connector");

    let mut moved = support::workspace_opts("claude-code-cli");
    moved.manifest_sha256 = "ab".repeat(32);
    let r = h.post_body(protocol::ROUTE_WORKSPACES, &moved).await;
    assert_eq!(r.status(), 422);
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "preflight_failed");
    assert!(
        !r.body().contains(&moved.grant_token),
        "R12: never the grant"
    );

    assert!(
        h.daemon.entries().expect("the entry list reads").is_empty(),
        "a refused preflight leaves no entry"
    );
}

/// A daemon whose own `connectors/<id>/` holds an edited hook template refuses to start a
/// run over it, and says which digest differed.
///
/// This is not the manifest refusal said twice. The manifest digest covers exactly one file,
/// and here it is byte-for-byte the shipped one — asserted below before anything is edited.
/// What moved is a file beside it: `render_tracker_files` reads the hook bodies out of
/// `connectors/<id>/files/`, records their rendered digests as the config this entry will
/// guard, and then guards them, so an edited body is not a file that slipped past a check —
/// it becomes the file every later check agrees with.
///
/// The daemon is the one that hashes it, which is why the tamper is done to the daemon's own
/// connectors directory rather than to a field in the request. A digest supplied by the
/// process being checked checks nothing.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_daemon_whose_hook_template_was_edited_refuses_to_start() {
    let connectors = tempfile::tempdir().expect("a temporary connectors directory");
    let shipped = support::shipped_connector_dir("claude-code-cli");
    let copied = connectors.path().join("claude-code-cli");
    support::copy_tree(&shipped, &copied);

    // One byte, in a template the manifest does not hash. Appended rather than rewritten, so
    // the file is still what it was and the run would otherwise proceed exactly as before.
    let template = copied.join("files/settings-launch.json");
    let mut body = std::fs::read(&template).expect("the shipped hook template");
    body.push(b'\n');
    std::fs::write(&template, &body).expect("the edited template writes");

    let opts = support::workspace_opts("claude-code-cli");
    assert_eq!(
        opts.manifest_sha256,
        support::shipped_manifest_sha256("claude-code-cli"),
        "the manifest is the shipped one, and stays so: what moved is a file beside it"
    );
    let (_server, upload_url) = support::devserver().await;
    let mut config = support::config_at(connectors.path(), &upload_url);
    config.connectors_dir = connectors.path().to_path_buf();
    let error = match goalcup_daemon::runtime::Daemon::start(config).await {
        Ok(_) => panic!("an edited scan authority must not start"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("connector directory") || error.contains("directory digest"),
        "the refusal names the directory rather than the manifest: {error}"
    );
    assert!(!error.contains(&opts.grant_token), "R12: never the grant");
}

/// A connector directory this daemon cannot hash refuses daemon startup, before the first write.
///
/// The sibling above covers the digest that moved. This covers the digest that could not be
/// taken at all: `connector_dir_sha256` refuses a symlink rather than following it, because
/// following one would pin bytes living outside the directory. Startup must refuse rather than
/// accepting scan declarations whose connector bytes were not measured. Unix-only, because
/// planting a symlink is how the unhashable case is reached.
#[cfg(unix)]
#[tokio::test]
async fn a_connector_directory_this_daemon_cannot_hash_refuses_daemon_start() {
    let connectors = tempfile::tempdir().expect("a temporary connectors directory");
    let copied = connectors.path().join("claude-code-cli");
    support::copy_tree(&support::shipped_connector_dir("claude-code-cli"), &copied);
    std::os::unix::fs::symlink("project-settings.json", copied.join("files/alias.json"))
        .expect("the symlink plants");

    let opts = support::workspace_opts("claude-code-cli");
    let (_server, upload_url) = support::devserver().await;
    let mut config = support::config_at(connectors.path(), &upload_url);
    config.connectors_dir = connectors.path().to_path_buf();
    let error = match goalcup_daemon::runtime::Daemon::start(config).await {
        Ok(_) => panic!("an unhashable scan authority must not start"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("symlink"),
        "the refusal says the directory could not be hashed: {error}"
    );
    assert!(!error.contains(&opts.grant_token), "R12: never the grant");
}

/// The three calls that take a run from a prepared workspace to a launched harness, and the
/// one that undoes it. Arming runs the pre-arm scan first; the long poll answers `408` in
/// the gate's own words; the handle is recorded; and abandoning before the goal removes
/// the entry and its artifacts.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn arming_registering_and_abandoning_follow_the_rules_the_gate_states() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("codex-cli"),
        )
        .await;
    let route = |suffix| protocol::entry_route(&info.entry_id, suffix);
    assert_eq!(h.post(&route(control::SUFFIX_ARM)).await.status(), 204);

    // The gate is armed and the signal has not arrived: 408, in the gate's own words.
    let r = h
        .get(&format!(
            "{}?{}=50",
            route(control::SUFFIX_START_SIGNAL),
            control::QUERY_TIMEOUT_MS
        ))
        .await;
    assert_eq!(r.status(), 408);
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "arming_timeout");
    assert!(
        body.error
            .detail
            .to_string()
            .contains("first transcript record"),
        "R4's on-screen text and the API's refusal are the same words: {}",
        body.error.detail
    );
    let receiver_token = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the registry reads")
        .expect("the entry is there")
        .keys
        .receiver_token;
    for secret in [h.control_token.as_str(), receiver_token.as_str()] {
        assert!(!r.body().contains(secret));
    }

    let mut handle = support::process_handle();
    handle.native_binary = support::manifest("codex-cli").kill.native_binary;
    assert_eq!(
        h.post_body(
            &route(control::SUFFIX_PROCESS),
            &support::process_registration(info.process_generation, handle.clone()),
        )
        .await
        .status(),
        204
    );
    let entries = h.daemon.entries().expect("the entry list reads");
    assert_eq!(entries[0].entry_id, info.entry_id);
    assert_eq!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the entry reads")
            .expect("it is there")
            .last_process,
        Some(handle),
        "the handle reaches the registry, so a `goalcup seal` in another process sees it"
    );

    // Once a daemon accepted an unstarted child handoff, abandoning would orphan a process
    // it is responsible for, so V1 fails closed.
    let artifact_root = std::path::PathBuf::from(&info.artifact_root);
    let above = artifact_root.parent().expect("a parent").to_path_buf();
    assert_eq!(h.post(&route(control::SUFFIX_ABANDON)).await.status(), 409);
    // `entries()` may correctly answer `Busy` while one pump borrows the live runtime.  The
    // durable registry row below is the postcondition this refusal promises and is readable
    // without making this gate depend on that transient live-map availability.
    assert!(artifact_root.exists());
    assert!(
        above.is_dir(),
        "the refusal leaves the containing root alone"
    );
    assert!(h.config.artifact_root.is_dir());
    assert!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the entry reads")
            .is_some()
    );
    assert!(
        std::path::Path::new(&info.status_cache_path).exists(),
        "the refusal keeps the entry's signed status available for repair"
    );

    // The workspace is the participant's directory and is not the daemon's to delete: it
    // may hold work, and abandoning removes the entry.
    assert!(std::path::Path::new(&info.workspace).is_dir());
}

/// Through the control API: a harness home that already holds a session working
/// in a workspace this daemon tracks is refused before the entry moves to `Armed`, and the
/// refusal names the session so the participant can find it.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn arming_over_an_occupied_harness_home_is_a_409_that_names_the_session() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("codex-cli"),
        )
        .await;
    support::plant_rollout(
        std::path::Path::new(&info.harness_home),
        std::path::Path::new(&info.workspace),
        "the-occupying-session",
    );

    let r = h
        .post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
        .await;
    assert_eq!(r.status(), 409);
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "conflict");
    assert!(
        body.error.message.contains("the-occupying-session"),
        "{}",
        body.error.message
    );
    // The scan runs first, so a refused arming leaves the entry exactly as it was.
    let entry = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the entry reads")
        .expect("it is there");
    assert!(entry.armed_at.is_none(), "nothing armed");
    assert_eq!(entry.state, goalcup_daemon::registry::EntryState::Created);
}

/// `pgid` is set only when the launch called `setpgid`, which makes the group the leader's
/// own pid. Anything else is a number the daemon would signal a group with — so it is
/// refused as malformed rather than stored.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_process_handle_whose_pgid_is_not_its_pid_is_refused() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );

    let mut wrong = support::process_handle();
    wrong.pgid = Some(wrong.pid + 1);
    let r = h
        .post_body(
            &route,
            &support::process_registration(info.process_generation, wrong),
        )
        .await;
    assert_eq!(r.status(), 400);
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "malformed");
    assert!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the entry reads")
            .expect("it is there")
            .last_process
            .is_none(),
        "a refused handle is not stored"
    );

    // The two spellings a launch may honestly report, one per entry: an entry takes exactly
    // one handle, so the second spelling is exercised on a second workspace.
    let mut own_group = support::process_handle();
    own_group.pgid = Some(own_group.pid);
    assert_eq!(
        h.post_body(
            &route,
            &support::process_registration(info.process_generation, own_group),
        )
        .await
        .status(),
        204
    );
    let second: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_eq!(
        h.post(&protocol::entry_route(
            &second.entry_id,
            control::SUFFIX_ARM
        ))
        .await
        .status(),
        204
    );
    assert_eq!(
        h.post_body(
            &protocol::entry_route(&second.entry_id, control::SUFFIX_PROCESS),
            &support::process_registration(second.process_generation, support::process_handle())
        )
        .await
        .status(),
        204
    );

    // A body that is not a `ProcessHandle` at all is the same refusal, in the contract's
    // shape rather than in axum's.
    let junk = h
        .request(
            "POST",
            &route,
            Some(h.control_token.clone()),
            vec![(
                "x-goalcup-control-version".to_string(),
                protocol::CONTROL_API_VERSION.to_string(),
            )],
            "{\"pid\":\"not a number\"}".to_string(),
        )
        .await;
    assert_eq!(junk.status(), 400);
    let body: protocol::ErrorBody = junk.json().expect("the contract's error body");
    assert_eq!(body.error.code, "malformed");
}

/// A connector that declares no native identity cannot receive a native PID hint. The
/// manifest's `None` is an authority boundary, not a default that lets a control bearer pick
/// an arbitrary same-name process.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
#[tokio::test]
async fn process_refuses_native_pid_when_the_manifest_declares_no_native_identity() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );

    let mut root = support::spawn_harness();
    let mut unrelated = support::spawn_harness();
    let mut handle = support::handle_for_now(root.id());
    handle.native_pid = Some(unrelated.id());
    let r = h
        .post_body(
            &route,
            &support::process_registration(info.process_generation, handle),
        )
        .await;

    assert_eq!(r.status(), 400, "{}", r.body());
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "malformed");
    assert!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the entry reads")
            .expect("the entry exists")
            .last_process
            .is_none(),
        "a rejected native PID never becomes termination authority"
    );
    assert!(
        goalcup_protocol::process::is_alive(unrelated.id()),
        "validation must not signal the unrelated process"
    );
    let _ = root.kill();
    let _ = root.wait();
    let _ = unrelated.kill();
    let _ = unrelated.wait();
}

/// A legacy row can replay its exact prior authority, but an absent creation identity is
/// never sufficient to open a fresh process handoff. Otherwise a same-argv PID recycle can
/// be turned into a new termination target by a control bearer.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
#[tokio::test]
async fn process_refuses_a_new_handoff_without_a_process_instance_identity() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let mut child = support::spawn_harness();
    let mut handle = support::handle_for_now(child.id());
    handle.process_start = None;

    let response = h
        .post_body(
            &route,
            &support::process_registration(info.process_generation, handle),
        )
        .await;
    assert_eq!(response.status(), 400, "{}", response.body());
    assert!(goalcup_protocol::process::is_alive(child.id()));
    let _ = child.kill();
    let _ = child.wait();
}

/// Missing on both sides is not an identity match. A child can exit between launch and
/// admission, and `None == None` must not turn its recycled numeric PID into durable signal
/// authority.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
#[tokio::test]
async fn process_refuses_a_dead_handoff_when_both_instance_identity_reads_are_absent() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let mut child = support::spawn_harness();
    let pid = child.id();
    let mut handle = support::handle_for_now(pid);
    child.kill().unwrap();
    child.wait().unwrap();
    handle.process_start = None;
    assert_eq!(goalcup_protocol::process::process_start_identity(pid), None);

    let response = h
        .post_body(
            &route,
            &support::process_registration(info.process_generation, handle),
        )
        .await;
    assert_eq!(response.status(), 400, "{}", response.body());
    let row = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(
        row.process_handoff_phase(),
        goalcup_daemon::registry::ProcessHandoffPhase::OpenInitial
    );
    assert!(row.last_process.is_none());
}

/// A legacy Prepared row is not yet an acknowledgement: promoting it would grant signal
/// authority without an instance identity. It remains durable and ambiguous instead of
/// accepting a recycled numeric PID.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn process_keeps_an_exact_legacy_prepared_replay_ambiguous() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let registration = goalcup_protocol::process::ProcessRegistration {
        generation: info.process_generation,
        handle: support::entry::recorded_launch_handle(),
    };
    h.daemon
        .registry()
        .prepare_process_handoff(&info.entry_id, registration.clone())
        .expect("the historical request was durably prepared before its response was lost");

    let response = h.post_body(&route, &registration).await;
    assert_eq!(response.status(), 500, "{}", response.body());
    let body: protocol::ErrorBody = response.json().unwrap();
    assert_eq!(body.error.code, "process_admission_uncertain");
    let row = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(
        row.process_handoff_phase(),
        goalcup_daemon::registry::ProcessHandoffPhase::Prepared
    );
    assert!(row.last_process.is_none());
}

/// A restart that cannot revalidate Prepared authority owns no capture runtime. It must
/// withhold the previous daemon's cached Online document rather than advertise recording
/// that is no longer happening.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn restart_withholds_stale_status_when_prepared_authority_cannot_be_revalidated() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    assert!(h.daemon.signed_status(&info.entry_id).unwrap().is_some());
    let registration = goalcup_protocol::process::ProcessRegistration {
        generation: info.process_generation,
        handle: support::entry::recorded_launch_handle(),
    };
    h.daemon
        .registry()
        .prepare_process_handoff(&info.entry_id, registration)
        .unwrap();

    let h = support::restart_empty_control(h).await;
    assert!(h.daemon.entries().unwrap().is_empty());
    assert_eq!(h.daemon.signed_status(&info.entry_id).unwrap(), None);
    let response = h
        .get_no_token(&protocol::entry_route(
            &info.entry_id,
            control::SUFFIX_STATUS,
        ))
        .await;
    assert_eq!(response.status(), 404, "{}", response.body());
    let row = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(
        row.process_handoff_phase(),
        goalcup_daemon::registry::ProcessHandoffPhase::Prepared
    );
    assert!(row.last_process.is_none());
    let cycles = h.daemon.cycle_errors();
    assert!(cycles.tick > 0);
    assert!(cycles.last.unwrap_or_default().contains(&info.entry_id));
}

/// The same exact replay must also work after seal/restart has evicted the runtime. This
/// takes the early legacy-replay branch, whose first `register_process` answers Missing and
/// must fall through to the registry-only sealed acknowledgement rather than returning 404.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn process_acknowledges_an_exact_replay_after_seal_eviction() {
    let h = support::control_harness().await;
    let registration = goalcup_protocol::process::ProcessRegistration {
        generation: h.entry().process_handoff_generation(),
        handle: h
            .entry()
            .last_process
            .clone()
            .expect("the started fixture registered its launch"),
    };
    let entry_id = h.entry_id.clone();
    let _: goalcup_protocol::seal::SealManifest = h
        .post_json(&protocol::entry_route(&entry_id, control::SUFFIX_SEAL), &())
        .await;
    let h = support::restart_control(h).await;
    let response = h
        .post_body(
            &protocol::entry_route(&entry_id, control::SUFFIX_PROCESS),
            &registration,
        )
        .await;
    assert_eq!(response.status(), 204, "{}", response.body());
}

/// An unsealed Registered row still denotes a live harness that must have a runtime watching
/// it.  Durable idempotency proves the digest, but it does not prove monitoring survived a
/// failed rehydrate; registry-only acknowledgement is reserved for terminal sealed rows.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn an_unsealed_exact_registered_replay_is_ambiguous_while_runtime_is_unavailable() {
    let h = support::control_harness().await;
    let entry = h.entry();
    let registration = goalcup_protocol::process::ProcessRegistration {
        generation: entry.process_handoff_generation(),
        handle: entry
            .last_process
            .clone()
            .expect("the fixture has registered termination authority"),
    };
    let before = entry;
    std::fs::remove_file(h.trace_dir.join("chain.jsonl"))
        .expect("the crash fixture makes rehydration impossible");
    let h = support::restart_control(h).await;
    assert!(
        h.daemon.entries().unwrap().is_empty(),
        "the successor owns no runtime that could monitor this unsealed harness"
    );

    let response = h
        .post_body(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS),
            &registration,
        )
        .await;
    assert_eq!(response.status(), 500, "{}", response.body());
    let body: protocol::ErrorBody = response.json().expect("the ambiguity contract body");
    assert_eq!(body.error.code, "process_admission_uncertain");
    assert_eq!(h.entry(), before, "the replay is registry-read-only");
}

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

/// `/arm` is a real V1 handler under `v1-compat`; it must identify recovered v5 authority before
/// looking for a V1 runtime or running the pre-arm scan.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_reopened_capture_v1_arm_refuses_409_without_mutation() {
    let h = support::empty_control_harness().await;
    let info = create_capture(&h, "v1-arm").await;
    let h = support::restart_empty_control(h).await;
    let before = reopened_capture_snapshot(&h, &info.entry_id);

    let response = h
        .post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
        .await;
    assert_eq!(response.status(), 409, "{}", response.body());
    let error: protocol::ErrorBody = response.json().expect("the conflict response shape");
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
        "arm changes no retained capture state"
    );
}

/// `/process/cancel` checks recovered-capture authority before parsing or entering its V1 runtime
/// path. A valid cancellation body still receives the same capture-specific `409` and no state
/// changes.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_reopened_capture_v1_process_cancel_refuses_409_without_mutation() {
    let h = support::empty_control_harness().await;
    let info = create_capture(&h, "v1-process-cancel").await;
    let h = support::restart_empty_control(h).await;
    let before = reopened_capture_snapshot(&h, &info.entry_id);

    let response = h
        .post_body(
            &protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS_CANCEL),
            &protocol::ProcessHandoffCancelRequest {
                expected_generation: info.process_generation,
                request_digest: None,
            },
        )
        .await;
    assert_eq!(response.status(), 409, "{}", response.body());
    let error: protocol::ErrorBody = response.json().expect("the conflict response shape");
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
        "process cancellation changes no retained capture state"
    );
}

/// Prepared is not success yet: it still needs the live writer to reconcile the process
/// event before advancing to Registered. If that runtime is temporarily unavailable, the
/// daemon must preserve the exact Prepared row and keep its transient `busy` contract so the
/// client continues retrying the one immutable request.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn an_exact_prepared_replay_stays_busy_while_the_runtime_is_unavailable() {
    use goalcup_daemon::registry::ProcessHandoffPhase;

    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let registration = support::process_registration(
        info.process_generation,
        support::entry::recorded_launch_handle(),
    );
    h.daemon
        .registry()
        .prepare_process_handoff(&info.entry_id, registration.clone())
        .expect("the original request reached durable Prepared");
    let before = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    let _held = h
        .daemon
        .borrow_entry(&info.entry_id)
        .expect("the runtime is unavailable for reconciliation");

    let response = h
        .post_body(
            &protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS),
            &registration,
        )
        .await;
    assert_eq!(response.status(), 409, "{}", response.body());
    let body: protocol::ErrorBody = response.json().expect("the busy contract body");
    assert_eq!(body.error.code, "process_admission_busy");
    let after = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(
        after, before,
        "an unavailable writer does not mutate Prepared"
    );
    assert_eq!(after.process_handoff_phase(), ProcessHandoffPhase::Prepared);
}

/// Preparing records authority but does not freeze a PID against reuse.  An exact replay
/// must prove the stored root still names the same OS process immediately before promotion;
/// otherwise the durable row stays Prepared and the wire answer remains ambiguous.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn an_exact_prepared_replay_never_promotes_a_stale_process_instance() {
    use goalcup_daemon::registry::ProcessHandoffPhase;

    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let mut handle = support::entry::recorded_launch_handle();
    handle.process_start = Some("the original process instance is gone".to_string());
    let registration = support::process_registration(info.process_generation, handle);
    h.daemon
        .registry()
        .prepare_process_handoff(&info.entry_id, registration.clone())
        .expect("the original request reached durable Prepared while the child was live");
    let before = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();

    let response = h
        .post_body(
            &protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS),
            &registration,
        )
        .await;
    assert_eq!(response.status(), 500, "{}", response.body());
    let body: protocol::ErrorBody = response.json().expect("the ambiguity contract body");
    assert_eq!(body.error.code, "process_admission_uncertain");
    let after = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(after.process_handoff_phase(), ProcessHandoffPhase::Prepared);
    assert_eq!(after.last_process, None);
    assert_eq!(after, before, "failed revalidation is registry-read-only");
}

#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn an_exact_prepared_replay_never_promotes_a_stale_native_descendant() {
    use goalcup_daemon::registry::ProcessHandoffPhase;

    let fixture_id = "stale-native-prepared-fixture";
    let connectors = tempfile::tempdir().expect("a temporary connectors directory");
    let copied = connectors.path().join(fixture_id);
    support::copy_tree(&support::shipped_connector_dir("claude-code-cli"), &copied);
    let manifest_path = copied.join("manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .expect("the fixture manifest reads")
        .replace(
            "id = \"claude-code-cli\"",
            &format!("id = \"{fixture_id}\""),
        )
        .replace(
            "harness = \"claude-code-cli\"",
            &format!("harness = \"{fixture_id}\""),
        )
        .replace(
            "strategy = \"process_group\"\ngrace_seconds",
            "strategy = \"native_pid\"\nnative_binary = \"sleep\"\ngrace_seconds",
        );
    let foreign_scan = manifest
        .find("[[foreign_scan]]")
        .expect("the copied Claude manifest declares foreign scans");
    let redaction = manifest
        .find("[redaction]")
        .expect("the copied manifest resumes after foreign scans");
    let manifest = format!("{}{}", &manifest[..foreign_scan], &manifest[redaction..]);
    std::fs::write(&manifest_path, manifest).expect("the fixture manifest writes");
    let h = support::empty_control_harness_with(|config| {
        config.connectors_dir = connectors.path().to_path_buf();
    })
    .await;
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.connector_id = fixture_id.to_string();
    opts.manifest_sha256 = goalcup_daemon::workspace::sha256_hex(
        &std::fs::read(&manifest_path).expect("the fixture manifest rereads"),
    );
    let info: protocol::WorkspaceInfo = h.post_json(protocol::ROUTE_WORKSPACES, &opts).await;
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let script = "sleep 30 & wait";
    let mut root = std::process::Command::new("/bin/sh")
        .args(["-c", script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the live wrapper starts");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let native_pid = loop {
        if let Ok(table) = goalcup_protocol::process::snapshot()
            && let Some(pid) = goalcup_protocol::process::find_named(root.id(), "sleep", &table)
        {
            break pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the live native descendant appeared"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let real_native_start = goalcup_protocol::process::process_start_identity(native_pid)
        .expect("the live native identity is readable");
    let mut handle = support::handle_for_now(root.id());
    handle.program = "/bin/sh".to_string();
    handle.argv = vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()];
    handle.native_pid = Some(native_pid);
    handle.native_binary = Some("sleep".to_string());
    handle.native_process_start = Some(format!("{real_native_start}-stale"));
    assert_eq!(
        goalcup_protocol::process::process_start_identity(root.id()),
        handle.process_start,
        "root instance validation passes"
    );
    let table = goalcup_protocol::process::snapshot().expect("the live tree snapshots");
    assert!(
        goalcup_protocol::process::native_pid_is_bound_to_handle(&handle, &table),
        "native name and descendant lineage validation pass"
    );
    assert_ne!(
        goalcup_protocol::process::process_start_identity(native_pid),
        handle.native_process_start,
        "only the stored native creation identity is stale"
    );
    let validation = goalcup_protocol::process::validate_live_process_handle(&handle)
        .expect_err("the stale native creation identity is the rejecting predicate");
    assert!(validation.contains("native_process_start"), "{validation}");
    let mut corrected = handle.clone();
    corrected.native_process_start = Some(real_native_start);
    goalcup_protocol::process::validate_live_process_handle(&corrected)
        .expect("the same live root and native lineage pass with the exact native identity");
    let registration = support::process_registration(info.process_generation, handle);
    h.daemon
        .registry()
        .prepare_process_handoff(&info.entry_id, registration.clone())
        .expect("the child was valid when the original request prepared it");
    let before = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();

    let response = h
        .post_body(
            &protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS),
            &registration,
        )
        .await;
    assert_eq!(response.status(), 500, "{}", response.body());
    let body: protocol::ErrorBody = response.json().expect("the ambiguity contract body");
    assert_eq!(body.error.code, "process_admission_uncertain");
    let after = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(after.process_handoff_phase(), ProcessHandoffPhase::Prepared);
    assert_eq!(after.last_process, None);
    assert_eq!(
        after, before,
        "native identity refusal is registry-read-only"
    );

    let _ = root.kill();
    let _ = root.wait();
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &native_pid.to_string()])
        .status();
}

/// Once the registry has durably recorded this exact generation and digest as Prepared,
/// no reconciliation error may look like a settled rejection on the wire.  A CLI that saw
/// a 4xx would kill the child and discard the only exact body capable of replaying this
/// state.  The writer refusal is injected at the first HomeSeeded append, after Prepare.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_post_prepare_refusal_is_ambiguous_and_the_exact_request_remains_replayable() {
    use goalcup_daemon::registry::ProcessHandoffPhase;

    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let process = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let mut handle = support::process_handle();
    handle.home_seed = Some(goalcup_protocol::events::HomeSeedRecord {
        wire_layout: Default::default(),
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "claude-code-cli".to_string(),
        from: "{home}/.claude.json".to_string(),
        keys: Vec::new(),
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
    });
    let registration = support::process_registration(info.process_generation, handle);
    let mut held = h
        .daemon
        .borrow_entry(&info.entry_id)
        .expect("the runtime is held for fault injection");
    held.get()
        .expect("the runtime is present")
        .chain_writer_mut()
        .expect("the chain is open")
        .refuse_append_after(0);
    drop(held);

    let failed = h.post_body(&process, &registration).await;
    assert_eq!(failed.status(), 500, "{}", failed.body());
    let body: protocol::ErrorBody = failed.json().expect("the contract error body");
    assert_eq!(body.error.code, "process_admission_uncertain");
    assert_eq!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .unwrap()
            .unwrap()
            .process_handoff_phase(),
        ProcessHandoffPhase::Prepared,
        "the exact child and digest remain durable after reconciliation refuses"
    );
    assert!(
        h.daemon
            .registry()
            .is_exact_accepted_process_replay(&info.entry_id, &registration)
            .unwrap(),
        "the failed response leaves the byte-identical replay eligible"
    );
    assert_eq!(
        h.post_body(&process, &registration).await.status(),
        204,
        "the one-shot refusal clears and the exact Prepared replay reconciles"
    );
}

/// `DaemonError::Json` normally maps to the same 400 `malformed` used for request parsing.
/// Corrupting the independently scanned chain makes that error arise only after Prepare;
/// restoring the pre-request bytes then proves the retained exact body can finish.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_post_prepare_json_error_is_not_a_malformed_request_refusal() {
    use goalcup_daemon::registry::ProcessHandoffPhase;
    use std::io::{Seek, SeekFrom, Write};

    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let process = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let mut handle = support::process_handle();
    handle.home_seed = Some(goalcup_protocol::events::HomeSeedRecord {
        wire_layout: Default::default(),
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "claude-code-cli".to_string(),
        from: "{home}/.claude.json".to_string(),
        keys: Vec::new(),
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
    });
    let registration = support::process_registration(info.process_generation, handle);
    let chain = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .unwrap()
        .unwrap()
        .trace_dir
        .join(goalcup_protocol::export::CHAIN_FILE);
    let clean_len = std::fs::metadata(&chain).unwrap().len();
    let mut raw = std::fs::OpenOptions::new()
        .append(true)
        .open(&chain)
        .expect("the test corrupts the independent replay scan");
    raw.write_all(b"{not-json}\n").unwrap();
    drop(raw);

    let failed = h.post_body(&process, &registration).await;
    assert_eq!(failed.status(), 500, "{}", failed.body());
    let body: protocol::ErrorBody = failed.json().expect("the contract error body");
    assert_eq!(body.error.code, "process_admission_uncertain");
    assert_eq!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .unwrap()
            .unwrap()
            .process_handoff_phase(),
        ProcessHandoffPhase::Prepared
    );

    let mut restored = std::fs::OpenOptions::new()
        .write(true)
        .open(&chain)
        .expect("the chain reopens for restoration");
    restored.set_len(clean_len).unwrap();
    restored.seek(SeekFrom::Start(clean_len)).unwrap();
    drop(restored);
    assert_eq!(
        h.post_body(&process, &registration).await.status(),
        204,
        "the exact Prepared replay remains capable of reconciliation"
    );
}

/// The fail-closed check is not a blanket ban on native-PID connectors: the native process
/// may be the registered root itself when the launcher execs into it.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
#[tokio::test]
async fn process_accepts_a_native_pid_the_snapshot_proves_is_the_registered_root() {
    let fixture_id = "native-pid-fixture";
    let connectors = tempfile::tempdir().expect("a temporary connectors directory");
    let copied = connectors.path().join(fixture_id);
    support::copy_tree(&support::shipped_connector_dir("claude-code-cli"), &copied);
    let manifest_path = copied.join("manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).expect("the fixture manifest reads");
    let manifest = manifest
        .replace(
            "id = \"claude-code-cli\"",
            &format!("id = \"{fixture_id}\""),
        )
        .replace(
            "harness = \"claude-code-cli\"",
            &format!("harness = \"{fixture_id}\""),
        )
        .replace(
            "strategy = \"process_group\"\ngrace_seconds",
            "strategy = \"native_pid\"\nnative_binary = \"sleep\"\ngrace_seconds",
        );
    // A test-only connector ID is intentionally outside the release directory-digest pins.
    // Remove Claude's foreign-scan declarations rather than pretending a modified scanner
    // has release authority; termination admission does not need scanners.
    let foreign_scan = manifest
        .find("[[foreign_scan]]")
        .expect("the copied Claude manifest declares foreign scans");
    let redaction = manifest
        .find("[redaction]")
        .expect("the copied Claude manifest resumes after foreign scans");
    let manifest = format!("{}{}", &manifest[..foreign_scan], &manifest[redaction..]);
    std::fs::write(&manifest_path, manifest).expect("the fixture manifest writes");
    let h = support::empty_control_harness_with(|config| {
        config.connectors_dir = connectors.path().to_path_buf();
    })
    .await;
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.connector_id = fixture_id.to_string();
    opts.manifest_sha256 = goalcup_daemon::workspace::sha256_hex(
        &std::fs::read(&manifest_path).expect("the fixture manifest rereads"),
    );
    let info: protocol::WorkspaceInfo = h.post_json(protocol::ROUTE_WORKSPACES, &opts).await;
    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );

    let mut root = support::spawn_harness();
    let mut handle = support::handle_for_now(root.id());
    handle.program = "/bin/sh".to_string();
    handle.native_pid = Some(root.id());
    handle.native_process_start = goalcup_protocol::process::process_start_identity(root.id());
    assert_eq!(
        h.post_body(
            &route,
            &support::process_registration(info.process_generation, handle.clone()),
        )
        .await
        .status(),
        400,
        "a native PID cannot fall back to its wrapper program identity"
    );
    handle.native_binary = Some("sleep".to_string());
    let mut unrelated = support::spawn_harness();
    handle.native_pid = Some(unrelated.id());
    handle.native_process_start = goalcup_protocol::process::process_start_identity(unrelated.id());
    assert_eq!(
        h.post_body(
            &route,
            &support::process_registration(info.process_generation, handle.clone()),
        )
        .await
        .status(),
        400,
        "the manifest identity still needs current root/descendant lineage"
    );
    assert!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the entry reads")
            .expect("the entry exists")
            .last_process
            .is_none(),
        "the rejected same-name stranger never becomes termination authority"
    );
    assert!(
        goalcup_protocol::process::is_alive(unrelated.id()),
        "admission must never signal the same-name stranger"
    );
    handle.native_pid = Some(root.id());
    handle.native_process_start = goalcup_protocol::process::process_start_identity(root.id());
    assert_eq!(
        h.post_body(
            &route,
            &support::process_registration(info.process_generation, handle.clone()),
        )
        .await
        .status(),
        204
    );
    assert_eq!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the entry reads")
            .expect("the entry exists")
            .last_process,
        Some(handle),
        "the validated root remains the entry's termination authority"
    );
    let _ = root.kill();
    let _ = root.wait();
    let _ = unrelated.kill();
    let _ = unrelated.wait();
}

/// V3 has no compatibility fallback for a direct mutating caller.  The exact control
/// version and the closed request documents are checked before any route can prepare a
/// handoff, catch up a tail, or consume a generation.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn mutating_handoff_routes_require_the_exact_version_and_closed_typed_documents() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let process = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    let before = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the row reads")
        .expect("the row exists");
    let chain_before = std::fs::read(before.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
        .expect("the baseline exists");
    let registration =
        support::process_registration(info.process_generation, support::process_handle());

    let missing_version = h
        .request(
            "POST",
            &process,
            Some(h.control_token.clone()),
            Vec::new(),
            serde_json::to_string(&registration).expect("the registration serialises"),
        )
        .await;
    assert_eq!(missing_version.status(), 426, "{}", missing_version.body());

    // A V2 client does not preserve V3's cache cleanup latch. The daemon rejects it before
    // a mutating handler can consume or prepare any authority.
    let previous_v2 = h
        .request(
            "POST",
            &process,
            Some(h.control_token.clone()),
            vec![(control::VERSION_HEADER.to_string(), "2".to_string())],
            serde_json::to_string(&registration).expect("the registration serialises"),
        )
        .await;
    assert_eq!(previous_v2.status(), 426, "{}", previous_v2.body());

    let bare_handle = h.post_body(&process, &registration.handle).await;
    assert_eq!(bare_handle.status(), 400, "{}", bare_handle.body());
    let mut nested_unknown =
        serde_json::to_value(&registration).expect("the registration serialises");
    nested_unknown["handle"]["unexpected"] = serde_json::json!(true);
    let nested_unknown = h.post_body(&process, &nested_unknown).await;
    assert_eq!(
        nested_unknown.status(),
        400,
        "nested ProcessHandle fields are part of the closed V1 document: {}",
        nested_unknown.body()
    );
    let unknown_registration = h
        .post_body(
            &process,
            &serde_json::json!({
                "generation": info.process_generation,
                "handle": registration.handle,
                "unexpected": true,
            }),
        )
        .await;
    assert_eq!(
        unknown_registration.status(),
        400,
        "{}",
        unknown_registration.body()
    );
    let after = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the row reads")
        .expect("the row remains");
    assert_eq!(after, before, "all process-wire refusals are zero-mutation");
    assert_eq!(
        std::fs::read(after.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
        chain_before,
        "the rejected seed/body never reached HomeSeeded"
    );

    let started = support::control_harness().await;
    let resume = protocol::entry_route(&started.entry_id, control::SUFFIX_RESUME);
    let row_before = started.entry();
    let resume_chain_before = support::read_chain(&started.trace_dir);
    let empty = started.post_body(&resume, &serde_json::json!({})).await;
    assert_eq!(empty.status(), 400, "{}", empty.body());
    let unknown = started
        .post_body(
            &resume,
            &serde_json::json!({ "expected_generation": 1, "unexpected": true }),
        )
        .await;
    assert_eq!(unknown.status(), 400, "{}", unknown.body());
    let stale = started
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: 0,
            },
        )
        .await;
    assert_eq!(stale.status(), 409, "{}", stale.body());

    let previous_v2_resume = started
        .request(
            "POST",
            &resume,
            Some(started.control_token.clone()),
            vec![(control::VERSION_HEADER.to_string(), "2".to_string())],
            serde_json::to_string(&protocol::ResumeRequest {
                expected_generation: row_before.process_handoff_generation(),
            })
            .unwrap(),
        )
        .await;
    assert_eq!(
        previous_v2_resume.status(),
        426,
        "{}",
        previous_v2_resume.body()
    );
    let previous_v2_cancel = started
        .request(
            "POST",
            &protocol::entry_route(&started.entry_id, control::SUFFIX_PROCESS_CANCEL),
            Some(started.control_token.clone()),
            vec![(control::VERSION_HEADER.to_string(), "2".to_string())],
            serde_json::to_string(&protocol::ProcessHandoffCancelRequest {
                expected_generation: row_before.process_handoff_generation(),
                request_digest: None,
            })
            .unwrap(),
        )
        .await;
    assert_eq!(
        previous_v2_cancel.status(),
        426,
        "{}",
        previous_v2_cancel.body()
    );
    assert_eq!(
        started.entry(),
        row_before,
        "bad resume documents and V2 callers consume no generation"
    );
    assert_eq!(
        support::read_chain(&started.trace_dir),
        resume_chain_before,
        "bad resume documents do not catch up"
    );
}

/// The other half: after the goal is stamped the run has a signed receipt, a chained
/// baseline and a deadline the server witnessed, and none of that is the daemon's to
/// withdraw. The refusal is a `409` and it removes nothing.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn abandoning_a_started_entry_is_refused_and_takes_nothing_with_it() {
    let h = support::control_harness().await;
    let r = h
        .post(&protocol::entry_route(&h.entry_id, control::SUFFIX_ABANDON))
        .await;
    assert_eq!(r.status(), 409);
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "conflict");

    assert!(h.artifact_dir.is_dir(), "nothing was removed");
    assert!(h.trace_dir.join("chain.jsonl").is_file());
    assert!(
        h.daemon
            .entries()
            .expect("the entry list reads")
            .iter()
            .any(|e| e.entry_id == h.entry_id)
    );
}

/// Every one of these routes is behind the bearer, and an id this daemon does not hold
/// is a `404` in the contract's shape on each of them.
#[tokio::test]
async fn task_21_s_routes_need_the_bearer_and_answer_404_for_an_entry_that_is_not_here() {
    let h = support::empty_control_harness().await;
    for path in [
        protocol::ROUTE_WORKSPACES.to_string(),
        protocol::entry_route("no-such-entry", control::SUFFIX_ARM),
        protocol::entry_route("no-such-entry", control::SUFFIX_PROCESS),
        protocol::entry_route("no-such-entry", control::SUFFIX_ABANDON),
        protocol::entry_route("no-such-entry", control::SUFFIX_START_SIGNAL),
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

    // Each route with the body it actually takes: `arm` and `abandon` take none, and
    // `process` takes the handle. An id this daemon does not hold is the same 404 on all
    // three, and the refusal names no path — the caller knows what it asked for.
    for r in [
        h.post(&protocol::entry_route("no-such-entry", control::SUFFIX_ARM))
            .await,
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
    let r = h
        .get(&protocol::entry_route(
            "no-such-entry",
            control::SUFFIX_START_SIGNAL,
        ))
        .await;
    assert_eq!(r.status(), 404);
}

/// The long poll takes one parameter. A query it does not know is a request the daemon
/// cannot honour as written, and answering it as if the parameter had been absent would be
/// a timeout the caller never asked for.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn the_start_signal_poll_refuses_a_query_it_does_not_know() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_START_SIGNAL);

    for query in ["forever=1", "timeout_ms=soon", "timeout_ms=1&extra=2"] {
        let r = h.get(&format!("{route}?{query}")).await;
        assert_eq!(r.status(), 400, "{query}");
        let body: protocol::ErrorBody = r.json().expect("the contract's error body");
        assert_eq!(body.error.code, "malformed");
    }
}

/// The long poll's other answer. Claude's whole arming signal is
/// `launch+first_transcript_record` (D16), so the first record the harness writes is the
/// signal: one tick arms, corroborates and starts, and the poll then reports what the gate
/// saw rather than a timeout.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn the_start_signal_poll_reports_what_the_gate_actually_saw() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_START_SIGNAL);
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    assert_eq!(
        h.post_body(
            &protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS),
            &support::process_registration(info.process_generation, support::process_handle()),
        )
        .await
        .status(),
        204,
        "the launch process is handed over before its first transcript can start the run"
    );

    // The harness's first transcript record, in the per-run home the daemon created and in
    // the session the daemon pinned into the launch recipe.
    let workspace = std::path::PathBuf::from(&info.workspace);
    support::append_line(
        &support::transcript_path(
            std::path::Path::new(&info.harness_home),
            &workspace,
            &info.session_id,
        ),
        &support::transcript_line(&workspace, &info.session_id, "the goal"),
    );
    h.daemon.tick().await.expect("the first tick runs");

    let r = h.get(&route).await;
    assert_eq!(r.status(), 200, "{}", r.body());
    let seen: protocol::StartSignal = r.json().expect("the contract's start signal");
    assert_eq!(seen.entry_id, info.entry_id);
    assert_eq!(
        seen.signal,
        support::manifest("claude-code-cli").arming.signal,
        "the connector's own word, echoed back"
    );
    assert_eq!(seen.session_id.as_deref(), Some(info.session_id.as_str()));
    assert_eq!(seen.source, "leg1", "leg 1 witnessed it");
    assert!(goalcup_protocol::control::parse_z(&seen.observed_at).is_some());
    let receiver_token = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the registry reads")
        .expect("the entry is there")
        .keys
        .receiver_token;
    for secret in [h.control_token.as_str(), receiver_token.as_str()] {
        assert!(!r.body().contains(secret));
    }

    // And a started run is one abandon may not undo, whichever route stamped it.
    let refused = h
        .post(&protocol::entry_route(
            &info.entry_id,
            control::SUFFIX_ABANDON,
        ))
        .await;
    assert_eq!(refused.status(), 409);
    assert!(std::path::Path::new(&info.artifact_root).is_dir());
}

// =========================================================================================
// The reads and the two calls that end a run.
// =========================================================================================

/// The one signed document, and the one call `goalcup doctor` makes.
///
/// `status` is public, so what it may not carry is the whole point: neither the CLI's
/// bearer nor D24's per-run receiver token appears anywhere in it. The two other
/// assertions are the ones a page acts on — a run's remaining time is inside the 168 hours
/// the rules give it, and the document a poll just fetched is not already stale.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn the_status_route_is_the_one_signed_document_and_health_counts_what_is_open() {
    let h = support::control_harness().await;
    let path = protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS);
    let signed: protocol::SignedStatus = h
        .get_no_token(&path)
        .await
        .json()
        .expect("the contract's signed status");
    assert!(signed.verify_with(&h.run_public_key).is_ok());
    let raw = serde_json::to_string(&signed).expect("it re-serialises");
    for secret in [h.receiver_token.as_str(), h.control_token.as_str()] {
        assert!(
            !raw.contains(secret),
            "the public document carries a secret"
        );
    }
    assert!(
        signed
            .payload
            .seconds_remaining
            .expect("a started run has a deadline")
            <= 168 * 3_600
    );
    assert!(
        signed
            .stale_seconds(&goalcup_daemon::now_rfc3339())
            .expect("both timestamps parse")
            < protocol::STALE_AFTER_SECONDS,
        "a document the poll just fetched is not stale"
    );

    let health: protocol::DaemonHealth = h
        .get(protocol::ROUTE_HEALTH)
        .await
        .json()
        .expect("the contract's health document");
    assert_eq!(health.control_api_version, protocol::CONTROL_API_VERSION);
    assert_eq!(health.entries_open, 1);
}

/// `GET /v0/entries` and `GET /v0/entries/current`: the rows `goalcup entries` prints and
/// the entry a bare `goalcup status` means.
///
/// Both are authenticated and neither carries a secret — a directory of the participant's
/// workspaces is not a public document, and it is not a place either token belongs either.
/// The seal is what closes the pointer: a `current` naming a run that has ended would send
/// a bare `goalcup status` into a chain that is already closed.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn the_entries_list_and_the_current_pointer_are_what_goalcup_entries_prints() {
    let h = support::control_harness().await;

    let r = h.get(protocol::ROUTE_ENTRIES).await;
    assert_eq!(r.status(), 200, "{}", r.body());
    for secret in [h.receiver_token.as_str(), h.control_token.as_str()] {
        assert!(!r.body().contains(secret), "an entry row carries a secret");
    }
    let rows: Vec<protocol::EntrySummary> = r.json().expect("the contract's rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entry_id, h.entry_id);
    assert_eq!(rows[0].connector_id, "claude-code-cli");
    assert!(rows[0].started_at.is_some(), "the harness's entry started");
    assert_eq!(rows[0].sealed_at, None);

    let current: Option<String> = h
        .get(protocol::ROUTE_ENTRIES_CURRENT)
        .await
        .json()
        .expect("the contract's current pointer");
    assert_eq!(current.as_deref(), Some(h.entry_id.as_str()));

    let _: goalcup_protocol::seal::SealManifest = h
        .post_json(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL),
            &(),
        )
        .await;
    let current: Option<String> = h
        .get(protocol::ROUTE_ENTRIES_CURRENT)
        .await
        .json()
        .expect("the contract's current pointer");
    assert_eq!(current, None, "a sealed run is not the current one");
    let rows: Vec<protocol::EntrySummary> = h
        .get(protocol::ROUTE_ENTRIES)
        .await
        .json()
        .expect("the contract's rows");
    assert!(rows[0].sealed_at.is_some(), "the row says the run ended");
}

/// A transient terminal-marker persistence failure is an API truth boundary, not merely a
/// status-file retry.  The authenticated entries directory must not summarize a live sealed
/// row as healthy while the daemon has deliberately withheld its all-clear document.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn entries_list_marks_a_live_withheld_terminal_row_for_recovery() {
    let h = support::control_harness().await;
    h.post_json::<_, goalcup_protocol::seal::SealManifest>(
        &protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL),
        &(),
    )
    .await;
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .fail_next_terminal_drain_quarantine_persistence_for_test();
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the impossible ACK is staged");
    h.daemon
        .pump_all()
        .await
        .expect_err("the terminal marker persistence failure reaches the cadence");

    let rows: Vec<protocol::EntrySummary> = h
        .get(protocol::ROUTE_ENTRIES)
        .await
        .json()
        .expect("the authenticated rows parse");
    let row = rows
        .iter()
        .find(|row| row.entry_id == h.entry_id)
        .expect("the live withheld entry remains listed");
    assert_eq!(row.arming, protocol::ArmingState::Started);
    assert_eq!(
        row.health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded
    );
}

/// The same public summary rule applies once a fully acknowledged row has left the live
/// map and is owned only by the retained terminal drain.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn entries_list_marks_a_retained_withheld_terminal_row_for_recovery() {
    let h = support::control_harness().await;
    h.post_json::<_, goalcup_protocol::seal::SealManifest>(
        &protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL),
        &(),
    )
    .await;
    h.daemon
        .pump_all()
        .await
        .expect("the live terminal drain reaches its first completion");
    let terminal = h.entry().chain_seq.expect("the terminal sequence");
    h.daemon
        .registry()
        .update(&h.entry_id, |entry| entry.last_ack_seq = Some(terminal + 1))
        .expect("the retained row receives the impossible ACK");
    h.daemon
        .fail_next_terminal_drain_quarantine_persistence_for_test();
    let error = h
        .daemon
        .pump_all()
        .await
        .expect_err("the retained terminal marker persistence failure reaches the cadence");
    assert!(error.to_string().contains("persistence"), "{error}");
    assert!(
        h.daemon.terminal_status_withheld_for_test(&h.entry_id),
        "the retained persistence failure installs its process-local latch"
    );

    let rows: Vec<protocol::EntrySummary> = h
        .get(protocol::ROUTE_ENTRIES)
        .await
        .json()
        .expect("the authenticated rows parse");
    let row = rows
        .iter()
        .find(|row| row.entry_id == h.entry_id)
        .unwrap_or_else(|| panic!("the retained withheld entry remains listed: {rows:?}"));
    assert_eq!(row.arming, protocol::ArmingState::Started);
    assert_eq!(
        row.health,
        goalcup_protocol::events::HealthState::SealingRecoveryNeeded
    );
}

/// Resume and seal through the front door.
///
/// `resume` polls first and reports the count, because a relaunch over an un-captured tail
/// loses exactly the records a kill is most likely to have stranded; then it says what a
/// relaunch needs. `seal` is the participant's early finish and calls the same code the
/// deadline calls, so the two produce the same bytes in the same order — and a second seal
/// is a conflict, because `EventBody::Seal` is the chain's last link and there is no
/// second last link.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn resume_reports_what_a_relaunch_needs_and_seal_is_the_same_code_the_deadline_runs() {
    let h = support::control_harness().await;
    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, "s1", "the tail the kill stranded"),
    );

    let resume_route = protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME);
    let r = h
        .post_body(
            &resume_route,
            &protocol::ResumeRequest {
                expected_generation: 1,
            },
        )
        .await;
    assert_eq!(r.status(), 200, "{}", r.body());
    let info: protocol::ResumeInfo = r.json().expect("the contract's resume document");
    assert_eq!(info.entry_id, h.entry_id);
    assert_eq!(info.session_id, "s1");
    assert!(
        info.caught_up_events >= 1,
        "R16: the tail is caught up before a relaunch"
    );
    assert!(info.last_process.is_some(), "the last ProcessHandle (D22)");
    assert_eq!(info.workspace, h.workspace.display().to_string());
    assert_eq!(info.artifact_root, h.artifact_dir.display().to_string());
    assert_eq!(info.harness_home, h.home.display().to_string());
    assert_eq!(
        info.status_line_path,
        h.entry().status_line_path.display().to_string()
    );
    assert!(
        !r.body().contains(h.control_token.as_str()),
        "the CLI's bearer is not a field of any body"
    );

    // The wire shape is the contract's and nothing else: what the daemon sent and what the
    // contract's type round-trips have the same field names, so no field is invented here
    // and none the protocol declares is dropped.
    let sent: serde_json::Value = serde_json::from_str(r.body()).expect("the body is JSON");
    let round_tripped = serde_json::to_value(&info).expect("the type re-serialises");
    assert_eq!(
        sent.as_object().map(|o| o.keys().collect::<Vec<_>>()),
        round_tripped
            .as_object()
            .map(|o| o.keys().collect::<Vec<_>>()),
    );

    // The relaunch the resume exists to make possible, in the order the CLI does it. The
    // resume cleared the handle, so this is the registration that gives the seal
    // below something to end.
    assert_eq!(
        h.post_body(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS),
            &support::process_registration(
                info.process_generation,
                support::handle_for_now(h.child_pid)
            ),
        )
        .await
        .status(),
        204,
        "the relaunched harness registers into the slot the resume opened"
    );

    let seal_route = protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL);
    let seal: goalcup_protocol::seal::SealManifest = h.post_json(&seal_route, &()).await;
    assert_eq!(seal.entry_id, h.entry_id);
    let chain = support::read_chain(&h.trace_dir);
    assert!(
        matches!(
            chain.last().map(|e| &e.body),
            Some(goalcup_protocol::envelope::EventBody::Seal { .. })
        ),
        "C4: the seal is the chain's last link, written by TraceExport::write_seal"
    );
    assert!(
        !goalcup_protocol::process::is_alive(h.child_pid),
        "R15: the manifest, then the kill"
    );

    // Sealing twice is a conflict, not a second seal.
    let again = h.post(&seal_route).await;
    assert_eq!(again.status(), 409, "{}", again.body());
    let body: protocol::ErrorBody = again.json().expect("the contract's error body");
    assert_eq!(body.error.code, "conflict");
    assert_eq!(
        support::read_chain(&h.trace_dir).len(),
        chain.len(),
        "the refused second seal wrote nothing"
    );
}

/// The other half of resume, and the reason the check is the daemon's rather than the CLI's.
///
/// A codex resume that cannot take the thread-writer lock silently becomes a new thread —
/// a run whose second half is chained under a session nobody bound. So a resume asks where
/// that lock is, refuses while anything is at the path, and names the one it found. The CLI
/// refuses on its side too; this is the authority.
///
/// The entry here has not sealed: a check that consulted the seal report alone would let a
/// harness killed without a seal leave its lock on disk
/// with no report anywhere recording it, and serve a resume straight over it.
/// A sealed entry never reaches this check at all — it is refused for being sealed, by
/// `a_sealed_entry_is_refused_a_resume_rather_than_handed_a_token_or_a_500`.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_resume_is_refused_while_the_connector_s_lock_path_still_exists() {
    let h = support::control_harness().await;
    let (codex, codex_home) = support::codex_runtime(&h).await;
    h.daemon.register(codex).expect("a codex entry registers");
    h.daemon.arm("e-codex").expect("the codex entry arms");
    h.daemon
        .register_process(
            "e-codex",
            support::process_registration(1, support::process_handle()),
        )
        .expect("the launch process is registered before its committed start");
    h.daemon
        .registry()
        .update("e-codex", |entry| {
            entry.state = goalcup_daemon::registry::EntryState::Started;
            entry.started_at = Some("2026-09-10T00:00:00Z".to_string());
            entry.session_id = Some(support::DAEMON_SESSION.to_string());
        })
        .expect("the focused lock fixture has a committed session");
    let resume = protocol::entry_route("e-codex", control::SUFFIX_RESUME);
    let process = protocol::entry_route("e-codex", control::SUFFIX_PROCESS);
    let codex_trace = h.artifact_root.join("artifacts/e-codex/trace");
    let initial_generation = h
        .daemon
        .registry()
        .get("e-codex")
        .expect("the row reads")
        .expect("it is there")
        .process_handoff_generation();

    assert!(
        h.daemon
            .registry()
            .get("e-codex")
            .expect("the row reads")
            .expect("it is there")
            .seal_outcome
            .is_none(),
        "this entry has not sealed, so the paths are the connector's, resolved live"
    );
    let first = h
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: initial_generation,
            },
        )
        .await;
    assert_eq!(
        first.status(),
        200,
        "nothing is at the lock path: {}",
        first.body()
    );
    let first: protocol::ResumeInfo = first.json().expect("the first resume document");
    let mut replacement = support::process_handle();
    replacement.native_binary = Some("codex".to_string());
    assert_eq!(
        h.post_body(
            &process,
            &support::process_registration(first.process_generation, replacement),
        )
        .await
        .status(),
        204,
        "the replacement process closes the one-winner resume generation"
    );

    // Neither the lock directory nor the `.coordination.lock` inside it is a lock.
    // `codex-cli`'s `lock_paths[]` names the per-thread file
    // `{config_dir}/thread-writer-locks/{session_id}.lock`; the directory and the
    // coordination file are both made at startup and both survive a clean quit, and refusing
    // over either would refuse every resume of a run that had ended properly.
    let lock = codex_home.join("thread-writer-locks");
    std::fs::create_dir_all(&lock).expect("the harness's lock directory");
    std::fs::write(lock.join(".coordination.lock"), b"").expect("what a clean quit leaves");
    let second = h
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: first.process_generation,
            },
        )
        .await;
    assert_eq!(
        second.status(),
        200,
        "the directory and its coordination file are the harness's residue, not a lock: {}",
        second.body()
    );
    let second: protocol::ResumeInfo = second.json().expect("the second resume document");
    let mut second_replacement = support::process_handle();
    second_replacement.native_binary = Some("codex".to_string());
    assert_eq!(
        h.post_body(
            &process,
            &support::process_registration(second.process_generation, second_replacement),
        )
        .await
        .status(),
        204,
        "the second replacement makes the lock-path request test the real next leg"
    );

    // The lock the killed harness left behind, with no seal anywhere recording it: named
    // after the thread this entry is bound to, which is the only one it can refuse over.
    let held = lock.join(format!("{}.lock", support::DAEMON_SESSION));
    std::fs::write(&held, "4242").expect("the lock file the harness left");
    let before = support::read_chain(&codex_trace).len();

    let r = h
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: second.process_generation,
            },
        )
        .await;
    assert_eq!(r.status(), 409, "{}", r.body());
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "conflict");
    assert!(
        body.error.message.contains(&lock.display().to_string()),
        "the refusal names the path a relaunch would have to take: {}",
        body.error.message
    );
    assert_eq!(
        support::read_chain(&codex_trace).len(),
        before,
        "a refused resume catches nothing up"
    );

    // …and it goes away with the file, not with the directory: the harness's own directory
    // outlives the run and a participant is never asked to remove it.
    std::fs::remove_file(&held).expect("the participant clears the lock file");
    let r = h
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: second.process_generation,
            },
        )
        .await;
    assert_eq!(
        r.status(),
        200,
        "the directory is still there and holds nothing, so the resume goes ahead: {}",
        r.body()
    );
}

/// `Daemon`'s accessors live in the file that owns the private maps they
/// read, and are exercised here, beside the handlers that are their only callers.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn every_accessor_these_handlers_call_answers_over_a_live_entry() {
    let h = support::control_harness().await;
    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, "s1", "one more record to catch up"),
    );
    let d = &h.daemon;
    assert!(d.arming_of(&h.entry_id).unwrap().is_some());
    assert_eq!(d.start_signal_source(&h.entry_id).unwrap(), "leg1");
    assert!(!d.arming_signal(&h.entry_id).unwrap().is_empty());
    assert_eq!(d.session_id(&h.entry_id).unwrap().as_deref(), Some("s1"));
    assert!(!d.on_screen(&h.entry_id).unwrap().is_empty());
    assert!(d.signed_status(&h.entry_id).unwrap().is_some());
    assert_eq!(d.entries().unwrap().len(), 1);
    assert_eq!(
        d.current_entry().unwrap().as_deref(),
        Some(h.entry_id.as_str())
    );
    assert!(d.catch_up(&h.entry_id).unwrap().caught_up_events >= 1);
    assert!(d.uptime_seconds() < 60);
    assert!(d.upload_ok());
    assert!(d.ui_state().cache.lock().unwrap().contains_key(&h.entry_id));
    // Refused after `started_at`: a started run ends by sealing.
    d.abandon(&h.entry_id, "test").unwrap_err();
}

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

/// `status-latest.json` is the one document the MCP server reads when it is not
/// told which entry it wants. An abandon that left it naming the entry it had just dropped
/// would answer a run that no longer exists, signed and all, until the next refresh — and
/// on a daemon holding nothing else there is no next refresh.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn abandoning_an_entry_leaves_the_shared_status_pointer_naming_no_run() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    h.daemon.refresh_statuses().expect("the statuses publish");
    let latest = h.config_dir.join("cache").join("status-latest.json");
    assert!(
        std::fs::read_to_string(&latest)
            .expect("the shared pointer")
            .contains(&info.entry_id),
        "the pointer names the only run there is"
    );

    assert_eq!(
        h.post(&protocol::entry_route(
            &info.entry_id,
            control::SUFFIX_ABANDON
        ))
        .await
        .status(),
        204
    );
    let after = std::fs::read_to_string(&latest).unwrap_or_default();
    assert!(
        !after.contains(&info.entry_id),
        "the shared pointer still names a run that is gone: {after}"
    );
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

// ------------------------------------------------------------------ state guards
//
// Four state guards, and the cleanup a refusal owes the disk.

/// Arming stamps `armed_at` and opens a new gate. After the start both are facts the run's
/// signed receipt witnessed, so a second `POST /arm` is a `409`; before the start signal a
/// re-arm is a participant retrying a launch that never took, and it is a `204`.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn arming_is_refused_once_the_entry_started_and_is_a_retry_before_it() {
    let empty = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = empty
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("codex-cli"),
        )
        .await;
    let arm = protocol::entry_route(&info.entry_id, control::SUFFIX_ARM);
    assert_eq!(empty.post(&arm).await.status(), 204);
    assert_eq!(
        empty.post(&arm).await.status(),
        204,
        "before the start signal a re-arm is a retry, not a refusal"
    );

    let h = support::control_harness().await;
    let armed_at = h.entry().armed_at.expect("the fixture armed the entry");
    let r = h
        .post(&protocol::entry_route(&h.entry_id, control::SUFFIX_ARM))
        .await;
    assert_eq!(r.status(), 409);
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "conflict");
    assert_eq!(
        h.entry().armed_at,
        Some(armed_at),
        "a refused arm re-stamps nothing and replaces no gate"
    );
}

/// The registered handle is what `harness_alive` measures and what the seal terminates. A
/// second one posted on the loopback would redirect both at a process the launch never made,
/// so the standing handle wins and everything else is a `409` — before the start and, as the
/// second half asserts, after it. Only a resume opens that slot; a second POST never
/// does.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_second_process_registration_is_refused_and_the_first_handle_still_stands() {
    let empty = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = empty
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    assert_eq!(
        empty
            .post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let first = support::process_handle();
    assert_eq!(
        empty
            .post_body(
                &route,
                &support::process_registration(info.process_generation, first.clone()),
            )
            .await
            .status(),
        204
    );

    // A decoy that is well formed in every other way: a different live process, no process
    // group, and the creation identity that proves this pid is not a recycled number.
    let mut decoy_child = support::spawn_harness();
    let decoy = support::handle_for_now(decoy_child.id());
    let r = empty
        .post_body(
            &route,
            &support::process_registration(info.process_generation, decoy),
        )
        .await;
    assert_eq!(r.status(), 409);
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "conflict");
    assert_eq!(
        empty
            .daemon
            .registry()
            .get(&info.entry_id)
            .expect("the entry reads")
            .expect("it is there")
            .last_process,
        Some(first),
        "`harness_alive` and the seal's terminate target still name the launch's own process"
    );
    let _ = decoy_child.kill();
    let _ = decoy_child.wait();

    // …and after the start, where the launch is under a signed receipt and the handle the
    // receipt witnessed is still the one this entry names.
    let h = support::control_harness().await;
    let before = h.entry().last_process;
    let r = h
        .post_body(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS),
            &support::process_registration(1, support::process_handle()),
        )
        .await;
    assert_eq!(r.status(), 409);
    assert_eq!(h.entry().last_process, before, "nothing was redirected");
}

/// `resume --print-launch` needs pins but must not become a weaker spelling of resume.
///
/// This is a real daemon and registry, not a mocked control method: the initial process
/// handle stands before the GET, then remains standing afterwards, which proves the route
/// did not clear the only slot a real resume clears. The chain snapshot makes the same claim
/// for `catch_up`'s capture side effect.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn resume_preview_is_authenticated_get_only_and_does_not_mutate_a_real_entry() {
    let h = support::control_harness().await;
    let preview = protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME_PREVIEW);
    let process = protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS);
    let before_process = h.entry().last_process;
    let before_chain = support::read_chain(&h.trace_dir);

    let anonymous = h.get_no_token(&preview).await;
    assert_eq!(anonymous.status(), 401, "{}", anonymous.body());
    let anonymous_error: protocol::ErrorBody = anonymous.json().expect("the auth refusal");
    assert_eq!(anonymous_error.error.code, "unauthorized");

    let wrong_method = h.post(&preview).await;
    assert_eq!(wrong_method.status(), 405, "{}", wrong_method.body());
    let wrong_method_error: protocol::ErrorBody = wrong_method.json().expect("the 405 body");
    assert_eq!(wrong_method_error.error.code, "malformed");

    let response = h.get(&preview).await;
    assert_eq!(response.status(), 200, "{}", response.body());
    let info: protocol::ResumePreviewInfo = response.json().expect("the preview pins");
    assert_eq!(info.entry_id, h.entry_id);
    assert_eq!(
        info.process_generation, 1,
        "the daemon reports its authority generation"
    );
    assert_eq!(info.process_handoff_phase, "registered");
    assert_eq!(info.session_id, "s1");
    assert_eq!(info.workspace, h.workspace.display().to_string());
    assert_eq!(info.artifact_root, h.artifact_dir.display().to_string());
    assert_eq!(info.harness_home, h.home.display().to_string());
    assert_eq!(
        info.status_line_path,
        h.entry().status_line_path.display().to_string()
    );
    let sent: serde_json::Value = serde_json::from_str(response.body()).expect("JSON body");
    assert_eq!(
        sent.as_object().map(|fields| fields.len()),
        Some(9),
        "preview has the seven launch pins plus authoritative handoff diagnostics"
    );
    for absent in ["caught_up_events", "last_process"] {
        assert!(
            sent.get(absent).is_none(),
            "preview must not include {absent}"
        );
    }

    assert_eq!(
        h.entry().last_process,
        before_process,
        "preview never clears the registered process handle"
    );
    assert_eq!(
        support::read_chain(&h.trace_dir),
        before_chain,
        "preview never catches up or otherwise appends capture state"
    );
    assert_eq!(
        h.post_body(
            &process,
            &support::process_registration(1, support::process_handle())
        )
        .await
        .status(),
        409,
        "the still-registered handle continues to protect its one process slot"
    );
}

/// A resume retains the predecessor until the replacement registers once.
///
/// The two rules are one rule. `register_process` refuses while a handle stands, because a
/// second one posted on the loopback would redirect both `harness_alive` and the seal's
/// terminate target at a process this run did not launch. A resume is the one moment that
/// stops being true only for the exact generation the daemon just issued: the CLI is about
/// to replace the process the entry names, and the relaunched harness has to be registrable
/// without making the seal lose the predecessor if the local relaunch then fails.
///
/// So the durable entry keeps the predecessor here — after the sealed check and the lock
/// check have passed and after `catch_up` has succeeded — until the exact replacement reaches
/// `Registered`. A refused resume consumes nothing, and a failed local relaunch leaves a
/// known seal target rather than an empty authority slot.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_resume_retains_the_predecessor_until_the_relaunched_harness_registers_once() {
    let h = support::control_harness().await;
    let process = protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS);
    let resume = protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME);
    assert!(
        h.entry().last_process.is_some(),
        "the launch registered the process this run made"
    );
    assert_eq!(
        h.post_body(
            &process,
            &support::process_registration(1, support::process_handle()),
        )
        .await
        .status(),
        409,
        "and while it stands a second handle is refused, started or not"
    );

    let predecessor = h.entry().last_process;
    let r = h
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: 1,
            },
        )
        .await;
    assert_eq!(r.status(), 200, "{}", r.body());
    let info: protocol::ResumeInfo = r.json().expect("the contract's resume document");
    assert!(
        info.last_process.is_some(),
        "the answer still names the process the relaunch is replacing"
    );
    assert_eq!(
        h.entry().last_process,
        predecessor,
        "the entry keeps the predecessor as seal authority until its replacement is registered"
    );
    assert_eq!(
        h.entry().expected_process_handoff(),
        goalcup_daemon::registry::ExpectedProcessHandoff::Required,
        "the launch contract remains in force across a resumable process replacement"
    );

    let relaunched = support::process_handle();
    assert_eq!(
        h.post_body(
            &process,
            &support::process_registration(info.process_generation, relaunched.clone()),
        )
        .await
        .status(),
        204,
        "the relaunched harness registers, on a run that started days ago"
    );
    assert_eq!(
        h.entry().last_process,
        Some(relaunched),
        "`harness_alive` and the seal now measure the process that is actually running"
    );
    let mut decoy_child = support::spawn_harness();
    let decoy = support::handle_for_now(decoy_child.id());
    assert_eq!(
        h.post_body(
            &process,
            &support::process_registration(info.process_generation, decoy),
        )
        .await
        .status(),
        409,
        "…and once, not twice: the next resume is what opens the slot again"
    );
    let _ = decoy_child.kill();
    let _ = decoy_child.wait();
}

/// The resume CAS is deliberately after catch-up.  If the tail cannot be chained, the old
/// registered process and its generation remain usable rather than stranding an unlaunched
/// `OpenResume` generation on a capture failure.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_catch_up_failure_does_not_consume_the_registered_process_generation() {
    let h = support::control_harness().await;
    let resume = protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME);
    let before = h.entry();
    let chain_before = support::read_chain(&h.trace_dir);
    support::append_line(
        &h.store_path,
        &support::transcript_line(
            &h.workspace,
            "s1",
            "a tail the failed catch-up cannot commit",
        ),
    );
    let mut held = h
        .daemon
        .borrow_entry(&h.entry_id)
        .expect("the runtime is held for injection");
    held.get()
        .expect("the runtime is present")
        .chain_writer_mut()
        .expect("the chain writer is open")
        .fail_append_after(0);
    drop(held);

    let failed = h
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: before.process_handoff_generation(),
            },
        )
        .await;
    assert_eq!(failed.status(), 500, "{}", failed.body());
    assert_eq!(
        h.entry(),
        before,
        "the failed catch-up did not clear the termination handle or advance authority"
    );
    assert_eq!(
        support::read_chain(&h.trace_dir),
        chain_before,
        "the injected writer failure appended no partial tail record"
    );
}

/// Resume is a start-of-new-leg mutation, not an alternative route to make an uncommitted
/// run live.  It must refuse before the tail catch-up can consume a cursor or append a
/// record, both when the row is merely armed and when an interrupted/legacy writer left the
/// impossible `Started`/missing-session shape behind.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn resume_requires_a_durable_started_session_before_it_can_catch_up_or_open_a_generation() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let workspace = std::path::PathBuf::from(&info.workspace);
    support::append_line(
        &support::transcript_path(
            std::path::Path::new(&info.harness_home),
            &workspace,
            &info.session_id,
        ),
        &support::transcript_line(&workspace, &info.session_id, "tail must remain pending"),
    );
    let trace = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the row reads")
        .expect("the row exists")
        .trace_dir;
    let chain_before = std::fs::read(trace.join(goalcup_protocol::export::CHAIN_FILE))
        .expect("the baseline exists");
    let resume = protocol::entry_route(&info.entry_id, control::SUFFIX_RESUME);
    let request = protocol::ResumeRequest {
        expected_generation: info.process_generation,
    };

    let unstarted = h.post_body(&resume, &request).await;
    assert_eq!(unstarted.status(), 409, "{}", unstarted.body());
    assert_eq!(
        std::fs::read(trace.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
        chain_before,
        "the pre-start refusal ran before catch_up"
    );

    h.daemon
        .registry()
        .update(&info.entry_id, |entry| {
            entry.state = goalcup_daemon::registry::EntryState::Started;
            entry.started_at = Some("2026-09-10T00:00:00Z".to_string());
            entry.session_id = None;
        })
        .expect("the interrupted row is planted");
    let no_session = h.post_body(&resume, &request).await;
    assert_eq!(no_session.status(), 409, "{}", no_session.body());
    assert_eq!(
        std::fs::read(trace.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
        chain_before,
        "a Started row without a committed session is equally unable to catch up"
    );
}

/// `--attach` is an invocation mode, not a connector manifest mode: the shipped Claude
/// connector is a `tui`, but this workspace did not launch a child. Resume still resumes that
/// launchable connector by atomically promoting its next leg to a required process handoff.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn attach_rejects_unsolicited_processes_then_resume_promotes_and_registers_once() {
    let h = support::empty_control_harness().await;
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.attach = true;
    let info: protocol::WorkspaceInfo = h.post_json(protocol::ROUTE_WORKSPACES, &opts).await;
    let process = protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS);
    let resume = protocol::entry_route(&info.entry_id, control::SUFFIX_RESUME);
    let before = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the created row reads")
        .expect("the created row exists");
    assert_eq!(
        before.expected_process_handoff(),
        goalcup_daemon::registry::ExpectedProcessHandoff::NotRequired
    );

    let mut unsolicited = support::process_handle();
    unsolicited.home_seed = Some(goalcup_protocol::events::HomeSeedRecord {
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "claude-code-cli".to_string(),
        from: "{home}/.claude.json".to_string(),
        keys: Vec::new(),
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
        wire_layout: Default::default(),
    });
    let chain_before = std::fs::read(before.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
        .expect("the attach trace has its baseline");
    assert_eq!(
        h.post_body(
            &process,
            &support::process_registration(info.process_generation, unsolicited),
        )
        .await
        .status(),
        409,
        "an attached run cannot be granted a process authority it never launched"
    );
    assert_eq!(
        std::fs::read(before.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
        chain_before,
        "the rejected request reaches no HomeSeeded side effect"
    );

    // An attached origin has no initial launch authority, but once its own capture evidence
    // has started the run it may open the first resume generation for a launchable connector.
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    let workspace = std::path::PathBuf::from(&info.workspace);
    support::append_line(
        &support::transcript_path(
            std::path::Path::new(&info.harness_home),
            &workspace,
            &info.session_id,
        ),
        &support::transcript_line(&workspace, &info.session_id, "the attached run started"),
    );
    h.daemon
        .tick()
        .await
        .expect("the attached evidence starts the run");
    assert!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the row reads")
            .expect("the row remains")
            .started_at
            .is_some(),
        "only a started attached run may obtain a resume process generation"
    );

    let resume_response = h
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: info.process_generation,
            },
        )
        .await;
    assert_eq!(resume_response.status(), 200, "{}", resume_response.body());
    let resumed: protocol::ResumeInfo = resume_response.json().expect("the resume document");
    let after_resume = h.daemon.registry().get(&info.entry_id).unwrap().unwrap();
    assert_eq!(
        after_resume.expected_process_handoff(),
        goalcup_daemon::registry::ExpectedProcessHandoff::Required,
        "the launchable connector's resumed leg owes the relaunched process"
    );
    assert_eq!(
        after_resume.last_process, None,
        "the promotion and slot clear are one durable transition"
    );
    assert_eq!(
        h.post_body(
            &process,
            &support::process_registration(resumed.process_generation, support::process_handle()),
        )
        .await
        .status(),
        204,
        "the one resume-authorized relaunch registers"
    );
    assert!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .unwrap()
            .unwrap()
            .last_process
            .is_some()
    );
}

/// A true `attach_only` connector cannot resume because it has no launch recipe. More
/// importantly, that refusal is before `catch_up`: a pending transcript must not advance
/// the chain or cursor merely because a route that cannot resume was requested.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn attach_only_resume_refuses_before_catch_up_changes_pending_capture() {
    let connectors = tempfile::tempdir().expect("a private connectors directory");
    let connector_id = "attach-only-test";
    let connector = connectors.path().join(connector_id);
    support::copy_tree(
        &support::shipped_connector_dir("claude-code-cli"),
        &connector,
    );
    let manifest_path = connector.join("manifest.toml");
    let mut manifest = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(&manifest_path).expect("the copied manifest reads"),
    )
    .expect("the copied manifest parses");
    // An unpinned declared id makes this an intentionally development-only connector;
    // reusing a shipped id would correctly fail the release directory-digest boundary
    // before the route under test could create an entry.
    manifest.id = connector_id.to_string();
    manifest.launch.mode = goalcup_protocol::manifest::LaunchMode::AttachOnly;
    // A test-only manifest deliberately differs from this build's directory pin. Foreign
    // scans demand a pinned complete connector tree, but they are irrelevant to this route
    // assertion, so remove that independent declaration rather than weakening the fixture.
    manifest.foreign_scan.clear();
    std::fs::write(
        &manifest_path,
        toml::to_string(&manifest).expect("the attach-only manifest serialises"),
    )
    .expect("the attach-only manifest writes");

    let h = support::empty_control_harness_with(|config| {
        config.connectors_dir = connectors.path().to_path_buf();
    })
    .await;
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.connector_id = connector_id.to_string();
    opts.manifest_sha256 = goalcup_daemon::workspace::sha256_hex(
        &std::fs::read(&manifest_path).expect("the modified manifest reads"),
    );
    let info: protocol::WorkspaceInfo = h.post_json(protocol::ROUTE_WORKSPACES, &opts).await;
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    // Save the zero-position baseline before appending the tail. If resume wrongly catches
    // up below, this exact cursor document must advance with the pending record.
    h.daemon
        .tick()
        .await
        .expect("the idle armed tick saves the initial cursor");

    let workspace = std::path::PathBuf::from(&info.workspace);
    support::append_line(
        &support::transcript_path(
            std::path::Path::new(&info.harness_home),
            &workspace,
            &info.session_id,
        ),
        &support::transcript_line(&workspace, &info.session_id, "pending attach-only tail"),
    );
    let before = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the created row reads")
        .expect("the created row exists");
    assert_eq!(
        before.expected_process_handoff(),
        goalcup_daemon::registry::ExpectedProcessHandoff::NotRequired
    );
    let chain_before = std::fs::read(before.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
        .expect("the trace starts with its baseline");
    let cursor_before = std::fs::read(before.artifact_dir.join("cursors.json"))
        .expect("the trace saves an initial cursor");

    assert_eq!(
        h.post_body(
            &protocol::entry_route(&info.entry_id, control::SUFFIX_RESUME),
            &protocol::ResumeRequest {
                expected_generation: info.process_generation,
            },
        )
        .await
        .status(),
        409,
        "an attach-only connector refuses before catch-up"
    );

    let after = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the refused row reads")
        .expect("the refused row remains");
    assert_eq!(after, before, "the refusal changes no registry field");
    assert_eq!(
        std::fs::read(after.trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
        chain_before,
        "the pending tail is not caught up"
    );
    assert_eq!(
        std::fs::read(after.artifact_dir.join("cursors.json")).unwrap(),
        cursor_before,
        "the pending tail does not advance the cursor"
    );
}

/// A failure after workspace rendering removes mutable participant/workspace files and the
/// bearer-bearing artifact directory through the retained authority.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_workspace_refused_after_rendering_removes_bearer_artifacts() {
    let h = support::empty_control_harness().await;
    // `Daemon::register` creates the entry's harness home under `<artifact_root>/homes`, and
    // that is the first thing that can fail once `prepare` has already written every
    // rendered file. A plain file at that name is a `create_dir_all` that cannot succeed —
    // and no connector this daemon ships installs anything under the harness home, so
    // nothing earlier trips over it.
    std::fs::create_dir_all(&h.config.artifact_root).expect("the artifact root");
    std::fs::write(h.config.artifact_root.join("homes"), b"not a directory")
        .expect("the planted file");

    let r = h
        .post_body(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert_ne!(
        r.status(),
        200,
        "the harness home cannot be created, so this start is refused: {}",
        r.body()
    );

    let left: Vec<_> = std::fs::read_dir(&h.config.workspace_parent)
        .map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(
        left.is_empty(),
        "the daemon created this workspace and a refusal takes the whole of it back: {left:?}"
    );
    assert!(
        std::fs::read_dir(h.config.artifact_root.join("artifacts"))
            .expect("the capability-created artifacts parent remains")
            .next()
            .is_none(),
        "setup failure removes the entire bearer-bearing entry artifact"
    );
    assert!(h.daemon.entries().expect("the entry list reads").is_empty());
}

/// Registry artifact paths are diagnostics, never recursive-delete authority. Cleanup uses
/// the live runtime capability, so a stale row cannot redirect it outside.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_mutable_registry_artifact_dir_is_never_remove_authority() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;

    let outside = h.root.join("not-the-daemons");
    std::fs::create_dir_all(&outside).expect("the directory that must survive");
    std::fs::write(outside.join("keep.txt"), b"the participant's own").expect("its file");
    h.daemon
        .registry()
        .update(&info.entry_id, |e| e.artifact_dir = outside.clone())
        .expect("the row updates");

    assert_eq!(
        h.post(&protocol::entry_route(
            &info.entry_id,
            control::SUFFIX_ABANDON
        ))
        .await
        .status(),
        204
    );
    assert!(
        outside.join("keep.txt").is_file(),
        "abandon must leave a registry-selected outside sentinel untouched"
    );
    assert!(
        !std::path::Path::new(&info.artifact_root).exists(),
        "the daemon removes its own artifact through the retained runtime capability"
    );
}

#[cfg(feature = "v1-compat")]
#[cfg(unix)]
#[tokio::test]
async fn cleanup_failure_refuses_abandon_and_retains_the_registry_row() {
    use std::os::unix::fs::PermissionsExt;

    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let artifacts = h.config.artifact_root.join("artifacts");
    std::fs::set_permissions(&artifacts, std::fs::Permissions::from_mode(0o555)).unwrap();

    let response = h
        .post(&protocol::entry_route(
            &info.entry_id,
            control::SUFFIX_ABANDON,
        ))
        .await;

    std::fs::set_permissions(&artifacts, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(response.status(), 409, "{}", response.body());
    let row = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("registry reads")
        .expect("cleanup failure retains the authoritative entry record");
    assert_eq!(
        row.state,
        goalcup_daemon::registry::EntryState::CleanupPending,
        "a partially removed entry is durable cleanup debt, never an ordinary runtime"
    );
    let artifact = std::path::Path::new(&info.artifact_root);
    assert!(
        artifact.is_dir(),
        "the failed final unlink remains addressable"
    );
    assert_eq!(
        std::fs::read_dir(artifact).unwrap().count(),
        0,
        "recursive cleanup completed before the final parent unlink failed"
    );
    let arm = h
        .post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
        .await;
    assert_eq!(
        arm.status(),
        409,
        "normal mutation remains closed while cleanup is pending"
    );
    assert!(arm.body().contains("cleanup pending"), "{}", arm.body());
    let resume_preview = h
        .get(&protocol::entry_route(
            &info.entry_id,
            control::SUFFIX_RESUME_PREVIEW,
        ))
        .await;
    assert_eq!(
        resume_preview.status(),
        409,
        "resume remains closed while cleanup is pending"
    );
    assert!(
        !response.body().contains("artifact directory is gone"),
        "failure must not report successful cleanup"
    );

    let h = support::restart_empty_control(h).await;
    assert!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("registry reads after restart")
            .is_none(),
        "restart retries and commits the pending cleanup"
    );
    assert!(
        !artifact.exists(),
        "restart removes the retained empty artifact"
    );
}

#[cfg(all(unix, not(target_os = "macos")))]
#[tokio::test]
async fn restart_finishes_cleanup_after_a_durably_recorded_successful_termination() {
    use std::os::unix::fs::PermissionsExt;

    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    let route = |suffix| protocol::entry_route(&info.entry_id, suffix);
    assert_eq!(h.post(&route(control::SUFFIX_ARM)).await.status(), 204);
    let mut child = support::spawn_harness();
    let handle = support::handle_for_now(child.id());
    assert_eq!(
        h.post_body(
            &route(control::SUFFIX_PROCESS),
            &support::process_registration(info.process_generation, handle),
        )
        .await
        .status(),
        204
    );

    let artifacts = h.config.artifact_root.join("artifacts");
    std::fs::set_permissions(&artifacts, std::fs::Permissions::from_mode(0o555)).unwrap();
    let abandoned = h.post(&route(control::SUFFIX_ABANDON)).await;
    std::fs::set_permissions(&artifacts, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(abandoned.status(), 409, "{}", abandoned.body());
    child
        .wait()
        .expect("the successfully terminated child is reaped by its test");

    let pending = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .unwrap()
        .expect("the failed deletion leaves cleanup debt");
    assert_eq!(
        pending.state,
        goalcup_daemon::registry::EntryState::CleanupPending
    );
    assert_eq!(
        pending.process_handoff_phase(),
        goalcup_daemon::registry::ProcessHandoffPhase::Registered
    );
    assert!(pending.last_process.is_some());
    assert!(
        h.daemon
            .registry()
            .cleanup_termination_complete_for_test(&info.entry_id)
            .unwrap(),
        "the successful kill is committed before cleanup can fail"
    );

    let h = support::restart_empty_control(h).await;
    assert!(
        h.daemon.registry().get(&info.entry_id).unwrap().is_none(),
        "the completed termination fact lets restart resume deletion without re-checking an absent PID"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn selected_artifact_root_swap_is_refused_before_workspace_artifacts_are_written() {
    use std::os::unix::fs::symlink;

    let h = support::empty_control_harness().await;
    let detached = h.root.join("detached-holder");
    let outside = h.root.join("outside-holder");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("sentinel.txt"), b"outside").unwrap();
    std::fs::rename(&h.config.artifact_root, &detached).unwrap();
    symlink(&outside, &h.config.artifact_root).unwrap();

    let response = h
        .post_body(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;

    assert_ne!(response.status(), 200, "{}", response.body());
    assert_eq!(
        std::fs::read(outside.join("sentinel.txt")).unwrap(),
        b"outside"
    );
    assert!(!outside.join("artifacts").exists());
    assert!(h.daemon.entries().unwrap().is_empty());
}

/// A relative `workspace_parent` would be joined against whatever directory the daemon was
/// started in, which is a workspace neither side named. It is a `400`, and nothing is made.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_relative_workspace_parent_is_malformed_and_creates_nothing() {
    let h = support::empty_control_harness().await;
    let mut opts = support::workspace_opts("claude-code-cli");
    opts.workspace_parent = Some("relative/elsewhere".to_string());

    let r = h.post_body(protocol::ROUTE_WORKSPACES, &opts).await;
    assert_eq!(r.status(), 400);
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "malformed");
    assert!(h.daemon.entries().expect("the entry list reads").is_empty());

    // The absolute spelling of the same intent is served.
    let parent = h.root.join("chosen-parent");
    opts.workspace_parent = Some(parent.display().to_string());
    let info: protocol::WorkspaceInfo = h.post_json(protocol::ROUTE_WORKSPACES, &opts).await;
    assert!(std::path::Path::new(&info.workspace).starts_with(&parent));
}

// -------------------------------------------------- resume, seal and the shared pointer
//
// The resume lock check reads the durable copy of the paths and covers entries that have
// not sealed; `resume` refuses a closed chain; the seal does not hold the runtime for the
// length of a kill; and the shared pointer is decided by a clock rather than by an id.

/// The check does not read the daemon's in-memory seal map, which a restarted
/// daemon does not have — every path it would ask about would be vacuously free, in
/// exactly the situation the check exists for: a participant resumes after the process that
/// sealed is gone.
///
/// [`Daemon::resume_lock_paths`] is where the rule lives, and both of its branches are
/// asserted here against one entry. Sealed, it answers the registry row's
/// `SealOutcome::lock_paths` — the copy that outlives this process — and the row is
/// repointed to prove it is not reading the in-memory report, which still names the path the
/// seal resolved. Unsealed, it answers the connector's `lock_paths[]` resolved live, which is
/// what `a_resume_is_refused_while_the_connector_s_lock_path_still_exists` then refuses over.
///
/// [`Daemon::resume_lock_paths`]: goalcup_daemon::runtime::Daemon::resume_lock_paths
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn the_resume_lock_paths_come_from_the_row_the_seal_left_and_not_this_process_s_memory() {
    let h = support::control_harness().await;
    let (codex, codex_home) = support::codex_runtime(&h).await;
    h.daemon.register(codex).expect("a codex entry registers");

    // Unsealed: the connector's templates, resolved through this entry's own variables.
    let live = h
        .daemon
        .resume_lock_paths("e-codex")
        .expect("the live paths resolve");
    assert!(
        live.iter()
            .any(|p| p.starts_with(&codex_home.join("thread-writer-locks").display().to_string())),
        "an entry that has not sealed has no recorded paths, and its lock is where the \
         connector says it is now: {live:?}"
    );

    let _: goalcup_protocol::seal::SealManifest = h
        .post_json(&protocol::entry_route("e-codex", control::SUFFIX_SEAL), &())
        .await;
    assert_eq!(
        h.daemon
            .resume_lock_paths("e-codex")
            .expect("the recorded paths read"),
        live,
        "the seal recorded what the run last knew, so the two agree until the row moves"
    );

    // What the daemon that comes next reads: the row, and only the row.
    let elsewhere = h
        .artifact_root
        .parent()
        .expect("a root")
        .join("lock-a-restart-would-find")
        .display()
        .to_string();
    h.daemon
        .registry()
        .update("e-codex", |e| {
            if let Some(outcome) = e.seal_outcome.as_mut() {
                outcome.lock_paths = vec![elsewhere.clone()];
            }
        })
        .expect("the row updates");
    assert_eq!(
        h.daemon
            .resume_lock_paths("e-codex")
            .expect("the recorded paths read"),
        vec![elsewhere],
        "the durable row is the authority: a restarted daemon has no seal report to read"
    );
    assert_eq!(
        h.daemon
            .seal_report("e-codex")
            .expect("the in-memory report reads")
            .expect("this process is the one that sealed")
            .lock_paths,
        live,
        "…and it is not the in-memory map, which still names what the seal resolved"
    );
}

/// A sealed chain has no tail to catch up and nothing a relaunch could record,
/// so `resume` refuses it first — before the lock check and before `catch_up`.
///
/// Otherwise a Claude seal, which records no lock path, would pass the check and the
/// route would answer `200` with a `ResumeInfo` carrying the run's `receiver_token` for a chain
/// that is closed; and a store grown since the seal would meet `ExportError::Sealed` in
/// `catch_up`'s poll, and the participant would read a `500` about a state that is not an error.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_sealed_entry_is_refused_a_resume_rather_than_handed_a_token_or_a_500() {
    let h = support::control_harness().await;
    let resume = protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME);
    let _: goalcup_protocol::seal::SealManifest = h
        .post_json(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL),
            &(),
        )
        .await;
    let closed = support::read_chain(&h.trace_dir).len();

    let r = h
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: 1,
            },
        )
        .await;
    assert_eq!(r.status(), 409, "{}", r.body());
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "conflict");
    assert!(
        !r.body().contains(h.receiver_token.as_str()),
        "a refused resume hands out no per-run token: {}",
        r.body()
    );

    // The store grows after the seal — a harness writing its last records on the way out.
    // `catch_up` would meet `ExportError::Sealed`; the refusal runs before it does.
    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, "s1", "written after the chain closed"),
    );
    let r = h
        .post_body(
            &resume,
            &protocol::ResumeRequest {
                expected_generation: 1,
            },
        )
        .await;
    assert_eq!(
        r.status(),
        409,
        "a closed chain is a conflict, not an internal error: {}",
        r.body()
    );
    assert_eq!(
        support::read_chain(&h.trace_dir).len(),
        closed,
        "the refused resume chained nothing onto a closed chain"
    );
}

/// Long enough that an inline seal is unmistakable against the bound below, short enough
/// that this is not the slow test in the suite.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
const IGNORED_TERM_GRACE_SECONDS: u64 = 4;

/// `Daemon::seal_entry` walks the workspace, writes the chain and then calls
/// `terminate`, which sleeps out the handle's `grace_seconds` whenever the harness survives
/// its SIGTERM. Run inline on the async worker, that would stop every other route for the
/// length of the kill — the status page and the status route included.
///
/// The harness here ignores SIGTERM, so the grace runs to its end and the seal is genuinely
/// slow. The status route has to answer while it does.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
#[tokio::test]
async fn a_seal_serving_out_its_grace_does_not_hold_the_other_routes() {
    let h = support::control_harness().await;
    let (codex, _home) = support::codex_runtime(&h).await;
    h.daemon.register(codex).expect("a codex entry registers");
    h.daemon.arm("e-codex").expect("the codex entry arms");

    // `sh` with TERM trapped: the signal lands and is ignored, so `terminate` waits out the
    // whole grace before escalating. A `while` loop cannot be exec'd away, so the process
    // the kill guard compares against `ps` stays the `sh` the handle names.
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("trap '' TERM; while true; do sleep 1; done")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a harness that ignores SIGTERM starts");
    let mut handle = support::handle_for_now(child.id());
    handle.program = "sh".to_string();
    handle.argv = vec!["/bin/sh".to_string(), "-c".to_string()];
    handle.grace_seconds = IGNORED_TERM_GRACE_SECONDS;
    h.daemon
        .register_process("e-codex", support::process_registration(1, handle))
        .expect("the process is registered");

    let client = support::ControlClient {
        addr: h.control_addr,
        token: h.control_token.clone(),
    };
    let seal_route = protocol::entry_route("e-codex", control::SUFFIX_SEAL);
    let sealing = tokio::spawn(async move { client.post(&seal_route).await });

    // The status route, asked while the seal is in its grace. It reads a map and a cache
    // and owes nobody a kill, so it answers at once — or the seal is holding the runtime.
    let status = protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS);
    let began = std::time::Instant::now();
    let r = h.get_no_token(&status).await;
    let waited = began.elapsed();
    assert_eq!(r.status(), 200, "{}", r.body());
    assert!(
        waited < std::time::Duration::from_millis(1_500),
        "the status route waited {waited:?} while a seal slept out its grace"
    );

    let sealed = sealing.await.expect("the seal request finishes");
    assert_eq!(sealed.status(), 200, "{}", sealed.body());
    let _ = child.kill();
    let _ = child.wait();
}

/// The abandon's cleanup parses `status-latest.json` and compares its `entry_id` exactly:
/// a substring match would let abandoning `e1` delete `e10`'s pointer — and `goalcup-mcp`, which
/// reads that document when not told which entry it wants, would have nothing to read at all.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn abandoning_an_entry_whose_id_prefixes_another_leaves_the_shared_pointer_alone() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    h.daemon.refresh_statuses().expect("the statuses publish");

    // The pointer, republished for a second run whose id has this one's as a strict prefix.
    // Written from the document the daemon just signed, so the shape is the real one.
    let latest = h.config_dir.join("cache").join("status-latest.json");
    let mut pointer: protocol::SignedStatus =
        serde_json::from_str(&std::fs::read_to_string(&latest).expect("the shared pointer"))
            .expect("the contract's signed document");
    let sibling = format!("{}0", info.entry_id);
    pointer.payload.entry_id = sibling.clone();
    std::fs::write(
        &latest,
        serde_json::to_vec_pretty(&pointer).expect("the pointer serialises"),
    )
    .expect("the pointer is republished");

    assert_eq!(
        h.post(&protocol::entry_route(
            &info.entry_id,
            control::SUFFIX_ABANDON
        ))
        .await
        .status(),
        204
    );
    let after: protocol::SignedStatus = serde_json::from_str(
        &std::fs::read_to_string(&latest).expect("the pointer survived the abandon"),
    )
    .expect("and is still the whole signed document");
    assert_eq!(
        after.payload.entry_id, sibling,
        "a substring test deleted the pointer for a run that is still open"
    );
}

// ------------------------------------------------------------------ post-restart rehydration

/// Through the front door: a second daemon over the same tree takes up
/// every run the first one was capturing.
///
/// Startup loads unsealed registry rows into the entry map; without that, after a restart
/// the rows would be on disk and the runs would not: `GET /v0/entries` would answer an empty list,
/// `resume` would answer `404`, tailing would stop and the deadline seal would find nothing. The
/// four assertions below are the four halves of "as if the daemon had never stopped": the
/// entry is listed, `resume` answers, the chain continues (one `DaemonRestart` over the
/// previous head — not a second chain, and not a silent join), and the tailer picks up from
/// its saved position rather than re-reading a transcript it has already shipped.
///
/// The record is appended before the stop, which is the case that matters: it is the tail a
/// kill strands, and a rehydrated tailer is the only thing that can still reach it.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_second_daemon_takes_up_every_run_the_first_one_was_capturing() {
    let h = support::control_harness().await;
    let (head_seq, head_hash) = {
        let entry = h.entry();
        (
            entry.chain_seq.expect("the first daemon chained"),
            entry.chain_head.clone(),
        )
    };
    let before = support::read_chain(&h.trace_dir).len();
    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, "s1", "the tail the stop stranded"),
    );

    let h = support::restart_control(h).await;

    let rows: Vec<protocol::EntrySummary> = h
        .get(protocol::ROUTE_ENTRIES)
        .await
        .json()
        .expect("the contract's rows");
    assert_eq!(
        rows.iter().map(|r| r.entry_id.as_str()).collect::<Vec<_>>(),
        vec![h.entry_id.as_str()],
        "an unsealed row is a run this machine is still capturing"
    );

    let r = h
        .post_body(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME),
            &protocol::ResumeRequest {
                expected_generation: 1,
            },
        )
        .await;
    assert_eq!(r.status(), 200, "{}", r.body());
    let info: protocol::ResumeInfo = r.json().expect("the contract's resume document");
    assert_eq!(info.session_id, "s1", "the run kept the session it bound");
    assert!(
        info.caught_up_events >= 1,
        "the record appended before the stop reached the chain after it"
    );

    let chain = support::read_chain(&h.trace_dir);
    let restarts: Vec<u64> = chain
        .iter()
        .filter(|e| matches!(e.body, EventBody::DaemonRestart { .. }))
        .map(|e| e.link.seq)
        .collect();
    assert_eq!(
        restarts,
        vec![head_seq + 1],
        "one restart, over the old head"
    );
    match &chain[head_seq as usize + 1].body {
        EventBody::DaemonRestart {
            previous_head_seq,
            previous_head_hash,
            ..
        } => {
            assert_eq!(*previous_head_seq, head_seq);
            assert_eq!(previous_head_hash, &head_hash, "the chain CONTINUES");
        }
        other => panic!("the link above the old head is the restart: {other:?}"),
    }
    assert!(
        !chain[..before]
            .iter()
            .any(|e| matches!(e.body, EventBody::DaemonRestart { .. })),
        "nothing below the old head moved"
    );

    let goals = chain
        .iter()
        .filter(|e| {
            serde_json::to_string(e)
                .unwrap_or_default()
                .contains("the goal")
        })
        .count();
    assert_eq!(
        goals, 1,
        "D19: `cursors.json` restored the tailer's position, so the record the FIRST daemon \
         shipped is not chained a second time under a new seq"
    );
}

/// A row this daemon cannot rebuild is one entry's failure, named, and never
/// a seal.
///
/// The corrupt chain is the case the export already refuses (`ChainCorrupt`), and what this
/// adds is what happens next. Sealing on the spot would be the worst answer available: a
/// seal is the chain's last link and the daemon writing one has to have walked the
/// workspace to do it, so a startup that sealed on a read error would publish a manifest
/// for a run it could not open. It is counted and skipped instead, and the run beside it
/// is untouched.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_row_with_a_corrupt_chain_is_counted_and_skipped_and_does_not_take_the_others_down() {
    let h = support::control_harness().await;
    let second = support::runtime_sharing_harness_home(&h).await;
    let broken = second.entry().entry_id.clone();
    let broken_chain = second.entry().trace_dir.join("chain.jsonl");
    h.daemon.register(second).expect("the second entry opens");
    assert!(broken_chain.is_file(), "and has a chain to break");

    // Half a line: the export deserialises every line on `resume` and anything it cannot
    // read is `ChainCorrupt`, which is a daemon killed mid-append.
    let text = std::fs::read_to_string(&broken_chain).expect("the chain reads");
    std::fs::write(&broken_chain, &text[..text.len() / 2]).expect("the chain is truncated");

    let h = support::restart_control(h).await;

    let rows: Vec<protocol::EntrySummary> = h
        .get(protocol::ROUTE_ENTRIES)
        .await
        .json()
        .expect("the contract's rows");
    assert_eq!(
        rows.iter().map(|r| r.entry_id.as_str()).collect::<Vec<_>>(),
        vec![h.entry_id.as_str()],
        "the healthy run came back and the broken one did not block it"
    );

    let cycles = h.daemon.cycle_errors();
    assert!(cycles.tick >= 1, "the failure is counted, not swallowed");
    let last = cycles.last.unwrap_or_default();
    assert!(
        last.contains(&broken),
        "a count that does not say WHICH run stopped is a number nobody can act on: {last}"
    );
    assert!(
        !last.contains('/'),
        "and it carries no path out of the home"
    );

    assert_eq!(
        h.daemon
            .registry()
            .get(&broken)
            .expect("the row reads")
            .expect("and is still there")
            .sealed_at,
        None,
        "a row this daemon could not open is not a row it may seal"
    );

    // "Listed" is the weakest thing a rehydrated entry can be, and what this test is really
    // about is the neighbour coming back as a run rather than as a row. So
    // the healthy one is resumed and then tailed: a record appended after the restart has to
    // reach its chain.
    let r = h
        .post_body(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME),
            &protocol::ResumeRequest {
                expected_generation: 1,
            },
        )
        .await;
    assert_eq!(r.status(), 200, "{}", r.body());
    let before = support::read_chain(&h.trace_dir).len();
    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, "s1", "the neighbour is still being tailed"),
    );
    h.daemon
        .tick()
        .await
        .expect("the tick polls the healthy run");
    let chain = support::read_chain(&h.trace_dir);
    assert!(chain.len() > before, "the tailer moved after the restart");
    assert_eq!(
        chained_once(&chain, "the neighbour is still being tailed"),
        1,
        "and the record it read is in the chain exactly once"
    );
}

/// How many chained links carry `needle` anywhere in them.
///
/// Over the whole serialised envelope rather than one field: a leg-1 payload is the
/// harness's own line and a duplicate of it is a duplicate wherever it sits, and a test that
/// reached into `EventBody::Leg1Record { payload, .. }` would stop counting the moment a
/// record arrived on any other leg.
#[cfg(feature = "v1-compat")]
fn chained_once(chain: &[goalcup_protocol::envelope::Envelope], needle: &str) -> usize {
    chain
        .iter()
        .filter(|e| {
            serde_json::to_string(e)
                .unwrap_or_default()
                .contains(needle)
        })
        .count()
}

/// The raw legacy row carries a seeded process, but it never traversed the modern
/// prepare/register path.  Rebuild must convert that process to durable termination
/// authority without treating the old seed as a new `HomeSeeded` event.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_rebuilt_raw_started_seeded_legacy_process_is_registered_once_without_a_home_seed_event()
{
    let h = support::started_entry_with_work().await;
    let mut seeded = support::recorded_launch_handle();
    seeded.home_seed = Some(goalcup_protocol::events::HomeSeedRecord {
        wire_layout: Default::default(),
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "codex-cli".to_string(),
        from: "{home}/.codex/config.toml".to_string(),
        keys: Vec::new(),
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
    });
    let expected = goalcup_protocol::process::ProcessRegistration {
        generation: 1,
        handle: seeded.clone(),
    };

    let (h, ()) = support::restart_as_raw_legacy_required(
        h,
        EntryState::Started,
        Some(seeded),
        |_| {},
        |_, raw_bytes| {
            let raw: serde_json::Value =
                serde_json::from_slice(raw_bytes).expect("the raw historical fixture is JSON");
            for key in [
                "process_handoff_generation",
                "process_handoff_phase",
                "process_handoff_digest",
                "prepared_process_handle",
                "prepared_home_seed",
                "cancelled_process_handoff_digest",
            ] {
                assert!(
                    raw.get(key).is_none(),
                    "the persisted historical fixture omits {key} before restart"
                );
            }
        },
    )
    .await;

    assert!(
        h.daemon
            .entries()
            .expect("the live entries read")
            .iter()
            .any(|entry| entry.entry_id == h.entry_id),
        "the materialized row is a live runtime, not merely a rewritten registry row"
    );
    assert_eq!(
        support::read_chain(&h.trace_dir)
            .iter()
            .filter(|envelope| matches!(envelope.body, EventBody::DaemonRestart { .. }))
            .count(),
        1,
        "the rebuilt runtime appends exactly one restart witness"
    );

    let tail = "legacy materialization restart tail";
    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, support::DAEMON_SESSION, tail),
    );
    h.daemon
        .tick()
        .await
        .expect("the rehydrated runtime tails a post-restart transcript record");
    assert_eq!(
        chained_once(&support::read_chain(&h.trace_dir), tail),
        1,
        "the live rehydrated runtime chains the post-restart record exactly once"
    );

    let row = h.entry();
    assert_eq!(
        row.process_handoff_phase(),
        goalcup_daemon::registry::ProcessHandoffPhase::Registered
    );
    assert_eq!(row.process_handoff_generation(), 1);
    assert_eq!(row.last_process, Some(expected.stripped_handle()));
    assert_eq!(
        serde_json::to_value(&row).expect("the materialized row serializes")["process_handoff_digest"],
        serde_json::json!(expected.canonical_handle_digest(&h.entry_id))
    );
    assert!(
        support::read_chain(&h.trace_dir)
            .iter()
            .all(|envelope| !matches!(envelope.body, EventBody::HomeSeeded(_))),
        "a raw old seed is not replayed as a new HomeSeeded event"
    );
}

/// An impossible state/timestamp row has no authority to rehydrate. Its rejection happens
/// after read-only row validation but before re-rendering tracker files, opening the buffer,
/// or creating the absent harness home.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn an_impossible_legacy_lifecycle_tuple_refuses_before_rebuild_side_effects() {
    let h = support::started_entry_with_work().await;
    let (h, (row_before, chain_before, home, buffer)) = support::restart_as_raw_legacy_required(
        h,
        EntryState::Armed,
        None,
        |_| {},
        |raw, raw_bytes| {
            let home = raw.harness_home.clone().expect("the raw row names a home");
            let buffer = goalcup_daemon::buffer::Buffer::default_path(&raw.artifact_dir);
            let chain_before =
                std::fs::read(raw.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
                    .expect("the pre-restart chain reads");
            std::fs::remove_dir_all(&home).expect("the historical home is absent");
            std::fs::remove_file(&buffer).expect("the historical buffer is absent");
            (raw_bytes.to_vec(), chain_before, home, buffer)
        },
    )
    .await;

    assert!(
        !home.exists() && !buffer.exists(),
        "refusal happens before register can create the home or EntryRuntime can open a buffer"
    );
    assert_eq!(
        std::fs::read(h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the chain remains readable"),
        chain_before,
        "refusal does not append a restart or otherwise alter chain bytes"
    );
    assert_eq!(
        h.daemon
            .registry()
            .raw_json_for_test(&h.entry_id)
            .expect("the refused raw row reads")
            .expect("the refused raw row remains"),
        row_before,
        "refusal preserves the persisted impossible raw row byte-for-byte"
    );
    assert!(
        h.daemon.entries().expect("the entry list reads").is_empty(),
        "the refused row never reaches the live registry map"
    );
}

/// A row that says it chained links, over a `chain.jsonl` that holds
/// none.
///
/// `open_trace` branches on the file and not on the row, so without the refusal in `rebuild`
/// this entry would take the create branch — a second `EventBody::Baseline` at seq 0, a fresh
/// chain over a run that already had links, and `sync_head` then writing the row's head
/// backwards over the one the last acknowledgement anchored. It is not a hypothetical
/// shape: `baseline.json` and `chain.jsonl` sit in one directory, and a backup tool, a
/// partial copy or a participant deleting the file that looked like a log leaves exactly
/// this.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_row_whose_chain_file_is_gone_is_skipped_rather_than_started_over() {
    let h = support::control_harness().await;
    let second = support::runtime_sharing_harness_home(&h).await;
    let broken = second.entry().entry_id.clone();
    let trace_dir = second.entry().trace_dir.clone();
    h.daemon.register(second).expect("the second entry opens");

    let row = h
        .daemon
        .registry()
        .get(&broken)
        .expect("the row reads")
        .expect("and is there");
    let head_seq = row.chain_seq.expect("the first daemon chained a baseline");
    let head_hash = row.chain_head.clone();

    std::fs::remove_file(trace_dir.join("chain.jsonl")).expect("the chain is removed");
    assert!(
        trace_dir.join("baseline.json").is_file(),
        "and `baseline.json` is still beside it, which is what makes the create branch \
         reachable"
    );

    let h = support::restart_control(h).await;

    let rows: Vec<protocol::EntrySummary> = h
        .get(protocol::ROUTE_ENTRIES)
        .await
        .json()
        .expect("the contract's rows");
    assert_eq!(
        rows.iter().map(|r| r.entry_id.as_str()).collect::<Vec<_>>(),
        vec![h.entry_id.as_str()],
        "the entry with no chain to resume is skipped, and the healthy one is not"
    );

    let after = h
        .daemon
        .registry()
        .get(&broken)
        .expect("the row reads")
        .expect("and is still there");
    assert_eq!(
        (after.chain_seq, after.chain_head.as_str()),
        (Some(head_seq), head_hash.as_str()),
        "the row's head did not move, and above all did not move BACKWARDS"
    );
    assert!(
        !trace_dir.join("chain.jsonl").exists(),
        "and nothing wrote a second genesis baseline over the run"
    );
    assert_eq!(
        after.sealed_at, None,
        "a row this daemon could not open is not a row it may seal"
    );

    let last = h.daemon.cycle_errors().last.unwrap_or_default();
    assert!(
        last.contains(&broken),
        "the skip is counted and names the entry: {last}"
    );
    assert!(
        !last.contains('/'),
        "and carries no path out of the home: {last}"
    );
}

/// A record that is in the chain and a cursor that never caught up
/// with it.
///
/// `cursors.json` is written after the chain, so a hard kill between the two leaves the
/// saved position one record behind. The tailer that restores it re-reads that record and
/// emits nothing — it went back from no offset it knew about — so the second copy lands in
/// the chain with no `Reread` above it, which V9 reads as a replay on a run that did
/// nothing wrong. The daemon saves the cursor per batch rather than per tick, and
/// compares each store's first record after a resume against what the chain already holds.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_record_the_cursor_lost_after_it_was_chained_is_not_chained_twice() {
    let h = support::control_harness().await;
    let offset_before = std::fs::metadata(&h.store_path)
        .expect("the store file")
        .len();
    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, "s1", "the record the cursor lost"),
    );
    h.daemon.tick().await.expect("the tick chains it");
    assert_eq!(
        chained_once(
            &support::read_chain(&h.trace_dir),
            "the record the cursor lost"
        ),
        1,
        "the first daemon chained it once"
    );

    // The kill: the record is in the chain and the cursor covering it never reached disk.
    support::roll_cursor_back(&h, "main", &h.store_path, offset_before, 1);

    let h = support::restart_control(h).await;
    h.daemon.tick().await.expect("the tick re-reads it");

    assert_eq!(
        chained_once(
            &support::read_chain(&h.trace_dir),
            "the record the cursor lost"
        ),
        1,
        "and the second daemon did not chain it a second time"
    );

    // …and the comparison is spent on that one record, not standing between the store and
    // everything after it.
    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, "s1", "and the next one still chains"),
    );
    h.daemon.tick().await.expect("the tick chains the new one");
    assert_eq!(
        chained_once(
            &support::read_chain(&h.trace_dir),
            "and the next one still chains"
        ),
        1,
        "a new record after the skipped one reaches the chain normally"
    );
}

/// The cursor is saved per batch, so a kill part-way through one
/// leaves it behind by however many of that batch's records were already chained.
///
/// Comparing only the first re-read record against the chain's last would catch a
/// cursor one record short and nothing wider: three chained records coming back would mean the
/// first was skipped and the second and third were chained a second time, at new seqs, with
/// no notice above them. The whole first batch
/// is scanned for the record the chain ends on, and everything up to and including it is
/// skipped.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_batch_the_cursor_lost_part_way_through_re_chains_only_what_was_not_chained() {
    let h = support::control_harness().await;
    let offset_before = std::fs::metadata(&h.store_path)
        .expect("the store file")
        .len();
    for i in 1..=3 {
        support::append_line(
            &h.store_path,
            &support::transcript_line(&h.workspace, "s1", &format!("record {i} of the batch")),
        );
    }
    h.daemon
        .tick()
        .await
        .expect("the tick chains the first three");
    let chain = support::read_chain(&h.trace_dir);
    for i in 1..=3 {
        assert_eq!(
            chained_once(&chain, &format!("record {i} of the batch")),
            1,
            "the first daemon chained record {i}"
        );
    }

    // The other two of the same window, written before the kill and never read: what comes
    // back on the first poll is one batch of five, three of which are in the chain.
    for i in 4..=5 {
        support::append_line(
            &h.store_path,
            &support::transcript_line(&h.workspace, "s1", &format!("record {i} of the batch")),
        );
    }
    support::roll_cursor_back(&h, "main", &h.store_path, offset_before, 3);

    let h = support::restart_control(h).await;
    h.daemon.tick().await.expect("the tick re-reads the batch");

    let chain = support::read_chain(&h.trace_dir);
    for i in 1..=5 {
        assert_eq!(
            chained_once(&chain, &format!("record {i} of the batch")),
            1,
            "record {i} is in the chain exactly once: the three it already held were \
             skipped, and the two the kill stranded were chained"
        );
    }
    for (index, env) in chain.iter().enumerate() {
        assert_eq!(
            env.link.seq, index as u64,
            "the sequence has no hole and no repeat: a skipped record must cost no seq"
        );
    }
}

/// A store whose cursor came back and which was read again says so.
///
/// A restored cursor that sat behind the chain re-reads silently — the tailer went back from
/// no offset it knew about, so it emits nothing — so one `Reread`, built exactly as the
/// lost-cursor path builds it and chained before the store's first resumed record, is what
/// keeps the repeat witnessed rather than silent.
///
/// The witness follows the count, not the batch: writing one whenever a
/// restored store's first batch held anything a payload compare did not match would
/// include a store that had simply been written to since the stop, and an ordinary restart would
/// chain a `Reread` about a re-read that never happened. `cursors.json` carries the chain
/// seq its position covered, so the number of records the tailer will replay is counted
/// rather than guessed, and the witness says so exactly when there are some.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_restored_store_read_again_after_a_restart_is_witnessed_before_its_first_record() {
    let h = support::control_harness().await;
    let offset_before = std::fs::metadata(&h.store_path)
        .expect("the store file")
        .len();
    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, "s1", "the tail the stop stranded"),
    );
    h.daemon.tick().await.expect("the tick chains it");
    // The kill: the record is in the chain and the cursor covering it never reached disk,
    // so the restored tailer is about to hand it back.
    support::roll_cursor_back(&h, "main", &h.store_path, offset_before, 1);

    let h = support::restart_control(h).await;
    h.daemon
        .tick()
        .await
        .expect("the tick reads the store again");

    let chain = support::read_chain(&h.trace_dir);
    let restart_at = support::seq_of_restart(&chain);
    let witnesses: Vec<&goalcup_protocol::envelope::Envelope> = chain
        .iter()
        .filter(|e| {
            matches!(&e.body, EventBody::TailNotice { kind, store, .. }
                     if *kind == goalcup_protocol::events::TailNoticeKind::Reread
                        && store == "main")
        })
        .collect();
    assert_eq!(
        witnesses.len(),
        1,
        "one witness for the store that was read again, and not one per poll"
    );
    let witness = witnesses[0];
    assert!(
        witness.link.seq > restart_at,
        "the restart is the earlier link, then what it cost"
    );
    assert_eq!(
        chained_once(&chain, "the tail the stop stranded"),
        1,
        "the record the first daemon chained is read again and not chained again"
    );
    assert!(
        support::first_leg1_seq_above(&chain, "main", restart_at).is_none(),
        "so there is no second copy for the witness to sit under"
    );
    match &witness.body {
        EventBody::TailNotice {
            path,
            offset_before,
            offset_after,
            inode_before,
            inode_after,
            ..
        } => {
            assert_eq!(
                path,
                &h.store_path.display().to_string(),
                "the witness names the file, the way the lost-cursor witness does"
            );
            assert_eq!(
                (*offset_before, *offset_after, *inode_before, *inode_after),
                (0, 0, None, None),
                "and populates the verifier-facing fields identically"
            );
        }
        other => panic!("the witness is a tail notice: {other:?}"),
    }
}

/// A skip budget is about the file the cursor was measured against, and a rewrite
/// is a different file.
///
/// The count of records to replay is taken at `open_trace`, from the chain, against the
/// position `cursors.json` saved. If the transcript is truncated or replaced before the
/// tailer gets to it, what comes back is not what the count was about — and a budget left
/// standing would silently swallow the first records of the rewritten file, which are the
/// ones a rewrite leaves. The rewrite notice explains the re-read on its own, so the
/// budget is dropped when one arrives.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_rewritten_store_drops_the_skip_budget_the_restart_computed() {
    let h = support::control_harness().await;
    let offset_before = std::fs::metadata(&h.store_path)
        .expect("the store file")
        .len();
    for i in 1..=2 {
        support::append_line(
            &h.store_path,
            &support::transcript_line(&h.workspace, "s1", &format!("record {i} before the kill")),
        );
    }
    h.daemon.tick().await.expect("the tick chains both");
    support::roll_cursor_back(&h, "main", &h.store_path, offset_before, 2);

    // …and then the transcript is rewritten under the daemon: shorter than the position it
    // saved — which is the on-disk signature of a rewrite — and holding one record that
    // has never been in the chain.
    std::fs::write(
        &h.store_path,
        b"{\"type\":\"user\",\"text\":\"the only line the rewrite left\"}\n".as_slice(),
    )
    .expect("the store is rewritten");
    assert!(
        std::fs::metadata(&h.store_path)
            .expect("the store file")
            .len()
            < offset_before,
        "the rewrite has to shrink the file, or there is no rewrite to detect"
    );

    let h = support::restart_control(h).await;
    h.daemon.tick().await.expect("the tick reads the new file");

    let chain = support::read_chain(&h.trace_dir);
    assert_eq!(
        chained_once(&chain, "the only line the rewrite left"),
        1,
        "a budget computed against the old file must not eat the new file's first records"
    );
    assert!(
        chain
            .iter()
            .any(|e| matches!(&e.body, EventBody::TailNotice { kind, .. }
                     if *kind == goalcup_protocol::events::TailNoticeKind::Truncated)),
        "and the rewrite is in the chain as its own signature (B-2)"
    );
}

/// A line the harness writes that is byte-identical to the last one it chained is
/// still a record, and the restart dedupe must not eat it.
///
/// Comparing the re-read records' payloads against the last
/// `Leg1Record` the store had in the chain, and skipping everything up to the match, answers
/// the wrong question: a harness that legitimately writes the same line
/// twice — a repeated tool result, a retried turn, an idle poll — would have its second copy
/// dropped silently, and when the match landed on the last record of the batch the whole
/// batch would go with it, with no notice and no counter. `cursors.json` records the chain
/// seq its position covered, so the number of records the tailer replays is counted off the
/// chain instead.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_new_line_identical_to_the_last_chained_record_is_still_chained() {
    let h = support::control_harness().await;
    let line = support::transcript_line(&h.workspace, "s1", "the same words twice");
    support::append_line(&h.store_path, &line);
    h.daemon.tick().await.expect("the tick chains the first");
    assert_eq!(
        chained_once(&support::read_chain(&h.trace_dir), "the same words twice"),
        1,
        "one copy so far"
    );

    // A quiet stop and start: the cursor covers what the chain holds, so nothing is
    // re-read. The harness then writes the same line again, which is a second record.
    let h = support::restart_control(h).await;
    support::append_line(&h.store_path, &line);
    h.daemon.tick().await.expect("the tick chains the second");

    let chain = support::read_chain(&h.trace_dir);
    assert_eq!(
        chained_once(&chain, "the same words twice"),
        2,
        "the harness wrote the line twice and the chain holds it twice: a dedupe that \
         compares bytes deletes evidence a participant did not fabricate"
    );
    let restart_at = support::seq_of_restart(&chain);
    assert!(
        !chain.iter().any(|e| e.link.seq > restart_at
            && matches!(&e.body, EventBody::TailNotice { kind, .. }
                        if *kind == goalcup_protocol::events::TailNoticeKind::Reread)),
        "and nothing was re-read, so nothing says it was"
    );
}

/// A cursor that was current re-read nothing, and a `Reread` over a
/// store that was not read again would be a statement about something that did not happen.
///
/// The second half is the one a stale flag breaks: nothing is skipped either, so the first
/// record the store writes after the quiet restart is chained like any other.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_restored_store_with_nothing_to_re_read_is_not_witnessed_and_skips_nothing() {
    let h = support::control_harness().await;
    h.daemon
        .tick()
        .await
        .expect("the tick leaves every cursor covering what the chain holds");

    let h = support::restart_control(h).await;
    h.daemon
        .tick()
        .await
        .expect("the first tick after the restart finds nothing to re-read");

    let chain = support::read_chain(&h.trace_dir);
    let restart_at = support::seq_of_restart(&chain);
    assert!(
        !chain.iter().any(|e| e.link.seq > restart_at
            && matches!(&e.body, EventBody::TailNotice { kind, .. }
                        if *kind == goalcup_protocol::events::TailNoticeKind::Reread)),
        "no store was read again, so no store is witnessed as having been"
    );

    support::append_line(
        &h.store_path,
        &support::transcript_line(&h.workspace, "s1", "the first record after a quiet restart"),
    );
    h.daemon.tick().await.expect("the tick chains it");
    assert_eq!(
        chained_once(
            &support::read_chain(&h.trace_dir),
            "the first record after a quiet restart"
        ),
        1,
        "the resume skipped nothing and stands between the store and nothing"
    );
}

/// A resumed gate remembers which leg carried the start
/// signal.
///
/// `see_signal` returns early once `start_signal_at` is set, so a `signal_source` left
/// `None` by `resume` could never be filled in later, and `GET …/start-signal` would answer a
/// resumed run with an empty `source` for the rest of the run. Nothing is guessed to fill
/// it: `start_signal.required` is what routes a signal, and Claude declares it `false`, so
/// leg 1 is the only door the signal could have come through.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_resumed_gate_still_says_which_leg_carried_the_start_signal() {
    let h = support::control_harness().await;
    let route = protocol::entry_route(&h.entry_id, control::SUFFIX_START_SIGNAL);
    let before: protocol::StartSignal = h
        .get(&route)
        .await
        .json()
        .expect("the contract's start signal");
    assert_eq!(
        before.source, "leg1",
        "the live gate watched leg 1 carry it"
    );

    let h = support::restart_control(h).await;

    let after: protocol::StartSignal = h
        .get(&route)
        .await
        .json()
        .expect("the contract's start signal");
    assert_eq!(
        after.source, before.source,
        "and a restart is not a reason to stop being able to say so"
    );
    assert_eq!(after.session_id, before.session_id);
}

/// A run whose corroboration is already in the chain comes back
/// corroborated.
///
/// `resume` restores `leg1_at` with `start_signal_at`; otherwise `may_stamp_started` would be false
/// on a row that had everything it needed, the run would sit in `StartSignalSeen` until
/// another record happened to arrive, and would be refused at the original timeout if none did.
/// On a connector whose first leg-1 record is the signal the two instants are one number,
/// which is what the gate is told here, and the very first tick after the restart stamps
/// the start with no new record at all.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_corroborated_row_starts_on_the_first_tick_after_a_restart() {
    let h = support::control_harness().await;
    let second = support::runtime_sharing_harness_home(&h).await;
    let pending = second.entry().entry_id.clone();
    h.daemon.register(second).expect("the second entry opens");

    // The row a daemon killed between the corroboration and the receipt leaves: the signal
    // is stamped, the session is bound, and `started_at` is not.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs_f64();
    h.daemon
        .registry()
        .update(&pending, |e| {
            e.state = goalcup_daemon::registry::EntryState::StartSignalSeen;
            e.armed_at = Some(now);
            e.start_signal_at = Some(now);
            e.session_id = Some("s2".to_string());
        })
        .expect("the row is stamped");
    h.daemon
        .registry()
        .open_initial_process_handoff(&pending)
        .expect("the reconstructed armed launch has issued its initial process generation");

    let h = support::restart_control(h).await;
    h.daemon.tick().await.expect("the unregistered tick");

    let row = h
        .daemon
        .registry()
        .get(&pending)
        .expect("the row reads")
        .expect("and is there");
    assert!(
        row.started_at.is_none(),
        "a required launch remains unstarted after restart until its process handoff lands"
    );
    h.daemon
        .register_process(
            &pending,
            support::process_registration(1, support::process_handle()),
        )
        .expect("the relaunched process registers");
    h.daemon.tick().await.expect("the registered tick");

    let row = h
        .daemon
        .registry()
        .get(&pending)
        .expect("the row reads")
        .expect("and is there");
    assert!(
        row.started_at.is_some(),
        "the corroboration the row already carried is what starts the run, and it did not \
         have to be re-earned by a fresh record"
    );
    assert_eq!(row.session_id.as_deref(), Some("s2"));
}

/// The other half: a sealed row is left where the reap found it.
///
/// Rehydrating one would open a run whose chain's last link is already written — every
/// `event`, `leg1` and `leg2` on it is `ExportError::Sealed` — so it stays out of the entry
/// map, and the refusal a participant gets for resuming it has to survive that. It does:
/// `is_sealed` asks the registry row when there is no runtime to ask, so the resume refusal is
/// still the `409` that names the seal rather than a `404` about a run whose `seal.json` is
/// on disk in front of them.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_sealed_row_is_not_rehydrated_and_resuming_it_is_still_a_conflict() {
    let h = support::control_harness().await;
    let replay = support::process_registration(
        h.entry().process_handoff_generation(),
        h.entry()
            .last_process
            .clone()
            .expect("the started fixture registered its launch"),
    );
    let _: goalcup_protocol::seal::SealManifest = h
        .post_json(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL),
            &(),
        )
        .await;
    let sealed_len = support::read_chain(&h.trace_dir).len();

    let h = support::restart_control(h).await;

    let rows: Vec<protocol::EntrySummary> = h
        .get(protocol::ROUTE_ENTRIES)
        .await
        .json()
        .expect("the contract's rows");
    assert!(rows.is_empty(), "a sealed run is not reopened");

    assert_eq!(
        h.post_body(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS),
            &replay,
        )
        .await
        .status(),
        204,
        "an exact lost-response replay is acknowledged from the sealed registry row without rehydrating it"
    );
    let mut different = replay.clone();
    different.handle.pid = different.handle.pid.saturating_add(1);
    assert_eq!(
        h.post_body(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS),
            &different,
        )
        .await
        .status(),
        409,
        "the registry-only fallback never accepts a new termination identity"
    );

    let r = h
        .post_body(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_RESUME),
            &protocol::ResumeRequest {
                expected_generation: 1,
            },
        )
        .await;
    assert_eq!(r.status(), 409, "{}", r.body());
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert_eq!(body.error.code, "conflict");

    assert_eq!(
        support::read_chain(&h.trace_dir).len(),
        sealed_len,
        "and no `DaemonRestart` was appended after the seal"
    );
}

/// A terminal arming refusal is evidence, not an unsealed capture waiting to be rebuilt.
///
/// In particular, a restart must not append its own `DaemonRestart` evidence, recreate a
/// live runtime, or turn an unstarted refusal back into an armable entry.  The registry row
/// remains the quarantine record, and every lifecycle mutation refuses without touching it.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_refused_row_stays_quarantined_across_restart_and_lifecycle_mutations() {
    fn files_below(
        root: &std::path::Path,
    ) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        fn visit(
            root: &std::path::Path,
            current: &std::path::Path,
            files: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
        ) {
            for child in std::fs::read_dir(current)
                .expect("the existing harness-home directory reads")
                .flatten()
            {
                let path = child.path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else if path.is_file() {
                    files.insert(
                        path.strip_prefix(root)
                            .expect("the walk stays under its root")
                            .to_path_buf(),
                        std::fs::read(&path).expect("the existing harness-home file reads"),
                    );
                }
            }
        }

        let mut files = std::collections::BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("codex-cli"),
        )
        .await;
    let route = |suffix| protocol::entry_route(&info.entry_id, suffix);
    let trace = std::path::PathBuf::from(&info.artifact_root).join("trace");
    let chain_before = std::fs::read(trace.join(goalcup_protocol::export::CHAIN_FILE))
        .expect("the fresh trace has its baseline");
    let home = std::path::PathBuf::from(&info.harness_home);
    let home_before = files_below(&home);

    h.daemon
        .registry()
        .update(&info.entry_id, |entry| {
            entry.state = goalcup_daemon::registry::EntryState::Refused;
        })
        .expect("the terminal refusal is persisted");
    let refused_before = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the row reads")
        .expect("the row exists");

    let h = support::restart_empty_control(h).await;

    assert!(
        h.daemon
            .entries()
            .expect("the live entries read")
            .is_empty(),
        "a refused row is quarantined rather than rehydrated"
    );
    assert_eq!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the quarantined row reads"),
        Some(refused_before.clone()),
        "restart preserves every field of the terminal evidence row"
    );
    assert_eq!(
        std::fs::read(trace.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the original trace remains readable"),
        chain_before,
        "quarantine does not append a DaemonRestart or any other trace evidence"
    );
    assert_eq!(
        files_below(&home),
        home_before,
        "quarantine does not recreate or modify the harness-home files"
    );

    for response in [
        h.post(&route(control::SUFFIX_ARM)).await,
        h.get(&route(control::SUFFIX_START_SIGNAL)).await,
        h.get(&route(control::SUFFIX_RESUME_PREVIEW)).await,
        h.post_body(
            &route(control::SUFFIX_RESUME),
            &protocol::ResumeRequest {
                expected_generation: info.process_generation,
            },
        )
        .await,
        h.post_body(
            &route(control::SUFFIX_PROCESS),
            &support::process_registration(info.process_generation, support::process_handle()),
        )
        .await,
        h.post_body(
            &route(control::SUFFIX_PROCESS_CANCEL),
            &protocol::ProcessHandoffCancelRequest {
                expected_generation: info.process_generation,
                request_digest: None,
            },
        )
        .await,
        h.post(&route(control::SUFFIX_SEAL)).await,
        h.post(&route(control::SUFFIX_ABANDON)).await,
    ] {
        assert_eq!(
            response.status(),
            409,
            "a terminal refusal is fail-closed, not an invitation to recreate the run: {}",
            response.body()
        );
    }

    assert_eq!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the row still reads"),
        Some(refused_before),
        "none of the refused lifecycle calls can overwrite or remove terminal evidence"
    );
    assert_eq!(
        std::fs::read(trace.join(goalcup_protocol::export::CHAIN_FILE))
            .expect("the original trace remains readable"),
        chain_before,
        "the refused lifecycle calls append no trace evidence"
    );
    assert_eq!(files_below(&home), home_before);
}

/// A refusal still wins if a process handoff was already persisted before it was recorded.
///
/// This deliberately keeps the runtime live: restart quarantine proves that a refused row
/// has no slot, while this route reaches the registry's unstarted-termination decision under
/// a real slot.  An abandon must neither signal the accepted child nor remove or rewrite the
/// row that explains the terminal refusal.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_refused_registered_handoff_is_never_terminated_or_abandoned() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("codex-cli"),
        )
        .await;
    let route = |suffix| protocol::entry_route(&info.entry_id, suffix);

    assert_eq!(h.post(&route(control::SUFFIX_ARM)).await.status(), 204);
    let mut child = support::spawn_harness();
    let mut handle = support::handle_for_now(child.id());
    handle.native_binary = support::manifest("codex-cli").kill.native_binary;
    assert_eq!(
        h.post_body(
            &route(control::SUFFIX_PROCESS),
            &support::process_registration(info.process_generation, handle),
        )
        .await
        .status(),
        204,
        "the row holds an accepted, unstarted termination handle"
    );

    h.daemon
        .registry()
        .update(&info.entry_id, |entry| {
            entry.state = goalcup_daemon::registry::EntryState::Refused;
        })
        .expect("the terminal refusal is durable after the handoff");
    let refused_before = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the terminal row reads")
        .expect("the terminal row remains present");
    assert_eq!(
        refused_before.process_handoff_phase(),
        goalcup_daemon::registry::ProcessHandoffPhase::Registered,
        "the durable row still names the process that abandon would otherwise terminate"
    );
    assert!(
        refused_before.last_process.is_some(),
        "the termination handle is persisted on the terminal row"
    );
    let trace = std::path::PathBuf::from(&info.artifact_root)
        .join("trace")
        .join(goalcup_protocol::export::CHAIN_FILE);
    let chain_before = std::fs::read(&trace).expect("the trace baseline reads");

    let response = h.post(&route(control::SUFFIX_ABANDON)).await;
    assert_eq!(response.status(), 409, "{}", response.body());
    assert!(
        child.try_wait().expect("the child status reads").is_none(),
        "a terminal refusal never invokes the persisted handle's termination path"
    );
    assert_eq!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the row still reads"),
        Some(refused_before),
        "abandon cannot delete or rewrite the terminal handoff evidence"
    );
    assert_eq!(
        std::fs::read(trace).expect("the trace remains readable"),
        chain_before,
        "the refused abandon records no replacement lifecycle evidence"
    );

    let _ = child.kill();
    let _ = child.wait();
}

/// End to end: the mid-upload refusal answers `409` with the body code `busy`.
///
/// `runtime::held_elsewhere` raises `DaemonError::Busy`, not `Refused`: an entry held for
/// the length of one pump must not reach a client as `conflict`, which is the code that means
/// "this will never work". A client cannot tell the two apart from the status they share, so the
/// whole rule — retry on `busy`, report `conflict` — rests on this body.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn an_entry_held_for_the_length_of_a_pump_answers_busy_and_not_conflict() {
    let h = support::control_harness().await;

    // The window, held open by hand: `bind_start` and `pump` take the runtime out of the
    // entry map exactly this way, and a call that would have to hold the slot across it is
    // refused rather than made to wait.
    let held = h.daemon.borrow_entry(&h.entry_id).expect("borrowed");
    let r = h
        .post(&protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL))
        .await;
    assert_eq!(r.status(), 409, "{}", r.body());
    let body: serde_json::Value = r.json().expect("the contract's error body");
    assert_eq!(
        body["error"]["code"],
        "busy",
        "a transient hold is `busy`; `conflict` would tell a client to give up: {}",
        r.body()
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("mid-upload")),
        "and it still says what the hold is: {}",
        r.body()
    );
    drop(held);

    // And with the runtime back, the same route works — which is what makes the retry the
    // right answer rather than a client papering over a refusal.
    let r = h
        .post(&protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL))
        .await;
    assert_eq!(r.status(), 200, "{}", r.body());
}

/// Registered replay is a registry acknowledgement, not another runtime mutation. A pump
/// may hold the live slot, but the exact immutable body must still receive a bare 204.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn an_exact_registered_replay_does_not_turn_a_runtime_hold_into_uncertainty() {
    let h = support::control_harness().await;
    let registration = goalcup_protocol::process::ProcessRegistration {
        generation: h.entry().process_handoff_generation(),
        handle: h.entry().last_process.clone().unwrap(),
    };
    let held = h.daemon.borrow_entry(&h.entry_id).expect("borrowed");
    let response = h
        .post_body(
            &protocol::entry_route(&h.entry_id, control::SUFFIX_PROCESS),
            &registration,
        )
        .await;
    assert_eq!(response.status(), 204, "{}", response.body());
    assert_eq!(response.body().as_bytes(), b"");
    drop(held);
}

// -------------------------------------------------- the `{server_port}` reservation

/// `{server_port}` has a daemon-side half: a connector whose launch or resume template names
/// `{server_port}` gets one reserved port, remembered on the row, so a resumed launch does
/// not render `--port ""`.
///
/// What decides the reservation is the template, not `launch.mode`: `mode == server` is
/// a statement about who owns the terminal and not about
/// whether an argv names a port, and a TUI can pin the port of
/// the server it embeds. The daemon reserves when any launch or resume template of the
/// connector names `{server_port}`.
///
/// No shipped connector names the port, so
/// only the negative half is asserted.
///
/// Both directions in one test, because the field is only right if it is `Some` for a
/// connector whose recipes ask for a port and `None` for the ones that never mention it —
/// a `catch_up` that answered the create's port for every entry would pass a one-sided
/// assertion and hand a launch a `--port` its recipe cannot render.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn no_port_is_reserved_for_a_connector_whose_recipes_never_name_it() {
    let h = support::empty_control_harness().await;

    // The negative case, and it is the whole other half of the rule: a connector that names
    // `{server_port}` in no template of its own reserves nothing, at create and at resume
    // alike. A port here would be a flag the recipe has nowhere to put.
    for id in ["claude-code-cli", "codex-cli"] {
        let quiet: protocol::WorkspaceInfo = h
            .post_json(protocol::ROUTE_WORKSPACES, &support::workspace_opts(id))
            .await;
        assert_eq!(
            quiet.server_port, None,
            "{id} names {{server_port}} nowhere, so it reserves none"
        );
        assert_eq!(
            h.post(&protocol::entry_route(&quiet.entry_id, control::SUFFIX_ARM))
                .await
                .status(),
            204
        );
        let mut handle = support::process_handle();
        handle.native_binary = support::manifest(id).kill.native_binary;
        assert_eq!(
            h.post_body(
                &protocol::entry_route(&quiet.entry_id, control::SUFFIX_PROCESS),
                &support::process_registration(quiet.process_generation, handle),
            )
            .await
            .status(),
            204
        );
        h.daemon
            .registry()
            .update(&quiet.entry_id, |entry| {
                entry.state = goalcup_daemon::registry::EntryState::Started;
                entry.started_at = Some("2026-09-10T00:00:00Z".to_string());
                entry.session_id = Some("port-session".to_string());
            })
            .expect("the focused resume fixture commits its session");
        let r = h
            .post_body(
                &protocol::entry_route(&quiet.entry_id, control::SUFFIX_RESUME),
                &protocol::ResumeRequest {
                    expected_generation: quiet.process_generation,
                },
            )
            .await;
        assert_eq!(r.status(), 200, "{}", r.body());
        let resumed: protocol::ResumeInfo = r.json().expect("the contract's resume document");
        assert_eq!(resumed.server_port, None, "{id}");
    }
}

// ------------------------------------------------- a workspace parent reached through a link

/// Asserted on the first run, with no restart anywhere in it.
///
/// `POST /v0/workspaces` builds the entry's [`PathVars`] after
/// `workspace::prepare` created the workspace directory. `PathVars::new` canonicalises once,
/// at construction, and falls back to the raw path when `std::fs::canonicalize` fails —
/// which it would every time, with nothing there yet. Under a workspace parent
/// reached through a symlink, which is the default shape on macOS (`/var` →
/// `/private/var`, so every tempdir, `/tmp`, and any linked home), leg 1 would be lost
/// without a word: `{slug}` would render from the logical path while the harness, which
/// reports its own `getcwd`, writes under the physical one.
///
/// A restart would hide it — `Daemon::rebuild` builds its `PathVars` over a workspace that
/// exists by then — so only a first run shows it.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
#[tokio::test]
async fn a_first_run_under_a_symlinked_workspace_parent_captures_leg_one() {
    let h = support::empty_control_harness().await;

    // `<root>/real`, with `<root>/link` pointing at it — the shape a macOS tempdir has by
    // default, built here rather than assumed, so the case exists on every platform this
    // suite runs on.
    let real_parent = h.root.join("real");
    std::fs::create_dir_all(&real_parent).expect("the real workspace parent");
    let linked_parent = h.root.join("link");
    std::os::unix::fs::symlink(&real_parent, &linked_parent).expect("the symlinked parent");

    let mut opts = support::workspace_opts("claude-code-cli");
    opts.workspace_parent = Some(linked_parent.display().to_string());
    let info: protocol::WorkspaceInfo = h.post_json(protocol::ROUTE_WORKSPACES, &opts).await;

    let logical = linked_parent.join(&info.entry_id);
    let physical = real_parent.join(&info.entry_id);
    assert_ne!(
        logical, physical,
        "the case only exists while the two spellings of one directory differ"
    );
    assert!(physical.is_dir(), "the workspace is a real directory");

    // One spelling everywhere. The CLI is answered the canonical path — the
    // string it will cache, hand back on a resume, and compare against what the daemon
    // says about this run — and the path the participant typed comes back beside it, for
    // display and for nothing else.
    assert_eq!(
        info.workspace,
        physical.display().to_string(),
        "`WorkspaceInfo.workspace` is the canonical path"
    );
    assert_eq!(
        info.workspace_requested,
        logical.display().to_string(),
        "…and the requested spelling is carried, not lost"
    );
    // The entry is bound to the canonical one, which is the spelling every glob renders
    // from and every post is compared against — and it is the same string the CLI was
    // just answered.
    let row = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the row reads")
        .expect("the entry is there");
    assert_eq!(
        row.workspace, physical,
        "the registry row holds the path `std::fs::canonicalize` returns"
    );
    assert_eq!(
        row.workspace.display().to_string(),
        info.workspace,
        "the row and the answer are one string: a cache written from this document agrees \
         with every later answer the daemon gives, including `ResumeInfo.workspace`"
    );

    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );

    assert_eq!(
        h.post_body(
            &protocol::entry_route(&info.entry_id, control::SUFFIX_PROCESS),
            &support::process_registration(info.process_generation, support::process_handle()),
        )
        .await
        .status(),
        204,
        "the launch hands over its process before the first transcript can commit start"
    );

    // Leg 1, written where a real harness writes it: under the slug of its own
    // `getcwd`, which is the physical path. Nothing in this test ever writes under the
    // logical slug, so a chained record can only mean the entry rendered `{slug}` the way
    // the harness did.
    let harness_home = std::path::PathBuf::from(&info.harness_home);
    let store = support::transcript_path(&harness_home, &physical, &info.session_id);
    support::append_line(
        &store,
        &support::transcript_line(&physical, &info.session_id, "the goal"),
    );
    h.daemon.tick().await.expect("the first tick runs");

    let trace_dir = std::path::PathBuf::from(&info.artifact_root).join("trace");
    assert_eq!(
        support::leg1_count(&trace_dir, "main"),
        1,
        "the record the harness wrote under the physical slug reached the chain"
    );

    // And `POST resume` answers the same string: the test above stops at the registry
    // row, and `goalcup resume` refuses a relaunch when the workspace the daemon answers is
    // not the one the entry cache holds. `catch_up` reads the canonical row, so the two agree
    // by construction, and the parent this runs under is the shape that would hide a
    // disagreement.
    let r = h
        .post_body(
            &protocol::entry_route(&info.entry_id, control::SUFFIX_RESUME),
            &protocol::ResumeRequest {
                expected_generation: info.process_generation,
            },
        )
        .await;
    assert_eq!(r.status(), 200, "{}", r.body());
    let resumed: protocol::ResumeInfo = r.json().expect("the contract's resume document");
    assert_eq!(
        resumed.workspace, info.workspace,
        "`ResumeInfo.workspace` is byte for byte what `create_workspace` answered, under a \
         parent reached through a symlink: a CLI comparing the two refuses a resume where \
         they differ"
    );
    assert_eq!(
        resumed.workspace,
        physical.display().to_string(),
        "…and that string is the canonical spelling, not the requested one"
    );
    assert_eq!(
        resumed.harness_home, info.harness_home,
        "the other root the CLI's lock check resolves against is one string too"
    );
}

/// A row whose `workspace` holds the logical
/// spelling, under a parent reached through a symlink. The daemon takes it
/// up and captures leg 1 anyway.
///
/// Rows created now are canonical; an older row is not, and its run may still be open on
/// the machine it was started on. `EntryRuntime::rebuild`'s workspace carries it: it
/// resolves the stored path rather than trusting it, so `{slug}` renders the way the
/// harness writes.
///
/// The row is planted by writing the logical spelling back onto a row this daemon made,
/// which is the one way to get a genuinely old-shaped row without a fixture that
/// hand-builds an entire tree — everything else about the run is what it was.
#[cfg(feature = "v1-compat")]
#[cfg(unix)]
#[tokio::test]
async fn a_row_written_before_the_fix_still_tails() {
    let h = support::empty_control_harness().await;

    let real_parent = h.root.join("real");
    std::fs::create_dir_all(&real_parent).expect("the real workspace parent");
    let linked_parent = h.root.join("link");
    std::os::unix::fs::symlink(&real_parent, &linked_parent).expect("the symlinked parent");

    let mut opts = support::workspace_opts("claude-code-cli");
    opts.workspace_parent = Some(linked_parent.display().to_string());
    let info: protocol::WorkspaceInfo = h.post_json(protocol::ROUTE_WORKSPACES, &opts).await;
    let logical = linked_parent.join(&info.entry_id);
    let physical = real_parent.join(&info.entry_id);
    assert_ne!(
        logical, physical,
        "the case only exists while the two spellings of one directory differ"
    );

    // The trace and row are aged together. An older daemon wrote the logical
    // workspace spelling into both the registry row and baseline. Rewriting only the row
    // manufactures an incoherent trace, which rebuilt construction correctly refuses.
    // Generate a fresh baseline link through the real export so its digest, link hash and
    // row mirror remain one claim.
    let trace_dir = std::path::PathBuf::from(&info.artifact_root).join("trace");
    let mut legacy_baseline: goalcup_protocol::seal::SealManifest = serde_json::from_slice(
        &std::fs::read(trace_dir.join(goalcup_protocol::export::BASELINE_FILE))
            .expect("the current baseline reads"),
    )
    .expect("the current baseline parses");
    legacy_baseline.workspace = logical.display().to_string();
    let generated = tempfile::tempdir_in(&h.root).expect("a private generator directory");
    let mut legacy_export = goalcup_protocol::export::TraceExport::create(generated.path())
        .expect("the legacy trace generator opens");
    let (_, legacy_head) = legacy_export
        .write_baseline(&legacy_baseline)
        .expect("the logical baseline is chained");
    drop(legacy_export);
    for name in [
        goalcup_protocol::export::CHAIN_FILE,
        goalcup_protocol::export::BASELINE_FILE,
    ] {
        std::fs::rename(generated.path().join(name), trace_dir.join(name))
            .expect("the coherent legacy trace file replaces the current one");
    }
    h.daemon
        .registry()
        .update(&info.entry_id, |e| {
            e.workspace = logical.clone();
            e.chain_seq = Some(legacy_head.seq);
            e.chain_head = legacy_head.hash.clone();
        })
        .expect("the row takes the pre-fix identity and generated chain head");

    // A second daemon reads it: `Daemon::rebuild` is the path that has to hold up over an
    // old row.
    let h = support::restart_empty_control(h).await;
    let row = h
        .daemon
        .registry()
        .get(&info.entry_id)
        .expect("the row reads")
        .expect("the entry is there");
    assert_eq!(
        row.workspace, logical,
        "the rehydrated row is still the old one: nothing rewrote it behind the test's back"
    );

    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );

    // Leg 1, written where the harness writes it — under the slug of its own `getcwd`,
    // which is the physical path. The stored spelling is logical, so a tailer that rendered
    // `{slug}` from the row as written would watch a directory nothing is in.
    let harness_home = std::path::PathBuf::from(&info.harness_home);
    let store = support::transcript_path(&harness_home, &physical, &info.session_id);
    support::append_line(
        &store,
        &support::transcript_line(&physical, &info.session_id, "the goal"),
    );
    h.daemon.tick().await.expect("the first tick runs");
    assert_eq!(
        support::leg1_count(&trace_dir, "main"),
        1,
        "the record under the PHYSICAL slug reached the chain, over a row spelled logically"
    );
}

/// After the seal the document says `sealed`, and the countdown stops.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_sealed_entrys_document_reads_sealed_and_stops_counting_down() {
    let h = support::control_harness().await;
    let status = protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS);

    let before: protocol::SignedStatus = h.get_no_token(&status).await.json().expect("a status");
    assert_ne!(
        before.payload.health,
        goalcup_protocol::events::HealthState::Sealed,
        "whatever rung the ladder picked, an open run is not sealed"
    );
    assert!(before.payload.sealed_at.is_none());
    assert!(before.payload.seconds_remaining.unwrap_or(0) > 0);

    h.post_json::<_, goalcup_protocol::seal::SealManifest>(
        &protocol::entry_route(&h.entry_id, control::SUFFIX_SEAL),
        &(),
    )
    .await;

    let after: protocol::SignedStatus = h.get_no_token(&status).await.json().expect("a status");
    assert!(after.verify_with(&h.run_public_key).is_ok(), "still signed");
    assert_eq!(
        after.payload.health,
        goalcup_protocol::events::HealthState::Sealed
    );
    assert_eq!(after.payload.seconds_remaining, Some(0));
    assert_eq!(
        after.payload.sealed_at,
        h.entry().sealed_at,
        "the moment the chain closed, off the row"
    );
    assert!(
        after.payload.detail.contains("sealed"),
        "the plain-words line says so too: {}",
        after.payload.detail
    );
}

/// A status the daemon has not published in this process still answers.
///
/// The in-memory cache is a map one process filled, so a daemon that restarted after a run
/// sealed must not answer `404 no status for that entry` about a run whose signed document is on
/// disk.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_status_this_process_never_published_is_read_off_the_cache_file() {
    let h = support::control_harness().await;

    // The document the run published, taken from the entry this harness holds and written
    // where `status::mirror` writes one, under an id no entry map in this process knows.
    let signed: protocol::SignedStatus = h
        .get_no_token(&protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS))
        .await
        .json()
        .expect("a status");
    let mut copy = signed.clone();
    copy.payload.entry_id = "e-restarted".to_string();
    let cache = h.config.config_dir.join("cache");
    std::fs::create_dir_all(&cache).expect("the cache directory");
    std::fs::write(
        cache.join("status-e-restarted.json"),
        serde_json::to_vec_pretty(&copy).expect("it serialises"),
    )
    .expect("the mirrored document");

    assert!(
        h.daemon
            .entries()
            .expect("the rows")
            .iter()
            .all(|e| e.entry_id != "e-restarted"),
        "no entry of that id is in the map: the file is the only source"
    );
    let read = h
        .daemon
        .signed_status("e-restarted")
        .expect("the read answers")
        .expect("the file is the answer");
    assert_eq!(read.payload.entry_id, "e-restarted");
    assert_eq!(
        h.get_no_token(&protocol::entry_route(
            "e-restarted",
            control::SUFFIX_STATUS
        ))
        .await
        .status(),
        200,
        "and the route answers it rather than 404"
    );

    // A file naming a different entry answers about nobody.
    std::fs::write(
        cache.join("status-e-someone-else.json"),
        serde_json::to_vec_pretty(&copy).expect("it serialises"),
    )
    .expect("a mismatched document");
    assert!(
        h.daemon
            .signed_status("e-someone-else")
            .expect("the read answers")
            .is_none(),
        "the payload names another entry, so it is not this entry's document"
    );
}

/// A run that never started seals, says so, and leaves the list.
///
/// Nothing uploads before the start receipt — the server refuses a chunk without one, by
/// design — so a never-started entry's acknowledgement watermark can never reach its chain
/// head, so the drain test alone would keep it in `goalcup entries` for
/// the life of the daemon, republished once a second, with no run in it.
///
/// The cadence counters are asserted here too: the failure counters read
/// `cycles {tick: 0, upload: 0, status: 0}` while a document is being re-signed every
/// second, because those count failures and nothing has failed. The passes say the
/// loops are turning.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_sealed_entry_that_never_started_is_evicted_and_its_document_says_why() {
    let h = support::empty_control_harness().await;
    let info: protocol::WorkspaceInfo = h
        .post_json(
            protocol::ROUTE_WORKSPACES,
            &support::workspace_opts("claude-code-cli"),
        )
        .await;
    assert!(
        h.daemon
            .registry()
            .get(&info.entry_id)
            .expect("the row reads")
            .expect("it is there")
            .started_at
            .is_none(),
        "this entry never started, which is the whole case"
    );

    h.daemon.refresh_statuses().expect("the statuses publish");
    h.daemon.poll_all().expect("one tick");
    h.daemon.pump_all().await.expect("one drain");
    let cycles = h.daemon.cycle_errors();
    assert!(
        cycles.last_pass_at.is_some(),
        "the loops turned, and the document says WHEN rather than how many times \
         (dogfood fix 4, P5): {cycles:?}"
    );
    assert_eq!(
        (cycles.tick, cycles.upload, cycles.status),
        (0, 0, 0),
        "and nothing failed, which is what the four failure counters are for"
    );

    h.post_json::<_, goalcup_protocol::seal::SealManifest>(
        &protocol::entry_route(&info.entry_id, control::SUFFIX_SEAL),
        &(),
    )
    .await;

    // The document the seal published, which is what a participant is left with.
    let signed: protocol::SignedStatus = h
        .get_no_token(&protocol::entry_route(
            &info.entry_id,
            control::SUFFIX_STATUS,
        ))
        .await
        .json()
        .expect("a status");
    assert_eq!(
        signed.payload.detail,
        "sealed, never started — nothing to upload"
    );

    h.daemon.pump_all().await.expect("the drain that evicts it");
    assert!(
        h.daemon
            .entries()
            .expect("the rows")
            .iter()
            .all(|e| e.entry_id != info.entry_id),
        "a sealed run with nothing in it does not stay in the list"
    );
    // …and its document still answers, off the cache file, which is what makes the
    // eviction safe.
    assert_eq!(
        h.get_no_token(&protocol::entry_route(
            &info.entry_id,
            control::SUFFIX_STATUS
        ))
        .await
        .status(),
        200
    );
}

/// The version the CLI measured survives the run, and the document shows it.
///
/// `WorkspaceOpts.harness_version` reaches the row (asserted where the workspace is
/// created), and the risk this covers is the other end: `EntryRuntime::sample_version` runs
/// only where the connector declares `version.self_updating`, and no shipped connector does
/// — so a poll must leave the number alone rather than overwrite it with an empty string.
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_poll_does_not_clear_the_version_the_cli_measured() {
    let h = support::control_harness().await;
    assert!(
        !h.daemon.manifests().iter().any(|m| m.version.self_updating),
        "no shipped connector samples its own version, which is why the CLI sends it"
    );
    let before = h.entry().harness_version;
    assert!(!before.is_empty(), "the entry was created with one");
    h.daemon.poll_all().expect("one tick");
    h.daemon.refresh_statuses().expect("the statuses publish");
    assert_eq!(h.entry().harness_version, before, "a poll changes nothing");

    let signed: protocol::SignedStatus = h
        .get_no_token(&protocol::entry_route(&h.entry_id, control::SUFFIX_STATUS))
        .await
        .json()
        .expect("a status");
    assert_eq!(
        signed.payload.harness_version.as_deref(),
        Some(before.as_str())
    );
}

/// A connector edited on disk is seen without restarting the daemon.
///
/// A map loaded once, at startup, would have `create_workspace` compare the CLI's digest
/// against a manifest this process read minutes earlier — and refuse the start with a
/// message about a directory that was there the whole time.
#[tokio::test]
async fn a_connector_edited_on_disk_is_seen_without_restarting_the_daemon() {
    let tmp = tempfile::tempdir().expect("a temporary root");
    let connectors = tmp.path().join("connectors/codex-cli");
    std::fs::create_dir_all(&connectors).expect("the connector directory");
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors/codex-cli/manifest.toml");
    let shipped = std::fs::read_to_string(&source).expect("the shipped manifest");
    let mut parsed = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(&shipped)
        .expect("the shipped manifest parses");
    parsed.foreign_scan.clear();
    let text = toml::to_string(&parsed).expect("the non-scanning subset manifest serialises");
    std::fs::write(connectors.join("manifest.toml"), &text).expect("the copy");

    let mut config = support::config_at(tmp.path(), "http://127.0.0.1:7788");
    config.connectors_dir = tmp.path().join("connectors");
    let daemon = goalcup_daemon::runtime::Daemon::start(config)
        .await
        .expect("the daemon starts");

    let range = |d: &goalcup_daemon::runtime::Daemon| {
        d.manifests()
            .into_iter()
            .find(|m| m.id == "codex-cli")
            .expect("the connector")
            .tested_version_range
    };
    let first = range(&daemon);
    std::fs::write(
        connectors.join("manifest.toml"),
        text.replace(
            &format!("tested_version_range = \"{first}\""),
            "tested_version_range = \"=9.9.9\"",
        ),
    )
    .expect("the edit a participant makes to a connector");
    assert_eq!(range(&daemon), first, "the cached map is what it was");

    let fresh = daemon.reload_manifests().expect("the directory is re-read");
    assert_eq!(
        fresh
            .get("codex-cli")
            .expect("the connector")
            .tested_version_range,
        "=9.9.9",
        "the reload `create_workspace` runs sees the file on disk"
    );
    assert_eq!(
        range(&daemon),
        "=9.9.9",
        "and the map it left behind carries it"
    );
    drop(daemon);
}

/// A gate that has already refused answers now, not after the poll slice.
///
/// `arming_of` maps `GateState::Refused` to `None` and does that for no other state, so the
/// information is right there — treated exactly like `Armed`, the loop would sleep
/// out the whole requested wait, up to `MAX_START_SIGNAL_WAIT_MS`, before `arming_timeout`
/// reported the refusal in the gate's own words. `ArmingState::Refused` exists so a
/// reader can tell "will never arm" from "still pending".
#[cfg(feature = "v1-compat")]
#[tokio::test]
async fn a_gate_that_has_already_refused_answers_without_waiting_out_the_poll() {
    let h = support::empty_control_harness().await;
    let mut opts = support::workspace_opts("claude-code-cli");
    // Nothing will ever arrive, and the window is already over.
    opts.arming_timeout_seconds = Some(0);
    let info: protocol::WorkspaceInfo = h.post_json(protocol::ROUTE_WORKSPACES, &opts).await;
    assert_eq!(
        h.post(&protocol::entry_route(&info.entry_id, control::SUFFIX_ARM))
            .await
            .status(),
        204
    );
    // One tick past the window settles the gate: no signal ever arrived, so the refusal is
    // `StartSignalMissing`.
    h.daemon.tick().await.expect("the tick runs");

    let route = protocol::entry_route(&info.entry_id, control::SUFFIX_START_SIGNAL);
    let began = std::time::Instant::now();
    let r = h.get(&format!("{route}?timeout_ms=10000")).await;
    let waited = began.elapsed();

    assert_eq!(r.status(), 408, "{}", r.body());
    assert!(
        waited < std::time::Duration::from_secs(2),
        "a settled refusal is answered now, not after the requested ten seconds: {waited:?}"
    );
    let body: protocol::ErrorBody = r.json().expect("the contract's error body");
    assert!(
        !body.error.detail.is_null(),
        "and the detail is the gate's own on-screen words: {:?}",
        body.error
    );
}

/// D42: a default build mounts one stub on every V1-only route, behind the same bearer gate. An
/// anonymous caller meets `401` as on any other route; an authenticated one is told the path
/// belongs to the V1 transcript path, for a capture entry's id and for an unknown one alike (the
/// stub never reads the id). `resume` and `resume-preview` are the exception: they refuse a
/// capture entry exactly as a `v1-compat` build does.
#[cfg(not(feature = "v1-compat"))]
#[tokio::test]
async fn a_default_build_answers_every_v1_route_with_the_stub_behind_the_bearer() {
    let h = support::capture_control_harness().await;
    let entry = h.entry_id.clone();
    let routes: [(&str, String); 5] = [
        ("POST", protocol::ROUTE_WORKSPACES.to_string()),
        ("POST", protocol::entry_route(&entry, control::SUFFIX_ARM)),
        (
            "POST",
            protocol::entry_route("no-such-entry", control::SUFFIX_PROCESS_CANCEL),
        ),
        (
            "GET",
            protocol::entry_route(&entry, control::SUFFIX_START_SIGNAL),
        ),
        (
            "POST",
            protocol::entry_route("no-such-entry", control::SUFFIX_RESUME),
        ),
    ];
    for (method, path) in routes {
        let anonymous = h
            .request(method, &path, None, Vec::new(), String::new())
            .await;
        assert_eq!(anonymous.status(), 401, "{method} {path}");
        let answered = if method == "GET" {
            h.get(&path).await
        } else {
            h.post(&path).await
        };
        assert_eq!(
            answered.status(),
            404,
            "{method} {path}: {}",
            answered.body()
        );
        let body: protocol::ErrorBody = answered.json().expect("the contract's error body");
        assert_eq!(body.error.code, "not_found", "{method} {path}");
        assert!(
            body.error.message.contains("v1-compat"),
            "{method} {path} names the missing feature: {}",
            body.error.message
        );
    }

    // A capture entry is refused as one, not as a missing route.
    for path in [
        protocol::entry_route(&entry, control::SUFFIX_RESUME),
        protocol::entry_route(&entry, control::SUFFIX_RESUME_PREVIEW),
    ] {
        let refused = if path.ends_with(control::SUFFIX_RESUME_PREVIEW) {
            h.get(&path).await
        } else {
            h.post(&path).await
        };
        assert_eq!(refused.status(), 409, "{path}: {}", refused.body());
        assert!(
            refused.body().contains("a capture entry cannot be resumed"),
            "{path}: {}",
            refused.body()
        );
    }
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
            (protocol::ROUTE_UI.to_string(), false),
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

    let addr = h.client.addr.to_string();
    let body = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(&addr).expect("connect");
        write!(
            s,
            "GET {} HTTP/1.1\r\nHost: evil-marker.example:{port}\r\nConnection: close\r\n\r\n",
            protocol::ROUTE_UI
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
