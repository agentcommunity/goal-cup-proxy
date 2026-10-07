//! V1–V5 over the three committed M0 traces.
//!
//! Every fixture is loaded through `goalcup_verifier::Pipeline` — the sequence
//! `verify_trace_with_options` itself runs — so a check is asserted against the context
//! the shipped verifier builds. Where a test needs a state these fixtures do not hold, it
//! borrows that context and overrides the one field it means to change, which is how a
//! constructed context stops being able to assert a combination the verifier cannot
//! produce.

// The shared fixture pipeline, pulled in by path so every check test reaches it the same
// way — several of them already carry a file-local `mod support` of scenario builders.
#[path = "support/mod.rs"]
mod fixture_support;

use ed25519_dalek::{Signer, SigningKey};
use fixture_support::pipeline::{one, results};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::{ConfigDeviation, ForeignScanOutcome, ForeignScanUnavailableReason};
use goalcup_verifier::checks::v4_continuity::V4Continuity;
use goalcup_verifier::checks::v9_config::V9Config;
use goalcup_verifier::checks::{Check, CheckResult, Severity, all_checks};
use goalcup_verifier::gaps::{classify, verify_acks};
use goalcup_verifier::parsers::ConfigFact;

#[test]
fn no_finding_carries_an_absolute_machine_path() {
    for id in ["claude-code-cli", "codex-cli"] {
        for result in results(id) {
            for finding in &result.findings {
                // Codex's `<environment_context>` header is read as a real (unwitnessed) human
                // message — V1's goal (`goal_recovered`) and V3's `leg1_without_leg2_post`
                // both quote it — and that header's own text carries the real `<cwd>`. Every
                // other code, on every other fixture, still must not leak one.
                if id == "codex-cli"
                    && matches!(
                        finding.code.as_str(),
                        "goal_recovered" | "leg1_without_leg2_post"
                    )
                {
                    continue;
                }
                assert!(
                    !finding.message.contains("/Users/"),
                    "{id} {}/{}: message leaks an absolute path: {}",
                    result.id,
                    finding.code,
                    finding.message
                );
                for ev in &finding.evidence {
                    assert!(
                        !ev.note.contains("/Users/"),
                        "{id} {}/{}: evidence note leaks an absolute path: {}",
                        result.id,
                        finding.code,
                        ev.note
                    );
                }
            }
        }
    }

    // An M0 fixture never carries an absolute path in a
    // `ConfigDeviation.path` or a `ConfigFact.detail` that reaches this far, because none
    // of the three ships one. This borrows the shipped context for the claude fixture and
    // overrides only what it plants, so the assertion is checked against adversarial bytes
    // sitting in a context the verifier can actually build.
    let fixture = fixture_support::pipeline::m0("claude-code-cli");
    let entry_id = fixture.context().trace.entry_id.clone();
    let mut parsed = fixture.parsed().clone();
    // A fake hook body: the whole POST body `ConfigFact.detail` carries on Claude Code's
    // real `ConfigChange` event, including a machine path and something that reads like a
    // secret. Never interpolated — only its length may appear.
    parsed.config_events.push(ConfigFact {
        event: "ConfigChange".to_string(),
        at: None,
        detail: r#"{"hookHome":"/Users/real/.claude","apiKey":"do-not-print-me"}"#.to_string(),
        shim: None,
    });
    let events = vec![
        Envelope {
            protocol_version: 1,
            entry_id: entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T00:00:00Z".to_string(),
            link: Link {
                seq: 1,
                prev: "0".repeat(64),
                hash: "a".repeat(64),
            },
            body: EventBody::ConfigDeviation(ConfigDeviation {
                path: "/Users/real/.claude/settings.json".to_string(),
                expected_sha256: "a".repeat(64),
                observed_sha256: "b".repeat(64),
                detected_at: "2026-09-04T00:00:00Z".to_string(),
                diff: "- old /Users/real/secret-path\n+ new".to_string(),
                restored: true,
                arming_lost: false,
            }),
            redaction: None,
        },
        Envelope {
            protocol_version: 1,
            entry_id: entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T00:00:00Z".to_string(),
            link: Link {
                seq: 2,
                prev: "a".repeat(64),
                hash: "b".repeat(64),
            },
            body: EventBody::TailNotice {
                store: "main".to_string(),
                path: "/Users/real/.claude/projects/x/main.jsonl".to_string(),
                kind: goalcup_protocol::events::TailNoticeKind::Truncated,
                offset_before: 4096,
                offset_after: 128,
                inode_before: Some(1),
                inode_after: Some(1),
                at: "2026-09-04T00:00:00Z".to_string(),
            },
            redaction: None,
        },
    ];
    let mut cx = fixture.context();
    cx.parsed = &parsed;
    cx.events = &events;
    let result = V9Config.run(&cx);
    assert_eq!(
        result.verdict,
        Severity::Fail,
        "the deviation and the truncation are both unexplained"
    );
    for finding in &result.findings {
        assert!(
            !finding.message.contains("/Users/"),
            "V9/{}: message leaks an absolute path: {}",
            finding.code,
            finding.message
        );
        assert!(
            !finding.message.contains("do-not-print-me"),
            "V9/{}: message leaks a hook body: {}",
            finding.code,
            finding.message
        );
        assert!(
            !finding.message.contains("secret-path"),
            "V9/{}: message leaks diff text: {}",
            finding.code,
            finding.message
        );
        for ev in &finding.evidence {
            assert!(
                !ev.note.contains("/Users/"),
                "V9/{}: evidence note leaks an absolute path: {}",
                finding.code,
                ev.note
            );
        }
    }

    // The M0 fixtures carry no acknowledgements and no heartbeats, so `cx.gaps` is empty on
    // every one of them and the loop above never reaches V4's gap path. This constructs an
    // anchored context: one acknowledgement, genuinely signed by the key
    // this build pins and reconciled by `verify_acks` against a record the fixture really
    // holds, and two heartbeats whose timestamps bracket the records' own (07:42–07:48 in
    // this fixture), so a dirty gap is produced and rendered.
    //
    // Signed rather than asserted: the anchor allowlist `classify` reads and the
    // reconciliation the context carries are the same one, which is the pairing
    // `verify_trace` produces. `NO_ACKS` beside a `classify(…, &[0])` is a state no run of
    // the verifier can reach.
    //
    // Bracketing by time rather than by sequence matters: these
    // are `raw_jsonl` stores, chained here from 1 per store, so `main`, `subagent` and
    // `a9-other-folder` all hold a seq 20 and none of them is the daemon's seq 20. A
    // record from such a store is placed by its own timestamp or not at all.
    let events = vec![
        Envelope {
            protocol_version: 1,
            entry_id: entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T00:00:00Z".to_string(),
            link: Link {
                seq: 50,
                prev: "0".repeat(64),
                hash: "c".repeat(64),
            },
            body: EventBody::Heartbeat {
                ts: "2026-09-03T07:00:00Z".to_string(),
                stores_seen: Vec::new(),
                receiver_ok: true,
            },
            redaction: None,
        },
        Envelope {
            protocol_version: 1,
            entry_id: entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T00:00:00Z".to_string(),
            link: Link {
                seq: 150,
                prev: "c".repeat(64),
                hash: "d".repeat(64),
            },
            body: EventBody::Heartbeat {
                ts: "2026-09-03T08:00:00Z".to_string(),
                stores_seen: Vec::new(),
                receiver_ok: true,
            },
            redaction: None,
        },
    ];
    let server = SigningKey::from_bytes(&[5u8; 32]);
    let key = hex::encode(server.verifying_key().to_bytes());
    let main = cx
        .stores
        .iter()
        .find(|s| s.name == "main")
        .expect("the claude fixture declares a `main` store");
    let anchored_at = &main.records[99];
    let mut ack = AckCheckpoint {
        entry_id: entry_id.clone(),
        seq: anchored_at.seq,
        hash: anchored_at.hash.clone(),
        acked_at: "2026-09-03T15:00:00Z".to_string(),
        server_signature: String::new(),
    };
    ack.server_signature = hex::encode(server.sign(&ack.signing_bytes().unwrap()).to_bytes());
    let acks = vec![ack];
    let reconciliation = verify_acks(None, cx.stores, &acks, Some(&key), &entry_id);
    assert_eq!(
        reconciliation.verified,
        vec![0],
        "the anchor must genuinely verify: {:?}",
        reconciliation.mismatches
    );
    let gaps = classify(
        cx.stores,
        &acks,
        &events,
        cx.manifest,
        &reconciliation.verified,
    );
    assert!(
        gaps.iter().any(|g| !g.appended.is_empty()),
        "the constructed context must actually produce a gap with records in it"
    );
    let mut cx = fixture.context();
    cx.events = &events;
    cx.acks = &acks;
    cx.acks_verified = &reconciliation;
    cx.gaps = &gaps;
    cx.server_key = Some(&key);
    let results: Vec<CheckResult> = all_checks().iter().map(|c| c.run(&cx)).collect();
    assert!(
        results
            .iter()
            .flat_map(|r| r.findings.iter())
            .any(|f| f.code == "dirty_gap"),
        "V4 must render the gap, or this assertion proves nothing"
    );
    for result in &results {
        for finding in &result.findings {
            assert!(
                !finding.message.contains("/Users/"),
                "{}/{}: message leaks an absolute path: {}",
                result.id,
                finding.code,
                finding.message
            );
            for ev in &finding.evidence {
                assert!(
                    !ev.note.contains("/Users/"),
                    "{}/{}: evidence note leaks an absolute path: {}",
                    result.id,
                    finding.code,
                    ev.note
                );
            }
        }
    }
}

#[test]
fn v1_recovers_each_goal_within_four_thousand_characters() {
    // Codex's goal is asserted separately below, so only Claude Code is checked for the real
    // goal here.
    let rs = results("claude-code-cli");
    let v1 = one(&rs, "V1");
    assert_eq!(v1.verdict, Severity::Pass, "{:?}", v1.findings);
    assert!(v1.findings[0].message.starts_with("goal recovered"));
    assert!(v1.findings[0].message.contains("create hello.py"));

    // Codex's index-0 record is not the real goal.
    let rs = results("codex-cli");
    let v1 = one(&rs, "V1");
    assert_eq!(v1.verdict, Severity::Pass, "{:?}", v1.findings);
    assert!(v1.findings[0].message.starts_with("goal recovered"));
    assert!(
        v1.findings[0].message.contains("(unwitnessed")
            && v1.findings[0].message.contains("<environment_context>"),
        "the interim reading: `<environment_context>` is read as the goal: {}",
        v1.findings[0].message
    );
}

#[test]
fn v2_fails_on_all_three_fixtures_and_names_the_offending_messages() {
    // Every M0 fixture is a deliberately non-compliant script: it exercises the rule.
    let rs = results("claude-code-cli");
    let v2 = one(&rs, "V2");
    assert_eq!(v2.verdict, Severity::Fail);
    let msgs: Vec<&str> = v2.findings.iter().map(|f| f.message.as_str()).collect();
    assert_eq!(
        msgs.len(),
        4,
        "4 of 7 later messages are not `keep going`: {msgs:?}"
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("use a subagent to count the lines in hello.py"))
    );
    assert!(msgs.iter().any(|m| m.contains("also add a README")));
    assert!(msgs.iter().any(|m| m.contains("what model are you?")));

    let rs = results("codex-cli");
    let v2 = one(&rs, "V2");
    assert_eq!(v2.verdict, Severity::Fail);
    let msgs: Vec<&str> = v2.findings.iter().map(|f| f.message.as_str()).collect();
    // 5, not 4. Codex declares `positive = []`, so nothing distinguishes the main thread's
    // synthetic `<environment_context>` header from a real message; it becomes the
    // unwitnessed goal at index 0 (V1's problem, not V2's — V2 skips an unwitnessed index-0
    // record on purpose), which bumps every real message up one index and means the actual
    // first typed message, `create hello.py...`, is not recognised as the goal either. V2
    // wrongly fails it as `not_keep_going` too.
    assert_eq!(
        msgs.len(),
        5,
        "the wrongly-demoted goal, the `going` typo, the subagent request, 2+2, and the README: {msgs:?}"
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("message 1 is `create hello.py")),
        "the interim reading: the real goal is no longer recognised as the goal: {msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.contains("message 3 is `going`")),
        "the typo must be named exactly, not normalised away: {msgs:?}"
    );
    assert!(msgs.iter().any(|m| m.contains("also add a README")));
    assert!(msgs.iter().any(|m| m.contains("what is 2 + 2?")));
}

#[test]
fn v3_reports_the_blocked_prompt_as_a_flag_not_a_disagreement() {
    for id in ["claude-code-cli", "codex-cli"] {
        let rs = results(id);
        let v3 = one(&rs, "V3");
        assert_eq!(v3.verdict, Severity::Flag, "{id}: {:?}", v3.findings);
        assert!(
            v3.findings
                .iter()
                .any(|f| f.code == "leg2_without_leg1_record"),
            "{id} must flag the blocked over-length prompt"
        );
        assert!(
            !v3.findings.iter().any(|f| f.code == "text_divergence"),
            "{id}: the legs agree byte for byte on every paired message"
        );
        assert!(
            !v3.findings.iter().any(|f| f.code == "duplicate_pairing"),
            "{id}: MF-2 — no post pairs twice in a genuine capture"
        );
    }

    // Only the post whose own text the harness retained is annotated as blocked.
    // `contains("exceeds")` would be true for every post as soon as any block had happened.
    let rs = results("claude-code-cli");
    let v3 = one(&rs, "V3");
    let unpaired: Vec<&str> = v3
        .findings
        .iter()
        .filter(|f| f.code == "leg2_without_leg1_record")
        .map(|f| f.message.as_str())
        .collect();
    // The harness's own `<task-notification>` record is excluded by the positive test
    // (`origin.kind` is not `human`), and no join reattaches its leg-2 post to that excluded
    // leg-1 record. So both orphaned posts read as plain capture gaps: the over-length paste
    // and the task-notification.
    assert_eq!(
        unpaired.len(),
        2,
        "the over-length paste and the task-notification, neither joined to a leg-1 record"
    );
    assert!(
        unpaired
            .iter()
            .any(|m| m.contains("<task-notification>") && !m.contains("a blocking hook refused it")),
        "the task-notification post has no leg-1 record joined to it any more: {unpaired:?}"
    );
    assert!(
        !v3.findings
            .iter()
            .any(|f| f.code == "excluded_message_witnessed_by_field"
                || f.code == "excluded_message_witnessed"),
        "the excluded-twin join is gone; nothing emits these codes any more"
    );
    assert!(
        unpaired
            .iter()
            .any(|m| m.contains("OVERLENGTH-TEST-A7") && m.contains("a blocking hook refused it")),
        "the 4,100-character paste WAS refused, and its text survives in the notice"
    );

    // The cross-leg identifier clause is implemented, not prose.
    let rs = results("codex-cli");
    let v3 = one(&rs, "V3");
    let ids = v3
        .findings
        .iter()
        .find(|f| f.code == "cross_leg_identifiers")
        .expect("V3 reports what the legs agree on identifier-wise");
    assert!(ids.message.contains("2 shared"), "got {}", ids.message);
    assert!(ids.message.contains("turn_id+order"), "got {}", ids.message);
}

#[test]
fn v4_sees_one_root_one_resume_and_the_foreign_sessions() {
    let rs = results("claude-code-cli");
    let v4 = one(&rs, "V4");
    assert_eq!(v4.verdict, Severity::Flag);
    assert!(
        v4.findings
            .iter()
            .any(|f| f.code == "foreign_session" && f.message.contains("a9-other-folder"))
    );
    assert!(
        v4.findings
            .iter()
            .any(|f| f.code == "resume" && f.message.contains("1 resume"))
    );

    let rs = results("codex-cli");
    let v4 = one(&rs, "V4");
    assert_eq!(v4.verdict, Severity::Flag);
    // The mis-launched `01a06616` thread cannot be a
    // `second_root_thread` on this fixture: `wrong-cwd-01a06616-excerpt.jsonl` carries no
    // `session_meta`, so no `SessionFact` exists for it and the id appears nowhere in its
    // bytes. It is reported — as the declared-foreign store — and that is the expectation
    // the fixture supports. `second_root_thread` stays implemented and stays tested on
    // Claude Code's `a9-other-folder` probe, whose fixture does carry a session id.
    assert!(
        v4.findings
            .iter()
            .any(|f| f.code == "foreign_session" && f.message.contains("wrong-cwd-01a06616")),
        "the mis-launched thread is reported as a foreign store (D9/D10)"
    );
    assert!(
        !v4.findings.iter().any(|f| f.code == "second_root_thread"),
        "no second root can be established from a file with no session_meta"
    );
    // Claude's `a9-other-folder` probe is where the second-root detector is exercised.
    let rs_claude = results("claude-code-cli");
    assert!(
        one(&rs_claude, "V4")
            .findings
            .iter()
            .any(|f| f.code == "second_root_thread"),
        "the A9 probe IS a second root session under the same config dir, and says so"
    );
}

#[test]
fn v4_flags_only_explicit_unavailable_foreign_scan_coverage() {
    let fixture = fixture_support::pipeline::m0("claude-code-cli");
    let coverage = Envelope {
        protocol_version: 2,
        entry_id: fixture.context().trace.entry_id.clone(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-04T00:00:00Z".to_string(),
        link: Link {
            seq: 99,
            prev: "a".repeat(64),
            hash: "b".repeat(64),
        },
        body: EventBody::ForeignScanCoverage {
            declaring_connector: "opencode".to_string(),
            declaring_manifest_sha256: "c".repeat(64),
            scan_index: 0,
            outcome: ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Schema),
            at: "2026-09-04T00:00:00Z".to_string(),
        },
        redaction: None,
    };
    let mut complete = coverage.clone();
    complete.link.seq = 100;
    complete.body = EventBody::ForeignScanCoverage {
        declaring_connector: "opencode".to_string(),
        declaring_manifest_sha256: "c".repeat(64),
        scan_index: 0,
        outcome: ForeignScanOutcome::Complete,
        at: "2026-09-04T00:01:00Z".to_string(),
    };
    let mut absent = complete.clone();
    absent.link.seq = 101;
    absent.body = EventBody::ForeignScanCoverage {
        declaring_connector: "opencode".to_string(),
        declaring_manifest_sha256: "c".repeat(64),
        scan_index: 0,
        outcome: ForeignScanOutcome::Absent,
        at: "2026-09-04T00:02:00Z".to_string(),
    };
    let events = vec![coverage, complete, absent];
    let mut cx = fixture.context();
    cx.events = &events;
    let result = V4Continuity.run(&cx);
    let incomplete: Vec<_> = result
        .findings
        .iter()
        .filter(|finding| finding.code == "foreign_scan_incomplete")
        .collect();
    assert_eq!(incomplete.len(), 1, "complete and absent are not findings");
    assert_eq!(incomplete[0].severity, Severity::Flag);
    assert_eq!(incomplete[0].evidence[0].seq, Some(99));
    let evidence_hash = "b".repeat(64);
    assert_eq!(
        incomplete[0].evidence[0].sha256.as_deref(),
        Some(evidence_hash.as_str())
    );

    let empty = Vec::new();
    cx.events = &empty;
    let historical = V4Continuity.run(&cx);
    assert!(
        historical
            .findings
            .iter()
            .all(|finding| finding.code != "foreign_scan_incomplete")
    );
}

#[test]
fn v5_reports_model_identity_and_every_switch() {
    let rs = results("claude-code-cli");
    let v5 = one(&rs, "V5");
    // Reported, and Pass-level. No rule says an entry runs on one model, so the
    // list of models is the same kind of statement `single_model` is — it belongs in
    // `Verdict::checks`, where a reviewer reads what happened, and not in the review
    // queue, where a reviewer reads what needs a decision.
    assert_eq!(
        v5.verdict,
        Severity::Pass,
        "a mid-run switch is reported, not queued"
    );
    assert_eq!(v5.findings[0].code, "model_changed");
    let msg = &v5.findings[0].message;
    assert!(msg.contains("claude-opus-5"), "{msg}");
    assert!(msg.contains("claude-sonnet-5"), "{msg}");

    let rs = results("codex-cli");
    let v5 = one(&rs, "V5");
    let msg = &v5.findings[0].message;
    assert!(
        msg.contains("gpt-5.6-sol") && msg.contains("gpt-5.6-luna"),
        "{msg}"
    );

    // Version drift is a reported clause, not prose. An M0 trace carries no
    // event stream, so every connector reports the absence explicitly.
    for id in ["claude-code-cli", "codex-cli"] {
        let rs = results(id);
        let v5 = one(&rs, "V5");
        assert!(
            v5.findings.iter().any(|f| f.code == "no_version_drift"),
            "{id}: V5 must say whether the harness version moved, not stay silent"
        );
        assert!(!v5.findings.iter().any(|f| f.code == "version_drift"));
    }
}
