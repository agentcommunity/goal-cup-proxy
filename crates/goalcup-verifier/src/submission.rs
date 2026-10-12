//! Independent validation of retained V4 workspace-submission bytes.
//!
//! The fixture is deliberately assembled from canonical bytes instead of a writer result:
//! reader acceptance must not inherit the writer's serialization assumptions.

use crate::VerifierError;
use crate::trace_fs::{PrivateDir, TracePath, TraceRoot};
use goalcup_protocol::provenance::Sha256Digest;
use goalcup_protocol::submission::{SubmissionCapturedV1, SubmissionManifestV1};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;

const SUBMISSION_DIR: &str = "submission";
const MANIFEST_NAME: &str = "manifest.json";
const OBJECT_DIR: &str = "submission/objects/sha256";
const MANIFEST_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// Read an immutable private submission without consulting the live workspace or legacy seal.
///
/// The manifest is decoded and canonical-hashed before any object is opened. Every retained
/// object is then enumerated through the trace root's directory capability and re-hashed from
/// its own no-follow private-file handle.
pub(crate) fn verify_submission(
    root: &TraceRoot,
    captured: &SubmissionCapturedV1,
) -> Result<SubmissionManifestV1, VerifierError> {
    let submission_dir = root.open_private_dir(&TracePath::parse(SUBMISSION_DIR)?)?;
    require_exact_names(
        &submission_dir,
        3,
        &[MANIFEST_NAME.to_string(), "objects".to_string()],
    )?;
    let manifest_bytes = read_private_child_limited(
        &submission_dir,
        MANIFEST_NAME,
        MANIFEST_MAX_BYTES,
        "submission/manifest.json",
    )?;
    let manifest: SubmissionManifestV1 =
        serde_json::from_slice(&manifest_bytes).map_err(|source| VerifierError::Json {
            path: "submission/manifest.json".to_string(),
            line: 0,
            source,
        })?;
    let canonical = manifest.canonical_bytes().map_err(|error| {
        VerifierError::Trace(format!("submission manifest is invalid: {error}"))
    })?;
    if canonical != manifest_bytes {
        return Err(VerifierError::Trace(
            "submission manifest is not canonical JSON bytes".into(),
        ));
    }
    let manifest_digest = Sha256Digest::from_bytes(Sha256::digest(&manifest_bytes).into());
    if manifest_digest != captured.manifest_sha256 {
        return Err(VerifierError::Trace(
            "submission manifest digest does not match the captured finalization claim".into(),
        ));
    }

    let mut expected = BTreeMap::<String, u64>::new();
    for entry in &manifest.entries {
        expected.insert(entry.content_sha256.as_str().to_owned(), entry.bytes);
    }
    let child_limit = manifest
        .effective_limits
        .object_count
        .checked_add(1)
        .ok_or_else(|| VerifierError::Trace("submission object count cannot be bounded".into()))?;
    let objects = submission_dir.open_private_dir_child("objects")?;
    require_exact_names(&objects, 2, &["sha256".to_string()])?;
    let object_store = objects.open_private_dir_child("sha256")?;
    let names = object_store.child_names(child_limit)?;
    let expected_names = expected.keys().cloned().collect::<Vec<_>>();
    if names != expected_names {
        return Err(VerifierError::Trace(format!(
            "submission object store contents do not exactly match the manifest ({} found, {} expected)",
            names.len(),
            expected.len()
        )));
    }

    #[cfg(test)]
    let mut first_object = true;
    for name in &names {
        let expected_bytes = expected
            .get(name)
            .expect("equal sorted object-name sets have matching entries");
        hash_private_object(&object_store, name, *expected_bytes)?;
        #[cfg(test)]
        if first_object {
            tests::run_after_first_object_hash();
            first_object = false;
        }
    }
    if object_store.child_names(child_limit)? != names {
        return Err(VerifierError::Trace(
            "submission object store changed while objects were hashed".into(),
        ));
    }
    require_exact_names(&objects, 2, &["sha256".to_string()])?;
    require_exact_names(
        &submission_dir,
        3,
        &[MANIFEST_NAME.to_string(), "objects".to_string()],
    )?;
    Ok(manifest)
}

fn require_exact_names(
    directory: &PrivateDir,
    limit: u64,
    expected: &[String],
) -> Result<(), VerifierError> {
    let names = directory.child_names(limit)?;
    if names != expected {
        return Err(VerifierError::Trace(format!(
            "submission fixed layout differs from the required entries {expected:?}"
        )));
    }
    Ok(())
}

fn read_private_child_limited(
    directory: &PrivateDir,
    name: &str,
    limit: u64,
    display: &str,
) -> Result<Vec<u8>, VerifierError> {
    let file = directory.open_private_regular_child(name)?;
    let length = file
        .metadata()
        .map_err(|source| VerifierError::Io {
            path: display.to_owned(),
            source,
        })?
        .len();
    if length > limit {
        return Err(VerifierError::EvidenceTooLarge {
            path: display.to_owned(),
            limit,
        });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| VerifierError::Io {
            path: display.to_owned(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(VerifierError::EvidenceTooLarge {
            path: display.to_owned(),
            limit,
        });
    }
    Ok(bytes)
}

fn hash_private_object(
    store: &PrivateDir,
    name: &str,
    expected_bytes: u64,
) -> Result<(), VerifierError> {
    let display = format!("{OBJECT_DIR}/{name}");
    let mut file = store.open_private_regular_child(name)?;
    let length = file
        .metadata()
        .map_err(|source| VerifierError::Io {
            path: display.clone(),
            source,
        })?
        .len();
    if length != expected_bytes {
        return Err(VerifierError::Trace(format!(
            "submission object {:?} has {length} bytes, expected {expected_bytes}",
            display
        )));
    }
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| VerifierError::Io {
            path: display.clone(),
            source,
        })?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| {
                VerifierError::Trace(format!("submission object {:?} length overflowed", display))
            })?;
        if total > expected_bytes {
            return Err(VerifierError::Trace(format!(
                "submission object {:?} grew while being read",
                display
            )));
        }
        hasher.update(&buffer[..read]);
    }
    if total != expected_bytes {
        return Err(VerifierError::Trace(format!(
            "submission object {:?} was short while being read",
            display
        )));
    }
    let actual = hex::encode(hasher.finalize());
    if actual != name {
        return Err(VerifierError::Trace(format!(
            "submission object {:?} digest does not match its filename",
            display
        )));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use crate::trace_fs::TraceRoot;
    use goalcup_protocol::provenance::Sha256Digest;
    use goalcup_protocol::submission::{SubmissionCapturedV1, SubmissionManifestV1};
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;
    use std::os::unix::fs::{PermissionsExt, symlink};

    thread_local! {
        static AFTER_FIRST_OBJECT_HASH: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    }

    pub(super) fn install_after_first_object_hash(hook: impl FnOnce() + 'static) {
        AFTER_FIRST_OBJECT_HASH.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }

    pub(super) fn run_after_first_object_hash() {
        AFTER_FIRST_OBJECT_HASH.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
    }

    const MANIFEST: &str = r#"{"profile":"workspace_snapshot_v1","limit_profile":{"id":"macos-v1"},"effective_limits":{"object_count":8,"total_bytes":4096,"pre_cutoff_drain":5000,"process_walk":32},"entry_id":"entry-1","receipt_sha256":"1111111111111111111111111111111111111111111111111111111111111111","cutoff_seq":9,"cutoff_chain_hash":"2222222222222222222222222222222222222222222222222222222222222222","finish_reason":"manual","entries":[{"path":"empty","kind":"regular","posix_owner_executable":false,"bytes":0,"content_sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"},{"path":"run","kind":"regular","posix_owner_executable":true,"bytes":2,"content_sha256":"73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac"},{"path":"sym","kind":"symbolic_link","posix_owner_executable":false,"bytes":10,"content_sha256":"62ca1d92c4a3fc44a5fa30d1ddc593be1a9945ca21c0821af53d4f2b604075e7"}],"excluded":[".codex",".git"],"copy_started_at":"2026-09-14T00:00:00Z","copy_completed_at":"2026-09-14T00:00:01Z","readback_verified_at":"2026-09-14T00:00:02Z","object_count":3,"total_bytes":12}"#;
    const MANIFEST_SHA256: &str =
        "86c61e3f57bac049fcf339eecad8c0a624dbf6ad96d273e043ed652fbb55ae2d";

    fn private_file(path: &std::path::Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn independent_submission_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let objects = dir.path().join("submission/objects/sha256");
        std::fs::create_dir_all(&objects).unwrap();
        private_file(
            &dir.path().join("submission/manifest.json"),
            MANIFEST.as_bytes(),
        );
        private_file(
            &objects.join("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            b"",
        );
        private_file(
            &objects.join("73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac"),
            b"x\n",
        );
        private_file(
            &objects.join("62ca1d92c4a3fc44a5fa30d1ddc593be1a9945ca21c0821af53d4f2b604075e7"),
            b"../outside",
        );
        dir
    }

    fn captured(bytes: &[u8]) -> SubmissionCapturedV1 {
        SubmissionCapturedV1 {
            manifest_sha256: Sha256Digest::from_bytes(Sha256::digest(bytes).into()),
        }
    }

    fn verify_fixture(
        dir: &std::path::Path,
        manifest: &[u8],
    ) -> Result<SubmissionManifestV1, crate::VerifierError> {
        private_file(&dir.join("submission/manifest.json"), manifest);
        let root = TraceRoot::open(dir).unwrap();
        super::verify_submission(&root, &captured(manifest))
    }

    #[test]
    fn reader_accepts_independent_canonical_private_submission() {
        let dir = independent_submission_fixture();
        let root = TraceRoot::open(dir.path()).unwrap();
        let captured = captured(MANIFEST.as_bytes());

        let retained = super::verify_submission(&root, &captured).unwrap();

        assert_eq!(retained.digest().unwrap().as_str(), MANIFEST_SHA256);
        assert_eq!(
            retained
                .entries
                .iter()
                .filter(|e| e.path.as_str() == "empty")
                .count(),
            1
        );
    }

    #[test]
    fn fixture_is_parseable_before_the_reader_opens_objects() {
        let manifest: SubmissionManifestV1 = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(manifest.digest().unwrap().as_str(), MANIFEST_SHA256);
    }

    #[test]
    fn reader_refuses_missing_and_extra_objects() {
        let missing = independent_submission_fixture();
        std::fs::remove_file(missing.path().join(
            "submission/objects/sha256/73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac",
        ))
        .unwrap();
        assert!(verify_fixture(missing.path(), MANIFEST.as_bytes()).is_err());

        let extra = independent_submission_fixture();
        private_file(
            &extra.path().join(
                "submission/objects/sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            b"unlisted",
        );
        assert!(verify_fixture(extra.path(), MANIFEST.as_bytes()).is_err());

        let corrupt = independent_submission_fixture();
        private_file(
            &corrupt.path().join(
                "submission/objects/sha256/73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac",
            ),
            b"y\n",
        );
        assert!(verify_fixture(corrupt.path(), MANIFEST.as_bytes()).is_err());
    }

    #[test]
    fn reader_refuses_entries_outside_the_exact_fixed_layout() {
        for relative in ["submission/hidden", "submission/objects/other-algorithm"] {
            let fixture = independent_submission_fixture();
            let path = fixture.path().join(relative);
            if relative.ends_with("hidden") {
                private_file(&path, b"unbound");
            } else {
                std::fs::create_dir(&path).unwrap();
            }
            assert!(
                verify_fixture(fixture.path(), MANIFEST.as_bytes()).is_err(),
                "reader accepted extra fixed-layout entry {relative}"
            );
        }
    }

    #[test]
    fn reader_refuses_fixed_layout_entries_inserted_during_hashing() {
        for (relative, directory) in [
            ("submission/hidden", false),
            ("submission/objects/other-algorithm", true),
            (
                "submission/objects/sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                false,
            ),
        ] {
            let fixture = independent_submission_fixture();
            let inserted = fixture.path().join(relative);
            install_after_first_object_hash(move || {
                if directory {
                    std::fs::create_dir(&inserted).unwrap();
                } else {
                    private_file(&inserted, b"inserted");
                }
            });

            assert!(
                verify_fixture(fixture.path(), MANIFEST.as_bytes()).is_err(),
                "reader accepted {relative} after its pre-hash snapshot"
            );
        }
    }

    #[test]
    fn reader_refuses_private_store_mode_and_multi_link_object() {
        let mode = independent_submission_fixture();
        let object = mode.path().join(
            "submission/objects/sha256/73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac",
        );
        std::fs::set_permissions(&object, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(verify_fixture(mode.path(), MANIFEST.as_bytes()).is_err());

        let links = independent_submission_fixture();
        let object = links.path().join(
            "submission/objects/sha256/73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac",
        );
        std::fs::hard_link(&object, links.path().join("another-name-for-object")).unwrap();
        assert!(verify_fixture(links.path(), MANIFEST.as_bytes()).is_err());
    }

    #[test]
    fn reader_refuses_nonregular_object_and_symlinked_ancestor() {
        let nonregular = independent_submission_fixture();
        let object = nonregular.path().join(
            "submission/objects/sha256/73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac",
        );
        std::fs::remove_file(&object).unwrap();
        symlink("/tmp", &object).unwrap();
        assert!(verify_fixture(nonregular.path(), MANIFEST.as_bytes()).is_err());

        let ancestor = independent_submission_fixture();
        let objects = ancestor.path().join("submission/objects");
        let moved = ancestor.path().join("moved-objects");
        std::fs::rename(&objects, &moved).unwrap();
        symlink(&moved, &objects).unwrap();
        assert!(verify_fixture(ancestor.path(), MANIFEST.as_bytes()).is_err());
    }
}
