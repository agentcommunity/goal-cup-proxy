//! The decoder registry: which [`DecoderV1`] factories a decode runs, in order.
//!
//! A registry is immutable once built and shared by reference across sessions and threads.
//! Each session makes a fresh decoder from every factory, so no decoder state crosses
//! traces. [`DecoderRegistry::builtin`] is the one place the in-tree decoders are listed: a
//! new one is one more [`Registered`], naming the real traffic it was measured on and the
//! forms only synthetic tests have exercised.
//!
//! The registry supplies decoder candidates; it calls each [`DecoderV1::accepts`] with the full
//! `StreamOpen`, and each decoder decides which fields matter. The hint names the retained-byte
//! format, while `goalcup_protocol::hints` validates only its open vocabulary. A stream is
//! capture-only when the session records no accepting decoder; an isolated factory or `accepts`
//! failure can also leave no selector. It is recorded, never read, and named in
//! [`DecodeOutcome::undecoded`](super::session::DecodeOutcome::undecoded).
//! An unsupported content type may instead be accepted and emitted as `unknown`.
//! Research-report schema 1 does not yet serialize the capture-only inventory.

use super::api::{DecoderV1, StreamOpen};
use super::session::quiet;
use super::{anthropic, chat, native, openai};
use std::fmt;
use std::sync::Arc;

/// Makes a fresh decoder for one trace. A closure, so a host can capture configuration (a
/// tenant's limits); `Send + Sync` so one registry serves every thread.
pub type DecoderFactory = Arc<dyn Fn() -> Box<dyn DecoderV1> + Send + Sync>;

/// One registered decoder: a factory and what the registry knows of it. Its id and version are
/// read once, from one instance, at registration; every instance a session makes must report
/// the same, or that decoder fails.
#[derive(Clone)]
pub struct Registered {
    id: &'static str,
    version: &'static str,
    make: DecoderFactory,
    measured_on: &'static str,
    synthetic_only: &'static [&'static str],
}

impl Registered {
    /// Registers `make`. The factory runs here, outside any isolation: registration is
    /// trusted code. `measured_on` names the real traffic the decoder was checked against and
    /// `synthetic_only` the forms only synthetic tests have exercised.
    pub fn new(
        make: impl Fn() -> Box<dyn DecoderV1> + Send + Sync + 'static,
        measured_on: &'static str,
        synthetic_only: &'static [&'static str],
    ) -> Self {
        let instance = make();
        Self {
            id: instance.id(),
            version: instance.version(),
            make: Arc::new(make),
            measured_on,
            synthetic_only,
        }
    }

    pub fn id(&self) -> &'static str {
        self.id
    }

    pub fn version(&self) -> &'static str {
        self.version
    }

    pub fn measured_on(&self) -> &'static str {
        self.measured_on
    }

    pub fn synthetic_only(&self) -> &'static [&'static str] {
        self.synthetic_only
    }

    pub(super) fn make(&self) -> Box<dyn DecoderV1> {
        (self.make)()
    }
}

/// Why a decoder was not registered.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistryError {
    /// The id is not `^[a-z][a-z0-9-]{0,63}$`.
    InvalidId(&'static str),
    /// A decoder with this id is already registered.
    DuplicateId(&'static str),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(id) => write!(f, "decoder id {id:?} is not [a-z][a-z0-9-]{{0,63}}"),
            Self::DuplicateId(id) => write!(f, "decoder id {id:?} is already registered"),
        }
    }
}

impl std::error::Error for RegistryError {}

fn valid_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    id.len() <= 64
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Immutable once built; `Send + Sync`, shared by reference across sessions and threads.
#[derive(Clone, Default)]
pub struct DecoderRegistry {
    decoders: Vec<Registered>,
}

impl DecoderRegistry {
    /// No decoder: every stream is capture-only.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The in-tree decoders, in the order a report lists them.
    pub fn builtin() -> Self {
        let mut registry = Self::empty();
        for registered in [
            Registered::new(
                || Box::new(native::CodexNativeDecoder::new()),
                "Codex CLI 0.153.0 rollout JSONL: a cli-launched session and real exec-launched \
                 chains",
                &[],
            ),
            Registered::new(
                || Box::new(native::ClaudeNativeDecoder::new()),
                "Claude Code project JSONL: 2.1.259 sessions and real 2.1.283 chains",
                &[],
            ),
            Registered::new(
                || Box::new(openai::OpenAiDecoder::new()),
                "the WebSocket traffic of one live Codex CLI 0.153.0 run",
                &[
                    "function_call and function_call_output items",
                    "the HTTP POST and server-sent-events fallback",
                    "response.failed, response.incomplete, error events and HTTP JSON error \
                     bodies",
                ],
            ),
            Registered::new(
                || Box::new(anthropic::AnthropicDecoder::new()),
                "one real Claude Code 2.1.283 exchange",
                &["the error event", "the JSON error document"],
            ),
            Registered::new(
                || Box::new(chat::ChatCompletionsDecoder::new()),
                "no real traffic yet: the live Hermes/DeepSeek row predates this decoder and has \
                 not been re-decoded",
                &[
                    "SSE chunks: content deltas, finish_reason, per-chunk model, [DONE]",
                    "tool_calls deltas merged by index",
                    "usage, including a usage-only chunk",
                    "the non-stream chat.completion body",
                    "error objects and JSON error bodies, including a 401 masked-key echo",
                    "a stream that ends without [DONE], and data after [DONE]",
                ],
            ),
        ] {
            registry
                .register(registered)
                .expect("the built-in decoders have distinct, valid ids");
        }
        registry
    }

    /// Adds a decoder after the ones already registered. A report lists decoders, and
    /// observations interleave, in registration order.
    pub fn register(&mut self, registered: Registered) -> Result<(), RegistryError> {
        if !valid_id(registered.id) {
            return Err(RegistryError::InvalidId(registered.id));
        }
        if self.decoders.iter().any(|r| r.id == registered.id) {
            return Err(RegistryError::DuplicateId(registered.id));
        }
        self.decoders.push(registered);
        Ok(())
    }

    /// `(id, version)` of every decoder, in registration order.
    pub fn ids(&self) -> Vec<(&'static str, &'static str)> {
        self.decoders.iter().map(|r| (r.id, r.version)).collect()
    }

    /// The decoders that accept a stream, in registration order: the same answer a session
    /// routes by. Empty means the stream is capture-only. A decoder whose `accepts` panics
    /// is not listed.
    pub fn accepting(&self, open: &StreamOpen<'_>) -> Vec<&'static str> {
        self.decoders
            .iter()
            .filter(|r| quiet(|| r.make().accepts(open)).unwrap_or(false))
            .map(|r| r.id)
            .collect()
    }

    pub(super) fn entries(&self) -> &[Registered] {
        &self.decoders
    }
}
