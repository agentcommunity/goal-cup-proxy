//! The server-signed start receipt (D16).

use crate::ProtocolError;
use crate::provenance::Sha256Digest;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Domain tag prefixed to every receipt's signing bytes, in the shape `chain::CHAIN_DOMAIN`
/// established. Version-tagged, so a receipt's bytes can never be presented as an
/// acknowledgement's or a status's, and a v2 receipt can never be read as a v1 one.
///
/// It is a prefix on the *bytes*, not a JSON field, so it is outside anything a
/// participant can edit in the stored document and still have it parse.
pub const RECEIPT_DOMAIN: &[u8] = b"goalcup-receipt-v1\0";
/// Domain for the digest that binds the complete signed receipt, independent of file formatting.
pub const START_RECEIPT_DIGEST_DOMAIN: &[u8] = b"goalcup:start-receipt-digest:v1\0";

/// The published placeholder server key, hex: the Ed25519 verifying key of the seed
/// `[5u8; 32]`, which is written in this repository (`goalcup_devserver::DEV_SERVER_SEED`,
/// the verifier's fixtures).
///
/// **Anybody can sign a receipt or an acknowledgement under it**, so a run anchored to it is
/// anchored to nothing until the project server's real key replaces it (V1.1). It lives in the
/// protocol crate, which every side already links, so the daemon can say so at startup without
/// linking the devserver (D3). `crates/goalcup-devserver/tests/full/placeholder_seed.rs` derives it
/// from the seed, so the constant cannot drift from what signs under it.
///
/// `goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY` aliases this constant; the one spelling
/// cannot drift from the seed the development fixtures use.
pub const PLACEHOLDER_SERVER_PUBLIC_KEY_HEX: &str =
    "6e7a1cdd29b0b78fd13af4c5598feff4ef2a97166e3ca6f2e4fbfccd80505bf1";

/// Why a receipt was refused. A distinct type rather than `ProtocolError`, because V13
/// renders each of these as a different sentence and "signature: invalid" is not a
/// finding a reviewer can act on.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReceiptError {
    #[error("the receipt names public key {found}, and this build pins {pinned}")]
    KeyMismatch { found: String, pinned: String },
    /// Three paths reach this: the hex does not decode, it decodes to something other than
    /// 32 bytes, or it decodes to 32 bytes that are not a point on the curve. The last is
    /// why the sentence cannot say "not 64 hex characters" — such a key is exactly 64 hex
    /// characters and still unusable, and a reviewer told to count them would find 64.
    #[error(
        "the pinned server key is not a usable Ed25519 public key: 64 hex characters decoding to a point on the curve"
    )]
    PinnedKeyMalformed,
    #[error("the receipt's signature is not 128 hex characters")]
    SignatureMalformed,
    #[error("the receipt's signature does not verify against the pinned server key")]
    SignatureInvalid,
    #[error("serialising the receipt's claims: {0}")]
    Serialise(String),
}

/// Everything the server signs. Field order is the canonical signing order:
/// `serde_json` emits struct fields in declaration order, so `signing_bytes`
/// is stable across runs and machines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartReceiptClaims {
    pub entry_id: String,
    pub connector_id: String,
    pub workspace: String,
    pub session_id: String,
    pub started_at: String,
    pub deadline_at: String,
    pub manifest_sha256: String,
    pub chain_head: String,
    /// The digest of the whole connector **directory** the daemon rendered this run's
    /// tracker files out of — `connector_dir::connector_dir_sha256`, which covers the hook
    /// bodies under `files/` that `manifest_sha256` cannot see.
    ///
    /// **The daemon computes it itself**, over its own `connectors_dir`, and takes no
    /// caller's word for it: a digest supplied by the process being checked checks nothing.
    /// The server signs whatever it is handed here — it verifies the run key, not the
    /// connector — and the verifier is what compares the signed value against
    /// `CONNECTOR_DIR_SHA256S`. That comparison is the whole reason the field exists: a
    /// trace carries the connector's `manifest.toml` and no other byte of the connector
    /// directory, so without this claim nothing post-hoc can say which hook templates
    /// captured the run.
    ///
    /// `Option`, and **absent from the signed bytes when `None`** — see
    /// [`Self::signing_bytes`]. Every receipt signed before this field existed therefore
    /// still verifies, byte for byte, and V13 reports the absence as
    /// `connector_dir_unbound` rather than reading an old receipt as a broken one. This
    /// field remains the ninth field even when later optional bindings are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_dir_sha256: Option<String>,
    /// The per-run Ed25519 public key, once a producer has independently verified it.
    /// Receipts bound under the `NewEvent` or `WorkspaceSubmission` policy carry it; the
    /// `Legacy` policy, which every v5 capture entry uses today, keeps it absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_public_key: Option<String>,
    /// The independently configured questionnaire contract version.
    /// Appended last so every historical signing layout remains byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub questionnaire_version: Option<String>,
}

impl StartReceiptClaims {
    /// `RECEIPT_DOMAIN ++ serde_json::to_vec(self)`, struct field order, no whitespace.
    ///
    /// # The byte layout, exactly
    ///
    /// ```text
    /// b"goalcup-receipt-v1\0"
    /// {"entry_id":…,"connector_id":…,"workspace":…,"session_id":…,"started_at":…,
    ///  "deadline_at":…,"manifest_sha256":…,"chain_head":…[,"connector_dir_sha256":…]
    ///  [,"run_public_key":…][,"questionnaire_version":…]}
    /// ```
    ///
    /// `serde_json` emits struct fields in declaration order and writes no whitespace, so
    /// the object above is the object this method produces, on every machine.
    ///
    /// **`connector_dir_sha256` is in those bytes only when it is `Some`.** It carries
    /// `skip_serializing_if = "Option::is_none"`, so a `None` leaves no key, no comma and
    /// no `null` behind: the bytes of a receipt without a connector-directory digest are
    /// exactly the bytes this method produced before the field existed. That is what keeps
    /// every receipt signed under the eight-field layout — the committed fixtures included
    /// — verifying unchanged. Later optional bindings follow it for the same compatibility
    /// reason.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut bytes = RECEIPT_DOMAIN.to_vec();
        bytes.extend_from_slice(&serde_json::to_vec(self)?);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartReceipt {
    pub claims: StartReceiptClaims,
    /// Hex-encoded 32-byte Ed25519 verifying key.
    pub public_key: String,
    /// Hex-encoded 64-byte Ed25519 signature over `claims.signing_bytes()`.
    pub signature: String,
}

impl StartReceipt {
    pub fn sign(
        claims: StartReceiptClaims,
        key: &SigningKey,
    ) -> Result<StartReceipt, ProtocolError> {
        let bytes = claims.signing_bytes()?;
        let sig: Signature = key.sign(&bytes);
        Ok(StartReceipt {
            claims,
            public_key: hex::encode(key.verifying_key().to_bytes()),
            signature: hex::encode(sig.to_bytes()),
        })
    }

    /// Verify against a key this build pins.
    ///
    /// The document's own `public_key` is compared to the pinned one **first**, and a
    /// difference is `KeyMismatch`, so a receipt the participant signed themselves is
    /// refused before any cryptography runs. There is deliberately no `verify(&self)`:
    /// a document that names the key that checks it checks nothing.
    ///
    /// Hex case is not meaningful, so the comparison is ASCII-case-insensitive — the
    /// descriptor validator accepts uppercase hex, so a pin that came from one must not
    /// read as a different key.
    pub fn verify_with(&self, server_public_key_hex: &str) -> Result<(), ReceiptError> {
        if !self.public_key.eq_ignore_ascii_case(server_public_key_hex) {
            return Err(ReceiptError::KeyMismatch {
                found: self.public_key.clone(),
                pinned: server_public_key_hex.to_string(),
            });
        }
        let pk_bytes: [u8; 32] = hex::decode(server_public_key_hex)
            .map_err(|_| ReceiptError::PinnedKeyMalformed)?
            .try_into()
            .map_err(|_| ReceiptError::PinnedKeyMalformed)?;
        let sig_bytes: [u8; 64] = hex::decode(&self.signature)
            .map_err(|_| ReceiptError::SignatureMalformed)?
            .try_into()
            .map_err(|_| ReceiptError::SignatureMalformed)?;
        let vk =
            VerifyingKey::from_bytes(&pk_bytes).map_err(|_| ReceiptError::PinnedKeyMalformed)?;
        let sig = Signature::from_bytes(&sig_bytes);
        let bytes = self
            .claims
            .signing_bytes()
            .map_err(|e| ReceiptError::Serialise(e.to_string()))?;
        // `verify_strict`, not `verify`: it rejects non-canonical signature encodings and
        // small-order public keys, both of which let one signature verify under more than
        // one key.
        vk.verify_strict(&bytes, &sig)
            .map_err(|_| ReceiptError::SignatureInvalid)
    }

    /// Hash the compact canonical complete receipt, including its key label and signature.
    pub fn canonical_digest(&self) -> Result<Sha256Digest, ProtocolError> {
        let mut hasher = Sha256::new();
        hasher.update(START_RECEIPT_DIGEST_DOMAIN);
        hasher.update(serde_json::to_vec(self)?);
        Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
    }
}
