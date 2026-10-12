use ed25519_dalek::SigningKey;
use goalcup_protocol::control::{
    ArmingState, CONTROL_API_VERSION, ControlError, CycleErrors, DaemonHealth, EntrySummary,
    EventStatus, EventStatusSource, InstalledFile, ProcessHandoffCancelRequest, ROUTE_ENTRIES,
    ROUTE_ENTRIES_CURRENT, ROUTE_HEALTH, ROUTE_UI, ROUTE_WORKSPACES, ResumeInfo, ResumePreviewInfo,
    ResumeRequest, STALE_AFTER_SECONDS, SignedStatus, StartSignal, StatusPayload,
    WorkspaceFinalizationStatusV1, WorkspaceInfo, WorkspaceOpts, entry_route, is_public_route,
};
use goalcup_protocol::events::HealthState;
use goalcup_protocol::manifest::{InstallScope, KillStrategy};
use goalcup_protocol::process::ProcessHandle;

fn payload() -> StatusPayload {
    StatusPayload {
        entry_id: "entry-0001".into(),
        connector_id: "claude-code-cli".into(),
        workspace: "/Users/team/goalcup/ws-0001".into(),
        session_id: Some("95addb64-9dac-4046-8ef6-8aa53fa0c7f1".into()),
        harness_version: Some("2.1.259".into()),
        arming: ArmingState::StartSignalSeen,
        health: HealthState::Online,
        started_at: Some("2026-09-04T09:00:00Z".into()),
        deadline_at: Some("2026-09-11T09:00:00Z".into()),
        sealed_at: None,
        seconds_remaining: Some(604_800),
        last_sync_at: Some("2026-09-04T09:00:12Z".into()),
        events_uploaded: 206,
        events_pending: 0,
        arming_recipe: None,
        detail: "capturing".into(),
        event: EventStatus {
            t0: "2026-09-04T00:00:00Z".into(),
            start_window_closes_at: "2026-09-05T00:00:00Z".into(),
            publication_closes_at: "2026-09-12T00:00:00Z".into(),
            rules_version: "2026-09-03".into(),
        },
        event_status_source: EventStatusSource::Config,
        cycles: CycleErrors::default(),
        posts_dropped: 0,
        posts_dropped_after_seal: 0,
        pins_dropped: 0,
        polls_capped: 0,
        cursor_restore_failures: 0,
        skipped_replayed: 0,
        buffer_gap_at: None,
        files_capped: 0,
        harness_unverified: false,
        finalization: None,
        capture: None,
        producers_consumed: None,
        admissions_refused: None,
    }
}

#[test]
fn signed_status_adds_an_exact_optional_workspace_finalization_projection() {
    use goalcup_protocol::provenance::{
        PreCutoffDrainFailureV1, PreCutoffDrainOutcomeV1, Rfc3339Utc, TerminationOutcomeV1,
    };
    use goalcup_protocol::submission::{SubmissionFailureV1, SubmissionOutcomeV1};

    let legacy = payload();
    let legacy_wire = serde_json::to_value(&legacy).unwrap();
    assert!(
        legacy_wire.get("finalization").is_none(),
        "an ordinary V1-V3 status must keep its historical wire shape",
    );
    assert_eq!(
        serde_json::from_value::<StatusPayload>(legacy_wire)
            .unwrap()
            .finalization,
        None,
        "an old signed status decodes without inventing finalization state",
    );

    let checked_at = Rfc3339Utc::try_from("2026-09-14T00:00:03Z").unwrap();
    let cases = [
        (
            WorkspaceFinalizationStatusV1::Finishing {},
            serde_json::json!({ "state": "finishing" }),
        ),
        (
            WorkspaceFinalizationStatusV1::Incomplete {
                pre_cutoff_drain: Some(PreCutoffDrainOutcomeV1::Degraded {
                    reason: PreCutoffDrainFailureV1::RestartStateAmbiguous,
                }),
                termination: None,
                submission: Some(SubmissionOutcomeV1::Incomplete {
                    reason: SubmissionFailureV1::RecoveryAmbiguous,
                }),
            },
            serde_json::json!({
                "state": "incomplete",
                "pre_cutoff_drain": { "degraded": { "reason": "restart_state_ambiguous" } },
                "termination": null,
                "submission": { "incomplete": { "reason": "recovery_ambiguous" } }
            }),
        ),
        (
            WorkspaceFinalizationStatusV1::Terminal {
                pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
                termination: TerminationOutcomeV1::AlreadyStopped {
                    checked_at: checked_at.clone(),
                },
                submission: SubmissionOutcomeV1::Incomplete {
                    reason: SubmissionFailureV1::ReadbackFailed,
                },
            },
            serde_json::json!({
                "state": "terminal",
                "pre_cutoff_drain": "complete",
                "termination": { "already_stopped": { "checked_at": "2026-09-14T00:00:03Z" } },
                "submission": { "incomplete": { "reason": "readback_failed" } }
            }),
        ),
    ];

    for (status, expected) in cases {
        let actual = serde_json::to_value(&status).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            serde_json::from_value::<WorkspaceFinalizationStatusV1>(actual).unwrap(),
            status,
        );
    }

    let mut unknown = serde_json::json!({ "state": "finishing" });
    unknown["guessed_live_workspace"] = serde_json::json!(true);
    assert!(
        serde_json::from_value::<WorkspaceFinalizationStatusV1>(unknown).is_err(),
        "status cannot silently accept a live/path-derived finalization fact",
    );
}

#[test]
fn workspace_opts_and_info_use_the_documented_field_names() {
    let opts = WorkspaceOpts {
        connector_id: "codex-cli".into(),
        manifest_sha256: "ab".repeat(32),
        workspace_parent: None,
        attach: false,
        deadline_hours: 168,
        grant_token: "grant-abc".into(),
        arming_timeout_seconds: None,
        // The CLI's own measurement, which the daemon writes onto the entry row
        // rather than probing for itself.
        harness_version: Some("0.153.0".into()),
    };
    let v: serde_json::Value = serde_json::to_value(&opts).unwrap();
    for key in [
        "connector_id",
        "manifest_sha256",
        "workspace_parent",
        "attach",
        "deadline_hours",
        "grant_token",
        "arming_timeout_seconds",
        "harness_version",
    ] {
        assert!(
            v.get(key).is_some(),
            "WorkspaceOpts must carry {key}, got {v}"
        );
    }

    let info = WorkspaceInfo {
        entry_id: "entry-0001".into(),
        process_generation: 1,
        workspace: "/Users/team/goalcup/ws-0001".into(),
        workspace_requested: "/Users/team/goalcup/ws-0001".into(),
        artifact_root: "/Users/team/Library/Application Support/goalcup/runs/entry-0001".into(),
        harness_home: "/Users/team/Library/Application Support/goalcup/homes/entry-0001".into(),
        run_public_key: "ef".repeat(32),
        status_url: "http://127.0.0.1:53310/v0/ui?entry=entry-0001".into(),
        status_cache_path: "/Users/team/Library/Application Support/goalcup/cache/status-entry-0001.json".into(),
        status_line_path: "/Users/team/Library/Application Support/goalcup/runs/entry-0001/status-line.txt".into(),
        session_id: "95addb64-9dac-4046-8ef6-8aa53fa0c7f1".into(),
        server_port: Some(7788),
        installed_files: vec![
            InstalledFile {
                path: "/Users/team/Library/Application Support/goalcup/runs/entry-0001/settings-launch.json".into(),
                scope: InstallScope::ArtifactRoot,
                relative_path: "settings-launch.json".into(),
                sha256: "11".repeat(32),
                bytes: 4_096,
            },
            InstalledFile {
                path: "/Users/team/goalcup/ws-0001/.claude/settings.json".into(),
                scope: InstallScope::Workspace,
                relative_path: ".claude/settings.json".into(),
                sha256: "22".repeat(32),
                bytes: 512,
            },
        ],
        workspace_relative: vec![".claude/settings.json".into()],
        created_at: "2026-09-04T08:59:00Z".into(),
    };
    let back: WorkspaceInfo = serde_json::from_str(&serde_json::to_string(&info).unwrap()).unwrap();
    assert_eq!(info, back);
    // The daemon renders and writes; the CLI is told what landed where.
    assert_eq!(
        info.workspace_relative,
        vec![".claude/settings.json".to_string()]
    );
    assert!(info.installed_files.iter().any(
        |f| f.scope == InstallScope::ArtifactRoot && f.relative_path == "settings-launch.json"
    ));
}

#[test]
fn authority_bearing_control_bodies_reject_unknown_fields() {
    for (name, body) in [
        (
            "resume",
            serde_json::json!({ "expected_generation": 2, "unexpected": true }),
        ),
        (
            "process cancel",
            serde_json::json!({
                "expected_generation": 2,
                "request_digest": "ab",
                "unexpected": true,
            }),
        ),
    ] {
        let accepted = match name {
            "resume" => serde_json::from_value::<ResumeRequest>(body).is_ok(),
            "process cancel" => serde_json::from_value::<ProcessHandoffCancelRequest>(body).is_ok(),
            _ => unreachable!("the table only names typed request bodies"),
        };
        assert!(!accepted, "{name} accepts no undeclared authority field");
    }
}

#[test]
fn signed_status_verifies_and_rejects_a_rewritten_deadline() {
    let key = SigningKey::from_bytes(&[9u8; 32]);
    let pinned = hex::encode(key.verifying_key().to_bytes());
    let signed = SignedStatus::sign(payload(), "2026-09-04T09:00:13Z".into(), &key).unwrap();
    signed
        .verify_with(&pinned)
        .expect("freshly signed status verifies");

    let text = serde_json::to_string(&signed).unwrap();
    let mut back: SignedStatus = serde_json::from_str(&text).unwrap();
    back.verify_with(&pinned)
        .expect("round-tripped status verifies");

    back.payload.deadline_at = Some("2099-01-01T00:00:00Z".into());
    assert!(
        back.verify_with(&pinned).is_err(),
        "a rewritten deadline must not verify"
    );
}

#[test]
fn a_status_verifies_only_against_the_run_key_the_caller_pins() {
    // The status route is unauthenticated by ruling, so a local process that binds the
    // control port first can serve a status signed with its own key. Checking the
    // document against the key the document itself names would report that forgery as
    // trusted; the caller pins `WorkspaceInfo.run_public_key` instead.
    let run = SigningKey::from_bytes(&[9u8; 32]);
    let impostor = SigningKey::from_bytes(&[10u8; 32]);
    let pinned = hex::encode(run.verifying_key().to_bytes());

    let genuine = SignedStatus::sign(payload(), "2026-09-04T09:00:13Z".into(), &run).unwrap();
    assert!(genuine.verify_with(&pinned).is_ok());

    let forged = SignedStatus::sign(payload(), "2026-09-04T09:00:13Z".into(), &impostor).unwrap();
    assert!(
        forged.verify_with(&pinned).is_err(),
        "a status signed with a key the caller did not pin must not verify"
    );

    // Domain separation: the status bytes are not the bare `{cached_at, payload}` JSON.
    assert!(
        genuine
            .signing_bytes()
            .unwrap()
            .starts_with(goalcup_protocol::control::STATUS_DOMAIN)
    );
}

#[test]
fn signed_status_reports_staleness_against_a_supplied_now() {
    let key = SigningKey::from_bytes(&[9u8; 32]);
    let signed = SignedStatus::sign(payload(), "2026-09-04T09:00:00Z".into(), &key).unwrap();
    assert_eq!(signed.stale_seconds("2026-09-04T09:02:30Z"), Some(150));
    assert_eq!(signed.stale_seconds("2026-09-04T09:00:00Z"), Some(0));
    assert_eq!(signed.stale_seconds("not-a-timestamp"), None);
}

#[test]
fn one_staleness_threshold_and_the_three_unauthenticated_routes() {
    // `goalcup status` reads this one number.
    assert_eq!(STALE_AFTER_SECONDS, 120);
    let key = SigningKey::from_bytes(&[9u8; 32]);
    let signed = SignedStatus::sign(payload(), "2026-09-04T09:00:00Z".into(), &key).unwrap();
    assert!(signed.stale_seconds("2026-09-04T09:02:30Z").unwrap() > STALE_AFTER_SECONDS);
    assert!(signed.stale_seconds("2026-09-04T09:01:00Z").unwrap() < STALE_AFTER_SECONDS);

    // The loopback-only, read-only routes the bundled page uses. `entry_route` builds
    // the per-entry status path the page polls; `/v0/entries/current` is NOT among them
    // and the page must never fetch it.
    assert_eq!(ROUTE_UI, "/v0/ui");
    assert_eq!(
        entry_route("entry-0001", "status"),
        "/v0/entries/entry-0001/status"
    );
    assert_eq!(ROUTE_ENTRIES_CURRENT, "/v0/entries/current");
    assert_ne!(
        ROUTE_ENTRIES_CURRENT,
        entry_route("entry-0001", "status"),
        "the page polls the per-entry route, which is unauthenticated; `current` is not"
    );

    // The signed status carries nothing secret, which is what makes an unauthenticated
    // loopback route acceptable. These are the fields a token would otherwise ride in.
    let text = serde_json::to_string(&signed).unwrap();
    for secret in [
        "receiver_token",
        "control_token",
        "grant_token",
        "buffer_key",
    ] {
        assert!(
            !text.contains(secret),
            "StatusPayload must stay secret-free: {secret}"
        );
    }
}

/// Every `.rs` file under `dir`, recursing into subdirectories (the crate's
/// source is not guaranteed to stay a flat list of files under `src/`, so a grep limited
/// to one named file would silently stop covering the crate the moment a module grows a
/// submodule directory).
fn rust_files_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            rust_files_under(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// `ROUTE_SPONSORS` does not exist: the sponsor list is markup inside `STATUS_HTML`, and no
/// route serves it. This greps the crate's own source for the name rather than referencing
/// the removed symbol, so a reintroduced `ROUTE_SPONSORS` fails this test loudly instead of
/// a compile error nobody runs.
///
/// It walks every `crates/goalcup-protocol/src/**/*.rs`, not just `src/control.rs`: a
/// `ROUTE_SPONSORS` reintroduced in a different file (or a new submodule) is just as much a
/// reintroduction as one in `control.rs`.
#[test]
fn route_sponsors_stays_gone() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files_under(&src_dir, &mut files);
    assert!(
        !files.is_empty(),
        "expected to find .rs files under {}",
        src_dir.display()
    );
    for path in files {
        let src =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            !src.contains("ROUTE_SPONSORS"),
            "{}: the sponsor list is inline markup in STATUS_HTML now; the route constant \
             must not come back",
            path.display()
        );
    }
}

/// The auth rule is a function, and this is the test that pins it. The daemon's middleware
/// and the CLI's `HttpDaemonClient` both call `is_public_route`, and
/// `goalcup-protocol/tests/it/control_consumer.rs` asserts the same six lines from the
/// consuming side. Public means **exactly** the two routes the rule names.
#[test]
fn exactly_two_routes_are_public_and_everything_else_is_authed() {
    assert!(is_public_route(ROUTE_UI));
    // `ROUTE_SPONSORS` does not exist: the sponsor list is markup inside the bundled page.
    // Nothing serves `/v0/assets/*` either, so no prefix is public for it.
    assert!(
        !is_public_route("/v0/assets/sponsors.json"),
        "a prefix with no route behind it is not public"
    );
    assert!(!is_public_route("/v0/assets/anything-else.png"));
    assert!(is_public_route(&entry_route("e1", "status")));
    assert!(
        is_public_route("/v0/entries/e1/status?poll=1"),
        "a query string is not part of the routing decision"
    );

    for authed in [
        ROUTE_WORKSPACES,
        ROUTE_ENTRIES,
        ROUTE_ENTRIES_CURRENT,
        ROUTE_HEALTH,
    ] {
        assert!(
            !is_public_route(authed),
            "{authed} must carry the bearer token"
        );
    }
    for verb in [
        "arm",
        "start-signal",
        "resume",
        "seal",
        "process",
        "abandon",
    ] {
        let route = entry_route("e1", verb);
        assert!(
            !is_public_route(&route),
            "{route} must carry the bearer token"
        );
    }
    // Neither an empty id nor a nested path buys a free pass.
    assert!(!is_public_route("/v0/entries//status"));
    assert!(!is_public_route("/v0/entries/e1/nested/status"));
}

#[test]
fn arming_state_and_error_codes_serialise_as_the_contract_spells_them() {
    assert_eq!(
        serde_json::to_string(&ArmingState::Armed).unwrap(),
        "\"armed\""
    );
    assert_eq!(
        serde_json::to_string(&ArmingState::StartSignalSeen).unwrap(),
        "\"start_signal_seen\""
    );
    assert_eq!(
        serde_json::to_string(&ArmingState::Started).unwrap(),
        "\"started\""
    );
    // The `Refused` variant (the gate will never arm) must carry
    // the same wire discipline as its three siblings above — serialise as `"refused"`
    // and round-trip back through `serde_json::from_str`.
    assert_eq!(
        serde_json::to_string(&ArmingState::Refused).unwrap(),
        "\"refused\""
    );
    assert_eq!(
        serde_json::from_str::<ArmingState>("\"refused\"").unwrap(),
        ArmingState::Refused
    );

    let e = ControlError::from_code("arming_timeout", "no SessionStart in 300 s");
    assert!(matches!(e, ControlError::ArmingTimeout { .. }));
    assert_eq!(e.code(), "arming_timeout");
    assert_eq!(ControlError::from_code("weird", "x").code(), "internal");
}

/// `from_code` must round-trip every code `.code()` ever emits over the wire.
///
/// `ControlError::Unreachable` is the one variant deliberately left out — it is never in
/// a JSON error body (if the daemon cannot be reached, there is no body to carry a code
/// at all), so nothing has ever called `from_code("unreachable", …)` and nothing should:
/// a stranger daemon that DID send that code back is unrecognised on purpose, and
/// `from_code`'s "an unrecognised code becomes `Internal`, never a silent success" rule
/// covers it exactly like any other code this crate has never heard of.
#[test]
fn from_code_round_trips_every_wire_code_except_unreachable() {
    let cases: &[ControlError] = &[
        ControlError::NotFound("x".into()),
        ControlError::Conflict("x".into()),
        ControlError::Unauthorized("x".into()),
        ControlError::ArmingTimeout { detail: "x".into() },
        ControlError::PreflightFailed("x".into()),
        ControlError::UnsupportedConnector("x".into()),
        ControlError::Malformed("x".into()),
        ControlError::Unsupported("x".into()),
        ControlError::ProcessAdmissionUncertain("x".into()),
        ControlError::ProcessAdmissionBusy("x".into()),
        ControlError::Busy("x".into()),
        ControlError::FinishDeadline("x".into()),
        ControlError::Internal("x".into()),
    ];
    for e in cases {
        let code = e.code();
        let back = ControlError::from_code(code, "x");
        assert_eq!(
            back.code(),
            code,
            "from_code(\"{code}\", …).code() must be \"{code}\" again"
        );
    }

    // The two codes the CLI raises itself, for a malformed response and for an unsupported
    // platform: those are not "the daemon raised something this client cannot name", they
    // are exactly nameable codes and must not fall through to `Internal`.
    assert!(matches!(
        ControlError::from_code("malformed", "bad json"),
        ControlError::Malformed(m) if m == "bad json"
    ));
    assert!(matches!(
        ControlError::from_code("unsupported", "windows"),
        ControlError::Unsupported(m) if m == "windows"
    ));

    // `busy` and `conflict` share a status and must not share a variant. A client is told
    // to retry on exactly one of them, so a `from_code` that folded `busy` into `Conflict`
    // — or into `Internal`, the way an unrecognised code goes — would leave the retry rule
    // with nothing to read.
    assert!(matches!(
        ControlError::from_code("busy", "mid-upload"),
        ControlError::Busy(m) if m == "mid-upload"
    ));
    assert!(matches!(
        ControlError::from_code("conflict", "sealed"),
        ControlError::Conflict(m) if m == "sealed"
    ));
}

#[test]
fn the_remaining_types_round_trip() {
    let sig = StartSignal {
        entry_id: "entry-0001".into(),
        signal: "SessionStart".into(),
        session_id: Some("thread-1".into()),
        observed_at: "2026-09-04T09:00:05Z".into(),
        source: "leg2".into(),
    };
    assert_eq!(
        sig,
        serde_json::from_str(&serde_json::to_string(&sig).unwrap()).unwrap()
    );

    let sum = EntrySummary {
        entry_id: "entry-0001".into(),
        connector_id: "opencode".into(),
        workspace: "/tmp/ws".into(),
        arming: ArmingState::Armed,
        health: HealthState::ArmingPending,
        started_at: None,
        deadline_at: None,
        sealed_at: None,
    };
    assert_eq!(
        sum,
        serde_json::from_str(&serde_json::to_string(&sum).unwrap()).unwrap()
    );

    let r = ResumeInfo {
        entry_id: "entry-0001".into(),
        process_generation: 2,
        session_id: "thread-1".into(),
        caught_up_events: 12,
        workspace: "/tmp/ws".into(),
        artifact_root: "/tmp/root".into(),
        harness_home: "/tmp/home".into(),
        status_line_path: "/tmp/root/status-line.txt".into(),
        server_port: None,
        last_process: Some(ProcessHandle {
            pid: 4242,
            pgid: None,
            native_pid: Some(4243),
            native_binary: Some("native-harness".to_string()),
            native_process_start: Some("native-fixture-start".to_string()),
            process_start: Some("fixture-start".to_string()),
            strategy: KillStrategy::NativePid,
            grace_seconds: 5,
            launched_at: "2026-09-04T09:00:00Z".into(),
            program: "/opt/homebrew/bin/claude".into(),
            argv: vec![
                "--settings".into(),
                "/runs/entry-0001/settings-launch.json".into(),
            ],
            env_keys: vec!["CLAUDE_CONFIG_DIR".into(), "OTEL_LOG_RAW_API_BODIES".into()],
            home_seed: None,
        }),
    };
    assert_eq!(
        r,
        serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap()
    );

    let preview = ResumePreviewInfo {
        entry_id: r.entry_id.clone(),
        process_generation: r.process_generation,
        process_handoff_phase: "registered".to_string(),
        session_id: r.session_id.clone(),
        workspace: r.workspace.clone(),
        artifact_root: r.artifact_root.clone(),
        harness_home: r.harness_home.clone(),
        status_line_path: r.status_line_path.clone(),
        server_port: r.server_port,
    };
    assert_eq!(
        preview,
        serde_json::from_str(&serde_json::to_string(&preview).unwrap()).unwrap()
    );
    let preview_json: serde_json::Value = serde_json::to_value(&preview).unwrap();
    assert_eq!(
        preview_json.as_object().unwrap().len(),
        9,
        "the read-only preview carries the seven launch pins plus authoritative handoff diagnostics"
    );

    // Twelve serialized fields, including every process identity the daemon checks, are
    // pinned by name:
    // they cross the wire through `register_process` and are stored in the registry, so
    // a rename here silently breaks the launch hand-off.
    let handle_json: serde_json::Value =
        serde_json::to_value(r.last_process.as_ref().unwrap()).unwrap();
    for key in [
        "pid",
        "pgid",
        "native_pid",
        "native_binary",
        "native_process_start",
        "process_start",
        "strategy",
        "grace_seconds",
        "launched_at",
        "program",
        "argv",
        "env_keys",
    ] {
        assert!(
            handle_json.get(key).is_some(),
            "ProcessHandle must carry {key}"
        );
    }
    assert_eq!(handle_json.as_object().unwrap().len(), 12);

    let h = DaemonHealth {
        control_api_version: CONTROL_API_VERSION,
        daemon_version: "0.1.0".into(),
        uptime_seconds: 90,
        entries_open: 1,
        upload_ok: true,
    };
    assert_eq!(
        h,
        serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap()
    );
    assert_eq!(
        CONTROL_API_VERSION, 5,
        "the capture workspace request changed and requires a coordinated control boundary",
    );
}

#[test]
fn capture_workspace_requires_one_canonical_connector_directory_digest() {
    use goalcup_protocol::control::{CaptureWorkspaceInfo, CaptureWorkspaceRequest};

    let request = serde_json::json!({
        "workspace": "/private/tmp/run/workspace",
        "profile_id": "codex-cli",
        "connector_dir_sha256": "ab".repeat(32),
        "reserve_bytes": 4 * 1024 * 1024,
        "run_home": "/private/tmp/run/home",
        "config_home": "/private/tmp/config/codex-cli"
    });
    let decoded: CaptureWorkspaceRequest =
        serde_json::from_value(request.clone()).expect("the v5 request shape");
    assert_eq!(serde_json::to_value(decoded).unwrap(), request);

    let mut missing = request.clone();
    missing
        .as_object_mut()
        .unwrap()
        .remove("connector_dir_sha256");
    assert!(serde_json::from_value::<CaptureWorkspaceRequest>(missing).is_err());

    let mut uppercase = request;
    uppercase["connector_dir_sha256"] = serde_json::Value::String("AB".repeat(32));
    assert!(serde_json::from_value::<CaptureWorkspaceRequest>(uppercase).is_err());

    let response = CaptureWorkspaceInfo {
        entry_id: "a1".repeat(8),
        process_generation: 1,
        reserved_bytes: 4 * 1024 * 1024,
        ingest_addr: "127.0.0.1:31000".parse().unwrap(),
        ingest_secret: "not-printed".into(),
        producer_id: "b2".repeat(16),
        run_public_key: "cd".repeat(32),
    };
    let response = serde_json::to_value(response).unwrap();
    assert!(response.get("connector_dir_sha256").is_none());
}

#[test]
fn a_dot_segment_never_reaches_a_public_route() {
    use goalcup_protocol::control::is_public_route;
    assert!(is_public_route("/v0/ui"));
    assert!(is_public_route("/v0/entries/e1/status"));

    for path in [
        "/v0/entries/../status",
        "/v0/entries/./../../v0/entries",
        "/v0/entries/../entries/e1/status",
        "/v0/ui/../health",
        // The same traversal in the alphabet a URL decoder speaks. A decoder sits between
        // this function and the filesystem, so the encoded spelling must be refused too.
        "/v0/entries/%2e%2e/status",
        "/v0/entries/%2E%2E/status",
        "/v0/entries/%2e/status",
        // A `%` that opens no valid escape: undecodable, so not decidable, so not public.
        "/v0/entries/bad%zz/status",
        // The prefix that used to be public, with and without a dot segment.
        "/v0/assets/../entries/e1/seal",
        "/v0/assets/x",
    ] {
        assert!(!is_public_route(path), "`{path}` must carry a bearer token");
    }
    // A well-formed escape that spells nothing traversing stays public: the rule refuses
    // `.` and `..`, not percent-encoding.
    assert!(is_public_route("/v0/entries/id%20a/status"));
    assert!(!is_public_route("/v0/entries/current"));
}

#[test]
fn debug_never_prints_a_live_token() {
    use goalcup_protocol::control::WorkspaceOpts;
    let opts = WorkspaceOpts {
        connector_id: "codex-cli".to_string(),
        manifest_sha256: "ab".repeat(32),
        workspace_parent: None,
        attach: false,
        deadline_hours: 168,
        grant_token: "gt_SUPERSECRET".to_string(),
        arming_timeout_seconds: None,
        harness_version: Some("0.153.0".to_string()),
    };
    let printed = format!("{opts:?}");
    assert!(!printed.contains("SUPERSECRET"));
    assert!(printed.contains("<redacted>"));
}

/// `format_z` is `parse_z`'s inverse, and the pair is asserted against fixed points rather
/// than only against itself: a formatter that agreed with a broken reader would round-trip
/// perfectly and still write the wrong day.
///
/// The generator that writes the adversarial fixture's envelopes is a caller:
/// every `recorded_at` it stamps for a leg-1 record is a second it computed,
/// and a fixture whose timestamps drift by a day is a fixture whose planted heartbeat hole
/// silently stops containing anything.
#[test]
fn format_z_is_the_inverse_of_parse_z_including_over_a_leap_day() {
    use goalcup_protocol::control::{format_z, parse_z};

    // Fixed points, read from the outside: each string and its second, stated twice.
    let known: [(&str, u64); 6] = [
        ("1970-01-01T00:00:00Z", 0),
        ("2000-02-29T12:34:56Z", 951_827_696),
        ("2024-02-29T23:59:59Z", 1_709_251_199),
        ("2026-09-03T07:44:00Z", 1_788_421_440),
        ("2026-09-04T00:00:00Z", 1_788_480_000),
        ("2100-03-01T00:00:00Z", 4_107_542_400),
    ];
    for (text, secs) in known {
        assert_eq!(parse_z(text), Some(secs), "parse_z({text})");
        assert_eq!(format_z(secs), text, "format_z({secs})");
    }

    // And the round trip over a stride that crosses every month boundary, leap years
    // included, so the two algorithms cannot agree on a handful of dates by accident.
    let mut t = 0u64;
    while t < 4_200_000_000 {
        assert_eq!(
            parse_z(&format_z(t)),
            Some(t),
            "round trip at {t}: {}",
            format_z(t)
        );
        t += 86_400 * 13 + 3_607;
    }
}

/// The seal response is an additive control shape. Historical daemons returned a bare
/// `SealManifest`, so the response must remain untagged while the V4 workspace response
/// carries only daemon-owned terminal facts. A caller cannot mistake an unknown V4 field
/// for a legacy manifest and silently reopen the live workspace.
#[test]
fn seal_response_preserves_legacy_bytes_and_closes_the_workspace_shape() {
    use goalcup_protocol::control::{FinishReportV1, SealResponse};
    use goalcup_protocol::provenance::{Rfc3339Utc, TerminationFailureV1, TerminationOutcomeV1};
    use goalcup_protocol::seal::SealManifest;
    use goalcup_protocol::submission::{SubmissionFailureV1, SubmissionOutcomeV1};

    let legacy_json = serde_json::json!({
        "entry_id": "legacy-entry",
        "sealed_at": "2026-09-11T00:00:00Z",
        "workspace": "/private/tmp/legacy",
        "files": [],
        "excluded": [".git"]
    });
    let decoded: SealResponse = serde_json::from_value(legacy_json.clone()).unwrap();
    assert_eq!(
        decoded,
        SealResponse::Legacy(serde_json::from_value::<SealManifest>(legacy_json.clone()).unwrap())
    );
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy_json);

    let workspace = SealResponse::Workspace(FinishReportV1 {
        entry_id: "entry-v4".to_string(),
        trace_dir: "/private/var/goalcup/artifacts/entry-v4/trace".to_string(),
        termination: TerminationOutcomeV1::Unresolved {
            requested_at: Rfc3339Utc::try_from("2026-09-14T00:00:02Z").unwrap(),
            stopped_waiting_at: Rfc3339Utc::try_from("2026-09-14T00:00:03Z").unwrap(),
            reason: TerminationFailureV1::ConfirmationTimedOut,
        },
        submission: SubmissionOutcomeV1::Incomplete {
            reason: SubmissionFailureV1::RecoveryAmbiguous,
        },
    });
    let workspace_json = serde_json::to_value(&workspace).unwrap();
    assert_eq!(
        workspace_json,
        serde_json::json!({
            "entry_id": "entry-v4",
            "trace_dir": "/private/var/goalcup/artifacts/entry-v4/trace",
            "termination": {
                "unresolved": {
                    "requested_at": "2026-09-14T00:00:02Z",
                    "stopped_waiting_at": "2026-09-14T00:00:03Z",
                    "reason": "confirmation_timed_out"
                }
            },
            "submission": {
                "incomplete": { "reason": "recovery_ambiguous" }
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<SealResponse>(workspace_json.clone()).unwrap(),
        workspace
    );

    let mut future = workspace_json;
    future
        .as_object_mut()
        .unwrap()
        .insert("future_required_fact".to_string(), serde_json::json!(true));
    assert!(
        serde_json::from_value::<SealResponse>(future).is_err(),
        "an unknown workspace response must not downgrade to the legacy arm"
    );
}

// ---------------------------------------------------------------- H-10 4a: one finish budget

#[test]
fn capture_seal_request_remaining_defaults_clamps_and_refuses_zero() {
    use goalcup_protocol::control::{
        CaptureSealRequest, FINISH_COMMIT_RESERVE, LOCAL_CAPTURE_FINISH_BUDGET,
    };
    use std::time::Duration;

    // The shipped constants: the one 10-second budget and its 2-second commit share.
    assert_eq!(LOCAL_CAPTURE_FINISH_BUDGET, Duration::from_secs(10));
    assert_eq!(FINISH_COMMIT_RESERVE, Duration::from_secs(2));

    // Absent: an older CLI's body decodes, and the finish gets the full budget.
    let old: CaptureSealRequest = serde_json::from_str(r#"{"proxy":null}"#).unwrap();
    assert_eq!(old.remaining_finish_ms, None);
    assert_eq!(old.remaining_finish(), LOCAL_CAPTURE_FINISH_BUDGET);
    // And the field is not written when absent, so an old daemon's strict decode still reads
    // exactly what an old CLI sent.
    assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"proxy":null}"#);

    // Present: whole milliseconds, carried exactly.
    let some = CaptureSealRequest {
        proxy: None,
        remaining_finish_ms: Some(1_500),
        native_scope: None,
    };
    assert_eq!(
        serde_json::to_string(&some).unwrap(),
        r#"{"proxy":null,"remaining_finish_ms":1500}"#
    );
    assert_eq!(some.remaining_finish(), Duration::from_millis(1_500));

    // Larger than the budget: clamped to it. Zero: zero, which the daemon refuses.
    let large = CaptureSealRequest {
        proxy: None,
        remaining_finish_ms: Some(u64::MAX),
        native_scope: None,
    };
    assert_eq!(large.remaining_finish(), LOCAL_CAPTURE_FINISH_BUDGET);
    let zero = CaptureSealRequest {
        proxy: None,
        remaining_finish_ms: Some(0),
        native_scope: None,
    };
    assert_eq!(zero.remaining_finish(), Duration::ZERO);

    // Strict decoding stays: an undeclared field is refused.
    assert!(
        serde_json::from_str::<CaptureSealRequest>(r#"{"proxy":null,"remaining_finish_s":3}"#)
            .is_err()
    );
}

/// Hermes lane B, S7: the late native scope rides on the finish request. Absent, the body is
/// exactly an older CLI's; present, it is carried verbatim; and it is a declared field, so a
/// daemon that predates it refuses the whole request as malformed rather than finishing without it.
#[test]
fn capture_seal_request_carries_a_late_native_scope_only_when_there_is_one() {
    use goalcup_protocol::control::CaptureSealRequest;

    let old: CaptureSealRequest = serde_json::from_str(r#"{"proxy":null}"#).unwrap();
    assert_eq!(old.native_scope, None);
    assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"proxy":null}"#);

    let with = CaptureSealRequest {
        proxy: None,
        remaining_finish_ms: Some(3_000),
        native_scope: Some("20260930_141500_a1b2c3".to_string()),
    };
    let text = serde_json::to_string(&with).unwrap();
    assert_eq!(
        text,
        r#"{"proxy":null,"remaining_finish_ms":3000,"native_scope":"20260930_141500_a1b2c3"}"#
    );
    assert_eq!(
        serde_json::from_str::<CaptureSealRequest>(&text).unwrap(),
        with
    );
}

#[test]
fn finish_deadline_is_its_own_wire_code_and_never_busy() {
    let error = ControlError::FinishDeadline("workspace_walk".into());
    assert_eq!(error.code(), "finish_deadline");
    let back = ControlError::from_code("finish_deadline", "workspace_walk");
    assert!(matches!(back, ControlError::FinishDeadline(ref stage) if stage == "workspace_walk"));
    assert_ne!(error.code(), ControlError::Busy("x".into()).code());
    assert_ne!(error.code(), ControlError::Conflict("x".into()).code());
}

#[test]
fn a_capture_seal_is_not_admitted_only_for_busy_unauthorized_and_a_refused_connection() {
    use goalcup_protocol::control::CaptureSealError;

    // In-process clients carry no transport origin: only the two framed pre-handler codes are
    // provably before the finish.
    for (error, not_admitted) in [
        (ControlError::Busy("x".into()), true),
        (ControlError::Unauthorized("x".into()), true),
        (ControlError::NotFound("x".into()), false),
        (ControlError::Malformed("x".into()), false),
        (ControlError::Conflict("x".into()), false),
        (ControlError::Internal("x".into()), false),
        (ControlError::Unreachable("x".into()), false),
        (ControlError::FinishDeadline("x".into()), false),
    ] {
        let code = error.code();
        let classified = CaptureSealError::from_unclassified(error);
        assert_eq!(classified.not_admitted(), not_admitted, "{code}");
        assert_eq!(classified.into_error().code(), code);
    }
}

#[test]
fn the_termination_bound_is_the_documented_one() {
    use goalcup_protocol::process::{KILL_SWEEP_TIMEOUT_MS, RECONFIRM_DELAY_MS, termination_bound};
    use std::time::Duration;
    assert_eq!(KILL_SWEEP_TIMEOUT_MS, 2_000);
    assert_eq!(RECONFIRM_DELAY_MS, 250);
    // TERMINATION.md "The duration bound": two graces, two kill sweeps, one reconfirm.
    assert_eq!(termination_bound(0), Duration::from_millis(4_250));
    assert_eq!(termination_bound(10), Duration::from_millis(24_250));
    // Saturating, never a panic, for a manifest value no connector ships.
    let _ = termination_bound(u64::MAX);
}
