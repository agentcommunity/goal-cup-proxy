//! PRD-11 slice 8: the Hermes connector copies no session database (owner decision 1 Nov) and
//! still takes the delegation manifest and task logs.

use std::fs;
use std::path::{Path, PathBuf};

use goalcup::connector;
use goalcup::native::pattern::parse;
use goalcup::native::walk::{Row, Walker};
use goalcup::pipeline::Recorder;

fn rows(home: &Path) -> Vec<Row> {
    let c = connector::load("hermes-cli", None).unwrap();
    assert!(c.first_screen.as_deref().is_some_and(|s| !s.is_empty()));
    c.native
        .iter()
        .map(|n| Row {
            name: n.name.clone(),
            root: n.root.replace("{home}", home.to_str().unwrap()).into(),
            pattern: parse(&n.pattern).unwrap(),
        })
        .collect()
}

#[test]
fn hermes_has_no_state_row_and_keeps_the_delegation_rows() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let live = home.join("cache/delegation/live/run-1");
    fs::create_dir_all(&live).unwrap();
    fs::write(home.join("state.db"), b"x").unwrap();
    fs::write(live.join("manifest.json"), b"{}").unwrap();
    fs::write(live.join("task-1.log"), b"line\n").unwrap();

    let rows = rows(&home);
    let (rec, _) = Recorder::for_tests(1 << 20);
    let found = Walker::default().pass(&rec, &rows).found;
    let names: Vec<PathBuf> = found
        .iter()
        .map(|f| rows[f.row].root.join(&f.rel))
        .collect();
    assert!(!names.iter().any(|p| p.ends_with("state.db")), "{names:?}");
    assert!(names.contains(&live.join("manifest.json")), "{names:?}");
    assert!(names.contains(&live.join("task-1.log")), "{names:?}");
}
