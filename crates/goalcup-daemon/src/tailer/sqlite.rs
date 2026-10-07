//! The SQLite tailer: rows out of a harness's own database, read-only and WAL-safe (D19).
//!
//! OpenCode keeps its `event`, `message` and `part` stores in one SQLite file that the
//! harness is writing while the daemon reads it. Three properties make that safe, and each
//! one is here because leaving it out cost a measured run:
//!
//!   * **read-only, and not `immutable`.** The connection is
//!     `file:<path>?mode=ro` opened with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI`. It is
//!     deliberately not `immutable=1`: `immutable` tells SQLite the file cannot change and
//!     to ignore the `-wal` sidecar, and every row a tracked harness has written in WAL
//!     mode and not yet checkpointed lives in exactly that sidecar. A tracker that read
//!     zero rows would still pass a test whose fixture had been checkpointed first.
//!     `nolock` is not used either: it would let this reader see a torn page.
//!   * **the paging is in SQL.** `WHERE _rid > ?1 ORDER BY _rid LIMIT ?2`, bound to
//!     [`SqliteTailer::last_rid`]. Paging without the `WHERE` clause, skipping rows already
//!     seen in Rust, would fetch the same first [`MAX_POLL_ROWS`] + 1 rows on every poll once
//!     the table held more than that, and emit nothing for ever: total capture loss a few
//!     seconds into a 168-hour run.
//!   * **bounded memory.** At most [`MAX_POLL_ROWS`] rows per poll and never a whole-table
//!     read. A poll that fills its page sets [`TailBatch::more_available`], which is not a
//!     notice — nothing went wrong, and the next poll continues from where this one stopped.
//!   * **the file's identity is checked on every poll.** The connection is held across
//!     polls, so a database that was deleted and recreated would be read through the old,
//!     unlinked inode for ever: zero rows, no notice, and a poll loop that looks healthy
//!     while capture has stopped. The device and inode are stated instead, and a change
//!     reopens the connection, chains [`TailNotice::FileReplaced`] and starts the
//!     paging again — a recreated database is a new rowid space, and a cursor that kept
//!     its old high-water mark would swallow the whole of it in silence.
//!
//! **An in-place `UPDATE` or `DELETE` is unwitnessed, and that is accepted for v1.**
//! The paging above moves strictly forward by rowid, so a row rewritten where it
//! sits — OpenCode moves `message.time_updated`, and
//! `spikes/opencode/fixtures/db-message-part-main.jsonl` line 1 carries the field — or
//! removed outright is never read a second time and chains no notice. It is the one rewrite
//! class with no on-disk signature, unlike a truncation, a replaced inode or a recreated
//! database, each of which this reader states. Closing it means re-reading rows already
//! paged past, which is a whole-table read on every poll and the opposite of the bound this
//! reader is built on. So v1 accepts it as a known gap, stated here and in the cheat
//! checklist, rather than having the daemon invent a signature it cannot observe.
//!
//! **A missing or busy database is a [`TailNotice::StoreUnavailable`], never an `Err`.** The
//! store does not exist until the harness starts, and one unreadable store must not stop the
//! poll of every other one. The notice is a health signal about the daemon's own reach; it
//! chains no `TailNotice` envelope, precisely so that a database the daemon could not open
//! is never read as an on-disk signature of a rewritten transcript.
//!
//! That rule is the whole module's, not this reader's alone: [`super::store_unavailable`] is
//! how the line and directory readers state the same fact about one file of a glob, so no
//! reader here turns a path it could not read into an error that discards what the same poll
//! already captured.
//!
//! Nothing here interprets a rule. The tailer reports the rows it read and the number it
//! read them by; what a row *means* is `goalcup-verifier`'s.

use super::{CapturedRecord, TailBatch, TailNotice, Tailer, stat};
use crate::cursor::{CursorState, cursor_type_name};
use crate::{DaemonError, MAX_POLL_ROWS};
use base64::Engine;
use goalcup_protocol::manifest::{CursorType, Store};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

/// The column the paging subquery projects the rowid into. It rides along in the payload —
/// at the top level for a flat row, inside `row` for an envelope — because a record that
/// does not carry the number it was read by cannot be checked against the cursor that
/// claims to have read it.
const RID: &str = "_rid";

/// How long a poll waits for a lock before giving up and reporting the store unavailable.
/// A WAL reader does not block behind a writer at all; this covers a database still in
/// rollback-journal mode, and it is short because a poll that waited would delay every
/// other store behind it.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Which JSON shape a store's rows ship in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadShape {
    /// `{db, table, row}`.
    Envelope,
    /// The row's columns at the top level.
    Flat,
}

/// The shape a store's rows ship in — **per store, not per tailer**: one connector's
/// `event` and `part` stores go out in different shapes from the same reader.
pub fn payload_shape(store: &Store) -> PayloadShape {
    match store.name.as_str() {
        "message" | "part" | "session" => PayloadShape::Flat,
        _ => PayloadShape::Envelope,
    }
}

pub struct SqliteTailer {
    name: String,
    /// The database file. Also the `db` field of an envelope payload and every record's
    /// `source_path`, and the path every error and notice from this store carries.
    path: String,
    table: String,
    shape: PayloadShape,
    /// The cursor type the manifest declares. Only `rowid` and `sqlite_pk` reach here;
    /// anything else is refused in [`SqliteTailer::new`].
    kind: CursorType,
    /// The column a `sqlite_pk` store declares (`id`). `None` for a `rowid` store.
    pk_column: Option<String>,
    /// Opened on the first poll and kept, because the store does not exist until the
    /// harness starts and a daemon that opened every database at startup would refuse to
    /// run before the harness did. Dropped again whenever SQLite reports a failure, so the
    /// next poll reopens rather than nursing a connection to a file that has been replaced.
    conn: Option<Connection>,
    /// The device and inode the open connection is reading, as of the last poll that found
    /// the file. `None` until the first poll finds it. It is deliberately kept while the
    /// file is missing, so a database that goes away and comes back is compared against
    /// what was there before rather than adopted as if it were new.
    file_id: Option<FileId>,
    /// **The paging key, and always a rowid — whatever cursor the manifest declares.**
    /// Say it here so a later reader does not "fix" the paging to run off the declared
    /// column: OpenCode's `message` and `part` declare `sqlite_pk` on an `id` that is TEXT,
    /// `WHERE id > 'msg_…'` is a string comparison over ids that are not ordered, and the
    /// no-`WHERE` alternative is the total-capture-loss defect D19 warns about.
    last_rid: i64,
    /// The number the declared cursor reports: the rowid for a `rowid` store, and for a
    /// `sqlite_pk` store the declared column when it holds an integer. When it does not —
    /// OpenCode's ids are TEXT — it is the rowid, which is why the two numbers never disagree
    /// across a restart.
    last_key: i64,
    records: u64,
}

impl SqliteTailer {
    pub fn new(store: &Store, path: String) -> Result<SqliteTailer, DaemonError> {
        let Some(table) = store.table.clone() else {
            return Err(DaemonError::Missing(format!(
                "store `{}` is mode `sqlite` and declares no `table`: there is nothing to \
                 select from",
                store.name
            )));
        };
        // The table name is the one part of the statement that cannot be a bound parameter,
        // so it is checked against an identifier rather than quoted and hoped for. A
        // manifest is a file on the participant's machine.
        if !is_plain_identifier(&table) {
            return Err(DaemonError::Refused(format!(
                "store `{}` names table `{table}`, which is not a plain identifier: a table \
                 name is interpolated into the statement and cannot be a bound parameter",
                store.name
            )));
        }
        let (kind, pk_column) = match store.cursor.kind {
            CursorType::Rowid => (CursorType::Rowid, None),
            CursorType::SqlitePk => (CursorType::SqlitePk, store.cursor.column.clone()),
            other => {
                return Err(DaemonError::Guard(format!(
                    "store `{}` is mode `sqlite` and declares cursor type `{}`; a SQLite \
                     store is paged by `rowid` or `sqlite_pk`",
                    store.name,
                    cursor_type_name(other)
                )));
            }
        };
        Ok(SqliteTailer {
            name: store.name.clone(),
            path,
            table,
            shape: payload_shape(store),
            kind,
            pk_column,
            conn: None,
            file_id: None,
            // `-1` and not `0`: a row at rowid 0 exists, and `> last` must include it.
            last_rid: -1,
            last_key: -1,
            records: 0,
        })
    }

    pub fn table(&self) -> &str {
        &self.table
    }

    fn unavailable(&self, reason: String) -> TailNotice {
        TailNotice::StoreUnavailable {
            store: self.name.clone(),
            path: self.path.clone(),
            reason,
        }
    }

    /// The one statement this tailer runs. The inner select is the SQL spike 5 measured —
    /// it read 458 of 458 `event` rows with zero misses, where a global `event.seq` cursor
    /// would have dropped the child session's 42 — and the `WHERE` is the paging.
    fn sql(&self) -> String {
        format!(
            "SELECT * FROM (SELECT rowid AS {RID}, * FROM \"{}\") \
             WHERE {RID} > ?1 ORDER BY {RID} LIMIT ?2",
            self.table
        )
    }
}

impl Tailer for SqliteTailer {
    fn name(&self) -> &str {
        &self.name
    }

    fn cursor(&self) -> CursorState {
        match self.kind {
            CursorType::SqlitePk => CursorState::SqlitePk {
                last: self.last_key,
                records: self.records,
            },
            // `new` admits no third type.
            _ => CursorState::Rowid {
                last: self.last_key,
                records: self.records,
            },
        }
    }

    fn restore(&mut self, cursor: CursorState) -> Result<(), DaemonError> {
        let (last, records) = match (&cursor, self.kind) {
            (CursorState::Rowid { last, records }, CursorType::Rowid)
            | (CursorState::SqlitePk { last, records }, CursorType::SqlitePk) => (*last, *records),
            (other, _) => {
                return Err(DaemonError::Guard(format!(
                    "store `{}` is a `{}` SQLite tailer; it cannot resume from a `{:?}` cursor",
                    self.name,
                    cursor_type_name(self.kind),
                    other.kind()
                )));
            }
        };
        // The persisted number becomes the paging key as well as the declared one. For a
        // `rowid` store they are the same number by definition; for a `sqlite_pk` store
        // they are the same number because the declared column is either an INTEGER PRIMARY
        // KEY — which SQLite makes an alias for the rowid — or a TEXT id, in which case
        // `last_key` already fell back to the rowid when it was recorded.
        self.last_rid = last;
        self.last_key = last;
        self.records = records;
        Ok(())
    }

    fn poll(&mut self) -> Result<TailBatch, DaemonError> {
        let mut batch = TailBatch::empty();

        // The file's identity first, before anything is read through the connection this
        // tailer is holding. A deleted and recreated database is a different file at the
        // same path, and a held connection goes on reading the unlinked one without ever
        // saying so.
        match stat(std::path::Path::new(&self.path)) {
            Ok(Some(facts)) => {
                let now = FileId {
                    dev: facts.dev,
                    inode: facts.inode,
                };
                if let Some(before) = self.file_id
                    && before != now
                {
                    batch.notices.push(TailNotice::FileReplaced {
                        store: self.name.clone(),
                        path: self.path.clone(),
                        inode_before: Some(before.inode),
                        inode_after: Some(now.inode),
                        // The envelope carries one pair of offsets for every kind; for a
                        // SQLite store the number the tailer had reached is the rowid.
                        offset_before: self.last_rid.max(0) as u64,
                    });
                    // A new file is a new rowid space. A recreated database numbers its
                    // rows from one again, so a high-water mark carried over from the old
                    // one would page past every row the replacement ever holds — the whole
                    // store missing from the chain with nothing to show for it. `0` and
                    // not `-1`: the rows SQLite assigns start at 1.
                    self.conn = None;
                    self.last_rid = 0;
                    self.last_key = 0;
                }
                self.file_id = Some(now);
            }
            // Gone. The connection goes with it, and the recorded identity does not: the
            // file coming back is then compared against what was there before, which is
            // what makes a delete-and-recreate a replacement rather than a fresh start.
            Ok(None) => {
                self.conn = None;
                batch
                    .notices
                    .push(self.unavailable(format!("no database at {}", self.path)));
                return Ok(batch);
            }
            Err(e) => {
                self.conn = None;
                batch.notices.push(self.unavailable(e.to_string()));
                return Ok(batch);
            }
        }

        if self.conn.is_none() {
            match open_read_only(&self.path) {
                Ok(c) => {
                    // Checked at open, once per connection: `restore` makes the persisted
                    // `sqlite_pk` number the paging key, and that is only sound when the
                    // declared column really is an alias for the rowid.
                    if self.kind == CursorType::SqlitePk
                        && let Some(column) = self.pk_column.as_deref()
                        && let Err(reason) = check_rowid_alias(&c, &self.name, &self.table, column)
                    {
                        // `self.conn` is left unset, so a table the harness later creates
                        // with the right schema is checked again rather than refused for
                        // the life of the daemon.
                        batch.notices.push(self.unavailable(reason));
                        return Ok(batch);
                    }
                    self.conn = Some(c);
                }
                // The store does not exist until the harness starts. Not an error: the
                // daemon is running before the thing it watches, which is the normal order.
                Err(e) => {
                    batch.notices.push(self.unavailable(e.to_string()));
                    return Ok(batch);
                }
            }
        }
        let Some(conn) = self.conn.as_ref() else {
            // Unreachable — the block above either set it or returned. Stated rather than
            // unwrapped: `unwrap` is not allowed in this crate.
            batch
                .notices
                .push(self.unavailable("no connection".to_string()));
            return Ok(batch);
        };

        let rows = match read_page(
            conn,
            &self.sql(),
            &self.table,
            self.last_rid,
            self.pk_column.as_deref(),
        ) {
            Ok(rows) => rows,
            // A table the harness has not created yet, a database mid-recovery, a lock this
            // poll would not wait for, a paging key that is not a number. Same fact about
            // the same store: the daemon could not read it this time. The connection goes
            // with it, so the next poll reopens.
            Err(reason) => {
                self.conn = None;
                batch.notices.push(self.unavailable(reason));
                return Ok(batch);
            }
        };

        // A full page means there is more in the table than this poll took. Not a notice:
        // nothing went wrong, and the next poll continues from the rowid this one reached.
        batch.more_available = rows.len() == MAX_POLL_ROWS;

        for row in rows {
            self.last_rid = row.rid;
            self.last_key = row.key;
            let payload = match self.shape {
                PayloadShape::Flat => row.value.clone(),
                PayloadShape::Envelope => serde_json::json!({
                    "db": self.path,
                    "table": self.table,
                    "row": row.value.clone(),
                }),
            };
            // `bytes` are what the redactor is handed and enter no hash: the chain
            // link is `Envelope::chain_bytes()` over the redacted payload. A row
            // has no bytes on disk, so they are the canonical JSON of the payload — the
            // whole payload, envelope included, because the envelope carries the database
            // path and a redactor that never saw it could not redact it.
            let bytes = serde_json::to_vec(&payload)?;
            batch.records.push(CapturedRecord {
                store: self.name.clone(),
                source_path: self.path.clone(),
                payload,
                // The row itself, both shapes: the partitioner reads `type` and
                // `aggregate_id` off the row and not off a wrapper.
                value: row.value,
                bytes,
            });
        }

        self.records = self.records.saturating_add(batch.records.len() as u64);
        Ok(batch)
    }
}

/// **The one way this daemon opens a database it does not own.** `file:<path>?mode=ro`
/// with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI` and the short busy timeout above — the
/// three properties the module header argues for, in one place rather than at each call
/// site.
///
/// `pub(crate)` because [`crate::foreign`] reads a foreign harness's `session` table
/// through it. A second `Connection::open` in this crate would be a
/// second answer to "how does the daemon read somebody else's store", and the answer that
/// matters — read-only, not `immutable`, not `nolock` — is this one.
pub(crate) fn open_read_only(path: &str) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open_with_flags(
        format!("file:{}?mode=ro", uri_encode_path(path)),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(conn)
}

/// Which file the open connection is reading. Device as well as inode: an inode number is
/// unique only within one filesystem, so the pair is the identity and neither half is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    dev: u64,
    inode: u64,
}

/// Whether the column a `sqlite_pk` store pages by really is an alias for the rowid, as
/// [`Tailer::restore`] assumes when it makes the persisted number the paging key.
///
/// SQLite aliases exactly one thing: a single-column `INTEGER PRIMARY KEY`. An INTEGER
/// column that is anything else — not the key, or one of several — holds numbers of the
/// harness's own choosing, and resuming a page from one of those after a restart skips or
/// re-ships rows with nothing saying so. That is refused by name, as a health signal.
///
/// A column that is not declared INTEGER is untouched: OpenCode's `message.id` and
/// `part.id` are TEXT, the number was never theirs, and [`read_page`] already falls back
/// to the rowid. A column the table does not have at all is the same case — the fallback
/// is what runs — and so is a table that does not exist yet, which reports itself when the
/// page is read.
fn check_rowid_alias(
    conn: &Connection,
    store: &str,
    table: &str,
    column: &str,
) -> Result<(), String> {
    let err = |e: rusqlite::Error| e.to_string();
    // The table name is checked against an identifier in `new`, exactly as it is for the
    // paging statement; `PRAGMA` takes no bound parameter either.
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .map_err(err)?;
    let mut rows = stmt.query([]).map_err(err)?;
    let mut declared: Option<(String, i64)> = None;
    let mut pk_columns = 0usize;
    while let Some(row) = rows.next().map_err(err)? {
        let name: String = row.get("name").map_err(err)?;
        let ty: String = row.get("type").map_err(err)?;
        let pk: i64 = row.get("pk").map_err(err)?;
        if pk > 0 {
            pk_columns += 1;
        }
        if name == column {
            declared = Some((ty, pk));
        }
    }
    let Some((ty, pk)) = declared else {
        return Ok(());
    };
    if !ty.eq_ignore_ascii_case("INTEGER") || (pk == 1 && pk_columns == 1) {
        return Ok(());
    }
    Err(format!(
        "store `{store}` pages by `sqlite_pk` on column `{column}` of table `{table}`, \
         which is declared `{ty}` but is not a single-column INTEGER PRIMARY KEY (pk {pk} \
         of {pk_columns}): it is not an alias for the rowid, so a restored cursor would \
         page the table from a number that is not one"
    ))
}

/// One row, as the tailer read it.
struct Row {
    /// The paging key: always the rowid.
    rid: i64,
    /// The number the declared cursor reports.
    key: i64,
    value: serde_json::Value,
}

/// At most [`MAX_POLL_ROWS`] rows above `last_rid`, in rowid order. Every SQLite failure
/// travels out as an `Err` here and becomes a `StoreUnavailable` at the call site, so this
/// function never decides what a failure means.
fn read_page(
    conn: &Connection,
    sql: &str,
    table: &str,
    last_rid: i64,
    pk_column: Option<&str>,
) -> Result<Vec<Row>, String> {
    let err = |e: rusqlite::Error| e.to_string();
    let mut stmt = conn.prepare(sql).map_err(err)?;
    // Owned before the query borrows the statement, and read from the prepared statement
    // rather than from the table's declared schema, so `_rid` and every column the harness
    // has added since are named the way this statement actually returns them.
    let columns: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();

    let mut out = Vec::new();
    let mut rows = stmt
        .query(rusqlite::params![last_rid, MAX_POLL_ROWS as i64])
        .map_err(err)?;
    while let Some(row) = rows.next().map_err(err)? {
        let mut map = serde_json::Map::new();
        let mut rid: i64 = last_rid;
        let mut key: Option<i64> = None;
        for (i, name) in columns.iter().enumerate() {
            let raw = row.get_ref(i).map_err(err)?;
            // Column 0 and not "the column called `_rid`": `SELECT rowid AS _rid, *`
            // projects the rowid first, and a table that already had a column of that name
            // would otherwise give the statement two of them and let the harness's own
            // value stand in for the paging key.
            if i == 0 {
                // A table with a column of its own called `rowid` shadows SQLite's, so the
                // statement projects the harness's value into the paging key. Reported,
                // and not read past: `WHERE _rid > ?1` compares TEXT above every number,
                // so the key would never advance and this same page would come back on
                // every poll for the rest of the run — total capture loss that reads as a
                // healthy poll.
                let ValueRef::Integer(n) = raw else {
                    return Err(format!(
                        "table `{table}` returns a `{RID}` that is not an integer, so the \
                         paging key cannot advance and the same page would be read on \
                         every poll: the table has a column of its own named `rowid` or \
                         `{RID}` shadowing the one the statement pages by"
                    ));
                };
                rid = n;
            }
            if pk_column == Some(name.as_str())
                && let ValueRef::Integer(n) = raw
            {
                key = Some(n);
            }
            map.insert(name.clone(), json_of(raw));
        }
        // The paging key, stated last so it is the one that survives a table with a column
        // of the same name. Every record carries the number it was read by; a record that
        // did not could not be checked against the cursor claiming to have read it.
        map.insert(RID.to_string(), serde_json::Value::from(rid));
        out.push(Row {
            rid,
            // The declared column when it holds an integer; the rowid when it does not.
            // OpenCode's `message.id` and `part.id` are TEXT, and the rowid is the fallback, written
            // down here rather than left implicit.
            key: key.unwrap_or(rid),
            value: serde_json::Value::Object(map),
        });
    }
    Ok(out)
}

/// One column value as JSON.
///
/// The two SQLite types JSON has no room for go through base64: a BLOB always, and a TEXT
/// column whose bytes are not valid UTF-8. Base64 and not a lossy rendering, because
/// `String::from_utf8_lossy` maps distinct byte strings onto the same replacement
/// characters, and a chain that cannot distinguish two payloads cannot be evidence about
/// either. The committed fixtures hold this shape.
fn json_of(v: ValueRef<'_>) -> serde_json::Value {
    match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(n) => serde_json::Value::from(n),
        // A non-finite REAL has no JSON form. It becomes `null` rather than a string that
        // would read as text on the other side; SQLite cannot store NaN at all, and an
        // infinity here is a column no shipped connector writes.
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ValueRef::Text(b) => match std::str::from_utf8(b) {
            Ok(s) => serde_json::Value::String(s.to_string()),
            Err(_) => serde_json::Value::String(b64(b)),
        },
        ValueRef::Blob(b) => serde_json::Value::String(b64(b)),
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// `[A-Za-z_][A-Za-z0-9_]*`. Narrower than SQLite's own rules on purpose: every table the
/// shipped connectors name is one of these, and a name that is not gets a refusal rather
/// than a quoted interpolation whose escaping has to be argued about.
fn is_plain_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A filesystem path as the path part of a SQLite URI. `?` opens the query string and `#`
/// the fragment, and `%` introduces an escape, so a path holding any of them would be read
/// as something other than itself; everything outside the unreserved set is escaped rather
/// than that list being maintained by hand. `/` is kept, because it is the separator.
fn uri_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
