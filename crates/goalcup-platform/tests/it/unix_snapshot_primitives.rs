use goalcup_platform::tracecap::TraceDir;
use goalcup_platform::{
    handle_budget::NativeHandleBudgetV1,
    sourcecap::{SourceDir, SourceLookup, SourceName},
};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "goalcup-snapshot-primitives-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn bounded_child_listing_is_sorted_and_refuses_a_limit_before_copying() {
    let fixture = Fixture::new();
    let root = TraceDir::create(fixture.path()).unwrap();
    for name in ["z", "a", "middle"] {
        std::fs::write(fixture.path().join(name), b"").unwrap();
    }

    assert_eq!(
        root.child_names(3).unwrap(),
        vec!["a".to_string(), "middle".to_string(), "z".to_string()]
    );
    assert!(root.child_names(2).is_err());
}

#[test]
fn source_root_journal_token_is_versioned_native_identity_not_debug_output() {
    let fixture = Fixture::new();
    let root = SourceDir::open_selected(fixture.path(), &NativeHandleBudgetV1::new()).unwrap();
    let token = root.identity().journal_token();

    assert!(token.starts_with("source-root-v1:unix:"), "{token}");
    assert!(!token.contains("SourceIdentity"), "{token}");
    #[cfg(target_os = "macos")]
    assert_eq!(token.split(':').count(), 6, "{token}");
}

#[test]
fn source_observations_expose_full_native_facts_for_pre_and_post_copy_checks() {
    let fixture = Fixture::new();
    std::fs::write(fixture.path().join("payload"), b"snapshot").unwrap();
    std::fs::set_permissions(
        fixture.path().join("payload"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let root = SourceDir::open_selected(fixture.path(), &NativeHandleBudgetV1::new()).unwrap();
    let name = SourceName::from_component("payload").unwrap();
    let observation = match root.observe(&name).unwrap() {
        SourceLookup::Present(value) => value,
        SourceLookup::Missing => panic!("fixture file disappeared"),
    };

    let root_facts = root.facts().unwrap();
    let file_facts = observation.facts();
    assert_eq!(file_facts.length, 8);
    assert_eq!(file_facts.mode, Some(0o700));
    assert_eq!(file_facts.posix_owner_executable, Some(true));
    assert!(file_facts.identity.starts_with("source-root-v1:unix:"));
    assert!(file_facts.modified_nanos >= 0);
    assert!(file_facts.changed_nanos >= 0);
    assert!(root_facts.identity.starts_with("source-root-v1:unix:"));
    assert_ne!(root_facts.identity, file_facts.identity);
}
