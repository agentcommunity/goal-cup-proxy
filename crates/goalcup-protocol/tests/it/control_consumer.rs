//! The CLI's half of the contract, asserted at compile time. If this file stops compiling,
//! the control API changed under the CLI: fix it in `goalcup-protocol` and re-serve it in
//! the daemon, never here.

use goalcup_protocol::control::{
    ArmingState, CONTROL_API_VERSION, CaptureWorkspaceInfo, ControlError, DaemonClient,
    DaemonHealth, EntrySummary, ROUTE_ENTRIES, ROUTE_HEALTH, STALE_AFTER_SECONDS, SUFFIX_ABANDON,
    SUFFIX_PROCESS, SUFFIX_SEAL, SUFFIX_STATUS, SealResponse, SignedStatus, entry_route,
    is_public_route,
};
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};

/// Every call site the CLI makes, in one function that must type-check.
#[allow(dead_code)]
fn the_trait_methods(c: &dyn DaemonClient, h: &ProcessHandle) {
    let _: Result<SignedStatus, ControlError> = c.status("e1");
    let _: Result<Vec<EntrySummary>, ControlError> = c.entries();
    let _: Result<SealResponse, ControlError> = c.seal("e1");
    let registration = ProcessRegistration {
        generation: 1,
        handle: h.clone(),
    };
    let _: Result<(), ControlError> = c.register_process("e1", &registration);
    let _: Result<DaemonHealth, ControlError> = c.health();
    let _: Result<(), ControlError> = c.abandon("e1", "arming timed out");
}

/// Every field the CLI reads, in one place that must compile.
#[allow(dead_code)]
fn the_cli_reads_but_never_writes(info: &CaptureWorkspaceInfo) {
    let (_, _, _) = (&info.entry_id, &info.run_public_key, &info.trace_dir);
    let _: u64 = info.process_generation;
}

#[test]
fn the_routes_and_the_suffixes_are_the_ones_this_plan_calls() {
    assert_eq!(CONTROL_API_VERSION, 8);
    assert_eq!(ROUTE_ENTRIES, "/v0/entries");
    assert_eq!(ROUTE_HEALTH, "/v0/health");
    assert_eq!(STALE_AFTER_SECONDS, 120);
    // The suffixes are constants, not literals typed on both sides.
    assert_eq!(entry_route("e1", SUFFIX_STATUS), "/v0/entries/e1/status");
    assert_eq!(entry_route("e1", SUFFIX_PROCESS), "/v0/entries/e1/process");
    assert_eq!(entry_route("e1", SUFFIX_ABANDON), "/v0/entries/e1/abandon");
    assert_eq!(entry_route("e1", SUFFIX_SEAL), "/v0/entries/e1/seal");
    assert_eq!(
        serde_json::to_string(&ArmingState::Started).unwrap(),
        "\"started\""
    );
}

/// The authentication rule, as one function both sides call. `status` is PUBLIC:
/// treating every trait route as authed would put a bearer token on an unauthenticated
/// route on every `goalcup status`. Nothing under `/v0/assets/` is: no route serves it.
#[test]
fn exactly_one_route_is_public_and_it_is_status() {
    assert!(
        !is_public_route("/v0/ui"),
        "the removed V1 page is not public"
    );
    assert!(!is_public_route("/v0/assets/anything-else.png"));
    assert!(is_public_route(&entry_route("e1", SUFFIX_STATUS)));
    for authed in [ROUTE_ENTRIES, ROUTE_HEALTH] {
        assert!(
            !is_public_route(authed),
            "{authed} must carry the bearer token"
        );
    }
    for suffix in [SUFFIX_SEAL, SUFFIX_PROCESS, SUFFIX_ABANDON] {
        let route = entry_route("e1", suffix);
        assert!(
            !is_public_route(&route),
            "{route} must carry the bearer token"
        );
    }
    // The query string is not part of the routing decision, so an authed route is still
    // authed with a parameter attached.
    assert!(!is_public_route("/v0/entries/e1/seal?timeout_ms=50"));
    // Traversal, and its percent-encoded spelling, are never public (the
    // client must ask about the RAW path, which is the string the server will see).
    assert!(!is_public_route("/v0/entries/e1/status/../../seal"));
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
        "preflight_failed",
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
