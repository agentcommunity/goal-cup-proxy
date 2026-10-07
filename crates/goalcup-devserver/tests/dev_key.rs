//! What this server's signature is worth, said out loud.
//!
//! The default key IS `PLACEHOLDER_SERVER_PUBLIC_KEY` — the seed `[5u8; 32]` is written in
//! this repository — so a trace this server signed reads as `KeySource::Pinned` and V13
//! says `placeholder_server_key` over it. That Flag is the whole point: the strongest word
//! a verdict has for a signature must not be reachable by anybody holding a public seed
//! without the verdict saying so. `--dev` suppresses it, and replacing the pinned key with
//! the organiser's is the release gate, after which nothing this server signed verifies.
//!
//! Below that, four rules: the run key is the RECEIPT's, an acknowledgement is earned by
//! envelopes that actually link, the dates on a receipt are the server's clock rather than
//! the client's word, and the anchor does not advance on `seq` alone.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_devserver::{DevError, DevServer};
use support::chain;

#[test]
fn the_default_key_is_the_placeholder_and_says_so_in_every_verdict() {
    // The dev key IS `PLACEHOLDER_SERVER_PUBLIC_KEY` — the seed is `[5u8; 32]`, the same
    // one `goalcup-verifier`'s compliant fixture signs with — so this crate adds no second
    // public seed and asks the verifier to pin nothing new.
    let key = goalcup_devserver::dev_server_public_key_hex();
    assert_eq!(key, goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY);
    assert!(goalcup_verifier::SERVER_PUBLIC_KEYS.contains(&key.as_str()));

    let t = support::trace_signed_by_this_devserver();
    // Strictly: the signature stands, and the verdict says out loud which key it stood
    // against. Not `receipt_unverified` — that Flag means no key resolved at all.
    let strict = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert_eq!(strict.key_source, goalcup_verifier::KeySource::Pinned);
    assert!(
        strict
            .review_queue
            .iter()
            .any(|f| f.code == "placeholder_server_key"),
        "a seed in this repository vouches for nothing on its own: {:?}",
        strict.review_queue
    );

    // Under `--dev` that one Flag is suppressed and nothing replaces it: this is the reading
    // the round trip asserts, and the reading the committed compliant fixture's
    // `expected-verdict.json` already records.
    let dev = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: Vec::new(),
            dev_mode: true,
        },
    )
    .unwrap();
    assert_eq!(dev.key_source, goalcup_verifier::KeySource::Pinned);
    assert!(dev.review_queue.is_empty(), "{:?}", dev.review_queue);
}

#[test]
fn a_devserver_under_key_seed_is_supplied_rather_than_pinned() {
    // The `--key-seed` path, and the only place `extra_server_keys` is needed. A seed the
    // build does not pin cannot reach `Pinned` however it is passed, so `server_key_unpinned`
    // is the honest reading and `dev_mode` does not touch it.
    let s = DevServer::with_seed([7u8; 32]);
    assert_ne!(
        s.public_key_hex(),
        goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY
    );

    let t = support::trace_signed_by(&s);
    let v = goalcup_verifier::verify_trace_with_options(
        t.path(),
        goalcup_verifier::VerifyOptions {
            allow_descriptor_key: false,
            extra_server_keys: vec![s.public_key_hex()],
            dev_mode: true,
        },
    )
    .unwrap();
    assert_eq!(
        v.key_source,
        goalcup_verifier::KeySource::Supplied,
        "a borrowed anchor is recorded as borrowed"
    );
    assert!(
        v.review_queue
            .iter()
            .any(|f| f.code == "server_key_unpinned")
    );

    // And with nothing supplied, no key resolves: `receipt_unverified`, not `receipt_invalid`,
    // because nothing was checked rather than checked and found wanting.
    let none = goalcup_verifier::verify_trace(t.path()).unwrap();
    assert_eq!(none.key_source, goalcup_verifier::KeySource::None);
    assert!(
        none.review_queue
            .iter()
            .any(|f| f.code == "receipt_unverified")
    );
}

/// The two dates on a receipt are the SERVER's, not the participant's.
///
/// A participant who picks their own `started_at` picks their own deadline, and the rule
/// says success after dispatch is immutable precisely so that they cannot. The request here
/// asks for 1999 and is told 2026, a week wide.
#[test]
fn a_receipt_is_dated_by_the_server_clock_and_issued_once() {
    use goalcup_protocol::receipt::ReceiptError;
    let s = DevServer::new();
    let links = chain::chain("e1");
    let run = chain::run_key();
    let request = chain::receipt_request("e1", &links[0].link.hash, &chain::run_key_hex(&run));
    assert_eq!(request.claims.started_at, "1999-01-01T00:00:00Z");

    s.set_clock(chain::AT);
    let r = s.issue_receipt(request.clone()).unwrap();
    assert_eq!(
        (r.claims.started_at.as_str(), r.claims.deadline_at.as_str()),
        (chain::AT, chain::DEADLINE),
        "started_at is the clock at issuance and deadline_at is that plus 168 hours"
    );
    // Everything the server does not compute is the request's, unchanged.
    assert_eq!(r.claims.chain_head, request.claims.chain_head);
    assert_eq!(r.claims.session_id, request.claims.session_id);

    assert_eq!(
        r.verify_with(&goalcup_devserver::dev_server_public_key_hex()),
        Ok(())
    );
    assert!(matches!(
        r.verify_with(&"11".repeat(32)),
        Err(ReceiptError::KeyMismatch { .. })
    ));

    // Existing default callers retain the one-shot refusal. The configurable
    // qualification-window constructor has the idempotent retry seam tested below.
    assert!(matches!(
        s.issue_receipt(request.clone()),
        Err(DevError::AlreadyIssued(_))
    ));
    let theirs = chain::other_run_key();
    let hijack = chain::receipt_request("e1", &links[0].link.hash, &chain::run_key_hex(&theirs));
    assert!(matches!(
        s.issue_receipt(hijack),
        Err(DevError::AlreadyIssued(_))
    ));
    // And the clock moving does not produce a second, later-dated receipt.
    s.set_clock("2026-09-05T00:00:00Z");
    assert!(matches!(
        s.issue_receipt(request),
        Err(DevError::AlreadyIssued(_))
    ));
}

#[test]
fn a_qualification_window_is_positive_and_the_server_owns_its_deadline() {
    let server = DevServer::with_qualification_window(2).expect("a positive window builds");
    let links = chain::chain("short-window");
    let mut request = chain::receipt_request(
        "short-window",
        &links[0].link.hash,
        &chain::run_key_hex(&chain::run_key()),
    );
    request.claims.started_at = "1999-01-01T00:00:00Z".to_string();
    request.claims.deadline_at = "2036-10-01T00:00:00Z".to_string();

    server.set_clock(chain::AT);
    let receipt = server
        .issue_receipt(request)
        .expect("the server signs the short qualification receipt");

    assert_eq!(receipt.claims.started_at, chain::AT);
    assert_eq!(
        receipt.claims.deadline_at, "2026-09-01T00:00:02Z",
        "the signed deadline is the selected window, not the legacy 168-hour default"
    );
    receipt
        .verify_with(&server.public_key_hex())
        .expect("the short receipt remains signed by the server");
}

#[test]
fn a_qualification_window_rejects_zero_and_checked_timestamp_overflow() {
    assert!(matches!(
        DevServer::with_qualification_window(0),
        Err(DevError::BadReceiptAuthority(
            "qualification window must be strictly positive"
        ))
    ));

    let server = DevServer::with_qualification_window(u64::MAX)
        .expect("a positive window is accepted before clock arithmetic");
    let links = chain::chain("overflow-window");
    server.set_clock(chain::AT);
    assert!(matches!(
        server.issue_receipt(chain::receipt_request(
            "overflow-window",
            &links[0].link.hash,
            &chain::run_key_hex(&chain::run_key()),
        )),
        Err(DevError::Clock(message)) if message.contains("timestamp addition overflow")
    ));
}

#[test]
fn a_positive_window_rejects_a_deadline_outside_the_protocol_timestamp_grammar() {
    let start = goalcup_protocol::control::parse_z(chain::AT).expect("the fixture clock parses");
    let window = 253_402_300_800u64
        .checked_sub(start)
        .expect("the fixture clock is before the first unsupported year");
    let deadline = start
        .checked_add(window)
        .expect("the boundary window does not overflow u64");
    let formatted = goalcup_protocol::control::format_z(deadline);
    assert_eq!(formatted, "10000-01-01T00:00:00Z");
    assert_ne!(
        goalcup_protocol::control::parse_z(&formatted),
        Some(deadline),
        "the boundary value is intentionally outside the fixed-width protocol grammar"
    );

    let server = DevServer::with_qualification_window(window)
        .expect("the positive window is arithmetically valid");
    server.set_clock(chain::AT);
    let links = chain::chain("unrepresentable-window");
    let err = server
        .issue_receipt(chain::receipt_request(
            "unrepresentable-window",
            &links[0].link.hash,
            &chain::run_key_hex(&chain::run_key()),
        ))
        .expect_err("the server must reject an unreadable signed deadline");
    assert!(matches!(
        err,
        DevError::Clock(message)
            if message.contains("deadline timestamp cannot round-trip")
                && message.contains("10000-01-01T00:00:00Z")
    ));
}

#[test]
fn a_qualification_receipt_retry_returns_the_original_server_decision() {
    let server = DevServer::with_qualification_window(2).expect("a positive window builds");
    let links = chain::chain("retry-window");
    let mut request = chain::receipt_request(
        "retry-window",
        &links[0].link.hash,
        &chain::run_key_hex(&chain::run_key()),
    );
    request.claims.started_at = "1999-01-01T00:00:00Z".to_string();
    request.claims.deadline_at = "2036-10-01T00:00:00Z".to_string();
    server.set_clock(chain::AT);
    let first = server
        .issue_receipt(request.clone())
        .expect("the first request opens the entry");

    // A caller retry may carry different participant timing fields, but those fields are
    // overwritten by the server and cannot change the already signed decision.
    request.claims.started_at = "2000-01-01T00:00:00Z".to_string();
    request.claims.deadline_at = "2037-10-01T00:00:00Z".to_string();
    server.set_clock("2026-09-01T00:00:01Z");
    let retry = server
        .issue_receipt(request)
        .expect("a retry returns the one receipt already issued");
    assert_eq!(retry, first);
}

/// The receipt pins the run key, so an entry has one participant.
///
/// A key learned from the first chunk that happened to verify would mean whoever uploaded
/// first owned the entry — an ordering race standing in for an authorisation rule.
#[test]
fn the_receipt_pins_the_run_key_and_a_chunk_under_another_one_is_refused() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    let mine = chain::run_key();
    let theirs = chain::other_run_key();
    s.set_clock(chain::AT);
    s.issue_receipt(chain::receipt_request(
        "e1",
        &links[0].link.hash,
        &chain::run_key_hex(&mine),
    ))
    .unwrap();

    let hijack = chain::chunk_over("e1", &links[0..=3], &theirs);
    assert!(
        matches!(s.accept_chunk(&hijack), Err(DevError::BadSignature(_))),
        "the run key is the receipt's, not the first uploader's"
    );
    assert_eq!(s.acked_seq("e1"), None, "a refused chunk moves no anchor");

    // The same range under the key the receipt named is acknowledged, so what was refused
    // was the KEY and not the range.
    let honest = chain::chunk_over("e1", &links[0..=3], &mine);
    let ack = s.accept_chunk(&honest).unwrap();
    assert_eq!(
        (ack.seq, ack.hash.as_str()),
        (3, links[3].link.hash.as_str())
    );
    assert!(
        ack.verify(&goalcup_devserver::dev_server_public_key_hex())
            .is_ok()
    );
    assert!(
        ack.acked_at_secs().is_some(),
        "acked_at is inside the signature (R-C)"
    );
    assert_eq!(s.acked_seq("e1"), Some(3));

    // A chunk whose id names a range it does not carry is refused before the signature is
    // even considered, and `accept_chunk` surfaces that as a refusal rather than storing
    // one range's envelopes under another range's idempotency key.
    let mut lying = chain::chunk_over("e1", &links[4..=7], &mine);
    lying.claims.chunk_id = honest.claims.chunk_id.clone();
    assert!(matches!(
        s.accept_chunk(&lying),
        Err(DevError::BadSignature(_))
    ));
}

/// No receipt, no acknowledgement: an acknowledgement over an entry the server never
/// opened is a signature over a run nobody declared.
#[test]
fn a_chunk_for_an_entry_with_no_receipt_is_refused() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    let chunk = chain::chunk_over("e1", &links[0..=3], &chain::run_key());
    assert!(matches!(
        s.accept_chunk(&chunk),
        Err(DevError::NoReceipt(_))
    ));
    assert_eq!(s.acked_seq("e1"), None);
}

/// A chunk that carries nothing is acknowledging nothing: an ack at the head the claims
/// name would take the head on the client's word.
#[test]
fn a_chunk_carrying_no_envelopes_is_refused() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    let run = chain::run_key();
    s.set_clock(chain::AT);
    s.issue_receipt(chain::receipt_request(
        "e1",
        &links[0].link.hash,
        &chain::run_key_hex(&run),
    ))
    .unwrap();

    let hollow = chain::chunk_without_envelopes("e1", 0, 3, &links[3].link.hash, &run);
    assert!(matches!(
        s.accept_chunk(&hollow),
        Err(DevError::EmptyChunk(_))
    ));
    assert_eq!(s.acked_seq("e1"), None);
}

/// The very first chunk's seq 0 is the baseline the receipt witnesses.
#[test]
fn a_first_chunk_whose_baseline_is_not_the_receipts_chain_head_is_refused() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    let other = chain::links("e1", 8, 1);
    let run = chain::run_key();
    s.set_clock(chain::AT);
    // The receipt witnesses one baseline; the upload carries another.
    s.issue_receipt(chain::receipt_request(
        "e1",
        &other[0].link.hash,
        &chain::run_key_hex(&run),
    ))
    .unwrap();

    let chunk = chain::chunk_over("e1", &links[0..=3], &run);
    let err = s.accept_chunk(&chunk).unwrap_err();
    assert!(matches!(err, DevError::ChainMismatch { .. }), "{err}");
    assert!(err.to_string().contains("chain_head"), "{err}");
    assert_eq!(s.acked_seq("e1"), None);
}

/// A chunk that does not continue from the last acknowledged hash is refused,
/// however well it is signed.
#[test]
fn a_chunk_whose_first_prev_is_not_the_last_acked_hash_is_refused() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    let forked = chain::links("e1", 8, 1);
    let run = chain::run_key();
    s.set_clock(chain::AT);
    s.issue_receipt(chain::receipt_request(
        "e1",
        &links[0].link.hash,
        &chain::run_key_hex(&run),
    ))
    .unwrap();
    assert_eq!(
        s.accept_chunk(&chain::chunk_over("e1", &links[0..=3], &run))
            .unwrap()
            .seq,
        3
    );

    // Seqs 4..=7 of a DIFFERENT chain: contiguous, internally consistent, correctly signed,
    // and continuing from a link this server never acknowledged.
    let fork = chain::chunk_over("e1", &forked[4..=7], &run);
    let err = s.accept_chunk(&fork).unwrap_err();
    assert!(matches!(err, DevError::ChainMismatch { .. }), "{err}");
    assert_eq!(
        s.acked_seq("e1"),
        Some(3),
        "a refused chunk moves no anchor"
    );
    assert_eq!(s.acked_head("e1").unwrap().1, links[3].link.hash);
}

/// From the inside: an envelope whose recorded hash is not the one its own
/// content produces breaks the chunk, because the hash is recomputed rather than read.
#[test]
fn an_envelope_whose_hash_is_not_its_own_is_refused() {
    let s = DevServer::new();
    let mut links = chain::chain("e1");
    let run = chain::run_key();
    s.set_clock(chain::AT);
    s.issue_receipt(chain::receipt_request(
        "e1",
        &links[0].link.hash,
        &chain::run_key_hex(&run),
    ))
    .unwrap();

    // The body is edited and the recorded hash left alone: exactly the edit a chain exists
    // to make visible.
    links[2].recorded_at = "2030-01-01T00:00:00Z".to_string();
    let chunk = chain::chunk_over("e1", &links[0..=3], &run);
    let err = s.accept_chunk(&chunk).unwrap_err();
    assert!(matches!(err, DevError::ChainMismatch { .. }), "{err}");
    assert_eq!(s.acked_seq("e1"), None);
}

/// The range the claims name has to be the range the envelopes are.
#[test]
fn a_chunk_whose_envelopes_do_not_fill_its_range_is_refused() {
    use goalcup_protocol::chunk::{Chunk, ChunkClaims};
    let s = DevServer::new();
    let links = chain::chain("e1");
    let run = chain::run_key();
    s.set_clock(chain::AT);
    s.issue_receipt(chain::receipt_request(
        "e1",
        &links[0].link.hash,
        &chain::run_key_hex(&run),
    ))
    .unwrap();

    // Claims 0..=3, carries 0..=2 with the head of link 2. Signed honestly over its own
    // claims — the lie is between the claims and the cargo.
    let short = Chunk::sign(
        ChunkClaims::new("e1", 0, 3, &links[2].link.hash),
        links[0..=2].to_vec(),
        &run,
    )
    .unwrap();
    let err = s.accept_chunk(&short).unwrap_err();
    assert!(matches!(err, DevError::ChainMismatch { .. }), "{err}");

    // And a gap inside the range: seq 0, 1, 3 with 2 dropped.
    let gapped_envelopes = vec![links[0].clone(), links[1].clone(), links[3].clone()];
    let gapped = Chunk::sign(
        ChunkClaims::new("e1", 0, 3, &links[3].link.hash),
        gapped_envelopes,
        &run,
    )
    .unwrap();
    let err = s.accept_chunk(&gapped).unwrap_err();
    assert!(matches!(err, DevError::ChainMismatch { .. }), "{err}");
    assert_eq!(s.acked_seq("e1"), None);
}

/// `claims.head` is checked against the last envelope, not believed.
#[test]
fn a_chunk_whose_head_is_not_its_last_link_is_refused() {
    use goalcup_protocol::chunk::{Chunk, ChunkClaims};
    let s = DevServer::new();
    let links = chain::chain("e1");
    let run = chain::run_key();
    s.set_clock(chain::AT);
    s.issue_receipt(chain::receipt_request(
        "e1",
        &links[0].link.hash,
        &chain::run_key_hex(&run),
    ))
    .unwrap();

    let lying = Chunk::sign(
        ChunkClaims::new("e1", 0, 3, &"cd".repeat(32)),
        links[0..=3].to_vec(),
        &run,
    )
    .unwrap();
    let err = s.accept_chunk(&lying).unwrap_err();
    assert!(matches!(err, DevError::ChainMismatch { .. }), "{err}");
    assert!(err.to_string().contains("head"), "{err}");
}

/// The happy path, twice: two chunks in sequence, each acknowledged at the head the SERVER
/// computed from the links it was handed.
#[test]
fn a_two_chunk_sequence_is_acked_at_the_head_the_server_computed() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    let run = chain::run_key();
    s.set_clock(chain::AT);
    s.issue_receipt(chain::receipt_request(
        "e1",
        &links[0].link.hash,
        &chain::run_key_hex(&run),
    ))
    .unwrap();

    let first = s
        .accept_chunk(&chain::chunk_over("e1", &links[0..=3], &run))
        .unwrap();
    assert_eq!(
        (first.seq, first.hash.as_str()),
        (3, links[3].link.hash.as_str())
    );

    s.set_clock("2026-09-02T00:00:00Z");
    let second = s
        .accept_chunk(&chain::chunk_over("e1", &links[4..=7], &run))
        .unwrap();
    assert_eq!(
        (second.seq, second.hash.as_str()),
        (7, links[7].link.hash.as_str()),
        "the ack names the last link the server hashed, not the head the client claimed"
    );
    assert_eq!(second.acked_at, "2026-09-02T00:00:00Z");
    assert!(
        second
            .verify(&goalcup_devserver::dev_server_public_key_hex())
            .is_ok()
    );
    assert_eq!(s.acked_seq("e1"), Some(7));

    // Idempotency: the last chunk re-uploaded is the same document
    // and gets the same acknowledgement back — the same `acked_at`, therefore the same
    // signature — rather than a second one at a later time.
    s.set_clock("2026-09-03T00:00:00Z");
    let retry = s
        .accept_chunk(&chain::chunk_over("e1", &links[4..=7], &run))
        .unwrap();
    assert_eq!(retry, second);
}

/// The anchor does not advance on `seq` alone. A second chunk at an equal `last_seq`
/// carrying a different hash would otherwise be acknowledged without moving the anchor, and
/// the server would sign two contradictory statements about one sequence. Prev continuity
/// refuses the second outright: the range is already acknowledged, so the only chunk this
/// entry will take next starts at seq 4.
#[test]
fn a_second_chunk_at_the_same_last_seq_is_refused_rather_than_ignored() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    let forked = chain::links("e1", 8, 1);
    let run = chain::run_key();
    s.set_clock(chain::AT);
    s.issue_receipt(chain::receipt_request(
        "e1",
        &links[0].link.hash,
        &chain::run_key_hex(&run),
    ))
    .unwrap();
    let first = s
        .accept_chunk(&chain::chunk_over("e1", &links[0..=3], &run))
        .unwrap();

    // The same range, a different chain. `last_seq` is 3 either way.
    let rival = chain::chunk_over("e1", &forked[0..=3], &run);
    assert_eq!(rival.claims.last_seq, first.seq);
    assert_ne!(rival.claims.head, first.hash);
    let err = s.accept_chunk(&rival).unwrap_err();
    assert!(matches!(err, DevError::ChainMismatch { .. }), "{err}");
    assert_eq!(s.acked_head("e1"), Some((3, links[3].link.hash.clone())));
}

/// The two test hooks, which exist so a daemon test can play an outage and a clock.
///
/// `set_offline` is a refusal and not a slow answer: `DevError::Offline` is what every
/// entry point returns while it is set, so nothing is signed, nothing is stored, and the
/// anchor does not move. `set_clock` fixes `acked_at`, which is inside the signature — a
/// server whose clock a test could not move could not produce the anchor timeline V12
/// reads.
#[test]
fn offline_signs_nothing_and_the_clock_is_the_one_the_test_set() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    let run = chain::run_key();
    let request = chain::receipt_request("e1", &links[0].link.hash, &chain::run_key_hex(&run));

    s.set_offline(true);
    assert!(matches!(
        s.issue_receipt(request.clone()),
        Err(DevError::Offline)
    ));
    s.set_offline(false);
    s.set_clock(chain::AT);
    s.issue_receipt(request).unwrap();

    let chunk = chain::chunk_over("e1", &links[0..=3], &run);
    s.set_offline(true);
    assert!(matches!(s.accept_chunk(&chunk), Err(DevError::Offline)));
    assert_eq!(s.acked_seq("e1"), None);

    s.set_offline(false);
    s.set_clock("2026-09-03T12:00:00Z");
    let ack = s.accept_chunk(&chunk).unwrap();
    assert_eq!(ack.acked_at, "2026-09-03T12:00:00Z");
    assert!(
        ack.verify(&goalcup_devserver::dev_server_public_key_hex())
            .is_ok()
    );
    // Moving the clock afterwards does not move an acknowledgement already signed: the
    // stored answer to a repeated `chunk_id` is the one that was signed.
    s.set_clock("2026-09-04T12:00:00Z");
    assert_eq!(s.accept_chunk(&chunk).unwrap().acked_at, ack.acked_at);
}

/// A clock a test set to something `parse_z` refuses is the server's problem, not the
/// client's: an acknowledgement carries the string verbatim, but a deadline is arithmetic,
/// so a receipt cannot be dated from it at all.
#[test]
fn a_receipt_cannot_be_dated_from_a_clock_nothing_can_read() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    let run = chain::run_key();
    s.set_clock("the fifth of never");
    assert!(matches!(
        s.issue_receipt(chain::receipt_request(
            "e1",
            &links[0].link.hash,
            &chain::run_key_hex(&run)
        )),
        Err(DevError::Clock(_))
    ));
}

/// A run key that is not a usable Ed25519 public key is refused at the receipt, which is
/// the one moment this server can refuse it: pinning it would mean every later chunk failed
/// with a cryptography error over a key nobody could have signed with.
#[test]
fn a_receipt_naming_an_unusable_run_key_is_refused() {
    let s = DevServer::new();
    let links = chain::chain("e1");
    s.set_clock(chain::AT);
    for bad in ["not hex at all", &"ab".repeat(31)] {
        assert!(
            matches!(
                s.issue_receipt(chain::receipt_request("e1", &links[0].link.hash, bad)),
                Err(DevError::BadRunKey(_))
            ),
            "{bad}"
        );
    }
    assert_eq!(s.acked_seq("e1"), None);
}
