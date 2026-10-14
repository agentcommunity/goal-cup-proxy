use goalcup_platform::disk::{ReserveFile, free_bytes};
use std::io::ErrorKind;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MIB: u64 = 1024 * 1024;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "goalcup-disk-{}-{}",
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
fn a_reserve_file_holds_its_blocks_and_gives_them_back() {
    let tmp = Fixture::new();
    let path = tmp.path().join("reserve");
    let mut reserve = ReserveFile::hold(&path, 64 * MIB).unwrap();
    assert!(reserve.allocated() >= 64 * MIB, "{}", reserve.allocated());
    reserve.resize(16 * MIB).unwrap();
    assert!(reserve.allocated() < 20 * MIB, "{}", reserve.allocated());
    reserve.resize(48 * MIB).unwrap();
    assert!(reserve.allocated() >= 48 * MIB, "{}", reserve.allocated());
    reserve.release().unwrap();
    assert!(!path.exists());
}

#[test]
fn hold_reopens_without_doubling() {
    let tmp = Fixture::new();
    let path = tmp.path().join("reserve");
    let first = ReserveFile::hold(&path, 32 * MIB).unwrap();
    let before = first.allocated();
    drop(first);
    let second = ReserveFile::hold(&path, 32 * MIB).unwrap();
    assert!(second.allocated() >= 32 * MIB);
    assert!(
        second.allocated() < before + 8 * MIB,
        "{} after {before}",
        second.allocated()
    );
    second.release().unwrap();
}

#[test]
fn hold_refuses_more_than_the_volume_has() {
    let tmp = Fixture::new();
    let path = tmp.path().join("reserve");
    let ask = free_bytes(tmp.path()).unwrap() + (1 << 40);
    let error = ReserveFile::hold(&path, ask).expect_err("must refuse");
    assert_eq!(error.kind(), ErrorKind::StorageFull);
    assert!(!path.exists());
}

#[test]
fn hold_refuses_a_symlink_and_leaves_its_target_alone() {
    let tmp = Fixture::new();
    let target = tmp.path().join("target");
    std::fs::write(&target, b"x").unwrap();
    let path = tmp.path().join("reserve");
    symlink(&target, &path).unwrap();
    ReserveFile::hold(&path, 4 * MIB).expect_err("a symlink is refused");
    assert_eq!(std::fs::read(&target).unwrap(), b"x");
    assert!(std::fs::symlink_metadata(&path).unwrap().is_symlink());
}

#[test]
fn hold_refuses_a_path_that_is_not_a_regular_file() {
    let tmp = Fixture::new();
    let path = tmp.path().join("reserve");
    std::fs::create_dir(&path).unwrap();
    ReserveFile::hold(&path, 4 * MIB).expect_err("a directory is refused");
    assert!(path.is_dir());
}

#[test]
fn a_created_reserve_is_private_to_its_owner() {
    let tmp = Fixture::new();
    let path = tmp.path().join("reserve");
    let reserve = ReserveFile::hold(&path, 4 * MIB).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    reserve.release().unwrap();
}

#[test]
fn reopening_at_another_size_ends_at_that_size_without_doubling() {
    let tmp = Fixture::new();
    let path = tmp.path().join("reserve");
    drop(ReserveFile::hold(&path, 16 * MIB).unwrap());

    let larger = ReserveFile::hold(&path, 32 * MIB).unwrap();
    assert!(larger.allocated() >= 32 * MIB, "{}", larger.allocated());
    assert!(larger.allocated() < 40 * MIB, "{}", larger.allocated());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 32 * MIB);
    drop(larger);

    let smaller = ReserveFile::hold(&path, 16 * MIB).unwrap();
    assert!(smaller.allocated() >= 16 * MIB, "{}", smaller.allocated());
    assert!(smaller.allocated() < 20 * MIB, "{}", smaller.allocated());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 16 * MIB);
    smaller.release().unwrap();
}

#[test]
fn growing_past_the_free_space_is_storage_full_and_keeps_the_size() {
    let tmp = Fixture::new();
    let path = tmp.path().join("reserve");
    let mut reserve = ReserveFile::hold(&path, 4 * MIB).unwrap();
    let allocated = reserve.allocated();
    let ask = free_bytes(tmp.path()).unwrap() + (1 << 40);
    let error = reserve.resize(ask).expect_err("must refuse");
    assert_eq!(error.kind(), ErrorKind::StorageFull);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 4 * MIB);
    assert_eq!(reserve.allocated(), allocated);
    reserve.release().unwrap();
}
