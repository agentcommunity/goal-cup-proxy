//! `HttpDaemonClient` over one transport, and the mock every command test runs against.
//!
//! The two halves of this file test two different things on purpose. The bearer rule, the
//! wire codes and the body shape are properties of a REQUEST, so they are asserted against a
//! real socket. The mock holds no headers and models none; what it is for is call order and
//! a signature a test can pin a key against.
//!
//! The `busy` retry is asserted on BOTH, which is the point of where it lives: it is
//! `retry::retry_busy`, above the trait, so the same helper covers a command driven by the
//! wire client and a command driven by the mock.

use crate::common::support;

use goalcup_cli::CliError;
use goalcup_cli::daemon::{CALL_TIMEOUT, HttpDaemonClient, MockDaemonClient, SLOW_CALL_TIMEOUT};
use goalcup_cli::retry::{BUSY_ATTEMPTS, BUSY_BACKOFF, BUSY_BUDGET, retry_busy, retry_busy_within};
use goalcup_protocol::control::{
    self, ArmingState, ControlError, DaemonClient, EntrySummary, SUFFIX_SEAL, SUFFIX_STATUS,
    entry_route, is_public_route,
};
use goalcup_protocol::events::HealthState;
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use std::time::{Duration, Instant};

fn process_registration() -> ProcessRegistration {
    ProcessRegistration {
        generation: 7,
        handle: ProcessHandle {
            pid: 1234,
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: None,
            strategy: KillStrategy::NativePid,
            grace_seconds: 5,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/usr/bin/true".to_string(),
            argv: vec!["true".to_string()],
            env_keys: Vec::new(),
        },
    }
}

fn version_header() -> Vec<(String, String)> {
    vec![(
        "x-goalcup-control-version".to_string(),
        goalcup_protocol::control::CONTROL_API_VERSION.to_string(),
    )]
}

#[test]
fn process_registration_accepts_only_a_bare_204_with_one_matching_version_header() {
    let version = goalcup_protocol::control::CONTROL_API_VERSION.to_string();
    let cases = [
        ("missing header", 204, Vec::new(), Vec::new()),
        (
            "duplicate header",
            204,
            vec![
                ("x-goalcup-control-version".to_string(), version.clone()),
                ("x-goalcup-control-version".to_string(), version.clone()),
            ],
            Vec::new(),
        ),
        (
            "wrong header",
            204,
            vec![("x-goalcup-control-version".to_string(), "999".to_string())],
            Vec::new(),
        ),
        (
            "204 JSON null body",
            204,
            version_header(),
            b"null".to_vec(),
        ),
        (
            "204 whitespace body",
            204,
            version_header(),
            b" \t\r\n".to_vec(),
        ),
        ("200 json body", 200, version_header(), b"{}".to_vec()),
        ("another 2xx", 201, version_header(), Vec::new()),
    ];

    for (name, status, headers, body) in cases {
        let srv = support::raw_response_daemon(status, headers, body);
        let client = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();
        let failure = client
            .register_process("e1", &process_registration())
            .expect_err("{name} is not a process acknowledgement");
        assert!(
            matches!(failure, ControlError::Malformed(_)),
            "{name}: {failure:?}"
        );
    }

    let srv = support::raw_response_daemon(204, version_header(), Vec::new());
    let client = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();
    let result = client.register_process("e1", &process_registration());
    assert!(
        result.is_ok(),
        "a contract 204 must acknowledge: {result:?}"
    );
}

#[test]
fn a_chunked_204_process_reply_is_not_a_bare_acknowledgement() {
    let srv = support::raw_chunked_response_daemon(204, version_header(), b"null".to_vec());
    let client = HttpDaemonClient::new(srv.base(), "control-token".into()).unwrap();

    let failure = client
        .register_process("e1", &process_registration())
        .expect_err("a Transfer-Encoding response is not a bare 204 acknowledgement");
    assert!(matches!(failure, ControlError::Malformed(_)), "{failure:?}");
    assert_eq!(
        srv.last().1.as_deref(),
        Some("Bearer control-token"),
        "the raw protected route received the normal control bearer"
    );
}

#[test]
fn process_registration_reports_truncated_and_non_json_replies_as_malformed() {
    let registration = process_registration();
    let client = HttpDaemonClient::new(support::truncated_daemon().base(), "t".into()).unwrap();
    let failure = client
        .register_process("e1", &registration)
        .expect_err("a truncated acknowledgement is not an acknowledgement");
    assert!(matches!(failure, ControlError::Malformed(_)), "{failure:?}");

    let srv = support::raw_response_daemon(200, version_header(), b"not json".to_vec());
    let client = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();
    let failure = client
        .register_process("e1", &registration)
        .expect_err("a malformed acknowledgement is not an acknowledgement");
    assert!(matches!(failure, ControlError::Malformed(_)), "{failure:?}");
}

#[test]
fn a_framed_process_refusal_keeps_its_wire_code() {
    for (status, code) in [
        (400, "malformed"),
        (401, "unauthorized"),
        (404, "not_found"),
        (409, "busy"),
        (409, "process_admission_busy"),
        (409, "process_admission_uncertain"),
        (422, "preflight_failed"),
    ] {
        let srv = support::raw_response_daemon(
            status,
            version_header(),
            format!(r#"{{"error":{{"code":"{code}","message":"route refusal"}}}}"#).into_bytes(),
        );
        let client = HttpDaemonClient::new(srv.base(), "control-token".into()).unwrap();

        let failure = client
            .register_process("e1", &process_registration())
            .expect_err("a daemon refusal is surfaced");

        assert_eq!(failure.code(), code, "the framed {status} body survives");
        assert_eq!(
            srv.last().1.as_deref(),
            Some("Bearer control-token"),
            "the protected {status} route received the control bearer"
        );
    }
}

#[test]
fn the_bearer_follows_is_public_route_on_the_raw_path_in_both_directions() {
    // A recording listener that answers 200 and remembers every Authorization header.
    let srv = support::recording_daemon();
    let c = HttpDaemonClient::new(srv.base(), "control-token".into()).expect("a loopback base");
    let entry = "e1";

    let _ = c.status(entry);
    let (path, auth) = srv.last();
    assert_eq!(path, entry_route(entry, SUFFIX_STATUS));
    assert!(is_public_route(&path));
    assert_eq!(auth, None, "the status route is public: no bearer, ever");

    let _ = c.entries();
    let (path, auth) = srv.last();
    assert_eq!(path, control::ROUTE_ENTRIES);
    assert_eq!(auth.as_deref(), Some("Bearer control-token"));

    let _ = c.seal(entry);
    let (path, auth) = srv.last();
    assert_eq!(path, entry_route(entry, SUFFIX_SEAL));
    assert!(auth.is_some(), "every mutating route keeps the token");
}

#[test]
fn a_daemon_that_is_not_there_is_unreachable_and_a_wire_code_survives_the_wire() {
    let c =
        HttpDaemonClient::new("http://127.0.0.1:1".into(), "t".into()).expect("a loopback base");
    assert!(matches!(c.health(), Err(ControlError::Unreachable(_))));

    let srv = support::erroring_daemon(409, "conflict", "already sealed at 2026-09-05T00:00:00Z");
    let c = HttpDaemonClient::new(srv.base(), "t".into()).expect("a loopback base");
    match c.seal("e1") {
        Err(ControlError::Conflict(m)) => assert!(m.contains("already sealed")),
        other => panic!("the wire codes must survive the wire: {other:?}"),
    }
    let srv = support::erroring_daemon(500, "brand_new_code", "x");
    let c = HttpDaemonClient::new(srv.base(), "t".into()).expect("a loopback base");
    assert!(
        matches!(c.seal("e1"), Err(ControlError::Internal(_))),
        "an unrecognised code becomes Internal, never a silent success"
    );
}

/// `busy` and `conflict` share `409` and mean the opposite things. The retry that tells them
/// apart is `retry::retry_busy`, one layer above the transport, and this is it over the
/// wire.
#[test]
fn a_busy_409_is_retried_by_the_helper_and_a_conflict_409_is_not() {
    // Answers 409/`busy` once, then 200.
    let srv = support::recording_daemon_busy_once();
    let c = HttpDaemonClient::new(srv.base(), "t".into()).expect("a loopback base");
    retry_busy(|| c.abandon("e1", "stopping early"))
        .expect("a busy daemon is a daemon to ask again");
    assert_eq!(
        srv.attempts(),
        2,
        "bounded: one retry was enough, and it was taken"
    );

    let srv = support::erroring_daemon(409, "conflict", "already sealed");
    let c = HttpDaemonClient::new(srv.base(), "t".into()).expect("a loopback base");
    assert!(matches!(
        retry_busy(|| c.seal("e1")),
        Err(ControlError::Conflict(_))
    ));
    assert_eq!(srv.attempts(), 1, "a conflict is reported, never retried");
}

/// The other end of the same bound: a daemon that answers `busy` and goes on answering it
/// is reported as busy rather than asked forever.
#[test]
fn a_daemon_that_stays_busy_is_reported_after_a_bounded_number_of_attempts() {
    let srv = support::erroring_daemon(409, "busy", "an upload is holding this entry");
    let c = HttpDaemonClient::new(srv.base(), "t".into()).expect("a loopback base");
    assert!(matches!(
        retry_busy(|| c.abandon("e1", "stopping early")),
        Err(ControlError::Busy(_))
    ));
    assert_eq!(
        srv.attempts(),
        BUSY_ATTEMPTS,
        "bounded, and the bound is the one the helper names"
    );
}

/// The transport sends one request and reports what came back. The retry
/// is above the trait, which is the only place a command driven by the MOCK can meet it.
#[test]
fn the_transport_itself_never_retries_anything() {
    let srv = support::erroring_daemon(409, "busy", "an upload is holding this entry");
    let c = HttpDaemonClient::new(srv.base(), "t".into()).expect("a loopback base");
    assert!(matches!(
        c.abandon("e1", "stopping early"),
        Err(ControlError::Busy(_))
    ));
    assert_eq!(
        srv.attempts(),
        1,
        "one call on the client, one request on the wire"
    );
}

/// A command is written against the trait and tested against the mock, so the retry has to
/// work there — `busy_once` then success, one call to the helper, and the fake counting both
/// attempts. That a command retries a busy once is testable because of this test's shape, not
/// the wire one's.
#[test]
fn the_helper_retries_a_busy_through_the_mock_the_commands_are_driven_by() {
    let tmp = tempfile::tempdir().unwrap();
    let m = MockDaemonClient::armed_ok(tmp.path(), "claude-code-cli");
    let info = m.info();

    m.busy_once("abandon");
    retry_busy(|| m.abandon(&info.entry_id, "stopping early"))
        .expect("one busy, asked again, and answered");

    assert_eq!(
        m.calls().iter().filter(|call| *call == "abandon").count(),
        2,
        "the mock's own record of the attempts: the same retry the wire client gets, \
         because both are reached through the trait"
    );

    // And the other `409` is still never retried, on this side of the seam too.
    m.seal(&info.entry_id).expect("the first seal");
    assert!(matches!(
        retry_busy(|| m.seal(&info.entry_id)),
        Err(ControlError::Conflict(_))
    ));
    assert_eq!(
        m.calls().iter().filter(|call| *call == "seal").count(),
        2,
        "one seal, one conflict, and no third ask"
    );
}

/// Attempts alone are not a bound. Three attempts against a route with a
/// five-minute deadline is a quarter of an hour of held terminal, so the helper carries a
/// wall clock too — two ordinary calls, never the slow one.
#[test]
fn the_helper_stops_at_its_wall_clock_budget_and_not_only_at_its_attempt_count() {
    assert_eq!(BUSY_BUDGET, CALL_TIMEOUT * 2, "two ordinary calls");
    assert!(
        BUSY_BUDGET < SLOW_CALL_TIMEOUT,
        "the budget is written in the NORMAL call timeout: a single slow call outlasts the \
         whole of a retried one, which is why the slow route stays outside this helper"
    );
    assert!(BUSY_BUDGET < BUSY_ATTEMPTS as u32 * SLOW_CALL_TIMEOUT);

    // A budget already spent. Nothing sleeps and nothing is asked twice — asserted on a
    // closure rather than a socket so the timing is the helper's and nothing else's.
    let mut asked = 0usize;
    let began = Instant::now();
    let answered: Result<(), ControlError> = retry_busy_within(Duration::ZERO, || {
        asked += 1;
        Err(ControlError::Busy("an upload is holding this entry".into()))
    });
    match answered {
        Err(ControlError::Busy(said)) => assert!(
            said.contains("upload"),
            "the daemon's own words survive the budget"
        ),
        other => panic!("a spent budget surfaces the busy: {other:?}"),
    }
    assert_eq!(asked, 1, "the budget was gone before the second ask");
    assert!(
        began.elapsed() < BUSY_BACKOFF,
        "a backoff that would end past the budget is not taken at all"
    );

    // The same over the wire, counted by the server.
    let srv = support::erroring_daemon(409, "busy", "an upload is holding this entry");
    let c = HttpDaemonClient::new(srv.base(), "t".into()).expect("a loopback base");
    assert!(matches!(
        retry_busy_within(Duration::ZERO, || c.abandon("e1", "stopping early")),
        Err(ControlError::Busy(_))
    ));
    assert_eq!(srv.attempts(), 1);
}

/// The bearer goes to this machine or it does not go. `from_config_dir` reaches that by
/// building the base itself; a caller that hands one over meets it in `new`.
#[test]
fn a_base_that_is_not_loopback_is_refused_before_any_request_is_made() {
    let token = "a-control-token-that-must-never-be-quoted";
    for base in [
        "http://10.0.0.7:53310",
        "http://example.com:53310",
        // A name that resolves back today is still a name, and this process is not what
        // resolves it.
        "http://localhost:53310",
        // A host whose PREFIX is a loopback literal, which a string comparison would have
        // taken for one.
        "http://127.0.0.1.example.com:53310",
        "https://127.0.0.1:53310",
    ] {
        match HttpDaemonClient::new(base.to_string(), token.into()) {
            Err(CliError::Refused(said)) => {
                assert!(said.contains(base), "the refusal names the base: {said}");
                assert!(!said.contains(token), "and never the token: {said}");
            }
            Err(other) => panic!("{base} is a refusal the participant can fix: {other:?}"),
            Ok(_) => panic!("{base} is not this machine"),
        }
    }
    for base in [
        "http://127.0.0.1:53310",
        // The whole of 127.0.0.0/8 is loopback, and `IpAddr::is_loopback` is the rule.
        "http://127.0.0.5:53310",
        "http://[::1]:53310",
    ] {
        assert!(
            HttpDaemonClient::new(base.to_string(), "t".into()).is_ok(),
            "{base} is this machine"
        );
    }
}

/// A daemon that answered badly is not a daemon that is not there. `Unreachable` is the
/// sentence a participant acts on by restarting a daemon — which, here, was answering.
#[test]
fn a_body_that_stopped_early_or_is_not_utf8_is_malformed_and_not_unreachable() {
    let srv = support::truncated_daemon();
    let c = HttpDaemonClient::new(srv.base(), "t".into()).expect("a loopback base");
    match c.entries() {
        Err(ControlError::Malformed(_)) => {}
        other => panic!("a body that stopped early is malformed: {other:?}"),
    }

    let srv = support::non_utf8_daemon();
    let c = HttpDaemonClient::new(srv.base(), "t".into()).expect("a loopback base");
    match c.entries() {
        Err(ControlError::Malformed(_)) => {}
        other => panic!("a body that is not UTF-8 is malformed: {other:?}"),
    }
}

/// The control file the daemon writes is the control file this CLI reads. The daemon writes
/// it with its own `write_control_file`, and both halves of the CLI read it back: the plain
/// `read_control_file`, and `from_config_dir`, whose request must then carry the file's port
/// and token. Nothing here is a hand-written copy of the document, so a field the daemon adds,
/// renames or re-types is a failure here and not a fixture that keeps passing.
#[test]
fn the_control_file_the_daemon_writes_is_the_one_this_cli_reads() {
    let srv = support::recording_daemon();
    let cfg = support::empty_cfg();
    let port = srv
        .base()
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .expect("the recording daemon's port");
    let written = goalcup_daemon::control::write_control_file(&cfg, port, "the-token-in-the-file")
        .expect("the daemon writes its control file");
    assert_eq!(written, goalcup_cli::config::control_path(&cfg));

    let file = goalcup_cli::config::read_control_file(&cfg).expect("the CLI reads it back");
    assert_eq!(file.control_api_version, control::CONTROL_API_VERSION);
    assert_eq!(file.port, port);
    assert_eq!(file.address, format!("127.0.0.1:{port}"));
    assert_eq!(file.token, "the-token-in-the-file");

    let c = HttpDaemonClient::from_config_dir(&cfg).expect("the daemon's own control.json");
    let _ = c.entries();
    let (path, auth) = srv.last();
    assert_eq!(path, goalcup_protocol::control::ROUTE_ENTRIES);
    assert_eq!(
        auth.as_deref(),
        Some("Bearer the-token-in-the-file"),
        "the port and the token both came from the file"
    );
}

/// A V2 daemon cannot enforce the current control contract. Refuse its control file before any
/// cache or mutating-route work can pair the stale binaries.
#[test]
fn from_config_dir_refuses_a_v2_control_file_before_any_request() {
    let cfg = support::empty_cfg();
    std::fs::write(
        cfg.join("control.json"),
        r#"{"control_api_version":2,"daemon_version":"v2","address":"127.0.0.1:1","port":1,"receiver_address":"127.0.0.1:2","token":"old-v2-token"}"#,
    )
    .unwrap();

    let error = HttpDaemonClient::from_config_dir(&cfg)
        .expect_err("the current CLI must not pair with a V2 daemon");
    let current = format!("this goalcup speaks {}", control::CONTROL_API_VERSION);
    assert!(
        error.to_string().contains("speaks control API version 2")
            && error.to_string().contains(&current),
        "the refusal identifies the incompatible pair without contacting the route: {error}"
    );
}

/// The CLI and the daemon ship together in one release, so a pair from two releases is refused,
/// one control API version apart in either direction: by the control file before any request,
/// and on the wire by the version every answer carries.
#[test]
fn a_daemon_from_another_release_is_refused_with_a_clear_message() {
    let ours = control::CONTROL_API_VERSION;
    for theirs in [ours - 1, ours + 1] {
        let cfg = support::empty_cfg();
        std::fs::write(
            cfg.join("control.json"),
            format!(
                r#"{{"control_api_version":{theirs},"daemon_version":"other","address":"127.0.0.1:1","port":1,"token":"other-token"}}"#
            ),
        )
        .unwrap();
        let error = HttpDaemonClient::from_config_dir(&cfg)
            .expect_err("the current CLI must not pair with another release's daemon");
        let said = error.to_string();
        assert!(
            said.contains(&format!("speaks control API version {theirs}"))
                && said.contains(&format!("this goalcup speaks {ours}"))
                && said.contains("install one release of both"),
            "{theirs}: the refusal names both versions and the fix: {said}"
        );
        assert!(!said.contains("old daemon"), "{theirs}: {said}");
        assert!(!said.contains("other-token"), "{theirs}: {said}");

        // A daemon that answers with another version's header is not read as this one.
        let srv = support::raw_response_daemon(
            200,
            vec![("x-goalcup-control-version".to_string(), theirs.to_string())],
            format!(
                r#"{{"control_api_version":{theirs},"daemon_version":"other","uptime_seconds":1,"entries_open":0}}"#
            )
            .into_bytes(),
        );
        let client = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();
        let failure = client
            .health()
            .expect_err("an answer from another control version is refused");
        assert!(
            matches!(failure, ControlError::Malformed(_)),
            "{theirs}: {failure:?}"
        );
        assert!(failure.to_string().contains(&ours.to_string()), "{failure}");
    }
}

/// The bearer is the one secret this client holds, and every refusal it raises is printed
/// by `main.rs` and read by `goalcup doctor`.
#[test]
fn no_refusal_this_client_raises_ever_carries_the_control_token() {
    let token = "a-control-token-that-must-never-be-quoted";

    let mut said = Vec::new();
    let unreachable =
        HttpDaemonClient::new("http://127.0.0.1:1".into(), token.into()).expect("a loopback base");
    said.push(format!("{:?}", unreachable.health()));
    said.push(format!("{:?}", unreachable));

    let srv = support::erroring_daemon(401, "unauthorized", "this route needs the control token");
    let refused = HttpDaemonClient::new(srv.base(), token.into()).expect("a loopback base");
    said.push(format!("{:?}", refused.entries()));
    if let Err(e) = refused.entries() {
        said.push(e.to_string());
    }

    // A `200` this client cannot read as the type the route promises.
    let srv = support::recording_daemon();
    let unreadable = HttpDaemonClient::new(srv.base(), token.into()).expect("a loopback base");
    said.push(format!("{:?}", unreadable.entries()));

    // A `200` larger than the client's cap.
    let srv = support::oversized_daemon(goalcup_cli::daemon::MAX_RESPONSE_BYTES as usize + 64);
    let oversized = HttpDaemonClient::new(srv.base(), token.into()).expect("a loopback base");
    match oversized.health() {
        Err(e) => {
            said.push(e.to_string());
            said.push(format!("{e:?}"));
        }
        Ok(_) => panic!("a body over the cap is refused, not read"),
    }

    for text in said {
        assert!(
            !text.contains(token),
            "the control token reached a message a participant can read: {text}"
        );
    }
}

/// `n` well-formed rows, as the daemon's `GET /v0/entries` answers them: every row the
/// registry ever held, open and sealed.
fn entries_body(n: usize) -> Vec<u8> {
    let rows: Vec<EntrySummary> = (0..n)
        .map(|i| EntrySummary {
            entry_id: format!("entry-{i:08}"),
            connector_id: "codex-cli".to_string(),
            workspace: "/Users/participant/goalcup/workspace".to_string(),
            arming: ArmingState::Started,
            health: HealthState::Online,
            started_at: Some("2026-10-13T00:00:00Z".to_string()),
            deadline_at: Some("2026-10-20T00:00:00Z".to_string()),
            sealed_at: Some("2026-10-13T00:02:00Z".to_string()),
        })
        .collect();
    serde_json::to_vec(&rows).expect("rows serialize")
}

/// A participant whose launcher relaunches the harness every couple of minutes passes two
/// hundred rows within hours, and the list is every row ever held. It must not trip the
/// ordinary cap, which is for answers that do not grow.
#[test]
fn an_entries_list_past_the_ordinary_cap_is_read_and_parsed() {
    let body = entries_body(400);
    assert!(
        body.len() as u64 > goalcup_cli::daemon::MAX_RESPONSE_BYTES,
        "the fixture must be past the ordinary cap to mean anything: {} bytes",
        body.len()
    );
    assert!((body.len() as u64) < goalcup_cli::daemon::MAX_LIST_RESPONSE_BYTES);
    let srv = support::raw_response_daemon(200, version_header(), body);
    let client = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();

    let rows = client.entries().expect("a list under the list cap is read");

    assert_eq!(rows.len(), 400);
    assert_eq!(rows[399].entry_id, "entry-00000399");
}

/// The list cap is larger, not absent: a daemon answering with more than it cannot make
/// this client allocate without limit, and the refusal names the cap that was hit.
#[test]
fn an_entries_list_past_the_list_cap_is_refused_naming_it() {
    let cap = goalcup_cli::daemon::MAX_LIST_RESPONSE_BYTES;
    let srv = support::oversized_daemon(cap as usize + 64);
    let client = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();

    match client.entries() {
        Err(ControlError::Malformed(message)) => assert!(
            message.contains(&format!("more than {cap} bytes")),
            "the refusal names the cap: {message}"
        ),
        other => panic!("a list over the list cap is refused, not read: {other:?}"),
    }
}

/// Only the list gets the larger cap: every other route still refuses at the ordinary one.
#[test]
fn a_non_list_answer_past_the_ordinary_cap_is_still_refused() {
    let cap = goalcup_cli::daemon::MAX_RESPONSE_BYTES;
    let srv = support::oversized_daemon(cap as usize + 64);
    let client = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();

    match client.health() {
        Err(ControlError::Malformed(message)) => assert!(
            message.contains(&format!("more than {cap} bytes")),
            "the refusal names the cap: {message}"
        ),
        other => panic!("a body over the ordinary cap is refused, not read: {other:?}"),
    }
}

/// The mock's hooks, exercised once here so a method a command test reaches for is a method
/// that already exists and already behaves.
#[test]
fn the_mocks_remaining_hooks_do_what_the_tasks_that_call_them_expect() {
    let tmp = tempfile::tempdir().unwrap();
    let m = MockDaemonClient::armed_ok(tmp.path(), "claude-code-cli");
    let info = m.info();

    // One `busy`, then the call goes through, and the mock counts both.
    m.busy_once("health");
    assert!(matches!(m.health(), Err(ControlError::Busy(_))));
    assert!(m.health().is_ok(), "one busy, and then success");

    // An arbitrary refusal, once.
    m.fail_next("seal", ControlError::NotFound("no such entry".into()));
    assert!(matches!(
        m.seal(&info.entry_id),
        Err(ControlError::NotFound(_))
    ));

    // The default status is a capture's, as every document the daemon signs is, and it
    // verifies against the run key the entry record pins.
    let signed = m.status(&info.entry_id).unwrap();
    assert!(
        signed.payload.capture.is_some(),
        "the default is a capture status"
    );
    assert_eq!(signed.payload.detail, m.status_detail());
    assert!(signed.verify_with(&info.run_public_key).is_ok());

    // A hook that edits the payload re-signs it.
    let mut capture = signed.payload.capture.clone().unwrap();
    capture.gap_count = 3;
    m.set_capture_status(capture, "recording with losses", 2, 1);
    let signed = m.status(&info.entry_id).unwrap();
    assert_eq!(signed.payload.capture.as_ref().unwrap().gap_count, 3);
    assert_eq!(signed.payload.detail, "recording with losses");
    assert!(
        signed.verify_with(&info.run_public_key).is_ok(),
        "a hook that edits the payload re-signs it"
    );

    // The one way to reach the "did not verify" path.
    m.tamper_status_payload();
    let signed = m.status(&info.entry_id).unwrap();
    assert!(
        signed.verify_with(&info.run_public_key).is_err(),
        "the payload changed and the signature did not"
    );

    // The relaunch is handed over, and the entry cache can follow it.
    let handle = goalcup_protocol::process::ProcessHandle {
        pid: 4242,
        pgid: Some(4242),
        native_pid: None,
        native_binary: None,
        native_process_start: None,
        process_start: None,
        strategy: goalcup_protocol::manifest::KillStrategy::ProcessGroup,
        grace_seconds: 5,
        launched_at: support::NOW.to_string(),
        program: "/usr/bin/true".to_string(),
        argv: vec!["true".to_string()],
        env_keys: vec!["GOALCUP_RECEIVER_TOKEN".to_string()],
    };
    assert_eq!(m.last_registered_pid(), None);
    m.register_process(
        &info.entry_id,
        &goalcup_protocol::process::ProcessRegistration {
            generation: info.process_generation,
            handle: handle.clone(),
        },
    )
    .unwrap();
    assert_eq!(m.last_registered_pid(), Some(4242));

    // What the CLI abandoned, and why.
    m.abandon(&info.entry_id, "the arming window closed")
        .unwrap();
    assert_eq!(
        m.abandoned(),
        vec![(
            info.entry_id.clone(),
            "the arming window closed".to_string()
        )]
    );

    assert_eq!(m.entry_count(), 1);

    // The seal, once, and the mirror that disagrees with the workspace.
    m.make_sealed_manifest_differ_on("README.md");
    let control::SealResponse::Legacy(sealed) = m.seal(&info.entry_id).unwrap() else {
        panic!("the legacy mock seal must return the historical manifest arm");
    };
    assert!(sealed.files.iter().any(|f| f.path == "README.md"));
    assert!(
        matches!(m.seal(&info.entry_id), Err(ControlError::Conflict(_))),
        "a second seal is a conflict, and a conflict is never retried"
    );
    m.seal_once_then_conflict();
    assert!(m.seal(&info.entry_id).is_ok(), "the seal was re-armed once");

    // The rest of the trait, so no command test is the first to call one of these.
    assert_eq!(
        m.health().unwrap().control_api_version,
        control::CONTROL_API_VERSION
    );
    // Zero, and it was one before the seal above: `GET /v0/entries` is the daemon's LIVE map and
    // a sealed entry is evicted from it, while the signed status route keeps answering for
    // that entry out of the frozen cache — so the mock's `seal` evicts too. A mock that still
    // listed a sealed run could not show the one disagreement `goalcup entries` exists to
    // explain.
    assert_eq!(m.entries().unwrap().len(), 0);
    assert_eq!(MockDaemonClient::signing_key().to_bytes(), [9u8; 32]);
}

// ---------------------------------------------------------------- the capture seal

fn capture_request(remaining_ms: u64) -> control::CaptureSealRequest {
    control::CaptureSealRequest {
        proxy: None,
        remaining_finish_ms: Some(remaining_ms),
        native_scope: None,
    }
}

#[test]
fn capture_seal_classifies_only_busy_unauthorized_and_refused_connection_as_not_admitted() {
    use control::CaptureSealError;
    let timeout = Duration::from_secs(5);

    for (status, code, message) in [(409, "busy", "held"), (401, "unauthorized", "no")] {
        let srv = support::erroring_daemon(status, code, message);
        let c = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();
        let failure = c
            .seal_capture("e1", &capture_request(5_000), timeout)
            .expect_err("a refusal");
        assert!(
            failure.not_admitted(),
            "{code} comes before the handler: {failure:?}"
        );
        assert_eq!(failure.into_error().code(), code);
    }

    // A connection the daemon never accepted: nothing was written.
    let c = HttpDaemonClient::new("http://127.0.0.1:1".into(), "t".into()).unwrap();
    let failure = c
        .seal_capture("e1", &capture_request(5_000), timeout)
        .expect_err("nothing listens");
    assert!(
        matches!(
            failure,
            CaptureSealError::NotAdmitted(ControlError::Unreachable(_))
        ),
        "{failure:?}"
    );
}

// ---------------------------------------------------------------- H-14: a finish after a restart

/// A control.json for a daemon at `base`, as a restarted daemon writes it: a new port and a new
/// token over the same configuration directory.
fn write_control_file(cfg: &std::path::Path, base: &str, token: &str) {
    let port: u16 = base.rsplit(':').next().unwrap().parse().unwrap();
    let document = serde_json::json!({
        "control_api_version": control::CONTROL_API_VERSION,
        "daemon_version": "0.1.0",
        "address": format!("127.0.0.1:{port}"),
        "port": port,
        "receiver_address": format!("127.0.0.1:{}", port + 1),
        "token": token,
    });
    std::fs::write(
        cfg.join("control.json"),
        serde_json::to_string(&document).unwrap(),
    )
    .unwrap();
}

/// What a restarted daemon answers a capture seal with.
fn finished_daemon() -> support::RecordingDaemon {
    let report = control::CaptureFinishReport {
        entry_id: "e1".to_string(),
        trace_dir: "/x/trace".to_string(),
        termination: goalcup_protocol::provenance::TerminationOutcomeV1::AlreadyStopped {
            checked_at: goalcup_protocol::provenance::Rfc3339Utc::try_from("2026-10-02T00:00:00Z")
                .unwrap(),
        },
        terminal_seq: Some(3),
        seal: control::LocalSealOutcome::SealWritten,
    };
    support::raw_response_daemon(
        200,
        vec![(
            "x-goalcup-control-version".to_string(),
            control::CONTROL_API_VERSION.to_string(),
        )],
        serde_json::to_vec(&report).unwrap(),
    )
}

/// H-14: the launcher's control client is built once, before the run, and the daemon's token is
/// minted per start. A daemon restarted between the launch and the finish is at another port or
/// behind another token; the finish re-reads control.json once and asks again, but only when the
/// first attempt was refused before the finish began (`unauthorized`, a connection never
/// accepted), so it can never replay a finish that may have been admitted.
#[test]
fn a_finish_after_a_daemon_restart_rereads_control_json_once() {
    let timeout = Duration::from_secs(5);
    let request = capture_request(5_000);
    for (what, stale) in [
        (
            "a token re-minted",
            support::erroring_daemon(401, "unauthorized", "no").base(),
        ),
        ("a port nobody listens on", "http://127.0.0.1:1".to_string()),
    ] {
        let cfg = support::empty_cfg();
        let restarted = finished_daemon();
        write_control_file(&cfg, &restarted.base(), "the-new-token");
        let first = HttpDaemonClient::new(stale, "the-old-token".into()).unwrap();
        let mut fresh = None;
        let report = goalcup_cli::daemon::seal_capture_following_restart(
            &first,
            &mut fresh,
            Some(&cfg),
            "e1",
            &request,
            timeout,
        )
        .unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert_eq!(report.entry_id, "e1", "{what}");
        assert_eq!(restarted.attempts(), 1, "{what}");
        let (path, auth) = restarted.last();
        assert_eq!(path, entry_route("e1", SUFFIX_SEAL), "{what}");
        assert_eq!(auth.as_deref(), Some("Bearer the-new-token"), "{what}");
        assert!(fresh.is_some(), "{what}: the rebuilt client is kept");

        // The next attempt goes straight to the rebuilt client.
        let again = goalcup_cli::daemon::seal_capture_following_restart(
            &first,
            &mut fresh,
            Some(&cfg),
            "e1",
            &request,
            timeout,
        );
        assert!(again.is_ok(), "{what}: {again:?}");
        assert_eq!(restarted.attempts(), 2, "{what}");
    }
}

/// Only a refusal before the finish began is asked again; everything else is the answer.
#[test]
fn a_finish_is_not_replayed_on_a_restarted_daemon_unless_it_was_never_admitted() {
    use control::CaptureSealError;
    let timeout = Duration::from_secs(5);
    let request = capture_request(5_000);
    let cfg = support::empty_cfg();
    let restarted = finished_daemon();
    write_control_file(&cfg, &restarted.base(), "the-new-token");

    // A busy entry, and every answer from inside the finish, are returned as they came.
    for (status, code) in [
        (409, "busy"),
        (500, "internal"),
        (404, "not_found"),
        (409, "finish_deadline"),
    ] {
        let stale = support::erroring_daemon(status, code, "x");
        let first = HttpDaemonClient::new(stale.base(), "t".into()).unwrap();
        let mut fresh = None;
        let failure = goalcup_cli::daemon::seal_capture_following_restart(
            &first,
            &mut fresh,
            Some(&cfg),
            "e1",
            &request,
            timeout,
        )
        .expect_err(code);
        assert_eq!(
            failure.not_admitted(),
            code == "busy",
            "{code}: {failure:?}"
        );
        assert_eq!(failure.into_error().code(), code);
        assert!(fresh.is_none(), "{code}");
    }
    assert_eq!(restarted.attempts(), 0, "no finish was replayed");

    // No control.json to re-read, or no reconnection asked for: the first failure stands.
    let dead = HttpDaemonClient::new("http://127.0.0.1:1".into(), "t".into()).unwrap();
    for cfg in [Some(support::empty_cfg()), None] {
        let mut fresh = None;
        let failure = goalcup_cli::daemon::seal_capture_following_restart(
            &dead,
            &mut fresh,
            cfg.as_deref(),
            "e1",
            &request,
            timeout,
        )
        .expect_err("nobody to ask");
        assert!(
            matches!(
                failure,
                CaptureSealError::NotAdmitted(ControlError::Unreachable(_))
            ),
            "{failure:?}"
        );
        assert!(fresh.is_none());
    }
}

#[test]
fn handler_400_and_404_and_read_timeout_as_maybe_admitted() {
    use control::CaptureSealError;
    let timeout = Duration::from_secs(5);

    // The handler's own 400, a 404 that can follow the owner's stop, a conflict, and an
    // internal error are all answers from inside the finish, or may be.
    for (status, code) in [
        (400, "malformed"),
        (404, "not_found"),
        (409, "conflict"),
        (500, "internal"),
        (409, "finish_deadline"),
    ] {
        let srv = support::erroring_daemon(status, code, "x");
        let c = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();
        let failure = c
            .seal_capture("e1", &capture_request(5_000), timeout)
            .expect_err("a refusal");
        assert!(
            matches!(failure, CaptureSealError::MaybeAdmitted(_)),
            "{code}: {failure:?}"
        );
        assert_eq!(failure.into_error().code(), code);
    }

    // A daemon that accepted and never answered: the read timeout is ambiguous.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let held = std::thread::spawn(move || {
        let held = listener.accept();
        std::thread::sleep(Duration::from_millis(1_500));
        drop(held);
    });
    let c = HttpDaemonClient::new(base, "t".into()).unwrap();
    let began = Instant::now();
    let failure = c
        .seal_capture("e1", &capture_request(5_000), Duration::from_millis(300))
        .expect_err("no answer");
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "{:?}",
        began.elapsed()
    );
    assert!(
        matches!(
            failure,
            CaptureSealError::MaybeAdmitted(ControlError::Unreachable(_))
        ),
        "{failure:?}"
    );
    held.join().unwrap();
}

#[test]
fn commit_stage_expiry_is_finish_deadline_not_busy_and_not_replayed() {
    use control::CaptureSealError;
    let srv = support::erroring_daemon(409, "finish_deadline", "local_finish");
    let c = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();
    let failure = goalcup_cli::retry::retry_capture_seal(
        Instant::now() + Duration::from_secs(10),
        |remaining| {
            c.seal_capture(
                "e1",
                &capture_request(remaining.as_millis() as u64),
                Duration::from_secs(5),
            )
        },
    )
    .expect_err("a refusal");
    assert_eq!(srv.attempts(), 1, "an expired finish is never replayed");
    assert!(
        matches!(
            failure,
            CaptureSealError::MaybeAdmitted(ControlError::FinishDeadline(ref stage))
                if stage == "local_finish"
        ),
        "{failure:?}"
    );
}

#[test]
fn the_capture_seal_carries_its_remaining_budget_and_its_own_read_timeout() {
    let srv = support::recording_daemon();
    let c = HttpDaemonClient::new(srv.base(), "t".into()).unwrap();
    // `{}` is not a finish report: the call fails to parse, which is fine; what went OUT is
    // the assertion.
    let _ = c.seal_capture("e1", &capture_request(8_123), Duration::from_secs(5));
    let body = srv.last_body().expect("a body");
    assert!(body.contains("\"remaining_finish_ms\":8123"), "{body}");
    assert_eq!(
        srv.last().0,
        entry_route("e1", SUFFIX_SEAL),
        "the same seal route as the legacy call"
    );

    // remaining + termination allowance + the connect slack, all from existing constants.
    assert_eq!(
        goalcup_cli::daemon::capture_seal_read_timeout(
            Duration::from_secs(8),
            goalcup_protocol::process::termination_bound(0)
        ),
        Duration::from_millis(8_000 + 4_250 + 2_000)
    );
    assert_eq!(
        goalcup_cli::daemon::capture_seal_read_timeout(
            Duration::from_secs(10),
            goalcup_protocol::process::termination_bound(10)
        ),
        Duration::from_millis(10_000 + 24_250 + 2_000)
    );
}

/// The pause points (CONTRACTS.md, C6 local finish budget) exist only under `test-support`: a shipped build has the no-op
/// below the gated definition, and nothing in it reads the environment variable.
#[test]
fn pause_points_exist_only_under_test_support() {
    let source = include_str!("../../src/commands/start/mod.rs");
    let gated = "#[cfg(feature = \"test-support\")]\nfn pause_at(point: &'static str) {";
    let shipped = "#[cfg(not(feature = \"test-support\"))]\n#[inline(always)]\nfn pause_at(_point: &'static str) {}";
    let (before, rest) = source
        .split_once(gated)
        .expect("the pause point's test-support definition");
    let (body, after) = rest
        .split_once(shipped)
        .expect("the shipped build's empty pause point");
    // Mentioned in the definition's own doc comment (`///`) and read once in its body; nowhere
    // else.
    for line in before
        .lines()
        .filter(|l| l.contains("GOALCUP_TEST_PAUSE_DIR"))
    {
        assert!(line.trim_start().starts_with("///"), "{line}");
    }
    assert_eq!(body.matches("GOALCUP_TEST_PAUSE_DIR").count(), 1);
    assert!(
        !after.contains("GOALCUP_TEST_PAUSE_DIR"),
        "no other code reads the pause directory"
    );
    // The one use is a call of the one function, unconditional.
    assert!(
        include_str!("../../src/commands/start/finish.rs")
            .contains("pause_at(\"capture-finish-started\")")
    );
}

/// The only places this crate starts a process are the launch module's one command builder
/// (which the ordinary spawn, the gate and the connector's `capture.configure` commands all
/// run), the run-scoped daemon's start
/// (`daemon/spawn.rs`), the D53 version probe (`harness_version.rs`), the `git` that clones
/// the participant's GitHub repository (`account/github.rs`) and macOS's `/usr/bin/security`,
/// which exports the system's trusted roots for a shared CA row's file (`ca_bundle.rs`, with a
/// cleared environment and no input); the V1 version probe
/// went with the V1 manifest (OS-29). The child-only credential is set on the one launch
/// builder, so another site would be another place a key could be forgotten or leaked. The
/// daemon's start and the version probe carry no key: both clear their environment, which
/// `run_scoped_daemon::the_started_daemon_inherits_none_of_the_clis_secrets_or_descriptors`
/// observes for the daemon and `harness_version_probe` for the probe. The GitHub token is
/// never given to a child: `git` is run with the repository's public URL and no token in its
/// argv or environment (`account::the_token_never_appears_in_a_git_argv`), and gets the token
/// only by running `goalcup git-credential` itself, later, from the agent's own `git push`.
#[test]
fn the_only_command_site_is_the_launch_builder() {
    fn sites(dir: &std::path::Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sites(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                // A file's test module may spawn helpers; it is test code.
                let shipped = text.split("#[cfg(test)]").next().unwrap();
                for (n, line) in shipped.lines().enumerate() {
                    if line.contains("Command::new(") && !line.trim_start().starts_with("//") {
                        found.push(format!(
                            "{}:{}",
                            path.file_name().unwrap().to_string_lossy(),
                            n + 1
                        ));
                    }
                }
            }
        }
    }
    let mut found = Vec::new();
    sites(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    found.sort();
    let files: Vec<&str> = found.iter().map(|f| f.split(':').next().unwrap()).collect();
    assert_eq!(
        files,
        [
            "ca_bundle.rs",
            "github.rs",
            "harness_version.rs",
            "launch.rs",
            "spawn.rs"
        ],
        "{found:?}"
    );
    let bundle = include_str!("../../src/ca_bundle.rs");
    assert!(
        bundle.contains("Command::new(\"/usr/bin/security\")") && bundle.contains(".env_clear()"),
        "the trust-store export runs one absolute program from nothing"
    );
    let spawn = include_str!("../../src/daemon/spawn.rs");
    assert!(
        spawn.contains(".env_clear()"),
        "the daemon starts from nothing"
    );
    let probe = include_str!("../../src/harness_version.rs");
    assert!(
        probe.contains(".env_clear()") && !probe.contains("secret"),
        "the version probe starts from nothing and carries no key"
    );
}

/// D40 (CONTRACTS.md, C6 capture spawn gate): every capture child is spawned only through the pre-exec gate, whatever
/// pipes it carries, and is released only after its registration. A scan
/// of the one function that launches a capture run and of the launch module's spawners.
#[test]
fn capture_children_spawn_only_through_the_gate() {
    let start = concat!(
        include_str!("../../src/commands/start/mod.rs"),
        include_str!("../../src/commands/start/phases.rs"),
        include_str!("../../src/commands/start/finish.rs"),
    );
    // The capture launch is its phase functions, which run in the order they are written:
    // `capture` through `finish_run`, up to the words for a harness's end.
    let (_, rest) = start
        .split_once("\nfn capture(")
        .expect("the capture launch");
    let (body, _) = rest
        .split_once("\n/// A harness's end in words")
        .expect("the end of the capture phases");
    let flat_body = body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("( ", "(")
        .replace(", )", ")");
    let body = body.to_string();
    for banned in [
        "spawn(",
        "spawn_stdio(",
        "spawn_scoped_capture",
        "spawn_marked_capture",
        "kill_registered(",
        "kill(",
        "Command::new",
    ] {
        assert!(
            !body.contains(banned),
            "`capture` reaches the harness without the gate: {banned}"
        );
    }
    let at = |needle: &str| {
        body.find(needle)
            .unwrap_or_else(|| panic!("`capture` has no {needle}"))
    };
    // The order is the gate's: forked and parked, handle, durable registration, only then
    // release.
    assert!(at("spawn_gated(") < at("gated_handle("));
    assert!(at("gated_handle(") < at("register_process("));
    assert!(at("register_process(") < at("gated.release()"));
    // A failed registration aborts the gate; it never reaches the release.
    let (failed, _) = body
        .split_once("gated.release()")
        .expect("the release is the next step");
    assert!(
        failed.matches("gated.abort();").count() >= 3,
        "a failed handle, registration, or an interrupt aborts the gate"
    );
    // The gate is handed the plan, and nothing else: no secret rides on it (D61).
    assert!(flat_body.contains("spawn_gated(&plan, GATE_READY_WITHIN)"));

    // And the spawners the launch module still has are the ungated ordinary one and the gate.
    let launch = include_str!("../../src/launch.rs");
    for gone in [
        "fn spawn_scoped_capture",
        "fn spawn_marked_capture",
        "fn spawn_stdio",
    ] {
        assert!(!launch.contains(gone), "{gone} is gone");
    }
    assert_eq!(
        launch.matches("command.spawn()").count(),
        2,
        "the ordinary spawn and the gate's one spawning thread"
    );
}
