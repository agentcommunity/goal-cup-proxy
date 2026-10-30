//! The hash chain, one link per flush (section 10).
//!
//! ```text
//! hash(n)  = SHA-256( "GCR1-LINK" || hash(n-1) || covered bytes )
//! hash(-1) = 32 zero bytes
//! covered  = every raw byte from the end of LINK n-1 (or the start of the entry) up to and
//!            including LINK n's own kind, len, seq, wall_ns and mono_ns; everything but its hash
//! ```

use crate::LINK_DOMAIN;
use sha2::{Digest, Sha256};

/// `hash(-1)`.
pub const ZERO: [u8; 32] = [0; 32];

/// A running link hash: feed every raw byte, then [`Chain::link`] with the LINK's head.
#[derive(Clone)]
pub struct Chain {
    h: Sha256,
}

impl Chain {
    /// A chain whose last link hash is `prev`.
    pub fn new(prev: [u8; 32]) -> Self {
        let mut h = Sha256::new();
        h.update(LINK_DOMAIN);
        h.update(prev);
        Self { h }
    }

    pub fn feed(&mut self, raw: &[u8]) {
        self.h.update(raw);
    }

    /// Ends a link: hashes `head` (the encoded LINK without its last 32 bytes) and starts the next.
    pub fn link(&mut self, head: &[u8]) -> [u8; 32] {
        self.h.update(head);
        let hash: [u8; 32] = std::mem::take(&mut self.h).finalize().into();
        *self = Self::new(hash);
        hash
    }
}
