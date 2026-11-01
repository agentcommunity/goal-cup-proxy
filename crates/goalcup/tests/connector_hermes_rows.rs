//! Hermes native rows (point 4): none matches `state.db*`, whose search index holds text out of
//! context that masking cannot see, and none reaches a request dump; the delegation manifest and
//! task log are the only rows.

use std::fs;
use std::path::Path;

use goalcup::connector;
use goalcup::native::pattern::parse;
use goalcup::native::walk::{Row, Walker};
use goalcup::pipeline::Recorder;

#[test]
fn hermes_rows_take_no_state_database() {
    let c = connector::load("hermes-cli", None).unwrap();
    let names: Vec<&str> = c.native.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["delegation-manifest", "delegation"]);
    assert!(c.native.iter().all(|n| !n.pattern.starts_with("state.db")));

    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    fs::write(home.join("sessions/request_dump_x.json"), b"{}").unwrap();
    fs::write(home.join("state.db"), b"SQLite format 3\0").unwrap();
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
    let names: Vec<String> = found
        .iter()
        .map(|f| rows[f.row].root.join(&f.rel).to_string_lossy().into_owned())
        .collect();
    assert!(
        !names
            .iter()
            .any(|p| p.contains("state.db") || p.contains("request_dump")),
        "{names:?}"
    );
}
