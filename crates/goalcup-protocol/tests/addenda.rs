//! The contract shapes the daemon consumes, defined once here in `goalcup-protocol`. Some
//! have no consumer in this crate at all, so the assertions below are the only thing
//! holding them to their meaning.

use goalcup_protocol::chunk::{CHUNK_DOMAIN, Chunk, ChunkClaims};
use goalcup_protocol::cursor::{CursorUnit, StoreCursor};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::manifest::ConnectorManifest;

fn claims() -> ChunkClaims {
    ChunkClaims::new("e1", 0, 9, &"cd".repeat(32))
}

/// The smallest manifest that loads and validates is not a hand-written literal in this
/// tree: `crates/goalcup-protocol/tests/manifest_load.rs` reads the shipped connector
/// files, and this file does the same: reading the shipped file is the tree's convention.
fn shipped(id: &str) -> String {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id)
        .join("manifest.toml");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn a_cursor_says_what_its_number_counts_and_only_records_are_comparable() {
    let bytes = StoreCursor {
        store: "main".to_string(),
        unit: CursorUnit::Bytes,
        position: Some(4096),
        describe: "inode:123,offset:4096".to_string(),
    };
    // The whole reason the type exists: 4,096 bytes is not four thousand records.
    assert_eq!(bytes.records(), None);

    let records = StoreCursor {
        store: "event".to_string(),
        unit: CursorUnit::Records,
        position: Some(512),
        describe: "ordinal:512".to_string(),
    };
    assert_eq!(records.records(), Some(512));

    let opaque = StoreCursor {
        store: "raw_bodies".to_string(),
        unit: CursorUnit::Opaque,
        position: None,
        describe: "12 files seen".to_string(),
    };
    assert_eq!(opaque.records(), None);

    // A `Records` cursor with no number is still no number: `records()` reports what the
    // daemon holds, and never invents a zero for a cursor that named none.
    assert_eq!(
        StoreCursor {
            store: "event".to_string(),
            unit: CursorUnit::Records,
            position: None,
            describe: "no rows yet".to_string(),
        }
        .records(),
        None
    );

    // Serialised inside a heartbeat, through the path `events` re-exports.
    let hb = EventBody::Heartbeat {
        ts: "2026-09-04T00:00:00Z".to_string(),
        stores_seen: vec![bytes.clone()],
        receiver_ok: true,
    };
    let text = serde_json::to_string(&hb).unwrap();
    assert!(text.contains("\"unit\":\"bytes\""), "{text}");
    let back: EventBody = serde_json::from_str(&text).unwrap();
    match back {
        EventBody::Heartbeat { stores_seen, .. } => assert_eq!(stores_seen, vec![bytes]),
        other => panic!("{other:?}"),
    }
    let _: goalcup_protocol::events::StoreCursor = records; // the events path resolves
}

#[test]
fn a_chunk_is_signed_over_a_domain_tagged_range_and_verifies_against_the_run_key_only() {
    use ed25519_dalek::{Signer, SigningKey};
    let run = SigningKey::from_bytes(&[9u8; 32]);
    let other = SigningKey::from_bytes(&[10u8; 32]);
    let pinned = hex::encode(run.verifying_key().to_bytes());

    let c = claims();
    assert!(c.signing_bytes().unwrap().starts_with(CHUNK_DOMAIN));
    assert_eq!(
        c.chunk_id,
        ChunkClaims::id("e1", 0, 9, &"cd".repeat(32)),
        "the same range is the same chunk, so a retry is idempotent"
    );
    assert_ne!(c.chunk_id, ChunkClaims::id("e1", 0, 10, &"cd".repeat(32)));

    let good = Chunk::sign(c.clone(), Vec::new(), &run).unwrap();
    assert_eq!(good.run_public_key, pinned);
    assert!(good.verify_with(&pinned).is_ok());
    // A chunk signed by a key the server never issued is refused, and the range it claims
    // cannot be edited after the fact.
    assert!(
        Chunk::sign(c.clone(), Vec::new(), &other)
            .unwrap()
            .verify_with(&pinned)
            .is_err()
    );
    let mut edited = good.clone();
    edited.claims.last_seq = 99;
    assert!(edited.verify_with(&pinned).is_err());

    // The id names the range or the chunk is refused. It rides inside the signed bytes, so
    // a signature alone cannot vouch for it: a signer could honestly sign an id naming some
    // other range, and the server that files by `chunk_id` would store this range's
    // envelopes under that one's idempotency key. The devserver may rely on the id.
    let mut misfiled = claims();
    misfiled.chunk_id = ChunkClaims::id("e1", 0, 10, &"cd".repeat(32));
    let bytes = misfiled.signing_bytes().unwrap();
    let forged = Chunk {
        claims: misfiled,
        run_public_key: pinned.clone(),
        signature: hex::encode(run.sign(&bytes).to_bytes()),
        envelopes: Vec::new(),
    };
    assert!(matches!(
        forged.verify_with(&pinned),
        Err(goalcup_protocol::ProtocolError::ChunkIdMismatch { .. })
    ));

    // The envelopes ride along and are covered by `head`, not by a second signature.
    let with_bodies = Chunk::sign(
        c,
        vec![Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: "e1".to_string(),
            source: "main".to_string(),
            recorded_at: "2026-09-04T00:00:00Z".to_string(),
            link: goalcup_protocol::chain::Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Leg2Post {
                event: "Stop".to_string(),
                payload: serde_json::json!({}),
                path: None,
                receiver_meta: None,
            },
            redaction: None,
        }],
        &run,
    )
    .unwrap();
    assert_eq!(
        with_bodies.signature, good.signature,
        "the claims are the signed bytes"
    );
    assert!(with_bodies.verify_with(&pinned).is_ok());
}

/// The domain tag earns its place: a chunk's signed bytes are not a receipt's, so a
/// signature over one can never be presented as a signature over the other.
#[test]
fn a_chunks_signing_bytes_are_tagged_apart_from_every_other_signed_document() {
    assert_eq!(CHUNK_DOMAIN, b"goalcup-chunk-v1\0");
    assert_ne!(CHUNK_DOMAIN, goalcup_protocol::receipt::RECEIPT_DOMAIN);
    assert_ne!(CHUNK_DOMAIN, goalcup_protocol::ack::ACK_DOMAIN);
    assert_ne!(CHUNK_DOMAIN, goalcup_protocol::chain::CHAIN_DOMAIN);

    let bytes = claims().signing_bytes().unwrap();
    let payload = &bytes[CHUNK_DOMAIN.len()..];
    assert_eq!(
        payload,
        serde_json::to_vec(&claims()).unwrap(),
        "the tag is a prefix on the bytes, not a field in the document"
    );
}

#[test]
fn a_recorded_leg2_partition_is_accepted_and_ignored() {
    // Appended as a TABLE, not as a bare key: a bare key after the last table would
    // belong to that table. A table header at the end of the file is unambiguous.
    //
    // The field belongs to the hook leg. It is accepted and ignored
    // so recorded V1 manifests still parse and stay bound to their receipt's
    // manifest_sha256, and it is never serialised.
    let m: ConnectorManifest = ConnectorManifest::from_toml_str(&format!(
        "{}\n[leg2_partition]\npath = \"payload.cwd\"\n",
        shipped("claude-code-cli")
    ))
    .unwrap();
    m.validate().expect("an ignored section decides nothing");
    assert!(
        !serde_json::to_string(&m)
            .unwrap()
            .contains("leg2_partition"),
        "an ignored leg2_partition must not be serialised"
    );
}
