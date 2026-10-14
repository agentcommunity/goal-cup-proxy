//! The two protocol behaviours the daemon relies on that no `goalcup-protocol` test pins:
//! how the export refuses a stamped seq that is not the next one, and the wire tags of the two
//! bodies only `TraceExport` writes.

use goalcup_protocol::capture::{
    CaptureRecord, CaptureTime, GapReason, M2_PROTOCOL_VERSION, TERMINAL_RESERVE_BYTES,
};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{ExportError, TraceExport};
use goalcup_protocol::seal::SealManifest;

#[test]
fn a_stamped_seq_that_is_not_the_next_is_refused_with_nothing_chained() {
    let c = tempfile::tempdir().unwrap();
    let mut w = TraceExport::create(c.path()).unwrap();
    w.configure_capture_quota(TERMINAL_RESERVE_BYTES * 4)
        .unwrap();
    w.write_baseline_for_protocol(
        M2_PROTOCOL_VERSION,
        &SealManifest {
            entry_id: "e1".to_string(),
            sealed_at: "2026-09-04T00:00:00Z".to_string(),
            workspace: "/ws".to_string(),
            files: Vec::new(),
            excluded: Vec::new(),
            git: None,
            home: None,
        },
    )
    .unwrap();
    let before = std::fs::read(c.path().join("chain.jsonl")).unwrap();
    let mut wrong = Envelope {
        protocol_version: M2_PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-04T00:00:00Z".to_string(),
        link: goalcup_protocol::chain::Link {
            seq: 7,
            prev: "aa".repeat(32),
            hash: "bb".repeat(32),
        },
        body: EventBody::Capture {
            record: CaptureRecord::Gap {
                stream: None,
                observed: CaptureTime {
                    wall: "2026-09-04T00:00:00Z".to_string(),
                    mono_ns: 1,
                    clock_id: "daemon-5b0c3a1e".to_string(),
                },
                reason: GapReason::StorageIo,
                lost_bytes: None,
            },
        },
        redaction: None,
    };
    assert!(matches!(
        w.append_chain(&mut wrong),
        Err(ExportError::OutOfOrder {
            found: 7,
            expected_after: Some(0)
        })
    ));
    assert_eq!(std::fs::read(c.path().join("chain.jsonl")).unwrap(), before);
}

#[test]
fn baseline_and_seal_carry_their_own_kind_tags() {
    assert!(
        serde_json::to_string(&EventBody::Baseline {
            manifest_sha256: "00".repeat(32),
            file_count: 0,
        })
        .unwrap()
        .contains(r#""kind":"baseline""#)
    );
    assert!(
        serde_json::to_string(&EventBody::Seal {
            manifest_sha256: "00".repeat(32),
            sealed_at: "2026-09-08T00:00:00Z".to_string(),
            file_count: 0,
        })
        .unwrap()
        .contains(r#""kind":"seal""#)
    );
}
