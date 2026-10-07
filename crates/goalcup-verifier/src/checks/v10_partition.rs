//! V10: no record in the chain fails its store's `partition_key`, and a foreign session
//! detected inside a bound workspace is reported (DECISIONS D20, D10).
//!
//! Like V4, the live source of `foreign_records_present` is `EventBody::ForeignSession`;
//! the `trace.toml` `foreign = true` path is kept **only** for the committed M0
//! fixture sets, which predate the daemon and carry no event stream.
//!
//! It also reports `unattributable_store`: a `file_first_record` store
//! whose file carries no teaching record has no partition key at all, so its records can
//! be neither attributed to the tracked session nor silently dropped.
//!
//! **The chain sweep.** The two checks above look at *sessions* and at a store's *first*
//! record. Neither one reads the records in between, so a store whose partition value can
//! vary **per record** (`PartitionSpec::Direct`) is swept here, record by record. Codex's
//! `history.jsonl` is exactly this shape: it is one shared, append-only log across every
//! thread on the machine, `partition = { strategy = "direct", path = "session_id" }`, and
//! the M0 fixture's own bytes measure three records (seq 6-8) stamped with the *other*
//! thread's session id (`01a06616-...`, the same session `wrong-cwd-01a06616` names)
//! landing in the tracked thread's `history` store. Each such record Fails.
//!
//! **What this check does not report.** A store declaring no `partition_key` is a manifest
//! property, not a per-run finding: it would say the same thing on every run of a given
//! connector whatever the bytes did. `ConnectorManifest::validate()` requires a store to
//! declare either a `partition_key` or an `unpartitioned_reason`: the connector author
//! states the judgement once, in words, and a manifest that states neither does not load at
//! all.

use super::{Check, CheckContext, CheckResult, Evidence, fail, flag, pass, pass_with};
use goalcup_protocol::envelope::EventBody;
use goalcup_protocol::events::ForeignScanOutcome;
use goalcup_protocol::manifest::PartitionSpec;

pub struct V10Partition;

impl Check for V10Partition {
    fn id(&self) -> &'static str {
        "V10"
    }
    fn title(&self) -> &'static str {
        "no record fails its store's partition key"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        let mut findings = Vec::new();

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
                    "partition evidence coverage for `{declaring_connector}` scan {scan_index} \
                     was unavailable ({})",
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

        for s in &cx.parsed.sessions {
            if !s.tracked {
                continue;
            }
            match &s.directory {
                Some(d) if d == &cx.trace.workspace => {}
                // Neither the session's own directory nor the trace's workspace is put in
                // the message — both are absolute machine paths on a real entry. The
                // session id and store name are citation enough for a reviewer to look the
                // record up.
                _ => findings.push(fail(
                    "partition_violation",
                    format!(
                        "tracked session {} in store `{}` does not report the tracked \
                         workspace directory (D20)",
                        s.id, s.store
                    ),
                    vec![Evidence::note(&s.store, format!("session {}", s.id))],
                )),
            }
        }

        // Sweep every record of every store whose manifest entry declares a `Direct`
        // partition. Unlike `FileFirstRecord` (the value is taught once, by the file's
        // first record) or `AggregateLearned` (the value is joined in through an aggregate
        // id the parser already resolves), a `Direct` partition's value is present on the
        // record itself, so it can vary per record — and does, on Codex's shared
        // `history.jsonl`.
        for manifest_store in &cx.manifest.store {
            let Some(PartitionSpec::Direct { path }) = &manifest_store.partition else {
                continue;
            };
            let Some(store) = cx.stores.iter().find(|s| s.name == manifest_store.name) else {
                continue;
            };
            for rec in &store.records {
                let Some(value) = crate::legs::dotted_through_strings(&rec.value, path) else {
                    findings.push(fail(
                        "partition_violation",
                        format!(
                            "store `{}` record seq {} has no string value at its declared \
                             partition field `{path}` (D20)",
                            store.name, rec.seq
                        ),
                        vec![Evidence::at(
                            &store.name,
                            rec.seq,
                            Some(rec.hash.clone()),
                            "declared partition field is missing or not a string",
                        )],
                    ));
                    continue;
                };
                if cx
                    .trace
                    .tracked_sessions
                    .iter()
                    .any(|s| s.as_str() == value.as_ref())
                {
                    continue;
                }
                // The field is documented (Direct) as a thread/session id, never a path
                // (`connectors/codex-cli/manifest.toml`'s own comment); guarded anyway,
                // since a finding must never carry an absolute machine path regardless of
                // what a future connector's `Direct` field happens to name.
                let shown = if value.starts_with('/') {
                    "<path>".to_string()
                } else {
                    value.to_string()
                };
                findings.push(fail(
                    "partition_violation",
                    format!(
                        "store `{}` record seq {} carries `{path}` = `{shown}`, not one of \
                         the tracked session(s) (D20)",
                        store.name, rec.seq
                    ),
                    vec![Evidence::at(
                        &store.name,
                        rec.seq,
                        Some(rec.hash.clone()),
                        "partition field does not match the tracked session",
                    )],
                ));
            }
        }

        // Live source: the daemon's foreign-harness watcher (D10).
        let mut live = 0usize;
        for env in cx.events {
            if let EventBody::ForeignSession {
                harness,
                session_id,
                ..
            } = &env.body
            {
                live += 1;
                // Pass-level, evidence and all. This says records were
                // correctly partitioned away: a foreign harness wrote in the workspace and
                // its records are not in this entry's chain. A record that actually
                // reached the chain in breach of its key is `partition_violation` above,
                // and that is still a Fail.
                findings.push(pass_with(
                    "foreign_records_present",
                    format!(
                        "`{harness}` session `{session_id}` wrote inside a bound workspace; \
                         its records must be discarded before hashing (D20)"
                    ),
                    vec![Evidence::at(
                        "events",
                        env.link.seq,
                        Some(env.link.hash.clone()),
                        // The foreign harness's own absolute store path is never put in a
                        // finding. The session id is already in the message; the note
                        // repeats it rather than the path (D20: identifiers only, never
                        // content, never a path).
                        format!("session {session_id}"),
                    )],
                ));
            }
        }
        // Fixture path: only for the committed M0 sets.
        if live == 0 {
            for spec in cx.trace.leg1.iter().filter(|s| s.foreign) {
                findings.push(pass_with(
                    "foreign_records_present",
                    format!(
                        "store `{}` is declared foreign; its records must be discarded before hashing (D20)",
                        spec.name
                    ),
                    vec![Evidence::note(
                        &spec.name,
                        "declared foreign in trace.toml — a fixture declaration, not a \
                         live detection",
                    )],
                ));
            }
        }

        // A file_first_record store whose file has no teaching record has no key.
        // It is neither attributed nor silently dropped; it is named.
        //
        // The connector's file-first-record spec, if it declares one. Codex does; Claude
        // Code declares no partition at all, so on it this whole block is inert.
        let first_record_spec = cx.manifest.store.iter().find_map(|s| match &s.partition {
            Some(PartitionSpec::FileFirstRecord {
                select_path,
                select_value,
                ..
            }) => Some((select_path.as_str(), select_value.as_str())),
            _ => None,
        });
        for spec in &cx.trace.leg1 {
            let Some((select_path, select_value)) = first_record_spec else {
                break;
            };
            let Some(store) = cx.stores.iter().find(|s| s.name == spec.name) else {
                continue;
            };
            if store.records.is_empty() {
                continue;
            }
            // A trace store whose name exactly matches a manifest store with a *different*
            // strategy is governed by that strategy instead — Codex's `history` is a flat
            // `direct` store and has no first-record header by design.
            let governed_otherwise = cx.manifest.store.iter().any(|s| {
                s.name == spec.name
                    && !matches!(&s.partition, Some(PartitionSpec::FileFirstRecord { .. }))
            });
            if governed_otherwise {
                continue;
            }
            let teaches = crate::legs::dotted(&store.records[0].value, select_path)
                .and_then(|v| v.as_str())
                == Some(select_value);
            if !teaches {
                findings.push(flag(
                    "unattributable_store",
                    format!(
                        "this connector keys a file on its first `{select_value}`, and store \
                         `{}` has none, so the file has no key: its records belong to no \
                         session that can be established from its own bytes. They are \
                         loaded and chained as evidence and attributed to nothing",
                        spec.name
                    ),
                    vec![Evidence::at(
                        &spec.name,
                        store.records[0].seq,
                        Some(store.records[0].hash.clone()),
                        "no teaching record",
                    )],
                ));
            }
        }

        if findings.is_empty() {
            findings.push(pass(
                "partitioned",
                "every record satisfies its partition key".to_string(),
            ));
        }
        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{Severity, support};
    use crate::legs::{Leg1Record, Leg1Store};
    use crate::parsers::SessionFact;
    use goalcup_protocol::chain::Link;
    use goalcup_protocol::envelope::Envelope;

    fn env(seq: u64, body: EventBody) -> Envelope {
        Envelope {
            protocol_version: 1,
            entry_id: "test-entry".to_string(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T00:00:00Z".to_string(),
            link: Link {
                seq,
                prev: "0".repeat(64),
                hash: "h".repeat(64),
            },
            body,
            redaction: None,
        }
    }

    /// None of the fixtures ever fails its own partition key — asserted in
    /// `checks_v6_v11.rs`. This constructs the violation directly, and checks that neither
    /// the session's real directory nor the trace's workspace (both absolute machine paths)
    /// ends up in the finding, per the binding constraint.
    #[test]
    fn a_tracked_session_with_the_wrong_directory_is_a_partition_violation_naming_no_path() {
        let mut trace = support::trace("claude-code-cli");
        trace.workspace = "/Users/real/workspace".to_string();
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.sessions.push(SessionFact {
            id: "s1".to_string(),
            store: "main".to_string(),
            directory: Some("/Users/real/somewhere-else".to_string()),
            parent: None,
            thread_source: "user".to_string(),
            is_root: true,
            tracked: true,
        });
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
        let result = V10Partition.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        let f = result
            .findings
            .iter()
            .find(|f| f.code == "partition_violation")
            .expect("a tracked session outside the workspace must fail");
        assert!(f.message.contains("s1"));
        assert!(
            !f.message.contains("/Users/"),
            "leaks a path: {}",
            f.message
        );
        for e in &f.evidence {
            assert!(!e.note.contains("/Users/"), "leaks a path: {}", e.note);
        }
    }

    /// A tracked session with no directory at all (never seen a `cwd`-bearing record)
    /// fails the same way, not with a panic on a missing `Some`.
    #[test]
    fn a_tracked_session_with_no_directory_at_all_is_also_a_violation() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let mut parsed = support::parsed();
        parsed.sessions.push(SessionFact {
            id: "s1".to_string(),
            store: "main".to_string(),
            directory: None,
            parent: None,
            thread_source: "user".to_string(),
            is_root: true,
            tracked: true,
        });
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
        let result = V10Partition.run(&cx);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == "partition_violation")
        );
    }

    /// The live source (`EventBody::ForeignSession`) takes precedence over the fixture
    /// declaration and must not leak the foreign harness's own absolute store path.
    #[test]
    fn a_live_foreign_session_event_is_reported_without_its_store_path() {
        let trace = support::trace("claude-code-cli");
        let manifest = support::manifest("claude-code-cli");
        let parsed = support::parsed();
        let human = support::human();
        let events = vec![env(
            1,
            EventBody::ForeignSession {
                harness: "codex-cli".to_string(),
                session_id: "foreign-1".to_string(),
                store_path: "/Users/real/.codex/sessions/foreign-1.jsonl".to_string(),
                first_seen: "2026-09-04T00:00:00Z".to_string(),
            },
        )];
        let cx = CheckContext {
            trace: &trace,
            manifest: &manifest,
            parsed: &parsed,
            human: &human,
            join_strategy: "prompt_id",
            stores: &[],
            events: &events,
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
        let result = V10Partition.run(&cx);
        let f = result
            .findings
            .iter()
            .find(|f| f.code == "foreign_records_present")
            .expect("the live event must be reported");
        assert!(f.message.contains("foreign-1"));
        assert!(
            !f.message.contains("/Users/"),
            "leaks a path: {}",
            f.message
        );
        for e in &f.evidence {
            assert!(!e.note.contains("/Users/"), "leaks a path: {}", e.note);
        }
    }

    fn leg1_record(seq: u64, value: serde_json::Value) -> Leg1Record {
        Leg1Record {
            store: "history".to_string(),
            seq,
            prev: "0".repeat(64),
            hash: format!("{seq:0>64}"),
            recorded_at: None,
            value,
            raw: Vec::new(),
        }
    }

    /// A `Direct`-partition store's records are swept one by one — a record whose partition
    /// field names a session this run never tracked is a `partition_violation`, cited by
    /// store and seq, even though the file's session facts (built from a *different* store)
    /// never notice it.
    #[test]
    fn a_direct_partition_store_is_swept_record_by_record() {
        let mut trace = support::trace("codex-cli");
        trace.leg1.push(crate::trace::Leg1Spec {
            name: "history".to_string(),
            kind: crate::trace::StoreKind::RawJsonl,
            path: crate::trace_fs::TracePath::parse("test.jsonl").unwrap(),
            foreign: false,
            scrubbed: false,
            expected_reread: false,
        });
        trace.tracked_sessions = vec!["tracked-session".to_string()];
        let manifest = support::manifest("codex-cli");
        let parsed = support::parsed();
        let human = support::human();
        let stores = vec![Leg1Store::from_records(
            "history",
            crate::trace::StoreKind::RawJsonl,
            crate::legs::SeqSpace::LocalPerStore,
            vec![
                leg1_record(1, serde_json::json!({"session_id": "tracked-session"})),
                leg1_record(2, serde_json::json!({"session_id": "some-other-session"})),
                leg1_record(3, serde_json::json!({"session_id": "tracked-session"})),
            ],
        )];
        let cx = CheckContext {
            trace: &trace,
            manifest: &manifest,
            parsed: &parsed,
            human: &human,
            join_strategy: "turn_id+order",
            stores: &stores,
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
        let result = V10Partition.run(&cx);
        assert_eq!(result.verdict, Severity::Fail);
        let violations: Vec<_> = result
            .findings
            .iter()
            .filter(|f| f.code == "partition_violation")
            .collect();
        assert_eq!(violations.len(), 1, "{:?}", result.findings);
        assert!(violations[0].message.contains("history"));
        assert!(violations[0].message.contains("seq 2"));
        assert!(violations[0].message.contains("some-other-session"));
        assert_eq!(violations[0].evidence[0].seq, Some(2));
    }

    /// A connector that declares a direct partition key makes that key mandatory on
    /// every record. Missing and non-string values cannot be silently skipped: either
    /// shape would otherwise let a foreign record evade the partition sweep.
    #[test]
    fn a_direct_partition_field_that_is_missing_or_not_a_string_fails_closed() {
        let mut trace = support::trace("codex-cli");
        trace.tracked_sessions = vec!["tracked-session".to_string()];
        let manifest = support::manifest("codex-cli");
        let parsed = support::parsed();
        let human = support::human();
        let stores = vec![Leg1Store::from_records(
            "history",
            crate::trace::StoreKind::RawJsonl,
            crate::legs::SeqSpace::LocalPerStore,
            vec![
                leg1_record(7, serde_json::json!({"message": "missing"})),
                leg1_record(
                    8,
                    serde_json::json!({"session_id": ["/Users/secret/session"]}),
                ),
            ],
        )];
        let cx = CheckContext {
            trace: &trace,
            manifest: &manifest,
            parsed: &parsed,
            human: &human,
            join_strategy: "turn_id+order",
            stores: &stores,
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

        let result = V10Partition.run(&cx);
        let violations: Vec<_> = result
            .findings
            .iter()
            .filter(|f| f.code == "partition_violation")
            .collect();
        assert_eq!(result.verdict, Severity::Fail);
        assert_eq!(violations.len(), 2, "{:?}", result.findings);
        assert_eq!(violations[0].evidence[0].seq, Some(7));
        assert_eq!(violations[1].evidence[0].seq, Some(8));
        for finding in violations {
            assert!(finding.message.contains("history"));
            assert!(!finding.message.contains("/Users/"));
            assert!(!finding.evidence[0].note.contains("/Users/"));
            assert!(finding.evidence[0].sha256.is_some());
        }
    }
}
