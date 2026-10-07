//! V12: the acknowledgement anchors.
//!
//! The fixtures are loaded through `goalcup_verifier::Pipeline` — the sequence
//! `verify_trace_with_options` itself runs. A test that needs an acknowledgement stream or
//! a key the fixture does not carry borrows that context and overrides those fields, so
//! everything it does not name is what the shipped verifier would have handed the check.

// The shared fixture helpers. This file's scenario builders remain under its local
// `support` module, so the common helpers use a distinct module name.
#[path = "support/mod.rs"]
mod fixture_support;

use ed25519_dalek::{Signer, SigningKey};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_verifier::Pipeline;
use goalcup_verifier::checks::v12_anchors::{LATE_FIRST_ANCHOR_SECONDS, anchor_timeline};
use goalcup_verifier::checks::{CheckResult, Severity, all_checks};
use goalcup_verifier::gaps::verify_acks;
use goalcup_verifier::legs::Leg1Store;

/// One of the three committed M0 traces, loaded the way the verifier loads it.
fn fixture(id: &str) -> Pipeline {
    fixture_support::pipeline::m0(id)
}

/// The `main` leg-1 store of a loaded fixture, which is where these tests take a real
/// sequence and a real record hash from to acknowledge.
fn main_store(p: &Pipeline) -> &Leg1Store {
    p.context()
        .stores
        .iter()
        .find(|s| s.name == "main")
        .expect("every M0 fixture declares a `main` store")
}

/// Run V12 alone, with an acknowledgement stream and key supplied by the test.
fn run_v12(p: &Pipeline, acks: &[AckCheckpoint], key: Option<&str>) -> CheckResult {
    // The descriptor with the key the caller is standing in for. These tests exercise V12
    // directly, so they play `verify_trace`'s allowlist decision: `server_key` below is the
    // resolved key, and the descriptor carries it only so the two agree.
    let mut trace = p.context().trace.clone();
    trace.server_public_key = key.map(str::to_string);
    // Reconciled once and handed to the check, exactly as `verify_trace` does it. V12 does
    // not call `verify_acks` itself.
    // `None`: these are M0 traces with no chain, so the legacy per-store path is the one
    // under test here.
    let reconciliation = verify_acks(None, p.context().stores, acks, key, &trace.entry_id);
    let mut cx = p.context();
    cx.trace = &trace;
    cx.acks = acks;
    cx.acks_verified = &reconciliation;
    cx.server_key = key;
    let v12 = all_checks()
        .into_iter()
        .find(|c| c.id() == "V12")
        .expect("V12 is in all_checks()");
    v12.run(&cx)
}

#[test]
fn all_checks_returns_thirteen_in_order() {
    let ids: Vec<&str> = all_checks().iter().map(|c| c.id()).collect();
    assert_eq!(
        ids,
        vec![
            "V1", "V2", "V3", "V4", "V5", "V6", "V7", "V8", "V9", "V10", "V11", "V12", "V13"
        ]
    );
}

#[test]
fn a_trace_with_no_acknowledgements_raises_no_anchor_as_a_flag() {
    // All three M0 fixture sets are in this state, and it is the honest reading: nothing
    // to reconcile against is not the same as nothing to report.
    for id in ["claude-code-cli", "codex-cli"] {
        let f = fixture(id);
        let r = run_v12(&f, &[], None);
        assert_eq!(r.verdict, Severity::Flag, "{id}: {:?}", r.findings);
        let finding = r.findings.iter().find(|x| x.code == "no_anchor").unwrap();
        assert_eq!(finding.severity, Severity::Flag);
        assert!(
            finding.message.contains("0 acknowledgement"),
            "{id}: {}",
            finding.message
        );
    }
}

#[test]
fn an_honest_acknowledgement_stream_passes_and_reports_what_it_checked() {
    let f = fixture("claude-code-cli");
    let main = main_store(&f);
    let server = SigningKey::from_bytes(&[5u8; 32]);
    let key = hex::encode(server.verifying_key().to_bytes());
    let acks: Vec<AckCheckpoint> = [49usize, 99, 147]
        .iter()
        .map(|i| {
            let r = &main.records[*i];
            let mut a = AckCheckpoint {
                entry_id: "m0-spike4".into(),
                seq: r.seq,
                hash: r.hash.clone(),
                acked_at: "2026-09-03T07:51:11Z".into(),
                server_signature: String::new(),
            };
            a.server_signature = hex::encode(server.sign(&a.signing_bytes().unwrap()).to_bytes());
            a
        })
        .collect();

    let r = run_v12(&f, &acks, Some(&key));
    assert_eq!(r.verdict, Severity::Pass, "{:?}", r.findings);
    let msg = &r.findings[0].message;
    assert!(msg.contains("3 acknowledgement"), "{msg}");
    assert!(msg.contains("3 verified"), "{msg}");
    assert!(!r.findings.iter().any(|x| x.code == "no_anchor"));
}

#[test]
fn a_chain_rewritten_below_the_last_acknowledgement_fails() {
    // The attack: re-chain from seq 1 with the instructions removed, below the last ack.
    // The result recomputes cleanly and yields no gaps — and fails here.
    let f = fixture("claude-code-cli");
    let main = main_store(&f);
    let acks = vec![AckCheckpoint {
        entry_id: "m0-spike4".into(),
        seq: main.records[49].seq,
        // What the server saw. The chain now holds something else at that sequence.
        hash: "ab".repeat(32),
        acked_at: "2026-09-03T07:45:00Z".into(),
        server_signature: String::new(),
    }];
    let r = run_v12(&f, &acks, None);
    assert_eq!(r.verdict, Severity::Fail);
    let finding = r
        .findings
        .iter()
        .find(|x| x.code == "ack_chain_mismatch")
        .unwrap();
    assert_eq!(finding.severity, Severity::Fail);
    assert!(finding.message.contains(&main.records[49].seq.to_string()));
    assert!(
        !finding.evidence.is_empty(),
        "the finding must cite the chain position"
    );
}

#[test]
fn a_rewritten_acked_at_fails_even_though_the_hash_and_the_key_are_genuine() {
    // The attack: stay offline all week, fabricate the run, connect an hour before
    // the deadline, then move the server's acknowledgement time back so the evidence
    // reads as anchored while it happened. `acked_at` is inside the signature, so the
    // rewrite breaks it.
    let f = fixture("claude-code-cli");
    let main = main_store(&f);
    let server = SigningKey::from_bytes(&[5u8; 32]);
    let key = hex::encode(server.verifying_key().to_bytes());
    let r0 = &main.records[49];
    let mut ack = AckCheckpoint {
        entry_id: "m0-spike4".into(),
        seq: r0.seq,
        // The honest hash, off the participant's own chain, genuinely acknowledged.
        hash: r0.hash.clone(),
        // The hour the server really saw it: an hour before the deadline.
        acked_at: "2026-09-10T06:45:00Z".into(),
        server_signature: String::new(),
    };
    ack.server_signature = hex::encode(server.sign(&ack.signing_bytes().unwrap()).to_bytes());
    assert!(ack.verify(&key).is_ok(), "the genuine acknowledgement");

    // Backdate it to the week the work is claimed to have happened in.
    let backdated = AckCheckpoint {
        acked_at: "2026-09-03T07:45:00Z".into(),
        ..ack
    };
    assert!(backdated.verify(&key).is_err());

    let r = run_v12(&f, &[backdated], Some(&key));
    assert_eq!(r.verdict, Severity::Fail);
    assert!(r.findings.iter().any(|x| x.code == "ack_signature_invalid"));
}

#[test]
fn a_forged_signature_fails_even_when_the_hash_matches() {
    // The attack: hand-write the acknowledgement stream for the offline self-check. The
    // hashes are correct, because the participant read them off their own chain.
    let f = fixture("claude-code-cli");
    let main = main_store(&f);
    let server = SigningKey::from_bytes(&[5u8; 32]);
    let forger = SigningKey::from_bytes(&[6u8; 32]);
    let r0 = &main.records[49];
    let mut ack = AckCheckpoint {
        entry_id: "m0-spike4".into(),
        seq: r0.seq,
        hash: r0.hash.clone(),
        acked_at: "2026-09-03T07:45:00Z".into(),
        server_signature: String::new(),
    };
    ack.server_signature = hex::encode(forger.sign(&ack.signing_bytes().unwrap()).to_bytes());

    let key = hex::encode(server.verifying_key().to_bytes());
    let r = run_v12(&f, &[ack], Some(&key));
    assert_eq!(r.verdict, Severity::Fail);
    assert!(r.findings.iter().any(|x| x.code == "ack_signature_invalid"));
}

#[test]
fn acknowledgements_with_no_pinned_key_are_reported_as_unverified() {
    let f = fixture("claude-code-cli");
    let main = main_store(&f);
    let r0 = &main.records[49];
    let acks = vec![AckCheckpoint {
        entry_id: "m0-spike4".into(),
        seq: r0.seq,
        hash: r0.hash.clone(),
        acked_at: "2026-09-03T07:45:00Z".into(),
        server_signature: "aa".repeat(64),
    }];
    let r = run_v12(&f, &acks, None);
    // The hashes reconcile, so this is not a Fail — but the signatures were not checked,
    // and saying "verified" would be a lie.
    assert_eq!(r.verdict, Severity::Flag);
    assert!(r.findings.iter().any(|x| x.code == "anchor_unverified"));
    // And the timeline says why it is empty: blaming unreadable timestamps would point a
    // reader at the acknowledgements when the thing missing is the key that would let
    // anyone check them — `acked_at` here parses fine.
    let tl = &r
        .findings
        .iter()
        .find(|x| x.code == "anchor_timeline")
        .unwrap()
        .message;
    assert!(tl.contains("no pinned server key"), "{tl}");
    assert!(!tl.contains("no readable time"), "{tl}");
}

#[test]
fn an_ack_from_another_entry_fails_even_when_validly_signed_and_the_hash_reconciles() {
    // `AckCheckpoint::signing_bytes` includes `entry_id`, but a validly
    // signed acknowledgement only promises the server once acknowledged some entry's chain
    // at that sequence — never that it is this trace's. Replaying a genuinely
    // server-signed acknowledgement from another entry, at a seq/hash pair that happens to
    // match this chain, must still fail.
    let f = fixture("claude-code-cli");
    let main = main_store(&f);
    let server = SigningKey::from_bytes(&[5u8; 32]);
    let key = hex::encode(server.verifying_key().to_bytes());
    let r0 = &main.records[49];
    let mut ack = AckCheckpoint {
        entry_id: "some-other-entry".into(),
        seq: r0.seq,
        hash: r0.hash.clone(),
        acked_at: "2026-09-03T07:45:00Z".into(),
        server_signature: String::new(),
    };
    ack.server_signature = hex::encode(server.sign(&ack.signing_bytes().unwrap()).to_bytes());

    let r = run_v12(&f, &[ack], Some(&key));
    assert_eq!(r.verdict, Severity::Fail, "{:?}", r.findings);
    let finding = r
        .findings
        .iter()
        .find(|x| x.code == "ack_entry_mismatch")
        .unwrap();
    assert_eq!(finding.severity, Severity::Fail);
    assert!(
        finding.message.contains("some-other-entry"),
        "{}",
        finding.message
    );
    assert!(finding.message.contains("m0-spike4"), "{}", finding.message);
}

// ---------------------------------------------------------------------------------------
// The anchor timeline.
// ---------------------------------------------------------------------------------------

#[test]
fn offline_all_week_then_one_upload_is_flagged() {
    // started_at Monday; the only verified acknowledgement lands the following Sunday.
    let started = goalcup_protocol::control::parse_z("2026-09-01T00:00:00Z").unwrap();
    let acks = vec![support::signed_ack(120, "2026-09-07T23:00:00Z")];
    let t = anchor_timeline(&acks, &[0], Some(started), None);
    assert_eq!(t.verified, 1);
    assert_eq!(t.first_verified_at.as_deref(), Some("2026-09-07T23:00:00Z"));
    assert_eq!(t.last_verified_at.as_deref(), Some("2026-09-07T23:00:00Z"));
    assert!(t.longest_unanchored_seconds > LATE_FIRST_ANCHOR_SECONDS);

    let r = support::run_v12_with(&acks, &[0], Some(started));
    assert_eq!(r.verdict, Severity::Flag);
    let f = r
        .findings
        .iter()
        .find(|f| f.code == "late_first_anchor")
        .expect("late_first_anchor");
    assert!(f.message.contains("first verified"), "{}", f.message);
    let tl = r
        .findings
        .iter()
        .find(|f| f.code == "anchor_timeline")
        .unwrap();
    assert_eq!(tl.severity, Severity::Pass);
    assert!(tl.message.contains("2026-09-07T23:00:00Z"));
}

#[test]
fn hourly_acknowledgements_across_the_week_raise_nothing() {
    let started = goalcup_protocol::control::parse_z("2026-09-01T00:00:00Z").unwrap();
    let acks: Vec<_> = (0..24 * 7)
        .map(|h| support::signed_ack(h as u64, &support::plus_hours("2026-09-01T00:00:00Z", h)))
        .collect();
    let verified: Vec<usize> = (0..acks.len()).collect();
    let t = anchor_timeline(&acks, &verified, Some(started), None);
    assert_eq!(t.longest_unanchored_seconds, 3_600);
    let r = support::run_v12_with(&acks, &verified, Some(started));
    assert_eq!(r.verdict, Severity::Pass, "{:?}", r.findings);
}

#[test]
fn the_headline_counts_only_what_was_signature_checked() {
    // Two acknowledgements reconcile against the chain; only one has a verifiable
    // signature.
    let r = support::run_v12_with(&support::two_acks(), &[1], None);
    let m = &r
        .findings
        .iter()
        .find(|f| f.code == "anchors_checked")
        .unwrap()
        .message;
    assert!(m.contains("1 verified"), "{m}");
    assert!(!m.contains("2 matched"), "{m}");
    // Stricter than the line above: the word `matched` must not survive anywhere in the
    // sentence, `mismatched` included.
    assert!(!m.contains("matched"), "{m}");
}

#[test]
fn an_unverified_acknowledgement_is_never_an_anchor_time() {
    // `two_acks()` is an hour apart and only the second verifies. Reading the first as an
    // anchor would say the server saw this chain an hour earlier than anything here can be
    // held to — which is the whole distinction the timeline exists to draw.
    let acks = support::two_acks();
    let t = anchor_timeline(&acks, &[1], None, None);
    assert_eq!(t.verified, 1);
    assert_eq!(t.first_verified_at.as_deref(), Some("2026-09-01T01:00:00Z"));
    assert_eq!(t.longest_unanchored_seconds, 0);
    // The same two acknowledgements with both counted, to show the difference is real and
    // not an artefact of there being one time either way.
    let both = anchor_timeline(&acks, &[0, 1], None, None);
    assert_eq!(
        both.first_verified_at.as_deref(),
        Some("2026-09-01T00:00:00Z")
    );
    assert_eq!(both.longest_unanchored_seconds, 3_600);
}

#[test]
fn the_timeline_reads_the_signed_time_and_not_the_order_of_the_file() {
    // Written newest-first, which costs a participant nothing: the order of lines in
    // `acks.jsonl` is theirs to choose, and `acked_at` is not — it is inside the
    // signature. A timeline read off file order would call the last upload the first
    // anchor and report a span of zero.
    let acks = vec![
        support::signed_ack(3, "2026-09-08T00:00:00Z"),
        support::signed_ack(2, "2026-09-04T00:00:00Z"),
        support::signed_ack(1, "2026-09-01T00:00:00Z"),
    ];
    let t = anchor_timeline(&acks, &[0, 1, 2], None, None);
    assert_eq!(t.first_verified_at.as_deref(), Some("2026-09-01T00:00:00Z"));
    assert_eq!(t.last_verified_at.as_deref(), Some("2026-09-08T00:00:00Z"));
    assert_eq!(t.longest_unanchored_seconds, 4 * 86_400);

    let started = goalcup_protocol::control::parse_z("2026-09-01T00:00:00Z").unwrap();
    let r = support::run_v12_with(&acks, &[0, 1, 2], Some(started));
    assert_eq!(r.verdict, Severity::Flag, "{:?}", r.findings);
    let f = r
        .findings
        .iter()
        .find(|f| f.code == "long_unanchored_span")
        .expect("a four-day hole between anchors");
    // The first anchor is on time, so the span gets its own code and the late-first one is
    // absent: a reader of the queue can tell a run that started unwitnessed from one that
    // went dark in the middle.
    assert!(
        !r.findings.iter().any(|f| f.code == "late_first_anchor"),
        "{:?}",
        r.findings
    );
    assert!(!f.message.contains("first verified"), "{}", f.message);
    assert!(f.message.contains("345600s"), "{}", f.message);
}

#[test]
fn the_silence_between_the_last_acknowledgement_and_the_seal_is_measured() {
    // The hole after the last anchor. Anchor hourly for a
    // day, stop, and seal thirty hours later: every span *between* anchors is an hour, the
    // first is on time, and the run is unwitnessed for the last thirty hours of its life.
    let start = "2026-09-01T00:00:00Z";
    let started = goalcup_protocol::control::parse_z(start).unwrap();
    let acks: Vec<_> = (0..24)
        .map(|h| support::signed_ack(h as u64, &support::plus_hours(start, h)))
        .collect();
    let verified: Vec<usize> = (0..acks.len()).collect();
    let sealed_at = support::plus_hours(start, 23 + 30);
    let chain = support::sealed_chain(&sealed_at);

    let ended = goalcup_protocol::control::parse_z(&sealed_at).unwrap();
    let t = anchor_timeline(&acks, &verified, Some(started), Some(ended));
    assert_eq!(t.tail_unanchored_seconds, Some(30 * 3_600));
    assert_eq!(t.longest_unanchored_seconds, 30 * 3_600);

    let r = support::run_v12_sealed(&acks, &verified, Some(started), Some(&chain));
    assert_eq!(r.verdict, Severity::Flag, "{:?}", r.findings);
    let f = r
        .findings
        .iter()
        .find(|f| f.code == "long_unanchored_span")
        .expect("thirty hours of silence before the seal");
    assert!(f.message.contains("108000s"), "{}", f.message);
    assert!(
        f.message.contains("end of the run"),
        "the finding says the hole is the tail: {}",
        f.message
    );
    assert!(
        !r.findings.iter().any(|f| f.code == "late_first_anchor"),
        "the first anchor is on time: {:?}",
        r.findings
    );
}

#[test]
fn hourly_acknowledgements_up_to_the_seal_raise_nothing() {
    // The same shape with the tail closed: the last acknowledgement is an hour before the
    // seal, exactly as every other span is an hour. Without this the test above would pass
    // against a check that flagged every sealed trace.
    let start = "2026-09-01T00:00:00Z";
    let started = goalcup_protocol::control::parse_z(start).unwrap();
    let acks: Vec<_> = (0..24)
        .map(|h| support::signed_ack(h as u64, &support::plus_hours(start, h)))
        .collect();
    let verified: Vec<usize> = (0..acks.len()).collect();
    let chain = support::sealed_chain(&support::plus_hours(start, 24));

    let r = support::run_v12_sealed(&acks, &verified, Some(started), Some(&chain));
    assert_eq!(r.verdict, Severity::Pass, "{:?}", r.findings);
    let tl = r
        .findings
        .iter()
        .find(|f| f.code == "anchor_timeline")
        .unwrap();
    assert!(
        tl.message.contains("3600s is after the last one"),
        "the timeline states the tail it measured: {}",
        tl.message
    );
}

#[test]
fn the_run_end_comes_from_the_chains_seal_link_and_not_the_declared_file() {
    // `seal.json` sits at a participant-named path and the chain's seal link does not, so
    // where the two disagree the tail is dated by the link. A participant who wanted the
    // thirty-hour hole above to disappear would edit the file.
    let start = "2026-09-01T00:00:00Z";
    let started = goalcup_protocol::control::parse_z(start).unwrap();
    let acks = vec![support::signed_ack(1, start)];
    // The chain says the run sealed thirty hours after its only acknowledgement; the
    // declared manifest says it sealed a minute after.
    let chain = support::sealed_chain(&support::plus_hours(start, 30));
    let declared = support::seal_manifest("2026-09-01T00:01:00Z");

    let r = support::run_v12_full(&acks, &[0], Some(started), Some(&chain), Some(&declared));
    assert_eq!(r.verdict, Severity::Flag, "{:?}", r.findings);
    let f = r
        .findings
        .iter()
        .find(|f| f.code == "long_unanchored_span")
        .expect("the chain's own seal time is what the tail is measured to");
    assert!(f.message.contains("108000s"), "{}", f.message);
}

#[test]
fn a_verified_acknowledgement_with_an_unreadable_time_is_counted_and_not_placed() {
    // The headline count and the timeline's population are different numbers, and the
    // check says which is which rather than quietly reporting the smaller one twice.
    let acks = vec![support::signed_ack(7, "whenever")];
    let t = anchor_timeline(&acks, &[0], None, None);
    assert_eq!(t.verified, 0);
    assert_eq!(t.first_verified_at, None);
    assert_eq!(t.longest_unanchored_seconds, 0);

    let r = support::run_v12_with(&acks, &[0], None);
    assert_eq!(r.verdict, Severity::Pass, "{:?}", r.findings);
    let headline = &r
        .findings
        .iter()
        .find(|f| f.code == "anchors_checked")
        .unwrap()
        .message;
    assert!(headline.contains("1 verified"), "{headline}");
    let tl = &r
        .findings
        .iter()
        .find(|f| f.code == "anchor_timeline")
        .unwrap()
        .message;
    assert!(tl.contains("no readable time"), "{tl}");
    assert!(tl.contains("1 verified acknowledgement(s)"), "{tl}");
}

/// The in-memory helpers the anchor-timeline tests run on. Nothing here loads a trace
/// directory: V12 reads `acks`, `acks_verified`, `server_key`, `receipt` and the
/// descriptor's `entry_id`, and every other field of `CheckContext` is the cheapest valid
/// empty value. Nothing is read from or written under `spikes/`.
mod support {
    use ed25519_dalek::{Signer, SigningKey};
    use goalcup_protocol::ack::AckCheckpoint;
    use goalcup_protocol::chain::{GENESIS, Link};
    use goalcup_protocol::control::{format_z, parse_z};
    use goalcup_protocol::envelope::{Envelope, EventBody};
    use goalcup_protocol::manifest::ConnectorManifest;
    use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
    use goalcup_protocol::seal::SealManifest;
    use goalcup_verifier::atif::{Agent, Trajectory};
    use goalcup_verifier::chain::VerifiedChain;
    use goalcup_verifier::checks::{CheckContext, CheckResult, all_checks};
    use goalcup_verifier::gaps::AckReconciliation;
    use goalcup_verifier::human_rule::HumanRuleOutcome;
    use goalcup_verifier::legs::Leg2Stream;
    use goalcup_verifier::parsers::ParsedLegs;
    use goalcup_verifier::trace::TraceDescriptor;
    use std::path::PathBuf;

    /// The entry every helper acknowledgement names, and the one the helper descriptor
    /// declares — so an acknowledgement here is never an `ack_entry_mismatch` by accident.
    pub const ENTRY_ID: &str = "e1";

    /// The seed the rest of the suite pins as "the server": its verifying key is the one
    /// entry in the verifier's own allowlist.
    fn server() -> SigningKey {
        SigningKey::from_bytes(&[5u8; 32])
    }

    fn sign_with(key: &SigningKey, seq: u64, acked_at: &str) -> AckCheckpoint {
        let mut a = AckCheckpoint {
            entry_id: ENTRY_ID.to_string(),
            seq,
            hash: "ab".repeat(32),
            acked_at: acked_at.to_string(),
            server_signature: String::new(),
        };
        a.server_signature = hex::encode(key.sign(&a.signing_bytes().unwrap()).to_bytes());
        a
    }

    /// An `AckCheckpoint` over `(entry_id: "e1", seq, hash: "ab".repeat(32), acked_at)`,
    /// signed with the server key over the `ACK_DOMAIN`-tagged bytes.
    pub fn signed_ack(seq: u64, acked_at: &str) -> AckCheckpoint {
        sign_with(&server(), seq, acked_at)
    }

    /// `base` (RFC 3339 with a trailing `Z`) advanced by `hours`, back as RFC 3339.
    pub fn plus_hours(base: &str, hours: i64) -> String {
        let base_secs = parse_z(base).unwrap_or_else(|| panic!("`{base}` is not an RFC 3339 Z"));
        let shifted = base_secs as i64 + hours * 3_600;
        assert!(
            shifted >= 0,
            "`{base}` shifted by {hours}h is before the epoch"
        );
        format_z(shifted as u64)
    }

    /// Two `signed_ack`s an hour apart, the first signed with a different key, so both
    /// reconcile against the chain and exactly one verifies.
    pub fn two_acks() -> Vec<AckCheckpoint> {
        vec![
            sign_with(
                &SigningKey::from_bytes(&[6u8; 32]),
                1,
                "2026-09-01T00:00:00Z",
            ),
            signed_ack(2, "2026-09-01T01:00:00Z"),
        ]
    }

    /// Run V12 alone over an acknowledgement stream, an already-decided `verified` set and
    /// an optional declared start. `mismatches` is empty on purpose: these tests are about
    /// what the check says when nothing contradicts the chain, which is exactly the state
    /// a bare match count reads as "everything matched".
    pub fn run_v12_with(
        acks: &[AckCheckpoint],
        verified: &[usize],
        started_at: Option<u64>,
    ) -> CheckResult {
        run_v12_full(acks, verified, started_at, None, None)
    }

    /// The same, with a chain whose seal link says where the run ended.
    pub fn run_v12_sealed(
        acks: &[AckCheckpoint],
        verified: &[usize],
        started_at: Option<u64>,
        chain: Option<&VerifiedChain>,
    ) -> CheckResult {
        run_v12_full(acks, verified, started_at, chain, None)
    }

    /// A one-link chain carrying nothing but a seal at `sealed_at`. V12 reads the chain
    /// for exactly one thing — where the run ended — so this is the whole fixture. The
    /// link's own hash is not computed and does not need to be: `chain::load_and_verify`
    /// is what checks links, and V12 recomputes none.
    pub fn sealed_chain(sealed_at: &str) -> VerifiedChain {
        let seal = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: ENTRY_ID.to_string(),
            source: "daemon".to_string(),
            recorded_at: sealed_at.to_string(),
            link: Link {
                seq: 0,
                prev: GENESIS.to_string(),
                hash: "0".repeat(64),
            },
            body: EventBody::Seal {
                manifest_sha256: "0".repeat(64),
                sealed_at: sealed_at.to_string(),
                file_count: 0,
            },
            redaction: None,
        };
        VerifiedChain {
            entry_id: ENTRY_ID.to_string(),
            genesis_link: seal.link.hash.clone(),
            head_seq: 0,
            head_hash: seal.link.hash.clone(),
            envelopes: vec![seal],
            stores: Vec::new(),
            posts: Leg2Stream { posts: Vec::new() },
            events: Vec::new(),
            redactions: Vec::new(),
            seal_link: Some(0),
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

    /// A declared `seal.json` naming `sealed_at` and nothing else — the participant-written
    /// half of the pair, for the test that the chained time is what wins.
    pub fn seal_manifest(sealed_at: &str) -> SealManifest {
        SealManifest {
            entry_id: ENTRY_ID.to_string(),
            sealed_at: sealed_at.to_string(),
            workspace: "/ws".to_string(),
            files: Vec::new(),
            excluded: Vec::new(),
        }
    }

    pub fn run_v12_full(
        acks: &[AckCheckpoint],
        verified: &[usize],
        started_at: Option<u64>,
        chain: Option<&VerifiedChain>,
        seal: Option<&SealManifest>,
    ) -> CheckResult {
        let key = hex::encode(server().verifying_key().to_bytes());
        let reconciliation = AckReconciliation {
            mismatches: Vec::new(),
            verified: verified.to_vec(),
        };
        let receipt = started_at.map(|s| {
            StartReceipt::sign(
                StartReceiptClaims {
                    entry_id: ENTRY_ID.to_string(),
                    connector_id: "claude-code-cli".to_string(),
                    workspace: "/ws".to_string(),
                    session_id: "s1".to_string(),
                    started_at: format_z(s),
                    deadline_at: format_z(s + 7 * 24 * 3_600),
                    manifest_sha256: "0".repeat(64),
                    chain_head: "0".repeat(64),
                    // V12 reads the receipt for its `started_at` and for nothing else, so
                    // the connector-directory claim is left absent here rather than given a
                    // value this scenario would not be measuring.
                    connector_dir_sha256: None,
                    run_public_key: None,
                    questionnaire_version: None,
                },
                &server(),
            )
            .expect("sign the receipt")
        });
        let cx = CheckContext {
            trace: &trace(),
            manifest: &manifest(),
            parsed: &parsed(),
            human: &human(),
            join_strategy: "prompt_id",
            stores: &[],
            events: &[],
            acks,
            acks_verified: &reconciliation,
            seal,
            gaps: &[],
            server_key: Some(&key),
            key_source: goalcup_verifier::KeySource::Pinned,
            dev_mode: false,
            chain,
            receipt: receipt.as_ref(),
            baseline: None,
            // No fixture to borrow from: this context is synthetic from the descriptor
            // up, built to put an acknowledgement timeline in front of V12 and nothing
            // else. V12 reads neither field, and a resolved pin would be a claim about a
            // manifest no trace here names.
            manifest_sha256: "",
            manifest_pin: goalcup_verifier::ManifestPinning::default(),
        };
        all_checks()
            .into_iter()
            .find(|c| c.id() == "V12")
            .expect("V12 is in all_checks()")
            .run(&cx)
    }

    /// One of the committed connector manifests — never one built by hand. V12 never reads
    /// it; `CheckContext` has no optional fields, so it still has to be a real one.
    fn manifest() -> ConnectorManifest {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../connectors/claude-code-cli/manifest.toml");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        ConnectorManifest::from_toml_str(&text).unwrap()
    }

    fn trace() -> TraceDescriptor {
        // V12 reads descriptor identity fields but no evidence bytes in this synthetic
        // context. Loading a tiny descriptor keeps its rooted capability private rather
        // than constructing one in an integration test.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("trace.toml"),
            format!(
                "connector = \"claude-code-cli\"\nentry_id = \"{ENTRY_ID}\"\nworkspace = \"/ws\"\ntracked_sessions = [\"s1\"]\nharness_version = \"0.0.0\"\nconnector_manifest = \"manifest.toml\"\nchain = \"chain.jsonl\"\n"
            ),
        )
        .unwrap();
        TraceDescriptor::load(dir.path()).unwrap()
    }

    fn parsed() -> ParsedLegs {
        ParsedLegs {
            trajectory: Trajectory {
                schema_version: goalcup_verifier::atif::SCHEMA_VERSION.to_string(),
                session_id: None,
                trajectory_id: None,
                agent: Agent {
                    name: "test".to_string(),
                    version: "0".to_string(),
                    model_name: None,
                    tool_definitions: Vec::new(),
                    extra: None,
                },
                steps: Vec::new(),
                notes: None,
                final_metrics: None,
                continued_trajectory_ref: None,
                extra: None,
                subagent_trajectories: Vec::new(),
            },
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

    fn human() -> HumanRuleOutcome {
        HumanRuleOutcome {
            messages: Vec::new(),
            excluded: Vec::new(),
            unwitnessed: Vec::new(),
            leg1_without_post: Vec::new(),
            leg2_without_record: Vec::new(),
            duplicate_pairings: Vec::new(),
            divergent: Vec::new(),
        }
    }
}
