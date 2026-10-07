//! Exact, policy-free capture-record values shared by report and integrity test suites.

use goalcup_protocol::capture::StreamKey;

/// A stream key with no clock, source, metadata, or producer-default policy hidden in it.
pub fn key(producer: &str, stream: u64) -> StreamKey {
    StreamKey {
        producer: producer.to_string(),
        stream,
    }
}
