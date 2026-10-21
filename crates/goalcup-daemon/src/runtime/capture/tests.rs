use super::*;
use goalcup_protocol::capture::{MAX_OPEN_CONNECTIONS, TERMINAL_RESERVE_BYTES};
use goalcup_protocol::seal::SealManifest;

fn clock() -> IngestClock {
    crate::ingest::monotonic_clock("c".repeat(32))
}

fn id(n: u8) -> String {
    format!("{n:032x}")
}

/// A quota-only v5 chain with its baseline, in `dir`.
fn chain(dir: &Path, quota: u64) -> ChainWriter {
    let mut export = TraceExport::create(dir).unwrap();
    export.configure_capture_quota(quota).unwrap();
    let baseline = SealManifest {
        entry_id: "e1".to_string(),
        sealed_at: "2026-09-29T00:00:00Z".to_string(),
        workspace: "/w".to_string(),
        files: Vec::new(),
        excluded: Vec::new(),
        git: None,
        home: None,
    };
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &baseline)
        .unwrap();
    ChainWriter::new(export, "e1".to_string())
}

/// A registry row for an entry whose artifacts are in `dir`.
fn entry(dir: &Path) -> Entry {
    Entry::new(
        "e1",
        "test-cli",
        PathBuf::from("/w"),
        dir.to_path_buf(),
        "digest",
        "",
        ExpectedProcessHandoff::Required,
        RunKeys::generate(),
    )
}

fn writer(dir: &Path, quota: u64) -> Arc<Mutex<ChainWriter>> {
    Arc::new(Mutex::new(chain(dir, quota)))
}

fn records(dir: &Path) -> Vec<CaptureRecord> {
    std::fs::read_to_string(dir.join("chain.jsonl"))
        .unwrap()
        .lines()
        .filter_map(
            |line| match serde_json::from_str::<Envelope>(line).unwrap().body {
                EventBody::Capture { record } => Some(record),
                _ => None,
            },
        )
        .collect()
}

fn frame(producer: &str, seq: u64, stream: u64, event: FrameEvent) -> CaptureFrame {
    CaptureFrame {
        stream: StreamKey {
            producer: producer.to_string(),
            stream,
        },
        seq,
        observed: CaptureTime {
            wall: "2026-09-29T00:00:00.000000000Z".to_string(),
            mono_ns: seq,
            clock_id: producer.to_string(),
        },
        event,
    }
}

fn open(producer: &str, seq: u64, stream: u64) -> CaptureFrame {
    let metadata = CaptureMeta {
        wire: None,
        label: format!("native/{stream}.jsonl#g1"),
        protocol_hint: None,
        method: None,
        host: None,
        path: None,
        status: None,
        content_type: None,
    };
    let event = FrameEvent::Open {
        source: CaptureSource::Native,
        metadata,
    };
    frame(producer, seq, stream, event)
}

fn bytes(producer: &str, seq: u64, stream: u64, offset: u64, data: &[u8]) -> CaptureFrame {
    let event = FrameEvent::Bytes {
        offset,
        data: data.to_vec(),
    };
    frame(producer, seq, stream, event)
}

fn end(producer: &str, seq: u64, stream: u64) -> CaptureFrame {
    let event = FrameEvent::End {
        reason: EndReason::Complete,
    };
    frame(producer, seq, stream, event)
}

fn connection(producer: &str, seq: u64, connection: &str, event: ConnectionEvent) -> CaptureFrame {
    frame(
        producer,
        seq,
        0,
        FrameEvent::Connection {
            connection: connection.to_string(),
            event,
        },
    )
}

fn connection_record(
    producer: &str,
    connection: &str,
    event: ConnectionEvent,
    observed: &CaptureTime,
) -> CaptureRecord {
    CaptureRecord::Connection {
        producer: producer.to_string(),
        connection: connection.to_string(),
        observed: observed.clone(),
        event,
    }
}

fn connection_open(already_open: bool) -> ConnectionEvent {
    ConnectionEvent::Open {
        alpn: None,
        already_open,
    }
}

fn connection_close() -> ConnectionEvent {
    ConnectionEvent::Close {
        outcome: goalcup_protocol::capture::ConnectionOutcome::Complete,
    }
}

fn recovery(producer: &str, previous: &str) -> CaptureFrame {
    frame(
        producer,
        0,
        0,
        FrameEvent::Recovery {
            previous_producer: previous.to_string(),
            reason: GapReason::IngestLost,
        },
    )
}

/// A full source chunk of plain text the redactor decides at once.
fn text_chunk() -> Vec<u8> {
    b"lorem ipsum dolor sit\n"
        .iter()
        .copied()
        .cycle()
        .take(MAX_SEGMENT_BYTES)
        .collect()
}

fn gaps(records: &[CaptureRecord], reason: GapReason) -> usize {
    records
        .iter()
        .filter(|r| matches!(r, CaptureRecord::Gap { reason: g, .. } if *g == reason))
        .count()
}

#[test]
fn connection_open_bound_is_entry_wide() {
    let sink = OwnerSink::default();
    let p = id(1);
    let q = id(2);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    assert_eq!(sink.try_begin_epoch(&q), EpochAdmission::Ready);
    let mut seq = [0u64; 2];
    let ids: Vec<_> = (0..=MAX_OPEN_CONNECTIONS)
        .map(|n| format!("{n:032x}"))
        .collect();
    for (n, connection_id) in ids[..MAX_OPEN_CONNECTIONS].iter().enumerate() {
        let owner = n % 2;
        let producer = [&p, &q][owner];
        assert_eq!(
            sink.try_submit(connection(
                producer,
                seq[owner],
                connection_id,
                connection_open(false),
            )),
            SubmitResult::Queued
        );
        seq[owner] += 1;
    }
    assert_eq!(
        sink.try_submit(connection(&p, seq[0], &ids[0], connection_close())),
        SubmitResult::Queued
    );
    seq[0] += 1;
    assert_eq!(
        sink.try_submit(connection(
            &p,
            seq[0],
            &ids[MAX_OPEN_CONNECTIONS],
            connection_open(false),
        )),
        SubmitResult::Queued,
        "a valid Close releases one entry-wide connection slot"
    );
    seq[0] += 1;
    assert_eq!(
        sink.try_submit(connection(
            &p,
            seq[0],
            &format!("{:032x}", MAX_OPEN_CONNECTIONS + 1),
            connection_open(false),
        )),
        SubmitResult::Disabled(GapReason::LimitExceeded),
        "the 129th active connection is refused before insertion"
    );

    let dir = tempfile::tempdir().unwrap();
    let mut writer = chain(dir.path(), 64 << 20);
    let observed = clock()();
    for (n, connection_id) in ids[..MAX_OPEN_CONNECTIONS].iter().enumerate() {
        let producer = [&p, &q][n % 2];
        assert_eq!(
            writer.append(
                connection_record(producer, connection_id, connection_open(false), &observed),
                &observed,
            ),
            Appended::Written
        );
    }
    assert_eq!(
        writer.append(
            connection_record(&p, &ids[0], connection_close(), &observed),
            &observed,
        ),
        Appended::Written
    );
    assert_eq!(
        writer.append(
            connection_record(
                &p,
                &ids[MAX_OPEN_CONNECTIONS],
                connection_open(false),
                &observed,
            ),
            &observed,
        ),
        Appended::Written
    );
    assert_eq!(
        writer.append(
            connection_record(
                &p,
                &format!("{:032x}", MAX_OPEN_CONNECTIONS + 1),
                connection_open(false),
                &observed,
            ),
            &observed,
        ),
        Appended::Discarded
    );
    assert_eq!(writer.connections.len(), MAX_OPEN_CONNECTIONS);
}

#[test]
fn duplicate_connection_open_preserves_owner() {
    let sink = OwnerSink::default();
    let p = id(1);
    let q = id(2);
    let connection_id = id(10);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    assert_eq!(sink.try_begin_epoch(&q), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(connection(&p, 0, &connection_id, connection_open(false))),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_submit(connection(&q, 0, &connection_id, connection_open(false))),
        SubmitResult::Disabled(GapReason::ProducerLost)
    );
    assert_eq!(
        sink.try_submit(connection(
            &p,
            1,
            &connection_id,
            ConnectionEvent::UpgradedWebSocket,
        )),
        SubmitResult::Queued,
        "the refused duplicate does not overwrite the first owner"
    );

    let dir = tempfile::tempdir().unwrap();
    let mut writer = chain(dir.path(), 64 << 20);
    let observed = clock()();
    assert_eq!(
        writer.append(
            connection_record(&p, &connection_id, connection_open(false), &observed),
            &observed,
        ),
        Appended::Written
    );
    assert_eq!(
        writer.append(
            connection_record(&q, &connection_id, connection_open(false), &observed),
            &observed,
        ),
        Appended::Discarded
    );
    assert_eq!(writer.connections.get(&connection_id), Some(&p));
}

#[test]
fn connection_close_releases_only_owner() {
    let sink = OwnerSink::default();
    let p = id(1);
    let q = id(2);
    let fresh = id(3);
    let a = id(10);
    let b = id(11);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    assert_eq!(sink.try_begin_epoch(&q), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(connection(&p, 0, &a, connection_open(false))),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_submit(connection(&q, 0, &b, connection_open(false))),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_submit(connection(&p, 1, &b, connection_close())),
        SubmitResult::Disabled(GapReason::ProducerLost)
    );
    assert_eq!(
        sink.try_submit(connection(&q, 1, &b, connection_close())),
        SubmitResult::Queued,
        "the non-owner's refused Close leaves the owner's context intact"
    );
    assert_eq!(sink.try_begin_epoch(&fresh), EpochAdmission::Ready);
    assert_eq!(sink.try_submit(recovery(&fresh, &p)), SubmitResult::Queued);
    assert_eq!(
        sink.try_submit(connection(&fresh, 1, &a, connection_open(true))),
        SubmitResult::Queued,
        "recovery releases the lost owner's surviving context for re-observation"
    );

    let closed_sink = OwnerSink::default();
    let closed = id(20);
    let after_close = id(21);
    let released = id(22);
    assert_eq!(closed_sink.try_begin_epoch(&closed), EpochAdmission::Ready);
    assert_eq!(
        closed_sink.try_submit(connection(&closed, 0, &released, connection_open(false))),
        SubmitResult::Queued
    );
    assert_eq!(
        closed_sink.try_close_epoch(&closed, Some(0)),
        SubmitResult::Queued
    );
    assert_eq!(
        closed_sink.try_begin_epoch(&after_close),
        EpochAdmission::Ready
    );
    assert_eq!(
        closed_sink.try_submit(connection(
            &after_close,
            0,
            &released,
            connection_open(false),
        )),
        SubmitResult::Queued,
        "ProducerClosed releases only that producer's connection contexts"
    );

    let dir = tempfile::tempdir().unwrap();
    let mut writer = chain(dir.path(), 64 << 20);
    let observed = clock()();
    for (producer, connection_id) in [(&p, &a), (&q, &b)] {
        assert_eq!(
            writer.append(
                connection_record(producer, connection_id, connection_open(false), &observed),
                &observed,
            ),
            Appended::Written
        );
    }
    assert_eq!(
        writer.append(
            connection_record(&p, &b, connection_close(), &observed),
            &observed,
        ),
        Appended::Discarded
    );
    assert_eq!(writer.connections.get(&a), Some(&p));
    assert_eq!(writer.connections.get(&b), Some(&q));
    assert_eq!(
        writer.append(
            connection_record(&q, &b, connection_close(), &observed),
            &observed,
        ),
        Appended::Written
    );
    assert_eq!(writer.connections.get(&a), Some(&p));
    assert!(!writer.connections.contains_key(&b));
}

#[test]
fn connection_history_does_not_grow_after_close() {
    let sink = OwnerSink::default();
    let p = id(1);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    let mut seq = 0;
    for n in 0..(MAX_OPEN_CONNECTIONS * 2) {
        let connection_id = format!("{:032x}", n + 1);
        assert_eq!(
            sink.try_submit(connection(&p, seq, &connection_id, connection_open(false))),
            SubmitResult::Queued
        );
        seq += 1;
        assert_eq!(
            sink.try_submit(connection(&p, seq, &connection_id, connection_close())),
            SubmitResult::Queued
        );
        seq += 1;
    }
    for n in 0..MAX_OPEN_CONNECTIONS {
        assert_eq!(
            sink.try_submit(connection(
                &p,
                seq,
                &format!("{:032x}", 10_000 + n),
                connection_open(false),
            )),
            SubmitResult::Queued,
            "completed connection history must not consume active slots"
        );
        seq += 1;
    }
    assert_eq!(
        sink.try_submit(connection(
            &p,
            seq,
            &format!("{:032x}", 20_000),
            connection_open(false),
        )),
        SubmitResult::Disabled(GapReason::LimitExceeded)
    );

    let dir = tempfile::tempdir().unwrap();
    let mut writer = chain(dir.path(), 64 << 20);
    let observed = clock()();
    for n in 0..(MAX_OPEN_CONNECTIONS * 2) {
        let connection_id = format!("{:032x}", n + 1);
        assert_eq!(
            writer.append(
                connection_record(&p, &connection_id, connection_open(false), &observed),
                &observed,
            ),
            Appended::Written
        );
        assert_eq!(
            writer.append(
                connection_record(&p, &connection_id, connection_close(), &observed),
                &observed,
            ),
            Appended::Written
        );
        assert!(writer.connections.is_empty());
    }
}

#[test]
fn connection_loss_and_recovery_release_only_superseded_contexts() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let p = id(1);
    let q = id(2);
    let recovered = id(3);
    let orphan = id(4);
    let fresh = id(5);
    let a = id(10);
    let b = id(11);
    for producer in [&p, &q] {
        assert_eq!(sink.try_begin_epoch(producer), EpochAdmission::Ready);
    }
    assert_eq!(
        sink.try_submit(connection(&p, 0, &a, connection_open(false))),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_submit(connection(&q, 0, &b, connection_open(false))),
        SubmitResult::Queued
    );
    lose(&mut sink.lock(), &p, GapReason::IngestLost);
    assert_eq!(sink.try_begin_epoch(&recovered), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(recovery(&recovered, &p)),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_submit(connection(&recovered, 1, &a, connection_open(true))),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_submit(connection(&q, 1, &b, ConnectionEvent::UpgradedWebSocket,)),
        SubmitResult::Queued,
        "a nonzero recovery preserves a distinct live producer's socket"
    );
    owner.drain_all();
    assert_eq!(lock(&writer).connections.get(&a), Some(&recovered));
    assert_eq!(lock(&writer).connections.get(&b), Some(&q));

    assert_eq!(sink.try_begin_epoch(&orphan), EpochAdmission::Ready);
    lose(&mut sink.lock(), &orphan, GapReason::IngestLost);
    assert_eq!(sink.try_begin_epoch(&fresh), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(recovery(&fresh, &orphan)),
        SubmitResult::Queued
    );
    owner.drain_all();
    assert!(
        lock(&writer).connections.is_empty(),
        "the CP-A4 zero-record recovery releases every superseded wire context"
    );
    assert_eq!(
        sink.try_submit(connection(&fresh, 1, &a, connection_open(true))),
        SubmitResult::Queued
    );
    assert_eq!(
        sink.try_submit(connection(&fresh, 2, &b, connection_open(true))),
        SubmitResult::Queued,
        "surviving sockets can be re-observed by the fresh epoch"
    );
    owner.drain_all();
    owner.flush_all();
    let written = lock(&writer);
    assert_eq!(written.connections.get(&a), Some(&fresh));
    assert_eq!(written.connections.get(&b), Some(&fresh));
    assert!(written.gaps > 0, "loss remains sticky after recovery");
    drop(written);
    assert!(
        records(dir.path())
            .iter()
            .any(|record| matches!(record, CaptureRecord::Gap { .. })),
        "re-observation never erases the chained loss"
    );
}

#[test]
fn recovery_refuses_a_recorded_non_wire_predecessor() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let native = id(30);
    let refused = id(31);
    let writer_only = id(32);
    let orphan = id(33);
    let recovered = id(34);

    assert_eq!(sink.try_begin_epoch(&native), EpochAdmission::Ready);
    assert_eq!(sink.try_submit(open(&native, 0, 1)), SubmitResult::Queued);
    assert_eq!(sink.try_submit(end(&native, 1, 1)), SubmitResult::Queued);
    lose(&mut sink.lock(), &native, GapReason::IngestLost);
    assert_eq!(sink.try_begin_epoch(&refused), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(recovery(&refused, &native)),
        SubmitResult::Disabled(GapReason::ProducerLost),
        "a recorded non-wire producer is not a recoverable wire epoch"
    );
    owner.drain_all();

    let observed = clock()();
    assert_eq!(
        lock(&writer).append(
            CaptureRecord::Recovery {
                previous_producer: native.clone(),
                producer: writer_only,
                observed: observed.clone(),
                reason: GapReason::IngestLost,
            },
            &observed,
        ),
        Appended::Discarded,
        "written state refuses the same reader-invalid recovery"
    );

    assert_eq!(sink.try_begin_epoch(&orphan), EpochAdmission::Ready);
    lose(&mut sink.lock(), &orphan, GapReason::IngestLost);
    assert_eq!(sink.try_begin_epoch(&recovered), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(recovery(&recovered, &orphan)),
        SubmitResult::Queued,
        "a true zero-record epoch retains CP-A4 recovery"
    );
    owner.drain_all();
    assert!(records(dir.path()).iter().any(
        |record| matches!(record, CaptureRecord::Recovery { producer, .. } if *producer == recovered)
    ));
}

fn loss_frame(producer: &str, seq: u64, reason: GapReason) -> CaptureFrame {
    frame(producer, seq, 0, FrameEvent::Loss { reason })
}

/// H-16: an admitted `Loss` becomes a stream-less gap on the producer's own clock, ahead
/// of that producer's `ProducerClosed`, whose `final_seq` attests it.
#[test]
fn a_loss_is_chained_as_a_producer_clocked_streamless_gap_before_the_close() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let p = id(1);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(loss_frame(&p, 0, GapReason::DecodeFailed)),
        SubmitResult::Queued
    );
    assert_eq!(sink.try_close_epoch(&p, Some(0)), SubmitResult::Queued);
    owner.drain_all();
    owner.flush_all();
    let records = records(dir.path());
    let gap = records
        .iter()
        .position(|r| {
            matches!(
                r,
                CaptureRecord::Gap {
                    stream: None,
                    reason: GapReason::DecodeFailed,
                    lost_bytes: None,
                    observed,
                } if observed.clock_id == p
            )
        })
        .expect("a producer-clocked stream-less gap");
    let closed = records
        .iter()
        .position(|r| {
            matches!(
                r,
                CaptureRecord::ProducerClosed { producer, final_seq: Some(0), .. }
                    if *producer == p
            )
        })
        .expect("the producer's close");
    assert!(gap < closed, "the loss precedes the close that attests it");
    assert_eq!(lock(&writer).gaps, 1);
}

/// H-16: a loss-only epoch is a wire epoch in the sink, so a fresh epoch may name it in a
/// `Recovery`; the writer accepts that recovery because the stream-less gap named no
/// producer and the predecessor is unrecorded (CP-A4). The two rules must stay aligned: a
/// change to either one has to keep this row passing.
#[test]
fn a_loss_only_epoch_is_recoverable() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let p = id(1);
    let q = id(2);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(loss_frame(&p, 0, GapReason::LimitExceeded)),
        SubmitResult::Queued
    );
    lose(&mut sink.lock(), &p, GapReason::IngestLost);
    assert_eq!(sink.try_begin_epoch(&q), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(recovery(&q, &p)),
        SubmitResult::Queued,
        "a loss-only epoch is recoverable, like one that opened a connection"
    );
    owner.drain_all();
    owner.flush_all();
    let records = records(dir.path());
    assert!(
        records.iter().any(|r| matches!(
            r,
            CaptureRecord::Recovery { previous_producer, producer, .. }
                if *previous_producer == p && *producer == q
        )),
        "the writer chained the recovery of a loss-only epoch"
    );
    assert!(
        records.iter().any(|r| matches!(
            r,
            CaptureRecord::Gap { stream: None, reason: GapReason::LimitExceeded, observed, .. }
                if observed.clock_id == p
        )),
        "and the loss it recovers from"
    );
}

/// H-16 / CP-A4: a loss-only epoch records nothing under its own name (a `Loss` chains only
/// as a stream-less gap), so the reader reads a `Recovery` naming it as unrecorded and
/// retires every other wire producer that has not closed. The sink, the receiver and the
/// writer share that one definition: a live second wire producer is superseded with it,
/// nothing later from it is queued, and nothing later from it is written, so the chain is
/// exactly what the reader accepts (valid, incomplete).
#[test]
fn a_recovery_of_a_loss_only_epoch_is_an_orphan_recovery_in_sink_and_writer() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let loss_only = id(1);
    let live = id(2);
    let fresh = id(3);
    let socket = id(10);
    assert_eq!(sink.try_begin_epoch(&loss_only), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(loss_frame(&loss_only, 0, GapReason::LimitExceeded)),
        SubmitResult::Queued
    );
    assert_eq!(sink.try_begin_epoch(&live), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(connection(&live, 0, &socket, connection_open(false))),
        SubmitResult::Queued
    );
    lose(&mut sink.lock(), &loss_only, GapReason::IngestLost);
    assert_eq!(sink.try_begin_epoch(&fresh), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(recovery(&fresh, &loss_only)),
        SubmitResult::Queued,
        "a loss-only epoch is wire-marked and recoverable (H-09)"
    );
    assert_eq!(
        sink.lock().producers[&live].status,
        ProducerStatus::Superseded,
        "the sink retires the other live wire producer, as the reader does"
    );
    assert_eq!(
        sink.try_submit(connection(&live, 1, &socket, connection_close())),
        SubmitResult::Disabled(GapReason::ProducerLost),
        "nothing later from the superseded producer is queued"
    );
    assert_eq!(
        sink.try_close_epoch(&live, Some(0)),
        SubmitResult::Disabled(GapReason::ProducerLost)
    );
    owner.drain_all();
    assert!(
        lock(&writer).connections.is_empty(),
        "the writer released the superseded producer's socket"
    );
    owner.flush_all();
    let chained = records(dir.path());
    let shape: Vec<String> = chained
        .iter()
        .map(|r| match r {
            CaptureRecord::Gap {
                stream: None,
                reason,
                ..
            } => format!("gap {reason:?}"),
            CaptureRecord::Connection { producer, .. } => format!("connection {producer}"),
            CaptureRecord::Recovery {
                previous_producer,
                producer,
                ..
            } => {
                format!("recovery {previous_producer} -> {producer}")
            }
            CaptureRecord::ProducerClosed { producer, .. } => format!("closed {producer}"),
            other => format!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            "gap LimitExceeded".to_string(),
            format!("connection {live}"),
            "gap IngestLost".to_string(),
            "gap ProducerLost".to_string(),
            format!("recovery {loss_only} -> {fresh}"),
        ],
        "no ProducerClosed, and no record of the superseded producer after the Recovery"
    );
}

/// The writer's own rule, whatever the sink queued: a producer a retained Recovery
/// superseded never gets another record, because the reader refuses one
/// (`record_from_lost_producer`). A producer that closed first is not superseded by an
/// orphan recovery.
#[test]
fn the_writer_refuses_every_record_of_a_producer_its_recovery_superseded() {
    let dir = tempfile::tempdir().unwrap();
    let mut chain = chain(dir.path(), 64 << 20);
    let now = clock()();
    let (live, closed, fresh, socket) = (id(2), id(4), id(3), id(10));
    let closed_record = |producer: &str| CaptureRecord::ProducerClosed {
        producer: producer.to_string(),
        observed: now.clone(),
        final_seq: Some(0),
    };
    let write = |chain: &mut ChainWriter, record: CaptureRecord| chain.append(record, &now);
    let open_connection = |producer: &str, connection: &str| {
        connection_record(producer, connection, connection_open(false), &now)
    };
    assert_eq!(
        write(&mut chain, open_connection(&live, &socket)),
        Appended::Written
    );
    assert_eq!(
        write(&mut chain, open_connection(&closed, &id(11))),
        Appended::Written
    );
    assert_eq!(write(&mut chain, closed_record(&closed)), Appended::Written);
    // An orphan recovery: it names a producer the chain never recorded.
    assert_eq!(
        write(
            &mut chain,
            CaptureRecord::Recovery {
                previous_producer: id(1),
                producer: fresh.clone(),
                observed: now.clone(),
                reason: GapReason::IngestLost,
            },
        ),
        Appended::Written
    );
    for record in [
        closed_record(&live),
        open_connection(&live, &id(12)),
        connection_record(&live, &socket, connection_close(), &now),
    ] {
        assert_eq!(
            write(&mut chain, record),
            Appended::Discarded,
            "the superseded live producer is refused"
        );
    }
    assert!(!chain.lost_producers.contains(&closed));
    assert_eq!(
        write(&mut chain, open_connection(&fresh, &id(13))),
        Appended::Written,
        "the fresh epoch is not superseded"
    );
}

/// The writer alone never writes a second recovery of a producer: a `Recovery` naming one an
/// earlier Recovery superseded (`producer_recovered_twice` to the reader), or one that closed
/// (`recovery_after_producer_closed`), is refused, whatever the sink admitted.
#[test]
fn the_writer_refuses_a_second_recovery_of_a_producer() {
    let dir = tempfile::tempdir().unwrap();
    let mut chain = chain(dir.path(), 64 << 20);
    let now = clock()();
    let (wire, closed, never_recorded) = (id(2), id(4), id(1));
    let recovery_record = |previous: &str, producer: String| CaptureRecord::Recovery {
        previous_producer: previous.to_string(),
        producer,
        observed: now.clone(),
        reason: GapReason::IngestLost,
    };
    let open = connection_record(&wire, &id(10), connection_open(false), &now);
    assert_eq!(chain.append(open, &now), Appended::Written);
    let open = connection_record(&closed, &id(11), connection_open(false), &now);
    assert_eq!(chain.append(open, &now), Appended::Written);
    let close = CaptureRecord::ProducerClosed {
        producer: closed.clone(),
        observed: now.clone(),
        final_seq: Some(0),
    };
    assert_eq!(chain.append(close, &now), Appended::Written);
    // An orphan recovery supersedes `never_recorded` and the live wire producer.
    assert_eq!(
        chain.append(recovery_record(&never_recorded, id(5)), &now),
        Appended::Written
    );
    for (previous, why) in [
        (&wire, "a wire producer the orphan recovery superseded"),
        (&never_recorded, "the named predecessor, recovered already"),
        (&closed, "a producer that closed"),
    ] {
        assert_eq!(
            chain.append(recovery_record(previous, id(6)), &now),
            Appended::Discarded,
            "{why}"
        );
    }
    assert_eq!(
        chain.append(recovery_record(&id(5), id(7)), &now),
        Appended::Written,
        "the fresh epoch is still recoverable once"
    );
}

/// H-16: a `Loss` the sink refuses for its queue latches the producer lost, so
/// the close that would attest the stream is refused and no `ProducerClosed` is chained.
#[test]
fn a_refused_loss_leaves_the_producer_unclosable() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let p = id(1);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    // The payload lane is full: the next frame, a loss included, does not fit.
    sink.lock().payload_bytes = PAYLOAD_QUEUE_BYTES;
    assert_eq!(
        sink.try_submit(loss_frame(&p, 0, GapReason::DecodeFailed)),
        SubmitResult::Disabled(GapReason::QueueFull)
    );
    assert_eq!(
        sink.try_close_epoch(&p, None),
        SubmitResult::Disabled(GapReason::QueueFull),
        "the close of a producer whose loss was refused is refused"
    );
    owner.drain_all();
    owner.flush_all();
    let records = records(dir.path());
    assert_eq!(
        gaps(&records, GapReason::QueueFull),
        1,
        "the sink's own gap"
    );
    assert!(
        !records
            .iter()
            .any(|r| matches!(r, CaptureRecord::ProducerClosed { .. })),
        "no clean close was chained"
    );
}

#[test]
fn append_head_is_not_durable_status() {
    let dir = tempfile::tempdir().unwrap();
    let mut chain = chain(dir.path(), 64 << 20);
    chain.sync().unwrap();
    let capture = CaptureEntry::new(entry(dir.path()), Goal::Open, chain, clock());
    let storage = || capture.snapshot(None, false).storage;
    let synced = storage().durable.expect("the baseline is synced");
    let now = clock()();
    let gap = CaptureRecord::Gap {
        stream: None,
        observed: now.clone(),
        reason: GapReason::ShutdownDeadline,
        lost_bytes: None,
    };
    lock(&capture.writer).require(gap, &now).unwrap();
    let appended = storage();
    assert!(appended.used_bytes > synced.chain_bytes);
    assert_eq!(
        appended.durable,
        Some(synced),
        "an appended record is not reported durable until it is synced"
    );
    lock(&capture.writer).sync().unwrap();
    let durable = storage();
    assert_eq!(
        durable.durable.map(|d| d.chain_bytes),
        Some(durable.used_bytes)
    );
}

#[test]
fn a_retried_launch_appends_one_launch_record() {
    let dir = tempfile::tempdir().unwrap();
    let goal = StreamKey {
        producer: id(9),
        stream: 1,
    };
    let chain = chain(dir.path(), 64 << 20);
    let capture = CaptureEntry::new(
        entry(dir.path()),
        Goal::Accepted(goal.clone()),
        chain,
        clock(),
    );
    let request = CaptureLaunchRequest {
        goal_stream: goal,
        spawned_at: clock()(),
        harness_version: "0.153.0".to_string(),
        continuation: "exec".to_string(),
        native_scope: None,
    };
    // The row update after the first append fails, so the row does not say `Started`:
    // the retry marks the row again and appends nothing.
    capture
        .record_launch(&mut capture.lock(), &request, None)
        .unwrap();
    capture
        .record_launch(&mut capture.lock(), &request, None)
        .unwrap();
    let launches = records(dir.path())
        .iter()
        .filter(|r| matches!(r, CaptureRecord::Launch { .. }))
        .count();
    assert_eq!(launches, 1);
}

#[test]
fn full_queue_gap_is_sticky() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let p = id(1);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    assert_eq!(sink.try_submit(open(&p, 0, 1)), SubmitResult::Queued);
    let chunk = text_chunk();
    let mut seq = 1;
    // The owner is not draining: the queue fills and refuses the producer.
    loop {
        let offset = (seq - 1) * MAX_SEGMENT_BYTES as u64;
        match sink.try_submit(bytes(&p, seq, 1, offset, &chunk)) {
            SubmitResult::Queued => seq += 1,
            SubmitResult::Disabled(reason) => break assert_eq!(reason, GapReason::QueueFull),
        }
        assert!(seq < 64, "an 8 MiB queue refuses long before this");
    }
    let later = bytes(&p, seq, 1, 0, b"later");
    assert_eq!(
        sink.try_submit(later),
        SubmitResult::Disabled(GapReason::QueueFull),
        "the refusal is sticky for the producer"
    );
    owner.drain_all();
    // A fresh epoch records normally afterwards, and the loss stays counted.
    let q = id(2);
    assert_eq!(sink.try_begin_epoch(&q), EpochAdmission::Ready);
    sink.try_submit(open(&q, 0, 1));
    sink.try_submit(end(&q, 1, 1));
    sink.try_close_epoch(&q, Some(1));
    owner.drain_all();
    owner.flush_all();
    let records = records(dir.path());
    assert_eq!(gaps(&records, GapReason::QueueFull), 1);
    assert!(
        records
            .iter()
            .any(|r| matches!(r, CaptureRecord::ProducerClosed { producer, .. } if *producer == q))
    );
    assert!(
        !records
            .iter()
            .any(|r| matches!(r, CaptureRecord::ProducerClosed { producer, .. } if *producer == p))
    );
    assert_eq!(lock(&writer).gaps, 1);
}

/// A producer ID past the 255 that `id` spells.
fn wide(n: u32) -> String {
    format!("{n:032x}")
}

#[test]
fn five_hundred_recovered_epochs_are_admitted_and_chained() {
    // Each daemon stall ends an epoch and its recovery is a fresh producer (D58).
    const EPOCHS: u32 = 500;
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let socket = id(200);
    assert_eq!(sink.try_begin_epoch(&wide(1)), EpochAdmission::Ready);
    assert_eq!(
        sink.try_submit(connection(&wide(1), 0, &socket, connection_open(false))),
        SubmitResult::Queued
    );
    for n in 2..=EPOCHS {
        let fresh = wide(n);
        assert_eq!(
            sink.try_begin_epoch(&fresh),
            EpochAdmission::Ready,
            "epoch {n}"
        );
        assert_eq!(
            sink.try_submit(recovery(&fresh, &wide(n - 1))),
            SubmitResult::Queued
        );
        assert_eq!(
            sink.try_submit(connection(&fresh, 1, &socket, connection_open(true))),
            SubmitResult::Queued
        );
        owner.drain_all();
    }
    owner.flush_all();
    let records = records(dir.path());
    let recoveries = records
        .iter()
        .filter(|r| matches!(r, CaptureRecord::Recovery { .. }))
        .count();
    assert_eq!(recoveries, EPOCHS as usize - 1);
    assert_eq!(gaps(&records, GapReason::ProducerLost), EPOCHS as usize - 1);
    assert_eq!(lock(&writer).connections.get(&socket), Some(&wide(EPOCHS)));
}

#[test]
fn a_fresh_epoch_waits_while_max_live_producers_are_live_or_queued() {
    let sink = OwnerSink::default();
    for n in 1..=MAX_LIVE_PRODUCERS as u32 {
        assert_eq!(sink.try_begin_epoch(&wide(n)), EpochAdmission::Ready);
    }
    let over = wide(MAX_LIVE_PRODUCERS as u32 + 1);
    assert_eq!(sink.try_begin_epoch(&over), EpochAdmission::Pending);

    // A close moves one live producer to a queued control: the reserve still counts it.
    assert_eq!(sink.try_close_epoch(&wide(1), None), SubmitResult::Queued);
    assert_eq!(sink.try_begin_epoch(&over), EpochAdmission::Pending);
    assert_eq!(sink.drain(usize::MAX, Duration::ZERO).len(), 1);
    assert_eq!(sink.try_begin_epoch(&over), EpochAdmission::Ready);
    assert_eq!(
        sink.try_begin_epoch(&wide(MAX_LIVE_PRODUCERS as u32 + 2)),
        EpochAdmission::Pending
    );
}

#[test]
fn capacity_exhaustion_uses_bounded_terminal_records() {
    let dir = tempfile::tempdir().unwrap();
    let quota = TERMINAL_RESERVE_BYTES + 200_000;
    let writer = writer(dir.path(), quota);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let p = id(1);
    sink.try_begin_epoch(&p);
    sink.try_submit(open(&p, 0, 1));
    sink.try_submit(open(&p, 1, 2));
    let chunk = text_chunk();
    let mut seq = 2;
    for round in 0..4u64 {
        for stream in [1, 2] {
            let offset = round * MAX_SEGMENT_BYTES as u64;
            sink.try_submit(bytes(&p, seq, stream, offset, &chunk));
            seq += 1;
        }
        owner.drain_all();
        owner.flush_all();
    }
    sink.try_submit(end(&p, seq, 1));
    sink.try_submit(end(&p, seq + 1, 2));
    assert_eq!(
        sink.try_close_epoch(&p, Some(seq + 1)),
        SubmitResult::Queued
    );
    owner.drain_all();
    owner.sync();
    let records = records(dir.path());
    let storage = gaps(&records, GapReason::StorageFull);
    assert!(
        (1..=3).contains(&storage),
        "one for the entry and at most one per open stream: {storage}"
    );
    let ends = records
        .iter()
        .filter(|r| matches!(r, CaptureRecord::End { .. }))
        .count();
    assert_eq!(ends, 2, "closures still spend the terminal allowance");
    assert!(matches!(
        records.last(),
        Some(CaptureRecord::ProducerClosed { .. })
    ));
    let writer = lock(&writer);
    assert!(writer.export.capture_storage().used_bytes <= quota);
    assert!(writer.failure.is_none());
}

#[test]
fn flush_precedes_sync_in_one_tick() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        None,
    );
    let p = id(1);
    sink.try_begin_epoch(&p);
    sink.try_submit(open(&p, 0, 1));
    sink.try_submit(bytes(&p, 1, 1, 0, b"hello\n"));
    owner.drain_all();
    assert!(
        !records(dir.path())
            .iter()
            .any(|r| matches!(r, CaptureRecord::Segment { .. })),
        "the redactor holds the bytes until it is flushed"
    );
    owner.last_flush = Instant::now() - FLUSH_EVERY;
    owner.last_sync = Instant::now() - SYNC_EVERY;
    owner.tick();
    let records = records(dir.path());
    assert!(matches!(
        records.last(),
        Some(CaptureRecord::Segment { .. })
    ));
    let storage = lock(&writer).export.capture_storage();
    assert_eq!(
        storage.durable.map(|d| d.chain_bytes),
        Some(storage.used_bytes),
        "the segment flushed in this tick is inside the frontier synced in it"
    );
}

#[test]
fn owner_exit_is_failure_only_when_undrained() {
    for fault in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let writer = writer(dir.path(), 64 << 20);
        let sink = Arc::new(OwnerSink::default());
        let control = Arc::new(OwnerControl::default());
        control.fault.store(fault, Ordering::Release);
        let p = id(1);
        sink.try_begin_epoch(&p);
        sink.try_submit(open(&p, 0, 1));
        control.stop.store(!fault, Ordering::Release);
        let thread = Owner::new(sink, writer, clock(), &[], Default::default(), None, None)
            .spawn(control.clone())
            .unwrap();
        assert_eq!(thread.join().is_err(), fault);
        assert!(control.exited.load(Ordering::Acquire));
        assert_eq!(control.failed(), fault, "fault injected: {fault}");
        let opened = records(dir.path())
            .iter()
            .any(|r| matches!(r, CaptureRecord::Open { .. }));
        assert_eq!(opened, !fault, "a normal exit drains the queue first");
    }
}

// ----------------------------------------------------------- H-10 4a: the finish budget

fn at(requested: &str, outcome: &str) -> TerminationOutcomeV1 {
    let time = goalcup_protocol::provenance::Rfc3339Utc::try_from(requested)
        .expect("a timestamp this test wrote");
    match outcome {
        "already_stopped" => TerminationOutcomeV1::AlreadyStopped { checked_at: time },
        "confirmed" => TerminationOutcomeV1::Confirmed {
            requested_at: time.clone(),
            confirmed_at: time,
            method: goalcup_protocol::provenance::TerminationMethodV1::ProcessSignal,
        },
        reason => TerminationOutcomeV1::Unresolved {
            requested_at: time.clone(),
            stopped_waiting_at: time,
            reason: match reason {
                "still_running" => TerminationFailureV1::ProcessStillRunning,
                "request_failed" => TerminationFailureV1::RequestFailed,
                _ => TerminationFailureV1::ProcessAuthorityUnavailable,
            },
        },
    }
}

const T: &str = "2026-10-02T00:00:00Z";

/// Run `bind_termination` and say what it bound and whether it terminated.
fn bound(recorded: Option<&str>, fresh: &str) -> (String, bool) {
    let (outcome, ran) = bind_termination(recorded.map(|r| at(T, r)), || {
        Ok::<_, DaemonError>(at(T, fresh))
    })
    .unwrap();
    let name = match outcome {
        TerminationOutcomeV1::AlreadyStopped { .. } => "already_stopped",
        TerminationOutcomeV1::Confirmed { .. } => "confirmed",
        TerminationOutcomeV1::Unresolved {
            reason: TerminationFailureV1::ProcessStillRunning,
            ..
        } => "still_running",
        TerminationOutcomeV1::Unresolved {
            reason: TerminationFailureV1::RequestFailed,
            ..
        } => "request_failed",
        TerminationOutcomeV1::Unresolved { .. } => "unavailable",
    };
    (name.to_string(), ran)
}

#[test]
fn a_recorded_confirmed_outcome_is_reused_and_nothing_is_signalled_again() {
    let mut calls = 0;
    let (outcome, ran) = bind_termination(Some(at(T, "confirmed")), || {
        calls += 1;
        Ok::<_, DaemonError>(at(T, "already_stopped"))
    })
    .unwrap();
    assert!(matches!(outcome, TerminationOutcomeV1::Confirmed { .. }));
    assert!(!ran);
    assert_eq!(calls, 0, "the walk is not run again");
}

#[test]
fn an_inconclusive_recorded_outcome_is_never_reused() {
    // CONTRACTS.md, C6 local finish budget ¶5: a parked, refused or
    // still-running harness may be running now, so the next finish asks again and binds
    // what it finds.
    assert_eq!(
        bound(Some("unavailable"), "confirmed"),
        ("confirmed".into(), true)
    );
    assert_eq!(
        bound(Some("unavailable"), "already_stopped"),
        ("already_stopped".into(), true),
        "nothing was signalled, so an exit found later is an honest AlreadyStopped"
    );
    assert_eq!(
        bound(Some("unavailable"), "unavailable"),
        ("unavailable".into(), true)
    );
    assert_eq!(
        bound(Some("still_running"), "confirmed"),
        ("confirmed".into(), true)
    );
    assert_eq!(
        bound(Some("still_running"), "still_running"),
        ("still_running".into(), true)
    );
    assert_eq!(bound(None, "confirmed"), ("confirmed".into(), true));
    assert_eq!(
        bound(None, "already_stopped"),
        ("already_stopped".into(), true)
    );
}

#[test]
fn an_exit_found_after_an_attempt_that_signalled_keeps_the_recorded_outcome() {
    // The trace never reads "ended on its own" over a harness this daemon signalled.
    for recorded in ["still_running", "request_failed"] {
        assert_eq!(
            bound(Some(recorded), "already_stopped"),
            (recorded.into(), true),
            "{recorded}"
        );
    }
}

#[test]
fn a_termination_failure_is_not_swallowed_by_the_recorded_outcome() {
    let failed = bind_termination(Some(at(T, "unavailable")), || {
        Err::<TerminationOutcomeV1, _>(DaemonError::Refused("ps".into()))
    });
    assert!(failed.is_err());
}

#[test]
fn the_stage_tokens_are_the_wire_messages() {
    for (stage, token) in [
        (FinishStage::NotStarted, "not_started"),
        (FinishStage::BlockingPermit, "blocking_permit"),
        (FinishStage::RunLock, "run_lock"),
        (FinishStage::OwnerDrain, "owner_drain"),
        (FinishStage::Termination, "termination"),
        (FinishStage::WorkspaceWalk, "workspace_walk"),
        (FinishStage::LocalFinish, "local_finish"),
    ] {
        assert_eq!(stage.token(), token);
        assert!(matches!(
            refused_at(stage),
            DaemonError::FinishDeadline { stage } if stage == token
        ));
    }
}

#[test]
fn the_budget_splits_into_a_drain_share_and_a_commit_reserve() {
    let request = |ms| CaptureSealRequest {
        proxy: None,
        remaining_finish_ms: ms,
        native_scope: None,
    };
    // Zero is refused before anything starts.
    assert!(matches!(
        FinishBudget::from_request(&request(Some(0)), false),
        Err(DaemonError::FinishDeadline {
            stage: "not_started"
        })
    ));

    // The full budget of a workspace that is not a clone: a 178 s drain share and a 2 s commit
    // reserve, 1 s of it the margin. No Git read, so no floor.
    let full = FinishBudget::from_request(&request(None), false).unwrap();
    assert_eq!(full.deadline - full.drain_end, FINISH_COMMIT_RESERVE);
    let native = full.native_budget();
    assert!(native <= Duration::from_secs(178) && native > Duration::from_millis(177_500));

    // A clone (D56): the drain share ends 30 s earlier, left for the Git read: 148 s.
    let clone = FinishBudget::from_request(&request(None), true).unwrap();
    assert_eq!(
        clone.deadline - clone.drain_end,
        FINISH_COMMIT_RESERVE + GIT_READ_FLOOR
    );
    let native = clone.native_budget();
    assert!(native <= Duration::from_secs(148) && native > Duration::from_millis(147_500));
    // Less than the floor and the reserve: no drain share, the Git read gets it all.
    let small = FinishBudget::from_request(&request(Some(4_000)), true).unwrap();
    assert_eq!(small.native_budget(), Duration::ZERO);
    let small = FinishBudget::from_request(&request(Some(4_000)), false).unwrap();
    assert!(small.native_budget() > Duration::from_millis(1_500));
    assert!(full.admits());
    assert_eq!(full.last_start() + FINISH_RESPONSE_MARGIN, full.deadline);

    // Less than the reserve left: no drain share at all, and no stage is admitted inside the
    // margin.
    let tight = FinishBudget::from_request(&request(Some(900)), false).unwrap();
    assert_eq!(tight.native_budget(), Duration::ZERO);
    assert!(!tight.admits(), "900 ms is inside the 1 s margin");
    assert!(
        tight.excluding_termination(Duration::from_secs(1)).admits(),
        "termination time is added back to the deadline"
    );

    // Larger than the budget is clamped to it.
    let large = FinishBudget::from_request(&request(Some(u64::MAX)), false).unwrap();
    assert!(
        large.deadline - tokio::time::Instant::now()
            <= goalcup_protocol::control::LOCAL_CAPTURE_FINISH_BUDGET
    );
}

#[test]
fn an_expired_finish_merges_into_what_an_earlier_one_left() {
    let dir = tempfile::tempdir().unwrap();
    let capture = CaptureEntry::new(
        entry(dir.path()),
        Goal::Open,
        chain(dir.path(), 64 << 20),
        clock(),
    );
    let counters = ProxyCounters {
        capture_table_full: 3,
        ..ProxyCounters::default()
    };
    capture.expire(FinishStage::BlockingPermit, Some(counters), None);
    // A later refusal that carries nothing keeps the earlier counters, and gains an outcome.
    capture.expire(FinishStage::WorkspaceWalk, None, Some(at(T, "confirmed")));
    let left = lock(&capture.expired).take().expect("a latch");
    assert_eq!(left.proxy_pending, Some(counters));
    assert!(matches!(
        left.termination,
        Some(TerminationOutcomeV1::Confirmed { .. })
    ));

    // A run-lock refusal is another finish's entry: it latches nothing.
    capture.expire(FinishStage::RunLock, Some(counters), None);
    assert!(lock(&capture.expired).is_none());
}

/// MEASURE, the first freeze gate of H-10 4a: the owner's post-stop work on a full queue
/// (redaction, append, flush, sync) against the commit reserve's roughly one second. The
/// run prints what it measured; the assertion is a loose ceiling so a regression is loud
/// and the number itself is read from the output and recorded in the task report.
#[test]
fn owner_post_stop_drain_fits_commit_reserve() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let control = Arc::new(OwnerControl::default());
    let chunk = text_chunk();
    let p = id(1);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    assert_eq!(sink.try_submit(open(&p, 0, 1)), SubmitResult::Queued);
    // Fill the queue until it refuses: its 8 MiB bound is accounted in encoded bytes, so
    // this is about 2 MiB of source text the redactor decides at once.
    let (mut seq, mut queued) = (1u64, 0usize);
    loop {
        let offset = (seq - 1) * MAX_SEGMENT_BYTES as u64;
        match sink.try_submit(bytes(&p, seq, 1, offset, &chunk)) {
            SubmitResult::Queued => {
                seq += 1;
                queued += chunk.len();
            }
            SubmitResult::Disabled(reason) => break assert_eq!(reason, GapReason::QueueFull),
        }
    }
    control.stop.store(true, Ordering::Release);
    let began = Instant::now();
    let thread = Owner::new(sink, writer, clock(), &[], Default::default(), None, None)
        .spawn(control.clone())
        .unwrap();
    thread.join().unwrap();
    let took = began.elapsed();
    assert!(control.drained.load(Ordering::Acquire));
    let written = std::fs::metadata(dir.path().join("chain.jsonl"))
        .unwrap()
        .len();
    eprintln!(
        "MEASURE owner_post_stop: {queued} source bytes queued, {written} chain bytes \
         written, post-stop {took:?}"
    );
    assert!(
        took < Duration::from_secs(10),
        "post-stop drain took {took:?}"
    );
    // The freeze gate, enforced on the profile that ships: the owner's stop must leave the
    // commit reserve its room. Half the response margin is about 3.6x the measured 138 ms.
    // A debug build is about 25x slower and may refuse at `owner_drain` after a fully spent
    // drain share (CONTRACTS.md, C6 local finish budget ¶10), so it keeps only the loose ceiling above.
    #[cfg(not(debug_assertions))]
    assert!(
        took < FINISH_RESPONSE_MARGIN / 2,
        "release post-stop drain took {took:?}, over half the response margin"
    );
}

/// The frozen connector fixtures (their recipes before chat mode, converted to it), not the
/// shipped ones, so these tests keep their subject when the shipped connectors change.
fn frozen_manifest_paths() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../goalcup-protocol/tests/fixtures/frozen-connectors");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("harness.toml"))
        .collect();
    paths.sort();
    paths
}

/// The shipped connector directories, sorted by folder name.
fn shipped_connector_dirs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|dir| dir.join("harness.toml").is_file())
        .collect();
    dirs.sort();
    dirs
}

/// One shipped connector with a single real home: its directory, manifest text and parsed
/// manifest. Any one serves; these tests are about the loader, not about a harness.
pub(super) fn shipped_connector() -> (PathBuf, String, HarnessManifest) {
    shipped_connector_dirs()
        .into_iter()
        .find_map(|dir| {
            let text = std::fs::read_to_string(dir.join("harness.toml")).unwrap();
            let manifest = HarnessManifest::from_toml_str(&text).ok()?;
            (manifest.capture.home.real_dirs.len() == 1).then_some((dir, text, manifest))
        })
        .expect("a shipped connector with one real home")
}

/// `text` with its one `from` replaced by `to`; the replacement must happen.
pub(super) fn replaced_once(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "{from} is not in the manifest");
    text.replacen(from, to, 1)
}

fn manifest_at(path: &Path) -> HarnessManifest {
    let text = std::fs::read_to_string(path)
        .unwrap()
        .replace("@WORKLOAD@", "/synthetic/workload.py")
        .replace("@CONFIG@", "/synthetic/config.json");
    HarnessManifest::from_toml_str(&text).unwrap()
}

/// The plan copies what each connector declares: its one source's label, hint, form and
/// resolved root. Every frozen fixture, every shipped connector and the controlled workload are
/// read; none is named here.
#[test]
fn schema_native_plan_copies_label_hint_form_and_root() {
    let mut paths = frozen_manifest_paths();
    paths.extend(
        shipped_connector_dirs()
            .into_iter()
            .map(|dir| dir.join("harness.toml")),
    );
    paths.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../goalcup-cli/tests/fixtures/m2-controlled-workload/harness.toml"),
    );
    let mut planned = 0;
    for path in paths {
        let manifest = manifest_at(&path);
        let Some(native) = manifest.capture.native.first() else {
            continue;
        };
        let plan = native_plan(
            &manifest.capture,
            Path::new("/run"),
            Path::new("/config"),
            0,
        )
        .unwrap();
        let at = path.display();
        let [source] = plan.as_slice() else {
            panic!("{at}: one source, got {plan:?}");
        };
        let root = native
            .root
            .replace("{run_home}", "/run")
            .replace("{config_home}", "/config");
        assert_eq!(source.root, Path::new(&root), "{at}");
        assert_eq!(source.scope_id, None, "{at}");
        let form = match native.form {
            goalcup_protocol::harness::NativeForm::AppendOnly => NativeStoreForm::AppendOnly,
            goalcup_protocol::harness::NativeForm::Snapshot => NativeStoreForm::Snapshot,
            other => panic!("{at}: form {other:?}"),
        };
        assert_eq!(
            (source.name.as_str(), source.form),
            (native.name.as_str(), form),
            "{at}"
        );
        assert_eq!(
            source.hint.as_ref().map(NativeHint::as_str),
            native.protocol_hint.as_deref(),
            "{at}"
        );
        planned += 1;
    }
    assert!(planned >= 5, "{planned} planned");
}

#[test]
fn a_since_run_start_source_filters_from_the_run_start() {
    // The frozen Hermes, whose per-run source does not declare it.
    let frozen = frozen_manifest_paths()
        .into_iter()
        .find(|path| {
            !std::fs::read_to_string(path)
                .unwrap()
                .contains("since_run_start")
        })
        .expect("a frozen fixture without since_run_start");
    let text = replaced_once(
        &std::fs::read_to_string(&frozen).unwrap(),
        "form = \"snapshot\"",
        "form = \"snapshot\"\nsince_run_start = true",
    );
    let manifest = HarnessManifest::from_toml_str(&text).unwrap();
    // A start no clock can hold is refused, not a panic.
    let error = native_plan(
        &manifest.capture,
        Path::new("/run"),
        Path::new("/config"),
        u64::MAX,
    )
    .expect_err("an unrepresentable start, no plan");
    assert!(error.to_string().contains("native_since"), "{error}");
    let plan = native_plan(
        &manifest.capture,
        Path::new("/run"),
        Path::new("/config"),
        1_760_000_000,
    )
    .unwrap();
    assert_eq!(
        plan[0].modified_since,
        Some(std::time::UNIX_EPOCH + Duration::from_secs(1_760_000_000))
    );
    // A source that does not declare it ignores the start time.
    let shipped = manifest_at(&frozen);
    let plan = native_plan(
        &shipped.capture,
        Path::new("/run"),
        Path::new("/config"),
        1_760_000_000,
    )
    .unwrap();
    assert_eq!(plan[0].modified_since, None);
    // With no run start in the request, the floor is the row's own create time.
    assert_eq!(
        create_time_floor("2025-10-09T08:53:20Z").unwrap(),
        1_760_000_000
    );
}

#[test]
fn unknown_valid_native_hint_is_copied_byte_for_byte() {
    let frozen = frozen_manifest_paths().remove(0);
    let original = manifest_at(&frozen);
    let hint = original.capture.native[0]
        .protocol_hint
        .as_deref()
        .expect("the fixture's native source declares a hint");
    let text = replaced_once(
        &std::fs::read_to_string(&frozen).unwrap(),
        &format!("protocol_hint = {hint:?}"),
        "protocol_hint = \"future-hint-v7\"",
    );
    let manifest = HarnessManifest::from_toml_str(&text).unwrap();
    let plan = native_plan(
        &manifest.capture,
        Path::new("/run"),
        Path::new("/config"),
        0,
    )
    .unwrap();
    assert_eq!(plan[0].hint.as_ref().unwrap().as_str(), "future-hint-v7");
}

#[test]
fn a_shared_run_home_is_held_to_the_config_home_isolation() {
    use goalcup_protocol::harness::RunHome;
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("config");
    std::fs::create_dir(&config_dir).unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("a test HOME"));
    let real_dirs = ["{user_home}/.example".to_string()];
    // D59: a run `HOME` kept across runs is never the participant's own, nor a real harness
    // home, and the refusal says which.
    for refused in [home.clone(), home.join(".example/sessions")] {
        let error = isolated_run_home(RunHome::Shared, &refused, &real_dirs, &config_dir)
            .expect_err("refused");
        assert!(matches!(error, DaemonError::Refused(_)), "{error}");
        assert!(error.to_string().contains("real_dirs"), "{error}");
    }
    // It lives under `<config_dir>/capture-homes`, nowhere else.
    for refused in [
        temp.path().join("elsewhere/example.run-home"),
        config_dir.join("example.run-home"),
        config_dir.join("capture-homes"),
    ] {
        let error = isolated_run_home(RunHome::Shared, &refused, &real_dirs, &config_dir)
            .expect_err("outside the capture homes");
        assert!(error.to_string().contains("capture-homes"), "{error}");
    }
    let kept = config_dir.join("capture-homes/example.run-home");
    assert_eq!(
        isolated_run_home(RunHome::Shared, &kept, &real_dirs, &config_dir).unwrap(),
        std::fs::canonicalize(&config_dir)
            .unwrap()
            .join("capture-homes/example.run-home")
    );
    // A per-run `HOME` is the launcher's, fresh inside the run directory: not judged here.
    assert_eq!(
        isolated_run_home(RunHome::Empty, &home, &real_dirs, &config_dir).unwrap(),
        home
    );
}

#[test]
fn a_nonshipped_real_dir_is_part_of_the_config_home_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("participant-home");
    std::fs::create_dir(&home).unwrap();
    // The real homes are data: each connector's `real_dirs`, never a list in this crate.
    let shipped_real_dirs = [
        "{user_home}/.first".to_string(),
        "{user_home}/.second".to_string(),
    ];
    for path in [home.join(".first"), home.join(".second")] {
        assert!(matches!(
            isolated_config_home_for(&path, &shipped_real_dirs, &home),
            Err(DaemonError::Refused(_))
        ));
        assert!(isolated_config_home_for(&path, &[], &home).is_ok());
    }
    assert!(matches!(
        isolated_config_home_for(&home, &[], &home),
        Err(DaemonError::Refused(_))
    ));

    let connectors = temp.path().join("connectors");
    std::fs::create_dir(&connectors).unwrap();
    let (_, shipped, manifest) = shipped_connector();
    for (folder, id, real_dir) in [
        ("selected", "selected-dev", ".selected"),
        ("nonshipped", "nonshipped-dev", ".nonshipped"),
    ] {
        let dir = connectors.join(folder);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(
            dir.join("harness.toml"),
            replaced_once(
                &replaced_once(
                    &shipped,
                    &format!("id = {:?}", manifest.id),
                    &format!("id = {id:?}"),
                ),
                &format!("real_dirs = [{:?}]", manifest.capture.home.real_dirs[0]),
                &format!("real_dirs = [\"{{user_home}}/{real_dir}\"]"),
            ),
        )
        .unwrap();
    }
    let config = crate::config::DaemonConfig::from_toml_str(&format!(
        "artifact_root = {artifacts:?}\n\
         connectors_dir = {connectors:?}\n\
         config_dir = {config:?}\n\
         workspace_parent = {workspaces:?}\n\
         control_addr = \"127.0.0.1:0\"\n",
        artifacts = temp.path().join("artifacts"),
        config = temp.path().join("config"),
        workspaces = temp.path().join("workspaces"),
    ))
    .unwrap();
    let (_, _, every_loaded_real_dir) = capture_harness(&config, "selected-dev").unwrap();
    assert_eq!(
        every_loaded_real_dir,
        [
            "{user_home}/.nonshipped".to_string(),
            "{user_home}/.selected".to_string(),
        ]
    );
    assert!(matches!(
        isolated_config_home_for(
            &home.join(".nonshipped/sessions"),
            &every_loaded_real_dir,
            &home,
        ),
        Err(DaemonError::Refused(_))
    ));
}

// ------------------------------------------- D57: native discovery losses reach the chain

fn streamless(records: &[CaptureRecord], reason: GapReason) -> usize {
    records
        .iter()
        .filter(|r| matches!(r, CaptureRecord::Gap { stream: None, reason: g, .. } if *g == reason))
        .count()
}

#[test]
fn a_native_file_past_max_files_is_one_streamless_gap_until_the_count_grows() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        std::fs::write(home.path().join(format!("{name}.jsonl")), b"x\n").unwrap();
    }
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let native = NativeCapture::new(
        NativeCaptureConfig {
            sources: vec![NativeSourceSpec {
                name: "store".into(),
                root: home.path().to_path_buf(),
                pattern: "*.jsonl".into(),
                form: NativeStoreForm::AppendOnly,
                hint: None,
                scope_id: None,
                modified_since: None,
            }],
            limits: NativeCaptureLimits {
                max_files: 2,
                ..NativeCaptureLimits::default()
            },
        },
        sink.clone(),
        Box::new(SystemNativeClock::new()),
    )
    .unwrap();
    let mut owner = Owner::new(
        sink,
        writer.clone(),
        clock(),
        &[],
        Default::default(),
        None,
        Some(native),
    );
    for _ in 0..5 {
        owner.poll_native();
    }
    owner.drain_all();
    assert_eq!(
        streamless(&records(dir.path()), GapReason::LimitExceeded),
        1,
        "one loss, however many polls see it"
    );

    // A second file past the bound grows the count: recorded, at the latest by the finish.
    std::fs::write(home.path().join("d.jsonl"), b"x\n").unwrap();
    owner.poll_native();
    let control = OwnerControl::default();
    control.stop.store(true, Ordering::Release);
    owner.run(&control);
    assert!(control.drained.load(Ordering::Acquire));
    let chain = records(dir.path());
    assert_eq!(streamless(&chain, GapReason::LimitExceeded), 2);
    assert_eq!(
        chain
            .iter()
            .filter(|r| matches!(r, CaptureRecord::Open { .. }))
            .count(),
        2,
        "the tracked files are captured as before"
    );
}

#[test]
fn a_producer_lost_to_a_failed_flush_notes_its_cause() {
    let dir = tempfile::tempdir().unwrap();
    let writer = writer(dir.path(), 64 << 20);
    let sink = Arc::new(OwnerSink::default());
    let broken = Arc::new(AtomicBool::new(false));
    let good = clock();
    let flag = broken.clone();
    let cycles = Arc::new(CycleCounters::default());
    let mut owner = Owner::new(
        sink.clone(),
        writer.clone(),
        Arc::new(move || {
            let mut now = good();
            if flag.load(Ordering::Acquire) {
                now.wall = "not a time".to_string();
            }
            now
        }),
        &[],
        Default::default(),
        None,
        None,
    )
    .noting_to(cycles.clone());
    let p = id(1);
    assert_eq!(sink.try_begin_epoch(&p), EpochAdmission::Ready);
    assert_eq!(sink.try_submit(open(&p, 0, 1)), SubmitResult::Queued);
    owner.drain_all();
    broken.store(true, Ordering::Release);
    owner.flush_all();
    broken.store(false, Ordering::Release);
    let counted = cycles.snapshot();
    assert_eq!(counted.tick, 1);
    let last = counted.last.unwrap();
    assert!(last.contains("a capture producer was lost"), "{last}");
    assert!(last.contains("flush time is not a capture time"), "{last}");
    assert_eq!(
        gaps(&records(dir.path()), GapReason::ProducerLost),
        1,
        "the loss is still chained, as before"
    );
}
