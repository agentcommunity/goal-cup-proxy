#![cfg(unix)]

use goalcup_platform::tracecap::{FixedDocument, TraceCapError, TraceDir};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::current_dir()
            .unwrap()
            .join("target/unix-tracecap-native-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&path).unwrap();
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
fn held_root_owns_fixed_and_descriptor_writes_after_path_exchange() {
    let tmp = Fixture::new();
    let path = tmp.path().join("trace");
    let detached = tmp.path().join("detached");
    let root = TraceDir::create(&path).unwrap();
    std::fs::rename(&path, &detached).unwrap();
    std::fs::create_dir(&path).unwrap();

    root.create_fixed(FixedDocument::Receipt)
        .unwrap()
        .write_all_sync_checked(b"receipt")
        .unwrap();
    root.replace_descriptor(b"entry_id = 'e1'\n").unwrap();

    assert_eq!(
        std::fs::read(detached.join("receipt.json")).unwrap(),
        b"receipt"
    );
    assert_eq!(
        std::fs::read(detached.join("trace.toml")).unwrap(),
        b"entry_id = 'e1'\n"
    );
    assert!(!path.join("receipt.json").exists());
    assert!(!path.join("trace.toml").exists());
}

#[test]
fn fixed_creation_refuses_existing_links_without_touching_their_targets() {
    let tmp = Fixture::new();
    let path = tmp.path().join("trace");
    let root = TraceDir::create(&path).unwrap();
    let outside = tmp.path().join("outside");
    std::fs::write(&outside, b"outside").unwrap();
    std::fs::hard_link(&outside, path.join("receipt.json")).unwrap();

    assert!(root.create_fixed(FixedDocument::Receipt).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside");

    symlink(&outside, path.join("baseline.json")).unwrap();
    assert!(root.create_fixed(FixedDocument::Baseline).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
}

#[test]
fn descriptor_replaces_a_link_without_following_it_and_preserves_regular_mode() {
    let tmp = Fixture::new();
    let path = tmp.path().join("trace");
    let root = TraceDir::create(&path).unwrap();
    let outside = tmp.path().join("outside");
    std::fs::write(&outside, b"outside").unwrap();
    symlink(&outside, path.join("trace.toml")).unwrap();
    root.replace_descriptor(b"first").unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    assert_eq!(std::fs::read(path.join("trace.toml")).unwrap(), b"first");

    std::fs::set_permissions(
        path.join("trace.toml"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    root.replace_descriptor(b"second").unwrap();
    assert_eq!(std::fs::read(path.join("trace.toml")).unwrap(), b"second");
    assert_eq!(
        std::fs::metadata(path.join("trace.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
}

#[test]
fn descriptor_refuses_a_hardlinked_regular_target_without_changing_either_alias() {
    let tmp = Fixture::new();
    let root = TraceDir::create(tmp.path()).unwrap();
    let outside = tmp.path().join("outside");
    std::fs::write(&outside, b"old").unwrap();
    std::fs::hard_link(&outside, tmp.path().join("trace.toml")).unwrap();
    assert!(root.replace_descriptor(b"new").is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"old");
    assert_eq!(
        std::fs::read(tmp.path().join("trace.toml")).unwrap(),
        b"old"
    );
}

#[test]
fn retained_writer_survives_leaf_exchange_but_mismatched_reader_poisons_it() {
    let tmp = Fixture::new();
    let path = tmp.path().join("trace");
    let root = TraceDir::create(&path).unwrap();
    let mut chain = root.open_chain_for_create().unwrap();
    chain.append_checked(b"first\n").unwrap();

    let original = path.join("original-chain");
    let replacement = path.join("chain.jsonl");
    std::fs::rename(&replacement, &original).unwrap();
    std::fs::write(&replacement, b"replacement\n").unwrap();
    chain.append_checked(b"retained\n").unwrap();
    let original_before_refusal = std::fs::read(&original).unwrap();

    assert!(root.open_matching_reader(&mut chain).is_err());
    std::fs::remove_file(&replacement).unwrap();
    std::fs::rename(&original, &replacement).unwrap();
    assert!(matches!(
        chain.append_checked(b"after\n"),
        Err(TraceCapError::Poisoned { .. })
    ));
    assert_eq!(
        std::fs::read(&replacement).unwrap(),
        original_before_refusal
    );
}

#[test]
fn matching_reader_has_an_independent_offset() {
    let tmp = Fixture::new();
    let root = TraceDir::create(tmp.path()).unwrap();
    let mut chain = root.open_chain_for_create().unwrap();
    chain.append_checked(b"abcdef").unwrap();
    chain.seek(SeekFrom::Start(3)).unwrap();

    let mut reader = root.open_matching_reader(&mut chain).unwrap();
    let mut text = String::new();
    reader.read_to_string(&mut text).unwrap();
    assert_eq!(text, "abcdef");
    assert_eq!(chain.stream_position().unwrap(), 3);
}

#[test]
fn chain_and_ack_poison_are_independent() {
    let tmp = Fixture::new();
    let root = TraceDir::create(tmp.path()).unwrap();
    let mut chain = root.open_chain_for_create().unwrap();
    let mut acks = root.open_acks_for_create().unwrap();
    std::fs::hard_link(tmp.path().join("acks.jsonl"), tmp.path().join("ack-alias")).unwrap();
    assert!(acks.append_checked(b"ack\n").is_err());
    std::fs::remove_file(tmp.path().join("ack-alias")).unwrap();
    assert!(matches!(
        acks.append_checked(b"ack\n"),
        Err(TraceCapError::Poisoned { .. })
    ));
    chain.append_checked(b"chain\n").unwrap();
    assert_eq!(
        std::fs::read(tmp.path().join("chain.jsonl")).unwrap(),
        b"chain\n"
    );
    assert!(
        std::fs::read(tmp.path().join("acks.jsonl"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn fixed_writer_checks_links_again_before_consuming_bytes() {
    let tmp = Fixture::new();
    let root = TraceDir::create(tmp.path()).unwrap();
    let writer = root.create_fixed(FixedDocument::Receipt).unwrap();
    std::fs::hard_link(
        tmp.path().join("receipt.json"),
        tmp.path().join("receipt-alias"),
    )
    .unwrap();
    assert!(writer.write_all_sync_checked(b"receipt").is_err());
    assert!(
        std::fs::read(tmp.path().join("receipt.json"))
            .unwrap()
            .is_empty()
    );
    assert!(
        std::fs::read(tmp.path().join("receipt-alias"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn sweep_accepts_only_the_canonical_private_grammar() {
    let tmp = Fixture::new();
    let root = TraceDir::create(tmp.path()).unwrap();
    let removed = tmp.path().join("trace.toml.123.4.tmp");
    std::fs::write(&removed, b"stale").unwrap();
    for retained in [
        "foo.tmp",
        "trace.toml.01.4.tmp",
        "trace.toml.1.+4.tmp",
        "trace.toml.1.4.more.tmp",
    ] {
        std::fs::write(tmp.path().join(retained), retained).unwrap();
    }
    root.sweep_descriptor_temps().unwrap();
    assert!(!removed.exists());
    for retained in [
        "foo.tmp",
        "trace.toml.01.4.tmp",
        "trace.toml.1.+4.tmp",
        "trace.toml.1.4.more.tmp",
    ] {
        assert!(tmp.path().join(retained).exists(), "{retained}");
    }
    let second = tmp.path().join("trace.toml.8.9.tmp");
    std::fs::write(&second, b"second sweep").unwrap();
    root.sweep_descriptor_temps().unwrap();
    assert!(!second.exists());
}

#[test]
fn sweep_entry_limit_refuses_before_deleting_any_owned_temp() {
    let tmp = Fixture::new();
    let root = TraceDir::create(tmp.path()).unwrap();
    let owned = tmp.path().join("trace.toml.1.1.tmp");
    std::fs::write(&owned, b"stale").unwrap();
    for index in 0..4096 {
        std::fs::write(tmp.path().join(format!("unmatched-{index}")), b"").unwrap();
    }
    assert!(root.sweep_descriptor_temps().is_err());
    assert!(owned.exists());
}

#[test]
fn final_root_symlinks_and_multilink_streams_are_refused() {
    let tmp = Fixture::new();
    let real = tmp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let linked = tmp.path().join("linked");
    symlink(&real, &linked).unwrap();
    assert!(TraceDir::open(&linked).is_err());

    let root = TraceDir::open(&real).unwrap();
    std::fs::write(real.join("chain.jsonl"), b"").unwrap();
    std::fs::hard_link(real.join("chain.jsonl"), real.join("alias")).unwrap();
    assert!(root.open_chain_for_create().is_err());

    std::fs::remove_file(real.join("alias")).unwrap();
    std::fs::remove_file(real.join("chain.jsonl")).unwrap();
    std::fs::create_dir(real.join("chain.jsonl")).unwrap();
    assert!(root.open_chain_for_create().is_err());
}
