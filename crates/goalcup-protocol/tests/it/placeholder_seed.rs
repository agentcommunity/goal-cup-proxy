//! The published placeholder server key is the verifying key of the seed `[5u8; 32]`.
//!
//! The daemon says at startup that its configured server key is the published placeholder
//! (anybody can sign under it), and the verifier's fixtures sign with that seed. This keeps the
//! constant from drifting from the seed.

use ed25519_dalek::SigningKey;
use goalcup_protocol::receipt::PLACEHOLDER_SERVER_PUBLIC_KEY_HEX;

#[test]
fn the_published_seed_derives_the_placeholder_key() {
    let key = SigningKey::from_bytes(&[5u8; 32]);
    assert_eq!(
        hex::encode(key.verifying_key().to_bytes()),
        PLACEHOLDER_SERVER_PUBLIC_KEY_HEX
    );
}
