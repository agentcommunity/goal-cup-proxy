//! A trace this server actually signed.
//!
//! The claim `dev_key.rs` makes is about a whole verdict — `KeySource`, one Flag strictly,
//! an empty queue under `--dev` — and a verdict is only reachable over a complete trace
//! directory: descriptor, manifest, chain, baseline, seal, receipt and acknowledgements,
//! every rule satisfied. Exactly one such generator exists in this tree, and it is
//! `goalcup-verifier`'s compliant fixture generator.
//!
//! **So it is included, not copied.** `#[path]` reaches into the verifier crate's own test
//! support, which is unusual and deliberate: the alternative is five hundred lines of
//! duplicated fixture that drifts the first time a rule moves, and a "compliant" trace that
//! is only compliant until it silently stops being. The verifier is already a
//! dev-dependency of this crate (D3), and this borrows the same crate's test module rather
//! than adding a dependency. If the compliant generator moves, this line moves with it and
//! says so loudly by failing to compile.
//!
//! What this module then does: it takes the trace that
//! generator wrote and **re-issues its receipt and its acknowledgements through a
//! `DevServer`**, so the signatures under test are the ones this crate produces rather than
//! ones a fixture wrote with the same seed. Under the default seed the result is byte
//! identical to what the fixture holds — asserted below, because that identity shows this
//! crate and the fixture agree about what is signed — and under `--key-seed` it is the
//! same trace anchored by a key nothing pins.
//!
//! **Every acknowledgement here is earned over real envelopes.** `accept_chunk` verifies
//! the chain a chunk carries, so the uploads below read `chain.jsonl` back and ship the
//! actual links the fixture wrote, contiguously, from the baseline at seq 0. And a
//! receipt's `started_at` is the server's clock, so the clock is set to the fixture's
//! declared start before the receipt is asked for — which is what keeps the byte-identity
//! assertion true.

#![allow(dead_code)]

/// The compliant trace generator, borrowed from the crate that owns the rules. See the
/// module header for why this is a `#[path]` include and not a copy.
#[allow(dead_code)]
#[path = "../../../goalcup-verifier/tests/support/compliant.rs"]
pub mod compliant;

/// The round trip: a whole run driven through the shipped daemon, against a live
/// `DevServer`, and the trace directory it leaves behind. Its own file because it is the
/// only fixture here that links `goalcup-daemon`, and `dev_key.rs`'s reader should be able
/// to see at a glance that the compliant trace it verifies is the hand-written one.
pub mod roundtrip;
// Re-exported so a test says `support::compliant_run`; `allow` because every test binary in
// this directory compiles this module and only `roundtrip_verify.rs` drives a daemon.
#[allow(unused_imports)]
pub use roundtrip::*;

/// The link, chunk and receipt-request builders. A separate file so `dev_http.rs` can
/// `#[path]`-include it without dragging the compliant fixture generator in behind it.
pub mod chain;

pub use chain::run_key;

use goalcup_devserver::{DevServer, ReceiptRequest};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::envelope::Envelope;
use goalcup_protocol::export::{ACKS_FILE, CHAIN_FILE, DESCRIPTOR_FILE, RECEIPT_FILE};
use goalcup_protocol::receipt::StartReceipt;

/// The compliant trace, with every server signature in it produced by a default
/// `DevServer` — whose key is `PLACEHOLDER_SERVER_PUBLIC_KEY`.
pub fn trace_signed_by_this_devserver() -> tempfile::TempDir {
    trace_signed_by(&DevServer::new())
}

/// The compliant trace, with every server signature in it produced by `server`.
///
/// The `TempDir` is returned rather than a path so the directory outlives the caller's
/// `verify_trace` and is removed when the test's binding drops.
pub fn trace_signed_by(server: &DevServer) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory for the trace");
    let out = dir.path();
    compliant::generate(out);

    // What the fixture signed, read back before anything is overwritten: the receipt's
    // claims and the (entry, seq, hash, acked_at) of every acknowledgement. Nothing here
    // invents a number — the ranges, the heads and the times are the chain's own.
    let fixture_receipt: StartReceipt = serde_json::from_slice(
        &std::fs::read(out.join(RECEIPT_FILE)).expect("the fixture receipt"),
    )
    .expect("the fixture receipt parses");
    let fixture_acks: Vec<AckCheckpoint> = std::fs::read_to_string(out.join(ACKS_FILE))
        .expect("the fixture acknowledgements")
        .lines()
        .map(|l| serde_json::from_str(l).expect("an acknowledgement parses"))
        .collect();
    let fixture_ack_bytes = std::fs::read(out.join(ACKS_FILE)).expect("the fixture acks");
    let fixture_receipt_bytes = std::fs::read(out.join(RECEIPT_FILE)).expect("the fixture receipt");
    // The links themselves, which are what an upload has to carry: `accept_chunk`
    // recomputes every hash, so a chunk's word about its own chain is not the
    // evidence for it.
    let envelopes: Vec<Envelope> = std::fs::read_to_string(out.join(CHAIN_FILE))
        .expect("the fixture chain")
        .lines()
        .map(|l| serde_json::from_str(l).expect("an envelope parses"))
        .collect();

    // ---- the receipt, issued by the server rather than signed by the fixture.
    //
    // The clock is set to the fixture's declared start FIRST: `started_at` is the server's
    // clock at issuance and `deadline_at` is that plus 168 hours, so a server
    // asked at the fixture's `STARTED_AT` produces the fixture's `STARTED_AT` and the
    // fixture's `SEALED_AT` — the two values the committed bytes carry. Everything else in
    // the claims is the fixture's, handed over unchanged.
    server.set_clock(compliant::STARTED_AT);
    let run = run_key();
    let receipt = server
        .issue_receipt(ReceiptRequest {
            claims: fixture_receipt.claims.clone(),
            run_public_key: chain::run_key_hex(&run),
        })
        .expect("the devserver issues the entry's one receipt");
    assert_eq!(
        (
            receipt.claims.started_at.as_str(),
            receipt.claims.deadline_at.as_str()
        ),
        (compliant::STARTED_AT, compliant::SEALED_AT),
        "the server dates the receipt from its own clock, a week apart"
    );
    std::fs::write(
        out.join(RECEIPT_FILE),
        serde_json::to_vec(&receipt).expect("serialise the receipt"),
    )
    .expect("write the receipt");

    // ---- the acknowledgements, earned by an upload rather than written by hand. Each one
    // is the answer to a signed `Chunk` carrying the actual links of the range that ends at
    // the link the fixture acknowledged, so what reaches `acks.jsonl` is what this server
    // would have said to a daemon that uploaded that range.
    let mut acks = Vec::new();
    let mut first_seq = 0u64;
    for fixture in &fixture_acks {
        // The server's clock is what `acked_at` records, and `acked_at` is inside the
        // signature — so it is set to the time the fixture's timeline says the server saw
        // this range, not to now.
        server.set_clock(&fixture.acked_at);
        let slice = &envelopes[first_seq as usize..=fixture.seq as usize];
        let chunk = chain::chunk_over(&fixture.entry_id, slice, &run);
        let ack = server
            .accept_chunk(&chunk)
            .expect("the devserver acknowledges");
        assert_eq!(
            (ack.seq, ack.hash.as_str(), ack.acked_at.as_str()),
            (
                fixture.seq,
                fixture.hash.as_str(),
                fixture.acked_at.as_str()
            ),
            "an acknowledgement names the range it was asked about"
        );
        acks.push(ack);
        first_seq = fixture.seq + 1;
    }
    let mut lines = Vec::new();
    for a in &acks {
        lines.extend_from_slice(&serde_json::to_vec(a).expect("serialise an acknowledgement"));
        lines.push(b'\n');
    }
    std::fs::write(out.join(ACKS_FILE), &lines).expect("write the acknowledgements");

    // ---- the descriptor names the key that checks all of it. One key in one file, so the
    // `--key-seed` variant is the same trace with a different anchor.
    let key = server.public_key_hex();
    let fixture_key = hex::encode(compliant::server_key().verifying_key().to_bytes());
    let descriptor = std::fs::read_to_string(out.join(DESCRIPTOR_FILE)).expect("the descriptor");
    assert!(
        descriptor.contains(&fixture_key),
        "the descriptor names the key the fixture signed with"
    );
    std::fs::write(
        out.join(DESCRIPTOR_FILE),
        descriptor.replace(&fixture_key, &key),
    )
    .expect("write the descriptor");

    // Over bytes. The default key IS the placeholder, so a default `DevServer` has to
    // reproduce the committed fixture's receipt and acknowledgements exactly — not merely
    // produce signatures that also verify. Anything else would mean this crate and the
    // fixture disagree about what is signed.
    if key == fixture_key {
        assert_eq!(
            std::fs::read(out.join(RECEIPT_FILE)).expect("the reissued receipt"),
            fixture_receipt_bytes,
            "the default devserver signs the fixture's receipt, byte for byte"
        );
        assert_eq!(
            lines, fixture_ack_bytes,
            "the default devserver signs the fixture's acknowledgements, byte for byte"
        );
    }
    dir
}
