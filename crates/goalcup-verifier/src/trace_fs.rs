//! Pinned, handle-relative reads for participant-controlled trace exports.
//!
//! A descriptor may choose evidence names but never a host path.  In particular, do not
//! replace this with `canonicalize` followed by `std::fs::read`: an ancestor can change
//! between those operations.

use crate::VerifierError;
use std::path::Path;

/// A validated descriptor-relative evidence name. Its host-path representation is private.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracePath(String);

impl TracePath {
    pub(crate) fn parse(value: impl AsRef<str>) -> Result<Self, VerifierError> {
        let value = value.as_ref();
        if value.is_empty() || value.contains('\0') || value.contains('\\') {
            return Err(VerifierError::Trace(format!(
                "trace path {value:?} is not a non-empty relative path"
            )));
        }
        // Parse the descriptor grammar directly, rather than relying on host `Path`
        // normalization. Empty components, `.` and a terminal slash would otherwise be
        // silently normalized before the no-follow reader sees them.
        if value.starts_with('/')
            || value.ends_with('/')
            || value.starts_with("//")
            || (value.len() >= 2
                && value.as_bytes()[0].is_ascii_alphabetic()
                && value.as_bytes()[1] == b':')
        {
            return Err(VerifierError::Trace(format!(
                "trace path {value:?} is not a relative descendant"
            )));
        }
        for component in value.split('/') {
            if component.is_empty() || matches!(component, "." | "..") {
                return Err(VerifierError::Trace(format!(
                    "trace path {value:?} is not a relative descendant"
                )));
            }
        }
        Ok(Self(value.to_owned()))
    }

    pub fn display(&self) -> &str {
        &self.0
    }

    pub(crate) fn file_stem(&self) -> String {
        Path::new(&self.0)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

#[cfg(unix)]
#[derive(Debug, Clone)]
pub(crate) struct TraceRoot(std::sync::Arc<std::fs::File>);

/// One retained directory capability. Enumeration and child opens stay relative to this
/// exact handle, so an attacker cannot replace an ancestor between the two operations.
#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct PrivateDir {
    file: std::fs::File,
    display: String,
}

#[cfg(not(unix))]
#[derive(Debug)]
pub(crate) struct PrivateDir;

#[cfg(unix)]
fn selected_root_name(root: &Path) -> Result<std::path::PathBuf, VerifierError> {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let mut bytes = root.as_os_str().as_bytes().to_vec();
    if bytes.contains(&0) {
        return Err(VerifierError::Trace(
            "selected trace root contains an interior NUL".into(),
        ));
    }
    // The OS accepts `link/` and `link/.` by resolving link before O_NOFOLLOW gets a
    // chance. Strip only those terminal spellings; ancestors remain ordinary OS paths.
    while bytes.len() > 1 && bytes.last() == Some(&b'/') {
        bytes.pop();
    }
    while bytes.len() >= 2 && bytes.ends_with(b"/.") {
        if bytes == b"/." {
            bytes.truncate(1);
            break;
        }
        bytes.truncate(bytes.len() - 2);
        while bytes.len() > 1 && bytes.last() == Some(&b'/') {
            bytes.pop();
        }
    }
    if bytes.is_empty() {
        return Err(VerifierError::Trace("selected trace root is empty".into()));
    }
    Ok(std::path::PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(unix)]
impl TraceRoot {
    pub(crate) fn open_private_dir(&self, path: &TracePath) -> Result<PrivateDir, VerifierError> {
        use std::ffi::CString;
        use std::os::unix::io::{AsRawFd, FromRawFd};

        let mut current: Option<std::fs::File> = None;
        for component in path.0.split('/') {
            let name = CString::new(component.as_bytes()).map_err(|_| {
                VerifierError::Trace(format!(
                    "trace path {:?} contains an interior NUL",
                    path.display()
                ))
            })?;
            let parent_fd = current
                .as_ref()
                .map_or_else(|| self.0.as_raw_fd(), AsRawFd::as_raw_fd);
            // SAFETY: the parent descriptor is open and the name is a live NUL-terminated CString for the call; the flags include O_NOFOLLOW so no final symlink is followed.
            let fd = unsafe {
                libc::openat(
                    parent_fd,
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(VerifierError::Io {
                    path: path.display().to_owned(),
                    source: std::io::Error::last_os_error(),
                });
            }
            // SAFETY: the descriptor was just returned by a successful system call (checked non-negative), nothing else owns it, and ownership moves into the wrapper.
            current = Some(unsafe { std::fs::File::from_raw_fd(fd) });
        }
        Ok(PrivateDir {
            file: current.expect("TracePath has at least one component"),
            display: path.display().to_owned(),
        })
    }

    pub(crate) fn open(root: &Path) -> Result<Self, VerifierError> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::io::FromRawFd;

        let selected = selected_root_name(root)?;
        let path = CString::new(selected.as_os_str().as_bytes()).map_err(|_| {
            VerifierError::Trace("selected trace root contains an interior NUL".into())
        })?;
        // Ancestors use ordinary OS traversal (so /var aliases work); spelling a final
        // root as `link/` or `link/.` is normalized back to `link` before this no-follow
        // open, so the selected final directory can never be a symlink.
        // SAFETY: the path is a live NUL-terminated CString for the call; O_NOFOLLOW and O_DIRECTORY refuse a symlinked or non-directory root.
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(VerifierError::Io {
                path: "trace root".to_string(),
                source: std::io::Error::last_os_error(),
            });
        }
        // SAFETY: the descriptor was just returned by a successful system call (checked non-negative), nothing else owns it, and ownership moves into the wrapper.
        Ok(Self(std::sync::Arc::new(unsafe {
            std::fs::File::from_raw_fd(fd)
        })))
    }

    /// Open one regular descendant and return the exact retained-root handle acquired by the
    /// no-follow traversal. Streaming and bounded readers consume this handle directly; no
    /// checked path is reopened by name.
    pub(crate) fn open_reader(&self, path: &TracePath) -> Result<std::fs::File, VerifierError> {
        use std::ffi::CString;
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::io::{AsRawFd, FromRawFd};

        let components: Vec<_> = path
            .0
            .split('/')
            .map(|component| component.as_bytes().to_owned())
            .collect();
        let mut current: Option<std::fs::File> = None;
        for (i, component) in components.iter().enumerate() {
            let name = CString::new(component.as_slice()).map_err(|_| {
                VerifierError::Trace(format!(
                    "trace path {:?} contains an interior NUL",
                    path.display()
                ))
            })?;
            let parent_fd = current
                .as_ref()
                .map_or_else(|| self.0.as_raw_fd(), std::os::unix::io::AsRawFd::as_raw_fd);
            let last = i + 1 == components.len();
            let flags = if last {
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC
            } else {
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC
            };
            // SAFETY: the parent descriptor is open and the name is a live NUL-terminated CString for the call; the flags include O_NOFOLLOW so no final symlink is followed.
            let fd = unsafe { libc::openat(parent_fd, name.as_ptr(), flags) };
            if fd < 0 {
                return Err(VerifierError::Io {
                    path: path.display().to_string(),
                    source: std::io::Error::last_os_error(),
                });
            }
            // SAFETY: the descriptor was just returned by a successful system call (checked non-negative), nothing else owns it, and ownership moves into the wrapper.
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            if last {
                let metadata = file.metadata().map_err(|source| VerifierError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
                if !metadata.is_file() {
                    return Err(VerifierError::Trace(format!(
                        "trace path {:?} is not a regular file",
                        path.display()
                    )));
                }
                if metadata.nlink() != 1 {
                    return Err(VerifierError::Trace(format!(
                        "trace path {:?} is hard linked",
                        path.display()
                    )));
                }
                return Ok(file);
            }
            current = Some(file);
        }
        unreachable!("TracePath has at least one component")
    }
}

#[cfg(unix)]
impl PrivateDir {
    pub(crate) fn open_private_dir_child(&self, name: &str) -> Result<PrivateDir, VerifierError> {
        use std::ffi::CString;
        use std::os::unix::io::{AsRawFd, FromRawFd};

        if name.is_empty() || name.contains(['/', '\\', '\0']) || matches!(name, "." | "..") {
            return Err(VerifierError::Trace(format!(
                "trace directory {:?} contains an invalid child name",
                self.display
            )));
        }
        let name_c = CString::new(name).map_err(|_| {
            VerifierError::Trace(format!(
                "trace directory {:?} contains an invalid child name",
                self.display
            ))
        })?;
        // SAFETY: the parent descriptor is open and the name is a live NUL-terminated CString for the call; the flags include O_NOFOLLOW so no final symlink is followed.
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(VerifierError::Io {
                path: format!("{}/{name}", self.display),
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(PrivateDir {
            // SAFETY: the descriptor was just returned by a successful system call (checked non-negative), nothing else owns it, and ownership moves into the wrapper.
            file: unsafe { std::fs::File::from_raw_fd(fd) },
            display: format!("{}/{name}", self.display),
        })
    }

    pub(crate) fn open_private_regular_child(
        &self,
        name: &str,
    ) -> Result<std::fs::File, VerifierError> {
        use std::ffi::CString;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::os::unix::io::{AsRawFd, FromRawFd};

        if name.is_empty() || name.contains(['/', '\\', '\0']) || matches!(name, "." | "..") {
            return Err(VerifierError::Trace(format!(
                "trace directory {:?} contains an invalid child name",
                self.display
            )));
        }
        let name_c = CString::new(name).map_err(|_| {
            VerifierError::Trace(format!(
                "trace directory {:?} contains an invalid child name",
                self.display
            ))
        })?;
        // SAFETY: the parent descriptor is open and the name is a live NUL-terminated CString for the call; the flags include O_NOFOLLOW so no final symlink is followed.
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name_c.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(VerifierError::Io {
                path: format!("{}/{name}", self.display),
                source: std::io::Error::last_os_error(),
            });
        }
        // SAFETY: the descriptor was just returned by a successful system call (checked non-negative), nothing else owns it, and ownership moves into the wrapper.
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        let metadata = file.metadata().map_err(|source| VerifierError::Io {
            path: format!("{}/{name}", self.display),
            source,
        })?;
        if !metadata.is_file() {
            return Err(VerifierError::Trace(format!(
                "trace path {:?} is not a regular file",
                format!("{}/{name}", self.display)
            )));
        }
        if metadata.nlink() != 1 {
            return Err(VerifierError::Trace(format!(
                "trace path {:?} is hard linked",
                format!("{}/{name}", self.display)
            )));
        }
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(VerifierError::Trace(format!(
                "trace path {:?} is not an owner-only 0600 private file",
                format!("{}/{name}", self.display)
            )));
        }
        Ok(file)
    }
}

impl TraceRoot {
    /// Read one inherently-whole document through the retained handle, capped at `limit`.
    pub(crate) fn read_bytes_limited(
        &self,
        path: &TracePath,
        limit: u64,
    ) -> Result<Vec<u8>, VerifierError> {
        use std::io::Read;

        let file = self.open_reader(path)?;
        let length = file
            .metadata()
            .map_err(|source| VerifierError::Io {
                path: path.display().to_owned(),
                source,
            })?
            .len();
        if length > limit {
            return Err(VerifierError::EvidenceTooLarge {
                path: path.display().to_owned(),
                limit,
            });
        }
        let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
        let read = file
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| VerifierError::Io {
                path: path.display().to_owned(),
                source,
            })?;
        if u64::try_from(read).unwrap_or(u64::MAX) > limit {
            return Err(VerifierError::EvidenceTooLarge {
                path: path.display().to_owned(),
                limit,
            });
        }
        Ok(bytes)
    }

    pub(crate) fn read_text_limited(
        &self,
        path: &TracePath,
        limit: u64,
    ) -> Result<String, VerifierError> {
        let bytes = self.read_bytes_limited(path, limit)?;
        String::from_utf8(bytes).map_err(|source| {
            VerifierError::Trace(format!(
                "trace path {:?} is not UTF-8: {source}",
                path.display()
            ))
        })
    }
}

#[cfg(unix)]
impl PrivateDir {
    /// Enumerate one retained directory through a no-follow descriptor-relative handle.
    ///
    /// `read_dir(path)` would reopen a participant-controlled path after validation.  This
    /// duplicates the root directory descriptor and asks `readdir` to enumerate that exact
    /// handle instead. Names are returned sorted, and callers open each selected child
    /// relative to this same retained directory capability before reading bytes.
    pub(crate) fn child_names(&self, limit: u64) -> Result<Vec<String>, VerifierError> {
        use std::ffi::CStr;
        use std::os::unix::io::AsRawFd;

        // SAFETY: the retained directory descriptor is open for the duration of the call; dup only creates a new descriptor.
        let duplicate = unsafe { libc::dup(self.file.as_raw_fd()) };
        if duplicate < 0 {
            return Err(VerifierError::Io {
                path: self.display.clone(),
                source: std::io::Error::last_os_error(),
            });
        }
        // SAFETY: `duplicate` is an open directory descriptor owned here; on success the stream takes it over, and the failure path closes it.
        let stream = unsafe { libc::fdopendir(duplicate) };
        if stream.is_null() {
            let source = std::io::Error::last_os_error();
            // SAFETY: `duplicate` is owned here and the stream did not take it over, so closing it exactly once is sound.
            unsafe { libc::close(duplicate) };
            return Err(VerifierError::Io {
                path: self.display.clone(),
                source,
            });
        }
        // `dup` retains the same open-file-description offset. Rewind the duplicated
        // stream so repeated stability snapshots always enumerate from the beginning.
        // SAFETY: the stream is non-null (checked above), still open and used only by this thread.
        unsafe { libc::rewinddir(stream) };

        let mut names = Vec::new();
        loop {
            set_errno_zero();
            // SAFETY: the stream is non-null, open and used only by this thread; a null return is distinguished from an error through errno.
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                let source = std::io::Error::last_os_error();
                if source.raw_os_error().unwrap_or(0) != 0 {
                    // SAFETY: the stream is non-null and still open; it is closed exactly once on this path and not used afterwards.
                    unsafe { libc::closedir(stream) };
                    return Err(VerifierError::Io {
                        path: self.display.clone(),
                        source,
                    });
                }
                break;
            }
            // SAFETY: `entry` is non-null from readdir; d_name is NUL terminated and stays valid until the next readdir/closedir, and the bytes are copied or inspected before then.
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if matches!(bytes, b"." | b"..") {
                continue;
            }
            let name = match std::str::from_utf8(bytes) {
                Ok(name) => name,
                Err(_) => {
                    // SAFETY: the stream is non-null and still open; it is closed exactly once on this path and not used afterwards.
                    unsafe { libc::closedir(stream) };
                    return Err(VerifierError::Trace(format!(
                        "trace directory {:?} contains a non-UTF-8 entry name",
                        self.display
                    )));
                }
            };
            if name.is_empty() {
                // SAFETY: the stream is non-null and still open; it is closed exactly once on this path and not used afterwards.
                unsafe { libc::closedir(stream) };
                return Err(VerifierError::Trace(format!(
                    "trace directory {:?} contains an empty entry name",
                    self.display
                )));
            }
            if u64::try_from(names.len()).unwrap_or(u64::MAX) >= limit {
                // SAFETY: the stream is non-null and still open; it is closed exactly once on this path and not used afterwards.
                unsafe { libc::closedir(stream) };
                return Err(VerifierError::Trace(format!(
                    "trace directory {:?} exceeds its bounded child count of {limit}",
                    self.display
                )));
            }
            names.push(name.to_owned());
        }
        // SAFETY: the stream is non-null and still open; it is closed exactly once on this path and not used afterwards.
        if unsafe { libc::closedir(stream) } != 0 {
            return Err(VerifierError::Io {
                path: self.display.clone(),
                source: std::io::Error::last_os_error(),
            });
        }
        names.sort();
        Ok(names)
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_errno_zero() {
    // SAFETY: the platform errno accessor returns a valid, writable thread-local pointer.
    unsafe { *libc::__errno_location() = 0 };
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn set_errno_zero() {
    // SAFETY: the platform errno accessor returns a valid, writable thread-local pointer.
    unsafe { *libc::__error() = 0 };
}

#[cfg(any(target_os = "solaris", target_os = "illumos"))]
fn set_errno_zero() {
    // SAFETY: the platform errno accessor returns a valid, writable thread-local pointer.
    unsafe { *libc::___errno() = 0 };
}

#[cfg(not(unix))]
impl PrivateDir {
    pub(crate) fn child_names(&self, _limit: u64) -> Result<Vec<String>, VerifierError> {
        Err(VerifierError::Trace(
            "handle-relative directory enumeration is unsupported on this platform".into(),
        ))
    }

    pub(crate) fn open_private_regular_child(
        &self,
        _name: &str,
    ) -> Result<std::fs::File, VerifierError> {
        Err(VerifierError::Trace(
            "private submission storage is unsupported on this platform".into(),
        ))
    }

    pub(crate) fn open_private_dir_child(&self, _name: &str) -> Result<PrivateDir, VerifierError> {
        Err(VerifierError::Trace(
            "private submission storage is unsupported on this platform".into(),
        ))
    }
}

#[cfg(not(unix))]
impl TraceRoot {
    pub(crate) fn open_private_dir(&self, _path: &TracePath) -> Result<PrivateDir, VerifierError> {
        Err(VerifierError::Trace(
            "private submission storage requires Unix capability and mode validation".into(),
        ))
    }
}

#[cfg(not(unix))]
#[derive(Debug, Clone)]
pub(crate) struct TraceRoot;

#[cfg(not(unix))]
impl TraceRoot {
    pub fn open(_root: &Path) -> Result<Self, VerifierError> {
        Err(VerifierError::Trace(
            "trace containment is unsupported on this platform".into(),
        ))
    }
    pub fn open_reader(&self, _path: &TracePath) -> Result<std::fs::File, VerifierError> {
        Err(VerifierError::Trace(
            "trace containment is unsupported on this platform".into(),
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{TracePath, TraceRoot, selected_root_name};
    use std::io::Read;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn selected_root_name_preserves_the_filesystem_root_and_rejects_empty_input() {
        for spelling in ["/", "/.", "/./", "/./."] {
            assert_eq!(
                selected_root_name(std::path::Path::new(spelling)).unwrap(),
                std::path::Path::new("/"),
                "{spelling:?} must keep selecting the filesystem root"
            );
        }
        assert!(selected_root_name(std::path::Path::new("")).is_err());
    }

    #[test]
    fn descriptor_path_grammar_rejects_normalized_or_ambiguous_forms() {
        for path in ["", "a//b", "a/./b", "a/../b", "a/", "a\0b"] {
            assert!(TracePath::parse(path).is_err(), "{path:?} must be rejected");
        }
        assert!(TracePath::parse("evidence/leaf.json").is_ok());
    }

    #[test]
    fn selected_final_root_never_follows_link_spelling_variants() {
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("real");
        std::fs::create_dir(&target).unwrap();
        let link = parent.path().join("link");
        symlink(&target, &link).unwrap();
        for spelling in [
            link.clone(),
            std::path::PathBuf::from(format!("{}/", link.display())),
            std::path::PathBuf::from(format!("{}/.", link.display())),
        ] {
            assert!(
                TraceRoot::open(&spelling).is_err(),
                "{}",
                spelling.display()
            );
        }
        assert!(TraceRoot::open(&target).is_ok());
        assert!(TraceRoot::open(std::path::Path::new(".")).is_ok());

        let real_parent = parent.path().join("real-parent");
        std::fs::create_dir(&real_parent).unwrap();
        let aliased_target = real_parent.join("trace");
        std::fs::create_dir(&aliased_target).unwrap();
        let ancestor_alias = parent.path().join("ancestor-alias");
        symlink(&real_parent, &ancestor_alias).unwrap();
        assert!(TraceRoot::open(&ancestor_alias.join("trace")).is_ok());
    }

    #[test]
    fn acquired_root_survives_root_and_descendant_replacements() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("trace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("trace.toml"), "trusted descriptor").unwrap();
        std::fs::create_dir(root.join("evidence")).unwrap();
        std::fs::write(root.join("evidence/manifest.toml"), "trusted manifest").unwrap();
        let cap = TraceRoot::open(&root).unwrap();
        let descriptor = TracePath::parse("trace.toml").unwrap();
        let manifest = TracePath::parse("evidence/manifest.toml").unwrap();

        let moved = parent.path().join("moved");
        std::fs::rename(&root, &moved).unwrap();
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("trace.toml"), "OUTSIDE_SENTINEL").unwrap();
        std::fs::create_dir(root.join("evidence")).unwrap();
        std::fs::write(root.join("evidence/manifest.toml"), "OUTSIDE_SENTINEL").unwrap();

        assert_eq!(
            cap.read_text_limited(&descriptor, 1024).unwrap(),
            "trusted descriptor"
        );
        assert_eq!(
            cap.read_text_limited(&manifest, 1024).unwrap(),
            "trusted manifest"
        );
    }

    #[test]
    fn retained_private_directory_binds_enumeration_and_child_open_to_one_handle() {
        let parent = tempfile::tempdir().unwrap();
        let object_dir = parent.path().join("submission/objects/sha256");
        std::fs::create_dir_all(&object_dir).unwrap();
        let digest = "a".repeat(64);
        let object = object_dir.join(&digest);
        std::fs::write(&object, b"trusted").unwrap();
        std::fs::set_permissions(&object, std::fs::Permissions::from_mode(0o600)).unwrap();

        let root = TraceRoot::open(parent.path()).unwrap();
        let retained = root
            .open_private_dir(&TracePath::parse("submission/objects/sha256").unwrap())
            .unwrap();
        assert_eq!(retained.child_names(2).unwrap(), vec![digest.clone()]);

        let moved = parent.path().join("retained-objects");
        std::fs::rename(parent.path().join("submission/objects"), &moved).unwrap();
        std::fs::create_dir_all(&object_dir).unwrap();
        let replacement = object_dir.join(&digest);
        std::fs::write(&replacement, b"replacement").unwrap();
        std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();

        let mut bytes = Vec::new();
        retained
            .open_private_regular_child(&digest)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"trusted");
    }
}
