#[path = "support/mod.rs"]
mod fixture_support;

use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_verifier::atif::StepSource;
use goalcup_verifier::legs::Leg1Store;
use goalcup_verifier::parsers::{FieldValue, parser_for_family};
use goalcup_verifier::trace::TraceDescriptor;

fn parsed() -> goalcup_verifier::parsers::ParsedLegs {
    let fixture = fixture_support::materialize_fixture("tests/traces/claude-code-cli");
    let trace = TraceDescriptor::load(fixture.path()).unwrap();
    let manifest = ConnectorManifest::from_toml_str(&trace.manifest_text().unwrap()).unwrap();
    let leg1: Vec<Leg1Store> = trace
        .leg1
        .iter()
        .map(|s| trace.load_leg1(s).unwrap())
        .collect();
    let leg2 = trace.load_leg2(trace.leg2.as_ref().unwrap()).unwrap();
    parser_for_family(&manifest.family)
        .unwrap()
        .parse(&trace, &manifest, &leg1, &leg2)
        .unwrap()
}

#[test]
fn extracts_every_user_slot_record_with_its_provenance() {
    let p = parsed();
    let main: Vec<_> = p.user_slots.iter().filter(|u| u.store == "main").collect();
    // 8 typed + <task-notification> + `Continue from where you left off.` = 10 text user slots.
    assert_eq!(
        main.len(),
        10,
        "got {:?}",
        main.iter().map(|u| &u.text).collect::<Vec<_>>()
    );

    let goal = &main[0];
    assert_eq!(
        goal.text,
        "create hello.py that prints the current date, run it, then tell me the output"
    );
    assert_eq!(
        goal.field("origin.kind"),
        Some(&FieldValue::Str("human".into()))
    );
    assert_eq!(
        goal.field("promptSource"),
        Some(&FieldValue::Str("typed".into()))
    );
    assert_eq!(
        goal.pair_key.as_deref(),
        Some("58740b59-3dfe-46fb-8704-7d049d21a7ed")
    );
    assert_eq!(goal.session_id, "95addb64-9dac-4046-8ef6-8aa53fa0c7f1");
    assert_eq!(
        goal.directory.as_deref(),
        Some("/Users/team/goalcup-spikes/ws-claude-code-cli-20260903T073054Z")
    );

    let notif = main
        .iter()
        .find(|u| u.text.starts_with("<task-notification>"))
        .unwrap();
    assert_eq!(
        notif.field("origin.kind"),
        Some(&FieldValue::Str("task-notification".into()))
    );
    assert_eq!(
        notif.field("promptSource"),
        Some(&FieldValue::Str("system".into()))
    );
    assert!(
        notif.field("isMeta").is_none(),
        "spike 4 A2: isMeta is absent on this record"
    );

    let resume = main
        .iter()
        .find(|u| u.text == "Continue from where you left off.")
        .unwrap();
    assert_eq!(resume.field("isMeta"), Some(&FieldValue::Bool(true)));
    assert!(
        resume.pair_key.is_none(),
        "the synthetic resume turn has no promptId"
    );
}

#[test]
fn leg2_candidates_dedupe_the_two_hook_sources() {
    let p = parsed();
    // 20 UserPromptSubmit posts, 10 distinct prompt_ids.
    assert_eq!(p.leg2_candidates.len(), 10);
    let overlength = p
        .leg2_candidates
        .iter()
        .find(|c| c.text.starts_with("OVERLENGTH-TEST-A7"))
        .expect("the blocked prompt is present on leg 2");
    assert_eq!(overlength.text.chars().count(), 4100);
    assert_eq!(
        p.blocked_prompts.len(),
        1,
        "one system/informational block notice"
    );
    assert!(p.blocked_prompts[0].starts_with("UserPromptSubmit operation blocked by hook:"));
}

#[test]
fn builds_an_atif_trajectory_with_raw_links_and_a_subagent() {
    let p = parsed();
    assert_eq!(p.trajectory.agent.name, "claude-code-cli");
    assert_eq!(p.trajectory.agent.version, "2.1.259");
    assert_eq!(
        p.trajectory.session_id.as_deref(),
        Some("95addb64-9dac-4046-8ef6-8aa53fa0c7f1")
    );
    assert!(
        p.trajectory.steps.len() >= 27,
        "10 user slots + 19 assistant records at minimum"
    );
    assert_eq!(p.trajectory.steps[0].source, StepSource::User);
    let raw = &p.trajectory.steps[0].extra.as_ref().unwrap().goalcup.raw;
    assert_eq!(raw.len(), 1);
    assert_eq!(raw[0].store, "main");
    assert_eq!(raw[0].sha256.len(), 64);
    assert_eq!(p.trajectory.subagent_trajectories.len(), 1);
    assert_eq!(
        p.trajectory.subagent_trajectories[0].steps.len(),
        3,
        "1 text-bearing user record + 2 assistant records: the parser pushes user steps \
         into the subagent trajectory too (fidelity M5)"
    );
    assert_eq!(
        p.trajectory.subagent_trajectories[0]
            .trajectory_id
            .as_deref(),
        Some("a8ca2ea8d4a8a1673")
    );
}

#[test]
fn records_models_tokens_sessions_and_resume_markers() {
    let p = parsed();
    let models: Vec<&str> = p.models.iter().map(|m| m.model.as_str()).collect();
    assert!(models.contains(&"claude-opus-5"));
    assert!(models.contains(&"claude-sonnet-5"));
    assert!(
        models.contains(&"<synthetic>"),
        "the resume record's model made no API call"
    );

    assert!(!p.tokens.is_empty());
    assert!(p.tokens.iter().all(|t| t.prompt > 0 || t.completion > 0));

    // Two sessions live under the same CLAUDE_CONFIG_DIR: ours and the `a9-other-folder` probe.
    assert_eq!(p.sessions.len(), 2);
    let tracked: Vec<_> = p.sessions.iter().filter(|s| s.tracked).collect();
    assert_eq!(tracked.len(), 1);
    // The directory comes from the first record that has a
    // `cwd` (index 5), not from record 0 (`{"type":"mode",…}`, which has none). Without
    // this the tracked session's directory is `None` and V10 emits a false `Fail`.
    assert_eq!(
        tracked[0].directory.as_deref(),
        Some("/Users/team/goalcup-spikes/ws-claude-code-cli-20260903T073054Z"),
        "the tracked session's directory must be the workspace, not None"
    );
    let foreign = p.sessions.iter().find(|s| !s.tracked).unwrap();
    assert_eq!(foreign.store, "a9-other-folder");

    // One resume, deduped from two SessionStart posts.
    assert_eq!(p.resume_markers, vec!["resume".to_string()]);
}

#[test]
fn raw_bodies_are_hash_linked_to_the_step_and_classified() {
    // The descriptor names four bodies and they are read, so V1's "raw files hash-linked
    // from `extra`" holds.
    let p = parsed();
    assert_eq!(p.raw_bodies.len(), 4);
    assert!(
        p.raw_bodies
            .iter()
            .all(|b| b.sha256.len() == 64 && b.bytes > 0)
    );

    let response = p
        .raw_bodies
        .iter()
        .find(|b| b.name == "goal-turn.response")
        .unwrap();
    let step_id = response.linked_step_id.expect(
        "the response body's own `id` field joins its step through the step's `message.id`",
    );
    let step = p
        .trajectory
        .steps
        .iter()
        .find(|s| s.step_id == step_id)
        .unwrap();
    assert_eq!(step.source, StepSource::Agent);
    assert!(
        step.extra
            .as_ref()
            .unwrap()
            .goalcup
            .raw
            .iter()
            .any(|l| l.store == "raw_bodies" && l.sha256 == response.sha256),
        "the step must carry a RawLink to the body's hash"
    );
    assert!(
        !response.requires_paired_human,
        "a response body is not a turn in the user slot"
    );

    // The goal turn's request body's trailing non-system message is the typed prompt (the
    // literal last array element is a synthetic `role: "system"` reminder every captured
    // request carries), so it is a new user-slot turn and a human message must pair with it.
    let request = p
        .raw_bodies
        .iter()
        .find(|b| b.name == "goal-turn.request")
        .unwrap();
    assert!(
        request.requires_paired_human,
        "the trailing non-system message is role=user with a text block and no tool_result, \
         so it is a new turn"
    );

    // The orphaned killed-turn request has no assistant record to link to: the kill
    // landed between the request and the response. It is still hashed.
    let orphan = p
        .raw_bodies
        .iter()
        .find(|b| b.name == "killed-turn-orphan.request")
        .unwrap();
    assert_eq!(orphan.sha256.len(), 64);
}

#[test]
fn a_subagent_joinable_raw_body_never_lands_on_the_main_trajectory() {
    // `Trajectory::push` numbers steps from 1 *per
    // trajectory*, so the main trajectory's step_id 2 and the subagent trajectory's
    // step_id 2 are unrelated steps that merely share a number. A raw body that only
    // joins a subagent step (through that step's `message.id`) must be attached to the
    // subagent trajectory's step_id 2, never to the main trajectory's same-numbered step.
    //
    // The subagent's first assistant reply (`subagent-a8ca2ea8d4a8a1673.jsonl` record
    // index 7, subagent step_id 2) carries `requestId: "req_011Ceg8ViefT1epa2LXJgemr"`,
    // `message.id: "msg_011Ceg8VjXFq6ewqUmY8iXkQ"` — distinct from the main trajectory's
    // own step_id 2 (the "goal turn" response, `message.id:
    // "msg_011Ceg8Tvu4RRy7ZrUGaa9UY"`). A synthetic raw body carrying the subagent's
    // `message.id` as its own `id` exercises exactly the collision.
    //
    // Built from a fresh, temporary trace directory (never touching the read-only
    // `spikes/` fixtures) that points at the same real leg1/leg2/manifest files as
    // `tests/traces/claude-code-cli/trace.toml`, plus one extra `[[raw]]` entry for the
    // synthetic body.
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures = repo_root.join("spikes/claude-code-cli/fixtures");
    let source = tempfile::tempdir().unwrap();
    let raw_path = source.path().join("rawbody-subagent-reply.response.json");
    std::fs::write(&raw_path, r#"{"id":"msg_011Ceg8VjXFq6ewqUmY8iXkQ"}"#).unwrap();
    // Keep the original regression's one-raw roster and non-path identity fields. The
    // shared test-only materializer copies these trusted bytes into a contained root.
    let toml = format!(
        r#"connector = "claude-code-cli"
entry_id = "regression-subagent-step-id-collision"
workspace = "/Users/team/goalcup-spikes/ws-claude-code-cli-20260903T073054Z"
tracked_sessions = ["95addb64-9dac-4046-8ef6-8aa53fa0c7f1"]
harness_version = "2.1.259"
connector_manifest = "{manifest}"

[[leg1]]
name = "main"
kind = "raw_jsonl"
path = "{main}"

[[leg1]]
name = "subagent"
kind = "raw_jsonl"
path = "{subagent}"

[leg2]
path = "{hooks}"

[[raw]]
name = "subagent-reply.response"
path = "{raw}"
"#,
        manifest = repo_root
            .join("connectors/claude-code-cli/manifest.toml")
            .display(),
        main = fixtures.join("main-session.jsonl").display(),
        subagent = fixtures.join("subagent-a8ca2ea8d4a8a1673.jsonl").display(),
        hooks = fixtures.join("hooks.jsonl").display(),
        raw = raw_path.display(),
    );
    std::fs::write(source.path().join("trace.toml"), toml).unwrap();
    let tmp = fixture_support::materialize_trace(source.path());

    let trace = TraceDescriptor::load(tmp.path()).unwrap();
    assert_eq!(trace.entry_id, "regression-subagent-step-id-collision");
    assert_eq!(
        trace.raw.len(),
        1,
        "the original custom raw roster is retained"
    );
    let manifest = ConnectorManifest::from_toml_str(&trace.manifest_text().unwrap()).unwrap();
    let leg1: Vec<Leg1Store> = trace
        .leg1
        .iter()
        .map(|s| trace.load_leg1(s).unwrap())
        .collect();
    let leg2 = trace.load_leg2(trace.leg2.as_ref().unwrap()).unwrap();
    let p = parser_for_family(&manifest.family)
        .unwrap()
        .parse(&trace, &manifest, &leg1, &leg2)
        .unwrap();

    let fact = p
        .raw_bodies
        .iter()
        .find(|b| b.name == "subagent-reply.response")
        .unwrap();
    let step_id = fact
        .linked_step_id
        .expect("joins the subagent step through message.id");
    assert_eq!(
        step_id, 2,
        "the subagent's first assistant reply is subagent step_id 2"
    );

    let main_step = p
        .trajectory
        .steps
        .iter()
        .find(|s| s.step_id == step_id)
        .unwrap();
    assert!(
        !main_step
            .extra
            .as_ref()
            .unwrap()
            .goalcup
            .raw
            .iter()
            .any(|l| l.sha256 == fact.sha256),
        "a body that only joins a subagent step must never attach to the main trajectory's \
         same-numbered step"
    );

    let sub_step = p.trajectory.subagent_trajectories[0]
        .steps
        .iter()
        .find(|s| s.step_id == step_id)
        .unwrap();
    assert!(
        sub_step
            .extra
            .as_ref()
            .unwrap()
            .goalcup
            .raw
            .iter()
            .any(|l| l.sha256 == fact.sha256),
        "the body must be attached to the subagent trajectory's step whose message.id it matches"
    );
}

/// Reorder the two `[[leg1]]` lines and nothing may change. Were the subagent store walked
/// first, its `thread_source: "subagent"` would be recorded for the session id it shares
/// with the main transcript — `seen_sessions.insert` keeps whichever arrival comes first —
/// and the eight human messages would become zero.
///
/// The claim is stated over the whole verdict, not over the human list: a second
/// field the order reaches is V5's "models observed, in order of first appearance", which
/// would read the subagent's model as the run's first whenever the subagent store was
/// declared first. Asserting the human count alone would leave that one standing.
#[test]
fn session_facts_do_not_depend_on_leg1_declaration_order() {
    let fixture = fixture_support::materialize_fixture("tests/traces/claude-code-cli");
    let normal = goalcup_verifier::verify_trace(fixture.path()).unwrap();
    let swapped =
        goalcup_verifier::verify_trace(support::claude_trace_with_stores_reversed().path())
            .unwrap();
    assert_eq!(swapped.human_messages.len(), normal.human_messages.len());
    // Eight: seven is the codex fixture's count, and
    // `tests/traces/claude-code-cli/expected-verdict.json` says eight.
    assert_eq!(swapped.human_messages.len(), 8);
    assert_eq!(
        swapped
            .human_messages
            .iter()
            .map(|m| m.text.clone())
            .collect::<Vec<_>>(),
        normal
            .human_messages
            .iter()
            .map(|m| m.text.clone())
            .collect::<Vec<_>>(),
    );

    // Not one field of the verdict differs.
    assert_eq!(swapped.to_json().unwrap(), normal.to_json().unwrap());

    // The session facts themselves, not only what the human rule made of them.
    let p_normal = support::parsed_m0("claude-code-cli");
    let p_swapped = support::parsed(&support::claude_trace_with_stores_reversed());
    let facts = |p: &goalcup_verifier::parsers::ParsedLegs| {
        let mut rows: Vec<(String, String, String, bool, bool, Option<String>)> = p
            .sessions
            .iter()
            .map(|s| {
                (
                    s.id.clone(),
                    s.store.clone(),
                    s.thread_source.clone(),
                    s.is_root,
                    s.tracked,
                    s.directory.clone(),
                )
            })
            .collect();
        rows.sort();
        rows
    };
    assert_eq!(facts(&p_swapped), facts(&p_normal));
    let tracked = p_swapped.sessions.iter().find(|s| s.tracked).unwrap();
    assert_eq!(
        tracked.thread_source, "user",
        "the shared session id is the user's, whichever file the loop met first"
    );
    assert!(tracked.is_root);
}

/// Two subagents, neither store named `subagent`, must be two trajectories and must
/// not enter the human list. The manifest's real store glob is
/// `…/{session_id}/subagents/agent-*.jsonl`, so the store name the daemon lands on is
/// `agent-<id>` and never the literal `subagent`.
#[test]
fn subagents_are_detected_by_the_record_and_keyed_by_session_and_agent() {
    let t = support::claude_trace_with_two_agent_named_subagent_stores();
    let v = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert_eq!(
        v.human_messages.len(),
        8,
        "no sidechain record reaches the human list, and none of the eight real ones is lost"
    );

    let p = support::parsed(&t);
    assert_eq!(p.trajectory.subagent_trajectories.len(), 2);
    let ids: Vec<Option<String>> = p
        .trajectory
        .subagent_trajectories
        .iter()
        .map(|s| s.trajectory_id.clone())
        .collect();
    assert_eq!(
        ids.iter().filter(|i| i.is_some()).count(),
        2,
        "each carries its own agentId"
    );
    assert_ne!(
        ids[0], ids[1],
        "two subagents are two keys, not one collision"
    );

    // The record's own answer travels with the user slot, so the scoping clause can read
    // it without depending on the one manifest exclusion line that also catches it.
    for u in p
        .user_slots
        .iter()
        .filter(|u| u.store.starts_with("agent-"))
    {
        assert_eq!(
            u.field("thread_source"),
            Some(&FieldValue::Str("subagent".into())),
            "a sidechain record must never sit in the user slot as thread_source `user`: {:?}",
            u.text
        );
    }
    assert!(
        p.user_slots.iter().any(|u| u.store.starts_with("agent-")),
        "the synthetic stores contributed user slots at all"
    );
}

#[test]
fn desktop_fixture_preserves_human_provenance_raw_links_and_the_real_subagent() {
    let t = support::claude_desktop_trace();
    let p = support::parsed(&t);

    let goal = p
        .user_slots
        .iter()
        .find(|u| {
            u.text
                == "create hello.py that prints the current date, run it, then tell me the output"
        })
        .expect("the measured Desktop goal is a user slot");
    assert_eq!(
        goal.field("origin.kind"),
        Some(&FieldValue::Str("human".into()))
    );
    assert_eq!(
        goal.field("promptSource"),
        Some(&FieldValue::Str("sdk".into()))
    );
    assert_eq!(
        goal.pair_key.as_deref(),
        Some("94fefa08-a2bc-4658-b62a-0240d9d602d3")
    );

    let meta: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(t.path().join("subagent-ac4cb0667aadfa93a.meta.json"))
            .expect("the copied subagent metadata is readable"),
    )
    .expect("the copied subagent metadata is JSON");
    let parent_call_id = meta
        .get("toolUseId")
        .and_then(|value| value.as_str())
        .expect("the measured child metadata names its parent Agent call");
    assert_eq!(parent_call_id, "toolu_011qJjvtDXwTaznKz9vasodY");
    assert!(
        p.tool_calls.iter().any(|call| {
            call.leg == "leg1" && call.name == "Agent" && call.call_id == parent_call_id
        }),
        "the metadata's parent tool-use id is a parsed root Agent call"
    );
    let hooks_file = std::fs::File::open(t.path().join("hooks.jsonl")).unwrap();
    let hooks = goalcup_verifier::legs::Leg2Stream::from_reader("hooks.jsonl", hooks_file)
        .expect("the copied Desktop hooks load");
    assert_eq!(
        hooks
            .by_event("SubagentStop")
            .iter()
            .filter(|post| post.body.get("agent_type").and_then(|value| value.as_str()) == Some(""))
            .count(),
        5,
        "all five measured suggestion-only SubagentStop hooks are present"
    );
    assert_eq!(p.trajectory.subagent_trajectories.len(), 1);
    assert_eq!(
        p.trajectory.subagent_trajectories[0]
            .trajectory_id
            .as_deref(),
        Some("ac4cb0667aadfa93a")
    );
    assert_eq!(
        p.raw_bodies.len(),
        3,
        "both measured raw bodies and the subagent metadata are hash-read"
    );
    assert!(p.raw_bodies.iter().all(|body| body.sha256.len() == 64));
    assert!(
        p.raw_bodies
            .iter()
            .any(|body| body.linked_step_id.is_some()),
        "a native id match links a raw body to its trajectory step"
    );
    assert!(
        p.resume_markers.is_empty(),
        "the absent Desktop project HTTP SessionStart never manufactures a resume marker"
    );
}

/// A `mod support` inside the test file, and every generated trace is written into a
/// tempdir — `spikes/` is a read-only input and is only ever copied out of.
mod support {
    use goalcup_protocol::manifest::ConnectorManifest;
    use goalcup_verifier::legs::Leg1Store;
    use goalcup_verifier::parsers::{ParsedLegs, parser_for_family};
    use goalcup_verifier::trace::TraceDescriptor;
    use std::path::{Path, PathBuf};

    /// The workspace root. Every path inside the M0 descriptors is written relative to the
    /// trace directory as `../../../../../…`, which is this directory.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("the workspace root exists")
    }

    /// The committed M0 trace directory for a connector.
    pub fn m0_trace(id: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/traces")
            .join(id)
    }

    /// One `[[leg1]]` table, exactly as the M0 claude descriptor writes it.
    fn leg1_table(name: &str, path: &Path) -> String {
        format!(
            "[[leg1]]\nname = \"{name}\"\nkind = \"raw_jsonl\"\npath = \"{}\"\n",
            path.display()
        )
    }

    fn copy(source: &Path, destination: &Path) {
        std::fs::copy(source, destination).expect("fixture copy succeeds");
    }

    /// A self-contained legacy descriptor over the redacted Claude Desktop files. The
    /// descriptor deliberately retains the CLI connector id: this tests shared record
    /// shapes before a shipping Desktop connector is registered.
    pub fn claude_desktop_trace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let desktop = repo_root().join("spikes/claude-desktop/fixtures");
        for name in [
            "main-session.jsonl",
            "subagent-ac4cb0667aadfa93a.jsonl",
            "hooks.jsonl",
            "rawbody-goal-turn.request.json",
            "rawbody-goal-turn.response.json",
            "subagent-ac4cb0667aadfa93a.meta.json",
        ] {
            copy(&desktop.join(name), &dir.path().join(name));
        }
        copy(
            &repo_root().join("connectors/claude-code-cli/manifest.toml"),
            &dir.path().join("manifest.toml"),
        );
        std::fs::write(
            dir.path().join("trace.toml"),
            r#"connector = "claude-code-cli"
entry_id = "claude-desktop-parser-shape"
workspace = "/Users/team/dev/PROJECTS/AgentCommunity/goal-cup/spikes/claude-desktop/out/ws-20260903T041927Z/ws"
tracked_sessions = ["6abc89a2-bf72-4e5f-b5c6-be093c9930b4"]
harness_version = "2.1.258"
connector_manifest = "manifest.toml"

[[leg1]]
name = "main"
kind = "raw_jsonl"
path = "main-session.jsonl"

[[leg1]]
name = "subagent"
kind = "raw_jsonl"
path = "subagent-ac4cb0667aadfa93a.jsonl"

[leg2]
path = "hooks.jsonl"

[[raw]]
name = "goal-turn.request"
path = "rawbody-goal-turn.request.json"

[[raw]]
name = "goal-turn.response"
path = "rawbody-goal-turn.response.json"

[[raw]]
name = "subagent.meta"
path = "subagent-ac4cb0667aadfa93a.meta.json"
"#,
        )
        .expect("desktop descriptor is written");
        dir
    }

    /// Copy the M0 claude trace into a tempdir and rewrite `trace.toml` with its `main` and
    /// `subagent` `[[leg1]]` tables in the opposite order. The store files are not copied
    /// at all — the two declarations point at the same committed bytes; only the
    /// declaration order moves.
    pub fn claude_trace_with_stores_reversed() -> tempfile::TempDir {
        let dir = super::fixture_support::materialize_fixture("tests/traces/claude-code-cli");
        let text = std::fs::read_to_string(dir.path().join("trace.toml")).unwrap();
        let main = leg1_table("main", Path::new("main-session.jsonl"));
        let subagent = leg1_table("subagent", Path::new("subagent-a8ca2ea8d4a8a1673.jsonl"));
        assert!(
            text.contains(&main) && text.contains(&subagent),
            "the M0 claude descriptor still declares `main` then `subagent`"
        );
        let swapped = text
            .replace(&main, "\u{0}MAIN\u{0}")
            .replace(&subagent, &main)
            .replace("\u{0}MAIN\u{0}", &subagent);
        assert_ne!(swapped, text, "the two tables actually moved");
        std::fs::write(dir.path().join("trace.toml"), swapped).unwrap();
        dir
    }

    /// Copy the M0 claude trace into a tempdir and replace its one `subagent` store with
    /// two synthetic ones, `agent-a.jsonl` and `agent-b.jsonl`, each a copy of the
    /// committed subagent fixture with every record's `agentId` rewritten to `agent-a` /
    /// `agent-b`. Neither store is named `subagent`, which is the whole point: the record
    /// decides, not the name. It replaces rather than adds so that the two trajectories
    /// the test counts are exactly the two synthetic agents.
    pub fn claude_trace_with_two_agent_named_subagent_stores() -> tempfile::TempDir {
        let dir = super::fixture_support::materialize_fixture("tests/traces/claude-code-cli");
        let source =
            std::fs::read_to_string(dir.path().join("subagent-a8ca2ea8d4a8a1673.jsonl")).unwrap();

        let mut tables = String::new();
        for agent in ["agent-a", "agent-b"] {
            let mut out = String::new();
            for line in source.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                let mut row: serde_json::Value = serde_json::from_str(line).unwrap();
                if row.get("agentId").is_some() {
                    row["agentId"] = serde_json::Value::String(agent.to_string());
                }
                out.push_str(&serde_json::to_string(&row).unwrap());
                out.push('\n');
            }
            let path = dir.path().join(format!("{agent}.jsonl"));
            std::fs::write(&path, out).unwrap();
            tables.push_str(&leg1_table(agent, Path::new(&format!("{agent}.jsonl"))));
            tables.push('\n');
        }

        let text = std::fs::read_to_string(dir.path().join("trace.toml")).unwrap();
        let old = leg1_table("subagent", Path::new("subagent-a8ca2ea8d4a8a1673.jsonl"));
        assert!(text.contains(&old), "the M0 descriptor declares `subagent`");
        std::fs::write(
            dir.path().join("trace.toml"),
            text.replace(&old, tables.trim_end_matches('\n')),
        )
        .unwrap();
        dir
    }

    /// The `ParsedLegs` for a generated trace directory.
    pub fn parsed(t: &tempfile::TempDir) -> ParsedLegs {
        parse_dir(t.path())
    }

    /// The same, for the committed M0 trace directory: the control every generated trace
    /// is compared against.
    pub fn parsed_m0(id: &str) -> ParsedLegs {
        let fixture = super::fixture_support::materialize_trace(&m0_trace(id));
        parse_dir(fixture.path())
    }

    fn parse_dir(dir: &Path) -> ParsedLegs {
        let trace = TraceDescriptor::load(dir).unwrap();
        let manifest = ConnectorManifest::from_toml_str(&trace.manifest_text().unwrap()).unwrap();
        let leg1: Vec<Leg1Store> = trace
            .leg1
            .iter()
            .map(|s| trace.load_leg1(s).unwrap())
            .collect();
        let leg2 = trace.load_leg2(trace.leg2.as_ref().unwrap()).unwrap();
        parser_for_family(&manifest.family)
            .unwrap()
            .parse(&trace, &manifest, &leg1, &leg2)
            .unwrap()
    }
}
