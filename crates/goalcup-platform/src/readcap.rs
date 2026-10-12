//! Safe acquisition of one path-named regular file for bounded readers.
//!
//! Cache and lock files are attacker-influenceable leaves even when their parent directory
//! is selected by the tracker. Callers receive the same handle that was checked, never a
//! metadata result followed by a pathname reopen. Unix opens nonblocking and without
//! following a final link. A build for any other target fails at `goalcup-platform`'s crate
//! root.

use crate::filecap::{FileCapError, RefusalKind};
use std::fs::File;
use std::path::Path;

/// Open `path` as an ordinary, local file without following a final link.
///
/// `O_NONBLOCK` makes acquisition of FIFOs and devices safe to classify before any read.
pub fn open_regular(path: &Path) -> Result<File, FileCapError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        let mut options = std::fs::OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let file = options.open(path).map_err(|source| {
            if source.raw_os_error() == Some(libc::ELOOP) {
                FileCapError::refused("open regular file", RefusalKind::ReparsePoint)
            } else {
                FileCapError::io("open regular file", source)
            }
        })?;
        let metadata = file
            .metadata()
            .map_err(|source| FileCapError::io("inspect regular file handle", source))?;
        if !metadata.file_type().is_file() {
            return Err(FileCapError::refused(
                "inspect regular file handle",
                RefusalKind::NotRegularFile,
            ));
        }
        Ok(file)
    }
}
