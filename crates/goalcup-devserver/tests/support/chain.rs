//! A chain a chunk can actually carry, built the way the writer builds one.
//!
//! Every test needs the same three things: a chain whose seq 0 is the
//! baseline, a receipt request naming that baseline's hash as `chain_head`, and a chunk
//! signed over a slice of that chain. Building them by hand in each test would mean five
//! copies of a link formula, which is the one thing a test of a link formula must not
//! contain — so the hashes here come from `Envelope::expected_hash`, the protocol crate's
//! own function, and nothing in this file computes a sha256.
//!
//! Included by `#[path]` from `dev_cli.rs` and `dev_http.rs`; this crate's tests have no
//! `support/mod.rs`.

#![allow(dead_code)]

use ed25519_dalek::SigningKey;
use goalcup_devserver::ReceiptRequest;
use goalcup_protocol::chain::{GENESIS, Link};
use goalcup_protocol::chunk::{Chunk, ChunkClaims};
use goalcup_protocol::control::format_z;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::receipt::StartReceiptClaims;

/// The per-run key these fixtures upload under. A run key is the participant's and never
/// the server's, so it is minted in the tests rather than anywhere in `src/`.
pub const RUN_SEED: [u8; 32] = [9u8; 32];
/// A second participant, for the take-over tests.
pub const OTHER_RUN_SEED: [u8; 32] = [10u8; 32];
/// The clock every test that does not care about time sets, so `started_at` is a fixed
/// string rather than whenever the suite happened to run.
pub const AT: &str = "2026-09-01T00:00:00Z";
/// `AT` + 168 hours, which is what the server must compute for `deadline_at`.
pub const DEADLINE: &str = "2026-09-08T00:00:00Z";

pub fn run_key() -> SigningKey {
    SigningKey::from_bytes(&RUN_SEED)
}

pub fn other_run_key() -> SigningKey {
    SigningKey::from_bytes(&OTHER_RUN_SEED)
}

pub fn run_key_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

/// `n` linked envelopes for `entry_id`: seq 0 is the baseline — the link a receipt's
/// `chain_head` witnesses — and every later link is a heartbeat. `salt` changes the bodies
/// without changing the shape, so two chains over the same range can be built with
/// different hashes.
pub fn links(entry_id: &str, n: u64, salt: u64) -> Vec<Envelope> {
    let mut prev = GENESIS.to_string();
    let mut out: Vec<Envelope> = Vec::new();
    for seq in 0..n {
        let at = format_z(1_756_684_800 + salt * 1_000 + seq);
        let body = if seq == 0 {
            EventBody::Baseline {
                manifest_sha256: format!("{:02x}", salt.min(255)).repeat(32),
                file_count: 1,
            }
        } else {
            EventBody::Heartbeat {
                ts: at.clone(),
                stores_seen: Vec::new(),
                receiver_ok: true,
            }
        };
        let mut env = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: entry_id.to_string(),
            source: "daemon".to_string(),
            recorded_at: at,
            link: Link {
                seq,
                prev: prev.clone(),
                hash: String::new(),
            },
            body,
            redaction: None,
        };
        // The one place a hash is produced here, and it is the protocol crate's.
        env.link.hash = env
            .expected_hash(&prev)
            .expect("an envelope of strings and integers serialises");
        prev = env.link.hash.clone();
        out.push(env);
    }
    out
}

/// The default chain: eight links, baseline first.
pub fn chain(entry_id: &str) -> Vec<Envelope> {
    links(entry_id, 8, 0)
}

/// A receipt request over `chain_head`, whose `started_at` and `deadline_at` are
/// deliberately wrong: the server overwrites them from its own clock, and a
/// request that asked for the right ones could not show that it does.
pub fn receipt_request(entry_id: &str, chain_head: &str, run_public_key: &str) -> ReceiptRequest {
    ReceiptRequest {
        claims: StartReceiptClaims {
            entry_id: entry_id.to_string(),
            connector_id: "claude-code-cli".to_string(),
            workspace: format!("/ws/{entry_id}"),
            session_id: "s1".to_string(),
            started_at: "1999-01-01T00:00:00Z".to_string(),
            deadline_at: "1999-01-02T00:00:00Z".to_string(),
            manifest_sha256: "ab".repeat(32),
            chain_head: chain_head.to_string(),
            // Signed exactly as sent: this server verifies the run key, not the connector,
            // so a digest no build pins is the right value here — it shows the claim
            // travelled rather than that it was checked.
            connector_dir_sha256: Some("ef".repeat(32)),
            run_public_key: None,
            questionnaire_version: None,
        },
        run_public_key: run_public_key.to_string(),
    }
}

/// A signed chunk over `envelopes`, with the range and the head read off them rather than
/// supplied — a chunk whose claims disagreed with what it carries is a separate test.
pub fn chunk_over(entry_id: &str, envelopes: &[Envelope], key: &SigningKey) -> Chunk {
    let first = envelopes
        .first()
        .expect("a chunk carries envelopes")
        .link
        .seq;
    let last = envelopes.last().expect("a chunk carries envelopes");
    Chunk::sign(
        ChunkClaims::new(entry_id, first, last.link.seq, &last.link.hash),
        envelopes.to_vec(),
        key,
    )
    .expect("sign the upload")
}

/// A signed chunk that claims a range and carries no envelopes at all — which the server
/// refuses.
pub fn chunk_without_envelopes(
    entry_id: &str,
    first_seq: u64,
    last_seq: u64,
    head: &str,
    key: &SigningKey,
) -> Chunk {
    Chunk::sign(
        ChunkClaims::new(entry_id, first_seq, last_seq, head),
        Vec::new(),
        key,
    )
    .expect("sign the upload")
}
