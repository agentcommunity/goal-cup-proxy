//! The Worker and the verifier must agree on signing bytes, byte for byte. The vectors live in
//! one file, `server/worker/test/vectors/signing.json`; the Worker's `sign.test.ts` reads the
//! same file. If the receipt or acknowledgement bytes change here (field order, domain tag),
//! this test fails, and so would every Worker receipt at verify time.

use ed25519_dalek::{Signer, SigningKey};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use serde_json::Value;

fn vectors() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../server/worker/test/vectors/signing.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("signing.json parses")
}

fn key(v: &Value) -> SigningKey {
    let seed: [u8; 32] = hex::decode(v["seed_hex"].as_str().unwrap())
        .unwrap()
        .try_into()
        .expect("32-byte seed");
    SigningKey::from_bytes(&seed)
}

#[test]
fn the_shared_vectors_are_what_rust_signs() {
    let v = vectors();
    let key = key(&v);
    let public = hex::encode(key.verifying_key().to_bytes());
    assert_eq!(public, v["public_key"].as_str().unwrap());

    let claims: StartReceiptClaims =
        serde_json::from_value(v["receipt"]["claims"].clone()).expect("claims parse");
    let receipt = StartReceipt::sign(claims, &key).unwrap();
    assert_eq!(receipt.signature, v["receipt"]["signature"].as_str().unwrap());
    receipt.verify_with(&public).expect("the receipt verifies");

    let mut ack: AckCheckpoint = serde_json::from_value(v["ack"].clone()).expect("ack parses");
    let signed = key.sign(&ack.signing_bytes().unwrap());
    assert_eq!(hex::encode(signed.to_bytes()), v["ack"]["server_signature"].as_str().unwrap());
    ack.verify(&public).expect("the acknowledgement verifies");
    ack.seq += 1;
    assert!(ack.verify(&public).is_err(), "a changed seq must not verify");
}
