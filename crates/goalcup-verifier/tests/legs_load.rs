use goalcup_verifier::legs::{Leg1Store, dotted, dotted_through_strings};

#[path = "support/mod.rs"]
mod fixture_support;
use goalcup_verifier::trace::TraceDescriptor;

fn trace(id: &str) -> tempfile::TempDir {
    fixture_support::materialize_fixture(&format!("tests/traces/{id}"))
}

fn load_trace(fixture: &tempfile::TempDir, id: &str) -> TraceDescriptor {
    TraceDescriptor::load(fixture.path()).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn claude_code_trace_loads_both_legs() {
    let fixture = trace("claude-code-cli");
    let t = load_trace(&fixture, "claude-code-cli");
    assert_eq!(t.connector, "claude-code-cli");
    assert_eq!(
        t.tracked_sessions,
        vec!["95addb64-9dac-4046-8ef6-8aa53fa0c7f1".to_string()]
    );
    assert!(t.manifest_text().is_ok(), "manifest is rooted and readable");

    let main = t
        .load_leg1(t.leg1.iter().find(|s| s.name == "main").unwrap())
        .unwrap();
    assert_eq!(main.records.len(), 148);
    assert_eq!(main.records[0].seq, 1);
    assert_eq!(dotted(&main.records[0].value, "type").unwrap(), "mode");
    let rep = main.link_report();
    assert!(
        rep.recomputable,
        "a raw jsonl store is chained by the verifier itself"
    );
    assert!(!rep.scrubbed);
    assert_eq!(
        rep.recomputed, 148,
        "every record's hash was recomputed and matched"
    );
    assert_eq!(rep.recompute_mismatches, 0);
    assert_eq!(rep.broken, 0);
    assert_eq!(rep.seq_gaps, 0);
    assert_eq!(
        main.records[147].hash,
        "c2161937817723a5c4f33bb5b04eff93d45afc8a91a09cdf3d08154a275da5c5"
    );

    let sub = t
        .load_leg1(t.leg1.iter().find(|s| s.name == "subagent").unwrap())
        .unwrap();
    assert_eq!(sub.records.len(), 17);

    let leg2 = t.load_leg2(t.leg2.as_ref().unwrap()).unwrap();
    assert_eq!(leg2.posts.len(), 50);
    assert_eq!(
        leg2.by_event("UserPromptSubmit").len(),
        20,
        "two hook sources, ten prompts each"
    );
    assert_eq!(
        leg2.by_event("SessionStart").len(),
        3,
        "one startup, two for the single resume"
    );
    assert_eq!(
        leg2.by_event("SubagentStop").len(),
        4,
        "1 real + 3 suggestion-agent"
    );
}

#[test]
fn codex_trace_loads_both_rollouts() {
    let fixture = trace("codex-cli");
    let t = load_trace(&fixture, "codex-cli");
    let main = t
        .load_leg1(t.leg1.iter().find(|s| s.name == "rollout-main").unwrap())
        .unwrap();
    assert_eq!(main.records.len(), 161);
    assert_eq!(
        dotted(&main.records[0].value, "type").unwrap(),
        "session_meta"
    );
    assert_eq!(
        dotted(&main.records[0].value, "payload.thread_source").unwrap(),
        "user"
    );
    let sub = t
        .load_leg1(
            t.leg1
                .iter()
                .find(|s| s.name == "rollout-subagent")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(sub.records.len(), 39);
    assert_eq!(
        dotted(&sub.records[0].value, "payload.thread_source").unwrap(),
        "subagent"
    );
    // The second record is the parent's session_meta, verbatim.
    assert_eq!(
        dotted(&sub.records[1].value, "type").unwrap(),
        "session_meta"
    );
    assert_eq!(
        dotted(&sub.records[1].value, "payload.thread_source").unwrap(),
        "user"
    );

    let leg2 = t.load_leg2(t.leg2.as_ref().unwrap()).unwrap();
    assert_eq!(leg2.posts.len(), 45, "44 real posts plus the /Ping warm-up");
    assert_eq!(leg2.by_event("UserPromptSubmit").len(), 8);
    assert_eq!(leg2.by_event("SessionStart").len(), 2);
}

#[test]
fn a_file_first_record_store_with_no_teaching_record_has_no_key() {
    // `wrong-cwd-01a06616-excerpt.jsonl` is 2,310 bytes of two `response_item`
    // records and no `session_meta`. The id `01a06616` appears nowhere in its bytes —
    // only in the filename and the fixture README. Codex keys a file on its first
    // `session_meta`, so this file has no key, and the rule is: no key ⇒ discard
    // and report as an unattributable store. Assert that from the bytes rather than
    // trusting the descriptor's `foreign = true`.
    let fixture = trace("codex-cli");
    let t = load_trace(&fixture, "codex-cli");
    let spec = t
        .leg1
        .iter()
        .find(|s| s.name == "wrong-cwd-01a06616")
        .unwrap();
    assert!(
        spec.foreign,
        "the descriptor declares it, but the assertion below measures it"
    );
    let store = t.load_leg1(spec).unwrap();
    assert_eq!(
        store.records.len(),
        2,
        "two response_item records and nothing else"
    );
    assert!(
        !store
            .records
            .iter()
            .any(|r| dotted(&r.value, "type").and_then(|t| t.as_str()) == Some("session_meta")),
        "the excerpt carries no teaching record, so the file has no partition key"
    );
    let text = t.leg1_text(spec).unwrap();
    assert!(
        !text.contains("01a06616"),
        "the thread id is in the filename, not in the bytes; V4 cannot corroborate it"
    );
}

#[test]
fn raw_bodies_are_loaded_hashed_and_keyed_by_their_filename_stem() {
    // Every trace descriptor names the `[[raw]]` bodies; loading them makes V1's "raw files
    // hash-linked from `extra`" true.
    let fixture = trace("claude-code-cli");
    let t = load_trace(&fixture, "claude-code-cli");
    let raw = t.load_raw().unwrap();
    assert_eq!(raw.len(), 4, "four [[raw]] bodies in this descriptor");
    let goal_response = raw
        .iter()
        .find(|r| r.name == "goal-turn.response")
        .expect("the goal turn's response body");
    assert_eq!(goal_response.sha256.len(), 64);
    assert!(goal_response.bytes > 0);
    // The stem is the join key back to the transcript's `requestId`.
    assert_eq!(goal_response.stem, "rawbody-goal-turn.response");
    // The response body's own `id` is the Claude API message id
    // (`msg_011Ceg8Tvu4RRy7ZrUGaa9UY`), not the assistant record's `requestId`
    // (`req_011Ceg8TuCNL5CMWq1MQVz7j`, per `fixtures/README.md`'s parenthetical):
    // `grep -c req_ rawbody-goal-turn.response.json` is 0. The README's `(req_…)` names
    // which transcript record this response joins to by file curation, not a field the
    // response body itself carries. `spikes/` is a read-only input, so the assertion
    // matches the real, redacted bytes.
    assert_eq!(
        goal_response.value.get("id").and_then(|v| v.as_str()),
        Some("msg_011Ceg8Tvu4RRy7ZrUGaa9UY"),
        "the response body's own id is the API message id, not the transcript's requestId"
    );
    // Two runs over the same file give the same hash: the link is stable across machines.
    assert_eq!(t.load_raw().unwrap()[0].sha256, raw[0].sha256);
}

#[test]
fn the_m0_descriptors_carry_no_anchor_and_no_seal() {
    // The three M0 fixture sets predate the daemon: no events stream, no acknowledgements,
    // no seal and no server key. V12 reports `no_anchor` on all three, and it is right to.
    for id in ["claude-code-cli", "codex-cli"] {
        let fixture = trace(id);
        let t = load_trace(&fixture, id);
        assert!(
            t.acks.is_none(),
            "{id}: an M0 fixture carries no acknowledgements"
        );
        assert!(
            t.seal.is_none(),
            "{id}: no M0 spike produced a SealManifest"
        );
        assert!(t.server_public_key.is_none(), "{id}: no key to pin");
        assert!(t.load_acks().unwrap().is_empty());
        assert!(
            t.is_legacy(),
            "an M0 descriptor declares no chain, so it carries no envelope stream"
        );
        assert!(t.load_chain().unwrap().is_none());
        assert!(t.load_seal().unwrap().is_none());
        assert!(
            t.leg1.iter().all(|s| !s.expected_reread),
            "{id}: no store in an M0 trace declares an expected re-read"
        );
    }
}

/// A `data` column that is a SQLite TEXT column reaches the verifier as a JSON string at
/// `data`, not an object. A plain [`dotted`] lookup stops at that string and never reaches
/// `directory`; [`dotted_through_strings`] parses it and continues, so V10's
/// `Direct`-partition sweep cannot disagree with what the daemon partitioned on at capture
/// time.
#[test]
fn dotted_through_strings_descends_through_a_json_encoded_data_column() {
    let row = serde_json::json!({
        "type": "session.created.1",
        "data": "{\"sessionID\":\"ses_1\",\"info\":{\"directory\":\"/ws\"}}"
    });
    assert_eq!(
        dotted_through_strings(&row, "data.info.directory").as_deref(),
        Some("/ws"),
        "the walk must parse the encoded `data` string and continue inside it"
    );
    assert!(
        dotted(&row, "data.info.directory").is_none(),
        "plain `dotted` stops at the string node — this is the divergence being closed"
    );
    // A path that never crosses an encoded string still works exactly as `dotted` does.
    assert_eq!(
        dotted_through_strings(&row, "type").as_deref(),
        Some("session.created.1")
    );
    // An absent field, and a string leaf that is not JSON where more path remains, are
    // both `None` rather than a guess.
    assert!(dotted_through_strings(&row, "data.missing.field").is_none());
    assert!(dotted_through_strings(&row, "type.no_such_field").is_none());
}

#[test]
fn an_unscrubbed_chain_export_is_recomputable() {
    // What the daemon actually writes: the recorded hash is
    // link_hash(prev, canonical_json(line)). Built here rather than read from a fixture,
    // because no committed fixture is unscrubbed.
    use goalcup_protocol::chain::Chain;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chain.jsonl");
    let mut chain = Chain::new();
    let mut text = String::new();
    for i in 0..5u64 {
        let line = serde_json::json!({"db": "harness.db", "table": "event", "row": {"n": i}});
        let raw = serde_json::to_vec(&line).unwrap();
        let link = chain.try_append(&raw).unwrap();
        text.push_str(
            &serde_json::json!({
                "t": 1_788_434_416.0 + i as f64,
                "src": "event",
                "seq": link.seq,
                "prev": link.prev,
                "hash": link.hash,
                "line": line,
            })
            .to_string(),
        );
        text.push('\n');
    }
    std::fs::write(&path, text).unwrap();

    let store = Leg1Store::from_chain_jsonl_reader(
        "event",
        &path.display().to_string(),
        std::fs::File::open(&path).unwrap(),
        false,
    )
    .unwrap();
    let rep = store.link_report();
    assert!(!rep.scrubbed);
    assert_eq!(rep.recomputed, 5);
    assert_eq!(rep.recompute_mismatches, 0);
    assert!(
        rep.recomputable,
        "a live daemon export recomputes; only a scrub does not"
    );
    assert_eq!(rep.broken, 0);
}

/// The `[leg2]` file a legacy descriptor declares. `leg2` is an `Option` because a chain
/// trace carries its posts in the chain and declares no `[leg2]` table; every descriptor
/// this file loads is an M0 one, which declares it.

// ---------------------------------------------------------------- the tail
//
// A hard kill leaves a half-written final line on disk — the exact scenario the tracker
// exists to record — and the verifier answers it with a verdict, never exit 3 and no
// verdict at all.

#[test]
fn a_half_written_final_line_is_a_flag_and_still_yields_a_verdict() {
    // Reproduce a hard kill by truncating the last line mid-record.
    let t = fixture_support::materialize_fixture("tests/traces/claude-code-cli");
    let main = t.path().join("main-session.jsonl");
    let text = std::fs::read_to_string(&main).unwrap();
    let cut = text.len() - 40;
    std::fs::write(&main, &text[..cut]).unwrap();

    let v = goalcup_verifier::verify_trace(t.path()).expect("a truncated tail is not exit 3");
    let f = v
        .review_queue
        .iter()
        .find(|f| f.code == "truncated_tail")
        .unwrap_or_else(|| {
            panic!(
                "truncated_tail; queue held {:?}",
                v.review_queue.iter().map(|f| &f.code).collect::<Vec<_>>()
            )
        });
    assert_eq!(f.severity, goalcup_verifier::checks::Severity::Flag);
    assert!(
        f.message.contains("byte"),
        "the shortfall is named: {}",
        f.message
    );
    assert!(
        f.message.contains("897"),
        "the byte length is the shortfall itself: {}",
        f.message
    );
    // The store the tail belongs to is named, and it is the one that was cut.
    assert!(
        f.evidence.iter().any(|e| e.store == "main"),
        "{:?}",
        f.evidence
    );
    // A verdict, not a crash: exit 1 or 2, never 3.
    assert!(
        v.exit_code() == 1 || v.exit_code() == 2,
        "exit {}",
        v.exit_code()
    );

    // Every complete record before the tail still parsed: 148 records, one cut off.
    let d = goalcup_verifier::trace::TraceDescriptor::load(t.path()).unwrap();
    let store = d
        .load_leg1(d.leg1.iter().find(|s| s.name == "main").unwrap())
        .unwrap();
    assert_eq!(store.records.len(), 147);
    assert_eq!(store.truncated_tail, Some(897));

    // A break in the middle is still a hard error: only the tail is forgiven.
    let broken = fixture_support::materialize_fixture("tests/traces/claude-code-cli");
    support::corrupt_middle_line(&broken.path().join("main-session.jsonl"));
    let err = goalcup_verifier::verify_trace(broken.path())
        .expect_err("a corrupted middle line is not a tail");
    let text = err.to_string();
    assert!(text.contains("main-session.jsonl"), "{text}");
    assert!(text.contains("line 75"), "the line number is named: {text}");
}

/// A cut that lands exactly on a line boundary is not a truncated tail: every byte on
/// disk parses, so there is nothing unfinished to report. The file is simply shorter,
/// which is what a store that stopped being written looks like — a fact for V9's tail
/// notice and the gap classifier, not a parse failure this loader can see.
#[test]
fn a_cut_on_a_line_boundary_is_not_a_tail_because_nothing_is_half_written() {
    let t = fixture_support::materialize_fixture("tests/traces/claude-code-cli");
    let main = t.path().join("main-session.jsonl");
    let text = std::fs::read_to_string(&main).unwrap();
    let keep: String = text.lines().take(100).map(|l| format!("{l}\n")).collect();
    std::fs::write(&main, &keep).unwrap();

    let d = goalcup_verifier::trace::TraceDescriptor::load(t.path()).unwrap();
    let store = d
        .load_leg1(d.leg1.iter().find(|s| s.name == "main").unwrap())
        .unwrap();
    assert_eq!(store.records.len(), 100);
    assert_eq!(store.truncated_tail, None);

    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert!(
        !v.review_queue.iter().any(|f| f.code == "truncated_tail"),
        "no byte on disk is half-written"
    );
}

/// The committed fixtures carry no truncated tail, so the field is silent where it
/// should be — a flag that fired on every clean trace would say nothing about a kill.
#[test]
fn a_complete_store_reports_no_tail() {
    for id in ["claude-code-cli", "codex-cli"] {
        let fixture = trace(id);
        let t = load_trace(&fixture, id);
        for spec in &t.leg1 {
            let store = t.load_leg1(spec).unwrap();
            assert_eq!(
                store.truncated_tail, None,
                "{id}/{}: a committed fixture ends in a newline",
                spec.name
            );
        }
        let v = goalcup_verifier::verify_trace(fixture.path()).unwrap();
        assert!(!v.review_queue.iter().any(|f| f.code == "truncated_tail"));
    }
}

// ------------------------------------------------------------- the census

#[test]
fn every_ignored_record_kind_is_counted_and_published() {
    let fixture = trace("claude-code-cli");
    let v = goalcup_verifier::verify_trace(fixture.path()).unwrap();
    let c = &v.parser_census;
    assert!(!c.is_empty(), "9 kinds were dropped silently before this");
    assert!(
        c.contains_key("last-prompt"),
        "{:?}",
        c.keys().collect::<Vec<_>>()
    );
    assert!(c.contains_key("queue-operation"));
    // 48 = `main` + `subagent`. The descriptor declares a third store, `a9-other-folder`,
    // with `foreign = true`; its 7 further drops are V10's story and are not counted here.
    assert_eq!(c.values().sum::<u64>(), 48);
    // Per-kind, so the scoping is asserted and not only the total: `main` holds 5
    // `atis-latch` rows and the foreign store holds 3 more that must not be added.
    assert_eq!(c.get("atis-latch"), Some(&5));
    assert_eq!(c.get("last-prompt"), Some(&7));
    assert_eq!(c.get("queue-operation"), Some(&2));
    assert_eq!(c.len(), 9, "9 kinds, all from the entry's own stores");
    // Named in full, so a kind appearing or disappearing is read in a diff rather than
    // absorbed by a total that still sums to 48.
    let named: Vec<(&str, u64)> = c.iter().map(|(k, n)| (k.as_str(), *n)).collect();
    assert_eq!(
        named,
        vec![
            ("ai-title", 4),
            ("atis-latch", 5),
            ("bridge-session", 8),
            ("cost-state", 3),
            ("file-history-snapshot", 9),
            ("last-prompt", 7),
            ("mode", 5),
            ("permission-mode", 5),
            ("queue-operation", 2),
        ]
    );
    // Deterministic and ordered, because the verdict is compared byte for byte.
    let json = v.to_json().unwrap();
    let again = goalcup_verifier::verify_trace(fixture.path()).unwrap();
    assert_eq!(json, again.to_json().unwrap());
}

/// The census names kinds and never contents. `last-prompt` is in it precisely because
/// that kind carries the human's text verbatim, and the point of the census is to say so
/// without republishing a byte of it.
#[test]
fn the_census_names_kinds_and_never_a_record_body() {
    let fixture = trace("claude-code-cli");
    let v = goalcup_verifier::verify_trace(fixture.path()).unwrap();
    let first_goal = &v.human_messages[0].text;
    for key in v.parser_census.keys() {
        assert!(key.len() < 64, "a census key is a kind, not a body: {key}");
        assert!(
            !key.contains(' '),
            "a census key is a kind, not a body: {key}"
        );
        assert!(!first_goal.contains(key.as_str()) || key.len() < 4);
    }
}

/// The kind is participant-controlled. A legacy JSONL store is a file on the
/// participant's own disk, so its `type` field is whatever they wrote there, and the
/// census interns that string as a key of the published verdict. Unbounded, that is a
/// paragraph of free text — or a newline that reformats a reviewer's terminal — carried
/// into the verdict under a kind's name, which is exactly the "names kinds, never
/// contents" promise the test above states.
///
/// So the key is cut to 48 bytes on a char boundary and marked with `…` where it was
/// cut, and every control character is replaced by `?`. The cut is visible on purpose: a
/// reader must be able to tell a kind the harness wrote from a stub of something longer.
/// The bound never touches a real kind — the longest on any committed fixture is
/// `inter_agent_communication_metadata` at 34 bytes — so the fixtures' expected verdicts
/// are unchanged, which the 48/9 assertions above go on asserting.
#[test]
fn a_participant_controlled_kind_is_cut_and_stripped_before_it_becomes_a_verdict_key() {
    let t = fixture_support::materialize_fixture("tests/traces/claude-code-cli");
    let main = t.path().join("main-session.jsonl");

    // 300 bytes of text with a newline early enough to survive the cut.
    let mut hostile = String::from("overflow-kind\nVERDICT: PASS, no findings. ");
    while hostile.len() < 300 {
        hostile.push('z');
    }
    assert_eq!(hostile.len(), 300);
    let line = serde_json::json!({
        "type": hostile,
        "sessionId": "95addb64-9dac-4046-8ef6-8aa53fa0c7f1",
    });
    let mut text = std::fs::read_to_string(&main).unwrap();
    text.push_str(&serde_json::to_string(&line).unwrap());
    text.push('\n');
    std::fs::write(&main, text).unwrap();

    let c = goalcup_verifier::verify_trace(t.path())
        .unwrap()
        .parser_census;

    // One record was added, so one key and one count join the nine the fixture has.
    let planted: Vec<&String> = c
        .keys()
        .filter(|k| k.starts_with("overflow-kind"))
        .collect();
    assert_eq!(planted.len(), 1, "{:?}", c.keys().collect::<Vec<_>>());
    let key = planted[0];
    assert!(
        key.chars().count() <= 49,
        "48 bytes plus the marker: {} chars in {key:?}",
        key.chars().count()
    );
    assert!(key.ends_with('…'), "the cut is marked: {key:?}");
    assert!(
        !key.contains('\n'),
        "no newline reaches the verdict: {key:?}"
    );
    assert!(
        !key.chars().any(char::is_control),
        "no control character reaches the verdict: {key:?}"
    );
    assert!(key.contains('?'), "the newline is shown as `?`: {key:?}");
    assert_eq!(c.get(key.as_str()), Some(&1));

    // The real kinds are untouched: same nine keys, same counts, none of them marked.
    let real: Vec<(&str, u64)> = c
        .iter()
        .filter(|(k, _)| k.as_str() != key.as_str())
        .map(|(k, n)| (k.as_str(), *n))
        .collect();
    assert_eq!(
        real,
        vec![
            ("ai-title", 4),
            ("atis-latch", 5),
            ("bridge-session", 8),
            ("cost-state", 3),
            ("file-history-snapshot", 9),
            ("last-prompt", 7),
            ("mode", 5),
            ("permission-mode", 5),
            ("queue-operation", 2),
        ]
    );
    assert_eq!(
        c.values().sum::<u64>(),
        49,
        "48 real drops plus the planted one"
    );
}

/// The other two harnesses, measured from their own fixtures rather than predicted.
#[test]
fn the_census_is_measured_on_every_committed_trace() {
    let census = |id: &str| {
        let fixture = trace(id);
        goalcup_verifier::verify_trace(fixture.path())
            .unwrap()
            .parser_census
    };

    // Codex: the rollout's four unread record kinds. `session_meta` counts once — the
    // subagent rollout's second one, the one that repeats the parent's meta verbatim.
    // Each file's first `session_meta` is the record the parser keys the whole file on,
    // so it is read and not ignored. The `foreign = true` `wrong-cwd-01a06616` excerpt
    // contributes nothing, and neither does `history`, which the parser reads in full.
    let want = |pairs: &[(&str, u64)]| -> std::collections::BTreeMap<String, u64> {
        pairs.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    };
    assert_eq!(
        census("codex-cli"),
        want(&[
            ("inter_agent_communication_metadata", 2),
            ("session_meta", 1),
            ("token_usage_record", 17),
            ("world_state", 9),
        ])
    );
}

/// The other legacy loader. `from_raw_jsonl` is what the truncation test above exercises
/// through `verify_trace`; `from_chain_jsonl` reads a different shape of line and has its
/// own hard error, so the tail is measured here too rather than assumed to behave the same.
#[test]
fn a_chain_jsonl_store_reports_its_tail_instead_of_refusing_the_file() {
    use goalcup_protocol::chain::Chain;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("event.jsonl");
    let mut chain = Chain::new();
    let mut text = String::new();
    for i in 0..4u64 {
        let line = serde_json::json!({"db": "harness.db", "table": "event", "row": {"n": i}});
        let link = chain
            .try_append(&serde_json::to_vec(&line).unwrap())
            .unwrap();
        text.push_str(
            &serde_json::json!({
                "t": 1_788_434_416.0 + i as f64,
                "seq": link.seq, "prev": link.prev, "hash": link.hash, "line": line,
            })
            .to_string(),
        );
        text.push('\n');
    }
    // A kill mid-append: the last envelope stops 30 bytes short of finishing.
    let partial = &text[..text.len() - 30];
    std::fs::write(&path, partial).unwrap();
    // What is left of the interrupted line: everything after the last terminator.
    let unfinished = partial.rsplit('\n').next().unwrap();

    let store = Leg1Store::from_chain_jsonl_reader(
        "event",
        &path.display().to_string(),
        std::fs::File::open(&path).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(
        store.records.len(),
        3,
        "the three finished lines still load"
    );
    assert_eq!(store.truncated_tail, Some(unfinished.len() as u64));
    assert!(
        store.link_report().recomputable,
        "the survivors still recompute"
    );

    // A break in the middle of the same file is still a refusal.
    let mut lines: Vec<&str> = text.lines().collect();
    lines[1] = "{";
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    let err = Leg1Store::from_chain_jsonl_reader(
        "event",
        &path.display().to_string(),
        std::fs::File::open(&path).unwrap(),
        false,
    )
    .unwrap_err();
    assert!(err.to_string().contains("line 2"), "{err}");
}

/// Copying the fixtures rather than editing them: `spikes/` is a read-only input, so a test
/// that needs a truncated store makes its own copy.
mod support {
    use std::path::Path;

    /// Replace the middle line of a JSONL file with a bare `{` — a parse failure that is
    /// neither the final line nor unterminated, so it must stay a hard error.
    pub fn corrupt_middle_line(path: &Path) {
        let text = std::fs::read_to_string(path).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let middle = lines.len() / 2;
        lines[middle] = "{".to_string();
        let mut out = lines.join("\n");
        out.push('\n');
        std::fs::write(path, out).unwrap();
    }
}
