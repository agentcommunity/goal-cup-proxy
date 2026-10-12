use goalcup_platform::handle_budget::{MAX_GIT_LIVE_HANDLES_V1, NativeHandleBudgetV1};
use goalcup_platform::sourcecap::{SourceDir, SourceKind, SourceLookup, SourceName};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "goalcup-sourcecap-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn selected_directory_enumerates_and_reads_a_matching_regular_file() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("pack.dat"), b"0123456789").unwrap();
    let budget = NativeHandleBudgetV1::new();
    let root = SourceDir::open_selected(&fixture.0, &budget).unwrap();
    let name = SourceName::from_component("pack.dat").unwrap();

    let observed = match root.observe(&name).unwrap() {
        SourceLookup::Present(observed) => observed,
        SourceLookup::Missing => panic!("fixture entry disappeared"),
    };
    assert_eq!(observed.kind(), SourceKind::Regular);
    assert_eq!(observed.length(), 10);
    assert_eq!(observed.posix_owner_executable(), Some(false));

    let mut file = root.open_regular_matching(&name, &observed, 10).unwrap();
    assert_eq!(file.seek(SeekFrom::Start(4)).unwrap(), 4);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"456789");
    assert_eq!(file.finish().unwrap(), observed);

    let mut entries = root.entries().unwrap();
    assert_eq!(
        entries.next_entry().unwrap().unwrap().as_os_str(),
        "pack.dat"
    );
    assert!(entries.next_entry().unwrap().is_none());
}

#[test]
fn source_name_truthfully_enumerates_a_posix_name_rejected_by_windows_alias_rules() {
    let fixture = Fixture::new();
    let native = std::ffi::OsString::from("windows-alias.");
    fs::write(fixture.0.join(&native), b"x").unwrap();
    let root = SourceDir::open_selected(&fixture.0, &NativeHandleBudgetV1::new()).unwrap();
    let mut entries = root.entries().unwrap();
    let returned = entries.next_entry().unwrap().unwrap();
    assert_eq!(
        returned.as_os_str().as_bytes(),
        native.as_os_str().as_bytes()
    );
    assert!(SourceName::from_component("").is_err());
    assert!(SourceName::from_component(".").is_err());
    assert!(SourceName::from_component("..").is_err());
    assert!(SourceName::from_component("a/b").is_err());
    assert!(SourceName::from_component("a\0b").is_err());
}

#[test]
fn fixed_budget_refuses_the_thirty_third_live_handle_and_releases_on_drop() {
    let fixture = Fixture::new();
    let budget = NativeHandleBudgetV1::new();
    let root = SourceDir::open_selected(&fixture.0, &budget).unwrap();
    let mut permits = Vec::new();
    for _ in 1..MAX_GIT_LIVE_HANDLES_V1 {
        permits.push(budget.reserve_scratch().unwrap());
    }
    assert!(budget.reserve_scratch().is_err());
    permits.pop();
    assert!(budget.reserve_scratch().is_ok());
    drop(root);
}

#[test]
fn repeat_and_early_drop_enumeration_each_restart_from_the_first_entry() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("only"), b"x").unwrap();
    let root = SourceDir::open_selected(&fixture.0, &NativeHandleBudgetV1::new()).unwrap();
    for drop_early in [true, false] {
        let mut entries = root.entries().unwrap();
        assert_eq!(entries.next_entry().unwrap().unwrap().as_os_str(), "only");
        if !drop_early {
            assert!(entries.next_entry().unwrap().is_none());
        }
        drop(entries);
    }
    let mut final_entries = root.entries().unwrap();
    assert_eq!(
        final_entries.next_entry().unwrap().unwrap().as_os_str(),
        "only"
    );
    assert!(final_entries.next_entry().unwrap().is_none());
}

#[test]
fn retained_root_and_child_handles_survive_path_replacement() {
    let fixture = Fixture::new();
    let selected = fixture.0.join("selected");
    fs::create_dir(&selected).unwrap();
    fs::create_dir(selected.join("objects")).unwrap();
    fs::write(selected.join("objects/kept"), b"inside").unwrap();
    let root = SourceDir::open_selected(&selected, &NativeHandleBudgetV1::new()).unwrap();
    let objects_name = SourceName::from_component("objects").unwrap();
    let objects_observation = match root.observe(&objects_name).unwrap() {
        SourceLookup::Present(value) => value,
        SourceLookup::Missing => panic!("objects disappeared"),
    };
    let objects = root
        .open_dir_matching(&objects_name, &objects_observation)
        .unwrap();
    fs::rename(&selected, fixture.0.join("moved")).unwrap();
    fs::create_dir(&selected).unwrap();
    fs::create_dir(selected.join("objects")).unwrap();
    fs::write(selected.join("objects/kept"), b"outside").unwrap();
    let kept = SourceName::from_component("kept").unwrap();
    let observed = match objects.observe(&kept).unwrap() {
        SourceLookup::Present(value) => value,
        SourceLookup::Missing => panic!("kept disappeared"),
    };
    let mut file = objects.open_regular_matching(&kept, &observed, 6).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"inside");
    file.finish().unwrap();
}

#[test]
fn stable_hardlink_reads_but_alias_write_is_detected_at_finish() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("object"), b"before").unwrap();
    fs::hard_link(fixture.0.join("object"), fixture.0.join("alias")).unwrap();
    let root = SourceDir::open_selected(&fixture.0, &NativeHandleBudgetV1::new()).unwrap();
    let name = SourceName::from_component("object").unwrap();
    let observed = match root.observe(&name).unwrap() {
        SourceLookup::Present(value) => value,
        SourceLookup::Missing => panic!(),
    };
    let mut file = root.open_regular_matching(&name, &observed, 6).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"before");
    fs::write(fixture.0.join("alias"), b"after!").unwrap();
    assert!(file.finish().is_err());
}

#[test]
fn symlink_payload_is_descriptor_bound_and_a_full_buffer_is_refused() {
    let fixture = Fixture::new();
    symlink("target-bytes", fixture.0.join("link")).unwrap();
    let root = SourceDir::open_selected(&fixture.0, &NativeHandleBudgetV1::new()).unwrap();
    let name = SourceName::from_component("link").unwrap();
    let observed = match root.observe(&name).unwrap() {
        SourceLookup::Present(value) => value,
        SourceLookup::Missing => panic!(),
    };
    assert_eq!(observed.kind(), SourceKind::SymbolicLink);
    let mut destination = [0u8; 64];
    let count = root
        .read_symlink_into_matching(&name, &observed, &mut destination)
        .unwrap();
    assert_eq!(&destination[..count], b"target-bytes");
    let mut exact = [0u8; 12];
    assert!(
        root.read_symlink_into_matching(&name, &observed, &mut exact)
            .is_err()
    );
}

#[test]
fn reads_are_capped_to_one_mebibyte_and_seek_past_opening_length_poisons() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("large"), vec![7u8; (1 << 20) + 32]).unwrap();
    let root = SourceDir::open_selected(&fixture.0, &NativeHandleBudgetV1::new()).unwrap();
    let name = SourceName::from_component("large").unwrap();
    let observed = match root.observe(&name).unwrap() {
        SourceLookup::Present(value) => value,
        SourceLookup::Missing => panic!(),
    };
    let mut file = root
        .open_regular_matching(&name, &observed, observed.length())
        .unwrap();
    let mut destination = vec![0u8; (1 << 20) + 32];
    assert_eq!(file.read(&mut destination).unwrap(), 1 << 20);
    assert!(file.seek(SeekFrom::Start(observed.length() + 1)).is_err());
    assert!(file.finish().is_err());
}

#[test]
fn observed_regular_exchanged_for_symlink_is_refused_without_reading_target() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("source"), b"inside").unwrap();
    fs::write(fixture.0.join("sentinel"), b"outside-secret").unwrap();
    let root = SourceDir::open_selected(&fixture.0, &NativeHandleBudgetV1::new()).unwrap();
    let name = SourceName::from_component("source").unwrap();
    let observed = match root.observe(&name).unwrap() {
        SourceLookup::Present(value) => value,
        SourceLookup::Missing => panic!(),
    };
    fs::remove_file(fixture.0.join("source")).unwrap();
    symlink(fixture.0.join("sentinel"), fixture.0.join("source")).unwrap();
    assert!(root.open_regular_matching(&name, &observed, 64).is_err());
}

#[test]
fn regular_exchanged_for_fifo_is_refused_without_blocking() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("source"), b"inside").unwrap();
    let root = SourceDir::open_selected(&fixture.0, &NativeHandleBudgetV1::new()).unwrap();
    let name = SourceName::from_component("source").unwrap();
    let observed = match root.observe(&name).unwrap() {
        SourceLookup::Present(value) => value,
        SourceLookup::Missing => panic!(),
    };
    fs::remove_file(fixture.0.join("source")).unwrap();
    let path = std::ffi::CString::new(fixture.0.join("source").as_os_str().as_bytes()).unwrap();
    // SAFETY: the path is a NUL-terminated CString that outlives the call.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(root.open_regular_matching(&name, &observed, 64).is_err());
}

#[test]
fn failed_native_open_releases_its_reserved_permit() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("source"), b"inside").unwrap();
    let budget = NativeHandleBudgetV1::new();
    let root = SourceDir::open_selected(&fixture.0, &budget).unwrap();
    let name = SourceName::from_component("source").unwrap();
    let observed = match root.observe(&name).unwrap() {
        SourceLookup::Present(value) => value,
        SourceLookup::Missing => panic!(),
    };
    fs::remove_file(fixture.0.join("source")).unwrap();
    assert!(root.open_regular_matching(&name, &observed, 64).is_err());
    let permits: Vec<_> = (1..MAX_GIT_LIVE_HANDLES_V1)
        .map(|_| budget.reserve_scratch().unwrap())
        .collect();
    assert!(budget.reserve_scratch().is_err());
    drop(permits);
}
