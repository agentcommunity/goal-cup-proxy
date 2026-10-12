use goalcup_platform::tracecap::{ArtifactRoot, EntryArtifactRoot};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const SAFE: &[u8] = b"SAFE_ARTIFACT_BYTES";
const OUTSIDE: &[u8] = b"OUTSIDE_SENTINEL_BYTES";

struct Fixture {
    top: PathBuf,
    root: PathBuf,
    outside: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let top = std::env::temp_dir().join(format!(
            "goalcup-artifact-root-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = top.join("root");
        let outside = top.join("outside");
        fs::create_dir_all(root.join("artifacts/entry-0001")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("status-line.txt"), OUTSIDE).unwrap();
        Self { top, root, outside }
    }

    fn entry(&self) -> PathBuf {
        self.root.join("artifacts/entry-0001")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.top);
    }
}

#[test]
fn acquired_entry_keeps_trace_and_status_inside_after_artifacts_ancestor_swap() {
    let fixture = Fixture::new("ancestor-swap");
    let authority = EntryArtifactRoot::open(&fixture.root, "entry-0001").unwrap();
    let detached = fixture.top.join("detached-artifacts");
    fs::rename(fixture.root.join("artifacts"), &detached).unwrap();
    symlink(&fixture.outside, fixture.root.join("artifacts")).unwrap();

    authority
        .create_trace()
        .expect("create under acquired entry");
    authority.replace_status_line(SAFE).unwrap();

    assert_eq!(
        fs::read(detached.join("entry-0001/status-line.txt")).unwrap(),
        SAFE
    );
    assert!(detached.join("entry-0001/trace").is_dir());
    assert_eq!(
        fs::read(fixture.outside.join("status-line.txt")).unwrap(),
        OUTSIDE
    );
    assert!(!fixture.outside.join("entry-0001/trace").exists());
}

#[test]
fn fresh_trace_creation_refuses_an_existing_even_empty_final_trace() {
    let fixture = Fixture::new("exclusive-trace");
    fs::create_dir(fixture.entry().join("trace")).unwrap();
    let authority = EntryArtifactRoot::open(&fixture.root, "entry-0001").unwrap();

    assert!(authority.create_trace().is_err());
    authority
        .open_trace()
        .expect("restart opens existing trace");
}

#[test]
fn restart_trace_open_refuses_a_final_trace_swap_before_it_becomes_a_capability() {
    let fixture = Fixture::new("trace-before-open");
    let authority = EntryArtifactRoot::open(&fixture.root, "entry-0001").unwrap();
    symlink(&fixture.outside, fixture.entry().join("trace")).unwrap();

    assert!(
        authority.open_trace().is_err(),
        "restart must refuse the final trace symlink rather than open its target"
    );
    assert_eq!(
        fs::read(fixture.outside.join("status-line.txt")).unwrap(),
        OUTSIDE
    );
}

#[test]
fn acquired_final_trace_keeps_descriptor_writes_after_its_path_is_swapped() {
    let fixture = Fixture::new("trace-after-open");
    let authority = EntryArtifactRoot::open(&fixture.root, "entry-0001").unwrap();
    let trace = authority.create_trace().unwrap();
    let detached = fixture.top.join("detached-final-trace");
    fs::rename(fixture.entry().join("trace"), &detached).unwrap();
    fs::write(
        fixture.outside.join("trace.toml"),
        b"OUTSIDE_TRACE_SENTINEL",
    )
    .unwrap();
    symlink(&fixture.outside, fixture.entry().join("trace")).unwrap();

    trace
        .replace_descriptor(b"entry_id = 'retained'\n")
        .unwrap();

    assert_eq!(
        fs::read(detached.join("trace.toml")).unwrap(),
        b"entry_id = 'retained'\n"
    );
    assert_eq!(
        fs::read(fixture.outside.join("trace.toml")).unwrap(),
        b"OUTSIDE_TRACE_SENTINEL"
    );
}

#[test]
fn rooted_status_replace_and_remove_never_follow_a_live_entry_swap() {
    let fixture = Fixture::new("status-swap");
    let authority = EntryArtifactRoot::open(&fixture.root, "entry-0001").unwrap();
    authority.replace_status_line(SAFE).unwrap();
    let detached = fixture.top.join("detached-entry");
    fs::rename(fixture.entry(), &detached).unwrap();
    symlink(&fixture.outside, fixture.root.join("artifacts/entry-0001")).unwrap();

    authority.remove_status_line().unwrap();
    authority.replace_status_line(b"RETRIED_SAFE").unwrap();

    assert_eq!(
        fs::read(detached.join("status-line.txt")).unwrap(),
        b"RETRIED_SAFE"
    );
    assert_eq!(
        fs::read(fixture.outside.join("status-line.txt")).unwrap(),
        OUTSIDE
    );
}

#[test]
fn fresh_entry_creation_descends_from_the_configured_root_without_path_mkdirs() {
    let fixture = Fixture::new("fresh-entry");
    fs::remove_dir_all(fixture.root.join("artifacts")).unwrap();

    let authority = EntryArtifactRoot::create(&fixture.root, "entry-0002")
        .expect("the configured root creates artifacts and its absent entry by held handles");
    authority
        .create_trace()
        .expect("the new entry creates its trace exclusively");

    assert!(fixture.root.join("artifacts/entry-0002/trace").is_dir());
    assert!(
        EntryArtifactRoot::create(&fixture.root, "entry-0002").is_err(),
        "a duplicate fresh entry is refused rather than adopted"
    );
}

#[test]
fn selected_artifact_and_registry_components_never_follow_symlinks() {
    let fixture = Fixture::new("selected-components");
    let selected_link = fixture.top.join("selected-link");
    symlink(&fixture.outside, &selected_link).unwrap();
    assert!(
        ArtifactRoot::open(&selected_link).is_err(),
        "the selected artifact root itself is acquired without following"
    );

    let selected = fixture.top.join("selected");
    fs::create_dir(&selected).unwrap();
    let root = ArtifactRoot::open(&selected).unwrap();
    symlink(&fixture.outside, selected.join("registry")).unwrap();
    assert!(
        root.open_registry_file().is_err(),
        "the fixed registry parent is acquired relative to the selected root"
    );
    assert!(!fixture.outside.join("registry.redb").exists());

    fs::remove_file(selected.join("registry")).unwrap();
    fs::create_dir(selected.join("registry")).unwrap();
    let sentinel = fixture.outside.join("registry-sentinel.redb");
    fs::write(&sentinel, OUTSIDE).unwrap();
    symlink(&sentinel, selected.join("registry/registry.redb")).unwrap();
    assert!(
        root.open_registry_file().is_err(),
        "the registry leaf is opened without following"
    );
    assert_eq!(fs::read(sentinel).unwrap(), OUTSIDE);
}

#[test]
fn retained_artifact_root_rejects_path_swap_and_acquired_cleanup_stays_contained() {
    let fixture = Fixture::new("retained-root-cleanup");
    let selected = fixture.top.join("selected");
    fs::create_dir(&selected).unwrap();
    let root = ArtifactRoot::open(&selected).unwrap();
    let entry = root.create_entry("entry-cleanup").unwrap();
    entry.replace_launch_settings(b"bearer material").unwrap();
    fs::write(selected.join("artifacts/entry-cleanup/ordinary.txt"), SAFE).unwrap();

    let detached = fixture.top.join("detached-selected");
    fs::rename(&selected, &detached).unwrap();
    symlink(&fixture.outside, &selected).unwrap();

    assert!(root.create_entry("must-refuse").is_err());
    symlink(
        fixture.outside.join("status-line.txt"),
        detached.join("artifacts/entry-cleanup/outside-link"),
    )
    .unwrap();

    entry.remove_recursively().unwrap();

    assert!(!detached.join("artifacts/entry-cleanup").exists());
    assert!(!fixture.outside.join("artifacts").exists());
    assert_eq!(
        fs::read(fixture.outside.join("status-line.txt")).unwrap(),
        OUTSIDE
    );
}

#[test]
fn cleanup_rejects_a_named_entry_swap_without_touching_the_replacement() {
    let fixture = Fixture::new("cleanup-entry-swap");
    let root = ArtifactRoot::open(&fixture.root).unwrap();
    let entry = root.open_entry("entry-0001").unwrap();
    entry.replace_launch_settings(b"bearer material").unwrap();
    let detached = fixture.top.join("detached-entry-for-cleanup");
    fs::rename(fixture.entry(), &detached).unwrap();
    symlink(&fixture.outside, fixture.root.join("artifacts/entry-0001")).unwrap();

    assert!(entry.remove_recursively().is_err());
    assert_eq!(
        fs::read(fixture.outside.join("status-line.txt")).unwrap(),
        OUTSIDE
    );
    assert!(fixture.root.join("artifacts/entry-0001").is_symlink());
}
