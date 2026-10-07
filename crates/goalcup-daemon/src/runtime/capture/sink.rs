//! The owner's bounded sink: epoch admission, per-producer sequence and stream checks, and the
//! queue the owner drains in admission order.

use super::*;

/// Queue capacity held back from payload for closes and loss markers.
pub(super) const CONTROL_RESERVE_BYTES: usize = 128 * 1024;

/// The ceiling on queued payload frames, in encoded bytes.
pub(super) const PAYLOAD_QUEUE_BYTES: usize = MAX_CAPTURE_QUEUE_BYTES - CONTROL_RESERVE_BYTES;

/// A fresh epoch is answered `Pending` while queued payload is above this mark.
pub(super) const RECOVERY_LOW_WATER_BYTES: usize = PAYLOAD_QUEUE_BYTES / 2;

/// The fixed accounting cost of one control item.
pub(super) const CONTROL_ITEM_COST: usize = 512;

const _: () = assert!(MAX_PRODUCERS * CONTROL_ITEM_COST <= CONTROL_RESERVE_BYTES);

/// One item the owner drains, in admission order.
#[derive(Debug)]
// Frames move through the queue by value, as the sink received them.
#[allow(clippy::large_enum_variant)]
pub(super) enum QueueItem {
    Frame(CaptureFrame),
    /// A validated clean close: every frame the producer had admitted precedes it, it has no
    /// open payload stream, and `final_seq` is its last admitted sequence.
    Closed {
        producer: String,
        final_seq: Option<u64>,
    },
    /// The producer was refused or superseded and latched lost; what it admitted precedes this.
    Lost {
        producer: String,
        reason: GapReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProducerStatus {
    Live,
    Closed,
    Lost(GapReason),
    Superseded,
}

#[derive(Debug)]
pub(super) struct Producer {
    pub(super) status: ProducerStatus,
    next_seq: u64,
    high_stream: u64,
    /// Open payload streams and their source-offset position.
    open: BTreeMap<u64, Offsets>,
    /// Its first frame was a Recovery, or it admitted a Connection or Loss frame.
    wire: bool,
    /// It admitted a frame the chain records under its name: anything but a `Loss`, which
    /// chains only as a stream-less gap naming no producer. Without one the integrity reader
    /// calls it unrecorded, and a Recovery naming it is an orphan recovery (CONTRACTS, H-16
    /// ¶4). The receiver keeps the same flag.
    named: bool,
    /// Its first frame was a Recovery, so it may re-observe surviving connections.
    recovered: bool,
}

impl Producer {
    pub(super) fn new(status: ProducerStatus) -> Producer {
        Producer {
            status,
            next_seq: 0,
            high_stream: 0,
            open: BTreeMap::new(),
            wire: false,
            named: false,
            recovered: false,
        }
    }
}

/// Which share of the sink's capacity a queued item spends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    /// Payload frames, bounded by [`PAYLOAD_QUEUE_BYTES`].
    Payload,
    /// Closes and loss markers, the reserve a full payload queue never takes.
    Control,
}

/// One queued item with the capacity it spends.
pub(super) struct Queued {
    item: QueueItem,
    cost: usize,
    lane: Lane,
}

#[derive(Default)]
pub(super) struct SinkState {
    queue: VecDeque<Queued>,
    pub(super) payload_bytes: usize,
    control_bytes: usize,
    pub(super) producers: BTreeMap<String, Producer>,
    /// IDs answered `Pending`: each holds a producer slot until admitted.
    pending_ids: BTreeSet<String>,
    /// Open payload streams across every producer, a lost one's until a Recovery supersedes it.
    open_streams: usize,
    /// Active connection identity -> the producer that owns it.
    connections: BTreeMap<String, String>,
    /// Set when the owner stops: nothing more is admitted.
    closed: bool,
}

/// The entry owner's bounded sink, shared by the ingest receiver and the native adapter.
///
/// It checks what a receiver checks — epoch admission, per-producer sequence continuity,
/// stream transitions, the fresh-epoch recovery rule, and the entry-wide producer and
/// open-stream bounds across wire, native and goal producers — then queues. Every method takes
/// one short lock and never waits on disk or network, and none calls back into the receiver.
///
/// Queued payload is accounted in encoded bytes and refused with `QueueFull` above
/// [`PAYLOAD_QUEUE_BYTES`]. Closes and loss markers spend a separate reserve that at most one
/// terminal control per producer can fill, so a full payload queue never blocks a close or
/// hides a loss.
///
/// A Recovery supersedes the producer it names. When the chain records nothing under that
/// producer's name (it admitted no frame, or only `Loss` frames), it also supersedes every other
/// wire producer that is neither closed nor superseded, as the integrity reader does (one wire
/// lineage per entry). Each superseded producer that was still
/// live is queued lost before the Recovery itself, so the owner interrupts its redactor first.
#[derive(Default)]
pub(super) struct OwnerSink {
    state: Mutex<SinkState>,
    ready: Condvar,
}

impl OwnerSink {
    pub(super) fn lock(&self) -> MutexGuard<'_, SinkState> {
        lock(&self.state)
    }

    /// Take queued items in admission order: up to `max_items`, waiting at most `wait` for
    /// the first one.
    pub(super) fn drain(&self, max_items: usize, wait: Duration) -> Vec<QueueItem> {
        let mut state = self.lock();
        if state.queue.is_empty() && !wait.is_zero() {
            state = self
                .ready
                .wait_timeout_while(state, wait, |s| s.queue.is_empty())
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poison| poison.into_inner().0);
        }
        let take = state.queue.len().min(max_items);
        let taken: Vec<_> = state.queue.drain(..take).collect();
        let mut out = Vec::with_capacity(take);
        for Queued { item, cost, lane } in taken {
            match lane {
                Lane::Control => state.control_bytes -= cost,
                Lane::Payload => state.payload_bytes -= cost,
            }
            out.push(item);
        }
        out
    }

    /// Admit nothing more and latch every live producer lost: it never closed.
    pub(super) fn close_intake(&self) {
        let mut state = self.lock();
        state.closed = true;
        let live: Vec<String> = state
            .producers
            .iter()
            .filter(|(_, p)| p.status == ProducerStatus::Live)
            .map(|(id, _)| id.clone())
            .collect();
        for id in live {
            lose(&mut state, &id, GapReason::ShutdownDeadline);
        }
    }

    /// Count a producer that completed outside the sink (the goal) against the entry's
    /// producer bound. `false` when the bound is spent.
    pub(super) fn reserve_closed(&self, producer: &str) -> bool {
        let mut state = self.lock();
        if state.producers.len() + state.pending_ids.len() >= MAX_PRODUCERS {
            return false;
        }
        state
            .producers
            .insert(producer.to_string(), Producer::new(ProducerStatus::Closed));
        true
    }
}

/// Latch `id` lost (if live) and queue its loss marker from the control reserve.
pub(super) fn lose(state: &mut SinkState, id: &str, reason: GapReason) {
    let Some(producer) = state.producers.get_mut(id) else {
        return;
    };
    if producer.status != ProducerStatus::Live {
        return;
    }
    producer.status = ProducerStatus::Lost(reason);
    push_control(
        state,
        QueueItem::Lost {
            producer: id.to_string(),
            reason,
        },
    );
}

pub(super) fn push_control(state: &mut SinkState, item: QueueItem) {
    state.control_bytes += CONTROL_ITEM_COST;
    state.queue.push_back(Queued {
        item,
        cost: CONTROL_ITEM_COST,
        lane: Lane::Control,
    });
}

/// The frame's length as an ingest item: u32 prefix, the `{"kind":"frame",...}` wrapper and
/// the compact JSON body, computed without allocating the encoding.
pub(super) fn encoded_cost(frame: &CaptureFrame) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    // Serializing a validated frame into a counter cannot fail.
    let _ = serde_json::to_writer(&mut count, frame);
    4 + r#"{"kind":"frame",}"#.len() + count.0
}

/// Why a frame breaks the sequence or stream-state rules.
pub(super) enum Refusal {
    Invalid,
    Limit,
}

impl CaptureSink for OwnerSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        let valid_shape = frame.validate().is_ok();
        let cost = encoded_cost(&frame);
        let mut state = self.lock();
        let id = frame.stream.producer.clone();
        if state.closed {
            lose(&mut state, &id, GapReason::ShutdownDeadline);
            return SubmitResult::Disabled(GapReason::ShutdownDeadline);
        }
        let Some(producer) = state.producers.get(&id) else {
            return SubmitResult::Disabled(GapReason::ProducerLost);
        };
        match producer.status {
            ProducerStatus::Live => {}
            ProducerStatus::Lost(reason) => return SubmitResult::Disabled(reason),
            ProducerStatus::Closed | ProducerStatus::Superseded => {
                return SubmitResult::Disabled(GapReason::ProducerLost);
            }
        }
        let checked = if valid_shape {
            check_transition(&state, &frame)
        } else {
            Err(Refusal::Invalid)
        };
        let refused = match checked {
            Err(Refusal::Invalid) => Some(GapReason::ProducerLost),
            Err(Refusal::Limit) => Some(GapReason::LimitExceeded),
            Ok(()) if state.payload_bytes + cost > PAYLOAD_QUEUE_BYTES => {
                Some(GapReason::QueueFull)
            }
            Ok(()) => None,
        };
        if let Some(reason) = refused {
            lose(&mut state, &id, reason);
            self.ready.notify_all();
            return SubmitResult::Disabled(reason);
        }
        apply_transition(&mut state, &frame);
        if let Some(producer) = state.producers.get_mut(&id) {
            producer.next_seq += 1;
        }
        state.payload_bytes += cost;
        state.queue.push_back(Queued {
            item: QueueItem::Frame(frame),
            cost,
            lane: Lane::Payload,
        });
        self.ready.notify_all();
        SubmitResult::Queued
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        if !is_capture_id(producer) {
            return EpochAdmission::Disabled(GapReason::LimitExceeded);
        }
        let mut state = self.lock();
        if state.closed {
            return EpochAdmission::Disabled(GapReason::ShutdownDeadline);
        }
        if let Some(known) = state.producers.get(producer) {
            return match known.status {
                ProducerStatus::Live => EpochAdmission::Ready,
                // A retired identity is never admitted again.
                _ => EpochAdmission::RetryFresh,
            };
        }
        let known_pending = state.pending_ids.contains(producer);
        if !known_pending && state.producers.len() + state.pending_ids.len() >= MAX_PRODUCERS {
            return EpochAdmission::Disabled(GapReason::LimitExceeded);
        }
        if state.payload_bytes > RECOVERY_LOW_WATER_BYTES {
            state.pending_ids.insert(producer.to_string());
            return EpochAdmission::Pending;
        }
        state.pending_ids.remove(producer);
        state
            .producers
            .insert(producer.to_string(), Producer::new(ProducerStatus::Live));
        EpochAdmission::Ready
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        let mut state = self.lock();
        let Some(known) = state.producers.get(producer) else {
            return SubmitResult::Disabled(GapReason::ProducerLost);
        };
        match known.status {
            ProducerStatus::Live => {}
            ProducerStatus::Closed => return SubmitResult::Queued,
            ProducerStatus::Lost(reason) => return SubmitResult::Disabled(reason),
            ProducerStatus::Superseded => return SubmitResult::Disabled(GapReason::ProducerLost),
        }
        if state.closed {
            lose(&mut state, producer, GapReason::ShutdownDeadline);
            return SubmitResult::Disabled(GapReason::ShutdownDeadline);
        }
        if final_seq != known.next_seq.checked_sub(1) || !known.open.is_empty() {
            // A close that does not match what was admitted is unknown tail loss.
            lose(&mut state, producer, GapReason::ProducerLost);
            self.ready.notify_all();
            return SubmitResult::Disabled(GapReason::ProducerLost);
        }
        if let Some(known) = state.producers.get_mut(producer) {
            known.status = ProducerStatus::Closed;
        }
        state.connections.retain(|_, owner| owner != producer);
        push_control(
            &mut state,
            QueueItem::Closed {
                producer: producer.to_string(),
                final_seq,
            },
        );
        self.ready.notify_all();
        SubmitResult::Queued
    }
}

/// The sequence and stream-state checks, before anything is changed.
pub(super) fn check_transition(state: &SinkState, frame: &CaptureFrame) -> Result<(), Refusal> {
    let producer = &state.producers[&frame.stream.producer];
    if frame.seq != producer.next_seq {
        return Err(Refusal::Invalid);
    }
    let stream = frame.stream.stream;
    match &frame.event {
        FrameEvent::Connection { connection, event } => match event {
            ConnectionEvent::Open { already_open, .. } => {
                if *already_open && !producer.recovered {
                    return Err(Refusal::Invalid);
                }
                if state.connections.contains_key(connection) {
                    return Err(Refusal::Invalid);
                }
                if state.connections.len() >= MAX_OPEN_CONNECTIONS {
                    return Err(Refusal::Limit);
                }
                Ok(())
            }
            ConnectionEvent::UpgradedWebSocket | ConnectionEvent::Close { .. } => {
                if state.connections.get(connection) == Some(&frame.stream.producer) {
                    Ok(())
                } else {
                    Err(Refusal::Invalid)
                }
            }
        },
        // A stream-zero loss observation: no stream, connection or ordering state beyond its
        // sequence number, which is checked above.
        FrameEvent::Loss { .. } => Ok(()),
        FrameEvent::Recovery {
            previous_producer, ..
        } => {
            // A fresh epoch's first frame names an earlier producer that neither closed nor
            // was already superseded.
            if frame.seq != 0 {
                return Err(Refusal::Invalid);
            }
            // Only a wire epoch, or one that admitted no frame (CP-A4), is recoverable.
            match state.producers.get(previous_producer) {
                Some(p)
                    if matches!(p.status, ProducerStatus::Live | ProducerStatus::Lost(_))
                        && (p.wire || p.next_seq == 0) =>
                {
                    Ok(())
                }
                _ => Err(Refusal::Invalid),
            }
        }
        FrameEvent::Open { .. } => {
            if stream <= producer.high_stream {
                return Err(Refusal::Invalid);
            }
            if state.open_streams >= MAX_OPEN_STREAMS {
                return Err(Refusal::Limit);
            }
            Ok(())
        }
        FrameEvent::Bytes { offset, .. } => match producer.open.get(&stream) {
            Some(position) if position.admits(*offset) => Ok(()),
            _ => Err(Refusal::Invalid),
        },
        FrameEvent::End { .. } | FrameEvent::Gap { .. } => {
            if producer.open.contains_key(&stream) {
                Ok(())
            } else {
                Err(Refusal::Invalid)
            }
        }
    }
}

pub(super) fn apply_transition(state: &mut SinkState, frame: &CaptureFrame) {
    let id = &frame.stream.producer;
    let stream = frame.stream.stream;
    if !matches!(frame.event, FrameEvent::Loss { .. })
        && let Some(producer) = state.producers.get_mut(id)
    {
        producer.named = true;
    }
    match &frame.event {
        FrameEvent::Recovery {
            previous_producer, ..
        } => {
            // The reader's orphan: a predecessor the chain records nothing under its name for
            // (no admitted frame, or only `Loss` frames).
            let orphan = state
                .producers
                .get(previous_producer)
                .is_some_and(|p| !p.named);
            let superseded: Vec<String> = state
                .producers
                .iter()
                .filter(|(other, p)| {
                    *other == previous_producer
                        || (orphan
                            && *other != id
                            && p.wire
                            && matches!(p.status, ProducerStatus::Live | ProducerStatus::Lost(_)))
                })
                .map(|(other, _)| other.clone())
                .collect();
            for other in &superseded {
                let Some(p) = state.producers.get_mut(other) else {
                    continue;
                };
                let released = std::mem::take(&mut p.open).len();
                let was_live = p.status == ProducerStatus::Live;
                p.status = ProducerStatus::Superseded;
                state.open_streams -= released;
                if was_live {
                    // Superseded while live: its tail is unknown loss, queued before the
                    // Recovery so its redactor is interrupted first.
                    push_control(
                        state,
                        QueueItem::Lost {
                            producer: other.clone(),
                            reason: GapReason::ProducerLost,
                        },
                    );
                }
            }
            state
                .connections
                .retain(|_, owner| !superseded.contains(owner));
            if let Some(fresh) = state.producers.get_mut(id) {
                fresh.wire = true;
                fresh.recovered = true;
            }
        }
        FrameEvent::Loss { .. } => {
            // A loss-only epoch is a wire epoch: recoverable, like one that opened a connection.
            if let Some(producer) = state.producers.get_mut(id) {
                producer.wire = true;
            }
        }
        FrameEvent::Connection { connection, event } => {
            if let Some(producer) = state.producers.get_mut(id) {
                producer.wire = true;
            }
            match event {
                ConnectionEvent::Open { .. } => {
                    state.connections.insert(connection.clone(), id.clone());
                }
                ConnectionEvent::Close { .. } => {
                    state.connections.remove(connection);
                }
                ConnectionEvent::UpgradedWebSocket => {}
            }
        }
        FrameEvent::Open { .. } => {
            if let Some(producer) = state.producers.get_mut(id) {
                producer.high_stream = stream;
                producer.open.insert(stream, Offsets::default());
                state.open_streams += 1;
            }
        }
        FrameEvent::Bytes { offset, data } => {
            if let Some(position) = state
                .producers
                .get_mut(id)
                .and_then(|p| p.open.get_mut(&stream))
            {
                position.next = Some(offset + data.len() as u64);
                position.after_gap = false;
            }
        }
        FrameEvent::Gap { .. } => {
            if let Some(position) = state
                .producers
                .get_mut(id)
                .and_then(|p| p.open.get_mut(&stream))
            {
                position.after_gap = true;
            }
        }
        FrameEvent::End { .. } => {
            if let Some(producer) = state.producers.get_mut(id)
                && producer.open.remove(&stream).is_some()
            {
                state.open_streams -= 1;
            }
        }
    }
}
