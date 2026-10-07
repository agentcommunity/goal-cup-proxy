//! Gap classification (D9) and seal attribution (D11).
//!
//!   * `Gap.from_seq` for a gap derived from unacknowledged records is the **anchor** —
//!     the highest acknowledged sequence — not the first record above it. The window runs
//!     from the anchor to the head, which is what a reviewer needs to read.
//!   * The `ReadOnly` branch is exercised with a constructed `Read` step. The M0
//!     claude-code-cli capture contains no read-class call at all, so no file in that
//!     fixture can be "named only by a call that read it".
//!
//! The heartbeat envelopes below carry explicit chain sequences rather than a running
//! `Chain`, because hole membership is decided by sequence: a test that wants records
//! inside a hole has to bracket their sequences.

// The shared fixture pipeline. Pulled in by path rather than as `mod support;` because
// this file already carries a file-local `mod support` of its own scenario builders.
#[path = "support/mod.rs"]
mod fixture_support;

use ed25519_dalek::{Signer, SigningKey};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::Link;
use goalcup_protocol::cursor::CursorUnit;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_protocol::seal::{SealManifest, SealedFile, is_excluded};
use goalcup_verifier::Pipeline;
use goalcup_verifier::atif::{Message, Step, StepSource, ToolCall};
use goalcup_verifier::attribution::{Attribution, BULK_ATTRIBUTION_THRESHOLD, attribute_seal};
use goalcup_verifier::checks::v4_continuity::V4Continuity;
use goalcup_verifier::checks::v12_anchors::V12Anchors;
use goalcup_verifier::checks::{Check, Severity};
use goalcup_verifier::gaps::{Anchoring, GapKind, GapReason, anchoring, classify, verify_acks};
use goalcup_verifier::legs::Leg1Store;
use goalcup_verifier::parsers::ParsedLegs;
use goalcup_verifier::trace::TraceDescriptor;
use serde_json::{Value, json};
use std::slice;
use std::sync::OnceLock;

/// Above every sequence any envelope in these tests carries, so a hole's far edge is
/// covered by an acknowledgement and "quiet" can mean "clean".
const ACKED_THROUGH: u64 = 400;

/// The claude-code-cli fixture, loaded once through the shipped pipeline.
///
/// `goalcup_verifier::Pipeline` is the sequence `verify_trace_with_options` runs, so a
/// context borrowed from this is the context the verifier builds — including the manifest
/// gate, the connector gate and the ack reconciliation. Held in a `OnceLock` because every
/// test in this file reads the same fixture and nothing writes to it.
fn claude() -> &'static Pipeline {
    static CLAUDE: OnceLock<Pipeline> = OnceLock::new();
    CLAUDE.get_or_init(|| fixture_support::pipeline::m0("claude-code-cli"))
}

/// The connector manifest `classify` reads `heartbeat_seconds` and each store's
/// `partition_key` from.
fn claude_manifest() -> ConnectorManifest {
    claude().context().manifest.clone()
}

fn claude_main() -> Leg1Store {
    claude()
        .context()
        .stores
        .iter()
        .find(|s| s.name == "main")
        .expect("the fixture declares a `main` store")
        .clone()
}

/// The fixture's descriptor, manifest and parsed legs, owned, for the tests that mutate a
/// copy of the parse before attributing against it.
fn claude_parsed() -> (TraceDescriptor, ConnectorManifest, ParsedLegs) {
    let cx = claude().context();
    (cx.trace.clone(), cx.manifest.clone(), cx.parsed.clone())
}

fn call_step(tool_call_id: &str, function_name: &str, arguments: Value) -> Step {
    Step {
        step_id: 1,
        timestamp: None,
        source: StepSource::Agent,
        model_name: None,
        message: Message::Text(String::new()),
        reasoning_content: None,
        tool_calls: vec![ToolCall {
            tool_call_id: tool_call_id.to_string(),
            function_name: function_name.to_string(),
            arguments,
            extra: None,
        }],
        observation: None,
        metrics: None,
        llm_call_count: None,
        is_copied_context: None,
        extra: None,
    }
}

fn seal_of(workspace: &str, paths: &[&str]) -> SealManifest {
    SealManifest {
        entry_id: "m0-spike4".into(),
        sealed_at: "2026-09-03T07:48:50Z".into(),
        workspace: workspace.to_string(),
        files: paths
            .iter()
            .map(|p| SealedFile {
                path: (*p).to_string(),
                bytes: 1,
                sha256: "aa".repeat(32),
            })
            .collect(),
        excluded: vec![],
    }
    .sorted()
}

fn ack_at(seq: u64, hash: &str, acked_at: &str) -> AckCheckpoint {
    AckCheckpoint {
        entry_id: "e".into(),
        seq,
        hash: hash.to_string(),
        acked_at: acked_at.to_string(),
        server_signature: String::new(),
    }
}

fn envelope(seq: u64, prev: &str, body: EventBody) -> Envelope {
    Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: "e".into(),
        source: "daemon".into(),
        recorded_at: "2026-09-04T00:00:00Z".to_string(),
        link: Link {
            seq,
            prev: prev.to_string(),
            hash: format!("{seq:064}"),
        },
        body,
        redaction: None,
    }
}

/// A `Heartbeat` envelope at chain sequence `seq`, timestamped `ts`.
fn beat(seq: u64, ts: &str) -> Envelope {
    envelope(
        seq,
        &"0".repeat(64),
        EventBody::Heartbeat {
            ts: ts.to_string(),
            stores_seen: vec![],
            receiver_ok: true,
        },
    )
}

/// An honest `DaemonRestart`: it names the chain position immediately below itself.
fn restart(seq: u64, ts: &str) -> Envelope {
    let head = format!("{:064}", seq - 1);
    envelope(
        seq,
        &head,
        EventBody::DaemonRestart {
            at: ts.to_string(),
            previous_head_seq: seq - 1,
            previous_head_hash: head.clone(),
        },
    )
}

#[test]
fn a_fully_acknowledged_chain_has_no_gap() {
    let store = claude_main();
    let last = store.records.last().unwrap();
    let acks = vec![ack_at(last.seq, &last.hash, "2026-09-03T07:51:11Z")];
    assert!(classify(&[store], &acks, &[], &claude_manifest(), &[0]).is_empty());
}

#[test]
fn unacknowledged_records_are_a_dirty_gap_with_the_appended_lines() {
    let store = claude_main();
    let at = &store.records[99];
    let acks = vec![ack_at(at.seq, &at.hash, "2026-09-03T07:45:00Z")];
    let gaps = classify(&[store], &acks, &[], &claude_manifest(), &[0]);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].kind, GapKind::Dirty);
    assert!(!gaps[0].is_hole, "this is unwitnessed work, not downtime");
    assert_eq!(gaps[0].from_seq, 100, "the window runs from the anchor");
    assert_eq!(gaps[0].to_seq, 148);
    assert_eq!(gaps[0].reasons, vec![GapReason::Unanchored]);
    assert_eq!(gaps[0].appended.len(), 48);
    assert_eq!(gaps[0].appended[0].store, "main");
    assert_eq!(gaps[0].appended[0].seq, 101);
    assert_eq!(gaps[0].appended[0].sha256.len(), 64);
}

#[test]
fn a_store_with_no_acknowledgements_at_all_is_not_reported_as_one_enormous_gap() {
    // With no anchor, "above the last acknowledgement" means "the whole chain". Calling
    // that a gap would report every M0 fixture as one 148-record dirty window, which says
    // nothing a reader can act on and buries the finding that matters. The honest
    // statement about an unanchored trace is V12's `no_anchor`, made once, there.
    let store = claude_main();
    assert!(
        classify(&[store], &[], &[], &claude_manifest(), &[]).is_empty(),
        "no anchor is not the same as one big gap"
    );
    // …and "nothing to anchor against" is a distinct outcome, not a clean one.
    assert_eq!(anchoring(&[], &[]), Anchoring::NoAnchors);
}

#[test]
fn a_heartbeat_hole_is_clean_only_when_every_store_can_be_placed_outside_it() {
    // Kill the daemon, work under a different config dir, restart. No records above the
    // ack, and downtime still has to be a statement, not an absence.
    //
    // The statement is conditional, and the M0 fixture is exactly the case it is conditional
    // on: `main` is a `raw_jsonl` store chained here from 1, and 46 of its 148 records carry
    // no timestamp, so they can be placed neither inside the hole nor outside it. That is
    // `unplaceable_records`, and it is Dirty.
    let events = vec![
        beat(200, "2026-09-03T08:00:00Z"),
        beat(201, "2026-09-03T08:01:00Z"),
        // …six hours of silence…
        beat(300, "2026-09-03T14:01:00Z"),
        beat(301, "2026-09-03T14:02:00Z"),
    ];
    let acks = vec![ack_at(
        ACKED_THROUGH,
        &"a".repeat(64),
        "2026-09-03T15:00:00Z",
    )];
    let manifest = claude_manifest();

    let gaps = classify(&[claude_main()], &acks, &events, &manifest, &[0]);
    assert_eq!(gaps.len(), 1, "one hole longer than 2N: {gaps:?}");
    assert_eq!(gaps[0].kind, GapKind::Dirty);
    assert!(
        gaps[0].appended.is_empty(),
        "nothing could be placed inside"
    );
    assert_eq!(
        gaps[0].reasons,
        vec![GapReason::UnplaceableRecords {
            store: "main".to_string(),
            records: 46,
        }]
    );

    // With nothing whose position is in doubt, the same silence is a clean gap: the
    // daemon was down, no store grew, and the acknowledgement covers the far edge.
    let gaps = classify(&[], &acks, &events, &manifest, &[0]);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].kind, GapKind::Clean);
    assert!(gaps[0].is_hole);
    assert!(gaps[0].reasons.is_empty());
    assert_eq!(gaps[0].from.as_deref(), Some("2026-09-03T08:01:00Z"));
    assert_eq!(gaps[0].to.as_deref(), Some("2026-09-03T14:01:00Z"));
    assert_eq!((gaps[0].from_seq, gaps[0].to_seq), (201, 300));
    assert_eq!(anchoring(&acks, &events), Anchoring::Anchored);
}

#[test]
fn a_quiet_hole_the_server_never_witnessed_is_dirty() {
    // Per-hole anchoring. The same silence as above, but the only
    // acknowledgement predates the hole, so nobody witnessed the window in which the
    // daemon says nothing happened.
    let events = vec![
        beat(200, "2026-09-03T08:00:00Z"),
        beat(300, "2026-09-03T14:00:00Z"),
    ];
    let acks = vec![ack_at(199, &"a".repeat(64), "2026-09-03T07:59:00Z")];
    let gaps = classify(&[], &acks, &events, &claude_manifest(), &[0]);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].kind, GapKind::Dirty);
    assert!(gaps[0].appended.is_empty(), "still nothing was appended");
    assert_eq!(gaps[0].reasons, vec![GapReason::Unanchored]);
}

#[test]
fn a_single_missed_tick_is_not_a_gap_and_no_anchor_makes_every_hole_dirty() {
    let events = vec![
        beat(200, "2026-09-03T08:00:00Z"),
        // 90 seconds: one missed tick at N = 60, under the 2N window.
        beat(201, "2026-09-03T08:01:30Z"),
    ];
    let acks = vec![ack_at(
        ACKED_THROUGH,
        &"a".repeat(64),
        "2026-09-03T15:00:00Z",
    )];
    let manifest = claude_manifest();
    assert!(
        classify(&[claude_main()], &acks, &events, &manifest, &[0]).is_empty(),
        "one missed tick under load is not an incident"
    );

    // With no acknowledgement stream at all, nothing was watched, so every hole is
    // dirty even when nothing was appended in it.
    let events = vec![
        beat(200, "2026-09-03T08:00:00Z"),
        beat(300, "2026-09-03T14:00:00Z"),
    ];
    let gaps = classify(&[claude_main()], &[], &events, &manifest, &[]);
    assert!(
        gaps.iter()
            .any(|g| g.kind == GapKind::Dirty && g.from.is_some())
    );
    assert_eq!(anchoring(&[], &events), Anchoring::Unanchored);
}

#[test]
fn a_daemon_restart_bounds_a_hole_and_records_inside_it_make_it_dirty() {
    // `DaemonRestart` names the head the daemon found on start, which is where the
    // previous run stopped — so it bounds a hole even with no heartbeat on its side of it.
    // The bounding timestamps bracket the fixture's own record times (07:42–07:48), which
    // is how a `raw_jsonl` store's records are placed.
    let events = vec![
        beat(50, "2026-09-03T07:00:00Z"),
        restart(150, "2026-09-03T08:00:00Z"),
    ];
    let acks = vec![ack_at(
        ACKED_THROUGH,
        &"a".repeat(64),
        "2026-09-03T15:00:00Z",
    )];
    let gaps = classify(&[claude_main()], &acks, &events, &claude_manifest(), &[0]);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].kind, GapKind::Dirty);
    assert!(gaps[0].is_hole);
    assert_eq!(gaps[0].from.as_deref(), Some("2026-09-03T07:00:00Z"));
    assert_eq!(gaps[0].to.as_deref(), Some("2026-09-03T08:00:00Z"));
    assert_eq!((gaps[0].from_seq, gaps[0].to_seq), (50, 150));
    // The 102 records that carry a timestamp inside the window, and the 46 that carry
    // none and could be anywhere.
    assert_eq!(gaps[0].appended.len(), 102);
    assert!(gaps[0].reasons.contains(&GapReason::RecordsAppended));
    assert!(
        gaps[0]
            .reasons
            .iter()
            .any(|r| matches!(r, GapReason::UnplaceableRecords { records: 46, .. }))
    );
    assert!(
        !gaps[0]
            .reasons
            .iter()
            .any(|r| matches!(r, GapReason::RestartHeadMismatch { .. })),
        "this restart names the head the chain actually holds"
    );
}

#[test]
fn records_above_the_anchor_reach_a_reader_through_v4() {
    // This gap is not a hole, and V4 renders `dirty_gap` only for holes, so `Unanchored`
    // has a code of its own: the single most important thing the classifier can say
    // ("everything after seq N is unwitnessed") has to be displayed. This test goes through
    // the check, not through `classify`, because the check is where it is displayed.
    let fixture = claude();
    let loaded = fixture.context();
    let at = &loaded
        .stores
        .iter()
        .find(|s| s.name == "main")
        .expect("the fixture declares a `main` store")
        .records[99];
    // A real, signed acknowledgement, reconciled by `verify_acks`. The anchor allowlist
    // `classify` reads and the reconciliation the context carries
    // have to be the same one: passing `&[0]` beside `NO_ACKS` would assert a state
    // `verify_trace` cannot produce — one acknowledgement verified for the classifier and
    // none verified for the checks reading the same trace.
    let server = SigningKey::from_bytes(&[5u8; 32]);
    let key = hex::encode(server.verifying_key().to_bytes());
    let mut ack = ack_at(at.seq, &at.hash, "2026-09-03T07:45:00Z");
    ack.entry_id = loaded.trace.entry_id.clone();
    ack.server_signature = hex::encode(server.sign(&ack.signing_bytes().unwrap()).to_bytes());
    let acks = vec![ack];
    let reconciliation = verify_acks(
        None,
        loaded.stores,
        &acks,
        Some(&key),
        &loaded.trace.entry_id,
    );
    assert_eq!(
        reconciliation.verified,
        vec![0],
        "the anchor must genuinely verify: {:?}",
        reconciliation.mismatches
    );
    let gaps = classify(
        loaded.stores,
        &acks,
        &[],
        loaded.manifest,
        &reconciliation.verified,
    );
    assert!(gaps.iter().any(|g| !g.appended.is_empty()));

    // The shipped context, with the acknowledgement this test signed put into it. Every
    // field it does not name — the parse, the human rule, the manifest digest, the pin —
    // is what `verify_trace` would have handed V4 for this fixture.
    let mut cx = fixture.context();
    cx.acks = &acks;
    cx.acks_verified = &reconciliation;
    cx.gaps = &gaps;
    cx.server_key = Some(&key);
    let result = V4Continuity.run(&cx);
    let finding = result
        .findings
        .iter()
        .find(|f| f.code == "unanchored_records")
        .expect("the chain above the anchor must be reported");
    assert_eq!(finding.severity, Severity::Fail);
    assert_eq!(result.verdict, Severity::Fail);
    assert!(
        finding
            .evidence
            .iter()
            .all(|e| e.seq.is_some() && e.sha256.is_some() && !e.note.contains('/')),
        "cited by store, seq and hash only"
    );
}

#[test]
fn a_forged_high_seq_ack_does_not_raise_the_anchor_or_suppress_unanchored_records() {
    // `classify`'s `high` must come from
    // the verified set only — signature-checked against a pinned key, not merely "not a
    // known mismatch". A forged acknowledgement at a sequence above the last genuine one
    // must not raise the anchor and silently clear `unanchored_records` for everything
    // between the genuine anchor and the forged one — the attack: claim a later
    // acknowledgement than the server ever gave, so the chain above the real anchor reads
    // as witnessed when it was not.
    let store = claude_main();
    let genuine_seq = store.records[99].seq;
    let genuine_hash = store.records[99].hash.clone();
    let server = SigningKey::from_bytes(&[7u8; 32]);
    let key = hex::encode(server.verifying_key().to_bytes());
    let mut genuine = ack_at(genuine_seq, &genuine_hash, "2026-09-03T07:45:00Z");
    genuine.server_signature =
        hex::encode(server.sign(&genuine.signing_bytes().unwrap()).to_bytes());
    // A seq the chain never reached, with a hash the participant made up and no valid
    // server signature over it — the participant cannot forge one.
    let forged = ack_at(9_999, &"f".repeat(64), "2026-09-03T08:00:00Z");
    let acks = vec![genuine, forged];

    let reconciliation = verify_acks(None, slice::from_ref(&store), &acks, Some(&key), "e");
    assert_eq!(
        reconciliation.verified,
        vec![0],
        "only the genuine, signed ack verifies"
    );
    assert_eq!(reconciliation.mismatches.len(), 1);
    assert_eq!(reconciliation.mismatches[0].seq, 9_999);

    let gaps = classify(
        &[store],
        &acks,
        &[],
        &claude_manifest(),
        &reconciliation.verified,
    );
    let above_anchor = gaps
        .iter()
        .find(|g| g.reasons.contains(&GapReason::Unanchored))
        .expect("records above the genuine anchor must still be reported unanchored");
    assert_eq!(
        above_anchor.from_seq, genuine_seq,
        "anchored at the GENUINE ack, not the forged one"
    );
    assert!(
        above_anchor
            .appended
            .iter()
            .any(|l| l.seq == genuine_seq + 1)
    );

    // Sanity check: this is exactly the suppression the verified-only anchor prevents. A
    // caller that trusts every supplied ack (an explicit index set covering both, which is
    // what a caller passes when it has nothing to say about verification) reads the forged
    // 9,999 as the anchor, and every real record sits below it — so nothing is reported.
    let trusting_every_ack = classify(&[claude_main()], &acks, &[], &claude_manifest(), &[0, 1]);
    assert!(
        trusting_every_ack.is_empty(),
        "sanity check failed: the forged ack should have suppressed everything here"
    );
}

#[test]
fn with_no_pinned_key_a_hash_reconciling_ack_never_anchors_and_v12_still_flags_it() {
    // "Verified" means signature-verified against a pinned key.
    // Without one, an acknowledgement whose hash reconciles is exactly the
    // self-consistent evidence the participant can produce alone — it must never be a
    // Fail (V12 does not lie and call it a mismatch), but it must also never anchor the
    // chain (`classify` does not trust it either). Both checks have to agree here: nothing
    // rules tampering in, and nothing rules it out.
    let fixture = claude();
    let loaded = fixture.context();
    assert!(
        loaded.trace.server_public_key.is_none(),
        "the M0 fixture pins no server key"
    );
    assert!(
        loaded.server_key.is_none(),
        "and the pipeline resolved no key from it"
    );
    let high = loaded
        .stores
        .iter()
        .find(|s| s.name == "main")
        .expect("the fixture declares a `main` store")
        .records
        .last()
        .expect("the store is not empty");
    let acks = vec![AckCheckpoint {
        entry_id: loaded.trace.entry_id.clone(),
        seq: high.seq,
        hash: high.hash.clone(),
        acked_at: "2026-09-03T07:51:11Z".to_string(),
        server_signature: String::new(),
    }];

    let reconciliation = verify_acks(
        None,
        loaded.stores,
        &acks,
        loaded.server_key,
        &loaded.trace.entry_id,
    );
    assert!(
        reconciliation.mismatches.is_empty(),
        "the hash reconciles; this is not a mismatch"
    );
    assert!(
        reconciliation.verified.is_empty(),
        "no key pinned; never verified"
    );

    let gaps = classify(
        loaded.stores,
        &acks,
        &[],
        loaded.manifest,
        &reconciliation.verified,
    );
    let total_records: usize = loaded.stores.iter().map(|s| s.records.len()).sum();
    let total_unanchored: usize = gaps
        .iter()
        .filter(|g| g.reasons.contains(&GapReason::Unanchored))
        .map(|g| g.appended.len())
        .sum();
    assert_eq!(
        total_unanchored, total_records,
        "every record in every store must read as unanchored with nothing verified"
    );

    // The shipped context with this acknowledgement stream in it. The reconciliation
    // computed above — hash reconciles, nothing verified because the M0 fixture pins no
    // key — not the empty stand-in: `NO_ACKS` beside a non-empty `acks` says "there were
    // no acknowledgements", which is the one thing this test is not about.
    let mut cx = fixture.context();
    cx.acks = &acks;
    cx.acks_verified = &reconciliation;
    cx.gaps = &gaps;

    let v4 = V4Continuity.run(&cx);
    let finding = v4
        .findings
        .iter()
        .find(|f| f.code == "unanchored_records")
        .expect("every record must still reach a reader as unanchored");
    assert_eq!(finding.severity, Severity::Fail);

    let v12 = V12Anchors.run(&cx);
    assert_eq!(v12.verdict, Severity::Flag, "{:?}", v12.findings);
    assert!(v12.findings.iter().any(|f| f.code == "anchor_unverified"));
    assert!(
        !v12.findings.iter().any(|f| f.severity == Severity::Fail),
        "an unreconciled but unverifiable ack is a review item, not a Fail: {:?}",
        v12.findings
    );
}

#[test]
fn verify_acks_catches_a_chain_rewritten_below_the_last_acknowledgement() {
    // The attack this exists for: re-chain the entry from seq 1 with the instructions
    // removed, below the last acknowledgement. The result recomputes cleanly and yields no
    // gaps — and is caught here, because the server's hash at that sequence no longer
    // matches what the chain holds.
    let store = claude_main();
    let anchor = &store.records[49];
    let honest = ack_at(anchor.seq, &anchor.hash, "2026-09-03T07:45:00Z");
    assert!(
        verify_acks(
            None,
            slice::from_ref(&store),
            slice::from_ref(&honest),
            None,
            "e"
        )
        .mismatches
        .is_empty(),
        "an honest acknowledgement reconciles"
    );

    let rewritten = AckCheckpoint {
        hash: "ff".repeat(32),
        ..honest.clone()
    };
    let bad = verify_acks(None, slice::from_ref(&store), &[rewritten], None, "e");
    assert_eq!(bad.mismatches.len(), 1);
    assert_eq!(bad.mismatches[0].seq, anchor.seq);
    assert_eq!(
        bad.mismatches[0].observed.as_deref(),
        Some(anchor.hash.as_str())
    );

    // A chain rebuilt shorter than it was: the acknowledged sequence no longer exists.
    let beyond = AckCheckpoint {
        seq: 9_999,
        ..honest.clone()
    };
    let bad = verify_acks(None, slice::from_ref(&store), &[beyond], None, "e");
    assert_eq!(bad.mismatches.len(), 1);
    assert!(
        bad.mismatches[0].observed.is_none(),
        "no record at the acknowledged sequence"
    );
}

#[test]
fn verify_acks_rejects_a_hand_written_acknowledgement_stream() {
    // The participant writes the acknowledgement stream themselves for
    // the offline self-check. Without a signature check that stream is an anchor they
    // control end to end.
    let store = claude_main();
    let anchor = &store.records[49];
    let server = SigningKey::from_bytes(&[3u8; 32]);
    let mut ack = ack_at(anchor.seq, &anchor.hash, "2026-09-03T07:45:00Z");
    ack.server_signature = hex::encode(server.sign(&ack.signing_bytes().unwrap()).to_bytes());
    let key = hex::encode(server.verifying_key().to_bytes());
    let honest = verify_acks(
        None,
        slice::from_ref(&store),
        slice::from_ref(&ack),
        Some(&key),
        "e",
    );
    assert!(honest.mismatches.is_empty());
    assert_eq!(honest.verified, vec![0], "a genuine signature verifies");

    // Same chain position, signed by a key that is not the server's.
    let forger = SigningKey::from_bytes(&[4u8; 32]);
    let mut forged = ack.clone();
    forged.server_signature = hex::encode(forger.sign(&forged.signing_bytes().unwrap()).to_bytes());
    let bad = verify_acks(None, &[store], &[forged], Some(&key), "e");
    assert_eq!(
        bad.mismatches.len(),
        1,
        "a forged signature is not an anchor"
    );
    assert!(bad.mismatches[0].observed.is_none());
    assert!(bad.verified.is_empty());
}

#[test]
fn seal_attribution_matches_files_to_traced_tool_calls() {
    let (trace, manifest, mut parsed) = claude_parsed();
    // The M0 capture has no read-class call at all, so the shape — a file
    // named only by a `Read` — has to be constructed.
    parsed.trajectory.steps.push(call_step(
        "toolu_read_notes",
        "Read",
        json!({ "file_path": format!("{}/notes.md", trace.workspace) }),
    ));

    let seal = seal_of(
        &trace.workspace,
        &["hello.py", "README.md", "planted.txt", "notes.md"],
    );
    let attributed = attribute_seal(&seal, None, &parsed, &manifest);
    assert_eq!(attributed.len(), 4);
    assert!(
        attributed.windows(2).all(|w| w[0].path < w[1].path),
        "deterministic, path-sorted output"
    );

    // `cat > /…/hello.py <<'EOF'` — the redirect's own target, absolute spelling.
    let hello = attributed.iter().find(|a| a.path == "hello.py").unwrap();
    match &hello.attribution {
        Attribution::Traced { tool, .. } => assert!(
            manifest.tool_classes.write.contains(tool),
            "attributed by `{tool}`, which must be a write-class tool"
        ),
        other => panic!("hello.py was written by a traced Bash call, got {other:?}"),
    }
    // `ls -la && cat > README.md <<'EOF'` — the same, workspace-relative spelling.
    let readme = attributed.iter().find(|a| a.path == "README.md").unwrap();
    assert!(matches!(readme.attribution, Attribution::Traced { .. }));

    // Nothing names it at all.
    let planted = attributed.iter().find(|a| a.path == "planted.txt").unwrap();
    assert_eq!(planted.attribution, Attribution::Unexplained);

    // Named only by calls that read. A pass here is the whole "let the tracked agent
    // discover the results" attack.
    let notes = attributed.iter().find(|a| a.path == "notes.md").unwrap();
    assert!(
        matches!(notes.attribution, Attribution::ReadOnly { .. }),
        "a file named only by a Read is neither traced nor invisible, got {:?}",
        notes.attribution
    );
}

#[test]
fn a_read_never_attributes_and_a_shell_call_must_name_the_path_in_its_command() {
    let (trace, manifest, parsed) = claude_parsed();
    // The rule in one line: `Read` is not in `tool_classes.write`, `Bash` is, and `Bash`
    // matches on the command text rather than on the whole arguments blob.
    assert!(!manifest.tool_classes.write.iter().any(|t| t == "Read"));
    assert!(manifest.tool_classes.write.iter().any(|t| t == "Bash"));
    assert!(manifest.tool_classes.shell.iter().any(|t| t == "Bash"));

    let sealed = seal_of(&trace.workspace, &["unmentioned.txt"]);
    let out = attribute_seal(&sealed, None, &parsed, &manifest);
    assert_eq!(out[0].attribution, Attribution::Unexplained);
}

#[test]
fn a_stderr_redirect_over_a_file_the_command_only_read_never_traces_it() {
    // End to end: `cat path 2>/dev/null` contains `>`, but it does not attribute the very
    // file it only read — the "let the tracked agent discover the results" attack wearing a
    // redirect.
    let (trace, manifest, mut parsed) = claude_parsed();
    parsed.trajectory.steps.clear();
    parsed.trajectory.steps.push(call_step(
        "toolu_cat",
        "Bash",
        json!({ "command": format!("cat {}/planted.txt 2>/dev/null", trace.workspace) }),
    ));
    let out = attribute_seal(
        &seal_of(&trace.workspace, &["planted.txt"]),
        None,
        &parsed,
        &manifest,
    );
    assert_eq!(
        out[0].attribution,
        Attribution::Unexplained,
        "a stderr redirect is not a write"
    );

    // The same command with a real redirect into the file does trace it.
    parsed.trajectory.steps.clear();
    parsed.trajectory.steps.push(call_step(
        "toolu_write",
        "Bash",
        json!({ "command": format!("cat x > {}/planted.txt", trace.workspace) }),
    ));
    let out = attribute_seal(
        &seal_of(&trace.workspace, &["planted.txt"]),
        None,
        &parsed,
        &manifest,
    );
    assert!(matches!(out[0].attribution, Attribution::Traced { .. }));

    // …and so does handing it to a write operator.
    parsed.trajectory.steps.clear();
    parsed.trajectory.steps.push(call_step(
        "toolu_tee",
        "Bash",
        json!({ "command": format!("echo x | tee {}/planted.txt", trace.workspace) }),
    ));
    let out = attribute_seal(
        &seal_of(&trace.workspace, &["planted.txt"]),
        None,
        &parsed,
        &manifest,
    );
    assert!(matches!(out[0].attribution, Attribution::Traced { .. }));

    // A redirect that names the path but writes somewhere else does not.
    parsed.trajectory.steps.clear();
    parsed.trajectory.steps.push(call_step(
        "toolu_elsewhere",
        "Bash",
        json!({
            "command": format!(
                "echo {}/planted.txt > {}/other.txt",
                trace.workspace, trace.workspace
            )
        }),
    ));
    let out = attribute_seal(
        &seal_of(&trace.workspace, &["planted.txt"]),
        None,
        &parsed,
        &manifest,
    );
    assert_eq!(out[0].attribution, Attribution::Unexplained);
}

#[test]
fn a_bare_mention_a_basename_and_an_interpreter_one_liner_all_stay_unexplained() {
    let (trace, manifest, mut parsed) = claude_parsed();
    parsed.trajectory.steps.clear();
    parsed.trajectory.steps.push(call_step(
        "toolu_echo",
        "Bash",
        json!({ "command": format!("echo {}/planted.txt", trace.workspace) }),
    ));
    parsed.trajectory.steps.push(call_step(
        "toolu_basename",
        "Bash",
        json!({ "command": "cat > /somewhere/else/OLD-basename.txt.bak" }),
    ));
    parsed.trajectory.steps.push(call_step(
        "toolu_write_elsewhere",
        "Write",
        json!({ "file_path": format!("{}/other/basename.txt", trace.workspace) }),
    ));
    // The real second Bash call of the M0 capture: `long.py` is named inside a heredoc'd
    // Python program, and the only "operator" in the text is the `rm` of "Performance".
    parsed.trajectory.steps.push(call_step(
        "toolu_interpreter",
        "Bash",
        json!({
            "command": "python3 - <<'PYGEN'\nsections = [(\"Performance\", \"what we do \
                        about it\")]\nwith open(\"long.py\", \"w\") as f:\n    \
                        f.write(text)\nPYGEN"
        }),
    ));

    let seal = seal_of(
        &trace.workspace,
        &["planted.txt", "basename.txt", "long.py"],
    );
    let out = attribute_seal(&seal, None, &parsed, &manifest);
    for a in &out {
        assert_eq!(
            a.attribution,
            Attribution::Unexplained,
            "`{}` must not be attributed: a bare mention, a basename and an interpreter \
             one-liner are not traced writes",
            a.path
        );
    }
}

#[test]
fn the_real_readme_heredoc_does_not_trace_the_file_its_prose_mentions() {
    // The fixture's third `Bash` call (`ls -la && cat > README.md <<'EOF'`) does not trace
    // `long.py`, although the heredoc's markdown table names `long.py` and the command
    // contains `>`: the redirect's target is `README.md`, and the mention inside the prose
    // is `` `long.py` `` — backticks, which are not shell quotes and are deliberately not
    // stripped.
    //
    // The coarseness that remains, and that no rule short of parsing shell can remove: a
    // command's write operator attributes any of its argument tokens, so `rm long.py`
    // would attribute a file it deleted rather than created, and a heredoc body that
    // happens to contain a bare `long.py` token after a `cp` would attribute it too. The
    // seal's content hashes, not attribution, are what tell a reviewer what the file
    // holds.
    let (trace, manifest, parsed) = claude_parsed();
    let out = attribute_seal(
        &seal_of(&trace.workspace, &["README.md", "long.py"]),
        None,
        &parsed,
        &manifest,
    );
    let readme = out.iter().find(|a| a.path == "README.md").unwrap();
    assert!(
        matches!(readme.attribution, Attribution::Traced { .. }),
        "the redirect's own target is traced"
    );
    let long = out.iter().find(|a| a.path == "long.py").unwrap();
    assert_eq!(
        long.attribution,
        Attribution::Unexplained,
        "a file named only in the prose of another file's heredoc is not written by it"
    );
}

#[test]
fn workspace_exclusions_come_from_the_manifest_through_one_shared_function() {
    // This is `goalcup_protocol::seal::is_excluded`, the same function the daemon's seal
    // walk and the CLI's self-check call.
    let codex = fixture_support::pipeline::m0("codex-cli");
    let ex = &codex.context().manifest.workspace_exclusions;
    assert!(is_excluded(".codex/config.toml", ex));
    assert!(is_excluded(".git/config", ex));
    // The exclusion is anchored at the workspace root. A nested `.git` is the agent's work
    // and needs an explanation: an any-component rule would let a `mkdir -p vendor/lib/.git`
    // take a whole subtree out of both the seal and this attribution pass.
    assert!(!is_excluded("vendor/lib/.git/HEAD", ex));
    assert!(!is_excluded("hello.py", ex));
}

#[test]
fn an_excluded_sealed_file_is_never_attributed() {
    let (trace, manifest, parsed) = claude_parsed();
    // `.claude` and `.git` are this connector's `workspace_exclusions`.
    let seal = seal_of(&trace.workspace, &[".claude/settings.json", "hello.py"]);
    let out = attribute_seal(&seal, None, &parsed, &manifest);
    assert_eq!(out.len(), 1, "the excluded file needs no explanation");
    assert_eq!(out[0].path, "hello.py");
}

/// `verify_acks` is handed the chain because on a daemon-produced trace an
/// acknowledgement names an **envelope's** seq, and the head it names is as likely to be a
/// heartbeat as a leg-1 record. Reconciled against leg-1 records alone, a clean hourly run
/// would produce a guaranteed false `ack_chain_mismatch` at every hour.
#[test]
fn an_ack_whose_head_is_a_heartbeat_reconciles_against_the_chain() {
    use goalcup_protocol::export::{CHAIN_FILE, TraceExport};

    let tmp = tempfile::tempdir().unwrap();
    let mut x = TraceExport::create(tmp.path()).unwrap();
    let mut record = Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: "e".to_string(),
        source: "main".to_string(),
        recorded_at: "2026-09-04T00:00:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::Leg1Record {
            store: "main".to_string(),
            payload: Value::String("{\"type\":\"user\"}".to_string()),
        },
        redaction: None,
    };
    x.append_chain(&mut record).unwrap();
    let mut beat = Envelope {
        source: "daemon".to_string(),
        recorded_at: "2026-09-04T01:00:00Z".to_string(),
        body: EventBody::Heartbeat {
            ts: "2026-09-04T01:00:00Z".to_string(),
            stores_seen: Vec::new(),
            receiver_ok: true,
        },
        ..record.clone()
    };
    beat.link = Link {
        seq: 0,
        prev: String::new(),
        hash: String::new(),
    };
    let head = x.append_chain(&mut beat).unwrap();
    drop(x);

    let chain = goalcup_verifier::chain::load_and_verify_text(
        &tmp.path().join(CHAIN_FILE).display().to_string(),
        &std::fs::read_to_string(tmp.path().join(CHAIN_FILE)).unwrap(),
    )
    .unwrap();
    assert!(chain.break_at.is_none());
    assert_eq!(chain.seal_link, None);

    let server = SigningKey::from_bytes(&[3u8; 32]);
    let mut ack = ack_at(head.seq, &head.hash, "2026-09-04T01:00:05Z");
    ack.server_signature = hex::encode(server.sign(&ack.signing_bytes().unwrap()).to_bytes());
    let key = hex::encode(server.verifying_key().to_bytes());

    let r = verify_acks(
        Some(&chain),
        &chain.stores,
        slice::from_ref(&ack),
        Some(&key),
        "e",
    );
    assert!(
        r.mismatches.is_empty(),
        "the acknowledged head is a heartbeat, and it is in the chain: {:?}",
        r.mismatches
    );
    assert_eq!(r.verified, vec![0]);

    // And the chain is still what decides it: an acknowledgement over a hash the chain
    // does not hold at that sequence is a mismatch, chain or no chain.
    let mut wrong = AckCheckpoint {
        hash: "f".repeat(64),
        ..ack.clone()
    };
    wrong.server_signature = hex::encode(server.sign(&wrong.signing_bytes().unwrap()).to_bytes());
    let r = verify_acks(
        Some(&chain),
        &chain.stores,
        slice::from_ref(&wrong),
        Some(&key),
        "e",
    );
    assert_eq!(r.mismatches.len(), 1);
    assert_eq!(
        r.mismatches[0].observed.as_deref(),
        Some(head.hash.as_str())
    );
    assert!(r.verified.is_empty());
}

// ---------------------------------------------------------------------------
// The baseline, and one call that explains a tree.
// ---------------------------------------------------------------------------

#[test]
fn a_file_the_baseline_already_held_needs_no_traced_write() {
    let baseline = support::seal(&[("README.md", "aa"), ("src/lib.rs", "bb")]);
    let seal = support::seal(&[("README.md", "aa"), ("src/lib.rs", "cc"), ("new.py", "dd")]);
    let a = attribute_seal(
        &seal,
        Some(&baseline),
        &support::parsed_no_calls(),
        &support::manifest("claude-code-cli"),
    );

    let by = |p: &str| a.iter().find(|x| x.path == p).unwrap().attribution.clone();
    assert_eq!(
        by("README.md"),
        Attribution::Baseline {
            sha256: "aa".to_string()
        }
    );
    // Same path, different bytes: the agent rewrote it, so it is owed an explanation.
    assert_eq!(by("src/lib.rs"), Attribution::Unexplained);
    assert_eq!(by("new.py"), Attribution::Unexplained);

    // Without a baseline nothing changes for the two the baseline did not cover.
    let none = attribute_seal(
        &seal,
        None,
        &support::parsed_no_calls(),
        &support::manifest("claude-code-cli"),
    );
    assert_eq!(
        none.iter()
            .filter(|x| x.attribution == Attribution::Unexplained)
            .count(),
        3
    );
}

#[test]
fn a_baseline_file_raises_no_finding_and_counts_towards_no_call() {
    // The point of `Baseline`: it is silence, not a flag. A workspace that legitimately
    // starts from a checkout must not fill the review queue with its own contents.
    let baseline = support::seal(&[("README.md", "aa")]);
    let seal = support::seal(&[("README.md", "aa"), ("planted.txt", "ee")]);
    let a = attribute_seal(
        &seal,
        Some(&baseline),
        &support::parsed_no_calls(),
        &support::manifest("claude-code-cli"),
    );
    let findings = support::seal_findings(
        &a,
        &support::parsed_no_calls(),
        &support::manifest("claude-code-cli"),
    );
    assert_eq!(findings.len(), 1, "only the file the baseline did not hold");
    assert_eq!(findings[0].code, "unexplained_seal_file");
    assert!(findings[0].message.contains("planted.txt"));
}

/// A baseline listing one path twice is malformed, and a malformed baseline exempts
/// nothing.
///
/// The lookup is a binary search over a path-sorted index, so two entries under one path
/// mean the search returns whichever of them the sort happens to leave in the middle:
/// a participant could list `secret.py` twice, once with the digest the seal will carry
/// and once with anything, and the exemption would come down to a tie-break. Both entries
/// are dropped instead, and the file walks the ordinary ladder.
#[test]
fn a_baseline_that_names_one_path_twice_exempts_it_under_neither_digest() {
    let mut baseline = support::seal(&[("README.md", "aa")]);
    baseline.files.push(goalcup_protocol::seal::SealedFile {
        path: "secret.py".to_string(),
        bytes: 9,
        sha256: "bb".to_string(),
    });
    baseline.files.push(goalcup_protocol::seal::SealedFile {
        path: "secret.py".to_string(),
        bytes: 9,
        sha256: "cc".to_string(),
    });
    let baseline = baseline.sorted();

    // Both digests the malformed baseline offers, and the file the same baseline names
    // once, which must still be exempt: one bad entry does not void the whole document.
    for sha256 in ["bb", "cc"] {
        let seal = support::seal(&[("README.md", "aa"), ("secret.py", sha256)]);
        let a = attribute_seal(
            &seal,
            Some(&baseline),
            &support::parsed_no_calls(),
            &support::manifest("claude-code-cli"),
        );
        let by = |p: &str| a.iter().find(|x| x.path == p).unwrap().attribution.clone();
        assert_eq!(
            by("secret.py"),
            Attribution::Unexplained,
            "digest {sha256}: a duplicated path is no match at all"
        );
        assert_eq!(
            by("README.md"),
            Attribution::Baseline {
                sha256: "aa".to_string()
            }
        );
    }
}

#[test]
fn one_shell_call_explaining_a_whole_tree_is_flagged() {
    // The pre-start import, traced, in a single call.
    let files: Vec<(String, String)> = (0..BULK_ATTRIBUTION_THRESHOLD + 1)
        .map(|i| (format!("imported/f{i}.py"), format!("{i:02}")))
        .collect();
    let seal = support::seal_owned(&files);
    let parsed = support::parsed_with_shell_call("call_bulk", &support::import_command(&files));
    let a = attribute_seal(&seal, None, &parsed, &support::manifest("claude-code-cli"));
    assert!(
        a.iter()
            .all(|x| matches!(x.attribution, Attribution::Traced { .. }))
    );

    let findings = support::seal_findings(&a, &parsed, &support::manifest("claude-code-cli"));
    let f = findings
        .iter()
        .find(|f| f.code == "bulk_attribution")
        .expect("bulk_attribution");
    assert_eq!(f.severity, goalcup_verifier::checks::Severity::Flag);
    assert!(f.message.contains("call_bulk"), "{}", f.message);
    assert!(
        f.message
            .contains(&format!("{}", BULK_ATTRIBUTION_THRESHOLD + 1)),
        "{}",
        f.message
    );

    // Exactly at the threshold is not a finding: the constant is a limit, not a target.
    // The command is the same one, still naming all nine paths — so this also pins that
    // the count is of distinct sealed files, not of path mentions.
    let smaller = support::seal_owned(&files[..BULK_ATTRIBUTION_THRESHOLD]);
    let a2 = attribute_seal(
        &smaller,
        None,
        &parsed,
        &support::manifest("claude-code-cli"),
    );
    assert!(
        !support::seal_findings(&a2, &parsed, &support::manifest("claude-code-cli"))
            .iter()
            .any(|f| f.code == "bulk_attribution")
    );
}

#[test]
fn a_non_shell_write_call_never_raises_bulk_attribution() {
    // An `Edit` explains one file by construction, so a run of them is not a tree import
    // however long it is. Only the shell class can explain more than it names.
    let files: Vec<(String, String)> = (0..BULK_ATTRIBUTION_THRESHOLD + 3)
        .map(|i| (format!("w{i}.py"), format!("{i:02}")))
        .collect();
    let seal = support::seal_owned(&files);
    // One `Write` call id against every sealed file — impossible in a real trace, and
    // exactly the shape the exemption has to survive.
    let a: Vec<_> = seal
        .files
        .iter()
        .map(|f| goalcup_verifier::attribution::SealAttribution {
            path: f.path.clone(),
            attribution: Attribution::Traced {
                call_id: "call_write".to_string(),
                tool: "Write".to_string(),
            },
        })
        .collect();
    let parsed = support::parsed_with_call("call_write", "Write", json!({ "file_path": "w0.py" }));
    assert!(
        !support::seal_findings(&a, &parsed, &support::manifest("claude-code-cli"))
            .iter()
            .any(|f| f.code == "bulk_attribution")
    );
}

/// A byte cursor is not a record count. Two heartbeats bounding one window, a byte cursor
/// that advanced 100 KiB, and three records in the window raise nothing, and neither does a
/// cursor that moved 2 bytes: bytes moved are never read as records shipped.
#[test]
fn a_byte_offset_is_never_compared_with_a_count_of_records() {
    let bytes_moved = |before: u64, after: u64| {
        support::classify_one_window(
            support::three_records_in_the_window(),
            support::cursor("main", CursorUnit::Bytes, before, "inode:1,offset:"),
            support::cursor("main", CursorUnit::Bytes, after, "inode:1,offset:"),
        )
    };
    assert!(
        bytes_moved(0, 102_400).is_empty(),
        "100 KiB is not 102,400 records"
    );
    assert!(
        bytes_moved(0, 2).is_empty(),
        "and 2 bytes is not fewer than three records"
    );

    // Under `Records` the comparison is the one the check was written for, and it fires.
    let reasons = support::classify_one_window(
        support::three_records_in_the_window(),
        support::cursor("main", CursorUnit::Records, 0, "ordinal:"),
        support::cursor("main", CursorUnit::Records, 1, "ordinal:"),
    );
    assert!(
        reasons
            .iter()
            .any(|r| matches!(r, GapReason::CursorMismatch { records: 3, .. })),
        "{reasons:?}"
    );
}

/// …and the same asymmetry on the `Flag` side: a byte cursor that moved over an empty
/// window raises no `CursorAheadOfChain` either, because "it moved 4,096" and "it shipped
/// nothing" are only comparable once the number counts records.
#[test]
fn a_byte_offset_never_raises_a_cursor_ahead_of_the_chain_either() {
    let quiet = support::classify_one_window(
        support::no_records_in_the_window(),
        support::cursor("main", CursorUnit::Bytes, 0, "inode:1,offset:"),
        support::cursor("main", CursorUnit::Bytes, 4096, "inode:1,offset:"),
    );
    assert!(quiet.is_empty(), "{quiet:?}");

    // …and the store really is there to be reported on, so the assertion above is not
    // vacuous: under `Records` the same empty window raises the finding.
    let reasons = support::classify_one_window(
        support::no_records_in_the_window(),
        support::cursor("main", CursorUnit::Records, 0, "ordinal:"),
        support::cursor("main", CursorUnit::Records, 7, "ordinal:"),
    );
    assert_eq!(
        reasons,
        vec![GapReason::CursorAheadOfChain {
            store: "main".to_string(),
            cursor_delta: Some(7),
        }]
    );
}

/// Codex's `exec` is in both `tool_classes.write` and `tool_classes.shell`. A seal-time
/// branch of `if is_shell { shell rule } else { write rule }` would keep a real Codex write
/// from ever reaching `patch_target_paths`, which exists for this exact payload, and every
/// honest Codex run would fail on `unexplained_seal_file`.
///
/// The arguments below are the shape a live Codex chain carried at seq 33: the
/// `apply_patch` DSL inside a JS-source string, its newlines escaped as a literal
/// backslash and `n`, under the `input` key the Codex parser writes for a
/// `custom_tool_call`. `input` is also one of the shell rule's command keys, so this call
/// reaches the shell rule too — and the shell rule does not attribute it, which is the
/// whole finding.
#[test]
fn a_codex_exec_apply_patch_attributes_the_file_it_adds() {
    let input = r#"text(await tools.apply_patch("*** Begin Patch\n*** Add File: hello.py\n+print(\"hello world\")\n*** End Patch"));
"#;
    let parsed = support::parsed_with_call(
        "call_HttSZaWk9ISYQbx9iAb5SAPq",
        "exec",
        json!({ "input": input, "arguments": serde_json::Value::Null }),
    );
    let seal = support::seal(&[("hello.py", "aa")]);
    let a = attribute_seal(&seal, None, &parsed, &support::manifest("codex-cli"));
    assert_eq!(
        a.iter()
            .find(|x| x.path == "hello.py")
            .map(|x| &x.attribution),
        Some(&Attribution::Traced {
            call_id: "call_HttSZaWk9ISYQbx9iAb5SAPq".to_string(),
            tool: "exec".to_string(),
        })
    );
    assert!(
        support::seal_findings(&a, &parsed, &support::manifest("codex-cli")).is_empty(),
        "an honest Codex write raises nothing"
    );
}

/// The other half of the same branch: `exec` is still judged by the shell rule when the
/// write rule finds nothing, so a shell redirect inside the same tool still attributes —
/// and a command that only reads the sealed file still does not.
#[test]
fn a_codex_exec_shell_command_keeps_both_halves_of_the_rule() {
    let seal = support::seal(&[("hello.py", "aa")]);

    let wrote = support::parsed_with_call(
        "call_redirect",
        "exec",
        json!({ "input": "echo hi > /ws/task7/hello.py" }),
    );
    let a = attribute_seal(&seal, None, &wrote, &support::manifest("codex-cli"));
    assert!(matches!(a[0].attribution, Attribution::Traced { .. }));

    let read = support::parsed_with_call(
        "call_read",
        "exec",
        json!({ "input": "cat /ws/task7/hello.py 2>/dev/null" }),
    );
    let b = attribute_seal(&seal, None, &read, &support::manifest("codex-cli"));
    assert_eq!(b[0].attribution, Attribution::Unexplained);
}

/// The both-classes rule is not Codex's alone: `Bash` and `bash`
/// are declared write and shell in the two other shipped manifests, so `path_arg_attributes`
/// — and through it `patch_target_paths` — walks their arguments too. Without a guard,
/// a shell call that merely prints an `apply_patch` header would explain a file planted
/// through a side channel, which is exactly the Fail the seal exists to raise.
///
/// The guard is the `*** Begin Patch` envelope, and it is mitigation rather than closure:
/// the second half below pins the door that is still open, so the next reader finds the
/// limit here and not only in `docs/CHEAT-CHECKLIST.md`.
#[test]
fn a_bash_echo_of_a_patch_header_does_not_explain_a_planted_file() {
    let seal = support::seal(&[("planted.txt", "aa")]);
    let manifest = support::manifest("claude-code-cli");

    let echoed = support::parsed_with_call(
        "call_echo",
        "Bash",
        json!({ "command": "echo '*** Add File: planted.txt\n'" }),
    );
    let a = attribute_seal(&seal, None, &echoed, &manifest);
    assert_eq!(
        a[0].attribution,
        Attribution::Unexplained,
        "an echo writes nothing, so the planted file is still unexplained"
    );
    assert!(
        support::seal_findings(&a, &echoed, &manifest)
            .iter()
            .any(|f| f.code == "unexplained_seal_file"),
        "and the seal still raises the finding that is its whole job"
    );

    // Not vacuous, and honest about the bound: the same header line inside a real patch
    // envelope still attributes. The condition raised the bar; it did not shut the door.
    let quoted = support::parsed_with_call(
        "call_quote",
        "Bash",
        json!({ "command": "echo '*** Begin Patch\n*** Add File: planted.txt\n*** End Patch'" }),
    );
    let b = attribute_seal(&seal, None, &quoted, &manifest);
    assert!(
        matches!(b[0].attribution, Attribution::Traced { .. }),
        "the accepted gap in docs/CHEAT-CHECKLIST.md, pinned so it cannot widen unnoticed"
    );
}

/// A `mod support` inside the test file, as in `checks_v13.rs`, because Rust makes each
/// file under `tests/` its own crate.
mod support {
    use goalcup_protocol::chain::Link;
    use goalcup_protocol::cursor::{CursorUnit, StoreCursor};
    use goalcup_protocol::envelope::{Envelope, EventBody};
    use goalcup_protocol::manifest::ConnectorManifest;
    use goalcup_protocol::seal::{SealManifest, SealedFile};
    use goalcup_verifier::atif::{Agent, Message, Step, StepSource, ToolCall, Trajectory};
    use goalcup_verifier::gaps::{GapReason, classify};
    use goalcup_verifier::legs::{Leg1Record, Leg1Store};
    use goalcup_verifier::parsers::{ParsedLegs, ToolFact};
    use serde_json::{Value, json};

    /// The two heartbeats `classify_one_window` writes, and the three record timestamps
    /// that fall between them. One minute apart — the claude-code-cli manifest's own
    /// `heartbeat_seconds` — so the window is punctual and the only thing that can make it
    /// a gap is the cursor comparison under test.
    const BEFORE_AT: &str = "2026-09-04T08:00:00Z";
    const AFTER_AT: &str = "2026-09-04T08:01:00Z";

    /// A `StoreCursor` with the given unit and position; `describe` is `prefix` followed by
    /// the position, so the human form is present and demonstrably unread.
    pub fn cursor(store: &str, unit: CursorUnit, position: u64, prefix: &str) -> StoreCursor {
        StoreCursor {
            store: store.to_string(),
            unit,
            position: Some(position),
            describe: format!("{prefix}{position}"),
        }
    }

    /// Three leg-1 records in one store, timestamped inside the window the two heartbeats
    /// below bound. `Leg1Store::from_chain` is the constructor the chain loader itself
    /// uses, so these records live in `SeqSpace::EntryChain` and are placed by their own
    /// harness timestamps exactly as a daemon-written record is.
    pub fn three_records_in_the_window() -> Vec<Leg1Store> {
        let records = ["08:00:15Z", "08:00:30Z", "08:00:45Z"]
            .iter()
            .enumerate()
            .map(|(i, at)| {
                let seq = 20 + i as u64;
                Leg1Record {
                    store: "main".to_string(),
                    seq,
                    prev: "0".repeat(64),
                    hash: format!("{seq:064}"),
                    recorded_at: None,
                    value: json!({ "timestamp": format!("2026-09-04T{at}") }),
                    raw: Vec::new(),
                }
            })
            .collect();
        vec![Leg1Store::from_chain("main", records)]
    }

    /// The same store with nothing in the window: `main` has no `partition_key`, so a
    /// cursor that moved over an empty window is what `CursorAheadOfChain` reports.
    pub fn no_records_in_the_window() -> Vec<Leg1Store> {
        vec![Leg1Store::from_chain("main", Vec::new())]
    }

    /// One call to `gaps::classify` over exactly two heartbeats bounding one punctual
    /// window and carrying `before` and `after`, with no acknowledgements and the
    /// claude-code-cli manifest, returning the single resulting `Gap`'s reasons — or
    /// nothing, when the window produced no gap at all.
    pub fn classify_one_window(
        stores: Vec<Leg1Store>,
        before: StoreCursor,
        after: StoreCursor,
    ) -> Vec<GapReason> {
        let beat = |seq: u64, ts: &str, c: &StoreCursor| Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: "e".to_string(),
            source: "daemon".to_string(),
            recorded_at: ts.to_string(),
            link: Link {
                seq,
                prev: "0".repeat(64),
                hash: format!("{seq:064}"),
            },
            body: EventBody::Heartbeat {
                ts: ts.to_string(),
                stores_seen: vec![c.clone()],
                receiver_ok: true,
            },
            redaction: None,
        };
        let events = vec![beat(10, BEFORE_AT, &before), beat(40, AFTER_AT, &after)];
        let gaps = classify(&stores, &[], &events, &manifest("claude-code-cli"), &[]);
        assert!(gaps.len() <= 1, "one window, at most one gap: {gaps:?}");
        gaps.into_iter()
            .next()
            .map(|g| g.reasons)
            .unwrap_or_default()
    }

    /// Re-exported so the test's call reads like every other `support::` one.
    pub use goalcup_verifier::verdict::seal_findings;

    /// The workspace every constructed seal here declares. Never reported: it exists only
    /// so `attribute_seal` can build the absolute spelling it compares against.
    const WORKSPACE: &str = "/ws/task7";

    /// A `SealManifest` over `(relative_path, sha256)` pairs, path-sorted, with `bytes`
    /// set to the path's length so the field is not zero.
    pub fn seal(files: &[(&str, &str)]) -> SealManifest {
        let owned: Vec<(String, String)> = files
            .iter()
            .map(|(p, h)| ((*p).to_string(), (*h).to_string()))
            .collect();
        seal_owned(&owned)
    }

    /// The same, over owned pairs, for a list built in a loop.
    pub fn seal_owned(files: &[(String, String)]) -> SealManifest {
        SealManifest {
            entry_id: "task7".to_string(),
            sealed_at: "2026-09-08T00:00:00Z".to_string(),
            workspace: WORKSPACE.to_string(),
            files: files
                .iter()
                .map(|(path, sha256)| SealedFile {
                    path: path.clone(),
                    bytes: path.len() as u64,
                    sha256: sha256.clone(),
                })
                .collect(),
            excluded: Vec::new(),
        }
        .sorted()
    }

    /// A `ParsedLegs` with every collection empty — no tool call, so nothing can attribute.
    pub fn parsed_no_calls() -> ParsedLegs {
        ParsedLegs {
            trajectory: Trajectory::new(Agent {
                name: "task7".to_string(),
                version: "0".to_string(),
                model_name: None,
                tool_definitions: Vec::new(),
                extra: None,
            }),
            user_slots: Vec::new(),
            leg2_candidates: Vec::new(),
            sessions: Vec::new(),
            models: Vec::new(),
            tokens: Vec::new(),
            tool_calls: Vec::new(),
            config_events: Vec::new(),
            resume_markers: Vec::new(),
            blocked_prompts: Vec::new(),
            raw_bodies: Vec::new(),
            census: std::collections::BTreeMap::new(),
            tool_args_conflicts: Vec::new(),
        }
    }

    /// `parsed_no_calls()` plus one shell-class call (`Bash`, for `claude-code-cli`)
    /// carrying this `call_id` and command.
    pub fn parsed_with_shell_call(call_id: &str, command: &str) -> ParsedLegs {
        parsed_with_call(call_id, "Bash", json!({ "command": command }))
    }

    /// `parsed_no_calls()` plus one call of any class. The trajectory entry is the
    /// load-bearing one — `attribute_seal` reads `trajectory.steps[].tool_calls` — and the
    /// matching `ToolFact` is what a real parser would also have produced.
    pub fn parsed_with_call(call_id: &str, tool: &str, arguments: Value) -> ParsedLegs {
        let mut parsed = parsed_no_calls();
        parsed.trajectory.push(Step {
            step_id: 0,
            timestamp: None,
            source: StepSource::Agent,
            model_name: None,
            message: Message::Text(String::new()),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                tool_call_id: call_id.to_string(),
                function_name: tool.to_string(),
                arguments,
                extra: None,
            }],
            observation: None,
            metrics: None,
            llm_call_count: None,
            is_copied_context: None,
            extra: None,
        });
        parsed.tool_calls.push(ToolFact {
            call_id: call_id.to_string(),
            name: tool.to_string(),
            leg: "leg1",
            turn_key: None,
            order: 0,
        });
        parsed
    }

    /// The pre-start import, as one command.
    ///
    /// The shell rule attributes a file only when a write operator names *that path* as an
    /// argument token, so a recursive copy of a directory (`cp -r ~/prepared/. .`) attributes
    /// nothing at all and every sealed file would be `Unexplained`. The threat survives the
    /// rule by spelling the import out per file — still one `call_id`, which is what
    /// `bulk_attribution` counts. The recursive copy leads, so the shape of the attack is on
    /// the page.
    pub fn import_command(files: &[(String, String)]) -> String {
        std::iter::once("cp -r ~/prepared/. .".to_string())
            .chain(
                files
                    .iter()
                    .map(|(path, _)| format!("cp ~/prepared/{path} {path}")),
            )
            .collect::<Vec<_>>()
            .join(" && ")
    }

    /// `ConnectorManifest::from_toml_str` over the shipped `connectors/<id>/manifest.toml`,
    /// `validate()`d, panicking with the id on failure.
    pub fn manifest(id: &str) -> ConnectorManifest {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../connectors")
            .join(id)
            .join("manifest.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read the `{id}` connector manifest: {e}"));
        let m = ConnectorManifest::from_toml_str(&text)
            .unwrap_or_else(|e| panic!("parse the `{id}` connector manifest: {e}"));
        m.validate()
            .unwrap_or_else(|e| panic!("validate the `{id}` connector manifest: {e}"));
        m
    }
}
