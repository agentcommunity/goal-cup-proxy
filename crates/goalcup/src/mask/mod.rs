//! Masking (PRD-05, seam S2): every recorded byte and every meta string passes here before it
//! reaches disk. A masked byte becomes `*`, so lengths and offsets stay those of the source.
//!
//! - `rules`: the whole-record matcher over the shared literals of `shapes.tsv` (S12), the
//!   connector rules and the exact values, with the left guard, the quote cut and escape
//!   alignment.
//! - `stream`: the streaming hold, whose output equals the whole-record masker below the hold.
//! - `values`: exact values in their 11 forms (13 for a value with a `"` or `\`), and their
//!   readers: the project's key files and the harness environment goalcup builds, behind the
//!   section 9 name gate.
//! - `meta`: header, scheme, URL and query rules for metadata.
//!
//! The masker is pure: no I/O but the key-file reader, no threads, no clock, no panics by
//! design. No error text here ever formats a byte, a value or the length of a value.

mod meta;
mod rules;
mod stream;
mod values;

use crate::connector::{MaskPattern, MaskRule};
use goalcup_format::StreamKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub use values::{env_values, launch_values, workspace_values};

/// The name of the rule that stars bytes the masker could not decide.
pub const FAIL_CLOSED: &str = stream::FAIL_CLOSED_KIND;

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
    /// A connector rule, by its index in the rules given.
    #[error("a connector mask rule cannot be used (rule {0})")]
    Rule(usize),
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
/// hashed or logged. `Debug` prints the source and `left_out` only.
///
/// On the wire `bytes` is a JSON string. A used value is printable ASCII (`values::is_usable`),
/// so it round-trips exactly; a value that is not UTF-8 becomes U+FFFD in its lossy form, which
/// is not printable ASCII, so it stays unusable. One count-only entry
/// ([`SecretValue::left_out`]) carries the number of values a reader left out past the guard of
/// 32, so the start event's `skipped` stays right.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretValue {
    source: ValueSource,
    #[serde(with = "value_text")]
    bytes: Vec<u8>,
    #[serde(default, skip_serializing_if = "is_zero")]
    left_out: u32,
}

impl SecretValue {
    pub fn new(source: ValueSource, bytes: Vec<u8>) -> SecretValue {
        SecretValue {
            source,
            bytes,
            left_out: 0,
        }
    }

    /// The count-only entry: `n` values were left out by a reader and are counted in `skipped`.
    /// It holds no byte.
    pub fn left_out(n: u32) -> SecretValue {
        SecretValue {
            source: ValueSource::Workspace,
            bytes: Vec::new(),
            left_out: n,
        }
    }

    pub fn source(&self) -> ValueSource {
        self.source
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// `SecretValue.bytes` as one JSON string.
mod value_text {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&String::from_utf8_lossy(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        Ok(String::deserialize(d)?.into_bytes())
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretValue")
            .field("source", &self.source)
            .field("left_out", &self.left_out)
            .finish_non_exhaustive()
    }
}

/// The masker of one recorder boot. `Send + Sync`; a clone shares the policy.
#[derive(Debug, Clone)]
pub struct Masker {
    policy: Arc<stream::Policy>,
    /// The connector names that make a header a credential in meta.
    meta_names: Arc<meta::Names>,
}

impl Masker {
    /// Builds the masker: the shared shapes, then `rules` (PRD-11 data), then the exact values.
    /// A built-in shape that differs from its recognizer or does not compile is
    /// `MaskError::Shape`; a connector rule whose regex does not compile, or whose kind is
    /// `run_secret` or `fail_closed`, is `MaskError::Rule`. A value that cannot be used is
    /// skipped and counted in `skipped`, never an error. `workspace` and `env` values are used
    /// (the readers apply the section 9 name gate, so a model name or a base URL never arrives);
    /// a `harness` value (source 4, not built) is counted in `skipped`. The readers order, dedupe
    /// and cut the values before the first control line; a count-only entry adds its count to
    /// `skipped`. Here a value equal to one already used is counted nowhere, and past 32 used
    /// values the rest are `skipped`.
    pub fn new(rules: &[MaskRule], values: Vec<SecretValue>) -> Result<Masker, MaskError> {
        let policy = stream::Policy::new(rules, &values)?;
        let mut names = meta::Names::default();
        for rule in rules {
            match &rule.pattern {
                MaskPattern::JsonField { field, .. } => {
                    names.field_names.push(field.to_ascii_lowercase());
                }
                MaskPattern::HeaderPrefix(prefix) => {
                    names.header_prefixes.push(prefix.to_ascii_lowercase());
                }
            }
        }
        Ok(Masker {
            policy: Arc::new(policy),
            meta_names: Arc::new(names),
        })
    }

    /// Test-only: a masker whose streams and meta pass through unchanged, with no spans.
    #[cfg(feature = "test-hooks")]
    pub fn passthrough_for_tests() -> Masker {
        Masker {
            policy: Arc::new(stream::Policy::clear()),
            meta_names: Arc::default(),
        }
    }

    /// A masker for one stream. Every kind gets the same rules.
    pub fn stream(&self, kind: StreamKind) -> StreamMasker {
        let _ = kind;
        StreamMasker {
            scan: stream::Scanner::new(Arc::clone(&self.policy)),
        }
    }

    /// Masks `bytes` as one whole input: the same rules and guard as the streams, never a
    /// `fail_closed` span. The oracle a stream equals below its hold.
    pub fn mask_whole(&self, bytes: &[u8]) -> Masked {
        if self.policy.is_clear() {
            return Masked {
                bytes: bytes.to_vec(),
                spans: Vec::new(),
            };
        }
        stream::mask_whole(&self.policy, bytes)
    }

    /// Masks a metadata value in place, same length (PRD-05 section 7). Never fails.
    pub fn mask_meta(&self, meta: &mut Value) {
        if self.policy.is_clear() {
            return;
        }
        let scrub = |text: &str| -> String {
            String::from_utf8_lossy(&self.mask_whole(text.as_bytes()).bytes).into_owned()
        };
        meta::mask_meta(meta, &self.meta_names, &scrub);
    }

    /// One name per kind, indexed by MASK `rule`: the shared kinds, the connector kinds, then
    /// `run_secret`, and `fail_closed` last.
    pub fn mask_rules(&self) -> Vec<String> {
        self.policy.names().to_vec()
    }

    /// Counts only, for the start event.
    pub fn value_counts(&self) -> ValueCounts {
        self.policy.counts
    }
}

/// The masker of one stream. Idle, it holds no buffer.
#[derive(Debug)]
pub struct StreamMasker {
    scan: stream::Scanner,
}

impl StreamMasker {
    /// Masks the next piece. Returns the next released bytes, possibly fewer or more than were
    /// pushed (undecided bytes are held).
    #[must_use]
    pub fn push(&mut self, bytes: &[u8]) -> Masked {
        let mut out = Masked::default();
        if self.scan.policy().is_clear() {
            out.bytes = bytes.to_vec();
        } else {
            self.scan.feed(bytes, &mut out);
        }
        out
    }

    /// End of a WebSocket message: held bytes are decided as end of input, a fail-closed window
    /// ends, and the next message starts like a new stream.
    #[must_use]
    pub fn boundary(&mut self) -> Masked {
        let mut out = Masked::default();
        self.scan.finish_complete(&mut out);
        self.scan.restart();
        out
    }

    /// Called before any GAP on the stream: held bytes that look like secret material are
    /// starred to the cut, the rest decided as end of input; the bytes after the gap start
    /// like a new stream.
    #[must_use]
    pub fn gap(&mut self) -> Masked {
        let mut out = Masked::default();
        self.scan.finish_cut(&mut out);
        self.scan.restart();
        out
    }

    /// End of the stream: releases what is held, decided as end of input.
    #[must_use]
    pub fn finish(mut self) -> Masked {
        let mut out = Masked::default();
        self.scan.finish_complete(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Res = Result<(), Box<dyn std::error::Error>>;

    fn fake_key() -> Vec<u8> {
        let mut v = b"sk-".to_vec();
        v.extend(std::iter::repeat_n(b'x', 20));
        v.extend(std::iter::repeat_n(b'7', 20));
        v
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
        let m = Masker::new(
            &[],
            vec![
                v,
                SecretValue::new(ValueSource::Env, key.clone()),
                SecretValue::new(ValueSource::Workspace, b"short".to_vec()),
            ],
        )?;
        let counts = m.value_counts();
        assert_eq!(
            (counts.workspace, counts.env, counts.harness, counts.skipped),
            (1, 0, 0, 1)
        );
        assert!(!format!("{m:?}").contains(std::str::from_utf8(&key)?));
        Ok(())
    }

    #[cfg(feature = "test-hooks")]
    #[test]
    fn passthrough_keeps_bytes() {
        let m = Masker::passthrough_for_tests();
        let key = fake_key();
        let mut s = m.stream(StreamKind::Native);
        let out = s.push(&key);
        assert_eq!(out.bytes, key);
        assert!(out.spans.is_empty());
        let mut meta = serde_json::json!({"path": "/x"});
        m.mask_meta(&mut meta);
        assert_eq!(meta, serde_json::json!({"path": "/x"}));
    }

    const fn send_sync_clone<T: Send + Sync + Clone>() {}
    const fn send<T: Send>() {}
    const _: () = send_sync_clone::<Masker>();
    const _: () = send::<StreamMasker>();
    const _: () = assert!(size_of::<StreamMasker>() <= 256);
}
