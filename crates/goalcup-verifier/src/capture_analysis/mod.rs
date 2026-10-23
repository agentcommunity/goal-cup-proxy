//! Capture analysis for protocol-v5 traces: bounded integrity reading, versioned decoders
//! that turn retained capture records into source-referenced observations, and the private
//! research report that streams them.
//!
//! The stable decoder contract is [`api::DecoderV1`]; a [`registry::DecoderRegistry`] holds
//! the decoders, a [`session::DecodeSession`] runs them over records, and [`report`] writes
//! the result as NDJSON. [`stats`] counts the same observations into one numbers-only run
//! summary.

/// The Anthropic Messages decoder.
pub mod anthropic;
/// Shared facts and the Decoder v1 contract.
pub mod api;
/// The OpenAI Chat Completions decoder.
pub mod chat;
/// Bounded producer identity and recovery supersession shared by reader and session.
mod lineage;
/// Native-transcript decoders.
pub mod native;
/// The OpenAI Responses decoder.
pub mod openai;
/// The bounded facts a decode keeps about the records it read.
pub(crate) mod operations;
/// The bounded capture-integrity reader.
pub mod reader;
/// Which decoders a decode runs.
pub mod registry;
/// The private streaming research report.
pub mod report;
/// One decode of one trace, with per-decoder isolation.
pub mod session;
/// The numbers-only run summary (`--stats`).
pub mod stats;
/// The application thread WebSocket decoder.
pub mod thread;
/// The stream tracker shared by the OpenAI Responses and Chat Completions decoders.
mod tracker;
/// Shared plumbing of the wire decoders: segment assembler, SSE framer, field helpers.
mod wire;
