//! Masking (PRD-05, seam S2): every recorded byte and every meta string passes here before it
//! reaches disk.
//!
//! This is the S2 stub. It stars every byte as `fail_closed`, same length, so no build writes a
//! raw byte before the core slice lands. The core slice keeps these shapes and adds the rules.
//! The masker is pure: no I/O, no threads, no clock, no panics. No error text here ever formats a
//! byte, a value or the length of a value.

use goalcup_format::StreamKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// The name of the rule that stars bytes the masker could not decide.
pub const FAIL_CLOSED: &str = "fail_closed";

/// The index of `fail_closed` in the stub's `mask_rules()`.
const FAIL_CLOSED_RULE: usize = 0;

/// One masked run: `start` is relative to [`Masked::bytes`], `rule` indexes `mask_rules()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
    pub rule: usize,
}

/// The next released bytes of a stream, masked, contiguous with the previous return. Its length
/// always equals the source bytes it stands for.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Masked {
    pub bytes: Vec<u8>,
    pub spans: Vec<Span>,
}

/// Why a masker could not be built. Fixed text only: never a byte, a value or a length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MaskError {
    /// A built-in shape, by its index in the shared list.
    #[error("a built-in mask shape does not compile (shape {0})")]
    Shape(usize),
    #[error("a mask value could not be used")]
    Value,
}

/// Counts of exact values by source: the start event's `mask_values` object (PRD-01 section 8).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueCounts {
    pub workspace: u32,
    pub env: u32,
    pub harness: u32,
    pub skipped: u32,
}

/// Where an exact value came from (PRD-05 section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueSource {
    Workspace,
    Env,
    Harness,
}

/// One exact secret value. It crosses the first control line (S17) and is never written,
/// hashed or logged. `Debug` prints the source only; drop overwrites the bytes with zeros.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretValue {
    source: ValueSource,
    bytes: Vec<u8>,
}

impl SecretValue {
    pub fn new(source: ValueSource, bytes: Vec<u8>) -> SecretValue {
        SecretValue { source, bytes }
    }

    pub fn source(&self) -> ValueSource {
        self.source
    }

    #[allow(dead_code, reason = "read by the exact-value rule of the core slice")]
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretValue")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.bytes.fill(0);
        std::hint::black_box(&self.bytes);
    }
}

/// What every stream of one masker shares.
#[derive(Debug)]
struct Policy {
    counts: ValueCounts,
    /// Test-only: pass bytes through unchanged.
    #[cfg(any(test, feature = "test-hooks"))]
    clear: bool,
}

/// The masker of one recorder boot. `Send + Sync`; a clone shares the policy.
#[derive(Debug, Clone)]
pub struct Masker {
    policy: Arc<Policy>,
}

impl Masker {
    /// Builds the masker. The stub ignores `rules` and stars every byte; the core slice takes
    /// `&[connector::MaskRule]` once S3 is on integration (call sites written against S2 compile
    /// either way). Values are counted by source and never matched by the stub.
    pub fn new<R>(rules: &[R], values: Vec<SecretValue>) -> Result<Masker, MaskError> {
        let _ = rules;
        let mut counts = ValueCounts::default();
        for value in &values {
            let slot = match value.source() {
                ValueSource::Workspace => &mut counts.workspace,
                ValueSource::Env => &mut counts.env,
                ValueSource::Harness => &mut counts.harness,
            };
            *slot = slot.saturating_add(1);
        }
        Ok(Masker {
            policy: Arc::new(Policy {
                counts,
                #[cfg(any(test, feature = "test-hooks"))]
                clear: false,
            }),
        })
    }

    /// Test-only: a masker whose streams and meta pass through unchanged, with no spans.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn passthrough_for_tests() -> Masker {
        Masker {
            policy: Arc::new(Policy {
                counts: ValueCounts::default(),
                clear: true,
            }),
        }
    }

    /// A masker for one stream. Every kind gets the same rules.
    pub fn stream(&self, kind: StreamKind) -> StreamMasker {
        let _ = kind;
        StreamMasker {
            policy: Arc::clone(&self.policy),
        }
    }

    /// Masks a metadata value in place, same length. Never fails. The stub stars every string
    /// leaf; object keys, numbers, bools and null stay.
    pub fn mask_meta(&self, meta: &mut Value) {
        if self.is_clear() {
            return;
        }
        let mut stack: Vec<&mut Value> = vec![meta];
        while let Some(value) = stack.pop() {
            match value {
                Value::String(s) => *s = "*".repeat(s.len()),
                Value::Array(items) => stack.extend(items.iter_mut()),
                Value::Object(map) => stack.extend(map.values_mut()),
                Value::Null | Value::Bool(_) | Value::Number(_) => {}
            }
        }
    }

    /// One name per rule, indexed by MASK `rule`; `fail_closed` is last.
    pub fn mask_rules(&self) -> Vec<String> {
        vec![FAIL_CLOSED.to_string()]
    }

    /// Counts only, for the start event.
    pub fn value_counts(&self) -> ValueCounts {
        self.policy.counts
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn is_clear(&self) -> bool {
        self.policy.clear
    }

    #[cfg(not(any(test, feature = "test-hooks")))]
    fn is_clear(&self) -> bool {
        false
    }
}

/// The masker of one stream. The stub holds nothing between calls.
#[derive(Debug)]
pub struct StreamMasker {
    policy: Arc<Policy>,
}

impl StreamMasker {
    /// Masks the next piece. The stub returns `bytes.len()` stars as one `fail_closed` span.
    #[must_use]
    pub fn push(&mut self, bytes: &[u8]) -> Masked {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.policy.clear {
            return Masked {
                bytes: bytes.to_vec(),
                spans: Vec::new(),
            };
        }
        if bytes.is_empty() {
            return Masked::default();
        }
        Masked {
            bytes: vec![b'*'; bytes.len()],
            spans: vec![Span {
                start: 0,
                len: bytes.len(),
                rule: FAIL_CLOSED_RULE,
            }],
        }
    }

    /// End of a WebSocket message: held bytes are decided as end of input.
    #[must_use]
    pub fn boundary(&mut self) -> Masked {
        Masked::default()
    }

    /// Called before any GAP on the stream.
    #[must_use]
    pub fn gap(&mut self) -> Masked {
        Masked::default()
    }

    /// End of the stream: releases what is held.
    #[must_use]
    pub fn finish(self) -> Masked {
        Masked::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    type Res = Result<(), Box<dyn std::error::Error>>;

    fn fake_key() -> Vec<u8> {
        let mut v = b"sk-".to_vec();
        v.extend(std::iter::repeat_n(b'x', 20));
        v.extend(std::iter::repeat_n(b'7', 20));
        v
    }

    fn stub() -> Result<Masker, MaskError> {
        Masker::new::<()>(&[], Vec::new())
    }

    fn fail_closed_index(m: &Masker) -> Option<usize> {
        m.mask_rules().iter().position(|r| r == FAIL_CLOSED)
    }

    /// Every output byte is `*` and the spans cover each byte once, all `fail_closed`.
    fn assert_starred(m: &Masker, input_len: usize, out: &Masked) {
        assert_eq!(out.bytes.len(), input_len);
        assert!(out.bytes.iter().all(|&b| b == b'*'));
        let mut covered = vec![0u8; out.bytes.len()];
        for span in &out.spans {
            assert_eq!(Some(span.rule), fail_closed_index(m));
            for c in covered.iter_mut().skip(span.start).take(span.len) {
                *c += 1;
            }
            assert!(span.start + span.len <= out.bytes.len());
        }
        assert!(covered.iter().all(|&c| c == 1));
    }

    #[test]
    fn push_stars_every_byte_same_length() -> Res {
        let m = stub()?;
        let mut ascii = b"{\"key\":\"".to_vec();
        ascii.extend(fake_key());
        ascii.extend(b"\"}\n");
        let inputs: [Vec<u8>; 3] = [Vec::new(), ascii, vec![0xff, 0xfe, b'a', 0xc3]];
        for input in &inputs {
            let mut s = m.stream(StreamKind::HttpReq);
            let out = s.push(input);
            assert_starred(&m, input.len(), &out);
        }
        Ok(())
    }

    #[test]
    fn boundary_gap_finish_return_nothing_raw() -> Res {
        let m = stub()?;
        let key = fake_key();
        let mut s = m.stream(StreamKind::WsUp);
        let mut out = Vec::new();
        let mut input_len = 0;
        for piece in key.chunks(7) {
            input_len += piece.len();
            out.push(s.push(piece));
            out.push(s.boundary());
            out.push(s.gap());
        }
        out.push(s.finish());
        let mut total = 0;
        for piece in &out {
            assert_starred(&m, piece.bytes.len(), piece);
            total += piece.bytes.len();
        }
        assert_eq!(total, input_len);
        Ok(())
    }

    fn strings(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(items) => items.iter().for_each(|i| strings(i, out)),
            Value::Object(map) => map.values().for_each(|i| strings(i, out)),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    #[test]
    fn mask_meta_keeps_lengths_and_no_original() -> Res {
        let m = stub()?;
        let bearer = format!("Bearer {}", String::from_utf8(fake_key())?);
        let mut meta = json!({
            "method": "POST",
            "path": "/v1/responses",
            "host": "api.example.test",
            "port": 443,
            "tls": true,
            "alpn": null,
            "headers": [
                ["authorization", bearer],
                ["content-type", "application/json"],
                ["x-empty", ""],
            ],
        });
        let before = meta.clone();
        m.mask_meta(&mut meta);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        strings(&before, &mut a);
        strings(&meta, &mut b);
        assert_eq!(a.len(), b.len());
        for (orig, masked) in a.iter().zip(&b) {
            assert_eq!(orig.len(), masked.len());
            assert!(masked.bytes().all(|c| c == b'*'));
        }
        let text = serde_json::to_string(&meta)?;
        assert!(a.iter().filter(|s| !s.is_empty()).all(|s| !text.contains(s.as_str())));
        let keys = |v: &Value| v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>());
        assert_eq!(keys(&before), keys(&meta));
        assert_eq!(meta.get("port"), before.get("port"));
        assert_eq!(meta.get("tls"), before.get("tls"));
        assert_eq!(meta.get("alpn"), before.get("alpn"));
        Ok(())
    }

    #[test]
    fn fail_closed_is_last() -> Res {
        let m = stub()?;
        assert_eq!(m.mask_rules().last().map(String::as_str), Some(FAIL_CLOSED));
        Ok(())
    }

    #[test]
    fn secret_value_never_prints_and_round_trips() -> Res {
        let key = fake_key();
        let v = SecretValue::new(ValueSource::Workspace, key.clone());
        let shown = format!("{v:?}");
        assert!(!shown.contains(std::str::from_utf8(&key)?));
        assert!(!shown.contains(&key.len().to_string()));
        let back: SecretValue = serde_json::from_str(&serde_json::to_string(&v)?)?;
        assert_eq!(back, v);
        assert_eq!(back.source(), ValueSource::Workspace);
        assert_eq!(back.bytes(), key.as_slice());
        let m = Masker::new::<()>(
            &[],
            vec![
                v,
                SecretValue::new(ValueSource::Env, key.clone()),
                SecretValue::new(ValueSource::Env, key),
            ],
        )?;
        let counts = m.value_counts();
        assert_eq!((counts.workspace, counts.env, counts.harness, counts.skipped), (1, 2, 0, 0));
        Ok(())
    }

    #[test]
    fn passthrough_keeps_bytes() {
        let m = Masker::passthrough_for_tests();
        let key = fake_key();
        let mut s = m.stream(StreamKind::Native);
        let out = s.push(&key);
        assert_eq!(out.bytes, key);
        assert!(out.spans.is_empty());
        let mut meta = json!({"path": "/x"});
        m.mask_meta(&mut meta);
        assert_eq!(meta, json!({"path": "/x"}));
    }

    const fn send_sync_clone<T: Send + Sync + Clone>() {}
    const fn send<T: Send>() {}
    const _: () = send_sync_clone::<Masker>();
    const _: () = send::<StreamMasker>();
    const _: () = assert!(size_of::<StreamMasker>() <= 256);
}
