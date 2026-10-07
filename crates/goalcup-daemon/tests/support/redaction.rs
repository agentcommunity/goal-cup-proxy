//! What the two streaming-redaction suites share: one producer driving one
//! `StreamRedactor`, the clocks its frames carry, a seeded generator, and the check that
//! finds the bytes an oracle redacts and a stream retained raw.
//!
//! Included by file path, so a redaction test does not link the daemon harnesses in
//! `support/mod.rs`.

#![allow(dead_code)]

use goalcup_daemon::redact::stream::StreamRedactor;
use goalcup_protocol::capture::*;
use goalcup_protocol::redaction::RedactionAuditSpan;

pub const P1: &str = "11111111111111111111111111111111";
pub const CONN: &str = "5e1f0a2b3c4d5e6f708192a3b4c5d6e7";
pub const DAEMON_CLOCK: &str = "daemon-5b0c3a1e";

pub fn producer_time(producer: &str, mono: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T10:00:00.000Z".into(),
        mono_ns: mono,
        clock_id: producer.into(),
    }
}

pub fn daemon_time(mono: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-27T10:00:05.000Z".into(),
        mono_ns: mono,
        clock_id: DAEMON_CLOCK.into(),
    }
}

pub fn http_meta(source: CaptureSource) -> CaptureMeta {
    CaptureMeta {
        wire: Some(WireIdentity {
            connection: CONN.into(),
            exchange: 1,
            message_index: None,
        }),
        label: "POST /v1/responses".into(),
        protocol_hint: Some("openai-responses".into()),
        method: Some("POST".into()),
        host: Some("api.example.test".into()),
        path: Some("/v1/responses".into()),
        status: None,
        content_type: Some("application/json".into()),
    }
    .with_source(source)
}

trait WithSource {
    fn with_source(self, source: CaptureSource) -> Self;
}

impl WithSource for CaptureMeta {
    fn with_source(mut self, source: CaptureSource) -> Self {
        if matches!(source, CaptureSource::Native | CaptureSource::Goal) {
            self.wire = None;
        }
        if matches!(source, CaptureSource::WsClient | CaptureSource::WsServer)
            && let Some(wire) = self.wire.as_mut()
        {
            wire.message_index = Some(0);
        }
        self
    }
}

/// One producer driving one redactor with correct sequence numbers, offsets and a
/// monotonic producer clock.
pub struct Producer {
    pub id: String,
    pub seq: u64,
    pub mono: u64,
    pub offsets: std::collections::BTreeMap<u64, u64>,
    pub redactor: StreamRedactor,
}

impl Producer {
    /// A producer on the shared policy, `StreamRedactor::new()`.
    pub fn new(id: &str) -> Producer {
        Producer::with_redactor(id, StreamRedactor::new())
    }

    pub fn with_redactor(id: &str, redactor: StreamRedactor) -> Producer {
        Producer {
            id: id.into(),
            seq: 0,
            mono: 0,
            offsets: Default::default(),
            redactor,
        }
    }

    pub fn next_frame(&mut self, stream: u64, event: FrameEvent) -> CaptureFrame {
        self.mono += 10;
        let f = CaptureFrame {
            stream: StreamKey {
                producer: self.id.clone(),
                stream,
            },
            seq: self.seq,
            observed: producer_time(&self.id, self.mono),
            event,
        };
        self.seq += 1;
        f
    }

    pub fn try_push(
        &mut self,
        stream: u64,
        event: FrameEvent,
    ) -> std::io::Result<Vec<CaptureRecord>> {
        let f = self.next_frame(stream, event);
        self.redactor.push(f)
    }

    pub fn push(&mut self, stream: u64, event: FrameEvent) -> Vec<CaptureRecord> {
        self.try_push(stream, event).expect("frame admitted")
    }

    pub fn open(&mut self, stream: u64, source: CaptureSource) -> Vec<CaptureRecord> {
        self.offsets.insert(stream, 0);
        self.push(
            stream,
            FrameEvent::Open {
                source,
                metadata: http_meta(source),
            },
        )
    }

    pub fn bytes(&mut self, stream: u64, data: &[u8]) -> Vec<CaptureRecord> {
        let offset = self.offsets.get(&stream).copied().unwrap_or(0);
        self.offsets.insert(stream, offset + data.len() as u64);
        self.push(
            stream,
            FrameEvent::Bytes {
                offset,
                data: data.to_vec(),
            },
        )
    }

    pub fn end(&mut self, stream: u64, reason: EndReason) -> Vec<CaptureRecord> {
        self.push(stream, FrameEvent::End { reason })
    }

    pub fn gap(&mut self, stream: u64, reason: GapReason, lost: Option<u64>) -> Vec<CaptureRecord> {
        if let Some(n) = lost {
            *self.offsets.entry(stream).or_default() += n;
        }
        self.push(
            stream,
            FrameEvent::Gap {
                reason,
                lost_bytes: lost,
            },
        )
    }
}

/// Stream `input` through `redactor` on a fresh producer's stream 1, in the given chunk
/// sizes (cycled), with a flush after every `flush_every` pushes, ending `Complete`.
pub fn stream_through(
    redactor: StreamRedactor,
    input: &[u8],
    chunks: &[usize],
    flush_every: usize,
) -> Vec<CaptureRecord> {
    let mut p = Producer::with_redactor(P1, redactor);
    let mut records = p.open(1, CaptureSource::WireRequest);
    let mut at = 0;
    let mut i = 0;
    while at < input.len() {
        let n = chunks[i % chunks.len()]
            .clamp(1, MAX_SEGMENT_BYTES)
            .min(input.len() - at);
        records.extend(p.bytes(1, &input[at..at + n]));
        at += n;
        i += 1;
        if flush_every > 0 && i % flush_every == 0 {
            records.extend(p.redactor.flush(daemon_time(i as u64)).unwrap());
        }
    }
    records.extend(p.end(1, EndReason::Complete));
    records
}

/// A small deterministic generator, so a failure names its seed.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

pub fn random_chunks(rng: &mut Rng, max: usize) -> Vec<usize> {
    (0..64).map(|_| 1 + rng.below(max)).collect()
}

pub fn pick<'a>(rng: &mut Rng, options: &[&'a [u8]]) -> &'a [u8] {
    options[rng.below(options.len())]
}

/// Byte ranges of `want` (the oracle's spans) that no span of `got` (the stream's spans)
/// covers: bytes the oracle redacts and the stream retained raw.
pub fn uncovered(
    want: &[RedactionAuditSpan],
    got: &[RedactionAuditSpan],
) -> Vec<(String, u64, u64)> {
    let mut missing = Vec::new();
    for w in want {
        let mut at = w.start;
        for g in got.iter().filter(|g| g.end > w.start && g.start < w.end) {
            if g.start > at {
                missing.push((w.kind.clone(), at, g.start.min(w.end)));
            }
            at = at.max(g.end);
        }
        if at < w.end {
            missing.push((w.kind.clone(), at, w.end));
        }
    }
    missing
}
