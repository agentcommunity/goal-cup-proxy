//! The stream tracker shared by the two OpenAI wire decoders, Responses (`openai.rs`) and Chat
//! Completions (`chat.rs`).
//!
//! The tracker owns everything about a selected stream that is not the protocol's meaning: which
//! streams are open and within `MAX_OPEN_STREAMS`, how a stream's retained bytes are framed into
//! logical records (one whole-stream JSON document, SSE events through the shared framer in
//! `wire.rs`, or a body of an unsupported content type), the buffer budget, and how a `Gap`, an
//! invalid redaction audit, a `Recovery`, a closed producer and an `End` are reported.
//!
//! What a complete record means is the protocol's. The tracker calls the plain functions of a
//! [`Protocol`] at the few points where the two decoders differ: a complete JSON document, an SSE
//! event, an SSE event the framer could not hand over, bytes an invalid audit discarded inside an
//! SSE body, and the end of an SSE body. The calls are static; there is no dynamic dispatch.

use super::api::{DecoderFailure, DecoderSummary, Observation};
use super::wire::{
    Close, LossCauses, MAX_BUFFERED_BYTES, MAX_EVENT_BYTES, Origin, Out, ResyncCause, SegmentView,
    Span, Sse, SseItem, Tally, classify_close,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, MAX_OPEN_STREAMS, StreamKey, WireIdentity,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io;

/// What one wire protocol does with the records the tracker frames.
pub(super) trait Protocol {
    /// What an SSE body keeps beside the framer: what its events add up to, if anything.
    type SseState: Debug + Default;

    /// Whether this protocol reads a stream: the one selection rule.
    fn selects(source: CaptureSource, hint: Option<&str>) -> bool;

    /// One complete whole-stream JSON document of `source` (a WebSocket message or an HTTP
    /// JSON body) with at least one contributing segment.
    fn json(
        bytes: &[u8],
        source: CaptureSource,
        span: &Span,
        origin: &Origin<'_>,
        out: &mut Out<'_>,
    );

    /// One complete SSE event's data.
    fn event(
        state: &mut Self::SseState,
        data: &[u8],
        span: &Span,
        origin: &Origin<'_>,
        out: &mut Out<'_>,
    );

    /// An SSE event the framer could not hand over: a gap (with its cause) or an over-budget
    /// event. The protocol reports it.
    fn lost(
        state: &mut Self::SseState,
        reason: &str,
        span: &Span,
        origin: &Origin<'_>,
        out: &mut Out<'_>,
    );

    /// Bytes of an SSE body discarded at an invalid redaction audit. The tracker has already
    /// reported them under `invalid_redaction_audit`.
    fn discarded(state: &mut Self::SseState, span: &Span);

    /// The span a gap at the end of an SSE body is reported over when no event was pending.
    fn stream_span(state: &Self::SseState, history: &Span) -> Span;

    /// The SSE body ended (or `finish` found it still open). A trailing gap has already been
    /// reported through [`Protocol::lost`].
    fn close(state: Self::SseState, end: &SseEnd<'_>, origin: &Origin<'_>, out: &mut Out<'_>);
}

/// How an SSE body ended, for [`Protocol::close`].
pub(super) struct SseEnd<'a> {
    /// How the stream ended, when it did not end complete.
    pub(super) cut: Option<&'static str>,
    /// Why the bytes of a last, undispatched event were never decoded (never `gap`: a trailing
    /// gap is reported before the close).
    pub(super) leftover: Option<&'static str>,
    /// Whether that undispatched event held a `data` field with a value.
    pub(super) pending_data: bool,
    /// The segments of that undispatched event.
    pub(super) pending: &'a Span,
    /// Every segment of the stream.
    pub(super) history: &'a Span,
}

/// The open streams of one decoder, the shared buffer budget and the decoder's counters.
#[derive(Debug)]
pub(super) struct Tracker<P: Protocol> {
    streams: BTreeMap<StreamKey, Tracked<P>>,
    buffered: usize,
    tally: Tally,
}

impl<P: Protocol> Default for Tracker<P> {
    fn default() -> Self {
        Self {
            streams: BTreeMap::new(),
            buffered: 0,
            tally: Tally::default(),
        }
    }
}

impl<P: Protocol> Tracker<P> {
    pub(super) fn push(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        self.read(record, emit)
            .map_err(|error| DecoderFailure::from_io(&error))
    }

    pub(super) fn push_recovery(
        &mut self,
        record: &CaptureRecord,
        lost_streams: &[StreamKey],
        emit: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        if !matches!(record, CaptureRecord::Recovery { .. }) {
            return Err(DecoderFailure::new("recovery_scope_unsupported"));
        }
        self.recovery(lost_streams, emit);
        Ok(())
    }

    pub(super) fn finish(
        &mut self,
        emit: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        self.summarize(emit)
            .map_err(|error| DecoderFailure::from_io(&error))
    }

    fn open(&mut self, stream: &StreamKey, source: CaptureSource, metadata: &CaptureMeta) {
        // `accepts` selected this stream; this is a guard, not the selection.
        if !P::selects(source, metadata.protocol_hint.as_deref()) {
            debug_assert!(false, "a stream `accepts` refused was routed here");
            return;
        }
        let content_type = metadata.content_type.as_deref();
        let framing = match source {
            // Only a protocol with a WebSocket form selects these sources.
            CaptureSource::WsClient | CaptureSource::WsServer => Framing::Json(Vec::new()),
            CaptureSource::WireRequest | CaptureSource::WireResponse
                if content_type == Some("application/json") =>
            {
                Framing::Json(Vec::new())
            }
            CaptureSource::WireResponse if content_type == Some("text/event-stream") => {
                Framing::Sse(Box::default())
            }
            CaptureSource::WireRequest | CaptureSource::WireResponse => {
                Framing::Unsupported(metadata.content_type.clone())
            }
            // Refused by `selects` above; the arm only keeps the match exhaustive.
            CaptureSource::Native | CaptureSource::Goal => return,
        };
        if self.streams.contains_key(stream) {
            // A reused stream key is invalid evidence (the reader refuses it); the first stays.
            return;
        }
        if self.streams.len() >= MAX_OPEN_STREAMS {
            // Untracked, so its records are lost to this analysis: counted, never guessed.
            self.tally.unknown += 1;
            self.tally.incomplete = true;
            return;
        }
        self.streams.insert(
            stream.clone(),
            Tracked {
                source,
                wire: metadata.wire.clone(),
                status: metadata.status,
                framing,
                span: Span::default(),
                history: Span::default(),
                loss: LossCauses::default(),
                invalid_audit_loss: false,
                resync_cause: None,
            },
        );
    }

    fn segment(
        &mut self,
        stream: &StreamKey,
        view: &SegmentView<'_>,
        data_b64: &str,
        emit: &mut dyn FnMut(Observation),
    ) {
        let Self {
            streams,
            buffered,
            tally,
        } = self;
        let Some(tracked) = streams.get_mut(stream) else {
            return;
        };
        let Ok(data) = STANDARD.decode(data_b64) else {
            tracked.span.touch_unprojected(view);
            tracked.history.touch_unprojected(view);
            tracked.damage(buffered, tally, "undecodable_segment");
            return;
        };
        let view = view.with_retained(&data);
        tracked.history.touch(&view);
        if view.invalid_audit() {
            tracked.span.touch(&view);
            if let Framing::Sse(body) = &mut tracked.framing {
                P::discarded(&mut body.state, &tracked.span);
            }
            let origin = Origin {
                stream,
                source: tracked.source,
                wire: tracked.wire.as_ref(),
                status: tracked.status,
            };
            Out {
                tally,
                buffered,
                emit,
            }
            .unknown(
                &origin,
                &tracked.span,
                "invalid_redaction_audit",
                Map::new(),
            );
            tracked.damage(buffered, tally, "invalid_redaction_audit");
            return;
        }
        let Tracked {
            source,
            wire,
            status,
            framing,
            span,
            history: _,
            loss,
            invalid_audit_loss: _,
            resync_cause,
        } = tracked;
        match framing {
            Framing::Json(buf) => {
                span.touch(&view);
                if !loss.is_empty() {
                    return;
                }
                if buf.len() + data.len() > MAX_EVENT_BYTES
                    || *buffered + data.len() > MAX_BUFFERED_BYTES
                {
                    *buffered -= buf.len();
                    *buf = Vec::new();
                    loss.remember("over_budget");
                    tally.incomplete = true;
                } else {
                    buf.extend_from_slice(&data);
                    *buffered += data.len();
                }
            }
            Framing::Unsupported(_) => span.touch(&view),
            Framing::Sse(body) => {
                let origin = Origin {
                    stream,
                    source: *source,
                    wire: wire.as_ref(),
                    status: *status,
                };
                let mut out = Out {
                    tally,
                    buffered,
                    emit,
                };
                let SseBody { sse, state } = &mut **body;
                sse.feed(
                    span,
                    &view,
                    &data,
                    &mut out,
                    &mut |item, span, out| match item {
                        SseItem::Data { data, .. } => P::event(state, data, span, &origin, out),
                        SseItem::Gap => {
                            let findings = resync_cause
                                .take()
                                .map(ResyncCause::findings)
                                .unwrap_or([None, Some("gap"), None]);
                            for reason in findings.into_iter().flatten() {
                                P::lost(state, reason, span, &origin, out);
                            }
                        }
                        SseItem::OverBudget => P::lost(state, "over_budget", span, &origin, out),
                    },
                );
            }
        }
    }

    fn gap(&mut self, stream: &StreamKey) {
        let Self {
            streams,
            buffered,
            tally,
        } = self;
        if let Some(tracked) = streams.get_mut(stream) {
            tracked.damage(buffered, tally, "gap");
        }
    }

    fn recovery(&mut self, lost_streams: &[StreamKey], emit: &mut dyn FnMut(Observation)) {
        self.tally.incomplete = true;
        for key in lost_streams {
            if let Some(tracked) = self.streams.remove(key) {
                self.close(key, tracked, Close::ProducerLost, emit);
            }
        }
    }

    fn close(
        &mut self,
        stream: &StreamKey,
        tracked: Tracked<P>,
        how: Close,
        emit: &mut dyn FnMut(Observation),
    ) {
        let Tracked {
            source,
            wire,
            status,
            framing,
            span,
            history,
            loss,
            invalid_audit_loss,
            resync_cause,
        } = tracked;
        let origin = Origin {
            stream,
            source,
            wire: wire.as_ref(),
            status,
        };
        let mut out = Out {
            tally: &mut self.tally,
            buffered: &mut self.buffered,
            emit,
        };
        let close = classify_close(how);
        let cut = close.finding;
        // D45: an interrupted or decode-error end is reported, and the bytes that arrived stay
        // decoded; a lost producer or a stream nobody ended is a loss.
        if close.loss {
            out.tally.incomplete = true;
        }
        match framing {
            Framing::Json(buf) => {
                *out.buffered -= buf.len();
                if !loss.is_empty() {
                    for reason in loss.findings().into_iter().flatten() {
                        if reason != "invalid_redaction_audit" || !invalid_audit_loss {
                            out.unknown(&origin, &span, reason, Map::new());
                        }
                    }
                    if let Some(cut) = cut {
                        out.unknown(&origin, &span, cut, Map::new());
                    }
                } else if let Some(reason) = cut {
                    out.unknown(&origin, &span, reason, Map::new());
                } else if span.first.is_none() {
                    if matches!(source, CaptureSource::WsClient | CaptureSource::WsServer) {
                        out.unknown(&origin, &span, "empty_message", Map::new());
                    }
                } else {
                    P::json(&buf, source, &span, &origin, &mut out);
                }
            }
            Framing::Unsupported(content_type) => {
                if span.first.is_some() || !loss.is_empty() || cut.is_some() {
                    for reason in loss.findings().into_iter().flatten() {
                        if reason != "invalid_redaction_audit" || !invalid_audit_loss {
                            let mut extra = Map::new();
                            if let Some(content_type) = &content_type {
                                extra.insert(
                                    "content_type".into(),
                                    Value::String(content_type.clone()),
                                );
                            }
                            out.unknown(&origin, &span, reason, extra);
                        }
                    }
                    let mut extra = Map::new();
                    if let Some(content_type) = content_type {
                        extra.insert("content_type".into(), Value::String(content_type));
                    }
                    out.unknown(&origin, &span, "unsupported_content_type", extra);
                    if let Some(cut) = cut {
                        out.unknown(&origin, &span, cut, Map::new());
                    }
                }
            }
            Framing::Sse(body) => {
                // An SSE body's losses are resync causes of the framer, never whole-record losses.
                debug_assert!(loss.is_empty(), "an SSE body remembered a record loss");
                let SseBody { sse, mut state } = *body;
                let pending_data = sse.pending_has_data();
                let mut leftover = sse.finish(out.buffered, &span, cut);
                if leftover == Some("gap") {
                    let loss_span = if span.first.is_some() {
                        span.clone()
                    } else {
                        P::stream_span(&state, &history)
                    };
                    let findings = resync_cause.map(ResyncCause::findings).unwrap_or([
                        None,
                        Some("gap"),
                        None,
                    ]);
                    for reason in findings.into_iter().flatten() {
                        P::lost(&mut state, reason, &loss_span, &origin, &mut out);
                    }
                    leftover = None;
                }
                let end = SseEnd {
                    cut,
                    leftover,
                    pending_data,
                    pending: &span,
                    history: &history,
                };
                P::close(state, &end, &origin, &mut out);
            }
        }
    }

    fn read(
        &mut self,
        record: &CaptureRecord,
        emit: &mut dyn FnMut(Observation),
    ) -> io::Result<()> {
        match record {
            CaptureRecord::Open {
                stream,
                source,
                metadata,
                ..
            } => self.open(stream, *source, metadata),
            CaptureRecord::Segment {
                stream,
                part,
                source_offset,
                source_len,
                first,
                last,
                data_b64,
                audit,
            } => {
                let view = SegmentView {
                    part: *part,
                    first,
                    last,
                    audit: &audit.spans,
                    source_offset: *source_offset,
                    source_len: *source_len,
                };
                self.segment(stream, &view, data_b64, emit);
            }
            CaptureRecord::Gap {
                stream: Some(stream),
                ..
            } => self.gap(stream),
            // A stream-less loss may have taken selected traffic with it.
            CaptureRecord::Gap { stream: None, .. } => self.tally.incomplete = true,
            CaptureRecord::End { stream, reason, .. } => {
                if let Some(tracked) = self.streams.remove(stream) {
                    self.close(stream, tracked, Close::End(*reason), emit);
                }
            }
            CaptureRecord::Recovery {
                previous_producer, ..
            } => {
                let lost: Vec<StreamKey> = self
                    .streams
                    .keys()
                    .filter(|key| &key.producer == previous_producer)
                    .cloned()
                    .collect();
                self.recovery(&lost, emit);
            }
            CaptureRecord::ProducerClosed { producer, .. } => {
                let open: Vec<StreamKey> = self
                    .streams
                    .keys()
                    .filter(|key| &key.producer == producer)
                    .cloned()
                    .collect();
                for key in open {
                    if let Some(tracked) = self.streams.remove(&key) {
                        self.close(&key, tracked, Close::Unterminated, emit);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn summarize(&mut self, emit: &mut dyn FnMut(Observation)) -> io::Result<DecoderSummary> {
        let open = std::mem::take(&mut self.streams);
        for (key, tracked) in open {
            self.close(&key, tracked, Close::Unterminated, emit);
        }
        Ok(DecoderSummary::new(
            self.tally.observations,
            self.tally.unknown,
            self.tally.incomplete,
        ))
    }
}

/// One selected stream and its current logical record.
#[derive(Debug)]
struct Tracked<P: Protocol> {
    source: CaptureSource,
    wire: Option<WireIdentity>,
    status: Option<u16>,
    framing: Framing<P>,
    /// The segments contributing to the current record (for SSE: the event being framed).
    span: Span,
    /// Every segment of the stream, retained after dispatched SSE events for exact early-end
    /// attribution.
    history: Span,
    /// Why the current whole-stream record can no longer be decoded.
    loss: LossCauses,
    /// An invalid audit discarded bytes and permanently limits this stream's coverage.
    invalid_audit_loss: bool,
    /// The cause whose discard boundary the shared SSE framer has not consumed yet.
    resync_cause: Option<ResyncCause>,
}

impl<P: Protocol> Tracked<P> {
    /// A loss inside the current record: release what it buffered and never parse it.
    fn damage(&mut self, buffered: &mut usize, tally: &mut Tally, reason: &'static str) {
        tally.incomplete = true;
        if reason == "invalid_redaction_audit" {
            self.invalid_audit_loss = true;
        }
        match &mut self.framing {
            Framing::Json(buf) => {
                *buffered -= buf.len();
                *buf = Vec::new();
                self.loss.remember(reason);
            }
            Framing::Unsupported(_) => {
                self.loss.remember(reason);
            }
            Framing::Sse(body) => {
                let pending_over_budget = body.sse.lose(buffered);
                if pending_over_budget {
                    ResyncCause::remember(&mut self.resync_cause, "over_budget");
                }
                ResyncCause::remember(&mut self.resync_cause, reason);
            }
        }
    }
}

#[derive(Debug)]
enum Framing<P: Protocol> {
    /// A whole-stream JSON document: a WebSocket message or an HTTP JSON body.
    Json(Vec<u8>),
    /// An HTTP `text/event-stream` body: the framer and what the protocol keeps beside it.
    Sse(Box<SseBody<P>>),
    /// A selected HTTP body of another (or no) content type, with that type.
    Unsupported(Option<String>),
}

#[derive(Debug)]
struct SseBody<P: Protocol> {
    sse: Sse,
    state: P::SseState,
}

impl<P: Protocol> Default for SseBody<P> {
    fn default() -> Self {
        Self {
            sse: Sse::default(),
            state: P::SseState::default(),
        }
    }
}
