//! The polling tailers (D19): what a poll reads, what it holds back, what it refuses, and
//! what its cursor says afterwards.
//!
//! Two properties here are not "does the code work" but "can a verifier check anything":
//!
//!   * a captured record is the harness's own line, not a `{path,text}` wrapper around it
//!     — with a wrapper, every live Claude and Codex export would parse to zero
//!     sessions, because both parsers read `value.get("type")` straight off the record; and
//!   * every cursor a line-oriented store reports counts records, because `gaps.rs`
//!     compares a cursor's movement against a record count only
//!     under `CursorUnit::Records`, and reporting `Bytes` here would put `cursor_mismatch` — the
//!     Fail that catches records in the chain the daemon never accounted for — out of reach
//!     on every store the shipped connectors declare.
#![cfg(feature = "v1-compat")]

use goalcup_daemon::cursor::CursorState;
use goalcup_daemon::tailer::{DirTailer, JsonlTailer, TailNotice, Tailer};
use goalcup_protocol::cursor::CursorUnit;
use goalcup_protocol::events::TailNoticeKind;

fn append(p: &std::path::Path, lines: &[&str]) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

fn connectors_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

#[test]
fn the_jsonl_tailer_emits_the_harness_record_itself_not_a_wrapper() {
    // A `{path,text}` wrapper would make every live Claude and Codex export parse to
    // zero sessions, because the parsers read `value.get("type")` off the record.
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("main.jsonl");
    append(
        &f,
        &[
            r#"{"type":"user","sessionId":"s1"}"#,
            r#"{"type":"assistant"}"#,
        ],
    );

    let mut tl = JsonlTailer::new("main", t.path().join("*.jsonl").display().to_string(), None);
    let b = tl.poll().unwrap();
    assert_eq!(b.records.len(), 2);
    assert_eq!(b.records[0].value["type"], "user");
    assert_eq!(b.records[0].value["sessionId"], "s1");
    assert!(
        b.records[0].payload.is_string(),
        "a line store ships the raw line as a string"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(b.records[0].payload.as_str().unwrap()).unwrap()
            ["type"],
        "user"
    );
    assert!(b.notices.is_empty());

    // A second poll sees only what was appended since.
    append(&f, &[r#"{"type":"user","sessionId":"s1"}"#]);
    assert_eq!(tl.poll().unwrap().records.len(), 1);
}

#[test]
fn truncation_and_replacement_are_rewrites_and_a_capacity_bound_is_not() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("main.jsonl");
    append(&f, &[r#"{"a":1}"#, r#"{"a":2}"#]);
    let mut tl = JsonlTailer::new("main", f.display().to_string(), None);
    tl.poll().unwrap();

    std::fs::write(&f, b"{\"a\":1}\n").unwrap(); // shrink below the offset
    let b = tl.poll().unwrap();
    let n = b
        .notices
        .iter()
        .find(|n| matches!(n, TailNotice::Truncated { .. }))
        .unwrap();
    assert_eq!(n.kind(), Some(TailNoticeKind::Truncated));
    assert!(
        n.is_rewrite(),
        "a shrink is an on-disk signature of a rewritten transcript"
    );

    // A single line over the per-line cap is a capacity bound. Chaining it as `Truncated`
    // would make V9 report a 1 MiB overrun as `store_rewritten` at Fail.
    let long = format!(
        "{{\"a\":\"{}\"}}",
        "x".repeat((goalcup_daemon::MAX_LINE_BYTES + 8) as usize)
    );
    append(&f, &[&long]);
    let b = tl.poll().unwrap();
    let n = b
        .notices
        .iter()
        .find(|n| matches!(n, TailNotice::OverlongLine { .. }))
        .unwrap();
    assert_eq!(n.kind(), Some(TailNoticeKind::OverlongLine));
    assert!(!n.is_rewrite());
    match n.to_event("2026-09-04T00:00:00Z".to_string()).unwrap() {
        goalcup_protocol::envelope::EventBody::TailNotice {
            kind,
            offset_before,
            offset_after,
            ..
        } => {
            assert_eq!(kind, TailNoticeKind::OverlongLine);
            assert!(
                offset_after > offset_before,
                "the shortfall the chain does not carry"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_cursor_reports_a_number_with_a_unit_and_the_dir_tailer_remembers_what_it_saw() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("a.json"), b"{\"x\":1}").unwrap();
    let mut d = DirTailer::new("raw_bodies", t.path().join("*.json").display().to_string());
    assert_eq!(d.poll().unwrap().records.len(), 1);
    assert!(
        d.poll().unwrap().records.is_empty(),
        "a seen file is not re-captured"
    );

    let c = d.cursor();
    let (pos, unit) = c.position();
    assert_eq!(
        unit,
        CursorUnit::Records,
        "the tailer counts what it emitted"
    );
    assert_eq!(pos, Some(1));
    let sc = c.to_store_cursor("raw_bodies");
    assert_eq!(sc.store, "raw_bodies");
    assert_eq!(
        sc.records(),
        Some(1),
        "or `cursor_mismatch` can never fire on this store"
    );
    assert!(
        sc.describe.contains("mtime"),
        "the mtime high-water mark survives in `describe`"
    );

    // A restart restores the seen-set rather than re-capturing tens of GB.
    let mut d2 = DirTailer::new("raw_bodies", t.path().join("*.json").display().to_string());
    d2.restore(c.clone()).unwrap();
    assert!(d2.poll().unwrap().records.is_empty());

    // The offset cursor reports records too, and keeps its bytes in `describe`. Reporting
    // `Bytes` here would make `StoreCursor::records()` `None` on every store the shipped
    // connectors declare, which would put `gaps.rs`'s `cursor_mismatch` Fail out of reach.
    let io = CursorState::InodeOffset {
        files: [(
            "f".to_string(),
            goalcup_daemon::cursor::FileCursor {
                inode: 1,
                offset: 4096,
            },
        )]
        .into_iter()
        .collect(),
        records: 3,
    };
    assert_eq!(io.position(), (Some(3), CursorUnit::Records));
    assert!(
        io.describe().contains("offset:4096"),
        "the byte offset is reported, not compared"
    );
    assert_eq!(io.to_store_cursor("main").records(), Some(3));

    let rid = CursorState::Rowid {
        last: 512,
        records: 7,
    };
    assert_eq!(rid.position(), (Some(7), CursorUnit::Records));
    assert!(
        rid.describe().contains("rowid:512"),
        "a rowid means nothing outside its own store"
    );

    // The one state with no count, and the one this milestone refuses to build.
    let (_, unit) = CursorState::SessionSeq {
        last: Default::default(),
    }
    .position();
    assert_eq!(unit, CursorUnit::Opaque);
}

#[test]
fn an_unsafe_cursor_and_the_unimplemented_session_seq_are_both_refused_by_name() {
    let mut m = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("codex-cli/manifest.toml")).unwrap(),
    )
    .unwrap();
    // OpenCode, the one shipped connector with a SQLite `event` store on
    // `unsafe_cursors[]`, left in the m2/wire deletion pass; the same declaration is
    // planted on a copy of Codex's manifest.
    m.store.push(support::sqlite_store("event", "event"));
    m.unsafe_cursors = vec!["event.seq".to_string()];
    let mut store = m.store_named("event").unwrap().clone();
    store.cursor = goalcup_protocol::manifest::Cursor {
        kind: goalcup_protocol::manifest::CursorType::SessionSeq,
        column: Some("seq".to_string()),
        group_column: Some("session_id".to_string()),
    };
    let e = goalcup_daemon::tailer::guard_unsafe(&m, &store)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("event.seq"),
        "D19: never through a cursor on unsafe_cursors[]: {e}"
    );

    let vars = goalcup_daemon::paths::PathVars::new(
        "/ws".into(),
        "/home".into(),
        "/cfg".into(),
        "/art".into(),
        None,
        ("2026".into(), "09".into(), "04".into()),
    );
    let mut ok = m.store_named("event").unwrap().clone();
    ok.cursor.kind = goalcup_protocol::manifest::CursorType::SessionSeq;
    // `.err().unwrap()` and not `.unwrap_err()`: `unwrap_err` needs the Ok type to be
    // `Debug`, and `Tailer` is `Send` and nothing more. Making it `Debug` would put a bound on
    // every tailer for the sake of one assertion.
    let e = goalcup_daemon::tailer::tailer_for(&ok, &m, &vars)
        .err()
        .unwrap()
        .to_string();
    assert!(
        e.contains("session_seq") && e.contains("M3"),
        "cut, and it says so: {e}"
    );
    assert!(
        !e.contains("event.seq"),
        "the SessionSeq refusal is by name and comes BEFORE guard_unsafe: this same \
         store is on unsafe_cursors[], and a guard-first order would answer with that \
         message instead and hide the cut"
    );
}

/// A 700 MB rollout, in miniature: one poll reads at most `MAX_POLL_BYTES` from one
/// file, so a file above the cap is drained over several polls rather than held whole.
#[test]
fn a_file_over_the_poll_cap_is_drained_over_several_polls_and_never_read_whole() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("main.jsonl");
    // Each line is 100 bytes including its newline; 30,000 of them is ~2.9 MiB, so the
    // drain takes at least three polls.
    let lines = 30_000usize;
    {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(std::fs::File::create(&f).unwrap());
        for i in 0..lines {
            let body = "y".repeat(80);
            writeln!(w, r#"{{"i":{i:06},"b":"{body}"}}"#).unwrap();
        }
        w.flush().unwrap();
    }
    let total = std::fs::metadata(&f).unwrap().len();
    assert!(total > 2 * goalcup_daemon::MAX_POLL_BYTES);

    let mut tl = JsonlTailer::new("main", f.display().to_string(), None);
    let mut seen = 0usize;
    let mut polls = 0usize;
    let mut last_offset = 0u64;
    loop {
        let b = tl.poll().unwrap();
        polls += 1;
        // Bounded: no poll may take more than the cap out of one file.
        let now = offset_of(&tl.cursor());
        assert!(
            now - last_offset <= goalcup_daemon::MAX_POLL_BYTES,
            "poll {polls} advanced {} bytes",
            now - last_offset
        );
        last_offset = now;
        seen += b.records.len();
        assert!(
            b.notices.is_empty(),
            "hitting the per-POLL cap sets `more_available` and chains no notice: nothing \
             went wrong, and a notice here would put a capacity bound in the chain"
        );
        if !b.more_available {
            break;
        }
        assert!(polls < 64, "the drain is not making progress");
    }
    assert!(polls >= 3, "a {total}-byte file drained in {polls} poll(s)");
    assert_eq!(seen, lines);
    assert_eq!(last_offset, total, "the cursor ends at the end of the file");
    assert!(tl.poll().unwrap().records.is_empty());
}

fn offset_of(c: &CursorState) -> u64 {
    match c {
        CursorState::InodeOffset { files, .. } => files.values().map(|f| f.offset).sum(),
        other => panic!("{other:?}"),
    }
}

/// A record is a whole line or it is nothing: the tail of a file the harness is still
/// writing is held back until its newline arrives, and it is then emitted intact.
#[test]
fn a_partial_trailing_line_is_held_back_until_its_newline_arrives() {
    use std::io::Write;
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("main.jsonl");
    let mut w = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&f)
        .unwrap();
    write!(w, "{{\"type\":\"user\",\"n\":1}}\n{{\"type\":\"assi").unwrap();
    w.flush().unwrap();

    let mut tl = JsonlTailer::new("main", f.display().to_string(), None);
    let b = tl.poll().unwrap();
    assert_eq!(b.records.len(), 1, "the half-written line is not a record");
    assert!(b.notices.is_empty(), "an unfinished line is not a notice");
    let held = offset_of(&tl.cursor());

    writeln!(w, "stant\",\"n\":2}}").unwrap();
    w.flush().unwrap();
    let b = tl.poll().unwrap();
    assert_eq!(b.records.len(), 1);
    assert_eq!(
        b.records[0].value["type"], "assistant",
        "the held-back half is emitted whole, not as two records"
    );
    assert_eq!(b.records[0].value["n"], 2);
    assert!(offset_of(&tl.cursor()) > held);
}

/// A rotation the harness performs by writing a new file at the same path is a replacement
/// and not a truncation: the inode changed, and V9 reads the two differently.
#[test]
fn a_new_inode_at_the_same_path_is_reported_as_file_replaced() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("main.jsonl");
    append(&f, &[r#"{"a":1}"#, r#"{"a":2}"#]);
    let mut tl = JsonlTailer::new("main", f.display().to_string(), None);
    assert_eq!(tl.poll().unwrap().records.len(), 2);

    // Rotate: a fresh inode at the same path, longer than the old file, so a size
    // comparison alone would see nothing.
    let other = t.path().join("rotated.jsonl");
    append(&other, &[r#"{"a":10}"#, r#"{"a":11}"#, r#"{"a":12}"#]);
    std::fs::rename(&other, &f).unwrap();

    let b = tl.poll().unwrap();
    let n = b
        .notices
        .iter()
        .find(|n| matches!(n, TailNotice::FileReplaced { .. }))
        .unwrap();
    assert_eq!(n.kind(), Some(TailNoticeKind::FileReplaced));
    assert!(n.is_rewrite());
    match n {
        TailNotice::FileReplaced {
            inode_before,
            inode_after,
            offset_before,
            ..
        } => {
            assert!(inode_before.is_some() && inode_after.is_some());
            assert_ne!(inode_before, inode_after);
            assert!(*offset_before > 0);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        b.notices
            .iter()
            .any(|n| matches!(n, TailNotice::Reread { .. })),
        "the re-read from zero is stated, so V9 has the witness the rule asks for"
    );
    assert_eq!(b.records.len(), 3, "the replacement file is read from zero");
}

/// A restart restores the byte position exactly: the same file, the same inode, the same
/// offset, and no record already chained is chained twice.
#[test]
fn a_jsonl_cursor_is_byte_stable_across_a_restart() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("main.jsonl");
    append(&f, &[r#"{"a":1}"#, r#"{"a":2}"#]);
    let mut tl = JsonlTailer::new("main", f.display().to_string(), None);
    assert_eq!(tl.poll().unwrap().records.len(), 2);
    let saved = tl.cursor();

    let mut tl2 = JsonlTailer::new("main", f.display().to_string(), None);
    tl2.restore(saved.clone()).unwrap();
    assert_eq!(tl2.cursor(), saved, "restore is byte-for-byte");
    let b = tl2.poll().unwrap();
    assert!(b.records.is_empty(), "nothing already read is read again");
    assert!(b.notices.is_empty());

    append(&f, &[r#"{"a":3}"#]);
    assert_eq!(tl2.poll().unwrap().records.len(), 1);
}

/// The ordinal cursor is the offset cursor with a second key: bytes are read by inode and
/// offset, and a record whose ordinal was already emitted is dropped, so a re-read of a
/// file the tailer had already passed is idempotent per file (D19).
#[test]
fn an_ordinal_cursor_drops_a_record_it_already_emitted() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("rollout-a.jsonl");
    append(
        &f,
        &[
            r#"{"payload":{"ordinal":1},"type":"a"}"#,
            r#"{"payload":{"ordinal":2},"type":"b"}"#,
        ],
    );
    let mut tl = JsonlTailer::new(
        "rollout",
        f.display().to_string(),
        Some("payload.ordinal".to_string()),
    );
    let b = tl.poll().unwrap();
    assert_eq!(b.records.len(), 2);
    let c = tl.cursor();
    assert!(matches!(c, CursorState::Ordinal { .. }));
    assert_eq!(c.position(), (Some(2), CursorUnit::Records));

    // A truncation-free rewrite that re-appends the same two records and one new one: the
    // tailer re-reads from zero (the file shrank first) and drops what it already emitted.
    std::fs::write(&f, b"").unwrap();
    tl.poll().unwrap();
    append(
        &f,
        &[
            r#"{"payload":{"ordinal":1},"type":"a"}"#,
            r#"{"payload":{"ordinal":2},"type":"b"}"#,
            r#"{"payload":{"ordinal":3},"type":"c"}"#,
        ],
    );
    let b = tl.poll().unwrap();
    assert_eq!(b.records.len(), 1, "only the ordinal it had not passed");
    assert_eq!(b.records[0].value["type"], "c");
}

/// A whole-file capture above `MAX_FILE_BYTES` is an `OversizeFile` — a capacity bound with
/// a producer — and never a `Truncated`, which is a rewrite signature.
#[test]
fn a_whole_file_over_the_file_cap_is_oversize_and_is_not_a_rewrite() {
    let t = tempfile::tempdir().unwrap();
    let big = t.path().join("body.json");
    let len = goalcup_daemon::MAX_FILE_BYTES + 4096;
    std::fs::write(&big, vec![b'z'; len as usize]).unwrap();

    let mut d = DirTailer::new("raw_bodies", t.path().join("*.json").display().to_string());
    let b = d.poll().unwrap();
    assert_eq!(b.records.len(), 1, "what fits still reaches the chain");
    assert_eq!(
        b.records[0].bytes.len() as u64,
        goalcup_daemon::MAX_FILE_BYTES
    );
    let n = b
        .notices
        .iter()
        .find(|n| matches!(n, TailNotice::OversizeFile { .. }))
        .unwrap();
    assert_eq!(n.kind(), Some(TailNoticeKind::OversizeFile));
    assert!(!n.is_rewrite(), "a capacity bound is not a rewritten store");
    assert!(
        !b.notices
            .iter()
            .any(|n| matches!(n, TailNotice::Truncated { .. })),
        "never Truncated: that is the signature of attack 6b"
    );
    match n.to_event("2026-09-04T00:00:00Z".to_string()).unwrap() {
        goalcup_protocol::envelope::EventBody::TailNotice {
            kind,
            offset_before,
            offset_after,
            ..
        } => {
            assert_eq!(kind, TailNoticeKind::OversizeFile);
            assert_eq!(offset_before, goalcup_daemon::MAX_FILE_BYTES);
            assert_eq!(offset_after, len, "the shortfall the chain does not carry");
        }
        other => panic!("{other:?}"),
    }
}

/// Stated over the shipped manifests rather than over an example: every
/// store the shipped connectors declare reports a cursor whose number is a record count,
/// so `gaps::cursor_mismatch` is reachable on all of them.
#[test]
fn every_store_the_three_connectors_declare_reports_a_records_cursor() {
    for id in ["claude-code-cli", "codex-cli"] {
        let m = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
            &std::fs::read_to_string(connectors_dir().join(id).join("manifest.toml")).unwrap(),
        )
        .unwrap();
        assert!(!m.store.is_empty(), "{id} declares no store");
        for s in &m.store {
            let c = CursorState::initial(s.cursor.kind);
            assert_eq!(c.kind(), s.cursor.kind, "{id}/{}", s.name);
            let sc = c.to_store_cursor(&s.name);
            assert_eq!(sc.unit, CursorUnit::Records, "{id}/{}", s.name);
            assert_eq!(sc.records(), Some(0), "{id}/{}", s.name);
        }
    }
}

/// The dotted-field reader the ordinal cursor and the partitioner share.
#[test]
fn a_dotted_path_reads_a_nested_field_and_nothing_else() {
    use goalcup_daemon::tailer::dotted_json;
    let v: serde_json::Value =
        serde_json::from_str(r#"{"payload":{"ordinal":7,"cwd":"/ws"},"type":"x"}"#).unwrap();
    assert_eq!(dotted_json(&v, "payload.ordinal").unwrap(), &7);
    assert_eq!(dotted_json(&v, "type").unwrap(), "x");
    assert!(dotted_json(&v, "payload.missing").is_none());
    assert!(dotted_json(&v, "type.ordinal").is_none());
    assert!(dotted_json(&v, "").is_none());
}

/// A new inode is a new ordinal space. The ordinal drops a record this tailer already
/// emitted from the file it read; a replacement file at the same path is a different file,
/// and keeping the old high-water mark across it silently drops every record the new file
/// numbered from one — the exact shape of a session the chain never sees.
#[test]
fn a_replaced_file_starts_a_new_ordinal_space() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("rollout-a.jsonl");
    append(
        &f,
        &[
            r#"{"payload":{"ordinal":1},"type":"a"}"#,
            r#"{"payload":{"ordinal":2},"type":"b"}"#,
            r#"{"payload":{"ordinal":3},"type":"c"}"#,
            r#"{"payload":{"ordinal":4},"type":"d"}"#,
            r#"{"payload":{"ordinal":5},"type":"e"}"#,
        ],
    );
    let mut tl = JsonlTailer::new(
        "rollout",
        f.display().to_string(),
        Some("payload.ordinal".to_string()),
    );
    assert_eq!(tl.poll().unwrap().records.len(), 5);

    // Rotate: a fresh inode at the same path, numbering its own records from one.
    let other = t.path().join("rollout-b.jsonl");
    append(
        &other,
        &[
            r#"{"payload":{"ordinal":1},"type":"x"}"#,
            r#"{"payload":{"ordinal":2},"type":"y"}"#,
            r#"{"payload":{"ordinal":3},"type":"z"}"#,
        ],
    );
    std::fs::rename(&other, &f).unwrap();

    let b = tl.poll().unwrap();
    assert!(
        b.notices
            .iter()
            .any(|n| matches!(n, TailNotice::FileReplaced { .. })),
        "the new inode is reported as a replacement"
    );
    assert!(
        b.notices
            .iter()
            .any(|n| matches!(n, TailNotice::Reread { .. })),
        "and the re-read from zero is stated after it"
    );
    assert_eq!(
        b.records.len(),
        3,
        "every record of the replacement file reaches the chain, ordinals 1..3 included"
    );
    let types: Vec<&str> = b
        .records
        .iter()
        .map(|r| r.value["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["x", "y", "z"]);

    // The high-water mark is the new file's, so the new space still drops what it emitted.
    let b = tl.poll().unwrap();
    assert!(b.records.is_empty());
    append(&f, &[r#"{"payload":{"ordinal":2},"type":"y"}"#]);
    assert!(
        tl.poll().unwrap().records.is_empty(),
        "an ordinal already emitted from THIS file is still dropped"
    );
}

/// The two line-cap checks agree to the byte. A whole line is overlong above the cap
/// (`line.len() > MAX_LINE_BYTES`), so a partial line of exactly the cap is not yet
/// overlong — it is a line still being written, and it is held back like any other.
#[test]
fn a_partial_line_of_exactly_the_line_cap_is_held_back_not_discarded() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("main.jsonl");
    std::fs::write(&f, vec![b'x'; goalcup_daemon::MAX_LINE_BYTES as usize]).unwrap();

    let mut tl = JsonlTailer::new("main", f.display().to_string(), None);
    let b = tl.poll().unwrap();
    assert!(b.records.is_empty(), "no newline yet, so no record");
    assert!(
        b.notices.is_empty(),
        "exactly the cap is not over it: no OverlongLine is chained"
    );
    assert!(!b.more_available);
    assert_eq!(
        offset_of(&tl.cursor()),
        0,
        "the offset stops at the partial line's first byte"
    );
}

/// The `Store` the SQLite cases read, built once rather than as a fourteen-field literal
/// per test. `name` decides the payload shape; `table` is the table the tailer selects
/// from. The cursor is the one OpenCode's manifest declares for that store: `rowid` for
/// `event`, `sqlite_pk` on the TEXT `id` for `message` and `part`.
mod support {
    use goalcup_protocol::manifest::{Cursor, CursorType, Store, StoreMode};

    pub fn sqlite_store(name: &str, table: &str) -> Store {
        let (kind, column) = match name {
            "message" | "part" => (CursorType::SqlitePk, "id"),
            _ => (CursorType::Rowid, "rowid"),
        };
        Store {
            name: name.to_string(),
            mode: StoreMode::Sqlite,
            path_template: "{config_dir}/data/opencode/opencode.db".to_string(),
            cursor: Cursor {
                kind,
                column: Some(column.to_string()),
                group_column: None,
            },
            table: Some(table.to_string()),
            partition_key: Some("session.directory".to_string()),
            partition: None,
            unpartitioned_reason: None,
            subagent_path_template: None,
            spill_path_template: None,
        }
    }
}

/// A WAL database with `rows` event rows, left uncheckpointed: the write connection is
/// returned alongside the path and the caller holds it open, so the rows live in the `-wal`
/// file and nowhere else. An `immutable=1` open would be told to ignore that file and would
/// read zero rows, which is why the tailer does not use one.
fn wal_db(dir: &std::path::Path, rows: usize) -> (std::path::PathBuf, rusqlite::Connection) {
    let p = dir.join("opencode.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.pragma_update(None, "journal_mode", "WAL").unwrap();
    c.execute(
        "CREATE TABLE event (id TEXT, type TEXT, session_id TEXT, blob BLOB)",
        [],
    )
    .unwrap();
    for i in 0..rows {
        c.execute(
            "INSERT INTO event VALUES (?1, 'session.created.1', 's1', ?2)",
            rusqlite::params![format!("evt_{i}"), vec![0xffu8, 0x00]],
        )
        .unwrap();
    }
    (p, c)
}

#[test]
fn a_bounded_poll_pages_in_sql_and_never_stalls_above_the_row_cap() {
    let t = tempfile::tempdir().unwrap();
    let (db, _held) = wal_db(t.path(), goalcup_daemon::MAX_POLL_ROWS + 88);
    let store = support::sqlite_store("event", "event");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, db.display().to_string())
            .unwrap();
    assert_eq!(tl.table(), "event");

    let first = tl.poll().unwrap();
    assert_eq!(first.records.len(), goalcup_daemon::MAX_POLL_ROWS);
    assert!(first.more_available);
    let second = tl.poll().unwrap();
    assert_eq!(
        second.records.len(),
        88,
        "the second page is the remainder, not the same page"
    );
    assert!(tl.poll().unwrap().records.is_empty());

    // No rowid twice: a query with no `WHERE` clause that skipped seen rows in
    // Rust would fetch the same page forever above `MAX_POLL_ROWS + 1` rows.
    let mut rids: Vec<i64> = first
        .records
        .iter()
        .chain(second.records.iter())
        .map(|r| r.payload["row"]["_rid"].as_i64().unwrap())
        .collect();
    let before = rids.len();
    rids.sort_unstable();
    rids.dedup();
    assert_eq!(rids.len(), before, "a rowid was emitted twice");

    // The row shape the verifier reads. An `event` row is `{db, table, row}`, because
    // `parsers/opencode.rs` dispatches on `row.type`.
    let r = &first.records[0];
    assert!(r.payload.is_object());
    assert_eq!(r.payload["table"], "event");
    assert_eq!(r.payload["db"], db.display().to_string());
    assert!(r.payload["row"]["_rid"].is_i64());
    assert!(
        r.payload["row"]["blob"].is_string(),
        "a blob column is base64, not lost"
    );
    assert_eq!(
        r.payload["row"]["blob"], "/wA=",
        "base64 of the two bytes 0xff 0x00, not a lossy UTF-8 rendering"
    );
    assert_eq!(
        r.value["type"], "session.created.1",
        "the partitioner reads the row directly"
    );
    assert_eq!(r.source_path, db.display().to_string());
    assert_eq!(r.store, "event");
    // `bytes` are the text the payload carries — for an envelope store, the envelope.
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&r.bytes).unwrap()["row"]["type"],
        "session.created.1"
    );

    // The connection string, asserted where it bites. The write connection is still open,
    // so every one of these rows lives in the uncheckpointed `-wal` sidecar and nowhere
    // else. `mode=ro` reads them; `immutable=1` is told to ignore that sidecar and cannot
    // so much as see the table — which is why the tailer does not use one, and why a
    // fixture that had been checkpointed first would have let the wrong flag pass.
    assert!(
        std::fs::metadata(db.with_extension("db-wal"))
            .map(|m| m.len() > 0)
            .unwrap_or(false),
        "the fixture is not actually uncheckpointed, so this asserts nothing"
    );
    let immutable = rusqlite::Connection::open_with_flags(
        format!("file:{}?mode=ro&immutable=1", db.display()),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();
    assert!(
        immutable
            .query_row("SELECT count(*) FROM event", [], |r| r.get::<_, i64>(0))
            .is_err(),
        "an `immutable=1` open misses every uncheckpointed row"
    );

    let c = tl.cursor();
    assert_eq!(
        c.position(),
        (
            Some((goalcup_daemon::MAX_POLL_ROWS + 88) as u64),
            CursorUnit::Records
        )
    );
    assert!(
        c.describe()
            .contains(&format!("rowid:{}", goalcup_daemon::MAX_POLL_ROWS + 88))
    );
}

#[test]
fn a_message_or_part_row_ships_flat_because_that_is_what_the_parser_reads() {
    // The tailer emits what the parser reads, per store.
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("opencode.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute(
        "CREATE TABLE part (id TEXT, message_id TEXT, type TEXT, text TEXT)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO part VALUES ('prt_1', 'msg_1', 'text', 'hello')",
        [],
    )
    .unwrap();
    let store = support::sqlite_store("part", "part");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, p.display().to_string()).unwrap();

    assert_eq!(
        goalcup_daemon::tailer::sqlite::payload_shape(&store),
        goalcup_daemon::tailer::sqlite::PayloadShape::Flat
    );
    let r = &tl.poll().unwrap().records[0];
    assert_eq!(
        r.payload["message_id"], "msg_1",
        "top-level, or the parts index is empty"
    );
    assert!(
        r.payload.get("row").is_none(),
        "no {{db,table,row}} wrapper on a part row"
    );
    assert!(
        r.payload["_rid"].is_i64(),
        "the paging key rides along at the top level"
    );
    assert_eq!(r.value["message_id"], "msg_1");

    // And the `event` store is the other shape, in the same connector, from the same tailer.
    assert_eq!(
        goalcup_daemon::tailer::sqlite::payload_shape(&support::sqlite_store("event", "event")),
        goalcup_daemon::tailer::sqlite::PayloadShape::Envelope
    );
    // All three stores the shape rule names, and not just the one this test writes rows
    // for: `session` and `message` are the two whose predicates (`parent_id` + `directory`,
    // and `id` + `role`) fail silently under a wrapper — an empty parts index at least
    // leaves the messages, a wrapped `session` leaves nothing to attach them to.
    for name in ["session", "message"] {
        assert_eq!(
            goalcup_daemon::tailer::sqlite::payload_shape(&support::sqlite_store(name, name)),
            goalcup_daemon::tailer::sqlite::PayloadShape::Flat,
            "`{name}` rows are read at the top level"
        );
    }

    // `part` declares `sqlite_pk` on a TEXT `id`, so the declared column cannot be the
    // number. The cursor falls back to the rowid and says `sqlite_pk`, so a
    // restart resumes through `restore`'s own type check.
    let c = tl.cursor();
    assert_eq!(c.kind(), goalcup_protocol::manifest::CursorType::SqlitePk);
    assert_eq!(c.position(), (Some(1), CursorUnit::Records));
    assert!(c.describe().contains("id:1"));
}

#[test]
fn a_missing_database_is_a_notice_and_not_an_error() {
    let t = tempfile::tempdir().unwrap();
    let store = support::sqlite_store("event", "event");
    let mut tl = goalcup_daemon::tailer::sqlite::SqliteTailer::new(
        &store,
        t.path().join("not-yet.db").display().to_string(),
    )
    .unwrap();
    let b = tl
        .poll()
        .expect("the store does not exist until the harness starts");
    assert!(b.records.is_empty());
    assert!(matches!(b.notices[0], TailNotice::StoreUnavailable { .. }));
    assert!(
        b.notices[0].kind().is_none(),
        "not an on-disk rewrite signature"
    );
    assert!(
        b.notices[0].path().contains("not-yet.db"),
        "every sqlite error carries the path"
    );
    assert!(
        b.notices[0]
            .to_event("2026-09-04T00:00:00Z".to_string())
            .is_none()
    );

    // And a table the harness has not created yet is the same fact about the same store.
    let p = t.path().join("not-yet.db");
    let _c = rusqlite::Connection::open(&p).unwrap();
    let b = tl.poll().unwrap();
    assert!(b.records.is_empty());
    assert!(matches!(b.notices[0], TailNotice::StoreUnavailable { .. }));
}

/// Bounded polling in the row dimension: a table far above the cap drains over many polls, the
/// cursor advances every time, and no row is emitted twice.
#[test]
fn a_two_thousand_row_table_drains_over_at_least_four_polls_with_the_cursor_advancing() {
    let t = tempfile::tempdir().unwrap();
    let (db, _held) = wal_db(t.path(), 2000);
    let store = support::sqlite_store("event", "event");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, db.display().to_string())
            .unwrap();

    let mut seen: Vec<i64> = Vec::new();
    let mut polls = 0usize;
    let mut last = rowid_of(&tl.cursor());
    loop {
        let b = tl.poll().unwrap();
        polls += 1;
        assert!(
            b.records.len() <= goalcup_daemon::MAX_POLL_ROWS,
            "bounded memory: no poll reads a whole table"
        );
        for r in &b.records {
            seen.push(r.payload["row"]["_rid"].as_i64().unwrap());
        }
        let now = rowid_of(&tl.cursor());
        assert!(now > last, "poll {polls} did not advance the cursor");
        last = now;
        if !b.more_available {
            break;
        }
        assert!(polls < 64, "the drain is not making progress");
    }
    assert!(polls >= 4, "2000 rows drained in {polls} poll(s)");
    assert_eq!(seen.len(), 2000);
    let mut dedup = seen.clone();
    dedup.sort_unstable();
    dedup.dedup();
    assert_eq!(dedup.len(), 2000, "a row was emitted twice");
}

fn rowid_of(c: &CursorState) -> i64 {
    match c {
        CursorState::Rowid { last, .. } | CursorState::SqlitePk { last, .. } => *last,
        other => panic!("{other:?}"),
    }
}

/// A restart resumes from the persisted cursor rather than re-shipping the table, and the
/// tailer never opens the harness's database for writing.
#[test]
fn a_restart_continues_from_the_persisted_cursor_and_the_db_is_never_opened_writable() {
    let t = tempfile::tempdir().unwrap();
    let (db, held) = wal_db(t.path(), 3);
    let store = support::sqlite_store("event", "event");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, db.display().to_string())
            .unwrap();
    assert_eq!(tl.poll().unwrap().records.len(), 3);
    let saved = tl.cursor();

    let mut tl2 =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, db.display().to_string())
            .unwrap();
    tl2.restore(saved.clone()).unwrap();
    assert!(
        tl2.poll().unwrap().records.is_empty(),
        "a restored cursor does not re-ship the table"
    );
    held.execute(
        "INSERT INTO event VALUES ('evt_new', 'session.created.1', 's1', NULL)",
        [],
    )
    .unwrap();
    assert_eq!(tl2.poll().unwrap().records.len(), 1);

    // A cursor from another tailer is refused by type rather than silently read as a rowid.
    let e = tl2
        .restore(CursorState::SeenFiles {
            seen: Default::default(),
            records: 0,
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("event"), "the refusal names the store: {e}");

    // Read-only, asserted where it counts: the tailer's own connection cannot write.
    let store2 = support::sqlite_store("event", "event");
    let tl3 = goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store2, db.display().to_string())
        .unwrap();
    drop(tl3);
    // The harness still holds the database open and is still able to write to it: the
    // tailer's presence never takes the write lock away from the process it is watching.
    held.execute(
        "INSERT INTO event VALUES ('evt_after', 'session.created.1', 's1', NULL)",
        [],
    )
    .unwrap();
}

/// WAL: a writer inside an open transaction does not make the poll error out, and the
/// poll reads what was committed and nothing that was not.
#[test]
fn a_concurrent_writer_does_not_make_the_poll_fail() {
    let t = tempfile::tempdir().unwrap();
    let (db, held) = wal_db(t.path(), 2);
    let store = support::sqlite_store("event", "event");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, db.display().to_string())
            .unwrap();
    assert_eq!(tl.poll().unwrap().records.len(), 2);

    let mut writer = rusqlite::Connection::open(&db).unwrap();
    let tx = writer.transaction().unwrap();
    tx.execute(
        "INSERT INTO event VALUES ('evt_open', 'session.created.1', 's1', NULL)",
        [],
    )
    .unwrap();
    let b = tl
        .poll()
        .expect("a held write transaction is not a poll failure");
    assert!(b.notices.is_empty(), "{:?}", b.notices);
    assert!(b.records.is_empty(), "an uncommitted row is not a record");
    tx.commit().unwrap();
    assert_eq!(tl.poll().unwrap().records.len(), 1);
    drop(held);
}

/// A row inserted below the cursor — a rowid reused after a delete — is not emitted. The
/// paging key is monotonic by construction (`WHERE _rid > ?1`), which is exactly why D19
/// puts a key that is not store-wide monotonic on `unsafe_cursors[]`. Stated as a test so
/// the behaviour is a decision on the record rather than a surprise in a 168-hour run.
#[test]
fn a_rowid_reused_below_the_cursor_is_not_emitted_and_that_is_the_paging_key_saying_so() {
    let t = tempfile::tempdir().unwrap();
    let (db, held) = wal_db(t.path(), 4);
    let store = support::sqlite_store("event", "event");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, db.display().to_string())
            .unwrap();
    assert_eq!(tl.poll().unwrap().records.len(), 4);
    assert_eq!(rowid_of(&tl.cursor()), 4);

    // Free rowid 2 and let SQLite hand it back out.
    held.execute("DELETE FROM event WHERE rowid = 2", [])
        .unwrap();
    held.execute(
        "INSERT INTO event (rowid, id, type, session_id, blob) \
         VALUES (2, 'evt_reused', 'session.created.1', 's1', NULL)",
        [],
    )
    .unwrap();
    let b = tl.poll().unwrap();
    assert!(
        b.records.is_empty(),
        "a row below the paging key is behind the cursor and is not re-read"
    );
    assert_eq!(rowid_of(&tl.cursor()), 4, "and the cursor does not go back");

    // A row above it still arrives.
    held.execute(
        "INSERT INTO event VALUES ('evt_5', 'session.created.1', 's1', NULL)",
        [],
    )
    .unwrap();
    assert_eq!(tl.poll().unwrap().records.len(), 1);
}

/// Every SQLite type reaches the payload as itself, and the two that JSON has no room for
/// go through base64: a BLOB, and a TEXT column holding bytes that are not valid UTF-8.
#[test]
fn column_types_survive_and_the_two_that_json_cannot_hold_are_base64() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("opencode.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute(
        "CREATE TABLE event (i INTEGER, r REAL, s TEXT, b BLOB, n TEXT, bad TEXT)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO event VALUES (?1, ?2, ?3, ?4, NULL, CAST(?5 AS TEXT))",
        rusqlite::params![
            42i64,
            1.5f64,
            "héllo",
            vec![0x00u8, 0x01, 0x02],
            vec![0xffu8, 0xfeu8]
        ],
    )
    .unwrap();

    let store = support::sqlite_store("event", "event");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, p.display().to_string()).unwrap();
    let b = tl.poll().unwrap();
    let row = &b.records[0].payload["row"];
    assert_eq!(row["i"], 42);
    assert_eq!(row["r"], 1.5);
    assert_eq!(row["s"], "héllo");
    assert_eq!(row["b"], "AAEC", "a blob is base64");
    assert!(row["n"].is_null(), "a NULL column is JSON null, not absent");
    assert_eq!(
        row["bad"], "//4=",
        "a TEXT column that is not valid UTF-8 is base64 too, not a lossy rendering"
    );
}

/// A store whose declared cursor no SQLite reader can page by is refused when it is built,
/// by name, rather than silently reading the table from zero on every poll.
#[test]
fn a_sqlite_store_without_a_table_or_with_a_line_cursor_is_refused_by_name() {
    let mut no_table = support::sqlite_store("event", "event");
    no_table.table = None;
    let e = goalcup_daemon::tailer::sqlite::SqliteTailer::new(&no_table, "/tmp/x.db".to_string())
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("event") && e.contains("table"), "{e}");

    let mut wrong = support::sqlite_store("event", "event");
    wrong.cursor.kind = goalcup_protocol::manifest::CursorType::InodeOffset;
    let e = goalcup_daemon::tailer::sqlite::SqliteTailer::new(&wrong, "/tmp/x.db".to_string())
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("event") && e.contains("inode_offset"), "{e}");

    // A table name that is not an identifier never reaches a `format!`ed statement.
    let mut evil = support::sqlite_store("event", "event\"; DROP TABLE event; --");
    evil.table = Some("event\"; DROP TABLE event; --".to_string());
    assert!(
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&evil, "/tmp/x.db".to_string()).is_err()
    );
}

/// `tailer_for` builds the SQLite reader for a `mode = "sqlite"` store; the three database
/// stores are planted on Codex's manifest.
#[test]
fn tailer_for_builds_a_reader_for_every_sqlite_store_a_manifest_declares() {
    let t = tempfile::tempdir().unwrap();
    let mut m = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("codex-cli/manifest.toml")).unwrap(),
    )
    .unwrap();
    for (name, table) in [("event", "event"), ("message", "message"), ("part", "part")] {
        m.store.push(support::sqlite_store(name, table));
    }
    let vars = goalcup_daemon::paths::PathVars::new(
        "/ws".into(),
        t.path().display().to_string().into(),
        t.path().display().to_string().into(),
        "/art".into(),
        None,
        ("2026".into(), "09".into(), "04".into()),
    );
    let mut sqlite_stores = 0;
    for s in &m.store {
        let mut tl = goalcup_daemon::tailer::tailer_for(s, &m, &vars)
            .unwrap_or_else(|e| panic!("{}: {e}", s.name));
        assert_eq!(tl.name(), s.name);
        if s.mode == goalcup_protocol::manifest::StoreMode::Sqlite {
            sqlite_stores += 1;
            // Nothing has been created under the per-run config dir yet, so every one of
            // them reports its own reach and none of them errors.
            let b = tl.poll().unwrap_or_else(|e| panic!("{}: {e}", s.name));
            assert!(b.records.is_empty());
            assert!(matches!(b.notices[0], TailNotice::StoreUnavailable { .. }));
            assert!(b.notices[0].path().ends_with("opencode.db"));
        }
    }
    assert_eq!(sqlite_stores, 3, "event, message and part");
}

/// A harness that deletes and recreates its database leaves a new file at the same path.
/// The connection is held across polls, so without an identity check the tailer reads the
/// unlinked inode for ever: zero rows, no notice, and a poll loop that looks healthy while
/// capture has stopped. The file's identity is checked on every poll instead, and a
/// replacement is chained once and then paged from the start of the new rowid space.
#[test]
fn a_deleted_and_recreated_database_is_file_replaced_and_a_new_rowid_space() {
    let t = tempfile::tempdir().unwrap();
    let (db, held) = wal_db(t.path(), 3);
    let store = support::sqlite_store("event", "event");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, db.display().to_string())
            .unwrap();
    assert_eq!(tl.poll().unwrap().records.len(), 3);
    assert_eq!(rowid_of(&tl.cursor()), 3);
    let inode_before = inode_of_path(&db);

    // The harness throws the database away, sidecars and all.
    drop(held);
    std::fs::remove_file(&db).unwrap();
    let _ = std::fs::remove_file(db.with_extension("db-wal"));
    let _ = std::fs::remove_file(db.with_extension("db-shm"));

    // While it is gone the store is out of reach. That is a health signal about the
    // daemon's own reach and not a rewrite: nothing on disk has been observed to change.
    let b = tl.poll().unwrap();
    assert!(b.records.is_empty());
    assert!(matches!(b.notices[0], TailNotice::StoreUnavailable { .. }));
    assert!(
        !b.notices.iter().any(|n| n.is_rewrite()),
        "a file that has not come back yet has not been replaced"
    );

    // It comes back, with two rows of its own and an inode of its own.
    let (_db2, _held2) = wal_db(t.path(), 2);
    let inode_after = inode_of_path(&db);
    assert_ne!(
        inode_after, inode_before,
        "the fixture did not get a new inode, so this test asserts nothing"
    );
    let b = tl.poll().unwrap();
    let replaced: Vec<&TailNotice> = b
        .notices
        .iter()
        .filter(|n| matches!(n, TailNotice::FileReplaced { .. }))
        .collect();
    assert_eq!(replaced.len(), 1, "chained once: {:?}", b.notices);
    match replaced[0] {
        TailNotice::FileReplaced {
            inode_before: a,
            inode_after: c,
            ..
        } => {
            assert_eq!(*a, Some(inode_before));
            assert_eq!(*c, Some(inode_after));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(replaced[0].kind(), Some(TailNoticeKind::FileReplaced));
    assert!(replaced[0].is_rewrite());

    // A recreated database is a new rowid space. The rows start at 1 again, and a cursor
    // that kept its old high-water mark would swallow both of them without a word.
    assert_eq!(b.records.len(), 2, "the new rows, read from rowid 1");
    let rids: Vec<i64> = b
        .records
        .iter()
        .map(|r| r.payload["row"]["_rid"].as_i64().unwrap())
        .collect();
    assert_eq!(rids, vec![1, 2]);

    // And the replacement is reported once, not on every poll after it.
    let b = tl.poll().unwrap();
    assert!(b.notices.is_empty(), "{:?}", b.notices);
    assert!(b.records.is_empty());
}

#[cfg(unix)]
fn inode_of_path(p: &std::path::Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).unwrap().ino()
}

/// `bytes` are the record's raw bytes for redaction and enter no hash at all —
/// the chain link is `Envelope::chain_bytes()` over the redacted payload. A row
/// has no bytes on disk, so its bytes are the canonical JSON of the payload it ships as.
/// They have to be the whole payload: an `event` row's envelope carries the database path,
/// which is under the participant's home directory, and a redactor handed only the row
/// would pass it through in the part of the payload it never saw.
#[test]
fn the_bytes_are_the_text_the_payload_carries_in_both_shapes() {
    let t = tempfile::tempdir().unwrap();
    let (db, _held) = wal_db(t.path(), 1);
    let ev = support::sqlite_store("event", "event");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&ev, db.display().to_string()).unwrap();
    let b = tl.poll().unwrap();
    let r = &b.records[0];
    assert_eq!(r.bytes, serde_json::to_vec(&r.payload).unwrap());
    assert_eq!(r.payload["db"], db.display().to_string());
    assert!(
        String::from_utf8(r.bytes.clone())
            .unwrap()
            .contains(&db.display().to_string()),
        "the db path is in the payload, so the redactor has to be given it"
    );
    // `value` stays the bare row in both shapes: it is the dotted-field view the
    // partitioner reads, and `type` is not `row.type` to it.
    assert_eq!(r.value["type"], "session.created.1");
    assert!(r.value.get("row").is_none());

    // A flat store's payload is its row, so there the two are the same text.
    let p2 = t.path().join("flat.db");
    let c = rusqlite::Connection::open(&p2).unwrap();
    c.execute("CREATE TABLE part (id TEXT, message_id TEXT)", [])
        .unwrap();
    c.execute("INSERT INTO part VALUES ('prt_1', 'msg_1')", [])
        .unwrap();
    let st = support::sqlite_store("part", "part");
    let mut tl2 =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&st, p2.display().to_string()).unwrap();
    let b2 = tl2.poll().unwrap();
    let r2 = &b2.records[0];
    assert_eq!(r2.payload, r2.value, "a flat row ships as itself");
    assert_eq!(r2.bytes, serde_json::to_vec(&r2.value).unwrap());
    assert_eq!(r2.bytes, serde_json::to_vec(&r2.payload).unwrap());
}

/// `restore` makes the persisted `sqlite_pk` number the paging key, which is only sound
/// when the declared column is an alias for the rowid. SQLite makes exactly one column
/// that: a single-column INTEGER PRIMARY KEY. Any other INTEGER column holds numbers of
/// the harness's own choosing, and paging from one of those after a restart would skip or
/// re-ship rows with nothing saying so — so the schema is checked at open and refused by
/// name. A TEXT primary key is not affected: its number was never the column's.
#[test]
fn a_sqlite_pk_on_an_integer_column_that_is_not_a_rowid_alias_is_refused_by_name() {
    let t = tempfile::tempdir().unwrap();
    let store = support::sqlite_store("message", "message");

    // INTEGER, but not the primary key at all.
    let bad = t.path().join("bad.db");
    let c = rusqlite::Connection::open(&bad).unwrap();
    c.execute("CREATE TABLE message (id INTEGER, role TEXT)", [])
        .unwrap();
    c.execute("INSERT INTO message VALUES (7, 'user')", [])
        .unwrap();
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, bad.display().to_string())
            .unwrap();
    let b = tl.poll().unwrap();
    assert!(
        b.records.is_empty(),
        "a cursor column that is not a rowid alias ships nothing"
    );
    match &b.notices[0] {
        TailNotice::StoreUnavailable { reason, .. } => assert!(
            reason.contains("id") && reason.contains("message"),
            "the refusal names the column: {reason}"
        ),
        other => panic!("{other:?}"),
    }
    assert!(
        b.notices[0].kind().is_none(),
        "a schema the daemon cannot page by is a health signal, not a rewrite"
    );

    // A composite primary key is not an alias either: `id` is one of two.
    let composite = t.path().join("composite.db");
    let c = rusqlite::Connection::open(&composite).unwrap();
    c.execute(
        "CREATE TABLE message (id INTEGER, seq INTEGER, PRIMARY KEY (id, seq))",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO message VALUES (7, 1)", []).unwrap();
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, composite.display().to_string())
            .unwrap();
    let b = tl.poll().unwrap();
    assert!(b.records.is_empty());
    assert!(matches!(b.notices[0], TailNotice::StoreUnavailable { .. }));

    // The one SQLite really does alias: the number is the rowid, so the cursor is too.
    let good = t.path().join("good.db");
    let c = rusqlite::Connection::open(&good).unwrap();
    c.execute(
        "CREATE TABLE message (id INTEGER PRIMARY KEY, role TEXT)",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO message VALUES (7, 'user')", [])
        .unwrap();
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, good.display().to_string())
            .unwrap();
    let b = tl.poll().unwrap();
    assert!(b.notices.is_empty(), "{:?}", b.notices);
    assert_eq!(b.records.len(), 1);
    assert_eq!(b.records[0].payload["_rid"], 7);
    assert_eq!(
        rowid_of(&tl.cursor()),
        7,
        "the declared column is the rowid, so both numbers are 7"
    );

    // And a TEXT primary key keeps the rowid fallback.
    let text = t.path().join("text.db");
    let c = rusqlite::Connection::open(&text).unwrap();
    c.execute("CREATE TABLE message (id TEXT PRIMARY KEY, role TEXT)", [])
        .unwrap();
    c.execute("INSERT INTO message VALUES ('msg_1', 'user')", [])
        .unwrap();
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, text.display().to_string())
            .unwrap();
    let b = tl.poll().unwrap();
    assert!(b.notices.is_empty(), "{:?}", b.notices);
    assert_eq!(b.records.len(), 1);
    assert_eq!(rowid_of(&tl.cursor()), 1, "the rowid, not the TEXT id");
}

/// A table with a column of its own called `rowid` shadows SQLite's, so
/// `SELECT rowid AS _rid` projects the harness's TEXT into the paging key. `WHERE _rid > ?`
/// then matches for ever (SQLite sorts TEXT above every number), the key never advances,
/// and the same page comes back on every poll — total capture loss that reads as a healthy
/// run. The tailer reports the store unavailable and names the table instead.
#[test]
fn a_paging_key_that_is_not_an_integer_is_a_notice_and_not_a_page_that_repeats_for_ever() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("opencode.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute("CREATE TABLE event (rowid TEXT, type TEXT)", [])
        .unwrap();
    c.execute(
        "INSERT INTO event VALUES ('not-a-number', 'session.created.1')",
        [],
    )
    .unwrap();
    let store = support::sqlite_store("event", "event");
    let mut tl =
        goalcup_daemon::tailer::sqlite::SqliteTailer::new(&store, p.display().to_string()).unwrap();
    for poll in 0..2 {
        let b = tl.poll().unwrap();
        assert!(
            b.records.is_empty(),
            "poll {poll} emitted a row off a key that cannot advance"
        );
        match &b.notices[0] {
            TailNotice::StoreUnavailable { reason, .. } => {
                assert!(
                    reason.contains("event"),
                    "the notice names the table: {reason}"
                )
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(rowid_of(&tl.cursor()), -1, "and the cursor never moved");
}

/// `jsonl.rs` says a line is overlong above the per-line cap, and both comparisons
/// in it are `>`. With `MAX_POLL_BYTES == MAX_LINE_BYTES` a
/// line of exactly the cap would fill the whole poll window with no newline in it, hit the
/// `capped && i == 0` arm — the one for a line that can never be assembled inside the
/// bounds — and be discarded as `OverlongLine`. One byte of headroom is the difference
/// between a bound the tailer states and a bound it exceeds by a byte and blames the
/// harness for.
#[test]
fn a_line_of_exactly_the_line_cap_is_captured_and_not_discarded_as_overlong() {
    assert_eq!(
        goalcup_daemon::MAX_POLL_BYTES,
        goalcup_daemon::MAX_LINE_BYTES + 1,
        "a poll window must hold a line at the cap AND the newline that ends it"
    );
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("main.jsonl");
    // Exactly MAX_LINE_BYTES of line, then the newline: `{"t":"` + filler + `"}`.
    let head = br#"{"t":""#;
    let tail = br#""}"#;
    let filler = goalcup_daemon::MAX_LINE_BYTES as usize - head.len() - tail.len();
    let mut line = Vec::new();
    line.extend_from_slice(head);
    line.extend(std::iter::repeat_n(b'x', filler));
    line.extend_from_slice(tail);
    assert_eq!(line.len() as u64, goalcup_daemon::MAX_LINE_BYTES);
    line.push(b'\n');
    std::fs::write(&f, &line).unwrap();

    let mut tl = JsonlTailer::new("main", t.path().join("*.jsonl").display().to_string(), None);
    let b = tl.poll().unwrap();
    assert_eq!(b.records.len(), 1, "at the cap is not above it");
    assert_eq!(
        b.records[0].payload.as_str().unwrap().len() as u64,
        goalcup_daemon::MAX_LINE_BYTES
    );
    assert!(
        b.notices.is_empty(),
        "nothing was dropped, so nothing is accounted for: {:?}",
        b.notices
    );

    // And one byte above it still is: the headroom moves the window, not the rule.
    let g = t.path().join("over.jsonl");
    let mut over = line.clone();
    over.pop();
    over.push(b'x');
    over.push(b'\n');
    std::fs::write(&g, &over).unwrap();
    let mut tl = JsonlTailer::new("over", g.display().to_string(), None);
    let b = tl.poll().unwrap();
    assert!(b.records.is_empty());
    assert!(matches!(b.notices[0], TailNotice::OverlongLine { .. }));
}

/// `tailers_for` builds the whole set a store declares, `subagent_path_template` and
/// `spill_path_template` included: Claude Code's subagent transcripts and its spilled
/// `tool-results/` have to reach the chain, because the human rule's `sidechain` exclusion
/// and its `not_subagent_thread_source` scoping both judge records that can only come from
/// there.
#[test]
fn a_store_is_read_through_every_path_template_it_declares_not_only_the_first() {
    let t = tempfile::tempdir().unwrap();
    let m = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("claude-code-cli/manifest.toml")).unwrap(),
    )
    .unwrap();

    // Claude Code's own layout: `{config_dir}/projects/{slug}/{session_id}.jsonl` beside
    // `…/{session_id}/subagents/agent-*.jsonl` and `…/{session_id}/tool-results/`.
    let config_dir = t.path().join("cfg");
    let slug = "-ws-e1";
    let session = "sess-1";
    let project = config_dir.join("projects").join(slug);
    std::fs::create_dir_all(project.join(session).join("subagents")).unwrap();
    std::fs::create_dir_all(project.join(session).join("tool-results")).unwrap();
    append(
        &project.join(format!("{session}.jsonl")),
        &[r#"{"type":"user","isSidechain":false,"sessionId":"sess-1"}"#],
    );
    append(
        &project
            .join(session)
            .join("subagents")
            .join("agent-a.jsonl"),
        &[
            r#"{"type":"user","isSidechain":true,"sessionId":"sess-1"}"#,
            r#"{"type":"assistant","isSidechain":true,"sessionId":"sess-1"}"#,
        ],
    );
    std::fs::write(
        project.join(session).join("tool-results").join("r1.json"),
        br#"{"tool":"Bash","stdout":"spilled"}"#,
    )
    .unwrap();

    let vars = goalcup_daemon::paths::PathVars::new(
        "/ws/e1".into(),
        t.path().display().to_string().into(),
        config_dir.display().to_string().into(),
        t.path().join("art").display().to_string().into(),
        Some(session.to_string()),
        ("2026".into(), "09".into(), "04".into()),
    );

    let main = m.store_named("main").unwrap();
    let built = goalcup_daemon::tailer::tailers_for(main, &m, &vars).unwrap();
    let names: Vec<&str> = built.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["main", "main_subagent", "main_spill"]);

    let mut seen: std::collections::BTreeMap<String, Vec<serde_json::Value>> = Default::default();
    for (name, mut tl) in built {
        assert_eq!(tl.name(), name);
        let b = tl.poll().unwrap();
        seen.insert(name, b.records.into_iter().map(|r| r.value).collect());
    }

    assert_eq!(seen["main"].len(), 1);
    // The subagent transcript is JSONL and is read line by line, like its parent store:
    // the verifier's `is_sidechain` reads `isSidechain` off each record, and a whole-file
    // capture of a two-line JSONL file parses to `null` and tells it nothing.
    assert_eq!(seen["main_subagent"].len(), 2);
    assert_eq!(seen["main_subagent"][0]["isSidechain"], true);
    assert_eq!(seen["main_subagent"][1]["type"], "assistant");
    // The spill directory holds whole files and is read as one.
    assert_eq!(seen["main_spill"].len(), 1);
    assert_eq!(seen["main_spill"][0]["stdout"], "spilled");
}

/// Codex declares `subagent_path_template` identical to its own `path_template` — the
/// subagent rollout is a file in the same dated directory, already inside the store's glob.
/// A second reader over the same pattern would chain every rollout record twice, which reads
/// downstream as a harness replaying its transcript.
#[test]
fn a_subagent_template_that_renders_to_the_stores_own_pattern_builds_no_second_reader() {
    let t = tempfile::tempdir().unwrap();
    let m = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("codex-cli/manifest.toml")).unwrap(),
    )
    .unwrap();
    let rollout = m.store_named("rollout").unwrap();
    assert_eq!(
        rollout.subagent_path_template.as_deref(),
        Some(rollout.path_template.as_str()),
        "the manifest this test is about"
    );
    let vars = goalcup_daemon::paths::PathVars::new(
        "/ws/e1".into(),
        t.path().display().to_string().into(),
        t.path().display().to_string().into(),
        t.path().join("art").display().to_string().into(),
        None,
        ("2026".into(), "09".into(), "04".into()),
    );
    let built = goalcup_daemon::tailer::tailers_for(rollout, &m, &vars).unwrap();
    let names: Vec<&str> = built.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["rollout"]);
}

/// The "already read" dedupe compares a derived template's rendered pattern against every
/// store's `path_template`, not only its own — so a manifest with a second store
/// already gloving the exact files a first store's `subagent_path_template` would read
/// (nothing in the schema rules that out) builds no redundant reader over it.
#[test]
fn a_derived_template_colliding_with_a_different_stores_pattern_builds_no_reader() {
    let t = tempfile::tempdir().unwrap();
    let mut m = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("claude-code-cli/manifest.toml")).unwrap(),
    )
    .unwrap();
    let main = m.store_named("main").unwrap().clone();

    // A synthetic sibling store whose own `path_template` renders to exactly what
    // `main`'s `subagent_path_template` renders to — a different store already reading
    // those bytes, not `main`'s own pattern (that case is the test above).
    let mut sibling = main.clone();
    sibling.name = "sibling".to_string();
    sibling.path_template = main.subagent_path_template.clone().unwrap();
    sibling.subagent_path_template = None;
    sibling.spill_path_template = None;
    m.store.push(sibling);

    let vars = goalcup_daemon::paths::PathVars::new(
        "/ws/e1".into(),
        t.path().display().to_string().into(),
        t.path().display().to_string().into(),
        t.path().join("art").display().to_string().into(),
        Some("sess-1".to_string()),
        ("2026".into(), "09".into(), "04".into()),
    );

    let main_ref = m.store_named("main").unwrap();
    let built = goalcup_daemon::tailer::tailers_for(main_ref, &m, &vars).unwrap();
    let names: Vec<&str> = built.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["main", "main_spill"],
        "`sibling` already reads the pattern `main_subagent` would have read, so no \
         second reader is built for it"
    );
}

/// `SqliteTailer::new` reads its name off the store, so a derived
/// reader (a subagent or spill template on a `mode = "sqlite"` store) would come back
/// answering `name()` with the parent's name. No shipped
/// connector declares one; this manifest is a synthetic double built here so the day one
/// does, this is where it is refused rather than silently building a mis-named reader.
#[test]
fn a_sqlite_store_with_a_derived_path_template_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let mut m = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("codex-cli/manifest.toml")).unwrap(),
    )
    .unwrap();
    m.store.push(support::sqlite_store("event", "event"));
    let event = m.store_named("event").unwrap().clone();
    assert_eq!(event.mode, goalcup_protocol::manifest::StoreMode::Sqlite);

    let event_mut = m.store.iter_mut().find(|s| s.name == "event").unwrap();
    event_mut.subagent_path_template =
        Some("{config_dir}/data/opencode/subagents/*.jsonl".to_string());

    let vars = goalcup_daemon::paths::PathVars::new(
        "/ws/e1".into(),
        t.path().display().to_string().into(),
        t.path().display().to_string().into(),
        t.path().join("art").display().to_string().into(),
        None,
        ("2026".into(), "09".into(), "04".into()),
    );

    let event_ref = m.store_named("event").unwrap();
    match goalcup_daemon::tailer::tailers_for(event_ref, &m, &vars) {
        Err(goalcup_daemon::DaemonError::Refused(msg)) => {
            assert!(msg.contains("event"), "{msg}");
            assert!(msg.contains("sqlite"), "{msg}");
        }
        Err(other) => panic!("expected DaemonError::Refused, got {other:?}"),
        Ok(_) => panic!("expected the derived sqlite reader to be refused"),
    }
}

/// A substituted session is an authority boundary, not an arbitrary string splice. If the
/// token shares a component with surrounding text, the daemon cannot prove that a concrete
/// session owns the directory it is about to read.
#[test]
fn a_derived_template_requires_the_session_to_be_its_own_path_component() {
    let mut manifest = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("claude-code-cli/manifest.toml")).unwrap(),
    )
    .unwrap();
    let main = manifest
        .store
        .iter_mut()
        .find(|store| store.name == "main")
        .unwrap();
    main.subagent_path_template = Some(
        "{config_dir}/projects/{slug}/session-{session_id}/subagents/agent-*.jsonl".to_string(),
    );

    match goalcup_daemon::tailer::binding_roster(&manifest) {
        Err(goalcup_daemon::DaemonError::Refused(message)) => {
            assert!(
                message.contains("exact session path component"),
                "{message}"
            );
        }
        Err(other) => panic!("expected DaemonError::Refused, got {other:?}"),
        Ok(_) => panic!("a session token embedded in a larger path component was accepted"),
    }
}

#[test]
fn a_derived_binding_refuses_a_separator_or_glob_in_the_bound_session_before_reader_creation() {
    let root = tempfile::tempdir().unwrap();
    let manifest = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("claude-code-cli/manifest.toml")).unwrap(),
    )
    .unwrap();
    let declarations = goalcup_daemon::tailer::binding_roster(&manifest).unwrap();
    let parent_order: Vec<String> = manifest
        .store
        .iter()
        .map(|store| store.name.clone())
        .collect();
    let vars = goalcup_daemon::paths::PathVars::new(
        root.path().join("workspace"),
        root.path().join("home"),
        root.path().join("config"),
        root.path().join("artifacts"),
        Some("sibling/*".to_string()),
        ("2026".into(), "09".into(), "10".into()),
    );

    match goalcup_daemon::tailer::build_bound_tailers(
        &declarations,
        &manifest,
        &vars,
        &parent_order,
    ) {
        Err(goalcup_daemon::DaemonError::Refused(message)) => {
            assert!(message.contains("safe literal path component"), "{message}");
        }
        Err(other) => panic!("expected DaemonError::Refused, got {other:?}"),
        Ok(_) => panic!("a session containing a glob path component was accepted"),
    }
}

#[test]
fn a_generated_derived_name_cannot_collide_with_a_declared_primary_store() {
    let mut manifest = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("claude-code-cli/manifest.toml")).unwrap(),
    )
    .unwrap();
    let mut collision = manifest.store_named("raw_bodies").unwrap().clone();
    collision.name = "main_subagent".to_string();
    collision.subagent_path_template = None;
    collision.spill_path_template = None;
    manifest.store.push(collision);

    match goalcup_daemon::tailer::binding_roster(&manifest) {
        Err(goalcup_daemon::DaemonError::Refused(message)) => {
            assert!(message.contains("main_subagent"), "{message}");
            assert!(message.contains("collides"), "{message}");
        }
        Err(other) => panic!("expected DaemonError::Refused, got {other:?}"),
        Ok(_) => panic!("a generated derived name collided with a declared primary store"),
    }
}

#[test]
fn a_nonidentical_derived_template_that_resolves_to_a_primary_pattern_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let mut manifest = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        &std::fs::read_to_string(connectors_dir().join("claude-code-cli/manifest.toml")).unwrap(),
    )
    .unwrap();
    let main = manifest.store_named("main").unwrap().clone();
    let mut sibling = main.clone();
    sibling.name = "sibling".to_string();
    sibling.path_template =
        "{home}/projects/{slug}/{session_id}/subagents/agent-*.jsonl".to_string();
    sibling.subagent_path_template = None;
    sibling.spill_path_template = None;
    manifest.store.push(sibling);
    let declarations = goalcup_daemon::tailer::binding_roster(&manifest)
        .expect("only the rendered collision is under test");
    let parent_order: Vec<String> = manifest
        .store
        .iter()
        .map(|store| store.name.clone())
        .collect();
    let vars = goalcup_daemon::paths::PathVars::new(
        root.path().join("workspace"),
        root.path().join("same-root"),
        root.path().join("same-root"),
        root.path().join("artifacts"),
        Some("session-a".to_string()),
        ("2026".into(), "09".into(), "10".into()),
    );

    match goalcup_daemon::tailer::build_bound_tailers(
        &declarations,
        &manifest,
        &vars,
        &parent_order,
    ) {
        Err(goalcup_daemon::DaemonError::Refused(message)) => {
            assert!(message.contains("active primary"), "{message}");
        }
        Err(other) => panic!("expected DaemonError::Refused, got {other:?}"),
        Ok(_) => panic!("a rendered collision with a distinct primary declaration was accepted"),
    }
}

/// `DirTailer.seen` holds an entry only for a file the store still has: one entry per file
/// the store ever held would live for the life of the run, and every entry is re-serialised into
/// `cursors.json` on every save. A store like Claude Code's spilled `tool-results/` is a
/// directory of them.
///
/// Eviction is the bound that matters, and it costs nothing: the poll already lists the
/// directory, so a file that is gone is forgotten in the same pass. Re-creating the path
/// makes it a new file, which is captured — the right answer for a store whose files are
/// written once and never appended to.
#[test]
fn the_seen_set_forgets_a_file_that_is_no_longer_there() {
    let t = tempfile::tempdir().unwrap();
    for i in 0..4 {
        std::fs::write(
            t.path().join(format!("r{i}.json")),
            format!("{{\"i\":{i}}}"),
        )
        .unwrap();
    }
    let mut d = DirTailer::new("raw_bodies", t.path().join("*.json").display().to_string());
    let b = d.poll().unwrap();
    assert_eq!(b.records.len(), 4);
    assert_eq!(d.seen_len(), 4);

    // Three of them are cleaned up by the harness mid-run.
    for i in 0..3 {
        std::fs::remove_file(t.path().join(format!("r{i}.json"))).unwrap();
    }
    let b = d.poll().unwrap();
    assert!(b.records.is_empty(), "nothing new to capture");
    assert_eq!(
        d.seen_len(),
        1,
        "the seen-set holds only the file that is still there"
    );
    match d.cursor() {
        CursorState::SeenFiles { seen, .. } => assert_eq!(
            seen.len(),
            1,
            "and the cursor `cursors.json` serialises is the same size"
        ),
        other => panic!("expected a seen-file cursor, got {other:?}"),
    }

    // A path that comes back is a new file.
    std::fs::write(t.path().join("r0.json"), "{\"i\":0}").unwrap();
    let b = d.poll().unwrap();
    assert_eq!(b.records.len(), 1, "a re-created path is captured again");
    assert_eq!(d.seen_len(), 2);
}

/// …and the backstop on what eviction cannot reach: files that are all still present.
/// At the cap the poll captures no file it has not already seen and says so through a
/// `StoreUnavailable` notice, which is a health signal and chains nothing. Capturing
/// without remembering would re-capture the same file on every poll for the rest of the
/// run — a bounded map traded for an unbounded chain.
#[test]
fn the_seen_set_stops_at_its_bound_rather_than_forgetting_a_live_file() {
    let t = tempfile::tempdir().unwrap();
    for i in 0..2 {
        std::fs::write(
            t.path().join(format!("r{i}.json")),
            format!("{{\"i\":{i}}}"),
        )
        .unwrap();
    }
    let mut d = DirTailer::new("raw_bodies", t.path().join("*.json").display().to_string());
    d.set_seen_cap(2);
    let b = d.poll().unwrap();
    assert_eq!(b.records.len(), 2);
    assert_eq!(d.seen_len(), 2);

    std::fs::write(t.path().join("r2.json"), "{\"i\":2}").unwrap();
    let b = d.poll().unwrap();
    assert!(
        b.records.is_empty(),
        "the third file is left for a poll that has room, not captured unremembered"
    );
    assert_eq!(d.seen_len(), 2, "and the bound holds");
    assert!(
        matches!(
            b.notices.as_slice(),
            [TailNotice::StoreUnavailable { store, reason, .. }]
                if store == "raw_bodies" && reason.contains("seen-set is at its bound")
        ),
        "the cap is reported, not silent: {:?}",
        b.notices
    );
    assert!(
        b.notices[0].kind().is_none(),
        "a health signal about the daemon's own reach chains no envelope"
    );

    // Room comes back the moment one of the two is gone.
    std::fs::remove_file(t.path().join("r0.json")).unwrap();
    let b = d.poll().unwrap();
    assert_eq!(b.records.len(), 1, "the third file is captured now");
    assert!(b.notices.is_empty());
    assert_eq!(d.seen_len(), 2);
}

/// One unreadable file in a glob must not throw away the records the same poll
/// already took out of the files sorted before it — whose offsets have already moved past
/// them, so they are never read again and nothing says they existed. The unreadable path
/// gets a `StoreUnavailable`, which chains nothing (it is the daemon's own reach, not an
/// on-disk signature), keeps its position, and is retried.
#[cfg(unix)]
#[test]
fn a_file_the_line_tailer_cannot_open_costs_that_file_and_not_the_batch() {
    use std::os::unix::fs::PermissionsExt;

    let t = tempfile::tempdir().unwrap();
    // `expand_globs` sorts, so the unreadable one is read between two readable ones.
    let a = t.path().join("a.jsonl");
    let b = t.path().join("b.jsonl");
    let c = t.path().join("c.jsonl");
    append(&a, &[r#"{"type":"user","n":1}"#]);
    append(&b, &[r#"{"type":"user","n":2}"#]);
    append(&c, &[r#"{"type":"user","n":3}"#]);
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o000)).unwrap();

    let mut tl = JsonlTailer::new("main", t.path().join("*.jsonl").display().to_string(), None);
    let batch = tl.poll().unwrap();

    let ns: Vec<i64> = batch
        .records
        .iter()
        .map(|r| r.value["n"].as_i64().unwrap())
        .collect();
    assert_eq!(
        ns,
        vec![1, 3],
        "the readable files on both sides of the failure are captured"
    );
    let unavailable: Vec<&TailNotice> = batch
        .notices
        .iter()
        .filter(|n| matches!(n, TailNotice::StoreUnavailable { .. }))
        .collect();
    assert_eq!(unavailable.len(), 1, "one notice, for one path");
    assert_eq!(unavailable[0].path(), b.display().to_string());
    assert_eq!(unavailable[0].store(), "main");
    assert!(
        unavailable[0].kind().is_none() && unavailable[0].to_event(String::new()).is_none(),
        "the daemon's own reach is a health signal, not a chained on-disk fact"
    );

    // The file kept its position, so when it becomes readable its record is still there.
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o644)).unwrap();
    let again = tl.poll().unwrap();
    let ns: Vec<i64> = again
        .records
        .iter()
        .map(|r| r.value["n"].as_i64().unwrap())
        .collect();
    assert_eq!(ns, vec![2], "nothing was lost, only deferred");
    assert!(
        !again
            .notices
            .iter()
            .any(|n| matches!(n, TailNotice::StoreUnavailable { .. })),
        "and the notice stops once the file reads"
    );
}

/// The same rule for the whole-file reader: a spill file the daemon may not read is one
/// file's notice, not the end of the poll: aborting would drop the files listed after it, and
/// the earlier ones are already in the seen-set and would never be
/// captured again.
#[cfg(unix)]
#[test]
fn a_file_the_dir_tailer_cannot_open_costs_that_file_and_not_the_batch() {
    use std::os::unix::fs::PermissionsExt;

    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a.json");
    let b = t.path().join("b.json");
    let c = t.path().join("c.json");
    std::fs::write(&a, r#"{"n":1}"#).unwrap();
    std::fs::write(&b, r#"{"n":2}"#).unwrap();
    std::fs::write(&c, r#"{"n":3}"#).unwrap();
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o000)).unwrap();

    let mut d = DirTailer::new("raw_bodies", t.path().join("*.json").display().to_string());
    let batch = d.poll().unwrap();

    let ns: Vec<i64> = batch
        .records
        .iter()
        .map(|r| r.value["n"].as_i64().unwrap())
        .collect();
    assert_eq!(ns, vec![1, 3], "the files either side of the failure");
    let unavailable: Vec<&TailNotice> = batch
        .notices
        .iter()
        .filter(|n| matches!(n, TailNotice::StoreUnavailable { .. }))
        .collect();
    assert_eq!(unavailable.len(), 1);
    assert_eq!(unavailable[0].path(), b.display().to_string());

    // Not marked seen, so it is captured whole once it reads — and the two that were
    // captured are not captured twice.
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o644)).unwrap();
    let again = d.poll().unwrap();
    let ns: Vec<i64> = again
        .records
        .iter()
        .map(|r| r.value["n"].as_i64().unwrap())
        .collect();
    assert_eq!(ns, vec![2]);
}

/// A directory the daemon cannot list is the module header's "health signal the poll
/// reports rather than an error that stops every other store" — and the seen-set is not
/// pruned against a listing that failed, because a file missing from a short listing is not
/// a file that is gone. Pruning it would re-capture every one of them on the next poll.
#[cfg(unix)]
#[test]
fn a_directory_the_dir_tailer_cannot_list_does_not_empty_the_seen_set() {
    use std::os::unix::fs::PermissionsExt;

    let t = tempfile::tempdir().unwrap();
    let inner = t.path().join("tool-output");
    std::fs::create_dir(&inner).unwrap();
    std::fs::write(inner.join("one.json"), r#"{"n":1}"#).unwrap();

    let mut d = DirTailer::new("raw_bodies", inner.display().to_string());
    assert_eq!(d.poll().unwrap().records.len(), 1);
    assert_eq!(d.seen_len(), 1, "the file is remembered");

    std::fs::set_permissions(&inner, std::fs::Permissions::from_mode(0o000)).unwrap();
    let batch = d.poll().unwrap();
    std::fs::set_permissions(&inner, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(batch.records.is_empty(), "nothing could be listed");
    assert!(
        batch
            .notices
            .iter()
            .any(|n| matches!(n, TailNotice::StoreUnavailable { .. })
                && n.path() == inner.display().to_string()),
        "the directory is named"
    );
    assert_eq!(
        d.seen_len(),
        1,
        "an unreadable listing is not evidence the file is gone"
    );
    assert!(
        d.poll().unwrap().records.is_empty(),
        "so the file it already captured is not captured a second time"
    );
}
