//! The per-line hash chain.
//!
//! The formula reproduces `spikes/stub-tailer.py`'s `Chain.emit` exactly:
//! `hash = hex(sha256(prev_hex_ascii || payload_bytes))`, starting from 64 zero
//! characters, with a monotonic sequence starting at 1. Every M0 leg-1 capture was
//! written this way, so changing it invalidates them.
//!
//! **One decision that is easy to lose.** `serde_json` is used **without** the
//! `preserve_order` feature, so a `Value::Object` is a `BTreeMap` and `to_vec` emits keys
//! in sorted order. That is what makes the determinism constraint cheap — but it also means
//! the bytes hashed for a `ChainJsonl` export are a
//! *re-serialised canonical form*, not the harness's original byte order. Turning
//! `preserve_order` on would change every hash ever recorded. Treat the feature set
//! of `serde_json` as part of the chain format.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Appending beyond the representable sequence space would make a chain non-monotonic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChainAppendError {
    #[error("chain sequence exhausted")]
    SequenceExhausted,
}

/// The `prev` of the first link: 64 ASCII zeros.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Domain tag prefixed to every envelope's chain bytes. Version-tagged, so a v2 chain
/// can never be confused with a v1 chain and a receipt's bytes can never be replayed
/// as an envelope's.
///
/// It is a prefix on the *bytes*, not a JSON field, so it is outside anything a
/// participant can edit in the stored line and still have the line parse.
pub const CHAIN_DOMAIN: &[u8] = b"goalcup-chain-v1\0";

/// `hex(sha256(prev.as_bytes() ++ bytes))`.
pub fn link_hash(prev: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev.as_bytes());
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// One link: its position, the hash it extends, and its own hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub seq: u64,
    pub prev: String,
    pub hash: String,
}

/// An append-only hash chain over raw record bytes.
#[derive(Debug, Clone)]
pub struct Chain {
    seq: u64,
    prev: String,
}

impl Chain {
    pub fn new() -> Chain {
        Chain {
            seq: 0,
            prev: GENESIS.to_string(),
        }
    }

    /// Continue a chain that already reached `seq` with head `prev`.
    /// Used after a daemon restart and by the verifier when a capture resumes.
    pub fn resume(seq: u64, prev: impl Into<String>) -> Chain {
        Chain {
            seq,
            prev: prev.into(),
        }
    }

    pub fn try_append(&mut self, bytes: &[u8]) -> Result<Link, ChainAppendError> {
        let prev = self.prev.clone();
        let hash = link_hash(&prev, bytes);
        let seq = self
            .seq
            .checked_add(1)
            .ok_or(ChainAppendError::SequenceExhausted)?;
        self.seq = seq;
        self.prev = hash.clone();
        Ok(Link { seq, prev, hash })
    }

    pub fn head(&self) -> &str {
        &self.prev
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }
}

impl Default for Chain {
    fn default() -> Self {
        Chain::new()
    }
}
