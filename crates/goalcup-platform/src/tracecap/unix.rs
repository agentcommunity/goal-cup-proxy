use super::{AppendAllocation, CleanupBudget, RefusalKind, TraceCapError};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::{ReservationFault, take_reservation_fault};
use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const ROOT_OPERATION: &str = "open trace root";
const RESERVE_OPERATION: &str = "reserve trace append stream";

/// The one `statfs(2)` `f_fstypename` measured keeping an `F_ALLOCATEPERSIST` reservation
/// across last close, `SIGKILL` and a real remount. HFS+ (`hfs`) and FAT32 (`msdos`) report
/// success and silently release it, so every other name is refused.
#[cfg(target_os = "macos")]
const QUALIFIED_ALLOCATOR: &[u8] = b"apfs";

/// The `statfs(2)` `f_type` magics whose `fallocate(FALLOC_FL_KEEP_SIZE)` blocks a later write
/// into the range is documented not to fail for space on: ext4 (`EXT4_SUPER_MAGIC`, shared with
/// ext2 and ext3, whose non-extent files refuse the call) and XFS (`XFS_SUPER_MAGIC`). Measured in
/// Docker on Linux 7.0.12 (aarch64) loop-mounted ext4 and XFS images: the allocation is retained
/// across close. Unlike the macOS allocator above, remount and `SIGKILL` were not measured on
/// Linux. btrfs (copy-on-write), ZFS, tmpfs, overlayfs, NFS, FUSE and every other magic are
/// refused.
#[cfg(target_os = "linux")]
const QUALIFIED_ALLOCATORS: [u64; 2] = [EXT4_SUPER_MAGIC, XFS_SUPER_MAGIC];
#[cfg(target_os = "linux")]
const EXT4_SUPER_MAGIC: u64 = 0xEF53;
#[cfg(target_os = "linux")]
const XFS_SUPER_MAGIC: u64 = 0x5846_5342;

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

    pub(super) fn replace_regular(
        &self,
        temporary: &str,
        target_name: &str,
        bytes: &[u8],
    ) -> Result<(), TraceCapError> {
        self.replace_named(temporary, target_name, bytes)
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
        name: &'static str,
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
        name: &'static str,
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
        name: &'static str,
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
            // SAFETY: the platform errno accessor returns a valid, writable thread-local pointer.
            unsafe { *errno_location() = 0 };
            let entry = unsafe {
                // SAFETY: directory remains live until closed below; readdir owns its buffer.
                libc::readdir(directory)
            };
            if entry.is_null() {
                // SAFETY: the platform errno accessor returns a valid thread-local pointer to read.
                let errno = unsafe { *errno_location() };
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

    /// Top this file's physical allocation up to `total_bytes`, its logical length included,
    /// without moving its logical end, then remeasure it (see `TraceDir::reserve_append`).
    ///
    /// One shape on both qualified platforms: a filesystem gate that refuses before anything
    /// is allocated, a free-space precheck on the missing difference, the platform's allocator,
    /// and the file's own `st_blocks × 512` as the only evidence. The allocator, the free-space
    /// measure (`available_bytes`) and the gate's allowlist (`qualified_volume`) differ per
    /// platform.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) fn reserve(&self, total_bytes: u64) -> Result<AppendAllocation, TraceCapError> {
        let fd = self.file.as_raw_fd();
        let fault = take_reservation_fault();
        let volume = fstatfs(fd, RESERVE_OPERATION)?;
        let qualified = match fault {
            #[cfg(target_os = "macos")]
            Some(ReservationFault::UnqualifiedFileSystem) => {
                qualified_allocator(&UNQUALIFIED_FSTYPENAME)
            }
            #[cfg(target_os = "linux")]
            Some(ReservationFault::UnqualifiedFileSystem) => qualified_allocator(UNQUALIFIED_MAGIC),
            _ => qualified_volume(&volume),
        };
        if !qualified {
            return Err(TraceCapError::refused(
                RESERVE_OPERATION,
                RefusalKind::UnsupportedFileSystem,
            ));
        }
        let before = measure_allocation(fd)?;
        let missing = total_bytes.saturating_sub(before.allocated_bytes);
        if missing > available_bytes(&volume) {
            return Err(TraceCapError::refused(
                RESERVE_OPERATION,
                RefusalKind::InsufficientSpace,
            ));
        }
        if fault != Some(ReservationFault::AllocatorGrantsNothing) {
            allocate(fd, total_bytes, missing)?;
        }
        let after = measure_allocation(fd)?;
        if after.logical_bytes != before.logical_bytes || after.allocated_bytes < total_bytes {
            return Err(TraceCapError::refused(
                RESERVE_OPERATION,
                RefusalKind::AllocationNotRetained,
            ));
        }
        Ok(after)
    }

    /// No allocator is qualified off macOS and Linux: refuse, never approximate.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(super) fn reserve(&self, _total_bytes: u64) -> Result<AppendAllocation, TraceCapError> {
        Err(TraceCapError::refused(
            RESERVE_OPERATION,
            RefusalKind::UnsupportedFileSystem,
        ))
    }
}

/// macOS (the measured T01 recipe): `F_PREALLOCATE` with `F_ALLOCATEALL | F_ALLOCATEPERSIST`
/// and `F_PEOFPOSMODE` grants all of the `missing` bytes or none, from the physical end of
/// file. It is additive, so a satisfied target (`missing == 0`) asks for nothing.
#[cfg(target_os = "macos")]
fn allocate(fd: RawFd, _total_bytes: u64, missing: u64) -> Result<(), TraceCapError> {
    if missing == 0 {
        return Ok(());
    }
    let length = libc::off_t::try_from(missing)
        .map_err(|_| TraceCapError::refused(RESERVE_OPERATION, RefusalKind::InsufficientSpace))?;
    let mut store = libc::fstore_t {
        fst_flags: libc::F_ALLOCATEALL | libc::F_ALLOCATEPERSIST,
        fst_posmode: libc::F_PEOFPOSMODE,
        fst_offset: 0,
        fst_length: length,
        fst_bytesalloc: 0,
    };
    let result = unsafe {
        // SAFETY: the retained append descriptor is live and `store` is a valid, writable
        // fstore_t for the duration of the call. F_ALLOCATEALL grants all or nothing; the
        // call never changes the logical length.
        libc::fcntl(fd, libc::F_PREALLOCATE, &raw mut store)
    };
    if result == -1 {
        return Err(TraceCapError::io(
            RESERVE_OPERATION,
            io::Error::last_os_error(),
        ));
    }
    // `fst_bytesalloc` echoes the request; the caller's remeasurement is the evidence.
    Ok(())
}

/// Linux (measured in Docker on ext4 and XFS): `fallocate(FALLOC_FL_KEEP_SIZE)` over the byte
/// range `[0, total_bytes)`. `KEEP_SIZE` is the macOS shape: blocks are granted behind the
/// logical end as unwritten extents and `st_size` never moves. The range is absolute rather
/// than "the missing difference from the physical end" because Linux has no physical-EOF mode;
/// it is idempotent, so a satisfied range grants nothing, and it is issued even when the block
/// count already looks sufficient because `st_blocks` also counts extent-map blocks, which
/// could otherwise hide a short data range. Unlike `F_ALLOCATEALL` a failure is not all or
/// nothing: ext4 and XFS may keep part of a failed request. That part is never logical content
/// (the size is kept) and the next call tops it up, so the failure is still a plain refusal.
///
/// `EOPNOTSUPP` is the 0xEF53 magic's ext2 and ext3 cases (indirect-block files have no
/// unwritten extents): an unsupported filesystem, not an I/O error.
#[cfg(target_os = "linux")]
fn allocate(fd: RawFd, total_bytes: u64, _missing: u64) -> Result<(), TraceCapError> {
    if total_bytes == 0 {
        return Ok(());
    }
    let length = libc::off_t::try_from(total_bytes)
        .map_err(|_| TraceCapError::refused(RESERVE_OPERATION, RefusalKind::InsufficientSpace))?;
    loop {
        let result = unsafe {
            // SAFETY: the retained append descriptor is live; KEEP_SIZE allocates blocks in
            // the range and never changes the logical length or any byte of content.
            libc::fallocate(fd, libc::FALLOC_FL_KEEP_SIZE, 0, length)
        };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EOPNOTSUPP) => {
                return Err(TraceCapError::refused(
                    RESERVE_OPERATION,
                    RefusalKind::UnsupportedFileSystem,
                ));
            }
            _ => return Err(TraceCapError::io(RESERVE_OPERATION, error)),
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn fstatfs(fd: RawFd, operation: &'static str) -> Result<libc::statfs, TraceCapError> {
    let mut volume = MaybeUninit::<libc::statfs>::uninit();
    let result = unsafe {
        // SAFETY: fd is live and the output buffer is writable storage of the required size.
        libc::fstatfs(fd, volume.as_mut_ptr())
    };
    if result != 0 {
        return Err(TraceCapError::io(operation, io::Error::last_os_error()));
    }
    // SAFETY: fstatfs reported success, so it fully initialised the output buffer.
    Ok(unsafe { volume.assume_init() })
}

/// The bytes the volume reports available to this unprivileged writer: necessary for a
/// reservation, never sufficient.
#[cfg(target_os = "macos")]
fn available_bytes(volume: &libc::statfs) -> u64 {
    volume.f_bavail.saturating_mul(u64::from(volume.f_bsize))
}

/// Linux `statfs(2)` counts `f_bavail` in units of `f_bsize`. Both field types vary by libc
/// and target, hence the conversions a 64-bit glibc build alone would not need.
#[cfg(target_os = "linux")]
#[allow(clippy::useless_conversion, clippy::unnecessary_fallible_conversions)]
fn available_bytes(volume: &libc::statfs) -> u64 {
    let blocks = u64::from(volume.f_bavail);
    let size = u64::try_from(volume.f_bsize).unwrap_or(0);
    blocks.saturating_mul(size)
}

#[cfg(target_os = "macos")]
fn qualified_volume(volume: &libc::statfs) -> bool {
    qualified_allocator(&volume.f_fstypename)
}

#[cfg(target_os = "linux")]
fn qualified_volume(volume: &libc::statfs) -> bool {
    qualified_allocator(statfs_magic(volume))
}

/// `f_type` is `__fsword_t` on glibc and unsigned elsewhere; every allowlisted magic is a
/// positive 32-bit value, so a sign-extended unknown can never match one.
#[cfg(target_os = "linux")]
#[allow(clippy::unnecessary_cast)]
fn statfs_magic(volume: &libc::statfs) -> u64 {
    volume.f_type as u64
}

/// What an injected [`ReservationFault::UnqualifiedFileSystem`] makes the gate observe: HFS+'s
/// measured `f_fstypename`.
#[cfg(target_os = "macos")]
const UNQUALIFIED_FSTYPENAME: [libc::c_char; 16] = {
    let mut name = [0; 16];
    name[0] = b'h' as libc::c_char;
    name[1] = b'f' as libc::c_char;
    name[2] = b's' as libc::c_char;
    name
};

/// What an injected [`ReservationFault::UnqualifiedFileSystem`] makes the gate observe on Linux:
/// tmpfs's `f_type` (`TMPFS_MAGIC`).
#[cfg(target_os = "linux")]
const UNQUALIFIED_MAGIC: u64 = 0x0102_1994;

/// The exact NUL-terminated measured name; an unterminated or differently spelled name is not.
#[cfg(target_os = "macos")]
fn qualified_allocator(fstypename: &[libc::c_char]) -> bool {
    fstypename
        .iter()
        .position(|unit| *unit == 0)
        .is_some_and(|length| {
            fstypename[..length]
                .iter()
                .map(|unit| *unit as u8)
                .eq(QUALIFIED_ALLOCATOR.iter().copied())
        })
}

/// Only an exact allowlisted `statfs(2)` `f_type` magic qualifies. Everything else —
/// btrfs, ZFS, tmpfs, overlayfs, NFS, FUSE, F2FS, any magic not listed — is refused.
#[cfg(target_os = "linux")]
fn qualified_allocator(magic: u64) -> bool {
    QUALIFIED_ALLOCATORS.contains(&magic)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn measure_allocation(fd: RawFd) -> Result<AppendAllocation, TraceCapError> {
    let stat = fstat(fd, RESERVE_OPERATION)?;
    let unmeasurable = || TraceCapError::refused(RESERVE_OPERATION, RefusalKind::NotRegularFile);
    Ok(AppendAllocation {
        logical_bytes: u64::try_from(stat.st_size).map_err(|_| unmeasurable())?,
        allocated_bytes: u64::try_from(stat.st_blocks)
            .ok()
            .and_then(|blocks| blocks.checked_mul(512))
            .ok_or_else(unmeasurable)?,
    })
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
/// # Safety
///
/// Returns the address of this thread's `errno`; the pointer is valid only on the calling thread.
unsafe fn errno_location() -> *mut libc::c_int {
    // SAFETY: the platform errno accessor takes no arguments and returns the calling thread's errno location, which is valid for the thread's lifetime.
    unsafe { libc::__error() }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
/// # Safety
///
/// Returns the address of this thread's `errno`; the pointer is valid only on the calling thread.
unsafe fn errno_location() -> *mut libc::c_int {
    // SAFETY: the platform errno accessor takes no arguments and returns the calling thread's errno location, which is valid for the thread's lifetime.
    unsafe { libc::__errno_location() }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
compile_error!("trace directory enumeration needs a target errno accessor");

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

    /// `(st_size, st_blocks × 512)` read by path, independently of the code under test.
    fn on_disk(path: &Path) -> (u64, u64) {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::symlink_metadata(path).unwrap();
        (meta.len(), meta.blocks() * 512)
    }

    /// A seeded chain stream beneath a fresh fixture.
    fn seeded_chain(tmp: &Fixture) -> (AppendFileHandle, PathBuf) {
        let root = TraceDirHandle::open(tmp.path()).unwrap();
        let mut chain = root.open_append("chain.jsonl", true).unwrap();
        chain.write_all(b"{\"seq\":0,\"seed\":true}\n").unwrap();
        (chain, tmp.path().join("chain.jsonl"))
    }

    #[cfg(target_os = "macos")]
    const ROUNDING: u64 = 64 * 1024;

    #[cfg(target_os = "macos")]
    fn fstypename(name: &[u8]) -> [libc::c_char; 16] {
        let mut raw = [0; 16];
        for (slot, byte) in raw.iter_mut().zip(name) {
            *slot = *byte as libc::c_char;
        }
        raw
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_the_measured_apfs_name_qualifies() {
        assert!(qualified_allocator(&fstypename(b"apfs")));
        for name in [
            b"hfs" as &[u8],
            b"msdos",
            b"hfs+",
            b"fat32",
            b"APFS",
            b"apfsx",
            b"exfat",
            b"smbfs",
            b"",
        ] {
            assert!(!qualified_allocator(&fstypename(name)), "{name:?}");
        }
        let mut unterminated = [b'x' as libc::c_char; 16];
        unterminated[..4].copy_from_slice(&fstypename(b"apfs")[..4]);
        assert!(!qualified_allocator(&unterminated));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reservation_allocates_behind_eof_and_tops_up_to_its_target() {
        let tmp = Fixture::new();
        let (chain, path) = seeded_chain(&tmp);
        let (seed_size, seed_allocated) = on_disk(&path);
        let target = 16 * 4096 * 3 + 123;
        assert!(seed_allocated < target);

        let reported = chain.reserve(target).unwrap();
        let (size, allocated) = on_disk(&path);
        assert_eq!(
            (reported.logical_bytes, reported.allocated_bytes),
            (size, allocated),
            "the reported allocation is the file's own measurement"
        );
        assert_eq!(size, seed_size, "logical EOF never moves");
        assert!(
            allocated >= target && allocated < target + ROUNDING,
            "{allocated}"
        );

        // F_PREALLOCATE is additive; the reservation is a target.
        chain.reserve(target).unwrap();
        assert_eq!(
            on_disk(&path),
            (size, allocated),
            "a satisfied target adds nothing"
        );
        chain.reserve(target / 2).unwrap();
        assert_eq!(on_disk(&path), (size, allocated), "nor does a smaller one");
        let larger = target + 3 * 4096 + 1;
        chain.reserve(larger).unwrap();
        let (grown_size, grown) = on_disk(&path);
        assert_eq!(grown_size, seed_size);
        assert!(grown >= larger && grown < larger + ROUNDING, "{grown}");
    }

    #[cfg(all(target_os = "macos", debug_assertions))]
    #[test]
    fn an_unqualified_filesystem_is_refused_before_any_allocation() {
        let tmp = Fixture::new();
        let (chain, path) = seeded_chain(&tmp);
        let before = on_disk(&path);
        assert!(!qualified_allocator(&UNQUALIFIED_FSTYPENAME));
        crate::tracecap::inject_reservation_fault(ReservationFault::UnqualifiedFileSystem);
        assert!(matches!(
            chain.reserve(256 * 1024),
            Err(TraceCapError::Refused {
                reason: RefusalKind::UnsupportedFileSystem,
                ..
            })
        ));
        assert_eq!(on_disk(&path), before, "nothing was allocated");
        // The fault was one-shot: the real APFS gate then admits the same call.
        chain.reserve(256 * 1024).unwrap();
        assert!(on_disk(&path).1 >= 256 * 1024);
    }

    #[cfg(all(target_os = "macos", debug_assertions))]
    #[test]
    fn an_allocation_that_does_not_remeasure_is_refused() {
        let tmp = Fixture::new();
        let (chain, path) = seeded_chain(&tmp);
        let before = on_disk(&path);
        crate::tracecap::inject_reservation_fault(ReservationFault::AllocatorGrantsNothing);
        assert!(matches!(
            chain.reserve(256 * 1024),
            Err(TraceCapError::Refused {
                reason: RefusalKind::AllocationNotRetained,
                ..
            })
        ));
        assert_eq!(on_disk(&path), before);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_request_beyond_available_space_is_refused_before_any_allocation() {
        let tmp = Fixture::new();
        let (chain, path) = seeded_chain(&tmp);
        let before = on_disk(&path);
        for impossible in [1u64 << 60, u64::MAX] {
            assert!(matches!(
                chain.reserve(impossible),
                Err(TraceCapError::Refused {
                    reason: RefusalKind::InsufficientSpace,
                    ..
                })
            ));
            assert_eq!(on_disk(&path), before);
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    #[test]
    fn reservation_is_refused_off_the_qualified_platform() {
        let tmp = Fixture::new();
        let (chain, path) = seeded_chain(&tmp);
        let before = on_disk(&path);
        assert!(matches!(
            chain.reserve(4096),
            Err(TraceCapError::Refused {
                reason: RefusalKind::UnsupportedFileSystem,
                ..
            })
        ));
        assert_eq!(on_disk(&path), before);
    }

    /// Linux: measured in Docker on Linux 7.0.12 (aarch64) loop-mounted ext4 and XFS images, and
    /// refused on overlayfs, tmpfs and FUSE. Which filesystem a test lands on is the host's, so
    /// every root is classified independently of the code under test (a path-based `statfs(2)`
    /// against magics written out here) and the expected outcome follows from that: an ext4 or
    /// XFS root must reserve, any other must refuse and allocate nothing. With
    /// `GOALCUP_TRACECAP_REQUIRE_QUALIFIED` set, finding no ext4 or XFS root is a failure rather
    /// than a skip.
    /// The roots are this crate's `target/`, `std::env::temp_dir()`, `/dev/shm` (tmpfs), and
    /// any directories listed in `GOALCUP_TRACECAP_TEST_ROOTS` (colon separated), which is how
    /// a by-hand run points them at a loop-mounted ext4 or XFS image.
    #[cfg(target_os = "linux")]
    mod linux_reservation {
        use super::*;

        const EXT4: u64 = 0xEF53;
        const XFS: u64 = 0x5846_5342;
        /// One top-up may round to the filesystem block (4 KiB on ext4 and XFS) and may count
        /// an extent-map block; far below a second target, so a doubled allocation cannot hide.
        const ROUNDING: u64 = 64 * 1024;

        fn magic_of(path: &Path) -> u64 {
            let encoded = CString::new(path.as_os_str().as_bytes()).unwrap();
            let mut volume = MaybeUninit::<libc::statfs>::uninit();
            let result = unsafe {
                // SAFETY: the path is NUL terminated and the output buffer is writable.
                libc::statfs(encoded.as_ptr(), volume.as_mut_ptr())
            };
            assert_eq!(result, 0, "statfs {}", path.display());
            // SAFETY: statfs reported success, so it fully initialised the output buffer.
            #[allow(clippy::unnecessary_cast)]
            let magic = unsafe { volume.assume_init() }.f_type as u64;
            magic
        }

        fn expected_qualified(magic: u64) -> bool {
            magic == EXT4 || magic == XFS
        }

        /// Every candidate root that exists and is a writable directory.
        fn roots() -> Vec<PathBuf> {
            let mut roots = vec![
                std::env::current_dir()
                    .unwrap()
                    .join("target/tracecap-unit-tests"),
                std::env::temp_dir(),
                PathBuf::from("/dev/shm"),
            ];
            if let Some(extra) = std::env::var_os("GOALCUP_TRACECAP_TEST_ROOTS") {
                roots.extend(std::env::split_paths(&extra));
            }
            roots.retain(|root| {
                std::fs::create_dir_all(root).is_ok()
                    && std::fs::metadata(root)
                        .is_ok_and(|meta| meta.is_dir() && !meta.permissions().readonly())
            });
            roots
        }

        /// A seeded chain under a fresh fixture on the first ext4 or XFS root, if any.
        fn qualified_fixture(test: &str) -> Option<Fixture> {
            let root = roots()
                .into_iter()
                .find(|root| expected_qualified(magic_of(root)));
            if root.is_none() && std::env::var_os("GOALCUP_TRACECAP_REQUIRE_QUALIFIED").is_some() {
                panic!(
                    "{test}: GOALCUP_TRACECAP_REQUIRE_QUALIFIED is set but no ext4 or XFS root is \
                     reachable"
                );
            }
            if root.is_none() {
                println!(
                    "{test}: no ext4 or XFS root is reachable here; set \
                     GOALCUP_TRACECAP_TEST_ROOTS to one to run it"
                );
            }
            root.map(|root| Fixture::under(&root))
        }

        #[test]
        fn only_the_ext4_and_xfs_magics_qualify() {
            assert!(qualified_allocator(EXT4));
            assert!(qualified_allocator(XFS));
            for magic in [
                0x9123_683e, // btrfs
                0x2fc1_2fc1, // zfs
                0x0102_1994, // tmpfs
                0x8584_58f6, // ramfs
                0x794c_7630, // overlayfs
                0x6969,      // nfs
                0x6573_5546, // fuse (virtiofs included)
                0xf2f5_2010, // f2fs
                0xef51,      // ext2's old magic
                0x5846_5343, // one off XFS
                0x0000_ef53_0000_0000,
                0xffff_ffff_ffff_ef53, // a sign-extended negative, never a magic
                0,
            ] {
                assert!(!qualified_allocator(magic), "{magic:#x}");
            }
            assert!(!qualified_allocator(UNQUALIFIED_MAGIC));
        }

        #[test]
        fn every_reachable_root_reserves_exactly_when_it_is_ext4_or_xfs() {
            let mut visited_qualified = false;
            for root in roots() {
                let magic = magic_of(&root);
                let qualified = expected_qualified(magic);
                visited_qualified |= qualified;
                println!(
                    "{}: f_type {magic:#x}, qualified {qualified}",
                    root.display()
                );
                let tmp = Fixture::under(&root);
                let (chain, path) = seeded_chain(&tmp);
                let (seed_size, seed_allocated) = on_disk(&path);
                let target = 16 * 4096 * 3 + 123;
                assert!(seed_allocated < target);

                if !qualified {
                    assert!(
                        matches!(
                            chain.reserve(target),
                            Err(TraceCapError::Refused {
                                reason: RefusalKind::UnsupportedFileSystem,
                                ..
                            })
                        ),
                        "{}",
                        root.display()
                    );
                    assert_eq!(
                        on_disk(&path),
                        (seed_size, seed_allocated),
                        "nothing was allocated on {}",
                        root.display()
                    );
                    continue;
                }

                let reported = chain.reserve(target).unwrap();
                let (size, allocated) = on_disk(&path);
                assert_eq!(
                    (reported.logical_bytes, reported.allocated_bytes),
                    (size, allocated),
                    "the reported allocation is the file's own measurement"
                );
                assert_eq!(size, seed_size, "logical EOF never moves");
                assert!(
                    allocated >= target && allocated < target + ROUNDING,
                    "{allocated}"
                );
                assert_eq!(
                    std::fs::read(&path).unwrap(),
                    b"{\"seq\":0,\"seed\":true}\n",
                    "no allocator fill reached the logical data"
                );

                // A satisfied range grants nothing; nor does a smaller one; a larger one tops up.
                chain.reserve(target).unwrap();
                assert_eq!(on_disk(&path), (size, allocated));
                chain.reserve(target / 2).unwrap();
                assert_eq!(on_disk(&path), (size, allocated));
                let larger = target + 3 * 4096 + 1;
                chain.reserve(larger).unwrap();
                let (grown_size, grown) = on_disk(&path);
                assert_eq!(grown_size, seed_size);
                assert!(grown >= larger && grown < larger + ROUNDING, "{grown}");

                // The blocks outlive the descriptor (close only; a remount is not tested here).
                drop(chain);
                assert_eq!(on_disk(&path), (seed_size, grown));
            }
            assert!(
                visited_qualified
                    || std::env::var_os("GOALCUP_TRACECAP_REQUIRE_QUALIFIED").is_none(),
                "GOALCUP_TRACECAP_REQUIRE_QUALIFIED is set but no ext4 or XFS root was visited"
            );
        }

        #[test]
        fn appends_land_inside_the_reservation_without_growing_it() {
            let Some(tmp) = qualified_fixture("appends_land_inside_the_reservation") else {
                return;
            };
            let (mut chain, path) = seeded_chain(&tmp);
            let (seed_size, _) = on_disk(&path);
            let target = 256 * 1024;
            chain.reserve(target).unwrap();
            let (_, allocated) = on_disk(&path);
            let line = [b'x'; 4095];
            for _ in 0..32 {
                chain.write_all(&line).unwrap();
                chain.write_all(b"\n").unwrap();
            }
            chain.sync_all().unwrap();
            let (size, after) = on_disk(&path);
            assert_eq!(size, seed_size + 32 * 4096);
            assert!(after >= target, "{after}");
            assert!(after < allocated + ROUNDING, "{after} against {allocated}");
            assert_eq!(chain.reserve(target).unwrap().allocated_bytes, after);
        }

        #[cfg(debug_assertions)]
        #[test]
        fn an_unqualified_filesystem_is_refused_before_any_allocation() {
            let Some(tmp) = qualified_fixture("an_unqualified_filesystem_is_refused") else {
                return;
            };
            let (chain, path) = seeded_chain(&tmp);
            let before = on_disk(&path);
            crate::tracecap::inject_reservation_fault(ReservationFault::UnqualifiedFileSystem);
            assert!(matches!(
                chain.reserve(256 * 1024),
                Err(TraceCapError::Refused {
                    reason: RefusalKind::UnsupportedFileSystem,
                    ..
                })
            ));
            assert_eq!(on_disk(&path), before, "nothing was allocated");
            // The fault was one-shot: the real ext4/XFS gate then admits the same call.
            chain.reserve(256 * 1024).unwrap();
            assert!(on_disk(&path).1 >= 256 * 1024);
        }

        #[cfg(debug_assertions)]
        #[test]
        fn an_allocation_that_does_not_remeasure_is_refused() {
            let Some(tmp) = qualified_fixture("an_allocation_that_does_not_remeasure") else {
                return;
            };
            let (chain, path) = seeded_chain(&tmp);
            let before = on_disk(&path);
            crate::tracecap::inject_reservation_fault(ReservationFault::AllocatorGrantsNothing);
            assert!(matches!(
                chain.reserve(256 * 1024),
                Err(TraceCapError::Refused {
                    reason: RefusalKind::AllocationNotRetained,
                    ..
                })
            ));
            assert_eq!(on_disk(&path), before);
        }

        #[test]
        fn a_request_beyond_available_space_is_refused_before_any_allocation() {
            let Some(tmp) = qualified_fixture("a_request_beyond_available_space") else {
                return;
            };
            let (chain, path) = seeded_chain(&tmp);
            let before = on_disk(&path);
            for impossible in [1u64 << 60, u64::MAX] {
                assert!(matches!(
                    chain.reserve(impossible),
                    Err(TraceCapError::Refused {
                        reason: RefusalKind::InsufficientSpace,
                        ..
                    })
                ));
                assert_eq!(on_disk(&path), before);
            }
        }
    }
}
