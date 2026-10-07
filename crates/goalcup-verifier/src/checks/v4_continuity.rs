//! V4: session continuity — one fresh start, resumes accounted for, no second root
//! agent, no foreign harness session inside the workspace (D9, D10).
//!
//! **Where `foreign_session` comes from.** The live source is `EventBody::ForeignSession`,
//! which the daemon's foreign-harness watcher emits when it finds a session belonging to
//! any installed harness whose directory is a bound workspace — identifiers and paths only,
//! never content (D20). That is D10's detector, and it is the only
//! source that can fire on a real entry.
//!
//! The `trace.toml` `foreign = true` path is kept **solely for the committed M0
//! fixture sets**, which were captured before the daemon existed and therefore carry no
//! event stream at all. A finding raised from the descriptor says so in its evidence
//! note, so a reviewer can never mistake a fixture author's declaration for a
//! measurement. When a trace carries an event stream, the descriptor flag is ignored.
//!
//! **`leg2_receiver_seq_gap`.** `Leg2Post::receiver_meta` carries the receiver's own
//! per-source counter, chained raw. `crate::gaps::receiver_seq_gaps` re-derives, from the
//! full persisted stream, whether each source's declared `seq` stayed contiguous; a hole is
//! a `Flag`, the same class an evidence gap gets elsewhere in this file, never a silent
//! `Pass` and never a `Fail` on its own. A source that never declares a counter (`seq:
//! None`, e.g. Claude Code's native `http` transport) takes no part in the rule.
//!
//! One discontinuity is not a hole: a counter that restarts at exactly 1 across a resume
//! the chain itself witnesses. Every hook source's counter belongs to the relaunched
//! process, so without that reading the rule would fail every honest run that resumes. The
//! forgiveness is narrow by construction — exactly 1, and only across a `SessionStart` post
//! declaring `source: "resume"` between the two posts; a `DaemonRestart` is not one,
//! because the shim's counter is a file that outlives the daemon.
//! `crate::gaps::receiver_seq_gaps` holds the reasoning.

use super::{Check, CheckContext, CheckResult, Evidence, flag, pass};
use goalcup_protocol::envelope::EventBody;
use goalcup_protocol::events::ForeignScanOutcome;

/// One gap reason as a sentence. No paths, no record text: store names, sequences and
/// counts only.
fn why(reason: &crate::gaps::GapReason) -> String {
    use crate::gaps::GapReason as R;
    match reason {
        R::RecordsAppended => {
            "the chain grew inside a hole in the heartbeat series, which the daemon was \
             not watching"
                .to_string()
        }
        R::UnplaceableRecords { store, records } => format!(
            "`{store}` holds {records} record(s) that cannot be placed inside or outside \
             this window: the store is chained per store rather than on the daemon's \
             chain, and those records carry no timestamp of their own"
        ),
        R::Unanchored => {
            "no acknowledgement covers the end of this window, so nobody witnessed what \
             the chain did in it"
                .to_string()
        }
        R::CursorMismatch {
            store,
            records,
            cursor_delta,
        } => format!(
            "the chain holds {records} record(s) for `{store}` in this window but the \
             daemon's own cursor moved {}: records it never accounted for",
            delta(cursor_delta)
        ),
        R::CursorAheadOfChain {
            store,
            cursor_delta,
        } => format!(
            "the daemon's cursor for `{store}` moved {} while nothing reached the chain, \
             and this store declares no partition key that would explain dropped rows",
            delta(cursor_delta)
        ),
        R::RestartHeadMismatch {
            claimed_seq,
            observed_seq,
        } => format!(
            "a daemon restart claimed it resumed from head seq {claimed_seq}, but the \
             chain holds seq {observed_seq} immediately below the restart"
        ),
        R::HeartbeatsOutOfOrder { seconds } => format!(
            "the heartbeat timestamps step {seconds}s backwards in chain order{}",
            if *seconds > crate::gaps::BACKSTEP_GRACE_SECONDS {
                ", so this window has no measurable duration"
            } else {
                " — within the clock-correction grace, so the window still stands"
            }
        ),
    }
}

fn delta(cursor_delta: &Option<u64>) -> String {
    match cursor_delta {
        Some(n) => format!("by {n}"),
        None => "in a form with no number to compare".to_string(),
    }
}

pub struct V4Continuity;

impl Check for V4Continuity {
    fn id(&self) -> &'static str {
        "V4"
    }
    fn title(&self) -> &'static str {
        "session continuity: fresh start, resumes, no second root, no foreign session"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        let mut findings = Vec::new();

        let tracked: Vec<_> = cx.parsed.sessions.iter().filter(|s| s.tracked).collect();
        match tracked.len() {
            1 => findings.push(pass(
                "one_tracked_session",
                format!("one tracked session: {}", tracked[0].id),
            )),
            n => findings.push(flag(
                "tracked_session_count",
                format!("{n} sessions match the tracked ids; expected exactly one"),
                Vec::new(),
            )),
        }

        for s in cx
            .parsed
            .sessions
            .iter()
            .filter(|s| !s.tracked && s.is_root)
        {
            findings.push(flag(
                "second_root_thread",
                format!(
                    "root session {} in store `{}` is not the tracked thread (D9: a silent degrade to a new thread)",
                    s.id, s.store
                ),
                vec![Evidence::note(
                    &s.store,
                    // Never `s.directory`, the session's absolute working-directory path.
                    // The store name is already `Evidence.store`; the session id is the
                    // rest of the citation, never the machine path it worked in.
                    format!("session {}", s.id),
                )],
            ));
        }

        for env in cx.events {
            let EventBody::ForeignScanCoverage {
                declaring_connector,
                scan_index,
                outcome: ForeignScanOutcome::Unavailable(reason),
                ..
            } = &env.body
            else {
                continue;
            };
            findings.push(flag(
                "foreign_scan_incomplete",
                format!(
                    "foreign-session continuity coverage for `{declaring_connector}` scan \
                     {scan_index} was unavailable ({})",
                    reason.as_str()
                ),
                vec![Evidence::at(
                    "events",
                    env.link.seq,
                    Some(env.link.hash.clone()),
                    "bounded foreign scan unavailable",
                )],
            ));
        }

        // Live source first: the daemon's foreign-harness watcher (D10).
        let mut live = 0usize;
        for env in cx.events {
            if let EventBody::ForeignSession {
                harness,
                session_id,
                store_path: _,
                first_seen,
            } = &env.body
            {
                live += 1;
                findings.push(flag(
                    "foreign_session",
                    format!(
                        "a `{harness}` session `{session_id}` was found writing inside a bound \
                         workspace, first seen {first_seen}; its rows must not enter the chain (D20)"
                    ),
                    vec![Evidence::at(
                        "events",
                        env.link.seq,
                        Some(env.link.hash.clone()),
                        // Never the foreign harness's own absolute store path. The session
                        // id is already in the message; the note repeats it rather than the
                        // machine path (D20: identifiers only, never content, and never a
                        // path either).
                        format!("session {session_id}"),
                    )],
                ));
            }
        }
        // Fixture path: only for the committed M0 sets, which predate the event stream.
        if live == 0 {
            for spec in cx.trace.leg1.iter().filter(|s| s.foreign) {
                findings.push(flag(
                    "foreign_session",
                    format!(
                        "store `{}` holds a session the tracker never armed; its rows must not enter the chain (D20)",
                        spec.name
                    ),
                    vec![Evidence::note(
                        &spec.name,
                        "declared foreign in trace.toml — a fixture declaration, not a \
                         live detection; this path exists only for the M0 fixtures",
                    )],
                ));
            }
        }

        // The receiver's own declared per-source counter, read off the raw chained
        // envelope stream — `VerifiedChain::envelopes` — because the derived `Leg2Post`
        // view drops `receiver_meta` on purpose (chain.rs). `None` on an M0 fixture, which
        // carries no chain at all and therefore no receiver bookkeeping to check.
        if let Some(chain) = cx.chain {
            for gap in crate::gaps::receiver_seq_gaps(&chain.envelopes) {
                // A reset to 1 reaches this point only with no resume witnessed between
                // the two posts, so say which of the two things went wrong: a counter that
                // restarted unexplained reads very differently from a skipped value.
                let how = if gap.observed == 1 {
                    "the counter restarted with no resume witnessed in the chain between \
                     the two posts"
                } else {
                    "a post between them is missing from this evidence"
                };
                findings.push(flag(
                    "leg2_receiver_seq_gap",
                    format!(
                        "receiver source `{}` declared seq {} where {} was expected: {how}",
                        gap.source, gap.observed, gap.expected
                    ),
                    vec![Evidence::at(
                        "events",
                        gap.seq,
                        Some(gap.sha256),
                        format!(
                            "source `{}`: expected seq {}, got {}",
                            gap.source, gap.expected, gap.observed
                        ),
                    )],
                ));
            }
        }

        // D9's gap policy: "the primary instrument, not a fallback" on five of eight
        // harnesses. A dirty gap is a window the trace cannot account for; a clean gap is
        // downtime the trace can positively account for — nothing appended, the daemon's
        // own cursors agreeing, and an acknowledgement covering the far edge.
        //
        // Evidence cites `store`, `seq` and `sha256` and nothing else. The reviewer opens
        // the store at that sequence; the verdict does not carry the participant's
        // filesystem.
        for g in cx.gaps {
            let window = match (&g.from, &g.to) {
                (Some(a), Some(b)) => format!(" between {a} and {b}"),
                _ => String::new(),
            };
            let evidence: Vec<Evidence> = g
                .appended
                .iter()
                .take(3)
                .map(|l| {
                    Evidence::at(
                        &l.store,
                        l.seq,
                        Some(l.sha256.clone()),
                        "appended inside this window",
                    )
                })
                .collect();
            // Only a real hole is downtime. A punctual window whose cursors contradict the
            // chain is reported through its reasons, not as an outage.
            if g.is_hole {
                match g.kind {
                    crate::gaps::GapKind::Dirty => findings.push(super::fail(
                        "dirty_gap",
                        format!(
                            "{} record(s) appended{window} (chain seq {}..{}): {}. Read \
                             exactly these lines in the store.",
                            g.appended.len(),
                            g.from_seq,
                            g.to_seq,
                            g.reasons.iter().map(why).collect::<Vec<_>>().join("; ")
                        ),
                        evidence.clone(),
                    )),
                    // Pass-level: the sentence ends "downtime accounted for",
                    // and a window the trace positively accounts for is not a decision a
                    // reviewer has to make. Its opposite, `dirty_gap`, is still a Fail.
                    crate::gaps::GapKind::Clean => findings.push(pass(
                        "clean_gap",
                        format!(
                            "the daemon was down{window} (chain seq {}..{}), nothing was \
                             appended to any store while it was, and an acknowledgement \
                             covers the end of the window; downtime accounted for (D9)",
                            g.from_seq, g.to_seq
                        ),
                    )),
                }
            }
            // Every reason that has a code of its own is raised under it, at its own
            // severity: a cursor running ahead of a partition-less store is a question, not
            // a verdict. This is also the only rendering a non-hole gap gets —
            // `unanchored_records`, the stretch of chain above the last acknowledgement,
            // reaches a reader through here.
            let mut rendered = g.is_hole;
            for reason in &g.reasons {
                let Some(code) = reason.code() else {
                    continue;
                };
                rendered = true;
                let message = format!(
                    "{}{window} (chain seq {}..{}); {} record(s) in this window",
                    why(reason),
                    g.from_seq,
                    g.to_seq,
                    g.appended.len()
                );
                findings.push(match reason.severity() {
                    super::Severity::Fail => super::fail(code, message, evidence.clone()),
                    _ => flag(code, message, evidence.clone()),
                });
            }
            // Fail closed: a dirty gap that named no coded reason still has to be seen.
            if !rendered && g.kind == crate::gaps::GapKind::Dirty {
                findings.push(super::fail(
                    "dirty_gap",
                    format!(
                        "{} record(s) appended{window} (chain seq {}..{}) in a window the \
                         trace cannot account for",
                        g.appended.len(),
                        g.from_seq,
                        g.to_seq
                    ),
                    evidence.clone(),
                ));
            }
        }

        // A beat the verifier cannot place is a beat that cannot bound a hole: say so
        // rather than dropping it.
        for n in crate::gaps::unparseable_heartbeats(cx.events) {
            findings.push(flag(
                "heartbeat_unparseable",
                "a proof-of-life envelope carries a timestamp that does not parse, so it \
                 cannot bound a hole in the series"
                    .to_string(),
                vec![Evidence::at("events", n.seq, Some(n.sha256), "heartbeat")],
            ));
        }

        // Both branches are Pass-level. One says the connector leaves no resume
        // marker, which is a property of the harness and identical on every run of it; the
        // other says a resume happened, which is a fact about the run and not a fault. A
        // resume that hid work is `dirty_gap`, and it is still a Fail.
        if cx.parsed.resume_markers.is_empty() {
            findings.push(pass(
                "no_resume_marker",
                format!(
                    "{} leaves no durable resume marker; gap classification is the only instrument (D9)",
                    cx.manifest.id
                ),
            ));
        } else {
            findings.push(pass(
                "resume",
                format!(
                    "{} resume(s) recorded: {}",
                    cx.parsed.resume_markers.len(),
                    cx.parsed.resume_markers.join(", ")
                ),
            ));
        }

        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::VerifiedChain;
    use crate::checks::Severity;
    use crate::checks::support;
    use crate::parsers::SessionFact;
    use goalcup_protocol::chain::Link;
    use goalcup_protocol::envelope::Envelope;

    fn session(id: &str, tracked: bool) -> SessionFact {
        SessionFact {
            id: id.to_string(),
            store: "main".to_string(),
            directory: Some("/ws".to_string()),
            parent: None,
            thread_source: "user".to_string(),
            is_root: true,
            tracked,
        }
    }

    /// Neither zero nor two tracked sessions is the one-fresh-start
    /// this check exists to confirm.
    #[test]
    fn two_tracked_sessions_is_a_flag_not_a_pass() {
        let mut parsed = support::parsed();
        parsed.sessions.push(session("s1", true));
        parsed.sessions.push(session("s2", true));
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let cx = CheckContext {
            trace: &trace,
            manifest: &manifest,
            parsed: &parsed,
            human: &human,
            join_strategy: "prompt_id",
            stores: &[],
            events: &[],
            acks: &[],
            acks_verified: &crate::gaps::NO_ACKS,
            seal: None,
            gaps: &[],
            server_key: None,
            key_source: crate::KeySource::None,
            dev_mode: false,
            chain: None,
            receipt: None,
            baseline: None,
            manifest_sha256: "",
            manifest_pin: crate::ManifestPinning::default(),
        };
        let result = V4Continuity.run(&cx);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "tracked_session_count" && f.message.contains("2 sessions"))
        );
    }

    fn leg2_post_envelope(seq: u64, source: &str, receiver_seq: Option<u64>) -> Envelope {
        Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: "e".to_string(),
            source: "hooks".to_string(),
            recorded_at: "2026-09-04T00:00:00Z".to_string(),
            link: Link {
                seq,
                prev: format!("{:064}", seq.wrapping_sub(1)),
                hash: format!("{seq:064}"),
            },
            body: EventBody::Leg2Post {
                event: "UserPromptSubmit".to_string(),
                payload: serde_json::json!({}),
                path: None,
                receiver_meta: Some(serde_json::json!({
                    "reported_config_hash": null,
                    "seq": receiver_seq,
                    "source": source,
                    "anomalies": [],
                })),
            },
            redaction: None,
        }
    }

    fn empty_verified_chain(envelopes: Vec<Envelope>) -> VerifiedChain {
        let head_hash = envelopes
            .last()
            .map(|e| e.link.hash.clone())
            .unwrap_or_else(|| "0".repeat(64));
        let head_seq = envelopes.last().map(|e| e.link.seq).unwrap_or(0);
        VerifiedChain {
            entry_id: "e".to_string(),
            genesis_link: "0".repeat(64),
            head_seq,
            head_hash,
            envelopes,
            stores: Vec::new(),
            posts: crate::legs::Leg2Stream { posts: Vec::new() },
            events: Vec::new(),
            redactions: Vec::new(),
            seal_link: None,
            baseline_link: None,
            baseline_digest: None,
            break_at: None,
            truncated_tail: None,
            unsupported_version: None,
            incompatible_envelope: None,
            redaction_audit_issues: Vec::new(),
            legacy_redaction_log_missing: Vec::new(),
        }
    }

    /// A hole in one source's declared counter is a `Flag` naming that source, and
    /// says nothing about a source that never declares one at all.
    #[test]
    fn leg2_receiver_seq_gap_flags_the_gapped_source_and_nothing_else() {
        let parsed = support::parsed();
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let chain = empty_verified_chain(vec![
            leg2_post_envelope(0, "hooks-http", Some(1)),
            leg2_post_envelope(1, "hooks-http", Some(3)), // 2 never arrived
            leg2_post_envelope(2, "http", None),          // uncounted; never flagged
        ]);
        let cx = CheckContext {
            trace: &trace,
            manifest: &manifest,
            parsed: &parsed,
            human: &human,
            join_strategy: "prompt_id",
            stores: &[],
            events: &[],
            acks: &[],
            acks_verified: &crate::gaps::NO_ACKS,
            seal: None,
            gaps: &[],
            server_key: None,
            key_source: crate::KeySource::None,
            dev_mode: false,
            chain: Some(&chain),
            receipt: None,
            baseline: None,
            manifest_sha256: "",
            manifest_pin: crate::ManifestPinning::default(),
        };
        let result = V4Continuity.run(&cx);
        let gaps: Vec<_> = result
            .findings
            .iter()
            .filter(|f| f.code == "leg2_receiver_seq_gap")
            .collect();
        assert_eq!(gaps.len(), 1, "{:?}", result.findings);
        assert_eq!(gaps[0].severity, Severity::Flag);
        assert!(
            gaps[0].message.contains("hooks-http"),
            "{}",
            gaps[0].message
        );
        assert_eq!(gaps[0].evidence.len(), 1);
        assert!(
            gaps[0].evidence[0].note.contains("expected seq 2, got 3"),
            "{}",
            gaps[0].evidence[0].note
        );
    }

    /// A trace with no chain (an M0 fixture) has no receiver bookkeeping to check, so the
    /// rule is silent rather than crashing on an absent stream.
    #[test]
    fn no_chain_means_no_receiver_seq_gap_finding() {
        let parsed = support::parsed();
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let human = support::human();
        let cx = CheckContext {
            trace: &trace,
            manifest: &manifest,
            parsed: &parsed,
            human: &human,
            join_strategy: "prompt_id",
            stores: &[],
            events: &[],
            acks: &[],
            acks_verified: &crate::gaps::NO_ACKS,
            seal: None,
            gaps: &[],
            server_key: None,
            key_source: crate::KeySource::None,
            dev_mode: false,
            chain: None,
            receipt: None,
            baseline: None,
            manifest_sha256: "",
            manifest_pin: crate::ManifestPinning::default(),
        };
        let result = V4Continuity.run(&cx);
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == "leg2_receiver_seq_gap")
        );
    }
}
