//! The two protocol behaviours the daemon relies on that no `goalcup-protocol` test pins:
//! how the export refuses a stamped seq on an empty chain, and the wire tags of the two
//! bodies only `TraceExport` writes.

use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{ExportError, TraceExport};

#[test]
fn a_stamped_seq_on_an_empty_chain_is_refused_with_nothing_chained() {
    let c = tempfile::tempdir().unwrap();
    let mut w = TraceExport::create(c.path()).unwrap();
    let mut wrong = Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: "main".to_string(),
        recorded_at: "2026-09-04T00:00:00Z".to_string(),
        link: goalcup_protocol::chain::Link {
            seq: 7,
            prev: "aa".repeat(32),
            hash: "bb".repeat(32),
        },
        body: EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
        redaction: None,
    };
    assert!(matches!(
        w.append_chain(&mut wrong),
        Err(ExportError::OutOfOrder {
            found: 7,
            expected_after: None
        })
    ));
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
