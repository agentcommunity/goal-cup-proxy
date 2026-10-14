//! The per-line hash chain.
//!
//! The formula reproduces `spikes/stub-tailer.py`'s `Chain.emit` exactly:
//! `hash = hex(sha256(prev_hex_ascii || payload_bytes))`, starting from 64 zero
//! characters. Every M0 leg-1 capture was written this way, and every v5 chain links its
//! envelopes' `chain_bytes` with the same [`link_hash`], so changing it invalidates both.
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
