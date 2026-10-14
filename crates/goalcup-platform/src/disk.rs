//! Free space on a volume, and a reserve file that holds blocks against a full disk.
//!
//! The reserve is physically allocated, not sparse: a run that would otherwise find the
//! volume full can shrink it and use the blocks it gave back.

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Bytes an unprivileged writer can still use on the volume holding `dir`.
pub fn free_bytes(dir: &Path) -> io::Result<u64> {
    let path = CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(ErrorKind::InvalidInput))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe {
        // SAFETY: `path` is NUL-terminated and `stat` is a writable statvfs for the call.
        libc::statvfs(path.as_ptr(), stat.as_mut_ptr())
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: statvfs returned 0, so it filled the struct.
    let stat = unsafe { stat.assume_init() };
    // The field widths differ by target (u32 and u64 on macOS, u64 on 64-bit Linux).
    #[allow(clippy::useless_conversion)]
    let free = u64::from(stat.f_bavail).saturating_mul(u64::from(stat.f_frsize));
    Ok(free)
}

/// A file whose logical length equals the bytes it holds, with every block allocated.
#[derive(Debug)]
pub struct ReserveFile {
    file: File,
    path: PathBuf,
}

impl ReserveFile {
    /// Create `path` (mode 0600, never through a symlink) or reopen an existing regular file,
    /// and allocate it up to `bytes`. An existing file keeps the blocks it already has. When
    /// the volume cannot supply the missing bytes this is `ErrorKind::StorageFull` and a file
    /// this call created is removed.
    pub fn hold(path: &Path, bytes: u64) -> io::Result<ReserveFile> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let (file, created) = match options.clone().create_new(true).open(path) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => (options.open(path)?, false),
            Err(error) => return Err(error),
        };
        let mut reserve = ReserveFile {
            file,
            path: path.to_path_buf(),
        };
        let held = reserve
            .file
            .metadata()
            .and_then(|meta| {
                if meta.is_file() {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "the reserve path is not a regular file",
                    ))
                }
            })
            .and_then(|()| reserve.resize(bytes));
        if let Err(error) = held {
            if created {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
        Ok(reserve)
    }

    /// Bytes physically allocated to the file (`st_blocks * 512`).
    pub fn allocated(&self) -> u64 {
        self.file
            .metadata()
            .map_or(0, |meta| meta.blocks().saturating_mul(512))
    }

    /// Allocate or give back blocks so the file holds `bytes`. Growing checks the volume
    /// first and is `StorageFull` when short; shrinking returns the blocks to the volume.
    pub fn resize(&mut self, bytes: u64) -> io::Result<()> {
        let missing = bytes.saturating_sub(self.allocated());
        if missing > 0 {
            let dir = self.path.parent().filter(|dir| !dir.as_os_str().is_empty());
            if free_bytes(dir.unwrap_or(Path::new(".")))? < missing {
                return Err(io::Error::from(ErrorKind::StorageFull));
            }
            allocate(&self.file, bytes, missing)?;
        }
        if self.file.metadata()?.len() != bytes {
            self.file.set_len(bytes)?;
        }
        Ok(())
    }

    /// Unlink the file; its blocks return to the volume.
    pub fn release(self) -> io::Result<()> {
        let ReserveFile { file, path } = self;
        drop(file);
        std::fs::remove_file(path)
    }
}

/// macOS: `F_PREALLOCATE` with `F_ALLOCATEALL | F_ALLOCATEPERSIST` from the physical end of
/// file grants all of the `missing` bytes or none; the caller then sets the logical length.
#[cfg(target_os = "macos")]
fn allocate(file: &File, _bytes: u64, missing: u64) -> io::Result<()> {
    let length =
        libc::off_t::try_from(missing).map_err(|_| io::Error::from(ErrorKind::InvalidInput))?;
    let mut store = libc::fstore_t {
        fst_flags: libc::F_ALLOCATEALL | libc::F_ALLOCATEPERSIST,
        fst_posmode: libc::F_PEOFPOSMODE,
        fst_offset: 0,
        fst_length: length,
        fst_bytesalloc: 0,
    };
    let result = unsafe {
        // SAFETY: the descriptor is live and `store` is a valid, writable fstore_t.
        libc::fcntl(file.as_raw_fd(), libc::F_PREALLOCATE, &raw mut store)
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Linux: `fallocate(0)` over `[0, bytes)`, which also makes the logical length `bytes`.
/// Filesystems without it (`EOPNOTSUPP`) get real zeros instead.
#[cfg(target_os = "linux")]
fn allocate(file: &File, bytes: u64, _missing: u64) -> io::Result<()> {
    let length =
        libc::off_t::try_from(bytes).map_err(|_| io::Error::from(ErrorKind::InvalidInput))?;
    loop {
        let result = unsafe {
            // SAFETY: the descriptor is live; mode 0 only allocates blocks in the range.
            libc::fallocate(file.as_raw_fd(), 0, 0, length)
        };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINTR) => {}
            Some(libc::EOPNOTSUPP) => return write_zeros(file, bytes),
            _ => return Err(error),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn allocate(file: &File, bytes: u64, _missing: u64) -> io::Result<()> {
    write_zeros(file, bytes)
}

/// Allocate by writing: zeros in 1 MiB chunks over `[0, bytes)`.
#[cfg(not(target_os = "macos"))]
fn write_zeros(file: &File, bytes: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;

    let chunk = vec![0u8; 1 << 20];
    let mut offset = 0u64;
    while offset < bytes {
        let take =
            usize::try_from(bytes - offset).map_or(chunk.len(), |left| left.min(chunk.len()));
        file.write_all_at(&chunk[..take], offset)?;
        offset += take as u64;
    }
    file.sync_all()
}
