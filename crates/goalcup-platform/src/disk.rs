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
    available(result, stat)
}

/// The same measurement on an open descriptor, so it names the volume the file lives on.
fn free_bytes_of(file: &File) -> io::Result<u64> {
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe {
        // SAFETY: the descriptor is live and `stat` is a writable statvfs for the call.
        libc::fstatvfs(file.as_raw_fd(), stat.as_mut_ptr())
    };
    available(result, stat)
}

fn available(result: libc::c_int, stat: std::mem::MaybeUninit<libc::statvfs>) -> io::Result<u64> {
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: statvfs returned 0, so it filled the struct.
    let stat = unsafe { stat.assume_init() };
    // The field widths differ by target (u32 and u64 on macOS, u64 on 64-bit Linux); the u32
    // block count on macOS caps a volume at 16 TiB of free space at 4 KiB blocks, which is
    // far above any reserve.
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
            // O_NONBLOCK so a FIFO at the path cannot block the open; it is refused below.
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

    /// Bytes physically allocated to the file (`st_blocks * 512`); 0 if it cannot be measured.
    pub fn allocated(&self) -> u64 {
        self.measure().unwrap_or(0)
    }

    fn measure(&self) -> io::Result<u64> {
        Ok(self.file.metadata()?.blocks().saturating_mul(512))
    }

    /// Allocate or give back blocks so the file holds `bytes`. Growing checks the volume
    /// first and is `StorageFull` when short, and verifies the result by measurement, writing
    /// zeros if the allocator did not deliver; shrinking returns the blocks to the volume.
    pub fn resize(&mut self, bytes: u64) -> io::Result<()> {
        let missing = bytes.saturating_sub(self.measure()?);
        if missing == 0 {
            return self.set_length(bytes);
        }
        if free_bytes_of(&self.file)? < missing {
            return Err(io::Error::from(ErrorKind::StorageFull));
        }
        let granted = match allocate(&self.file, bytes, missing) {
            Ok(()) => true,
            Err(error) if is_unsupported(&error) => false,
            Err(error) => return Err(error),
        };
        self.set_length(bytes)?;
        if !granted || self.measure()? < bytes {
            write_zeros(&self.file, bytes)?;
            if self.measure()? < bytes {
                return Err(io::Error::other("the volume did not allocate the reserve"));
            }
        }
        Ok(())
    }

    fn set_length(&self, bytes: u64) -> io::Result<()> {
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
            _ => return Err(error),
        }
    }
}

/// No native allocator is known here: the caller falls back to writing zeros.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn allocate(_file: &File, _bytes: u64, _missing: u64) -> io::Result<()> {
    Err(io::Error::from_raw_os_error(libc::ENOTSUP))
}

/// The errors a filesystem gives when it has no preallocation, as opposed to a real failure.
fn is_unsupported(error: &io::Error) -> bool {
    error.raw_os_error().is_some_and(|code| {
        code == libc::ENOTSUP || code == libc::EOPNOTSUPP || code == libc::EINVAL
    })
}

/// Allocate by writing: zeros in 1 MiB chunks over `[0, bytes)`.
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
