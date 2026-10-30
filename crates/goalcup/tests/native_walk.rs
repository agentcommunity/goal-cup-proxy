//! PRD-04 section 3 acceptance (points 2 and 3): one full walk finds every matching file, at any
//! depth, under a root made late, at scale, keeps no descriptor and never crosses a device.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use goalcup::native::pattern::parse;
use goalcup::native::walk::{Found, Row, Walker};
use goalcup::pipeline::{Call, Recorder};
use serde_json::Value;

fn row(name: &str, root: &Path, pattern: &str) -> Row {
    Row { name: name.into(), root: root.into(), pattern: parse(pattern).unwrap() }
}

fn rels(found: &[Found]) -> Vec<PathBuf> {
    found.iter().map(|f| f.rel.clone()).collect()
}

fn events(calls: &goalcup::pipeline::Calls) -> Vec<Value> {
    let all = calls.all().into_iter();
    all.filter_map(|c| if let Call::Event(v) = c { Some(v) } else { None }).collect()
}

#[test]
fn a_file_six_folders_deep_is_found() {
    let t = tempfile::tempdir().unwrap();
    let deep = t.path().join("a/b/c/d/e/f");
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("s.jsonl"), b"{}\n").unwrap();
    let (rec, _) = Recorder::for_tests(1 << 20);
    let found = Walker::default().pass(&rec, &[row("t", t.path(), "**/*.jsonl")]);
    assert_eq!(rels(&found), [PathBuf::from("a/b/c/d/e/f/s.jsonl")]);
    assert_eq!(found[0].row, 0);
}

#[test]
fn a_root_made_after_the_first_pass_is_found() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("later");
    let rows = [row("t", &root, "*.jsonl")];
    let (rec, calls) = Recorder::for_tests(1 << 20);
    let mut walker = Walker::default();
    assert!(walker.pass(&rec, &rows).is_empty());
    fs::create_dir(&root).unwrap();
    fs::write(root.join("s.jsonl"), b"{}\n").unwrap();
    assert_eq!(rels(&walker.pass(&rec, &rows)), [PathBuf::from("s.jsonl")]);
    assert!(events(&calls).is_empty(), "a missing root is skipped silently");
}

#[test]
fn twenty_thousand_files_in_four_thousand_folders_are_found_in_one_pass() {
    let t = tempfile::tempdir().unwrap();
    for i in 0..4000 {
        let dir = t.path().join(format!("p{}/s{}", i / 100, i % 100));
        fs::create_dir_all(&dir).unwrap();
        for j in 0..5 {
            fs::write(dir.join(format!("{j}.jsonl")), b"x").unwrap();
        }
    }
    let (rec, _) = Recorder::for_tests(1 << 20);
    let started = Instant::now();
    let found = Walker::default().pass(&rec, &[row("t", t.path(), "**/*.jsonl")]);
    eprintln!("one pass over 20,000 files took {:?}", started.elapsed());
    assert_eq!(found.len(), 20_000);
}

fn open_fds() -> usize {
    let dir = if cfg!(target_os = "linux") { "/proc/self/fd" } else { "/dev/fd" };
    fs::read_dir(dir).unwrap().count()
}

#[test]
fn no_descriptor_stays_open_after_a_pass() {
    let t = tempfile::tempdir().unwrap();
    for d in ["a/b/c", "a/x", "y"] {
        fs::create_dir_all(t.path().join(d)).unwrap();
        fs::write(t.path().join(d).join("s.jsonl"), b"x").unwrap();
    }
    let rows = [row("t", t.path(), "**/*.jsonl"), row("u", &t.path().join("a"), "*")];
    let (rec, _) = Recorder::for_tests(1 << 20);
    let mut walker = Walker::default();
    let before = open_fds();
    for _ in 0..20 {
        assert_eq!(walker.pass(&rec, &rows).len(), 3);
    }
    assert_eq!(open_fds(), before);
}

#[test]
fn a_folder_on_another_device_is_not_entered_and_named_once() {
    let dev = fs::symlink_metadata("/dev").unwrap().dev();
    let others: Vec<String> = fs::read_dir("/dev")
        .unwrap()
        .filter_map(|e| {
            let e = e.ok()?;
            let m = e.path().symlink_metadata().ok()?;
            (m.is_dir() && m.dev() != dev).then(|| e.file_name().to_string_lossy().into_owned())
        })
        .collect();
    if others.is_empty() {
        eprintln!("no folder under /dev is on another device here; nothing to check");
        return;
    }
    let rows = [row("dev", Path::new("/dev"), "**/*")];
    let (rec, calls) = Recorder::for_tests(1 << 20);
    let mut walker = Walker::default();
    let mut found = walker.pass(&rec, &rows);
    found.extend(walker.pass(&rec, &rows));
    let notes: Vec<Value> = events(&calls)
        .into_iter()
        .filter(|e| e["what"] == "native_other_device")
        .collect();
    for other in &others {
        assert!(!found.iter().any(|f| f.rel.starts_with(other)), "{other} was entered");
        let named = notes.iter().filter(|n| n["path"] == other.as_str()).count();
        assert_eq!(named, 1, "{other} must be named exactly once");
        assert!(notes.iter().all(|n| n["row"] == "dev"));
    }
}
