#![cfg(feature = "v1-compat")]
use goalcup_daemon::cursor::{CursorState, SavedCursor};
use goalcup_daemon::tailer::{DirTailer, JsonlTailer, SqliteTailer, TailNotice, Tailer};
use goalcup_protocol::manifest::{Cursor, CursorType, Store, StoreMode};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const VOLUME_BYTES: u64 = 256 * 1024 * 1024;
const JSONL_RECORDS: u64 = 16_384;
const JSONL_PAYLOAD_BYTES: usize = 16 * 1024;
const DIR_FILES: u64 = 512;
const DIR_PAYLOAD_BYTES: usize = 512 * 1024;
const SQLITE_ROWS: u64 = 16_384;
const SQLITE_PAYLOAD_BYTES: usize = 16 * 1024;

static VOLUME_TEST_LOCK: Mutex<()> = Mutex::new(());

struct FixtureRoot {
    root: PathBuf,
    _temporary: Option<tempfile::TempDir>,
}

impl FixtureRoot {
    fn new() -> Self {
        if let Some(root) = std::env::var_os("SOURCE_VOLUME_FIXTURE_ROOT") {
            return Self {
                root: PathBuf::from(root),
                _temporary: None,
            };
        }
        let temporary = tempfile::tempdir().expect("temporary synthetic source root");
        Self {
            root: temporary.path().to_path_buf(),
            _temporary: Some(temporary),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn require_prepared_or_temporary(&self, path: &Path) {
        if self._temporary.is_none()
            && std::env::var_os("SOURCE_VOLUME_PREPARE").as_deref() != Some("1".as_ref())
        {
            panic!(
                "measured reader requires prepared fixture {}, refusing in-child generation",
                path.display()
            );
        }
    }
}

#[derive(Clone)]
struct SourceSpec {
    name: &'static str,
    count: u64,
    payload: String,
}

impl SourceSpec {
    fn new(name: &'static str, count: u64, payload_bytes: usize) -> Self {
        let marker = match name {
            "jsonl" => b'j',
            "directory" => b'd',
            "sqlite" => b's',
            _ => unreachable!("fixed source names"),
        };
        Self {
            name,
            count,
            payload: String::from_utf8(vec![marker; payload_bytes])
                .expect("ASCII synthetic payload"),
        }
    }

    fn expected_digest(&self) -> String {
        let mut digest = Sha256::new();
        for id in 0..self.count {
            digest.update(id.to_le_bytes());
            digest.update(self.payload.as_bytes());
        }
        hex::encode(digest.finalize())
    }
}

struct Audit {
    source: &'static str,
    expected_count: u64,
    expected_digest: String,
    next_id: u64,
    observed: u64,
    digest: Sha256,
}

impl Audit {
    fn new(spec: &SourceSpec) -> Self {
        Self {
            source: spec.name,
            expected_count: spec.count,
            expected_digest: spec.expected_digest(),
            next_id: 0,
            observed: 0,
            digest: Sha256::new(),
        }
    }

    fn observe(&mut self, id: u64, payload: &str) -> Result<(), String> {
        if id != self.next_id {
            return Err(format!(
                "{} source order/capture corruption: expected id {}, observed {id}",
                self.source, self.next_id
            ));
        }
        self.digest.update(id.to_le_bytes());
        self.digest.update(payload.as_bytes());
        self.next_id += 1;
        self.observed += 1;
        Ok(())
    }

    fn finish(self) -> Result<String, String> {
        if self.observed != self.expected_count {
            return Err(format!(
                "{} source omission/duplication: expected {} records, observed {}",
                self.source, self.expected_count, self.observed
            ));
        }
        let actual = hex::encode(self.digest.finalize());
        if actual != self.expected_digest {
            return Err(format!(
                "{} source digest mismatch: expected {}, observed {actual}",
                self.source, self.expected_digest
            ));
        }
        Ok(actual)
    }
}

fn jsonl_spec() -> SourceSpec {
    SourceSpec::new("jsonl", JSONL_RECORDS, JSONL_PAYLOAD_BYTES)
}

fn dir_spec() -> SourceSpec {
    SourceSpec::new("directory", DIR_FILES, DIR_PAYLOAD_BYTES)
}

fn sqlite_spec() -> SourceSpec {
    SourceSpec::new("sqlite", SQLITE_ROWS, SQLITE_PAYLOAD_BYTES)
}

fn jsonl_path(root: &FixtureRoot) -> PathBuf {
    root.path("jsonl/main.jsonl")
}

fn dir_path(root: &FixtureRoot) -> PathBuf {
    root.path("directory")
}

fn sqlite_path(root: &FixtureRoot) -> PathBuf {
    root.path("sqlite/events.db")
}

fn ensure_jsonl(root: &FixtureRoot, spec: &SourceSpec) -> PathBuf {
    let path = jsonl_path(root);
    if path.exists() {
        assert!(fs::metadata(&path).unwrap().len() >= VOLUME_BYTES);
        return path;
    }
    root.require_prepared_or_temporary(&path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = BufWriter::with_capacity(1024 * 1024, File::create(&path).unwrap());
    for id in 0..spec.count {
        writeln!(writer, r#"{{"id":{id},"payload":"{}"}}"#, spec.payload).unwrap();
    }
    writer.flush().unwrap();
    assert!(fs::metadata(&path).unwrap().len() >= VOLUME_BYTES);
    path
}

fn source_bytes(path: &Path) -> u64 {
    if path.is_dir() {
        fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum()
    } else {
        fs::metadata(path).unwrap().len()
    }
}

fn ensure_directory(root: &FixtureRoot, spec: &SourceSpec) -> PathBuf {
    let path = dir_path(root);
    if path.exists() {
        assert!(source_bytes(&path) >= VOLUME_BYTES);
        return path;
    }
    root.require_prepared_or_temporary(&path);
    fs::create_dir_all(&path).unwrap();
    for id in 0..spec.count {
        let file = path.join(format!("{id:06}.json"));
        let mut writer = BufWriter::with_capacity(1024 * 1024, File::create(file).unwrap());
        write!(writer, r#"{{"id":{id},"payload":"{}"}}"#, spec.payload).unwrap();
        writer.flush().unwrap();
    }
    assert!(source_bytes(&path) >= VOLUME_BYTES);
    path
}

fn ensure_sqlite(root: &FixtureRoot, spec: &SourceSpec) -> PathBuf {
    let path = sqlite_path(root);
    if path.exists() {
        assert!(fs::metadata(&path).unwrap().len() >= VOLUME_BYTES);
        return path;
    }
    root.require_prepared_or_temporary(&path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    connection
        .execute("CREATE TABLE event (id INTEGER, payload TEXT)", [])
        .unwrap();
    let tx = connection.transaction().unwrap();
    {
        let mut statement = tx
            .prepare("INSERT INTO event (id, payload) VALUES (?1, ?2)")
            .unwrap();
        for id in 0..spec.count {
            statement
                .execute(rusqlite::params![id, spec.payload])
                .unwrap();
        }
    }
    tx.commit().unwrap();
    connection
        .pragma_update(None, "wal_checkpoint", "TRUNCATE")
        .unwrap();
    drop(connection);
    assert!(fs::metadata(&path).unwrap().len() >= VOLUME_BYTES);
    path
}

fn sqlite_store() -> Store {
    Store {
        name: "event".to_string(),
        mode: StoreMode::Sqlite,
        path_template: "{config_dir}/data/opencode/opencode.db".to_string(),
        cursor: Cursor {
            kind: CursorType::Rowid,
            column: Some("rowid".to_string()),
            group_column: None,
        },
        table: Some("event".to_string()),
        partition_key: Some("session.directory".to_string()),
        partition: None,
        unpartitioned_reason: None,
        subagent_path_template: None,
        spill_path_template: None,
    }
}

fn offset_of(cursor: &CursorState) -> u64 {
    match cursor {
        CursorState::InodeOffset { files, .. } => files.values().map(|file| file.offset).sum(),
        other => panic!("unexpected cursor: {other:?}"),
    }
}

fn persist_and_reload_cursor(root: &FixtureRoot, store: &str, cursor: CursorState) -> CursorState {
    let path = root.path("artifacts/cursors.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut cursors = BTreeMap::new();
    cursors.insert(
        store.to_string(),
        SavedCursor {
            cursor,
            last_leg1_seq: Some(17),
        },
    );
    fs::write(&path, serde_json::to_vec(&cursors).unwrap()).unwrap();
    let restored: BTreeMap<String, SavedCursor> =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    restored
        .get(store)
        .expect("the synthetic cursors.json keeps this store")
        .cursor
        .clone()
}

#[test]
fn prepare_source_volume_fixtures() {
    let _lock = VOLUME_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = FixtureRoot::new();
    let jsonl = ensure_jsonl(&root, &jsonl_spec());
    let directory = ensure_directory(&root, &dir_spec());
    let sqlite = ensure_sqlite(&root, &sqlite_spec());
    eprintln!(
        "SOURCE_VOLUME_PREP jsonl_bytes={} directory_bytes={} sqlite_bytes={}",
        source_bytes(&jsonl),
        source_bytes(&directory),
        source_bytes(&sqlite),
    );
}

#[test]
fn audit_rejects_omission_duplication_and_order_corruption() {
    let spec = SourceSpec::new("jsonl", 3, 8);
    let mut omitted = Audit::new(&spec);
    omitted.observe(0, &spec.payload).unwrap();
    omitted.observe(1, &spec.payload).unwrap();
    assert!(
        omitted
            .finish()
            .unwrap_err()
            .contains("omission/duplication")
    );
    let mut duplicated = Audit::new(&spec);
    duplicated.observe(0, &spec.payload).unwrap();
    assert!(
        duplicated
            .observe(0, &spec.payload)
            .unwrap_err()
            .contains("order/capture")
    );
    let mut reordered = Audit::new(&spec);
    assert!(
        reordered
            .observe(1, &spec.payload)
            .unwrap_err()
            .contains("order/capture")
    );
}

#[test]
fn jsonl_volume_drains_bounded_restarts_and_witnesses_append_and_rotation() {
    let _lock = VOLUME_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = FixtureRoot::new();
    let spec = jsonl_spec();
    let path = ensure_jsonl(&root, &spec);
    let bytes = source_bytes(&path);
    let mut tailer = JsonlTailer::new("jsonl", path.display().to_string(), None);
    let mut audit = Audit::new(&spec);
    let mut polls = 0_u64;
    let mut restarted = false;
    let mut last_offset = 0_u64;
    loop {
        let batch = tailer.poll().unwrap();
        polls += 1;
        let offset = offset_of(&tailer.cursor());
        assert!(offset - last_offset <= goalcup_daemon::MAX_POLL_BYTES);
        last_offset = offset;
        for record in &batch.records {
            audit
                .observe(
                    record.value["id"].as_u64().unwrap(),
                    record.value["payload"].as_str().unwrap(),
                )
                .unwrap();
        }
        assert!(batch.notices.is_empty());
        if !restarted && polls == 3 {
            let saved = persist_and_reload_cursor(&root, "jsonl", tailer.cursor());
            drop(tailer);
            tailer = JsonlTailer::new("jsonl", path.display().to_string(), None);
            tailer.restore(saved).unwrap();
            restarted = true;
        }
        if !batch.more_available {
            break;
        }
        assert!(polls < 1024);
    }
    let digest = audit.finish().unwrap();
    assert!(restarted && polls > 3);
    assert_eq!(last_offset, bytes);
    assert!(tailer.poll().unwrap().records.is_empty());
    let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
    write!(writer, r#"{{"id":999999,"payload":"partial"#).unwrap();
    writer.flush().unwrap();
    assert!(tailer.poll().unwrap().records.is_empty());
    writeln!(writer, r#""}}"#).unwrap();
    writer.flush().unwrap();
    let appended = tailer.poll().unwrap();
    assert_eq!(appended.records.len(), 1);
    assert_eq!(appended.records[0].value["id"], 999999);
    let replacement = root.path("jsonl/replacement.jsonl");
    fs::write(&replacement, b"{\"id\":1000000,\"payload\":\"rotated\"}\n").unwrap();
    fs::rename(&replacement, &path).unwrap();
    let rotated = tailer.poll().unwrap();
    assert!(
        rotated
            .notices
            .iter()
            .any(|n| matches!(n, TailNotice::FileReplaced { .. }))
    );
    assert!(
        rotated
            .notices
            .iter()
            .any(|n| matches!(n, TailNotice::Reread { .. }))
    );
    assert_eq!(rotated.records.len(), 1);
    assert_eq!(rotated.records[0].value["id"], 1_000_000);
    eprintln!(
        "SOURCE_VOLUME jsonl source_bytes={bytes} records={} polls={polls} digest={digest}",
        spec.count
    );
}

#[test]
fn directory_volume_drains_bounded_and_restarts_without_duplicate_files() {
    let _lock = VOLUME_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = FixtureRoot::new();
    let spec = dir_spec();
    let path = ensure_directory(&root, &spec);
    let bytes = source_bytes(&path);
    let mut tailer = DirTailer::new("directory", path.display().to_string());
    let mut audit = Audit::new(&spec);
    let mut polls = 0_u64;
    let mut restarted = false;
    let mut largest_batch = 0_u64;
    loop {
        let batch = tailer.poll().unwrap();
        polls += 1;
        let batch_bytes: u64 = batch.records.iter().map(|r| r.bytes.len() as u64).sum();
        // The directory reader admits a complete supported file whenever its cooperative
        // budget is non-zero.  Thus one final file can cross the nominal poll budget, but
        // no poll can exceed that budget plus MAX_FILE_BYTES (the per-file hard bound).
        assert!(batch_bytes < goalcup_daemon::MAX_POLL_BYTES + goalcup_daemon::MAX_FILE_BYTES);
        largest_batch = largest_batch.max(batch_bytes);
        for record in &batch.records {
            assert!(record.bytes.len() as u64 <= goalcup_daemon::MAX_FILE_BYTES);
            audit
                .observe(
                    record.value["id"].as_u64().unwrap(),
                    record.value["payload"].as_str().unwrap(),
                )
                .unwrap();
        }
        assert!(batch.notices.is_empty());
        if !restarted && polls == 3 {
            let saved = persist_and_reload_cursor(&root, "directory", tailer.cursor());
            drop(tailer);
            tailer = DirTailer::new("directory", path.display().to_string());
            tailer.restore(saved).unwrap();
            restarted = true;
        }
        if !batch.more_available {
            break;
        }
        assert!(polls < 1024);
    }
    let digest = audit.finish().unwrap();
    assert!(restarted && polls > 3);
    assert!(tailer.poll().unwrap().records.is_empty());
    eprintln!(
        "SOURCE_VOLUME directory source_bytes={bytes} records={} polls={polls} max_batch_bytes={largest_batch} digest={digest}",
        spec.count
    );
}

#[test]
fn sqlite_volume_drains_bounded_and_restarts_without_duplicate_rows() {
    let _lock = VOLUME_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = FixtureRoot::new();
    let spec = sqlite_spec();
    let path = ensure_sqlite(&root, &spec);
    let bytes = source_bytes(&path);
    let store = sqlite_store();
    let mut tailer = SqliteTailer::new(&store, path.display().to_string()).unwrap();
    let mut audit = Audit::new(&spec);
    let mut polls = 0_u64;
    let mut restarted = false;
    loop {
        let batch = tailer.poll().unwrap();
        polls += 1;
        assert!(batch.records.len() <= goalcup_daemon::MAX_POLL_ROWS);
        for record in &batch.records {
            audit
                .observe(
                    record.value["id"].as_u64().unwrap(),
                    record.value["payload"].as_str().unwrap(),
                )
                .unwrap();
        }
        assert!(batch.notices.is_empty());
        if !restarted && polls == 3 {
            let saved = persist_and_reload_cursor(&root, "event", tailer.cursor());
            drop(tailer);
            tailer = SqliteTailer::new(&store, path.display().to_string()).unwrap();
            tailer.restore(saved).unwrap();
            restarted = true;
        }
        if !batch.more_available {
            break;
        }
        assert!(polls < 1024);
    }
    let digest = audit.finish().unwrap();
    assert!(restarted && polls > 3);
    assert!(tailer.poll().unwrap().records.is_empty());
    eprintln!(
        "SOURCE_VOLUME sqlite source_bytes={bytes} records={} polls={polls} digest={digest}",
        spec.count
    );
}
