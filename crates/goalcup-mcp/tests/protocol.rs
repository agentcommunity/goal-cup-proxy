//! The protocol, as a pure function: a request line in, a response line out, over a
//! `<config_dir>` the fixtures wrote.

use goalcup_mcp::{SUPPORTED_PROTOCOL_VERSIONS, handle_request, tools_manifest};
use goalcup_protocol::control::STALE_AFTER_SECONDS;

mod support; // writes a cache signed with the fixed [9u8; 32] seed, and pins its key

#[test]
fn initialize_lists_tools_and_a_notification_gets_no_response() {
    let s = support::source_with_one_entry();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
            &s,
            support::NOW,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(v["result"]["serverInfo"]["name"], "goalcup");
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
    assert!(v["result"]["capabilities"]["tools"].is_object());
    // One version, and `2025-03-26` is NOT it. That revision requires a server
    // to accept a JSON-RPC batch and this server refuses every one, so advertising it would be a
    // promise the next test shows this process does not keep. A client that asks for it is
    // answered with the version whose rules this server does follow.
    assert_eq!(SUPPORTED_PROTOCOL_VERSIONS, ["2025-06-18"]);
    for asked in ["1999-01-01", "2025-03-26"] {
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "initialize",
            "params": {"protocolVersion": asked, "capabilities": {},
                       "clientInfo": {"name": "t", "version": "0"}},
        })
        .to_string();
        let v: serde_json::Value =
            serde_json::from_str(&handle_request(&line, &s, support::NOW).unwrap()).unwrap();
        // An unsupported version is answered with the newest supported one, not an error.
        assert_eq!(
            v["result"]["protocolVersion"],
            *SUPPORTED_PROTOCOL_VERSIONS.last().unwrap(),
            "a version this server does not implement is never echoed back"
        );
    }
    assert!(
        handle_request(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &s,
            support::NOW
        )
        .is_none()
    );

    let v: serde_json::Value = serde_json::from_str(
        &handle_request(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
            &s,
            support::NOW,
        )
        .unwrap(),
    )
    .unwrap();
    let tools = v["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "goalcup_event_status",
            "goalcup_run_status",
            "goalcup_entries"
        ]
    );
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object");
    }
}

#[test]
fn ping_is_answered_with_an_empty_result() {
    let s = support::source_with_one_entry();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(
            r#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#,
            &s,
            support::NOW,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(v["id"], 4);
    assert_eq!(v["result"], serde_json::json!({}));
}

#[test]
fn run_status_answers_from_the_signed_cache_and_goes_offline_at_the_shared_threshold() {
    let s = support::source_with_one_entry();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    let body = support::tool_body(&v);
    for k in [
        "seconds_remaining",
        "arming",
        "health",
        "last_sync_at",
        "offline",
    ] {
        assert!(body.get(k).is_some(), "missing {k}");
    }
    assert_eq!(body["offline"], false);

    let later = support::plus_seconds(STALE_AFTER_SECONDS + 480);
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, &later).unwrap(),
    )
    .unwrap();
    let body = support::tool_body(&v);
    assert_eq!(
        body["offline"], true,
        "PRD 5.3: it reports the last sync when offline"
    );
    assert_eq!(body["stale_seconds"], STALE_AFTER_SECONDS + 480);
    // The threshold is the shared constant in all three readers; no literal here.
    assert!(!include_str!("../src/lib.rs").contains("120"));
}

#[test]
fn legacy_surfaces_do_not_claim_m2_health() {
    use ed25519_dalek::SigningKey;
    use goalcup_protocol::capture::{
        AnalysisState, AnchorState, CaptureState, CaptureStorage, DeliveryState, M2Status, RunState,
    };
    use goalcup_protocol::control::SignedStatus;

    let source = support::source_with_one_entry();
    let path = source.config_dir().join("cache/status-entry-0001.json");
    let mut signed: SignedStatus = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    signed.payload.capture = Some(M2Status {
        run: RunState::Running,
        capture: CaptureState::Recording,
        delivery: DeliveryState::LocalOnly,
        analysis: AnalysisState::NotRun,
        anchor: AnchorState::LocalOnly,
        storage: CaptureStorage {
            quota_bytes: 1024,
            reserved_bytes: 1024,
            used_bytes: 0,
            durable: None,
            accounting_available: true,
        },
        gap_count: 0,
        assistance_observed: false,
    });
    let signed = SignedStatus::sign(
        signed.payload,
        signed.cached_at,
        &SigningKey::from_bytes(&support::SEED),
    )
    .unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    for name in ["status-entry-0001.json", "status-latest.json"] {
        std::fs::write(source.config_dir().join("cache").join(name), &bytes).unwrap();
    }

    for request in [
        support::call("goalcup_run_status"),
        support::call("goalcup_entries"),
    ] {
        let response: serde_json::Value =
            serde_json::from_str(&handle_request(&request, &source, support::NOW).unwrap())
                .unwrap();
        let body = support::tool_body(&response);
        let row = if request.contains("goalcup_entries") {
            &body["entries"][0]
        } else {
            &body
        };
        assert_eq!(row["m2_health"], "unavailable");
        assert!(
            row["detail"]
                .as_str()
                .unwrap()
                .contains("M2 capture details are unavailable")
        );
        assert!(row["detail"].as_str().unwrap().contains("goalcup status"));
        for legacy_field in ["health", "arming", "seconds_remaining", "events_uploaded"] {
            assert!(
                row.get(legacy_field).is_none(),
                "M2 row exposed {legacy_field}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn a_cache_symlink_is_refused_instead_of_becoming_status_data() {
    use std::os::unix::fs::symlink;

    let s = support::source_with_one_entry();
    let cache = s.config_dir().join("cache");
    let latest = cache.join("status-latest.json");
    std::fs::remove_file(&latest).unwrap();
    symlink(cache.join("status-entry-0001.json"), &latest).unwrap();

    let response: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(response["result"]["isError"], true, "{response}");
}

#[test]
fn a_tampered_or_wrongly_keyed_status_is_an_error_and_never_data() {
    let s = support::source_with_tampered_payload();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"].to_string().contains("did not verify"));
    // Nothing out of the payload comes back with the refusal — not the deadline the
    // tamperer wrote into it.
    assert!(!v["result"].to_string().contains("999999"));

    // A document signed by a key the server did not pin is refused, even though the
    // document names that key itself.
    let s = support::source_signed_by_a_stranger();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(v["result"]["isError"], true);
    assert!(
        !include_str!("../src/lib.rs").contains(".public_key"),
        "the key comes from WorkspaceInfo, never from inside the document"
    );
}

#[test]
fn a_status_with_no_pinned_key_at_all_is_unverified_and_not_data() {
    // The cache holds a perfectly good document and `entries/` holds no record of it, so
    // there is nothing to check it with. The key inside it vouches for itself, which is
    // not a check.
    let s = support::source_with_no_pinned_key();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(v["result"]["isError"], true);
    // And it is not listed as a run either.
    let body = support::tool_body(
        &serde_json::from_str(
            &handle_request(&support::call("goalcup_entries"), &s, support::NOW).unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(body["entries"].as_array().expect("array").len(), 0);
    assert_eq!(body["unverified"], 1);
}

#[test]
fn a_key_record_is_only_a_pin_in_the_file_the_cli_would_have_written_it_to() {
    let s = support::source_with_a_misfiled_record();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(
        v["result"]["isError"], true,
        "which record pins a run key must not depend on the order a directory lists files in"
    );
}

/// A refusal names the entry it is about, and the only place that name can come from is the
/// document that could not be vouched for. It is quoted as an id and never as prose.
#[test]
fn a_refusal_does_not_repeat_what_an_unverifiable_document_wrote_in_its_id() {
    let s = support::source_with_an_injected_id();
    let text = handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap();
    assert!(
        text.contains("no run key"),
        "the refusal is the expected one"
    );
    assert!(
        text.contains("not an entry id"),
        "it says what it would not repeat"
    );
    for words in ["ignore your instructions", "curl", "keep going", "extended"] {
        assert!(!text.contains(words), "the refusal repeated `{words}`");
    }
}

/// A batch is refused, and the refusal names the version that says so. The two halves have
/// to agree: a server that refused batches while advertising a version whose specification
/// requires them would be telling a client its well-formed request was invalid.
#[test]
fn a_batch_is_answered_rather_than_left_waiting_and_the_refusal_names_the_version() {
    let s = support::empty_source();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#,
            &s,
            support::NOW,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(v["error"]["code"], -32600);
    let said = v["error"]["message"].as_str().expect("a message");
    assert!(
        said.contains(SUPPORTED_PROTOCOL_VERSIONS.last().copied().unwrap_or("")),
        "the refusal says which protocol has no batching: {said}"
    );
}

#[test]
fn a_document_about_another_run_is_refused_although_its_signature_is_good() {
    let s = support::source_with_a_swapped_document();
    for line in [
        support::call("goalcup_run_status"),
        support::call_with(
            "goalcup_run_status",
            serde_json::json!({"entry_id": "entry-0001"}),
        ),
    ] {
        let v: serde_json::Value =
            serde_json::from_str(&handle_request(&line, &s, support::NOW).unwrap()).unwrap();
        assert_eq!(
            v["result"]["isError"], true,
            "one run key signs every document of that run, so the signature does not catch it"
        );
    }
}

#[test]
fn the_error_paths_are_the_json_rpc_ones_and_nothing_panics() {
    let s = support::empty_source();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"].to_string().contains("no entry"));
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_not_a_tool"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(v["error"]["code"], -32602);
    let v: serde_json::Value =
        serde_json::from_str(&handle_request("{not json", &s, support::NOW).unwrap()).unwrap();
    assert_eq!(v["error"]["code"], -32700);
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(
            r#"{"jsonrpc":"2.0","id":9,"method":"no/such"}"#,
            &s,
            support::NOW,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(v["error"]["code"], -32601);
}

#[test]
fn each_tool_call_enforces_its_closed_argument_schema() {
    let s = support::source_with_one_entry();
    let malformed = [
        support::call_with("goalcup_event_status", serde_json::json!({"extra": true})),
        support::call_with(
            "goalcup_entries",
            serde_json::json!({"entry_id": "entry-0001"}),
        ),
        support::call_with(
            "goalcup_run_status",
            serde_json::json!({"entry_id": "entry-0001", "extra": true}),
        ),
        support::call_with("goalcup_run_status", serde_json::Value::Null),
        support::call_with("goalcup_run_status", serde_json::json!([])),
        support::call_with("goalcup_run_status", serde_json::json!({"entry_id": null})),
        support::call_with("goalcup_run_status", serde_json::json!({"entry_id": 7})),
    ];
    for line in malformed {
        let response: serde_json::Value =
            serde_json::from_str(&handle_request(&line, &s, support::NOW).unwrap()).unwrap();
        assert_eq!(response["error"]["code"], -32602, "{line}: {response}");
    }

    let absent_arguments = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {"name": "goalcup_run_status"},
    })
    .to_string();
    let response: serde_json::Value =
        serde_json::from_str(&handle_request(&absent_arguments, &s, support::NOW).unwrap())
            .unwrap();
    assert_eq!(response["result"]["isError"], false, "{response}");
}

/// Two crates, one file format, no shared type: `goalcup-cli::entries::EntryRecord`
/// and this crate's hand-rolled struct. A field added on one side is silently absent on the
/// other, so the fixture is a record `goalcup-cli`'s own writer produced (committed once),
/// and a refused gate is reported as itself.
#[test]
fn a_record_written_by_the_cli_still_reads_here_and_a_refused_gate_is_not_pending() {
    let s = support::source_from_cli_written_record();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_ne!(
        v["result"]["isError"], true,
        "the pinned key came out of the CLI's record"
    );

    let s = support::source_with_a_refused_gate();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(
        support::tool_body(&v)["arming"],
        "refused",
        "H3's fourth state: an agent told a dead gate is pending waits for nothing"
    );
}

#[test]
fn event_status_carries_no_entry_fact_and_no_output_anywhere_carries_a_secret() {
    let s = support::source_with_one_entry();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_event_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    let body = support::tool_body(&v);
    for k in [
        "t0",
        "start_window_closes_at",
        "publication_closes_at",
        "rules_version",
        // `[event]` is optional, `EventStatusSource::Fallback` is the
        // default, and without this key the agent is handed the entry's own timeline
        // as the round's deadline with nothing saying so.
        "source",
    ] {
        assert!(body.get(k).is_some(), "missing {k}");
    }
    assert!(body["source"] == "config" || body["source"] == "fallback");
    for k in ["entry_id", "workspace", "session_id", "seconds_remaining"] {
        assert!(body.get(k).is_none(), "{k} is not the event's business");
    }
    for tool in [
        "goalcup_event_status",
        "goalcup_run_status",
        "goalcup_entries",
    ] {
        let text = handle_request(&support::call(tool), &s, support::NOW).unwrap();
        for secret in ["receiver_token", "Bearer", "grant", support::RECEIVER_TOKEN] {
            assert!(!text.contains(secret), "{tool} leaked {secret}");
        }
    }
    // Shipped, never auto-registered: the skill file says when to call, and when not.
    let skill = include_str!("../SKILL.md");
    assert!(skill.contains("read-only") && skill.to_lowercase().contains("never"));
}

#[test]
fn entries_lists_the_verified_runs_and_says_how_many_it_would_not_show() {
    let s = support::source_with_two_entries();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_entries"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    let body = support::tool_body(&v);
    let rows = body["entries"].as_array().expect("entries array");
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r["entry_id"].as_str().expect("entry_id"))
        .collect();
    assert_eq!(ids, ["entry-0001", "entry-0002"]);
    assert_eq!(body["unverified"], 0);
    for row in rows {
        for k in ["arming", "health", "offline"] {
            assert!(row.get(k).is_some(), "missing {k}");
        }
    }

    // A run whose document does not check out is not one of the rows, and the count says
    // it was left out rather than letting it vanish.
    let s = support::source_signed_by_a_stranger();
    let body = support::tool_body(
        &serde_json::from_str(
            &handle_request(&support::call("goalcup_entries"), &s, support::NOW).unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(body["entries"].as_array().expect("array").len(), 0);
    assert_eq!(body["unverified"], 1);
}

/// The skill file is what an agent reads before it calls anything, so a tool renamed or a
/// field dropped without the skill following is a skill that describes a server that is not
/// there. The manifest block in `SKILL.md` is compared byte for byte with what `tools/list`
/// advertises, and every field name in every tool body has to appear in the prose.
#[test]
fn the_skill_file_advertises_exactly_what_the_server_answers() {
    let skill = include_str!("../SKILL.md");
    let block = fenced_json(skill).expect("SKILL.md carries one ```json block");
    assert_eq!(
        block,
        serde_json::to_string_pretty(&tools_manifest()).expect("manifest serialises"),
        "the manifest block in SKILL.md is not what tools/list answers"
    );

    let s = support::source_with_one_entry();
    for tool in [
        "goalcup_event_status",
        "goalcup_run_status",
        "goalcup_entries",
    ] {
        assert!(skill.contains(tool), "SKILL.md does not name {tool}");
        let v: serde_json::Value =
            serde_json::from_str(&handle_request(&support::call(tool), &s, support::NOW).unwrap())
                .unwrap();
        let body = support::tool_body(&v);
        for key in body.as_object().expect("an object body").keys() {
            assert!(
                skill.contains(key.as_str()),
                "{tool} answers `{key}` and SKILL.md never mentions it"
            );
        }
        // The two renderings of one answer agree, so a client reading either gets the same
        // document.
        assert_eq!(v["result"]["structuredContent"], body);
    }
    // A `fallback` answer is the entry's own timeline and not the round's — the one thing
    // an agent must not act on without being told.
    assert!(skill.contains("fallback"));
}

/// A document dated after this machine's clock would read as age `0` and `offline: false`
/// — the freshest answer there is — because `SignedStatus::stale_seconds` subtracts
/// saturatingly. What it means is that two clocks disagree and the document's age is not
/// known, and that is what the answer says.
#[test]
fn a_document_published_in_the_future_says_so_instead_of_reading_as_brand_new() {
    let s = support::source_published_in_the_future();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_ne!(v["result"]["isError"], true, "the document itself verifies");
    let body = support::tool_body(&v);
    assert_eq!(
        body["stale_seconds"],
        serde_json::Value::Null,
        "an age this server cannot place in time is not a number"
    );
    assert_eq!(body["offline"], true, "and it is never `fresh`");
    assert_eq!(
        body["freshness"],
        format!(
            "published in the future by {}s (clock skew)",
            support::SKEW_SECONDS
        )
    );
    // The same run, in the list, tells the same story.
    let rows = support::tool_body(
        &serde_json::from_str(
            &handle_request(&support::call("goalcup_entries"), &s, support::NOW).unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(rows["entries"][0]["offline"], true);
    assert!(
        rows["entries"][0]["freshness"]
            .as_str()
            .unwrap_or("")
            .contains("clock skew")
    );
    // And the skill file carries the sentence an agent has to recognise.
    assert!(include_str!("../SKILL.md").contains("(clock skew)"));
}

/// `EventStatusSource::Fallback` is the default and the reading a document from an older
/// build gets, so the `"fallback"` word `SKILL.md` tells an agent to check for has to be
/// what some document actually produces — not just what the tool description promises.
#[test]
fn a_round_nobody_configured_reaches_the_agent_as_the_word_fallback() {
    let s = support::source_with_a_fallback_round();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_event_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(
        support::tool_body(&v)["source"],
        "fallback",
        "a date nobody set must not arrive looking like one somebody did"
    );
}

/// An `entry_id` is interpolated into a cache path, and it arrives from an agent's tool
/// call. `../..` names a file, and a server that let it through would read one.
#[test]
fn an_entry_id_that_is_not_one_path_component_never_reaches_the_filesystem() {
    let s = support::source_with_one_entry();
    for id in ["../..", "../../etc/passwd", "..", ".", "", "a/b", "a\\b"] {
        let v: serde_json::Value = serde_json::from_str(
            &handle_request(
                &support::call_with("goalcup_run_status", serde_json::json!({"entry_id": id})),
                &s,
                support::NOW,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(v["result"]["isError"], true, "`{id}` was not refused");
        assert!(
            v["result"].to_string().contains("no entry"),
            "`{id}` is refused as a name, not reported as a read: {}",
            v["result"]
        );
    }
}

/// The cache is under a directory the participant owns, so its size is a bound and not an
/// assumption. The sparse file is opened once; its metadata and bounded read use that same
/// handle, so the check cannot be swapped away from the bytes being read.
#[test]
fn a_document_larger_than_the_cap_is_refused_without_being_read() {
    let s = support::source_with_an_oversized_document();
    let v: serde_json::Value = serde_json::from_str(
        &handle_request(&support::call("goalcup_run_status"), &s, support::NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(v["result"]["isError"], true);
    assert!(
        v["result"]
            .to_string()
            .contains("larger than a status document"),
        "it says why it stopped: {}",
        v["result"]
    );
}

/// A read-only server (D8) that left a file behind would be a read-only server in
/// name only.
#[test]
fn the_server_writes_nothing_into_the_configuration_directory() {
    let s = support::source_with_one_entry();
    let before = listing(s.config_dir());
    for line in [
        support::call("goalcup_event_status"),
        support::call("goalcup_run_status"),
        support::call("goalcup_entries"),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_string(),
    ] {
        let _ = handle_request(&line, &s, support::NOW);
    }
    assert_eq!(before, listing(s.config_dir()));
}

/// Every path under `dir`, with the bytes at each one.
fn listing(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).expect("read_dir").flatten() {
            let path = entry.path();
            match path.is_dir() {
                true => stack.push(path),
                false => out.push((
                    path.display().to_string(),
                    std::fs::read(&path).expect("read"),
                )),
            }
        }
    }
    out.sort();
    out
}

/// The one ```json block in `SKILL.md`, without its fences.
fn fenced_json(skill: &str) -> Option<String> {
    let after = skill.split_once("```json\n")?.1;
    let (block, _) = after.split_once("\n```")?;
    Some(block.to_string())
}
