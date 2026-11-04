//! PRD-09 point 2, thread databases (subagent design item E): Crush and Goose name each call's
//! session on the wire but not its parent. The parent link comes from their session database
//! copy (`sessions.parent_session_id`), read for ids and parents only and kept only for sessions
//! the wire names. Crush's wire names a session by the xxh3-64 of its id. Every value is an
//! obvious fake; the hashes below were computed outside this crate (python `xxhash` 4.0.2). On
//! the recorded Crush run the same rule gave the two wire hashes, 9c7abda62c89e8b3 for the root
//! and 9b1df166f113a629 for the child.

mod common;

use common::{Entry, call_table, edit_event, mutate, tmp};
use goalcup_format::{EndReason, StreamKind};
use goalcup_verify::usage::{CallRow, CallTable, Join, Kind, Leg, Reason, Thread};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

const CRUSH_ROOT: &str = "00000000-fa4e-4000-8000-00000000c0de";
const CRUSH_KID: &str = "fa4e0000-0000-4000-8000-000000000001$$call_00_fake00000000000000000000";
const CRUSH_OLD: &str = "00000000-fa4e-4000-8000-0000000001d0";
/// xxh3-64 of the three ids above, as 16 lowercase hex digits.
const ROOT_HASH: &str = "fe70e8136e5f687d";
const KID_HASH: &str = "487a795fdcff4339";
const OLD_HASH: &str = "d4a77d2bc14f49cb";

const GOOSE_ROOT: &str = "20991231_1";
const GOOSE_KID: &str = "20991231_2";
const GOOSE_OLD: &str = "20991230_9";

/// A title no output may carry.
const MARK: &str = "FAKE-TITLE-NEVER-READ";

/// One `POST /chat/completions` with these request headers, answered whole and ok.
fn chat(e: &mut Entry, headers: &[(&str, &str)]) {
    let c = e.conn();
    let mut hs = vec![json!(["content-type", "application/json"])];
    hs.extend(headers.iter().map(|(k, v)| json!([k, v])));
    let head = json!({"method": "POST", "path": "/v1/chat/completions", "ver": "1.1",
        "headers": hs, "masked": [], "coding": null});
    let q = e.open(StreamKind::HttpReq, c, 0, head);
    let body = br#"{"model":"fake-model"}"#;
    e.data(q, 0, 0, body);
    e.end(q, EndReason::Complete, body.len() as u64, Value::Null);
    let body = json!({"id": "fake-reply", "model": "fake-model",
        "choices": [{"finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 3}})
    .to_string();
    let head = json!({"status": 200, "ver": "1.1", "decoded": true, "masked": [], "coding": null,
        "headers": [["content-type", "application/json"]]});
    let p = e.open(StreamKind::HttpResp, c, q, head);
    e.data(p, 0, 0, body.as_bytes());
    e.end(p, EndReason::Complete, body.len() as u64, Value::Null);
    e.end(c, EndReason::Complete, 0, json!({"up": 90, "down": 90}));
}

/// A session database with a `sessions` table of (id, parent, title) rows, as copied: the main
/// file, and the WAL when `wal` (kept open, no checkpoint). Returns (file name, bytes) pairs.
fn db(name: &str, wal: bool, rows: &[(&str, Option<&str>)]) -> Vec<(String, Vec<u8>)> {
    let dir = tmp(name);
    let p = dir.join(name);
    let conn = rusqlite::Connection::open(&p).unwrap();
    if wal {
        let sql = "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;";
        conn.execute_batch(sql).unwrap();
    }
    conn.execute_batch(
        "CREATE TABLE sessions(id TEXT PRIMARY KEY, parent_session_id TEXT, title TEXT,
            message_count INTEGER)",
    )
    .unwrap();
    for (id, parent) in rows {
        let sql = "INSERT INTO sessions VALUES (?1, ?2, ?3, 1)";
        let values = rusqlite::params![id, parent, MARK];
        conn.execute(sql, values).unwrap();
    }
    let mut out = vec![(name.to_string(), std::fs::read(&p).unwrap())];
    if wal {
        let w = format!("{name}-wal");
        out.push((w.clone(), std::fs::read(dir.join(&w)).unwrap()));
    }
    out
}

/// A sealed entry of this connector: its calls, then the database files as snapshot streams of
/// set `0.0` under the native row `row`.
fn entry(
    name: &str,
    connector: &str,
    row: &str,
    files: &[(String, Vec<u8>)],
    calls: impl FnOnce(&mut Entry),
) -> PathBuf {
    let mut e = Entry::new(name);
    calls(&mut e);
    for (file, body) in files {
        let meta = json!({"path": format!("{row}:{file}"), "gen": 0, "form": "final",
            "set": "0.0", "harness_exited": true, "size": body.len()});
        let s = e.open(StreamKind::Snapshot, 0, 0, meta);
        e.data(s, 0, 0, body);
        e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
    }
    e.count();
    let dir = e.seal();
    let set = |v: &mut Value| v["connector"] = connector.into();
    mutate(&dir, None, |r| edit_event(r, "start", 0, set))
}

fn thread<'a>(t: &'a CallTable, id: &str) -> &'a Thread {
    t.threads.iter().find(|x| x.id == id).expect("thread")
}

fn calls(t: &CallTable) -> Vec<&CallRow> {
    let wire = |r: &&CallRow| r.leg == Leg::Wire && r.kind == Kind::Call;
    t.rows.iter().filter(wire).collect()
}

/// The parent link of `kid`, its children map, and that the database added no thread, no
/// native call, no title and no unattributed call.
fn linked(t: &CallTable, root: &str, kid: &str, n: usize) {
    assert_eq!(t.threads.len(), 2);
    assert_eq!(thread(t, kid).parent.as_deref(), Some(root));
    assert_eq!(thread(t, root).parent, None);
    let kids = BTreeMap::from([(root.to_string(), vec![kid.to_string()])]);
    assert_eq!(t.children, Some(kids));
    let rows = calls(t);
    assert_eq!(rows.len(), n);
    assert!(rows.iter().all(|r| r.join == Some(Join::WireOnly)));
    let mine = |r: &&&CallRow| r.thread.as_deref() == Some(kid);
    let of_kid: Vec<_> = rows.iter().filter(mine).collect();
    assert!(!of_kid.is_empty());
    assert!(of_kid.iter().all(|r| r.parent.as_deref() == Some(root)));
    assert!(t.rows.iter().all(|r| r.leg == Leg::Wire));
    assert!(t.threads.iter().all(|x| x.label.is_none()));
    assert!(!format!("{t:?}").contains(MARK));
    // No native call reader: the participant is told there is none.
    assert_eq!(t.reason, Some(Reason::NoReader));
}

#[test]
fn crush_parent_comes_from_its_database_by_the_wire_hash() {
    let rows = [
        (CRUSH_ROOT, None),
        (CRUSH_KID, Some(CRUSH_ROOT)),
        (CRUSH_OLD, None),
    ];
    let files = db("crush.db", false, &rows);
    let dir = entry("thread-db-crush", "crush-cli", "db", &files, |e| {
        chat(e, &[("x-session-id", ROOT_HASH)]);
        chat(e, &[("x-session-id", KID_HASH)]);
        chat(e, &[("x-session-id", KID_HASH)]);
        chat(e, &[("x-session-id", ROOT_HASH)]);
    });
    let t = call_table(&dir);
    linked(&t, ROOT_HASH, KID_HASH, 4);
    // A session of the shared workspace the wire never names is no thread, hashed or not.
    let ids: Vec<&str> = t.threads.iter().map(|x| x.id.as_str()).collect();
    for raw in [OLD_HASH, CRUSH_OLD, CRUSH_ROOT, CRUSH_KID] {
        assert!(!ids.contains(&raw));
    }
}

#[test]
fn goose_parent_comes_from_its_database_through_the_wal() {
    let rows = [
        (GOOSE_ROOT, None),
        (GOOSE_KID, Some(GOOSE_ROOT)),
        (GOOSE_OLD, None),
    ];
    let files = db("sessions.db", true, &rows);
    assert_eq!(files.len(), 2);
    let dir = entry("thread-db-goose", "goose-cli", "sessions", &files, |e| {
        chat(e, &[("agent-session-id", GOOSE_ROOT)]);
        chat(e, &[("agent-session-id", GOOSE_KID)]);
        chat(e, &[("agent-session-id", GOOSE_ROOT)]);
    });
    let t = call_table(&dir);
    linked(&t, GOOSE_ROOT, GOOSE_KID, 3);
    assert!(t.threads.iter().all(|x| x.id != GOOSE_OLD));
}

#[test]
fn with_no_database_copy_the_wire_threads_stay_without_parents() {
    let dir = entry("thread-db-none", "crush-cli", "db", &[], |e| {
        chat(e, &[("x-session-id", ROOT_HASH)]);
        chat(e, &[("x-session-id", KID_HASH)]);
    });
    let t = call_table(&dir);
    assert_eq!(t.threads.len(), 2);
    assert!(t.threads.iter().all(|x| x.parent.is_none()));
    assert_eq!(t.children, None);
}

#[test]
fn an_empty_crush_parent_is_no_parent_not_the_hash_of_nothing() {
    // xxh3-64 of "", which must never stand as a parent.
    const EMPTY_HASH: &str = "2d06800538d394c2";
    let rows = [(CRUSH_ROOT, Some("")), (CRUSH_KID, Some(CRUSH_ROOT))];
    let files = db("crush.db", false, &rows);
    let dir = entry("thread-db-crush-empty", "crush-cli", "db", &files, |e| {
        chat(e, &[("x-session-id", ROOT_HASH)]);
        chat(e, &[("x-session-id", KID_HASH)]);
    });
    let t = call_table(&dir);
    linked(&t, ROOT_HASH, KID_HASH, 2);
    assert!(!format!("{t:?}").contains(EMPTY_HASH));
}

/// A database file built by `sql` alone, as copied (no WAL).
fn raw_db(name: &str, sql: &str) -> Vec<(String, Vec<u8>)> {
    let p = tmp(name).join(name);
    rusqlite::Connection::open(&p)
        .unwrap()
        .execute_batch(sql)
        .unwrap();
    vec![(name.to_string(), std::fs::read(&p).unwrap())]
}

/// The private copies of this process left in the temp dir (nextest runs one test per process).
fn copies_left() -> Vec<String> {
    let mine = format!("goalcup-sqlite-{}-", std::process::id());
    let names = std::fs::read_dir(std::env::temp_dir()).unwrap().flatten();
    let names = names.map(|d| d.file_name().to_string_lossy().into_owned());
    names.filter(|n| n.starts_with(&mine)).collect()
}

/// Goose sessions ROOT and KID as rows of `table`.
fn goose_rows(table: &str) -> String {
    format!(
        "CREATE TABLE {table}(id TEXT PRIMARY KEY, parent_session_id TEXT);
         INSERT INTO {table} VALUES ('{GOOSE_ROOT}', NULL), ('{GOOSE_KID}', '{GOOSE_ROOT}');"
    )
}

#[test]
fn a_database_with_a_trigger_or_view_function_still_reads_its_table() {
    // Schema objects the reader never names: a trigger, a view calling `load_extension` and a
    // view over a virtual table. The reader still reads the real table and leaves no copy.
    let sql = goose_rows("sessions")
        + "CREATE TABLE log(x);
           CREATE TRIGGER t AFTER INSERT ON sessions BEGIN INSERT INTO log VALUES (new.id); END;
           CREATE VIEW ext AS SELECT load_extension('/nonexistent/evil');
           CREATE VIEW vt AS SELECT * FROM pragma_database_list;";
    let files = raw_db("sessions.db", &sql);
    let dir = entry(
        "thread-db-extra-schema",
        "goose-cli",
        "sessions",
        &files,
        |e| {
            chat(e, &[("agent-session-id", GOOSE_ROOT)]);
            chat(e, &[("agent-session-id", GOOSE_KID)]);
        },
    );
    let before = copies_left();
    let t = call_table(&dir);
    linked(&t, GOOSE_ROOT, GOOSE_KID, 2);
    assert_eq!(copies_left(), before);
}

#[test]
fn a_database_with_a_view_over_a_virtual_table_is_not_run() {
    // `sessions` is a view the record's author wrote, reaching a virtual table SQLite does not
    // call innocuous. With `trusted_schema=OFF` the view does not run, so no parent is read.
    let sql = goose_rows("s")
        + "CREATE VIEW sessions AS SELECT s.id, s.parent_session_id
             FROM s, pragma_database_list AS d WHERE d.name = 'main';";
    let files = raw_db("sessions.db", &sql);
    let dir = entry(
        "thread-db-view-vtab",
        "goose-cli",
        "sessions",
        &files,
        |e| {
            chat(e, &[("agent-session-id", GOOSE_ROOT)]);
            chat(e, &[("agent-session-id", GOOSE_KID)]);
        },
    );
    let t = call_table(&dir);
    assert_eq!(t.threads.len(), 2);
    assert!(t.threads.iter().all(|x| x.parent.is_none()));
    assert_eq!(t.children, None);
}
