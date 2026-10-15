use super::{CleanupBudget, RefusalKind, TraceCapError};
use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const ROOT_OPERATION: &str = "open trace root";

pub(super) struct TraceDirHandle {
    fd: OwnedFd,
}

pub(super) struct AppendFileHandle {
    file: File,
    identity: Identity,
}

pub(super) struct FixedWriterHandle {
    file: File,
    identity: Identity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Identity {
    device: libc::dev_t,
    inode: libc::ino_t,
}

impl TraceDirHandle {
    pub(super) fn open(root: &Path) -> Result<Self, TraceCapError> {
        let path = CString::new(root.as_os_str().as_bytes())
            .map_err(|_| TraceCapError::refused(ROOT_OPERATION, RefusalKind::UnsupportedRoot))?;
        let raw = unsafe {
            // SAFETY: `path` is NUL terminated and remains live through the call. O_NOFOLLOW
            // refuses a final symlink, and O_CLOEXEC is applied atomically at acquisition.
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let fd = owned_fd(raw, ROOT_OPERATION)?;
        let stat = fstat(fd.as_raw_fd(), ROOT_OPERATION)?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(TraceCapError::refused(
                ROOT_OPERATION,
                RefusalKind::NotDirectory,
            ));
        }
        Ok(Self { fd })
    }

    pub(super) fn identity(&self) -> Result<String, TraceCapError> {
        let stat = fstat(self.fd.as_raw_fd(), "identify trace root")?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(TraceCapError::refused(
                "identify trace root",
                RefusalKind::NotDirectory,
            ));
        }
        Ok(format!("unix:{}:{}", stat.st_dev, stat.st_ino))
    }

    pub(super) fn restrict(&self, mode: u32) -> Result<(), TraceCapError> {
        let result = unsafe {
            // SAFETY: this capability owns a live directory descriptor and mode contains
            // only the permission bits selected by the fixed caller.
            libc::fchmod(self.fd.as_raw_fd(), mode as libc::mode_t)
        };
        if result != 0 {
            return Err(TraceCapError::io(
                "restrict contained directory",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    pub(super) fn remove_contents(
        &self,
        budget: &mut CleanupBudget,
        depth: usize,
    ) -> Result<(), TraceCapError> {
        budget.enter(depth)?;
        let mut names = Vec::new();
        self.for_each_name(|name| {
            budget.account()?;
            names.push(name.to_os_string());
            Ok(())
        })?;
        for name in names {
            self.remove_child_tree(&name, budget, depth.saturating_add(1))?;
        }
        Ok(())
    }

    pub(super) fn remove_child_dir_matching(
        &self,
        name: &str,
        child: &Self,
    ) -> Result<(), TraceCapError> {
        self.remove_child_dir_matching_os(OsStr::new(name), child)
    }

    fn remove_child_tree(
        &self,
        name: &OsStr,
        budget: &mut CleanupBudget,
        depth: usize,
    ) -> Result<(), TraceCapError> {
        let encoded = c_os_name(name)?;
        let raw = unsafe {
            // SAFETY: name is one entry returned by this retained directory's enumerator.
            libc::openat(
                self.fd.as_raw_fd(),
                encoded.as_ptr(),
                libc::O_RDONLY
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
            )
        };
        if raw >= 0 {
            let child = Self {
                fd: owned_fd(raw, "open artifact cleanup directory")?,
            };
            let stat = fstat(child.fd.as_raw_fd(), "open artifact cleanup directory")?;
            if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
                return Err(TraceCapError::refused(
                    "open artifact cleanup directory",
                    RefusalKind::NotDirectory,
                ));
            }
            child.remove_contents(budget, depth)?;
            return self.remove_child_dir_matching_os(name, &child);
        }

        let open_error = io::Error::last_os_error();
        let mut stat = MaybeUninit::<libc::stat>::uninit();
        let observed = unsafe {
            // SAFETY: the parent and encoded name are live and AT_SYMLINK_NOFOLLOW
            // classifies the entry itself rather than a target it may name.
            libc::fstatat(
                self.fd.as_raw_fd(),
                encoded.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if observed != 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(TraceCapError::io("inspect artifact cleanup child", error))
            };
        }
        // SAFETY: the preceding system call reported success, so it fully initialised the output buffer.
        let stat = unsafe { stat.assume_init() };
        if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
            return Err(TraceCapError::io(
                "open artifact cleanup directory",
                open_error,
            ));
        }
        let removed = unsafe {
            // SAFETY: unlinkat without AT_REMOVEDIR removes only this directory entry and
            // never follows a symlink. A raced directory is refused by the kernel.
            libc::unlinkat(self.fd.as_raw_fd(), encoded.as_ptr(), 0)
        };
        if removed != 0 {
            return Err(TraceCapError::io(
                "remove artifact cleanup child",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    fn remove_child_dir_matching_os(
        &self,
        name: &OsStr,
        child: &Self,
    ) -> Result<(), TraceCapError> {
        let encoded = c_os_name(name)?;
        let expected = fstat(child.fd.as_raw_fd(), "identify artifact cleanup directory")?;
        let mut named = MaybeUninit::<libc::stat>::uninit();
        let observed = unsafe {
            // SAFETY: the retained parent and direct name are live and output is writable.
            libc::fstatat(
                self.fd.as_raw_fd(),
                encoded.as_ptr(),
                named.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if observed != 0 {
            return Err(TraceCapError::io(
                "match artifact cleanup directory",
                io::Error::last_os_error(),
            ));
        }
        // SAFETY: the preceding system call reported success, so it fully initialised the output buffer.
        let named = unsafe { named.assume_init() };
        if expected.st_dev != named.st_dev
            || expected.st_ino != named.st_ino
            || named.st_mode & libc::S_IFMT != libc::S_IFDIR
        {
            return Err(TraceCapError::refused(
                "match artifact cleanup directory",
                RefusalKind::IdentityChanged,
            ));
        }
        let removed = unsafe {
            // SAFETY: identity was checked against the retained child immediately above;
            // a later exchange can only make rmdir fail or remove another empty child of
            // this retained parent, never follow it outside.
            libc::unlinkat(self.fd.as_raw_fd(), encoded.as_ptr(), libc::AT_REMOVEDIR)
        };
        if removed != 0 {
            return Err(TraceCapError::io(
                "remove artifact cleanup directory",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    pub(super) fn open_child_dir(&self, name: &str) -> Result<Self, TraceCapError> {
        let operation = "open contained directory";
        let name = c_name(name);
        let raw = unsafe {
            // SAFETY: this is one validated direct component beneath the retained parent.
            libc::openat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let fd = owned_fd(raw, operation)?;
        let stat = fstat(fd.as_raw_fd(), operation)?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(TraceCapError::refused(operation, RefusalKind::NotDirectory));
        }
        Ok(Self { fd })
    }

    pub(super) fn create_child_dir(&self, name: &str) -> Result<Self, TraceCapError> {
        let operation = "create contained directory";
        let name = c_name(name);
        let result = unsafe {
            // SAFETY: this is one validated direct component beneath the retained parent.
            libc::mkdirat(self.fd.as_raw_fd(), name.as_ptr(), 0o700)
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(TraceCapError::refused(
                    operation,
                    RefusalKind::EntryAlreadyExists,
                ));
            }
            return Err(TraceCapError::io(operation, error));
        }
        // Re-acquire by the retained parent, rejecting a replacement before it becomes a
        // capability.  A concurrent rename can make this fail closed but cannot redirect it.
        self.open_child_dir(name.to_str().expect("validated component is UTF-8"))
    }

    /// Open the child if it exists, else create it exclusively, and verify what was opened.
    ///
    /// Not one `O_CREAT` open: on macOS (APFS), two processes that `openat(O_CREAT)` the same
    /// absent name at the same moment can see one of them fail with `ENOENT`. Two daemons
    /// starting together on a fresh artifact root did exactly that on `registry.redb`, so the
    /// loser exited with "No such file or directory" instead of reaching the registry lock.
    /// Open-existing, then `O_CREAT | O_EXCL`, is race-free: `ENOENT` on the first means
    /// "absent, try to create", `EEXIST` on the second means "someone else created it, open
    /// theirs". A name that keeps flipping between the two is refused after a few rounds.
    pub(super) fn open_or_create_regular(&self, name: &str) -> Result<File, TraceCapError> {
        const ROUNDS: usize = 8;
        let operation = "open contained regular file";
        let name = c_name(name);
        let base = libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        let mut last = io::Error::from(io::ErrorKind::NotFound);
        for _ in 0..ROUNDS {
            let raw = unsafe {
                // SAFETY: root is retained and name is one validated direct child.
                libc::openat(self.fd.as_raw_fd(), name.as_ptr(), base)
            };
            if raw >= 0 {
                let fd = owned_fd(raw, operation)?;
                validate_regular(fd.as_raw_fd(), operation)?;
                return Ok(File::from(fd));
            }
            last = io::Error::last_os_error();
            if last.kind() != io::ErrorKind::NotFound {
                return Err(TraceCapError::io(operation, last));
            }
            let raw = unsafe {
                // SAFETY: as above; O_EXCL makes absence part of the one create call.
                libc::openat(
                    self.fd.as_raw_fd(),
                    name.as_ptr(),
                    base | libc::O_CREAT | libc::O_EXCL,
                    0o600,
                )
            };
            if raw >= 0 {
                let fd = owned_fd(raw, operation)?;
                validate_regular(fd.as_raw_fd(), operation)?;
                return Ok(File::from(fd));
            }
            last = io::Error::last_os_error();
            if last.kind() != io::ErrorKind::AlreadyExists {
                return Err(TraceCapError::io(operation, last));
            }
        }
        Err(TraceCapError::io(operation, last))
    }

    pub(super) fn sync(&self) -> Result<(), TraceCapError> {
        let result = unsafe {
            // SAFETY: the retained directory descriptor is live for the call.
            libc::fsync(self.fd.as_raw_fd())
        };
        if result != 0 {
            return Err(TraceCapError::io(
                "sync trace directory",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    pub(super) fn read_regular(&self, name: &str) -> Result<Option<File>, TraceCapError> {
        let operation = "read contained regular file";
        let name = c_name(name);
        let raw = unsafe {
            // SAFETY: root is retained and name is one validated direct child.
            libc::openat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if raw < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(TraceCapError::io(operation, error))
            };
        }
        let fd = owned_fd(raw, operation)?;
        validate_regular(fd.as_raw_fd(), operation)?;
        Ok(Some(File::from(fd)))
    }

    fn remove_regular(&self, name: &str) -> Result<(), TraceCapError> {
        let operation = "remove contained regular file";
        let name = c_name(name);
        let raw = unsafe {
            // SAFETY: root is retained and name is one validated direct child.
            libc::openat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if raw < 0 && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            return Ok(());
        }
        let fd = owned_fd(raw, operation)?;
        let (identity, _) = validate_regular(fd.as_raw_fd(), operation)?;
        let mut named = MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            // SAFETY: root/name remain live and output storage is valid.
            libc::fstatat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                named.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            return Err(TraceCapError::io(operation, io::Error::last_os_error()));
        }
        // SAFETY: the preceding system call reported success, so it fully initialised the output buffer.
        let named = unsafe { named.assume_init() };
        if identity
            != (Identity {
                device: named.st_dev,
                inode: named.st_ino,
            })
        {
            return Err(TraceCapError::refused(
                operation,
                RefusalKind::IdentityChanged,
            ));
        }
        let result = unsafe {
            // SAFETY: this exact regular leaf was acquired and identity checked above.
            libc::unlinkat(self.fd.as_raw_fd(), name.as_ptr(), 0)
        };
        if result != 0 {
            return Err(TraceCapError::io(operation, io::Error::last_os_error()));
        }
        Ok(())
    }

    pub(super) fn open_append(
        &self,
        name: &str,
        create: bool,
    ) -> Result<AppendFileHandle, TraceCapError> {
        let operation = "open trace append stream";
        let name = c_name(name);
        let base_flags =
            libc::O_RDWR | libc::O_APPEND | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        let raw = if create {
            let exclusive = unsafe {
                // SAFETY: root is a live directory fd and name is one fixed NUL-terminated child.
                libc::openat(
                    self.fd.as_raw_fd(),
                    name.as_ptr(),
                    base_flags | libc::O_CREAT | libc::O_EXCL,
                    0o666,
                )
            };
            if exclusive >= 0 {
                exclusive
            } else {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(TraceCapError::io(operation, error));
                }
                unsafe {
                    // SAFETY: same fixed child and retained root; O_NOFOLLOW remains in force.
                    libc::openat(self.fd.as_raw_fd(), name.as_ptr(), base_flags)
                }
            }
        } else {
            unsafe {
                // SAFETY: same fixed child and retained root; O_NOFOLLOW remains in force.
                libc::openat(self.fd.as_raw_fd(), name.as_ptr(), base_flags)
            }
        };
        let fd = owned_fd(raw, operation)?;
        let (identity, _) = validate_regular(fd.as_raw_fd(), operation)?;
        Ok(AppendFileHandle {
            file: File::from(fd),
            identity,
        })
    }

    pub(super) fn entry_exists(&self, name: &str) -> Result<bool, TraceCapError> {
        let name = c_name(name);
        let mut stat = MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            // SAFETY: root/name are live and stat points to writable output storage. The flag
            // observes the directory entry itself rather than following a symlink.
            libc::fstatat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(TraceCapError::io("inspect fixed trace document", error))
        }
    }

    pub(super) fn open_optional_reader(
        &self,
        name: &'static str,
    ) -> Result<Option<File>, TraceCapError> {
        let operation = "open fixed trace document";
        let raw = unsafe {
            // SAFETY: root is retained and name is a fixed NUL-terminated child.
            libc::openat(
                self.fd.as_raw_fd(),
                c_name(name).as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if raw < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(TraceCapError::io(operation, error))
            };
        }
        let fd = owned_fd(raw, operation)?;
        validate_regular(fd.as_raw_fd(), operation)?;
        Ok(Some(File::from(fd)))
    }

    pub(super) fn create_fixed(
        &self,
        name: &'static str,
    ) -> Result<FixedWriterHandle, TraceCapError> {
        let operation = "create fixed trace document";
        let raw = unsafe {
            // SAFETY: root is retained and name is a fixed NUL-terminated child. O_EXCL makes
            // absence a write-time condition; O_NOFOLLOW prevents link traversal.
            libc::openat(
                self.fd.as_raw_fd(),
                c_name(name).as_ptr(),
                libc::O_WRONLY
                    | libc::O_CREAT
                    | libc::O_EXCL
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
                0o666,
            )
        };
        if raw < 0 && io::Error::last_os_error().kind() == io::ErrorKind::AlreadyExists {
            return Err(TraceCapError::refused(
                operation,
                RefusalKind::EntryAlreadyExists,
            ));
        }
        let fd = owned_fd(raw, operation)?;
        let (identity, _) = validate_regular(fd.as_raw_fd(), operation)?;
        Ok(FixedWriterHandle {
            file: File::from(fd),
            identity,
        })
    }

    pub(super) fn open_matching_reader(
        &self,
        name: &str,
        writer: &AppendFileHandle,
    ) -> Result<File, TraceCapError> {
        let operation = "open matching trace reader";
        let raw = unsafe {
            // SAFETY: root is retained and the child name came from the writer capability.
            libc::openat(
                self.fd.as_raw_fd(),
                c_name(name).as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        let fd = owned_fd(raw, operation)?;
        let (identity, _) = validate_regular(fd.as_raw_fd(), operation)?;
        if identity != writer.identity {
            return Err(TraceCapError::refused(
                operation,
                RefusalKind::IdentityChanged,
            ));
        }
        Ok(File::from(fd))
    }

    /// Require the fixed child name to still be the append writer's own regular file.
    pub(super) fn check_append_binding(
        &self,
        name: &str,
        writer: &AppendFileHandle,
    ) -> Result<(), TraceCapError> {
        let operation = "check trace append stream name";
        let name = c_name(name);
        let mut named = MaybeUninit::<libc::stat>::uninit();
        let observed = unsafe {
            // SAFETY: the retained parent and fixed child name are live, output is writable,
            // and AT_SYMLINK_NOFOLLOW observes the entry itself rather than a link target.
            libc::fstatat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                named.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if observed != 0 {
            return Err(TraceCapError::io(operation, io::Error::last_os_error()));
        }
        // SAFETY: the preceding system call reported success, so it fully initialised the output buffer.
        let named = unsafe { named.assume_init() };
        if named.st_mode & libc::S_IFMT != libc::S_IFREG
            || writer.identity
                != (Identity {
                    device: named.st_dev,
                    inode: named.st_ino,
                })
        {
            return Err(TraceCapError::refused(
                operation,
                RefusalKind::IdentityChanged,
            ));
        }
        Ok(())
    }

    pub(super) fn replace_descriptor(
        &self,
        temporary: &str,
        bytes: &[u8],
    ) -> Result<(), TraceCapError> {
        self.replace_fixed(temporary, "trace.toml", bytes)
    }

    pub(super) fn replace_fixed(
        &self,
        temporary: &str,
        target_name: &'static str,
        bytes: &[u8],
    ) -> Result<(), TraceCapError> {
        self.replace_named(temporary, target_name, bytes)
    }

    fn replace_named(
        &self,
        temporary: &str,
        target_name: &str,
        bytes: &[u8],
    ) -> Result<(), TraceCapError> {
        let temporary = c_name(temporary);
        let target = c_name(target_name);
        let mode = self.fixed_document_mode(&target)?;
        let mut created_temporary = false;
        let result = (|| {
            let operation = "create fixed-document temporary";
            let raw = unsafe {
                // SAFETY: root and canonical temporary name are live for the call.
                libc::openat(
                    self.fd.as_raw_fd(),
                    temporary.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC
                        | libc::O_NONBLOCK,
                    mode.unwrap_or(0o666) as libc::c_uint,
                )
            };
            let fd = owned_fd(raw, operation)?;
            created_temporary = true;
            let (identity, _) = validate_regular(fd.as_raw_fd(), operation)?;
            let mut file = File::from(fd);
            file.write_all(bytes)
                .map_err(|source| TraceCapError::io("write fixed-document temporary", source))?;
            validate_same_regular(file.as_raw_fd(), identity, "check fixed-document temporary")?;
            if let Some(mode) = mode {
                let result = unsafe {
                    // SAFETY: file owns a live descriptor; only permission bits are supplied.
                    libc::fchmod(file.as_raw_fd(), mode)
                };
                if result != 0 {
                    return Err(TraceCapError::io(
                        "carry fixed-document mode",
                        io::Error::last_os_error(),
                    ));
                }
            }
            file.sync_all()
                .map_err(|source| TraceCapError::io("sync fixed-document temporary", source))?;
            validate_same_regular(file.as_raw_fd(), identity, "check fixed-document temporary")?;
            let renamed = unsafe {
                // SAFETY: both names are direct children of the same retained directory.
                libc::renameat(
                    self.fd.as_raw_fd(),
                    temporary.as_ptr(),
                    self.fd.as_raw_fd(),
                    target.as_ptr(),
                )
            };
            if renamed != 0 {
                return Err(TraceCapError::io(
                    "replace fixed trace document",
                    io::Error::last_os_error(),
                ));
            }
            Ok(())
        })();
        if result.is_err() && created_temporary {
            unsafe {
                // SAFETY: cleanup names only the exclusive temporary under the retained root.
                libc::unlinkat(self.fd.as_raw_fd(), temporary.as_ptr(), 0);
            }
        }
        result
    }

    fn fixed_document_mode(&self, target: &CStr) -> Result<Option<libc::mode_t>, TraceCapError> {
        #[cfg(test)]
        if FORCE_DESCRIPTOR_OBSERVATION_ERROR.with(std::cell::Cell::take) {
            return Err(TraceCapError::io(
                "inspect fixed trace document",
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected observation failure",
                ),
            ));
        }
        let mut stat = MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            // SAFETY: target/root are live and AT_SYMLINK_NOFOLLOW observes the entry itself.
            libc::fstatat(
                self.fd.as_raw_fd(),
                target.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            // SAFETY: the preceding system call reported success, so it fully initialised the output buffer.
            let stat = unsafe { stat.assume_init() };
            if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
                return Ok(None);
            }
            if stat.st_nlink != 1 {
                return Err(TraceCapError::refused(
                    "inspect fixed trace document",
                    RefusalKind::MultipleLinks,
                ));
            }
            return Ok(Some(stat.st_mode & 0o7777));
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(TraceCapError::io("inspect fixed trace document", error))
        }
    }

    pub(super) fn for_each_name(
        &self,
        mut visitor: impl FnMut(&OsStr) -> Result<(), TraceCapError>,
    ) -> Result<(), TraceCapError> {
        let duplicate = unsafe {
            // SAFETY: root fd is live. F_DUPFD_CLOEXEC duplicates and sets noninheritance in
            // one kernel operation, before another thread could spawn a child.
            libc::fcntl(self.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0)
        };
        let duplicate = owned_fd(duplicate, "duplicate trace directory for enumeration")?;
        let raw = duplicate.as_raw_fd();
        let directory = unsafe {
            // SAFETY: duplicate is a live directory descriptor. fdopendir assumes ownership.
            libc::fdopendir(raw)
        };
        if directory.is_null() {
            return Err(TraceCapError::io(
                "open trace directory enumeration",
                io::Error::last_os_error(),
            ));
        }
        std::mem::forget(duplicate);
        unsafe {
            // SAFETY: directory is a live DIR pointer. A duplicated descriptor shares the
            // root's open-file-description offset, so reset it before every bounded sweep.
            libc::rewinddir(directory);
        }

        let mut outcome = Ok(());
        loop {
            crate::errno::clear();
            let entry = unsafe {
                // SAFETY: directory remains live until closed below; readdir owns its buffer.
                libc::readdir(directory)
            };
            if entry.is_null() {
                let errno = crate::errno::get();
                if errno != 0 {
                    outcome = Err(TraceCapError::io(
                        "enumerate trace directory",
                        io::Error::from_raw_os_error(errno),
                    ));
                }
                break;
            }
            let bytes = unsafe {
                // SAFETY: d_name is NUL terminated for the lifetime of this readdir result.
                CStr::from_ptr((*entry).d_name.as_ptr())
            }
            .to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if let Err(error) = visitor(OsStr::from_bytes(bytes)) {
                outcome = Err(error);
                break;
            }
        }
        let closed = unsafe {
            // SAFETY: fdopendir returned this unique DIR pointer and it has not been closed.
            libc::closedir(directory)
        };
        if closed != 0 && outcome.is_ok() {
            outcome = Err(TraceCapError::io(
                "close trace directory enumeration",
                io::Error::last_os_error(),
            ));
        }
        outcome
    }

    pub(super) fn remove_owned_temp(&self, name: &OsStr) -> Result<(), TraceCapError> {
        self.remove_regular(name.to_str().ok_or_else(|| {
            TraceCapError::refused(
                "remove stale descriptor temporary",
                RefusalKind::NameTooLong,
            )
        })?)
    }
}

impl AppendFileHandle {
    pub(super) fn len(&self) -> Result<u64, TraceCapError> {
        let stat = fstat(self.file.as_raw_fd(), "measure trace append stream")?;
        u64::try_from(stat.st_size).map_err(|_| {
            TraceCapError::refused("measure trace append stream", RefusalKind::NotRegularFile)
        })
    }

    pub(super) fn validate(&self) -> Result<(), TraceCapError> {
        validate_same_regular(
            self.file.as_raw_fd(),
            self.identity,
            "check trace append stream",
        )
    }

    pub(super) fn write_all(&mut self, bytes: &[u8]) -> Result<(), TraceCapError> {
        self.file
            .write_all(bytes)
            .map_err(|source| TraceCapError::io("append trace stream", source))
    }

    pub(super) fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.file.read(buffer)
    }

    pub(super) fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }

    /// Flush data and length to stable storage. Rust's `sync_all` is `F_FULLFSYNC` on
    /// Apple platforms, not the weaker `fsync` that may stop at the drive cache.
    pub(super) fn sync_all(&self) -> Result<(), TraceCapError> {
        self.file
            .sync_all()
            .map_err(|source| TraceCapError::io("sync trace append stream", source))
    }
}

impl FixedWriterHandle {
    pub(super) fn validate(&self) -> Result<(), TraceCapError> {
        validate_same_regular(
            self.file.as_raw_fd(),
            self.identity,
            "check fixed trace document",
        )
    }

    pub(super) fn write_all(&mut self, bytes: &[u8]) -> Result<(), TraceCapError> {
        self.file
            .write_all(bytes)
            .map_err(|source| TraceCapError::io("write fixed trace document", source))
    }

    pub(super) fn sync_all(&self) -> Result<(), TraceCapError> {
        self.file
            .sync_all()
            .map_err(|source| TraceCapError::io("sync fixed trace document", source))
    }
}

/// `root` without a terminal `/` or `/.`, which the OS would resolve through a final link
/// before `O_NOFOLLOW` applies; `/` itself stays `/`. An empty root is refused.
pub(super) fn selected_root_name(root: &Path) -> Result<std::path::PathBuf, TraceCapError> {
    use std::os::unix::ffi::OsStringExt;

    let mut bytes = root.as_os_str().as_bytes().to_vec();
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
        return Err(TraceCapError::refused(
            ROOT_OPERATION,
            RefusalKind::UnsupportedRoot,
        ));
    }
    Ok(std::ffi::OsString::from_vec(bytes).into())
}

fn c_name(name: &str) -> CString {
    CString::new(name).expect("callers validated contained name before native traversal")
}

fn c_os_name(name: &OsStr) -> Result<CString, TraceCapError> {
    CString::new(name.as_bytes())
        .map_err(|_| TraceCapError::refused("remove artifact tree", RefusalKind::NameTooLong))
}

fn owned_fd(raw: RawFd, operation: &'static str) -> Result<OwnedFd, TraceCapError> {
    if raw < 0 {
        return Err(TraceCapError::io(operation, io::Error::last_os_error()));
    }
    Ok(unsafe {
        // SAFETY: a successful open/openat/fcntl returned one newly owned descriptor.
        OwnedFd::from_raw_fd(raw)
    })
}

fn fstat(fd: RawFd, operation: &'static str) -> Result<libc::stat, TraceCapError> {
    let mut stat = MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        // SAFETY: fd is live and stat is writable output storage of the required size.
        libc::fstat(fd, stat.as_mut_ptr())
    };
    if result != 0 {
        return Err(TraceCapError::io(operation, io::Error::last_os_error()));
    }
    // SAFETY: fstat reported success, so it fully initialised the output buffer.
    Ok(unsafe { stat.assume_init() })
}

fn validate_regular(
    fd: RawFd,
    operation: &'static str,
) -> Result<(Identity, libc::stat), TraceCapError> {
    let stat = fstat(fd, operation)?;
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(TraceCapError::refused(
            operation,
            RefusalKind::NotRegularFile,
        ));
    }
    if stat.st_nlink != 1 {
        return Err(TraceCapError::refused(
            operation,
            RefusalKind::MultipleLinks,
        ));
    }
    Ok((
        Identity {
            device: stat.st_dev,
            inode: stat.st_ino,
        },
        stat,
    ))
}

fn validate_same_regular(
    fd: RawFd,
    identity: Identity,
    operation: &'static str,
) -> Result<(), TraceCapError> {
    let (current, _) = validate_regular(fd, operation)?;
    if current != identity {
        return Err(TraceCapError::refused(
            operation,
            RefusalKind::IdentityChanged,
        ));
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    static FORCE_DESCRIPTOR_OBSERVATION_ERROR: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            Self::under(
                &std::env::current_dir()
                    .unwrap()
                    .join("target/tracecap-unit-tests"),
            )
        }

        /// A fresh private directory beneath `base`, which may be on any filesystem.
        fn under(base: &Path) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = base.join(format!(
                "unix-{}-{}",
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

    fn assert_cloexec(fd: RawFd) {
        let flags = unsafe {
            // SAFETY: every caller passes a live descriptor owned for the whole call.
            libc::fcntl(fd, libc::F_GETFD)
        };
        assert!(flags >= 0);
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
    }

    #[test]
    fn selected_root_name_preserves_the_filesystem_root_and_rejects_empty_input() {
        for spelling in ["/", "/.", "/./", "/./."] {
            assert_eq!(
                selected_root_name(Path::new(spelling)).unwrap(),
                Path::new("/"),
                "{spelling:?} must keep selecting the filesystem root"
            );
        }
        assert!(selected_root_name(Path::new("")).is_err());
    }

    #[test]
    fn acquired_root_and_child_descriptors_are_atomically_cloexec() {
        let tmp = Fixture::new();
        let root = TraceDirHandle::open(tmp.path()).unwrap();
        assert_cloexec(root.fd.as_raw_fd());
        let chain = root.open_append("chain.jsonl", true).unwrap();
        assert_cloexec(chain.file.as_raw_fd());
        let fixed = root.create_fixed("receipt.json").unwrap();
        assert_cloexec(fixed.file.as_raw_fd());
    }

    #[test]
    fn descriptor_observation_error_preserves_old_entry_and_leaves_no_owned_temp() {
        let tmp = Fixture::new();
        let root = TraceDirHandle::open(tmp.path()).unwrap();
        std::fs::write(tmp.path().join("trace.toml"), b"old").unwrap();
        FORCE_DESCRIPTOR_OBSERVATION_ERROR.with(|flag| flag.set(true));
        assert!(
            root.replace_descriptor("trace.toml.17.4.tmp", b"new")
                .is_err()
        );
        assert_eq!(
            std::fs::read(tmp.path().join("trace.toml")).unwrap(),
            b"old"
        );
        assert!(!tmp.path().join("trace.toml.17.4.tmp").exists());
    }

    #[test]
    fn descriptor_temp_collision_preserves_both_existing_entries() {
        let tmp = Fixture::new();
        let root = TraceDirHandle::open(tmp.path()).unwrap();
        let temporary = "trace.toml.17.5.tmp";
        std::fs::write(tmp.path().join("trace.toml"), b"old descriptor").unwrap();
        std::fs::write(tmp.path().join(temporary), b"owned by another writer").unwrap();

        assert!(
            root.replace_descriptor(temporary, b"new descriptor")
                .is_err()
        );

        assert_eq!(
            std::fs::read(tmp.path().join("trace.toml")).unwrap(),
            b"old descriptor"
        );
        assert_eq!(
            std::fs::read(tmp.path().join(temporary)).unwrap(),
            b"owned by another writer"
        );
        let mut names = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, ["trace.toml", temporary]);
    }

    #[test]
    fn descriptor_post_create_failure_removes_only_the_new_temporary() {
        let tmp = Fixture::new();
        let root = TraceDirHandle::open(tmp.path()).unwrap();
        let temporary = "trace.toml.17.6.tmp";
        std::fs::create_dir(tmp.path().join("trace.toml")).unwrap();
        std::fs::write(tmp.path().join("trace.toml").join("sentinel"), b"keep me").unwrap();

        assert!(
            root.replace_descriptor(temporary, b"new descriptor")
                .is_err()
        );

        assert_eq!(
            std::fs::read(tmp.path().join("trace.toml").join("sentinel")).unwrap(),
            b"keep me"
        );
        assert!(!tmp.path().join(temporary).exists());
        let names = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, ["trace.toml"]);
    }
}
