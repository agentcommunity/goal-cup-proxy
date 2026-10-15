//! Per-connector sample replay: a new harness adds a sample and what the decoders make of it,
//! as data, with no Rust.
//!
//! Each `tests/fixtures/connectors/<connector-id>/<name>.json` is one sample. Its directory is a
//! shipped connector id. It names its bytes in one of two forms, by a path relative to itself:
//!
//! - `"records": "<file>"`: capture records, one JSON object per line, with a Segment's bytes
//!   written as `text` in place of `data_b64` (what a `derive.py` writes, see
//!   `fixtures/m2-anthropic/`).
//! - `"native": {"file": "<file>", "hint": "<protocol_hint>"}`: the harness's own session file,
//!   replayed as one native stream that the connector's `[[capture.native]]` would open with
//!   that hint.
//!
//! Every hint the sample's streams carry must be one its connector's `harness.toml` declares, so a
//! sample cannot drift from the manifest it stands for.
//!
//! `expect` is what the built-in decoders made of the bytes: for each decoder that emitted or
//! lost anything, its summary counts and its observations by kind; and the streams no decoder
//! read (`undecoded`). `REGENERATE_CONNECTOR_SAMPLES=1` rewrites every `expect` from the
//! current decoders; read the diff before committing it.
//!
//! Committed sample bytes go through `spikes/redact.py` and are reviewed by eye (AGENTS.md).

use crate::common::decode::run;
use crate::common::records::{from_text_lines, key};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, EndReason, MAX_SEGMENT_BYTES,
};
use goalcup_protocol::pins::shipped_connector_ids;
use goalcup_protocol::redaction::RedactionAudit;
use goalcup_verifier::capture_analysis::registry::DecoderRegistry;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const PRODUCER: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

fn samples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/connectors")
}

fn at(tick: u64) -> CaptureTime {
    CaptureTime {
        wall: format!("2026-10-16T00:00:00.{tick:09}Z"),
        mono_ns: tick,
        clock_id: PRODUCER.to_string(),
    }
}

/// A session file as the one native stream a connector would open for it: Open, its bytes in
/// segments of at most `MAX_SEGMENT_BYTES`, End(Complete).
fn native_records(bytes: &[u8], hint: &str) -> Vec<CaptureRecord> {
    let stream = key(PRODUCER, 1);
    let mut records = vec![CaptureRecord::Open {
        stream: stream.clone(),
        observed: at(1),
        source: CaptureSource::Native,
        metadata: CaptureMeta {
            wire: None,
            label: "sample".to_string(),
            protocol_hint: Some(hint.to_string()),
            method: None,
            host: None,
            path: None,
            status: None,
            content_type: None,
        },
    }];
    for (part, chunk) in bytes.chunks(MAX_SEGMENT_BYTES).enumerate() {
        let part = part as u64;
        records.push(CaptureRecord::Segment {
            stream: stream.clone(),
            part,
            source_offset: part * MAX_SEGMENT_BYTES as u64,
            source_len: chunk.len() as u64,
            first: at(10 + 2 * part),
            last: at(11 + 2 * part),
            data_b64: STANDARD.encode(chunk),
            audit: RedactionAudit { spans: vec![] },
        });
    }
    records.push(CaptureRecord::End {
        stream,
        observed: at(1_000_000),
        reason: EndReason::Complete,
    });
    records
}

fn sample_records(sample: &Path, spec: &Value) -> Vec<CaptureRecord> {
    let dir = sample.parent().unwrap();
    let read = |file: &Value| {
        let path = dir.join(file.as_str().expect("a path"));
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    match (spec.get("records"), spec.get("native")) {
        (Some(file), None) => from_text_lines(&String::from_utf8(read(file)).unwrap()),
        (None, Some(native)) => native_records(
            &read(&native["file"]),
            native["hint"]
                .as_str()
                .expect("a native sample names its hint"),
        ),
        _ => panic!(
            "{}: name exactly one of \"records\" and \"native\"",
            sample.display()
        ),
    }
}

/// What the built-in decoders made of `records`, in the form `expect` holds.
fn replay(records: &[CaptureRecord]) -> Value {
    let (outcome, observations) = run(&DecoderRegistry::builtin(), records);
    let mut decoders = Map::new();
    for decoder in &outcome.decoders {
        let summary = decoder
            .result
            .as_ref()
            .unwrap_or_else(|failed| panic!("{} failed: {failed:?}", decoder.id));
        if summary.observations == 0 && summary.unknown_records == 0 && !summary.incomplete {
            continue;
        }
        let mut kinds: BTreeMap<&str, u64> = BTreeMap::new();
        for (_, observation) in observations.iter().filter(|(id, _)| *id == decoder.id) {
            *kinds.entry(observation.kind.as_str()).or_default() += 1;
        }
        decoders.insert(
            decoder.id.to_string(),
            json!({
                "observations": summary.observations,
                "unknown_records": summary.unknown_records,
                "incomplete": summary.incomplete,
                "kinds": kinds,
            }),
        );
    }
    let undecoded: Vec<Value> = outcome
        .undecoded
        .iter()
        .map(|group| {
            json!({"source": group.source, "hint": group.protocol_hint,
                "streams": group.streams, "bytes": group.bytes})
        })
        .collect();
    json!({"decoders": decoders, "undecoded": undecoded})
}

/// Every `protocol_hint` the sample's streams open with is declared by its connector.
fn check_hints(sample: &Path, records: &[CaptureRecord]) {
    let connector = sample.parent().unwrap().file_name().unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(connector)
        .join("harness.toml");
    let manifest = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
    for record in records {
        if let CaptureRecord::Open { metadata, .. } = record
            && let Some(hint) = &metadata.protocol_hint
        {
            assert!(
                manifest.contains(&format!("protocol_hint = \"{hint}\"")),
                "{}: {hint} is not a hint {connector:?} declares",
                sample.display()
            );
        }
    }
}

/// Every sample under its connector, sorted.
fn samples() -> Vec<PathBuf> {
    let connectors = shipped_connector_ids();
    let mut found = Vec::new();
    let root = samples_root();
    for entry in std::fs::read_dir(&root).unwrap_or_else(|e| panic!("{}: {e}", root.display())) {
        let dir = entry.unwrap().path();
        let id = dir.file_name().unwrap().to_str().unwrap().to_string();
        assert!(
            dir.is_dir() && connectors.contains(&id),
            "{}: a sample folder is named for a shipped connector",
            dir.display()
        );
        for file in std::fs::read_dir(&dir).unwrap() {
            let path = file.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "json") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn every_connector_sample_decodes_as_expected() {
    let regenerate = std::env::var_os("REGENERATE_CONNECTOR_SAMPLES").is_some_and(|v| v == "1");
    let samples = samples();
    assert!(!samples.is_empty(), "no connector sample was found");
    for sample in samples {
        let text = std::fs::read_to_string(&sample).unwrap();
        let mut spec: Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", sample.display()));
        let records = sample_records(&sample, &spec);
        check_hints(&sample, &records);
        let got = replay(&records);
        if regenerate {
            spec["expect"] = got.clone();
            let mut bytes = serde_json::to_vec_pretty(&spec).unwrap();
            bytes.push(b'\n');
            std::fs::write(&sample, bytes).unwrap();
        }
        assert_eq!(
            spec["expect"],
            got,
            "{}: the decoders' reading changed",
            sample.display()
        );
    }
}
