//! The entry's persistence half: every retained record reaches the chain through
//! [`ChainWriter::append`], which keeps the chain one the integrity reader validates.

use super::*;

/// What happened to one record handed to [`ChainWriter::append`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Appended {
    Written,
    /// Not persisted: discarded after quota exhaustion, or naming a stream or connection the
    /// chain never opened because its opening record was itself discarded.
    Discarded,
}

/// The entry's persistence half: every retained record reaches `TraceExport::append_chain`
/// through [`ChainWriter::append`], which keeps the chain one the integrity reader validates
/// even when the reservation runs out.
///
/// The writer refuses payload once it would reach the protected terminal allowance, and a
/// smaller later record could still fit, so the first payload refusal is sticky: every later
/// payload record is discarded, and the loss is chained once for the entry and once per stream
/// that was open when its bytes started being discarded. Closures (the records allowed to spend
/// the terminal allowance) are still written, but only for streams and connections the chain
/// opened, so their number stays bounded. After the terminal allowance itself refuses a record,
/// nothing more is appended. A record is appended exactly as the redactor emitted it, or not at
/// all.
pub(super) struct ChainWriter {
    pub(super) export: TraceExport,
    entry_id: String,
    /// Streams whose Open is in the chain and whose End is not, and whether their payload is
    /// being discarded.
    streams: BTreeMap<StreamKey, bool>,
    /// Active connection identity -> the producer whose record opened or re-observed it.
    pub(super) connections: BTreeMap<String, String>,
    /// Producers with a retained wire record, used to recognize a lost zero-record epoch.
    wire_producers: BTreeSet<String>,
    /// Producers with any retained record. Bounded like the reader's producer table: the sink
    /// admits at most `MAX_PRODUCERS`, and only their records are ever written.
    recorded_producers: BTreeSet<String>,
    /// Fresh epochs whose first retained record was Recovery.
    recovered_producers: BTreeSet<String>,
    /// Producers whose `ProducerClosed` is in the chain.
    closed_producers: BTreeSet<String>,
    /// Producers a retained Recovery superseded: the named predecessor and, for an orphan
    /// recovery, every wire producer not yet closed. The reader refuses any later record from
    /// one (`record_from_lost_producer`), so the writer never appends one.
    pub(super) lost_producers: BTreeSet<String>,
    storage_full: bool,
    entry_gap_written: bool,
    closure_refused: bool,
    /// Gap records written, never decreasing.
    pub(super) gaps: u64,
    /// A write or flush failed; nothing more is appended.
    pub(super) failure: Option<String>,
}

/// A gap that names no stream: the record of a loss the chain cannot attribute to one.
pub(super) fn streamless_gap_record(reason: GapReason, observed: &CaptureTime) -> CaptureRecord {
    CaptureRecord::Gap {
        stream: None,
        observed: observed.clone(),
        reason,
        lost_bytes: None,
    }
}

/// The producer a record is recorded under, when it names one. A stream-less `Gap` and the
/// daemon's own records name none.
pub(super) fn record_producer(record: &CaptureRecord) -> Option<&str> {
    match record {
        CaptureRecord::Open { stream, .. }
        | CaptureRecord::Segment { stream, .. }
        | CaptureRecord::End { stream, .. }
        | CaptureRecord::Gap {
            stream: Some(stream),
            ..
        } => Some(&stream.producer),
        CaptureRecord::Connection { producer, .. }
        | CaptureRecord::Recovery { producer, .. }
        | CaptureRecord::ProducerClosed { producer, .. } => Some(producer),
        _ => None,
    }
}

/// Whether the writer spends the protected terminal allowance on this record.
pub(super) fn is_closure(record: &CaptureRecord) -> bool {
    matches!(
        record,
        CaptureRecord::Gap { .. }
            | CaptureRecord::End { .. }
            | CaptureRecord::ProducerClosed { .. }
            | CaptureRecord::Assistance { .. }
            | CaptureRecord::Connection {
                event: ConnectionEvent::Close { .. },
                ..
            }
            | CaptureRecord::LocalFinish { .. }
    )
}

impl ChainWriter {
    pub(super) fn new(export: TraceExport, entry_id: String) -> Self {
        Self {
            export,
            entry_id,
            streams: BTreeMap::new(),
            connections: BTreeMap::new(),
            wire_producers: BTreeSet::new(),
            recorded_producers: BTreeSet::new(),
            recovered_producers: BTreeSet::new(),
            closed_producers: BTreeSet::new(),
            lost_producers: BTreeSet::new(),
            storage_full: false,
            entry_gap_written: false,
            closure_refused: false,
            gaps: 0,
            failure: None,
        }
    }

    /// Append one record, keeping the chain valid under quota exhaustion. `now` stamps the
    /// envelope and any coalesced storage gap.
    pub(super) fn append(&mut self, record: CaptureRecord, now: &CaptureTime) -> Appended {
        if self.failure.is_some() || self.closure_refused || !self.admissible(&record) {
            return Appended::Discarded;
        }
        if self.storage_full && !is_closure(&record) {
            self.note_payload_loss(&record, now);
            return Appended::Discarded;
        }
        match self.write(&record, now) {
            Ok(()) => {
                self.note_written(&record);
                Appended::Written
            }
            Err(ExportError::CaptureStorageFull {
                terminal_allowance: false,
                ..
            }) => {
                self.storage_full = true;
                self.note_payload_loss(&record, now);
                Appended::Discarded
            }
            Err(ExportError::CaptureStorageFull {
                terminal_allowance: true,
                ..
            }) => {
                self.closure_refused = true;
                Appended::Discarded
            }
            Err(error) => {
                self.fail(&error);
                Appended::Discarded
            }
        }
    }

    /// Append a stream-less gap stamped `now`.
    pub(super) fn streamless_gap(&mut self, reason: GapReason, now: &CaptureTime) -> Appended {
        self.append(streamless_gap_record(reason, now), now)
    }

    /// [`Self::append`] for a record the caller cannot go on without.
    pub(super) fn require(
        &mut self,
        record: CaptureRecord,
        now: &CaptureTime,
    ) -> Result<(), DaemonError> {
        match self.append(record, now) {
            Appended::Written => Ok(()),
            Appended::Discarded => Err(match &self.failure {
                Some(failure) => DaemonError::Io {
                    path: "the capture chain".to_string(),
                    source: std::io::Error::other(failure.clone()),
                },
                None => DaemonError::Refused(
                    "the capture chain's reservation has no room for this record".to_string(),
                ),
            }),
        }
    }

    /// The loopback proxy's health counters as stream-less gaps, one per non-zero counter: a
    /// fail-open tunnel or an opaque fallback is `DecodeFailed`, a full capture table
    /// `LimitExceeded`, an unsettled cut or an unterminated message `ShutdownDeadline`. Sink
    /// refusals are already chained as the sink's own gaps and add none.
    pub(super) fn proxy_gaps(&mut self, counters: &ProxyCounters, now: &CaptureTime) {
        for (count, reason) in [
            (counters.tunneled_fail_open, GapReason::DecodeFailed),
            (counters.opaque_fallbacks, GapReason::DecodeFailed),
            (counters.capture_table_full, GapReason::LimitExceeded),
            (counters.cut_unsettled, GapReason::ShutdownDeadline),
            (counters.unterminated, GapReason::ShutdownDeadline),
        ] {
            if count > 0 {
                self.streamless_gap(reason, now);
            }
        }
    }

    /// Flush the chain: the only way appended records become the durable frontier. A flushed
    /// part that is due is then closed and the next one opened (D55); a failed rotation stops
    /// the writer exactly as a failed flush does.
    pub(super) fn sync(&mut self) -> Result<Option<DurableFrontier>, DaemonError> {
        if self.failure.is_some() {
            return Ok(self.export.capture_storage().durable);
        }
        let synced = self.export.sync_capture().and_then(|frontier| {
            self.export
                .rotate_if_due(std::time::Instant::now())
                .map(|_| frontier)
        });
        synced.map_err(|error| {
            self.fail(&error);
            DaemonError::Export(error)
        })
    }

    pub(super) fn fail(&mut self, error: &ExportError) {
        self.failure.get_or_insert_with(|| error.to_string());
    }

    /// Whether the record's stream and connection exist in the chain, so appending it cannot
    /// be the reader's `stream_not_open` or `connection_not_open`.
    pub(super) fn admissible(&self, record: &CaptureRecord) -> bool {
        if let Some(producer) = record_producer(record)
            && self.lost_producers.contains(producer)
        {
            return false;
        }
        match record {
            CaptureRecord::Segment { stream, .. } => self
                .streams
                .get(stream)
                .is_some_and(|discarding| !discarding),
            CaptureRecord::End { stream, .. }
            | CaptureRecord::Gap {
                stream: Some(stream),
                ..
            } => self.streams.contains_key(stream),
            CaptureRecord::Connection {
                producer,
                connection,
                event,
                ..
            } => match event {
                ConnectionEvent::Open { already_open, .. } => {
                    (!already_open || self.recovered_producers.contains(producer))
                        && !self.connections.contains_key(connection)
                        && self.connections.len() < MAX_OPEN_CONNECTIONS
                }
                ConnectionEvent::UpgradedWebSocket | ConnectionEvent::Close { .. } => {
                    self.connections.get(connection) == Some(producer)
                }
            },
            // The reader accepts a Recovery naming a wire producer or one with no record at all
            // (CP-A4); a recorded non-wire producer is `recovery_names_non_wire_producer`. It
            // refuses one naming a producer an earlier Recovery superseded
            // (`producer_recovered_twice`) or that closed (`recovery_after_producer_closed`), so
            // the writer alone never writes a second recovery of a producer.
            CaptureRecord::Recovery {
                previous_producer, ..
            } => {
                (self.wire_producers.contains(previous_producer)
                    || !self.recorded_producers.contains(previous_producer))
                    && !self.lost_producers.contains(previous_producer)
                    && !self.closed_producers.contains(previous_producer)
            }
            _ => true,
        }
    }

    /// Record, once per stream and once per entry, that payload is being discarded.
    pub(super) fn note_payload_loss(&mut self, record: &CaptureRecord, now: &CaptureTime) {
        if let CaptureRecord::Segment { stream, .. } = record
            && let Some(discarding) = self.streams.get_mut(stream)
            && !*discarding
        {
            *discarding = true;
            self.write_storage_gap(Some(stream.clone()), now);
        }
        if !self.entry_gap_written {
            self.entry_gap_written = true;
            self.write_storage_gap(None, now);
        }
    }

    pub(super) fn write_storage_gap(&mut self, stream: Option<StreamKey>, now: &CaptureTime) {
        let gap = CaptureRecord::Gap {
            stream,
            observed: now.clone(),
            reason: GapReason::StorageFull,
            lost_bytes: None,
        };
        match self.write(&gap, now) {
            Ok(()) => self.note_written(&gap),
            Err(ExportError::CaptureStorageFull { .. }) => self.closure_refused = true,
            Err(error) => self.fail(&error),
        }
    }

    pub(super) fn write(
        &mut self,
        record: &CaptureRecord,
        now: &CaptureTime,
    ) -> Result<(), ExportError> {
        let mut envelope = Envelope {
            protocol_version: M2_PROTOCOL_VERSION,
            entry_id: self.entry_id.clone(),
            source: "daemon".to_string(),
            recorded_at: now.wall.clone(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Capture {
                record: record.clone(),
            },
            redaction: None,
        };
        self.export.append_chain(&mut envelope).map(|_| ())
    }

    pub(super) fn note_written(&mut self, record: &CaptureRecord) {
        if let Some(producer) = record_producer(record) {
            self.recorded_producers.insert(producer.to_string());
        }
        match record {
            CaptureRecord::Open { stream, .. } => {
                self.streams.insert(stream.clone(), false);
            }
            CaptureRecord::End { stream, .. } => {
                self.streams.remove(stream);
            }
            CaptureRecord::Gap { .. } => self.gaps += 1,
            CaptureRecord::Connection {
                producer,
                connection,
                event,
                ..
            } => {
                self.wire_producers.insert(producer.clone());
                match event {
                    ConnectionEvent::Open { .. } => {
                        self.connections
                            .insert(connection.clone(), producer.clone());
                    }
                    ConnectionEvent::Close { .. } => {
                        self.connections.remove(connection);
                    }
                    ConnectionEvent::UpgradedWebSocket => {}
                }
            }
            CaptureRecord::Recovery {
                previous_producer,
                producer,
                ..
            } => {
                let orphan = !self.wire_producers.contains(previous_producer);
                self.lost_producers.insert(previous_producer.clone());
                if orphan {
                    let closed = &self.closed_producers;
                    self.lost_producers.extend(
                        self.wire_producers
                            .iter()
                            .filter(|wire| !closed.contains(*wire))
                            .cloned(),
                    );
                    self.streams
                        .retain(|k, _| !self.wire_producers.contains(&k.producer));
                    self.connections
                        .retain(|_, owner| !self.wire_producers.contains(owner));
                } else {
                    self.streams.retain(|k, _| k.producer != *previous_producer);
                    self.connections
                        .retain(|_, owner| owner != previous_producer);
                }
                self.wire_producers.insert(producer.clone());
                self.recovered_producers.insert(producer.clone());
            }
            CaptureRecord::ProducerClosed { producer, .. } => {
                self.connections.retain(|_, owner| owner != producer);
                self.closed_producers.insert(producer.clone());
            }
            CaptureRecord::Segment { .. }
            | CaptureRecord::Launch { .. }
            | CaptureRecord::Assistance { .. }
            | CaptureRecord::LocalFinish { .. } => {}
        }
    }
}
