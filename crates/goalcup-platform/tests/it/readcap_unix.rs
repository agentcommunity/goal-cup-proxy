use goalcup_platform::filecap::{FileCapError, RefusalKind};
use goalcup_platform::readcap::open_regular;
use std::ffi::CString;
use std::io::Read;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::current_dir()
            .unwrap()
            .join("target/readcap-unix-tests")
            .join(format!("{label}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("failed to clean readcap fixture {:?}: {error}", self.0);
        }
    }
}

#[test]
fn regular_file_is_opened_without_following_a_final_symlink() {
    let fixture = Fixture::new("symlink");
    let outside = fixture.path().join("outside.txt");
    std::fs::write(&outside, b"outside sentinel").unwrap();
    let link = fixture.path().join("status.json");
    symlink(&outside, &link).unwrap();

    let error = open_regular(&link).expect_err("a final symlink must be refused");
    assert!(
        matches!(
            error,
            FileCapError::Refused {
                reason: RefusalKind::ReparsePoint,
                ..
            }
        ),
        "unexpected error: {error}"
    );
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside sentinel");
}

#[test]
fn fifo_and_directory_are_refused_without_waiting_for_a_writer() {
    let fixture = Fixture::new("fifo");
    let fifo = fixture.path().join("lock.fifo");
    let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: the CString is NUL terminated and points to an owned fixture path.
    assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);

    let started = Instant::now();
    let error = open_regular(&fifo).expect_err("a fifo must be refused");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(
        error,
        FileCapError::Refused {
            reason: RefusalKind::NotRegularFile,
            ..
        }
    ));

    assert!(matches!(
        open_regular(fixture.path()),
        Err(FileCapError::Refused {
            reason: RefusalKind::NotRegularFile,
            ..
        })
    ));
}

#[test]
fn regular_file_bytes_come_from_the_validated_handle() {
    let fixture = Fixture::new("ordinary");
    let path = fixture.path().join("ordinary.txt");
    std::fs::write(&path, b"ordinary bytes").unwrap();

    let mut file = open_regular(&path).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"ordinary bytes");
}
