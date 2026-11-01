//! Hermes native rows (point 4): none matches `state.db*`, whose search index holds text out of
//! context that masking cannot see; the delegation manifest and task log are the only matches.

use std::fs;
use std::path::{Path, PathBuf};

use goalcup::connector;
use goalcup::native::pattern::parse;
use goalcup::native::walk::{Row, Walker};
use goalcup::pipeline::Recorder;

#[test]
fn hermes_rows_take_no_state_database_and_no_request_dump() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    fs::write(home.join("sessions/request_dump_x.json"), b"{}").unwrap();
    let live = home.join("cache/delegation/live/run-1");
    fs::create_dir_all(&live).unwrap();
    fs::write(live.join("manifest.json"), b"{}").unwrap();
    fs::write(live.join("task-1.log"), b"line\n").unwrap();
    for f in ["state.db", "state.db-wal", "state.db.fts_rebuild.lock"] {
        fs::write(home.join(f), b"SQLite format 3\0").unwrap();
    }
    let c = connector::load("hermes-cli", None).unwrap();
    let home_s = home.to_str().unwrap();
    let rows: Vec<Row> = c
        .native
        .iter()
        .map(|n| Row {
            name: n.name.clone(),
            root: Path::new(&n.root.replace("{home}", home_s)).into(),
            pattern: parse(&n.pattern).unwrap(),
        })
        .collect();
    let (rec, _) = Recorder::for_tests(1 << 20);
    let found = Walker::default().pass(&rec, &rows).found;
    let mut got: Vec<PathBuf> = found
        .iter()
        .map(|f| rows[f.row].root.join(&f.rel))
        .collect();
    let mut want = vec![live.join("manifest.json"), live.join("task-1.log")];
    got.sort();
    want.sort();
    assert_eq!(got, want, "{got:?}");
}
