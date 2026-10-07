mod support;
use goalcup_protocol::manifest::{ConnectorManifest, ScopingClause};
use goalcup_verifier::human_rule::{ExclusionReason, HumanRule, HumanRuleOutcome};
use goalcup_verifier::legs::Leg1Store;
use goalcup_verifier::parsers::parser_for_family;
use goalcup_verifier::trace::TraceDescriptor;

/// The human rule over one committed M0 trace, as the shipped pipeline applies it.
fn outcome(id: &str) -> HumanRuleOutcome {
    support::pipeline::m0(id).human().clone()
}

/// The same rule over a descriptor a test has modified in memory — which is not a
/// directory `Pipeline::load` could read, so this half of the loading stays hand-written.
fn outcome_of(trace: TraceDescriptor) -> HumanRuleOutcome {
    let manifest = ConnectorManifest::from_toml_str(&trace.manifest_text().unwrap()).unwrap();
    let leg1: Vec<Leg1Store> = trace
        .leg1
        .iter()
        .map(|s| trace.load_leg1(s).unwrap())
        .collect();
    let leg2 = trace.load_leg2(trace.leg2.as_ref().unwrap()).unwrap();
    let parsed = parser_for_family(&manifest.family)
        .unwrap()
        .parse(&trace, &manifest, &leg1, &leg2)
        .unwrap();
    let rule = HumanRule::new(
        manifest.human_rule.clone(),
        manifest.normalisation.clone(),
        &trace.workspace,
        &trace.tracked_sessions,
    );
    rule.apply(
        &parsed.user_slots,
        &parsed.leg2_candidates,
        &parsed.sessions,
    )
}

#[test]
fn claude_code_yields_exactly_the_eight_typed_messages() {
    let o = outcome("claude-code-cli");
    let texts: Vec<&str> = o.messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            "create hello.py that prints the current date, run it, then tell me the output",
            "use a subagent to count the lines in hello.py",
            "keep going",
            "write long.py with 300 numbered comment lines, then explain every section of it in detail in your reply",
            "keep going",
            "also add a README",
            "keep going",
            "what model are you?",
        ]
    );
    assert!(o.messages.iter().all(|m| m.leg1_sha256.len() == 64));
    assert_eq!(o.messages[0].index, 0);

    // These four harness-authored records all fail the connector's ordinary positive test
    // (`origin.kind` is not `human`); none is `ExclusionReason::Excluded` under a catalogue
    // id. They are excluded, and not human messages.
    let excluded: Vec<&str> = o
        .excluded
        .iter()
        .filter(|e| matches!(e.reason, ExclusionReason::FailedPositive))
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        excluded
            .iter()
            .any(|t| t.starts_with("<task-notification>"))
    );
    assert!(excluded.contains(&"Continue from where you left off."));
    assert_eq!(
        excluded.len(),
        4,
        "task-notification, continue-from-where-you-left-off, the subagent's sidechain \
         echo and the A9 probe's own record: {excluded:?}"
    );

    // Two posts have no leg-1 record, not one — and each is asserted by identity,
    // because a bare count would hide the second one.
    let unpaired: Vec<&str> = o
        .leg2_without_record
        .iter()
        .filter_map(|c| c.pair_key.as_deref())
        .collect();
    assert_eq!(o.leg2_without_record.len(), 2, "have {unpaired:?}");
    assert!(
        o.leg2_without_record
            .iter()
            .any(|c| c.text.starts_with("<task-notification>")
                && c.pair_key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("777250ec"))),
        "the <task-notification> post: excluded on leg 1, present on leg 2"
    );
    assert!(
        o.leg2_without_record
            .iter()
            .any(|c| c.text.starts_with("OVERLENGTH-TEST-A7")
                && c.pair_key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("2143f780"))),
        "the 4,100-character paste the block kept out of the transcript"
    );

    // The excluded record carries the key its orphaned post carries, so
    // a downstream check can join the two instead of matching them by eye on the text.
    let notification = o
        .excluded
        .iter()
        .find(|e| e.text.starts_with("<task-notification>"))
        .expect("the task-notification record is excluded");
    let notification_post = o
        .leg2_without_record
        .iter()
        .find(|c| c.text.starts_with("<task-notification>"))
        .expect("and its post is orphaned");
    assert_eq!(notification.pair_key, notification_post.pair_key);
    assert!(notification.pair_key.is_some());

    assert!(
        o.leg1_without_post.is_empty(),
        "every human record has a post"
    );
    assert!(
        o.unwitnessed.is_empty(),
        "MF-3: nothing was typed into an unhooked window"
    );
    assert!(
        o.duplicate_pairings.is_empty(),
        "MF-2: pairing is one-to-one"
    );
    assert!(o.divergent.is_empty());
    assert!(
        o.messages.iter().all(|m| m.leg2_index.is_some()),
        "all eight are witnessed"
    );
}

#[test]
fn codex_yields_seven_including_the_going_typo() {
    let o = outcome("codex-cli");
    let texts: Vec<&str> = o.messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            "create hello.py that prints the current date, run it, then tell me the output",
            "use a subagent to count the lines in hello.py",
            "going",
            "keep going",
            "keep going",
            "what is 2 + 2?",
            "also add a README",
        ]
    );
    // The main thread's own `<environment_context>` header is not `excluded`: Codex's empty
    // `positive` list has nothing to reject it on, and it passes every scoping clause too (it
    // genuinely is the tracked session's first record). It becomes the unwitnessed message at
    // index 0, which V1 reads as the goal.
    assert!(
        !o.excluded
            .iter()
            .any(|e| e.store != "rollout-subagent" && e.text.starts_with("<environment_context>")),
        "the interim reading: the main thread's header is no longer excluded at all"
    );
    assert_eq!(o.unwitnessed.len(), 1, "{:?}", o.unwitnessed);
    assert_eq!(o.unwitnessed[0].index, 0);
    assert!(o.unwitnessed[0].text.starts_with("<environment_context>"));

    // The subagent rollout's own copy of `<environment_context>` is a different record (store
    // `rollout-subagent`, not `main`) from the one above. It falls through to scoping like its
    // two replayed-parent-turn neighbours, so all three of the subagent rollout's `role=user`
    // records land in `FailedScoping`, not two.
    let scoped: Vec<&goalcup_verifier::human_rule::ExcludedRecord> = o
        .excluded
        .iter()
        .filter(|e| matches!(e.reason, ExclusionReason::FailedScoping(_)))
        .collect();
    assert_eq!(
        scoped.len(),
        3,
        "the subagent's own <environment_context> plus the two replayed parent turns: {scoped:?}"
    );
    assert!(scoped.iter().all(|e| e.store == "rollout-subagent"));
    assert!(
        scoped
            .iter()
            .any(|e| e.text.starts_with("<environment_context>")),
        "the subagent rollout's own header is scoped out too, just not by name any more"
    );

    // Every clause a record failed, not only the first. Codex's subagent records fail
    // `first_session_meta_is_tracked` and `not_subagent_thread_source`; reporting only the
    // first would leave a bucket nothing reached.
    let clauses = match &scoped[0].reason {
        ExclusionReason::FailedScoping(cs) => cs.clone(),
        other => panic!("expected FailedScoping, got {other:?}"),
    };
    assert!(clauses.contains(&ScopingClause::FirstSessionMetaIsTracked));
    assert!(clauses.contains(&ScopingClause::NotSubagentThreadSource));

    // Exactly one unpaired post on Codex.
    assert_eq!(o.leg2_without_record.len(), 1);
    assert!(
        o.leg2_without_record[0]
            .text
            .starts_with("OVERLENGTH-TEST-A7")
    );
    assert!(o.duplicate_pairings.is_empty());
}

#[test]
fn normalisation_strips_exactly_one_trailing_newline() {
    let fixture = support::materialize_fixture("tests/traces/codex-cli");
    let trace = TraceDescriptor::load(fixture.path()).unwrap();
    let manifest = ConnectorManifest::from_toml_str(&trace.manifest_text().unwrap()).unwrap();
    let rule = HumanRule::new(
        manifest.human_rule.clone(),
        manifest.normalisation.clone(),
        &trace.workspace,
        &trace.tracked_sessions,
    );
    assert_eq!(rule.normalise("keep going\n"), "keep going");
    assert_eq!(
        rule.normalise("keep going\n\n"),
        "keep going\n",
        "exactly one, never two"
    );
    assert_eq!(rule.normalise("keep going"), "keep going");
    assert_eq!(
        rule.normalise(" keep going "),
        " keep going ",
        "no trimming of spaces"
    );
}

/// The duplicate-pairing defence must be reachable through the real
/// pipeline. The parsers collapse the http hook's copy and the `-cmd` shim's copy of one
/// prompt into a single candidate — but a second post over a transport that already
/// carried the submission is a replay, and it reaches the human rule, which records it
/// without counting it twice.
///
/// The trace is the committed Claude Code one with a temp leg 2: the fixture's bytes with
/// one `/UserPromptSubmit` line repeated. `spikes/` is never written.
fn claude_outcome_with_http_replay(after_the_cmd_copy: bool) -> HumanRuleOutcome {
    let fixture = support::materialize_fixture("tests/traces/claude-code-cli");
    let trace = TraceDescriptor::load(fixture.path()).unwrap();

    let hooks = trace.leg2_text(trace.leg2.as_ref().unwrap()).unwrap();
    let mut lines: Vec<String> = Vec::new();
    // The replayed line is always an http one — the transport that already carried the
    // submission. A repost on a transport that has not carried it is the shim's copy, and
    // the parser is right to drop that.
    let mut http: Option<serde_json::Value> = None;
    let mut replayed = false;
    for line in hooks.lines() {
        let post: serde_json::Value = serde_json::from_str(line).unwrap();
        lines.push(line.to_string());
        if post["path"] == "/UserPromptSubmit" && http.is_none() {
            http = Some(post);
            if !after_the_cmd_copy {
                // http, http-replay, cmd.
                lines.push(line.to_string());
                replayed = true;
            }
        } else if after_the_cmd_copy
            && !replayed
            && post["path"] == "/UserPromptSubmit-cmd"
            && let Some(first) = &http
        {
            // http, cmd, http-replay — the realistic ordering of a retrying hook, and the
            // one a "some other transport already has it" test would have dropped.
            assert_eq!(
                post["body"]["prompt_id"], first["body"]["prompt_id"],
                "the -cmd line must be the shim's copy of the same submission"
            );
            lines.push(serde_json::to_string(first).unwrap());
            replayed = true;
        }
    }
    assert!(
        replayed,
        "the fixture has an http-transport UserPromptSubmit"
    );

    // A directory of this run's own, removed when `tmp` drops: a fixed name under the shared
    // temp directory would be shared by every concurrent run, each deleting the directory the
    // others were reading. The descriptor is handle-rooted, so mutate an owned contained
    // materialization rather than assigning an outside host path to a loaded descriptor.
    let tmp = support::materialize_fixture("tests/traces/claude-code-cli");
    std::fs::write(tmp.path().join("hooks.jsonl"), lines.join("\n") + "\n").unwrap();
    outcome_of(goalcup_verifier::trace::TraceDescriptor::load(tmp.path()).unwrap())
}

fn assert_one_replay_recorded_and_not_counted(o: &HumanRuleOutcome) {
    assert_eq!(
        o.duplicate_pairings.len(),
        1,
        "one replay, one duplicate pairing: {:?}",
        o.duplicate_pairings
    );
    let (paired, replay) = o.duplicate_pairings[0];
    assert!(replay > paired, "the replay is the later post");
    assert_eq!(o.messages.len(), 8, "the replay is not a ninth message");
    assert_eq!(
        o.messages[0].text,
        "create hello.py that prints the current date, run it, then tell me the output"
    );
    assert_eq!(
        o.leg2_without_record.len(),
        2,
        "the replay is consumed, not reported as a post with no record"
    );
    assert!(o.unwitnessed.is_empty());
    assert!(o.divergent.is_empty());
}

#[test]
fn a_post_replayed_on_the_same_transport_is_recorded_not_counted() {
    let o = claude_outcome_with_http_replay(false);
    assert_one_replay_recorded_and_not_counted(&o);
}

/// The same replay, arriving after the `-cmd` shim's copy — the ordering a retrying http
/// hook actually produces. Dropping a post because *some other* transport already carried
/// the submission would make this case vanish and `duplicate_pairings` unreachable; a
/// transport that has already carried a submission keeps every later post it carries.
#[test]
fn a_replay_arriving_after_the_other_transports_copy_is_still_recorded() {
    let o = claude_outcome_with_http_replay(true);
    assert_one_replay_recorded_and_not_counted(&o);
}
