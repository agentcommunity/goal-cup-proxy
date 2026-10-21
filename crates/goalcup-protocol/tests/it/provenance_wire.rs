//! The provenance primitives a v5 chain still carries: the digest, the timestamp and the run
//! key spelling.

use ed25519_dalek::SigningKey;
use goalcup_protocol::provenance::*;

fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::try_from(byte.to_string().repeat(64)).unwrap()
}

#[test]
fn validated_scalars_refuse_noncanonical_values() {
    assert_eq!(digest('a').as_str(), "a".repeat(64));
    assert!(Sha256Digest::try_from("A".repeat(64)).is_err());
    assert!(Sha256Digest::try_from("a".repeat(63)).is_err());

    assert!(Rfc3339Utc::try_from("2024-02-29T23:59:59.123Z").is_ok());
    for invalid in [
        "2025-02-29T00:00:00Z",
        "2026-09-10T24:00:00Z",
        "2026-09-10T00:00:00+00:00",
        "2026-09-10T00:00:00.Z",
    ] {
        assert!(Rfc3339Utc::try_from(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn the_run_key_spelling_is_bounded_and_canonical() {
    let key = SigningKey::from_bytes(&[21; 32]);
    let lowercase = hex::encode(key.verifying_key().to_bytes());
    assert_eq!(
        canonical_run_public_key(&lowercase.to_ascii_uppercase()).unwrap(),
        lowercase
    );
    assert!(canonical_run_public_key(&"a".repeat(63)).is_err());
    assert!(canonical_run_public_key(&"zz".repeat(32)).is_err());
    assert!(
        canonical_run_public_key(
            "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f"
        )
        .is_err()
    );
    let weak_identity = format!("01{}", "00".repeat(31));
    assert!(canonical_run_public_key(&weak_identity).is_err());
}
