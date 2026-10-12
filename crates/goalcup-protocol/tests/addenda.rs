//! The contract shapes the daemon consumes, defined once here in `goalcup-protocol`. The
//! cursor has no consumer in this crate at all, so the assertions below are the only thing
//! holding it to its meaning.

use goalcup_protocol::cursor::{CursorUnit, StoreCursor};
use goalcup_protocol::envelope::EventBody;

#[test]
fn a_cursor_says_what_its_number_counts_and_only_records_are_comparable() {
    let bytes = StoreCursor {
        store: "main".to_string(),
        unit: CursorUnit::Bytes,
        position: Some(4096),
        describe: "inode:123,offset:4096".to_string(),
    };
    // The whole reason the type exists: 4,096 bytes is not four thousand records.
    assert_eq!(bytes.records(), None);

    let records = StoreCursor {
        store: "event".to_string(),
        unit: CursorUnit::Records,
        position: Some(512),
        describe: "ordinal:512".to_string(),
    };
    assert_eq!(records.records(), Some(512));

    let opaque = StoreCursor {
        store: "raw_bodies".to_string(),
        unit: CursorUnit::Opaque,
        position: None,
        describe: "12 files seen".to_string(),
    };
    assert_eq!(opaque.records(), None);

    // A `Records` cursor with no number is still no number: `records()` reports what the
    // daemon holds, and never invents a zero for a cursor that named none.
    assert_eq!(
        StoreCursor {
            store: "event".to_string(),
            unit: CursorUnit::Records,
            position: None,
            describe: "no rows yet".to_string(),
        }
        .records(),
        None
    );

    // Serialised inside a heartbeat, through the path `events` re-exports.
    let hb = EventBody::Heartbeat {
        ts: "2026-09-04T00:00:00Z".to_string(),
        stores_seen: vec![bytes.clone()],
        receiver_ok: true,
    };
    let text = serde_json::to_string(&hb).unwrap();
    assert!(text.contains("\"unit\":\"bytes\""), "{text}");
    let back: EventBody = serde_json::from_str(&text).unwrap();
    match back {
        EventBody::Heartbeat { stores_seen, .. } => assert_eq!(stores_seen, vec![bytes]),
        other => panic!("{other:?}"),
    }
    let _: goalcup_protocol::events::StoreCursor = records; // the events path resolves
}
