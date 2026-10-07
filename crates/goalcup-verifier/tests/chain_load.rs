//! The verifier's one chain loader, against a `chain.jsonl` the protocol crate wrote.
//!
//! Every chain here comes out of `TraceExport`, never out of a literal: the loader's
//! whole job is to read the bytes the daemon's writer produces, and a test that built its
//! own lines would be asserting agreement between two things written in this file.

use goalcup_protocol::chain::{GENESIS, Link};
use goalcup_protocol::envelope::{
    Envelope, EnvelopeVersionCompatibilityError, EventBody, V2OnlyEventBody,
};
use goalcup_protocol::events::ForeignScanOutcome;
use goalcup_protocol::export::TraceExport;
use goalcup_protocol::provenance::{
    CaptureUnavailableV1, CutoffReachedV1, FinalizationV1, NoRepositoryReasonV1,
    ObservedRepositoryStateV1, PortableRelativePathOrRoot, PreCutoffDrainOutcomeV1,
    PublisherNotAttemptedReasonV1, PublisherNotAttemptedV1, RepositoryNonSelectionV1,
    RepositoryObservedV1, RepositoryTransitionV1, Rfc3339Utc, Sha256Digest, SnapshotNotAttemptedV1,
    TerminationOutcomeV1,
};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan, RedactionRecord};
use goalcup_verifier::chain::{
    ChainBreakReason, EnvelopeCompatibilityIssue, RedactionAuditIssueReason, load_and_verify_text,
};
use goalcup_verifier::legs::SeqSpace;

const HISTORICAL_V1_HOME_SEEDED: &str =
    include_str!("../../goalcup-protocol/tests/fixtures/historical-v1-home-seeded.jsonl");
const HISTORICAL_V1_HOME_SEEDED_HASH: &str =
    "5658cadb5d2aa41e5978bc139d9167b7097f9e2cc737404847e2a35d46a0064a";

fn load_and_verify(
    path: &std::path::Path,
) -> Result<goalcup_verifier::chain::VerifiedChain, goalcup_verifier::chain::ChainError> {
    let text = std::fs::read_to_string(path).map_err(|source| {
        goalcup_verifier::chain::ChainError::Io {
            path: path.display().to_string(),
            source,
        }
    })?;
    load_and_verify_text(&path.display().to_string(), &text)
}

fn write_chain(dir: &std::path::Path) -> Vec<Envelope> {
    let mut x = TraceExport::create(dir).unwrap();
    let mut out = Vec::new();
    let bodies = vec![
        (
            "main",
            EventBody::Leg1Record {
                store: "main".to_string(),
                payload: serde_json::Value::String(
                    r#"{"type":"user","sessionId":"s1","message":{"content":"go"}}"#.to_string(),
                ),
            },
        ),
        (
            "hooks",
            EventBody::Leg2Post {
                event: "UserPromptSubmit".to_string(),
                payload: serde_json::json!({"prompt": "go", "prompt_id": "p1"}),
                path: None,
                receiver_meta: None,
            },
        ),
        (
            "daemon",
            EventBody::Heartbeat {
                ts: "2026-09-04T00:01:00Z".to_string(),
                stores_seen: Vec::new(),
                receiver_ok: true,
            },
        ),
    ];
    for (i, (source, body)) in bodies.into_iter().enumerate() {
        let mut e = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: "e1".to_string(),
            source: source.to_string(),
            recorded_at: format!("2026-09-04T00:00:0{i}Z"),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body,
            redaction: None,
        };
        x.append_chain(&mut e).unwrap();
        out.push(e);
    }
    out
}

fn append(
    export: &mut TraceExport,
    protocol_version: u32,
    source: &str,
    body: EventBody,
    redaction: Option<RedactionAudit>,
) -> Envelope {
    let mut env = Envelope {
        protocol_version,
        entry_id: "e1".to_string(),
        source: source.to_string(),
        recorded_at: "2026-09-04T00:00:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body,
        redaction,
    };
    export.append_chain(&mut env).unwrap();
    env
}

fn append_correctly_linked(
    path: &std::path::Path,
    previous: &Envelope,
    mut env: Envelope,
) -> Envelope {
    use std::io::Write as _;

    env.link.seq = previous.link.seq + 1;
    env.link.prev = previous.link.hash.clone();
    env.link.hash = env.expected_hash(&env.link.prev).unwrap();
    let mut line = serde_json::to_vec(&env).unwrap();
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(&line)
        .unwrap();
    env
}

#[test]
fn a_maximum_sequence_envelope_is_a_deterministic_structural_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let mut envelope = Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-04T00:00:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::Heartbeat {
            ts: "2026-09-04T00:00:00Z".to_string(),
            stores_seen: Vec::new(),
            receiver_ok: true,
        },
        redaction: None,
    };
    export.append_chain(&mut envelope).unwrap();
    drop(export);

    envelope.link.seq = u64::MAX;
    envelope.link.prev = goalcup_protocol::chain::GENESIS.to_string();
    envelope.link.hash = envelope.expected_hash(&envelope.link.prev).unwrap();
    std::fs::write(
        tmp.path().join(goalcup_protocol::export::CHAIN_FILE),
        format!("{}\n", serde_json::to_string(&envelope).unwrap()),
    )
    .unwrap();

    let report = goalcup_verifier::chain::load_and_verify_reader(
        "chain.jsonl",
        std::fs::File::open(tmp.path().join(goalcup_protocol::export::CHAIN_FILE)).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        report.break_at,
        Some(goalcup_verifier::chain::ChainBreak {
            reason: goalcup_verifier::chain::ChainBreakReason::SeqOutOfOrder { expected: 0 },
            ..
        })
    ));
}

fn home_seed() -> goalcup_protocol::events::HomeSeedRecord {
    goalcup_protocol::events::HomeSeedRecord {
        handoff_generation: None,
        handoff_request_digest: None,
        connector: "fixture".to_string(),
        from: "{home}/settings.json".to_string(),
        keys: Vec::new(),
        keys_unchanged: Vec::new(),
        written: Vec::new(),
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
        wire_layout: Default::default(),
    }
}

/// Every null-bearing line below carries the hash recomputed over the equivalent omitted,
/// writer-produced shape.  A parser that conflates explicit JSON `null` with field absence
/// would therefore retain a forged-but-hash-valid prefix.
#[derive(Clone, Copy)]
enum HomeSeedNullField {
    HandoffGeneration,
    HandoffRequestDigest,
    KeysUnchanged,
    Written,
    KeyValue,
    WrittenKeyValue,
}

impl HomeSeedNullField {
    fn name(self) -> &'static str {
        match self {
            Self::HandoffGeneration => "handoff_generation",
            Self::HandoffRequestDigest => "handoff_request_digest",
            Self::KeysUnchanged => "keys_unchanged",
            Self::Written => "written",
            Self::KeyValue => "keys[].value",
            Self::WrittenKeyValue => "written[].value",
        }
    }
}

fn null_normalized_home_seed_lines() -> Vec<(&'static str, String)> {
    let legacy: Envelope = serde_json::from_str(HISTORICAL_V1_HOME_SEEDED).unwrap();
    let mut current = Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-10T00:00:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::HomeSeeded(home_seed()),
        redaction: None,
    };
    let EventBody::HomeSeeded(seed) = &mut current.body else {
        unreachable!("the test fixture is a HomeSeeded envelope");
    };
    seed.keys.push(goalcup_protocol::events::SeededKey {
        name: "theme".to_string(),
        value: None,
    });
    seed.written.push(goalcup_protocol::events::SeededKey {
        name: "workspace_trusted".to_string(),
        value: None,
    });

    [
        HomeSeedNullField::HandoffGeneration,
        HomeSeedNullField::HandoffRequestDigest,
        HomeSeedNullField::KeysUnchanged,
        HomeSeedNullField::Written,
    ]
    .into_iter()
    .map(|field| {
        (
            field.name(),
            line_with_explicit_null_and_normalized_hash(legacy.clone(), field),
        )
    })
    .chain(
        [
            HomeSeedNullField::KeyValue,
            HomeSeedNullField::WrittenKeyValue,
        ]
        .into_iter()
        .map(|field| {
            (
                field.name(),
                line_with_explicit_null_and_normalized_hash(current.clone(), field),
            )
        }),
    )
    .collect()
}

fn line_with_explicit_null_and_normalized_hash(
    mut normalized: Envelope,
    field: HomeSeedNullField,
) -> String {
    normalized.link.prev = GENESIS.to_string();
    normalized.link.hash = normalized.expected_hash(GENESIS).unwrap();
    let normalized_hash = normalized.link.hash.clone();
    let mut wire = serde_json::to_value(&normalized).unwrap();
    let content = wire["body"]["content"].as_object_mut().unwrap();

    match field {
        HomeSeedNullField::HandoffGeneration
        | HomeSeedNullField::HandoffRequestDigest
        | HomeSeedNullField::KeysUnchanged
        | HomeSeedNullField::Written => {
            assert!(content.get(field.name()).is_none());
            content.insert(field.name().to_string(), serde_json::Value::Null);
        }
        HomeSeedNullField::KeyValue => {
            let key = content
                .get_mut("keys")
                .and_then(serde_json::Value::as_array_mut)
                .unwrap()
                .first_mut()
                .unwrap()
                .as_object_mut()
                .unwrap();
            assert!(key.get("value").is_none());
            key.insert("value".to_string(), serde_json::Value::Null);
        }
        HomeSeedNullField::WrittenKeyValue => {
            let key = content
                .get_mut("written")
                .and_then(serde_json::Value::as_array_mut)
                .unwrap()
                .first_mut()
                .unwrap()
                .as_object_mut()
                .unwrap();
            assert!(key.get("value").is_none());
            key.insert("value".to_string(), serde_json::Value::Null);
        }
    }

    assert_eq!(wire["hash"], normalized_hash);
    let mut line = serde_json::to_string(&wire).unwrap();
    line.push('\n');
    line
}

fn repository_observed() -> RepositoryObservedV1 {
    RepositoryObservedV1 {
        transition: RepositoryTransitionV1::Disappeared,
        state: ObservedRepositoryStateV1::Disappeared {
            workspace_relative_root: PortableRelativePathOrRoot::try_from("").unwrap(),
        },
    }
}

fn cutoff() -> CutoffReachedV1 {
    CutoffReachedV1 {
        receipt_sha256: Sha256Digest::try_from("a".repeat(64)).unwrap(),
        deadline_at: Rfc3339Utc::try_from("2026-09-04T00:01:00Z").unwrap(),
        pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
    }
}

fn finalization() -> FinalizationV1 {
    FinalizationV1::CaptureUnavailable {
        receipt_sha256: Sha256Digest::try_from("a".repeat(64)).unwrap(),
        termination: TerminationOutcomeV1::AlreadyStopped {
            checked_at: Rfc3339Utc::try_from("2026-09-04T00:01:00Z").unwrap(),
        },
        capture: CaptureUnavailableV1::NotAttempted(SnapshotNotAttemptedV1 {
            selection: RepositoryNonSelectionV1::None {
                reason: NoRepositoryReasonV1::NotFound,
            },
        }),
        publisher: PublisherNotAttemptedV1 {
            reason: PublisherNotAttemptedReasonV1::SelectionFailed,
        },
    }
}

fn foreign_scan_coverage() -> EventBody {
    EventBody::ForeignScanCoverage {
        declaring_connector: "opencode".to_string(),
        declaring_manifest_sha256: "a".repeat(64),
        scan_index: 0,
        outcome: ForeignScanOutcome::Complete,
        at: "2026-09-04T00:01:00Z".to_string(),
    }
}

#[test]
fn v2_redaction_audit_semantics_are_reported_as_typed_issues() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let leg = || EventBody::Leg1Record {
        store: "main".to_string(),
        payload: serde_json::Value::String("<redacted:email>".to_string()),
    };
    let audit = |spans| Some(RedactionAudit { spans });
    let span = |kind: &str, start, end| RedactionAuditSpan {
        kind: kind.to_string(),
        start,
        end,
    };

    append(&mut x, 2, "main", leg(), audit(Vec::new()));
    append(&mut x, 2, "main", leg(), audit(vec![span("", 0, 1)]));
    append(&mut x, 2, "main", leg(), audit(vec![span("email", 2, 2)]));
    append(
        &mut x,
        2,
        "main",
        leg(),
        audit(vec![span("email", 2, 4), span("email", 3, 5)]),
    );
    append(
        &mut x,
        2,
        "daemon",
        EventBody::Heartbeat {
            ts: "2026-09-04T00:00:00Z".to_string(),
            stores_seen: Vec::new(),
            receiver_ok: true,
        },
        audit(vec![span("email", 0, 1)]),
    );
    drop(x);

    let c = load_and_verify(&tmp.path().join("chain.jsonl")).unwrap();
    assert_eq!(
        c.redaction_audit_issues
            .iter()
            .map(|i| i.reason.clone())
            .collect::<Vec<_>>(),
        vec![
            RedactionAuditIssueReason::EmptySpans,
            RedactionAuditIssueReason::EmptyKind { span: 0 },
            RedactionAuditIssueReason::EmptyOrReversedSpan { span: 0 },
            RedactionAuditIssueReason::UnorderedOrOverlappingSpan { span: 1 },
            RedactionAuditIssueReason::AuditOnNonLeg,
        ]
    );
}

/// V3 changed recovery-marker bytes, not the inline-redaction contract.  A V3 reader must
/// keep auditing a retained V2 line, its own newly written V3 line, and a V4 line;
/// V1 is the only legacy spelling with the salted side-log rule.
#[test]
fn v2_v3_and_v4_inline_redaction_audits_are_all_checked() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let body = || EventBody::Leg1Record {
        store: "main".to_string(),
        payload: serde_json::Value::String("<redacted:email>".to_string()),
    };
    let empty = || Some(RedactionAudit { spans: Vec::new() });
    append(&mut x, 2, "main", body(), empty());
    append(&mut x, 3, "main", body(), empty());
    append(&mut x, 4, "main", body(), empty());
    drop(x);

    let chain = load_and_verify(&tmp.path().join("chain.jsonl")).unwrap();
    assert_eq!(
        chain
            .redaction_audit_issues
            .iter()
            .map(|issue| (issue.seq, issue.reason.clone()))
            .collect::<Vec<_>>(),
        vec![
            (0, RedactionAuditIssueReason::EmptySpans),
            (1, RedactionAuditIssueReason::EmptySpans),
            (2, RedactionAuditIssueReason::EmptySpans),
        ]
    );
}

/// An older reader must stop at a future-version envelope before it recomputes a link over
/// a lossy deserialization.  That reports the honest unsupported boundary without inventing
/// a second HashMismatch for the same new bytes.
#[test]
fn unknown_v5_stops_hash_validation_at_the_supported_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    append(
        &mut x,
        goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
        "main",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("old but supported".to_string()),
        },
        None,
    );
    append(
        &mut x,
        goalcup_protocol::PROTOCOL_VERSION,
        "main",
        EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("new marker bytes".to_string()),
        },
        None,
    );
    drop(x);

    let path = tmp.path().join("chain.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    let rewritten = text.replacen(
        &format!(
            "\"protocol_version\":{}",
            goalcup_protocol::PROTOCOL_VERSION
        ),
        "\"protocol_version\":5",
        1,
    );
    std::fs::write(&path, rewritten).unwrap();

    let chain = load_and_verify(&path).unwrap();
    assert_eq!(chain.unsupported_version, Some((1, 5)));
    assert!(
        chain.break_at.is_none(),
        "unsupported bytes are not reserialized into a manufactured hash mismatch"
    );
    assert_eq!(chain.head_seq, 0, "the verified prefix ends before v4");
}

#[test]
fn unknown_v5_body_and_later_invalid_suffix_do_not_escape_the_supported_prefix() {
    use std::io::Write as _;

    let tmp = tempfile::tempdir().unwrap();
    let prefix = write_chain(tmp.path());
    let path = tmp.path().join("chain.jsonl");
    let mut chain = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(
        chain,
        r#"{{"protocol_version":5,"seq":3,"body":{{"kind":"future_only","content":{{"opaque":true}}}}}}"#
    )
    .unwrap();
    writeln!(chain, "this later suffix is not JSON").unwrap();
    drop(chain);

    let verified = load_and_verify(&path).unwrap();
    assert_eq!(verified.unsupported_version, Some((3, 5)));
    assert_eq!(verified.break_at, None);
    assert_eq!(verified.truncated_tail, None);
    assert_eq!(verified.envelopes, prefix);
    assert_eq!(verified.head_seq, 2);
    assert_eq!(verified.head_hash, prefix[2].link.hash);
    assert_eq!(verified.stores.len(), 1);
    assert_eq!(verified.stores[0].records.len(), 1);
    assert_eq!(verified.posts.posts.len(), 1);
    assert_eq!(verified.events.len(), 1);
    assert_eq!(verified.seal_link, None);
    assert_eq!(verified.baseline_link, None);
}

/// A V1 line as its original writer produced it.  The loader must keep its original
/// `keys_kept`/`sha256` layout long enough to verify the link rather than silently
/// retaining a hash-mismatched prefix after normalising it to the later record shape.
#[test]
fn load_and_verify_accepts_the_53d3019_home_seeded_wire_layout() {
    let verified =
        load_and_verify_text("historical-v1-home-seeded.jsonl", HISTORICAL_V1_HOME_SEEDED).unwrap();

    assert_eq!(verified.break_at, None);
    assert_eq!(verified.incompatible_envelope, None);
    assert_eq!(verified.head_seq, 0);
    assert_eq!(verified.head_hash, HISTORICAL_V1_HOME_SEEDED_HASH);
    assert_eq!(verified.envelopes.len(), 1);
    assert_eq!(
        verified.envelopes[0].expected_hash(GENESIS).unwrap(),
        HISTORICAL_V1_HOME_SEEDED_HASH
    );
}

/// Unknown or ambiguous `HomeSeeded` members are malformed evidence, not a prefix that a
/// loader may normalise into a different preimage and merely label hash-mismatched.
#[test]
fn load_and_verify_refuses_unknown_or_ambiguous_home_seeded_members_at_the_parse_boundary() {
    let digest = "a".repeat(64);
    let cases = [
        (
            "unknown home-seed member",
            HISTORICAL_V1_HOME_SEEDED.replacen(
                "\"connector\":\"fixture\"",
                "\"unexpected\":true,\"connector\":\"fixture\"",
                1,
            ),
        ),
        (
            "unknown seeded key member",
            HISTORICAL_V1_HOME_SEEDED.replacen(
                "\"name\":\"theme\",\"value\":\"dark\"",
                "\"name\":\"theme\",\"value\":\"dark\",\"unexpected\":true",
                1,
            ),
        ),
        (
            "unknown seeded file member",
            HISTORICAL_V1_HOME_SEEDED.replacen(
                "\"sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
                "\"sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"unexpected\":true",
                1,
            ),
        ),
        (
            "null legacy key list mixed into a later layout",
            HISTORICAL_V1_HOME_SEEDED.replacen(
                &format!(
                    "\"keys_kept\":[\"theme\"],\"files\":[{{\"name\":\"auth.json\",\"bytes\":3,\"sha256\":\"{digest}\"}}]"
                ),
                "\"keys_kept\":null,\"keys_unchanged\":[\"theme\"],\"written\":[],\"files\":[{\"name\":\"auth.json\",\"bytes\":3}]",
                1,
            ),
        ),
    ];

    for (name, text) in cases {
        assert!(
            load_and_verify_text(name, &text).is_err(),
            "{name} is rejected while parsing rather than retained as a bad hash"
        );
    }
}

/// These candidate lines keep a hash recomputed from the null-free serialized record, so a
/// loader must reject them at JSON parsing rather than normalize the source bytes away and
/// retain the forged prefix.
#[test]
fn load_and_verify_refuses_null_normalization_of_every_optional_home_seed_field() {
    for (name, text) in null_normalized_home_seed_lines() {
        assert!(
            load_and_verify_text(name, &text).is_err(),
            "{name} is rejected while parsing rather than retained as a normalized hash match"
        );
    }
}

#[test]
fn every_version_body_incompatibility_is_a_typed_prefix_boundary() {
    let historical = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(historical.path()).unwrap();
    let historical_home_seed = append(
        &mut export,
        goalcup_protocol::LEGACY_PROTOCOL_VERSION,
        "daemon",
        EventBody::HomeSeeded(home_seed()),
        None,
    );
    drop(export);
    let retained = load_and_verify(&historical.path().join("chain.jsonl")).unwrap();
    assert_eq!(retained.incompatible_envelope, None);
    assert_eq!(retained.envelopes, vec![historical_home_seed]);

    for (name, version, body, redaction, expected) in [
        (
            "v1 HomeSeeded handoff generation",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            {
                let mut seed = home_seed();
                seed.handoff_generation = Some(7);
                EventBody::HomeSeeded(seed)
            },
            None,
            EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                protocol_version: goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            },
        ),
        (
            "v1 HomeSeeded handoff digest",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            {
                let mut seed = home_seed();
                seed.handoff_request_digest = Some("ab".repeat(32));
                EventBody::HomeSeeded(seed)
            },
            None,
            EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                protocol_version: goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            },
        ),
        (
            "v2 HomeSeeded handoff generation",
            goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            {
                let mut seed = home_seed();
                seed.handoff_generation = Some(7);
                EventBody::HomeSeeded(seed)
            },
            None,
            EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                protocol_version: goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            },
        ),
        (
            "v2 HomeSeeded handoff digest",
            goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            {
                let mut seed = home_seed();
                seed.handoff_request_digest = Some("ab".repeat(32));
                EventBody::HomeSeeded(seed)
            },
            None,
            EnvelopeVersionCompatibilityError::HomeSeededRequiresV3 {
                protocol_version: goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            },
        ),
        (
            "v1 RepositoryObservedV1",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::RepositoryObservedV1(repository_observed()),
            None,
            EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                body: V2OnlyEventBody::RepositoryObservedV1,
            },
        ),
        (
            "v1 CutoffReachedV1",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::CutoffReachedV1(cutoff()),
            None,
            EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                body: V2OnlyEventBody::CutoffReachedV1,
            },
        ),
        (
            "v1 FinalizationV1",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::FinalizationV1(finalization()),
            None,
            EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                body: V2OnlyEventBody::FinalizationV1,
            },
        ),
        (
            "v1 ForeignScanCoverage",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            foreign_scan_coverage(),
            None,
            EnvelopeVersionCompatibilityError::BodyRequiresV2 {
                body: V2OnlyEventBody::ForeignScanCoverage,
            },
        ),
        (
            "v1 inline redaction",
            goalcup_protocol::LEGACY_PROTOCOL_VERSION,
            EventBody::Leg1Record {
                store: "main".to_string(),
                payload: serde_json::json!({"redacted": true}),
            },
            Some(RedactionAudit {
                spans: vec![RedactionAuditSpan {
                    kind: "email".to_string(),
                    start: 0,
                    end: 1,
                }],
            }),
            EnvelopeVersionCompatibilityError::InlineRedactionRequiresV2,
        ),
        (
            "v2 legacy redaction body",
            goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            EventBody::Redaction(RedactionRecord {
                store: "main".to_string(),
                target_seq: 0,
                salt: "legacy".to_string(),
                spans: Vec::new(),
            }),
            None,
            EnvelopeVersionCompatibilityError::LegacyRedactionRequiresV1 {
                protocol_version: goalcup_protocol::PREVIOUS_PROTOCOL_VERSION,
            },
        ),
        (
            "v3 legacy redaction body",
            goalcup_protocol::PROTOCOL_VERSION,
            EventBody::Redaction(RedactionRecord {
                store: "main".to_string(),
                target_seq: 0,
                salt: "legacy".to_string(),
                spans: Vec::new(),
            }),
            None,
            EnvelopeVersionCompatibilityError::LegacyRedactionRequiresV1 {
                protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            },
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let mut export = TraceExport::create(tmp.path()).unwrap();
        let prefix = append(
            &mut export,
            goalcup_protocol::PROTOCOL_VERSION,
            "main",
            EventBody::Leg1Record {
                store: "main".to_string(),
                payload: serde_json::json!({"prefix": true}),
            },
            None,
        );
        drop(export);
        let path = tmp.path().join("chain.jsonl");
        let incompatible = append_correctly_linked(
            &path,
            &prefix,
            Envelope {
                protocol_version: version,
                entry_id: "e1".to_string(),
                source: "daemon".to_string(),
                recorded_at: "2026-09-04T00:00:01Z".to_string(),
                link: Link {
                    seq: 0,
                    prev: String::new(),
                    hash: String::new(),
                },
                body,
                redaction,
            },
        );
        append_correctly_linked(
            &path,
            &incompatible,
            Envelope {
                protocol_version: goalcup_protocol::PROTOCOL_VERSION,
                entry_id: "e1".to_string(),
                source: "main".to_string(),
                recorded_at: "2026-09-04T00:00:02Z".to_string(),
                link: Link {
                    seq: 0,
                    prev: String::new(),
                    hash: String::new(),
                },
                body: EventBody::Leg1Record {
                    store: "main".to_string(),
                    payload: serde_json::json!({"after": "must be ignored"}),
                },
                redaction: None,
            },
        );

        let verified = load_and_verify(&path).unwrap();
        assert_eq!(
            verified.incompatible_envelope,
            Some(EnvelopeCompatibilityIssue {
                seq: 1,
                reason: expected,
            }),
            "{name}"
        );
        assert_eq!(verified.envelopes, vec![prefix]);
        assert_eq!(verified.head_seq, 0);
        assert_eq!(verified.stores.len(), 1);
        assert_eq!(verified.stores[0].records.len(), 1);
        assert!(verified.posts.posts.is_empty());
        assert!(verified.events.is_empty());
        assert_eq!(verified.unsupported_version, None);
        assert_eq!(verified.truncated_tail, None);
    }
}

#[test]
fn only_an_exact_unpaired_legacy_marker_is_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let legacy = |text: &str| EventBody::Leg1Record {
        store: "main".to_string(),
        payload: serde_json::Value::String(format!(r#"{{"text":"{text}"}}"#)),
    };
    append(
        &mut x,
        1,
        "main",
        legacy("<redacted:email:0123456789abcdef>"),
        None,
    );
    append(
        &mut x,
        1,
        "main",
        legacy("<redacted:email:fedcba9876543210>"),
        None,
    );
    append(
        &mut x,
        1,
        "main",
        EventBody::Redaction(RedactionRecord {
            store: "main".to_string(),
            target_seq: 1,
            salt: "legacy".to_string(),
            spans: Vec::new(),
        }),
        None,
    );
    append(&mut x, 2, "main", legacy("<redacted:email>"), None);
    append(
        &mut x,
        2,
        "main",
        legacy("<redacted:email:0123456789ABCDEF>"),
        None,
    );
    append(
        &mut x,
        1,
        "hooks",
        EventBody::Leg2Post {
            event: "UserPromptSubmit".to_string(),
            payload: serde_json::json!({"<redacted:email:1111111111111111>": "literal"}),
            path: None,
            receiver_meta: None,
        },
        None,
    );
    append(
        &mut x,
        1,
        "hooks",
        EventBody::Leg2Post {
            event: "UserPromptSubmit".to_string(),
            payload: serde_json::json!({"<redacted:email:2222222222222222>": "literal"}),
            path: None,
            receiver_meta: None,
        },
        None,
    );
    append(
        &mut x,
        1,
        "hooks",
        EventBody::Redaction(RedactionRecord {
            store: "hooks".to_string(),
            target_seq: 6,
            salt: "legacy".to_string(),
            spans: Vec::new(),
        }),
        None,
    );
    drop(x);

    let c = load_and_verify(&tmp.path().join("chain.jsonl")).unwrap();
    assert_eq!(c.legacy_redaction_log_missing.len(), 2);
    assert_eq!(c.legacy_redaction_log_missing[0].seq, 0);
    assert_eq!(c.legacy_redaction_log_missing[0].store, "main");
    assert_eq!(c.legacy_redaction_log_missing[1].seq, 5);
    assert_eq!(c.legacy_redaction_log_missing[1].store, "hooks");
}

#[test]
fn a_good_chain_splits_by_store_and_unwraps_the_harness_line_once() {
    let tmp = tempfile::tempdir().unwrap();
    write_chain(tmp.path());
    let c = load_and_verify(&tmp.path().join("chain.jsonl")).unwrap();

    assert_eq!(c.entry_id, "e1");
    assert_eq!((c.head_seq, c.envelopes.len()), (2, 3));
    assert_eq!(c.genesis_link, c.envelopes[0].link.hash);
    assert!(c.break_at.is_none());
    assert_eq!(c.truncated_tail, None);
    assert_eq!(c.unsupported_version, None);

    assert_eq!(c.stores.len(), 1);
    let main = &c.stores[0];
    assert_eq!(main.name, "main");
    assert_eq!(
        main.space,
        SeqSpace::EntryChain,
        "chain records share the entry's seq space"
    );
    assert_eq!(
        main.records[0].seq, 0,
        "the envelope's seq, not a per-store counter"
    );
    // Unwrapped once: the parser sees the harness record, not a wrapper.
    assert_eq!(main.records[0].value["type"], "user");
    assert_eq!(main.records[0].value["sessionId"], "s1");

    assert_eq!(c.posts.posts.len(), 1);
    assert_eq!(c.posts.posts[0].event, "UserPromptSubmit");
    assert_eq!(c.posts.posts[0].body["prompt_id"], "p1");

    assert_eq!(
        c.events.len(),
        1,
        "the heartbeat is neither a record nor a post"
    );
    assert!(matches!(c.events[0].body, EventBody::Heartbeat { .. }));
}

#[test]
fn deleting_a_record_is_reported_at_the_first_break_and_is_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    write_chain(tmp.path());
    let path = tmp.path().join("chain.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    let kept: Vec<&str> = text
        .lines()
        .enumerate()
        .filter(|(i, _)| *i != 1)
        .map(|(_, l)| l)
        .collect();
    std::fs::write(&path, kept.join("\n") + "\n").unwrap();

    // The attack: one deleted line, every surviving hash
    // still recomputes against its own stored `prev`.
    let c = load_and_verify(&path).expect("a broken chain still yields a chain, not an error");
    let b = c.break_at.expect("the break must be reported");
    assert_eq!(b.seq, 2);
    assert_eq!(b.reason, ChainBreakReason::SeqOutOfOrder { expected: 1 });
    assert_eq!(c.head_seq, 2, "the loader still reports what it read");
}

#[test]
fn a_correctly_linked_foreign_entry_id_stops_before_the_forged_suffix() {
    let tmp = tempfile::tempdir().unwrap();
    let written = write_chain(tmp.path());
    let previous = written.last().unwrap();
    let forged = Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: "foreign-entry".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-04T00:01:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::ForeignSession {
            harness: "codex-cli".to_string(),
            session_id: "must-not-be-retained".to_string(),
            store_path: "/Users/secret/foreign.jsonl".to_string(),
            first_seen: "2026-09-04T00:01:00Z".to_string(),
        },
        redaction: None,
    };
    let forged = append_correctly_linked(&tmp.path().join("chain.jsonl"), previous, forged);

    let verified = load_and_verify(&tmp.path().join("chain.jsonl")).unwrap();
    let chain_break = verified.break_at.expect("mixed entry ids must break");
    assert_eq!(chain_break.seq, forged.link.seq);
    assert_eq!(chain_break.reason, ChainBreakReason::EntryIdMismatch);
    assert_eq!(chain_break.expected, "e1");
    assert_eq!(chain_break.observed, "foreign-entry");
    assert_eq!(verified.entry_id, "e1");
    assert_eq!(verified.envelopes.len(), written.len());
    assert_eq!(verified.head_seq, previous.link.seq);
    assert_eq!(verified.head_hash, previous.link.hash);
    assert!(verified.events.iter().all(|env| {
        !matches!(
            &env.body,
            EventBody::ForeignSession { session_id, .. }
                if session_id == "must-not-be-retained"
        )
    }));
}

#[test]
fn a_rewritten_body_breaks_on_the_recomputed_hash() {
    let tmp = tempfile::tempdir().unwrap();
    write_chain(tmp.path());
    let path = tmp.path().join("chain.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines[0] = lines[0].replace("\\\"content\\\":\\\"go\\\"", "\\\"content\\\":\\\"stop\\\"");
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();

    let c = load_and_verify(&path).unwrap();
    let b = c.break_at.expect("a rewritten body must break the chain");
    assert_eq!(b.seq, 0);
    assert_eq!(b.reason, ChainBreakReason::HashMismatch);
    assert_ne!(b.expected, b.observed);
}

/// `seal_link` is stated by the loader and judged by nobody here: V13 is what decides that
/// its absence on a sealed trace is `seal_not_chained`. Both readings are asserted, because
/// a loader that always answered `None` would pass a test that only sealed.
#[test]
fn the_seal_link_is_the_seq_of_the_seal_and_is_none_when_nothing_sealed() {
    let unsealed = tempfile::tempdir().unwrap();
    write_chain(unsealed.path());
    let c = load_and_verify(&unsealed.path().join("chain.jsonl")).unwrap();
    assert_eq!(c.seal_link, None, "nothing sealed this chain");

    let sealed = tempfile::tempdir().unwrap();
    write_chain(sealed.path());
    let mut x = TraceExport::resume(sealed.path()).unwrap();
    let (_, link) = x
        .write_seal(&goalcup_protocol::seal::SealManifest {
            entry_id: "e1".to_string(),
            sealed_at: "2026-09-08T00:00:00Z".to_string(),
            workspace: "/ws".to_string(),
            files: Vec::new(),
            excluded: Vec::new(),
        })
        .unwrap();
    drop(x);

    let c = load_and_verify(&sealed.path().join("chain.jsonl")).unwrap();
    assert_eq!(c.seal_link, Some(link.seq));
    assert_eq!(c.seal_link, Some(c.head_seq), "the seal is the last link");
    assert!(c.break_at.is_none(), "sealing does not break the chain");
    assert!(
        matches!(
            c.events.last().map(|e| &e.body),
            Some(EventBody::Seal { .. })
        ),
        "the seal is neither a leg-1 record nor a leg-2 post"
    );
}

/// `baseline_link` and the digest beside it are stated by the loader and judged by nobody
/// here: V13 is what decides that a chain whose seq 0 is not a baseline is
/// `baseline_not_chained`. Both readings are asserted, because a loader that always
/// answered `None` would pass a test that only wrote a baseline.
#[test]
fn the_baseline_link_is_the_seq_of_the_baseline_and_the_digest_it_claims() {
    let none = tempfile::tempdir().unwrap();
    write_chain(none.path());
    let c = load_and_verify(&none.path().join("chain.jsonl")).unwrap();
    assert_eq!(c.baseline_link, None, "this chain declares no baseline");
    assert_eq!(c.baseline_digest, None);

    let with = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(with.path()).unwrap();
    let m = goalcup_protocol::seal::SealManifest {
        entry_id: "e1".to_string(),
        sealed_at: "2026-09-04T00:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: Vec::new(),
        excluded: Vec::new(),
    };
    let (_, link) = x.write_baseline(&m).unwrap();
    drop(x);

    let c = load_and_verify(&with.path().join("chain.jsonl")).unwrap();
    assert_eq!(c.baseline_link, Some(0), "a baseline is the first link");
    assert_eq!(c.baseline_link, Some(link.seq));
    // The digest the link claims, read off the link. V13 compares it against the digest
    // of the manifest it loaded; the loader states it and compares nothing.
    assert_eq!(
        c.baseline_digest.as_deref(),
        Some(
            goalcup_protocol::export::manifest_sha256(&m)
                .unwrap()
                .as_str()
        )
    );
    assert_eq!(
        c.genesis_link, link.hash,
        "the baseline IS the genesis link a receipt witnesses"
    );
    assert!(c.break_at.is_none());
}

/// A daemon killed mid-write leaves bytes with no newline after them. That is a fact about
/// the file, reported as one — not a parse error that would cost the trace its verdict.
#[test]
fn a_half_written_final_line_is_reported_and_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    write_chain(tmp.path());
    let path = tmp.path().join("chain.jsonl");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("{\"protocol_version\":1,\"entry");
    std::fs::write(&path, &text).unwrap();

    let c = load_and_verify(&path).unwrap();
    assert_eq!(c.truncated_tail, Some(28));
    assert_eq!(c.head_seq, 2, "the whole lines still linked");
    assert!(c.break_at.is_none());
}

/// The store a record belongs to is the body's `store`, never the envelope's `source`.
///
/// A real daemon emits every leg-1 record with `source: "daemon"` — `source` names the
/// component that observed the line, and the daemon is what tails the harness's files.
/// Reading the store name off it would collapse every store in the entry into one called
/// `daemon`, so a manifest's per-store partition key, a heartbeat's per-store cursor and a
/// `TailNotice`'s `store` would all name stores the verifier did not have.
#[test]
fn a_records_store_is_the_bodys_store_and_never_the_emitting_component() {
    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    for (i, store) in ["main", "subagent", "main"].into_iter().enumerate() {
        let mut e = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: "e1".to_string(),
            // Every one of them emitted by the daemon, as a real capture emits them.
            source: "daemon".to_string(),
            recorded_at: format!("2026-09-04T00:00:0{i}Z"),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Leg1Record {
                store: store.to_string(),
                payload: serde_json::Value::String(format!(r#"{{"type":"user","n":{i}}}"#)),
            },
            redaction: None,
        };
        x.append_chain(&mut e).unwrap();
    }
    drop(x);

    let c = load_and_verify(&tmp.path().join("chain.jsonl")).unwrap();
    let names: Vec<&str> = c.stores.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["main", "subagent"],
        "two stores in first-appearance order, not one store called `daemon`"
    );
    assert_eq!(c.stores[0].records.len(), 2);
    assert_eq!(c.stores[1].records.len(), 1);
    // And the record itself carries the same name, so a finding that cites `store` cites
    // something the descriptor and the manifest can name too.
    assert_eq!(c.stores[1].records[0].store, "subagent");
    assert_eq!(c.stores[0].records[1].seq, 2, "the entry chain's own seq");
}

/// The transport survives the chain, so one submission on two hook sources is
/// not read as one source replaying a submission.
///
/// Claude Code carries every `UserPromptSubmit` over both the launch settings' `http` hook
/// (`/UserPromptSubmit`) and the project settings' `curl` shim (`/UserPromptSubmit-cmd`),
/// with the same `X-GoalCup-Event` header on both. The daemon chains the header's name —
/// which is right, that is the event — and `EventBody::Leg2Post.path` keeps the `-cmd`:
/// without it `Leg2Post::transport()` would read both copies as `http`, the parser's
/// transport dedupe would never fire, and the second copy would become a `duplicate_pairing`
/// Fail.
#[test]
fn a_chained_post_keeps_the_transport_that_carried_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(dir.path()).unwrap();
    for (at, path) in [
        ("2026-09-04T00:00:00Z", None),
        (
            "2026-09-04T00:00:01Z",
            Some("/UserPromptSubmit-cmd".to_string()),
        ),
    ] {
        let mut e = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: "e1".to_string(),
            source: "hooks".to_string(),
            recorded_at: at.to_string(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Leg2Post {
                event: "UserPromptSubmit".to_string(),
                payload: serde_json::json!({"prompt": "go", "prompt_id": "p1"}),
                path,
                receiver_meta: None,
            },
            redaction: None,
        };
        x.append_chain(&mut e).unwrap();
    }
    let loaded = load_and_verify(&dir.path().join("chain.jsonl")).unwrap();
    let posts = &loaded.posts.posts;
    assert_eq!(posts.len(), 2);
    // Both are the same event — the name the header gave — and two transports.
    assert_eq!(posts[0].event, "UserPromptSubmit");
    assert_eq!(posts[1].event, "UserPromptSubmit");
    assert_eq!(
        posts[0].transport(),
        "http",
        "a post with no path is the http hook"
    );
    assert_eq!(
        posts[1].transport(),
        "cmd",
        "and the shim's path is what says so: {}",
        posts[1].path
    );
}
