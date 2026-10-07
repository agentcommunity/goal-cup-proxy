//! The CLI's half of the contract, asserted at compile time. If this file stops compiling,
//! the control API changed under the CLI: fix it in `goalcup-protocol` and re-serve it in
//! the daemon, never here.

use goalcup_protocol::control::{
    ArmingState, CONTROL_API_VERSION, ControlError, DaemonClient, DaemonHealth, EntrySummary,
    QUERY_TIMEOUT_MS, ROUTE_ENTRIES, ROUTE_ENTRIES_CURRENT, ROUTE_HEALTH, ROUTE_UI,
    ROUTE_WORKSPACES, ResumeInfo, ResumePreviewInfo, ResumeRequest, STALE_AFTER_SECONDS,
    SUFFIX_ABANDON, SUFFIX_ARM, SUFFIX_PROCESS, SUFFIX_RESUME, SUFFIX_RESUME_PREVIEW, SUFFIX_SEAL,
    SUFFIX_START_SIGNAL, SUFFIX_STATUS, SealResponse, SignedStatus, StartSignal, WorkspaceInfo,
    WorkspaceOpts, entry_route, is_public_route, start_signal_route,
};
use goalcup_protocol::manifest::InstallScope;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use std::time::Duration;

/// Every call site the CLI makes, in one function that must type-check.
#[allow(dead_code)]
fn the_twelve_methods(c: &dyn DaemonClient, h: &ProcessHandle) {
    let opts = WorkspaceOpts {
        connector_id: "claude-code-cli".into(),
        manifest_sha256: "ab".repeat(32),
        workspace_parent: None,
        attach: false,
        deadline_hours: 168,
        grant_token: "grant".into(),
        arming_timeout_seconds: None,
        harness_version: Some("2.1.259".into()),
    };
    let _: Result<WorkspaceInfo, ControlError> = c.create_workspace("claude-code-cli", &opts);
    let _: Result<(), ControlError> = c.arm("e1");
    let _: Result<StartSignal, ControlError> = c.wait_start_signal("e1", Duration::from_secs(1));
    let _: Result<SignedStatus, ControlError> = c.status("e1");
    let _: Result<Vec<EntrySummary>, ControlError> = c.entries();
    let resume = ResumeRequest {
        expected_generation: 1,
    };
    let _: Result<ResumeInfo, ControlError> = c.resume("e1", &resume);
    let _: Result<ResumePreviewInfo, ControlError> = c.resume_preview("e1");
    let _: Result<SealResponse, ControlError> = c.seal("e1");
    let registration = ProcessRegistration {
        generation: 1,
        handle: h.clone(),
    };
    let _: Result<(), ControlError> = c.register_process("e1", &registration);
    let _: Result<DaemonHealth, ControlError> = c.health();
    let _: Result<Option<String>, ControlError> = c.current_entry();
    let _: Result<(), ControlError> = c.abandon("e1", "arming timed out");
}

/// The daemon renders and writes the config; the CLI is told what landed where.
/// Every field the CLI reads, in one place that must compile.
#[allow(dead_code)]
fn the_cli_reads_but_never_writes(
    info: &WorkspaceInfo,
    r: &ResumeInfo,
    preview: &ResumePreviewInfo,
) {
    let _settings: Option<&String> = info
        .installed_files
        .iter()
        .find(|f| f.scope == InstallScope::ArtifactRoot)
        .map(|f| &f.path);
    let _: &Vec<String> = &info.workspace_relative;
    let (_, _, _) = (&info.workspace, &info.artifact_root, &info.harness_home);
    let _ = &info.run_public_key;
    let (_, _, _) = (
        &info.status_url,
        &info.status_cache_path,
        &info.status_line_path,
    );
    let (_, _): (&String, &Option<u16>) = (&info.session_id, &info.server_port);
    let _: u64 = r.caught_up_events;
    let _: &Option<ProcessHandle> = &r.last_process;
    let (_, _) = (&preview.workspace, &preview.status_line_path);
}

#[test]
fn the_routes_and_the_suffixes_are_the_ones_this_plan_calls() {
    assert_eq!(CONTROL_API_VERSION, 5);
    assert_eq!(ROUTE_WORKSPACES, "/v0/workspaces");
    assert_eq!(ROUTE_ENTRIES, "/v0/entries");
    assert_eq!(ROUTE_ENTRIES_CURRENT, "/v0/entries/current");
    assert_eq!(ROUTE_HEALTH, "/v0/health");
    assert_eq!(ROUTE_UI, "/v0/ui");
    assert_eq!(STALE_AFTER_SECONDS, 120, "one threshold, three readers");
    // The suffixes are constants, not literals typed on both sides.
    assert_eq!(entry_route("e1", SUFFIX_ARM), "/v0/entries/e1/arm");
    assert_eq!(
        entry_route("e1", SUFFIX_START_SIGNAL),
        "/v0/entries/e1/start-signal"
    );
    assert_eq!(entry_route("e1", SUFFIX_STATUS), "/v0/entries/e1/status");
    assert_eq!(entry_route("e1", SUFFIX_PROCESS), "/v0/entries/e1/process");
    assert_eq!(entry_route("e1", SUFFIX_ABANDON), "/v0/entries/e1/abandon");
    assert_eq!(entry_route("e1", SUFFIX_RESUME), "/v0/entries/e1/resume");
    assert_eq!(
        entry_route("e1", SUFFIX_RESUME_PREVIEW),
        "/v0/entries/e1/resume-preview"
    );
    assert_eq!(entry_route("e1", SUFFIX_SEAL), "/v0/entries/e1/seal");
    assert_eq!(QUERY_TIMEOUT_MS, "timeout_ms");
    assert_eq!(
        start_signal_route("e1", Duration::from_millis(1_500)),
        "/v0/entries/e1/start-signal?timeout_ms=1500"
    );
    // A deadline so large it would wrap is clamped, never wrapped: a wrapped long-poll
    // returns instantly and reads to a participant as "the signal never came".
    assert!(start_signal_route("e1", Duration::MAX).ends_with(&u64::MAX.to_string()));
    assert_eq!(
        serde_json::to_string(&ArmingState::Started).unwrap(),
        "\"started\""
    );
}

/// The authentication rule, as one function both sides call. `status` is PUBLIC:
/// treating every trait route as authed would put a bearer token on an unauthenticated
/// route on every `goalcup status`. Nothing under `/v0/assets/` is: no route serves it.
#[test]
fn exactly_the_two_public_routes_are_public_and_status_is_one_of_them() {
    assert!(is_public_route(ROUTE_UI));
    assert!(!is_public_route("/v0/assets/anything-else.png"));
    assert!(is_public_route(&entry_route("e1", SUFFIX_STATUS)));
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
    for suffix in [
        SUFFIX_ARM,
        SUFFIX_START_SIGNAL,
        SUFFIX_RESUME,
        SUFFIX_RESUME_PREVIEW,
        SUFFIX_SEAL,
        SUFFIX_PROCESS,
        SUFFIX_ABANDON,
    ] {
        let route = entry_route("e1", suffix);
        assert!(
            !is_public_route(&route),
            "{route} must carry the bearer token"
        );
    }
    // The query string is not part of the routing decision, so the long-poll route is
    // still authed with its parameter attached.
    assert!(!is_public_route(&start_signal_route(
        "e1",
        Duration::from_millis(50)
    )));
    // Traversal, and its percent-encoded spelling, are never public (the
    // client must ask about the RAW path, which is the string the server will see).
    assert!(!is_public_route("/v0/ui/../entries"));
    assert!(!is_public_route("/v0/entries/%2e%2e/status"));
    assert!(
        !is_public_route("/v0/entries/%zz/status"),
        "a malformed escape is not public"
    );
}

/// Every wire code round-trips. `internal` also remains the fallback for an unknown code;
/// `unreachable` deliberately does not — it is what a client says when nothing answered,
/// so it can never arrive on a wire.
#[test]
fn every_mapped_error_code_round_trips_and_unreachable_is_not_one_of_them() {
    for code in [
        "not_found",
        "conflict",
        "unauthorized",
        "arming_timeout",
        "preflight_failed",
        "unsupported_connector",
        "malformed",
        "unsupported",
        "process_admission_uncertain",
        "process_admission_busy",
        "busy",
    ] {
        assert_eq!(ControlError::from_code(code, "x").code(), code);
    }
    // The fallback round-trips too, by being the fallback.
    assert_eq!(ControlError::from_code("internal", "x").code(), "internal");
    // `busy` and `conflict` share 409 and mean the opposite things: one is retried, one is
    // reported. They must not collapse into each other here.
    assert_ne!(
        ControlError::from_code("busy", "x").code(),
        ControlError::from_code("conflict", "x").code()
    );
    // An unrecognised code becomes Internal, never a silent success — and so does
    // `unreachable`, which no daemon ever sends.
    assert_eq!(ControlError::from_code("brand_new", "x").code(), "internal");
    assert_eq!(
        ControlError::from_code("unreachable", "x").code(),
        "internal"
    );
}
