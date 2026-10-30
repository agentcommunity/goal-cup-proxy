//! No-follow file system calls for the native reader (PRD-04 section 9).
//!
//! Behaviour copied from `crates/goalcup-platform/src/sourcecap/unix.rs` and rewritten small:
//! the root open from `:87-118`, the `fstatat` check from `:128-155`, the listing on a duplicate
//! from `:157-176` and `:326-388`, the folder and file opens from `:178-253`, and the `stat`
//! fields from `:403-473`. Gone: the `statx` mount check (the walk compares `st_dev` instead),
//! symlink reading, retry loops and the handle budget.
//!
//! A symlink is never followed at any level, a FIFO or device is never read, and opening one never
//! hangs (`O_NONBLOCK`, then a regular-file check on the descriptor). The reader never writes,
//! locks, renames or deletes, and holds no state between calls.

use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::File;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use crate::pipeline::cursors::Fid;

const DIR_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
const FILE_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;

/// What a name is, from `lstat` semantics: a symlink is described, never followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    /// A FIFO, socket or device: never opened.
    Other,
}

/// The facts of one `stat` (PRD-04 section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub kind: Kind,
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    /// macOS only; `None` elsewhere.
    pub birth_ns: Option<i64>,
}

impl Stat {
    pub fn fid(&self) -> Fid {
        Fid {
            dev: self.dev,
            ino: self.ino,
            birth_ns: self.birth_ns,
        }
    }
}

/// One short lower-hex string for the OPEN meta `fid`: `dev-ino`, or `dev-ino-birth` on macOS.
pub fn fid_hex(fid: &Fid) -> String {
    match fid.birth_ns {
        Some(b) => format!("{:x}-{:x}-{:x}", fid.dev, fid.ino, b),
        None => format!("{:x}-{:x}", fid.dev, fid.ino),
    }
}

/// One open folder. Dropping it closes the descriptor.
#[derive(Debug)]
pub struct Dir {
    fd: OwnedFd,
}

impl Dir {
    /// Opens a root by path; a symlinked root or a non-folder fails (`unix.rs:94-98`).
    pub fn open_root(path: &Path) -> io::Result<Dir> {
        let c = cname(path.as_os_str())?;
        // SAFETY: `c` is a live NUL-terminated string for the call.
        let fd = owned(unsafe { libc::open(c.as_ptr(), DIR_FLAGS) })?;
        Ok(Dir { fd })
    }

    /// Opens a child folder; a symlink or a non-folder fails (`unix.rs:185-190`).
    pub fn open_child(&self, name: &OsStr) -> io::Result<Dir> {
        let c = cname(name)?;
        // SAFETY: the parent descriptor is open and `c` is a live NUL-terminated string.
        let fd = owned(unsafe { libc::openat(self.fd.as_raw_fd(), c.as_ptr(), DIR_FLAGS) })?;
        Ok(Dir { fd })
    }

    /// The names in this folder, without `.` and `..`, unsorted.
    pub fn names(&self) -> io::Result<Vec<OsString>> {
        // SAFETY: the descriptor is open; F_DUPFD_CLOEXEC only makes a new descriptor.
        let dup = owned(unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) })?;
        // SAFETY: `dup` is an open folder descriptor; on success the stream owns it.
        let dir = unsafe { libc::fdopendir(dup.as_raw_fd()) };
        if dir.is_null() {
            return Err(io::Error::last_os_error());
        }
        std::mem::forget(dup);
        // The duplicate shares the offset with `self.fd`, so start from the top every time.
        // SAFETY: `dir` is non-null and open on this thread.
        unsafe { libc::rewinddir(dir) };
        let mut out = Vec::new();
        let result = loop {
            clear_errno();
            // SAFETY: `dir` is non-null and open; NULL with errno 0 is the end, else an error.
            let entry = unsafe { libc::readdir(dir) };
            if entry.is_null() {
                let e = io::Error::last_os_error();
                break match e.raw_os_error() {
                    Some(0) => Ok(()),
                    _ => Err(e),
                };
            }
            // SAFETY: `entry` is non-null; `d_name` is NUL terminated, copied before the next call.
            let b = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if b != b"." && b != b".." {
                out.push(OsString::from_vec(b.to_vec()));
            }
        };
        // SAFETY: `dir` is open and closed exactly once here; this also closes `dup`.
        unsafe { libc::closedir(dir) };
        result.map(|()| out)
    }

    /// `fstat` on this folder.
    pub fn stat(&self) -> io::Result<Stat> {
        fstat(self.fd.as_raw_fd())
    }

    /// `fstatat` with `AT_SYMLINK_NOFOLLOW`: a symlink is described, not followed.
    pub fn stat_at(&self, name: &OsStr) -> io::Result<Stat> {
        let c = cname(name)?;
        let mut st = MaybeUninit::<libc::stat>::uninit();
        // SAFETY: the parent descriptor is open, `c` is NUL terminated, `st` is a writable output.
        let r = unsafe {
            libc::fstatat(
                self.fd.as_raw_fd(),
                c.as_ptr(),
                st.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded, so it filled `st`.
        Ok(from_stat(unsafe { st.assume_init() }))
    }

    /// Opens a regular file whose `fid` must equal `expect` (`unix.rs:214-253`). A symlink, FIFO,
    /// device, folder or a file replaced since `expect` was seen is refused and its descriptor
    /// closed.
    pub fn open_file(&self, name: &OsStr, expect: Fid) -> io::Result<File> {
        let c = cname(name)?;
        // SAFETY: the parent descriptor is open and `c` is NUL terminated; O_NONBLOCK keeps a FIFO
        // open from hanging.
        let fd = owned(unsafe { libc::openat(self.fd.as_raw_fd(), c.as_ptr(), FILE_FLAGS) })?;
        let st = fstat(fd.as_raw_fd())?;
        if st.kind != Kind::File || st.fid() != expect {
            return Err(io::Error::other("not the regular file that was seen"));
        }
        Ok(File::from(fd))
    }
}

/// `fstat` on an open file, for the re-check after a read.
pub fn file_stat(f: &File) -> io::Result<Stat> {
    fstat(f.as_raw_fd())
}

fn cname(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

fn owned(raw: RawFd) -> io::Result<OwnedFd> {
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a non-negative return is a new descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn fstat(fd: RawFd) -> io::Result<Stat> {
    let mut st = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `fd` is open and `st` is a writable output, read only after success.
    if unsafe { libc::fstat(fd, st.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded, so it filled `st`.
    Ok(from_stat(unsafe { st.assume_init() }))
}

fn from_stat(st: libc::stat) -> Stat {
    let kind = match st.st_mode & libc::S_IFMT {
        libc::S_IFREG => Kind::File,
        libc::S_IFDIR => Kind::Dir,
        libc::S_IFLNK => Kind::Symlink,
        _ => Kind::Other,
    };
    // Redundant on Linux glibc, where dev_t is already u64, but real on macOS, where dev_t is
    // i32: `u64::from` does not compile there, so the cast form is the one that works on both
    // (`sourcecap/unix.rs:420-447`).
    #[allow(
        clippy::unnecessary_cast,
        reason = "dev_t is i32 on macOS; the cast widens there even though it is a no-op on Linux glibc"
    )]
    let dev = st.st_dev as u64;
    #[cfg(target_os = "macos")]
    let birth_ns = Some(ns(st.st_birthtime, st.st_birthtime_nsec));
    #[cfg(not(target_os = "macos"))]
    let birth_ns = None;
    Stat {
        kind,
        dev,
        ino: st.st_ino,
        size: u64::try_from(st.st_size).unwrap_or(0),
        mtime_ns: ns(st.st_mtime, st.st_mtime_nsec),
        ctime_ns: ns(st.st_ctime, st.st_ctime_nsec),
        birth_ns,
    }
}

fn ns(secs: i64, nsec: i64) -> i64 {
    secs.saturating_mul(1_000_000_000).saturating_add(nsec)
}

/// Clears this thread's `errno`, for the `readdir` end-of-folder idiom
/// (`goalcup-platform/src/errno.rs`).
fn clear_errno() {
    #[cfg(target_os = "macos")]
    // SAFETY: the accessor returns this thread's errno location, valid for the thread's lifetime.
    let p = unsafe { libc::__error() };
    #[cfg(not(target_os = "macos"))]
    // SAFETY: as above.
    let p = unsafe { libc::__errno_location() };
    // SAFETY: `p` is a valid, writable pointer to this thread's errno.
    unsafe { *p = 0 };
}
