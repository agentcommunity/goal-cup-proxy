//! Exact, policy-free capture-record values shared by report and integrity test suites.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{CaptureRecord, StreamKey};
use serde_json::Value;

/// A stream key with no clock, source, metadata, or producer-default policy hidden in it.
pub fn key(producer: &str, stream: u64) -> StreamKey {
    StreamKey {
        producer: producer.to_string(),
        stream,
    }
}

/// Capture records written one per line as a sanitized fixture writes them (a `derive.py`):
/// a Segment's bytes as `text`, UTF-8, so the file can be reviewed by eye, in place of
/// `data_b64`.
pub fn from_text_lines(text: &str) -> Vec<CaptureRecord> {
    text.lines()
        .map(|line| {
            let mut value: Value = serde_json::from_str(line).unwrap();
            let object = value.as_object_mut().unwrap();
            if let Some(text) = object.remove("text") {
                let bytes = text.as_str().unwrap().as_bytes();
                object.insert("data_b64".into(), Value::String(STANDARD.encode(bytes)));
            }
            serde_json::from_value(value).unwrap()
        })
        .collect()
}
