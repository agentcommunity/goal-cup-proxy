//! Server acknowledgement of a chain prefix, the late evidence that the chain was not edited
//! afterwards.
//!
//! This is the load-bearing type of the whole tamper argument. An edit to the chain is made
//! visible because the chain is hash-chained and the server acknowledges it as it grows
//! (docs/ARCHITECTURE.md, "Upload and the server"); an acknowledgement whose `hash` is never
//! compared to the chain, and whose `server_signature` is never verified, makes that claim
//! false. Everything else in the evidence — the chain, the buffer, the encrypted spool — is
//! under the participant's own control on their own machine. This is not.

use crate::ProtocolError;
use crate::control::parse_z;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Domain tag prefixed to every acknowledgement's signing bytes, in the shape
/// `chain::CHAIN_DOMAIN` established. Version-tagged, so an acknowledgement's bytes can
/// never be presented as a receipt's or a status's, and a v2 acknowledgement can never be
/// read as a v1 one.
pub const ACK_DOMAIN: &[u8] = b"goalcup-ack-v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckCheckpoint {
    pub entry_id: String,
    /// Highest sequence the server has durably stored.
    pub seq: u64,
    /// The chain hash at that sequence.
    pub hash: String,
    pub acked_at: String,
    /// Hex-encoded server signature over `signing_bytes()`.
    pub server_signature: String,
}

impl AckCheckpoint {
    /// Canonical signing bytes:
    /// `ACK_DOMAIN ++ {"entry_id":…,"seq":…,"hash":…,"acked_at":…}` in that order.
    ///
    /// `acked_at` is **inside** the signature. The server's clock is not evidence about
    /// the participant's machine, but it *is* evidence about when the server saw the
    /// chain, and that is the only statement distinguishing continuous anchoring from one
    /// upload before the deadline: without it a participant may stay offline all week,
    /// fabricate the run, connect an hour before the deadline and be acknowledged exactly
    /// as if the evidence had been anchored as it happened. The cost is
    /// that a clock correction invalidates the acknowledgement it corrects — which is the
    /// right trade, because a re-issued acknowledgement is cheap and an unfalsifiable
    /// timestamp is not.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        #[derive(Serialize)]
        struct Signed<'a> {
            entry_id: &'a str,
            seq: u64,
            hash: &'a str,
            acked_at: &'a str,
        }
        let mut bytes = ACK_DOMAIN.to_vec();
        bytes.extend_from_slice(&serde_json::to_vec(&Signed {
            entry_id: &self.entry_id,
            seq: self.seq,
            hash: &self.hash,
            acked_at: &self.acked_at,
        })?);
        Ok(bytes)
    }

    /// Verify `server_signature` against a hex-encoded Ed25519 server public key **the
    /// caller pinned**. The acknowledgement never names the key that checks it.
    ///
    /// The key is pinned in the trace descriptor (`server_public_key`) and, in the
    /// shipped release, in the signed binary. A trace that carries acknowledgements but no
    /// key is reported by V12 as unverifiable rather than as verified.
    pub fn verify(&self, server_public_key_hex: &str) -> Result<(), ProtocolError> {
        let pk: [u8; 32] = hex::decode(server_public_key_hex)
            .map_err(|e| ProtocolError::Signature(e.to_string()))?
            .try_into()
            .map_err(|_| ProtocolError::Signature("server key is not 32 bytes".into()))?;
        let sg: [u8; 64] = hex::decode(&self.server_signature)
            .map_err(|e| ProtocolError::Signature(e.to_string()))?
            .try_into()
            .map_err(|_| ProtocolError::Signature("signature is not 64 bytes".into()))?;
        let vk =
            VerifyingKey::from_bytes(&pk).map_err(|e| ProtocolError::Signature(e.to_string()))?;
        // `verify_strict`, not `verify`: it rejects non-canonical signature encodings and
        // small-order public keys, both of which let one signature verify under more than
        // one key.
        vk.verify_strict(&self.signing_bytes()?, &Signature::from_bytes(&sg))
            .map_err(|e| ProtocolError::Signature(e.to_string()))
    }

    /// `acked_at` as seconds since the epoch, or `None` when it does not parse — which
    /// the caller reports as "unknown", never as a guess. `acked_at` is signed,
    /// so this is a number a check may reason about.
    pub fn acked_at_secs(&self) -> Option<u64> {
        parse_z(&self.acked_at)
    }
}
