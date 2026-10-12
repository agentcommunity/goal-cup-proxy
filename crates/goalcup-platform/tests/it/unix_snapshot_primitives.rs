use goalcup_platform::tracecap::{EntryArtifactRoot, RefusalKind, TraceCapError, TraceDir};
use goalcup_platform::{
    handle_budget::NativeHandleBudgetV1,
    sourcecap::{SourceDir, SourceLookup, SourceName},
};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{TryRecvError, sync_channel};
use std::thread;
use std::time::Duration;

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
fn exclusive_private_object_promotion_never_replaces_an_existing_name() {
    let fixture = Fixture::new();
    let root = TraceDir::create(fixture.path()).unwrap();
    root.create_child_dir("private-staging").unwrap();
    assert_eq!(
        std::fs::metadata(fixture.path().join("private-staging"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o700
    );

    let mut staged = root.create_regular_exclusive("staged-object").unwrap();
    staged.write_all(b"retained bytes").unwrap();
    staged.sync_all().unwrap();
    drop(staged);
    root.sync().unwrap();

    root.promote_child_exclusive("staged-object", "final-object")
        .unwrap();
    assert_eq!(
        std::fs::read(fixture.path().join("final-object")).unwrap(),
        b"retained bytes"
    );
    assert_eq!(
        std::fs::metadata(fixture.path().join("final-object"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );

    let mut collision = root.create_regular_exclusive("collision").unwrap();
    collision.write_all(b"must survive collision").unwrap();
    collision.sync_all().unwrap();
    drop(collision);
    assert!(
        root.promote_child_exclusive("collision", "final-object")
            .is_err()
    );
    assert_eq!(
        std::fs::read(fixture.path().join("final-object")).unwrap(),
        b"retained bytes"
    );
    assert_eq!(
        std::fs::read(fixture.path().join("collision")).unwrap(),
        b"must survive collision"
    );
}

#[test]
fn staged_private_directory_promotion_is_exclusive_and_preserves_the_tree() {
    let fixture = Fixture::new();
    let root = TraceDir::create(fixture.path()).unwrap();
    let staging = root.create_child_dir("submission-staging").unwrap();
    let mut manifest = staging.create_regular_exclusive("manifest.json").unwrap();
    manifest.write_all(b"canonical manifest").unwrap();
    manifest.sync_all().unwrap();
    drop(manifest);
    staging.sync().unwrap();

    root.promote_child_dir_matching_exclusive("submission-staging", &staging, "submission")
        .unwrap();
    assert_eq!(
        std::fs::read(fixture.path().join("submission/manifest.json")).unwrap(),
        b"canonical manifest"
    );
    assert_eq!(
        std::fs::metadata(fixture.path().join("submission"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o700
    );

    root.create_child_dir("second-staging").unwrap();
    assert!(
        root.promote_child_dir_exclusive("second-staging", "submission")
            .is_err()
    );
    assert!(fixture.path().join("second-staging").is_dir());
}

#[test]
fn directory_promotion_refuses_when_named_stage_is_not_the_retained_stage() {
    let fixture = Fixture::new();
    let root = TraceDir::create(fixture.path()).unwrap();
    let expected_stage = root.create_child_dir("expected-stage").unwrap();
    root.create_child_dir("replacement-stage").unwrap();

    let error = root
        .promote_child_dir_matching_exclusive("replacement-stage", &expected_stage, "final")
        .unwrap_err();
    assert!(matches!(
        error,
        TraceCapError::Refused {
            reason: RefusalKind::IdentityChanged,
            ..
        }
    ));
    assert!(fixture.path().join("replacement-stage").is_dir());
    assert!(!fixture.path().join("final").exists());
}

#[test]
fn capture_lock_blocks_a_second_holder_until_the_first_drops() {
    let fixture = Fixture::new();
    let root = TraceDir::create(fixture.path()).unwrap();
    let first = root.acquire_capture_lock().unwrap();
    let lock_metadata = std::fs::metadata(fixture.path().join("capture.lock")).unwrap();
    assert!(lock_metadata.file_type().is_file());
    assert_eq!(lock_metadata.nlink(), 1);
    assert_eq!(lock_metadata.permissions().mode() & 0o7777, 0o600);
    let (attempted_tx, attempted_rx) = sync_channel(0);
    let (acquired_tx, acquired_rx) = sync_channel(0);
    let path = fixture.path().to_path_buf();
    let second = thread::spawn(move || {
        let root = TraceDir::open(&path).unwrap();
        attempted_tx.send(()).unwrap();
        let _lock = root.acquire_capture_lock().unwrap();
        acquired_tx.send(()).unwrap();
    });

    attempted_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the second caller reaches the blocking lock");
    assert!(matches!(acquired_rx.try_recv(), Err(TryRecvError::Empty)));

    drop(first);
    acquired_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the second caller proceeds after the first lock drops");
    second.join().unwrap();
}

#[test]
fn capture_lock_refuses_a_hardlinked_child() {
    let fixture = Fixture::new();
    let root = TraceDir::create(fixture.path()).unwrap();
    std::fs::write(fixture.path().join("capture.lock"), b"").unwrap();
    std::fs::hard_link(
        fixture.path().join("capture.lock"),
        fixture.path().join("capture-lock-alias"),
    )
    .unwrap();

    assert!(matches!(
        root.acquire_capture_lock(),
        Err(TraceCapError::Refused {
            reason: RefusalKind::MultipleLinks,
            ..
        })
    ));
}

#[test]
fn capture_lock_refuses_a_non_private_child() {
    let fixture = Fixture::new();
    let root = TraceDir::create(fixture.path()).unwrap();
    let lock = fixture.path().join("capture.lock");
    std::fs::write(&lock, b"").unwrap();
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert!(matches!(
        root.acquire_capture_lock(),
        Err(TraceCapError::Refused {
            reason: RefusalKind::NotPrivate,
            ..
        })
    ));
}

#[test]
fn bounded_child_listing_is_sorted_and_refuses_a_limit_before_copying() {
    let fixture = Fixture::new();
    let root = TraceDir::create(fixture.path()).unwrap();
    for name in ["z", "a", "middle"] {
        let file = root.create_regular_exclusive(name).unwrap();
        file.sync_all().unwrap();
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

#[test]
fn source_journal_is_private_once_only_and_keeps_its_original_bytes() {
    let fixture = Fixture::new();
    let configured = fixture.path().join("configured-root");
    std::fs::create_dir(&configured).unwrap();
    let artifact = EntryArtifactRoot::create(&configured, "entry-0001").unwrap();

    let mut journal = artifact.create_sealing_source().unwrap();
    journal
        .write_all(b"{\"identity\":\"source-root-v1:unix:1:2\"}")
        .unwrap();
    journal.sync_all().unwrap();
    drop(journal);
    assert!(artifact.create_sealing_source().is_err());
    let mut retained = String::new();
    artifact
        .read_sealing_source()
        .unwrap()
        .unwrap()
        .read_to_string(&mut retained)
        .unwrap();
    assert_eq!(retained, "{\"identity\":\"source-root-v1:unix:1:2\"}");
    assert_eq!(
        std::fs::metadata(configured.join("artifacts/entry-0001/sealing-source.json"),)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
}

#[test]
fn source_journal_install_is_atomic_once_only_and_cleans_its_owned_loser() {
    let temporary = Fixture::new();
    let artifact = EntryArtifactRoot::create(temporary.path(), "entry-atomic-source").unwrap();

    artifact.install_sealing_source_once(b"first").unwrap();
    let error = artifact.install_sealing_source_once(b"second").unwrap_err();

    assert!(matches!(
        error,
        TraceCapError::Refused {
            reason: RefusalKind::EntryAlreadyExists,
            ..
        }
    ));
    assert_eq!(
        std::fs::read(
            temporary
                .path()
                .join("artifacts/entry-atomic-source/sealing-source.json")
        )
        .unwrap(),
        b"first"
    );
    assert_eq!(
        std::fs::read_dir(temporary.path().join("artifacts/entry-atomic-source"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".sealing-source."))
            .count(),
        0
    );
}
