#[path = "support/mod.rs"]
mod fixture_support;

use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_verifier::atif::StepSource;
use goalcup_verifier::legs::Leg1Store;
use goalcup_verifier::parsers::{FieldValue, ParsedLegs, parser_for_family};
use goalcup_verifier::trace::TraceDescriptor;
use std::path::PathBuf;
use std::process::Command;

fn parsed() -> ParsedLegs {
    let fixture = fixture_support::materialize_fixture("tests/traces/codex-cli");
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
fn user_slots_come_only_from_the_tracked_thread() {
    let p = parsed();
    let texts: Vec<&str> = p
        .user_slots
        .iter()
        .filter(|u| u.store == "rollout-main")
        .map(|u| u.text.as_str())
        .collect();
    assert_eq!(texts.len(), 8, "7 typed plus one <environment_context>");
    assert!(texts[0].starts_with("<environment_context>"));
    assert_eq!(
        &texts[1..],
        &[
            "create hello.py that prints the current date, run it, then tell me the output",
            "use a subagent to count the lines in hello.py",
            "going",
            "keep going",
            "keep going",
            "what is 2 + 2?",
            "also add a README",
        ]
    );
    // The subagent rollout replays the parent's turns; they are collected but marked.
    let sub: Vec<&goalcup_verifier::parsers::UserSlotRecord> = p
        .user_slots
        .iter()
        .filter(|u| u.store == "rollout-subagent")
        .collect();
    assert_eq!(
        sub.len(),
        3,
        "the replay: env context, the goal, the subagent request"
    );
    assert_eq!(
        sub[0].field("thread_source"),
        Some(&FieldValue::Str("subagent".into())),
        "every record in that file inherits the file's FIRST session_meta"
    );
}

#[test]
fn turn_order_join_assigns_the_right_turn_id() {
    let p = parsed();
    let main: Vec<_> = p
        .user_slots
        .iter()
        .filter(|u| u.store == "rollout-main")
        .collect();
    let goal = main
        .iter()
        .find(|u| u.text.starts_with("create hello.py"))
        .unwrap();
    assert_eq!(goal.ordinal, Some(8));
    assert_eq!(
        goal.pair_key.as_deref(),
        Some("01a06614-bd07-7892-abce-d9315bbd4511")
    );
    let readme = main.iter().find(|u| u.text == "also add a README").unwrap();
    assert_eq!(readme.ordinal, Some(131));
    assert_eq!(
        readme.pair_key.as_deref(),
        Some("01a0661c-e24a-77e3-ad10-68245a7dd2b4")
    );

    // The blocked prompt: leg 2 has a turn_id with no leg-1 user record.
    let leg2_keys: Vec<&str> = p
        .leg2_candidates
        .iter()
        .filter_map(|c| c.pair_key.as_deref())
        .collect();
    assert!(leg2_keys.contains(&"01a0661c-c680-7353-92d2-ae7125cad461"));
    assert!(
        !main
            .iter()
            .any(|u| u.pair_key.as_deref() == Some("01a0661c-c680-7353-92d2-ae7125cad461")),
        "the block kept it out of the rollout"
    );
    assert_eq!(p.leg2_candidates.len(), 8);
}

#[test]
fn sessions_subagent_link_models_and_tokens() {
    let p = parsed();
    let tracked: Vec<_> = p.sessions.iter().filter(|s| s.tracked).collect();
    assert_eq!(tracked.len(), 1);
    assert_eq!(tracked[0].id, "01a06614-8f11-79a1-abb9-0b0f45b7f104");
    assert_eq!(tracked[0].thread_source, "user");

    let sub = p
        .sessions
        .iter()
        .find(|s| s.id == "01a06615-7f69-7292-b401-be54c2817f5e")
        .expect("the subagent thread is a session of its own");
    assert_eq!(
        sub.parent.as_deref(),
        Some("01a06614-8f11-79a1-abb9-0b0f45b7f104")
    );
    assert_eq!(sub.thread_source, "subagent");
    assert!(!sub.is_root);

    // The mis-launched thread does not appear as a session, so it cannot become a new root,
    // and the fixture is why: `wrong-cwd-01a06616-excerpt.jsonl` is two `response_item`
    // records with no `session_meta`, so the file has no key and the parser's first guard
    // skips it. The thread is still reported — as a declared-foreign store, by V4's
    // `foreign_session` — but not as a second root thread, because nothing in the bytes says
    // it is one.
    assert!(
        !p.sessions.iter().any(|s| s.id.starts_with("01a06616")),
        "a file with no session_meta yields no SessionFact; the id is only in the filename"
    );
    assert!(
        p.sessions.iter().all(|s| s.store != "wrong-cwd-01a06616"),
        "the store is loaded and chained as evidence, but attributed to no session"
    );

    let models: Vec<&str> = p.models.iter().map(|m| m.model.as_str()).collect();
    assert!(models.contains(&"gpt-5.6-sol"));
    assert!(models.contains(&"gpt-5.6-luna"));

    assert!(!p.tokens.is_empty());
    assert!(
        p.tokens
            .iter()
            .all(|t| t.stripped_fields.contains(&"rate_limits".to_string())),
        "V11/D26: plan, credit and quota fields are stripped before publication"
    );

    assert_eq!(p.resume_markers, vec!["resume".to_string()]);

    // `history.jsonl` is a third witness and the parser reads it. Without
    // it, `blocked_prompts` is empty on Codex and V3 can never say a block occurred.
    assert_eq!(
        p.blocked_prompts.len(),
        1,
        "the 4,100-character paste, retained in history"
    );
    assert!(
        p.blocked_prompts[0].contains("4100"),
        "got {:?}",
        p.blocked_prompts
    );
}

#[test]
fn tool_ids_are_recorded_per_leg_because_the_join_cannot_rely_on_them() {
    // The `exec` family is disjoint;
    // the `collaboration*` tools share their `call_` ids across both legs. So the id
    // spaces overlap partially, which is a stronger argument for the turn_id+order join
    // than full disjointness would be — and it means an `is_disjoint` assertion is simply
    // false here.
    let p = parsed();
    let leg1: Vec<&str> = p
        .tool_calls
        .iter()
        .filter(|t| t.leg == "leg1")
        .map(|t| t.call_id.as_str())
        .collect();
    let leg2: Vec<&str> = p
        .tool_calls
        .iter()
        .filter(|t| t.leg == "leg2")
        .map(|t| t.call_id.as_str())
        .collect();
    assert!(!leg1.is_empty() && !leg2.is_empty());
    assert!(
        leg1.iter().all(|id| id.starts_with("call_")),
        "leg 1 stamps call_<id>"
    );
    assert!(
        leg2.iter().any(|id| id.starts_with("exec-")),
        "leg 2 stamps exec-<uuid>"
    );
    assert!(
        !leg2
            .iter()
            .any(|id| id.starts_with("exec-") && leg1.contains(id)),
        "no exec-<uuid> ever appears on leg 1: that family's id spaces are disjoint"
    );

    // The two ids that do appear on both legs, named so a future change is deliberate.
    for shared in [
        "call_FFBna0WHdpjVD98X321CRmft",
        "call_jlSnFwWNLyjkVsMhs93Sh0Fv",
    ] {
        assert!(
            leg1.contains(&shared),
            "{shared} is a leg-1 function_call.call_id"
        );
        assert!(
            leg2.contains(&shared),
            "{shared} is also a leg-2 PreToolUse.tool_use_id"
        );
    }
    let shared_count = leg1.iter().filter(|a| leg2.contains(a)).count();
    assert_eq!(
        shared_count, 2,
        "the two collaboration* calls, and only those"
    );

    // What the join actually rests on: every leg-1 call carries a turn key.
    assert!(
        p.tool_calls
            .iter()
            .filter(|t| t.leg == "leg1")
            .all(|t| t.turn_key.is_some())
    );
}

/// `total_token_usage` is cumulative (and resets when the thread rolls over), so summing it
/// overcounts. `payload.info.last_token_usage` is the per-turn delta and is what V11 sums.
#[test]
fn token_totals_sum_the_per_turn_delta_not_the_running_total() {
    let fixture = fixture_support::materialize_fixture("tests/traces/codex-cli");
    let v = goalcup_verifier::verify_trace(fixture.path()).unwrap();
    let prompt: u64 = v.tokens.by_model.iter().map(|t| t.prompt).sum();
    let completion: u64 = v.tokens.by_model.iter().map(|t| t.completion).sum();
    let cached: u64 = v.tokens.by_model.iter().map(|t| t.cached).sum();
    assert_eq!(
        prompt, 412_213,
        "sum of last_token_usage.input_tokens across both rollouts"
    );
    assert_eq!(completion, 1_737);
    assert_eq!(cached, 344_320);
    assert_ne!(
        prompt, 1_646_913,
        "the cumulative counter is not a per-turn number"
    );
    // Two models, both named, so the split is asserted and not only the total.
    let names: Vec<&str> = v.tokens.by_model.iter().map(|t| t.model.as_str()).collect();
    assert!(
        names.contains(&"gpt-5.6-sol") && names.contains(&"gpt-5.6-luna"),
        "{names:?}"
    );
}

#[test]
fn desktop_main_fixture_keeps_newline_and_marks_replayed_subagent_context() {
    let t = support::codex_desktop_main_trace();
    let p = support::parsed(&t);

    let goal = p
        .user_slots
        .iter()
        .find(|u| u.text.starts_with("create hello.py"))
        .expect("the Desktop goal is parsed from the root rollout");
    assert_eq!(
        goal.text,
        "create hello.py that prints the current date, run it, then tell me the output\n"
    );
    assert_eq!(
        goal.field("thread_source"),
        Some(&FieldValue::Str("user".into()))
    );
    assert!(
        p.user_slots
            .iter()
            .any(|u| u.text.starts_with("<recommended_plugins>")),
        "the concatenated recommendation/environment block stays available for the human rule"
    );
    assert!(
        p.leg2_candidates.is_empty(),
        "the historical main run has no hook witness"
    );
    assert!(
        p.sessions.iter().any(|session| {
            !session.tracked
                && session.directory.as_deref()
                    == Some("<goalcup-scrubbed len=34 private project path>")
        }),
        "the copied foreign rollout remains outside the tracked Desktop session"
    );

    let sub = p
        .trajectory
        .subagent_trajectories
        .iter()
        .find(|trajectory| {
            trajectory.trajectory_id.as_deref() == Some("01a06658-688f-7790-8124-1635bbceddc8")
        })
        .expect("the known Desktop child is separate from the root trajectory");
    let child_session = p
        .sessions
        .iter()
        .find(|session| session.id == "01a06658-688f-7790-8124-1635bbceddc8")
        .expect("the child session fact is parsed");
    assert_eq!(
        child_session.parent.as_deref(),
        Some("01a06657-c53d-7201-afa1-cefdaac2edf5")
    );
    let selected_main = p
        .sessions
        .iter()
        .find(|session| session.tracked)
        .expect("the selected main trace session is parsed");
    assert_eq!(
        child_session.parent.as_deref(),
        Some(selected_main.id.as_str()),
        "the committed Desktop bytes name the selected main session as the child's parent"
    );
    let replayed_users: Vec<_> = sub
        .steps
        .iter()
        .filter(|step| step.source == StepSource::User)
        .collect();
    assert_eq!(
        replayed_users.len(),
        3,
        "the child has exactly three replayed parent user steps"
    );
    assert!(
        replayed_users
            .iter()
            .all(|step| step.is_copied_context == Some(true)),
        "the replayed parent user context never becomes a fresh child turn"
    );
}

#[test]
fn desktop_probe_keeps_a_hook_blocked_prompt_out_of_the_rollout() {
    let t = support::codex_desktop_probe_trace();
    let p = support::parsed(&t);

    assert_eq!(
        p.leg2_candidates.len(),
        3,
        "three UserPromptSubmit posts were observed"
    );
    let blocked = p
        .leg2_candidates
        .iter()
        .find(|candidate| candidate.text.len() == 4_101)
        .expect("the app's 4,101-character paste plus one newline reaches leg 2");
    assert_eq!(
        blocked.pair_key.as_deref(),
        Some("01a06670-66dc-7fb0-a900-d7e1af6eaebc")
    );
    for accepted in [
        "what is 2 + 2?\n",
        "create hello2.py that prints hi and run it\n",
    ] {
        assert!(
            p.user_slots.iter().any(|slot| slot.text == accepted),
            "accepted short prompt remains in the rollout: {accepted:?}"
        );
    }
    assert!(
        !p.user_slots.iter().any(|slot| slot.text == blocked.text),
        "a prompt blocked by the hook never becomes a rollout user slot"
    );
    assert!(
        !p.user_slots
            .iter()
            .any(|slot| slot.pair_key == blocked.pair_key),
        "no rollout user slot carries the blocked hook turn id"
    );
    assert_eq!(
        p.sessions.iter().filter(|session| session.tracked).count(),
        1,
        "the probe's first session_meta remains the sole tracked session"
    );
}

#[test]
fn codex_app_a9_fixture_generator_is_reproducible_and_scrubbed() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root exists");
    let generator = repo_root.join("spikes/codex-app/make_fixtures.py");
    assert!(
        generator.is_file(),
        "the fixture generator is part of the public cut"
    );
    let status = Command::new("python3")
        .arg(generator)
        .arg("--self-check")
        .current_dir(&repo_root)
        .status()
        .expect("the fixture generator self-check starts");
    assert!(status.success(), "the fixture generator self-check passes");
}

mod support {
    use goalcup_protocol::manifest::ConnectorManifest;
    use goalcup_verifier::legs::Leg1Store;
    use goalcup_verifier::parsers::{ParsedLegs, parser_for_family};
    use goalcup_verifier::trace::TraceDescriptor;
    use std::path::{Path, PathBuf};

    /// The committed M0 trace directory for a connector.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("the workspace root exists")
    }

    fn copy(source: &Path, destination: &Path) {
        std::fs::copy(source, destination).expect("fixture copy succeeds");
    }

    fn desktop_trace(
        name: &str,
        tracked_session: &str,
        leg1: &[&str],
        leg2: &str,
    ) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let fixtures = repo_root().join("spikes/codex-app/fixtures");
        for file in leg1.iter().copied().chain(std::iter::once(leg2)) {
            copy(&fixtures.join(file), &dir.path().join(file));
        }
        copy(
            &repo_root().join("connectors/codex-cli/manifest.toml"),
            &dir.path().join("manifest.toml"),
        );
        let leg1_tables = leg1
            .iter()
            .map(|file| {
                format!(
                    "[[leg1]]\nname = \"{}\"\nkind = \"raw_jsonl\"\npath = \"{file}\"\n",
                    file.trim_end_matches(".jsonl")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(
            dir.path().join("trace.toml"),
            format!(
                "connector = \"codex-cli\"\nentry_id = \"{name}\"\nworkspace = \"/Users/team/goalcup-spikes/ws-codex-app-20260903T081322Z\"\ntracked_sessions = [\"{tracked_session}\"]\nharness_version = \"0.151.0-alpha.7.1\"\nconnector_manifest = \"manifest.toml\"\n\n{leg1_tables}\n[leg2]\npath = \"{leg2}\"\n"
            ),
        )
        .expect("desktop descriptor is written");
        dir
    }

    pub fn codex_desktop_main_trace() -> tempfile::TempDir {
        desktop_trace(
            "codex-desktop-main-parser-shape",
            "01a06657-c53d-7201-afa1-cefdaac2edf5",
            &[
                "rollout-main-01a06657.jsonl",
                "rollout-subagent-01a06658.jsonl",
                "rollout-a9-foreign-01a0665e-excerpt.jsonl",
            ],
            "hooks.jsonl",
        )
    }

    pub fn codex_desktop_probe_trace() -> tempfile::TempDir {
        desktop_trace(
            "codex-desktop-probe-parser-shape",
            "01a0666d-87ec-7d12-b578-6d79f921376b",
            &["rollout-probe-01a0666d.jsonl"],
            "probe-hooks.jsonl",
        )
    }

    pub fn parsed(t: &tempfile::TempDir) -> ParsedLegs {
        let trace = TraceDescriptor::load(t.path()).expect("desktop trace loads");
        let manifest =
            ConnectorManifest::from_toml_str(&trace.manifest_text().expect("manifest is readable"))
                .expect("manifest is valid");
        let leg1: Vec<Leg1Store> = trace
            .leg1
            .iter()
            .map(|store| trace.load_leg1(store).expect("leg 1 fixture loads"))
            .collect();
        let leg2 = trace
            .load_leg2(trace.leg2.as_ref().unwrap())
            .expect("leg 2 fixture loads");
        parser_for_family(&manifest.family)
            .expect("family parser exists")
            .parse(&trace, &manifest, &leg1, &leg2)
            .expect("desktop shape parses")
    }
}
