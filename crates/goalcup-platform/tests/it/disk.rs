use goalcup_platform::disk::{ReserveFile, free_bytes};
use std::io::ErrorKind;
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
