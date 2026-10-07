//! The signed upload chunk (the per-run signature on checkpoints).
//!
//! Checkpoints carry a per-run Ed25519 signature. `ChunkClaims` is the one
//! definition of what that signature covers, shared by the daemon and the server and tagged
//! like the receipt and the acknowledgement.
//!
//! The type exists so that the daemon signs, and
//! `goalcup-devserver` verifies, the same bytes — and so that the contract is written once,
//! here, rather than twice, differently.

use crate::ProtocolError;
use crate::envelope::Envelope;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One complete serialized signed upload request.
pub const MAX_UPLOAD_BYTES: usize = 1024 * 1024;

/// Domain tag over a chunk's claims, in the style of `RECEIPT_DOMAIN` and `ACK_DOMAIN`, so
/// a chunk's signed bytes can never be presented as a receipt's or an ack's.
pub const CHUNK_DOMAIN: &[u8] = b"goalcup-chunk-v1\0";

/// What a chunk's signature covers: which entry, which range, and the chain head that range
/// ends at. The envelopes themselves are NOT in the claims — `head` already commits to every
/// one of them through the link chain, and signing them twice would give a server two
/// definitions of the same fact and a way for them to disagree.
///
/// `chunk_id` is a *derived* field, not an independent one: it is `id()` over the other
/// four. It is carried in the struct because it is the server's idempotency key and has to
/// travel in the signed bytes, but it is never believed — `Chunk::sign` derives it and
/// `Chunk::verify_with` re-derives it. Build these with [`ChunkClaims::new`] rather than as
/// a literal; a literal whose `chunk_id` disagrees with its range will not verify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkClaims {
    pub entry_id: String,
    pub chunk_id: String,
    pub first_seq: u64,
    pub last_seq: u64,
    pub head: String,
}

impl ChunkClaims {
    /// Claims over one range, with the `chunk_id` derived rather than supplied. This is the
    /// only constructor; the id is a function of the other fields, so accepting one from a
    /// caller would only ever be a way for it to be wrong.
    pub fn new(entry_id: &str, first_seq: u64, last_seq: u64, head: &str) -> ChunkClaims {
        ChunkClaims {
            entry_id: entry_id.to_string(),
            chunk_id: ChunkClaims::id(entry_id, first_seq, last_seq, head),
            first_seq,
            last_seq,
            head: head.to_string(),
        }
    }

    /// `id()` over this claim's own range — what `chunk_id` must equal.
    pub fn derived_id(&self) -> String {
        ChunkClaims::id(&self.entry_id, self.first_seq, self.last_seq, &self.head)
    }

    /// `CHUNK_DOMAIN ++ serde_json::to_vec(self)` — struct field order, no whitespace, the
    /// same canonicalisation `StartReceiptClaims::signing_bytes` uses.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut bytes = CHUNK_DOMAIN.to_vec();
        bytes.extend_from_slice(&serde_json::to_vec(self)?);
        Ok(bytes)
    }

    /// `sha256(entry_id | first_seq | last_seq | head)`, hex. Deterministic, so the same
    /// range always produces the same `chunk_id` and a server that already stored it
    /// answers the same acknowledgement — idempotency without a table of UUIDs.
    ///
    /// The two strings are separated by a NUL and the two sequences are hashed as
    /// fixed-width big-endian, so the concatenation is unambiguous: no pair of different
    /// `(entry_id, first_seq, last_seq, head)` tuples can produce the same bytes by moving
    /// a character across a boundary.
    pub fn id(entry_id: &str, first_seq: u64, last_seq: u64, head: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(entry_id.as_bytes());
        hasher.update([0u8]);
        hasher.update(first_seq.to_be_bytes());
        hasher.update(last_seq.to_be_bytes());
        hasher.update(head.as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// One upload: the claims, the per-run key that signed them, and the envelopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub claims: ChunkClaims,
    /// The entry's per-run Ed25519 public key, hex. This field verifies the per-run signature
    /// on checkpoints.
    pub run_public_key: String,
    pub signature: String,
    pub envelopes: Vec<Envelope>,
}

impl Chunk {
    /// Sign one range. The `chunk_id` on the claims handed in is ignored and re-derived
    /// from the range before anything is signed: the id is a function of the other fields,
    /// so a signed chunk whose id and range disagree is a document this constructor must
    /// not be able to produce.
    pub fn sign(
        claims: ChunkClaims,
        envelopes: Vec<Envelope>,
        key: &SigningKey,
    ) -> Result<Chunk, ProtocolError> {
        let claims = ChunkClaims {
            chunk_id: claims.derived_id(),
            ..claims
        };
        let bytes = claims.signing_bytes()?;
        let sig: Signature = key.sign(&bytes);
        Ok(Chunk {
            claims,
            run_public_key: hex::encode(key.verifying_key().to_bytes()),
            signature: hex::encode(sig.to_bytes()),
            envelopes,
        })
    }

    /// Verify against the run key the caller pinned — the one the entry's start receipt
    /// named. `self.run_public_key` is compared to it first, so a chunk signed by a key the
    /// server never issued is refused before any cryptography runs. There is deliberately
    /// no `verify(&self)`: a document that names the key that checks it checks nothing.
    ///
    /// Hex case is not meaningful, so the comparison is ASCII-case-insensitive, exactly as
    /// `StartReceipt::verify_with`'s is.
    ///
    /// `chunk_id` is then re-derived from the range and compared, *before* the signature is
    /// verified. The id is inside the signed bytes, so a signer can sign an id that names a
    /// range the document does not carry; the signature would be valid and the document
    /// still a lie, and a server that files by `chunk_id` would store one range's envelopes
    /// under another range's idempotency key. A mismatch fails closed as
    /// [`ProtocolError::ChunkIdMismatch`], which is deliberately not a `Signature` error:
    /// the cryptography may be perfectly good.
    ///
    /// **What this does NOT check.** `self.envelopes` are not examined at all here — not
    /// rehashed against `head`, not checked to lie within `[first_seq, last_seq]`, not
    /// checked to be present, contiguous or in order. This function answers one question:
    /// did the run key sign these claims, and do the claims agree with themselves. Binding
    /// the carried envelopes to the range and the head is the receiving server's obligation,
    /// which is the side that has the chain to check them against.
    pub fn verify_with(&self, run_public_key_hex: &str) -> Result<(), ProtocolError> {
        if !self.run_public_key.eq_ignore_ascii_case(run_public_key_hex) {
            return Err(ProtocolError::Signature(format!(
                "the chunk names run key {}, and the caller pinned {run_public_key_hex}",
                self.run_public_key
            )));
        }
        let derived = self.claims.derived_id();
        if self.claims.chunk_id != derived {
            return Err(ProtocolError::ChunkIdMismatch {
                claimed: self.claims.chunk_id.clone(),
                derived,
            });
        }
        let pk: [u8; 32] = hex::decode(run_public_key_hex)
            .map_err(|e| ProtocolError::Signature(e.to_string()))?
            .try_into()
            .map_err(|_| ProtocolError::Signature("run key is not 32 bytes".into()))?;
        let sg: [u8; 64] = hex::decode(&self.signature)
            .map_err(|e| ProtocolError::Signature(e.to_string()))?
            .try_into()
            .map_err(|_| ProtocolError::Signature("signature is not 64 bytes".into()))?;
        let vk =
            VerifyingKey::from_bytes(&pk).map_err(|e| ProtocolError::Signature(e.to_string()))?;
        // `verify_strict`, not `verify`: it rejects non-canonical signature encodings and
        // small-order public keys, both of which let one signature verify under more than
        // one key.
        vk.verify_strict(&self.claims.signing_bytes()?, &Signature::from_bytes(&sg))
            .map_err(|e| ProtocolError::Signature(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(first: u64, last: u64) -> ChunkClaims {
        ChunkClaims::new("e1", first, last, &"cd".repeat(32))
    }

    /// The id is a function of the range, so a retry of the same range is the same chunk.
    #[test]
    fn the_id_is_a_pure_function_of_the_range_it_names() {
        assert_eq!(
            ChunkClaims::id("e1", 0, 9, "aa"),
            ChunkClaims::id("e1", 0, 9, "aa")
        );
        for other in [
            ChunkClaims::id("e2", 0, 9, "aa"),
            ChunkClaims::id("e1", 1, 9, "aa"),
            ChunkClaims::id("e1", 0, 10, "aa"),
            ChunkClaims::id("e1", 0, 9, "ab"),
        ] {
            assert_ne!(ChunkClaims::id("e1", 0, 9, "aa"), other);
        }
        assert_eq!(ChunkClaims::id("e1", 0, 9, "aa").len(), 64);
    }

    /// The boundary between the two strings is unambiguous: moving a character across it
    /// is a different chunk, not the same one.
    #[test]
    fn the_entry_id_and_the_head_cannot_be_slid_into_each_other() {
        assert_ne!(
            ChunkClaims::id("ab", 0, 0, "cd"),
            ChunkClaims::id("a", 0, 0, "bcd")
        );
    }

    /// A signature over a lie is still a lie. `chunk_id` is inside the signed bytes, so a
    /// signer can sign an id that names a range the document does not carry — and a server
    /// that files by `chunk_id` would then store one range's envelopes under another
    /// range's idempotency key. The id is re-derived, and a mismatch fails closed.
    #[test]
    fn an_id_that_does_not_name_its_own_range_is_refused() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let pinned = hex::encode(key.verifying_key().to_bytes());

        let mut forged = claims(0, 9);
        forged.chunk_id = ChunkClaims::id("e1", 0, 10, &"cd".repeat(32));
        // Signed honestly over the forged claims: the signature verifies; the id does not.
        let bytes = forged.signing_bytes().unwrap();
        let chunk = Chunk {
            claims: forged,
            run_public_key: pinned.clone(),
            signature: hex::encode(key.sign(&bytes).to_bytes()),
            envelopes: Vec::new(),
        };
        let err = chunk.verify_with(&pinned).unwrap_err();
        assert!(
            matches!(err, ProtocolError::ChunkIdMismatch { .. }),
            "{err:?}"
        );
    }

    /// The id is checked before the signature, so a document that is wrong in both ways
    /// reports the mismatch rather than the cryptography: the id is the cheaper check and
    /// the one that says what is actually wrong with the document.
    #[test]
    fn the_id_is_checked_before_the_signature() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let pinned = hex::encode(key.verifying_key().to_bytes());
        let mut forged = claims(0, 9);
        forged.chunk_id = ChunkClaims::id("e1", 0, 10, &"cd".repeat(32));
        let chunk = Chunk {
            claims: forged,
            run_public_key: pinned.clone(),
            signature: "ff".repeat(64),
            envelopes: Vec::new(),
        };
        let err = chunk.verify_with(&pinned).unwrap_err();
        assert!(
            matches!(err, ProtocolError::ChunkIdMismatch { .. }),
            "{err:?}"
        );
    }

    /// `sign` derives the id rather than accepting one, so no caller can produce a signed
    /// chunk whose id and range disagree — and the same range signed twice is the same
    /// document, which is what makes a retry idempotent.
    #[test]
    fn sign_derives_the_id_from_the_range_it_signs() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let pinned = hex::encode(key.verifying_key().to_bytes());

        let mut lying = claims(0, 9);
        lying.chunk_id = "an id nobody derived".to_string();
        let signed = Chunk::sign(lying, Vec::new(), &key).unwrap();
        assert_eq!(
            signed.claims.chunk_id,
            ChunkClaims::id("e1", 0, 9, &"cd".repeat(32))
        );
        assert!(signed.verify_with(&pinned).is_ok());

        let again = Chunk::sign(claims(0, 9), Vec::new(), &key).unwrap();
        assert_eq!(signed.claims.chunk_id, again.claims.chunk_id);
        assert_eq!(signed.signature, again.signature);
    }

    /// The constructor is the one place a `ChunkClaims` gets its id, so a caller never has
    /// to know the hash's byte layout to build one that verifies.
    #[test]
    fn the_constructor_derives_the_id_it_names() {
        let c = ChunkClaims::new("e1", 0, 9, &"cd".repeat(32));
        assert_eq!(c.chunk_id, ChunkClaims::id("e1", 0, 9, &"cd".repeat(32)));
        assert_eq!(c.entry_id, "e1");
        assert_eq!((c.first_seq, c.last_seq), (0, 9));
        assert_eq!(c.head, "cd".repeat(32));
    }

    #[test]
    fn a_signature_covers_the_range_and_not_the_bodies() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let bare = Chunk::sign(claims(0, 9), Vec::new(), &key).unwrap();
        let wrong = Chunk::sign(claims(0, 10), Vec::new(), &key).unwrap();
        assert_ne!(bare.signature, wrong.signature);
        assert!(
            bare.verify_with(&hex::encode(key.verifying_key().to_bytes()))
                .is_ok()
        );

        // A key nobody pinned is refused before any cryptography runs, and the message
        // says which key the document named.
        let err = bare.verify_with(&"aa".repeat(32)).unwrap_err().to_string();
        assert!(err.contains(&bare.run_public_key), "{err}");
    }
}
